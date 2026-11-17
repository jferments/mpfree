//! What the time engine's two halves say to each other, and the queues
//! they say it through (docs/dev/design/time-threads.md).
//!
//! Each queue is a bounded `std::sync::mpsc::sync_channel`, the kind
//! `main.rs` already uses between the screen and the audio thread, made
//! once with all its room. The halves only `try_send` and `try_recv`, so
//! neither waits for the other. A message that finds its queue full is
//! handed back to the sender and counted ([`Tx::full`]); the halves log
//! and show the counts (the fallback rule), and keep a view that could
//! not go back to be freed until it can.
//!
//! Every message but a view and a chain is a fixed size with nothing to
//! free, so the audio half sends and receives without allocating. A view
//! comes boxed and goes back boxed, so it is made and freed on the control
//! half; so is a track's chain of MIDI effects (`mpfree_notefx::Chain`,
//! about 400 KB), which the control half sends the first time the track
//! has an effect, and which stays with the track's arp from then on.

use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};

use mpfree_notefx::{Chain, Slot};
use mpfree_play::{Event, MuteLanded, PlayView, Rec, Started};
use mpfree_seq::mpfree_moves::{Input, Kind};
use mpfree_seq::{DestId, Lock, Note, TouchPoint, MAX_TRACKS};
use mpfree_time_ui::view::TransportInfo;
use mpfree_transport::{Command, Span};

use super::follow::FollowCounts;

/// Messages the audio half's queue holds: a busy second of periods
/// (spans, notes and recorded changes), with the control half late.
pub const TO_CONTROL: usize = 8192;
/// Messages the control half's queue holds.
pub const TO_AUDIO: usize = 256;
/// Pressure values in one [`ToControl::Touch`].
pub const TOUCH_CHUNK: usize = 12;
/// Gesture points in one [`ToControl::Points`].
pub const POINTS_CHUNK: usize = 6;

/// The sending end of a queue.
pub struct Tx<T> {
    tx: SyncSender<T>,
    full: u64,
}

/// The receiving end.
pub struct Rx<T> {
    rx: Receiver<T>,
}

/// A queue with room for `room` messages.
pub fn link<T>(room: usize) -> (Tx<T>, Rx<T>) {
    let (tx, rx) = sync_channel(room.max(1));
    (Tx { tx, full: 0 }, Rx { rx })
}

impl<T> Tx<T> {
    /// Send without waiting. A full queue (or one with nobody left to
    /// read it) hands the message back, and counts it.
    pub fn send(&mut self, m: T) -> Result<(), T> {
        match self.tx.try_send(m) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(m) | TrySendError::Disconnected(m)) => {
                self.full += 1;
                Err(m)
            }
        }
    }

    /// Messages handed back so far. It should stay 0.
    pub fn full(&self) -> u64 {
        self.full
    }
}

impl<T> Rx<T> {
    /// The next message, if one is waiting.
    pub fn recv(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }
}

/// The tracks' settings the audio half follows between views: which
/// track is armed, and each track's [`TrackSet`]. Each half keeps one; the
/// control half sends what changed ([`ToAudio::Tracks`], [`ToAudio::Track`]),
/// so a message stays small however many tracks there are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tracks {
    pub n: usize,
    pub armed: Option<usize>,
    pub set: [TrackSet; MAX_TRACKS],
}

/// One track's settings for the audio half: whether its clips play
/// through its arp, where its arp plays, and whether it records pad
/// pressure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackSet {
    pub through_arp: bool,
    pub dest: DestId,
    pub record_touch: bool,
}

impl Tracks {
    pub fn of(project: &mpfree_seq::Project, armed: Option<usize>) -> Tracks {
        let mut t = Tracks {
            n: project.tracks.len().min(MAX_TRACKS),
            armed,
            set: [TrackSet::default(); MAX_TRACKS],
        };
        for (s, track) in t.set.iter_mut().zip(&project.tracks) {
            *s = TrackSet {
                through_arp: track.through_arp,
                dest: track.dest,
                record_touch: track.record_touch,
            };
        }
        t
    }

    /// Whether the armed track's clips play through its arp, so it records
    /// the pads as pressed rather than what its arp played.
    pub fn records_pressed(&self) -> bool {
        self.armed
            .filter(|&t| t < self.n)
            .is_some_and(|t| self.set[t].through_arp)
    }

    /// Whether the armed track records pad pressure.
    pub fn records_touch(&self) -> bool {
        self.armed
            .filter(|&t| t < self.n)
            .is_some_and(|t| self.set[t].record_touch)
    }
}

