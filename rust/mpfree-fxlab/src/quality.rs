//! The FX quality report: every measured target of one class of effect
//! (delay, EQ, reverb; in the second round drive, crush, compressor,
//! filter and lo-fi, in `quality/round2.rs`; in the third the modulation
//! effects, in `quality/modulation.rs`; in the fourth the rest of the
//! dynamics, in `quality/round4.rs`), for each mode of its family
//! at its defaults and,
//! for the cheaper measures, at each factory preset. Each row is a number
//! with the plan's target id, which way is better, the target where the
//! plan sets one, and how much it may move before it counts as worse.
//!
//! The ratchet (`tests/quality.rs`, `quality/ratchet.tsv`) keeps every
//! row from getting worse: a change that improves a number re-records it
//! with `MPFREE_QUALITY_BLESS=1`, and the file's diff shows it in review.
//! Targets not yet met are listed, not failed. The targets and why each
//! was chosen: `docs/dev/design/fx-quality.md`.

use mpfree_fx::families::{self, Family, Mode};
use mpfree_fx::mpfree_node::Node;
use mpfree_fx::Preset;

use crate::card::label;
use crate::dut::{Dut, NodeDut};
use crate::signals::{db_to_amp, pink, Rng};
use crate::wav::Stereo;
use crate::{
    clarity, echo, flutter, harmonics, knob, loudness, mixlaw, prototype, response, room, wobble,
    zipper,
};

mod modulation;
mod round2;
mod round4;

/// A class of effect the plan covers so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Delay,
    Eq,
    Reverb,
    Drive,
    Crush,
    Compressor,
    Filter,
    Lofi,
    Amp,
    Modulation,
    Limiter,
    Gate,
    Deesser,
    Multiband,
    Ducker,
    Transient,
    Strip,
}

impl Class {
    pub const ALL: [Class; 17] = [
        Class::Delay,
        Class::Eq,
        Class::Reverb,
        Class::Drive,
        Class::Crush,
        Class::Compressor,
        Class::Filter,
        Class::Lofi,
        Class::Amp,
        Class::Modulation,
        Class::Limiter,
        Class::Gate,
        Class::Deesser,
        Class::Multiband,
        Class::Ducker,
        Class::Transient,
        Class::Strip,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Class::Delay => "delay",
            Class::Eq => "eq",
            Class::Reverb => "reverb",
            Class::Drive => "drive",
            Class::Crush => "crush",
            Class::Compressor => "compressor",
            Class::Filter => "filter",
            Class::Lofi => "lofi",
            Class::Amp => "amp",
            Class::Modulation => "modulation",
            Class::Limiter => "limiter",
            Class::Gate => "gate",
            Class::Deesser => "deesser",
            Class::Multiband => "multiband",
            Class::Ducker => "ducker",
            Class::Transient => "transient",
            Class::Strip => "strip",
        }
    }

    pub fn parse(s: &str) -> Option<Class> {
        Class::ALL.into_iter().find(|c| c.name() == s)
    }
}

/// Which way a number is better.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Better {
    Lower,
    Higher,
    /// Closer to this value.
    Near(f64),
    /// No better or worse: shown, not kept by the ratchet.
    Info,
}

/// One measured number.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The plan's target id ("D2").
    pub target_id: &'static str,
    /// What was measured: "delay/digital", or with a preset
    /// "delay/digital[Slapback]".
    pub subject: String,
    pub metric: String,
    pub value: f64,
    pub better: Better,
    /// The plan's target: the most (Lower), the least (Higher) or the
    /// largest distance (Near) that meets it.
    pub target: Option<f64>,
    /// How far it may move the wrong way before the ratchet calls it worse.
    pub tol: f64,
}

impl Row {
    /// The ratchet's key.
    pub fn key(&self) -> String {
        format!("{}/{}", self.subject, self.metric)
    }

    /// Whether it meets its target, if it has one.
    pub fn met(&self) -> Option<bool> {
        let t = self.target?;
        Some(match self.better {
            Better::Lower => self.value <= t,
            Better::Higher => self.value >= t,
            Better::Near(c) => (self.value - c).abs() <= t,
            Better::Info => return None,
        })
    }

    /// Whether this value is worse than `old` by more than the tolerance.
    pub fn worse_than(&self, old: f64) -> bool {
        match self.better {
            Better::Lower => self.value > old + self.tol,
            Better::Higher => self.value < old - self.tol,
            Better::Near(c) => (self.value - c).abs() > (old - c).abs() + self.tol,
            Better::Info => false,
        }
    }
}

/// One class's report.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub class: Class,
    pub rate: u32,
    pub rows: Vec<Row>,
    /// What could not be measured, and why.
    pub notes: Vec<String>,
}

impl Report {
    #[allow(clippy::too_many_arguments)]
    fn put(
        &mut self,
        target_id: &'static str,
        subject: &str,
        metric: impl Into<String>,
        value: Option<f64>,
        better: Better,
        target: Option<f64>,
        tol: f64,
    ) {
        let metric = metric.into();
        match value {
            Some(v) if v.is_finite() => self.rows.push(Row {
                target_id,
                subject: subject.to_string(),
                metric,
                value: v,
                better,
                target,
                tol,
            }),
            _ => self
                .notes
                .push(format!("{subject}/{metric}: could not be measured")),
        }
    }
}

