//! Playing it (phase 0C): how new notes are played, once their pitches
//! and rhythm are set. The second prototype's rules (after the KTH
//! performance rules), written into each note's decision so a chord
//! change later re-fits the notes and keeps how they were played
//! (contract 2).
//!
//! - **Loudness**: the style's level and accents per 1/16, a rise and fall
//!   over every four bars, the bar's tension, how colourful the note is
//!   over its chord (melodic charge), contour peaks and notes after a
//!   leap, and higher notes a little louder. How far these move a note is
//!   the instrument's velocity response (contract 10). Keep Accents
//!   leaves the loudness as yours.
//! - **Length**: the style's gate, repeated notes shorter, short notes a
//!   little shorter and softer, and a phrase's landing note held, then a
//!   breath.
//! - **Timing**: the drummer's groove where drums were read, else the
//!   style's slow drift where it has one (0 in Minimal): one smooth (1/f)
//!   curve shared by every part, never separate noise per note. Groove
//!   and playing offsets together stay within ±30 ms (contract 3).

use std::f64::consts::{PI, TAU};

use mpfree_context::chord::ChordTimeline;
use mpfree_context::groove::{GrooveTemplate, Lane, STEP};
use mpfree_context::PartRole;
use mpfree_transport::time::{Meter, Swing, Tick};

use crate::dice::{DiceName, Job};
use crate::event::{Event, Part};
use crate::plan::{BarJob, BarPlan};
use crate::style::Style;
use crate::timing::{drift_on, ms_to_ticks, Timing};

/// Velocity per decibel, roughly, on a synth's velocity curve.
pub const VELOCITY_PER_DB: f64 = 4.5;

/// Melodic charge (Friberg) of a note this many semitones above the
/// chord's root: stable notes low, colourful notes high.
fn charge(iv: i32) -> f64 {
    match iv.rem_euclid(12) {
        0 => 0.0,
        7 => 1.0,
        2 => 2.0,
        9 => 3.0,
        4 => 4.0,
        11 => 5.0,
        6 => 6.0,
        1 => 6.5,
        8 => 5.5,
        3 => 4.5,
        10 => 3.5,
        _ => 2.5,
    }
}

/// The drum lane a part's notes sit with: a bass with the kick, the rest
/// with the hats.
pub const fn lane_of(role: PartRole) -> Lane {
    match role {
        PartRole::Bass => Lane::Kick,
        _ => Lane::Hats,
    }
}

