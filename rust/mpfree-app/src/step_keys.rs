//! STEP SEQ on the audio loop: the pads' presses as step edits, and their
//! lights (sequencer decision 2; `docs/dev/design/step-pads.md`).
//!
//! While [`App::pads_are_steps`](crate::app::App::pads_are_steps), pad `p`
//! is step `16 * page + p` of the sound on the pads, in the clip Beats
//! shows (pad 1, bottom left, is the page's first step):
//!
//! - **An empty step** gets a note at the pad's velocity (127 with FULL
//!   LEVEL) as the press lands, and sounds once while the transport is
//!   stopped, so a beat can be built without playing it.
//! - **A lit step** comes off, with its locks, when the pad is let go
//!   (holding it will set it, in the next pull request).
//! - **ERASE + a step** clears it, locks included.
//!
//! No clip yet: the first tap on page 1 makes a one-bar clip and launches
//! it, as a tap on Beats does. Each edit is a take: UNDO takes it back.
//!
//! The lights ([`colours`]) are the sound's colour, brighter the harder;
//! purple for a step with locks, dim purple for locks with no note; dim
//! for the rest of a note held over; white for the playhead; a faint glow
//! on the first step of each beat; dark past the clip's end.

use crate::app::StepPads;
use crate::time::{StepAction, StepClip, StepsData, TimeHost};

/// What a press did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pressed {
    /// A line for the log (empty: nothing to say).
    pub log: String,
    /// A line for the screen, in place of the hint: why nothing happened,
    /// or what ERASE cleared.
    pub tell: Option<String>,
    /// Sound the step's sound (a pad) once at this velocity: the
    /// transport is stopped and a note went on.
    pub sound: Option<(u8, u8)>,
}

/// A step pad held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Down {
    sound: u8,
    step: usize,
    /// It was lit: its notes come off on the release.
    remove: bool,
    /// The press sounded the step: the release lets it go.
    sounding: bool,
}

/// The step pads held, from press to release.
#[derive(Debug, Default)]
pub struct StepKeys {
    down: [Option<Down>; 16],
}

/// What a release did: a log line, and whether to let go of the sound the
/// press played.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Released {
    pub log: String,
    /// The sound (a pad) the press sounded.
    pub silence: Option<u8>,
}

impl StepKeys {
    /// Pad `pad` is held as a step.
    pub fn held(&self, pad: usize) -> bool {
        self.down.get(pad).is_some_and(Option::is_some)
    }

    /// Pad `pad` pressed as a step of `s`, `velocity` loud (after Full or
    /// Half Level and the curve). With `erase` (ERASE held) the step is
    /// cleared. `rolling`: the transport plays, so a new note is heard
    /// when it comes round.
    pub fn press(
        &mut self,
        s: StepPads,
        pad: usize,
        velocity: u8,
        erase: bool,
        rolling: bool,
        time: &mut TimeHost,
    ) -> Pressed {
        if pad >= 16 {
            return Pressed::default();
        }
        let step = s.first() + pad;
        let steps = time.steps();
        let clip = steps.clip.as_ref();
        let name = sound_name(s.sound);
        if erase {
            // Held as a step, so its release plays nothing.
            self.down[pad] = Some(Down {
                sound: s.sound,
                step,
                remove: false,
                sounding: false,
            });
            let tell = match time.step_action(StepAction::ClearSteps {
                pad: s.sound,
                from: step,
                to: step + 1,
            }) {
                Ok(line) if line.is_empty() => {
                    format!("ERASE: step {} of {name} is empty", step + 1)
                }
                Ok(line) => format!("{line} (UNDO brings it back)"),
                Err(e) => format!("steps: {e}"),
            };
            return Pressed {
                log: String::new(),
                tell: Some(tell),
                sound: None,
            };
        }
        if let Some(why) = past_the_end(clip, step) {
            self.down[pad] = Some(Down {
                sound: s.sound,
                step,
                remove: false,
                sounding: false,
            });
            return Pressed {
                tell: Some(why),
                ..Pressed::default()
            };
        }
        let lit = clip.is_some_and(|c| c.row(s.sound).get(step).copied().flatten().is_some());
        if lit {
            self.down[pad] = Some(Down {
                sound: s.sound,
                step,
                remove: true,
                sounding: false,
            });
            return Pressed::default();
        }
        let r = time.step_action(StepAction::Tap {
            pad: s.sound,
            step,
            velocity: velocity.max(1),
        });
        let sounding = !rolling && r.as_ref().is_ok_and(|l| !l.is_empty());
        self.down[pad] = Some(Down {
            sound: s.sound,
            step,
            remove: false,
            sounding,
        });
        match r {
            Ok(log) => Pressed {
                log,
                tell: None,
                sound: sounding.then_some((s.sound, velocity.max(1))),
            },
            Err(e) => Pressed {
                log: String::new(),
                tell: Some(format!("steps: {e}")),
                sound: None,
            },
        }
    }

