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
//!
//! **Arps that work in notes** (the instrument redesign's step 5d,
//! [`TrackArps::set_on_notes`]). On a track whose synth plays notes the
//! live pads press notes into its arp, a clip of notes goes in as it is
//! ([`crate::arp::CLIP_NOTES`]), a clip of pad hits goes in as its pads'
//! notes ([`TrackArps::set_pads`], each hit's end and pressure reaching
//! what it pressed), and what the arp plays goes out as notes
//! ([`Out::notes`]). Elsewhere (a kit, the Drums engine, MIDI out) the arp
//! works in pad hits, as before.
//!
//! **MIDI effects** (`mpfree-notefx`, docs/dev/design/note-fx.md). A
//! track with any gets a [`Chain`] around its arp: the effects above the
//! arp's place ([`TrackFx::arp_at`]) change the notes going into it, the
//! rest what it plays, so whatever reaches the arp reaches them (the live
//! pads, and the clips with "Clips through the arp" on) and the rule above
//! holds for them too: what was recorded after them never goes through
//! them again. A chain is about 400 KB, so the host makes it off the sound
//! loop ([`TrackArps::give_chain`]); [`TrackArps::set_notes`] only sets
//! what it runs. A panic, or the switch changing, cuts the chain where it
//! lands ([`Chain::cut`]): what it has on ends there, and nothing waiting
//! (an echo, a strummed note) plays after it. So does a mute
//! ([`TrackArps::cut`]).
//!
//! **No ending is ever lost** (step sequencer refactor A2; playback.md,
//! "Endings always get through"). The period's notes for the sink wait in
//! a [`Table`] whose [`crate::room::Room`] keeps the player's notes and
//! the arps' apart, each paired on its own, so every note-off of a note it
//! let in has its place; each arp's input keeps the same rule
//! ([`crate::arp`]). A note-off goes where its note-on went: into the arp
//! if the arp has it from the clips, to the sink if not, whatever the
//! switch says now, so turning "Clips through the arp" on or off never
//! leaves a note hanging. A period too busy leaves new notes out
//! ([`TrackArps::refused`]); [`TrackArps::lost`] counts an ending that
//! found no place anyway, a bug, which must stay 0.

use mpfree_arp::{Engine, EventKind};
use mpfree_notefx::{Chain, Ev, Kind, Notes, Period, TrackFx, MAX_FX};
use mpfree_seq::{DestId, MAX_TRACKS};
use mpfree_transport::Block;

use crate::arp::{ArpHost, CLIPS, CLIP_NOTES, EFFECTS, PADS};
use crate::player::{Event, Out, Sink, ALL};
use crate::room::{Table, ARPS, PLAYER, ROOM};

/// Events one period can carry to the sink through [`TrackArps`]: its
/// table's places ([`ROOM`]).
pub const MAX_OUT: usize = ROOM;

/// Most notes one pad plays: a pad of chords
/// (`mpfree_arp::scale::pads::MAX_PAD_NOTES`).
pub const PAD_NOTES: usize = 4;

/// The pads of a track's pad layout: 16.
pub const LAYOUT_PADS: usize = 16;

/// The notes each of a track's 16 pads plays, a chord's root last (the
/// instrument redesign's step 5c and 5d): the step grid shows and edits a
/// clip of notes through them, and an arp that works in notes
/// ([`TrackArps::set_on_notes`]) plays a clip of pad hits by them. Fixed
/// size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadRows {
    notes: [[u8; PAD_NOTES]; LAYOUT_PADS],
    n: [u8; LAYOUT_PADS],
}

impl PadRows {
    /// Each pad plays its own number: a clip of pad hits' rows.
    pub const PADS: PadRows = {
        let mut r = PadRows {
            notes: [[0; PAD_NOTES]; LAYOUT_PADS],
            n: [1; LAYOUT_PADS],
        };
        let mut p = 0;
        while p < LAYOUT_PADS {
            r.notes[p][0] = p as u8;
            p += 1;
        }
        r
    };

    /// Pad `pad` plays `notes` (the first [`PAD_NOTES`]), or nothing when
    /// there are none (its notes fall outside 0 to 127: Convert to notes
    /// refuses its hits, and a hit on it plays nothing, as on its
    /// instrument). A pad past 16 changes nothing.
    pub fn set(&mut self, pad: usize, notes: &[u8]) {
        let Some(row) = self.notes.get_mut(pad) else {
            return;
        };
        let n = notes.len().min(PAD_NOTES);
        row[..n].copy_from_slice(&notes[..n]);
        self.n[pad] = n as u8;
    }

