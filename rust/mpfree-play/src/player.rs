//! The player: each period, plays the view's clips on the transport's spans.
//!
//! **Whole ticks.** A note's song position (after quantise, speed and
//! swing) is rounded to the nearest tick (1/960 of a beat, 0.26 ms at
//! 120 BPM, the MPC's own resolution). A note plays when its tick is one of
//! the ticks the transport plays in a span (`Span::ticks`), on that tick's
//! frame. The transport plays every tick exactly once between jumps, the
//! same for any period size, so notes inherit that: none is lost or doubled
//! at a period edge or a tempo change. Clip positions are worked out from
//! the tick each time, never counted, so a loop jump or a locate lands every
//! clip in the right place.
//!
//! **Note-offs** come on the tick where the note ends. A loop jump, a locate
//! or a stop ends every sounding note on the frame where it happens (sounds
//! keep their release tails); a panic (STOP while stopped) also sends
//! `AllOff`. A note that starts while the same note is still sounding on the
//! same track ends the old one first, on the same frame.
//!
//! **No allocation, locks or panics** in `process`: fixed tables only. Full
//! tables are counted in [`Stats`], never silently dropped.

use std::ops::Range;

use mpfree_seq::DestId;
use mpfree_transport::time::{Swing, Tick};
use mpfree_transport::{Block, Mark, Marker, Span, State};

use crate::view::{ClipView, PlayView};

/// Notes sounding at once, across all tracks.
pub const MAX_ACTIVE: usize = 256;
/// Notes starting in one span.
pub const MAX_STARTS: usize = 256;
/// `dest` and `track` of an event meant for every destination.
pub const ALL: u16 = u16::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    NoteOn { note: u8, velocity: u8 },
    NoteOff { note: u8 },
    /// Cut every sound (panic).
    AllOff,
}

/// One event for a destination, on an exact frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Out {
    /// Frame offset in the period.
    pub offset: u32,
    /// Absolute frame.
    pub frame: u64,
    /// The track it came from ([`ALL`] for `AllOff`).
    pub track: u16,
    /// The destination ([`ALL`] for every one).
    pub dest: DestId,
    pub event: Event,
}

/// Where the player's events go: the host maps each destination to the
/// sampler, the arp, MIDI out ... Events arrive in frame order.
pub trait Sink {
    fn send(&mut self, out: Out);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Notes started.
    pub notes: u64,
    /// Notes not started because a table was full. Should stay 0.
    pub dropped: u64,
    /// Note-offs that came after their tick (should stay 0).
    pub late_offs: u64,
}

#[derive(Clone, Copy, Debug)]
struct Sounding {
    track: u16,
    dest: DestId,
    note: u8,
    off: Tick,
    /// Start order, to break ties.
    seq: u64,
}

#[derive(Clone, Copy, Debug)]
struct Start {
    tick: Tick,
    track: u16,
    dest: DestId,
    note: u8,
    velocity: u8,
    off: Tick,
}

const NO_SOUND: Sounding = Sounding { track: 0, dest: 0, note: 0, off: 0, seq: 0 };
const NO_START: Start = Start { tick: 0, track: 0, dest: 0, note: 0, velocity: 0, off: 0 };

pub struct Player {
    sounding: [Sounding; MAX_ACTIVE],
    n_sounding: usize,
    starts: [Start; MAX_STARTS],
    n_starts: usize,
    seq: u64,
    stats: Stats,
}

impl Default for Player {
    fn default() -> Self {
        Player::new()
    }
}

