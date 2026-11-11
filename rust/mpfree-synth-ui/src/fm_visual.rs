//! FM's drawn visuals, worked out from its parameter values: the algorithm
//! as a diagram of its six operators, each operator's envelope, keyboard
//! scaling, the pitch envelope and the LFO.
//!
//! Like [`crate::visual`], every point is in a unit box: x from 0 to 1 left
//! to right, y from 0 (bottom) to 1 (top).

use crate::visual::{Values, POINTS};
use mpfree_synth::fm::{env_stages, lfo_hz, peg_octaves, peg_rate, scale_keyboard, Route};
use mpfree_synth::fm_params::{self as fp, op, op_id};
use std::f32::consts::PI;

// ---------------------------------------------------------------- algorithm

/// The drawing's vertical budget in the unit box (a plot is about 100 px
/// tall and a box 15 px): the output line under the carriers, room above
/// the top row for a feedback loop, a margin, and the most a row step gets
/// (algorithms with few rows spread out; four rows still fit).
const BUS_DROP: f32 = 0.15;
const TOP_ROOM: f32 = 0.135;
const MARGIN: f32 = 0.02;
const MAX_ROW: f32 = 0.42;
/// The widest the columns get, so a two-column algorithm stays compact.
const MAX_COL: f32 = 0.2;

/// Where an algorithm's operators go, worked out from its wiring: carriers
/// along the bottom, each modulator a row above what it modulates, a
/// modulator of several operators centred over them.
#[derive(Clone, Debug, PartialEq)]
pub struct AlgoLayout {
    /// Operators 1 to 6 (index 0 to 5): box centres.
    pub ops: [(f32, f32); 6],
    /// A bit per carrier.
    pub carriers: u8,
    /// The rows used (1 to 4).
    pub rows: usize,
    /// The output line's height.
    pub bus_y: f32,
    pub route: Route,
}

/// Algorithm `n` (1 to 32) laid out.
pub fn algorithm_layout(n: usize) -> AlgoLayout {
    let route = Route::new(n);
    let t = route.targets;
    // Row: the longest chain down to a carrier. A modulator always has a
    // higher number than what it modulates, so one pass upwards does it.
    let mut row = [0usize; 6];
    for j in 0..6 {
        row[j] = (0..j)
            .filter(|&k| t[j] & (1 << k) != 0)
            .map(|k| row[k] + 1)
            .max()
            .unwrap_or(0);
    }
    // Each modulator hangs off the lowest-numbered operator it modulates.
    let parent = |j: usize| (t[j] != 0).then(|| t[j].trailing_zeros() as usize);
    let children = |k: usize| (0..6).filter(move |&j| parent(j) == Some(k));
    let mut width = [1.0f32; 6];
    for k in (0..6).rev() {
        let w: f32 = children(k).map(|j| width[j]).sum();
        width[k] = w.max(1.0);
    }
    // Columns: carriers left to right, each subtree as wide as its leaves,
    // a parent centred over its span.
    let mut col = [0.0f32; 6];
    fn place(
        k: usize,
        left: f32,
        width: &[f32; 6],
        col: &mut [f32; 6],
        kids: &dyn Fn(usize) -> Vec<usize>,
    ) {
        col[k] = left + width[k] / 2.0;
        let mut x = left;
        for j in kids(k) {
            place(j, x, width, col, kids);
            x += width[j];
        }
    }
    let kids = |k: usize| children(k).collect::<Vec<_>>();
    let mut left = 0.0;
    for k in (0..6).filter(|&k| route.carriers & (1 << k) != 0) {
        place(k, left, &width, &mut col, &kids);
        left += width[k];
    }
    let cols = left;
    // A modulator of several operators sits over their middle.
    for j in 0..6 {
        if t[j].count_ones() > 1 {
            let xs: Vec<f32> = (0..6)
                .filter(|&k| t[j] & (1 << k) != 0)
                .map(|k| col[k])
                .collect();
            col[j] = xs.iter().sum::<f32>() / xs.len() as f32;
        }
    }
    let rows = row.iter().max().map_or(1, |r| r + 1);
    let step = (1.0 / cols).min(MAX_COL);
    let free = 1.0 - 2.0 * MARGIN - BUS_DROP - TOP_ROOM;
    let row_step = if rows > 1 {
        (free / (rows - 1) as f32).min(MAX_ROW)
    } else {
        0.0
    };
    // Centre the drawing: the output line at the bottom, room for a loop on top.
    let height = BUS_DROP + (rows - 1) as f32 * row_step + TOP_ROOM;
    let bus_y = (1.0 - height) / 2.0;
    let ops = std::array::from_fn(|j| {
        (
            0.5 + (col[j] - cols / 2.0) * step,
            bus_y + BUS_DROP + row[j] as f32 * row_step,
        )
    });
    AlgoLayout {
        ops,
        carriers: route.carriers,
        rows,
        bus_y,
        route,
    }
}

