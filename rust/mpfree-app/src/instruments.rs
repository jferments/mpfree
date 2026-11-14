//! Each track's own instrument: the track host (`mpfree-track`,
//! [track host](../../../docs/dev/design/track-host.md)) in the pad player.
//!
//! - **Any track.** Every sequencer track ([`TRACKS`]) can play its own
//!   instrument, up to [`SLOTS`] tracks at once (each instrument costs its
//!   share of the CPU). A track with none, as every track starts, plays the
//!   pads as before: the playing kit, or the synth while it has the pads.
//! - **Slots, lent.** The [`TrackHost`] has [`HOST_SLOTS`] slots, twice
//!   [`SLOTS`]. A track that picks an instrument is lent a free slot, routed
//!   to its mixer channel; it keeps the slot while it has an instrument, and
//!   a new instrument goes into the same slot (the old one rings out there).
//!   An instrument taken out rings out in its slot, still into its track's
//!   channel, and the slot is free again once it is quiet. With twice as
//!   many slots as instruments, a ring-out is cut short only when more than
//!   [`SLOTS`] ring out at once (counted).
//! - **Mixer channels.** Track `t`'s instrument feeds mixer track
//!   [`first_track`]` + t`, made with the mixer after the engines' tracks
//!   ([`crate::mixing::TRACKS`]), so the channel's fader, sends and effects
//!   stay with the track, whichever slot it was lent. The mixer panel shows
//!   only the ones in use ([`Instruments::in_use`]).
//! - **Notes.** A clip's notes and the pads are pads (0 to 15). A slot turns
//!   a pad into notes with its own pad map, set when the instrument went in
//!   (the Synth tab's layout, or Drums' kit notes), and remembers the notes
//!   each pad started, so the pad's note-off ends those even after the map
//!   changed. The arp's notes, once the app has given it a layout, are MIDI
//!   notes already and go in as they are. The host counts each note, so a
//!   clip and a held pad on the same note never cut each other short.
//! - **Putting one in** allocates nowhere here: the builder thread
//!   (`control.rs`) makes the [`Instrument`] (a copy of a synth engine's
//!   sound, or a plugin synth in its own process) and the audio loop puts it
//!   in with [`Instruments::put`]. The old one rings out into the same
//!   track and comes back through [`Instruments::pop_retired`], to be
//!   dropped on the builder thread.
//!
//! Everything after [`Instruments::new`] never allocates, frees or locks.

use mpfree_arp::scale::pads::{Pad, MAX_PAD_NOTES, PADS};
use mpfree_node::{Ctx, NodeEvent, Timed, Tuning};
use mpfree_track::{Instrument, Outputs, SlotStats, TrackHost};

/// Makes the instrument `id` for a track, ready for the audio thread:
/// a plugin synth (`clap:`...) in its own process, from `plugins`, or a
/// copy of a synth engine with `values` (its sound, as
/// [`crate::synth::Synth::sound_of`] gives it). None if it can't be made
/// (the plugins' lines say why for a plugin). Allocates and, for a
/// plugin, waits for its process: the builder thread's job.
pub fn make(
    plugins: &crate::plugins::Plugins,
    id: &'static str,
    values: &[f32],
    rate: u32,
    max_block: usize,
) -> Option<(Instrument, Held)> {
    let (node, name, instance) = if crate::plugins::Plugins::is_plugin(id) {
        let name = plugins.find(id)?.name;
        let (instance, node) = plugins.make_instance(id).ok()?;
        (node, name, instance)
    } else {
        let node = crate::synth::fresh_node(id, values, rate)?;
        let name = node.info().name;
        (node, name, 0)
    };
    Some((
        Instrument::new(node, rate, max_block),
        Held { id, name, instance },
    ))
}

/// The tracks that can play their own instrument: every sequencer track.
pub const TRACKS: usize = mpfree_seq::MAX_TRACKS;

/// The most tracks that play their own instrument at once; the others
/// play the kit.
pub const SLOTS: usize = 16;

/// The track host's slots: an instrument for each of [`SLOTS`] tracks, and
/// as many again for old ones ringing out.
pub const HOST_SLOTS: usize = 2 * SLOTS;

/// No slot (in [`Instruments::slot_of`]).
const NO_SLOT: u8 = u8::MAX;

