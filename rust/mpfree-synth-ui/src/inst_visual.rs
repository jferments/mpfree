//! The drawn visuals of Keys, Drawbar, Squelch and Analog, worked out from
//! their values with the engines' own formulas ([`mpfree_synth::keys`],
//! [`mpfree_synth::drawbar`], [`mpfree_synth::squelch`],
//! [`mpfree_synth::analog`]), so they follow a knob the moment it turns.
//!
//! - Keys: the pickup's curve with the swing of a hit across it, the
//!   hammer's contact, how long notes ring across the keyboard, and the
//!   tremolo.
//! - Drawbar: the nine drawbars, the wave they add up to, the percussion's
//!   fall and the rotors' speeds after a switch.
//! - Squelch: one cycle of the wave, the filter's response at rest and at
//!   the envelope's peak, and the cutoff's sweep for a plain and an
//!   accented note.
//! - Analog: both oscillators, the filter's response, the two envelopes,
//!   the LFO with its delay, and each voice card's own tuning at the Age
//!   set.
//!
//! Points are in a unit box, as in [`crate::visual`]: x from 0 to 1 left to
//! right, y from 0 (bottom) to 1 (top).

use crate::kit_visual as kv;
use crate::visual::{freq_x, Values, DB_BOTTOM, DB_TOP, F_HI, F_LO, POINTS};
use crate::widgets::{Card, Trace, QLINK_COLORS, TEXT, TEXT_DIM, TEXT_FAINT};
use mpfree_synth::analog_params as an;
use mpfree_synth::drawbar_params as db;
use mpfree_synth::keys_params as kp;
use mpfree_synth::squelch_params as sq;
use mpfree_synth::{analog, drawbar, keys, squelch};
use mpfree_ui::Color;
use std::f32::consts::{PI, TAU};

type Pts = Vec<(f32, f32)>;

/// A gain on the filter cards' dB scale.
fn db_y(gain: f32) -> f32 {
    let db = 20.0 * gain.max(1e-6).log10();
    ((db - DB_BOTTOM) / (DB_TOP - DB_BOTTOM)).clamp(0.0, 1.0)
}

/// A response across the filter cards' frequencies (log, 20 Hz to 20 kHz).
fn response(gain: impl Fn(f32) -> f32) -> Pts {
    (0..=POINTS)
        .map(|i| {
            let x = i as f32 / POINTS as f32;
            (x, db_y(gain(F_LO * (F_HI / F_LO).powf(x))))
        })
        .collect()
}

/// A level in dB on a card that shows `range` dB below 0 (the top).
fn level_y(level: f32, range: f32) -> f32 {
    let db = 20.0 * level.max(1e-9).log10();
    (1.0 + db / range).clamp(0.0, 1.0)
}

fn pick<'a>(names: &[&'a str], v: f32) -> &'a str {
    names
        .get(v.max(0.0).round() as usize)
        .copied()
        .unwrap_or("?")
}

// ------------------------------------------------------------ Keys

/// The note Keys' cards are drawn for: middle C.
pub const KEYS_HZ: f32 = 261.63;
/// The velocity the Pickup card's swing is drawn for (0 to 1).
pub const KEYS_VEL: f32 = 100.0 / 127.0;
/// The Hammer card's time span in seconds: a little over the longest
/// contact on middle C (half its period, 1.9 ms).
pub const HAMMER_SECS: f32 = 0.0025;
/// The Decay card's range, in dB below the start.
pub const DECAY_DB: f32 = 60.0;

/// Whether Keys plays the Reed model.
pub fn keys_reed(v: &impl Values) -> bool {
    v.get(kp::MODEL) >= 0.5
}

/// The pickup's curve across the range the tine or reed can reach (the
/// tine's flux, the reed's capacitance), the stretch of it a hit at
/// velocity 100 on middle C swings across, and where the voice rests.
pub fn keys_pickup(v: &impl Values) -> (Pts, Pts, f32) {
    let reed = keys_reed(v);
    let (rest, scale) = keys::pickup_shape(reed, v.get(kp::VOICING), v.get(kp::BARK), 0.0);
    let swing = scale * keys::strike_amp(KEYS_VEL) * keys::register_swing(KEYS_HZ);
    let (lo, hi) = if reed { (-0.85, 0.85) } else { (-1.6, 1.6) };
    let curve = |u: f32| {
        if reed {
            keys::capacitance(u)
        } else {
            keys::flux(u)
        }
    };
    let (c_lo, c_hi) = if reed {
        (curve(lo), curve(hi))
    } else {
        (curve(hi), curve(0.0))
    };
    let y = |u: f32| 0.05 + 0.9 * ((curve(u) - c_lo) / (c_hi - c_lo)).clamp(0.0, 1.0);
    let x = |u: f32| (u - lo) / (hi - lo);
    let full = (0..=POINTS)
        .map(|i| {
            let u = lo + (hi - lo) * i as f32 / POINTS as f32;
            (x(u), y(u))
        })
        .collect();
    let (a, b) = ((rest - swing).max(lo), (rest + swing).min(hi));
    let hit = (0..=POINTS / 2)
        .map(|i| {
            let u = a + (b - a) * i as f32 / (POINTS / 2) as f32;
            (x(u), y(u))
        })
        .collect();
    (full, hit, x(rest.clamp(lo, hi)))
}

/// The hammer's force over its contact on middle C, for a soft hit
/// (velocity 40) and a full one: half sines as long as each contact and as
/// high as each strikes.
pub fn keys_hammer(v: &impl Values) -> [Pts; 2] {
    let hard = v.get(kp::HARDNESS);
    let pulse = |vel: f32| -> Pts {
        let len = keys::contact_secs(hard, vel, KEYS_HZ);
        let amp = keys::strike_amp(vel) / keys::strike_amp(1.0);
        let n = POINTS / 2;
        let mut pts: Pts = (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                (
                    (t * len / HAMMER_SECS).min(1.0),
                    (0.9 * amp * (PI * t).sin()).max(0.0),
                )
            })
            .collect();
        pts.push((1.0, 0.0));
        pts
    };
    [pulse(40.0 / 127.0), pulse(1.0)]
}

