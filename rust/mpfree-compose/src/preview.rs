//! The preview's states (phase 0B, contract 7 of the design). Every
//! result is heard before anything is written, and Keep writes it as one
//! take that UNDO takes back.
//!
//! A preview is bound to one clip at one revision ([`Binding`]). The host
//! (the app's control thread) drives it and does what each state asks:
//!
//! - **Idle**: nothing asked; the clip plays as it is.
//! - **Preparing**: a request is out to the composer. A result that was
//!   playing (Try again while listening) goes on playing until the new one
//!   arrives, so the music never stops for it.
//! - **Ready**: a result is in, not playing yet. The host auditions it at
//!   the next bar line.
//! - **Auditioning**: the result plays in place of the clip. While
//!   [`Preview::plays`] says so, the host compiles the play view from
//!   [`crate::bind::audition`]'s project (in phase 1, through the shared
//!   audition layer the piano roll's previews build).
//! - **Committing**: Keep was pressed. The host writes the edit as one
//!   take (`mpfree_play::Takes::edit`, the transaction step edits use) and
//!   compiles, then says how it went ([`Preview::kept`]). A refused Keep
//!   writes nothing and leaves no preview running: the result is Ready
//!   again, with the refusal in words.
//! - **Cancelling**: Back was pressed while the result played. The host
//!   compiles the clip's own view and says when it is in
//!   ([`Preview::view_plain`]).
//! - **Invalidated**: the clip, or a clip the reading listened to, changed
//!   or went ([`Preview::clip_changed`], or found on arrival or at Keep).
//!   Nothing can be kept; Try again asks afresh.
//!
//! Results are numbered by request, so one that arrives after a newer
//! request, or after Back, is dropped and never heard. The last
//! [`HISTORY`] results for the clip are kept, and ‹ › steps through them
//! ([`Preview::step`]).

use mpfree_context::ClipRead;
use mpfree_seq::steps::Edit;
use mpfree_seq::{ModelError, Project};

use crate::bind::{Binding, Stale};

/// How many results ‹ › steps back through.
pub const HISTORY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Preparing,
    Ready,
    Auditioning,
    Committing,
    Cancelling,
    Invalidated,
}

/// A result from the composer: the edit Keep would make, and the clips
/// its reading listened to (each at the revision it heard).
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    pub edit: Edit,
    pub read: Vec<ClipRead>,
}

/// What became of a result that arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrival {
    /// It is the newest request's: ready, or playing in place of the last.
    Taken,
    /// A request since (or Back) made it old: dropped unheard.
    Dropped,
    /// What it was made from changed meanwhile: dropped, and the preview
    /// says so.
    Stale(Stale),
}

/// Why a step was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not in this state (Keep under way, or Back finishing).
    Busy(State),
    /// No result to hear or keep yet.
    Nothing,
    Stale(Stale),
    /// The clip refused the edit; nothing was written.
    Model(ModelError),
}