const _: () = assert!(HOST_SLOTS < NO_SLOT as usize);
// `in_use` has a bit for each track.
const _: () = assert!(TRACKS <= u128::BITS as usize);
// Each track has its own mixer track after the shared ones.
const _: () = assert!(crate::mixing::TRACKS + TRACKS <= mpfree_fx::MAX_TRACKS);

/// The mixer track track 0's instrument feeds; track `t`'s feeds this plus
/// `t`.
pub const fn first_track() -> usize {
    crate::mixing::TRACKS
}

/// What a track's slot holds, for the screens and the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    /// The node's id: an engine's (`mpfree.poly`) or a plugin's
    /// (`clap:<plugin id>`).
    pub id: &'static str,
    /// Its name for the screen ("Poly", a plugin's name).
    pub name: &'static str,
    /// A plugin's instance in the rack (`Rack::make_instance`), which its
    /// presets load into; 0 for an engine (and in the app's own copy).
    pub instance: u64,
}

/// Why [`Instruments::put`] handed an instrument back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No such track.
    NoTrack,
    /// [`SLOTS`] other tracks already play their own instrument.
    Full,
    /// The old instrument can't be retired yet (the instruments waiting to
    /// be dropped fill their list): try again in a moment.
    Busy,
}

/// The notes one pad started on a slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Started {
    notes: [u8; MAX_PAD_NOTES],
    len: u8,
}

impl Started {
    fn of(pad: &Pad) -> Started {
        let mut s = Started::default();
        for (i, &n) in pad.notes().iter().take(MAX_PAD_NOTES).enumerate() {
            s.notes[i] = n;
            s.len = i as u8 + 1;
        }
        s
    }

    fn notes(&self) -> &[u8] {
        &self.notes[..usize::from(self.len)]
    }
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    /// The track it was last lent to: the one its instrument plays for, or
    /// its old one rings out for. It feeds that track's mixer channel.
    track: Option<usize>,
    held: Option<Held>,
    /// Pad `p` plays these notes.
    pads: [Pad; PADS],
    /// The notes each pad started and hasn't ended yet.
    started: [Started; PADS],
    /// The slot's counts when it was lent to its track.
    base: SlotStats,
}

impl Default for Slot {
    fn default() -> Slot {
        Slot {
            track: None,
            held: None,
            pads: std::array::from_fn(|p| {
                Pad::single(mpfree_arp::scale::pads::BASE_NOTE + p as i16)
            }),
            started: [Started::default(); PADS],
            base: SlotStats::default(),
        }
    }
}

/// Every track's own instrument. See the module docs.
pub struct Instruments {
    host: TrackHost,
    slots: [Slot; HOST_SLOTS],
    /// The slot track `t`'s instrument is in ([`NO_SLOT`]: it has none).
    slot_of: [u8; TRACKS],
    /// Instruments put in that would not go (no room to retire the old
    /// one, or [`SLOTS`] tracks already had one): handed back, counted.
    refused: u64,
    /// Ring-outs cut short to lend their slot (more than [`SLOTS`] at once),
    /// or by a new instrument while an older one still rang out there.
    cut: u64,
}

impl Instruments {
    /// No track has an instrument, for blocks of at most `max_block`
    /// frames. Allocates.
    pub fn new(rate: u32, max_block: usize) -> Instruments {
        let mut host = TrackHost::new(HOST_SLOTS, rate, max_block);
        for s in 0..HOST_SLOTS {
            host.route(s, None);
        }
        Instruments {
            host,
            slots: [Slot::default(); HOST_SLOTS],
            slot_of: [NO_SLOT; TRACKS],
            refused: 0,
            cut: 0,
        }
    }

    /// The slot track `track`'s instrument is in.
    fn slot(&self, track: usize) -> Option<usize> {
        match self.slot_of.get(track) {
            Some(&s) if s != NO_SLOT => Some(usize::from(s)),
            _ => None,
        }
    }

    /// What track `track` plays (None: the pads).
    pub fn held(&self, track: usize) -> Option<Held> {
        self.slots[self.slot(track)?].held
    }

    /// Whether track `track` plays its own instrument.
    pub fn holds(&self, track: usize) -> bool {
        self.slot(track).is_some()
    }

    /// How many tracks play their own instrument.
    pub fn count(&self) -> usize {
        self.slot_of.iter().filter(|&&s| s != NO_SLOT).count()
    }

