//! One arpeggiator per track (owner decisions 2 and 3, 2026-11-11, in
//! docs/dev/design/arpeggiator.md).
//!
//! - **One arp per track.** Each track slot has its own [`ArpHost`] (and
//!   so its own engine, settings, latch and pattern). A latched arp keeps
//!   running while the pads play another track.
//! - **Clips through the arp** ([`TrackArps::set_through`], the project's
//!   `Track::through_arp`). On (melodic tracks): the track's clip notes go
//!   into its arp, as the live pads do, so a clip recorded from the chord
//!   you played is arpeggiated again on playback. Off (drum tracks, the
//!   default): clip notes skip the arp and play as recorded, since what was
//!   recorded is already what the arp played. Either way a part is never
//!   arpeggiated twice.
//! - **What to record** ([`TrackArps::records_pads`]): with the switch on,
//!   the recorder (and CAPTURE) takes the track's pad notes before the arp;
//!   off, it takes the arp's output. The host feeds the recorder; this
//!   module only says which side.
//!
//! Each period: `player.process(view, block, &mut arps.through(&mut sink))`,
//! [`TrackArps::pad`] for live pad notes (any time before the next step),
//! then [`TrackArps::finish`] with the same block and sink. Notes that skip
//! the arps and every arp's notes reach the sink together, in frame order
//! (notes on the same frame keep the order they came in, the player's
//! first). `AllOff` (a panic) passes through and clears every arp. A
//! through track's parameter locks skip its arp and reach the sink as the
//! player sent them, before the arp's notes on the same frame.
//!
//! [`TrackArps::new`] allocates (an engine for each of the 128 tracks a
//! project can have, about 1.5 MB) and belongs on the control thread at
//! start; nothing else allocates. Each period only visits the tracks up to
//! the highest one anything has asked for (its settings, a note, its
//! switch or destination), so a project of 8 tracks pays for 8. An arp
//! given nothing in a period whose engine is idle (`Engine::idle`) skips
//! that period (`Engine::skip`), so idle tracks cost next to nothing. An
//! engine a period doesn't visit has never been given anything, so it is
//! as a new one: its first sync lines it up with the transport.

use mpfree_arp::{Engine, EventKind};
use mpfree_seq::{DestId, MAX_TRACKS};
use mpfree_transport::Block;

use crate::arp::ArpHost;
use crate::player::{Event, Out, Sink, ALL};

/// Notes one period can carry to the sink through [`TrackArps`]; more are
/// counted in [`TrackArps::dropped`].
pub const MAX_OUT: usize = 1024;

/// The seed [`TrackArps::reseed`] gives every arp.
pub const RESEED: u32 = 0x2545_f491;

pub struct TrackArps {
    arps: Vec<ArpHost>,
    through: [bool; MAX_TRACKS],
    dest: [DestId; MAX_TRACKS],
    /// Tracks whose arp was given a note (or a panic) this period. An arp
    /// with none and an idle engine skips its period, which keeps 15 idle
    /// arps nearly free.
    fed: [bool; MAX_TRACKS],
    /// Tracks up to the highest one asked for: what a period visits.
    used: usize,
    /// This period's notes for the sink, until [`TrackArps::finish`].
    out: [Out; MAX_OUT],
    /// Which of `out` an arp played (rather than the player passing it on).
    from_arp: [bool; MAX_OUT],
    n_out: usize,
    dropped: u64,
}

impl TrackArps {
    /// One arp per track, each Direct (playing what it is given) until its
    /// settings change. Track `t` plays into destination `t` until
    /// [`TrackArps::set_dest`] says otherwise. Allocates.
    pub fn new(sample_rate: u32, bpm: f64) -> TrackArps {
        let blank = Out {
            offset: 0,
            frame: 0,
            track: 0,
            dest: 0,
            event: Event::AllOff,
        };
        TrackArps {
            arps: (0..MAX_TRACKS)
                .map(|_| ArpHost::new(Engine::new(sample_rate, bpm)))
                .collect(),
            through: [false; MAX_TRACKS],
            dest: core::array::from_fn(|t| t as DestId),
            fed: [false; MAX_TRACKS],
            used: 0,
            out: [blank; MAX_OUT],
            from_arp: [false; MAX_OUT],
            n_out: 0,
            dropped: 0,
        }
    }

