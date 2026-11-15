//! Report cards: one effect at one setting (its defaults or a preset),
//! measured by every analysis that fits its category, as named numbers.
//! The CLI writes them as JSON for baselines and comparisons.

use mpfree_fx::registry::{Category, FxEntry};
use mpfree_fx::Preset;

use crate::dut::{Dut, NodeDut};
use crate::signals::{db_to_amp, pink};
use crate::wav::Stereo;
use crate::{dynamics, harmonics, loudness, response, room, zipper};

/// What to measure.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub rate: u32,
    /// Fewer sines, no zipper sweep: for the quick gate.
    pub quick: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            rate: 44_100,
            quick: false,
        }
    }
}

/// One report card.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub effect: String,
    pub name: String,
    pub preset: Option<String>,
    pub rate: u32,
    pub metrics: Vec<(String, f64)>,
    pub notes: Vec<String>,
}

impl Card {
    pub fn get(&self, key: &str) -> Option<f64> {
        self.metrics.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
    }

    pub(crate) fn put(&mut self, key: impl Into<String>, v: f64) {
        if v.is_finite() {
            self.metrics.push((key.into(), v));
        } else {
            let key = key.into();
            self.notes.push(format!("{key}: not finite"));
        }
    }
}

/// Effects whose output is not at the input's frequencies (shifters,
/// ring modulation, grains), so a sine's harmonics and a sweep's response
/// mean nothing for them.
pub const MOVES_FREQUENCIES: &[&str] = &[
    "mpfree.fx.ringmod",
    "mpfree.fx.freqshift",
    "mpfree.fx.pitch",
    "mpfree.fx.pitch_correct",
    "mpfree.fx.harmonizer",
    "mpfree.fx.granulator",
];

/// A fresh device for `entry` with `preset` applied.
pub fn make(entry: &FxEntry, preset: Option<&Preset>, rate: u32) -> NodeDut {
    let mut d = NodeDut::new((entry.make)(), rate);
    if let Some(p) = preset {
        for (id, v) in p.values {
            d.set(*id, *v as f64);
        }
    }
    d
}

pub(crate) fn label(hz: f64) -> String {
    if hz >= 1000.0 {
        let k = hz / 1000.0;
        if (k - k.round()).abs() < 1e-9 {
            format!("{}k", k.round())
        } else {
            format!("{k:.1}k")
        }
    } else {
        format!("{hz}")
    }
}