    /// Whether an instrument for track `track` would go in: it has one
    /// already (the new one takes its place), or fewer than [`SLOTS`]
    /// tracks have one.
    pub fn has_room(&self, track: usize) -> bool {
        track < TRACKS && (self.holds(track) || self.count() < SLOTS)
    }

    /// The tracks whose mixer channels are in use, one bit each: playing
    /// their own instrument or still ringing out an old one.
    pub fn in_use(&self) -> u128 {
        (0..HOST_SLOTS)
            .filter(|&s| self.slots[s].held.is_some() || self.host.ringing(s))
            .filter_map(|s| self.slots[s].track)
            .fold(0, |m, t| m | 1 << t)
    }

    /// A slot to lend track `track`: an idle one (no instrument, nothing
    /// ringing out), else one ringing out for this same track, else any
    /// one ringing out (cut short). There is always one while fewer than
    /// [`SLOTS`] tracks hold an instrument.
    fn free_slot(&self, track: usize) -> Option<usize> {
        let empty = || (0..HOST_SLOTS).filter(|&s| self.slots[s].held.is_none());
        empty()
            .find(|&s| !self.host.ringing(s))
            .or_else(|| empty().find(|&s| self.slots[s].track == Some(track)))
            .or_else(|| empty().next())
    }

    /// Puts `inst` in for track `track`, with `pads` as its pad map and
    /// `tuning`: in the track's slot if it has one (the old instrument
    /// rings out), else in a slot lent to it. Says why if it can't go in,
    /// and hands `inst` back. Never allocates.
    pub fn put(
        &mut self,
        track: usize,
        inst: (Instrument, Held),
        pads: [Pad; PADS],
        tuning: &Tuning,
    ) -> Result<(), (Refused, Option<Instrument>)> {
        let (inst, held) = inst;
        if track >= TRACKS {
            return Err((Refused::NoTrack, Some(inst)));
        }
        let lend = self.slot(track).is_none();
        let s = match self.slot(track) {
            Some(s) => s,
            None if self.count() >= SLOTS => {
                self.refused += 1;
                return Err((Refused::Full, Some(inst)));
            }
            None => match self.free_slot(track) {
                Some(s) => s,
                None => {
                    self.refused += 1;
                    return Err((Refused::Full, Some(inst)));
                }
            },
        };
        let cuts = self.host.ringing(s);
        if let Err((_, back)) = self.host.swap(s, Some(inst)) {
            self.refused += 1;
            return Err((Refused::Busy, back));
        }
        self.cut += u64::from(cuts);
        self.host.set_tuning(s, tuning);
        if lend {
            self.host.route(s, Some(first_track() + track));
            self.slots[s].track = Some(track);
            self.slots[s].base = self.host.stats(s);
            self.slot_of[track] = s as u8;
        }
        let slot = &mut self.slots[s];
        slot.held = Some(held);
        slot.pads = pads;
        slot.started = [Started::default(); PADS];
        Ok(())
    }

    /// Takes track `track`'s instrument out, so it plays the pads again.
    /// The instrument rings out into the track's channel, and its slot is
    /// free once it is quiet. False if it can't be retired yet (an older
    /// one still rings out there and the list to drop is full): the track
    /// keeps it. Never allocates.
    pub fn take_out(&mut self, track: usize) -> bool {
        let Some(s) = self.slot(track) else {
            return true;
        };
        let cuts = self.host.ringing(s);
        if self.host.swap(s, None).is_err() {
            self.refused += 1;
            return false;
        }
        self.cut += u64::from(cuts);
        let slot = &mut self.slots[s];
        slot.held = None;
        slot.started = [Started::default(); PADS];
        self.slot_of[track] = NO_SLOT;
        true
    }

    /// A new pad map for track `track` (its notes so far end as they
    /// started).
    pub fn set_pads(&mut self, track: usize, pads: [Pad; PADS]) {
        if let Some(s) = self.slot(track) {
            self.slots[s].pads = pads;
        }
    }

    /// Pad `pad` down on track `track`, `offset` frames into the next
    /// block. False if the track holds no instrument (it plays the pads).
    pub fn pad_on(&mut self, track: usize, pad: usize, velocity: u8, offset: u32) -> bool {
        let Some(s) = self.slot(track) else {
            return false;
        };
        let slot = &mut self.slots[s];
        let Some(p) = slot.pads.get(pad) else {
            return true;
        };
        let started = Started::of(p);
        slot.started[pad] = started;
        for &n in started.notes() {
            self.host.note_on(s, offset, n, velocity.max(1));
        }
        true
    }