/// The loudness and length rules on a part's notes. `events` are the
/// part's decisions realised (the same order), so the rules can hear the
/// pitches; the results go into the decisions.
pub fn play(
    part: &mut Part,
    events: &[Event],
    plan: &[BarPlan],
    chords: &ChordTimeline,
    style: &Style,
    meter: Meter,
    keep_accents: bool,
) {
    let n = part.decisions.len().min(events.len());
    if n == 0 {
        return;
    }
    let p = &style.playing;
    let k = f64::from(part.kind.profile().velocity_response);
    let bar = meter.bar_ticks().max(1);
    let beat = meter.beat_ticks().max(1);
    let centre = events[..n].iter().map(|e| f64::from(e.pitch)).sum::<f64>() / n as f64;
    let range = f64::from(p.velocity_range);
    for i in 0..n {
        let e = events[i];
        let at = part.decisions[i].at;
        let b = (at.div_euclid(bar) as usize).min(plan.len().saturating_sub(1));
        let bp = plan.get(b).copied();
        let step = (at.rem_euclid(bar) / STEP) as usize;
        let prev = i.checked_sub(1).map(|j| i32::from(events[j].pitch));
        let next = events[..n].get(i + 1);
        let pitch = i32::from(e.pitch);

        // Loudness.
        let base = f64::from(p.velocity) + f64::from(p.accent[step % p.accent.len()]);
        let mut v = 0.0;
        let arch = (PI * (at.rem_euclid(4 * bar) as f64 / (4 * bar) as f64)).sin() - 0.5;
        v += arch * range * 0.5;
        v += (f64::from(bp.map_or(0.5, |b| b.tension)) - 0.5) * range * 0.4;
        if let Some(c) = chords.span_at(at).and_then(|s| s.answer.chord()) {
            v += 0.2 * charge(pitch - i32::from(c.root)) * VELOCITY_PER_DB * 0.5;
        }
        if let (Some(a), Some(nx)) = (prev, next) {
            if pitch > a && pitch > i32::from(nx.pitch) {
                v += 6.0;
            }
        }
        if prev.is_some_and(|a| (pitch - a).abs() >= 5) {
            v += 4.0;
        }
        v += (f64::from(e.pitch) - centre) / 12.0 * 3.0 * VELOCITY_PER_DB * 0.4;

        // Length.
        let gap = next.map_or(e.len, |x| x.at - at).max(1);
        let slot = e.len.min(gap);
        let mut gate = f64::from(p.gate);
        if next.is_some_and(|x| x.pitch == e.pitch) {
            gate *= 0.8;
        }
        if slot < 2 * STEP {
            gate *= 0.95;
            v -= 3.0;
        }
        let lands = bp.is_some_and(|b| matches!(b.job, BarJob::Land(_)))
            && next.is_none_or(|x| x.at.div_euclid(bar) != at.div_euclid(bar));
        let len = if lands {
            slot.max(gap.min(2 * beat)) - STEP / 2
        } else {
            (slot as f64 * gate) as Tick
        };

        let d = &mut part.decisions[i];
        d.len = len.max(STEP / 3);
        if !keep_accents {
            d.velocity = (base + v * k).round().clamp(20.0, 127.0) as u8;
        }
    }
}

/// A slow shared drift (Voss-McCartney 1/f noise): one offset per 1/16
/// step, in ticks, spread `sd_ms` at `bpm`. Drawn from the request's
/// seed alone, so every part of a request drifts together.
pub fn drift(seed: u64, steps: usize, sd_ms: f32, bpm: f64) -> Vec<f64> {
    let mut dice = DiceName {
        seed,
        part: 0,
        phrase: 0,
        job: Job::Drift,
        bar: 0,
        attempt: 0,
    }
    .dice();
    let mut normal = || {
        let u = dice.unit().clamp(1e-12, 1.0);
        let w = dice.unit();
        (-2.0 * u.ln()).sqrt() * (TAU * w).cos()
    };
    const ROWS: usize = 6;
    let mut rows = [0.0f64; ROWS];
    for r in rows.iter_mut() {
        *r = normal();
    }
    let scale = ms_to_ticks(f64::from(sd_ms), bpm) / (ROWS as f64).sqrt();
    (0..steps)
        .map(|i| {
            for (k, r) in rows.iter_mut().enumerate() {
                if i % (1 << k) == 0 {
                    *r = normal();
                }
            }
            rows.iter().sum::<f64>() * scale
        })
        .collect()
}