/// Measures every target of `class` at `rate`.
pub fn report(class: Class, rate: u32) -> Report {
    let mut r = Report {
        class,
        rate,
        rows: Vec::new(),
        notes: Vec::new(),
    };
    match class {
        Class::Delay => delay(&mut r),
        Class::Eq => eq(&mut r),
        Class::Reverb => reverb(&mut r),
        Class::Drive => round2::drive(&mut r),
        Class::Crush => round2::crush(&mut r),
        Class::Compressor => round2::compressor(&mut r),
        Class::Filter => round2::filter(&mut r),
        Class::Lofi => round2::lofi(&mut r),
        Class::Amp => round2::amp(&mut r),
        Class::Modulation => modulation::modulation(&mut r),
        Class::Limiter => round4::limiter(&mut r),
        Class::Gate => round4::gate(&mut r),
        Class::Deesser => round4::deesser(&mut r),
        Class::Multiband => round4::multiband(&mut r),
        Class::Ducker => round4::ducker(&mut r),
        Class::Transient => round4::transient(&mut r),
        Class::Strip => round4::strip(&mut r),
    }
    r
}

// ---------------------------------------------------------------------------
// Subjects: a family's modes and presets.

/// A mode of a family, perhaps at a preset.
struct Subject {
    name: String,
    id: &'static str,
    fixed: &'static [(u16, f32)],
    preset: Option<&'static Preset>,
}

fn slug(s: &str) -> String {
    s.to_lowercase().replace(' ', "-")
}

fn family(name: &str) -> &'static Family {
    families::FAMILIES
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no family {name}"))
}

/// Each mode of family `name` at its defaults, then each of the family's
/// presets in its mode.
fn subjects(name: &str, class: &str) -> (Vec<Subject>, Vec<Subject>) {
    let f = family(name);
    let mode = |m: &'static Mode, preset: Option<&'static Preset>| Subject {
        name: match preset {
            None => format!("{class}/{}", slug(m.label)),
            Some(p) => format!("{class}/{}[{}]", slug(m.label), p.name),
        },
        id: m.id,
        fixed: m.fixed,
        preset,
    };
    let defaults = f.modes.iter().map(|m| mode(m, None)).collect();
    let presets = families::presets(f)
        .into_iter()
        .map(|p| mode(&f.modes[p.mode], Some(p.preset)))
        .collect();
    (defaults, presets)
}

fn make(s: &Subject, rate: u32) -> NodeDut {
    let mut d =
        NodeDut::from_registry(s.id, rate).unwrap_or_else(|| panic!("{} is in the registry", s.id));
    if let Some(p) = s.preset {
        for (id, v) in p.values {
            d.set(*id, *v as f64);
        }
    }
    for (id, v) in s.fixed {
        d.set(*id, *v as f64);
    }
    d
}

fn pid(d: &NodeDut, name: &str) -> Option<u16> {
    d.params().iter().find(|p| p.name == name).map(|p| p.id)
}

fn range(d: &NodeDut, name: &str) -> Option<(f64, f64)> {
    d.params()
        .iter()
        .find(|p| p.name == name)
        .map(|p| (p.min as f64, p.max as f64))
}

/// Sets each named control the device has (others are skipped).
fn set(d: &mut NodeDut, values: &[(&str, f64)]) {
    for (name, v) in values {
        if let Some(id) = pid(d, name) {
            d.set(id, *v);
        }
    }
}

fn worst_zipper(s: &Subject, rate: u32) -> Option<f64> {
    let mut mk = || -> Box<dyn Dut> { Box::new(make(s, rate)) };
    zipper::sweep_all(&mut mk, 50.0)
        .iter()
        .map(|z| z.step_db.max(0.0))
        .reduce(f64::max)
}

/// Levels far under anything audible all read as -140 dB, so the ratchet
/// doesn't hold numerical noise to account.
fn floor(db: f64) -> f64 {
    db.max(-140.0)
}

/// Loudness of `d`'s output against `x`, LU, leaving out the first and
/// last second.
fn lu_change(d: &mut NodeDut, x: &Stereo) -> f64 {
    let rate = x.rate as usize;
    let lat = d.latency();
    let out = d.render(x, lat);
    let body = out.slice(rate + lat, x.len() - 2 * rate);
    loudness::integrated(&body) - loudness::integrated(&x.slice(rate, x.len() - 2 * rate))
}

/// The loudness of the output's mono sum against the output, LU.
fn mono_sum(d: &mut NodeDut, x: &Stereo) -> f64 {
    let rate = x.rate as usize;
    let out = d.render(x, 0);
    let body = out.slice(rate, x.len() - 2 * rate);
    loudness::integrated(&Stereo::mono(body.mid(), x.rate)) - loudness::integrated(&body)
}

fn pink_stim(rate: u32, secs: f64) -> Stereo {
    Stereo::mono(
        pink((secs * rate as f64) as usize, db_to_amp(-18.0), 7),
        rate,
    )
}

// ---------------------------------------------------------------------------
// Delay (plan section 5).