/// Measures `entry` at `preset` (None: its defaults).
pub fn measure(entry: &FxEntry, preset: Option<&Preset>, opts: Options) -> Card {
    let rate = opts.rate;
    let mut card = Card {
        effect: entry.id.to_string(),
        name: entry.name.to_string(),
        preset: preset.map(|p| p.name.to_string()),
        rate,
        metrics: Vec::new(),
        notes: Vec::new(),
    };
    let fresh = || make(entry, preset, rate);
    let mut d = fresh();
    card.put("latency_frames", d.latency() as f64);

    let linear_ok = !MOVES_FREQUENCIES.contains(&entry.id);
    if !linear_ok {
        card.notes
            .push("moves frequencies: no response or harmonics".into());
    }

    // Frequency response at a low level, so drives stay near linear.
    if linear_ok {
        let r = response::measure(&mut d, -24.0, 16_384);
        card.put("measured_delay_frames", r.delay as f64);
        for hz in [
            31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 12_500.0, 16_000.0,
            20_000.0,
        ] {
            if let Some(p) = r.points.iter().find(|p| p.0 == hz) {
                card.put(format!("resp_{}_db", label(hz)), p.1);
            }
        }
    }

    // Distortion and aliasing.
    let sines: &[(f64, f64)] = if !linear_ok {
        &[]
    } else if opts.quick {
        &[(1000.0, -18.0), (5000.0, 0.0)]
    } else {
        &[
            (100.0, 0.0),
            (1000.0, -18.0),
            (1000.0, 0.0),
            (5000.0, 0.0),
            (10_000.0, 0.0),
        ]
    };
    for &(hz, level) in sines {
        let h = harmonics::measure(&mut d, hz, level);
        let tag = format!("{}_{}", label(hz), level as i32);
        card.put(format!("fund_{tag}_db"), h.out_db);
        card.put(format!("thd_{tag}_db"), h.thd_db);
        card.put(format!("alias_{tag}_db"), h.alias_worst_db);
        // The same alias in dBFS: the relative figure says nothing when
        // the effect removes the fundamental (an octave up, a filter).
        card.put(format!("aliasfs_{tag}_db"), h.out_db + h.alias_worst_db);
        card.put(format!("other_{tag}_db"), h.other_total_db);
    }

    // Level on pink noise, true peak, and the idle floor after it.
    let n = 4 * rate as usize;
    let stim = Stereo::mono(pink(n, db_to_amp(-18.0), 11), rate);
    let tail = (d.node().tail_frames() as usize).min(6 * rate as usize) + rate as usize;
    let out = d.render(&stim, tail + d.latency());
    let body = out.slice(d.latency(), n);
    card.put(
        "lufs_delta_pink",
        loudness::integrated(&body) - loudness::integrated(&stim),
    );
    card.put("true_peak_pink_dbtp", loudness::true_peak(&body));
    let idle = out.slice(out.len() - rate as usize / 2, rate as usize / 2);
    let idle_rms = (idle
        .left
        .iter()
        .chain(&idle.right)
        .map(|v| v * v)
        .sum::<f64>()
        / (idle.len() * 2) as f64)
        .sqrt();
    card.put("idle_db", 20.0 * idle_rms.max(1e-15).log10());

    // Tails.
    if matches!(entry.category, Category::DelayReverb) {
        let frames = (d.node().tail_frames() as usize).clamp(rate as usize, 12 * rate as usize);
        let ir = room::impulse_response(&mut d, frames, 0.5);
        for b in room::decay(&ir) {
            if [500.0, 1000.0, 2000.0, 4000.0].contains(&b.hz) {
                if let Some(t) = b.t30.or(b.t20) {
                    card.put(format!("t30_{}_s", label(b.hz)), t);
                }
            }
            if b.hz == 1000.0 {
                if let Some(t) = b.edt {
                    card.put("edt_1k_s", t);
                }
            }
        }
        if let Some(t) = room::time_to_diffuse(&ir) {
            card.put("diffuse_ms", t * 1000.0);
        }
        if let Some(x) = room::tail_excess_db(&ir) {
            card.put("tail_excess_db", x);
        }
        card.put("iacc_tail", room::iacc(&ir, rate as usize / 10));
    }

    // Dynamics.
    if matches!(entry.category, Category::Dynamics) {
        for (i, o) in dynamics::static_curve(&mut d, &[-40.0, -30.0, -20.0, -10.0, 0.0]) {
            card.put(format!("curve_{}_db", i as i32), o);
        }
        let t = dynamics::timing(&mut d, -40.0, 0.0, 1.0);
        card.put("step_depth_db", t.depth_db);
        if let Some(v) = t.attack_ms {
            card.put("attack_ms", v);
        }
        if let Some(v) = t.release_ms {
            card.put("release_ms", v);
        }
    }

    // Zipper noise on every continuous parameter.
    if !opts.quick {
        let mut mk = || -> Box<dyn Dut> { Box::new(fresh()) };
        let z = zipper::sweep_all(&mut mk, 50.0);
        if let Some(w) = z.iter().max_by(|a, b| a.step_db.total_cmp(&b.step_db)) {
            card.put("zipper_worst_db", w.step_db);
            card.notes
                .push(format!("zipper worst: {} ({:.1} dB)", w.name, w.step_db));
        }
    }
    card
}

/// Writes cards as a JSON array.
pub fn to_json(cards: &[Card]) -> String {
    let esc = |s: &str| -> String {
        let mut o = String::with_capacity(s.len() + 2);
        o.push('"');
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                c => o.push(c),
            }
        }
        o.push('"');
        o
    };
    let mut s = String::from("[\n");
    for (i, c) in cards.iter().enumerate() {
        s.push_str("  {");
        s.push_str(&format!(
            "\"effect\": {}, \"name\": {}, ",
            esc(&c.effect),
            esc(&c.name)
        ));
        match &c.preset {
            Some(p) => s.push_str(&format!("\"preset\": {}, ", esc(p))),
            None => s.push_str("\"preset\": null, "),
        }
        s.push_str(&format!("\"rate\": {},\n    \"metrics\": {{", c.rate));
        for (j, (k, v)) in c.metrics.iter().enumerate() {
            if j > 0 {
                s.push_str(", ");
            }
            s.push_str(&format!("{}: {}", esc(k), round(*v)));
        }
        s.push_str("},\n    \"notes\": [");
        for (j, n) in c.notes.iter().enumerate() {
            if j > 0 {
                s.push_str(", ");
            }
            s.push_str(&esc(n));
        }
        s.push_str("]}");
        if i + 1 < cards.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push_str("]\n");
    s
}

/// Four decimals: enough for dB and ms, and stable across machines.
fn round(v: f64) -> String {
    let r = (v * 10_000.0).round() / 10_000.0;
    if r == 0.0 {
        "0".into()
    } else {
        format!("{r}")
    }
}
