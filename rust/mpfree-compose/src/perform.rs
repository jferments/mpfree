//! Playing that moves (revision 3, S3): the same notes played, not typed.
//!
//! The phase 0C rules ([`crate::playing`]) play each note from where it
//! falls: the style's accents, a rise and fall over every four bars, the
//! bar's tension, the note's colour over its chord. Here the playing
//! follows the phrase the sketch planned ([`Sketch`]) and the instrument
//! that plays it ([`Expression`]):
//!
//! - **Loudness leans to the high point and eases off at the end.** Each
//!   phrase (the notes up to a landing, or to the last note) grows towards
//!   its high point (the plan's peak where the phrase holds it, else its
//!   highest note) and eases off after it; the phrase with the peak moves
//!   furthest. This takes the place of the fixed four-bar rise and fall,
//!   and the peak note itself leans a little more.
//! - **Timing breathes with the phrase.** Notes climbing to the high point
//!   move a little ahead, the high point arrives a hair late, the notes
//!   after it lay back, and a phrase's last note before a breath comes
//!   late, most at a Home ending. A few ms, as the instrument plays: a lead
//!   most, a bass least (it sits on the kick). Groove and playing together
//!   stay within contract 3's ±30 ms ([`crate::timing::Timing::offset_ticks`]).
//! - **Notes join as the instrument joins them.** A one-note lead slurs a
//!   step on its way up (legato, no gap, so a mono synth glides where it
//!   glides); a held-back bar is played shorter; the high point is held a
//!   little longer, never into the next note on a one-note instrument.
//! - **Chords are voiced and rolled.** On keys and pads the notes of a
//!   chord come in low to high over a few ms (slower when soft), the top
//!   voice a little louder and the inner voices a little softer.
//!
//! Only loudness, length and timing change: the notes, their places and
//! the chords are the same with and without, so the critic's choice is
//! the same and the S3 listening gate can play both. The style's
//! [`crate::style::PlayingStyle::gesture`] sets how far (0: off, the
//! phase 0C playing). Keep Accents leaves the loudness as the 0C rules
//! made it. Drums have nothing here: a beat's feel is its own.

use std::f64::consts::PI;

use mpfree_context::groove::STEP;
use mpfree_transport::time::{Meter, Tick};

use crate::event::{Event, Part};
use crate::profile::InstrumentKind;
use crate::settings::Ending;
use crate::sketch::{AnchorKind, Gesture, Sketch};
use crate::style::Style;
use crate::timing::ms_to_ticks;

/// How an instrument's playing follows the phrase (the instrument
/// profiles of S3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Expression {
    /// How far loudness follows the phrase, against the style's range.
    pub dynamics: f32,
    /// The most a note moves ahead or lays back with the phrase, in ms.
    pub timing_ms: f32,
    /// How late a phrase's last note before a breath comes, in ms (at a
    /// Home ending; half that at a landing inside the new bars).
    pub last_ms: f32,
    /// Steps up are slurred (one-note leads).
    pub slur: bool,
    /// A chord's notes come in low to high over this many ms (0: at once).
    pub roll_ms: f32,
    /// Velocity the top voice of a chord gains (the inner voices lose most
    /// of it).
    pub voicing: f32,
}

impl Expression {
    /// Nothing moves (drums).
    pub const NONE: Expression = Expression {
        dynamics: 0.0,
        timing_ms: 0.0,
        last_ms: 0.0,
        slur: false,
        roll_ms: 0.0,
        voicing: 0.0,
    };
}

impl InstrumentKind {
    /// How this kind of instrument plays a phrase.
    pub const fn expression(self) -> Expression {
        match self {
            InstrumentKind::OneNote => Expression {
                dynamics: 1.0,
                timing_ms: 8.0,
                last_ms: 14.0,
                slur: true,
                roll_ms: 0.0,
                voicing: 0.0,
            },
            InstrumentKind::Chordal => Expression {
                dynamics: 0.9,
                timing_ms: 6.0,
                last_ms: 14.0,
                slur: false,
                roll_ms: 10.0,
                voicing: 4.0,
            },
            InstrumentKind::Bass => Expression {
                dynamics: 0.6,
                timing_ms: 3.0,
                last_ms: 8.0,
                slur: false,
                roll_ms: 0.0,
                voicing: 0.0,
            },
            InstrumentKind::Chords => Expression {
                dynamics: 0.7,
                timing_ms: 4.0,
                last_ms: 16.0,
                slur: false,
                roll_ms: 14.0,
                voicing: 4.0,
            },
            InstrumentKind::Percussive => Expression::NONE,
        }
    }
}