/// The delay set up to measure its loop: tone open, no saturation, no
/// wow, one head, feedback 80 %, wet only. For the multi-tap, only tap 4
/// (the one that feeds back) plays, `sixteenths` long; for the others
/// `ms` sets the time. Returns the device and its period in frames.
fn open_loop(mode: &Subject, rate: u32, ms: f64, sixteenths: f64) -> (NodeDut, f64) {
    let mut d = make(mode, rate);
    let tone = range(&d, "Tone").map_or(20_000.0, |r| r.1);
    let low = range(&d, "Low cut").map_or(20.0, |r| r.0);
    set(
        &mut d,
        &[
            ("Sync", 0.0),
            ("Time", ms),
            ("Feedback", 80.0),
            ("Tone", tone),
            ("Low cut", low),
            ("Saturation", 0.0),
            ("Wow", 0.0),
            ("Flutter", 0.0),
            ("Heads", 0.0),
            ("Spread", 0.0),
            ("Ping-pong", 0.0),
            ("Mix", 100.0),
        ],
    );
    if mode.id == "mpfree.fx.multitap" {
        for t in 1..=3 {
            set(&mut d, &[(format!("Tap {t} level").as_str(), 0.0)]);
        }
        set(
            &mut d,
            &[
                ("Tap 4 level", 100.0),
                ("Tap 4 time", sixteenths),
                ("Tap 4 pan", 0.0),
            ],
        );
        // A sixteenth at the measuring tempo, 120 BPM.
        (d, sixteenths * 0.125 * rate as f64)
    } else {
        (d, ms / 1000.0 * rate as f64)
    }
}

fn delay(r: &mut Report) {
    let rate = r.rate;
    let (defaults, presets) = subjects("Delay", "delay");
    for s in &defaults {
        let name = s.name.as_str();
        // D2 and D1: the loop's loss at a whole and at a half frame.
        let half_ms = 100.0 + 0.5 * 1000.0 / rate as f64;
        let reps = |ms: f64, six: f64| {
            let (mut d, period) = open_loop(s, rate, ms, six);
            let ir = room::impulse_response(&mut d, (period * 9.0) as usize + 4096, 0.5);
            (echo::repeats(&ir, 0.5, period, 8), period)
        };
        let (whole, _) = reps(100.0, 2.0);
        let (half, p_half) = reps(half_ms, 3.0);
        for (b, metric, target) in [
            (2, "hidden_loss_10k_db", 0.1),
            (3, "hidden_loss_15k_db", 0.3),
        ] {
            let v = echo::loss_per_repeat(&half, b)
                .zip(echo::loss_per_repeat(&whole, b))
                .map(|(h, w)| h - w);
            r.put("D2", name, metric, v, Better::Lower, Some(target), 0.02);
        }
        // The first repeat lands on Time (the loop's filters may add a
        // fraction of a frame per pass after it, which nobody hears).
        r.put(
            "D1",
            name,
            "time_error_frames",
            (!half.is_empty()).then(|| echo::worst_time_error(&half[..1], p_half)),
            Better::Near(0.0),
            Some(0.5),
            0.05,
        );

        // D10: ping-pong alternates, each repeat on its side.
        if s.id == "mpfree.fx.delay" {
            let (mut d, period) = open_loop(s, rate, 100.0, 0.0);
            set(&mut d, &[("Ping-pong", 1.0)]);
            let ir = room::impulse_response(&mut d, (period * 5.0) as usize + 4096, 0.5);
            let b: Vec<f64> = echo::repeats(&ir, 0.5, period, 4)
                .iter()
                .map(|x| x.balance_db.clamp(-60.0, 60.0))
                .collect();
            let alternates = b.len() == 4 && b.windows(2).all(|w| w[0] * w[1] < 0.0);
            let v = if alternates {
                b.iter().map(|x| x.abs()).fold(f64::INFINITY, f64::min)
            } else {
                0.0
            };
            r.put(
                "D10",
                name,
                "pingpong_side_db",
                Some(v),
                Better::Higher,
                Some(20.0),
                0.5,
            );
        }

        // D8: the tape's first echo already sounds like tape.
        if s.id == "mpfree.fx.tape_delay" {
            let mut d = make(s, rate);
            set(&mut d, &[("Wow", 0.0), ("Flutter", 0.0), ("Mix", 100.0)]);
            let ms = d.node().param(mpfree_fx::ids::tape::TIME) as f64;
            let period = ms / 1000.0 * rate as f64;
            let ir = room::impulse_response(&mut d, (period * 3.0) as usize + 4096, 0.5);
            let reps = echo::repeats(&ir, 0.5, period, 2);
            let v = (reps.len() == 2 && reps[1].tilt(2) < -0.5)
                .then(|| reps[0].tilt(2) / reps[1].tilt(2));
            r.put(
                "D8",
                name,
                "first_echo_colour_share",
                v,
                Better::Higher,
                Some(0.4),
                0.02,
            );
        }

        // D6: loudness across the Mix knob.
        let mix_id = pid(&make(s, rate), "Mix");
        if let Some(id) = mix_id {
            let mut mk = || -> Box<dyn Dut> { Box::new(make(s, rate)) };
            let pts = mixlaw::measure(&mut mk, id, &mixlaw::MIXES);
            let (dip, bump) = mixlaw::dip_and_bump(&pts);
            r.put(
                "D6",
                name,
                "mix_dip_lu",
                Some(dip),
                Better::Higher,
                Some(-1.0),
                0.05,
            );
            r.put(
                "D6",
                name,
                "mix_bump_lu",
                Some(bump),
                Better::Lower,
                Some(1.5),
                0.05,
            );
        }

        // D4: over-unity feedback stays bounded and free of DC: two
        // seconds of noise, then eight of silence while it rings.
        {
            let mut d = make(s, rate);
            let max = range(&d, "Feedback").map_or(100.0, |r| r.1);
            set(&mut d, &[("Feedback", max)]);
            let out = d.render(&pink_stim(rate, 2.0), 8 * rate as usize);
            let peak = loudness::sample_peak(&out);
            let last = out.slice(out.len() - 2 * rate as usize, 2 * rate as usize);
            let dc = last.mid().iter().sum::<f64>() / last.len() as f64;
            r.put(
                "D4",
                name,
                "runaway_peak_dbfs",
                Some(peak),
                Better::Lower,
                Some(6.02),
                0.1,
            );
            r.put(
                "D4",
                name,
                "runaway_dc_dbfs",
                Some(floor(20.0 * dc.abs().max(1e-10).log10())),
                Better::Lower,
                Some(-60.0),
                1.0,
            );
        }

        // D9: wow and flutter, off and at the defaults.
        if s.id == "mpfree.fx.tape_delay" {
            for (metric, wow, target, better) in [
                ("flutter_off_peak_pct", Some(0.0), Some(0.01), Better::Lower),
                ("flutter_default_peak_pct", None, None, Better::Info),
                ("flutter_full_peak_pct", Some(100.0), None, Better::Info),
            ] {
                let mut d = make(s, rate);
                set(&mut d, &[("Feedback", 0.0), ("Mix", 100.0)]);
                if let Some(w) = wow {
                    set(&mut d, &[("Wow", w), ("Flutter", w)]);
                }
                let f = flutter::measure(&mut d, 6.0).map(|f| f.peak_pct);
                r.put("D9", name, metric, f, better, target, 0.002);
            }
        }

        // D11: the wet signal summed to mono.
        {
            let mut d = make(s, rate);
            set(&mut d, &[("Mix", 100.0)]);
            let v = mono_sum(&mut d, &pink_stim(rate, 6.0));
            r.put(
                "D11",
                name,
                "mono_sum_lu",
                Some(v),
                Better::Higher,
                Some(-3.5),
                0.05,
            );
        }
    }

    // Per mode and preset: the tail after the slot rests, saturation's
    // aliases, zipper noise.
    for s in defaults.iter().chain(&presets) {
        let name = s.name.as_str();
        let mut d = make(s, rate);
        let tail = d.node().tail_frames() as usize;
        if tail <= 30 * rate as usize {
            let burst = rate as usize;
            let out = d.render(&pink_stim(rate, 1.0), tail + rate as usize / 2);
            let w = rate as usize / 10;
            let at = burst + tail;
            let piece = &out.mid()[at.saturating_sub(w)..at];
            let rms = (piece.iter().map(|v| v * v).sum::<f64>() / w as f64).sqrt();
            let v = 20.0 * (rms / db_to_amp(-18.0)).max(1e-10).log10();
            r.put(
                "D12",
                name,
                "tail_cut_db",
                Some(floor(v)),
                Better::Lower,
                Some(-60.0),
                1.0,
            );
        } else {
            r.notes.push(format!(
                "{name}: rings on (Freeze or a tail over 30 s), no tail cut"
            ));
        }
        let h = harmonics::measure(&mut make(s, rate), 5000.0, 0.0);
        r.put(
            "D5",
            name,
            "alias_5k_0_db",
            Some(floor(h.alias_worst_db)),
            Better::Lower,
            Some(-70.0),
            1.0,
        );
        r.put(
            "D7",
            name,
            "zipper_worst_db",
            worst_zipper(s, rate),
            Better::Lower,
            Some(3.0),
            0.5,
        );
    }
}

