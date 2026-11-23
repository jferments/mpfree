//! The pad filter against what its knobs say (the filter gap plan's step
//! F1; `docs/dev/design/fx-filter.md`, "The pad filter"). S5 holds each
//! type to its own analog prototype, so a change of design moves it with
//! the design; these rows hold the pad's filter to fixed targets instead,
//! the Filter effect's (`fx-quality.md`, F1 to F12) where they carry over:
//!
//! - **PF1, the cutoff:** the -3 dB point against Cutoff at Resonance 0,
//!   low-pass and high-pass at every slope (1, 2, 3, 4, 6 and 8 poles; 2
//!   and 4 since F1, the rest since F4), 100 Hz to 16 kHz, from the pad's
//!   response to a click against a dry pad's. Within 1 %. (The
//!   Ladder is left out: by the usual convention its Cutoff is where it
//!   rings, which DSP quality's Y2b holds.)
//! - **PF2, the shape:** the same responses at a 1 kHz cutoff against the
//!   textbook digital Butterworth of the same order (the bilinear
//!   transform's, pre-warped at the cutoff), 30 Hz to 8 kHz wherever it is
//!   above -30 dB. Within 0.1 dB.
//! - **PF3, the slope:** dB an octave well away from the cutoff (2 to 4
//!   kHz above a 500 Hz low-pass, 1 kHz down to 500 Hz below a 4 kHz
//!   high-pass; at 6 and 8 poles 500 Hz to 1 kHz above a 250 Hz low-pass
//!   and 2 kHz down to 1 kHz below a 4 kHz high-pass, where their
//!   stopbands are still well above the click's rounding). Within 1 dB of
//!   6 dB a pole.
//! - **PF4, the passband by Resonance:** a 100 Hz sine under a 1 kHz
//!   low-pass at 1, 2, 4 and 8 poles, at Resonance 0, 0.5 and 1. Within
//!   ±0.5 dB, as the effect's F2. The Ladder's are reported: its bass
//!   drops as Resonance rises by design, as the effect's does (-4.7 dB at
//!   full).
//! - **PF5, a loud sound at full Resonance:** pink at -18 dBFS RMS through
//!   a low-pass (1, 2, 4 and 8 poles, the Ladder), a band-pass (2 and 8
//!   poles) and the Peak that the filter envelope sweeps from 100 Hz to 10
//!   kHz in four seconds, at Resonance 0.75 and 1: the loudest sample, as
//!   if the pad passed the sound at unity. At most -1 dBFS at Resonance 1,
//!   as F3. The Peak's are reported: it passes the whole sound (its own
//!   loudest sample is -5.5 dBFS) and adds its ring, held as a low-pass's
//!   is.
//! - **PF6, a quiet sound at full Resonance:** a -40 dBFS sine at a 1 kHz
//!   cutoff, Resonance 1, low-pass at every slope, high-pass at 8 poles,
//!   band-pass at 2, 4 and 8 and the Peak: how far it rings up. At least
//!   28 dB, as F4. The Ladder sings on its own at Resonance 1, so its row
//!   is at 0.9, without a target.
//! - **PF7, a driven, resonant Ladder's aliases:** a sine at 0 dBFS
//!   through the Ladder at Drive 12 dB, below a cutoff where its loop
//!   works hard (2.5 kHz at a 5 kHz cutoff and Resonance 0.9, 5 kHz at 8
//!   kHz and 0.7, 7 kHz at 10 kHz and 0.9): the loudest bin that is no
//!   harmonic of the tone. At most -65 dB, as S6 (whose Ladder is open, so
//!   its loop hardly works). The drive's curve runs at twice the rate; the
//!   loop's own curve does not, which the 10 kHz row shows.
//! - **PF8, a loud sound stays centred:** a centred sawtooth at full scale
//!   and about 1.8 kHz (a whole number of frames a cycle, so its cycles
//!   average to 0) under a 2 kHz cutoff, every state-variable type at 2
//!   and 4 poles, the low-pass at 1 and 8, the high-pass and band-pass at
//!   8 and the Peak, at Resonance 0.3 and 1: the output's DC over whole
//!   cycles once settled, as if the pad passed the sound at unity. At
//!   most -60 dBFS, as the delay's D4. A filter that bends its states
//!   inside the loop makes DC of a loud, lopsided wave (the bounded
//!   filter did: -18 dBFS at Resonance 0.3); the resonance guard keeps the
//!   filter linear. The Ladder's rows, at Drive 0 and 12 dB, have no target
//!   yet: its drive's curve makes some DC, which step F6 rebuilds.
//!
//! - **PF9, the cutoff follows the key:** a 2-pole low-pass at 1 kHz on a
//!   keygroup (its root A3), Key track 1, 0.5 and -1, keys from two
//!   octaves down to two up, in 12-TET, just intonation and quarter tones
//!   (the filter gap plan's F3): the -3 dB point against 1 kHz moved by the
//!   key track times how far the key sounds from the sound's root in the
//!   kit's tuning, from the key's click against the same key's click on
//!   a dry keygroup. Within 1 %, as PF1.
//! - **PF10, the resonance at every slope, and the Peak** (the filter gap
//!   plan's F4): a -40 dBFS sine at a 1 kHz cutoff, Resonance 0.5, through
//!   the low-pass and high-pass at 1, 3, 4, 6 and 8 poles and the
//!   band-pass at 4, 6 and 8: its gain against the same type's 2-pole.
//!   Within 0.5 dB, as every slope peaks where the 2-pole does. The Peak at
//!   Resonance 0, 0.5 and 1: its boost at the cutoff (a -46 dBFS sine,
//!   under its guard's knee) against the 2-pole low-pass's ring there over
//!   its -3 dB (0 dB at Resonance 0, 35 dB at full), within 0.5 dB; and
//!   what it does to a sine at 100 Hz and at 10 kHz, at most 0.5 dB.
//!
//! DSP quality's measures (`crate::filters`, Y1a to Y2b) hold the pad's
//! filter to the curve its page draws, and cover two things the effect's
//! targets ask for here too: stepping under a fast envelope and LFO
//! against the same move every frame (Y1c, as F6) and the Ladder singing
//! in tune at a steady level (Y2b, as F9). The cost per voice is the
//! bench's sampler cases, as for S1 to S7.