    /// Track `track`'s arp (settings, latch, pattern; change them between
    /// periods). None past the last track.
    pub fn arp(&mut self, track: usize) -> Option<&mut ArpHost> {
        self.ask(track);
        self.arps.get_mut(track)
    }

    /// The engine of track `track`'s arp.
    pub fn engine(&mut self, track: usize) -> Option<&mut Engine> {
        self.ask(track);
        self.arps.get_mut(track).map(|a| &mut a.engine)
    }

    /// Every track's engine, to change a setting on all of them (the
    /// tempo, a project's arps) without marking them in use: settings
    /// alone never start an engine (it runs only with notes held).
    pub fn engines_mut(&mut self) -> impl Iterator<Item = &mut Engine> {
        self.arps.iter_mut().map(|a| &mut a.engine)
    }

    /// Track `track` is in use: periods visit it from now on.
    fn ask(&mut self, track: usize) {
        if track < self.arps.len() {
            self.used = self.used.max(track + 1);
        }
    }

    /// Tracks a period visits: up to the highest one asked for.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Track `track`'s engine to read (the last track's past the end).
    pub fn engine_at(&self, track: usize) -> &Engine {
        &self.arps[track.min(MAX_TRACKS - 1)].engine
    }

    /// Track `track`'s engine to change (the last track's past the end).
    pub fn engine_at_mut(&mut self, track: usize) -> &mut Engine {
        let t = track.min(MAX_TRACKS - 1);
        self.ask(t);
        &mut self.arps[t].engine
    }

    /// Every arp's random orders start again from the same seed, so a
    /// range played again from its start (an export's replay) plays the
    /// same random notes as before. Real-time safe.
    pub fn reseed(&mut self) {
        for a in &mut self.arps {
            a.engine.set_seed(RESEED);
        }
    }

    /// Notes no engine could queue, over every track, since the start.
    pub fn overflow(&self) -> u64 {
        self.arps.iter().map(|a| a.engine.overflow).sum()
    }

    /// "Clips through the arp" for `track`. A change clears that track's
    /// arp, if it holds anything (held and latched notes, and what they
    /// sound), at the start of the next period, so no note the clip held in it, or that should now reach
    /// it, is left hanging.
    pub fn set_through(&mut self, track: usize, on: bool) {
        self.ask(track);
        if let Some(t) = self.through.get_mut(track) {
            if *t != on {
                *t = on;
                if !self.arps[track].engine.active().is_empty() {
                    self.arps[track].input(0, Event::AllOff);
                    self.fed[track] = true;
                }
            }
        }
    }

    pub fn through(&self, track: usize) -> bool {
        self.through.get(track).copied().unwrap_or(false)
    }

    /// Whether the recorder and CAPTURE take `track`'s pad notes as played
    /// (true: the switch is on, the clip will play through the arp) or the
    /// arp's output (false).
    pub fn records_pads(&self, track: usize) -> bool {
        self.through(track)
    }

    /// Where `track`'s arp plays: the track's destination (the track
    /// host's slot, the sampler's kit ...).
    pub fn set_dest(&mut self, track: usize, dest: DestId) {
        self.ask(track);
        if let Some(d) = self.dest.get_mut(track) {
            *d = dest;
        }
    }

    /// A live pad note for `track`, `offset` frames into the coming period.
    /// The pads always play through their track's arp.
    pub fn pad(&mut self, track: usize, offset: u32, event: Event) {
        self.ask(track);
        if let Some(a) = self.arps.get_mut(track) {
            a.input(offset, event);
            self.fed[track] = true;
        }
    }

    /// Notes lost so far: the arps' own and notes that did not fit in one
    /// period's output. It should stay 0.
    pub fn dropped(&self) -> u64 {
        self.dropped + self.arps.iter().map(ArpHost::dropped).sum::<u64>()
    }

