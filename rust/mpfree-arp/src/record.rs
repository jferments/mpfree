//! Recording a pattern on the pads.
//!
//! The pad layout (owner decision, 2026-11-10), by pad slot (pad 1 = slot 0):
//!
//! | Pads | Writes |
//! |---|---|
//! | 1-9 | the 1st..9th held note (`1`..`9`) |
//! | several of 1-9 together | a chord step (`*`) |
//! | 13 | the next note in the order (`x`) |
//! | 14 | a random held note (`?`) |
//! | 15 | a tie (`_`) |
//! | 16 | a rest (`.`) |
//! | 10-12 | nothing (free for later) |
//!
//! A hit at or above the accent velocity writes an accented step (`!`).
//!
//! Two ways to record:
//!
//! - **Step**: each press writes the next step; there is no clock. Recording
//!   ends when the pattern reaches its length.
//! - **Free time**: the pattern plays and each press lands on the nearest
//!   step (quantised to the rate), overwriting what was there. Holding a pad
//!   past the next step writes ties after it. It loops until Done.
//!
//! The recorder only builds a [`Pattern`]; it never plays sound, so the app
//! keeps the pads sounding as usual while recording. No allocation.

use crate::pattern::{Pattern, Pick, Step, MAX_STEPS};

/// Pad slots with a job while recording (pad number minus one).
pub const PAD_NEXT: u8 = 12;
pub const PAD_RANDOM: u8 = 13;
pub const PAD_TIE: u8 = 14;
pub const PAD_REST: u8 = 15;

/// What a pad slot writes, or None for pads 10-12 and anything above 16.
pub fn step_for_pad(slot: u8) -> Option<Step> {
    match slot {
        0..=8 => Some(Step::play(Pick::Index(slot + 1))),
        PAD_NEXT => Some(Step::play(Pick::Next)),
        PAD_RANDOM => Some(Step::play(Pick::Random)),
        PAD_TIE => Some(Step::Tie),
        PAD_REST => Some(Step::Rest),
        _ => None,
    }
}

/// A short label for a pad slot on the record screen.
pub fn pad_label(slot: u8) -> &'static str {
    const NTH: [&str; 9] = [
        "1st", "2nd", "3rd", "4th", "5th", "6th", "7th", "8th", "9th",
    ];
    match slot {
        0..=8 => NTH[usize::from(slot)],
        PAD_NEXT => "x next",
        PAD_RANDOM => "? random",
        PAD_TIE => "_ tie",
        PAD_REST => ". rest",
        _ => "",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordMode {
    Step,
    FreeTime,
}

/// What a press did, for the screen and the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrote {
    /// Step `index` now holds this step.
    Step { index: usize, step: Step },
    /// The pad has no job while recording.
    Nothing,
    /// Step mode only: the pattern is full; press Done or Undo.
    Full,
}

#[derive(Clone, Copy, Debug)]
pub struct Recorder {
    mode: RecordMode,
    steps: [Step; MAX_STEPS],
    /// Steps written so far (Step mode) or the loop length (Free time).
    written: usize,
    length: usize,
    accent_from: u8,
    /// Per pad slot, while the pad is held: the step its press wrote and
    /// where the press was (Free time), in steps since the pattern started.
    held: [Option<(usize, f64)>; 16],
}

impl Recorder {
    /// Step recording of a new pattern of `length` steps (1..=64).
    pub fn step(length: usize) -> Self {
        Self {
            mode: RecordMode::Step,
            steps: [Step::Rest; MAX_STEPS],
            written: 0,
            length: length.clamp(1, MAX_STEPS),
            accent_from: 110,
            held: [None; 16],
        }
    }

    /// Free-time recording over `base`, which keeps playing and is
    /// overwritten where pads are pressed (its lanes are kept).
    pub fn free_time(base: &Pattern) -> Self {
        let mut r = Self::step(base.len());
        r.mode = RecordMode::FreeTime;
        r.steps[..base.len()].copy_from_slice(base.steps());
        r.written = base.len();
        r
    }

    /// Hits at or above this velocity write accented steps (1..=127; 127
    /// only when the pad is hit as hard as it goes).
    pub fn set_accent_from(&mut self, velocity: u8) {
        self.accent_from = velocity.clamp(1, 127);
    }

    pub fn mode(&self) -> RecordMode {
        self.mode
    }

    /// The pattern's length in steps.
    pub fn length(&self) -> usize {
        self.length
    }

