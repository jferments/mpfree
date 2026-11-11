//! Drums' and Pluck's drawn visuals, worked out from their values with the
//! engines' own formulas ([`mpfree_synth::drum::hit_shape`],
//! [`mpfree_synth::pluck::partials`]), so they follow a knob the moment it
//! turns.
//!
//! - Drums: the kit as the pads lie (pad 1 bottom left), a hit's parts
//!   (body, noise, click) over time, its pitch sweep, and every pad's level.
//! - Pluck: for a string, where it is picked, the harmonics a pluck gives
//!   (on average: each pluck's noise differs) and how fast each dies; for a
//!   mallet, its pulse, the material's modes as struck, and their decays.
//!   Both for middle C (note 60), the middle of the pads.
//!
//! Points are in a unit box, as in [`crate::visual`]: x from 0 to 1 left to
//! right, y from 0 (bottom) to 1 (top).

use crate::visual::{Values, POINTS};
use mpfree_synth::drum::{hit_shape, HitShape};
use mpfree_synth::drum_params::{field, pad_id, MODELS, PADS};
use mpfree_synth::pluck::{mallet_pulse_secs, partials, Partial, HARMONICS};
use mpfree_synth::pluck_params as kp;

/// The rate the shapes are worked out at (they are drawn in seconds).
pub const RATE: u32 = 48_000;
/// The note Pluck's cards are drawn for: middle C.
pub const NOTE: u8 = 60;
/// The Sweep card's time span, in seconds.
pub const SWEEP_SECS: f32 = 0.25;
/// The most the Sweep knob gives, in semitones: the Sweep card's top.
pub const SWEEP_TOP: f32 = 48.0;
/// The harmonics and modes cards' range, in dB below the loudest.
pub const PARTIAL_DB: f32 = 48.0;

/// A drum model's two-letter tag, for its box on the Kit card.
pub fn model_tag(model: &str) -> &'static str {
    match model {
        "Kick" => "BD",
        "Snare" => "SD",
        "Clap" => "CP",
        "Closed hat" => "CH",
        "Open hat" => "OH",
        "Tom" => "TM",
        "Rim" => "RS",
        "Cowbell" => "CB",
        "Cymbal" => "CY",
        "Zap" => "ZP",
        _ => "FM",
    }
}

/// What a model is made of, for its colour: 0 a tuned body (kick, tom,
/// zap, FM perc), 1 noise over a body (snare, clap, rim), 2 metal (the
/// hats, cymbal, cowbell).
pub fn model_family(model: &str) -> usize {
    match model {
        "Snare" | "Clap" | "Rim" => 1,
        "Closed hat" | "Open hat" | "Cymbal" | "Cowbell" => 2,
        _ => 0,
    }
}

/// Pad `pad`'s model (0 to 15).
pub fn pad_model(v: &impl Values, pad: usize) -> &'static str {
    MODELS[v.get(pad_id(pad.min(PADS - 1), field::MODEL)) as usize % MODELS.len()]
}

/// Where pad `pad` (0 to 15) sits on the Kit card: as on the MPC's pads,
/// pad 1 bottom left and pad 16 top right.
pub fn pad_xy(pad: usize) -> (f32, f32) {
    let (col, row) = (pad % 4, pad / 4);
    (0.13 + 0.245 * col as f32, 0.12 + 0.25 * row as f32)
}

/// Pad `pad`'s hit, at full velocity.
pub fn pad_hit(v: &impl Values, pad: usize) -> HitShape {
    hit_shape(|id| v.get(id), pad, RATE)
}

/// What's left of a part after `secs`, from its share left per frame.
fn left(mul: f32, secs: f32) -> f32 {
    mul.powf(secs * RATE as f32)
}

/// Seconds for a part to fall 60 dB, from its share left per frame.
fn t60(mul: f32) -> f32 {
    if mul >= 1.0 {
        f32::INFINITY
    } else {
        -6.907_755 / (RATE as f32 * mul.ln())
    }
}

/// How long the Hit card lasts: until the hit's longest part is 60 dB
/// down, from 50 ms to 5 s.
pub fn hit_window(s: &HitShape) -> f32 {
    let mut secs = 0.0f32;
    for (lvl, mul) in [s.body, s.noise, s.click] {
        if lvl > 0.0 {
            secs = secs.max(t60(mul));
        }
    }
    if let Some((bursts, _)) = s.bursts {
        secs = secs.max(bursts.last().copied().unwrap_or(0.0) + 0.02);
    }
    secs.clamp(0.05, 5.0)
}