impl Player {
    pub fn new() -> Player {
        Player {
            sounding: [NO_SOUND; MAX_ACTIVE],
            n_sounding: 0,
            starts: [NO_START; MAX_STARTS],
            n_starts: 0,
            seq: 0,
            stats: Stats::default(),
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Notes sounding now (started, not yet ended).
    pub fn sounding(&self) -> usize {
        self.n_sounding
    }

    /// Play one period: the transport's `block` for it, the clips in `view`.
    pub fn process(&mut self, view: &PlayView, block: &Block, out: &mut impl Sink) {
        let markers = block.markers();
        let mut m = 0;
        for span in block.spans() {
            // A marker on a span's first frame happened just before it.
            while m < markers.len() && markers[m].offset <= span.offset {
                self.marker(&markers[m], out);
                m += 1;
            }
            if matches!(span.state, State::Playing | State::Recording) {
                self.play_span(view, span, out);
            }
        }
        for marker in &markers[m..] {
            self.marker(marker, out);
        }
    }

    fn marker(&mut self, m: &Marker, out: &mut impl Sink) {
        match m.mark {
            Mark::Looped { .. } | Mark::Located { .. } | Mark::Stopped { .. } => self.end_all(m.offset, m.frame, out),
            Mark::Panic => {
                self.end_all(m.offset, m.frame, out);
                out.send(Out { offset: m.offset, frame: m.frame, track: ALL, dest: ALL, event: Event::AllOff });
            }
            _ => {}
        }
    }

    /// End every sounding note, in the order they started.
    fn end_all(&mut self, offset: u32, frame: u64, out: &mut impl Sink) {
        for s in &self.sounding[..self.n_sounding] {
            out.send(Out { offset, frame, track: s.track, dest: s.dest, event: Event::NoteOff { note: s.note } });
        }
        self.n_sounding = 0;
    }

    fn play_span(&mut self, view: &PlayView, span: &Span, out: &mut impl Sink) {
        let ticks = span.ticks();
        if ticks.is_empty() {
            return;
        }
        self.n_starts = 0;
        for (i, track) in view.tracks.iter().enumerate() {
            if track.mute {
                continue;
            }
            if let Some(clip) = &track.clip {
                self.collect(clip, i as u16, track.dest, span.swing, &ticks);
            }
        }
        // In tick order; equal ticks keep track order, then note order. An
        // insertion sort: stable, in place (no allocation), and fast here
        // because each track's notes already come in order.
        for i in 1..self.n_starts {
            let s = self.starts[i];
            let mut j = i;
            while j > 0 && self.starts[j - 1].tick > s.tick {
                self.starts[j] = self.starts[j - 1];
                j -= 1;
            }
            self.starts[j] = s;
        }
        let first = ticks.start;
        for i in 0..self.n_starts {
            let s = self.starts[i];
            // Notes ending on or before this tick end first.
            self.end_until(s.tick, first, span, out);
            let (offset, frame) = place(span, s.tick);
            if let Some(j) = self.find(s.track, s.dest, s.note) {
                out.send(Out { offset, frame, track: s.track, dest: s.dest, event: Event::NoteOff { note: s.note } });
                self.remove(j);
            }
            if self.n_sounding == MAX_ACTIVE {
                self.stats.dropped += 1;
                continue;
            }
            out.send(Out {
                offset,
                frame,
                track: s.track,
                dest: s.dest,
                event: Event::NoteOn { note: s.note, velocity: s.velocity },
            });
            self.sounding[self.n_sounding] =
                Sounding { track: s.track, dest: s.dest, note: s.note, off: s.off, seq: self.seq };
            self.n_sounding += 1;
            self.seq += 1;
            self.stats.notes += 1;
        }
        self.end_until(ticks.end - 1, first, span, out);
    }

    /// The notes of `clip` whose tick is in `ticks`.
    fn collect(&mut self, clip: &ClipView, track: u16, dest: DestId, transport_swing: Swing, ticks: &Range<Tick>) {
        if clip.notes.is_empty() || clip.song_len <= 0.0 {
            return;
        }
        let swing = clip.swing.unwrap_or(transport_swing);
        // Swing only delays notes, by at most this much; rounding moves them
        // by half a tick. Look a little wider and keep the exact matches.
        let delay = f64::from(swing.grid) * f64::from(swing.amount.saturating_sub(50)) / 50.0;
        let lo = ticks.start as f64 - delay - 1.0;
        let hi = ticks.end as f64 + 1.0;
        let origin = clip.origin as f64;
        let first_pass = ((lo - origin) / clip.song_len).floor().max(0.0);
        let last_pass = ((hi - origin) / clip.song_len).floor();
        if last_pass < first_pass {
            return;
        }
        let mut pass = first_pass;
        while pass <= last_pass {
            let start = origin + pass * clip.song_len;
            let from = clip.speed.to_clip(lo - start);
            let to = clip.speed.to_clip(hi - start);
            let first = clip.notes.partition_point(|n| n.local < from);
            for n in &clip.notes[first..] {
                if n.local > to {
                    break;
                }
                let straight = start + clip.speed.to_song(n.local);
                let tick = swing.apply(straight).round() as Tick;
                if !ticks.contains(&tick) {
                    continue;
                }
                let off = (swing.apply(straight + n.len).round() as Tick).max(tick + 1);
                if self.n_starts == MAX_STARTS {
                    self.stats.dropped += 1;
                    continue;
                }
                self.starts[self.n_starts] = Start { tick, track, dest, note: n.note, velocity: n.velocity, off };
                self.n_starts += 1;
            }
            pass += 1.0;
        }
    }

    /// End the sounding notes whose end tick is at or before `tick`, in
    /// tick order (ties in start order).
    fn end_until(&mut self, tick: Tick, first: Tick, span: &Span, out: &mut impl Sink) {
        loop {
            let mut best: Option<usize> = None;
            for (i, s) in self.sounding[..self.n_sounding].iter().enumerate() {
                if s.off <= tick && best.is_none_or(|b| (s.off, s.seq) < (self.sounding[b].off, self.sounding[b].seq)) {
                    best = Some(i);
                }
            }
            let Some(i) = best else { return };
            let s = self.sounding[i];
            let at = if s.off < first {
                self.stats.late_offs += 1;
                first
            } else {
                s.off
            };
            let (offset, frame) = place(span, at);
            out.send(Out { offset, frame, track: s.track, dest: s.dest, event: Event::NoteOff { note: s.note } });
            self.remove(i);
        }
    }

    fn find(&self, track: u16, dest: DestId, note: u8) -> Option<usize> {
        self.sounding[..self.n_sounding].iter().position(|s| s.track == track && s.dest == dest && s.note == note)
    }

    /// Remove keeping the start order.
    fn remove(&mut self, i: usize) {
        self.sounding.copy_within(i + 1..self.n_sounding, i);
        self.n_sounding -= 1;
    }
}

/// Offset in the period and absolute frame of a tick in a span.
fn place(span: &Span, tick: Tick) -> (u32, u64) {
    let frame = span.frame_of(tick as f64).clamp(span.frame, span.frame + u64::from(span.len) - 1);
    (span.offset + (frame - span.frame) as u32, frame)
}