/// The lines between the boxes: each modulator down to what it modulates
/// (down, across, down, like the DX7's panel), each carrier down to the
/// output line, and the output line itself.
pub fn algorithm_lines(l: &AlgoLayout) -> Vec<Vec<(f32, f32)>> {
    let mut lines = Vec::new();
    for j in 0..6 {
        let (xm, ym) = l.ops[j];
        for k in (0..6).filter(|&k| l.route.targets[j] & (1 << k) != 0) {
            let (xt, yt) = l.ops[k];
            let mid = (ym + yt) / 2.0;
            lines.push(vec![(xm, ym), (xm, mid), (xt, mid), (xt, yt)]);
        }
    }
    let carriers: Vec<(f32, f32)> = (0..6)
        .filter(|&k| l.carriers & (1 << k) != 0)
        .map(|k| l.ops[k])
        .collect();
    for &(x, y) in &carriers {
        lines.push(vec![(x, y), (x, l.bus_y)]);
    }
    let lo = carriers.iter().map(|c| c.0).fold(1.0, f32::min);
    let hi = carriers.iter().map(|c| c.0).fold(0.0, f32::max);
    // The output line runs a little past the last carrier, to the right.
    lines.push(vec![(lo, l.bus_y), (hi + 0.04, l.bus_y)]);
    lines
}

/// Whether operator `n` (1 to 6) sounds: switched on with some output.
pub fn op_sounds(v: &impl Values, n: usize) -> bool {
    v.get(op_id(n, op::ON)) >= 0.5 && v.get(op_id(n, op::LEVEL)) > 0.0
}

/// What operator `n` (1 to 6) does in algorithm `l`, for a caption:
/// "carrier", or "modulates 1" ("modulates 4, 5").
pub fn op_role(l: &AlgoLayout, n: usize) -> String {
    let j = n - 1;
    if l.carriers & (1 << j) != 0 {
        return "carrier".to_string();
    }
    let into: Vec<String> = (0..6)
        .filter(|&k| l.route.targets[j] & (1 << k) != 0)
        .map(|k| (k + 1).to_string())
        .collect();
    format!("modulates {}", into.join(", "))
}

/// The Algorithm card's caption: "22 · feedback 7" ("feedback 4 → 6" for a
/// loop through two operators).
pub fn algorithm_caption(v: &impl Values) -> String {
    let n = v.get(fp::ALGORITHM).round().clamp(1.0, 32.0) as usize;
    let fb = v.get(fp::FEEDBACK).round() as i32;
    let r = Route::new(n);
    if fb <= 0 {
        return format!("{n} · no feedback");
    }
    if r.fb_from == r.fb_to {
        format!("{n} · feedback {fb} on {}", r.fb_from + 1)
    } else {
        format!("{n} · feedback {fb}, {} → {}", r.fb_from + 1, r.fb_to + 1)
    }
}

// ---------------------------------------------------------------- envelopes

/// Share of the width the held part of an envelope takes.
const HOLD: f32 = 0.2;