/// The noise part's envelope at `t`: a clap's bursts over a third of its
/// tail, as the engine plays them.
fn noise_env(s: &HitShape, t: f32) -> f32 {
    let tail = left(s.noise.1, t);
    match s.bursts {
        Some((bursts, mul)) => {
            let since = bursts.iter().rev().find(|&&b| b <= t).map(|&b| t - b);
            tail * 0.35 + since.map_or(0.0, |d| left(mul, d))
        }
        None => tail,
    }
}

/// A hit's three parts over `secs`: the body, the noise (with a clap's
/// bursts) and the click, each at its level against the loudest part (in
/// the mix, before the filters). Empty for a part the hit doesn't have.
pub fn hit_parts(s: &HitShape, secs: f32) -> [Vec<(f32, f32)>; 3] {
    let noise_top = if s.bursts.is_some() { 1.35 } else { 1.0 };
    let top = s.body.0.max(s.noise.0 * noise_top).max(s.click.0);
    let curve = |lvl: f32, env: &dyn Fn(f32) -> f32| -> Vec<(f32, f32)> {
        if lvl <= 0.0 || top <= 0.0 {
            return Vec::new();
        }
        (0..=POINTS)
            .map(|i| {
                let x = i as f32 / POINTS as f32;
                (x, 0.9 * lvl / top * env(x * secs))
            })
            .collect()
    };
    [
        curve(s.body.0, &|t| left(s.body.1, t)),
        curve(s.noise.0, &|t| noise_env(s, t)),
        curve(s.click.0, &|t| left(s.click.1, t)),
    ]
}

/// The pitch sweep over [`SWEEP_SECS`], in semitones above the settled
/// pitch over [`SWEEP_TOP`]. Empty for a model with no body to bend.
pub fn sweep_curve(s: &HitShape) -> Vec<(f32, f32)> {
    if s.hz <= 0.0 {
        return Vec::new();
    }
    (0..=POINTS)
        .map(|i| {
            let x = i as f32 / POINTS as f32;
            let st = s.sweep.0 * left(s.sweep.1, x * SWEEP_SECS);
            (x, (st / SWEEP_TOP).clamp(0.0, 1.0) * 0.9)
        })
        .collect()
}

/// A time as "45 ms" or "1.8 s".
pub fn secs_text(secs: f32) -> String {
    if secs < 0.995 {
        format!("{:.0} ms", secs * 1000.0)
    } else {
        format!("{:.1} s", secs)
    }
}

/// The Hit card's caption: "Kick · 450 ms".
pub fn hit_caption(v: &impl Values, pad: usize) -> String {
    let decay = v.get(pad_id(pad.min(PADS - 1), field::DECAY)) * 0.001;
    format!("{} · {}", pad_model(v, pad), secs_text(decay))
}

/// The Sweep card's caption: "+24 st in 45 ms" (down to the pad's pitch),
/// or what the hit has instead.
pub fn sweep_caption(v: &impl Values, pad: usize, s: &HitShape) -> String {
    let time = v.get(pad_id(pad.min(PADS - 1), field::SWEEP_TIME)) * 0.001;
    if s.hz <= 0.0 {
        "no body to sweep".to_string()
    } else if s.sweep.0 < 0.5 {
        format!("{:.0} Hz, no sweep", s.hz)
    } else {
        format!("+{:.0} st in {}", s.sweep.0, secs_text(time))
    }
}

/// Each pad's level, -60 dB (silent) to +6 dB, as 0 to 1.
pub fn pad_levels(v: &impl Values) -> Vec<f32> {
    (0..PADS)
        .map(|p| ((v.get(pad_id(p, field::LEVEL)) + 60.0) / 66.0).clamp(0.0, 1.0))
        .collect()
}

/// Faint lines for a time span of `secs`: one to three, at round times.
pub fn time_grid(secs: f32) -> Vec<(f32, String)> {
    const STEPS: [f32; 11] = [
        0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0,
    ];
    let step = STEPS
        .iter()
        .copied()
        .find(|s| secs / s < 4.0)
        .unwrap_or(5.0);
    // Not so near the right edge that the label runs off the card.
    (1..)
        .map(|k| k as f32 * step)
        .take_while(|t| *t <= secs * 0.85)
        .map(|t| (t / secs, secs_text(t)))
        .collect()
}