use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::Pad;
use mpfree_tuning::{Builtin, Tuning};

use super::{floor, hit, kit, pad, play, put, sound, through, worst_alias_db};
use crate::fft::power_spectrum;
use crate::harmonics;
use crate::quality::{Better, Row};
use crate::signals::{coherent, db_to_amp, peak_of, pink, rms_of, sine};

/// The click responses' length: every Resonance 0 response here has died
/// away long before (the slowest, a 4-pole high-pass at 100 Hz, within
/// about 2,000 frames).
const IR: usize = 16_384;

/// A filter of `kind` at `cutoff_hz` and `resonance`, its envelope and
/// drive off.
fn filter(kind: FilterKind, poles: u8, cutoff_hz: f32, resonance: f32) -> Filter {
    Filter {
        kind,
        poles,
        cutoff_hz,
        resonance,
        env_octaves: 0.0,
        drive_db: 0.0,
    }
}

/// A pad's response to a click through `f` (None: dry), [`IR`] frames.
fn click_response(f: Option<Filter>, rate: u32) -> Vec<f64> {
    let mut click = vec![0.0; IR];
    click[32] = 0.5;
    through(&click, f, rate, IR)
}

/// `x`'s power at `hz` (any frequency, not only a bin's), by Goertzel's
/// recursion.
fn power_at(x: &[f64], hz: f64, rate: u32) -> f64 {
    let c = 2.0 * (2.0 * std::f64::consts::PI * hz / f64::from(rate)).cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s0 = v + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - c * s1 * s2
}

