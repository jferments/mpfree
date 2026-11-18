//! REC records what is armed (sequencer decision 1, Jay 16 Nov 2026): one
//! target at a time, the notes of the track the pads play or the picked
//! loop, and REC, OVERDUB and PLAY act on it from every page.
//!
//! - **The notes** record into the armed track's clip, as the transport's
//!   Record does. From stopped, REC makes them *ready* (OVERDUB: ready to
//!   add) and PLAY or PLAY START starts the take, after the count-in; the
//!   MPC's way, so REC held and PLAY pressed records at once. While playing,
//!   REC punches in replacing and OVERDUB adding; while recording, REC
//!   punches out and OVERDUB switches between replacing and adding. STOP
//!   lets go of ready. Following another device's Start and Stop while it
//!   is stopped ([`Now::clock`]), the ready take records from its Start
//!   (time/follow.rs), so PLAY keeps it ready and says so.
//! - **A loop** records as the looper does already: REC arms the picked
//!   loop (its first pass starts with PLAY, or on the next bar line while
//!   playing) and OVERDUB overdubs it. The looper's armed loop is its
//!   ready.
//!
//! What moves the arm ([`RecArm::move_to`]): a tap on an Arm pill or a row
//! of the target list, opening the Sequencer's Steps (the notes) or the
//! Looper tab (the loop), and picking a track (its notes). Nothing moves it while a take records (a tap says "Finish the
//! take first"). Pinned, only a tap moves it. When the arm moves while
//! ready, the ready goes with it.
//!
//! SHIFT + a tap on Arm adds the other target ([`RecArm::add`]), so REC
//! records the pads into a clip and the line into a loop in one pass, each
//! as it would alone; SHIFT + a tap on one of the two lets go of it.
//!
//! UNDO while a take records throws the take so far away
//! ([`RecArm::throw`]): recording stops and the music plays on.
//!
//! The arm isn't saved and isn't an undo step; a new or opened project
//! starts with the notes armed, unpinned. SHIFT + REC is still Capture,
//! whatever is armed.
//!
//! **A pad take** ([`RecArm::arm_pad`], sampler R5.7, Jay 18 Nov 2026
//! 08:52Z, one take only): an empty kit pad held + REC arms the sampler's
//! recorder onto that pad for one take, over the arm ([`RecArm::pad`]).
//! The target and both are never touched, so when the take ends (REC or
//! STOP keeps it, UNDO mid-take throws it away, or the recorder ends it
//! itself) REC records what it did before. While it is armed or records
//! it holds like a pin; a tap before it starts calls it off and arms what
//! was tapped; SHIFT + a tap is refused. The recorder is the Sample tab's
//! ([`PadDo`] out, [`PadNow`] back): it starts on its threshold, else once
//! the music plays ([`RecArm::rolled`]).

use mpfree_looper::Command as LoopCommand;
use mpfree_transport::{Command, State};

/// What REC records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    /// The notes of the track the pads play (the time engine's armed
    /// track), into its clip.
    #[default]
    Notes,
    /// The looper's picked loop.
    Loop,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Target::Notes => "the notes",
            Target::Loop => "the loop",
        }
    }

    /// The other target.
    pub fn other(self) -> Target {
        match self {
            Target::Notes => Target::Loop,
            Target::Loop => Target::Notes,
        }
    }
}

/// A pad take armed by holding an empty kit pad + REC: one take onto that
/// pad by the sampler's recorder, over the arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadTake {
    /// The sampler's kit slot (the armed track's kit) and its pad, 0 (A1)
    /// to 127 (H16).
    pub kit: usize,
    pub pad: usize,
    /// The music plays: the recorder was told to go ([`PadDo::Start`]).
    pub started: bool,
    /// The take records (the recorder said so, [`PadNow::Recording`]).
    pub recording: bool,
}

impl PadTake {
    /// "Pad A5".
    pub fn name(&self) -> String {
        pad_name(self.pad)
    }
}

/// A kit pad as the banks name it: "Pad A5" (pad 4), "Pad B1" (pad 16).
pub fn pad_name(pad: usize) -> String {
    format!("Pad {}", crate::app::kit_pad_name(pad))
}

/// What the arm asks of the sampler's recorder (the screen's Sample tab,
/// `SamplePage::pad_take`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadDo {
    /// A take onto the pad with the Rec tab's settings. With its Start on
    /// Threshold it waits for the threshold now; otherwise for
    /// [`PadDo::Start`].
    Arm { kit: usize, pad: usize },
    /// The music plays: the go (the Rec tab's Start then applies: at once,
    /// or the next bar line).
    Start,
    /// REC or STOP: keep what recorded (it lands on the pad), or call the
    /// take off if nothing has.
    End,
    /// UNDO while it records: throw it away.
    Throw,
}

/// What the recorder says back (`SamplePage::take_pad_now`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadNow {
    /// The take records.
    Recording,
    /// It landed, was called off, or couldn't run (the recorder says why
    /// in the log): REC records what it did before.
    Ended,
}

/// How a take records: over what was there, or adding to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Take {
    Replace,
    Add,
}

/// A press that the arm takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Rec,
    Overdub,
    /// PLAY (or PLAY START, `from_start`) while the notes are ready.
    Play {
        from_start: bool,
    },
    /// STOP while the notes are ready.
    Stop,
}