/// What [`shape`] plays the new notes from.
pub struct Shaping<'a> {
    pub sketch: &'a Sketch,
    /// Your bars: the new notes start after them.
    pub yours: usize,
    pub meter: Meter,
    pub bpm: f64,
    /// Keep Accents: the loudness stays as the 0C rules made it.
    pub keep_accents: bool,
}

/// One phrase of the new notes: its onsets, in order, and the one it
/// leans to.
struct Phrase {
    onsets: Vec<Tick>,
    climax: Tick,
    /// It holds the plan's high point.
    peak: bool,
}

/// Where a note of the phrase is: -0.5 at its start, +0.5 at its high
/// point, -0.6 at its end; and how far along the climb (0 to 1) or the
/// fall (0 to 1) it is.
fn arch(p: &Phrase, t: Tick) -> (f64, f64, f64) {
    let (s, e, c) = (p.onsets[0], *p.onsets.last().unwrap_or(&t), p.climax);
    if e <= s {
        return (0.0, 1.0, 0.0);
    }
    if t <= c {
        let x = if c > s {
            (t - s) as f64 / (c - s) as f64
        } else {
            1.0
        };
        (-0.5 + x, x, 0.0)
    } else {
        let y = if e > c {
            (t - c) as f64 / (e - c) as f64
        } else {
            1.0
        };
        (0.5 - 1.1 * y, 1.0, y)
    }
}