// ---------------------------------------------------------------------------
// EQ (plan section 6).

const EQ_ID: &str = "mpfree.fx.eq";
use mpfree_fx::ids::eq as eqi;

/// An EQ with `values` set, prepared at `rate`.
fn eq_with(values: &[(u16, f32)], rate: u32) -> mpfree_fx::Eq {
    let mut e = mpfree_fx::Eq::new();
    e.prepare(rate, 128);
    for (id, v) in values {
        e.set_param(*id, *v);
    }
    e
}

/// The EQ's own curve (what it draws, and within 0.05 dB what it plays)
/// at each of `freqs`, dB.
fn curve(e: &mpfree_fx::Eq, freqs: &[f64]) -> Vec<f64> {
    let hz: Vec<f32> = freqs.iter().map(|f| *f as f32).collect();
    let mut out = vec![0.0f32; hz.len()];
    e.response_db(&hz, &mut out);
    out.into_iter().map(f64::from).collect()
}

fn band(b: usize, kind: f32, hz: f32, gain: f32, q: f32) -> Vec<(u16, f32)> {
    vec![
        (eqi::band_type(b), kind),
        (eqi::freq(b), hz),
        (eqi::gain(b), gain),
        (eqi::q(b), q),
        (eqi::band_use(b), eqi::USE_ON),
    ]
}

const BELL: f32 = 0.0;
const LOW_SHELF: f32 = 1.0;
const HIGH_SHELF: f32 = 2.0;