    /// Pad `pad` let go: a lit step pressed comes off now.
    pub fn release(&mut self, pad: usize, time: &mut TimeHost) -> Released {
        let Some(d) = self.down.get_mut(pad).and_then(Option::take) else {
            return Released::default();
        };
        let mut out = Released {
            log: String::new(),
            silence: d.sounding.then_some(d.sound),
        };
        if d.remove {
            // Only if it is still lit: a recording may have changed it.
            let steps = time.steps();
            let lit = steps
                .clip
                .as_ref()
                .is_some_and(|c| c.row(d.sound).get(d.step).copied().flatten().is_some());
            if lit {
                out.log = time
                    .step_action(StepAction::Tap {
                        pad: d.sound,
                        step: d.step,
                        velocity: 1,
                    })
                    .unwrap_or_else(|e| format!("steps: {e}"));
            }
        }
        out
    }
}

/// A sound's name for the lines: the pad's number, as the kit's names are
/// the screen's.
fn sound_name(sound: u8) -> String {
    format!("pad {}", sound + 1)
}

/// Why a press on step `step` changes nothing: it is past the clip's end,
/// or there is no clip and it is not on page 1.
fn past_the_end(clip: Option<&StepClip>, step: usize) -> Option<String> {
    match clip {
        None if step >= crate::app::step_pads::PAGE => Some(
            "STEP SEQ: no clip yet; a step on page 1 (PAD BANK A) starts a one-bar clip"
                .to_string(),
        ),
        None => None,
        Some(c) if step >= c.steps => {
            let pages = c.steps.div_ceil(16).max(1);
            Some(format!(
                "STEP SEQ: {} is {} steps ({pages} page{}); − and + on Beats change its length",
                c.name,
                c.steps,
                if pages == 1 { "" } else { "s" }
            ))
        }
        Some(_) => None,
    }
}

/// The lock colour: purple, at full brightness.
pub const LOCK: [u8; 3] = [0x50, 0x00, 0x7F];
/// The playhead.
pub const PLAYHEAD: [u8; 3] = [0x7F, 0x7F, 0x7F];
/// The first step of a beat, when empty: a faint glow.
pub const BEAT: [u8; 3] = [0x06, 0x06, 0x06];
const DARK: [u8; 3] = [0; 3];

fn scaled(c: [u8; 3], f: f32) -> [u8; 3] {
    c.map(|v| ((v.min(0x7F) as f32 * f.clamp(0.0, 1.0)).round() as u8).min(0x7F))
}

/// How bright a step of `velocity` is: a quarter at the softest, full at
/// 127.
fn brightness(velocity: u8) -> f32 {
    0.25 + 0.75 * f32::from(velocity.min(127)) / 127.0
}