    /// Pad `pad` up on track `track`: ends the notes it started.
    pub fn pad_off(&mut self, track: usize, pad: usize, offset: u32) -> bool {
        let Some(s) = self.slot(track) else {
            return false;
        };
        let Some(st) = self.slots[s].started.get_mut(pad).map(std::mem::take) else {
            return true;
        };
        for &n in st.notes() {
            self.host.note_off(s, offset, n);
        }
        true
    }

    /// A clip's or the pad's pressure on pad `pad`'s notes (0 to 127).
    pub fn pad_pressure(&mut self, track: usize, pad: usize, value: u8, offset: u32) -> bool {
        let Some(s) = self.slot(track) else {
            return false;
        };
        let Some(st) = self.slots[s].started.get(pad).copied() else {
            return true;
        };
        for &note in st.notes() {
            self.host
                .event(s, Timed::new(offset, NodeEvent::Pressure { note, value }));
        }
        true
    }

    /// A MIDI note on track `track` (the arp's, with a layout).
    pub fn note_on(&mut self, track: usize, note: u8, velocity: u8, offset: u32) -> bool {
        let Some(s) = self.slot(track) else {
            return false;
        };
        self.host.note_on(s, offset, note, velocity.max(1));
        true
    }

    pub fn note_off(&mut self, track: usize, note: u8, offset: u32) -> bool {
        let Some(s) = self.slot(track) else {
            return false;
        };
        self.host.note_off(s, offset, note);
        true
    }

    /// A parameter of track `track`'s instrument (the Synth tab's knobs),
    /// from the next block's first frame.
    pub fn param(&mut self, track: usize, id: u16, value: f32) -> bool {
        self.slot(track)
            .is_some_and(|s| self.host.param(s, 0, id, value))
    }

    /// Track `track`'s instrument's node, while it holds one.
    pub fn node(&self, track: usize) -> Option<&dyn mpfree_node::Node> {
        self.host.instrument(self.slot(track)?).map(|i| i.node())
    }

    /// Every note on track `track` ends (a panic, a track muted).
    pub fn all_off(&mut self, track: usize) {
        if let Some(s) = self.slot(track) {
            self.slots[s].started = [Started::default(); PADS];
            self.host.event(s, Timed::new(0, NodeEvent::AllOff));
        }
    }

    /// Every note on every track ends with its release (the transport
    /// stopped, a panic to every track).
    pub fn all_off_every(&mut self) {
        for s in &mut self.slots {
            s.started = [Started::default(); PADS];
        }
        self.host.all_off();
    }

    /// STOP: every slot's notes end and fade out (`mpfree-track`).
    pub fn stop(&mut self) {
        for s in &mut self.slots {
            s.started = [Started::default(); PADS];
        }
        self.host.stop();
    }

    /// The tuning for track `track`'s instrument, from the next block (the
    /// instrument put in next takes the tuning given to
    /// [`Instruments::put`]).
    pub fn set_tuning(&mut self, track: usize, tuning: &Tuning) {
        if let Some(s) = self.slot(track) {
            self.host.set_tuning(s, tuning);
        }
    }

    /// Every slot's next `frames` frames into its track's mixer channel.
    pub fn render(&mut self, ctx: &Ctx, frames: usize, out: &mut dyn Outputs) {
        self.host.render(ctx, frames, out);
    }

    /// An instrument taken out, after its ring-out: drop it off the audio
    /// thread.
    pub fn pop_retired(&mut self) -> Option<Instrument> {
        self.host.pop_retired()
    }

    /// Track `track`'s counts since its slot was lent to it: notes, drops
    /// and resets. All 0 while it has no instrument.
    pub fn stats(&self, track: usize) -> SlotStats {
        let Some(s) = self.slot(track) else {
            return SlotStats::default();
        };
        let (now, base) = (self.host.stats(s), self.slots[s].base);
        SlotStats {
            notes: now.notes - base.notes,
            dropped: now.dropped - base.dropped,
            stray_offs: now.stray_offs - base.stray_offs,
            resets: now.resets - base.resets,
            swaps: now.swaps - base.swaps,
        }
    }