    /// The [`Sink`] to hand the player this period.
    pub fn through_sink(&mut self) -> Through<'_> {
        Through { arps: self }
    }

    /// Runs every track's arp over `block` and sends this period's notes,
    /// the player's that skipped the arps and the arps' own, to `sink` in
    /// frame order.
    pub fn finish(&mut self, block: &Block, sink: &mut impl Sink) {
        self.finish_with(block, |o, _| sink.send(*o));
    }

    /// [`TrackArps::finish`], telling `f` for each note whether an arp
    /// played it (true) or the player passed it on (false), so a host can
    /// treat what the arps play from the pads as live playing.
    pub fn finish_with(&mut self, block: &Block, mut f: impl FnMut(&Out, bool)) {
        let player_notes = self.n_out;
        self.from_arp[..player_notes].fill(false);
        for t in 0..self.used {
            if !std::mem::take(&mut self.fed[t]) && self.arps[t].engine.idle() {
                self.arps[t].engine.skip(block.frames);
                continue;
            }
            let (track, dest) = (t as u16, self.dest[t]);
            let start = block.start_frame;
            let (arps, out, from_arp, n_out, dropped) = (
                &mut self.arps,
                &mut self.out,
                &mut self.from_arp,
                &mut self.n_out,
                &mut self.dropped,
            );
            arps[t].process(block, &mut |offset, kind| {
                let event = match kind {
                    EventKind::On { note, velocity } => Event::NoteOn { note, velocity },
                    EventKind::Off { note } => Event::NoteOff { note },
                };
                if *n_out < MAX_OUT {
                    from_arp[*n_out] = true;
                }
                push(
                    out,
                    n_out,
                    dropped,
                    Out {
                        offset,
                        frame: start + u64::from(offset),
                        track,
                        dest,
                        event,
                    },
                );
            });
        }
        // Each source is in order already; a stable insertion sort merges
        // them without allocating (a period holds few notes).
        let n = self.n_out;
        let (out, from_arp) = (&mut self.out[..n], &mut self.from_arp[..n]);
        for i in player_notes.max(1)..n {
            let mut j = i;
            while j > 0 && out[j - 1].offset > out[j].offset {
                out.swap(j - 1, j);
                from_arp.swap(j - 1, j);
                j -= 1;
            }
        }
        for (o, &a) in out.iter().zip(from_arp.iter()) {
            f(o, a);
        }
        self.n_out = 0;
    }
}

fn push(out: &mut [Out; MAX_OUT], n: &mut usize, dropped: &mut u64, o: Out) {
    if *n == MAX_OUT {
        *dropped += 1;
    } else {
        out[*n] = o;
        *n += 1;
    }
}

/// The player's [`Sink`] for one period: a through track's notes go into
/// its arp (its pressure too, which also goes on), everything else (other tracks' notes, and every parameter
/// lock and unlock, which an arp has no use for) waits for
/// [`TrackArps::finish`].
pub struct Through<'a> {
    arps: &'a mut TrackArps,
}

impl Sink for Through<'_> {
    fn send(&mut self, o: Out) {
        let a = &mut *self.arps;
        let t = usize::from(o.track);
        if o.track == ALL || matches!(o.event, Event::AllOff) {
            // Only the tracks in use hold anything to clear.
            let used = a.used;
            for arp in a.arps[..used].iter_mut() {
                arp.input(o.offset, Event::AllOff);
            }
            a.fed[..used].fill(true);
        } else if a.through(t) && matches!(o.event, Event::NoteOn { .. } | Event::NoteOff { .. }) {
            a.arps[t].input(o.offset, o.event);
            a.fed[t] = true;
            return;
        } else if a.through(t) && matches!(o.event, Event::Pressure { .. }) {
            // The arp's repeats follow a clip's pressure, and the sound
            // hears it too, as with the live pads.
            a.arps[t].input(o.offset, o.event);
            a.fed[t] = true;
        }
        push(&mut a.out, &mut a.n_out, &mut a.dropped, o);
    }
}