    /// Steps written so far in Step mode; the length in Free time.
    pub fn written(&self) -> usize {
        self.written
    }

    pub fn is_full(&self) -> bool {
        self.mode == RecordMode::Step && self.written == self.length
    }

    /// The steps so far (Step mode) or the whole loop (Free time).
    pub fn steps(&self) -> &[Step] {
        &self.steps[..self.written]
    }

    /// Step mode: a pad press writes the next step. Pressing a 1-9 pad while
    /// another 1-9 pad of the same step is still held turns that step into
    /// a chord instead of writing a new one.
    pub fn press(&mut self, slot: u8, velocity: u8) -> Wrote {
        if self.mode != RecordMode::Step {
            return Wrote::Nothing;
        }
        let Some(step) = step_for_pad(slot) else {
            return Wrote::Nothing;
        };
        if let Some(index) = self.chord_with_held(slot) {
            return self.write(slot, index, Step::play(Pick::All), velocity, 0.0);
        }
        if self.written == self.length {
            return Wrote::Full;
        }
        let index = self.written;
        self.written += 1;
        self.write(slot, index, step, velocity, 0.0)
    }

    /// Free time: a press at `position` (in steps since the pattern started,
    /// e.g. from `Engine::position`) lands on the nearest step.
    pub fn press_at(&mut self, slot: u8, velocity: u8, position: f64) -> Wrote {
        if self.mode != RecordMode::FreeTime {
            return Wrote::Nothing;
        }
        let Some(step) = step_for_pad(slot) else {
            return Wrote::Nothing;
        };
        let index = self.nearest(position);
        if self.chord_with_held(slot) == Some(index) {
            return self.write(slot, index, Step::play(Pick::All), velocity, position);
        }
        self.write(slot, index, step, velocity, position)
    }

    /// Ends a held pad. In Free time, a pad held past the next step writes
    /// ties on the steps it covered (`position` as for `press_at`). In Step
    /// mode the position is ignored; use pad 15 for ties.
    pub fn release(&mut self, slot: u8, position: f64) {
        let Some((start, pressed)) = self.held.get_mut(usize::from(slot)).and_then(Option::take)
        else {
            return;
        };
        if self.mode != RecordMode::FreeTime || !matches!(self.steps[start], Step::Play { .. }) {
            return;
        }
        // A step is tied when the hold lasts past the middle of the step
        // before it ends, i.e. up to the release rounded to the nearest step.
        let reach = (position.round() - pressed.round()).clamp(0.0, self.length as f64) as usize;
        for k in 1..reach {
            self.steps[(start + k) % self.length] = Step::Tie;
        }
    }

    /// Step mode: removes the last step written. Returns false if none.
    pub fn undo(&mut self) -> bool {
        if self.mode != RecordMode::Step || self.written == 0 {
            return false;
        }
        self.written -= 1;
        self.steps[self.written] = Step::Rest;
        for h in &mut self.held {
            if matches!(*h, Some((i, _)) if i == self.written) {
                *h = None;
            }
        }
        true
    }

    /// Starts again: Step mode forgets every step, Free time rests every step.
    pub fn clear(&mut self) {
        self.steps = [Step::Rest; MAX_STEPS];
        self.held = [None; 16];
        if self.mode == RecordMode::Step {
            self.written = 0;
        }
    }

    /// The recorded pattern with `lanes_from`'s lanes, or None if nothing
    /// was written. In Step mode, a pattern stopped early is as long as
    /// what was written.
    pub fn pattern(&self, lanes_from: &Pattern) -> Option<Pattern> {
        let mut p = Pattern::from_steps(self.steps())?;
        p.lanes = lanes_from.lanes;
        Some(p)
    }

    fn nearest(&self, position: f64) -> usize {
        (position.round().rem_euclid(self.length as f64) as usize).min(self.length - 1)
    }

    /// The step that a still-held 1-9 pad wrote, if `slot` is also 1-9.
    fn chord_with_held(&self, slot: u8) -> Option<usize> {
        if slot > 8 {
            return None;
        }
        (0..9u8).filter(|&s| s != slot).find_map(|s| {
            self.held[usize::from(s)].map(|(i, _)| i).filter(|&i| {
                matches!(
                    self.steps[i],
                    Step::Play {
                        pick: Pick::Index(_) | Pick::All,
                        ..
                    }
                )
            })
        })
    }

