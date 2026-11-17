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
//!   lets go of ready.
//! - **A loop** records as the looper does already: REC arms the picked
//!   loop (its first pass starts with PLAY, or on the next bar line while
//!   playing) and OVERDUB overdubs it. The looper's armed loop is its
//!   ready.
//!
//! What moves the arm ([`RecArm::move_to`]): a tap on an Arm pill or a row
//! of the target list, opening Play's Beats view (the notes) or Play's
//! Loop view or the Looper tab (the loop), and picking a track (its
//! notes). Nothing moves it while a take records (a tap says "Finish the
//! take first"). Pinned, only a tap moves it. When the arm moves while
//! ready, the ready goes with it.
//!
//! UNDO while a take records throws the take so far away
//! ([`RecArm::throw`]): recording stops and the music plays on.
//!
//! The arm isn't saved and isn't an undo step; a new or opened project
//! starts with the notes armed, unpinned. SHIFT + REC is still Capture,
//! whatever is armed.

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
    /// A page with one target opened (Beats, the Loop view, the Looper tab).
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
    /// Said on the hint line and in the log.
    Tell(String),
    /// Written to the log only (the hint line says it already).
    Note(String),
}

/// The arm: what REC records, and whether the notes are ready.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecArm {
    pub target: Target,
    /// The notes wait for PLAY to record (only with [`Target::Notes`]; a
    /// loop's ready is the looper's own armed loop).
    pub ready: Option<Take>,
    /// Only a tap moves the arm (the target list's Pin the arm).
    pub pinned: bool,
}

/// The transport's mode set for `take`, if it isn't already.
fn mode(take: Take, overdub: bool) -> Option<Do> {
    (overdub != (take == Take::Add)).then_some(Do::Transport(Command::ToggleOverdub))
}

impl RecArm {
    /// A press of REC, OVERDUB, PLAY or STOP, as it finds the transport.
    pub fn press(&mut self, p: Press, now: &Now) -> Vec<Do> {
        let take = match p {
            Press::Rec => Take::Replace,
            Press::Overdub => Take::Add,
            Press::Stop => {
                self.ready = None;
                return Vec::new();
            }
            Press::Play { from_start } => {
                let Some(take) = self.ready.take() else {
                    return Vec::new();
                };
                let mut out: Vec<Do> = mode(take, now.overdub).into_iter().collect();
                if from_start {
                    out.push(Do::Transport(Command::Locate(0)));
                }
                out.push(Do::Transport(Command::Record));
                return out;
            }
        };
        if self.target == Target::Loop {
            return vec![Do::Looper(match take {
                Take::Replace => LoopCommand::Record,
                Take::Add => LoopCommand::Overdub,
            })];
        }
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

    /// Arm `target`, as `by` moves it. Refused while a take records (a tap
    /// says why, a page or a pick says nothing) and, pinned, unless by a
    /// tap. A ready goes with the arm: the notes' to the loop as Record (or
    /// Overdub), an armed loop's to the notes.
    pub fn move_to(&mut self, target: Target, by: Mover, now: &Now) -> Vec<Do> {
        if target == self.target || (self.pinned && by != Mover::Tap) {
            return Vec::new();
        }
        if now.taking() {
            return match by {
                Mover::Tap => vec![Do::Tell(format!(
                    "Finish the take first: REC records {} until it stops",
                    match self.target {
                        Target::Notes => now.notes.as_str(),
                        Target::Loop => "the loop",
                    }
                ))],
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

    /// UNDO while a take records: the take so far is thrown away and the
    /// music plays on (the notes punch out; a loop's first pass or overdub
    /// is undone). None when nothing records: UNDO is the page's.
    pub fn throw(&mut self, now: &Now) -> Option<Vec<Do>> {
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

    /// The lights: the notes ready to replace (REC blinks) or to add
    /// (OVERDUB blinks), PLAY blinking with either.
    pub fn ready_lights(&self) -> (bool, bool) {
        (
            self.ready == Some(Take::Replace),
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
}