/// How long a full hit's contact lasts on middle C, in seconds.
pub fn keys_contact(v: &impl Values) -> f32 {
    keys::contact_secs(v.get(kp::HARDNESS), 1.0, KEYS_HZ)
}

/// The notes the Decay card draws: C2, C4 (middle C) and C6.
pub const DECAY_NOTES: [f32; 3] = [65.41, 261.63, 1046.5];

/// The Decay card's span in seconds: one and a half times middle C's ring.
pub fn keys_decay_secs(v: &impl Values) -> f32 {
    (keys::decay_for(v.get(kp::DECAY), KEYS_HZ) * 1.5).clamp(0.5, 40.0)
}

/// The level of a held C2, C4 and C6 over the Decay card's span, in dB
/// (the fundamental's parts: one for a tine, a long and a quick one for a
/// reed).
pub fn keys_decay(v: &impl Values) -> [Pts; 3] {
    let modes = if keys_reed(v) { keys::REED } else { keys::TINE };
    let secs = keys_decay_secs(v);
    let ring = |hz: f32| -> Pts {
        let t60 = keys::decay_for(v.get(kp::DECAY), hz).max(1e-3);
        let parts: Vec<_> = modes.iter().filter(|m| m.ratio == 1.0).collect();
        let start: f32 = parts.iter().map(|m| m.level).sum();
        (0..=POINTS)
            .map(|i| {
                let x = i as f32 / POINTS as f32;
                let t = x * secs;
                let level: f32 = parts
                    .iter()
                    .map(|m| m.level * 10f32.powf(-3.0 * t / (t60 * m.decay)))
                    .sum();
                (x, level_y(level / start, DECAY_DB))
            })
            .collect()
    };
    DECAY_NOTES.map(ring)
}

/// Two cycles of the tremolo's gains: left and right for Pan, one curve
/// for Amp. Empty when the tremolo is off.
pub fn keys_tremolo(v: &impl Values) -> Vec<Pts> {
    let depth = v.get(kp::TREMOLO);
    if depth <= 0.0 {
        return Vec::new();
    }
    let amp = v.get(kp::TREM_MODE) >= 0.5;
    let gains: Vec<(f32, (f32, f32))> = (0..=POINTS)
        .map(|i| {
            let x = i as f32 / POINTS as f32;
            (x, keys::tremolo_gains(depth, amp, (2.0 * x).fract()))
        })
        .collect();
    // Gains run from 0 to about 1.2 (Pan's louder side): drawn up to 1.25.
    let y = |g: f32| (0.05 + 0.9 * g / 1.25).clamp(0.0, 1.0);
    let left = gains.iter().map(|&(x, (l, _))| (x, y(l))).collect();
    if amp {
        vec![left]
    } else {
        vec![left, gains.iter().map(|&(x, (_, r))| (x, y(r))).collect()]
    }
}

/// The Tremolo card's caption: its mode and rate, or off.
pub fn keys_tremolo_caption(v: &impl Values) -> String {
    if v.get(kp::TREMOLO) <= 0.0 {
        return "off".into();
    }
    format!(
        "{} · {:.1} Hz",
        pick(kp::TREM_MODES, v.get(kp::TREM_MODE)).to_lowercase(),
        v.get(kp::TREM_RATE)
    )
}

// ------------------------------------------------------------ Drawbar

/// Each drawbar's position as a share of all the way out (8).
pub fn drawbar_bars(v: &impl Values) -> [f32; 9] {
    db::DRAWBARS.map(|(id, _, _)| (v.get(id) / 8.0).clamp(0.0, 1.0))
}

/// The registration as organists write it: nine digits from the 16'.
pub fn drawbar_registration(v: &impl Values) -> String {
    db::DRAWBARS
        .iter()
        .map(|&(id, _, _)| char::from(b'0' + v.get(id).round().clamp(0.0, 8.0) as u8))
        .collect()
}

/// A drawbar's family, for its colour: 0 below the note (16', 5 1/3'),
/// 1 the note and its octaves (8', 4', 2', 1'), 2 the other harmonics.
pub fn drawbar_family(bar: usize) -> usize {
    match bar {
        0 | 1 => 0,
        2 | 3 | 5 | 8 => 1,
        _ => 2,
    }
}

/// One cycle of the 16' (two of the 8'): the drawbars' sines added at
/// their levels (with percussion on, the 1' is silent, as on the organ),
/// scaled to fit. Flat when every drawbar is in.
pub fn drawbar_wave(v: &impl Values) -> Pts {
    let perc = v.get(db::PERC) >= 0.5;
    let parts: Vec<(f32, f32)> = db::DRAWBARS
        .iter()
        .enumerate()
        .filter(|&(i, _)| !(perc && i == 8))
        .map(|(_, &(id, _, semis))| {
            (
                2.0 * 2f32.powf(semis as f32 / 12.0),
                drawbar::bar_level(v.get(id)),
            )
        })
        .filter(|&(_, level)| level > 0.0)
        .collect();
    let sum = |t: f32| -> f32 {
        parts
            .iter()
            .map(|&(ratio, level)| level * (TAU * ratio * t).sin())
            .sum()
    };
    let raw: Vec<f32> = (0..=POINTS)
        .map(|i| sum(i as f32 / POINTS as f32))
        .collect();
    let peak = raw.iter().fold(0.0f32, |m, y| m.max(y.abs()));
    raw.iter()
        .enumerate()
        .map(|(i, &y)| {
            let y = if peak > 0.0 { y / peak } else { 0.0 };
            (i as f32 / POINTS as f32, 0.5 + 0.45 * y)
        })
        .collect()
}

/// The Percussion card's span, in seconds.
pub const PERC_SECS: f32 = 4.5;
/// The Percussion card's range, in dB below a drawbar at 8.
pub const PERC_DB: f32 = 60.0;