fn eq(r: &mut Report) {
    let rate = r.rate;
    let audio = prototype::log_freqs(20.0, 20_000.0f64.min(0.49 * rate as f64), 400);
    let nyq = 0.4999 * rate as f64;

    // E3 at both Qualities: Normal (`eq`) and HQ 2x (`eq[HQ 2x]`).
    for (subject, quality) in [("eq", eqi::NORMAL), ("eq[HQ 2x]", eqi::HQ)] {
        eq_curves(r, subject, quality, &audio, nyq);
    }

    // E4: the conventions.
    {
        let e = eq_with(&band(1, BELL, 1000.0, 12.0, 1.41), rate);
        let w = prototype::half_gain_width(|f| curve(&e, &[f])[0], 12.0, 10.0, nyq);
        let v = w.map(|w| w - prototype::q_to_octaves(1.41));
        r.put(
            "E4",
            "eq",
            "q_convention_oct",
            v,
            Better::Near(0.0),
            Some(0.05),
            0.005,
        );
        let e = eq_with(&band(1, HIGH_SHELF, 1000.0, 6.0, 0.71), rate);
        let at = curve(&e, &[1000.0])[0];
        r.put(
            "E4",
            "eq",
            "shelf_midpoint_db",
            Some(at - 3.0),
            Better::Near(0.0),
            Some(0.1),
            0.01,
        );
    }

    // E5: a boost and the same cut make flat.
    {
        let mut v = band(1, BELL, 2000.0, 9.0, 2.0);
        v.extend(band(2, BELL, 2000.0, -9.0, 2.0));
        let e = eq_with(&v, rate);
        let worst = curve(&e, &audio).iter().fold(0.0f64, |m, x| m.max(x.abs()));
        r.put(
            "E5",
            "eq",
            "mirror_db",
            Some(worst),
            Better::Lower,
            Some(0.01),
            0.002,
        );
    }

    // E2: what it draws is what it plays; at HQ 2x with a broad bell up
    // high, where the half-bands round the bands count, to 20 kHz.
    for (subject, values, top) in [
        ("eq", band(1, BELL, 1000.0, 6.0, 1.0), 16_000.0),
        (
            "eq[HQ 2x]",
            {
                let mut v = band(1, BELL, 12_000.0, 12.0, 0.71);
                v.push((eqi::QUALITY, eqi::HQ));
                v
            },
            20_000.0f64.min(0.45 * rate as f64),
        ),
    ] {
        let mut d = NodeDut::from_registry(EQ_ID, rate).expect("the EQ is in the registry");
        for (id, v) in &values {
            d.set(*id, *v as f64);
        }
        let e = eq_with(&values, rate);
        let m = response::measure(&mut d, -12.0, 16_384);
        let worst = m.worst_error(30.0, top, |f| curve(&e, &[f])[0]);
        r.put(
            "E2",
            subject,
            "drawn_vs_heard_db",
            Some(worst),
            Better::Lower,
            Some(0.05),
            0.01,
        );
    }

    // E6: clean, even a narrow low boost.
    {
        let mut d = NodeDut::from_registry(EQ_ID, rate).expect("the EQ is in the registry");
        for (id, v) in band(1, BELL, 100.0, 12.0, 4.0) {
            d.set(id, v as f64);
        }
        let h = harmonics::measure(&mut d, 100.0, -18.0);
        r.put(
            "E6",
            "eq",
            "thd_100_db",
            Some(floor(h.thd_db)),
            Better::Lower,
            Some(-140.0),
            3.0,
        );
    }

    // E7: zipper at the defaults and every preset.
    let entry = mpfree_fx::registry::entry(EQ_ID).expect("the EQ is in the registry");
    let hq = Subject {
        name: "eq[HQ 2x]".into(),
        id: EQ_ID,
        fixed: &[(eqi::QUALITY, eqi::HQ)],
        preset: None,
    };
    let subjects = std::iter::once(None)
        .chain(entry.presets.iter().map(Some))
        .map(|p| Subject {
            name: match p {
                None => "eq".into(),
                Some(p) => format!("eq[{}]", p.name),
            },
            id: EQ_ID,
            fixed: &[],
            preset: p,
        })
        .chain(std::iter::once(hq));
    for s in subjects {
        r.put(
            "E7",
            &s.name,
            "zipper_worst_db",
            worst_zipper(&s, rate),
            Better::Lower,
            Some(3.0),
            0.5,
        );
    }
}

/// E3 for the EQ at `quality`: +12 dB bells and +6 dB shelves against
/// their analog prototypes, and the cuts against Butterworth, from its own
/// curve (which E2 holds to what it plays).
fn eq_curves(r: &mut Report, subject: &str, quality: f32, audio: &[f64], nyq: f64) {
    let rate = r.rate;
    let top = nyq.min(20_000.0);
    let with = |mut v: Vec<(u16, f32)>| {
        v.push((eqi::QUALITY, quality));
        v
    };
    for fc in [1000.0, 5000.0, 8000.0, 12_000.0, 16_000.0] {
        for q in [0.71, 1.41, 4.0] {
            let e = eq_with(&with(band(1, BELL, fc as f32, 12.0, q as f32)), rate);
            let c = curve(&e, audio);
            let (worst, _) =
                prototype::worst_diff(audio, &c, |f| prototype::bell_db(f, fc, 12.0, q));
            let tag = format!("bell_{}_q{q}", label(fc));
            r.put(
                "E3",
                subject,
                format!("{tag}_worst_db"),
                Some(worst),
                Better::Near(0.0),
                Some(0.5),
                0.02,
            );
            // Both widths inside the band the target is set on: a bell
            // whose half-gain edge lies above 20 kHz (or Nyquist) compares
            // what is left of it, not a digital width cut off at Nyquist
            // with an analog one that runs on.
            let dig = prototype::half_gain_width(|f| curve(&e, &[f])[0], 12.0, 10.0, top);
            let ana =
                prototype::half_gain_width(|f| prototype::bell_db(f, fc, 12.0, q), 12.0, 10.0, top);
            let ratio = dig.zip(ana).map(|(d, a)| d / a);
            r.put(
                "E3",
                subject,
                format!("{tag}_width_ratio"),
                ratio,
                Better::Near(1.0),
                Some(0.05),
                0.005,
            );
        }
    }
    // E3: shelves.
    for (kind, name, freqs) in [
        (HIGH_SHELF, "high_shelf", [2000.0, 5000.0, 8000.0, 12_000.0]),
        (LOW_SHELF, "low_shelf", [50.0, 100.0, 200.0, 500.0]),
    ] {
        for fc in freqs {
            let e = eq_with(&with(band(1, kind, fc as f32, 6.0, 0.71)), rate);
            let c = curve(&e, audio);
            let (worst, _) = prototype::worst_diff(audio, &c, |f| {
                if kind == HIGH_SHELF {
                    prototype::high_shelf_db(f, fc, 6.0, 0.71)
                } else {
                    prototype::low_shelf_db(f, fc, 6.0, 0.71)
                }
            });
            r.put(
                "E3",
                subject,
                format!("{name}_{}_worst_db", label(fc)),
                Some(worst),
                Better::Near(0.0),
                Some(0.5),
                0.02,
            );
        }
    }
    // The cuts against Butterworth, down to -12 dB (below that a digital
    // low-pass falls faster than the analog one toward its zero at
    // Nyquist, which is cut either way). Reported and kept, no target yet.
    for (high, fc, slope) in [
        (false, 100.0, 1.0),
        (false, 100.0, 2.0),
        (true, 5000.0, 1.0),
        (true, 5000.0, 2.0),
        (true, 12_000.0, 2.0),
    ] {
        let (fid, sid) = if high {
            (eqi::HIGH_CUT, eqi::HIGH_SLOPE)
        } else {
            (eqi::LOW_CUT, eqi::LOW_SLOPE)
        };
        let e = eq_with(&with(vec![(fid, fc as f32), (sid, slope)]), rate);
        let c = curve(&e, audio);
        let order = if slope == 1.0 { 2 } else { 4 };
        let analog = |f: f64| prototype::butterworth_db(f, fc, order, !high);
        let (fs, cs): (Vec<f64>, Vec<f64>) = audio
            .iter()
            .zip(&c)
            .filter(|(f, _)| analog(**f) > -12.0)
            .map(|(f, v)| (*f, *v))
            .unzip();
        let (worst, _) = prototype::worst_diff(&fs, &cs, analog);
        let name = format!(
            "{}_cut_{}_{}db_worst_db",
            if high { "high" } else { "low" },
            label(fc),
            6 * order
        );
        r.put(
            "E3",
            subject,
            name,
            Some(worst),
            Better::Near(0.0),
            None,
            0.02,
        );
    }
}