// ------------------------------------------------------------ Pluck

/// Whether Pluck is the Mallet model.
pub fn is_mallet(v: &impl Values) -> bool {
    v.get(kp::MODEL) >= 0.5
}

/// Middle C's partials: a string's first harmonics, or a mallet's modes.
pub fn note_partials(v: &impl Values) -> Vec<Partial> {
    let mut out = [Partial::default(); HARMONICS];
    let n = partials(|id| v.get(id), NOTE, RATE, &mut out);
    out[..n].to_vec()
}

/// A partial's level as a bar or stem height: 0 dB at the top,
/// [`PARTIAL_DB`] below it at the bottom.
pub fn level_height(level: f32) -> f32 {
    if level <= 0.0 {
        return 0.0;
    }
    ((20.0 * level.log10() + PARTIAL_DB) / PARTIAL_DB).clamp(0.0, 1.0)
}

/// The Modes card's x for a ratio: 0.4 to 20 times the note, on a log
/// scale.
pub fn ratio_x(ratio: f32) -> f32 {
    ((ratio / 0.4).ln() / (20.0f32 / 0.4).ln()).clamp(0.0, 1.0)
}

/// A mallet's modes as stems, from the bottom to each mode's level.
pub fn mode_stems(parts: &[Partial]) -> Vec<Vec<(f32, f32)>> {
    parts
        .iter()
        .filter(|p| p.level > 0.0)
        .map(|p| {
            let x = ratio_x(p.ratio);
            vec![(x, 0.0), (x, 0.9 * level_height(p.level))]
        })
        .collect()
}

/// The partials picked for the Decay card: a string's harmonics 1, 2, 4
/// and 8, or a mallet's first four modes (those that sound).
pub fn decay_partials(parts: &[Partial], mallet: bool) -> Vec<Partial> {
    let pick: &[usize] = if mallet { &[0, 1, 2, 3] } else { &[0, 1, 3, 7] };
    pick.iter()
        .filter_map(|&k| parts.get(k).copied())
        .filter(|p| p.level > 0.0 && p.t60 > 0.0)
        .collect()
}

/// How long the Decay card lasts: the longest decay drawn, 0.2 to 12 s.
pub fn decay_window(parts: &[Partial]) -> f32 {
    parts
        .iter()
        .map(|p| p.t60)
        .fold(0.0f32, f32::max)
        .clamp(0.2, 12.0)
}

/// The Decay card's range: 0 dB (the loudest partial at the start) at the
/// top, this far down at the bottom.
pub const DECAY_DB: f32 = 60.0;

/// One partial ringing over `secs`, in dB: a straight line from its level
/// down 60 dB in its T60, ending where it leaves the card.
pub fn decay_curve(p: &Partial, secs: f32) -> Vec<(f32, f32)> {
    let start = 20.0 * p.level.max(1e-6).log10();
    let mut pts = Vec::new();
    for i in 0..=POINTS {
        let x = i as f32 / POINTS as f32;
        let db = start - 60.0 * x * secs / p.t60.max(1e-3);
        let y = 0.9 * (1.0 + db / DECAY_DB);
        if y <= 0.0 {
            // Ends on the bottom edge.
            let prev = pts.last().copied().unwrap_or((x, 0.0));
            let along = if prev.1 > 0.0 {
                prev.0 + (x - prev.0) * prev.1 / (prev.1 - y)
            } else {
                x
            };
            pts.push((along, 0.0));
            break;
        }
        pts.push((x, y));
    }
    pts
}

/// The string, picked at Position: a line from end to end, pulled up at
/// the pick. The dot's x is the pick's.
pub fn pick_shape(v: &impl Values) -> (Vec<(f32, f32)>, f32) {
    let pos = (v.get(kp::POSITION) / 100.0).clamp(0.0, 1.0);
    let (a, b) = (0.04, 0.96);
    let x = a + (b - a) * pos;
    (vec![(a, 0.3), (x, 0.75), (b, 0.3)], x)
}