/// The percussion's level over the card's span, against a drawbar at 8.
/// Empty while it is off.
pub fn drawbar_perc(v: &impl Values) -> Pts {
    let at = |t: f32| drawbar::perc_level(|id| v.get(id), t);
    if at(0.0) <= 0.0 {
        return Vec::new();
    }
    (0..=POINTS)
        .map(|i| {
            let x = i as f32 / POINTS as f32;
            (x, level_y(at(x * PERC_SECS), PERC_DB))
        })
        .collect()
}

/// The Percussion card's caption: the harmonic ("2nd", "3rd") and the
/// decay, and "soft" when it is, or off.
pub fn drawbar_perc_caption(v: &impl Values) -> String {
    if v.get(db::PERC) < 0.5 {
        return "off".into();
    }
    let harm = if v.get(db::PERC_HARM) >= 0.5 {
        "3rd"
    } else {
        "2nd"
    };
    let mut s = format!(
        "{harm} · {}",
        pick(db::PERC_DECAYS, v.get(db::PERC_DECAY)).to_lowercase()
    );
    if v.get(db::PERC_VOL) >= 0.5 {
        s.push_str(" · soft");
    }
    s
}

/// The Rotor card's span, in seconds: long enough for the drum to settle.
pub const ROTOR_SECS: f32 = 10.0;

/// The horn's and the drum's speeds over the card's span after **Speed**
/// is switched to where it is now (to Fast from Slow, to Slow or Stop
/// from Fast). Empty when the rotary speaker is off.
pub fn drawbar_rotor(v: &impl Values) -> [Pts; 2] {
    if v.get(db::ROTARY) < 0.5 {
        return [Vec::new(), Vec::new()];
    }
    let speed = v.get(db::SPEED);
    let y = |hz: f32| 0.05 + 0.9 * (hz / drawbar::ROTOR_MAX).clamp(0.0, 1.0);
    let at = |x: f32| drawbar::rotor_ramp(speed, x * ROTOR_SECS);
    let curve = |horn: bool| -> Pts {
        (0..=POINTS)
            .map(|i| {
                let x = i as f32 / POINTS as f32;
                let (h, d) = at(x);
                (x, y(if horn { h } else { d }))
            })
            .collect()
    };
    [curve(true), curve(false)]
}

/// The Rotor card's caption: the speed and where the horn settles.
pub fn drawbar_rotor_caption(v: &impl Values) -> String {
    if v.get(db::ROTARY) < 0.5 {
        return "off".into();
    }
    let speed = v.get(db::SPEED);
    let (horn, drum) = drawbar::rotor_ramp(speed, 1e3);
    let name = pick(db::SPEEDS, speed).to_lowercase();
    if horn <= 0.0 {
        name
    } else {
        format!("{name} · {horn:.1} and {drum:.1} Hz")
    }
}

// ------------------------------------------------------------ Squelch

/// Two cycles of the wave: the saw, or the square shaped from it.
pub fn squelch_wave(v: &impl Values) -> Pts {
    let square = v.get(sq::WAVE) >= 0.5;
    (0..=POINTS)
        .map(|i| {
            let x = i as f32 / POINTS as f32;
            let saw = 2.0 * (2.0 * x).fract() - 1.0;
            let y = if square { squelch::square_of(saw) } else { saw };
            (x, 0.5 + 0.42 * y.clamp(-1.0, 1.0))
        })
        .collect()
}

/// Where a plain note's filter envelope starts (its peak) and where the
/// filter rests between notes, in Hz.
pub fn squelch_cutoffs(v: &impl Values) -> (f32, f32) {
    let c = squelch::cutoff_sweep(|id| v.get(id), false, &[0.0], 30.0, 2);
    (c[0], c[1])
}

/// The filter's response where it rests, and where a plain note's
/// envelope takes it (the dashed line; empty with no Env mod).
pub fn squelch_filter(v: &impl Values) -> [Pts; 2] {
    let (peak, rest) = squelch_cutoffs(v);
    let res = v.get(sq::RESONANCE);
    let curve = |fc: f32| response(|hz| squelch::response(fc, res, hz));
    let moved = if (peak / rest - 1.0).abs() > 0.01 {
        curve(peak)
    } else {
        Vec::new()
    };
    [curve(rest), moved]
}

/// The notes the Sweep card plays: four sixteenths at 120 BPM.
pub const SWEEP_STARTS: [f32; 4] = [0.0, 0.125, 0.25, 0.375];
/// The Sweep card's height: octaves above where the filter rests.
pub const SWEEP_OCT: f32 = 7.0;

/// The Sweep card's span in seconds: the four notes, then a little past
/// the longer decay.
pub fn squelch_sweep_secs(v: &impl Values) -> f32 {
    SWEEP_STARTS[3] + (0.001 * v.get(sq::DECAY).max(v.get(sq::ACCENT_DECAY)) * 1.25).clamp(0.2, 2.5)
}

/// The cutoff over the Sweep card's span for the four notes played plain
/// and played accented, in octaves above where the filter rests, so an
/// accent's build-up over close notes shows.
pub fn squelch_sweep(v: &impl Values) -> [Pts; 2] {
    let secs = squelch_sweep_secs(v);
    let (_, rest) = squelch_cutoffs(v);
    let curve = |accent: bool| -> Pts {
        squelch::cutoff_sweep(|id| v.get(id), accent, &SWEEP_STARTS, secs, POINTS + 1)
            .into_iter()
            .enumerate()
            .map(|(i, hz)| {
                let oct = (hz / rest).max(1e-6).log2() / SWEEP_OCT;
                (i as f32 / POINTS as f32, 0.05 + 0.9 * oct.clamp(0.0, 1.0))
            })
            .collect()
    };
    [curve(false), curve(true)]
}

// ------------------------------------------------------------ Analog

