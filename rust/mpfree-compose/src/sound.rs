//! Sound moves (revision 3, S3, second part): the sound follows the
//! phrase, as a player's touch does, without changing your patch.
//!
//! [`crate::perform`] plays the new notes louder towards the phrase's
//! high point, ahead on the climb and late before a breath. Here the
//! sound moves with them, on three controls in a musician's words
//! ([`Control`]):
//!
//! - **Brightness** opens towards the high point and closes a little after
//!   it, on every new note: the phrase with the plan's peak opens furthest.
//! - **Length**: a phrase's last note before a breath rings on (most at a
//!   Home ending), and a held-back bar's notes are cut a little shorter.
//! - **Space**: a phrase's last note before a rest of two beats or more is
//!   thrown into the echo.
//!
//! A move is an amount (-1 to 1) of the control's safe swing on one new
//! note's step ([`SoundMove`]); how far a swing goes on a sound is its
//! preset's ([`safe_controls`]): which knob is the brightness, whether
//! there is room on it, how far it may go. A move never changes your
//! patch: the app writes the moves as the clip's own lane (Continue's),
//! under your own locks, as a lock that ends lets the knob go back. How
//! much moves is [`SoundLevel`] (Subtle by default, Bold one step away)
//! times the style's gesture and the instrument's reach ([`Reach`]):
//! drums have none.

use crate::event::{Event, Part};
use crate::perform::{arch, phrasing, Phrasing, Shaping};
use crate::profile::InstrumentKind;
use crate::sketch::Gesture;
use crate::style::Style;
use mpfree_transport::time::Tick;

/// What a sound move moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Control {
    /// The filter, or the instrument's tone where it has no filter.
    Brightness,
    /// How long a note rings on after it is let go (its release).
    Length,
    /// How much of it goes to the echo (the track's first send).
    Space,
}

impl Control {
    pub const ALL: [Control; 3] = [Control::Brightness, Control::Length, Control::Space];

    pub fn name(self) -> &'static str {
        match self {
            Control::Brightness => "Brightness",
            Control::Length => "Length",
            Control::Space => "Space",
        }
    }
}

/// How much the sound moves: Continue's Sound row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SoundLevel {
    /// Only the notes' playing moves.
    Off,
    #[default]
    Subtle,
    Bold,
}

impl SoundLevel {
    pub const ALL: [SoundLevel; 3] = [SoundLevel::Off, SoundLevel::Subtle, SoundLevel::Bold];

    pub fn name(self) -> &'static str {
        match self {
            SoundLevel::Off => "Off",
            SoundLevel::Subtle => "Subtle",
            SoundLevel::Bold => "Bold",
        }
    }

    /// The share of each control's safe swing a full move takes.
    pub fn depth(self) -> f32 {
        match self {
            SoundLevel::Off => 0.0,
            SoundLevel::Subtle => 0.5,
            SoundLevel::Bold => 1.0,
        }
    }
}

/// How far each control follows the phrase on a kind of instrument.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reach {
    pub brightness: f32,
    pub length: f32,
    pub space: f32,
}

impl InstrumentKind {
    /// How far this kind of instrument's sound follows a phrase.
    pub const fn reach(self) -> Reach {
        match self {
            InstrumentKind::OneNote => Reach {
                brightness: 1.0,
                length: 0.8,
                space: 1.0,
            },
            InstrumentKind::Chordal => Reach {
                brightness: 0.8,
                length: 0.8,
                space: 0.7,
            },
            InstrumentKind::Chords => Reach {
                brightness: 0.6,
                length: 0.6,
                space: 0.5,
            },
            // A bass opens a little and keeps out of the echo.
            InstrumentKind::Bass => Reach {
                brightness: 0.5,
                length: 0.3,
                space: 0.0,
            },
            InstrumentKind::Percussive => Reach {
                brightness: 0.0,
                length: 0.0,
                space: 0.0,
            },
        }
    }
}

/// One move: `amount` (-1 to 1) of the control's safe swing, on the new
/// note's step at `at` (its written tick), until the next move of the
/// same control or the end of the note.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundMove {
    pub control: Control,
    pub at: Tick,
    pub amount: f32,
}

