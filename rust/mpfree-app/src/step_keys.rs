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
//! - **A lit step** comes off, with its locks, when the pad is let go, if
//!   it was the only step held and nothing was done while it was.
//! - **A step held** is the Q-Links' on Play ([`StepKeys::held_steps`]);
//!   held together, several steps are set together.
//! - **A later step tapped while one is held**, with no Q-Link turned yet,
//!   is a tie: the held step's note lasts to the end of it (or ends there,
//!   inside the note). A lit step, an earlier one, or any step after a
//!   turn joins the steps held instead.
//! - **ERASE + a step** clears it, locks included; **ERASE pressed while
//!   steps are held** takes their locks off and leaves their notes
//!   ([`StepKeys::erase_locks`]).
//! - **SHIFT + an empty step** holds it with no note: a knob turned then
//!   is a lock with no note.
//! - **COPY + a step** picks it; each step tapped after it, COPY still
//!   held, gets a copy, locks included.
//!
//! **On the synth** the pads are the steps of the whole line
//! ([`crate::time::LINE`]): an empty step gets the pen (the pad or chord
//! picked with STEP SEQ held) and sounds it once while stopped; a lit step
//! comes off whole; ties, ERASE, COPY and locks take the whole step.
//!
//! No clip yet: the first tap on page 1 makes a one-bar clip and launches
//! it, as a tap on Beats does. Each edit is a take: UNDO takes it back.
//!
//! The lights ([`colours`]) are the sound's colour, brighter the harder;
//! purple for a step with locks, dim purple for locks with no note; dim
//! for the rest of a note held over; white for the playhead; a faint glow
//! on the first step of each beat; dark past the clip's end.

use crate::app::step_pads::Pen;
use crate::app::StepPads;
use crate::time::{StepAction, StepClip, StepsData, TimeHost, LINE, STEP_TICKS};
use mpfree_seq::steps::Change;

/// What a press did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pressed {
    /// A line for the log (empty: nothing to say).
    pub log: String,
    /// A line for the screen, in place of the hint: why nothing happened,
    /// or what ERASE, COPY or a tie did.
    pub tell: Option<String>,
    /// Sound these pads once (pad, velocity): the transport is stopped
    /// and a note went on (the step's sound, or on the synth the pen).
    pub sound: Vec<(u8, u8)>,
}

/// How a pad was pressed: its velocity (after Full or Half Level and the
/// curve), the buttons held with it, and whether the transport rolls (a
/// new note is heard when it comes round).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Press {
    pub velocity: u8,
    /// ERASE held: the step is cleared.
    pub erase: bool,
    /// COPY held: the step is picked to copy, or gets the copy.
    pub copy: bool,
    /// SHIFT held: an empty step is held with no note, for a lock with
    /// no note.
    pub shift: bool,
    pub rolling: bool,
}

/// What a held step pad is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A step the Q-Links set while it is held. `remove`: it was lit and
    /// pressed alone, so it comes off on the release, unless a turn, a tie
    /// or another step comes first.
    Set { remove: bool },
    /// Pressed for ERASE, COPY, a tie's end or past the clip's end: the
    /// release does nothing.
    Done,
}

/// A step pad held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Down {
    sound: u8,
    step: usize,
    kind: Kind,
    /// The pads the press sounded: the release lets them go.
    sounding: Pen,
    /// When it went down, to find the step held first.
    order: u32,
}

/// The step pads held, from press to release.
#[derive(Debug, Default)]
pub struct StepKeys {
    down: [Option<Down>; 16],
    /// A Q-Link set the steps held since the first of them went down.
    turned: bool,
    /// COPY: the step picked to copy (its sound and step), while COPY is
    /// held.
    copy_from: Option<(u8, usize)>,
    presses: u32,
    /// The steps held as the screen was last told.
    shown: Option<(u8, Vec<usize>)>,
    /// A keyboard's keys on the pads they play, held while steps of the
    /// line are: the chord they go on as (up to `MAX_PEN`).
    keys: Pen,
    /// The keys still down, a bit a pad: the next key after all are up
    /// starts a new chord.
    keys_down: u16,
    /// Each key's velocity, by pad.
    key_velocity: [u8; 16],
}

/// What a release did: a log line, and whether to let go of the sound the
/// press played.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Released {
    pub log: String,
    /// The pads the press sounded.
    pub silence: Vec<u8>,
}