/// One naive cycle of an Analog wave at phase `t` (0 to 1): Saw, Pulse,
/// Saw+Pulse or Triangle, -1 to 1.
fn analog_wave(shape: f32, t: f32, pw: f32) -> f32 {
    let t = t.rem_euclid(1.0);
    let saw = 2.0 * t - 1.0;
    let pulse = if t < pw { 1.0 } else { -1.0 };
    match shape.round() as i32 {
        1 => pulse,
        2 => 0.5 * (saw + pulse),
        3 => 1.0 - 4.0 * (t - 0.5).abs(),
        _ => saw,
    }
}

/// Two cycles of oscillator 2 and, over the same time, oscillator 1 at its
/// pitch against it, restarting with each of oscillator 2's cycles when
/// synced. Each is scaled by its level; empty at level 0.
pub fn analog_oscillators(v: &impl Values) -> [Pts; 2] {
    let semis = 12.0 * (v.get(an::OSC1_OCT) - v.get(an::OSC2_OCT)) - v.get(an::OSC2_SEMI)
        + -v.get(an::OSC2_FINE) / 100.0;
    let ratio = 2f32.powf(semis / 12.0).clamp(1.0 / 16.0, 16.0);
    let sync = v.get(an::SYNC) >= 0.5;
    let trace = |level: f32, f: &dyn Fn(f32) -> f32| -> Pts {
        if level <= 0.0 {
            return Vec::new();
        }
        (0..=POINTS)
            .map(|i| {
                let x = i as f32 / POINTS as f32;
                (x, 0.5 + 0.45 * level.min(1.0) * f(x))
            })
            .collect()
    };
    let (w1, pw1) = (v.get(an::OSC1_WAVE), v.get(an::OSC1_PW) / 100.0);
    let (w2, pw2) = (v.get(an::OSC2_WAVE), v.get(an::OSC2_PW) / 100.0);
    let o1 = move |x: f32| {
        let master = 2.0 * x;
        let t = if sync {
            master.fract() * ratio
        } else {
            master * ratio
        };
        analog_wave(w1, t, pw1)
    };
    let o2 = move |x: f32| analog_wave(w2, 2.0 * x, pw2);
    [
        trace(v.get(an::OSC1_LEVEL) / 100.0, &o1),
        trace(v.get(an::OSC2_LEVEL) / 100.0, &o2),
    ]
}

/// The Osc card's caption, short enough for a fifth of the strip:
/// "saw, pulse" (oscillator 1, then 2), "saw × 2", "sync · saw+pulse"
/// (oscillator 1, synced).
pub fn analog_osc_caption(v: &impl Values) -> String {
    let name = |id: u16| match v.get(id).round() as i32 {
        1 => "pulse",
        2 => "saw+pulse",
        3 => "tri",
        _ => "saw",
    };
    let (w1, w2) = (name(an::OSC1_WAVE), name(an::OSC2_WAVE));
    let (on1, on2) = (v.get(an::OSC1_LEVEL) > 0.0, v.get(an::OSC2_LEVEL) > 0.0);
    if v.get(an::SYNC) >= 0.5 && on1 {
        return format!("sync · {w1}");
    }
    match (on1, on2) {
        (true, true) if w1 == w2 => format!("{w1} × 2"),
        (true, true) => format!("{w1}, {w2}"),
        (true, false) => w1.to_string(),
        (false, true) => w2.to_string(),
        (false, false) => "off".to_string(),
    }
}

/// The filter's response at **Cutoff**, and where the filter envelope's
/// peak takes it (the dashed line; empty with no Env amount).
pub fn analog_filter(v: &impl Values) -> [Pts; 2] {
    let mode = v.get(an::FILTER_TYPE);
    let fc = v.get(an::CUTOFF);
    let res = v.get(an::RESONANCE);
    let curve = |fc: f32| response(|hz| analog::filter_gain(mode, fc, res, hz));
    let peak = (fc * 2f32.powf(analog::ENV_OCT * v.get(an::ENV_AMT) / 100.0)).clamp(F_LO, F_HI);
    let moved = if (peak / fc - 1.0).abs() > 0.01 {
        curve(peak)
    } else {
        Vec::new()
    };
    [curve(fc), moved]
}

/// An envelope's shape with the CEM3310's attack (an RC charge toward 1.3
/// times the peak, stopped there) and decay and release falling to 1 %,
/// with times on a square-root scale, as Poly's Env card draws them.
pub fn analog_envelope(a_ms: f32, d_ms: f32, sustain: f32, r_ms: f32) -> Pts {
    let s = sustain.clamp(0.0, 1.0);
    let (wa, wd, wr) = (
        a_ms.max(0.0).sqrt(),
        d_ms.max(0.0).sqrt(),
        r_ms.max(0.0).sqrt(),
    );
    let hold = 0.22;
    let scale = (1.0 - hold) / (wa + wd + wr).max(1e-6);
    let (xa, xd, xr) = (wa * scale, wd * scale, wr * scale);
    let n = POINTS / 4;
    let mut pts = Vec::with_capacity(4 * n + 2);
    for i in 0..=n {
        let t = i as f32 / n as f32;
        pts.push((xa * t, analog::attack_shape(t)));
    }
    for i in 1..=n {
        let t = i as f32 / n as f32;
        pts.push((xa + xd * t, s + (1.0 - s) * (-4.6 * t).exp()));
    }
    pts.push((xa + xd + hold, s));
    for i in 1..=n {
        let t = i as f32 / n as f32;
        pts.push((xa + xd + hold + xr * t, s * (-4.6 * t).exp()));
    }
    pts
}

/// The amp envelope and the filter envelope.
pub fn analog_envelopes(v: &impl Values) -> [Pts; 2] {
    [
        analog_envelope(
            v.get(an::A_ATTACK),
            v.get(an::A_DECAY),
            v.get(an::A_SUSTAIN) / 100.0,
            v.get(an::A_RELEASE),
        ),
        analog_envelope(
            v.get(an::F_ATTACK),
            v.get(an::F_DECAY),
            v.get(an::F_SUSTAIN) / 100.0,
            v.get(an::F_RELEASE),
        ),
    ]
}

