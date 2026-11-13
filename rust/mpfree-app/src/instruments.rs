//! Each track's own instrument: the track host (`mpfree-track`,
//! [track host](../../../docs/dev/design/track-host.md)) in the pad player.
//!
//! - **Slots.** Track `t` plays into slot `t` of a [`TrackHost`] of
//!   [`SLOTS`] slots (its destination `t`, which the project keeps). An
//!   empty slot, as every slot starts, means the track plays the pads as
//!   before: the playing kit, or the synth while it has the pads.
//! - **Mixer channels.** Slot `t` feeds mixer track [`first_track`]` + t`,
//!   made with the mixer after the engines' tracks
//!   ([`crate::mixing::TRACKS`]). The mixer panel shows only the ones in use
//!   ([`Instruments::in_use`]).
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
    let (node, name) = if crate::plugins::Plugins::is_plugin(id) {
        let name = plugins.find(id)?.name;
        (plugins.make(id).ok()?, name)
    } else {
        let node = crate::synth::fresh_node(id, values, rate)?;
        let name = node.info().name;
        (node, name)
    };
    Some((Instrument::new(node, rate, max_block), Held { id, name }))
}

/// Slots: one per sequencer track.
pub const SLOTS: usize = mpfree_seq::MAX_TRACKS;

/// The mixer track slot 0 feeds; slot `t` feeds this plus `t`.
pub const fn first_track() -> usize {
    crate::mixing::TRACKS
}

/// What a slot holds, for the screens and the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    /// The node's id: an engine's (`mpfree.poly`) or a plugin's
    /// (`clap:<plugin id>`).
    pub id: &'static str,
    /// Its name for the screen ("Poly", a plugin's name).
    pub name: &'static str,
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
    held: Option<Held>,
    /// Pad `p` plays these notes.
    pads: [Pad; PADS],
    /// The notes each pad started and hasn't ended yet.
    started: [Started; PADS],
}

impl Default for Slot {
    fn default() -> Slot {
        Slot {
            held: None,
            pads: std::array::from_fn(|p| {
                Pad::single(mpfree_arp::scale::pads::BASE_NOTE + p as i16)
            }),
            started: [Started::default(); PADS],
        }
    }
}

/// Every track's own instrument. See the module docs.
pub struct Instruments {
    host: TrackHost,
    slots: [Slot; SLOTS],
    /// Instruments put in that would not go (no room to retire the old
    /// one): handed back, counted.
    refused: u64,
}

impl Instruments {
    /// Every slot empty, for blocks of at most `max_block` frames, feeding
    /// mixer tracks from [`first_track`]. Allocates.
    pub fn new(rate: u32, max_block: usize) -> Instruments {
        let mut host = TrackHost::new(SLOTS, rate, max_block);
        for s in 0..SLOTS {
            host.route(s, Some(first_track() + s));
        }
        Instruments {
            host,
            slots: [Slot::default(); SLOTS],
            refused: 0,
        }
    }

    /// What track `track`'s slot holds (None: it plays the pads).
    pub fn held(&self, track: usize) -> Option<Held> {
        self.slots.get(track)?.held
    }

    /// Whether track `track` plays its own instrument.
    pub fn holds(&self, track: usize) -> bool {
        self.held(track).is_some()
    }

    /// The slots in use, one bit each: holding an instrument or still
    /// ringing out an old one (their mixer tracks show).
    pub fn in_use(&self) -> u32 {
        (0..SLOTS)
            .filter(|&s| self.slots[s].held.is_some() || self.host.ringing(s))
            .fold(0, |m, s| m | 1 << s)
    }

    /// Puts `inst` in track `track`'s slot (None empties it, so the track
    /// plays the pads again), with `pads` as its pad map. The old
    /// instrument rings out. Hands `inst` back if there is no such slot
    /// or no room to retire the old one. Never allocates.
    pub fn put(
        &mut self,
        track: usize,
        inst: Option<(Instrument, Held)>,
        pads: [Pad; PADS],
    ) -> Result<(), Option<Instrument>> {
        if track >= SLOTS {
            return Err(inst.map(|(i, _)| i));
        }
        let (inst, held) = match inst {
            Some((i, h)) => (Some(i), Some(h)),
            None => (None, None),
        };
        match self.host.swap(track, inst) {
            Ok(()) => {
                let s = &mut self.slots[track];
                s.held = held;
                s.pads = pads;
                s.started = [Started::default(); PADS];
                Ok(())
            }
            Err((_, back)) => {
                self.refused += 1;
                Err(back)
            }
        }
    }

    /// A new pad map for track `track` (its notes so far end as they
    /// started).
    pub fn set_pads(&mut self, track: usize, pads: [Pad; PADS]) {
        if let Some(s) = self.slots.get_mut(track) {
            s.pads = pads;
        }
    }

    /// Pad `pad` down on track `track`, `offset` frames into the next
    /// block. False if the track holds no instrument (it plays the pads).
    pub fn pad_on(&mut self, track: usize, pad: usize, velocity: u8, offset: u32) -> bool {
        let Some(s) = self.slots.get_mut(track).filter(|s| s.held.is_some()) else {
            return false;
        };
        let Some(p) = s.pads.get(pad) else {
            return true;
        };
        let started = Started::of(p);
        s.started[pad] = started;
        for &n in started.notes() {
            self.host.note_on(track, offset, n, velocity.max(1));
        }
        true
    }