// ---------------------------------------------------------------------------
// Reverb (plan section 7).

/// Decay settings tried per mode, ms (clamped to the control's range).
fn decay_settings(mode: &str) -> &'static [f64] {
    match mode {
        "reverb/room" => &[300.0, 800.0, 2000.0, 5000.0],
        "reverb/plate" => &[500.0, 2000.0, 6000.0],
        _ => &[1000.0, 2500.0, 6000.0],
    }
}

/// Seconds for echo density to reach 0.9 that a mode should beat, ms.
fn diffuse_target(mode: &str) -> Option<f64> {
    match mode {
        "reverb/plate" => Some(30.0),
        "reverb/room" => Some(50.0),
        "reverb/hall" | "reverb/cloud" => Some(100.0),
        _ => None,
    }
}

/// What an ideal tail of `t60` seconds and `frames` long scores for
/// [`room::tail_excess_db`]: exponentially decaying Gaussian noise.
fn ideal_excess(t60: f64, frames: usize, rate: u32) -> Option<f64> {
    let (mut a, mut b) = (Rng::new(5), Rng::new(6));
    let k = (-6.907_755_278_982_137 / (t60 * rate as f64)).exp();
    let mut g = 0.1;
    let (mut l, mut r) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
    for _ in 0..frames {
        l.push(a.gauss() * g);
        r.push(b.gauss() * g);
        g *= k;
    }
    room::tail_excess_db(&Stereo::new(l, r, rate))
}