impl Refusal {
    pub fn words(self) -> &'static str {
        match self {
            Refusal::Busy(_) => "Still finishing the last step.",
            Refusal::Nothing => "Nothing to keep yet.",
            Refusal::Stale(s) => s.words(),
            Refusal::Model(ModelError::TooManyNotes) => {
                "The clip is full and can't hold these notes. Nothing was written."
            }
            Refusal::Model(ModelError::ClipLength) => {
                "The clip can't be that long. Nothing was written."
            }
            Refusal::Model(_) => "The clip can't take these notes. Nothing was written.",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    state: State,
    binding: Option<Binding>,
    /// The newest request's number.
    asked: u32,
    /// The last results for the bound clip, oldest first.
    results: Vec<Proposal>,
    /// The one shown: what plays and what Keep writes.
    shown: usize,
    /// The shown result is what plays in place of the clip.
    hearing: bool,
    stale: Option<Stale>,
    refused: Option<Refusal>,
    /// The take the last Keep wrote.
    take: Option<u32>,
}

impl Default for Preview {
    fn default() -> Self {
        Preview::new()
    }
}

impl Preview {
    pub fn new() -> Preview {
        Preview {
            state: State::Idle,
            binding: None,
            asked: 0,
            results: Vec::new(),
            shown: 0,
            hearing: false,
            stale: None,
            refused: None,
            take: None,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn binding(&self) -> Option<Binding> {
        self.binding
    }

    /// Why it was invalidated.
    pub fn stale(&self) -> Option<Stale> {
        self.stale
    }

    /// Why the last Keep was refused, until the next step.
    pub fn refused(&self) -> Option<Refusal> {
        self.refused
    }

    /// The take the last Keep wrote: UNDO takes it back.
    pub fn take(&self) -> Option<u32> {
        self.take
    }

    /// Which result is shown, from 1 (the oldest kept), and how many there
    /// are: what ‹ › shows.
    pub fn position(&self) -> Option<(usize, usize)> {
        (!self.results.is_empty()).then_some((self.shown + 1, self.results.len()))
    }

    fn current(&self) -> Option<&Proposal> {
        self.results.get(self.shown)
    }

    /// What the play view should play in place of the bound clip now, if
    /// anything: the host compiles [`crate::bind::audition`]'s project
    /// while this is `Some`, and the project itself otherwise.
    pub fn plays(&self) -> Option<(&Binding, &Edit)> {
        if !self.hearing {
            return None;
        }
        Some((self.binding.as_ref()?, &self.current()?.edit))
    }

    /// Ask for a result for the clip `b` names (Continue, or Try again).
    /// The request's number, which its result must carry.
    pub fn ask(&mut self, b: Binding) -> Result<u32, Refusal> {
        if matches!(self.state, State::Committing | State::Cancelling) {
            return Err(Refusal::Busy(self.state));
        }
        // Results for this same clip stay, and one playing goes on playing
        // until the new one arrives; another clip, or the clip changed,
        // starts afresh.
        if self.binding != Some(b) {
            self.hearing = false;
            self.results.clear();
            self.shown = 0;
        }
        self.binding = Some(b);
        self.asked = self.asked.wrapping_add(1);
        self.state = State::Preparing;
        self.stale = None;
        self.refused = None;
        Ok(self.asked)
    }

    /// The composer's result for request `n`, checked against the project
    /// as it is now. It becomes the shown result.
    pub fn arrived(&mut self, n: u32, p: Proposal, project: &Project) -> Arrival {
        if self.state != State::Preparing || n != self.asked {
            return Arrival::Dropped;
        }
        let Some(b) = self.binding else {
            return Arrival::Dropped;
        };
        if let Err(s) = b.check_all(project, &p.read) {
            self.invalidate(s);
            return Arrival::Stale(s);
        }
        if self.results.len() == HISTORY {
            self.results.remove(0);
        }
        self.results.push(p);
        self.shown = self.results.len() - 1;
        self.state = if self.hearing {
            State::Auditioning
        } else {
            State::Ready
        };
        Arrival::Taken
    }

    /// ‹ (`back` true) or ›: show the result before or after this one. It
    /// plays at once if one was playing. False at either end.
    pub fn step(&mut self, back: bool) -> bool {
        if !matches!(self.state, State::Ready | State::Auditioning) {
            return false;
        }
        let to = if back {
            self.shown.checked_sub(1)
        } else {
            Some(self.shown + 1).filter(|&i| i < self.results.len())
        };
        match to {
            Some(i) => {
                self.shown = i;
                self.refused = None;
                true
            }
            None => false,
        }
    }

    /// Play the shown result in place of the clip (at the bar line).
    pub fn audition(&mut self) -> Result<(), Refusal> {
        match self.state {
            State::Ready => {
                self.state = State::Auditioning;
                self.hearing = true;
                self.refused = None;
                Ok(())
            }
            State::Auditioning => Ok(()),
            State::Idle | State::Preparing | State::Invalidated => Err(Refusal::Nothing),
            s => Err(Refusal::Busy(s)),
        }
    }

    /// Stop playing the result; it stays ready to hear again or keep.
    pub fn stop(&mut self) {
        if self.state == State::Auditioning {
            self.state = State::Ready;
            self.hearing = false;
        }
    }

    /// Keep: the binding and the shown result's edit for the host to write
    /// as one take, if the bound clip and every clip the reading listened
    /// to are as they were. Then [`Preview::kept`].
    pub fn keep(&mut self, project: &Project) -> Result<(Binding, Edit), Refusal> {
        match self.state {
            State::Ready | State::Auditioning => {}
            State::Idle | State::Preparing | State::Invalidated => return Err(Refusal::Nothing),
            s => return Err(Refusal::Busy(s)),
        }
        let (Some(b), Some(p)) = (self.binding, self.current()) else {
            return Err(Refusal::Nothing);
        };
        if let Err(s) = b.check_all(project, &p.read) {
            self.invalidate(s);
            return Err(Refusal::Stale(s));
        }
        let edit = p.edit.clone();
        self.state = State::Committing;
        self.refused = None;
        Ok((b, edit))
    }

    /// How the host's write went: the take's number, or the clip's
    /// refusal. A refusal writes nothing and leaves no preview running:
    /// the result is Ready again, and [`Preview::refused`] says why.
    pub fn kept(&mut self, r: Result<u32, ModelError>) {
        if self.state != State::Committing {
            return;
        }
        self.hearing = false;
        match r {
            Ok(take) => {
                self.take = Some(take);
                self.results.clear();
                self.shown = 0;
                self.binding = None;
                self.state = State::Idle;
            }
            Err(e) => {
                self.refused = Some(Refusal::Model(e));
                self.state = State::Ready;
            }
        }
    }

    /// Back: nothing is written and the results go. False while Keep is
    /// under way.
    pub fn cancel(&mut self) -> bool {
        match self.state {
            State::Committing => return false,
            State::Idle | State::Cancelling => return true,
            _ => {}
        }
        self.state = if self.hearing {
            State::Cancelling
        } else {
            State::Idle
        };
        self.hearing = false;
        self.results.clear();
        self.shown = 0;
        self.binding = None;
        self.stale = None;
        self.refused = None;
        true
    }

    /// The clip's own view is playing again after Back.
    pub fn view_plain(&mut self) {
        if self.state == State::Cancelling {
            self.state = State::Idle;
        }
    }

    /// After any edit to the project (a step edit, a recording, an UNDO):
    /// if the bound clip, or a clip a result's reading listened to,
    /// changed or went, the preview is invalidated and stops playing. Why,
    /// if it was.
    pub fn clip_changed(&mut self, project: &Project) -> Option<Stale> {
        if !matches!(
            self.state,
            State::Preparing | State::Ready | State::Auditioning
        ) {
            return None;
        }
        let b = self.binding?;
        let s = match b.check(project) {
            Err(s) => s,
            Ok(_) => self
                .results
                .iter()
                .find_map(|p| b.check_all(project, &p.read).err())?,
        };
        self.invalidate(s);
        Some(s)
    }

    fn invalidate(&mut self, s: Stale) {
        self.state = State::Invalidated;
        self.stale = Some(s);
        self.hearing = false;
        self.results.clear();
        self.shown = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::{Clip, Note, Track};

    fn project() -> Project {
        let mut p = Project::new();
        let mut t = Track::new("Lead", 0);
        let mut c = Clip::new("riff", 3840);
        c.notes = vec![Note::new(0, 240, 60, 100), Note::new(960, 240, 63, 100)];
        t.slots[0] = Some(c);
        p.tracks.push(t);
        p.give_clip_ids();
        p
    }

    fn proposal(pitch: u8) -> Proposal {
        Proposal {
            edit: Edit {
                length: Some((3840, 7680)),
                add: vec![Note::new(3840, 240, pitch, 100)],
                ..Edit::default()
            },
            read: Vec::new(),
        }
    }

    fn playing(v: &Preview) -> Option<u8> {
        v.plays().map(|(_, e)| e.add[0].note)
    }

    #[test]
    fn heard_then_kept() {
        let p = project();
        let b = Binding::of(&p, 0, 0).unwrap();
        let mut v = Preview::new();
        assert_eq!(v.keep(&p), Err(Refusal::Nothing));
        let n = v.ask(b).unwrap();
        assert_eq!(v.state(), State::Preparing);
        assert_eq!(v.plays(), None);
        assert_eq!(v.arrived(n, proposal(65), &p), Arrival::Taken);
        assert_eq!(v.state(), State::Ready);
        v.audition().unwrap();
        assert_eq!(v.state(), State::Auditioning);
        assert_eq!(playing(&v), Some(65));
        let (kb, edit) = v.keep(&p).unwrap();
        assert_eq!((kb, edit.add[0].note), (b, 65));
        assert_eq!(v.state(), State::Committing);
        // Nothing else while Keep is under way.
        assert_eq!(v.ask(b), Err(Refusal::Busy(State::Committing)));
        assert!(!v.cancel());
        v.kept(Ok(4));
        assert_eq!(
            (v.state(), v.take(), v.plays(), v.position()),
            (State::Idle, Some(4), None, None)
        );
    }

    #[test]
    fn a_refused_keep_writes_nothing_and_leaves_no_preview_running() {
        let p = project();
        let mut v = Preview::new();
        let n = v.ask(Binding::of(&p, 0, 0).unwrap()).unwrap();
        v.arrived(n, proposal(65), &p);
        v.audition().unwrap();
        v.keep(&p).unwrap();
        v.kept(Err(ModelError::TooManyNotes));
        assert_eq!((v.state(), v.plays()), (State::Ready, None));
        assert_eq!(
            v.refused().map(Refusal::words),
            Some("The clip is full and can't hold these notes. Nothing was written.")
        );
        // The result is still there to hear again.
        v.audition().unwrap();
        assert_eq!(playing(&v), Some(65));
    }

    #[test]
    fn try_again_while_listening_plays_on_until_the_new_result() {
        let p = project();
        let b = Binding::of(&p, 0, 0).unwrap();
        let mut v = Preview::new();
        let first = v.ask(b).unwrap();
        v.arrived(first, proposal(65), &p);
        v.audition().unwrap();
        let second = v.ask(b).unwrap();
        assert_eq!(v.state(), State::Preparing);
        assert_eq!(playing(&v), Some(65), "the old one plays on");
        // A late copy of the first is dropped.
        assert_eq!(v.arrived(first, proposal(70), &p), Arrival::Dropped);
        assert_eq!(v.arrived(second, proposal(67), &p), Arrival::Taken);
        assert_eq!(v.state(), State::Auditioning);
        assert_eq!(playing(&v), Some(67));
    }

    #[test]
    fn step_back_through_the_last_eight() {
        let p = project();
        let b = Binding::of(&p, 0, 0).unwrap();
        let mut v = Preview::new();
        for pitch in 60..70 {
            let n = v.ask(b).unwrap();
            v.arrived(n, proposal(pitch), &p);
        }
        // Ten tries: the last eight are kept, the newest shown.
        assert_eq!(v.position(), Some((8, 8)));
        v.audition().unwrap();
        assert_eq!(playing(&v), Some(69));
        assert!(!v.step(false), "nothing after the newest");
        assert!(v.step(true));
        assert_eq!(playing(&v), Some(68));
        while v.step(true) {}
        assert_eq!((v.position(), playing(&v)), (Some((1, 8)), Some(62)));
        // Keep writes the one shown.
        assert_eq!(v.keep(&p).unwrap().1.add[0].note, 62);
    }

    #[test]
    fn an_edit_to_the_clip_invalidates_and_nothing_can_be_kept() {
        let mut p = project();
        let b = Binding::of(&p, 0, 0).unwrap();
        let mut v = Preview::new();
        let n = v.ask(b).unwrap();
        v.arrived(n, proposal(65), &p);
        v.audition().unwrap();
        assert_eq!(v.clip_changed(&p), None, "nothing changed");
        p.tracks[0].slots[0].as_mut().unwrap().notes[1].velocity = 70;
        assert_eq!(v.clip_changed(&p), Some(Stale::Changed));
        assert_eq!((v.state(), v.plays()), (State::Invalidated, None));
        assert_eq!(v.keep(&p), Err(Refusal::Nothing));
        assert_eq!(
            v.stale().map(Stale::words),
            Some("The clip changed. Try again?")
        );
        // Try again binds to the clip as it is now, and starts afresh.
        let n = v.ask(Binding::of(&p, 0, 0).unwrap()).unwrap();
        assert_eq!(v.arrived(n, proposal(65), &p), Arrival::Taken);
        assert_eq!(v.position(), Some((1, 1)));
    }

    #[test]
    fn a_result_made_from_an_old_clip_is_never_shown() {
        let mut p = project();
        let mut v = Preview::new();
        let n = v.ask(Binding::of(&p, 0, 0).unwrap()).unwrap();
        // The clip is edited while the composer works.
        p.tracks[0].slots[0].as_mut().unwrap().notes.pop();
        assert_eq!(
            v.arrived(n, proposal(65), &p),
            Arrival::Stale(Stale::Changed)
        );
        assert_eq!(v.state(), State::Invalidated);
    }

    #[test]
    fn keep_checks_the_clip_once_more() {
        let mut p = project();
        let mut v = Preview::new();
        let n = v.ask(Binding::of(&p, 0, 0).unwrap()).unwrap();
        v.arrived(n, proposal(65), &p);
        p.tracks[0].slots[0] = None;
        assert_eq!(v.keep(&p), Err(Refusal::Stale(Stale::Gone)));
        assert_eq!(v.state(), State::Invalidated);
    }

    #[test]
    fn back_while_playing_waits_for_the_clips_own_view() {
        let p = project();
        let b = Binding::of(&p, 0, 0).unwrap();
        let mut v = Preview::new();
        let n = v.ask(b).unwrap();
        v.arrived(n, proposal(65), &p);
        v.audition().unwrap();
        assert!(v.cancel());
        assert_eq!((v.state(), v.plays()), (State::Cancelling, None));
        assert_eq!(v.ask(b), Err(Refusal::Busy(State::Cancelling)));
        v.view_plain();
        assert_eq!(v.state(), State::Idle);
        // Back before anything played is at once, and a late result is
        // dropped.
        let n = v.ask(b).unwrap();
        assert!(v.cancel());
        assert_eq!(v.state(), State::Idle);
        assert_eq!(v.arrived(n, proposal(65), &p), Arrival::Dropped);
    }
}
