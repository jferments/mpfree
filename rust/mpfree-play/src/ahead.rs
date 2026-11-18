//! The look-ahead for slow gear (sequencer S5): docs/dev/design/midi-io.md,
//! "Look-ahead for slow gear". The app's time engine runs it after each
//! real period (`mpfree-app`'s `time::audio`); the instruction bench's
//! "Time: tracks ahead" case measures it.
//!
//! A track whose MIDI port's gear needs more than the sound's lead has an
//! ahead, in frames. After each real period, while the transport rolls, a
//! copy of the transport and the player plays on that far with the same
//! view (the forecast); the ahead tracks' events in it are handed to MIDI
//! out before their own period, each once, and logged. The real pass then
//! checks the log: what it plays that the forecast had is struck off, what
//! the forecast missed goes out late, and a note-on the forecast sent that
//! the real pass didn't play gets its note-off. Nothing here allocates
//! after [`Ahead::new`].

use crate::{Event, Out, PlayView, Player, Sink, ALL};
use mpfree_seq::MAX_TRACKS;
use mpfree_transport::{Block, Transport};

/// The most a track plays ahead, in frames: 100 ms at 48 kHz.
pub const MAX_AHEAD: u32 = 4_800;
// The forecast plays it as one period, and a track is a bit of a u128.
const _: () = assert!(MAX_AHEAD <= mpfree_transport::transport::MAX_PERIOD);
const _: () = assert!(MAX_TRACKS <= 128);
/// Forecast events kept in one period, of the ahead tracks only.
const MAX_KEPT: usize = 1_024;
/// Events handed over ahead and not yet reached by the real pass.
const MAX_SENT: usize = 2_048;

/// What the look-ahead did, for the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AheadCounts {
    /// Events handed to MIDI out ahead of their period.
    pub ahead: u64,
    /// Events the forecast missed (PLAY, a launch or a mute inside the
    /// window, an edit), sent when the real pass played them.
    pub late: u64,
    /// Note-ons sent ahead that the real pass didn't play: each got its
    /// note-off.
    pub wrong: u64,
    /// Forecast events that found no room (sent when the real pass
    /// played them, as late ones).
    pub dropped: u64,
}

/// Whether an event of a track that plays ahead goes ahead: what MIDI out
/// sends of a clip (notes, pressure, programs). Locks and lanes go to the
/// mod engine in their own period, and a panic is never forecast.
fn goes_ahead(e: &Event) -> bool {
    matches!(
        e,
        Event::NoteOn { .. }
            | Event::NoteOff { .. }
            | Event::Pressure { .. }
            | Event::Program { .. }
    )
}

/// A sink that keeps the ahead tracks' events of the forecast.
struct Kept<'a> {
    frames: &'a [u32; MAX_TRACKS],
    out: &'a mut [Out],
    n: &'a mut usize,
    dropped: &'a mut u64,
}

impl Sink for Kept<'_> {
    fn send(&mut self, out: Out) {
        let ahead = self
            .frames
            .get(usize::from(out.track))
            .is_some_and(|&a| a > 0);
        if !ahead || !goes_ahead(&out.event) {
            return;
        }
        if let Some(o) = self.out.get_mut(*self.n) {
            *o = out;
            *self.n += 1;
        } else {
            *self.dropped += 1;
        }
    }
}

/// The look-ahead's copy of the time engine and its bookkeeping, its large
/// parts on the heap (made once, in [`Ahead::new`]), so nothing large is
/// built or copied on the stack.
pub struct Ahead {
    transport: Box<Transport>,
    player: Box<Player>,
    block: Box<Block>,
    /// Each track's ahead in frames, as the app last set it (0: none).
    frames: [u32; MAX_TRACKS],
    /// Each track's events went out up to here (not including it).
    upto: [u64; MAX_TRACKS],
    kept: Box<[Out]>,
    n_kept: usize,
    /// Handed over ahead, in the order they went, until the real pass
    /// reaches their frame; `struck` marks those it played.
    sent: Box<[Out]>,
    struck: Box<[bool]>,
    n_sent: usize,
    counts: AheadCounts,
}

impl Ahead {
    /// A copy of `transport` to play ahead with (it is copied again each
    /// period before it plays).
    pub fn new(transport: &Transport) -> Ahead {
        let blank = Out {
            offset: 0,
            frame: 0,
            track: 0,
            dest: 0,
            event: Event::AllOff,
        };
        Ahead {
            transport: Box::new(*transport),
            player: Box::new(Player::new()),
            block: Box::new(Block::new()),
            frames: [0; MAX_TRACKS],
            upto: [0; MAX_TRACKS],
            kept: vec![blank; MAX_KEPT].into_boxed_slice(),
            n_kept: 0,
            sent: vec![blank; MAX_SENT].into_boxed_slice(),
            struck: vec![false; MAX_SENT].into_boxed_slice(),
            n_sent: 0,
            counts: AheadCounts::default(),
        }
    }

    /// Each track's ahead, in frames (capped at [`MAX_AHEAD`]); tracks
    /// past the slice have none.
    pub fn set(&mut self, frames: &[u32]) {
        for (t, a) in self.frames.iter_mut().enumerate() {
            *a = frames.get(t).copied().unwrap_or(0).min(MAX_AHEAD);
        }
    }