/// Lays out stages of (seconds, level) points: each stage as wide as the
/// square root of its time (so short and long both read), and the hold
/// after stage `hold_after`.
fn stages_to_curve(stages: &[Vec<(f32, f32)>], hold_after: usize) -> Vec<(f32, f32)> {
    let secs: Vec<f32> = stages
        .iter()
        .map(|s| s.last().map_or(0.0, |p| p.0))
        .collect();
    let total: f32 = secs.iter().map(|d| d.sqrt()).sum();
    let even = total < 1e-6;
    let scale = if even { 0.0 } else { (1.0 - HOLD) / total };
    let mut pts = Vec::with_capacity(POINTS);
    let mut x = 0.0;
    for (k, st) in stages.iter().enumerate() {
        let w = if even {
            (1.0 - HOLD) / stages.len() as f32
        } else {
            secs[k].sqrt() * scale
        };
        for &(t, y) in st {
            let f = if secs[k] > 0.0 { t / secs[k] } else { 0.0 };
            pts.push(((x + w * f).min(1.0), y.clamp(0.0, 1.0)));
        }
        x += w;
        if k == hold_after {
            let y = pts.last().map_or(0.0, |p| p.1);
            x += HOLD;
            pts.push((x.min(1.0), y));
        }
    }
    pts
}

fn bytes4(v: &impl Values, ids: [u16; 4]) -> [u8; 4] {
    ids.map(|id| v.get(id).round().clamp(0.0, 99.0) as u8)
}

/// Operator `n`'s envelope (1 to 6): L4 up through L1, L2 and L3, held,
/// then the release to L4.
pub fn op_env(v: &impl Values, n: usize) -> Vec<(f32, f32)> {
    let rates = bytes4(
        v,
        [
            op_id(n, op::R1),
            op_id(n, op::R2),
            op_id(n, op::R3),
            op_id(n, op::R4),
        ],
    );
    let levels = bytes4(
        v,
        [
            op_id(n, op::L1),
            op_id(n, op::L2),
            op_id(n, op::L3),
            op_id(n, op::L4),
        ],
    );
    stages_to_curve(&env_stages(&rates, &levels), 2)
}

/// The pitch envelope's levels in semitones (L1 to L4).
fn peg_semis(v: &impl Values) -> [f32; 4] {
    [fp::PEG_L1, fp::PEG_L2, fp::PEG_L3, fp::PEG_L4]
        .map(|id| peg_octaves(v.get(id).round().clamp(0.0, 99.0) as u8) * 12.0)
}

/// The pitch envelope card's range, in semitones each way: the smallest of
/// 2, 12, 24 and 48 that holds every level, so a small scoop still shows.
pub fn pitch_env_range(v: &impl Values) -> f32 {
    let most = peg_semis(v).iter().fold(0.0f32, |m, s| m.max(s.abs()));
    [2.0, 12.0, 24.0, 48.0]
        .into_iter()
        .find(|&r| most <= r)
        .unwrap_or(48.0)
}

/// The pitch envelope: from L4 through L1, L2 and L3, held, back to L4. The
/// middle of the card is no change; the top and bottom are
/// [`pitch_env_range`].
pub fn pitch_env(v: &impl Values) -> Vec<(f32, f32)> {
    let rates = bytes4(v, [fp::PEG_R1, fp::PEG_R2, fp::PEG_R3, fp::PEG_R4]);
    let semis = peg_semis(v);
    let range = pitch_env_range(v);
    let y = |s: f32| (0.5 + 0.45 * s / range).clamp(0.0, 1.0);
    let mut from = semis[3];
    let stages: Vec<Vec<(f32, f32)>> = (0..4)
        .map(|k| {
            let to = semis[k];
            let secs = (to - from).abs() / 12.0 / peg_rate(rates[k]).max(1e-6);
            let st = vec![(0.0, y(from)), (secs, y(to))];
            from = to;
            st
        })
        .collect();
    stages_to_curve(&stages, 2)
}

/// The pitch envelope's caption: how far it moves the pitch ("off" when all
/// four levels are 50).
pub fn pitch_env_caption(v: &impl Values) -> String {
    let semis = peg_semis(v);
    let (lo, hi) = semis
        .iter()
        .fold((0.0f32, 0.0f32), |(lo, hi), &s| (lo.min(s), hi.max(s)));
    if hi - lo < 0.05 {
        return "off".to_string();
    }
    let st = |s: f32| {
        if s.abs() >= 10.0 {
            format!("{s:+.0}")
        } else {
            format!("{s:+.1}")
        }
    };
    let what = match (lo < -0.05, hi > 0.05) {
        (true, true) => format!("{} to {} st", st(lo), st(hi)),
        (true, false) => format!("down to {} st", st(lo)),
        _ => format!("up to {} st", st(hi)),
    };
    format!("{what} · scale ±{}", pitch_env_range(v))
}