/// The Pick card's caption: "at 12 % · pick 85 %".
pub fn pick_caption(v: &impl Values) -> String {
    format!(
        "at {:.0} % · pick {:.0} %",
        v.get(kp::POSITION),
        v.get(kp::PICK)
    )
}

/// The mallet's pulse at middle C over the Mallet card's 8 ms: a half sine
/// as long as the mallet stays on the bar. Returns the points and its
/// length in seconds.
pub fn mallet_pulse(v: &impl Values) -> (Vec<(f32, f32)>, f32) {
    const SPAN: f32 = 0.008;
    let len = mallet_pulse_secs(|id| v.get(id), NOTE, RATE);
    let pts = (0..=POINTS)
        .map(|i| {
            let x = i as f32 / POINTS as f32;
            let t = x * SPAN;
            let y = if t < len {
                (std::f32::consts::PI * t / len).sin()
            } else {
                0.0
            };
            (x, 0.85 * y)
        })
        .collect();
    (pts, len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Node;
    use mpfree_synth::{Drums, Pluck};

    fn drums(kit: &str) -> Drums {
        let mut d = Drums::new(RATE);
        let k = Drums::presets().iter().find(|k| k.name == kit).unwrap();
        d.load_preset(k);
        d
    }

    fn pluck(name: &str) -> Pluck {
        let mut s = Pluck::new(RATE);
        let p = Pluck::presets().iter().find(|p| p.name == name).unwrap();
        s.load_preset(p);
        s
    }

    fn in_box(pts: &[(f32, f32)]) -> bool {
        pts.iter()
            .all(|&(x, y)| (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y))
    }

    #[test]
    fn the_kit_card_lies_as_the_pads_do() {
        assert_eq!(pad_xy(0).1, pad_xy(3).1);
        assert!(pad_xy(12).1 > pad_xy(0).1, "pad 13 above pad 1");
        assert!(pad_xy(3).0 > pad_xy(0).0, "pad 4 right of pad 1");
        let d = drums("Init Kit");
        let v = |id| d.param(id);
        let tags: Vec<&str> = (0..PADS).map(|p| model_tag(pad_model(&v, p))).collect();
        assert_eq!(&tags[..5], &["BD", "SD", "CP", "CH", "OH"]);
        for m in MODELS {
            assert_eq!(model_tag(m).len(), 2, "{m}");
        }
        assert_eq!(model_family("Kick"), 0);
        assert_eq!(model_family("Clap"), 1);
        assert_eq!(model_family("Open hat"), 2);
    }

    #[test]
    fn every_hit_and_sweep_stays_in_its_box() {
        for k in Drums::presets() {
            let d = drums(k.name);
            let v = |id| d.param(id);
            for pad in 0..PADS {
                let s = pad_hit(&v, pad);
                let secs = hit_window(&s);
                assert!((0.05..=5.0).contains(&secs));
                let parts = hit_parts(&s, secs);
                assert!(parts.iter().any(|p| !p.is_empty()), "{} pad {pad}", k.name);
                for p in parts.iter().chain([&sweep_curve(&s)]) {
                    assert!(in_box(p), "{} pad {pad}: {p:?}", k.name);
                }
                assert!(!time_grid(secs).is_empty());
            }
        }
    }

    #[test]
    fn the_kick_falls_and_sweeps_as_its_knobs_say() {
        let mut d = drums("Init Kit");
        d.set_param(pad_id(0, field::NOISE), 50.0);
        let s = pad_hit(&|id| d.param(id), 0);
        let [body, noise, click] = hit_parts(&s, hit_window(&s));
        // The body starts loudest and is 60 dB down at the end.
        assert!(body[0].1 > 0.5 && body.last().unwrap().1 < 0.001);
        // The click is gone long before the noise, the noise before the body.
        let at = |c: &[(f32, f32)], x: f32| c[(x * POINTS as f32) as usize].1 / c[0].1;
        assert!(at(&click, 0.05) < at(&noise, 0.05) && at(&noise, 0.2) < at(&body, 0.2));
        assert_eq!(hit_caption(&|id| d.param(id), 0), "Kick · 450 ms");
        assert_eq!(sweep_caption(&|id| d.param(id), 0, &s), "+24 st in 45 ms");
        let sweep = sweep_curve(&s);
        assert!((sweep[0].1 - 0.9 * 24.0 / SWEEP_TOP).abs() < 1e-4);
        assert!(sweep.last().unwrap().1 < 0.01);
        // A longer decay draws a longer card.
        d.set_param(pad_id(0, field::DECAY), 2000.0);
        assert!(hit_window(&pad_hit(&|id| d.param(id), 0)) > 1.5);
        // A hat has no body, and says so.
        let hat = pad_hit(&|id| d.param(id), 3);
        assert!(hit_parts(&hat, 0.1)[0].is_empty());
        assert!(sweep_curve(&hat).is_empty());
        assert_eq!(
            sweep_caption(&|id| d.param(id), 3, &hat),
            "no body to sweep"
        );
        // A clap's bursts: four peaks in its first 40 ms.
        let clap = pad_hit(&|id| d.param(id), 2);
        let noise = &hit_parts(&clap, 0.04)[1];
        let peaks = noise
            .windows(3)
            .filter(|w| w[1].1 > w[0].1 && w[1].1 >= w[2].1)
            .count();
        assert_eq!(peaks + 1, 4, "the first burst starts at the hit");
    }

    #[test]
    fn levels_and_time_grids_read_as_set() {
        let mut d = drums("Init Kit");
        d.set_param(pad_id(1, field::LEVEL), -60.0);
        d.set_param(pad_id(2, field::LEVEL), 6.0);
        let l = pad_levels(&|id| d.param(id));
        assert_eq!((l.len(), l[1], l[2]), (16, 0.0, 1.0));
        let g = time_grid(0.45);
        assert_eq!(
            g.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(),
            ["200 ms"]
        );
        assert!(time_grid(0.5).iter().all(|(x, _)| *x <= 0.85));
        assert_eq!(time_grid(2.0)[0].1, "1.0 s");
        assert_eq!(time_grid(0.008)[0].1, "5 ms");
        for secs in [0.008, 0.05, 0.25, 0.45, 1.0, 5.0, 12.0] {
            let n = time_grid(secs).len();
            assert!((1..=3).contains(&n), "{secs} s: {n} lines");
        }
        assert_eq!(secs_text(1.8), "1.8 s");
    }

    #[test]
    fn a_string_draws_its_harmonics_and_their_decays() {
        let s = pluck("Steel String");
        let v = |id| s.param(id);
        assert!(!is_mallet(&v));
        let parts = note_partials(&v);
        assert_eq!(parts.len(), HARMONICS);
        assert!(parts
            .iter()
            .all(|p| (0.0..=1.0).contains(&level_height(p.level))));
        let picked = decay_partials(&parts, false);
        assert_eq!(picked.len(), 4);
        let secs = decay_window(&picked);
        assert!((5.0..=6.1).contains(&secs), "6 s at middle C: {secs}");
        for p in &picked {
            let c = decay_curve(p, secs);
            assert!(in_box(&c));
            // A straight line in dB, falling.
            assert!(c.windows(2).all(|w| w[1].1 <= w[0].1));
        }
        // Higher harmonics die sooner: harmonic 8 reaches the bottom first.
        let ends: Vec<f32> = picked
            .iter()
            .map(|p| decay_curve(p, secs).last().unwrap().0)
            .collect();
        assert!(ends[3] < ends[0], "{ends:?}");
        let (string, x) = pick_shape(&v);
        assert!(in_box(&string) && (x - (0.04 + 0.92 * 0.12)).abs() < 1e-4);
        assert_eq!(pick_caption(&v), "at 12 % · pick 85 %");
    }

    #[test]
    fn a_mallet_draws_its_modes_and_its_pulse() {
        let mut s = pluck("Marimba");
        let v = |id| s.param(id);
        assert!(is_mallet(&v));
        let parts = note_partials(&v);
        assert_eq!(parts.len(), 3);
        let stems = mode_stems(&parts);
        assert_eq!(stems.len(), 3);
        assert!(stems.iter().all(|st| in_box(st)));
        assert!((stems[0][0].0 - ratio_x(1.0)).abs() < 1e-6);
        assert!(stems[1][0].0 > stems[0][0].0, "mode 2 is higher");
        let (pulse, len) = mallet_pulse(&v);
        assert!(in_box(&pulse) && len > 0.0005 && len < 0.008);
        // A harder mallet: a shorter pulse.
        s.set_param(kp::HARDNESS, 100.0);
        assert!(mallet_pulse(&|id| s.param(id)).1 < len);
    }
}
