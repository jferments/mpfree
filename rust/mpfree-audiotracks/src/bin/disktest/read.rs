//! `mpfree-disktest --read-only`: reads files already on partition 4 the
//! way the streaming engine will, and writes nothing (step D1 of
//! docs/dev/design/disk-streaming.md).
//!
//! It reads the audio files under `library/` and every file under
//! `backup/` (a creator stick's eMMC backup: one long file the streams
//! share out), regular files only, never through a link, never off
//! partition 4, at most 5,000. A file longer than 64 MiB is read as
//! pieces of 64 MiB, so one long file can feed many streams. Each file is
//! opened read-only with `O_NOFOLLOW`, and only the pieces about to be read
//! are dropped from the cache (`posix_fadvise DONTNEED` on their range), so
//! every read reaches the stick. Then:
//!
//! 1. A sequential read of the largest pieces, as fast as it goes.
//! 2. Streams, each a thread reading its own pieces in turn at the pace of
//!    one stereo track at 44.1 kHz: 4, 8, 16 and 24 streams of 24-bit, each
//!    with 64, 128, 256 and 512 KiB reads, then 16 streams of 16-bit and
//!    of float. A stream's start is spread over its first read's time, a
//!    read that ends a piece is short and the next one is due sooner (the
//!    pace is in bytes), and a stream that reaches the end of its pieces
//!    drops them from the cache and starts again.
//!
//! For each it gives the time per read (p50 to p99.99 and max, and how many
//! took over 50, 100, 250 and 500 ms), the worst shortfall (how far a
//! stream fell behind its schedule), the longest time a stream stayed
//! behind, and the window and RAM the plan's rule asks for: 2 x the worst
//! shortfall + 1 s, at least 4 s, a stream. It prints counts and sizes,
//! never a file's name, and checks at the end that the files it read are
//! as they were.

use super::{mbps, stopped, Times, MIB, STOP};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// The library's folder inside the data folder (`mpfree_library::LIBRARY`):
/// its audio files are read.
pub const LIBRARY: &str = "library";
/// A creator stick's eMMC backup inside the data folder: every file is read.
pub const BACKUP: &str = "backup";
/// Longer files are read as pieces of this size (1 MiB in a quick run).
const PIECE: u64 = 64 << 20;
/// The kinds of audio file read, by extension, any case.
const EXTENSIONS: [&str; 4] = ["wav", "aif", "aiff", "flac"];
/// Smaller files are left out: a stream reads long files.
const MIN_FILE: u64 = 64 * 1024;
const MAX_FILES: usize = 5000;
const MAX_DEPTH: usize = 16;
const KIB: usize = 1024;
/// Bytes a second of one stereo stream at 44.1 kHz.
const RATE_16: f64 = 44_100.0 * 2.0 * 2.0;
const RATE_24: f64 = 44_100.0 * 2.0 * 3.0;
const RATE_F32: f64 = 44_100.0 * 2.0 * 4.0;
/// The plan's window rule: 2 x the worst shortfall + 1 s, at least 4 s.
const WINDOW_MIN_S: f64 = 4.0;
/// Every stream's first read is due this long after the threads start,
/// so starting a thread is not counted as falling behind.
const LEAD_S: f64 = 0.05;

/// Why a read-only run did not finish: refused before reading anything
/// (exit code 3), or failed while reading (exit code 1).
pub enum Fail {
    Refused(String),
    Failed(String),
}

impl From<String> for Fail {
    fn from(e: String) -> Fail {
        Fail::Failed(e)
    }
}

/// A stretch of a file a stream reads: the whole file, or 64 MiB of it.
struct Piece {
    path: PathBuf,
    start: u64,
    len: u64,
}

/// What the walk found, and what it left out.
#[derive(Default)]
struct Found {
    pieces: Vec<Piece>,
    files: [usize; 2],
    bytes: [u64; 2],
    largest: u64,
    links: usize,
    other_fs: usize,
    small: usize,
    over_limit: bool,
}

/// Each file's length and change time, to check nothing changed.
type Snapshot = BTreeMap<PathBuf, (u64, i64, i64)>;