    /// Pad `pad` up on track `track`: ends the notes it started.
    pub fn pad_off(&mut self, track: usize, pad: usize, offset: u32) -> bool {
        let Some(s) = self.slots.get_mut(track).filter(|s| s.held.is_some()) else {
            return false;
        };
        let Some(st) = s.started.get_mut(pad).map(std::mem::take) else {
            return true;
        };
        for &n in st.notes() {
            self.host.note_off(track, offset, n);
        }
        true
    }

    /// A clip's or the pad's pressure on pad `pad`'s notes (0 to 127).
    pub fn pad_pressure(&mut self, track: usize, pad: usize, value: u8, offset: u32) -> bool {
        let Some(s) = self.slots.get(track).filter(|s| s.held.is_some()) else {
            return false;
        };
        let Some(st) = s.started.get(pad).copied() else {
            return true;
        };
        for &note in st.notes() {
            self.host.event(
                track,
                Timed::new(offset, NodeEvent::Pressure { note, value }),
            );
        }
        true
    }

    /// A MIDI note on track `track` (the arp's, with a layout).
    pub fn note_on(&mut self, track: usize, note: u8, velocity: u8, offset: u32) -> bool {
        self.holds(track) && {
            self.host.note_on(track, offset, note, velocity.max(1));
            true
        }
    }

    pub fn note_off(&mut self, track: usize, note: u8, offset: u32) -> bool {
        self.holds(track) && {
            self.host.note_off(track, offset, note);
            true
        }
    }

    /// A parameter of track `track`'s instrument (the Synth tab's knobs),
    /// from the next block's first frame.
    pub fn param(&mut self, track: usize, id: u16, value: f32) -> bool {
        self.holds(track) && self.host.param(track, 0, id, value)
    }

    /// Track `track`'s instrument's node, while it holds one.
    pub fn node(&self, track: usize) -> Option<&dyn mpfree_node::Node> {
        self.holds(track)
            .then(|| self.host.instrument(track))
            .flatten()
            .map(|i| i.node())
    }

    /// Every note on track `track` ends (a panic, a track muted).
    pub fn all_off(&mut self, track: usize) {
        if let Some(s) = self.slots.get_mut(track) {
            s.started = [Started::default(); PADS];
            self.host.event(track, Timed::new(0, NodeEvent::AllOff));
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

    /// Track `track`'s tuning, kept by its slot for the next instrument.
    pub fn set_tuning(&mut self, track: usize, tuning: &Tuning) {
        self.host.set_tuning(track, tuning);
    }

    /// Every slot's next `frames` frames into its mixer track's input.
    pub fn render(&mut self, ctx: &Ctx, frames: usize, out: &mut dyn Outputs) {
        self.host.render(ctx, frames, out);
    }

    /// An instrument taken out, after its ring-out: drop it off the audio
    /// thread.
    pub fn pop_retired(&mut self) -> Option<Instrument> {
        self.host.pop_retired()
    }

    /// Track `track`'s slot counts: notes, drops and resets.
    pub fn stats(&self, track: usize) -> SlotStats {
        self.host.stats(track)
    }

    /// The status line's part: each track's instrument with its notes,
    /// drops and resets, and instruments refused. None while no track has
    /// had one.
    pub fn describe(&self) -> Option<String> {
        let mut parts = Vec::new();
        for t in 0..SLOTS {
            let st = self.stats(t);
            if let Some(h) = self.slots[t].held {
                parts.push(format!(
                    "track {} {} ({} notes, {} dropped, {} resets)",
                    t + 1,
                    h.name,
                    st.notes,
                    st.dropped,
                    st.resets
                ));
            }
        }
        if parts.is_empty() && self.refused == 0 {
            return None;
        }
        if self.refused > 0 {
            parts.push(format!("{} refused", self.refused));
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
            bufs: vec![(vec![0.0; BLOCK], vec![0.0; BLOCK]); first_track() + SLOTS],
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
        i.put(2, Some(hum()), pads).unwrap();
        assert_eq!(i.held(2).map(|h| h.name), Some("Hum"));
        assert_eq!(i.in_use(), 1 << 2);
        assert!(i.pad_on(2, 0, 100, 0));
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        assert_eq!(level(&t, first_track() + 2), 0.5);
        for other in (0..first_track() + SLOTS).filter(|&k| k != first_track() + 2) {
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
            Some(hum()),
            std::array::from_fn(|p| Pad::single(60 + p as i16)),
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
            Some(hum()),
            std::array::from_fn(|p| Pad::single(60 + p as i16)),
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
            Some((inst, held)),
            synth.pads_for(crate::synth::Engine::Poly),
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
            Some(hum()),
            std::array::from_fn(|p| Pad::single(60 + p as i16)),
        )
        .unwrap();
        i.pad_on(4, 0, 90, 0);
        i.put(4, None, std::array::from_fn(|p| Pad::single(60 + p as i16)))
            .unwrap();
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
}