// ---------------------------------------------------------------- LFO

/// Two cycles of the LFO's wave, after a flat stretch for its delay.
pub fn lfo(v: &impl Values) -> Vec<(f32, f32)> {
    const HELD: [f32; 8] = [0.4, -0.7, 0.9, -0.2, 0.6, -0.9, 0.1, -0.5];
    let wave = v.get(fp::LFO_WAVE).round() as i32;
    let delay = (v.get(fp::LFO_DELAY) / 99.0).clamp(0.0, 1.0) * 0.5;
    (0..=POINTS)
        .map(|i| {
            let x = i as f32 / POINTS as f32;
            if x < delay {
                return (x, 0.5);
            }
            let u = (x - delay) / (1.0 - delay);
            let t = (u * 2.0).rem_euclid(1.0);
            let y = match wave {
                // The triangle and sine start from the middle, as the DX7's do.
                0 => {
                    if t < 0.25 {
                        4.0 * t
                    } else if t < 0.75 {
                        2.0 - 4.0 * t
                    } else {
                        4.0 * t - 4.0
                    }
                }
                1 => 1.0 - 2.0 * t,
                2 => 2.0 * t - 1.0,
                3 => {
                    if t < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                4 => (2.0 * PI * t).sin(),
                _ => HELD[((u * 8.0) as usize).min(7)],
            };
            (x, 0.5 + 0.42 * y)
        })
        .collect()
}

/// The LFO card's caption: "triangle · 5.2 Hz", with "delayed" when it
/// fades in.
pub fn lfo_caption(v: &impl Values) -> String {
    let wave = fp::LFO_WAVES
        .get(v.get(fp::LFO_WAVE).round() as usize)
        .copied()
        .unwrap_or("?");
    let hz = lfo_hz(v.get(fp::LFO_SPEED).round().clamp(0.0, 99.0) as u8);
    let hz = if hz >= 10.0 {
        format!("{hz:.0}")
    } else if hz >= 1.0 {
        format!("{hz:.1}")
    } else {
        format!("{hz:.2}")
    };
    let delayed = if v.get(fp::LFO_DELAY) > 0.0 {
        " · delayed"
    } else {
        ""
    };
    format!("{} · {hz} Hz{delayed}", wave.to_lowercase())
}

// ---------------------------------------------------------------- keyboard scaling

/// The keys the scaling card spans: A-1 to C7, the DX7's range.
pub const KEY_LO: u8 = 21;
pub const KEY_HI: u8 = 108;

/// The x position of a note on the scaling card.
pub fn key_x(note: u8) -> f32 {
    (f32::from(note.clamp(KEY_LO, KEY_HI) - KEY_LO) / f32::from(KEY_HI - KEY_LO)).clamp(0.0, 1.0)
}

fn kbd_units(v: &impl Values, n: usize) -> impl Iterator<Item = (u8, i32)> {
    let b = |f: u16| v.get(op_id(n, f)).round().clamp(0.0, 99.0) as u8;
    let (brk, ld, rd, lc, rc) = (
        b(op::BREAK),
        b(op::L_DEPTH),
        b(op::R_DEPTH),
        b(op::L_CURVE),
        b(op::R_CURVE),
    );
    (KEY_LO..=KEY_HI).map(move |note| (note, scale_keyboard(note, brk, ld, rd, lc, rc)))
}

/// The scaling card's range for operators `ops`, in dB each way: the
/// smallest of 12, 24, 48 and 96 that holds both curves (a level step is
/// 0.75 dB).
pub fn kbd_range_db(v: &impl Values, ops: &[usize]) -> f32 {
    let most = ops
        .iter()
        .flat_map(|&n| kbd_units(v, n))
        .map(|(_, u)| u.abs())
        .max()
        .unwrap_or(0) as f32
        * 0.75;
    [12.0, 24.0, 48.0, 96.0]
        .into_iter()
        .find(|&r| most <= r)
        .unwrap_or(96.0)
}

/// Operator `n`'s keyboard level scaling across the keys, in dB: the middle
/// of the card is no change, the top and bottom `range_db`.
pub fn kbd_curve(v: &impl Values, n: usize, range_db: f32) -> Vec<(f32, f32)> {
    kbd_units(v, n)
        .map(|(note, u)| {
            (
                key_x(note),
                (0.5 + 0.45 * u as f32 * 0.75 / range_db).clamp(0.0, 1.0),
            )
        })
        .collect()
}

/// A note's name, with C3 as middle C (note 60): "C3", "A-1".
pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        NAMES[usize::from(note % 12)],
        i32::from(note / 12) - 2
    )
}