/// From the control half.
pub enum ToAudio {
    /// A view to play, which says what it includes.
    View(Box<PlayView>),
    /// A transport command, from the next period on.
    Command(Command),
    /// How many tracks there are, and which is armed, changed.
    Tracks { n: usize, armed: Option<usize> },
    /// One track's settings changed.
    Track(usize, TrackSet),
    /// One place of a track's MIDI effects changed (`Track::note_fx`):
    /// its effect and the effect's id on the track (what names its
    /// settings for the mod engine).
    NoteFx {
        track: u16,
        place: u8,
        slot: Option<Slot>,
        id: u16,
    },
    /// Where a track's arp sits among its MIDI effects changed.
    NoteArp { track: u16, at: u8 },
    /// A chain for a track's MIDI effects, the first time it has one.
    Chain(u16, Box<Chain>),
    /// A step edit made to clip `slot` of `track`, one change a message,
    /// in the order the clip took them ([`InPlace`]).
    Edit {
        track: u16,
        slot: u16,
        change: InPlace,
    },
}

/// One change of a step edit, for the audio half to make to the view it
/// plays in place, as the clip took it (`mpfree_seq::Clip::apply`: the
/// notes and locks erased, then those added). The queue keeps them in
/// order with the views: a view sent before them was compiled without
/// them, and one sent after them has them, so they need no number and no
/// catch-up. A change the view cannot take (no room, or what it erases is
/// not there) asks for a compile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InPlace {
    AddNote(Note),
    EraseNote(Note),
    AddLock(Lock),
    EraseLock(Lock),
}

/// The audio half's counts of things lost, with each period.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Notes and locks lost to full tables.
    pub dropped: u64,
    /// Pressure values past the recorder's tables.
    pub touch_dropped: u64,
    /// Messages to the control half lost to a full queue.
    pub lost: u64,
    /// Views that could not catch up, and were sent back to compile again.
    pub behind: u64,
    /// Views it could not send back and kept rather than free (should
    /// stay 0).
    pub kept: u64,
    /// Transport commands the transport's queue refused.
    pub refused: u64,
    /// Step edits the view could not take in place, sent back to compile
    /// (should stay 0).
    pub missed: u64,
}

/// The end of a period.
#[derive(Clone, Copy, Debug)]
pub struct PeriodEnd {
    pub start_frame: u64,
    pub frames: u32,
    /// The transport as the period left it.
    pub info: TransportInfo,
    pub counts: Counts,
}

/// From the audio half, in the order things happened. A period's
/// messages end with its [`ToControl::Period`].
pub enum ToControl {
    /// A pad note played, `offset` frames into the coming or current
    /// period, for CAPTURE's history.
    Played(u32, Event),
    /// A Perform FX move, for the move log.
    Moved(u32, Input),
    /// A cue started on its bar line.
    Started(Started),
    /// A mute cue landed on its line (the mute queue).
    MuteLanded(MuteLanded),
    /// Pressure values for the next [`ToControl::Rec`], in order.
    Touch([TouchPoint; TOUCH_CHUNK], u8),
    /// Gesture points for the next [`ToControl::Rec`], in order.
    Points([(f64, f32, Kind); POINTS_CHUNK], u8),
    /// A recorded change, with its number. The audio half has made it to
    /// the view it plays.
    Rec(u64, Rec),
    /// The view had no room in place for a recorded change, or a new view
    /// could not catch up ([`Counts::behind`]): compile again.
    Compile,
    /// One of the period's spans, in order.
    Span(Span),
    /// What following another device's clock put right or lost, when it
    /// changed (before the period's end).
    Follow(FollowCounts),
    /// The period ended.
    Period(PeriodEnd),
    /// A view the audio half no longer plays, to free here.
    Done(Box<PlayView>),
    /// A chain the audio half had no use for (its track had one), to free
    /// here.
    Chain(Box<Chain>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_queue_hands_the_message_back_and_counts_it() {
        let (mut tx, rx) = link::<u32>(2);
        assert_eq!(tx.send(1), Ok(()));
        assert_eq!(tx.send(2), Ok(()));
        assert_eq!(tx.send(3), Err(3));
        assert_eq!(tx.full(), 1);
        assert_eq!((rx.recv(), rx.recv(), rx.recv()), (Some(1), Some(2), None));
    }

    #[test]
    fn messages_are_small() {
        // Each slot of the queues is as big as the biggest message.
        assert!(
            std::mem::size_of::<ToControl>() <= 128,
            "{}",
            std::mem::size_of::<ToControl>()
        );
        assert!(std::mem::size_of::<ToAudio>() <= 128);
    }
}