/// Each note's timing: the drummer's groove at its step where drums were
/// read (against the swing the clip plays with), else the style's drift
/// where it has one. A note off the 1/16 grid takes no groove.
pub fn timing(
    part: &mut Part,
    groove: Option<&GrooveTemplate>,
    style: &Style,
    swing: Swing,
    seed: u64,
    bpm: f64,
) {
    let lane = lane_of(part.role);
    let last = part.decisions.iter().map(|d| d.at).max().unwrap_or(0);
    let curve = drift_on(style.drift_ms, groove)
        .then(|| drift(seed, (last / STEP) as usize + 1, style.drift_ms, bpm));
    for d in part.decisions.iter_mut() {
        d.lane = lane;
        let step = d.at / STEP;
        let on_grid = d.at.rem_euclid(STEP) == 0;
        d.timing = match groove {
            Some(g) if on_grid => Timing::from_groove(g, lane, step as usize, swing),
            _ => Timing::default(),
        };
        if let Some(c) = &curve {
            d.timing.play = c.get(step as usize).copied().unwrap_or(0.0) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Motion, NoteDecision, PartId, Role, Source};
    use crate::plan::plan;
    use crate::profile::InstrumentKind;
    use crate::realize::{realize, Fit};
    use crate::settings::{Ending, Settings};
    use crate::style::{GROOVE, JAZZISH, MINIMAL};
    use crate::timing::LIMIT_MS;
    use mpfree_context::chord::{Chord, Heard, Quality, Tone};
    use mpfree_context::groove::Hit;
    use mpfree_context::key::{Key, Mode};
    use mpfree_transport::time::PPQ;

    const BEAT: Tick = PPQ as Tick;
    const BAR: Tick = 4 * BEAT;

    fn chords() -> ChordTimeline {
        ChordTimeline::of(&[(Chord::new(0, Quality::Minor), 4 * BAR)], Heard::Heard)
    }

    /// Two bars of a C minor line ending on a long C, as a part.
    fn line(style: &Style) -> (Part, Vec<Event>, Vec<BarPlan>) {
        let s = Settings {
            style: *style,
            ending: Ending::Home,
            ..Settings::default()
        };
        let plan = plan(&s, 1, 3, 2.0);
        let d = |at, role, m| NoteDecision::new(BAR + at, BEAT / 2, 90, role, m);
        let mut ds = vec![
            d(0, Role::Chord(Tone::Root), Motion::NEAREST),
            d(BEAT / 2, Role::Scale, Motion::up(2)),
            d(BEAT, Role::Chord(Tone::Third), Motion::up(1)),
            d(3 * BEAT / 2, Role::Chord(Tone::Fifth), Motion::up(4)),
            d(2 * BEAT, Role::Chord(Tone::Root), Motion::up(5)),
            d(5 * BEAT / 2, Role::Chord(Tone::Fifth), Motion::down(5)),
            d(3 * BEAT, Role::Chord(Tone::Fifth), Motion::NEAREST),
            d(BAR, Role::Chord(Tone::Third), Motion::down(4)),
            d(BAR + 2 * BEAT, Role::Chord(Tone::Root), Motion::down(3)),
        ];
        ds.last_mut().unwrap().len = 2 * BEAT;
        let part = Part {
            id: PartId(0),
            role: PartRole::Melody,
            kind: InstrumentKind::Chordal,
            register: 60,
            decisions: ds,
        };
        let t = chords();
        let fit = Fit {
            harmony: &t,
            key: Some(Key::new(0, Mode::Minor)),
            played_pcs: 0,
            origin: 0,
        };
        let ev = realize(&part, &fit);
        (part, ev, plan)
    }

    #[test]
    fn peaks_ring_out_repeats_are_shorter_and_the_landing_is_held() {
        let (mut part, ev, plan) = line(&MINIMAL);
        let pitches: Vec<u8> = ev.iter().map(|e| e.pitch).collect();
        assert_eq!(pitches, vec![60, 62, 63, 67, 72, 67, 67, 63, 60]);
        play(
            &mut part,
            &ev,
            &plan,
            &chords(),
            &MINIMAL,
            Meter::default(),
            false,
        );
        let v: Vec<u8> = part.decisions.iter().map(|d| d.velocity).collect();
        let len: Vec<Tick> = part.decisions.iter().map(|d| d.len).collect();
        // The top C, a peak after a leap, is the loudest note.
        let top = (0..v.len()).max_by_key(|&i| v[i]).unwrap();
        assert_eq!(top, 4, "{v:?}");
        // The G repeated straight after is shorter than the G before.
        assert!(len[5] < len[3], "{len:?}");
        // The last note lands home and is held, then a breath.
        assert_eq!(plan[2].job, BarJob::Land(Ending::Home));
        assert!(len[8] >= 2 * BEAT - STEP / 2, "{len:?}");
        assert!(len[0] < BEAT / 2, "Minimal's short gate");
        // Keep Accents keeps your loudness.
        let (mut kept, ev, plan) = line(&MINIMAL);
        play(
            &mut kept,
            &ev,
            &plan,
            &chords(),
            &MINIMAL,
            Meter::default(),
            true,
        );
        assert!(kept.decisions.iter().all(|d| d.velocity == 90));
        assert!(kept.decisions.iter().all(|d| d.source == Source::Mpfree));
    }

    #[test]
    fn a_softer_instrument_moves_loudness_less() {
        let spread = |kind: InstrumentKind| {
            let (mut part, ev, plan) = line(&GROOVE);
            part.kind = kind;
            play(
                &mut part,
                &ev,
                &plan,
                &chords(),
                &GROOVE,
                Meter::default(),
                false,
            );
            let v: Vec<i32> = part
                .decisions
                .iter()
                .map(|d| i32::from(d.velocity))
                .collect();
            v.iter().max().unwrap() - v.iter().min().unwrap()
        };
        assert!(spread(InstrumentKind::Chordal) > spread(InstrumentKind::OneNote));
    }

    #[test]
    fn drift_is_slow_shared_and_follows_the_style() {
        let d = drift(9, 256, 6.0, 120.0);
        assert_eq!(d, drift(9, 256, 6.0, 120.0), "the same for every part");
        let sd = (d.iter().map(|x| x * x).sum::<f64>() / d.len() as f64).sqrt();
        let want = ms_to_ticks(6.0, 120.0);
        assert!(sd > 0.4 * want && sd < 1.8 * want, "{sd} against {want}");
        // Smooth: neighbouring steps are closer than steps far apart.
        let near: f64 = d.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / 255.0;
        let far: f64 = (0..192).map(|i| (d[i + 64] - d[i]).abs()).sum::<f64>() / 192.0;
        assert!(near < far, "{near} {far}");

        // Minimal has none; Groove has it unless drums were read.
        let (mut part, _, _) = line(&MINIMAL);
        timing(&mut part, None, &MINIMAL, Swing::default(), 9, 120.0);
        assert!(part.decisions.iter().all(|d| d.timing == Timing::default()));
        let (mut part, _, _) = line(&GROOVE);
        timing(&mut part, None, &GROOVE, Swing::default(), 9, 120.0);
        assert!(part.decisions.iter().any(|d| d.timing.play != 0.0));
        assert!(part.decisions.iter().all(|d| d.lane == Lane::Hats));
        for d in &part.decisions {
            let ms = d.timing.offset_ticks(120.0) / ms_to_ticks(1.0, 120.0);
            assert!(ms.abs() <= LIMIT_MS + 1e-9);
        }
    }

    #[test]
    fn a_played_groove_sets_the_timing_and_the_drift_stays_off() {
        // Hats a 1/64 late on every off 1/8.
        let hits: Vec<Hit> = (0..8)
            .map(|i| Hit {
                lane: Lane::Hats,
                heard: (i * BEAT / 2 + if i % 2 == 1 { STEP / 4 } else { 0 }) as f64,
                velocity: 90,
            })
            .collect();
        let g = GrooveTemplate::read(&hits, 16, Swing::default());
        let (mut part, _, _) = line(&JAZZISH);
        timing(&mut part, Some(&g), &JAZZISH, Swing::default(), 9, 120.0);
        for d in &part.decisions {
            let off8 = (d.at % BEAT) == BEAT / 2;
            let want = if off8 { 0.25 } else { 0.0 };
            assert!((f64::from(d.timing.groove) - want).abs() < 1e-6, "{d:?}");
            assert_eq!(d.timing.play, 0.0, "no drift over a played groove");
        }
        // A bass follows the kick.
        let (mut bass, _, _) = line(&JAZZISH);
        bass.role = PartRole::Bass;
        timing(&mut bass, Some(&g), &JAZZISH, Swing::default(), 9, 120.0);
        assert!(bass
            .decisions
            .iter()
            .all(|d| d.lane == Lane::Kick && d.timing.groove == 0.0));
    }
}