/// The gain of `wet` against `dry` at `hz`, dB.
fn gain_db(wet: &[f64], dry: &[f64], hz: f64, rate: u32) -> f64 {
    10.0 * (power_at(wet, hz, rate) / power_at(dry, hz, rate).max(1e-300))
        .max(1e-30)
        .log10()
}

/// Where `gain(hz)` crosses -3 dB near `cutoff`, falling (a low-pass) or
/// rising (a high-pass) with frequency, by bisection on a log scale.
fn minus_3db(gain: impl Fn(f64) -> f64, cutoff: f64, rising: bool, rate: u32) -> f64 {
    let half = -10.0 * 2.0f64.log10();
    let (mut lo, mut hi) = (cutoff / 8.0, (cutoff * 8.0).min(0.499 * f64::from(rate)));
    for _ in 0..50 {
        let mid = (lo * hi).sqrt();
        if (gain(mid) > half) != rising {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo * hi).sqrt()
}

/// The textbook digital Butterworth of `poles` (the bilinear transform's,
/// pre-warped at `cutoff`), dB: a low-pass, or a high-pass when `high`.
fn butterworth_db(poles: u8, cutoff: f64, hz: f64, high: bool, rate: u32) -> f64 {
    let w = |f: f64| (std::f64::consts::PI * f / f64::from(rate)).tan();
    let x = w(hz) / w(cutoff);
    let x = if high { 1.0 / x } else { x };
    -10.0 * (1.0 + x.powi(2 * i32::from(poles))).log10()
}

/// `kind` named as the rows name it.
fn kind_name(kind: FilterKind, poles: u8) -> String {
    let k = match kind {
        FilterKind::LowPass => "low-pass",
        FilterKind::HighPass => "high-pass",
        FilterKind::BandPass => "band-pass",
        FilterKind::Notch => "notch",
        FilterKind::Peak => return "peak".into(),
        FilterKind::Ladder => return "ladder".into(),
        FilterKind::Vowel => return "vowel".into(),
        FilterKind::Comb => return "comb".into(),
        FilterKind::Off => return "off".into(),
    };
    format!("{k} {poles}-pole")
}

/// Every pad filter row, in the order measured.
pub(super) fn rows(rows: &mut Vec<Row>, rate: u32) {
    cutoffs(rows, rate);
    passband(rows, rate);
    loud(rows, rate);
    quiet(rows, rate);
    driven(rows, rate);
    centred(rows, rate);
    keyed(rows, rate);
    slopes(rows, rate);
}

// ---------------------------------------------------------------------------
// PF1 to PF3: the cutoff, the shape, the slope.

/// The low-pass and high-pass at every slope: 2 and 4 poles since F1,
/// the rest since F4.
const CLEAN: [(FilterKind, u8); 12] = [
    (FilterKind::LowPass, 2),
    (FilterKind::LowPass, 4),
    (FilterKind::HighPass, 2),
    (FilterKind::HighPass, 4),
    (FilterKind::LowPass, 1),
    (FilterKind::LowPass, 3),
    (FilterKind::LowPass, 6),
    (FilterKind::LowPass, 8),
    (FilterKind::HighPass, 1),
    (FilterKind::HighPass, 3),
    (FilterKind::HighPass, 6),
    (FilterKind::HighPass, 8),
];

fn cutoffs(rows: &mut Vec<Row>, rate: u32) {
    let dry = click_response(None, rate);
    for (kind, poles) in CLEAN {
        let high = kind == FilterKind::HighPass;
        let name = kind_name(kind, poles);
        let mut worst = 0.0f64;
        for hz in [100.0, 1_000.0, 5_000.0, 10_000.0, 16_000.0] {
            let wet = click_response(Some(filter(kind, poles, hz as f32, 0.0)), rate);
            let found = minus_3db(|f| gain_db(&wet, &dry, f, rate), hz, high, rate);
            worst = worst.max((found / hz - 1.0).abs() * 100.0);
        }
        put(
            rows,
            "PF1",
            format!("pad filter[{name}, resonance 0]"),
            "cutoff_error_pct",
            worst,
            Better::Lower,
            Some(1.0),
            0.05,
        );

        let wet = click_response(Some(filter(kind, poles, 1_000.0, 0.0)), rate);
        let mut error = 0.0f64;
        let mut hz = 30.0;
        while hz <= 8_000.0 {
            let target = butterworth_db(poles, 1_000.0, hz, high, rate);
            if target > -30.0 {
                error = error.max((gain_db(&wet, &dry, hz, rate) - target).abs());
            }
            hz *= 2.0f64.powf(1.0 / 24.0);
        }
        put(
            rows,
            "PF2",
            format!("pad filter[{name}, 1 kHz]"),
            "butterworth_error_db",
            error,
            Better::Lower,
            Some(0.1),
            0.05,
        );

        // A 6 or 8-pole slope nearer its cutoff (2 to 4 times it), where
        // its stopband is still far above the click's rounding.
        let (cutoff, near, far) = match (high, poles >= 6) {
            (true, false) => (4_000.0, 1_000.0, 500.0),
            (true, true) => (4_000.0, 2_000.0, 1_000.0),
            (false, false) => (500.0, 2_000.0, 4_000.0),
            (false, true) => (250.0, 500.0, 1_000.0),
        };
        let wet = click_response(Some(filter(kind, poles, cutoff as f32, 0.0)), rate);
        let slope = gain_db(&wet, &dry, near, rate) - gain_db(&wet, &dry, far, rate);
        put(
            rows,
            "PF3",
            format!("pad filter[{name}, {} Hz]", cutoff),
            "slope_db_per_octave",
            slope,
            Better::Near(6.0 * f64::from(poles)),
            Some(1.0),
            0.1,
        );
    }
}

// ---------------------------------------------------------------------------
// PF4 and PF6: steady sines.

/// The gain (dB) of a pad through `f` against a dry pad on a sine at about
/// `hz` (on a bin) and `level_db` dBFS peak, once it has settled.
fn sine_gain_db(f: Filter, hz: f64, level_db: f64, rate: u32) -> f64 {
    let settle = rate as usize / 2;
    let n = harmonics::N;
    let (bin, hz) = coherent(hz, n, rate);
    let x = sine(hz, db_to_amp(level_db), settle + n + 64, rate);
    let power = |filter: Option<Filter>| {
        let y = through(&x, filter, rate, settle + n);
        power_spectrum(&y[settle..settle + n])[bin]
    };
    10.0 * (power(Some(f)) / power(None).max(1e-300))
        .max(1e-30)
        .log10()
}

fn passband(rows: &mut Vec<Row>, rate: u32) {
    for (kind, poles) in [
        (FilterKind::LowPass, 2u8),
        (FilterKind::LowPass, 4),
        (FilterKind::Ladder, 4),
        (FilterKind::LowPass, 1),
        (FilterKind::LowPass, 8),
    ] {
        let ladder = kind == FilterKind::Ladder;
        for res in [0.0f32, 0.5, 1.0] {
            put(
                rows,
                "PF4",
                format!(
                    "pad filter[{}, 1 kHz, resonance {res}]",
                    kind_name(kind, poles)
                ),
                "passband_100hz_db",
                sine_gain_db(filter(kind, poles, 1_000.0, res), 100.0, -24.0, rate),
                if ladder {
                    Better::Info
                } else {
                    Better::Near(0.0)
                },
                (!ladder).then_some(0.5),
                0.05,
            );
        }
    }
}

fn quiet(rows: &mut Vec<Row>, rate: u32) {
    for (kind, poles, res, target) in [
        (FilterKind::LowPass, 2u8, 1.0f32, Some(28.0)),
        (FilterKind::LowPass, 4, 1.0, Some(28.0)),
        (FilterKind::BandPass, 2, 1.0, Some(28.0)),
        (FilterKind::Ladder, 4, 0.9, None),
        (FilterKind::LowPass, 1, 1.0, Some(28.0)),
        (FilterKind::LowPass, 3, 1.0, Some(28.0)),
        (FilterKind::LowPass, 6, 1.0, Some(28.0)),
        (FilterKind::LowPass, 8, 1.0, Some(28.0)),
        (FilterKind::HighPass, 8, 1.0, Some(28.0)),
        (FilterKind::BandPass, 4, 1.0, Some(28.0)),
        (FilterKind::BandPass, 8, 1.0, Some(28.0)),
        (FilterKind::Peak, 2, 1.0, Some(28.0)),
    ] {
        put(
            rows,
            "PF6",
            format!(
                "pad filter[{}, 1 kHz, resonance {res}, -40 dBFS sine]",
                kind_name(kind, poles)
            ),
            "ring_up_db",
            sine_gain_db(filter(kind, poles, 1_000.0, res), 1_000.0, -40.0, rate),
            Better::Higher,
            target,
            0.5,
        );
    }
}

// ---------------------------------------------------------------------------
// PF5: a loud sound at full Resonance.

fn loud(rows: &mut Vec<Row>, rate: u32) {
    let n = 4 * rate as usize;
    let x = pink(n, db_to_amp(-18.0), 11);
    let dry = through(&x, None, rate, n);
    // As if the pad passed the sound at unity.
    let unity = rms_of(&x) / rms_of(&dry).max(1e-30);
    let swept = |f: Filter| {
        let s = sound(&x, rate);
        let p = pad(&s, |p: &mut Pad| {
            p.filter = Filter {
                env_octaves: 100.0f32.log2(),
                ..f
            };
            p.filter_env = Ahdsr {
                attack_ms: 4_000.0,
                hold_ms: 0.0,
                decay_ms: 0.0,
                sustain: 1.0,
                release_ms: 5.0,
            };
        });
        let y = play(&kit(vec![(s, p)]), rate, &[(0, hit(0))], n);
        floor(
            20.0 * (unity * peak_of(&y.left).max(peak_of(&y.right)))
                .max(1e-15)
                .log10(),
        )
    };
    for (kind, poles) in [
        (FilterKind::LowPass, 2u8),
        (FilterKind::LowPass, 4),
        (FilterKind::BandPass, 2),
        (FilterKind::Ladder, 4),
        (FilterKind::LowPass, 1),
        (FilterKind::LowPass, 8),
        (FilterKind::BandPass, 8),
        (FilterKind::Peak, 2),
    ] {
        for res in [0.75f32, 1.0] {
            put(
                rows,
                "PF5",
                format!(
                    "pad filter[{}, swept 100 Hz to 10 kHz, resonance {res}, pink at -18 dBFS]",
                    kind_name(kind, poles)
                ),
                "peak_dbfs",
                swept(filter(kind, poles, 100.0, res)),
                Better::Lower,
                // A boost passes the whole sound and adds its held ring
                // to it, so the Peak's are reported.
                (res == 1.0 && kind != FilterKind::Peak).then_some(-1.0),
                0.3,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// PF7: a driven, resonant Ladder's aliases.

fn driven(rows: &mut Vec<Row>, rate: u32) {
    let settle = 4096;
    let n = harmonics::N;
    for (out_hz, cutoff_hz, res) in [
        (2_500.0, 5_000.0f32, 0.9f32),
        (5_000.0, 8_000.0, 0.7),
        (7_000.0, 10_000.0, 0.9),
    ] {
        let (bin, f) = coherent(out_hz, n, rate);
        let x = sine(f, 1.0, settle + n + 64, rate);
        let ladder = Filter {
            drive_db: 12.0,
            ..filter(FilterKind::Ladder, 4, cutoff_hz, res)
        };
        let y = through(&x, Some(ladder), rate, settle + n);
        put(
            rows,
            "PF7",
            format!(
                "pad filter[ladder at {} kHz, resonance {res}, 12 dB, {} kHz at 0 dBFS]",
                cutoff_hz / 1000.0,
                out_hz / 1000.0
            ),
            "alias_worst_db",
            worst_alias_db(&y[settle..settle + n], bin, rate),
            Better::Lower,
            Some(-65.0),
            0.5,
        );
    }
}

// ---------------------------------------------------------------------------
// PF8: a loud sound stays centred.

/// A sawtooth at full scale and about `hz`, `frames` long: a whole number
/// of frames a cycle, and centred, so that any whole number of its cycles
/// averages to 0. Also its cycle, in frames.
fn centred_saw(hz: f64, frames: usize, rate: u32) -> (Vec<f64>, usize) {
    let cycle = (f64::from(rate) / hz).round() as usize;
    let x = (0..frames)
        .map(|n| (2 * (n % cycle) + 1) as f64 / cycle as f64 - 1.0)
        .collect();
    (x, cycle)
}

fn centred(rows: &mut Vec<Row>, rate: u32) {
    let settle = rate as usize / 2;
    let cycle = (f64::from(rate) / 1_800.0).round() as usize;
    // Half a second of whole cycles after it settles.
    let n = rate as usize / 2 / cycle * cycle;
    let (x, _) = centred_saw(1_800.0, settle + n, rate);
    let dry = through(&x, None, rate, settle + n);
    // As if the pad passed the sound at unity.
    let unity = rms_of(&x[settle..]) / rms_of(&dry[settle..]).max(1e-30);
    let dc_dbfs = |f: Filter| {
        let y = through(&x, Some(f), rate, settle + n);
        let dc = y[settle..].iter().sum::<f64>() / n as f64;
        floor(20.0 * (unity * dc.abs()).max(1e-15).log10())
    };
    for (kind, poles) in [
        (FilterKind::LowPass, 2u8),
        (FilterKind::LowPass, 4),
        (FilterKind::HighPass, 2),
        (FilterKind::HighPass, 4),
        (FilterKind::BandPass, 2),
        (FilterKind::BandPass, 4),
        (FilterKind::Notch, 2),
        (FilterKind::Notch, 4),
        (FilterKind::LowPass, 1),
        (FilterKind::LowPass, 8),
        (FilterKind::HighPass, 8),
        (FilterKind::BandPass, 8),
        (FilterKind::Peak, 2),
    ] {
        for res in [0.3f32, 1.0] {
            put(
                rows,
                "PF8",
                format!(
                    "pad filter[{}, 2 kHz, resonance {res}, 1.8 kHz sawtooth at 0 dBFS]",
                    kind_name(kind, poles)
                ),
                "dc_dbfs",
                dc_dbfs(filter(kind, poles, 2_000.0, res)),
                Better::Lower,
                Some(-60.0),
                1.0,
            );
        }
    }
    for drive_db in [0.0f32, 12.0] {
        put(
            rows,
            "PF8",
            format!(
                "pad filter[ladder, 2 kHz, resonance 0.5, {drive_db} dB, 1.8 kHz sawtooth at 0 dBFS]"
            ),
            "dc_dbfs",
            dc_dbfs(Filter {
                drive_db,
                ..filter(FilterKind::Ladder, 4, 2_000.0, 0.5)
            }),
            Better::Lower,
            None,
            1.0,
        );
    }
}

// ---------------------------------------------------------------------------
// PF9: the cutoff follows the key.

/// The keygroup's root: A3, where its sound plays as recorded.
const ROOT: u8 = 57;

/// A click on key `note` of a keygroup over the whole keyboard (its root
/// [`ROOT`]) in `tuning`, through `f` at Key track `track` (None: dry),
/// the left channel's first `frames` frames.
fn keyed_click(
    f: Option<Filter>,
    track: f32,
    note: u8,
    tuning: &Tuning,
    rate: u32,
    frames: usize,
) -> Vec<f64> {
    let mut click = vec![0.0; IR];
    click[32] = 0.5;
    let s = sound(&click, rate);
    let p = pad(&s, |p| {
        if let Some(f) = f {
            p.filter = f;
        }
        p.key_track = track;
    });
    let mut project = kit(vec![(s, p)]);
    project.spread_to_keys(0, ROOT).expect("a lab keygroup");
    for s in project.sources() {
        s.make_octaves();
    }
    let mut e = Engine::new(rate, 8);
    e.set_view(EngineView::build(&project, rate));
    e.set_tuning(0, tuning);
    e.apply(Event::Key {
        note,
        velocity: 127,
    });
    let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    l.into_iter().map(f64::from).collect()
}

/// A PF9 row: its name, the kit's tuning, and the keys played, each with
/// its Key track.
type KeyRow<'a> = (&'a str, &'a Tuning, &'a [(f32, u8)]);

fn keyed(rows: &mut Vec<Row>, rate: u32) {
    // Two octaves down reads the click at a quarter of its speed.
    let frames = IR * 4;
    let equal = Tuning::standard();
    let just = Builtin::Just.tuning(0, 440.0);
    let quarter = Tuning::equal(24, 440.0);
    let cases: [KeyRow; 4] = [
        (
            "Key track 1, 12-TET",
            &equal,
            &[
                (1.0, ROOT - 24),
                (1.0, ROOT - 12),
                (1.0, ROOT - 5),
                (1.0, ROOT + 7),
                (1.0, ROOT + 12),
            ],
        ),
        (
            "Key track 0.5 and -1, 12-TET",
            &equal,
            &[
                (0.5, ROOT - 24),
                (0.5, ROOT + 24),
                (-1.0, ROOT - 12),
                (-1.0, ROOT + 12),
            ],
        ),
        (
            "Key track 1, just",
            &just,
            &[(1.0, 60), (1.0, 64), (1.0, 67)],
        ),
        (
            "Key track 1, 24-TET",
            &quarter,
            &[(1.0, 45), (1.0, 50), (1.0, 58), (1.0, 69)],
        ),
    ];
    let low_pass = filter(FilterKind::LowPass, 2, 1_000.0, 0.0);
    for (name, tuning, keys) in cases {
        let mut worst = 0.0f64;
        for &(track, note) in keys {
            // Where the key sounds from the sound's root, in the tuning.
            let cents = f64::from(i32::from(note) - i32::from(ROOT)) * 100.0
                + f64::from(tuning.offset_cents(f32::from(note)));
            let want = 1_000.0 * (f64::from(track) * cents / 1200.0).exp2();
            let dry = keyed_click(None, track, note, tuning, rate, frames);
            let wet = keyed_click(Some(low_pass), track, note, tuning, rate, frames);
            let found = minus_3db(|f| gain_db(&wet, &dry, f, rate), want, false, rate);
            worst = worst.max((found / want - 1.0).abs() * 100.0);
        }
        put(
            rows,
            "PF9",
            format!("pad filter[low-pass 2-pole, {name}]"),
            "cutoff_error_pct",
            worst,
            Better::Lower,
            Some(1.0),
            0.05,
        );
    }
}

// ---------------------------------------------------------------------------
// PF10: the resonance at every slope, and the Peak.

fn slopes(rows: &mut Vec<Row>, rate: u32) {
    // A quiet sine on a 1 kHz cutoff (under the guard's knee) at
    // Resonance 0.5: each slope's gain there against the 2-pole's.
    let at_cutoff =
        |kind, poles| sine_gain_db(filter(kind, poles, 1_000.0, 0.5), 1_000.0, -40.0, rate);
    for (kind, slopes) in [
        (FilterKind::LowPass, &[1u8, 3, 4, 6, 8][..]),
        (FilterKind::HighPass, &[1, 3, 4, 6, 8]),
        (FilterKind::BandPass, &[4, 6, 8]),
    ] {
        let two = at_cutoff(kind, 2);
        for &poles in slopes {
            put(
                rows,
                "PF10",
                format!(
                    "pad filter[{}, 1 kHz, resonance 0.5, -40 dBFS sine at the cutoff]",
                    kind_name(kind, poles)
                ),
                "against_2_pole_db",
                at_cutoff(kind, poles) - two,
                Better::Near(0.0),
                Some(0.5),
                0.05,
            );
        }
    }
    // The Peak: its boost at the cutoff is the 2-pole low-pass's ring
    // over its -3 dB (0 dB at Resonance 0, 35 dB at full), and away from
    // the cutoff it leaves the sound alone. The sine is under the boost's
    // own knee, √2 under the low-pass's.
    for res in [0.0f32, 0.5, 1.0] {
        let peak = filter(FilterKind::Peak, 2, 1_000.0, res);
        let lp = sine_gain_db(
            filter(FilterKind::LowPass, 2, 1_000.0, res),
            1_000.0,
            -46.0,
            rate,
        );
        put(
            rows,
            "PF10",
            format!("pad filter[peak, 1 kHz, resonance {res}, -46 dBFS sine at the cutoff]"),
            "boost_against_2_pole_db",
            sine_gain_db(peak, 1_000.0, -46.0, rate) - (lp + 10.0 * 2.0f64.log10()),
            Better::Near(0.0),
            Some(0.5),
            0.05,
        );
        let away = [100.0, 10_000.0]
            .into_iter()
            .map(|hz| sine_gain_db(peak, hz, -24.0, rate).abs())
            .fold(0.0f64, f64::max);
        put(
            rows,
            "PF10",
            format!("pad filter[peak, 1 kHz, resonance {res}, 100 Hz and 10 kHz]"),
            "away_db",
            away,
            Better::Lower,
            Some(0.5),
            0.05,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_centred_sawtooth_averages_to_zero_over_its_cycles() {
        let (x, cycle) = centred_saw(1_800.0, 10_000, 44_100);
        assert_eq!(cycle, 25);
        let n = 10_000 / cycle * cycle;
        let mean = x[..n].iter().sum::<f64>() / n as f64;
        assert!(mean.abs() < 1e-12, "{mean}");
        assert!(x.iter().all(|v| v.abs() < 1.0));
        assert!(x.iter().cloned().fold(0.0f64, f64::max) > 0.95);
    }

    #[test]
    fn the_butterworth_is_3_db_down_at_its_cutoff_and_falls_6_db_a_pole() {
        for poles in [2u8, 4] {
            assert!((butterworth_db(poles, 1_000.0, 1_000.0, false, 44_100) + 3.0103).abs() < 1e-3);
            assert!((butterworth_db(poles, 1_000.0, 1_000.0, true, 44_100) + 3.0103).abs() < 1e-3);
            let slope = butterworth_db(poles, 100.0, 400.0, false, 44_100)
                - butterworth_db(poles, 100.0, 800.0, false, 44_100);
            assert!((slope - 6.0 * f64::from(poles)).abs() < 0.2, "{slope}");
        }
        // Flat well inside the passband.
        assert!(butterworth_db(4, 1_000.0, 250.0, false, 44_100) > -0.01);
    }

    #[test]
    fn goertzel_finds_a_one_pole_s_gain_and_the_crossing() {
        // y = a x + (1 - a) y: a one-pole low-pass, its exact gain known.
        let rate = 44_100;
        let a = 0.1f64;
        let mut dry = vec![0.0; 8_192];
        dry[0] = 1.0;
        let mut wet = vec![0.0; 8_192];
        let mut y = 0.0;
        for (i, w) in wet.iter_mut().enumerate() {
            y = a * dry[i] + (1.0 - a) * y;
            *w = y;
        }
        let exact = |hz: f64| {
            let w = 2.0 * std::f64::consts::PI * hz / f64::from(rate);
            let (re, im) = (1.0 - (1.0 - a) * w.cos(), (1.0 - a) * w.sin());
            -10.0 * ((re * re + im * im) / (a * a)).log10()
        };
        for hz in [50.0, 700.0, 3_000.0] {
            assert!((gain_db(&wet, &dry, hz, rate) - exact(hz)).abs() < 1e-6);
        }
        let found = minus_3db(|f| gain_db(&wet, &dry, f, rate), 700.0, false, rate);
        assert!((exact(found) + 3.0103).abs() < 1e-3, "{found}");
    }
}
