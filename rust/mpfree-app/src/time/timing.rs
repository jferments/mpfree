//! One timing change (the smart loop end plan's step 0C): the tempo, the
//! song position and PLAY changed together on one frame, all or none,
//! with an answer.
//!
//! A change is made off the audio thread or ahead of time, against the
//! song's timeline as it was then ([`TimeAudio::timeline`]), and lands on
//! frame `at` of the transport's clock, in a period not yet rendered.
//! Until then it waits in the audio half, one at a time: a newer change
//! cancels an older one, and anything that moves time first (a locate, a
//! tempo, PLAY or STOP, a song edit that moves time, another project)
//! cancels it, so it is never applied to a song it wasn't made for. In
//! the period its frame falls in it goes to the transport's queue as one
//! piece (all its commands, or none when there is no room) and is
//! answered: committed on its frame, cancelled or refused, and why.
//!
//! What lands with it is the transport's own. While the song plays, the
//! tempo takes over on that frame (`Command::TempoNow`), not on the next
//! 1/16, with no jump: the notes sounding go on, and nothing is replayed
//! or skipped. A position is a jump on the same frame (`Command::Locate`),
//! so what was due before it in the new timing is skipped, never
//! replayed; each sounding note gets one note-off there
//! (`mpfree_play::Player::end_all`), and MIDI clock out sends Stop, Song
//! Position and Continue. All of it applies in one period
//! (`Transport::room_on`). Stopped, it is [`TimeAudio::start_at`]'s start,
//! which is now its stopped case.
//!
//! [`TimeAudio::timeline`]: super::audio::TimeAudio::timeline
//! [`TimeAudio::start_at`]: super::audio::TimeAudio::start_at

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use mpfree_transport::time::Tick;

/// A new timeline revision, never given out before in this process: each
/// project's time engine starts from a fresh one, so a change made against
/// one project can't match another's.
pub fn next_timeline() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A new ticket for a timing change, never given out before in this
/// process (0 is none).
fn next_ticket() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A tempo, a song position and PLAY, together on one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimingChange {
    /// The frame it lands on, on the transport's frame clock: the next
    /// period's first frame or later.
    pub at: u64,
    /// The tempo from `at` on. While the song's tempo points lead, a start
    /// leaves it out (they lead) and a change while playing is refused.
    pub tempo: Option<f64>,
    /// The song position on `at`: playing, a jump there. None leaves the
    /// song where it is, and a tempo alone takes over with no jump.
    pub position: Option<Tick>,
    /// PLAY on `at` (stopped only; nothing while playing).
    pub play: bool,
    /// Starting from stopped with a take ready, it records from `at` too:
    /// adding (true) or replacing, a punch-in with no count-in.
    pub take: Option<bool>,
    /// The timeline it was made against ([`super::audio::TimeAudio::timeline`]).
    pub made_against: u64,
}

/// Whether a timing change may be offered now, before anything is
/// offered or asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Allowed {
    /// Only what follows the grid plays on: it all follows the change.
    Safe,
    /// Recorded audio playing at its own speed would play out of step at
    /// the new tempo: ask first.
    Ask(&'static str),
    /// Not now, and why.
    Refused(&'static str),
}

/// What became of a timing change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Its frame hasn't come yet.
    Waiting,
    /// In the transport's queue for this frame, all of it.
    Committed(u64),
    /// Something moved time before its frame (the reason), or a newer
    /// change or its maker took it back.
    Cancelled(&'static str),
    /// It couldn't land when its frame came (the reason): nothing of it
    /// went in.
    Refused(&'static str),
}

/// A change waiting for its frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct Pending {
    pub ticket: u32,
    pub change: TimingChange,
}

/// The answers to the last few changes, by ticket.
#[derive(Clone, Copy, Debug)]
pub(super) struct Answers {
    last: [(u32, Answer); ANSWERS],
    next: usize,
}

/// Answers kept: a maker reads its answer within a period or two.
const ANSWERS: usize = 4;

impl Answers {
    pub fn new() -> Answers {
        Answers {
            last: [(0, Answer::Waiting); ANSWERS],
            next: 0,
        }
    }

    /// A new ticket, waiting.
    pub fn open(&mut self) -> u32 {
        let t = next_ticket();
        self.put(t, Answer::Waiting);
        t
    }

    /// `ticket`'s answer is `a` from now on.
    pub fn put(&mut self, ticket: u32, a: Answer) {
        if let Some(e) = self.last.iter_mut().find(|e| e.0 == ticket) {
            e.1 = a;
            return;
        }
        self.last[self.next] = (ticket, a);
        self.next = (self.next + 1) % ANSWERS;
    }

    /// `ticket`'s answer, if it is one of the last few.
    pub fn get(&self, ticket: u32) -> Option<Answer> {
        (ticket != 0)
            .then(|| self.last.iter().find(|e| e.0 == ticket).map(|e| e.1))
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_keep_the_last_few_by_ticket() {
        let mut a = Answers::new();
        let t: Vec<u32> = (0..5).map(|_| a.open()).collect();
        assert_eq!(a.get(0), None);
        // The oldest went to make room for the fifth.
        assert_eq!(a.get(t[0]), None);
        assert_eq!(a.get(t[4]), Some(Answer::Waiting));
        a.put(t[4], Answer::Committed(9));
        assert_eq!(a.get(t[4]), Some(Answer::Committed(9)));
        assert_eq!(a.get(t[1]), Some(Answer::Waiting));
        // Tickets and timelines are never given out twice.
        assert!(t.windows(2).all(|w| w[0] != w[1]));
        assert_ne!(next_timeline(), next_timeline());
    }
}
