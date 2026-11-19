//! Measures the Wind engine's bores against the tables `wind.rs` keeps:
//! each bore's level (`NORM_DB`) and how sharp it plays (`SHARP_CENTS`) at
//! breaths 0 (read at 0.01), 1/8, ... 1 and Tone 0, 50 and 100, for the
//! clarinet, the sax and the flute at Edge 0, 50 and 100, averaged over C3, C4 and C5, with the spread between
//! those notes (small when the error is the same in cents on every key, as
//! key-tracked losses make it).
//!
//! `cargo run --release -p mpfree-synth --example wind_calibrate [wide]`
//! (`wide` adds each bore across six octaves at breath 0.75), or
//! `... --example wind_calibrate scan MODEL [id=value ...]` for the
//! harmonics of C4 at five breaths with any parameters set.
//!
//! It prints the error left at each point (the level against `TARGET_DB`,
//! the pitch against the note), the worst of them, and the tables with
//! those errors taken out, to paste over the ones in `wind.rs`. Run it
//! again after pasting: the worst error should then be a few hundredths.
//!
//! Each case: breath from a breath controller held at `b` from the start,
//! Breath 100, the level law flat (Velocity 0 dB), no vibrato, noise,
//! growl or tonguing, Sustain 100, Attack 1 ms; one second to settle, then
//! the level (RMS of the mid over half a second) and the pitch (the
//! fundamental's peak, refined by the phase it turns through over one
//! second) from 1.0 s on.

use std::f64::consts::{PI, TAU};

use mpfree_node::{Ctl, Ctx, Node, NodeEvent, Timed};
use mpfree_synth::wind_params as wp;
use mpfree_synth::{wind, Wind};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;

fn hz(note: f64) -> f64 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

/// Renders one held note at breath `b` (2.6 s, mono mid).
fn render(model: f32, note: u8, b: f32, extra: &[(u16, f32)]) -> Vec<f64> {
    let mut w = Wind::new(RATE);
    for (id, v) in [
        (wp::MODEL, model),
        (wp::BREATH_FROM, wp::FROM_BREATH),
        (wp::BREATH, 100.0),
        (wp::VELOCITY, 0.0),
        (wp::VIB_DEPTH, 0.0),
        (wp::NOISE, 0.0),
        (wp::GROWL, 0.0),
        (wp::TONGUE, 0.0),
        (wp::SUSTAIN, 100.0),
        (wp::ATTACK, 1.0),
    ] {
        w.set_param(id, v);
    }
    for &(id, v) in extra {
        w.set_param(id, v);
    }
    let len = (2.6 * f64::from(RATE)) as usize;
    let mut out = Vec::with_capacity(len);
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut frame = 0u64;
    while out.len() < len {
        l.fill(0.0);
        r.fill(0.0);
        let ev: Vec<Timed> = if frame == 0 {
            vec![
                Timed::new(
                    0,
                    NodeEvent::Control {
                        ctl: Ctl::Breath,
                        value: b,
                    },
                ),
                Timed::new(
                    0,
                    NodeEvent::NoteOn {
                        note,
                        velocity: 100,
                    },
                ),
            ]
        } else {
            Vec::new()
        };
        w.process(&Ctx::new(RATE, frame), &ev, &mut l, &mut r);
        for i in 0..BLOCK {
            out.push(0.5 * f64::from(l[i] + r[i]));
        }
        frame += BLOCK as u64;
    }
    out.truncate(len);
    out
}

fn blackman_harris(n: usize) -> Vec<f64> {
    let m = (n - 1) as f64;
    (0..n)
        .map(|i| {
            let t = TAU * i as f64 / m;
            0.35875 - 0.48829 * t.cos() + 0.14128 * (2.0 * t).cos() - 0.01168 * (3.0 * t).cos()
        })
        .collect()
}

fn dft_at(x: &[f64], w: &[f64], f: f64) -> (f64, f64) {
    let dw = TAU * f / f64::from(RATE);
    let (mut re, mut im) = (0.0, 0.0);
    for (i, (a, b)) in x.iter().zip(w).enumerate() {
        let ph = dw * i as f64;
        re += a * b * ph.cos();
        im -= a * b * ph.sin();
    }
    (re, im)
}

fn amp_at(x: &[f64], w: &[f64], f: f64) -> f64 {
    let (re, im) = dft_at(x, w, f);
    2.0 * re.hypot(im) / w.iter().sum::<f64>()
}

