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
//! - **Mixer channels.** Track `t`'s instrument feeds its track's mixer
//!   channel, [`crate::mixing::track_channel`]: track 1's is the Drums
//!   strip (sampler R3 part 4b, so a track has one strip whatever it
//!   plays), every other track's its own, [`first_track`]` + t`, made with
//!   the mixer after the engines' tracks ([`crate::mixing::TRACKS`]). The
//!   channel's fader, sends and effects stay with the track, whichever
//!   slot it was lent. A project saved with settings on track 1's old own
//!   channel keeps track 1 there ([`Instruments::set_first_channel`]). The
//!   mixer panel shows only the own channels in use.
//! - **Notes.** A clip's notes and the pads are pads (0 to 15). A slot turns
//!   a pad into notes with its own pad map, set when the instrument went in
//!   (the Synth tab's layout, or Drums' kit notes), and remembers the notes
//!   each pad started, so the pad's note-off ends those even after the map
//!   changed. The arp's notes, once the app has given it a layout, are MIDI
//!   notes already and go in as they are. The host counts each note, so a
//!   clip and a held pad on the same note never cut each other short.
//! - **Sends** (instrument redesign step 4): a track with no instrument
//!   of its own can play another track's ([`Instruments::set_send`]), so
//!   several tracks share one. Its notes, pads, pedals, wheel and locks go
//!   to that instrument, whose sound stays in its own track's channel.
//!   Each track's notes are remembered apart, so a note ends where it
//!   started even after the send moved, and a send that moves or ends
//!   first ends the notes the track left sounding there. Taking an
//!   instrument out ends every send to it (its id is gone for good); a
//!   new sound in it keeps them (the id stays). [`Instruments::holds`],
//!   [`Instruments::held`] and [`Instruments::node`] are about the track's
//!   own instrument; [`Instruments::owner`] and [`Instruments::plays`] say
//!   what it plays.
//! - **Putting one in** allocates nowhere here: the builder thread
//!   (`control.rs`) makes the [`Instrument`] (a copy of a synth engine's
//!   sound, or a plugin synth in its own process) and the audio loop puts it
//!   in with [`Instruments::put`]. The old one rings out into the same
//!   track and comes back through [`Instruments::pop_retired`], to be
//!   dropped on the builder thread.
//!
//! Everything after [`Instruments::new`] never allocates, frees or locks.

use mpfree_arp::scale::pads::{Pad, MAX_PAD_NOTES, PADS};
use mpfree_arp::scale::Shift;
use mpfree_node::{Ctx, NodeEvent, Timed, Tuning};
use mpfree_track::{Instrument, LaneCounts, Outputs, SlotStats, TrackHost};

/// Makes the instrument `id` for a track, ready for the audio thread:
/// a plugin synth (`clap:`...) in its own process, from `plugins`, or a
/// copy of a synth engine's sound, its design in `extras` and its
/// `values` (as [`crate::synth::Synth::sound_of`] gives them). None if it
/// can't be made
/// (the plugins' lines say why for a plugin). Allocates and, for a
/// plugin, waits for its process: the builder thread's job.
pub fn make(
    plugins: &crate::plugins::Plugins,
    id: &'static str,
    extras: &[(String, String)],
    values: &[f32],
    rate: u32,
    max_block: usize,
) -> Option<(Instrument, Held)> {
    let (node, name, instance) = if crate::plugins::Plugins::is_plugin(id) {
        let name = plugins.find(id)?.name;
        let (instance, node) = plugins.make_instance(id).ok()?;
        (node, name, instance)
    } else {
        let node = crate::synth::fresh_node(id, extras, values, rate)?;
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

/// The most instruments loaded at once (the redesign's rule 9): the
/// tracks' own and the free ones together; the other tracks play the kit.
pub const SLOTS: usize = 16;

/// Free places: instruments no track has, which the pads play (the
/// instrument redesign's step 6, picked in the list of synths). Each
/// sounds on a mixer strip of its own ([`crate::mixing::free_channel`]).
pub const FREE: usize = 16;

/// Who can hold an instrument: each track (0 to [`TRACKS`] - 1), then
/// each free place ([`free`]).
pub const HOLDERS: usize = TRACKS + FREE;

/// The pads, as a player: they play a held instrument by a send, as a
/// track plays another's ([`Instruments::pads_play`]), so a note they
/// start ends where it started when they move on. They never hold one.
pub const THE_PADS: usize = HOLDERS;

/// The most synths the pads play at once (the instrument redesign's step
/// 11a, layers): what they play ([`THE_PADS`]) and as many again less one
/// by the layer players ([`layer`]).
pub const PAD_LAYERS: usize = 4;

/// The pads' layer `i` beyond the first (from 0, up to [`PAD_LAYERS`] -
/// 2), as a player: like [`THE_PADS`], it plays a held instrument by a
/// send ([`Instruments::set_layer`]), and its notes end where they
/// started.
pub const fn layer(i: usize) -> usize {
    THE_PADS + 1 + i
}

/// The most synths MIDI routes name at once (the instrument redesign's
/// step 11e): a keyboard's route can play a synth no track plays, by a
/// route row ([`routed`]).
pub const ROUTE_SYNTHS: usize = mpfree_midi::setup::ROUTE_SYNTHS;

/// Route row `j` (from 0, below [`ROUTE_SYNTHS`]), as a player: the synth
/// the `j`th synth a MIDI route names (step 11e), played by a send as
/// the pads' layers are ([`Instruments::set_routed`]), so its notes end
/// where they started.
pub const fn routed(j: usize) -> usize {
    THE_PADS + PAD_LAYERS + j
}

/// The note editor's audition (piano roll plan 1C.2), as a player: it
/// plays the instrument the editor's track plays by a send, as the pads
/// do, so the notes it starts end where they started and, the host
/// counting each note's holders, letting one go never cuts the same note
/// a clip holds. It never holds one.
pub const AUDITION: usize = THE_PADS + PAD_LAYERS + ROUTE_SYNTHS;

/// Every row: the holders, then the pads, then their layers, then the
/// route rows, then the audition.
const WHO: usize = AUDITION + 1;

/// Free place `n`'s holder number (from 0).
pub const fn free(n: usize) -> usize {
    TRACKS + n
}

/// The track host's slots: an instrument for each of [`SLOTS`] tracks, and
/// as many again for old ones ringing out.
pub const HOST_SLOTS: usize = 2 * SLOTS;

/// No slot (in [`Instruments::slot_of`]).
const NO_SLOT: u8 = u8::MAX;

/// No send (in [`Instruments::sends`]).
const NO_SEND: u8 = u8::MAX;

const _: () = assert!(HOST_SLOTS < NO_SLOT as usize);
const _: () = assert!(WHO <= NO_SEND as usize);
// `held_free` and `free_in_use` have a bit for each free place.
const _: () = assert!(FREE <= u16::BITS as usize);
// `in_use` has a bit for each track.
const _: () = assert!(TRACKS <= u128::BITS as usize);
// Each track has its own mixer track after the shared ones, then each
// free place.
const _: () = assert!(crate::mixing::TRACKS + HOLDERS <= mpfree_fx::MAX_TRACKS);

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

/// Why [`Instruments::set_send`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendRefused {
    /// No such track.
    NoTrack,
    /// The track plays its own instrument: take it out first.
    HasOwn,
    /// The track it would play has no instrument of its own.
    NothingThere,
    /// A track can't send to itself.
    Itself,
}

/// The notes one pad started on a slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Started {
    notes: [u8; MAX_PAD_NOTES],
    len: u8,
}

impl Started {
    /// The pad's notes, each moved by its track's live transpose (one off
    /// the keyboard is left out).
    fn of(pad: &Pad, shift: &Shift) -> Started {
        let mut s = Started::default();
        for n in pad.notes().iter().filter_map(|&n| shift.apply(n)) {
            if usize::from(s.len) == MAX_PAD_NOTES {
                break;
            }
            s.notes[usize::from(s.len)] = n;
            s.len += 1;
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
            base: SlotStats::default(),
        }
    }
}

/// Every track's own instrument. See the module docs.
pub struct Instruments {
    host: TrackHost,
    slots: [Slot; HOST_SLOTS],
    /// The slot holder `h`'s instrument is in ([`NO_SLOT`]: it has none),
    /// a track's or a free place's; the pads' row is always empty.
    slot_of: [u8; WHO],
    /// Instruments put in that would not go (no room to retire the old
    /// one, or [`SLOTS`] tracks already had one): handed back, counted.
    refused: u64,
    /// Ring-outs cut short to lend their slot (more than [`SLOTS`] at once),
    /// or by a new instrument while an older one still rang out there.
    cut: u64,
    /// The values under the clips' locks on the instruments' parameters
    /// ([`Instruments::lock`]), keyed by track and parameter.
    locks: crate::locks::Under,
    /// Track 1's mixer channel: Drums, or its old own channel for a
    /// project saved with settings there ([`Instruments::set_first_channel`]).
    first: usize,
    /// The holder whose instrument player `p` plays though it has none of
    /// its own ([`NO_SEND`]: none; a track then plays the pads): a track's,
    /// and the pads' ([`THE_PADS`]). A free place never sends.
    sends: [u8; WHO],
    /// The notes each player's pads started and haven't ended yet, by the
    /// player (not the slot), so two tracks on one instrument, or a track
    /// and the pads, each end their own.
    started: Box<[[Started; PADS]; WHO]>,
    /// The notes each player holds by number (a keyboard, the arp with a
    /// layout), one bit each, so a send that moves ends them where they
    /// sound.
    keys: Box<[u128; WHO]>,
}