/// Operator `n`'s break point as a note (break 0 is A-1, 39 is C3).
pub fn break_note(v: &impl Values, n: usize) -> u8 {
    (v.get(op_id(n, op::BREAK)).round().clamp(0.0, 99.0) as u8).saturating_add(KEY_LO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_synth::fm::ALGORITHMS;

    #[test]
    fn every_algorithm_lays_out_cleanly() {
        for n in 1..=32 {
            let l = algorithm_layout(n);
            for (j, &(x, y)) in l.ops.iter().enumerate() {
                assert!(
                    (0.05..=0.95).contains(&x) && (0.1..=0.9).contains(&y),
                    "alg {n} op {}: ({x}, {y})",
                    j + 1
                );
                for (k, &(x2, y2)) in l.ops.iter().enumerate().skip(j + 1) {
                    let apart = (x - x2).abs() > 0.09 || (y - y2).abs() > 0.2;
                    assert!(apart, "alg {n}: ops {} and {} overlap", j + 1, k + 1);
                }
            }
            // Carriers on the bottom row; every modulator above what it
            // modulates.
            let bottom = l.ops.iter().map(|o| o.1).fold(1.0, f32::min);
            for j in 0..6 {
                let carrier = l.carriers & (1 << j) != 0;
                assert_eq!(
                    carrier,
                    (l.ops[j].1 - bottom).abs() < 1e-4,
                    "alg {n} op {}",
                    j + 1
                );
            }
            for &(from, to) in ALGORITHMS[n - 1].edges {
                let (m, t) = (l.ops[usize::from(from) - 1], l.ops[usize::from(to) - 1]);
                assert!(m.1 > t.1 + 0.2, "alg {n}: {from} -> {to}");
            }
            assert!(l.bus_y > 0.0 && l.bus_y < bottom);
            for line in algorithm_lines(&l) {
                assert!(
                    line.iter()
                        .all(|p| (0.0..=1.0).contains(&p.0) && (0.0..=1.0).contains(&p.1)),
                    "alg {n}"
                );
            }
        }
    }

    #[test]
    fn known_layouts() {
        // Algorithm 1: two stacks, 2 on 1 and 6 on 5 on 4 on 3.
        let l = algorithm_layout(1);
        assert_eq!(l.rows, 4);
        assert_eq!(l.carriers, 0b000101);
        assert!((l.ops[0].0 - l.ops[1].0).abs() < 1e-6 && l.ops[0].0 < l.ops[2].0);
        assert!((l.ops[2].0 - l.ops[5].0).abs() < 1e-6);
        // Algorithm 32: six carriers in a row.
        let l = algorithm_layout(32);
        assert_eq!(l.rows, 1);
        assert!(l.ops.windows(2).all(|w| w[1].0 > w[0].0));
        // Algorithm 22: 6 modulates 3, 4 and 5 and sits over 4.
        let l = algorithm_layout(22);
        assert!((l.ops[5].0 - l.ops[3].0).abs() < 1e-6);
        assert_eq!(op_role(&l, 6), "modulates 3, 4, 5");
        assert_eq!(op_role(&l, 1), "carrier");
    }

    fn defaults(id: u16) -> f32 {
        use mpfree_node::Node;
        static FM: std::sync::OnceLock<Vec<(u16, f32)>> = std::sync::OnceLock::new();
        let fm = FM.get_or_init(|| {
            let fm = mpfree_synth::Fm::new(48_000);
            fm.params().iter().map(|p| (p.id, fm.param(p.id))).collect()
        });
        fm.iter().find(|p| p.0 == id).map_or(0.0, |p| p.1)
    }

    #[test]
    fn curves_stay_in_the_box() {
        let edits: [&[(u16, f32)]; 4] = [
            &[],
            &[
                (op_id(1, op::R1), 0.0),
                (op_id(1, op::R4), 0.0),
                (op_id(1, op::L4), 99.0),
            ],
            &[
                (fp::PEG_L1, 99.0),
                (fp::PEG_L4, 0.0),
                (fp::PEG_R2, 0.0),
                (fp::LFO_DELAY, 50.0),
                (fp::LFO_WAVE, 5.0),
            ],
            &[
                (op_id(1, op::L_DEPTH), 99.0),
                (op_id(1, op::R_DEPTH), 99.0),
                (op_id(1, op::R_CURVE), 2.0),
            ],
        ];
        for e in edits {
            let v = |id: u16| {
                e.iter()
                    .find(|p| p.0 == id)
                    .map_or_else(|| defaults(id), |p| p.1)
            };
            let all = [
                op_env(&v, 1),
                pitch_env(&v),
                lfo(&v),
                kbd_curve(&v, 1, kbd_range_db(&v, &[1])),
            ]
            .concat();
            for (x, y) in all {
                assert!(
                    (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
                    "({x}, {y})"
                );
            }
            assert!(
                op_env(&v, 1).windows(2).all(|w| w[1].0 >= w[0].0),
                "time runs forward"
            );
        }
    }

    #[test]
    fn an_organ_envelope_jumps_up_holds_and_drops() {
        // Rates 99, levels 99 99 99 0: full at once, held, gone at once.
        let v = |id: u16| defaults(id);
        let e = op_env(&v, 1);
        assert_eq!(e[0].1, 0.0);
        let top = e.iter().map(|p| p.1).fold(0.0, f32::max);
        assert!((top - 1.0).abs() < 1e-4);
        assert!(e.last().unwrap().1 < 0.01);
        assert!(
            e.iter().filter(|p| p.1 > 0.99).count() >= 2,
            "it holds at the top"
        );
    }

    #[test]
    fn captions() {
        let v = |id: u16| match id {
            fp::ALGORITHM => 4.0,
            fp::FEEDBACK => 6.0,
            _ => defaults(id),
        };
        assert_eq!(algorithm_caption(&v), "4 · feedback 6, 4 → 6");
        assert_eq!(algorithm_caption(&defaults), "1 · no feedback");
        assert_eq!(pitch_env_caption(&defaults), "off");
        let up = |id: u16| if id == fp::PEG_L4 { 99.0 } else { defaults(id) };
        assert!(
            pitch_env_caption(&up).starts_with("up to +"),
            "{}",
            pitch_env_caption(&up)
        );
        // A scoop of about a semitone is drawn on a ±2 semitone scale.
        let scoop = |id: u16| if id == fp::PEG_L1 { 47.0 } else { defaults(id) };
        assert_eq!(pitch_env_range(&scoop), 2.0);
        assert!(
            pitch_env_caption(&scoop).ends_with("scale ±2"),
            "{}",
            pitch_env_caption(&scoop)
        );
        let low = pitch_env(&scoop).iter().map(|p| p.1).fold(1.0, f32::min);
        assert!(low < 0.35, "the scoop shows: {low}");
        assert_eq!(lfo_caption(&defaults), "triangle · 5.2 Hz");
        assert_eq!(note_name(60), "C3");
        assert_eq!(note_name(21), "A-1");
        assert_eq!(
            break_note(
                &|id: u16| if id == op_id(2, op::BREAK) { 39.0 } else { 0.0 },
                2
            ),
            60
        );
        // No scaling: flat on the smallest scale. A -Exp right curve at
        // depth 45 takes about 10 dB off the top of the keyboard.
        assert_eq!(kbd_range_db(&defaults, &[1, 2]), 12.0);
        let tine = |id: u16| match id {
            x if x == op_id(2, op::BREAK) => 39.0,
            x if x == op_id(2, op::R_DEPTH) => 45.0,
            x if x == op_id(2, op::R_CURVE) => 1.0,
            _ => 0.0,
        };
        assert_eq!(kbd_range_db(&tine, &[1, 2]), 12.0);
        let c = kbd_curve(&tine, 2, 12.0);
        assert!(
            (c[0].1 - 0.5).abs() < 1e-6 && c.last().unwrap().1 < 0.2,
            "{:?}",
            c.last()
        );
    }
}