/// The fundamental near `near` Hz from frame `from`: the peak within
/// ±80 cents (a 2-cent grid, then a parabola), refined by the phase.
fn freq_near(x: &[f64], near: f64, from: usize) -> Option<f64> {
    const W: usize = 8_192;
    let hop = RATE as usize;
    if x.len() < from + hop + W {
        return None;
    }
    let win = blackman_harris(W);
    let seg = &x[from..from + W];
    let grid: Vec<(f64, f64)> = (-40..=40)
        .map(|c| {
            let f = near * (f64::from(c) * 2.0 / 1200.0).exp2();
            (f, amp_at(seg, &win, f))
        })
        .collect();
    let k = (1..grid.len() - 1).max_by(|&a, &b| grid[a].1.total_cmp(&grid[b].1))?;
    if grid[k].1 < 1e-6 {
        return None;
    }
    let (a, b, c) = (grid[k - 1].1.ln(), grid[k].1.ln(), grid[k + 1].1.ln());
    let d = a - 2.0 * b + c;
    let off = if d.abs() > 1e-12 {
        (0.5 * (a - c) / d).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let coarse = grid[k].0 * (off * 2.0 / 1200.0).exp2();
    let (r1, i1) = dft_at(seg, &win, coarse);
    let (r2, i2) = dft_at(&x[from + hop..from + hop + W], &win, coarse);
    let turned = i2.atan2(r2) - i1.atan2(r1);
    let expect = TAU * coarse * hop as f64 / f64::from(RATE);
    let mut e = (turned - expect) % TAU;
    if e > PI {
        e -= TAU;
    } else if e < -PI {
        e += TAU;
    }
    Some(coarse + e * f64::from(RATE) / (TAU * hop as f64))
}

struct Reading {
    db: f64,
    cents: Option<f64>,
    /// The second and third harmonics against the first, dB.
    h2: f64,
    h3: f64,
}

fn measure(x: &[f64], note: u8) -> Reading {
    let from = RATE as usize;
    let seg = &x[from..from + 24_000];
    let rms = (seg.iter().map(|v| v * v).sum::<f64>() / seg.len() as f64).sqrt();
    let db = 20.0 * rms.max(1e-9).log10();
    let f0 = hz(f64::from(note));
    let f = freq_near(x, f0, from);
    let cents = f.map(|f| 1200.0 * (f / f0).log2());
    let ff = f.unwrap_or(f0);
    let win = blackman_harris(8_192);
    let s = &x[from..from + 8_192];
    let a1 = amp_at(s, &win, ff).max(1e-9);
    let h2 = 20.0 * (amp_at(s, &win, 2.0 * ff).max(1e-9) / a1).log10();
    let h3 = 20.0 * (amp_at(s, &win, 3.0 * ff).max(1e-9) / a1).log10();
    Reading { db, cents, h2, h3 }
}

/// `scan MODEL [id=value ...]`: the harmonics at four breaths on C4.
fn scan(args: &[String]) {
    let m: f32 = args[0].parse().unwrap();
    let extra: Vec<(u16, f32)> = args[1..]
        .iter()
        .map(|a| {
            let (k, v) = a.split_once('=').unwrap();
            (k.parse().unwrap(), v.parse().unwrap())
        })
        .collect();
    let note = 60u8;
    for b in [0.01f32, 0.1, 0.25, 0.5, 0.75, 1.0] {
        let x = render(m, note, b, &extra);
        let r = measure(&x, note);
        let f0 = hz(f64::from(note)) * (r.cents.unwrap_or(0.0) / 1200.0).exp2();
        let win = blackman_harris(8_192);
        let s = &x[RATE as usize..RATE as usize + 8_192];
        let a1 = amp_at(s, &win, f0).max(1e-9);
        let hs: Vec<String> = (2..=8)
            .map(|h| {
                format!(
                    "{:6.1}",
                    20.0 * (amp_at(s, &win, f64::from(h) * f0).max(1e-9) / a1).log10()
                )
            })
            .collect();
        println!(
            "b {b:.2}: {:7.2} dB {:+6.2} c  h2..h8 {}",
            r.db,
            r.cents.unwrap_or(f64::NAN),
            hs.join(" ")
        );
    }
}

/// `speak MODEL [id=value ...]`: how soon C4 at velocity 127 comes within
/// 3 dB of where it settles (5 ms windows, 1 ms apart), from velocity.
fn speak(args: &[String]) {
    let m: f32 = args[0].parse().unwrap();
    let mut extra: Vec<(u16, f32)> =
        vec![(wp::BREATH_FROM, wp::FROM_VELOCITY), (wp::SUSTAIN, 90.0)];
    for a in &args[1..] {
        let (k, v) = a.split_once('=').unwrap();
        extra.push((k.parse().unwrap(), v.parse().unwrap()));
    }
    let mut w = Wind::new(RATE);
    w.set_param(wp::MODEL, m);
    for (id, v) in extra {
        w.set_param(id, v);
    }
    let len = RATE as usize * 3 / 2;
    let (mut l, mut r) = (vec![0.0f32; len], vec![0.0f32; len]);
    let ev = [Timed::new(
        0,
        NodeEvent::NoteOn {
            note: 60,
            velocity: 127,
        },
    )];
    for (b, (lc, rc)) in l.chunks_mut(BLOCK).zip(r.chunks_mut(BLOCK)).enumerate() {
        let e: &[Timed] = if b == 0 { &ev } else { &[] };
        w.process(&Ctx::new(RATE, (b * BLOCK) as u64), e, lc, rc);
    }
    let x: Vec<f64> = l.iter().map(|&v| f64::from(v)).collect();
    let p = |a: usize, b: usize| x[a..b].iter().map(|v| v * v).sum::<f64>() / (b - a) as f64;
    let settled = p(38_400, 57_600);
    let ms = (0..1000)
        .find(|&i| p(i * 48, i * 48 + 240) >= settled * 0.5)
        .unwrap_or(1000);
    println!("speaks in {ms} ms");
    if std::env::var_os("WIND_ENV").is_some() {
        for i in 0..20 {
            let a = i * 240;
            println!(
                "  {:3} ms {:7.2} dB",
                i * 5,
                10.0 * (p(a, a + 240) / settled).max(1e-12).log10()
            );
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("scan") {
        scan(&args[1..]);
        return;
    }
    if args.first().map(String::as_str) == Some("speak") {
        speak(&args[1..]);
        return;
    }
    let wide = args.iter().any(|a| a == "wide");
    let rows: [(&str, f32, f32); wind::ROWS] = [
        ("Clarinet", wp::CLARINET, 60.0),
        ("Sax", wp::SAX, 60.0),
        ("Flute, Edge 0", wp::FLUTE, 0.0),
        ("Flute, Edge 50", wp::FLUTE, 50.0),
        ("Flute, Edge 100", wp::FLUTE, 100.0),
    ];
    let tones = [0.0f32, 50.0, 100.0];
    let notes = [48u8, 60, 72];
    let target = f64::from(wind::TARGET_DB);
    let mut norm = wind::NORM_DB;
    let mut sharp = wind::SHARP_CENTS;
    let (mut worst_db, mut worst_c) = (0.0f64, 0.0f64);
    for (r, (name, m, edge)) in rows.into_iter().enumerate() {
        for (t, tone) in tones.into_iter().enumerate() {
            println!("== {name}, Tone {tone}");
            let extra = [(wp::EDGE, edge), (wp::TONE, tone)];
            for i in 0..=8 {
                let b = (i as f32 / 8.0).max(0.01);
                let rs: Vec<Reading> = notes
                    .iter()
                    .map(|&n| measure(&render(m, n, b, &extra), n))
                    .collect();
                let db = rs.iter().map(|r| r.db).sum::<f64>() / rs.len() as f64 - target;
                let cs: Vec<f64> = rs.iter().filter_map(|r| r.cents).collect();
                let c = if cs.is_empty() {
                    0.0
                } else {
                    cs.iter().sum::<f64>() / cs.len() as f64
                };
                let spread = |v: &[f64]| {
                    v.iter().copied().fold(f64::MIN, f64::max)
                        - v.iter().copied().fold(f64::MAX, f64::min)
                };
                let dbs: Vec<f64> = rs.iter().map(|r| r.db).collect();
                println!(
                    "b {b:.3}: {db:+7.2} dB (spread {:5.2})  {c:+6.2} c (spread {:5.2}, {} of {} read)  h2 {:6.1} h3 {:6.1} dB",
                    spread(&dbs),
                    spread(&cs),
                    cs.len(),
                    rs.len(),
                    rs[1].h2,
                    rs[1].h3
                );
                worst_db = worst_db.max(db.abs());
                worst_c = worst_c.max(c.abs());
                norm[r][t][i] += db as f32;
                sharp[r][t][i] += c as f32;
            }
        }
        if wide {
            println!("-- {name} across the keys at breath 0.75");
            for n in [24u8, 36, 48, 60, 72, 84, 96] {
                let r = measure(&render(m, n, 0.75, &[(wp::EDGE, edge)]), n);
                println!(
                    "  note {n}: {:+7.2} dB {:+6.2} c",
                    r.db - target,
                    r.cents.unwrap_or(f64::NAN)
                );
            }
        }
    }
    let table = |t: &wind::Table| {
        let mut s = String::from("[\n");
        for row in t {
            s += "    [\n";
            for tone in row {
                s += "        [";
                s += &tone
                    .iter()
                    .map(|v| format!("{v:.2}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                s += "],\n";
            }
            s += "    ],\n";
        }
        s + "]"
    };
    println!("worst error: {worst_db:.2} dB, {worst_c:.2} cents");
    println!("pub const NORM_DB: Table = {};", table(&norm));
    println!("pub const SHARP_CENTS: Table = {};", table(&sharp));
}