    /// The status line's part: each track's instrument with its notes,
    /// drops and resets, then instruments refused and ring-outs cut short.
    /// None while no track has had one.
    pub fn describe(&self) -> Option<String> {
        let mut parts = Vec::new();
        for t in (0..TRACKS).filter(|&t| self.holds(t)) {
            let (Some(h), st) = (self.held(t), self.stats(t)) else {
                continue;
            };
            parts.push(format!(
                "track {} {} ({} notes, {} dropped, {} resets)",
                t + 1,
                h.name,
                st.notes,
                st.dropped,
                st.resets
            ));
        }
        if parts.is_empty() && self.refused == 0 && self.cut == 0 {
            return None;
        }
        if self.refused > 0 {
            parts.push(format!("{} refused", self.refused));
        }
        if self.cut > 0 {
            let s = if self.cut == 1 { "" } else { "s" };
            parts.push(format!("{} ring-out{s} cut short", self.cut));
        }
        Some(parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::{Kind, Node, NodeInfo, ParamInfo};

    const RATE: u32 = 44_100;
    const BLOCK: usize = 64;

    /// A test instrument: a constant level while any note sounds.
    struct Hum {
        sounding: [u8; 128],
    }

    static HUM: NodeInfo = NodeInfo {
        id: "test.hum",
        name: "Hum",
        kind: Kind::Instrument,
    };

    impl Node for Hum {
        fn info(&self) -> &'static NodeInfo {
            &HUM
        }
        fn params(&self) -> &'static [ParamInfo] {
            &[]
        }
        fn param(&self, _: u16) -> f32 {
            0.0
        }
        fn set_param(&mut self, _: u16, _: f32) {}
        fn prepare(&mut self, _: u32, _: usize) {}
        fn reset(&mut self) {
            self.sounding = [0; 128];
        }
        fn process(&mut self, _: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
            for e in events {
                match e.ev {
                    NodeEvent::NoteOn { note, .. } => self.sounding[usize::from(note)] += 1,
                    NodeEvent::NoteOff { note } => self.sounding[usize::from(note)] = 0,
                    NodeEvent::AllOff => self.sounding = [0; 128],
                    _ => {}
                }
            }
            let v = if self.sounding.iter().any(|&n| n > 0) {
                0.5
            } else {
                0.0
            };
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                *l += v;
                *r += v;
            }
        }
    }

    fn hum() -> (Instrument, Held) {
        let node = Box::new(Hum { sounding: [0; 128] });
        (
            Instrument::new(node, RATE, BLOCK),
            Held {
                id: "test.hum",
                name: "Hum",
                instance: 0,
            },
        )
    }

    struct Tracks {
        bufs: Vec<(Vec<f32>, Vec<f32>)>,
    }

    impl Outputs for Tracks {
        fn track(&mut self, track: usize) -> Option<(&mut [f32], &mut [f32])> {
            self.bufs
                .get_mut(track)
                .map(|(l, r)| (l.as_mut_slice(), r.as_mut_slice()))
        }
    }

    fn tracks() -> Tracks {
        Tracks {
            bufs: vec![(vec![0.0; BLOCK], vec![0.0; BLOCK]); first_track() + TRACKS],
        }
    }

    fn level(t: &Tracks, track: usize) -> f32 {
        t.bufs[track].0[BLOCK - 1]
    }

    #[test]
    fn an_empty_track_plays_the_pads_and_a_track_with_an_instrument_plays_it_into_its_own_channel()
    {
        let mut i = Instruments::new(RATE, BLOCK);
        assert!(
            !i.pad_on(2, 0, 100, 0),
            "an empty slot leaves the pad to the kit"
        );
        assert_eq!(i.in_use(), 0);
        let pads = std::array::from_fn(|p| Pad::single(60 + p as i16));
        i.put(2, hum(), pads, &Tuning::standard()).unwrap();
        assert_eq!(i.held(2).map(|h| h.name), Some("Hum"));
        assert_eq!(i.in_use(), 1 << 2);
        assert!(i.pad_on(2, 0, 100, 0));
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        assert_eq!(level(&t, first_track() + 2), 0.5);
        for other in (0..first_track() + TRACKS).filter(|&k| k != first_track() + 2) {
            assert_eq!(level(&t, other), 0.0, "track {other}");
        }
        // The pad's note-off ends it.
        assert!(i.pad_off(2, 0, 0));
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        assert_eq!(level(&t, first_track() + 2), 0.0);
    }

    #[test]
    fn a_pad_ends_the_notes_it_started_after_the_map_changed() {
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(
            0,
            hum(),
            std::array::from_fn(|p| Pad::single(60 + p as i16)),
            &Tuning::standard(),
        )
        .unwrap();
        i.pad_on(0, 3, 90, 0);
        // A new layout before the pad is let go.
        i.set_pads(0, std::array::from_fn(|p| Pad::single(40 + p as i16)));
        i.pad_off(0, 3, 10);
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        assert_eq!(level(&t, first_track()), 0.0, "note 63 ended, not 43");
        assert_eq!(i.stats(0).stray_offs, 0);
    }

    #[test]
    fn a_clip_and_a_held_pad_on_one_note_end_with_the_last() {
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(
            1,
            hum(),
            std::array::from_fn(|p| Pad::single(60 + p as i16)),
            &Tuning::standard(),
        )
        .unwrap();
        i.pad_on(1, 0, 90, 0); // the clip
        i.pad_on(1, 0, 90, 4); // the pad
        i.pad_off(1, 0, 8); // the clip lets go
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        assert_eq!(
            level(&t, first_track() + 1),
            0.5,
            "the held pad still sounds"
        );
    }

    #[test]
    fn a_track_takes_a_copy_of_the_synth_tabs_sound_and_plays_it_as_the_synth_does() {
        let synth = crate::synth::Synth::new(RATE);
        let (id, values) = synth.sound_of(crate::synth::Engine::Poly);
        assert_eq!(id, "mpfree.poly");
        let plugins = crate::plugins::Plugins::none();
        let (inst, held) = make(&plugins, id, &values, RATE, BLOCK).expect("made");
        assert_eq!(held.name, "Poly");
        assert!(
            make(&plugins, id, &values[1..], RATE, BLOCK).is_none(),
            "not its values"
        );
        assert!(
            make(&plugins, "clap:none", &[], RATE, BLOCK).is_none(),
            "no such plugin"
        );
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(
            0,
            (inst, held),
            synth.pads_for(crate::synth::Engine::Poly),
            &Tuning::standard(),
        )
        .unwrap();
        i.pad_on(0, 0, 100, 0);
        let mut heard = 0.0f32;
        for b in 0..20 {
            let mut t = tracks();
            i.render(&Ctx::new(RATE, b * BLOCK as u64), BLOCK, &mut t);
            heard = heard.max(
                t.bufs[first_track()]
                    .0
                    .iter()
                    .fold(0.0, |m, x| m.max(x.abs())),
            );
        }
        assert!(heard > 0.01, "{heard}");
    }

    #[test]
    fn emptying_a_slot_rings_out_then_hands_the_instrument_back() {
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(
            4,
            hum(),
            std::array::from_fn(|p| Pad::single(60 + p as i16)),
            &Tuning::standard(),
        )
        .unwrap();
        i.pad_on(4, 0, 90, 0);
        assert!(i.take_out(4));
        assert!(!i.holds(4), "the track plays the pads again");
        assert_eq!(i.in_use(), 1 << 4, "its channel shows while it rings out");
        assert!(!i.pad_on(4, 0, 90, 0));
        let mut back = None;
        for b in 0..200 {
            let mut t = tracks();
            i.render(&Ctx::new(RATE, b * BLOCK as u64), BLOCK, &mut t);
            if let Some(x) = i.pop_retired() {
                back = Some(x);
                break;
            }
        }
        assert_eq!(back.expect("retired").node().info().id, "test.hum");
        assert_eq!(i.in_use(), 0);
    }

    fn chromatic() -> [Pad; PADS] {
        std::array::from_fn(|p| Pad::single(60 + p as i16))
    }

    fn put(i: &mut Instruments, track: usize) -> Result<(), Refused> {
        i.put(track, hum(), chromatic(), &Tuning::standard())
            .map_err(|(why, _)| why)
    }

    #[test]
    fn the_last_track_is_lent_a_slot_and_plays_into_its_own_channel() {
        let mut i = Instruments::new(RATE, BLOCK);
        let last = TRACKS - 1;
        put(&mut i, last).unwrap();
        assert_eq!(i.in_use(), 1 << last);
        assert!(i.pad_on(last, 0, 100, 0));
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        assert_eq!(level(&t, first_track() + last), 0.5);
        for other in (0..first_track() + TRACKS).filter(|&k| k != first_track() + last) {
            assert_eq!(level(&t, other), 0.0, "track {other}");
        }
        assert_eq!(i.stats(last).notes, 1);
        assert_eq!(put(&mut i, TRACKS), Err(Refused::NoTrack));
    }

    #[test]
    fn sixteen_tracks_at_once_have_their_own_and_a_track_set_to_the_kit_frees_a_place() {
        let mut i = Instruments::new(RATE, BLOCK);
        // Every other track from the top, so the slots don't follow the
        // track numbers.
        let tracks_with: Vec<usize> = (0..TRACKS).rev().step_by(TRACKS / SLOTS).collect();
        assert_eq!(tracks_with.len(), SLOTS);
        for &t in &tracks_with {
            put(&mut i, t).unwrap();
        }
        assert_eq!(i.count(), SLOTS);
        let other = (0..TRACKS).find(|t| !tracks_with.contains(t)).unwrap_or(0);
        if !tracks_with.contains(&other) {
            assert!(!i.has_room(other));
            assert_eq!(put(&mut i, other), Err(Refused::Full));
            assert!(!i.holds(other), "it plays the kit");
        }
        // A track that has one can still change it.
        let first = tracks_with[0];
        assert!(i.has_room(first));
        put(&mut i, first).unwrap();
        // One set to the kit rings out into its own channel and frees a
        // place at once.
        let freed = tracks_with[1];
        assert!(i.pad_on(freed, 0, 100, 0));
        assert!(i.take_out(freed));
        assert!(!i.holds(freed));
        assert_eq!(i.count(), SLOTS - 1);
        let next = if tracks_with.contains(&other) {
            freed
        } else {
            other
        };
        assert!(i.has_room(next));
        put(&mut i, next).unwrap();
        assert!(i.pad_on(next, 0, 100, 0));
        assert_ne!(
            i.in_use() & 1 << freed,
            0,
            "its channel shows while it rings out"
        );
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        assert_eq!(level(&t, first_track() + next), 0.5);
        assert_eq!(
            i.describe().map(|d| d.contains("refused")),
            Some(!tracks_with.contains(&other))
        );
    }

    #[test]
    fn a_slot_lent_again_counts_its_new_tracks_notes_only() {
        let mut i = Instruments::new(RATE, BLOCK);
        put(&mut i, 0).unwrap();
        i.pad_on(0, 0, 100, 0);
        i.pad_off(0, 0, 1);
        assert_eq!(i.stats(0).notes, 1);
        assert!(i.take_out(0));
        assert_eq!(i.stats(0), SlotStats::default());
        // Rung out and retired, so its slot is idle and lent again.
        for b in 0..200 {
            let mut t = tracks();
            i.render(&Ctx::new(RATE, b * BLOCK as u64), BLOCK, &mut t);
            if i.pop_retired().is_some() {
                break;
            }
        }
        assert_eq!(i.in_use(), 0);
        put(&mut i, 1).unwrap();
        assert_eq!(i.stats(1).notes, 0);
        i.pad_on(1, 0, 100, 0);
        assert_eq!(i.stats(1).notes, 1);
    }

    #[test]
    fn a_ring_out_is_cut_short_only_with_every_slot_busy_and_says_so() {
        let mut i = Instruments::new(RATE, BLOCK);
        let first: Vec<usize> = (0..SLOTS).map(|k| k * TRACKS / SLOTS).collect();
        // SLOTS instruments, all taken out (ringing out), then SLOTS more:
        // every host slot busy, nothing cut.
        for &t in &first {
            put(&mut i, t).unwrap();
        }
        for &t in &first {
            assert!(i.take_out(t));
        }
        for &t in &first {
            put(&mut i, t).unwrap();
        }
        assert_eq!(i.describe().map(|d| d.contains("cut short")), Some(false));
        // One more taken out and put back: no idle slot, so a ring-out is
        // cut, and the status line says so.
        assert!(i.take_out(first[0]));
        put(&mut i, first[0]).unwrap();
        let d = i.describe().unwrap();
        assert!(d.ends_with("1 ring-out cut short"), "{d}");
    }
}
