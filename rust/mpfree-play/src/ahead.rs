//! The look-ahead for slow gear (sequencer S5): docs/dev/design/midi-io.md,
//! "Look-ahead for slow gear". The app's time engine runs it after each
//! real period (`mpfree-app`'s `time::audio`); the instruction bench's
//! "Time: tracks ahead" case measures it.
//!
//! A track whose MIDI port's gear needs more than the sound's lead has an
//! ahead, in frames. After each real period, while the transport rolls, a
//! copy of the transport and the player plays on that far with the same
//! view (the forecast); the ahead tracks' events in it are handed to MIDI
//! out before their own period, each once, and logged: its notes, their
//! pressure, its clips' programs, and its locks and lanes on its own MIDI
//! out controls (`Owner::Midi`). The real pass then
//! checks the log: what it plays that the forecast had is struck off, what
//! the forecast missed goes out late, and a note-on the forecast sent that
//! the real pass didn't play gets its note-off. Nothing here allocates
//! after [`Ahead::new`].

use crate::mpfree_param::{Owner, ParamAddr};
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
/// A period's controls the real pass played without the forecast having
/// sent them (late), for [`Ahead::check`].
const MAX_LATE_CONTROLS: usize = 64;

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
/// sends of a clip (notes, pressure, programs), and its locks and lanes on
/// the track's own MIDI out controls ([`is_gear_control`]). Other locks
/// and lanes go to the mod engine in their own period, and a panic is
/// never forecast.
fn goes_ahead(o: &Out) -> bool {
    matches!(
        o.event,
        Event::NoteOn { .. }
            | Event::NoteOff { .. }
            | Event::Pressure { .. }
            | Event::Program { .. }
    ) || is_gear_control(o)
}

/// The address of a lock, a lane's value or their end.
fn lock_addr(e: &Event) -> Option<ParamAddr> {
    match *e {
        Event::Lock { addr, .. } | Event::Lane { addr, .. } | Event::Unlock { addr } => Some(addr),
        _ => None,
    }
}

/// A lock, a lane's value or their end on one of the event's own track's
/// MIDI out controls (`Owner::Midi`): a controller, the bend or the
/// pressure of the gear the track plays. On a track that plays ahead it
/// goes ahead with the notes: the app hands it to MIDI out, not the mod
/// engine, unless something else moves that control.
pub fn is_gear_control(o: &Out) -> bool {
    lock_addr(&o.event)
        .is_some_and(|a| matches!(a.owner, Owner::Midi(t) if u16::from(t) == o.track))
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
        if !ahead || !goes_ahead(&out) {
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
    /// Offered ahead, in the order they were, until the real pass reaches
    /// their frame; `went` marks those MIDI out took (the rest it refused,
    /// and they go in their own period as before), `struck` those the real
    /// pass played.
    sent: Box<[Out]>,
    went: Box<[bool]>,
    struck: Box<[bool]>,
    n_sent: usize,
    /// This period's controls the real pass played that hadn't gone
    /// ahead (track, address): a control sent ahead that it didn't play
    /// is put back only when the real pass sent it nothing.
    late_controls: [(u16, ParamAddr); MAX_LATE_CONTROLS],
    n_late_controls: usize,
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
            went: vec![false; MAX_SENT].into_boxed_slice(),
            struck: vec![false; MAX_SENT].into_boxed_slice(),
            n_sent: 0,
            late_controls: [(0, ParamAddr::new(Owner::Midi(0), 0)); MAX_LATE_CONTROLS],
            n_late_controls: 0,
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

    /// Whether track `t` plays ahead now.
    pub fn plays_ahead(&self, t: u16) -> bool {
        self.frames.get(usize::from(t)).is_some_and(|&a| a > 0)
    }

    /// The real pass's event `o`: true when it went out ahead already (the
    /// forecast had it), so MIDI out must not have it again. An event the
    /// forecast missed is false, and counted late when its track is past
    /// where it went out to; one MIDI out refused ahead is false and not
    /// late. A note-off the forecast missed is also true, skipped, when
    /// the forecast already sent the same note starting again after it,
    /// which ends the old one.
    pub fn sent_already(&mut self, o: &Out) -> bool {
        if o.track == ALL || !goes_ahead(o) {
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
            if !self.went[i] {
                self.note_late_control(o);
            }
            return self.went[i];
        }
        if let Event::NoteOff { note } = o.event {
            let again = (0..self.n_sent).any(|i| {
                let s = &self.sent[i];
                self.went[i]
                    && s.track == o.track
                    && s.dest == o.dest
                    && s.frame > o.frame
                    && matches!(s.event, Event::NoteOn { note: n, .. } if n == note)
            });
            if again {
                return true;
            }
        }
        self.counts.late += 1;
        self.note_late_control(o);
        false
    }

    /// A control of `o`'s that goes out in the real pass, not ahead.
    fn note_late_control(&mut self, o: &Out) {
        let Some(addr) = lock_addr(&o.event).filter(|_| is_gear_control(o)) else {
            return;
        };
        if let Some(l) = self.late_controls.get_mut(self.n_late_controls) {
            *l = (o.track, addr);
            self.n_late_controls += 1;
        }
    }

    /// After the real pass of the period that ends on frame `end`: each
    /// note-on sent ahead for before `end` that the real pass didn't play,
    /// with no note-off sent after it, is handed to `off` as the note-off
    /// it needs; a control's lock or lane value sent ahead that it didn't
    /// play is handed to `off` as the control's unlock, unless the real
    /// pass sent that control something itself (an edit's new value).
    /// Then the log keeps only what is still ahead.
    pub fn check(&mut self, end: u64, mut off: impl FnMut(&Out)) {
        let mut keep = 0;
        for i in 0..self.n_sent {
            let s = self.sent[i];
            if s.frame >= end {
                self.sent[keep] = s;
                self.went[keep] = self.went[i];
                self.struck[keep] = self.struck[i];
                keep += 1;
                continue;
            }
            if self.struck[i] || !self.went[i] {
                continue;
            }
            match s.event {
                Event::NoteOn { note, .. } => {
                    let ended = (i + 1..self.n_sent).any(|j| {
                        let e = &self.sent[j];
                        self.went[j]
                            && e.track == s.track
                            && e.dest == s.dest
                            && e.event == Event::NoteOff { note }
                    });
                    self.counts.wrong += 1;
                    if !ended {
                        off(&Out {
                            event: Event::NoteOff { note },
                            ..s
                        });
                    }
                }
                Event::Lock { addr, .. } | Event::Lane { addr, .. } => {
                    self.counts.wrong += 1;
                    let later = |j: usize| {
                        let e = &self.sent[j];
                        self.went[j] && e.track == s.track && lock_addr(&e.event) == Some(addr)
                    };
                    let resent = self.late_controls[..self.n_late_controls]
                        .contains(&(s.track, addr))
                        || (i + 1..self.n_sent).any(later);
                    if !resent {
                        off(&Out {
                            event: Event::Unlock { addr },
                            ..s
                        });
                    }
                }
                _ => {}
            }
        }
        self.n_sent = keep;
        self.n_late_controls = 0;
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
            let went = send(&ahead);
            self.sent[self.n_sent] = ahead;
            self.went[self.n_sent] = went;
            self.struck[self.n_sent] = false;
            self.n_sent += 1;
            if went {
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