    fn write(&mut self, slot: u8, index: usize, step: Step, velocity: u8, position: f64) -> Wrote {
        let step = match step {
            Step::Play {
                pick,
                octave,
                gate,
                ratchet,
                chance,
                accent,
            } => {
                // A chord keeps an accent from the first hit.
                let was = matches!(self.steps[index], Step::Play { accent: true, .. })
                    && pick == Pick::All;
                Step::Play {
                    pick,
                    octave,
                    gate,
                    ratchet,
                    chance,
                    accent: accent || was || velocity >= self.accent_from,
                }
            }
            other => other,
        };
        self.steps[index] = step;
        if let Some(h) = self.held.get_mut(usize::from(slot)) {
            *h = Some((index, position));
        }
        Wrote::Step { index, step }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(r: &Recorder) -> String {
        r.pattern(&Pattern::default())
            .map(|p| p.to_text())
            .unwrap_or_default()
    }

    fn tap(r: &mut Recorder, slot: u8, velocity: u8) -> Wrote {
        let w = r.press(slot, velocity);
        r.release(slot, 0.0);
        w
    }

    #[test]
    fn the_pad_layout() {
        let labels: Vec<&str> = (0..16).map(pad_label).collect();
        assert_eq!(labels[..3], ["1st", "2nd", "3rd"]);
        assert_eq!(
            labels[9..],
            ["", "", "", "x next", "? random", "_ tie", ". rest"]
        );
        assert_eq!(step_for_pad(9), None);
        assert_eq!(step_for_pad(16), None);
    }

    #[test]
    fn step_mode_writes_one_step_per_press_and_stops_when_full() {
        let mut r = Recorder::step(6);
        assert_eq!(
            tap(&mut r, 0, 120),
            Wrote::Step {
                index: 0,
                step: Pattern::parse("1!").unwrap().steps()[0]
            }
        );
        tap(&mut r, 2, 80);
        tap(&mut r, PAD_TIE, 80);
        tap(&mut r, PAD_NEXT, 80);
        tap(&mut r, PAD_RANDOM, 127);
        assert_eq!(tap(&mut r, 10, 80), Wrote::Nothing);
        tap(&mut r, PAD_REST, 80);
        assert!(r.is_full());
        assert_eq!(tap(&mut r, 0, 80), Wrote::Full);
        assert_eq!(text(&r), "1! 3 _ x ?! .");
        assert!(r.undo());
        assert_eq!(text(&r), "1! 3 _ x ?!");
        r.clear();
        assert_eq!(r.pattern(&Pattern::default()), None);
        assert!(!r.undo());
    }

    #[test]
    fn pads_pressed_together_write_a_chord() {
        let mut r = Recorder::step(4);
        r.press(0, 90);
        r.press(2, 90); // 1st still held: same step, a chord
        r.press(4, 90);
        r.release(0, 0.0);
        r.release(2, 0.0);
        r.release(4, 0.0);
        tap(&mut r, 1, 90);
        assert_eq!(text(&r), "* 2");
        // x, ? and the rest pad never join a chord.
        r.press(1, 90);
        r.press(PAD_NEXT, 90);
        assert_eq!(text(&r), "* 2 2 x");
    }

    #[test]
    fn free_time_quantises_overwrites_and_ties_held_pads() {
        let base = Pattern::parse("x x x x x x x x | vel 100 60").unwrap();
        let mut r = Recorder::free_time(&base);
        r.set_accent_from(127);
        // Just before step 2: lands on 2. Held until 4.6: ties 3 and 4.
        r.press_at(0, 100, 1.8);
        r.release(0, 4.6);
        // Step 6, held only a little past the next step's start: no tie.
        r.press_at(PAD_REST, 100, 6.2);
        r.release(PAD_REST, 7.3);
        // The second loop: 9.9 is step 10 = step 2 again; a chord with pad 3.
        r.press_at(0, 100, 9.9);
        r.press_at(2, 127, 10.1);
        r.release(0, 10.2);
        r.release(2, 10.2);
        // Held across the loop end, 7.0 to 9.4: ties step 0; step 1 began
        // only 0.4 of a step before the release, so it is left alone.
        r.press_at(1, 100, 7.0);
        r.release(1, 9.4);
        let p = r.pattern(&base).unwrap();
        assert_eq!(p.to_text(), "_ x *! _ _ x . 2 | vel 100 60");
        assert_eq!(
            r.press(0, 100),
            Wrote::Nothing,
            "Step-mode calls do nothing in Free time"
        );
    }
}