/// The LFO's span on its card, in seconds: two cycles, plus its delay.
pub fn analog_lfo_secs(v: &impl Values) -> f32 {
    2.0 / v.get(an::LFO_RATE).max(0.01) + 0.001 * v.get(an::LFO_DELAY)
}

/// The LFO over its card's span, fading in over its delay. Sample and hold
/// shows a fixed pattern of steps, as an example of what it does.
pub fn analog_lfo(v: &impl Values) -> Pts {
    const HELD: [f32; 8] = [0.4, -0.7, 0.9, -0.2, 0.6, -0.9, 0.1, -0.5];
    let shape = v.get(an::LFO_WAVE).round() as i32;
    let rate = v.get(an::LFO_RATE).max(0.01);
    let secs = analog_lfo_secs(v);
    let delay = 0.001 * v.get(an::LFO_DELAY);
    (0..=POINTS)
        .map(|i| {
            let x = i as f32 / POINTS as f32;
            let cycles = x * secs * rate;
            let t = cycles.fract();
            let y = match shape {
                0 => (TAU * t).sin(),
                1 => {
                    if t < 0.25 {
                        4.0 * t
                    } else if t < 0.75 {
                        2.0 - 4.0 * t
                    } else {
                        4.0 * t - 4.0
                    }
                }
                2 => 2.0 * t - 1.0,
                3 => {
                    if t < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                _ => HELD[(cycles as usize) % HELD.len()],
            };
            let fade = if delay > 0.0 {
                (x * secs / delay).min(1.0)
            } else {
                1.0
            };
            (x, 0.5 + 0.42 * y * fade)
        })
        .collect()
}

/// The LFO card's caption: its shape and rate (its delay shows as the
/// fade-in).
pub fn analog_lfo_caption(v: &impl Values) -> String {
    let hz = v.get(an::LFO_RATE);
    format!(
        "{} · {} Hz",
        pick(an::LFO_WAVES, v.get(an::LFO_WAVE)).to_lowercase(),
        if hz >= 10.0 {
            format!("{hz:.0}")
        } else {
            format!("{hz:.1}")
        }
    )
}

/// The Voices card's range: cents from the middle to the top.
pub const CARD_CENTS: f32 = 12.0;

/// Each voice card's fixed detune at the Age set, as stems from the middle
/// line: oscillator 1's, and oscillator 2's just right of it.
pub fn analog_cards(v: &impl Values) -> [Vec<Pts>; 2] {
    let cents = analog::card_cents(v.get(an::AGE));
    let n = cents.len() as f32;
    let stem = |c: usize, x_off: f32, ct: f32| -> Pts {
        let x = ((c as f32 + 0.5) / n + x_off).clamp(0.0, 1.0);
        let y = (0.5 + 0.45 * ct / CARD_CENTS).clamp(0.0, 1.0);
        vec![(x, 0.5), (x, y)]
    };
    let gap = 0.25 / n;
    [
        (0..cents.len())
            .map(|c| stem(c, -gap / 2.0, cents[c].0))
            .collect(),
        (0..cents.len())
            .map(|c| stem(c, gap / 2.0, cents[c].1))
            .collect(),
    ]
}

/// The Voices card's caption: the spread of the cards' tuning (RMS), or
/// "all alike" at Age 0.
pub fn analog_cards_caption(v: &impl Values) -> String {
    let age = v.get(an::AGE);
    if age <= 0.0 {
        return "all alike".into();
    }
    let cents = analog::card_cents(age);
    let rms = (cents.iter().map(|c| c.0 * c.0).sum::<f32>() / cents.len() as f32).sqrt();
    format!("±{rms:.1} cents")
}

// ------------------------------------------------------------ Cards

/// Below the note, the note and its octaves, the other harmonics: amber,
/// white and grey, as an organ's drawbars are brown, white and black.
const BAR_FAMILY: [Color; 3] = [QLINK_COLORS[2], TEXT, TEXT_DIM];

fn trace(pts: Pts, color: Color, width: f32, fill: bool, dashed: bool) -> Trace {
    Trace {
        pts,
        color,
        width,
        fill,
        dashed,
    }
}

/// Bright curves at 2.5 px, dashed ones at 1.5.
fn bright(pts: Pts, color: Color) -> Trace {
    trace(pts, color, 2.5, false, false)
}

fn freq_grid() -> Vec<(f32, String)> {
    [(100.0, "100"), (1000.0, "1k"), (10_000.0, "10k")]
        .iter()
        .map(|&(hz, s)| (freq_x(hz), s.to_string()))
        .collect()
}

fn hz_text(hz: f32) -> String {
    if hz >= 1000.0 {
        format!("{:.1} kHz", hz / 1000.0)
    } else {
        format!("{hz:.0} Hz")
    }
}

/// Whether this module draws the engine with this id.
pub fn draws(id: &str) -> bool {
    matches!(
        id,
        "mpfree.keys" | "mpfree.drawbar" | "mpfree.squelch" | "mpfree.analog"
    )
}

/// The cards of an engine this module draws, each with the page a tap on
/// it opens, marked active on that page. Empty for any other engine.
pub fn cards(id: &str, v: &impl Values, page: &str) -> Vec<(Card, &'static str)> {
    let mut cards = match id {
        "mpfree.keys" => keys_cards(v),
        "mpfree.drawbar" => drawbar_cards(v),
        "mpfree.squelch" => squelch_cards(v),
        "mpfree.analog" => analog_cards_strip(v),
        _ => Vec::new(),
    };
    for (c, home) in &mut cards {
        c.active = *home == page;
    }
    cards
}

/// Keys: the hammer and the decays (Strike), the pickup (Tone) and the
/// tremolo (Amp).
fn keys_cards(v: &impl Values) -> Vec<(Card, &'static str)> {
    let (blue, green) = (QLINK_COLORS[0], QLINK_COLORS[1]);
    let [soft, full] = keys_hammer(v);
    let secs = keys_decay_secs(v);
    let [low, mid, high] = keys_decay(v);
    let (curve, hit, rest) = keys_pickup(v);
    let ring = keys::decay_for(v.get(kp::DECAY), KEYS_HZ);
    vec![
        (
            Card {
                title: "Hammer".into(),
                caption: format!("{:.1} ms at full force", keys_contact(v) * 1000.0),
                traces: vec![
                    trace(soft, TEXT_DIM, 2.0, false, false),
                    trace(full, TEXT, 2.5, true, false),
                ],
                grid: kv::time_grid(HAMMER_SECS),
                ..Card::default()
            },
            "Strike",
        ),
        (
            Card {
                title: "Decay".into(),
                caption: format!("{} at middle C", kv::secs_text(ring)),
                traces: vec![bright(low, green), bright(mid, TEXT), bright(high, blue)],
                grid: kv::time_grid(secs),
                ..Card::default()
            },
            "Strike",
        ),
        (
            Card {
                title: "Pickup".into(),
                caption: if keys_reed(v) { "reed" } else { "tine" }.into(),
                traces: vec![
                    trace(curve, TEXT_FAINT, 1.5, false, false),
                    trace(hit, blue, 3.0, false, false),
                ],
                dot: Some(rest),
                ..Card::default()
            },
            "Tone",
        ),
        (
            Card {
                title: "Tremolo".into(),
                caption: keys_tremolo_caption(v),
                traces: keys_tremolo(v)
                    .into_iter()
                    .zip([blue, green])
                    .map(|(p, c)| bright(p, c))
                    .collect(),
                ..Card::default()
            },
            "Amp",
        ),
    ]
}

/// Drawbar: the drawbars and their wave (Drawbars), the percussion
/// (Percussion) and the rotors (Rotary).
fn drawbar_cards(v: &impl Values) -> Vec<(Card, &'static str)> {
    let (blue, green, amber) = (QLINK_COLORS[0], QLINK_COLORS[1], QLINK_COLORS[2]);
    let [horn, drum] = drawbar_rotor(v);
    let perc = drawbar_perc(v);
    vec![
        (
            Card {
                title: "Drawbars".into(),
                caption: drawbar_registration(v),
                bars: drawbar_bars(v).to_vec(),
                bar_colors: (0..9).map(|b| BAR_FAMILY[drawbar_family(b)]).collect(),
                bar_labels: ["16", "", "8", "4", "", "2", "", "", "1"]
                    .map(String::from)
                    .to_vec(),
                ..Card::default()
            },
            "Drawbars",
        ),
        (
            Card {
                title: "Wave".into(),
                caption: "one 16' cycle".into(),
                traces: vec![bright(drawbar_wave(v), TEXT)],
                midline: true,
                ..Card::default()
            },
            "Drawbars",
        ),
        (
            Card {
                title: "Percussion".into(),
                caption: drawbar_perc_caption(v),
                traces: (!perc.is_empty())
                    .then(|| trace(perc, amber, 2.5, true, false))
                    .into_iter()
                    .collect(),
                grid: kv::time_grid(PERC_SECS),
                ..Card::default()
            },
            "Percussion",
        ),
        (
            Card {
                title: "Rotor".into(),
                caption: drawbar_rotor_caption(v),
                traces: [(horn, blue), (drum, green)]
                    .into_iter()
                    .filter(|(p, _)| !p.is_empty())
                    .map(|(p, c)| bright(p, c))
                    .collect(),
                grid: kv::time_grid(ROTOR_SECS),
                ..Card::default()
            },
            "Rotary",
        ),
    ]
}

/// Squelch: the wave and the filter (Synth), the cutoff's sweep (Play).
fn squelch_cards(v: &impl Values) -> Vec<(Card, &'static str)> {
    let (blue, amber) = (QLINK_COLORS[0], QLINK_COLORS[2]);
    let [rest_curve, peak_curve] = squelch_filter(v);
    let (_, rest) = squelch_cutoffs(v);
    let [plain, accent] = squelch_sweep(v);
    vec![
        (
            Card {
                title: "Wave".into(),
                caption: pick(sq::WAVES, v.get(sq::WAVE)).to_lowercase(),
                traces: vec![bright(squelch_wave(v), blue)],
                midline: true,
                ..Card::default()
            },
            "Synth",
        ),
        (
            Card {
                title: "Filter".into(),
                caption: format!("rests at {}", hz_text(rest)),
                traces: [trace(rest_curve, blue, 2.5, true, false)]
                    .into_iter()
                    .chain(
                        (!peak_curve.is_empty())
                            .then(|| trace(peak_curve, TEXT_DIM, 1.5, false, true)),
                    )
                    .collect(),
                grid: freq_grid(),
                dot: Some(freq_x(rest)),
                ..Card::default()
            },
            "Synth",
        ),
        (
            Card {
                title: "Sweep".into(),
                caption: "16ths, plain and accented".into(),
                traces: vec![bright(plain, blue), bright(accent, amber)],
                grid: kv::time_grid(squelch_sweep_secs(v)),
                ..Card::default()
            },
            "Play",
        ),
    ]
}

/// Analog: the oscillators (Osc), the filter (Filter), the envelopes
/// (Envelopes), the LFO (Mod) and the voice cards (Voice).
fn analog_cards_strip(v: &impl Values) -> Vec<(Card, &'static str)> {
    let (blue, green, amber) = (QLINK_COLORS[0], QLINK_COLORS[1], QLINK_COLORS[2]);
    let [o1, o2] = analog_oscillators(v);
    let [f, moved] = analog_filter(v);
    let [amp, fenv] = analog_envelopes(v);
    let [c1, c2] = analog_cards(v);
    vec![
        (
            Card {
                title: "Osc".into(),
                caption: analog_osc_caption(v),
                traces: [(o1, blue), (o2, green)]
                    .into_iter()
                    .filter(|(p, _)| !p.is_empty())
                    .map(|(p, c)| bright(p, c))
                    .collect(),
                midline: true,
                ..Card::default()
            },
            "Osc",
        ),
        (
            Card {
                title: "Filter".into(),
                caption: pick(an::FILTERS, v.get(an::FILTER_TYPE)).to_string(),
                traces: [trace(f, blue, 2.5, true, false)]
                    .into_iter()
                    .chain((!moved.is_empty()).then(|| trace(moved, TEXT_DIM, 1.5, false, true)))
                    .collect(),
                grid: freq_grid(),
                dot: Some(freq_x(v.get(an::CUTOFF))),
                ..Card::default()
            },
            "Filter",
        ),
        (
            Card {
                title: "Env".into(),
                caption: "amp, filter".into(),
                traces: vec![trace(amp, TEXT, 2.5, true, false), bright(fenv, blue)],
                ..Card::default()
            },
            "Envelopes",
        ),
        (
            Card {
                title: "LFO".into(),
                caption: analog_lfo_caption(v),
                traces: vec![bright(analog_lfo(v), amber)],
                midline: true,
                ..Card::default()
            },
            "Mod",
        ),
        (
            Card {
                title: "Voices".into(),
                caption: analog_cards_caption(v),
                traces: c1
                    .into_iter()
                    .map(|p| trace(p, blue, 2.0, false, false))
                    .chain(c2.into_iter().map(|p| trace(p, green, 2.0, false, false)))
                    .collect(),
                midline: true,
                ..Card::default()
            },
            "Voice",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::{Node, ParamInfo};
    use mpfree_synth::{Analog, Drawbar, Keys, Squelch};

    fn in_box(pts: &[(f32, f32)]) -> bool {
        pts.iter().all(|&(x, y)| {
            x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)
        })
    }

    /// Every preset, then every parameter at its minimum and maximum on top
    /// of the defaults: `check` sees the values each time.
    fn sweep(node: &mut dyn Node, presets: &[mpfree_synth::Preset], check: &dyn Fn(&dyn Node)) {
        for p in presets {
            node.reset();
            for info in node.params().to_vec() {
                node.set_param(info.id, info.default);
            }
            for &(id, value) in p.values {
                node.set_param(id, value);
            }
            check(node);
        }
        let params: Vec<ParamInfo> = node.params().to_vec();
        for info in &params {
            for value in [info.min, info.max] {
                for q in &params {
                    node.set_param(q.id, q.default);
                }
                node.set_param(info.id, value);
                check(node);
            }
        }
    }

    #[test]
    fn every_card_stays_in_its_box_with_its_dot_and_bars() {
        let mut nodes: [(Box<dyn Node>, &[mpfree_synth::Preset]); 4] = [
            (Box::new(Keys::new(48_000)), Keys::presets()),
            (Box::new(Drawbar::new(48_000)), Drawbar::presets()),
            (Box::new(Squelch::new(48_000)), Squelch::presets()),
            (Box::new(Analog::new(48_000)), Analog::presets()),
        ];
        for (node, presets) in &mut nodes {
            let id = node.info().id;
            assert!(draws(id));
            sweep(node.as_mut(), presets, &|n| {
                let v = |id| n.param(id);
                for (c, page) in cards(id, &v, "") {
                    assert!(!c.active && !page.is_empty());
                    assert!(!c.caption.is_empty(), "{id} {}", c.title);
                    for t in &c.traces {
                        assert!(in_box(&t.pts), "{id} {}", c.title);
                    }
                    assert!(c.dot.is_none_or(|x| (0.0..=1.0).contains(&x)));
                    assert!(c.bars.iter().all(|b| (0.0..=1.0).contains(b)));
                }
            });
        }
        assert!(!draws("mpfree.poly"));
        assert!(cards("mpfree.poly", &|_: u16| 0.0, "Osc").is_empty());
    }

    #[test]
    fn keys_curves_stay_in_their_boxes() {
        let mut s = Keys::new(48_000);
        sweep(&mut s, Keys::presets(), &|n| {
            let v = |id| n.param(id);
            let (full, hit, x) = keys_pickup(&v);
            assert!(in_box(&full) && in_box(&hit) && (0.0..=1.0).contains(&x));
            assert!(!hit.is_empty());
            for p in keys_hammer(&v).iter().chain(keys_decay(&v).iter()) {
                assert!(in_box(p), "{p:?}");
            }
            for p in keys_tremolo(&v) {
                assert!(in_box(&p));
            }
            assert!(!keys_tremolo_caption(&v).is_empty());
        });
    }

    #[test]
    fn keys_voicing_moves_the_rest_point_and_hardness_shortens_the_hammer() {
        let at = |voicing: f32, hardness: f32| {
            let v = move |id: u16| match id {
                kp::VOICING => voicing,
                kp::HARDNESS => hardness,
                kp::BARK => 50.0,
                _ => 0.0,
            };
            (keys_pickup(&v).2, keys_contact(&v))
        };
        let (centred, soft) = at(0.0, 0.0);
        let (voiced, hard) = at(100.0, 100.0);
        assert!(
            (centred - 0.5).abs() < 1e-3,
            "centred on the pickup: {centred}"
        );
        assert!(voiced > 0.6, "voiced off the axis: {voiced}");
        assert!(hard < soft * 0.5, "{hard} {soft}");
        // Higher notes ring shorter: C6's curve falls below C2's.
        let v = |id: u16| if id == kp::DECAY { 8000.0 } else { 0.0 };
        let [c2, _, c6] = keys_decay(&v);
        assert!(c6[POINTS / 2].1 < c2[POINTS / 2].1);
    }

    #[test]
    fn drawbar_curves_stay_in_their_boxes() {
        let mut s = Drawbar::new(48_000);
        sweep(&mut s, Drawbar::presets(), &|n| {
            let v = |id| n.param(id);
            assert!(drawbar_bars(&v).iter().all(|b| (0.0..=1.0).contains(b)));
            assert_eq!(drawbar_registration(&v).len(), 9);
            assert!(in_box(&drawbar_wave(&v)));
            assert!(in_box(&drawbar_perc(&v)));
            for p in drawbar_rotor(&v) {
                assert!(in_box(&p));
            }
        });
    }

    #[test]
    fn drawbar_cards_follow_the_settings() {
        let s = Drawbar::new(48_000);
        let v = |id| s.param(id);
        assert_eq!(drawbar_registration(&v), "888000000");
        // Only the 8': one sine over two cycles, crossing the middle at 0.
        let only8 = |id: u16| if id == db::DB_8 { 8.0 } else { 0.0 };
        let w = drawbar_wave(&only8);
        assert!((w[0].1 - 0.5).abs() < 1e-3);
        assert!((w[POINTS / 8].1 - 0.95).abs() < 0.01, "{:?}", w[POINTS / 8]);
        // Percussion off draws nothing; slow falls slower than fast.
        assert!(drawbar_perc(&only8).is_empty());
        let perc = |slow: f32| {
            move |id: u16| match id {
                db::PERC => 1.0,
                db::PERC_DECAY => slow,
                _ => 0.0,
            }
        };
        let (fast, slow) = (drawbar_perc(&perc(0.0)), drawbar_perc(&perc(1.0)));
        assert!(fast[POINTS / 4].1 < slow[POINTS / 4].1);
        // Switched to Fast, the horn gets there long before the drum.
        let fast_rotor = |id: u16| match id {
            db::ROTARY => 1.0,
            db::SPEED => db::SPEED_FAST,
            _ => 0.0,
        };
        let [horn, drum] = drawbar_rotor(&fast_rotor);
        let tenth = POINTS / 10;
        assert!(
            horn[tenth].1 > 0.9 && drum[tenth].1 < 0.5,
            "{:?} {:?}",
            horn[tenth],
            drum[tenth]
        );
    }

    #[test]
    fn squelch_curves_stay_in_their_boxes() {
        let mut s = Squelch::new(48_000);
        sweep(&mut s, Squelch::presets(), &|n| {
            let v = |id| n.param(id);
            assert!(in_box(&squelch_wave(&v)));
            for p in squelch_filter(&v).iter().chain(squelch_sweep(&v).iter()) {
                assert!(in_box(p), "{p:?}");
            }
        });
    }

    #[test]
    fn squelch_resonance_peaks_at_the_cutoff_and_accents_sweep_higher() {
        let v = |res: f32| {
            move |id: u16| match id {
                sq::CUTOFF => 1000.0,
                sq::RESONANCE => res,
                sq::ENV_MOD => 50.0,
                sq::DECAY => 300.0,
                sq::ACCENT => 100.0,
                sq::ACCENT_DECAY => 200.0,
                _ => 0.0,
            }
        };
        let (_, rest) = squelch_cutoffs(&v(90.0));
        let peak_at = |res: f32| {
            let [f, _] = squelch_filter(&v(res));
            let top = f
                .iter()
                .cloned()
                .fold((0.0, 0.0), |m, p| if p.1 > m.1 { p } else { m });
            (top.0, top.1)
        };
        let (x, y) = peak_at(90.0);
        assert!(
            (x - freq_x(rest)).abs() < 0.03,
            "peak at {x}, rest at {}",
            freq_x(rest)
        );
        assert!(y > db_y(1.0), "resonance rises above the passband");
        assert!(peak_at(0.0).1 <= db_y(1.0) + 1e-3, "no peak at 0");
        let [plain, accent] = squelch_sweep(&v(90.0));
        assert!(accent.iter().map(|p| p.1).fold(0.0, f32::max) > plain[0].1);
        // Each note's start, just after: accents build up, plain notes
        // reach the same height every time.
        let secs = squelch_sweep_secs(&v(90.0));
        let at = |pts: &Pts, t: f32| pts[(t / secs * POINTS as f32).ceil() as usize].1;
        let last = SWEEP_STARTS[3];
        assert!(
            at(&accent, last) > at(&accent, 0.0) + 0.05,
            "accents build up"
        );
        assert!((at(&plain, last) - at(&plain, 0.0)).abs() < 0.02);
    }

    #[test]
    fn analog_curves_stay_in_their_boxes() {
        let mut s = Analog::new(48_000);
        sweep(&mut s, Analog::presets(), &|n| {
            let v = |id| n.param(id);
            for p in analog_oscillators(&v)
                .iter()
                .chain(analog_filter(&v).iter())
                .chain(analog_envelopes(&v).iter())
            {
                assert!(in_box(p), "{p:?}");
            }
            assert!(in_box(&analog_lfo(&v)));
            for stems in analog_cards(&v) {
                assert_eq!(stems.len(), analog::CARDS);
                assert!(stems.iter().all(|p| in_box(p)));
            }
            assert!(!analog_osc_caption(&v).is_empty() && !analog_lfo_caption(&v).is_empty());
        });
    }

    #[test]
    fn analog_cards_grow_with_age_and_the_attack_bows() {
        let at = |age: f32| {
            let v = move |id: u16| if id == an::AGE { age } else { 0.0 };
            analog_cards(&v)[0]
                .iter()
                .map(|s| (s[1].1 - 0.5).abs())
                .fold(0.0, f32::max)
        };
        assert_eq!(at(0.0), 0.0);
        assert!(at(100.0) > 2.0 * at(30.0) && at(30.0) > 0.0);
        let env = analog_envelope(100.0, 100.0, 0.5, 100.0);
        // Halfway through the attack it is past half (an RC curve bowed up).
        let half = env[POINTS / 8].1;
        assert!((half - 0.675).abs() < 0.02, "{half}");
        // The four-pole at full resonance peaks near its cutoff.
        let v = |id: u16| match id {
            an::CUTOFF => 1000.0,
            an::RESONANCE => 100.0,
            _ => 0.0,
        };
        let [f, _] = analog_filter(&v);
        let top = f
            .iter()
            .cloned()
            .fold((0.0, 0.0), |m, p| if p.1 > m.1 { p } else { m });
        assert!((top.0 - freq_x(1000.0)).abs() < 0.03, "{top:?}");
    }
}