/// Smaller moves than this are left out.
const LEAST: f32 = 0.04;

/// The sound moves for the new notes of `part` (decisions and the same
/// notes realised, in the same order), in time order; see the module.
/// Empty at Off, on drums, or with the style's gesture at 0.
pub fn sound_moves(
    part: &Part,
    events: &[Event],
    style: &Style,
    sh: &Shaping,
    level: SoundLevel,
) -> Vec<SoundMove> {
    let depth = level.depth() * style.playing.gesture.min(1.2);
    let reach = part.kind.reach();
    let mut out = Vec::new();
    if depth <= 0.0 {
        return out;
    }
    let Some(Phrasing {
        order: _,
        phrases,
        home,
    }) = phrasing(part, events, sh)
    else {
        return out;
    };
    let bar = sh.meter.bar_ticks().max(1);
    let beat = sh.meter.beat_ticks().max(1);
    let gesture_of = |t: Tick| {
        ((t / bar) as usize)
            .checked_sub(sh.yours)
            .and_then(|g| sh.sketch.gestures.get(g))
            .copied()
    };
    let mut push = |control: Control, at: Tick, amount: f32| {
        let amount = amount.clamp(-1.0, 1.0);
        if amount.abs() >= LEAST {
            out.push(SoundMove {
                control,
                at,
                amount,
            });
        }
    };
    for (pi, ph) in phrases.iter().enumerate() {
        let last_phrase = pi + 1 == phrases.len();
        let weight = if ph.peak { 1.0 } else { 0.7 };
        let e = *ph.onsets.last().unwrap_or(&ph.climax);
        for (oi, &t) in ph.onsets.iter().enumerate() {
            let next = ph
                .onsets
                .get(oi + 1)
                .copied()
                .or_else(|| phrases.get(pi + 1).map(|q| q.onsets[0]));
            // Brightness: from the phrase's start (0) up to its high point
            // (1), then a little under where it started by its end.
            let (a, _, _) = arch(ph, t);
            let b = if ph.onsets.len() >= 2 {
                (a + 0.5) * weight
            } else {
                0.0
            };
            push(Control::Brightness, t, b as f32 * depth * reach.brightness);
            // Length and space: the phrase's last note before a breath.
            let gap = next.map(|nx| nx - t);
            let breath = gap.is_none_or(|g| g >= beat);
            if t == e && ph.onsets.len() >= 2 && breath {
                let full = last_phrase && home;
                let l = if full { 1.0 } else { 0.6 };
                push(Control::Length, t, l * depth * reach.length);
                if gap.is_none_or(|g| g >= 2 * beat) {
                    let s = if last_phrase { 1.0 } else { 0.7 };
                    push(Control::Space, t, s * depth * reach.space);
                }
            } else if gesture_of(t) == Some(Gesture::HoldBack) {
                push(Control::Length, t, -0.5 * depth * reach.length);
            }
        }
    }
    out.sort_by_key(|m| (m.at, m.control));
    out
}

/// One of an instrument's knobs, as [`safe_controls`] reads it: what its
/// `ParamInfo` says and where the preset has it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knob<'a> {
    pub id: u16,
    pub name: &'a str,
    /// The page section, such as "Filter" or "Amp env".
    pub group: &'a str,
    pub min: f32,
    pub max: f32,
    /// Even steps in pitch or time (a cutoff, a release).
    pub log: bool,
    /// The preset's value.
    pub value: f32,
}

impl Knob<'_> {
    /// Where the value sits on the knob's travel, 0 to 1.
    pub fn travel(&self) -> f32 {
        unit(self.value, self.min, self.max, self.log)
    }
}

fn unit(v: f32, min: f32, max: f32, log: bool) -> f32 {
    if max <= min {
        return 0.0;
    }
    if log && min > 0.0 {
        ((v.max(min) / min).ln() / (max / min).ln()).clamp(0.0, 1.0)
    } else {
        ((v - min) / (max - min)).clamp(0.0, 1.0)
    }
}