impl StepKeys {
    /// Pad `pad` is held as a step.
    pub fn held(&self, pad: usize) -> bool {
        self.down.get(pad).is_some_and(Option::is_some)
    }

    /// The steps held to be set, the first held first, and their sound:
    /// what the Q-Links turn on Play. None while none is.
    pub fn held_steps(&self) -> Option<(u8, Vec<usize>)> {
        let mut set: Vec<&Down> = self
            .down
            .iter()
            .flatten()
            .filter(|d| matches!(d.kind, Kind::Set { .. }))
            .collect();
        set.sort_by_key(|d| d.order);
        let sound = set.first()?.sound;
        let mut steps: Vec<usize> = Vec::new();
        for d in set.into_iter().filter(|d| d.sound == sound) {
            if !steps.contains(&d.step) {
                steps.push(d.step);
            }
        }
        Some((sound, steps))
    }

    /// The steps held, when they changed since the screen was last told
    /// (`ToUi::StepsHeld`).
    pub fn held_news(&mut self) -> Option<Option<(u8, Vec<usize>)>> {
        let now = self.held_steps();
        (now != self.shown).then(|| {
            self.shown.clone_from(&now);
            now
        })
    }

    /// A Q-Link set the steps held: none of them comes off on its
    /// release, and a later step tapped joins them rather than tying.
    pub fn turned(&mut self) {
        if self.held_steps().is_some() {
            self.turned = true;
            self.keep_all();
        }
    }

    /// ERASE pressed while steps are held: their locks come off, as one
    /// undo, and their notes stay. None while no step is held; else what
    /// to tell.
    pub fn erase_locks(&mut self, time: &mut TimeHost) -> Option<String> {
        let (sound, steps) = self.held_steps()?;
        self.turned = true;
        self.keep_all();
        let bits = steps
            .iter()
            .filter(|&&i| i < 128)
            .fold(0u128, |b, &i| b | 1u128 << i);
        let (which, has) = if steps.len() == 1 {
            (format!("step {}", steps[0] + 1), "has")
        } else {
            let list: Vec<String> = steps.iter().map(|i| (i + 1).to_string()).collect();
            (format!("steps {}", list.join(", ")), "have")
        };
        Some(
            match time.step_action(StepAction::ClearLocks {
                pad: sound,
                steps: bits,
            }) {
                Ok(l) if l.is_empty() => format!("ERASE: {which} {has} no locks"),
                Ok(_) => format!("ERASE: the locks of {which} are off (UNDO brings them back)"),
                Err(e) => format!("steps: {e}"),
            },
        )
    }

    /// A keyboard's key on pad `pad` (the pad its note plays: the synth's
    /// pad layout) at `velocity`. While steps of the line are held, the
    /// keys held together go on every one of them, in place of what they
    /// held, as one undo (`StepAction::Keys`), and the steps stay on their
    /// release. None while no step of the line is held: the key just plays.
    pub fn key_down(&mut self, pad: u8, velocity: u8, time: &mut TimeHost) -> Option<Pressed> {
        let (sound, steps) = self.held_steps()?;
        if sound != LINE || pad >= 16 {
            return None;
        }
        if self.keys_down == 0 {
            self.keys = Pen::default();
        }
        self.keys.add(pad);
        self.keys_down |= 1 << pad;
        self.key_velocity[usize::from(pad)] = velocity.max(1);
        self.turned = true;
        self.keep_all();
        let bits = steps
            .iter()
            .filter(|&&i| i < 128)
            .fold(0u128, |b, &i| b | 1u128 << i);
        let (mut keys, n) = self.keys.keys(velocity.max(1));
        for k in keys.iter_mut().take(usize::from(n)) {
            k.1 = self.key_velocity[usize::from(k.0)];
        }
        Some(
            match time.step_action(StepAction::Keys {
                steps: bits,
                keys,
                n,
            }) {
                Ok(log) => Pressed {
                    log,
                    ..Pressed::default()
                },
                Err(e) => Pressed {
                    tell: Some(format!("steps: {e}")),
                    ..Pressed::default()
                },
            },
        )
    }

    /// A keyboard's key on pad `pad` let go.
    pub fn key_up(&mut self, pad: u8) {
        if pad < 16 {
            self.keys_down &= !(1 << pad);
        }
    }