    /// The notes pad `pad` plays, its root last; none for a pad that
    /// plays nothing, or past the 16 pads.
    pub fn notes(&self, pad: usize) -> &[u8] {
        match (self.notes.get(pad), self.n.get(pad)) {
            (Some(row), Some(&n)) => &row[..usize::from(n)],
            _ => &[],
        }
    }

    /// Each pad's root: the note its row shows (its own number for a pad
    /// that plays nothing).
    pub fn roots(&self) -> [u8; LAYOUT_PADS] {
        core::array::from_fn(|p| self.notes(p).last().copied().unwrap_or(p as u8))
    }
}

/// What a clip's pad hit pressed into an arp that works in notes: the
/// pad's notes as they were then, so its end and pressure reach the same
/// notes though the layout changed, and how many of its hits still hold
/// them.
#[derive(Clone, Copy, Debug, Default)]
struct Pressed {
    notes: [u8; PAD_NOTES],
    n: u8,
    holds: u8,
}

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
    /// This period's notes for the sink, until [`TrackArps::finish`],
    /// each marked when an arp played it (rather than the player passing
    /// it on).
    table: Table,
    rate: u32,
    /// Each track's MIDI effects, once the host has given it a chain.
    chains: Vec<Option<Box<Chain>>>,
    /// Where each track's arp sits among its effects ([`TrackFx::arp_at`]).
    arp_at: [u8; MAX_TRACKS],
    /// A mute landing in the coming period, per track: its offset.
    cuts: [Option<u32>; MAX_TRACKS],
    /// A chained track's notes in a period: into its chain, then what its
    /// arp played.
    lists: Box<[Notes; 2]>,
    /// Tracks whose arp works in notes ([`TrackArps::set_on_notes`]).
    on_notes: [bool; MAX_TRACKS],
    /// What each track's arp's notes go out as this period: notes or pad
    /// hits ([`Out::notes`]). It takes `on_notes` at each period's end, so
    /// the endings a change sends out go as what they end.
    notes_out: [bool; MAX_TRACKS],
    /// Each track's pads' notes, for its clips' pad hits into an arp that
    /// works in notes.
    pads: Box<[PadRows; MAX_TRACKS]>,
    /// What each track's clips' pad hits pressed into its arp, by pad.
    pressed: Box<[[Pressed; LAYOUT_PADS]; MAX_TRACKS]>,
}

impl TrackArps {
    /// One arp per track, each Direct (playing what it is given) until its
    /// settings change. Track `t` plays into destination `t` until
    /// [`TrackArps::set_dest`] says otherwise. Allocates.
    pub fn new(sample_rate: u32, bpm: f64) -> TrackArps {
        TrackArps {
            arps: (0..MAX_TRACKS)
                .map(|_| ArpHost::new(Engine::new(sample_rate, bpm)))
                .collect(),
            through: [false; MAX_TRACKS],
            dest: core::array::from_fn(|t| t as DestId),
            fed: [false; MAX_TRACKS],
            used: 0,
            table: Table::new(),
            rate: sample_rate,
            chains: (0..MAX_TRACKS).map(|_| None).collect(),
            arp_at: [0; MAX_TRACKS],
            cuts: [None; MAX_TRACKS],
            lists: Box::new([Notes::new(), Notes::new()]),
            on_notes: [false; MAX_TRACKS],
            notes_out: [false; MAX_TRACKS],
            pads: Box::new([PadRows::PADS; MAX_TRACKS]),
            pressed: Box::new([[Pressed::default(); LAYOUT_PADS]; MAX_TRACKS]),
        }
    }

    /// Track `track`'s MIDI effects and where its arp sits among them,
    /// between periods. Never allocates: a track with no chain plays as
    /// it is until the host gives it one ([`TrackArps::give_chain`]);
    /// call this again after.
    pub fn set_notes(&mut self, track: usize, fx: &TrackFx) {
        let Some(at) = self.arp_at.get_mut(track) else {
            return;
        };
        *at = fx.arp_place() as u8;
        if let Some(c) = self.chains[track].as_deref_mut() {
            c.apply(fx);
        }
        if !fx.is_empty() {
            self.ask(track);
        }
    }

    /// Whether `track` has a chain for its MIDI effects.
    pub fn has_chain(&self, track: usize) -> bool {
        self.chains.get(track).is_some_and(Option::is_some)
    }