/// How far a full move takes a knob.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Swing {
    /// Up or down this many octaves (a cutoff).
    Octaves(f32),
    /// Up or down this share of the knob's travel (a tone knob).
    Travel(f32),
    /// Longer by this factor at +1, shorter by it at -1 (a release).
    Times(f32),
}

/// A knob a sound move may turn on this preset, and how far.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Safe {
    pub id: u16,
    pub swing: Swing,
    /// The preset's value: a move goes from here.
    pub base: f32,
    /// The knob's range, and the most a move takes it to.
    pub min: f32,
    pub max: f32,
    pub log: bool,
}

impl Safe {
    /// The knob's value for a move of `amount` (-1 to 1): from the
    /// preset's value, within the knob's range.
    pub fn value(&self, amount: f32) -> f32 {
        let a = amount.clamp(-1.0, 1.0);
        let v = match self.swing {
            Swing::Octaves(o) => self.base * 2f32.powf(a * o),
            Swing::Times(f) => self.base * f.powf(a),
            Swing::Travel(t) => {
                let u = (unit(self.base, self.min, self.max, self.log) + a * t).clamp(0.0, 1.0);
                if self.log && self.min > 0.0 {
                    self.min * (self.max / self.min).powf(u)
                } else {
                    self.min + u * (self.max - self.min)
                }
            }
        };
        v.clamp(self.min, self.max)
    }
}

/// The knobs a preset lets sound moves turn: its brightness and its
/// length. Space is the track's send, not the instrument's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SafeControls {
    pub brightness: Option<Safe>,
    pub length: Option<Safe>,
}

impl SafeControls {
    pub fn get(&self, c: Control) -> Option<Safe> {
        match c {
            Control::Brightness => self.brightness,
            Control::Length => self.length,
            Control::Space => None,
        }
    }
}

/// A cutoff this far up its travel has no room to open: the tone knob is
/// used, or none.
const OPEN: f32 = 0.9;
/// A resonance this far up its travel halves the cutoff's swing, so a
/// sweep never whistles.
const RESONANT: f32 = 0.6;
/// A release this long already rings on: Length leaves it alone.
const LONG_MS: f32 = 1_500.0;

