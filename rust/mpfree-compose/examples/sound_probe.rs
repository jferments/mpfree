//! S0 sound probe: plays the same test notes through every factory preset
//! of the melodic synths and measures how each one sounds: level, attack,
//! brightness (spectral centroid, and how much of the sound sits in the
//! 2 to 5 kHz "presence" band where the ear is most sensitive, and above
//! 6 kHz), and how loudness and brightness follow velocity.
//!
//! cargo run --release -p mpfree-compose --example sound_probe -- OUT.tsv [FILTER]

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::{
    Analog, Bass, Choir, Drawbar, Fm, Keys, Pluck, Poly, Preset, Squelch, Strings, Wavetable, Wind,
};
use std::fmt::Write as _;

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
const ON: f64 = 0.6;
const OFF: f64 = 0.5;
const N: usize = 4096;

struct Shot {
    db: f64,
    peak_db: f64,
    attack_ms: f64,
    centroid: f64,
    presence: f64,
    air: f64,
    low: f64,
}

fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for i in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (ar, ai) = (re[i + k], im[i + k]);
                let (br, bi) = (re[i + k + len / 2], im[i + k + len / 2]);
                let (tr, ti) = (br * cr - bi * ci, br * ci + bi * cr);
                re[i + k] = ar + tr;
                im[i + k] = ai + ti;
                re[i + k + len / 2] = ar - tr;
                im[i + k + len / 2] = ai - ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
}

fn shot(node: &mut dyn Node, note: u8, velocity: u8) -> Shot {
    let frames = ((ON + OFF) * f64::from(RATE)) as usize;
    let frames = frames.div_ceil(BLOCK) * BLOCK;
    let off_at = (ON * f64::from(RATE)) as usize;
    let mut x = vec![0.0f64; frames];
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    for pos in (0..frames).step_by(BLOCK) {
        let mut ev = Vec::new();
        if pos == 0 {
            ev.push(Timed::new(0, NodeEvent::NoteOn { note, velocity }));
        }
        if off_at >= pos && off_at < pos + BLOCK {
            ev.push(Timed::new(
                (off_at - pos) as u32,
                NodeEvent::NoteOff { note },
            ));
        }
        l.fill(0.0);
        r.fill(0.0);
        let ctx = Ctx {
            rate: RATE,
            frame: pos as u64,
            bpm: 120.0,
            beats: pos as f64 / f64::from(RATE) * 2.0,
            playing: true,
            sidechain: None,
        };
        node.process(&ctx, &ev, &mut l, &mut r);
        for k in 0..BLOCK {
            x[pos + k] = 0.5 * f64::from(l[k] + r[k]);
        }
    }
    // Level over the held part; peak; attack to 90 % of the loudest 1 ms.
    let held = &x[..off_at];
    let rms = (held.iter().map(|v| v * v).sum::<f64>() / held.len() as f64).sqrt();
    let peak = x.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let ms = (RATE / 1000) as usize;
    let env: Vec<f64> = held
        .chunks(ms)
        .map(|c| (c.iter().map(|v| v * v).sum::<f64>() / c.len() as f64).sqrt())
        .collect();
    let top = env.iter().cloned().fold(0.0, f64::max);
    let attack = env.iter().position(|&e| e >= 0.9 * top).unwrap_or(0) as f64;
    // Spectrum: Hann frames from 30 ms into the note, averaged.
    let mut power = vec![0.0f64; N / 2];
    let start = (0.03 * f64::from(RATE)) as usize;
    let mut s = start;
    while s + N <= off_at.max(start + N).min(frames) {
        let mut re: Vec<f64> = (0..N)
            .map(|i| {
                let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / N as f64).cos();
                x[s + i] * w
            })
            .collect();
        let mut im = vec![0.0; N];
        fft(&mut re, &mut im);
        for k in 0..N / 2 {
            power[k] += re[k] * re[k] + im[k] * im[k];
        }
        s += N / 2;
    }
    let hz = |k: usize| k as f64 * f64::from(RATE) / N as f64;
    let total: f64 = power[1..].iter().sum::<f64>().max(1e-30);
    let band = |a: f64, b: f64| -> f64 {
        power
            .iter()
            .enumerate()
            .filter(|(k, _)| hz(*k) >= a && hz(*k) < b)
            .map(|(_, p)| p)
            .sum::<f64>()
            / total
    };
    let centroid = power
        .iter()
        .enumerate()
        .skip(1)
        .map(|(k, p)| hz(k) * p)
        .sum::<f64>()
        / total;
    let db = |v: f64| 20.0 * v.max(1e-9).log10();
    Shot {
        db: db(rms),
        peak_db: db(peak),
        attack_ms: attack,
        centroid,
        presence: band(2_000.0, 5_000.0),
        air: band(6_000.0, 24_000.0),
        low: band(0.0, 300.0),
    }
}

macro_rules! family {
    ($out:ident, $filter:ident, $ty:ident, $fam:expr, $notes:expr) => {
        for p in $ty::presets() {
            let p: &Preset = p;
            let name = format!("{}/{}", $fam, p.name);
            if p.name == "Init" || !name.contains($filter.as_str()) {
                continue;
            }
            for &note in $notes {
                for vel in [40u8, 80, 120] {
                    let mut s = $ty::new(RATE);
                    s.load_preset(p);
                    s.prepare(RATE, BLOCK);
                    let m = shot(&mut s, note, vel);
                    let _ = writeln!(
                        $out,
                        "{}\t{}\t{}\t{:.1}\t{:.1}\t{:.0}\t{:.0}\t{:.4}\t{:.4}\t{:.4}",
                        name,
                        note,
                        vel,
                        m.db,
                        m.peak_db,
                        m.attack_ms,
                        m.centroid,
                        m.presence,
                        m.air,
                        m.low
                    );
                }
            }
            eprintln!("{name}");
        }
    };
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out_path = args.get(1).cloned().unwrap_or_else(|| "probe.tsv".into());
    let filter = args.get(2).cloned().unwrap_or_default();
    let mut out =
        String::from("preset\tnote\tvel\tdb\tpeak_db\tattack_ms\tcentroid\tpresence\tair\tlow\n");
    let lead: &[u8] = &[48, 60, 72, 84];
    let low: &[u8] = &[28, 36, 48];
    family!(out, filter, Keys, "Keys", lead);
    family!(out, filter, Pluck, "Pluck", lead);
    family!(out, filter, Wind, "Wind", lead);
    family!(out, filter, Poly, "Poly", lead);
    family!(out, filter, Wavetable, "Wavetable", lead);
    family!(out, filter, Strings, "Strings", lead);
    family!(out, filter, Analog, "Analog", lead);
    family!(out, filter, Fm, "Fm", lead);
    family!(out, filter, Drawbar, "Drawbar", lead);
    family!(out, filter, Choir, "Choir", lead);
    family!(out, filter, Squelch, "Squelch", low);
    family!(out, filter, Bass, "Bass", low);
    std::fs::write(&out_path, out).expect("write");
}