    /// Track `track`'s chain, to read.
    pub fn chain(&self, track: usize) -> Option<&Chain> {
        self.chains.get(track).and_then(|c| c.as_deref())
    }

    /// A chain for `track`'s MIDI effects, made off the sound loop
    /// ([`Chain::new`] allocates). It runs once [`TrackArps::set_notes`]
    /// gives it its effects. One the track has already is kept, and the
    /// new one handed back to be freed where it was made.
    pub fn give_chain(&mut self, track: usize, mut chain: Box<Chain>) -> Option<Box<Chain>> {
        match self.chains.get_mut(track) {
            Some(slot @ None) => {
                chain.reseed(RESEED);
                *slot = Some(chain);
                self.ask(track);
                None
            }
            _ => Some(chain),
        }
    }

    /// A mute landing on `track` `offset` frames into the coming period:
    /// nothing waiting in its MIDI effects plays from there, and what they
    /// have on ends there ([`Chain::cut`]). Before [`TrackArps::finish`].
    pub fn cut(&mut self, track: usize, offset: u32) {
        if self.has_chain(track) {
            let c = &mut self.cuts[track];
            *c = Some(c.map_or(offset, |o| o.min(offset)));
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
        for c in self.chains.iter_mut().flatten() {
            c.reseed(RESEED);
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
                // One panic is enough however often it changes before the
                // period.
                self.panic(track);
            }
        }
    }

    /// A panic into track `track`'s arp at the period's start, unless it
    /// holds nothing or has one there already: its clips' notes end, and
    /// it lets go of what they pressed.
    fn panic(&mut self, track: usize) {
        let a = &mut self.arps[track];
        if !a.engine.active().is_empty() && a.first_all_off() != Some(0) {
            a.input(0, Event::AllOff);
            self.pressed[track] = [Pressed::default(); LAYOUT_PADS];
            self.fed[track] = true;
        }
    }

    /// Whether track `track`'s arp works in notes (true: its instrument
    /// plays notes, its own synth's or the shared synth's, the instrument
    /// redesign's step 5d) or in pad hits (a kit, the Drums engine, MIDI
    /// out). In notes, the live pads press their notes into it (the app
    /// gives it them), a clip of notes goes in as it is, a clip of pad hits
    /// goes in as the notes [`TrackArps::set_pads`] gives its pads (each
    /// hit's end and pressure reaching what it pressed), and what the arp
    /// plays goes out as notes ([`Out::notes`]). A change ends what the arp
    /// holds, as the "Clips through the arp" switch does; those endings
    /// go out as what they end.
    pub fn set_on_notes(&mut self, track: usize, on: bool) {
        let Some(&was) = self.on_notes.get(track) else {
            return;
        };
        if was != on {
            self.ask(track);
            self.on_notes[track] = on;
            if self.arps[track].engine.active().is_empty() {
                // Nothing to end: its next notes go out as the new kind.
                self.notes_out[track] = on;
            }
            self.panic(track);
        }
    }

    /// Whether track `track`'s arp works in notes.
    pub fn on_notes(&self, track: usize) -> bool {
        self.on_notes.get(track).copied().unwrap_or(false)
    }