fn is_audio(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// The pieces to read under `base` (the audio in `library/`, every file
/// in `backup/`), largest first, and a snapshot of every file seen there.
/// Never follows a link or leaves `base`'s file system.
fn find(base: &Path, piece: u64) -> Result<(Found, Snapshot), Fail> {
    let dev = fs::symlink_metadata(base)
        .map_err(|e| Fail::Refused(format!("{}: {e}", base.display())))?
        .dev();
    let mut found = Found::default();
    let mut snap = Snapshot::new();
    let mut count = 0usize;
    for (place, (name, audio_only)) in [(LIBRARY, true), (BACKUP, false)].into_iter().enumerate() {
        let top = base.join(name);
        match fs::symlink_metadata(&top) {
            Ok(m) if m.is_dir() && m.dev() == dev => {}
            Ok(m) if m.file_type().is_symlink() => {
                found.links += 1;
                continue;
            }
            _ => continue,
        }
        let mut stack = vec![(top, 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(rd) = fs::read_dir(&dir) else {
                continue;
            };
            let mut names: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            names.sort();
            for p in names {
                let Ok(m) = fs::symlink_metadata(&p) else {
                    continue;
                };
                if m.file_type().is_symlink() {
                    found.links += 1;
                } else if m.dev() != dev {
                    found.other_fs += 1;
                } else if m.is_dir() {
                    if depth + 1 < MAX_DEPTH {
                        stack.push((p, depth + 1));
                    }
                } else if m.is_file() {
                    snap.insert(p.clone(), (m.len(), m.mtime(), m.mtime_nsec()));
                    if audio_only && !is_audio(&p) {
                        continue;
                    }
                    if m.len() < MIN_FILE {
                        found.small += 1;
                    } else if count >= MAX_FILES {
                        found.over_limit = true;
                    } else {
                        count += 1;
                        found.files[place] += 1;
                        found.bytes[place] += m.len();
                        found.largest = found.largest.max(m.len());
                        let mut start = 0;
                        while start < m.len() {
                            let len = (m.len() - start).min(piece);
                            found.pieces.push(Piece {
                                path: p.clone(),
                                start,
                                len,
                            });
                            start += len;
                        }
                    }
                }
            }
        }
    }
    found.pieces.sort_by(|a, b| {
        b.len
            .cmp(&a.len)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.start.cmp(&b.start))
    });
    Ok((found, snap))
}

/// Open read-only, never through a link, and only on file system `dev`.
fn open(p: &Path, dev: u64) -> std::io::Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(p)?;
    let m = f.metadata()?;
    if m.dev() != dev || !m.is_file() {
        return Err(std::io::Error::other("not a plain file on partition 4"));
    }
    Ok(f)
}