/// The safe controls of a preset, from its knobs (names, sections, ranges
/// and the preset's values):
///
/// - **Brightness** is the filter's Cutoff (a log knob, in a section named
///   Filter where there is one) with room to open, an octave each way,
///   half that over its section's resonance past 60 %; else a knob named Brightness or Tone, 30 % of
///   its travel each way; else none.
/// - **Length** is the amp's Release (not the filter envelope's), up to
///   three times or a third as long, left alone from 1.5 s up; else none.
///
/// A preset with neither plays its moves on Space alone (the track's echo
/// send), and its notes' playing.
pub fn safe_controls(knobs: &[Knob]) -> SafeControls {
    let mut out = SafeControls::default();
    let filter = |k: &Knob| k.group.to_ascii_lowercase().contains("filter");
    // A cutoff in a Filter section first; else one anywhere (an acid
    // synth keeps its filter with the rest of the voice).
    let cutoff = knobs
        .iter()
        .filter(|k| k.name == "Cutoff" && k.log && k.min > 0.0)
        .min_by_key(|k| !filter(k));
    let resonance = cutoff
        .and_then(|c| {
            knobs
                .iter()
                .find(|k| k.name == "Resonance" && k.group == c.group)
        })
        .map_or(0.0, |k| k.travel());
    if let Some(k) = cutoff.filter(|k| k.travel() <= OPEN) {
        let o = if resonance > RESONANT { 0.5 } else { 1.0 };
        out.brightness = Some(Safe {
            id: k.id,
            swing: Swing::Octaves(o),
            base: k.value,
            min: k.min,
            max: k.max,
            log: true,
        });
    } else if let Some(k) = knobs
        .iter()
        .find(|k| matches!(k.name, "Brightness" | "Tone") && !k.log)
    {
        out.brightness = Some(Safe {
            id: k.id,
            swing: Swing::Travel(0.3),
            base: k.value,
            min: k.min,
            max: k.max,
            log: false,
        });
    }
    let release = knobs
        .iter()
        .filter(|k| k.name == "Release" && !filter(k) && k.log && k.min > 0.0)
        .min_by_key(|k| !k.group.to_ascii_lowercase().contains("amp"));
    if let Some(k) = release.filter(|k| k.value < LONG_MS) {
        out.length = Some(Safe {
            id: k.id,
            swing: Swing::Times(3.0),
            base: k.value,
            min: k.min,
            max: k.max.min(LONG_MS * 2.0),
            log: true,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perform::tests::{line, shaping, sketch, BAR};
    use crate::style::{GROOVE, MINIMAL};
    use mpfree_context::groove::STEP;

    /// The test line (perform's): bar 2 climbs to a landing on its third
    /// beat, bar 3 holds back, bar 4 climbs to the plan's high point on
    /// its second beat and falls, bar 5 lands home.
    #[test]
    fn the_sound_opens_to_the_high_point_and_the_last_notes_ring_on() {
        let sk = sketch();
        let (p, ev) = line(InstrumentKind::OneNote);
        let sh = shaping(&sk, false);
        let moves = sound_moves(&p, &ev, &GROOVE, &sh, SoundLevel::Bold);
        let of = |c: Control, t: Tick| {
            moves
                .iter()
                .find(|m| m.control == c && m.at == t)
                .map_or(0.0, |m| m.amount)
        };
        let peak = 3 * BAR + 6 * STEP;
        // Brightness climbs to the high point and falls after it.
        let climb: Vec<f32> = (0..4)
            .map(|j| of(Control::Brightness, 3 * BAR + j * 2 * STEP))
            .collect();
        assert!(climb.windows(2).all(|w| w[1] > w[0]), "{climb:?}");
        assert!(of(Control::Brightness, peak) > 0.9, "{moves:?}");
        assert!(of(Control::Brightness, 3 * BAR + 14 * STEP) < of(Control::Brightness, peak));
        // The phrase without the high point opens less.
        let first = (0..5)
            .map(|j| of(Control::Brightness, BAR + j * 2 * STEP))
            .fold(0.0f32, f32::max);
        assert!(first < 0.75 && first > 0.3, "{first}");
        // Home rings on and goes into the echo; the landing inside rings
        // on less; the held-back bar is shorter.
        assert!(of(Control::Length, 4 * BAR) > 0.7);
        assert!(of(Control::Space, 4 * BAR) > 0.9);
        let landing = of(Control::Length, BAR + 8 * STEP);
        assert!(
            landing > 0.3 && landing < of(Control::Length, 4 * BAR),
            "{landing}"
        );
        assert!(of(Control::Length, 2 * BAR) < 0.0);
        // Every move is on a new note's step, in time order, within -1 to 1.
        assert!(moves.iter().all(|m| m.at >= BAR && m.amount.abs() <= 1.0));
        assert!(moves.windows(2).all(|w| w[0].at <= w[1].at));
        // Subtle is half as far.
        let subtle = sound_moves(&p, &ev, &GROOVE, &sh, SoundLevel::Subtle);
        let s_peak = subtle
            .iter()
            .find(|m| m.control == Control::Brightness && m.at == peak)
            .unwrap();
        assert!((s_peak.amount - of(Control::Brightness, peak) / 2.0).abs() < 0.01);
    }

    #[test]
    fn off_drums_and_no_gesture_move_nothing() {
        let sk = sketch();
        let sh = shaping(&sk, false);
        let (p, ev) = line(InstrumentKind::OneNote);
        assert!(sound_moves(&p, &ev, &GROOVE, &sh, SoundLevel::Off).is_empty());
        let (d, dev) = line(InstrumentKind::Percussive);
        assert!(sound_moves(&d, &dev, &GROOVE, &sh, SoundLevel::Bold).is_empty());
        let mut flat = MINIMAL;
        flat.playing.gesture = 0.0;
        assert!(sound_moves(&p, &ev, &flat, &sh, SoundLevel::Bold).is_empty());
        // A bass keeps out of the echo.
        let (b, bev) = line(InstrumentKind::Bass);
        let bass = sound_moves(&b, &bev, &GROOVE, &sh, SoundLevel::Bold);
        assert!(!bass.is_empty() && bass.iter().all(|m| m.control != Control::Space));
    }

    fn knob<'a>(
        id: u16,
        name: &'a str,
        group: &'a str,
        min: f32,
        max: f32,
        log: bool,
        value: f32,
    ) -> Knob<'a> {
        Knob {
            id,
            name,
            group,
            min,
            max,
            log,
            value,
        }
    }

    #[test]
    fn a_filter_with_room_is_the_brightness_and_the_amp_release_the_length() {
        let knobs = [
            knob(1, "Cutoff", "Filter", 20.0, 20_000.0, true, 2_500.0),
            knob(2, "Resonance", "Filter", 0.0, 100.0, false, 20.0),
            knob(3, "Release", "Filter env", 1.0, 10_000.0, true, 295.0),
            knob(4, "Release", "Amp env", 1.0, 10_000.0, true, 246.0),
        ];
        let s = safe_controls(&knobs);
        let b = s.brightness.expect("a cutoff");
        assert_eq!((b.id, b.swing), (1, Swing::Octaves(1.0)));
        assert!((b.value(1.0) - 5_000.0).abs() < 1.0);
        assert!((b.value(-0.5) - 2_500.0 / 2f32.sqrt()).abs() < 1.0);
        assert_eq!(b.value(0.0), 2_500.0, "no move is the preset's own");
        let l = s.length.expect("the amp's release");
        assert_eq!(l.id, 4);
        assert!((l.value(1.0) - 738.0).abs() < 0.5);
        assert!((l.value(-1.0) - 82.0).abs() < 0.5);
    }

    #[test]
    fn an_open_or_resonant_filter_and_a_long_release_are_handled_safely() {
        // Resonant: half the swing.
        let knobs = [
            knob(1, "Cutoff", "Filter", 20.0, 12_000.0, true, 800.0),
            knob(2, "Resonance", "Filter", 0.0, 100.0, false, 80.0),
        ];
        let b = safe_controls(&knobs).brightness.unwrap();
        assert_eq!(b.swing, Swing::Octaves(0.5));
        // Wide open: the tone knob instead.
        let knobs = [
            knob(1, "Cutoff", "Filter", 20.0, 20_000.0, true, 19_000.0),
            knob(5, "Tone", "Tone", -100.0, 100.0, false, 0.0),
            knob(6, "Release", "Model", 20.0, 3_000.0, true, 2_000.0),
        ];
        let s = safe_controls(&knobs);
        let b = s.brightness.unwrap();
        assert_eq!((b.id, b.swing), (5, Swing::Travel(0.3)));
        assert!((b.value(1.0) - 60.0).abs() < 0.01);
        assert!((b.value(-1.0) + 60.0).abs() < 0.01);
        assert_eq!(s.length, None, "a release of 2 s already rings on");
        // An acid synth's cutoff, with the rest of its voice: resonant.
        let knobs = [
            knob(7, "Cutoff", "Synth", 30.0, 5_000.0, true, 400.0),
            knob(8, "Resonance", "Synth", 0.0, 100.0, false, 70.0),
            knob(9, "Decay", "Synth", 200.0, 2_000.0, true, 500.0),
        ];
        let s = safe_controls(&knobs);
        assert_eq!(
            s.brightness.map(|b| (b.id, b.swing)),
            Some((7, Swing::Octaves(0.5)))
        );
        assert_eq!(
            s.length, None,
            "its decay is the filter's sweep, not a release"
        );
        // Nothing to turn.
        assert_eq!(safe_controls(&[]), SafeControls::default());
        // Never past the knob's own range.
        let k = [knob(1, "Cutoff", "Filter", 20.0, 1_200.0, true, 700.0)];
        assert_eq!(safe_controls(&k).brightness.unwrap().value(1.0), 1_200.0);
    }
}