    /// The notes track `track`'s pads play, for its clips' pad hits into
    /// an arp that works in notes. A hit already in keeps what it pressed.
    pub fn set_pads(&mut self, track: usize, pads: &PadRows) {
        if let Some(p) = self.pads.get_mut(track) {
            *p = *pads;
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
    /// The pads always play through their track's arp. Whether it went in
    /// (a period too busy leaves a new note out; an `AllOff` lets go of
    /// every note the pads hold there).
    pub fn pad(&mut self, track: usize, offset: u32, event: Event) -> bool {
        self.ask(track);
        match self.arps.get_mut(track) {
            Some(a) => {
                self.fed[track] = true;
                a.input_from(PADS, offset, event)
            }
            None => false,
        }
    }

    /// Notes that gave way so far: new notes left out of periods too busy
    /// for them (here and in each arp), endings lost (a bug), and what the
    /// arps' engines and the MIDI effects dropped. It should stay 0.
    pub fn dropped(&self) -> u64 {
        self.table.refused()
            + self.table.lost()
            + self.arps.iter().map(ArpHost::dropped).sum::<u64>()
            + self
                .chains
                .iter()
                .flatten()
                .map(|c| c.dropped())
                .sum::<u64>()
            + self.lists.iter().map(|l| l.dropped).sum::<u64>()
    }

    /// New notes and values left out of periods too busy for them, so far:
    /// on the way to the sink and into each arp.
    pub fn refused(&self) -> u64 {
        self.table.refused() + self.in_use().map(ArpHost::refused).sum::<u64>()
    }

    /// Endings (note-offs, unlocks, panics) that found no place, so far: a
    /// bug, which must stay 0.
    pub fn lost(&self) -> u64 {
        self.table.lost() + self.in_use().map(ArpHost::lost).sum::<u64>()
    }

    /// Note-offs left out because their note-on was (a period too busy),
    /// so far. Nothing hangs from them.
    pub fn unowed(&self) -> u64 {
        self.table.room.unowed + self.in_use().map(ArpHost::unowed).sum::<u64>()
    }

    /// The arps a period visits: only they are ever given a note, so
    /// their counts are all the arps' (and cost what is in use to add up,
    /// each period).
    fn in_use(&self) -> impl Iterator<Item = &ArpHost> {
        self.arps[..self.used].iter()
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
        let player_notes = self.table.len();
        for t in 0..self.used {
            if self.chains[t].is_some() {
                self.run_chained(t, block);
                continue;
            }
            if !std::mem::take(&mut self.fed[t]) && self.arps[t].engine.idle() {
                self.arps[t].engine.skip(block.frames);
                continue;
            }
            let (track, dest, notes) = (t as u16, self.dest[t], self.notes_out[t]);
            let start = block.start_frame;
            let table = &mut self.table;
            self.arps[t].process(block, &mut |offset, kind| {
                let event = match kind {
                    EventKind::On { note, velocity } => Event::NoteOn { note, velocity },
                    EventKind::Off { note } => Event::NoteOff { note },
                };
                table.put(
                    Out {
                        offset,
                        frame: start + u64::from(offset),
                        track,
                        dest,
                        event,
                        notes,
                    },
                    ARPS,
                );
            });
        }
        // Each source is in order already; a stable insertion sort merges
        // them without allocating (a period holds few notes).
        self.table.merge_from(player_notes);
        for i in 0..self.table.len() {
            if let Some((o, a)) = self.table.get(i) {
                f(&o, a);
            }
        }
        self.table.clear();
        // From the next period an arp's notes go out as what it works in.
        let used = self.used;
        self.notes_out[..used].copy_from_slice(&self.on_notes[..used]);
    }
}

impl TrackArps {
    /// One period of a track with MIDI effects: its notes through the
    /// effects above its arp, the arp, then the effects below it. Runs
    /// every period, so notes its effects hold back (an echo, a strum)
    /// leave on time.
    fn run_chained(&mut self, t: usize, block: &Block) {
        let start = block.start_frame;
        let p = Period {
            start,
            frames: block.frames,
            rate: self.rate,
            bpm: block.spans().first().map_or(120.0, |s| s.bpm),
        };
        let (track, dest) = (t as u16, self.dest[t]);
        let at = usize::from(self.arp_at[t]).min(MAX_FX);
        let (arp, chain) = (&mut self.arps[t], self.chains[t].as_deref_mut());
        let Some(chain) = chain else {
            return;
        };
        let [notes, played] = &mut *self.lists;
        // A panic (or the switch changing) and a mute cut where they land.
        let cut = [arp.first_all_off(), self.cuts[t].take()]
            .into_iter()
            .flatten()
            .min();
        if let Some(offset) = cut {
            chain.cut(start + u64::from(offset));
        }
        notes.clear();
        arp.take_notes(start, notes);
        played.clear();
        chain.process_range(0, at, &p, notes.as_slice(), played);
        for e in played.as_slice() {
            arp.input_from(EFFECTS, offset_in(e, start), event_of(e.kind));
        }
        notes.clear();
        let fed = std::mem::take(&mut self.fed[t]) || !played.is_empty();
        if !fed && arp.engine.idle() {
            arp.engine.skip(block.frames);
        } else {
            arp.process(block, &mut |offset, kind| {
                let at = start + u64::from(offset);
                notes.push(match kind {
                    EventKind::On { note, velocity } => Ev::on(at, note, velocity),
                    EventKind::Off { note } => Ev::off(at, note),
                });
            });
        }
        played.clear();
        chain.process_range(at, MAX_FX, &p, notes.as_slice(), played);
        for e in played.as_slice() {
            self.table.put(
                Out {
                    offset: offset_in(e, start),
                    frame: e.at,
                    track,
                    dest,
                    event: event_of(e.kind),
                    notes: self.notes_out[t],
                },
                ARPS,
            );
        }
    }
}

impl TrackArps {
    /// Clip pad hit `o` on track `t`, whose arp works in notes: a hit goes
    /// in as its pad's notes when the clip plays through the arp, and its
    /// end and pressure reach what it pressed. True when the arp took it
    /// all; false sends it on to the sink (a hit when the clip doesn't play
    /// through the arp, its end, a pad past the 16, pressure, which the
    /// sound hears too, and everything that isn't a note).
    fn hit_as_notes(&mut self, t: usize, o: Out) -> bool {
        let (arp, pads) = (&mut self.arps[t], &self.pads[t]);
        match o.event {
            Event::NoteOn { note, velocity } if self.through[t] => {
                let Some(p) = self.pressed[t].get_mut(usize::from(note)) else {
                    return false;
                };
                if p.holds == 0 {
                    let notes = pads.notes(usize::from(note));
                    p.notes[..notes.len()].copy_from_slice(notes);
                    p.n = notes.len() as u8;
                }
                p.holds = p.holds.saturating_add(1);
                for &n in &p.notes[..usize::from(p.n)] {
                    arp.input_from(CLIPS, o.offset, Event::NoteOn { note: n, velocity });
                }
                self.fed[t] = true;
                true
            }
            Event::NoteOff { note } => {
                let Some(p) = self.pressed[t].get_mut(usize::from(note)) else {
                    return false;
                };
                if p.holds == 0 {
                    // Its start went to the sink.
                    return false;
                }
                p.holds -= 1;
                for &n in &p.notes[..usize::from(p.n)] {
                    if arp.owes(CLIPS, n) {
                        arp.input_from(CLIPS, o.offset, Event::NoteOff { note: n });
                    }
                }
                self.fed[t] = true;
                true
            }
            Event::Pressure { note, value } if self.through[t] => {
                if let Some(p) = self.pressed[t].get(usize::from(note)) {
                    if p.holds > 0 {
                        for &n in &p.notes[..usize::from(p.n)] {
                            arp.input_from(CLIPS, o.offset, Event::Pressure { note: n, value });
                        }
                        self.fed[t] = true;
                    }
                }
                false
            }
            _ => false,
        }
    }
}

/// `e`'s offset in the period starting at frame `start`.
fn offset_in(e: &Ev, start: u64) -> u32 {
    e.at.saturating_sub(start).min(u64::from(u32::MAX)) as u32
}

fn event_of(k: Kind) -> Event {
    match k {
        Kind::On { note, velocity } => Event::NoteOn { note, velocity },
        Kind::Off { note } => Event::NoteOff { note },
    }
}

/// The player's [`Sink`] for one period: a through track's notes go into
/// its arp (its pressure too, which also goes on), everything else (other
/// tracks' notes, and every parameter lock and unlock, which an arp has no
/// use for) waits for [`TrackArps::finish`]. A note-off goes where its
/// note-on went.
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
            a.pressed[..used].fill([Pressed::default(); LAYOUT_PADS]);
        } else if t < MAX_TRACKS && a.on_notes[t] && !o.notes {
            // A clip of pad hits into an arp that works in notes: each hit
            // as its pad's notes.
            if a.hit_as_notes(t, o) {
                return;
            }
        } else if t < MAX_TRACKS {
            // A clip of notes' notes are kept apart from a clip of pad
            // hits' in the arp, so the same number of each never meet.
            let src = if o.notes { CLIP_NOTES } else { CLIPS };
            let into_arp = match o.event {
                Event::NoteOn { .. } => a.through[t],
                // Where its note went, whatever the switch says now.
                Event::NoteOff { note } => a.arps[t].owes(src, note),
                Event::Pressure { .. } if a.through[t] => {
                    // The arp's repeats follow a clip's pressure, and the
                    // sound hears it too, as with the live pads.
                    a.arps[t].input_from(src, o.offset, o.event);
                    a.fed[t] = true;
                    false
                }
                _ => false,
            };
            if into_arp {
                a.arps[t].input_from(src, o.offset, o.event);
                a.fed[t] = true;
                return;
            }
        }
        a.table.put(o, PLAYER);
    }

    fn hold(&mut self) -> bool {
        self.arps.table.room.hold()
    }
}