fn reverb(r: &mut Report) {
    let rate = r.rate;
    let (defaults, presets) = subjects("Reverb", "reverb");
    for s in &defaults {
        let name = s.name.as_str();
        let wet = || {
            let mut d = make(s, rate);
            set(&mut d, &[("Mix", 100.0)]);
            d
        };

        // R1: Decay against the measured decay.
        if let Some(id) = pid(&wet(), "Decay") {
            let (lo, hi) = range(&wet(), "Decay").unwrap_or((0.0, f64::MAX));
            let settings: Vec<f64> = decay_settings(name)
                .iter()
                .map(|v| v.clamp(lo, hi))
                .collect();
            let mut mk = || -> Box<dyn Dut> { Box::new(wet()) };
            for p in knob::decay(&mut mk, id, &settings) {
                r.put(
                    "R1",
                    name,
                    format!("decay_error_pct@{}s", p.set_s),
                    Some(p.error_pct()),
                    Better::Near(0.0),
                    Some(10.0),
                    0.5,
                );
            }
        }

        // The defaults' impulse response: one slope, density, colour, width.
        let mut d = wet();
        let frames = (d.node().tail_frames() as usize).clamp(rate as usize, 12 * rate as usize);
        let ir = room::impulse_response(&mut d, frames, 0.5);
        // Hall and Cloud may build slower (up to 1.6); Echoes is a pattern
        // of echoes over 600 ms before its tail, so its early decay is
        // slower by design and only reported, as its density and colour.
        let one_slope = match name {
            "reverb/hall" | "reverb/cloud" => Some(1.6f64.log2()),
            "reverb/echoes" => None,
            _ => Some(1.25f64.log2()),
        };
        r.put(
            "R3",
            name,
            "edt_t30_log2",
            clarity::edt_over_t30(&ir).map(f64::log2),
            Better::Near(0.0),
            one_slope,
            0.02,
        );
        r.put(
            "R3",
            name,
            "c80_db",
            clarity::clarity_db(&ir, 80.0),
            Better::Info,
            None,
            0.0,
        );
        r.put(
            "R4",
            name,
            "diffuse_ms",
            room::time_to_diffuse(&ir).map(|t| t * 1000.0),
            Better::Lower,
            diffuse_target(name),
            2.0,
        );
        let t60 = knob::t60_mid(&ir);
        let excess = room::tail_excess_db(&ir)
            .zip(t60.and_then(|t| ideal_excess(t, ir.len(), rate)))
            .map(|(m, i)| m - i);
        let excess_target = (!matches!(name, "reverb/spring" | "reverb/echoes")).then_some(1.5);
        r.put(
            "R5",
            name,
            "tail_excess_over_ideal_db",
            excess,
            Better::Lower,
            excess_target,
            0.2,
        );
        r.put(
            "R8",
            name,
            "iacc_tail",
            Some(room::iacc(&ir, rate as usize / 10)),
            Better::Lower,
            Some(0.2),
            0.02,
        );
        r.put(
            "R8",
            name,
            "mono_sum_lu",
            Some(mono_sum(&mut wet(), &pink_stim(rate, 6.0))),
            Better::Higher,
            Some(-3.5),
            0.05,
        );

        // R10: a source on the left starts on the left.
        let left = clarity::left_impulse_response(&mut wet(), rate as usize / 2, 0.5);
        r.put(
            "R10",
            name,
            "early_side_db",
            clarity::early_side_db(&left, 50.0),
            Better::Higher,
            Some(3.0),
            0.2,
        );

        // R12: Pre-delay moves the start by what it says.
        if let Some(id) = pid(&wet(), "Pre-delay") {
            let mut mk = || -> Box<dyn Dut> { Box::new(wet()) };
            let e = knob::predelay_errors(&mut mk, id, &[0.0, 20.0, 80.0]);
            let worst = e.iter().map(|(_, f)| f.abs()).reduce(f64::max);
            r.put(
                "R12",
                name,
                "predelay_error_frames",
                worst,
                Better::Near(0.0),
                Some(1.0),
                0.1,
            );
        }

        // R9: a held note isn't smeared.
        r.put(
            "R9",
            name,
            "wobble_cents",
            wobble::measure(&mut wet(), 5.0),
            Better::Lower,
            Some(3.0),
            0.1,
        );

        // R7: loudness across the Mix knob.
        if let Some(id) = pid(&wet(), "Mix") {
            let mut mk = || -> Box<dyn Dut> { Box::new(make(s, rate)) };
            let pts = mixlaw::measure(&mut mk, id, &mixlaw::MIXES);
            let (dip, bump) = mixlaw::dip_and_bump(&pts);
            r.put(
                "R7",
                name,
                "mix_dip_lu",
                Some(dip),
                Better::Higher,
                Some(-1.0),
                0.05,
            );
            r.put(
                "R7",
                name,
                "mix_bump_lu",
                Some(bump),
                Better::Lower,
                Some(1.5),
                0.05,
            );
        }
    }

    // Per mode and preset: the wet level and zipper noise.
    for s in defaults.iter().chain(&presets) {
        let name = s.name.as_str();
        let mut d = make(s, rate);
        set(&mut d, &[("Mix", 100.0)]);
        let v = lu_change(&mut d, &pink_stim(rate, 6.0));
        r.put(
            "R6",
            name,
            "wet_lu",
            Some(v),
            Better::Near(0.0),
            Some(1.0),
            0.05,
        );
        r.put(
            "R11",
            name,
            "zipper_worst_db",
            worst_zipper(s, rate),
            Better::Lower,
            Some(3.0),
            0.5,
        );
    }
}

// ---------------------------------------------------------------------------
// Output and the ratchet.

fn num(v: f64) -> String {
    let r = (v * 10_000.0).round() / 10_000.0;
    if r == 0.0 {
        "0".into()
    } else {
        format!("{r}")
    }
}

/// The report as a table: target id, what, the number, its target, and
/// whether it is met.
pub fn to_text(rep: &Report) -> String {
    let mut s = format!("== {} ({} Hz)\n", rep.class.name(), rep.rate);
    for row in &rep.rows {
        let target = match (row.better, row.target) {
            (Better::Lower, Some(t)) => format!("<= {}", num(t)),
            (Better::Higher, Some(t)) => format!(">= {}", num(t)),
            (Better::Near(c), Some(t)) => format!("{} +- {}", num(c), num(t)),
            _ => "-".into(),
        };
        let met = match row.met() {
            Some(true) => "met",
            Some(false) => "NOT MET",
            None => "",
        };
        s.push_str(&format!(
            "{:<5} {:<64} {:>10}  {:<12} {}\n",
            row.target_id,
            row.key(),
            num(row.value),
            target,
            met
        ));
    }
    for n in &rep.notes {
        s.push_str(&format!("note: {n}\n"));
    }
    let (met, all) = rep
        .rows
        .iter()
        .filter_map(Row::met)
        .fold((0, 0), |(m, a), x| (m + usize::from(x), a + 1));
    s.push_str(&format!("{met} of {all} targets met\n"));
    s
}

