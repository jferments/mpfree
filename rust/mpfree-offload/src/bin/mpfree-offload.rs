//! `mpfree-offload`: the offload server, and a tool that measures a link.
//!
//! ```text
//! mpfree-offload serve [--listen ADDR] [--rate HZ] [--allow-lan] NODE...
//! mpfree-offload measure [--node NAME] [--partitions N] [--latency L]
//!                        [--seconds S] [--rate HZ] [--frames N] [--server ADDR]
//! ```
//!
//! `serve` renders one partition per NODE (echo, cloud-full, cloud-eco) and
//! prints `listening on ADDR` first. It listens on 127.0.0.1 unless given
//! `--allow-lan`, since it has no pairing yet, and stops when its standard
//! input closes.
//!
//! `measure` plays blocks at the real period, as an audio thread would,
//! through N partitions of NODE, and reports the round trip's tail, the
//! server's render time, what each partition costs this side's audio
//! thread, the misses, and whether every answer matched the same node run
//! here, L blocks later. Without `--server` it starts `serve` on loopback as
//! a separate process and stops it at the end.
//!
//! Exit status: 0 when every answer due came back in time and matched, 1
//! otherwise, 2 for a usage or start-up error.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, UdpSocket};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_node::{Ctx, Node};
use mpfree_offload::client::{self, Config, Outcome, MAX_LATENCY};
use mpfree_offload::server::{self, Server};
use mpfree_offload::wire::{HEADER_BYTES, MAX_FRAMES};
use mpfree_offload::{node_by_name, NODE_NAMES};

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  mpfree-offload serve [--listen ADDR] [--rate HZ] [--allow-lan] NODE...\n  \
         mpfree-offload measure [--node NAME] [--partitions N] [--latency L] [--seconds S] \
         [--rate HZ] [--frames N] [--server ADDR]\nnodes: {}",
        NODE_NAMES.join(", ")
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve_main(&args[1..]),
        Some("measure") => measure_main(&args[1..]),
        _ => usage(),
    }
}

/// `--name value` pairs and bare words.
struct Args {
    pairs: Vec<(String, String)>,
    flags: Vec<String>,
    words: Vec<String>,
}

fn parse_args(a: &[String], with_value: &[&str], flags: &[&str]) -> Result<Args, String> {
    let mut out = Args {
        pairs: Vec::new(),
        flags: Vec::new(),
        words: Vec::new(),
    };
    let mut i = 0;
    while i < a.len() {
        let s = &a[i];
        if with_value.contains(&s.as_str()) {
            let v = a.get(i + 1).ok_or(format!("{s} needs a value"))?;
            out.pairs.push((s.clone(), v.clone()));
            i += 2;
        } else if flags.contains(&s.as_str()) {
            out.flags.push(s.clone());
            i += 1;
        } else if s.starts_with("--") {
            return Err(format!("unknown option {s}"));
        } else {
            out.words.push(s.clone());
            i += 1;
        }
    }
    Ok(out)
}

impl Args {
    fn get<T: std::str::FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        match self.pairs.iter().rev().find(|(k, _)| k == name) {
            None => Ok(default),
            Some((_, v)) => v.parse().map_err(|_| format!("{name}: can't read {v:?}")),
        }
    }

    fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }
}