    /// Steps of the synth's line are held: a keyboard's keys go on them.
    pub fn line_held(&self) -> bool {
        self.held_steps().is_some_and(|(sound, _)| sound == LINE)
    }

    /// COPY was let go: the step picked is forgotten.
    pub fn forget_copy(&mut self) {
        self.copy_from = None;
    }

    /// Something happened while steps were held: none comes off.
    fn keep_all(&mut self) {
        for d in self.down.iter_mut().flatten() {
            if let Kind::Set { remove } = &mut d.kind {
                *remove = false;
            }
        }
    }

    /// The step held first, of sound `sound`, to be set.
    fn lead(&self, sound: u8) -> Option<Down> {
        self.down
            .iter()
            .flatten()
            .filter(|d| d.sound == sound && matches!(d.kind, Kind::Set { .. }))
            .min_by_key(|d| d.order)
            .copied()
    }

    fn hold(&mut self, pad: usize, sound: u8, step: usize, kind: Kind, sounding: Pen) {
        self.presses = self.presses.wrapping_add(1);
        self.down[pad] = Some(Down {
            sound,
            step,
            kind,
            sounding,
            order: self.presses,
        });
    }

    /// Pad `pad` pressed as a step of `s`, as `p` says (ERASE or COPY
    /// held, the velocity, whether the transport rolls).
    pub fn press(&mut self, s: StepPads, pad: usize, p: Press, time: &mut TimeHost) -> Pressed {
        if pad >= 16 {
            return Pressed::default();
        }
        if !p.copy {
            self.copy_from = None;
        }
        let step = s.first() + pad;
        let steps = time.steps();
        let clip = steps.clip.as_ref();
        let name = sound_name(s.sound);
        let alone = self.down.iter().all(Option::is_none);
        let lead = self.lead(s.sound).filter(|_| !self.turned);
        // Anything pressed while steps are held keeps them.
        self.keep_all();
        if p.erase {
            // Held as a step, so its release plays nothing.
            self.hold(pad, s.sound, step, Kind::Done, Pen::default());
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
                sound: Vec::new(),
            };
        }
        if let Some(why) = past_the_end(clip, step) {
            self.hold(pad, s.sound, step, Kind::Done, Pen::default());
            return Pressed {
                tell: Some(why),
                ..Pressed::default()
            };
        }
        let row = clip.map(|c| c.row(s.sound)).unwrap_or_default();
        let lit = row.get(step).copied().flatten().is_some();
        if p.copy {
            self.hold(pad, s.sound, step, Kind::Done, Pen::default());
            return self.copy(s.sound, step, lit, time);
        }
        if p.shift && !lit {
            // SHIFT + an empty step: held with no note, for a lock with no
            // note.
            self.hold(
                pad,
                s.sound,
                step,
                Kind::Set { remove: false },
                Pen::default(),
            );
            return Pressed {
                log: format!(
                    "steps: step {} of {name} held for a lock with no note",
                    step + 1
                ),
                ..Pressed::default()
            };
        }
        // A later step tapped while one is held, nothing turned yet: the
        // held step's note lasts to the end of it.
        if let Some((lead, note)) =
            lead.and_then(|d| Some((d, row.get(d.step).copied().flatten()?)))
        {
            if step > lead.step && !lit {
                self.hold(pad, s.sound, step, Kind::Done, Pen::default());
                let len = (step - lead.step + 1) as i64 * STEP_TICKS - note.nudge;
                let r = time.step_action(StepAction::Change {
                    pad: s.sound,
                    step: lead.step,
                    change: Change::Length(len.max(1)),
                });
                return match r {
                    Ok(_) => Pressed {
                        log: format!(
                            "steps: step {} of {name} lasts to the end of step {}",
                            lead.step + 1,
                            step + 1
                        ),
                        tell: Some(format!(
                            "Step {} lasts to the end of step {} (UNDO takes it back)",
                            lead.step + 1,
                            step + 1
                        )),
                        sound: Vec::new(),
                    },
                    Err(e) => Pressed {
                        tell: Some(format!("steps: {e}")),
                        ..Pressed::default()
                    },
                };
            }
        }
        if lit {
            self.hold(
                pad,
                s.sound,
                step,
                Kind::Set { remove: alone },
                Pen::default(),
            );
            return Pressed::default();
        }
        let velocity = p.velocity.max(1);
        // The step's sound, or on the synth the pen.
        let pen = if s.line() { s.pen } else { Pen::of(s.sound) };
        if pen.is_empty() {
            self.hold(pad, s.sound, step, Kind::Done, Pen::default());
            return Pressed {
                tell: Some(
                    "STEP SEQ: hold STEP SEQ and play a pad of the synth: new steps play it"
                        .to_string(),
                ),
                ..Pressed::default()
            };
        }
        let r = time.step_action(if s.line() {
            let (keys, n) = pen.keys(velocity);
            StepAction::Enter { step, keys, n }
        } else {
            StepAction::Tap {
                pad: s.sound,
                step,
                velocity,
            }
        });
        let sounding = !p.rolling && r.as_ref().is_ok_and(|l| !l.is_empty());
        let sounded = if sounding { pen } else { Pen::default() };
        self.hold(pad, s.sound, step, Kind::Set { remove: false }, sounded);
        match r {
            Ok(log) => Pressed {
                log,
                tell: None,
                sound: sounded.pads().iter().map(|&p| (p, velocity)).collect(),
            },
            Err(e) => Pressed {
                log: String::new(),
                tell: Some(format!("steps: {e}")),
                sound: Vec::new(),
            },
        }
    }

    /// COPY + step `step` of `sound`: the first picks it, each one after
    /// gets a copy.
    fn copy(&mut self, sound: u8, step: usize, lit: bool, time: &mut TimeHost) -> Pressed {
        let from = match self.copy_from {
            Some((was, from)) if was == sound && from != step => from,
            Some((was, from)) if was == sound && from == step => {
                return Pressed {
                    tell: Some(format!(
                        "COPY: step {} is the one picked; tap the steps it goes to",
                        step + 1
                    )),
                    ..Pressed::default()
                };
            }
            _ => {
                self.copy_from = Some((sound, step));
                let tell = if lit {
                    format!("COPY: step {} picked; tap the steps it goes to", step + 1)
                } else {
                    format!(
                        "COPY: step {} picked (it's empty: the steps you tap are cleared)",
                        step + 1
                    )
                };
                return Pressed {
                    tell: Some(tell),
                    ..Pressed::default()
                };
            }
        };
        match time.step_action(StepAction::CopySteps {
            pad: sound,
            from,
            to: step,
            count: 1,
        }) {
            Ok(log) if log.is_empty() => Pressed {
                tell: Some(format!(
                    "COPY: step {} is already the same as step {}",
                    step + 1,
                    from + 1
                )),
                ..Pressed::default()
            },
            Ok(log) => Pressed {
                log,
                tell: Some(format!(
                    "COPY: step {} to step {} (UNDO takes it back)",
                    from + 1,
                    step + 1
                )),
                sound: Vec::new(),
            },
            Err(e) => Pressed {
                tell: Some(format!("steps: {e}")),
                ..Pressed::default()
            },
        }
    }

    /// Pad `pad` let go: a lit step pressed alone comes off now, if
    /// nothing happened while it was held.
    pub fn release(&mut self, pad: usize, time: &mut TimeHost) -> Released {
        let Some(d) = self.down.get_mut(pad).and_then(Option::take) else {
            return Released::default();
        };
        if self.down.iter().all(Option::is_none) {
            self.turned = false;
        }
        let mut out = Released {
            log: String::new(),
            silence: d.sounding.pads().to_vec(),
        };
        if d.kind == (Kind::Set { remove: true }) {
            // Only if it is still lit: a recording may have changed it.
            let steps = time.steps();
            let lit = steps
                .clip
                .as_ref()
                .is_some_and(|c| c.row(d.sound).get(d.step).copied().flatten().is_some());
            if lit {
                // The whole step, on the synth's line.
                let off = if d.sound == LINE {
                    StepAction::ClearSteps {
                        pad: LINE,
                        from: d.step,
                        to: d.step + 1,
                    }
                } else {
                    StepAction::Tap {
                        pad: d.sound,
                        step: d.step,
                        velocity: 1,
                    }
                };
                out.log = time
                    .step_action(off)
                    .unwrap_or_else(|e| format!("steps: {e}"));
            }
        }
        out
    }
}