/// The key of track `track`'s parameter `id` in the lock table.
fn lock_key(track: usize, id: u16) -> u32 {
    (track as u32) << 16 | u32::from(id)
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
            slot_of: [NO_SLOT; WHO],
            refused: 0,
            cut: 0,
            locks: crate::locks::Under::new(rate),
            first: crate::mixing::track_channel(0),
            sends: [NO_SEND; WHO],
            started: Box::new([[Started::default(); PADS]; WHO]),
            keys: Box::new([0; WHO]),
        }
    }

    /// Holder `who`'s mixer channel, where its instrument plays: a
    /// track's channel, or a free place's strip of its own.
    pub fn channel(&self, who: usize) -> usize {
        if who == 0 {
            self.first
        } else if who < TRACKS {
            crate::mixing::track_channel(who)
        } else {
            crate::mixing::free_channel((who - TRACKS).min(FREE - 1))
        }
    }

    /// Track 1's mixer channel from now on (the Drums strip, or its old
    /// own channel: `MixHost::tend_open`); an instrument it has, or one
    /// ringing out for it, moves there. Never allocates.
    pub fn set_first_channel(&mut self, ch: usize) {
        if ch == self.first {
            return;
        }
        self.first = ch;
        for s in 0..HOST_SLOTS {
            if self.slots[s].track == Some(0) {
                self.host.route(s, Some(ch));
            }
        }
    }

    /// The slot track `track`'s instrument is in.
    fn slot(&self, track: usize) -> Option<usize> {
        match self.slot_of.get(track) {
            Some(&s) if s != NO_SLOT => Some(usize::from(s)),
            _ => None,
        }
    }

    /// The mixer strip track `track`'s own instrument sounds on, None if
    /// it has none. The track host keeps where each slot feeds, so this is
    /// the instrument's, not worked out from the track (the instrument
    /// redesign's step 4d): a slot is routed once, when it is lent
    /// ([`Instruments::put`], today the track's channel), and keeps that
    /// strip while its instrument plays, a new sound in it too.
    pub fn strip(&self, track: usize) -> Option<usize> {
        self.host.track_of(self.slot(track)?)
    }

    /// The mixer strip track `track` is heard on while it plays an
    /// instrument: its own instrument's, or that of the track whose
    /// instrument it plays (step 4a). None while it plays the pads (a kit,
    /// the synth). A clip's "this track's strip" goes there
    /// (`time::strips`).
    pub fn heard_on(&self, track: usize) -> Option<usize> {
        self.strip(self.owner(track)?)
    }

    /// The track whose own instrument sounds on mixer strip `strip`, and
    /// that instrument (one ringing out for a track that has none now
    /// doesn't count). Never allocates.
    pub fn on_strip(&self, strip: usize) -> Option<(usize, Held)> {
        (0..HOST_SLOTS).find_map(|s| {
            let t = self.slots[s].track?;
            let own = self.slot(t) == Some(s) && self.host.track_of(s) == Some(strip);
            own.then_some(())?;
            Some((t, self.slots[s].held?))
        })
    }

    /// Track `track`'s own instrument (None: it has none; it may still
    /// play another track's, [`Instruments::owner`]).
    pub fn held(&self, track: usize) -> Option<Held> {
        self.slots[self.slot(track)?].held
    }

    /// Whether track `track` has its own instrument.
    pub fn holds(&self, track: usize) -> bool {
        self.slot(track).is_some()
    }

    /// The track whose instrument track `track` plays: itself while it
    /// has its own, else the one it sends to. None: it plays the pads.
    pub fn owner(&self, track: usize) -> Option<usize> {
        if self.holds(track) {
            return Some(track);
        }
        let to = usize::from(*self.sends.get(track)?);
        self.holds(to).then_some(to)
    }

    /// Whether track `track` plays an instrument here: its own or another
    /// track's (else it plays the pads).
    pub fn plays(&self, track: usize) -> bool {
        self.owner(track).is_some()
    }

    /// The track whose instrument track `track` plays though it has none
    /// of its own.
    pub fn send(&self, track: usize) -> Option<usize> {
        match self.sends.get(track) {
            Some(&to) if to != NO_SEND => Some(usize::from(to)),
            _ => None,
        }
    }

    /// The tracks that play holder `owner`'s instrument, one bit each (the
    /// pads aren't counted: [`Instruments::pads_play_at`]).
    pub fn senders(&self, owner: usize) -> u128 {
        self.sends[..TRACKS]
            .iter()
            .enumerate()
            .filter(|&(_, &to)| usize::from(to) == owner)
            .fold(0, |bits, (t, _)| bits | 1 << t)
    }

    /// The slot track `track`'s notes go to: its own instrument's, else
    /// the one it sends to.
    fn play_slot(&self, track: usize) -> Option<usize> {
        self.slot(self.owner(track)?)
    }

    /// Track `track` (or the pads, [`THE_PADS`], one of their layers,
    /// [`layer`], a route row, [`routed`], or the audition, [`AUDITION`])
    /// plays holder `to`'s instrument from now on (None: the pads again;
    /// the other players: nothing). `to` is another track or a free place.
    /// The notes it left sounding on the old one end first. Refused while
    /// the track has its own instrument, and when `to` has none. Never
    /// allocates.
    pub fn set_send(&mut self, track: usize, to: Option<usize>) -> Result<(), SendRefused> {
        let player = track < TRACKS || (THE_PADS..WHO).contains(&track);
        if !player || to.is_some_and(|t| t >= HOLDERS) {
            return Err(SendRefused::NoTrack);
        }
        if self.send(track) == to {
            return Ok(());
        }
        if let Some(to) = to {
            if self.holds(track) {
                return Err(SendRefused::HasOwn);
            }
            if to == track {
                return Err(SendRefused::Itself);
            }
            if !self.holds(to) {
                return Err(SendRefused::NothingThere);
            }
        }
        self.end_notes(track, 0);
        self.sends[track] = to.map_or(NO_SEND, |t| t as u8);
        Ok(())
    }

    /// Every send ends (a project opens with its own), each track's notes
    /// on the instrument it played ending first.
    pub fn clear_sends(&mut self) {
        for t in 0..WHO {
            if self.sends[t] != NO_SEND {
                self.end_notes(t, 0);
                self.sends[t] = NO_SEND;
            }
        }
    }

    /// Ends the notes track `track` holds on the instrument it plays, its
    /// pads' and its keys', `offset` frames into the next block.
    fn end_notes(&mut self, track: usize, offset: u32) {
        if track >= WHO {
            return;
        }
        let started = std::mem::replace(&mut self.started[track], [Started::default(); PADS]);
        let keys = std::mem::take(&mut self.keys[track]);
        let Some(s) = self.play_slot(track) else {
            return;
        };
        for st in &started {
            for &n in st.notes() {
                self.host.note_off(s, offset, n);
            }
        }
        for n in (0..128u8).filter(|&n| keys >> n & 1 == 1) {
            self.host.note_off(s, offset, n);
        }
    }

    /// Forgets the notes held on track `owner`'s instrument, its own and
    /// those of the tracks that send to it (its instrument went or was
    /// silenced).
    fn forget_notes(&mut self, owner: usize) {
        for t in 0..WHO {
            if t == owner || usize::from(self.sends[t]) == owner {
                self.started[t] = [Started::default(); PADS];
                self.keys[t] = 0;
            }
        }
    }

    /// Track `track`'s instrument (its own or the one it sends to) plays
    /// notes (not the Drums engine's sounds), so it transposes (sequencer
    /// S4).
    pub fn plays_notes(&self, track: usize) -> bool {
        self.owner(track)
            .and_then(|o| self.held(o))
            .is_some_and(|h| h.id != crate::synth::Engine::Drums.node_id())
    }

    /// The tracks that play notes ([`Instruments::plays_notes`]), one bit
    /// each: where a new take holds notes (step 5c). Never allocates.
    pub fn note_tracks(&self) -> u128 {
        (0..TRACKS.min(128))
            .filter(|&t| self.plays_notes(t))
            .fold(0, |bits, t| bits | 1 << t)
    }

    /// The tracks that play their own instrument, one bit each (not one
    /// only ringing out an old one): what the project's list of
    /// instruments follows (`InstrumentList::follow`). Never allocates.
    pub fn held_tracks(&self) -> u128 {
        self.slot_of[..TRACKS]
            .iter()
            .enumerate()
            .filter(|&(_, &s)| s != NO_SLOT)
            .fold(0, |bits, (t, _)| bits | 1 << t)
    }

    /// The free places that hold an instrument, one bit each (place `n`:
    /// bit `n`): what the project's list follows for them
    /// (`InstrumentList::follow_free`). Never allocates.
    pub fn held_free(&self) -> u16 {
        self.slot_of[TRACKS..HOLDERS]
            .iter()
            .enumerate()
            .filter(|&(_, &s)| s != NO_SLOT)
            .fold(0, |bits, (n, _)| bits | 1 << n)
    }

    /// How many instruments are loaded: the tracks' own and the free ones.
    pub fn count(&self) -> usize {
        self.slot_of[..HOLDERS]
            .iter()
            .filter(|&&s| s != NO_SLOT)
            .count()
    }

    /// Whether an instrument for holder `track` would go in: it has one
    /// already (the new one takes its place), or fewer than [`SLOTS`] are
    /// loaded.
    pub fn has_room(&self, track: usize) -> bool {
        track < HOLDERS && (self.holds(track) || self.count() < SLOTS)
    }

    /// Voices sounding in the tracks' own instruments (a plugin counts
    /// none).
    pub fn voices(&self) -> usize {
        self.host.voices()
    }

    /// Load shedding (`shed.rs`): the voice to give up first across the
    /// tracks' instruments, with its slot, and giving it up. No allocation.
    pub fn cull_candidate(&self) -> Option<(usize, mpfree_node::Cull)> {
        self.host.cull_candidate()
    }

    pub fn cull(&mut self, slot: usize) -> bool {
        self.host.cull(slot)
    }

    /// The tracks whose mixer channels are in use, one bit each: playing
    /// their own instrument or still ringing out an old one.
    pub fn in_use(&self) -> u128 {
        (0..HOST_SLOTS)
            .filter(|&s| self.slots[s].held.is_some() || self.host.ringing(s))
            .filter_map(|s| self.slots[s].track)
            .filter(|&t| t < TRACKS)
            .fold(0, |m, t| m | 1 << t)
    }

    /// The free places whose strips are in use, one bit each: holding an
    /// instrument or still ringing out an old one.
    pub fn free_in_use(&self) -> u16 {
        (0..HOST_SLOTS)
            .filter(|&s| self.slots[s].held.is_some() || self.host.ringing(s))
            .filter_map(|s| self.slots[s].track)
            .filter(|&t| (TRACKS..HOLDERS).contains(&t))
            .fold(0, |m, t| m | 1 << (t - TRACKS))
    }

    /// The pads play holder `to`'s instrument from now on, by a send
    /// (None: they play the track again), the notes they left sounding on
    /// the old one ending first. False (nothing changes) when `to` holds
    /// none.
    pub fn pads_play(&mut self, to: Option<usize>) -> bool {
        self.set_send(THE_PADS, to).is_ok()
    }

    /// Note `note` heard on the instrument track `track` plays, at
    /// `velocity`, as the audition's own ([`AUDITION`], piano roll plan
    /// 1C.2): it moves to that instrument first, ending what it left
    /// sounding on another. False when the track plays none here.
    pub fn audition_on(&mut self, track: usize, note: u8, velocity: u8) -> bool {
        let Some(to) = self.owner(track) else {
            return false;
        };
        if self.owner(AUDITION) != Some(to) && self.set_send(AUDITION, Some(to)).is_err() {
            return false;
        }
        self.note_on(AUDITION, note, velocity, 0)
    }

    /// An audition note's end ([`Instruments::audition_on`]): the note
    /// goes quiet only if nothing else holds it.
    pub fn audition_off(&mut self, note: u8) -> bool {
        self.note_off(AUDITION, note, 0)
    }

    /// The holder whose instrument the pads play, if they play one here.
    pub fn pads_play_at(&self) -> Option<usize> {
        self.owner(THE_PADS)
    }

    /// The pads' layer `i` (step 11a, [`layer`]) plays holder `to`'s
    /// instrument from now on (None: nothing), the notes it left sounding
    /// on the old one ending first. False (nothing changes) when `to`
    /// holds none, or past the layers.
    pub fn set_layer(&mut self, i: usize, to: Option<usize>) -> bool {
        i + 1 < PAD_LAYERS && self.set_send(layer(i), to).is_ok()
    }

    /// The holder whose instrument the pads' layer `i` plays.
    pub fn layer_at(&self, i: usize) -> Option<usize> {
        (i + 1 < PAD_LAYERS).then(|| self.owner(layer(i))).flatten()
    }

    /// Route row `j` (step 11e, [`routed`]) plays holder `to`'s
    /// instrument from now on (None: nothing), the notes it left sounding
    /// on the old one ending first. False (nothing changes) when `to`
    /// holds none, or past the rows.
    pub fn set_routed(&mut self, j: usize, to: Option<usize>) -> bool {
        j < ROUTE_SYNTHS && self.set_send(routed(j), to).is_ok()
    }

    /// The holder whose instrument route row `j` plays.
    pub fn routed_at(&self, j: usize) -> Option<usize> {
        (j < ROUTE_SYNTHS).then(|| self.owner(routed(j))).flatten()
    }

    /// Whether player `who` (a track, the pads or a layer) holds key
    /// `note` on the instrument it plays (a keyboard's, the arp's), so
    /// its let-go has a note to end there.
    pub fn key_held(&self, who: usize, note: u8) -> bool {
        note < 128 && self.keys.get(who).is_some_and(|k| k >> note & 1 == 1)
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

    /// Puts `inst` in for holder `track` (a track, or a free place), with
    /// `pads` as its pad map and `tuning`: in its slot if it has one (the
    /// old instrument rings out), else in a slot lent to it, routed to its
    /// channel ([`Instruments::channel`]). Says why if it can't go in,
    /// and hands `inst` back. Never allocates.
    pub fn put(
        &mut self,
        track: usize,
        inst: (Instrument, Held),
        pads: [Pad; PADS],
        tuning: &Tuning,
    ) -> Result<(), (Refused, Option<Instrument>)> {
        let (inst, held) = inst;
        if track >= HOLDERS {
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
            // A track that played another's instrument plays its own now.
            if self.send(track).is_some() {
                self.end_notes(track, 0);
                self.sends[track] = NO_SEND;
            }
            self.host.route(s, Some(self.channel(track)));
            self.slots[s].track = Some(track);
            self.slots[s].base = self.host.stats(s);
            self.slot_of[track] = s as u8;
        }
        let slot = &mut self.slots[s];
        slot.held = Some(held);
        slot.pads = pads;
        // The tracks that send to it play the new one (the id stays).
        self.forget_notes(track);
        // The new instrument starts from its own sound: the old one's
        // locks are forgotten.
        let t = track as u32;
        self.locks.forget(|k| k >> 16 == t, |_, _| {});
        Ok(())
    }

    /// Takes track `track`'s instrument out, so it plays the pads again,
    /// and so do the tracks that sent to it ([`Instruments::senders`]
    /// before says which). A track that only sends stops sending. The
    /// instrument rings out into the track's channel, and its slot is
    /// free once it is quiet. False if it can't be retired yet (an older
    /// one still rings out there and the list to drop is full): the track
    /// keeps it. Never allocates.
    pub fn take_out(&mut self, track: usize) -> bool {
        let Some(s) = self.slot(track) else {
            let _ = self.set_send(track, None);
            return true;
        };
        let cuts = self.host.ringing(s);
        if self.host.swap(s, None).is_err() {
            self.refused += 1;
            return false;
        }
        self.cut += u64::from(cuts);
        self.slots[s].held = None;
        self.forget_notes(track);
        for to in self
            .sends
            .iter_mut()
            .filter(|to| usize::from(**to) == track)
        {
            *to = NO_SEND;
        }
        self.slot_of[track] = NO_SLOT;
        let t = track as u32;
        self.locks.forget(|k| k >> 16 == t, |_, _| {});
        true
    }

    /// A new pad map for track `track` (its notes so far end as they
    /// started).
    pub fn set_pads(&mut self, track: usize, pads: [Pad; PADS]) {
        if let Some(s) = self.slot(track) {
            self.slots[s].pads = pads;
        }
    }

    /// The notes pad `pad` plays on the instrument track `track` plays (its
    /// pad map's, as written): none if the track plays none, or past the
    /// pads.
    pub fn pad_notes(&self, track: usize, pad: usize) -> &[u8] {
        self.play_slot(track)
            .and_then(|s| self.slots[s].pads.get(pad))
            .map_or(&[], Pad::notes)
    }

    /// The pad map of the instrument track `track` plays (its own or
    /// another track's): what its pads press into the track's arp while it
    /// plays notes (step 5d). None if it plays none.
    pub fn pad_map(&self, track: usize) -> Option<[Pad; PADS]> {
        self.play_slot(track).map(|s| self.slots[s].pads)
    }

    /// The pad of the instrument track `track` plays that plays `note`
    /// alone (its pad map's), for a MIDI file's notes (`midi_file.rs`):
    /// None if the track plays none, Some(None) if no pad does.
    pub fn pad_of_note(&self, track: usize, note: u8) -> Option<Option<u8>> {
        let s = self.play_slot(track)?;
        Some(
            self.slots[s]
                .pads
                .iter()
                .position(|p| p.notes() == [note])
                .map(|p| p as u8),
        )
    }

    /// [`Instruments::pad_on_shifted`] as written (the tests').
    #[cfg(test)]
    pub fn pad_on(&mut self, track: usize, pad: usize, velocity: u8, offset: u32) -> bool {
        self.pad_on_shifted(track, pad, velocity, offset, &Shift::NONE)
    }

    /// Pad `pad` down on track `track`, `offset` frames into the next
    /// block, its notes moved by the track's live transpose (sequencer S4,
    /// `shift`), on the instrument it plays. False if it plays none (it
    /// plays the pads). Its note-off and pressure reach the notes it
    /// started, whatever the shift or the send is by then.
    pub fn pad_on_shifted(
        &mut self,
        track: usize,
        pad: usize,
        velocity: u8,
        offset: u32,
        shift: &Shift,
    ) -> bool {
        let Some(s) = self.play_slot(track) else {
            return false;
        };
        let Some(p) = self.slots[s].pads.get(pad) else {
            return true;
        };
        let started = Started::of(p, shift);
        self.started[track][pad] = started;
        for &n in started.notes() {
            self.host.note_on(s, offset, n, velocity.max(1));
        }
        true
    }

    /// Pad `pad` up on track `track`: ends the notes it started.
    pub fn pad_off(&mut self, track: usize, pad: usize, offset: u32) -> bool {
        let Some(s) = self.play_slot(track) else {
            return false;
        };
        let Some(st) = self.started[track].get_mut(pad).map(std::mem::take) else {
            return true;
        };
        for &n in st.notes() {
            self.host.note_off(s, offset, n);
        }
        true
    }

    /// A clip's or the pad's pressure on pad `pad`'s notes (0 to 127).
    pub fn pad_pressure(&mut self, track: usize, pad: usize, value: u8, offset: u32) -> bool {
        let Some(s) = self.play_slot(track) else {
            return false;
        };
        let Some(st) = self.started[track].get(pad).copied() else {
            return true;
        };
        for &note in st.notes() {
            self.host
                .event(s, Timed::new(offset, NodeEvent::Pressure { note, value }));
        }
        true
    }

    /// A MIDI note on track `track` (the arp's, with a layout; a
    /// keyboard's), on the instrument it plays.
    pub fn note_on(&mut self, track: usize, note: u8, velocity: u8, offset: u32) -> bool {
        let Some(s) = self.play_slot(track) else {
            return false;
        };
        if note < 128 {
            self.keys[track] |= 1 << note;
        }
        self.host.note_on(s, offset, note, velocity.max(1));
        true
    }

    pub fn note_off(&mut self, track: usize, note: u8, offset: u32) -> bool {
        let Some(s) = self.play_slot(track) else {
            return false;
        };
        if note < 128 {
            self.keys[track] &= !(1 << note);
        }
        self.host.note_off(s, offset, note);
        true
    }

    /// A keyboard's pressure on note `note` of track `track`'s instrument,
    /// while it holds the note.
    pub fn note_pressure(&mut self, track: usize, note: u8, value: u8) -> bool {
        self.note_pressure_at(track, note, value, 0)
    }

    /// [`Instruments::note_pressure`] `offset` frames into the next block
    /// (a clip's recorded pressure on a note it plays by number).
    pub fn note_pressure_at(&mut self, track: usize, note: u8, value: u8, offset: u32) -> bool {
        let Some(s) = self.play_slot(track) else {
            return false;
        };
        if self.host.holds_note(s, note) {
            self.host
                .event(s, Timed::new(offset, NodeEvent::Pressure { note, value }));
        }
        true
    }

    /// Whether the instrument track `track` plays holds note `note` (a
    /// note-on not yet ended, from whichever track or the pads).
    #[cfg(test)]
    pub fn holds_note(&self, track: usize, note: u8) -> bool {
        self.play_slot(track)
            .is_some_and(|s| self.host.holds_note(s, note))
    }

    /// A controller (a pedal, the mod wheel, breath) for track `track`'s
    /// instrument, 0 to 1, `offset` frames into the next block. The host
    /// holds the notes for an instrument that doesn't hear the pedals.
    pub fn control(
        &mut self,
        track: usize,
        ctl: mpfree_node::Ctl,
        value: f32,
        offset: u32,
    ) -> bool {
        let Some(s) = self.play_slot(track) else {
            return false;
        };
        self.host.control(s, offset, ctl, value);
        true
    }

    /// A keyboard's pitch wheel on track `track`'s instrument, -1 to 1 of
    /// its bend range.
    pub fn bend(&mut self, track: usize, value: f32) -> bool {
        let Some(s) = self.play_slot(track) else {
            return false;
        };
        if value.is_finite() {
            let value = value.clamp(-1.0, 1.0);
            self.host.event(s, Timed::new(0, NodeEvent::Bend { value }));
        }
        true
    }

    /// One note's own expression (an MPE controller's slide or tone) on
    /// track `track`'s instrument, while it holds the note.
    pub fn note_expression(
        &mut self,
        track: usize,
        note: u8,
        kind: mpfree_node::Expr,
        value: f32,
    ) -> bool {
        let Some(s) = self.play_slot(track) else {
            return false;
        };
        if self.host.holds_note(s, note) {
            self.host.event(
                s,
                Timed::new(0, NodeEvent::Expression { note, kind, value }),
            );
        }
        true
    }

    /// A parameter of the instrument track `track` plays (the Synth
    /// tab's knobs), from the next block's first frame.
    pub fn param(&mut self, track: usize, id: u16, value: f32) -> bool {
        let Some(owner) = self.owner(track) else {
            return false;
        };
        // Under a clip's lock, the turn is what its end comes back to, and
        // the hand holds the knob a moment (knob owners, locks.rs).
        self.locks.hand(lock_key(owner, id), value);
        self.slot(owner)
            .is_some_and(|s| self.host.param(s, 0, id, value))
    }

    /// The XY pad's hand on parameter `id` of the instrument track `track`
    /// plays (a layout of yours plays it by Set, `xy_own.rs`): `Some`
    /// holds it at the value, over the clips' locks when `over` (a track
    /// records); `None` lets go, back to its lock or its own value as they
    /// are now (locks.rs). The sound's own value is left alone. False when
    /// the track plays no instrument, it has no such parameter (or it is a
    /// meter), or the table is full.
    pub fn pad_hand(&mut self, track: usize, id: u16, value: Option<f32>, over: bool) -> bool {
        let Some(owner) = self.owner(track) else {
            return false;
        };
        let Some(s) = self.slot(owner) else {
            return false;
        };
        let key = lock_key(owner, id);
        let Some(value) = value else {
            if let Some(v) = self.locks.release(key) {
                self.host.param(s, 0, id, v);
            }
            return true;
        };
        let smooth = match self
            .host
            .params(s)
            .iter()
            .find(|p| p.id == id && !p.readonly)
        {
            Some(p) => crate::locks::glides(p),
            None => return false,
        };
        let Some(now) = self.host.param_value(s, id) else {
            return false;
        };
        match self.locks.press(key, value, now, smooth, over) {
            crate::locks::Pressed::Set(v) => self.host.param(s, 0, id, v),
            crate::locks::Pressed::Kept => true,
            crate::locks::Pressed::Full => false,
        }
    }

    /// Parameter `id` of the instrument track `track` plays, under the XY
    /// pad's hand: what a let-go goes back to now (its lock, else its own
    /// value), and its own value. None when the track plays no instrument
    /// with that parameter.
    pub fn pad_beneath(&self, track: usize, id: u16) -> Option<(f32, f32)> {
        let owner = self.owner(track)?;
        let now = self.host.param_value(self.slot(owner)?, id)?;
        let key = lock_key(owner, id);
        Some((
            self.locks.beneath(key).unwrap_or(now),
            self.locks.own(key).unwrap_or(now),
        ))
    }

    /// Each track's lasting id (`mpfree_seq::TrackId`, 0 for none), by
    /// track: when two tracks' clips lock one knob of an instrument they
    /// share, the lowest owns it (step 4b, locks.rs).
    pub fn set_ranks(&mut self, ids: &[u32; TRACKS]) {
        self.locks.set_ranks(ids);
    }

    /// A clip's lock on parameter `id` of the instrument track `track`
    /// plays (`Owner::Inst`): `Some` sets it on its frame, `offset` frames
    /// into the next block, `None` puts back the value under it (the
    /// sound's own, or the last turn on the Synth tab). Kept by the
    /// instrument's track, so two tracks on one instrument share the
    /// value under a knob, and one owns it at a time (step 4b, locks.rs):
    /// another track's lock is kept, and plays when it owns the knob.
    /// False when the track plays no instrument, the instrument has no
    /// such parameter (or it is a meter), or the table is full.
    pub fn lock(&mut self, track: usize, id: u16, value: Option<f32>, offset: u32) -> bool {
        let Some(owner) = self.owner(track) else {
            return false;
        };
        let Some(s) = self.slot(owner) else {
            return false;
        };
        let key = lock_key(owner, id);
        let Some(value) = value else {
            // The value put back is an ending: it always has a place, as
            // each new lock asked first (below; refactor A2b).
            if let Some(back) = self.locks.unlock(key, track) {
                self.host.unlock(s, offset, id, back);
            }
            return true;
        };
        let smooth = match self
            .host
            .params(s)
            .iter()
            .find(|p| p.id == id && !p.readonly)
        {
            Some(p) => crate::locks::glides(p),
            None => return false,
        };
        let Some(now) = self.host.param_value(s, id) else {
            return false;
        };
        // A new lock asks its slot's room first; in a period too busy it
        // is left out (counted) and its end finds nothing to put back.
        if !self.locks.held_by(key, track) && !self.host.hold(s) {
            return true;
        }
        match self.locks.lock(key, track, value, now, smooth) {
            Some(true) => self.host.param(s, offset, id, value),
            Some(false) => true,
            None => false,
        }
    }

    /// Track `track`'s instrument's node, while it holds one.
    pub fn node(&self, track: usize) -> Option<&dyn mpfree_node::Node> {
        self.host.instrument(self.slot(track)?).map(|i| i.node())
    }

    /// Every note on track `track` ends (a panic, a track muted): on its
    /// own instrument every note, the sending tracks' too; on one it
    /// sends to only its own notes.
    pub fn all_off(&mut self, track: usize) {
        if let Some(s) = self.slot(track) {
            self.forget_notes(track);
            self.host.event(s, Timed::new(0, NodeEvent::AllOff));
        } else {
            self.end_notes(track, 0);
        }
    }

    /// Every note on every track ends with its release (the transport
    /// stopped, a panic to every track).
    pub fn all_off_every(&mut self) {
        self.forget_every_note();
        self.host.all_off();
    }

    /// STOP: every slot's notes end and fade out (`mpfree-track`).
    pub fn stop(&mut self) {
        self.forget_every_note();
        self.host.stop();
    }

    fn forget_every_note(&mut self) {
        for row in self.started.iter_mut() {
            *row = [Started::default(); PADS];
        }
        self.keys.fill(0);
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
        // The locks that ended play now: forget them. A knob a new owner
        // glides to, or the hand let go of, moves from the block's start.
        self.locks.settle();
        let (locks, host, slot_of) = (&mut self.locks, &mut self.host, &self.slot_of);
        locks.tick(frames as u32, |key, value| {
            let (track, id) = ((key >> 16) as usize, (key & 0xFFFF) as u16);
            if let Some(&s) = slot_of.get(track).filter(|&&s| s != NO_SLOT) {
                host.param(usize::from(s), 0, id, value);
            }
        });
        self.host.render(ctx, frames, out);
    }

    /// The slots split over the audio thread and a lane per config
    /// (`TrackHost::set_lanes`; [`crate::cores::Cores::start_tracks`]).
    /// Allocates and starts the lanes' threads.
    pub fn set_lanes(&mut self, configs: Vec<mpfree_lanes::Config>) -> Vec<mpfree_lanes::Setup> {
        self.host.set_lanes(configs)
    }

    /// When the lanes' joins stop waiting this period (the period's one
    /// deadline, [`crate::cores::lane_deadline`]).
    pub fn set_lane_deadline(&mut self, at: Option<std::time::Instant>) {
        self.host.set_deadline(at);
    }

    /// After a late period: the lanes' instruments back, waiting until
    /// `deadline` at most and past it as long as it takes (counted). Call
    /// it right after each write. No allocation.
    pub fn settle(&mut self, deadline: std::time::Instant) {
        self.host.settle(deadline);
    }

    /// Lanes beside the audio thread (0: every instrument plays on it).
    #[cfg(test)]
    pub fn lanes(&self) -> usize {
        self.host.lanes()
    }

    /// Every lane's counts added up.
    pub fn lane_counts(&self) -> LaneCounts {
        let mut all = LaneCounts::default();
        for k in 0..self.host.lanes() {
            if let Some(c) = self.host.lane_counts(k) {
                all.periods += c.periods;
                all.late += c.late;
                all.waits += c.waits;
            }
        }
        all
    }

    /// Instruments moved between cores since the lanes started.
    pub fn lane_moves(&self) -> u64 {
        self.host.moves()
    }

    /// An instrument taken out, after its ring-out: drop it off the audio
    /// thread.
    pub fn pop_retired(&mut self) -> Option<Instrument> {
        self.host.pop_retired()
    }

    /// Endings the instruments' queues found no room for since the start:
    /// a bug, which must stay 0 (refactor A2b).
    pub fn ends_lost(&self) -> u64 {
        self.host.ends_lost()
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
            ends_lost: now.ends_lost - base.ends_lost,
            stray_offs: now.stray_offs - base.stray_offs,
            resets: now.resets - base.resets,
            swaps: now.swaps - base.swaps,
            rested: now.rested - base.rested,
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
        for n in (0..FREE).filter(|&n| self.holds(free(n))) {
            let (Some(h), st) = (self.held(free(n)), self.stats(free(n))) else {
                continue;
            };
            parts.push(format!(
                "free {} {} ({} notes, {} dropped, {} resets)",
                n + 1,
                h.name,
                st.notes,
                st.dropped,
                st.resets
            ));
        }
        let whose = |o: usize| {
            if o < TRACKS {
                format!("track {}'s", o + 1)
            } else {
                format!("free {}", o - TRACKS + 1)
            }
        };
        for t in (0..TRACKS).filter(|&t| !self.holds(t)) {
            if let Some(o) = self.owner(t) {
                parts.push(format!("track {} plays {}", t + 1, whose(o)));
            }
        }
        if let Some(o) = self.pads_play_at() {
            parts.push(format!("the pads play {}", whose(o)));
        }
        for l in 0..PAD_LAYERS - 1 {
            if let Some(o) = self.layer_at(l) {
                parts.push(format!("pads layer {} plays {}", l + 2, whose(o)));
            }
        }
        for j in 0..ROUTE_SYNTHS {
            if let Some(o) = self.routed_at(j) {
                parts.push(format!("MIDI route {} plays {}", j + 1, whose(o)));
            }
        }
        let locks = self.locks.describe();
        if parts.is_empty() && self.refused == 0 && self.cut == 0 && locks.is_none() {
            return None;
        }
        if self.refused > 0 {
            parts.push(format!("{} refused", self.refused));
        }
        if self.cut > 0 {
            let s = if self.cut == 1 { "" } else { "s" };
            parts.push(format!("{} ring-out{s} cut short", self.cut));
        }
        parts.extend(locks);
        Some(parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::{Kind, Node, NodeInfo, ParamInfo};

    const RATE: u32 = 44_100;
    const BLOCK: usize = 64;

    /// A test instrument: a constant level while any note sounds, raised
    /// by its bend and by a note's tone.
    struct Hum {
        sounding: [u8; 128],
        bend: f32,
        tone: f32,
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
            (self.bend, self.tone) = (0.0, 0.0);
        }
        fn process(&mut self, _: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
            for e in events {
                match e.ev {
                    NodeEvent::NoteOn { note, .. } => self.sounding[usize::from(note)] += 1,
                    NodeEvent::NoteOff { note } => self.sounding[usize::from(note)] = 0,
                    NodeEvent::AllOff => self.sounding = [0; 128],
                    NodeEvent::Bend { value } => self.bend = value,
                    NodeEvent::Expression { value, .. } => self.tone = value,
                    _ => {}
                }
            }
            let v = if self.sounding.iter().any(|&n| n > 0) {
                0.5 + 0.25 * self.bend + 0.125 * self.tone
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
        let node = Box::new(Hum {
            sounding: [0; 128],
            bend: 0.0,
            tone: 0.0,
        });
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
            bufs: vec![(vec![0.0; BLOCK], vec![0.0; BLOCK]); first_track() + HOLDERS],
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

    /// Track 1's instrument plays into the Drums strip, its mixer channel;
    /// an old project that keeps track 1 on its old own channel moves it
    /// there, held note and all.
    #[test]
    fn track_1s_instrument_plays_into_drums_or_its_old_channel() {
        let mut i = Instruments::new(RATE, BLOCK);
        assert_eq!(i.channel(0), crate::mixing::DRUMS);
        assert_eq!(i.channel(1), first_track() + 1);
        let pads = std::array::from_fn(|p| Pad::single(60 + p as i16));
        i.put(0, hum(), pads, &Tuning::standard()).unwrap();
        assert!(i.pad_on(0, 0, 100, 0));
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        assert_eq!(level(&t, crate::mixing::DRUMS), 0.5);
        assert_eq!(level(&t, first_track()), 0.0, "the old own channel");

        i.set_first_channel(first_track());
        assert_eq!(i.channel(0), first_track());
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        assert_eq!(level(&t, first_track()), 0.5, "the held note moved");
        assert_eq!(level(&t, crate::mixing::DRUMS), 0.0);
    }

    /// A keyboard's wheel, pressure and an MPE note's tone reach a track's
    /// own instrument; a note's own reach it only while it holds the note.
    #[test]
    fn a_keyboards_wheel_and_a_notes_tone_reach_a_tracks_instrument() {
        let mut i = Instruments::new(RATE, BLOCK);
        let tone = mpfree_node::Expr::Tone;
        assert!(!i.bend(1, 0.5), "no instrument: the synth takes it");
        assert!(!i.note_expression(1, 60, tone, 1.0));
        assert!(!i.note_pressure(1, 60, 9));
        let pads = std::array::from_fn(|p| Pad::single(60 + p as i16));
        i.put(1, hum(), pads, &Tuning::standard()).unwrap();
        assert!(i.note_on(1, 60, 90, 0));
        assert!(i.bend(1, 0.5));
        assert!(
            i.note_expression(1, 61, tone, 1.0),
            "taken, but 61 isn't held"
        );
        assert!(i.note_pressure(1, 60, 9));
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        assert_eq!(level(&t, first_track() + 1), 0.625);
        i.note_expression(1, 60, tone, 1.0);
        let mut t = tracks();
        i.render(&Ctx::new(RATE, BLOCK as u64), BLOCK, &mut t);
        assert_eq!(level(&t, first_track() + 1), 0.75);
    }

    /// A keyboard's sustain pedal holds a track's instrument's notes
    /// until it comes up (the host holds them: the instrument doesn't
    /// hear the pedal itself).
    #[test]
    fn a_keyboards_sustain_pedal_holds_a_tracks_instrument() {
        use mpfree_node::Ctl;
        let mut i = Instruments::new(RATE, BLOCK);
        assert!(
            !i.control(1, Ctl::Sustain, 1.0, 0),
            "no instrument: the synth takes it"
        );
        let pads = std::array::from_fn(|p| Pad::single(60 + p as i16));
        i.put(1, hum(), pads, &Tuning::standard()).unwrap();
        assert!(i.control(1, Ctl::Sustain, 1.0, 0));
        assert!(i.note_on(1, 60, 90, 0));
        assert!(i.note_off(1, 60, 1));
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        assert_eq!(level(&t, first_track() + 1), 0.5, "held by the pedal");
        assert!(i.control(1, Ctl::Sustain, 0.0, 0));
        let mut t = tracks();
        i.render(&Ctx::new(RATE, BLOCK as u64), BLOCK, &mut t);
        assert_eq!(level(&t, first_track() + 1), 0.0, "let go with it");
    }

    fn chromatic_from(base: i16) -> [Pad; PADS] {
        std::array::from_fn(|p| Pad::single(base + p as i16))
    }

    fn render_levels(i: &mut Instruments) -> Tracks {
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        t
    }

    /// Step 4: a track with no instrument of its own plays another
    /// track's, whose sound stays in that track's channel; it has no
    /// instrument and no channel of its own for it.
    #[test]
    fn a_track_plays_another_tracks_instrument_into_that_tracks_channel() {
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(2, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        assert_eq!(i.set_send(5, Some(2)), Ok(()));
        assert_eq!(
            (i.owner(5), i.send(5), i.plays(5)),
            (Some(2), Some(2), true)
        );
        assert_eq!((i.holds(5), i.held(5)), (false, None), "none of its own");
        assert_eq!(i.senders(2), 1 << 5);
        assert_eq!(i.in_use(), 1 << 2, "no channel of its own");
        assert!(i.plays_notes(5));
        assert_eq!(i.pad_of_note(5, 63), Some(Some(3)));
        assert!(i.pad_on(5, 0, 100, 0));
        let t = render_levels(&mut i);
        assert_eq!(level(&t, first_track() + 2), 0.5);
        assert_eq!(level(&t, first_track() + 5), 0.0);
        assert!(i.pad_off(5, 0, 0));
        let t = render_levels(&mut i);
        assert_eq!(level(&t, first_track() + 2), 0.0);
        assert_eq!(i.stats(2).stray_offs, 0);
        assert!(i.describe().unwrap().contains("track 6 plays track 3's"));
        // Back to the pads.
        assert_eq!(i.set_send(5, None), Ok(()));
        assert!(!i.plays(5) && !i.pad_on(5, 0, 100, 0));
    }

    /// Step 6: a free place holds an instrument no track has, on a mixer
    /// strip of its own. The pads play it by their own player, a track can
    /// play it by a send (heard on its strip), and taking it out empties
    /// the place. The places count toward the [`SLOTS`] instruments.
    #[test]
    fn hearing_a_note_never_cuts_the_same_note_a_clip_holds() {
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(2, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        // A clip holds note 64 on track 3's instrument; the note editor
        // hears the same note, and lets it go: the clip's plays on.
        assert!(i.note_on(2, 64, 100, 0));
        assert!(i.audition_on(2, 64, 90));
        assert!(i.audition_off(64));
        assert!(i.holds_note(2, 64), "the clip's note sounds on");
        assert!(i.note_off(2, 64, 0));
        assert!(!i.holds_note(2, 64));
        // Heard alone, it ends with its own let-go.
        assert!(i.audition_on(2, 67, 90));
        assert!(i.holds_note(2, 67));
        assert!(i.audition_off(67));
        assert!(!i.holds_note(2, 67));
        // Track 5 plays track 3's instrument: heard there too. A track
        // that plays the pads hears nothing here.
        assert_eq!(i.set_send(4, Some(2)), Ok(()));
        assert!(i.audition_on(4, 70, 90));
        assert!(i.holds_note(2, 70));
        assert!(!i.audition_on(6, 70, 90));
        // The instrument taken out: what the audition held is forgotten.
        assert!(i.take_out(2));
        assert!(!i.audition_off(70), "nothing there to end");
    }

    #[test]
    fn a_free_place_holds_an_instrument_the_pads_play_on_its_own_strip() {
        let mut i = Instruments::new(RATE, BLOCK);
        let place = free(0);
        let strip = crate::mixing::free_channel(0);
        assert!(!i.pads_play(Some(place)), "nothing there yet");
        assert!(i
            .put(HOLDERS, hum(), chromatic_from(60), &Tuning::standard())
            .is_err());
        i.put(place, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        assert_eq!(i.channel(place), strip);
        assert_eq!((i.held_free(), i.held_tracks(), i.in_use()), (1, 0, 0));
        assert_eq!((i.count(), i.free_in_use()), (1, 1));
        assert!(!i.pad_on(THE_PADS, 0, 100, 0), "the pads play the track");
        // The pads play it, into its strip only.
        assert!(i.pads_play(Some(place)));
        assert_eq!(i.pads_play_at(), Some(place));
        assert!(i.pad_on(THE_PADS, 0, 100, 0));
        let t = render_levels(&mut i);
        assert_eq!(level(&t, strip), 0.5);
        for other in (0..first_track() + HOLDERS).filter(|&k| k != strip) {
            assert_eq!(level(&t, other), 0.0, "channel {other}");
        }
        assert!(i.pad_off(THE_PADS, 0, 0));
        let t = render_levels(&mut i);
        assert_eq!(level(&t, strip), 0.0);
        // Track 6 plays it too, heard on its strip.
        assert_eq!(i.set_send(5, Some(place)), Ok(()));
        assert_eq!(i.heard_on(5), Some(strip));
        assert_eq!(i.senders(place), 1 << 5, "a track's, not the pads'");
        let said = i.describe().unwrap();
        assert!(said.contains("free 1 Hum"), "{said}");
        assert!(said.contains("track 6 plays free 1"), "{said}");
        assert!(said.contains("the pads play free 1"), "{said}");
        // The pads leave it, a note they left sounding ends; the track
        // leaves it; out it goes.
        i.pad_on(THE_PADS, 1, 100, 0);
        assert!(i.pads_play(None));
        let t = render_levels(&mut i);
        assert_eq!(level(&t, strip), 0.0, "the pads' note ended");
        assert_eq!(i.set_send(5, None), Ok(()));
        assert!(i.take_out(place));
        assert_eq!((i.held_free(), i.count()), (0, 0));
    }

    /// Step 11a: the pads' layers. Each layer player plays a held
    /// instrument by a send, as the pads' own does, so one hit sounds on
    /// each strip; a layer that moves on ends the notes it left there, and
    /// a key's let-go finds its own. Only holders with an instrument, and
    /// only the layers there are.
    #[test]
    fn the_pads_layers_play_each_hit_on_each_strip_and_end_where_they_started() {
        let mut i = Instruments::new(RATE, BLOCK);
        let (a, b) = (free(0), free(1));
        let (sa, sb) = (
            crate::mixing::free_channel(0),
            crate::mixing::free_channel(1),
        );
        assert!(!i.set_layer(0, Some(b)), "nothing there yet");
        for h in [a, b] {
            i.put(h, hum(), chromatic_from(60), &Tuning::standard())
                .unwrap();
        }
        assert!(i.pads_play(Some(a)));
        assert!(i.set_layer(0, Some(b)));
        assert!(!i.set_layer(PAD_LAYERS - 1, Some(b)), "past the layers");
        assert_eq!((i.layer_at(0), i.layer_at(1)), (Some(b), None));
        assert!(i.describe().unwrap().contains("pads layer 2 plays free 2"));
        // One hit, each strip.
        i.pad_on(THE_PADS, 0, 100, 0);
        i.pad_on(layer(0), 0, 100, 0);
        let t = render_levels(&mut i);
        assert_eq!((level(&t, sa), level(&t, sb)), (0.5, 0.5));
        i.pad_off(THE_PADS, 0, 0);
        i.pad_off(layer(0), 0, 0);
        let t = render_levels(&mut i);
        assert_eq!((level(&t, sa), level(&t, sb)), (0.0, 0.0));
        // A held key; the layer moves to A: the key's note ends on B.
        assert!(i.note_on(layer(0), 64, 100, 0));
        assert!(i.key_held(layer(0), 64) && !i.key_held(THE_PADS, 64));
        assert!(i.set_layer(0, Some(a)));
        assert!(!i.key_held(layer(0), 64), "ended where it started");
        let t = render_levels(&mut i);
        assert_eq!(level(&t, sb), 0.0);
        // Taken out: the layer plays nothing.
        assert!(i.take_out(a));
        assert_eq!((i.layer_at(0), i.pads_play_at()), (None, None));
        assert!(!i.pad_on(layer(0), 0, 100, 0));
        assert_eq!(i.stats(b).stray_offs, 0);
    }

    /// A MIDI route that names a synth (step 11e) plays it on a route row
    /// by a send, as a layer does: a key sounds on the synth's strip, ends
    /// there when the row moves on, and nothing plays once it is taken
    /// out. Only the rows there are.
    #[test]
    fn a_route_row_plays_the_synth_its_route_names() {
        let mut i = Instruments::new(RATE, BLOCK);
        let (a, b) = (free(0), free(1));
        let sb = crate::mixing::free_channel(1);
        assert!(!i.set_routed(0, Some(b)), "nothing there yet");
        for h in [a, b] {
            i.put(h, hum(), chromatic_from(60), &Tuning::standard())
                .unwrap();
        }
        assert!(i.set_routed(0, Some(b)));
        assert!(!i.set_routed(ROUTE_SYNTHS, Some(b)), "past the rows");
        assert_eq!((i.routed_at(0), i.routed_at(1)), (Some(b), None));
        assert!(i.describe().unwrap().contains("MIDI route 1 plays free 2"));
        assert!(i.note_on(routed(0), 64, 100, 0));
        let t = render_levels(&mut i);
        assert!(level(&t, sb) > 0.0, "heard on its strip");
        assert!(i.set_routed(0, Some(a)));
        assert!(!i.key_held(routed(0), 64), "ended where it started");
        let t = render_levels(&mut i);
        assert_eq!(level(&t, sb), 0.0);
        assert!(i.take_out(a));
        assert_eq!(i.routed_at(0), None);
        assert!(!i.note_on(routed(0), 64, 100, 0));
        assert_eq!(i.stats(b).stray_offs, 0);
    }

    /// Where a track is heard while it plays an instrument, for a clip's
    /// "this track's strip" (time/strips.rs): its own instrument's strip,
    /// or that of the instrument it plays; nowhere of this table's while
    /// it plays the pads.
    #[test]
    fn a_track_is_heard_where_the_instrument_it_plays_sounds() {
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(2, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        assert_eq!(i.set_send(5, Some(2)), Ok(()));
        let three = i.strip(2).expect("track 3's strip");
        assert_eq!(three, first_track() + 2);
        assert_eq!((i.heard_on(2), i.heard_on(5)), (Some(three), Some(three)));
        assert_eq!(i.heard_on(4), None, "on the pads");
        assert_eq!(i.set_send(5, None), Ok(()));
        assert_eq!(i.heard_on(5), None);
    }

    #[test]
    fn a_send_is_refused_where_it_cant_play() {
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(2, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        i.put(3, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        assert_eq!(i.set_send(2, Some(3)), Err(SendRefused::HasOwn));
        assert_eq!(i.set_send(5, Some(5)), Err(SendRefused::Itself));
        assert_eq!(i.set_send(5, Some(7)), Err(SendRefused::NothingThere));
        assert_eq!(i.set_send(TRACKS, Some(2)), Err(SendRefused::NoTrack));
        // A free place (step 6) with nothing in it, and past them.
        assert_eq!(i.set_send(5, Some(TRACKS)), Err(SendRefused::NothingThere));
        assert_eq!(i.set_send(5, Some(HOLDERS)), Err(SendRefused::NoTrack));
        // No chains: a track that sends has no instrument to send to.
        i.set_send(5, Some(2)).unwrap();
        assert_eq!(i.set_send(6, Some(5)), Err(SendRefused::NothingThere));
        assert_eq!(i.send(6), None);
    }

    /// Two tracks on one instrument each end their own notes, even on the
    /// same pad moved apart by their transposes.
    #[test]
    fn two_tracks_on_one_instrument_each_end_their_own_notes() {
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(2, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        i.set_send(5, Some(2)).unwrap();
        i.pad_on(2, 0, 100, 0);
        i.pad_on_shifted(5, 0, 100, 0, &Shift::semitones(12));
        i.pad_off(2, 0, 0);
        let t = render_levels(&mut i);
        assert_eq!(
            level(&t, first_track() + 2),
            0.5,
            "track 6's 72 still sounds"
        );
        i.pad_off(5, 0, 0);
        let t = render_levels(&mut i);
        assert_eq!(level(&t, first_track() + 2), 0.0);
        assert_eq!(i.stats(2).stray_offs, 0);
    }

    /// A send that moves ends the notes the track left sounding where they
    /// started; their note-offs later reach nothing.
    #[test]
    fn a_send_that_moves_ends_its_notes_where_they_started() {
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(1, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        i.put(2, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        i.set_send(5, Some(1)).unwrap();
        i.pad_on(5, 0, 100, 0);
        i.note_on(5, 70, 100, 0);
        let t = render_levels(&mut i);
        assert_eq!(level(&t, first_track() + 1), 0.5);
        i.set_send(5, Some(2)).unwrap();
        let t = render_levels(&mut i);
        assert_eq!(level(&t, first_track() + 1), 0.0, "pad and key ended there");
        i.pad_off(5, 0, 0);
        i.note_off(5, 70, 0);
        let t = render_levels(&mut i);
        assert_eq!(level(&t, first_track() + 2), 0.0);
        assert_eq!(i.stats(1).stray_offs, 0);
        // The key's note-off is a stray on track 3's (it never had it),
        // which the host counts; the pad's reached nothing.
        assert_eq!(i.stats(2).stray_offs, 1);
    }

    /// A new sound in an instrument keeps its sends (its id stays);
    /// taking it out ends them (its id is gone), and the tracks play the
    /// pads again.
    #[test]
    fn a_new_sound_keeps_the_sends_and_taking_it_out_ends_them() {
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(2, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        i.set_send(5, Some(2)).unwrap();
        i.set_send(6, Some(2)).unwrap();
        i.put(2, hum(), chromatic_from(48), &Tuning::standard())
            .unwrap();
        assert_eq!((i.owner(5), i.owner(6)), (Some(2), Some(2)));
        assert!(i.take_out(2));
        assert_eq!((i.send(5), i.send(6), i.senders(2)), (None, None, 0));
        assert!(!i.pad_on(5, 0, 100, 0), "the pads again");
        // A track that only sends is taken out by ending its send.
        i.put(3, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        i.set_send(7, Some(3)).unwrap();
        assert!(i.take_out(7));
        assert_eq!((i.send(7), i.owner(3)), (None, Some(3)));
    }

    /// A track that sends and is given its own instrument plays that, and
    /// its notes on the one it sent to end.
    #[test]
    fn a_track_given_its_own_instrument_stops_sending() {
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(2, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        i.set_send(5, Some(2)).unwrap();
        i.pad_on(5, 0, 100, 0);
        i.put(5, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        assert_eq!((i.send(5), i.owner(5), i.senders(2)), (None, Some(5), 0));
        let t = render_levels(&mut i);
        assert_eq!(level(&t, first_track() + 2), 0.0, "its note there ended");
        assert_eq!(level(&t, first_track() + 5), 0.0, "nothing held on its own");
    }

    /// A panic on a track that sends ends only its own notes; on the
    /// instrument's own track, every note on it.
    #[test]
    fn a_panic_on_a_sending_track_ends_only_its_notes() {
        let mut i = Instruments::new(RATE, BLOCK);
        i.put(2, hum(), chromatic_from(60), &Tuning::standard())
            .unwrap();
        i.set_send(5, Some(2)).unwrap();
        i.pad_on(2, 0, 100, 0);
        i.pad_on(5, 4, 100, 0);
        i.all_off(5);
        let t = render_levels(&mut i);
        assert_eq!(
            level(&t, first_track() + 2),
            0.5,
            "track 3's own note plays on"
        );
        i.pad_on(5, 4, 100, 0);
        i.all_off(2);
        let t = render_levels(&mut i);
        assert_eq!(level(&t, first_track() + 2), 0.0);
        i.pad_off(5, 4, 0);
        assert_eq!(i.stats(2).stray_offs, 0, "its note was forgotten");
    }

    /// A lock from a track that sends plays on the instrument it plays,
    /// and the value under it is the instrument's, whichever track's
    /// lock ends it; a turn by hand from either track is that value.
    #[test]
    fn a_lock_from_a_sending_track_moves_the_instrument_it_plays() {
        let synth = crate::synth::Synth::new(RATE);
        let (id, extras, values) = synth.sound_of(crate::synth::Engine::Poly);
        let plugins = crate::plugins::Plugins::none();
        let inst = make(&plugins, id, &extras, &values, RATE, BLOCK).expect("made");
        let mut i = Instruments::new(RATE, BLOCK);
        let pads = synth.pads_for(crate::synth::Engine::Poly);
        i.put(1, inst, pads, &Tuning::standard()).unwrap();
        i.set_send(4, Some(1)).unwrap();
        let node = i.node(1).unwrap();
        let p = *node
            .params()
            .iter()
            .find(|p| !p.readonly && p.max > p.min && p.curve == mpfree_node::Curve::Linear)
            .unwrap();
        let own = node.param(p.id);
        let locked = if own < (p.min + p.max) / 2.0 {
            p.max
        } else {
            p.min
        };
        let render = |i: &mut Instruments| {
            let mut t = tracks();
            i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        };
        assert!(i.lock(4, p.id, Some(locked), 0));
        assert!(i.lock(1, p.id, Some(locked), 0), "the same knob");
        render(&mut i);
        assert_eq!(i.node(1).unwrap().param(p.id), locked);
        // Track 2's lock ends while track 5's holds: the knob stays with
        // track 5 (knob owners, step 4b); the last end puts it back.
        assert!(i.lock(1, p.id, None, 0));
        render(&mut i);
        assert_eq!(i.node(1).unwrap().param(p.id), locked);
        assert!(i.lock(4, p.id, None, 0));
        render(&mut i);
        assert_eq!(i.node(1).unwrap().param(p.id), own, "the sound's own value");
        let mid = (p.min + p.max) / 2.0;
        assert!(
            i.param(4, p.id, mid),
            "the Synth tab of the track that sends"
        );
        render(&mut i);
        assert_eq!(i.node(1).unwrap().param(p.id), mid);
        assert!(i.node(4).is_none(), "no node of its own");
    }

    /// Step 4b: two tracks on one instrument lock one knob. The track
    /// with the lower lasting id owns it, whichever locked first; the
    /// other's lock is kept, not played. When the owner stops, the
    /// other's value comes back by a short glide; the last end puts the
    /// sound's own back.
    #[test]
    fn two_tracks_locking_one_knob_take_turns_by_their_lasting_ids() {
        let synth = crate::synth::Synth::new(RATE);
        let (id, extras, values) = synth.sound_of(crate::synth::Engine::Poly);
        let plugins = crate::plugins::Plugins::none();
        let inst = make(&plugins, id, &extras, &values, RATE, BLOCK).expect("made");
        let mut i = Instruments::new(RATE, BLOCK);
        let pads = synth.pads_for(crate::synth::Engine::Poly);
        i.put(1, inst, pads, &Tuning::standard()).unwrap();
        i.set_send(4, Some(1)).unwrap();
        let mut ids = [0; TRACKS];
        (ids[1], ids[4]) = (7, 2);
        i.set_ranks(&ids);
        let node = i.node(1).unwrap();
        let p = *node
            .params()
            .iter()
            .find(|p| !p.readonly && p.max > p.min && p.curve == mpfree_node::Curve::Linear)
            .unwrap();
        let own = node.param(p.id);
        let (a, b) = (p.min, p.max);
        let mut frame = 0u64;
        let mut render = |i: &mut Instruments| {
            let mut t = tracks();
            i.render(&Ctx::new(RATE, frame), BLOCK, &mut t);
            frame += BLOCK as u64;
            i.node(1).unwrap().param(p.id)
        };
        assert!(i.lock(1, p.id, Some(a), 0));
        assert!(i.lock(4, p.id, Some(b), 0), "track 5 has the lower id");
        assert!(i.lock(1, p.id, Some(a), 0), "taken, kept");
        assert_eq!(render(&mut i), b);
        assert!(i.lock(4, p.id, None, 0));
        let first = render(&mut i);
        assert!(first < b && first > a, "a glide: {first}");
        let blocks = (RATE as f32 * crate::locks::GLIDE_S) as usize / BLOCK + 1;
        let mut v = first;
        for _ in 0..blocks {
            v = render(&mut i);
        }
        assert_eq!(v, a, "track 2's value");
        assert!(i.lock(1, p.id, None, 0));
        assert_eq!(render(&mut i), own);
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
        // Track 1's instrument plays into the Drums strip.
        assert_eq!(
            level(&t, crate::mixing::DRUMS),
            0.0,
            "note 63 ended, not 43"
        );
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
    fn a_clips_lock_on_a_tracks_instrument_plays_and_its_end_puts_the_sound_back() {
        let synth = crate::synth::Synth::new(RATE);
        let (id, extras, values) = synth.sound_of(crate::synth::Engine::Poly);
        let plugins = crate::plugins::Plugins::none();
        let inst = make(&plugins, id, &extras, &values, RATE, BLOCK).expect("made");
        let mut i = Instruments::new(RATE, BLOCK);
        assert!(!i.lock(1, 0, Some(0.5), 0), "no instrument on the track");
        i.put(
            1,
            inst,
            synth.pads_for(crate::synth::Engine::Poly),
            &Tuning::standard(),
        )
        .unwrap();
        let node = i.node(1).unwrap();
        let p = *node
            .params()
            .iter()
            .find(|p| !p.readonly && p.max > p.min && p.curve == mpfree_node::Curve::Linear)
            .unwrap();
        let own = node.param(p.id);
        let locked = if own < (p.min + p.max) / 2.0 {
            p.max
        } else {
            p.min
        };
        let render = |i: &mut Instruments| {
            let mut t = tracks();
            i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        };
        assert!(i.lock(1, p.id, Some(locked), 5));
        render(&mut i);
        assert_eq!(i.node(1).unwrap().param(p.id), locked);
        assert!(i.lock(1, p.id, None, 0));
        render(&mut i);
        assert_eq!(i.node(1).unwrap().param(p.id), own);
        // A turn under the lock is what its end comes back to.
        let mid = (p.min + p.max) / 2.0;
        assert!(i.lock(1, p.id, Some(locked), 0));
        render(&mut i);
        i.param(1, p.id, mid);
        assert!(i.lock(1, p.id, None, 0));
        render(&mut i);
        assert_eq!(i.node(1).unwrap().param(p.id), mid);
        if let Some(m) = i.node(1).unwrap().params().iter().find(|p| p.readonly) {
            let m = m.id;
            assert!(!i.lock(1, m, Some(1.0), 0), "a meter");
        }
    }

    #[test]
    fn a_track_takes_a_copy_of_the_synth_tabs_sound_and_plays_it_as_the_synth_does() {
        let synth = crate::synth::Synth::new(RATE);
        let (id, extras, values) = synth.sound_of(crate::synth::Engine::Poly);
        assert_eq!(id, "mpfree.poly");
        let plugins = crate::plugins::Plugins::none();
        let (inst, held) = make(&plugins, id, &extras, &values, RATE, BLOCK).expect("made");
        assert_eq!(held.name, "Poly");
        assert!(
            make(&plugins, id, &extras, &values[1..], RATE, BLOCK).is_none(),
            "not its values"
        );
        assert!(
            make(&plugins, "clap:none", &[], &[], RATE, BLOCK).is_none(),
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
                t.bufs[crate::mixing::DRUMS]
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
        assert_eq!(put(&mut i, HOLDERS), Err(Refused::NoTrack));
    }

    /// The project's list of instruments follows the tracks' own (step 2b
    /// of the instrument redesign): a track's instrument keeps its id
    /// through a new sound in its place, and one taken out and put in
    /// again is a new instrument with a new id.
    #[test]
    fn the_list_of_instruments_follows_the_tracks_own() {
        use crate::instrument_list::{InstrumentId, InstrumentList};
        let mut i = Instruments::new(RATE, BLOCK);
        let mut list = InstrumentList::new();
        put(&mut i, 3).unwrap();
        put(&mut i, 9).unwrap();
        assert_eq!(i.held_tracks(), 1 << 3 | 1 << 9);
        assert!(list.follow(i.held_tracks()));
        let (three, nine) = (list.own_of(3), list.own_of(9));
        assert_eq!(
            (three, nine),
            (InstrumentId::from_number(2), InstrumentId::from_number(3))
        );
        put(&mut i, 3).unwrap();
        assert!(!list.follow(i.held_tracks()), "a new sound in its place");
        assert_eq!(list.own_of(3), three);
        assert!(i.take_out(3));
        assert!(list.follow(i.held_tracks()));
        assert_eq!(list.own_of(3), None);
        put(&mut i, 3).unwrap();
        assert!(list.follow(i.held_tracks()));
        assert_eq!(list.own_of(3), InstrumentId::from_number(4));
        assert_eq!(list.own_of(9), nine);
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
        assert_eq!(i.held_tracks() & 1 << freed, 0, "but it holds none");
        assert_eq!(i.held_tracks().count_ones() as usize, i.count());
        let mut t = tracks();
        i.render(&Ctx::new(RATE, 0), BLOCK, &mut t);
        assert_eq!(level(&t, i.channel(next)), 0.5);
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