/// What the presses find.
#[derive(Clone, Debug, PartialEq)]
pub struct Now {
    pub state: State,
    /// The transport's next take adds (OVERDUB) rather than replaces.
    pub overdub: bool,
    /// A loop records its first pass or overdubs.
    pub loop_taking: bool,
    /// The picked loop is armed: its first pass waits for PLAY or the bar.
    pub loop_armed: bool,
    /// The armed track's name, for what is said ("Drums").
    pub notes: String,
    /// Following another device's Start and Stop while it is stopped:
    /// its name ("MIDI In"). A ready take records from its Start.
    pub clock: Option<String>,
}

impl Now {
    /// A take records: the notes (or their count-in), or a loop.
    pub fn taking(&self) -> bool {
        matches!(self.state, State::Recording | State::CountIn) || self.loop_taking
    }
}

/// What moves the arm, for [`RecArm::move_to`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mover {
    /// A page with one target opened (the Sequencer's Steps, the Looper
    /// tab).
    Page,
    /// A track picked: its notes.
    Track,
    /// A tap on an Arm pill, a row of the target list, or the loop's own
    /// Record or Overdub: moves it even when pinned.
    Tap,
}

/// What a press does, for the desk to carry out in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Do {
    Transport(Command),
    Looper(LoopCommand),
    /// Punch out and throw away the notes' take so far
    /// (`TimeAction::ThrowTake`).
    ThrowTake,
    /// Let go of a loop that waits for PLAY ([`crate::looping::LoopAction::StopWaiting`]).
    StopWaiting,
    /// To the sampler's recorder, for a pad take.
    Pad(PadDo),
    /// Said on the hint line and in the log.
    Tell(String),
    /// Written to the log only (the hint line says it already).
    Note(String),
}

/// The arm: what REC records, and whether the notes are ready.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecArm {
    /// What REC records; with [`RecArm::both`], always the notes.
    pub target: Target,
    /// SHIFT + Arm added the other target: REC records the notes and the
    /// loop in one pass.
    pub both: bool,
    /// The notes wait for PLAY to record (only with [`Target::Notes`]; a
    /// loop's ready is the looper's own armed loop).
    pub ready: Option<Take>,
    /// Only a tap moves the arm (the target list's Pin the arm).
    pub pinned: bool,
    /// A pad take, for one take over the arm ([`RecArm::arm_pad`]).
    pub pad: Option<PadTake>,
}

/// The transport's mode set for `take`, if it isn't already.
fn mode(take: Take, overdub: bool) -> Option<Do> {
    (overdub != (take == Take::Add)).then_some(Do::Transport(Command::ToggleOverdub))
}

/// The looper's own command for `take`: Record, or Overdub.
fn looped(take: Take) -> Do {
    Do::Looper(match take {
        Take::Replace => LoopCommand::Record,
        Take::Add => LoopCommand::Overdub,
    })
}

impl RecArm {
    /// Whether REC records `t` (alone or with the other).
    pub fn armed(&self, t: Target) -> bool {
        self.pad.is_none() && (self.both || self.target == t)
    }