/// A sound's name for the lines: the pad's number, as the kit's names are
/// the screen's.
fn sound_name(sound: u8) -> String {
    if sound == LINE {
        "the line".to_string()
    } else {
        format!("pad {}", sound + 1)
    }
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
            pen: Pen::default(),
            page,
            follow: false,
            picking: false,
            copying: false,
            copy_page: None,
        }
    }

    /// A press `velocity` loud, with ERASE held or not, rolling or not.
    fn at(velocity: u8, erase: bool, rolling: bool) -> Press {
        Press {
            velocity,
            erase,
            copy: false,
            shift: false,
            rolling,
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
            on: true,
            ramp: 0,
            join: mpfree_seq::Join::Attack,
            notes: 1,
            low: 0,
            high: 0,
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
            lock_values: Vec::new(),
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
        let p = keys.press(view(2, 1), 3, at(90, false, false), &mut time);
        assert!(p.tell.is_some_and(|t| t.contains("no clip yet")));
        assert!(keys.held(3));
        assert_eq!(keys.release(3, &mut time), Released::default());
        assert!(time.steps().clip.is_none());
        // Pad 5 is step 5: a new clip, the note at 90, heard once.
        let p = keys.press(s, 4, at(90, false, false), &mut time);
        assert_eq!(p.sound, [(2, 90)]);
        assert!(p.log.contains("new clip"), "{}", p.log);
        assert_eq!(row(&time)[4], Some(90));
        assert_eq!(keys.release(4, &mut time).silence, [2]);
        // Pressed again while playing: it stays until the release, and
        // nothing sounds.
        let p = keys.press(s, 4, at(90, false, true), &mut time);
        assert_eq!(p, Pressed::default());
        assert_eq!(row(&time)[4], Some(90));
        let r = keys.release(4, &mut time);
        assert!(r.silence.is_empty());
        assert_eq!(row(&time)[4], None);
        // ERASE + a step clears it; an empty one says so.
        keys.press(s, 8, at(64, false, true), &mut time);
        keys.release(8, &mut time);
        let p = keys.press(s, 8, at(0, true, true), &mut time);
        assert!(p.tell.is_some_and(|t| t.contains("UNDO")));
        keys.release(8, &mut time);
        assert_eq!(row(&time)[8], None);
        let p = keys.press(s, 8, at(0, true, true), &mut time);
        assert!(p.tell.is_some_and(|t| t.contains("is empty")));
        keys.release(8, &mut time);
        // Past the end of the one-bar clip.
        let p = keys.press(view(2, 1), 0, at(90, false, true), &mut time);
        assert!(p.tell.is_some_and(|t| t.contains("16 steps (1 page)")));
    }

    /// On the synth the pads are the whole line's steps: an empty step
    /// gets the pen (a chord here) at the press's velocity and sounds each
    /// of its pads once while stopped; a tie lasts every note; a lit step
    /// pressed alone comes off whole on its release; ERASE clears the
    /// whole step; with no pen, a press says how to pick one.
    #[test]
    fn on_the_synth_a_step_takes_the_pen_and_comes_off_whole() {
        let Ok(mut time) = TimeHost::new(44_100, 120.0) else {
            return;
        };
        let mut keys = StepKeys::default();
        let mut pen = Pen::of(0);
        pen.add(4);
        pen.add(7);
        pen.add(4);
        let s = StepPads {
            pen,
            ..view(LINE, 0)
        };
        let line = |time: &TimeHost| -> Vec<Option<usize>> {
            let c = time.steps().clip.expect("a clip");
            c.row(LINE).iter().map(|st| st.map(|st| st.notes)).collect()
        };
        // Step 1: a new clip with the chord at 90, each pad heard once.
        let p = keys.press(s, 0, at(90, false, false), &mut time);
        assert_eq!(p.sound, [(0, 90), (4, 90), (7, 90)]);
        assert!(p.log.contains("new clip"), "{}", p.log);
        assert_eq!(line(&time)[0], Some(3));
        assert_eq!(keys.release(0, &mut time).silence, [0, 4, 7]);
        let c = time.steps().clip.unwrap();
        for pad in [0, 4, 7] {
            assert_eq!(c.row(pad)[0].map(|st| st.velocity), Some(90), "pad {pad}");
        }
        // Held, then step 4 tapped: every note lasts to its end.
        keys.press(s, 0, at(90, false, true), &mut time);
        keys.press(s, 3, at(90, false, true), &mut time);
        keys.release(3, &mut time);
        keys.release(0, &mut time);
        let c = time.steps().clip.unwrap();
        for pad in [0, 4, 7] {
            assert_eq!(c.row(pad)[0].map(|st| st.len), Some(4 * STEP_TICKS));
        }
        assert_eq!(line(&time)[0], Some(3), "the tie kept the step");
        // Lit and pressed alone: the whole step comes off on the release.
        keys.press(s, 0, at(90, false, true), &mut time);
        assert_eq!(line(&time)[0], Some(3));
        assert!(keys.release(0, &mut time).silence.is_empty());
        assert_eq!(line(&time)[0], None);
        // ERASE + a step clears the whole step.
        keys.press(s, 8, at(64, false, true), &mut time);
        keys.release(8, &mut time);
        assert_eq!(line(&time)[8], Some(3));
        let p = keys.press(s, 8, at(0, true, true), &mut time);
        assert!(p.tell.is_some_and(|t| t.contains("UNDO")));
        keys.release(8, &mut time);
        assert_eq!(line(&time)[8], None);
        // No pen: a press says how to pick one, and adds nothing.
        let p = keys.press(view(LINE, 0), 2, at(90, false, true), &mut time);
        assert!(p.tell.is_some_and(|t| t.contains("hold STEP SEQ")));
        keys.release(2, &mut time);
        assert_eq!(line(&time)[2], None);
    }

    /// On the synth, a keyboard's keys played while line steps are held go
    /// on them, the keys held together as a chord (each at its own
    /// velocity), one undo for the chord; the steps stay on their release.
    /// With no step held, or on a sound's steps, a key does nothing here.
    #[test]
    fn keys_played_while_line_steps_are_held_go_on_them() {
        use mpfree_time_ui::view::TimeAction;
        let Ok(mut time) = TimeHost::new(44_100, 120.0) else {
            return;
        };
        let mut keys = StepKeys::default();
        let s = StepPads {
            pen: Pen::of(2),
            ..view(LINE, 0)
        };
        let pads = |time: &TimeHost, step: usize| -> Vec<(u8, u8)> {
            let c = time.steps().clip.expect("a clip");
            (0..16u8)
                .filter_map(|p| Some((p, c.row(p).get(step).copied().flatten()?.velocity)))
                .collect()
        };
        // Nothing held: a key just plays.
        assert!(keys.key_down(4, 100, &mut time).is_none());
        keys.key_up(4);
        // Steps 1 and 5 put on with the pen, then both held: keys 4 and 7
        // together go on both, in place of the pen.
        for step in [0, 4] {
            keys.press(s, step, at(90, false, true), &mut time);
            keys.release(step, &mut time);
        }
        assert_eq!(pads(&time, 0), [(2, 90)]);
        keys.press(s, 0, at(90, false, true), &mut time);
        keys.press(s, 4, at(90, false, true), &mut time);
        assert!(keys.line_held());
        let p = keys.key_down(4, 80, &mut time).expect("held");
        assert!(
            p.log
                .contains("steps 1, 5 of the line: pads 5 from the keys"),
            "{}",
            p.log
        );
        keys.key_down(7, 120, &mut time).expect("held");
        assert_eq!(pads(&time, 0), [(4, 80), (7, 120)]);
        assert_eq!(pads(&time, 4), [(4, 80), (7, 120)]);
        // All keys up: the next key starts a new chord.
        keys.key_up(4);
        keys.key_up(7);
        keys.key_down(9, 100, &mut time).expect("held");
        assert_eq!(pads(&time, 0), [(9, 100)]);
        keys.key_up(9);
        // Let go: the steps stay (the keys kept them).
        keys.release(0, &mut time);
        keys.release(4, &mut time);
        assert!(!keys.line_held());
        assert_eq!(pads(&time, 4), [(9, 100)]);
        // One UNDO takes the keys' chords back to the pen.
        time.act(TimeAction::Undo).unwrap();
        assert_eq!(pads(&time, 0), [(2, 90)]);
        assert_eq!(pads(&time, 4), [(2, 90)]);
        // A sound's steps (a kit) take no keys.
        let kit = view(3, 0);
        keys.press(kit, 8, at(90, false, true), &mut time);
        assert!(!keys.line_held());
        assert!(keys.key_down(4, 100, &mut time).is_none());
        keys.release(8, &mut time);
    }

    /// The steps of sound 2 in the clip Beats shows: (step, velocity,
    /// length).
    fn notes(time: &TimeHost) -> Vec<(usize, u8, i64)> {
        let d = time.steps();
        let Some(c) = d.clip else { return Vec::new() };
        c.row(2)
            .iter()
            .enumerate()
            .filter_map(|(i, st)| st.map(|st| (i, st.velocity, st.len)))
            .collect()
    }

    /// A lit step held and set by a Q-Link stays on its release; held
    /// with a later empty step tapped, its note lasts to the end of that
    /// step (and ends inside it); a lit step or an earlier one joins the
    /// steps held, and a step after a turn joins too.
    #[test]
    fn holding_steps_sets_them_ties_them_and_keeps_them() {
        let Ok(mut time) = TimeHost::new(44_100, 120.0) else {
            return;
        };
        let mut keys = StepKeys::default();
        let s = view(2, 0);
        for pad in [0, 4, 8] {
            keys.press(s, pad, at(100, false, true), &mut time);
            keys.release(pad, &mut time);
        }
        assert_eq!(notes(&time).len(), 3);
        // Held alone, step 1 is the Q-Links'; turned, it stays.
        keys.press(s, 0, at(100, false, true), &mut time);
        assert_eq!(keys.held_steps(), Some((2, vec![0])));
        keys.turned();
        keys.release(0, &mut time);
        assert_eq!(keys.held_steps(), None);
        assert_eq!(notes(&time).len(), 3, "turned: it stays");
        // Hold step 1, tap step 3: step 1 lasts to the end of step 3.
        keys.press(s, 0, at(100, false, true), &mut time);
        let p = keys.press(s, 2, at(100, false, true), &mut time);
        assert!(p
            .tell
            .is_some_and(|t| t.contains("lasts to the end of step 3")));
        assert_eq!(notes(&time)[0], (0, 100, 3 * 240));
        assert_eq!(
            keys.held_steps(),
            Some((2, vec![0])),
            "a tie's end is not held"
        );
        keys.release(2, &mut time);
        // Inside the note: it ends there.
        keys.press(s, 1, at(100, false, true), &mut time);
        assert_eq!(notes(&time)[0], (0, 100, 2 * 240));
        keys.release(1, &mut time);
        keys.release(0, &mut time);
        assert_eq!(notes(&time).len(), 3, "a tie keeps the held step");
        // Hold step 5, then the lit step 9: both are held, nothing ties.
        keys.press(s, 4, at(100, false, true), &mut time);
        keys.press(s, 8, at(100, false, true), &mut time);
        assert_eq!(keys.held_steps(), Some((2, vec![4, 8])));
        assert_eq!(notes(&time)[1], (4, 100, 240));
        keys.release(8, &mut time);
        keys.release(4, &mut time);
        assert_eq!(notes(&time).len(), 3, "held together, neither comes off");
        // After a turn, a later empty step joins (with a note of its own).
        keys.press(s, 4, at(100, false, true), &mut time);
        keys.turned();
        keys.press(s, 6, at(80, false, true), &mut time);
        assert_eq!(keys.held_steps(), Some((2, vec![4, 6])));
        assert_eq!(notes(&time)[2], (6, 80, 240));
        keys.release(6, &mut time);
        keys.release(4, &mut time);
        // An earlier step joins too.
        keys.press(s, 8, at(100, false, true), &mut time);
        keys.press(s, 6, at(100, false, true), &mut time);
        assert_eq!(keys.held_steps(), Some((2, vec![8, 6])));
        keys.release(6, &mut time);
        keys.release(8, &mut time);
        assert_eq!(notes(&time).len(), 4);
        // A lit step tapped alone still comes off.
        keys.press(s, 6, at(100, false, true), &mut time);
        keys.release(6, &mut time);
        assert_eq!(notes(&time).len(), 3);
    }

    /// SHIFT + an empty step holds it with no note, so a turn is a lock
    /// with no note; ERASE pressed while steps are held takes their locks
    /// off, as one undo, and leaves their notes.
    #[test]
    fn shift_holds_an_empty_step_and_erase_takes_the_held_steps_locks_off() {
        use mpfree_play::mpfree_param::{OnTrack, RelAddr};
        let Ok(mut time) = TimeHost::new(44_100, 120.0) else {
            return;
        };
        let mut keys = StepKeys::default();
        let s = view(2, 0);
        let tune = RelAddr::new(OnTrack::Pad(2), 2);
        let lock_at = |time: &TimeHost, step: usize| {
            time.steps().clip.and_then(|c| c.lock_value(2, step, tune))
        };
        // A clip with a note on step 1.
        keys.press(s, 0, at(100, false, true), &mut time);
        keys.release(0, &mut time);
        // SHIFT + step 5 (empty): held, no note, nothing sounds.
        let p = keys.press(
            s,
            4,
            Press {
                shift: true,
                ..at(100, false, false)
            },
            &mut time,
        );
        assert!(p.log.contains("lock with no note"), "{}", p.log);
        assert!(p.sound.is_empty());
        assert_eq!(keys.held_steps(), Some((2, vec![4])));
        assert_eq!(notes(&time).len(), 1, "no note");
        time.step_action(StepAction::Lock {
            pad: 2,
            steps: 1 << 4,
            addr: tune,
            value: Some(700.0),
        })
        .expect("a lock");
        keys.turned();
        keys.release(4, &mut time);
        assert_eq!(lock_at(&time, 4), Some(700.0), "a lock with no note");
        assert_eq!(notes(&time).len(), 1);
        // SHIFT + a lit step is a plain hold.
        keys.press(
            s,
            0,
            Press {
                shift: true,
                ..at(100, false, true)
            },
            &mut time,
        );
        time.step_action(StepAction::Lock {
            pad: 2,
            steps: 1,
            addr: tune,
            value: Some(-300.0),
        })
        .expect("a lock");
        keys.turned();
        // ERASE while steps 1 and 5 are held: both lose their locks, as one
        // undo; the note on step 1 stays (SHIFT again: step 5 joins with no
        // note).
        keys.press(
            s,
            4,
            Press {
                shift: true,
                ..at(100, false, true)
            },
            &mut time,
        );
        assert_eq!(keys.held_steps(), Some((2, vec![0, 4])));
        let t = keys.erase_locks(&mut time).expect("steps held");
        assert!(t.contains("the locks of steps 1, 5 are off"), "{t}");
        assert_eq!((lock_at(&time, 0), lock_at(&time, 4)), (None, None));
        assert_eq!(notes(&time).len(), 1, "the note stays");
        let t = keys.erase_locks(&mut time).expect("steps held");
        assert!(t.contains("steps 1, 5 have no locks"), "{t}");
        keys.release(0, &mut time);
        keys.release(4, &mut time);
        assert_eq!(notes(&time).len(), 1, "erased: nothing comes off");
        time.act(mpfree_time_ui::view::TimeAction::Undo)
            .expect("undo");
        assert_eq!(
            (lock_at(&time, 0), lock_at(&time, 4)),
            (Some(-300.0), Some(700.0)),
            "UNDO brings both back"
        );
        assert_eq!(keys.erase_locks(&mut time), None, "nothing held");
    }

    /// COPY + a step picks it; each step after gets a copy; COPY let go
    /// forgets the step.
    #[test]
    fn copy_puts_the_step_picked_on_each_step_tapped() {
        let Ok(mut time) = TimeHost::new(44_100, 120.0) else {
            return;
        };
        let mut keys = StepKeys::default();
        let s = view(2, 0);
        keys.press(s, 0, at(70, false, true), &mut time);
        keys.release(0, &mut time);
        let copy = Press {
            copy: true,
            ..at(100, false, true)
        };
        let p = keys.press(s, 0, copy, &mut time);
        assert!(p.tell.is_some_and(|t| t.contains("step 1 picked")));
        keys.release(0, &mut time);
        assert_eq!(keys.held_steps(), None, "COPY's steps are not the Q-Links'");
        for pad in [4, 12] {
            let p = keys.press(s, pad, copy, &mut time);
            assert!(p.tell.is_some_and(|t| t.contains("UNDO")), "pad {pad}");
            keys.release(pad, &mut time);
        }
        let got: Vec<(usize, u8)> = notes(&time).iter().map(|n| (n.0, n.1)).collect();
        assert_eq!(got, vec![(0, 70), (4, 70), (12, 70)]);
        // COPY let go: the next COPY + a step picks again.
        keys.forget_copy();
        let p = keys.press(s, 8, copy, &mut time);
        assert!(p.tell.is_some_and(|t| t.contains("it's empty")));
        keys.release(8, &mut time);
        keys.press(s, 4, copy, &mut time);
        keys.release(4, &mut time);
        assert_eq!(notes(&time).len(), 2, "the empty step 9 cleared step 5");
    }
}