/// The reports as JSON.
pub fn to_json(reports: &[Report]) -> String {
    let esc = |x: &str| x.replace('\\', "\\\\").replace('"', "\\\"");
    let mut s = String::from("[\n");
    for (i, rep) in reports.iter().enumerate() {
        s.push_str(&format!(
            "  {{\"class\": \"{}\", \"rate\": {}, \"rows\": [\n",
            rep.class.name(),
            rep.rate
        ));
        for (j, row) in rep.rows.iter().enumerate() {
            let better = match row.better {
                Better::Lower => "\"lower\"".to_string(),
                Better::Higher => "\"higher\"".to_string(),
                Better::Near(c) => format!("{{\"near\": {}}}", num(c)),
                Better::Info => "\"info\"".to_string(),
            };
            s.push_str(&format!(
                "    {{\"id\": \"{}\", \"key\": \"{}\", \"value\": {}, \"better\": {better}, \"target\": {}, \"met\": {}}}{}\n",
                row.target_id,
                esc(&row.key()),
                num(row.value),
                row.target.map_or("null".into(), num),
                row.met().map_or("null".into(), |m| m.to_string()),
                if j + 1 < rep.rows.len() { "," } else { "" }
            ));
        }
        s.push_str("  ], \"notes\": [");
        s.push_str(
            &rep.notes
                .iter()
                .map(|n| format!("\"{}\"", esc(n)))
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push_str(&format!(
            "]}}{}\n",
            if i + 1 < reports.len() { "," } else { "" }
        ));
    }
    s.push_str("]\n");
    s
}

/// The ratchet file for `reports`: one `key<TAB>value` line per row that
/// has a better direction, sorted.
pub fn ratchet_text(reports: &[Report]) -> String {
    let mut lines: Vec<String> = reports
        .iter()
        .flat_map(|r| &r.rows)
        .filter(|row| row.better != Better::Info)
        .map(|row| format!("{}\t{}", row.key(), num(row.value)))
        .collect();
    lines.sort();
    let mut s = String::from(
        "# The FX quality ratchet: every number may stay or get better, never worse.\n\
         # Written by `MPFREE_QUALITY_BLESS=1 cargo test --release -p mpfree-fxlab --test quality`;\n\
         # see docs/dev/design/fx-quality.md. key<TAB>value\n",
    );
    for l in lines {
        s.push_str(&l);
        s.push('\n');
    }
    s
}

/// Reads a ratchet file.
pub fn ratchet_read(text: &str) -> Result<Vec<(String, f64)>, String> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (k, v) = l
                .split_once('\t')
                .ok_or_else(|| format!("no tab in {l:?}"))?;
            let v: f64 = v
                .trim()
                .parse()
                .map_err(|_| format!("not a number in {l:?}"))?;
            Ok((k.to_string(), v))
        })
        .collect()
}

/// Everything that fails the ratchet: rows worse than recorded, rows not
/// recorded yet, and recorded rows no longer measured (only for the
/// classes in `reports`).
pub fn ratchet_check(reports: &[Report], recorded: &[(String, f64)]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in reports.iter().flat_map(|r| &r.rows) {
        if row.better == Better::Info {
            continue;
        }
        let key = row.key();
        match recorded.iter().find(|(k, _)| *k == key) {
            Some((_, old)) => {
                if row.worse_than(*old) {
                    problems.push(format!(
                        "worse: {key} is {} (was {}, {:?} is better, tolerance {})",
                        num(row.value),
                        num(*old),
                        row.better,
                        row.tol
                    ));
                }
            }
            None => problems.push(format!("not recorded: {key} = {}", num(row.value))),
        }
        seen.insert(key);
    }
    for (k, _) in recorded {
        let class = k.split('/').next().unwrap_or("");
        let covered = reports.iter().any(|r| {
            let c = r.class.name();
            class == c || class.starts_with(&format!("{c}["))
        });
        if covered && !seen.contains(k) {
            problems.push(format!("no longer measured: {k}"));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(value: f64, better: Better) -> Row {
        Row {
            target_id: "X1",
            subject: "delay/digital".into(),
            metric: "m".into(),
            value,
            better,
            target: Some(1.0),
            tol: 0.1,
        }
    }

    #[test]
    fn rows_compare_the_right_way() {
        assert!(row(1.2, Better::Lower).worse_than(1.0));
        assert!(!row(1.05, Better::Lower).worse_than(1.0));
        assert!(row(0.8, Better::Higher).worse_than(1.0));
        assert!(row(-1.5, Better::Near(0.0)).worse_than(1.0));
        assert!(!row(-0.5, Better::Near(0.0)).worse_than(1.0));
        assert_eq!(row(0.5, Better::Lower).met(), Some(true));
        assert_eq!(row(0.5, Better::Higher).met(), Some(false));
        assert_eq!(row(0.5, Better::Info).met(), None);
    }

    #[test]
    fn the_ratchet_round_trips_and_catches_each_problem() {
        let rep = Report {
            class: Class::Delay,
            rate: 44_100,
            rows: vec![row(0.5, Better::Lower)],
            notes: Vec::new(),
        };
        let text = ratchet_text(std::slice::from_ref(&rep));
        let rec = ratchet_read(&text).unwrap();
        assert_eq!(rec, vec![("delay/digital/m".to_string(), 0.5)]);
        assert!(ratchet_check(std::slice::from_ref(&rep), &rec).is_empty());
        let mut worse = rep.clone();
        worse.rows[0].value = 0.7;
        assert_eq!(ratchet_check(&[worse], &rec).len(), 1);
        let gone = vec![("delay/tape/m".to_string(), 1.0)];
        let p = ratchet_check(std::slice::from_ref(&rep), &gone);
        assert_eq!(p.len(), 2, "{p:?}");
        // A recorded row of another class is not this run's business.
        let other = vec![
            ("delay/digital/m".to_string(), 0.5),
            ("eq/x".to_string(), 1.0),
        ];
        assert!(ratchet_check(&[rep], &other).is_empty());
    }
}