    /// Track `t` plays no further ahead (its clips play through its arp).
    pub fn clear(&mut self, t: usize) {
        if let Some(a) = self.frames.get_mut(t) {
            *a = 0;
        }
    }

    pub fn counts(&self) -> AheadCounts {
        self.counts
    }

    /// The real pass's event `o`: true when it went out ahead already (the
    /// forecast had it), so MIDI out must not have it again. An event the
    /// forecast missed is false, and counted late when its track is past
    /// where it went out to. A note-off the forecast missed is also true,
    /// skipped, when the forecast already sent the same note starting
    /// again after it, which ends the old one.
    pub fn sent_already(&mut self, o: &Out) -> bool {
        if o.track == ALL || !goes_ahead(&o.event) {
            return false;
        }
        let t = usize::from(o.track);
        if self.upto.get(t).is_none_or(|&u| o.frame >= u) {
            return false;
        }
        let same = |s: &Out| {
            s.frame == o.frame && s.track == o.track && s.dest == o.dest && s.event == o.event
        };
        if let Some(i) = (0..self.n_sent).find(|&i| !self.struck[i] && same(&self.sent[i])) {
            self.struck[i] = true;
            return true;
        }
        if let Event::NoteOff { note } = o.event {
            let again = self.sent[..self.n_sent].iter().any(|s| {
                s.track == o.track
                    && s.dest == o.dest
                    && s.frame > o.frame
                    && matches!(s.event, Event::NoteOn { note: n, .. } if n == note)
            });
            if again {
                return true;
            }
        }
        self.counts.late += 1;
        false
    }

    /// After the real pass of the period that ends on frame `end`: each
    /// note-on sent ahead for before `end` that the real pass didn't play,
    /// with no note-off sent after it, is handed to `off` as the note-off
    /// it needs; then the log keeps only what is still ahead.
    pub fn check(&mut self, end: u64, mut off: impl FnMut(&Out)) {
        let mut keep = 0;
        for i in 0..self.n_sent {
            let s = self.sent[i];
            if s.frame >= end {
                self.sent[keep] = s;
                self.struck[keep] = self.struck[i];
                keep += 1;
                continue;
            }
            if self.struck[i] {
                continue;
            }
            if let Event::NoteOn { note, .. } = s.event {
                let ended = (i + 1..self.n_sent).any(|j| {
                    let e = &self.sent[j];
                    e.track == s.track && e.dest == s.dest && e.event == Event::NoteOff { note }
                });
                self.counts.wrong += 1;
                if !ended {
                    off(&Out {
                        event: Event::NoteOff { note },
                        ..s
                    });
                }
            }
        }
        self.n_sent = keep;
    }

    /// The forecast: after the real pass of `frames` frames (the
    /// transport and the player as `transport` and `player` left it, at
    /// frame `end`), copies of both play on to the furthest ahead, in one
    /// period, with `view`. Each ahead track's events, from where it
    /// went out to up to `end` plus its ahead, go to `send` with their
    /// offset from the period's first frame (`end - frames`, so past the
    /// period's end); those it takes are logged. Not while stopped.
    pub fn forecast(
        &mut self,
        transport: &Transport,
        player: &Player,
        view: &PlayView,
        frames: u32,
        mut send: impl FnMut(&Out) -> bool,
    ) {
        let end = transport.frame();
        let start = end.saturating_sub(u64::from(frames));
        let most = self.frames.iter().copied().max().unwrap_or(0);
        if most == 0 || frames == 0 || !transport.state().rolling() {
            return;
        }
        // In place, with no temporary: the transport is Copy, and the
        // player copies only its tables' used part.
        *self.transport = *transport;
        self.player.copy_from(player);
        // Only the tracks that play ahead: the rest's events would be let
        // go.
        let mut only = 0u128;
        for (t, &a) in self.frames.iter().enumerate() {
            if a > 0 {
                only |= 1 << t;
            }
        }
        self.player.only(only);
        self.n_kept = 0;
        // In one period of the furthest ahead: the player and the
        // transport play the same events for any period size.
        if self.transport.process(most, &mut self.block) {
            let mut kept = Kept {
                frames: &self.frames,
                out: &mut self.kept,
                n: &mut self.n_kept,
                dropped: &mut self.counts.dropped,
            };
            self.player.process(view, &self.block, &mut kept);
        }
        for i in 0..self.n_kept {
            let o = self.kept[i];
            let t = usize::from(o.track);
            let (Some(&a), Some(&upto)) = (self.frames.get(t), self.upto.get(t)) else {
                continue;
            };
            if o.frame < upto || o.frame >= end + u64::from(a) {
                continue;
            }
            if self.n_sent == MAX_SENT {
                self.counts.dropped += 1;
                continue;
            }
            let ahead = Out {
                offset: u32::try_from(o.frame - start).unwrap_or(u32::MAX),
                ..o
            };
            if send(&ahead) {
                self.sent[self.n_sent] = ahead;
                self.struck[self.n_sent] = false;
                self.n_sent += 1;
                self.counts.ahead += 1;
            }
        }
        for (t, &a) in self.frames.iter().enumerate() {
            if a > 0 {
                self.upto[t] = self.upto[t].max(end + u64::from(a));
            }
        }
    }
}