/// Read until `buf` is full or the file ends; the bytes read.
fn fill(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Drop one piece's pages from the cache: advice only, on that range.
/// The 64-bit call, so offsets past 2 GiB are right on the MPC's 32-bit ARM.
fn drop_range(f: &File, start: u64, len: u64) {
    // SAFETY: a valid descriptor; advice only, on this range of this file.
    unsafe {
        libc::posix_fadvise64(
            f.as_raw_fd(),
            start as libc::off64_t,
            len as libc::off64_t,
            libc::POSIX_FADV_DONTNEED,
        );
    }
}

/// Drop the pages of these pieces from the cache (opening each read-only).
fn drop_pieces(pieces: &[&Piece], dev: u64) -> Result<(), String> {
    for a in pieces {
        let f = open(&a.path, dev).map_err(|e| format!("open a file on partition 4: {e}"))?;
        drop_range(&f, a.start, a.len);
    }
    Ok(())
}

/// Open a piece at its start.
fn open_piece(a: &Piece, dev: u64) -> std::io::Result<File> {
    let mut f = open(&a.path, dev)?;
    f.seek(SeekFrom::Start(a.start))?;
    Ok(f)
}

#[derive(Clone, Copy)]
struct Config {
    streams: usize,
    read: usize,
    rate: f64,
    kind: &'static str,
}

struct Plan {
    piece: u64,
    secs: f64,
    seq_max: u64,
    seq_secs: f64,
    configs: Vec<Config>,
}

fn plan(quick: bool) -> Plan {
    let mut configs = Vec::new();
    // Quick (the tests): few streams, short, and 16 times the rate so a
    // stream makes some reads in its fraction of a second.
    let (counts, reads, speed, formats_at): (&[usize], &[usize], f64, usize) = if quick {
        (&[2, 4], &[64, 128], 16.0, 4)
    } else {
        (&[4, 8, 16, 24], &[64, 128, 256, 512], 1.0, 16)
    };
    for &streams in counts {
        for &read in reads {
            configs.push(Config {
                streams,
                read: read * KIB,
                rate: RATE_24 * speed,
                kind: "24-bit",
            });
        }
    }
    for (rate, kind) in [(RATE_16, "16-bit"), (RATE_F32, "float")] {
        configs.push(Config {
            streams: formats_at,
            read: 128 * KIB,
            rate: rate * speed,
            kind,
        });
    }
    Plan {
        piece: if quick { MIB } else { PIECE },
        secs: if quick { 0.4 } else { 20.0 },
        seq_max: if quick { 4 * MIB } else { 256 * MIB },
        seq_secs: if quick { 1.0 } else { 15.0 },
        configs,
    }
}

/// Give each stream its own pieces, largest first to the stream with the
/// least, until each has `want` bytes or the pieces run out. None if a
/// stream would have fewer than two reads' worth.
fn assign(files: &[Piece], streams: usize, want: u64, read: usize) -> Option<Vec<Vec<&Piece>>> {
    if files.len() < streams {
        return None;
    }
    let mut out: Vec<Vec<&Piece>> = vec![Vec::new(); streams];
    let mut bytes = vec![0u64; streams];
    for a in files {
        let k = (0..streams).min_by_key(|&k| bytes[k])?;
        if bytes[k] >= want {
            break;
        }
        out[k].push(a);
        bytes[k] += a.len;
    }
    if bytes.iter().any(|&b| b < 2 * read as u64) {
        return None;
    }
    Some(out)
}

#[derive(Default)]
struct Result1 {
    reads: Times,
    opens: Times,
    bytes: u64,
    ends: u64,
    wraps: u64,
    behind_ms: f64,
    longest_behind_ms: f64,
}

/// One stream: read its pieces in turn at `cfg.rate` bytes a second until
/// `secs` have passed, starting `offset` s after `start`.
fn stream(
    pieces: &[&Piece],
    cfg: Config,
    dev: u64,
    secs: f64,
    start: Instant,
    offset: f64,
) -> Result<Result1, String> {
    let mut buf = vec![0u8; cfg.read];
    let mut r = Result1::default();
    let mut behind_since: Option<f64> = None;
    let (mut i, mut pass_bytes) = (0usize, 0u64);
    // The open piece and the bytes left in it.
    let mut cur: Option<(File, u64)> = None;
    loop {
        stopped()?;
        let due = offset + r.bytes as f64 / cfg.rate;
        if due >= secs {
            break;
        }
        let now = start.elapsed().as_secs_f64();
        if now < due {
            if let Some(s) = behind_since.take() {
                r.longest_behind_ms = r.longest_behind_ms.max((now - s) * 1e3);
            }
            std::thread::sleep(Duration::from_secs_f64(due - now));
        } else {
            r.behind_ms = r.behind_ms.max((now - due) * 1e3);
            behind_since.get_or_insert(due);
        }
        if cur.is_none() {
            let f = r.opens.time(|| open_piece(pieces[i], dev))?;
            cur = Some((f, pieces[i].len));
        }
        let (f, left) = cur.as_mut().expect("opened above");
        let want = (*left).min(cfg.read as u64) as usize;
        let n = r.reads.time(|| fill(f, &mut buf[..want]))?;
        *left -= n as u64;
        r.bytes += n as u64;
        pass_bytes += n as u64;
        if n < cfg.read {
            // The end of this piece (or of its file): drop its pages, so a
            // later pass reads the stick again, and go on with the next.
            r.ends += 1;
            if let Some((f, _)) = cur.take() {
                drop_range(&f, pieces[i].start, pieces[i].len);
            }
            i += 1;
            if i == pieces.len() {
                if pass_bytes == 0 {
                    return Err("a stream's files are empty now".into());
                }
                i = 0;
                pass_bytes = 0;
                r.wraps += 1;
            }
        }
    }
    if let Some(s) = behind_since {
        r.longest_behind_ms = r
            .longest_behind_ms
            .max((start.elapsed().as_secs_f64() - s) * 1e3);
    }
    Ok(r)
}

/// Run one configuration: every stream on its own thread.
fn run_config(cfg: Config, groups: &[Vec<&Piece>], dev: u64, secs: f64) -> Result<Result1, String> {
    let all: Vec<&Piece> = groups.iter().flatten().copied().collect();
    drop_pieces(&all, dev)?;
    let start = Instant::now();
    let spread = cfg.read as f64 / cfg.rate;
    let secs = secs + LEAD_S;
    let results: Vec<Result<Result1, String>> = std::thread::scope(|s| {
        let handles: Vec<_> = groups
            .iter()
            .enumerate()
            .map(|(k, files)| {
                let offset = LEAD_S + spread * k as f64 / groups.len() as f64;
                s.spawn(move || {
                    let r = stream(files, cfg, dev, secs, start, offset);
                    if r.is_err() {
                        STOP.store(true, Ordering::Relaxed);
                    }
                    r
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err("a stream's thread failed".into()))
            })
            .collect()
    });
    let mut all = Result1::default();
    for r in results {
        let r = r?;
        all.reads.merge(r.reads);
        all.opens.merge(r.opens);
        all.bytes += r.bytes;
        all.ends += r.ends;
        all.wraps += r.wraps;
        all.behind_ms = all.behind_ms.max(r.behind_ms);
        all.longest_behind_ms = all.longest_behind_ms.max(r.longest_behind_ms);
    }
    Ok(all)
}

/// "1 file", "2 files".
fn count(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

fn window_s(shortfall_ms: f64) -> f64 {
    (2.0 * shortfall_ms / 1e3 + 1.0).max(WINDOW_MIN_S)
}

fn sequential(found: &Found, dev: u64, z: &Plan) -> Result<(), String> {
    let mut pieces = Vec::new();
    let mut total = 0;
    for a in &found.pieces {
        if total >= z.seq_max {
            break;
        }
        pieces.push(a);
        total += a.len;
    }
    say!(
        "\n1. Sequential read of the largest pieces (up to {} MiB or {:.0} s), 512 KiB reads, cache dropped first",
        z.seq_max / MIB,
        z.seq_secs
    );
    drop_pieces(&pieces, dev)?;
    let mut buf = vec![0u8; 512 * KIB];
    let mut r = Times::default();
    let (mut bytes, t) = (0u64, Instant::now());
    'pieces: for a in &pieces {
        let mut f = open_piece(a, dev).map_err(|e| format!("open a file on partition 4: {e}"))?;
        let mut left = a.len;
        while left > 0 {
            stopped()?;
            if bytes >= z.seq_max || t.elapsed().as_secs_f64() >= z.seq_secs {
                drop_range(&f, a.start, a.len);
                break 'pieces;
            }
            let want = left.min(buf.len() as u64) as usize;
            let n = r.time(|| fill(&mut f, &mut buf[..want]))?;
            bytes += n as u64;
            left -= n as u64;
            if n < want {
                break;
            }
        }
        drop_range(&f, a.start, a.len);
    }
    say!(
        "  {:.1} MB/s ({:.1} MiB in {:.1} s)",
        mbps(bytes, t.elapsed()),
        bytes as f64 / MIB as f64,
        t.elapsed().as_secs_f64()
    );
    say!("{}", r.report_tails("read"));
    Ok(())
}

/// The read-only run on `base` (the data folder).
pub fn run(base: &Path, quick: bool) -> Result<(), Fail> {
    say!(
        "read-only: reads the audio under {}/{LIBRARY} and the files under {}/{BACKUP}, and writes nothing",
        base.display(),
        base.display()
    );
    let z = plan(quick);
    let (found, before) = find(base, z.piece)?;
    let dev = fs::symlink_metadata(base)
        .map_err(|e| Fail::Refused(format!("{}: {e}", base.display())))?
        .dev();
    let mib = |b: u64| b as f64 / MIB as f64;
    say!(
        "found: {} in {LIBRARY}/ ({:.1} MiB), {} in {BACKUP}/ ({:.1} MiB); the largest {:.1} MiB; {} pieces to read",
        count(found.files[0], "audio file"),
        mib(found.bytes[0]),
        count(found.files[1], "file"),
        mib(found.bytes[1]),
        mib(found.largest),
        found.pieces.len()
    );
    say!(
        "  left out: {} under 64 KiB, {} links, {} on another file system{}",
        found.small,
        found.links,
        found.other_fs,
        if found.over_limit {
            format!(", every file past the first {MAX_FILES}")
        } else {
            String::new()
        }
    );
    let first = z.configs[0];
    if assign(&found.pieces, first.streams, u64::MAX, first.read).is_none() {
        return Err(Fail::Refused(format!(
            "too little to read in {LIBRARY}/ and {BACKUP}/: it needs at least {} files of 64 KiB or more",
            first.streams
        )));
    }
    let t = Instant::now();
    sequential(&found, dev, &z)?;

    say!(
        "\n2. Streams, {:.1} s each: each stream reads its own pieces at its pace, cache dropped first",
        z.secs
    );
    say!("  (p99.9 and p99.99 mean something only past 1,000 and 10,000 reads: each line gives its count)");
    let mut summary = Vec::new();
    for cfg in &z.configs {
        let label = format!(
            "{} streams, {} ({:.1} kB/s each), {} KiB reads",
            cfg.streams,
            cfg.kind,
            cfg.rate / 1e3,
            cfg.read / KIB
        );
        let want = (cfg.rate * z.secs) as u64 + cfg.read as u64;
        let Some(groups) = assign(&found.pieces, cfg.streams, want, cfg.read) else {
            say!("\n  {label}: skipped, too little to give each stream two reads of its own");
            continue;
        };
        say!("\n  {label}");
        let r = run_config(*cfg, &groups, dev, z.secs)?;
        say!("  {}", r.reads.report_tails("read"));
        say!(
            "    worst shortfall {:.1} ms, longest behind {:.1} ms; {:.1} MiB read, {} file ends, {} passes started again",
            r.behind_ms,
            r.longest_behind_ms,
            r.bytes as f64 / MIB as f64,
            r.ends,
            r.wraps
        );
        say!("  {}", r.opens.report("open"));
        let w = window_s(r.behind_ms);
        let ram = cfg.streams as f64 * cfg.rate * w;
        say!(
            "    the plan's window (2 x shortfall + 1 s, at least 4 s): {w:.1} s a stream, {:.1} MiB for {} streams",
            ram / MIB as f64,
            cfg.streams
        );
        summary.push((
            *cfg,
            r.reads.quantile(0.99),
            r.reads.quantile(0.999),
            r.reads.max(),
            r.behind_ms,
            w,
            ram,
        ));
    }

    say!("\nSummary (times in ms, window in s a stream, RAM in MiB for all streams):");
    say!("  streams  kind     read KiB   p99     p99.9   max      shortfall  window  RAM");
    for (cfg, p99, p999, max, behind, w, ram) in &summary {
        say!(
            "  {:>7}  {:<7}  {:>8}  {:>6.1}  {:>7.1}  {:>7.1}  {:>9.1}  {:>6.1}  {:>5.1}",
            cfg.streams,
            cfg.kind,
            cfg.read / KIB,
            p99,
            p999,
            max,
            behind,
            w,
            ram / MIB as f64
        );
    }
    let (_, after) = find(base, z.piece)?;
    if after == before {
        say!("\nthe files read are as they were: nothing was written");
    } else {
        say!("\nfiles in {LIBRARY}/ or {BACKUP}/ changed during the run (something else saved there); this tool wrote nothing");
    }
    say!("took {:.0} s", t.elapsed().as_secs_f64());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(lens: &[u64]) -> Vec<Piece> {
        lens.iter()
            .enumerate()
            .map(|(i, &len)| Piece {
                path: PathBuf::from(format!("{i}.wav")),
                start: 0,
                len,
            })
            .collect()
    }

    #[test]
    fn streams_get_their_own_files_largest_first() {
        let files = audio(&[900, 800, 500, 400, 300, 100]);
        let g = assign(&files, 2, 1000, 100).unwrap();
        let lens: Vec<Vec<u64>> = g
            .iter()
            .map(|s| s.iter().map(|a| a.len).collect())
            .collect();
        assert_eq!(lens, vec![vec![900, 400], vec![800, 500]]);
        // Fewer files than streams, or too little for two reads: none.
        assert!(assign(&files, 7, 1000, 100).is_none());
        assert!(assign(&files, 2, 1000, 700).is_none());
        // With no target, every file is given out.
        let g = assign(&files, 2, u64::MAX, 100).unwrap();
        assert_eq!(g.iter().map(|s| s.len()).sum::<usize>(), 6);
    }

    #[test]
    fn the_window_rule() {
        assert_eq!(window_s(0.0), 4.0);
        assert_eq!(window_s(1000.0), 4.0);
        assert!((window_s(2500.0) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn audio_by_extension_any_case() {
        for p in ["a.wav", "b.WAV", "c.aif", "d.Aiff", "e.flac"] {
            assert!(is_audio(Path::new(p)), "{p}");
        }
        for p in ["a.peaks", "b.txt", "wav", "c.wav.part"] {
            assert!(!is_audio(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn the_full_plan_matches_the_design() {
        let z = plan(false);
        assert_eq!(z.configs.len(), 18);
        assert_eq!(z.secs, 20.0);
        let c = z.configs[15];
        assert_eq!((c.streams, c.read, c.kind), (24, 512 * KIB, "24-bit"));
        assert_eq!(c.rate, 264_600.0);
        let f = z.configs[17];
        assert_eq!(
            (f.streams, f.read, f.kind, f.rate),
            (16, 128 * KIB, "float", 352_800.0)
        );
    }
}