/// The 16 pads' colours for the steps of `s` in `clip` (None: no clip
/// yet). `colour` is the sound's hit colour (its brightest), `playing` the
/// step the playhead is on, if the clip plays.
pub fn colours(
    clip: Option<&StepClip>,
    s: StepPads,
    colour: [u8; 3],
    playing: Option<usize>,
) -> [[u8; 3]; 16] {
    let first = s.first();
    let Some(c) = clip else {
        // A tap on page 1 starts a clip: its beats glow.
        return std::array::from_fn(|p| {
            if first == 0 && p.is_multiple_of(4) {
                BEAT
            } else {
                DARK
            }
        });
    };
    let row = c.row(s.sound);
    let locks = c.locks(s.sound);
    // Steps a note started earlier still sounds on (a tie).
    let mut held_over = vec![false; c.steps];
    for (i, st) in row.iter().enumerate() {
        let Some(st) = st else { continue };
        let reach = st.nudge + st.len;
        let mut k = 1;
        while (k as i64) * crate::time::STEP_TICKS < reach && i + k < c.steps {
            held_over[i + k] = true;
            k += 1;
        }
    }
    std::array::from_fn(|p| {
        let i = first + p;
        if i >= c.steps {
            return DARK;
        }
        if playing == Some(i) {
            return PLAYHEAD;
        }
        let locked = locks.get(i).is_some_and(|&n| n > 0);
        match row.get(i).copied().flatten() {
            Some(st) if locked => scaled(LOCK, brightness(st.velocity)),
            Some(st) => scaled(colour, brightness(st.velocity)),
            None if locked => scaled(LOCK, 0.2),
            None if held_over[i] => scaled(colour, 0.15),
            None if i.is_multiple_of(4) => BEAT,
            None => DARK,
        }
    })
}

/// The step the playhead is on, while the clip plays.
pub fn playing(steps: &StepsData, pos: mpfree_transport::time::Tick) -> Option<usize> {
    steps.clip.as_ref()?.playing(pos)
}