/// The new notes of `part` (decisions and the same notes realised, in the
/// same order) played with the phrase and the instrument; see the module.
pub fn shape(part: &mut Part, events: &[Event], style: &Style, sh: &Shaping) {
    let depth = f64::from(style.playing.gesture);
    let ex = part.kind.expression();
    let n = part.decisions.len().min(events.len());
    if depth <= 0.0 || n == 0 || ex == Expression::NONE {
        return;
    }
    let sk = sh.sketch;
    let bar = sh.meter.bar_ticks().max(1);
    let beat = sh.meter.beat_ticks().max(1);
    let from = sh.yours as Tick * bar;
    let k = f64::from(part.kind.profile().velocity_response);
    let range = f64::from(style.playing.velocity_range);
    let mut order: Vec<usize> = (0..n).filter(|&i| part.decisions[i].at >= from).collect();
    order.sort_by_key(|&i| (part.decisions[i].at, events[i].pitch));
    let mut onsets: Vec<Tick> = order.iter().map(|&i| part.decisions[i].at).collect();
    onsets.dedup();
    if onsets.is_empty() {
        return;
    }
    let top = |t: Tick| {
        order
            .iter()
            .filter(|&&i| part.decisions[i].at == t)
            .map(|&i| events[i].pitch)
            .max()
            .unwrap_or(0)
    };

    // The phrases: up to each landing, then up to the last note.
    let tol = STEP / 2;
    let mut ends: Vec<Tick> = sk
        .anchors
        .iter()
        .filter(|a| matches!(a.kind, AnchorKind::Landing | AnchorKind::Close))
        .map(|a| Tick::from(a.on) * STEP)
        .collect();
    ends.sort_unstable();
    ends.dedup();
    let peak_at = sk
        .anchors
        .iter()
        .find(|a| a.kind == AnchorKind::Peak)
        .map(|a| Tick::from(a.on) * STEP);
    let mut phrases: Vec<Phrase> = Vec::new();
    let mut at_phrase = usize::MAX;
    for &t in &onsets {
        let p = ends.iter().filter(|&&e| e + tol < t).count();
        if p != at_phrase {
            at_phrase = p;
            phrases.push(Phrase {
                onsets: Vec::new(),
                climax: t,
                peak: false,
            });
        }
        if let Some(ph) = phrases.last_mut() {
            ph.onsets.push(t);
        }
    }
    for ph in phrases.iter_mut() {
        let (s, e) = (ph.onsets[0], *ph.onsets.last().unwrap_or(&ph.onsets[0]));
        let held = peak_at.filter(|&a| a + tol >= s && a <= e + tol);
        ph.peak = held.is_some();
        ph.climax = match held {
            Some(a) => *ph
                .onsets
                .iter()
                .min_by_key(|&&t| (t - a).abs())
                .unwrap_or(&s),
            // The highest note, the later of two.
            None => *ph.onsets.iter().max_by_key(|&&t| (top(t), t)).unwrap_or(&s),
        };
    }
    let home = matches!(sk.gestures.last(), Some(Gesture::Close(Ending::Home)));
    let gesture_of = |t: Tick| {
        let b = (t / bar) as usize;
        b.checked_sub(sh.yours)
            .and_then(|g| sk.gestures.get(g))
            .copied()
    };

    for (pi, ph) in phrases.iter().enumerate() {
        let last_phrase = pi + 1 == phrases.len();
        let weight = if ph.peak { 1.0 } else { 0.7 };
        let e = *ph.onsets.last().unwrap_or(&ph.climax);
        for (oi, &t) in ph.onsets.iter().enumerate() {
            let (a, x, y) = arch(ph, t);
            let next = ph
                .onsets
                .get(oi + 1)
                .copied()
                .or_else(|| phrases.get(pi + 1).map(|q| q.onsets[0]));
            let here: Vec<usize> = order
                .iter()
                .copied()
                .filter(|&i| part.decisions[i].at == t)
                .collect();

            // Timing, in ms: ahead on the climb, a hair late on the high
            // point, laid back after it, late on the last note before a
            // breath.
            let t_ms = f64::from(ex.timing_ms) * depth;
            let climbing = ph.onsets.len() >= 3;
            let mut off = if !climbing {
                0.0
            } else if t < ph.climax {
                -t_ms * (PI * x).sin() * 0.7
            } else if t == ph.climax {
                0.4 * t_ms
            } else {
                0.5 * t_ms * y
            };
            let breath = next.is_none_or(|nx| nx - t >= beat);
            if t == e && ph.onsets.len() >= 2 && breath {
                let full = last_phrase && home;
                off += f64::from(ex.last_ms) * depth * if full { 1.0 } else { 0.5 };
            }

            // Loudness: the phrase's arch in place of the fixed four-bar
            // rise and fall, the high point leaning a little more.
            let fixed = ((PI * (t.rem_euclid(4 * bar) as f64 / (4 * bar) as f64)).sin() - 0.5)
                * range
                * 0.9
                * k;
            let mut dv = depth * a * weight * range * 0.9 * k * f64::from(ex.dynamics)
                - depth.min(1.0) * fixed;
            if ph.peak && t == ph.climax {
                dv += 4.0 * k * depth;
            }

            // Length.
            let g = gesture_of(t);
            let hold_back = g == Some(Gesture::HoldBack);
            let longest_in_bar = order
                .iter()
                .filter(|&&i| part.decisions[i].at / bar == t / bar)
                .map(|&i| part.decisions[i].len)
                .max()
                .unwrap_or(0);

            // A chord: low to high, the top voice on top.
            let mut voices = here.clone();
            voices.sort_by_key(|&i| events[i].pitch);
            let m = voices.len();
            let mean = voices
                .iter()
                .map(|&i| f64::from(part.decisions[i].velocity))
                .sum::<f64>()
                / m.max(1) as f64;
            let soft = mean < 70.0;
            let gap = next.map(|nx| nx - t);
            let mono = !part.kind.profile().overlap;
            // The step to the next note, for a slur.
            let up = order
                .iter()
                .find(|&&q| Some(part.decisions[q].at) == next)
                .map(|&q| i32::from(events[q].pitch));
            for (j, &i) in voices.iter().enumerate() {
                let mut o = off;
                let mut v = dv;
                if m >= 2 && ex.roll_ms > 0.0 {
                    o += f64::from(ex.roll_ms)
                        * depth
                        * (j as f64 / (m - 1) as f64)
                        * if soft { 1.3 } else { 1.0 };
                    if j + 1 == m {
                        v += f64::from(ex.voicing) * k * depth;
                    } else if j > 0 {
                        v -= 0.75 * f64::from(ex.voicing) * k * depth;
                    }
                }
                let d = &mut part.decisions[i];
                d.timing.play += ms_to_ticks(o, sh.bpm) as f32;
                if !sh.keep_accents {
                    d.velocity = (f64::from(d.velocity) + v).round().clamp(20.0, 127.0) as u8;
                }
                if ex.slur && m == 1 && !hold_back {
                    let step = up.map(|p| p - i32::from(events[i].pitch));
                    if let (Some(gp), Some(step)) = (gap, step) {
                        if (1..=2).contains(&step) && t < ph.climax.max(e) {
                            d.len = d.len.max(gp);
                        }
                    }
                }
                if hold_back && d.len < longest_in_bar {
                    d.len = ((d.len as f64 * 0.8) as Tick).max(STEP / 3);
                }
                if ph.peak && t == ph.climax {
                    let longer = (d.len as f64 * if mono { 1.15 } else { 1.1 }) as Tick;
                    d.len = match gap {
                        Some(gp) if mono => longer.min(gp).max(d.len.min(gp)),
                        _ => longer,
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Motion, NoteDecision, PartId, Role, Source};
    use crate::sketch::Anchor;
    use crate::style::{GROOVE, MINIMAL};
    use crate::timing::Timing;
    use mpfree_context::PartRole;

    const FOUR: Meter = Meter { beats: 4, unit: 4 };
    const BAR: Tick = 16 * STEP;

    fn anchor(kind: AnchorKind, bar: usize, step: i32) -> Anchor {
        Anchor {
            kind,
            bar,
            on: bar as i32 * 16 + step,
            len: 4,
            aim: 0,
            notes: Vec::new(),
            chords: Vec::new(),
        }
    }

    /// One bar of yours, then four new bars: up to a landing on bar 2's
    /// third beat, the high point on bar 4, home on bar 5's first beat.
    fn sketch() -> Sketch {
        Sketch {
            gestures: vec![
                Gesture::Answer,
                Gesture::HoldBack,
                Gesture::Peak,
                Gesture::Close(Ending::Home),
            ],
            plan: Vec::new(),
            anchors: vec![
                anchor(AnchorKind::Landing, 1, 8),
                anchor(AnchorKind::Peak, 3, 6),
                anchor(AnchorKind::Close, 4, 0),
            ],
            rests: Vec::new(),
            surprise: None,
        }
    }

    /// Eighth notes climbing by step to each anchor, all at velocity 90.
    fn line(kind: InstrumentKind) -> (Part, Vec<Event>) {
        let mut notes: Vec<(Tick, u8)> = Vec::new();
        // Bar 2 up to the landing on its third beat.
        for (j, p) in [60u8, 62, 64, 65, 67].iter().enumerate() {
            notes.push((BAR + j as Tick * 2 * STEP, *p));
        }
        // Bar 3 held back: two notes.
        notes.push((2 * BAR, 64));
        notes.push((2 * BAR + 8 * STEP, 62));
        // Bar 4 up to the high point on its second beat, then down.
        for (j, p) in [65u8, 67, 69, 72, 71, 69, 67, 65].iter().enumerate() {
            notes.push((3 * BAR + j as Tick * 2 * STEP, *p));
        }
        // Home.
        notes.push((4 * BAR, 60));
        let mut decisions = Vec::new();
        let mut events = Vec::new();
        for (at, pitch) in notes {
            let d = NoteDecision::new(at, STEP, 90, Role::Scale, Motion::NEAREST);
            events.push(Event {
                at,
                len: STEP,
                pitch,
                velocity: 90,
                role: Role::Scale,
                source: Source::Mpfree,
                timing: Timing::default(),
            });
            decisions.push(d);
        }
        (
            Part {
                id: PartId(0),
                role: PartRole::Melody,
                kind,
                register: 60,
                decisions,
            },
            events,
        )
    }

    fn shaping(sk: &Sketch, keep_accents: bool) -> Shaping<'_> {
        Shaping {
            sketch: sk,
            yours: 1,
            meter: FOUR,
            bpm: 100.0,
            keep_accents,
        }
    }

    fn at(p: &Part, t: Tick) -> &NoteDecision {
        p.decisions.iter().find(|d| d.at == t).unwrap()
    }

    /// The same notes, played: loudness grows to the high point and eases
    /// off to the end, the notes climbing to it come a little early, the
    /// high point a hair late and held longer, a step up is slurred, the
    /// held-back bar is shorter, the last note comes late; nothing else
    /// moves.
    #[test]
    fn a_lead_leans_to_the_high_point_and_breathes_at_the_end() {
        let sk = sketch();
        let (mut p, ev) = line(InstrumentKind::OneNote);
        let before = p.clone();
        shape(&mut p, &ev, &GROOVE, &shaping(&sk, false));
        // The same notes in the same places.
        assert_eq!(
            p.decisions.iter().map(|d| d.at).collect::<Vec<_>>(),
            before.decisions.iter().map(|d| d.at).collect::<Vec<_>>()
        );
        let peak = 3 * BAR + 6 * STEP;
        let v = |t| i32::from(at(&p, t).velocity);
        assert!(v(peak) > v(3 * BAR), "louder at the high point");
        assert!(v(peak) > v(3 * BAR + 14 * STEP), "easing off after it");
        assert!(
            v(peak) > v(BAR + 8 * STEP),
            "the peak's phrase leans furthest"
        );
        let ms = |t| f64::from(at(&p, t).timing.play) / ms_to_ticks(1.0, 100.0);
        assert!(
            ms(3 * BAR + 2 * STEP) < -1.0,
            "ahead on the climb: {}",
            ms(3 * BAR + 2 * STEP)
        );
        assert!(ms(peak) > 0.0, "a hair late on the high point");
        assert!(ms(4 * BAR) > 10.0, "home comes late: {}", ms(4 * BAR));
        // The step up from 65 to 67 is slurred into the next note.
        assert_eq!(at(&p, 3 * BAR).len, 2 * STEP);
        // The held-back bar's shorter note.
        assert!(at(&p, 2 * BAR + 8 * STEP).len <= STEP);
        assert!(at(&p, peak).len > STEP, "the high point held longer");
        assert!(
            p.decisions
                .iter()
                .all(|d| d.timing.play.abs() < ms_to_ticks(30.0, 100.0) as f32),
            "within the timing contract"
        );
    }

    /// Off (gesture 0), on drums, or with Keep Accents for loudness:
    /// nothing moves that shouldn't.
    #[test]
    fn off_drums_and_keep_accents_leave_what_they_should() {
        let sk = sketch();
        let mut flat = GROOVE;
        flat.playing.gesture = 0.0;
        let (mut p, ev) = line(InstrumentKind::OneNote);
        let before = p.clone();
        shape(&mut p, &ev, &flat, &shaping(&sk, false));
        assert_eq!(p, before);
        let (mut d, ev) = line(InstrumentKind::Percussive);
        let before = d.clone();
        shape(&mut d, &ev, &GROOVE, &shaping(&sk, false));
        assert_eq!(d, before);
        let (mut k, ev) = line(InstrumentKind::OneNote);
        shape(&mut k, &ev, &MINIMAL, &shaping(&sk, true));
        assert!(k.decisions.iter().all(|d| d.velocity == 90), "Keep Accents");
        assert!(k.decisions.iter().any(|d| d.timing.play != 0.0));
    }

    /// A chord on keys comes in low to high, the top voice louder and the
    /// inner voice softer.
    #[test]
    fn a_chord_on_keys_is_rolled_and_voiced() {
        let sk = sketch();
        let (mut p, mut ev) = line(InstrumentKind::Chordal);
        // A three-note chord at home.
        for (pitch, _) in [(64u8, 0), (67, 1)] {
            let t = 4 * BAR;
            p.decisions
                .push(NoteDecision::new(t, STEP, 90, Role::Scale, Motion::NEAREST));
            ev.push(Event {
                at: t,
                len: STEP,
                pitch,
                velocity: 90,
                role: Role::Scale,
                source: Source::Mpfree,
                timing: Timing::default(),
            });
        }
        shape(&mut p, &ev, &GROOVE, &shaping(&sk, false));
        let chord: Vec<(u8, f32, u8)> = p
            .decisions
            .iter()
            .zip(&ev)
            .filter(|(d, _)| d.at == 4 * BAR)
            .map(|(d, e)| (e.pitch, d.timing.play, d.velocity))
            .collect();
        let low = chord.iter().find(|c| c.0 == 60).unwrap();
        let mid = chord.iter().find(|c| c.0 == 64).unwrap();
        let high = chord.iter().find(|c| c.0 == 67).unwrap();
        assert!(low.1 < mid.1 && mid.1 < high.1, "{chord:?}");
        assert!(high.2 > low.2 && mid.2 < low.2, "{chord:?}");
    }
}