    /// What REC records, for what is said: "the notes", "the loop", or
    /// "the notes and the loop" (with a pad take armed, what it records
    /// after it).
    pub fn name(&self) -> &'static str {
        if self.both {
            "the notes and the loop"
        } else {
            self.target.name()
        }
    }

    /// A press of REC, OVERDUB, PLAY or STOP, as it finds the transport.
    pub fn press(&mut self, p: Press, now: &Now) -> Vec<Do> {
        if let Some(take) = self.pad {
            return self.press_pad(take, p, now);
        }
        let take = match p {
            Press::Rec => Take::Replace,
            Press::Overdub => Take::Add,
            Press::Stop => {
                self.ready = None;
                return Vec::new();
            }
            Press::Play { from_start } => {
                let Some(take) = self.ready else {
                    return Vec::new();
                };
                if let Some(leader) = &now.clock {
                    let mut out = Vec::new();
                    if from_start {
                        out.push(Do::Transport(Command::Locate(0)));
                    }
                    out.push(Do::Tell(format!(
                        "Waiting for clock: {} records when {leader} starts",
                        now.notes
                    )));
                    return out;
                }
                self.ready = None;
                let mut out: Vec<Do> = mode(take, now.overdub).into_iter().collect();
                if from_start {
                    out.push(Do::Transport(Command::Locate(0)));
                }
                out.push(Do::Transport(Command::Record));
                return out;
            }
        };
        if self.both {
            return self.press_both(take, now);
        }
        if self.target == Target::Loop {
            return vec![looped(take)];
        }
        self.press_notes(take, now)
    }

    /// REC or OVERDUB on the notes alone.
    fn press_notes(&mut self, take: Take, now: &Now) -> Vec<Do> {
        match now.state {
            State::Stopped => {
                self.ready = (self.ready != Some(take)).then_some(take);
                // While ready the hint line says what PLAY records.
                vec![match self.ready {
                    Some(Take::Replace) => {
                        Do::Note(format!("rec: ready, PLAY records {}", now.notes))
                    }
                    Some(Take::Add) => Do::Note(format!("rec: ready, PLAY adds to {}", now.notes)),
                    None => Do::Tell("Not ready to record".to_string()),
                }]
            }
            // Punch in at once.
            State::Playing => {
                self.ready = None;
                let mut out: Vec<Do> = mode(take, now.overdub).into_iter().collect();
                out.push(Do::Transport(Command::Record));
                out
            }
            // REC punches out (the music plays on); OVERDUB switches
            // between replacing and adding.
            State::Recording | State::CountIn => vec![Do::Transport(match take {
                Take::Replace => Command::Record,
                Take::Add => Command::ToggleOverdub,
            })],
        }
    }

    /// REC or OVERDUB with both armed: each does what it does alone, in
    /// step. Stopped, the notes get ready and the loop waits for PLAY (or
    /// both let go). Playing, what records punches out (the loop's first
    /// pass closes on the bar line) or, with nothing recording, both start.
    fn press_both(&mut self, take: Take, now: &Now) -> Vec<Do> {
        if now.state == State::Stopped {
            let was = self.ready;
            let mut out = self.press_notes(take, now);
            match (was, self.ready) {
                (None, Some(_)) if !now.loop_armed => out.push(looped(take)),
                (Some(_), None) => out.push(Do::StopWaiting),
                _ => {}
            }
            return out;
        }
        let notes = matches!(now.state, State::Recording | State::CountIn);
        let looping = now.loop_taking || now.loop_armed;
        if notes || looping {
            let mut out = Vec::new();
            if notes {
                out.extend(self.press_notes(take, now));
            }
            if looping {
                out.push(looped(take));
            }
            return out;
        }
        let mut out = self.press_notes(take, now);
        out.push(looped(take));
        out
    }

    /// Arm `target`, as `by` moves it. Refused while a take records (a tap
    /// says why, a page or a pick says nothing) and, pinned, unless by a
    /// tap. A ready goes with the arm: the notes' to the loop as Record (or
    /// Overdub), an armed loop's to the notes.
    pub fn move_to(&mut self, target: Target, by: Mover, now: &Now) -> Vec<Do> {
        // A pad take holds like a pin; a tap before it records calls it
        // off and arms what was tapped.
        let mut out = Vec::new();
        if let Some(take) = self.pad {
            if by != Mover::Tap {
                return Vec::new();
            }
            if take.recording {
                return vec![self.busy(now)];
            }
            self.pad = None;
            out.push(Do::Pad(PadDo::End));
            out.push(Do::Tell(format!("{} take called off", take.name())));
        }
        out.extend(self.move_arm(target, by, now));
        out
    }

    fn move_arm(&mut self, target: Target, by: Mover, now: &Now) -> Vec<Do> {
        if self.pinned && by != Mover::Tap {
            return Vec::new();
        }
        // Both armed: a page or a pick finds its target armed already; a
        // tap keeps only the one tapped.
        if self.both {
            if by != Mover::Tap {
                return Vec::new();
            }
            return self.drop(target.other(), now);
        }
        if target == self.target {
            return Vec::new();
        }
        if now.taking() {
            return match by {
                Mover::Tap => vec![self.busy(now)],
                _ => Vec::new(),
            };
        }
        self.target = target;
        match target {
            Target::Loop => match self.ready.take() {
                Some(Take::Replace) => vec![Do::Looper(LoopCommand::Record)],
                Some(Take::Add) => vec![Do::Looper(LoopCommand::Overdub)],
                None => Vec::new(),
            },
            Target::Notes if now.loop_armed => {
                self.ready = Some(Take::Replace);
                vec![Do::StopWaiting]
            }
            Target::Notes => Vec::new(),
        }
    }

    /// What a tap says while a take records.
    fn busy(&self, now: &Now) -> Do {
        let what = if let Some(take) = self.pad.filter(|t| t.recording) {
            take.name()
        } else if self.both {
            format!("{} and the loop", now.notes)
        } else if self.target == Target::Notes {
            now.notes.clone()
        } else {
            "the loop".to_string()
        };
        Do::Tell(format!(
            "Finish the take first: REC records {what} until it stops"
        ))
    }

    /// SHIFT + a tap on Arm: add `target` to what REC records, or, with
    /// both armed, let go of it. The only target can't be let go of. A
    /// ready is shared: the loop added to the notes ready waits for PLAY
    /// too, the notes added to an armed loop get ready. Refused while a take
    /// records, pinned or not.
    pub fn add(&mut self, target: Target, now: &Now) -> Vec<Do> {
        if let Some(take) = self.pad {
            return vec![Do::Tell(format!(
                "A pad take records alone: REC ends {}'s, then SHIFT + Arm adds",
                take.name()
            ))];
        }
        if now.taking() {
            return vec![self.busy(now)];
        }
        if self.both {
            return self.drop(target, now);
        }
        if target == self.target {
            return vec![Do::Tell(format!(
                "REC records {}: SHIFT + Arm on another records both",
                self.name()
            ))];
        }
        self.both = true;
        self.target = Target::Notes;
        let mut out = Vec::new();
        match (target, self.ready) {
            (Target::Loop, Some(take)) => out.push(looped(take)),
            (Target::Notes, None) if now.loop_armed => self.ready = Some(Take::Replace),
            _ => {}
        }
        out.push(Do::Note(format!(
            "rec: {} and the loop armed: REC records both",
            now.notes
        )));
        out
    }

    /// With both armed, let go of `t`: the other records alone. The
    /// notes let go of their ready; the loop of its wait for PLAY.
    fn drop(&mut self, t: Target, now: &Now) -> Vec<Do> {
        if now.taking() {
            return vec![self.busy(now)];
        }
        self.both = false;
        self.target = t.other();
        match t {
            Target::Notes => {
                self.ready = None;
                Vec::new()
            }
            Target::Loop if self.ready.is_some() => vec![Do::StopWaiting],
            Target::Loop => Vec::new(),
        }
    }

    /// UNDO while a take records: the take so far is thrown away and the
    /// music plays on (the notes punch out; a loop's first pass or overdub
    /// is undone). None when nothing records: UNDO is the page's.
    pub fn throw(&mut self, now: &Now) -> Option<Vec<Do>> {
        if let Some(take) = self.pad.filter(|t| t.recording) {
            self.pad = None;
            return Some(vec![
                Do::Pad(PadDo::Throw),
                Do::Tell(format!(
                    "UNDO: {}'s take is thrown away; the music plays on",
                    take.name()
                )),
            ]);
        }
        let notes = matches!(now.state, State::Recording | State::CountIn);
        if !notes && !now.loop_taking {
            return None;
        }
        self.ready = None;
        let mut out = Vec::new();
        if notes {
            out.push(Do::ThrowTake);
        }
        if now.loop_taking {
            out.push(Do::Looper(LoopCommand::Undo));
        }
        out.push(Do::Tell(
            "UNDO: the take is thrown away; the music plays on".to_string(),
        ));
        Some(out)
    }

    /// What REC records once a pad take ends, for what is said: "Drums",
    /// "the loop", "Drums and the loop".
    fn back(&self, now: &Now) -> String {
        match (self.both, self.target) {
            (true, _) => format!("{} and the loop", now.notes),
            (false, Target::Notes) => now.notes.clone(),
            (false, Target::Loop) => "the loop".to_string(),
        }
    }

    /// An empty kit pad held + REC: arm the sampler's recorder onto pad
    /// `pad` of kit `kit` for one take. Refused while a take counts in or
    /// records (a pad take too); a notes ready and a loop waiting for PLAY
    /// let go and don't come back (one take at a time). REC with the same
    /// pad again calls it off; another empty pad moves it there. With the
    /// music playing the recorder gets its go at once.
    pub fn arm_pad(&mut self, kit: usize, pad: usize, now: &Now) -> Vec<Do> {
        let mut out = Vec::new();
        if let Some(take) = self.pad {
            if take.recording {
                return vec![self.busy(now)];
            }
            if (take.kit, take.pad) == (kit, pad) {
                return self.press_pad(take, Press::Rec, now);
            }
            out.push(Do::Pad(PadDo::End));
        } else if now.taking() {
            return vec![self.busy(now)];
        }
        self.ready = None;
        if now.loop_armed {
            out.push(Do::StopWaiting);
        }
        let take = PadTake {
            kit,
            pad,
            started: false,
            recording: false,
        };
        self.pad = Some(take);
        out.push(Do::Pad(PadDo::Arm { kit, pad }));
        out.extend(self.rolled(now));
        out.push(Do::Tell(format!(
            "REC → {}: one take, then REC records {} again · REC: off",
            take.name(),
            self.back(now)
        )));
        out
    }

    /// REC, OVERDUB, PLAY or STOP while a pad take is armed. REC and STOP
    /// end it (what recorded lands on the pad; nothing recorded calls it
    /// off), and REC records what it did before. STOP while it records
    /// leaves the music playing (the app doesn't stop the transport);
    /// while only armed, STOP also stops as usual. PLAY does its own job:
    /// the take gets its go once the music plays ([`RecArm::rolled`]).
    fn press_pad(&mut self, take: PadTake, p: Press, now: &Now) -> Vec<Do> {
        let name = take.name();
        let back = self.back(now);
        let ended = |kept: &str| {
            vec![
                Do::Pad(PadDo::End),
                Do::Tell(format!("{kept} · REC records {back} again")),
            ]
        };
        match p {
            Press::Rec | Press::Stop if take.recording => {
                self.pad = None;
                ended(&format!("{name}: the take ends and lands on the pad"))
            }
            Press::Rec => {
                self.pad = None;
                ended(&format!("{name} take called off"))
            }
            Press::Stop => {
                self.pad = None;
                vec![
                    Do::Pad(PadDo::End),
                    Do::Note(format!("rec: {name} take called off by STOP")),
                ]
            }
            Press::Overdub => vec![Do::Tell(format!(
                "OVERDUB doesn't add to a pad take: REC ends {name}'s"
            ))],
            Press::Play { .. } => Vec::new(),
        }
    }

    /// Every period: a pad take waiting for the music gets its go once
    /// the transport plays (PLAY, PLAY START, Arrange's Play, another
    /// device's Start; not during a count-in).
    pub fn rolled(&mut self, now: &Now) -> Vec<Do> {
        match &mut self.pad {
            Some(t) if !t.started && matches!(now.state, State::Playing | State::Recording) => {
                t.started = true;
                vec![Do::Pad(PadDo::Start)]
            }
            _ => Vec::new(),
        }
    }

    /// The recorder said how the pad take is going. True when the arm
    /// changed (the screen is told).
    pub fn pad_now(&mut self, n: PadNow) -> bool {
        match (n, &mut self.pad) {
            (PadNow::Recording, Some(t)) if !t.recording => {
                t.recording = true;
                true
            }
            (PadNow::Ended, Some(_)) => {
                self.pad = None;
                true
            }
            _ => false,
        }
    }

    /// The lights: the notes ready to replace (REC blinks) or to add
    /// (OVERDUB blinks), PLAY blinking with either.
    pub fn ready_lights(&self) -> (bool, bool) {
        (
            self.ready == Some(Take::Replace) || self.pad.is_some(),
            self.ready == Some(Take::Add),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now(state: State) -> Now {
        Now {
            state,
            overdub: false,
            loop_taking: false,
            loop_armed: false,
            notes: "Drums".into(),
            clock: None,
        }
    }

    fn t(c: Command) -> Do {
        Do::Transport(c)
    }

    #[test]
    fn rec_on_the_notes_readies_from_stopped_and_play_starts_the_take() {
        let mut a = RecArm::default();
        assert_eq!(a.target, Target::Notes);
        let stopped = now(State::Stopped);
        let said = a.press(Press::Rec, &stopped);
        assert_eq!(a.ready, Some(Take::Replace));
        assert!(matches!(&said[..], [Do::Note(l)] if l.contains("PLAY records Drums")));
        assert_eq!(a.ready_lights(), (true, false));
        // PLAY starts it after the count-in (the transport's Record).
        assert_eq!(
            a.press(Press::Play { from_start: false }, &stopped),
            [t(Command::Record)]
        );
        assert_eq!(a.ready, None);
        // PLAY START records from bar 1.
        a.press(Press::Rec, &stopped);
        assert_eq!(
            a.press(Press::Play { from_start: true }, &stopped),
            [t(Command::Locate(0)), t(Command::Record)]
        );
        // PLAY with nothing ready does nothing here (the app plays).
        assert!(a
            .press(Press::Play { from_start: false }, &stopped)
            .is_empty());
    }

    /// Following another device's Start and Stop while it is stopped, a
    /// ready take records from its Start: PLAY keeps it ready and says so,
    /// PLAY START moves to bar 1 first, and STOP still lets go.
    #[test]
    fn play_keeps_a_take_ready_while_it_waits_for_the_clock() {
        let mut a = RecArm::default();
        let waiting = Now {
            clock: Some("MIDI In".into()),
            ..now(State::Stopped)
        };
        a.press(Press::Overdub, &waiting);
        let said = Do::Tell("Waiting for clock: Drums records when MIDI In starts".into());
        assert_eq!(
            a.press(Press::Play { from_start: false }, &waiting),
            std::slice::from_ref(&said)
        );
        assert_eq!(a.ready, Some(Take::Add));
        assert_eq!(
            a.press(Press::Play { from_start: true }, &waiting),
            [t(Command::Locate(0)), said]
        );
        assert_eq!(a.ready, Some(Take::Add));
        a.press(Press::Stop, &waiting);
        assert_eq!(a.ready, None);
    }

    #[test]
    fn rec_again_or_stop_lets_go_and_overdub_readies_an_added_take() {
        let mut a = RecArm::default();
        let mut stopped = now(State::Stopped);
        a.press(Press::Rec, &stopped);
        let said = a.press(Press::Rec, &stopped);
        assert_eq!(a.ready, None);
        assert_eq!(said, [Do::Tell("Not ready to record".into())]);
        a.press(Press::Rec, &stopped);
        assert!(a.press(Press::Stop, &stopped).is_empty());
        assert_eq!(a.ready, None);
        // OVERDUB: ready to add; REC then switches it to replace.
        a.press(Press::Overdub, &stopped);
        assert_eq!(a.ready, Some(Take::Add));
        assert_eq!(a.ready_lights(), (false, true));
        a.press(Press::Rec, &stopped);
        assert_eq!(a.ready, Some(Take::Replace));
        // The transport set to add: a replace take turns it back first.
        stopped.overdub = true;
        assert_eq!(
            a.press(Press::Play { from_start: false }, &stopped),
            [t(Command::ToggleOverdub), t(Command::Record)]
        );
        a.press(Press::Overdub, &stopped);
        assert_eq!(
            a.press(Press::Play { from_start: false }, &stopped),
            [t(Command::Record)]
        );
    }

    #[test]
    fn while_playing_rec_and_overdub_punch_in_and_while_recording_they_punch_out_or_switch() {
        let mut a = RecArm::default();
        let playing = now(State::Playing);
        assert_eq!(a.press(Press::Rec, &playing), [t(Command::Record)]);
        assert_eq!(
            a.press(Press::Overdub, &playing),
            [t(Command::ToggleOverdub), t(Command::Record)]
        );
        let recording = now(State::Recording);
        assert_eq!(a.press(Press::Rec, &recording), [t(Command::Record)]);
        assert_eq!(
            a.press(Press::Overdub, &recording),
            [t(Command::ToggleOverdub)]
        );
        assert_eq!(a.ready, None);
    }

    #[test]
    fn the_loop_records_as_the_looper_does() {
        let mut a = RecArm {
            target: Target::Loop,
            ..RecArm::default()
        };
        for s in [State::Stopped, State::Playing, State::Recording] {
            assert_eq!(
                a.press(Press::Rec, &now(s)),
                [Do::Looper(LoopCommand::Record)]
            );
            assert_eq!(
                a.press(Press::Overdub, &now(s)),
                [Do::Looper(LoopCommand::Overdub)]
            );
        }
        assert_eq!(a.ready, None);
    }

    #[test]
    fn the_arm_moves_with_its_ready_but_not_while_a_take_records() {
        let mut a = RecArm::default();
        let stopped = now(State::Stopped);
        a.press(Press::Rec, &stopped);
        // To the loop: the loop is armed in its place.
        assert_eq!(
            a.move_to(Target::Loop, Mover::Page, &stopped),
            [Do::Looper(LoopCommand::Record)]
        );
        assert_eq!(
            a,
            RecArm {
                target: Target::Loop,
                ..RecArm::default()
            }
        );
        // Back to the notes with the loop armed: it lets go, the notes are ready.
        let armed = Now {
            loop_armed: true,
            ..stopped.clone()
        };
        assert_eq!(
            a.move_to(Target::Notes, Mover::Page, &armed),
            [Do::StopWaiting]
        );
        assert_eq!(a.ready, Some(Take::Replace));
        // Nothing moves it while a take records, notes or loop.
        for busy in [
            now(State::Recording),
            now(State::CountIn),
            Now {
                loop_taking: true,
                ..now(State::Playing)
            },
        ] {
            let mut b = RecArm::default();
            assert!(b.move_to(Target::Loop, Mover::Page, &busy).is_empty());
            assert_eq!(b.target, Target::Notes);
        }
        // The same target again does nothing.
        let mut c = RecArm::default();
        assert!(c.move_to(Target::Notes, Mover::Page, &stopped).is_empty());
    }

    #[test]
    fn pinned_only_a_tap_moves_the_arm_and_a_tap_while_recording_says_why() {
        let stopped = now(State::Stopped);
        let mut a = RecArm {
            pinned: true,
            ..RecArm::default()
        };
        for by in [Mover::Page, Mover::Track] {
            assert!(a.move_to(Target::Loop, by, &stopped).is_empty());
            assert_eq!(a.target, Target::Notes, "{by:?} doesn't move a pinned arm");
        }
        assert!(a.move_to(Target::Loop, Mover::Tap, &stopped).is_empty());
        assert_eq!(a.target, Target::Loop, "a tap does");
        // While a take records a tap is refused, and says why.
        let mut b = RecArm::default();
        let said = b.move_to(Target::Loop, Mover::Tap, &now(State::Recording));
        assert!(
            matches!(&said[..], [Do::Tell(l)] if l.starts_with("Finish the take first")),
            "{said:?}"
        );
        assert_eq!(b.target, Target::Notes);
    }

    #[test]
    fn shift_arm_adds_the_other_target_and_rec_records_both_in_step() {
        let stopped = now(State::Stopped);
        let mut a = RecArm::default();
        // SHIFT + Arm on the only target says how to add one.
        let said = a.add(Target::Notes, &stopped);
        assert!(matches!(&said[..], [Do::Tell(l)] if l.contains("SHIFT + Arm")));
        assert!(!a.both);
        // On the loop: both are armed.
        let said = a.add(Target::Loop, &stopped);
        assert!(matches!(&said[..], [Do::Note(l)] if l.contains("Drums and the loop armed")));
        assert!(a.both && a.armed(Target::Notes) && a.armed(Target::Loop));
        assert_eq!(a.name(), "the notes and the loop");
        // Stopped: REC readies the notes and arms the loop; again lets go.
        let said = a.press(Press::Rec, &stopped);
        assert_eq!(a.ready, Some(Take::Replace));
        assert_eq!(said.last(), Some(&Do::Looper(LoopCommand::Record)));
        let waiting = Now {
            loop_armed: true,
            ..stopped.clone()
        };
        let said = a.press(Press::Rec, &waiting);
        assert_eq!(a.ready, None);
        assert_eq!(said.last(), Some(&Do::StopWaiting));
        // PLAY starts the notes' take; the armed loop starts with the
        // transport, as it does alone.
        a.press(Press::Rec, &stopped);
        assert_eq!(
            a.press(Press::Play { from_start: false }, &waiting),
            [t(Command::Record)]
        );
        // Playing, nothing recording: both start.
        let playing = now(State::Playing);
        assert_eq!(
            a.press(Press::Rec, &playing),
            [t(Command::Record), Do::Looper(LoopCommand::Record)]
        );
        // Recording both: REC punches both out.
        let both = Now {
            loop_taking: true,
            ..now(State::Recording)
        };
        assert_eq!(
            a.press(Press::Rec, &both),
            [t(Command::Record), Do::Looper(LoopCommand::Record)]
        );
        // Only the loop recording (the notes punched out): only the loop.
        let looping = Now {
            loop_taking: true,
            ..playing.clone()
        };
        assert_eq!(
            a.press(Press::Rec, &looping),
            [Do::Looper(LoopCommand::Record)]
        );
        // A page or a pick finds its target armed already.
        assert!(a.move_to(Target::Loop, Mover::Page, &stopped).is_empty());
        assert!(a.move_to(Target::Notes, Mover::Track, &stopped).is_empty());
        assert!(a.both);
        // Nothing changes while a take records.
        let said = a.add(Target::Loop, &both);
        assert!(matches!(&said[..], [Do::Tell(l)] if l.contains("Drums and the loop")));
        assert!(a.both);
    }

    #[test]
    fn a_plain_tap_keeps_one_of_two_and_shift_lets_go_of_one() {
        let stopped = now(State::Stopped);
        // Both ready: a tap on the notes lets go of the loop's wait.
        let mut a = RecArm::default();
        a.add(Target::Loop, &stopped);
        a.press(Press::Rec, &stopped);
        assert_eq!(
            a.move_to(Target::Notes, Mover::Tap, &stopped),
            [Do::StopWaiting]
        );
        assert_eq!(
            (a.target, a.both, a.ready),
            (Target::Notes, false, Some(Take::Replace))
        );
        // SHIFT + Arm on the notes with both: the loop records alone, and
        // the notes let go of their ready.
        a.add(Target::Loop, &stopped);
        assert!(a.both);
        assert!(a.add(Target::Notes, &stopped).is_empty());
        assert_eq!((a.target, a.both, a.ready), (Target::Loop, false, None));
        // The notes added to an armed loop get ready with it.
        let waiting = Now {
            loop_armed: true,
            ..stopped.clone()
        };
        a.add(Target::Notes, &waiting);
        assert_eq!((a.both, a.ready), (true, Some(Take::Replace)));
        // The loop added to the notes ready to add waits to overdub.
        let mut b = RecArm::default();
        b.press(Press::Overdub, &stopped);
        let said = b.add(Target::Loop, &stopped);
        assert_eq!(said[0], Do::Looper(LoopCommand::Overdub));
    }

    #[test]
    fn undo_while_a_take_records_throws_it_away() {
        let mut a = RecArm::default();
        assert_eq!(a.throw(&now(State::Stopped)), None, "UNDO is the page's");
        assert_eq!(a.throw(&now(State::Playing)), None);
        let said = a.throw(&now(State::Recording)).unwrap();
        assert_eq!(said[0], Do::ThrowTake);
        assert!(matches!(&said[1], Do::Tell(l) if l.contains("thrown away")));
        let looping = Now {
            loop_taking: true,
            ..now(State::Playing)
        };
        assert_eq!(a.throw(&looping).unwrap()[0], Do::Looper(LoopCommand::Undo));
        // Both record: both go.
        let both = Now {
            loop_taking: true,
            ..now(State::Recording)
        };
        let said = a.throw(&both).unwrap();
        assert_eq!(&said[..2], [Do::ThrowTake, Do::Looper(LoopCommand::Undo)]);
    }

    fn pad(d: PadDo) -> Do {
        Do::Pad(d)
    }

    fn said(out: &[Do]) -> String {
        out.iter()
            .filter_map(|d| match d {
                Do::Tell(l) | Do::Note(l) => Some(l.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// An empty kit pad held + REC (sampler R5.7, one take only): the
    /// notes' ready lets go, the take waits for the music, records, and
    /// REC then records the notes again.
    #[test]
    fn an_empty_pad_and_rec_arm_one_take_and_then_rec_records_what_it_did() {
        let stopped = now(State::Stopped);
        let mut a = RecArm::default();
        a.press(Press::Rec, &stopped);
        assert_eq!(a.ready, Some(Take::Replace));
        let out = a.arm_pad(0, 4, &stopped);
        assert_eq!(out[0], pad(PadDo::Arm { kit: 0, pad: 4 }));
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(said(&out).contains("REC → Pad A5: one take, then REC records Drums"));
        assert_eq!(a.ready, None, "the notes' ready lets go");
        assert!(!a.armed(Target::Notes) && !a.armed(Target::Loop));
        assert_eq!(a.ready_lights(), (true, false), "REC blinks");
        // Stopped or counting in: no go yet; once the music plays, one go.
        assert!(a.rolled(&stopped).is_empty());
        assert!(a.rolled(&now(State::CountIn)).is_empty());
        let playing = now(State::Playing);
        assert_eq!(a.rolled(&playing), [pad(PadDo::Start)]);
        assert!(a.rolled(&playing).is_empty());
        // The recorder says it records; REC ends it and the arm goes back.
        assert!(a.pad_now(PadNow::Recording));
        assert!(!a.pad_now(PadNow::Recording), "said once");
        let out = a.press(Press::Rec, &playing);
        assert_eq!(out[0], pad(PadDo::End));
        assert!(said(&out).contains("Pad A5: the take ends and lands on the pad"));
        assert!(said(&out).contains("REC records Drums again"));
        assert_eq!(a, RecArm::default());
        // REC records the notes as before.
        assert_eq!(a.press(Press::Rec, &playing), [t(Command::Record)]);
        // Armed with the music playing: the go comes with it.
        let mut b = RecArm::default();
        let out = b.arm_pad(2, 17, &playing);
        assert_eq!(
            &out[..2],
            [pad(PadDo::Arm { kit: 2, pad: 17 }), pad(PadDo::Start)]
        );
        assert!(said(&out).contains("Pad B2"));
    }

    #[test]
    fn a_pad_take_holds_like_a_pin_and_a_tap_before_it_records_calls_it_off() {
        let stopped = now(State::Stopped);
        let mut a = RecArm::default();
        a.arm_pad(0, 4, &stopped);
        // Pages and track picks leave it; the user's pin is untouched.
        assert!(a.move_to(Target::Loop, Mover::Page, &stopped).is_empty());
        assert!(a.move_to(Target::Notes, Mover::Track, &stopped).is_empty());
        assert!(a.pad.is_some() && !a.pinned);
        // SHIFT + a tap is refused.
        let out = a.add(Target::Loop, &stopped);
        assert!(
            said(&out).starts_with("A pad take records alone"),
            "{out:?}"
        );
        assert!(a.pad.is_some() && !a.both);
        // A tap calls it off and arms what was tapped.
        let out = a.move_to(Target::Loop, Mover::Tap, &stopped);
        assert_eq!(out[0], pad(PadDo::End));
        assert!(said(&out).contains("Pad A5 take called off"));
        assert_eq!((a.pad, a.target), (None, Target::Loop));
        // While it records a tap is refused, naming the pad.
        let mut b = RecArm::default();
        b.arm_pad(0, 4, &stopped);
        b.pad_now(PadNow::Recording);
        let out = b.move_to(Target::Loop, Mover::Tap, &now(State::Playing));
        assert_eq!(
            out,
            [Do::Tell(
                "Finish the take first: REC records Pad A5 until it stops".into()
            )]
        );
        assert!(b.pad.is_some() && b.target == Target::Notes);
    }

    #[test]
    fn a_pad_take_waits_for_other_takes_and_lets_go_of_a_waiting_loop() {
        let stopped = now(State::Stopped);
        // Refused while a take counts in or records, notes or loop.
        for busy in [
            now(State::Recording),
            now(State::CountIn),
            Now {
                loop_taking: true,
                ..now(State::Playing)
            },
        ] {
            let mut a = RecArm::default();
            let out = a.arm_pad(0, 4, &busy);
            assert!(said(&out).starts_with("Finish the take first"), "{out:?}");
            assert_eq!(a.pad, None);
        }
        // A loop waiting for PLAY lets go; REC goes back to the loop.
        let waiting = Now {
            loop_armed: true,
            ..stopped.clone()
        };
        let mut a = RecArm {
            target: Target::Loop,
            ..RecArm::default()
        };
        let out = a.arm_pad(0, 4, &waiting);
        assert_eq!(
            &out[..2],
            [Do::StopWaiting, pad(PadDo::Arm { kit: 0, pad: 4 })]
        );
        // REC with the same pad again calls it off.
        let out = a.arm_pad(0, 4, &stopped);
        assert_eq!(out[0], pad(PadDo::End));
        assert!(said(&out).contains("Pad A5 take called off · REC records the loop again"));
        assert_eq!((a.pad, a.target), (None, Target::Loop));
        // Another empty pad moves it there; while it records, refused.
        a.arm_pad(0, 4, &stopped);
        let out = a.arm_pad(0, 5, &stopped);
        assert_eq!(
            &out[..2],
            [pad(PadDo::End), pad(PadDo::Arm { kit: 0, pad: 5 })]
        );
        assert_eq!(a.pad.map(|p| p.pad), Some(5));
        a.pad_now(PadNow::Recording);
        assert!(said(&a.arm_pad(0, 6, &stopped)).starts_with("Finish the take first"));
        assert_eq!(a.pad.map(|p| p.pad), Some(5));
        // Both armed: REC records both again afterwards.
        let mut b = RecArm::default();
        b.add(Target::Loop, &stopped);
        let out = b.arm_pad(0, 4, &stopped);
        assert!(said(&out).contains("then REC records Drums and the loop again"));
        b.press(Press::Rec, &stopped);
        assert!(b.both && b.pad.is_none());
    }

    #[test]
    fn stop_keeps_a_pad_take_undo_throws_it_and_the_recorder_can_end_it() {
        let (stopped, playing) = (now(State::Stopped), now(State::Playing));
        let mut a = RecArm::default();
        // Armed only: STOP calls it off (and stops as usual, the app's).
        a.arm_pad(0, 4, &stopped);
        let out = a.press(Press::Stop, &stopped);
        assert_eq!(out[0], pad(PadDo::End));
        assert!(said(&out).contains("called off by STOP"));
        assert_eq!(a.pad, None);
        // Recording: STOP keeps it.
        a.arm_pad(0, 4, &playing);
        a.pad_now(PadNow::Recording);
        let out = a.press(Press::Stop, &playing);
        assert_eq!(out[0], pad(PadDo::End));
        assert!(said(&out).contains("the take ends and lands on the pad"));
        // OVERDUB says what it doesn't do; PLAY is its own.
        a.arm_pad(0, 4, &playing);
        let out = a.press(Press::Overdub, &playing);
        assert!(said(&out).starts_with("OVERDUB doesn't add to a pad take"));
        assert!(a
            .press(Press::Play { from_start: true }, &playing)
            .is_empty());
        assert!(a.pad.is_some());
        // UNDO before it records is the page's; while it records it throws.
        assert_eq!(a.throw(&playing), None);
        a.pad_now(PadNow::Recording);
        let out = a.throw(&playing).unwrap();
        assert_eq!(out[0], pad(PadDo::Throw));
        assert!(said(&out).contains("Pad A5's take is thrown away"));
        assert_eq!(a.pad, None);
        // The recorder ends it itself (a length, silence, or it couldn't run).
        a.arm_pad(0, 4, &stopped);
        assert!(a.pad_now(PadNow::Ended));
        assert_eq!(a, RecArm::default());
        assert!(!a.pad_now(PadNow::Ended), "nothing to end");
    }
}