/// The PAD BANK lights while the pads are steps: the page shown lit, the
/// rest dim, and the page playing, when it is another of the four, blinks
/// (`blink` on half the time). Only the two values Akai's app was seen
/// sending for these buttons (1 and 3), so pages past the clip's end are
/// dim too.
pub fn bank_lights(page: usize, playing_page: Option<usize>, blink: bool) -> [u8; 4] {
    use crate::app::{LIGHT_OFF, LIGHT_ON};
    std::array::from_fn(|b| {
        let shown = page % 4 == b;
        let plays = playing_page.is_some_and(|q| q != page && q / 4 == page / 4 && q % 4 == b);
        if shown || (plays && blink) {
            LIGHT_ON
        } else {
            LIGHT_OFF
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::steps::Step;
    use mpfree_seq::Cond;

    const RED: [u8; 3] = [0x7F, 0, 0];

    fn view(sound: u8, page: usize) -> StepPads {
        StepPads {
            sound,
            page,
            follow: false,
            picking: false,
        }
    }

    fn step(velocity: u8, len: i64) -> Option<Step> {
        Some(Step {
            velocity,
            len,
            nudge: 0,
            chance: 100,
            cond: Cond::Always,
            ratchet: 1,
            notes: 1,
        })
    }

    fn clip(steps: usize, row: Vec<Option<Step>>, locks: Vec<usize>) -> StepClip {
        StepClip {
            slot: 0,
            name: "Pattern 1".into(),
            steps,
            song_length: steps as f64 * 240.0,
            song_step: 240.0,
            origin: None,
            rows: vec![(0, row)],
            locks: vec![(0, locks)],
        }
    }

    #[test]
    fn the_pads_light_each_step_as_the_design_says() {
        let mut row = vec![None; 16];
        row[0] = step(127, 240);
        row[1] = step(1, 240);
        row[4] = step(100, 240 * 3); // held over steps 5 and 6
        row[9] = step(127, 240);
        let mut locks = vec![0; 16];
        locks[9] = 1; // a step with locks
        locks[13] = 2; // locks with no note
        let c = clip(13, row, locks);
        let lights = colours(Some(&c), view(0, 0), RED, Some(2));
        assert_eq!(lights[0], RED, "the hardest is the sound's full colour");
        assert_eq!(lights[1], [0x21, 0, 0], "the softest about a quarter of it");
        assert_eq!(lights[2], PLAYHEAD);
        assert_eq!(lights[5], [0x13, 0, 0], "held over: dim");
        assert_eq!(lights[6], [0x13, 0, 0]);
        assert_eq!(lights[7], DARK, "the note ends before step 8");
        assert_eq!(lights[8], BEAT, "an empty beat start glows");
        assert_eq!(lights[9], LOCK);
        assert_eq!(lights[12], BEAT);
        // Past the clip's end (13 steps): dark, locks or not.
        assert_eq!(lights[13], DARK);
        assert_eq!(lights[15], DARK);
        // Page 2 of a one-page clip is all dark.
        assert_eq!(colours(Some(&c), view(0, 1), RED, None), [DARK; 16]);
    }

    #[test]
    fn with_no_clip_page_one_shows_its_beats() {
        let one = colours(None, view(0, 0), RED, None);
        assert_eq!(one[0], BEAT);
        assert_eq!(one[1], DARK);
        assert_eq!(one[12], BEAT);
        assert_eq!(colours(None, view(0, 1), RED, None), [DARK; 16]);
    }

    #[test]
    fn the_page_shown_is_lit_and_the_page_playing_blinks() {
        use crate::app::{LIGHT_OFF, LIGHT_ON};
        let (on, off) = (LIGHT_ON, LIGHT_OFF);
        assert_eq!(bank_lights(1, None, true), [off, on, off, off]);
        assert_eq!(bank_lights(1, Some(1), true), [off, on, off, off]);
        assert_eq!(bank_lights(1, Some(3), true), [off, on, off, on]);
        assert_eq!(bank_lights(1, Some(3), false), [off, on, off, off]);
        // Page 6 shows on B; page 2 playing is in the other four.
        assert_eq!(bank_lights(5, Some(1), true), [off, on, off, off]);
    }

    /// Pads as steps against the time engine: a tap adds a note at the
    /// pad's velocity (making a clip the first time), a lit step comes
    /// off on the release, ERASE clears a step, and a page past the end
    /// says why.
    #[test]
    fn presses_edit_the_clip_beats_shows() {
        let Ok(mut time) = TimeHost::new(44_100, 120.0) else {
            return;
        };
        let mut keys = StepKeys::default();
        let s = view(2, 0);
        let row = |time: &TimeHost| -> Vec<Option<u8>> {
            let d = time.steps();
            let c = d.clip.expect("a clip");
            c.row(2).iter().map(|st| st.map(|st| st.velocity)).collect()
        };
        // Page 2 with no clip: nothing, and why.
        let p = keys.press(view(2, 1), 3, 90, false, false, &mut time);
        assert!(p.tell.is_some_and(|t| t.contains("no clip yet")));
        assert!(keys.held(3));
        assert_eq!(keys.release(3, &mut time), Released::default());
        assert!(time.steps().clip.is_none());
        // Pad 5 is step 5: a new clip, the note at 90, heard once.
        let p = keys.press(s, 4, 90, false, false, &mut time);
        assert_eq!(p.sound, Some((2, 90)));
        assert!(p.log.contains("new clip"), "{}", p.log);
        assert_eq!(row(&time)[4], Some(90));
        assert_eq!(keys.release(4, &mut time).silence, Some(2));
        // Pressed again while playing: it stays until the release, and
        // nothing sounds.
        let p = keys.press(s, 4, 90, false, true, &mut time);
        assert_eq!(p, Pressed::default());
        assert_eq!(row(&time)[4], Some(90));
        let r = keys.release(4, &mut time);
        assert_eq!(r.silence, None);
        assert_eq!(row(&time)[4], None);
        // ERASE + a step clears it; an empty one says so.
        keys.press(s, 8, 64, false, true, &mut time);
        keys.release(8, &mut time);
        let p = keys.press(s, 8, 0, true, true, &mut time);
        assert!(p.tell.is_some_and(|t| t.contains("UNDO")));
        keys.release(8, &mut time);
        assert_eq!(row(&time)[8], None);
        let p = keys.press(s, 8, 0, true, true, &mut time);
        assert!(p.tell.is_some_and(|t| t.contains("is empty")));
        keys.release(8, &mut time);
        // Past the end of the one-bar clip.
        let p = keys.press(view(2, 1), 0, 90, false, true, &mut time);
        assert!(p.tell.is_some_and(|t| t.contains("16 steps (1 page)")));
    }
}