fn serve_main(a: &[String]) -> ExitCode {
    let args = match parse_args(a, &["--listen", "--rate"], &["--allow-lan"]) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("mpfree-offload serve: {e}");
            return usage();
        }
    };
    let (listen, rate) = match (
        args.get::<SocketAddr>("--listen", "127.0.0.1:0".parse().unwrap()),
        args.get::<u32>("--rate", 44_100),
    ) {
        (Ok(l), Ok(r)) => (l, r),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("mpfree-offload serve: {e}");
            return usage();
        }
    };
    if !listen.ip().is_loopback() && !args.has("--allow-lan") {
        eprintln!(
            "mpfree-offload serve: the server has no pairing yet, so it listens only on loopback. \
             Pass --allow-lan to listen on {listen} anyway: anyone who can reach that address \
             could then have it render audio."
        );
        return ExitCode::from(2);
    }
    if args.words.is_empty() {
        eprintln!("mpfree-offload serve: name at least one node");
        return usage();
    }
    let mut srv = Server::new(rate);
    for w in &args.words {
        match node_by_name(w) {
            Some(n) => {
                srv.add(n);
            }
            None => {
                eprintln!("mpfree-offload serve: no node called {w:?}");
                return usage();
            }
        }
    }
    let socket = match UdpSocket::bind(listen) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mpfree-offload serve: can't listen on {listen}: {e}");
            return ExitCode::from(2);
        }
    };
    let addr = socket
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    println!("listening on {addr}");
    println!(
        "partitions: {}",
        args.words
            .iter()
            .enumerate()
            .map(|(i, w)| format!("{i} {w}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let stop = Arc::new(AtomicBool::new(false));
    {
        // Stop when standard input closes: the measuring parent's way of
        // saying it's done (and a terminal's ^D).
        let stop = stop.clone();
        std::thread::spawn(move || {
            let _ = std::io::stdin().read_to_end(&mut Vec::new());
            stop.store(true, Ordering::Relaxed);
        });
    }
    match server::serve(&socket, &mut srv, &stop) {
        Ok(st) => {
            // The measuring parent may have stopped reading by now.
            let _ = writeln!(
                std::io::stdout(),
                "served {} requests, refused {}; per partition: {}",
                st.answered,
                st.refused,
                (0..srv.partitions() as u16)
                    .map(|p| {
                        let s = srv.stats(p).unwrap_or_default();
                        format!(
                            "{p}: {} answered, {} gaps, {} late",
                            s.answered, s.gaps, s.stale
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("mpfree-offload serve: {e}");
            ExitCode::from(2)
        }
    }
}

/// A server process this tool started; stopped when dropped.
struct Spawned(Child);

impl Drop for Spawned {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.0.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_server(node: &str, n: usize, rate: u32) -> Result<(Spawned, SocketAddr), String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't find this program: {e}"))?;
    let mut cmd = Command::new(exe);
    cmd.args([
        "serve",
        "--listen",
        "127.0.0.1:0",
        "--rate",
        &rate.to_string(),
    ]);
    for _ in 0..n {
        cmd.arg(node);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("can't start the server: {e}"))?;
    let out = child.stdout.take().ok_or("no server output")?;
    let spawned = Spawned(child);
    let mut line = String::new();
    let mut out = BufReader::new(out);
    out.read_line(&mut line)
        .map_err(|e| format!("reading the server's address: {e}"))?;
    // Keep reading what else it says, so it never writes into a closed pipe.
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut out, &mut std::io::sink());
    });
    let addr = line
        .trim()
        .strip_prefix("listening on ")
        .and_then(|a| a.parse().ok())
        .ok_or(format!("the server said {:?}", line.trim()))?;
    Ok((spawned, addr))
}

/// A deterministic test signal: noise bursts with gaps, so a reverb has
/// both attacks and tails to work on.
struct Signal(u64);

impl Signal {
    fn block(&mut self, frame: u64, rate: u32, l: &mut [f32], r: &mut [f32]) {
        for (i, (a, b)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
            let t = (frame + i as u64) % (rate as u64);
            let on = t < rate as u64 / 8;
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let x = ((self.0 >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.5;
            *a = if on { x } else { 0.0 };
            *b = if on { -x * 0.8 } else { 0.0 };
        }
    }
}

fn pct(v: &mut [u32], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_unstable();
    let i = ((p * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1;
    v[i] as f64
}

fn row(name: &str, v: &mut [u32], unit_div: f64) -> String {
    format!(
        "{name:<24}{:>9.1}{:>9.1}{:>9.1}{:>9.1}",
        pct(v, 0.50) / unit_div,
        pct(v, 0.99) / unit_div,
        pct(v, 0.999) / unit_div,
        pct(v, 1.0) / unit_div
    )
}

struct Part {
    remote: client::Remote,
    link: client::Link,
    local: Box<dyn Node>,
    /// The local render of the last L + 1 blocks, by block number.
    history: Vec<(Vec<f32>, Vec<f32>)>,
    /// The first block whose answer didn't match (or None).
    mismatch: Option<u64>,
    /// The first answer after a lost request (flagged GAP): comparisons
    /// stop there, since the remote node skipped frames the local one
    /// didn't. A late answer doesn't stop them: the server still rendered
    /// that block, it just wasn't played.
    first_gap: Option<u64>,
    compared: u64,
}

struct MeasureArgs {
    node: String,
    parts_n: usize,
    latency: u32,
    seconds: f64,
    rate: u32,
    frames: usize,
    server: Option<SocketAddr>,
}

fn measure_main(a: &[String]) -> ExitCode {
    let args = match parse_args(
        a,
        &[
            "--node",
            "--partitions",
            "--latency",
            "--seconds",
            "--rate",
            "--frames",
            "--server",
        ],
        &[],
    ) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("mpfree-offload measure: {e}");
            return usage();
        }
    };
    let read = || -> Result<MeasureArgs, String> {
        let server = match args.pairs.iter().rev().find(|(k, _)| k == "--server") {
            None => None,
            Some((_, v)) => Some(
                v.parse()
                    .map_err(|_| format!("--server: can't read {v:?}"))?,
            ),
        };
        Ok(MeasureArgs {
            node: args.get("--node", "cloud-full".to_string())?,
            parts_n: args.get("--partitions", 1usize)?,
            latency: args.get("--latency", 2u32)?,
            seconds: args.get("--seconds", 20.0f64)?,
            rate: args.get("--rate", 44_100u32)?,
            frames: args.get("--frames", 128usize)?,
            server,
        })
    };
    let MeasureArgs {
        node,
        parts_n,
        latency,
        seconds,
        rate,
        frames,
        server,
    } = match read() {
        Ok(x) => x,
        Err(e) => {
            eprintln!("mpfree-offload measure: {e}");
            return usage();
        }
    };
    if node_by_name(&node).is_none()
        || !(1..=16).contains(&parts_n)
        || !(1..=MAX_LATENCY).contains(&latency)
        || !(0.1..=3600.0).contains(&seconds)
        || !(8_000..=192_000).contains(&rate)
        || !(1..=MAX_FRAMES).contains(&frames)
    {
        eprintln!(
            "mpfree-offload measure: --node is one of {}, --partitions 1 to 16, --latency 1 to {MAX_LATENCY}, \
             --seconds 0.1 to 3600, --rate 8000 to 192000, --frames 1 to {MAX_FRAMES}",
            NODE_NAMES.join(", ")
        );
        return ExitCode::from(2);
    }

    let (_spawned, addr, whose) = match server {
        Some(a) => (None, a, "a server you started"),
        None => match spawn_server(&node, parts_n, rate) {
            Ok((s, a)) => (Some(s), a, "a server process on this machine"),
            Err(e) => {
                eprintln!("mpfree-offload measure: {e}");
                return ExitCode::from(2);
            }
        },
    };

    let mut parts = Vec::new();
    for p in 0..parts_n {
        let cfg = Config {
            partition: p as u16,
            latency,
            frames,
            send_audio: true,
        };
        let (remote, link) = match client::connect(addr, cfg) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("mpfree-offload measure: can't open a link to {addr}: {e}");
                return ExitCode::from(2);
            }
        };
        let mut local = node_by_name(&node).expect("checked above");
        local.prepare(rate, MAX_FRAMES);
        parts.push(Part {
            remote,
            link,
            local,
            history: (0..=latency)
                .map(|_| (vec![0.0; frames], vec![0.0; frames]))
                .collect(),
            mismatch: None,
            first_gap: None,
            compared: 0,
        });
    }

    let period = Duration::from_secs_f64(frames as f64 / rate as f64);
    let blocks = (seconds / period.as_secs_f64()).ceil() as u64;
    let mut cost_ns: Vec<u32> = Vec::with_capacity((blocks as usize) * parts_n);
    let (mut il, mut ir) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    let mut signal = Signal(1);
    let mut behind = 0u64;
    let start = Instant::now() + Duration::from_millis(100);
    for k in 0..blocks {
        let target = start + period * k as u32;
        let now = Instant::now();
        if now < target {
            std::thread::sleep(target - now);
        } else if now > target + period {
            behind += 1;
        }
        let frame = k * frames as u64;
        signal.block(frame, rate, &mut il, &mut ir);
        for p in &mut parts {
            l.copy_from_slice(&il);
            r.copy_from_slice(&ir);
            let gaps_before = p.remote.stats().gaps;
            let t = Instant::now();
            let out = p.remote.process(frame, &[], &mut l, &mut r);
            cost_ns.push(t.elapsed().as_nanos().min(u32::MAX as u128) as u32);

            // The same node here, for the comparison.
            let slot = (k % (latency as u64 + 1)) as usize;
            let (hl, hr) = &mut p.history[slot];
            hl.copy_from_slice(&il);
            hr.copy_from_slice(&ir);
            p.local.process(&Ctx::new(rate, frame), &[], hl, hr);
            if p.remote.stats().gaps > gaps_before {
                p.first_gap.get_or_insert(k - latency as u64);
            }
            match out {
                Outcome::Warmup | Outcome::Miss => {}
                Outcome::Hit => {
                    if p.first_gap.is_none() && p.mismatch.is_none() {
                        let due = ((k - latency as u64) % (latency as u64 + 1)) as usize;
                        let (el, er) = &p.history[due];
                        if *el == l && *er == r {
                            p.compared += 1;
                        } else {
                            p.mismatch = Some(k - latency as u64);
                        }
                    }
                }
            }
        }
    }
    let wall = start.elapsed();

    // Report.
    let mut rtt = Vec::new();
    let mut srv = Vec::new();
    let mut ok = true;
    let mut lines = Vec::new();
    let (mut misses, mut due, mut late, mut lost, mut gaps, mut ring_full) = (0, 0, 0, 0, 0, 0);
    for (i, p) in parts.into_iter().enumerate() {
        let st = p.remote.stats();
        let m = p.link.close();
        rtt.extend(m.round_trip_us.iter().copied());
        srv.extend(m.server_us.iter().map(|&x| x as u32));
        misses += st.misses;
        due += st.hits + st.misses;
        late += m.late;
        lost += m.sent.saturating_sub(m.received);
        gaps += st.gaps;
        ring_full += st.ring_full;
        let verdict = match (p.mismatch, p.first_gap) {
            (Some(b), _) => {
                ok = false;
                format!("DIFFERS from the local render at block {b}")
            }
            (None, Some(b)) => format!(
                "matched the local render for {} answers, until a lost request before block {b}",
                p.compared
            ),
            (None, None) => format!("matched the local render for all {} answers", p.compared),
        };
        lines.push(format!(
            "Partition {i}: {verdict}; {} errors on the link",
            m.errors
        ));
    }
    if misses > 0 {
        ok = false;
    }
    let deadline_us = latency as f64 * period.as_secs_f64() * 1e6;
    let worst = pct(&mut rtt.clone(), 1.0);
    let bytes = HEADER_BYTES + 2 * frames * 4;
    println!(
        "mpfree-offload measure: {parts_n} partition{} of {node} over {addr} ({whose})",
        if parts_n == 1 { "" } else { "s" }
    );
    println!(
        "L = {latency} period{} ({:.1} ms) at {frames} frames, {rate} Hz; {:.1} s, {blocks} blocks a partition",
        if latency == 1 { "" } else { "s" },
        deadline_us / 1000.0,
        wall.as_secs_f64()
    );
    println!();
    println!(
        "{:<24}{:>9}{:>9}{:>9}{:>9}   (µs)",
        "", "p50", "p99", "p99.9", "max"
    );
    println!("{}", row("Round trip", &mut rtt, 1.0));
    println!("{}", row("Server's render", &mut srv, 1.0));
    println!(
        "{}   (encode, hand over, take the answer)",
        row("This side, per block", &mut cost_ns, 1000.0)
    );
    println!();
    println!(
        "Misses: {misses} of {due} answers due ({:.1} a minute); late answers {late}; lost {lost}; gaps {gaps}; ring full {ring_full}",
        misses as f64 / wall.as_secs_f64() * 60.0
    );
    println!(
        "Link: {:.2} Mbit/s each way per partition ({bytes}-byte datagrams)",
        bytes as f64 * 8.0 / period.as_secs_f64() / 1e6
    );
    println!(
        "Deadline: an answer has {:.0} µs; the worst round trip used {:.0} % of it",
        deadline_us,
        worst / deadline_us * 100.0
    );
    if behind > 0 {
        println!("This machine fell more than a period behind {behind} times; its own timing adds to the numbers above.");
    }
    for l in lines {
        println!("{l}");
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
