//! The pad player's sound from round 6: MPFree's sampler engine
//! (`mpfree-sampler`) in place of the round 3 mixer in `kit.rs`.
//!
//! - Each kit in `kits.bin` becomes a sampler project: each sound is a
//!   source, played whole and one-shot from the pad that sends its note.
//!   Levels match the old mixer (velocity scales the level linearly, and a
//!   sound plays at full scale at velocity 127).
//! - One more kit, "Grains", plays the first kit's sounds as grain clouds
//!   while a pad is held. Pressing harder changes the cloud, by row: the
//!   bottom row (pads 1-4) makes it denser, pads 5-8 make the grains longer,
//!   pads 9-12 move where they read, and the top row (pads 13-16) raises the
//!   pitch up to an octave.
//! - Every engine view is built here, before the sound starts, and every
//!   kit sits in the engine at once (`Engine::with_kits`), sharing its
//!   voices: a kit per track. The clips' notes play their track's kit
//!   ([`Player::hit_kit_at`]); the pads play the kit [`Player::select`]
//!   picked (the armed track's), and nothing is built, allocated or freed
//!   in the audio loop when it changes.
//! - The kits' projects go to the screen thread for the Sample tab
//!   ([`Player::take_projects`]). An edit there builds the kit's view again
//!   on the screen thread and sends it here in a [`ViewSwap`];
//!   [`Player::swap_view`] puts it in place (sounds already playing hear
//!   the edit) and hands the old view back in the same box, so the screen
//!   thread frees it.
//! - While the Grain tab shows, a [`Cloud`] goes back and forth the same
//!   way: the screen thread names the pad, the audio thread copies that
//!   pad's grains into it ([`Player::fill_cloud`]) about 25 times a second
//!   and sends it back.
//! - While its source comes in (the line input while it is on, or a point
//!   in the mix: resampling), the Sample page makes the sampler's input
//!   buffer and sends its writing end here once, in a [`RecFeed`]; each
//!   period [`RecFeed::period`] appends the source and marks the
//!   transport's bar lines in it. When the source goes, the box goes back
//!   to the screen thread, which frees it.

use std::time::Instant;

use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{
    CloudState, Engine, EngineView, Event, GrainView, LaneConfig, LaneSetup, LaneStats, PadField,
    PadOut, MAX_GRAINS,
};
use mpfree_sampler::model::{
    GrainParams, Layer, Pad, PadKind, PlayMode, PressureRoute, PressureTarget, Project, Region,
    Source, PADS,
};

use mpfree_fx::{CapturePoint, StripId};
use mpfree_sampler::record::Feed;
use mpfree_transport::time::frames_between;
use mpfree_transport::Block;

use crate::kit::{Kit, Sound, MAX_KITS, MAX_VOICES};
use crate::looping::PERIOD;

/// The engine pans a centred sound −3 dB a side. Pads are raised by this
/// much, so a kit plays at the level the round 3 mixer played it.
pub const CENTRE_DB: f32 = 3.010_3;

/// The sampler setting a clip's lock on a kit pad's sound names
/// (`Owner::Pad`): its parameter is the [`PadField`]'s number (Level 0,
/// Pan 1, Tune 2, Cutoff 3, Resonance 4, Attack 5, Decay 6, Release 7), and
/// its value is in the field's own unit (dB, -1 to 1, cents, Hz, 0 to 1,
/// ms).
pub fn pad_field(param: u16) -> Option<PadField> {
    PadField::ALL.get(usize::from(param)).copied()
}

/// What each row of the Grains kit's pads changes with pressure: target,
/// depth.
const GRAIN_ROWS: [(PressureTarget, f32); 4] = [
    (PressureTarget::GrainDensity, 0.75), // ×8 at full pressure
    (PressureTarget::GrainSize, 0.75),    // ×4.8
    (PressureTarget::GrainPosition, 0.5), // half way through the sound
    (PressureTarget::Pitch, 1.0),         // an octave up
];

/// Counters for the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub steals: u64,
    pub dropped_events: u64,
    pub peak_grains: usize,
    pub skipped_grains: u64,
    /// Split ([`Player::split`]): parts of periods the lane rendered half
    /// the voices for, and the ones it was late for (that half unheard for
    /// the part). Both 0 unsplit.
    pub lane_parts: u64,
    pub lane_late: u64,
}

pub struct Player {
    /// Every kit's view, one kit slot each ([`Engine::with_kits`]).
    engine: Engine,
    /// The kit the pads play (the armed track's).
    kit: usize,
    /// Pads with a sound, per kit: bit p for pad p (pads 1 to 128).
    sounds: Vec<u128>,
    peak_grains: usize,
    /// Each kit's project, until the screen thread takes them.
    projects: Vec<Project>,
    /// What each kit's pads of another kind do ([`pad_routes`]), by kit:
    /// swapped in with the kit's view, never made or freed here.
    routes: Vec<Vec<Option<PadRoute>>>,
    /// The empty kit slots kept after the others ([`USER_KITS`]).
    users: std::ops::Range<usize>,
}

/// What each track plays when it has no instrument of its own: an entry
/// of the list of kits, a kit (the Grains kit among them) or the synth
/// after them. Every track plays kit 1 until a pick changes it; then the
/// pads play the armed track's entry, and each track's clips play its own
/// (a kit per track, the sampler review's decision 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackKits {
    entries: [u8; mpfree_seq::MAX_TRACKS],
    /// The synth's entry: the one after the kits.
    synth: u8,
}

impl Default for TrackKits {
    /// Every track on kit 1, before the kits are known.
    fn default() -> TrackKits {
        TrackKits::new(usize::MAX)
    }
}

impl TrackKits {
    /// For `kits` kits (the Grains kit counted), the synth after them.
    pub fn new(kits: usize) -> TrackKits {
        TrackKits {
            entries: [0; mpfree_seq::MAX_TRACKS],
            synth: kits.min(usize::from(u8::MAX)) as u8,
        }
    }

    /// Track `track`'s entry (kit 1's, 0, for a track out of range).
    pub fn entry(&self, track: usize) -> usize {
        self.entries.get(track).map_or(0, |&e| usize::from(e))
    }

    /// Track `track` plays `entry` from now on. False, and nothing
    /// changes, for a track or entry out of range.
    pub fn set(&mut self, track: usize, entry: usize) -> bool {
        match self.entries.get_mut(track) {
            Some(e) if entry <= usize::from(self.synth) => {
                *e = entry as u8;
                true
            }
            _ => false,
        }
    }

    /// Only `tracks` tracks are left (UNDO took a new track away): every
    /// place from there on plays kit 1 again, so a track added there later
    /// starts on kit 1, never on the kit the one before it had (which
    /// stays in its slot). True if any changed. Never allocates.
    pub fn forget_past(&mut self, tracks: usize) -> bool {
        let mut changed = false;
        for e in self.entries.iter_mut().skip(tracks) {
            if *e != 0 {
                *e = 0;
                changed = true;
            }
        }
        changed
    }

    /// Whether any track plays entry `entry` (a kit, or the synth).
    pub fn plays(&self, entry: usize) -> bool {
        self.entries.iter().any(|&e| usize::from(e) == entry)
    }

    /// Whether track `track` plays the synth.
    pub fn plays_synth(&self, track: usize) -> bool {
        self.entry(track) == usize::from(self.synth)
    }

    /// The kit track `track` plays, or None if it plays the synth.
    pub fn kit_of(&self, track: usize) -> Option<usize> {
        let e = self.entry(track);
        (e != usize::from(self.synth)).then_some(e)
    }

    /// The synth's entry.
    pub fn synth(&self) -> usize {
        usize::from(self.synth)
    }

    /// The project's track kits part ([`crate::project::TRACK_KITS`]): a
    /// line for each track that plays another kit than kit 1, or the
    /// synth, numbered from 1 as the screen shows them. Only the first
    /// line while every track plays kit 1, so the part isn't saved.
    pub fn text(&self) -> String {
        use std::fmt::Write as _;
        let mut s = format!("{}\n", crate::project::TRACK_KITS_FORMAT);
        for (t, &e) in self.entries.iter().enumerate().filter(|(_, &e)| e != 0) {
            let _ = if e == self.synth {
                writeln!(s, "track {} synth", t + 1)
            } else {
                writeln!(s, "track {} kit {}", t + 1, e + 1)
            };
        }
        s
    }

    /// Where each kit plays in the mixer (`MixHost::pads_homed`): `homes[k]`
    /// is 1 + the first track that plays kit `k` (`plays(t)`: track `t`
    /// plays its entry, not an instrument of its own or MIDI Out), or 0:
    /// kit 1, and a kit no track plays, sound on the pad channels. Tracks
    /// that share a kit share its channel. Never allocates.
    pub fn homes(&self, plays: impl Fn(usize) -> bool) -> [u8; mpfree_sampler::engine::MAX_KITS] {
        let mut homes = [0u8; mpfree_sampler::engine::MAX_KITS];
        for (t, &e) in self.entries.iter().enumerate() {
            let k = usize::from(e);
            if k > 0 && e != self.synth && homes.get(k) == Some(&0) && plays(t) {
                homes[k] = (t + 1) as u8;
            }
        }
        homes
    }
}

/// A kit's engine view, built again after an edit on the Sample tab, on its
/// way to the audio thread; then the view it replaced, on its way back to
/// the screen thread, which frees it. It travels boxed, so the channels
/// move a pointer and the audio thread never frees anything.
#[derive(Clone, Debug)]
pub struct ViewSwap {
    pub kit: usize,
    pub view: Option<EngineView>,
    /// What the kit's pads of another kind do, by pad ([`pad_routes`]):
    /// they travel with the view and swap with the ones the player held.
    pub routes: Vec<Option<PadRoute>>,
}

/// What a hit on a kit pad of another kind does (the Pads tab's
/// Instrument and MIDI out pads, the redesign's step 6b): the sampler
/// plays nothing for it, and the pad player sends it here. Worked out off
/// the audio thread, which only reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PadRoute {
    /// One voice of the synth's engine at a note.
    Voice(crate::synth::Engine, u8),
    /// Note `note` on `channel` (0 to 15) of the MIDI output named `port`.
    Midi { port: String, channel: u8, note: u8 },
}

/// Each of `project`'s pads' [`PadRoute`], one per pad: None for a pad the
/// sampler plays, and for an Instrument pad whose engine isn't one of the
/// synth's (the Pads tab says it plays nothing). Allocates.
pub fn pad_routes(project: &Project) -> Vec<Option<PadRoute>> {
    (0..PADS)
        .map(|i| match &project.pad(i)?.kind {
            PadKind::Sample => None,
            PadKind::Instrument { engine, note } => {
                crate::synth::Engine::from_folder(engine).map(|e| PadRoute::Voice(e, *note))
            }
            PadKind::MidiOut {
                port,
                channel,
                note,
            } => Some(PadRoute::Midi {
                port: port.clone(),
                channel: channel.saturating_sub(1).min(15),
                note: *note,
            }),
        })
        .collect()
}

impl PartialEq for ViewSwap {
    /// The same kit and the same project revision.
    fn eq(&self, other: &ViewSwap) -> bool {
        self.kit == other.kit
            && self.view.as_ref().map(|v| v.revision) == other.view.as_ref().map(|v| v.revision)
    }
}

/// How many grains a [`Cloud`] holds: two voices' worth.
pub const CLOUD_GRAINS: usize = 2 * MAX_GRAINS;

/// The grains one layer of one pad plays now, for the Grain tab's cloud.
/// The screen thread names the kit, pad and layer and sends it to the audio
/// thread, which fills it in place and sends it back in the same box: the
/// channels move a pointer, and neither side allocates for it after the
/// first.
#[derive(Clone, Debug, PartialEq)]
pub struct Cloud {
    pub kit: usize,
    pub pad: u8,
    pub layer: u8,
    /// How many of `grains` are filled.
    pub len: usize,
    pub grains: [GrainView; CLOUD_GRAINS],
    /// Where that layer's first voice reads, in source frames.
    pub head: Option<u64>,
    /// Grains playing on every pad, and how many may.
    pub live: usize,
    /// How that layer plays now: its spray as moved, and whether it is
    /// thinned ([`mpfree_sampler::engine::Engine::cloud_of`]).
    pub state: Option<CloudState>,
}

impl Default for Cloud {
    fn default() -> Cloud {
        Cloud {
            kit: 0,
            pad: 0,
            layer: 0,
            len: 0,
            grains: [GrainView::default(); CLOUD_GRAINS],
            head: None,
            live: 0,
            state: None,
        }
    }
}

impl Cloud {
    /// The grains filled in.
    pub fn grains(&self) -> &[GrainView] {
        &self.grains[..self.len.min(CLOUD_GRAINS)]
    }
}

/// How many playheads [`Playheads`] holds.
pub const PLAYHEADS: usize = 8;

/// Where the playing kit's voices are, in source frames (first layers
/// only), for the playheads on the Sample tab. Small and fixed, so the
/// audio thread sends it without allocating.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Playheads {
    pub kit: u16,
    pub len: u8,
    pub pad: [u8; PLAYHEADS],
    pub frame: [u32; PLAYHEADS],
    /// Live chop is taking cuts (its audition plays).
    pub marking: bool,
    /// The cuts live chop has made so far, and the last of them in source
    /// frames: cut `i` is `marks[i % PLAYHEADS]` while `marks_made - i` is
    /// at most [`PLAYHEADS`] ([`Playheads::mark`]).
    pub marks_made: u64,
    pub marks: [u32; PLAYHEADS],
}

impl Playheads {
    /// Live chop's cut `i`, while it is among the last [`PLAYHEADS`].
    pub fn mark(&self, i: u64) -> Option<u64> {
        (i < self.marks_made && self.marks_made - i <= PLAYHEADS as u64)
            .then(|| u64::from(self.marks[(i % PLAYHEADS as u64) as usize]))
    }

    /// The frames of pad `pad`'s voices.
    pub fn of(&self, pad: usize) -> impl Iterator<Item = u64> + '_ {
        let n = usize::from(self.len).min(PLAYHEADS);
        self.pad[..n]
            .iter()
            .zip(&self.frame[..n])
            .filter(move |(p, _)| usize::from(**p) == pad)
            .map(|(_, f)| u64::from(*f))
    }
}

/// The sound's audio as interleaved stereo floats at full scale ±1.
fn floats(s: &Sound) -> Vec<f32> {
    s.audio.iter().map(|&x| f32::from(x) / 32768.0).collect()
}

/// The velocity to send the engine for a hit at `v`. The engine's level
/// is (velocity / 127)²; this makes it v / 127, as the old mixer's was.
pub fn engine_velocity(v: u8) -> u8 {
    if v == 0 {
        return 0;
    }
    let x = f32::from(v.min(127)) / 127.0;
    ((x.sqrt() * 127.0).round() as u8).clamp(1, 127)
}

/// The sampler's voice pool: up to [`MAX_VOICES`] (24) full voices, as it
/// has always had, and the rest for plain one-shots (the kits' drum hits:
/// no filter, envelope open, at their own pitch), which cost about a
/// tenth as much each (the bench's "Sampler, drum kit" cases: 64 plain
/// voices take less than a fifth of what 24 full-path ones did). Jay's
/// round 4 kept 24 voices full most of the time; with Note Repeat 64 hold
/// four sounds on every pad.
pub const POOL_VOICES: usize = 64;

/// Voices one kit pad holds before its own oldest gives way. A pad hit
/// again and again (Note Repeat on a long sound) used to fill the whole
/// pool and cut short every other pad's sound (Jay's round 4: 1,592 of
/// 2,545 hits took a voice that was still playing); four copies of one
/// sound is already more than the ear tells apart.
pub const KIT_POLYPHONY: u8 = 4;

/// Empty kit slots the app keeps after the kits and Grains (sampler R3
/// part 3): the list of kits' **New kit** puts the next one on the armed
/// track, and a kit loaded onto a kit other tracks play goes into one, so
/// loading never changes another track's sound. A slot is only arrays
/// until sounds go on it; the slots are fixed, so the kits' places (and
/// the synth's after them) never move while MPFree runs.
pub const USER_KITS: usize = 8;

fn kit_pad(name: &str, region: Region) -> Pad {
    kit_pad_with(name, Layer::new(region))
}

/// A new pad as the kits' own play: centred at the level a kit plays at,
/// [`KIT_POLYPHONY`] voices. The browser's Load onto an empty pad makes
/// these too, so a loaded sound plays as loud as the kit's (a plain
/// [`Pad::new`] is 3 dB quieter: the engine's centre pan).
pub fn kit_pad_with(name: &str, layer: Layer) -> Pad {
    let mut pad = Pad::new(name, layer);
    pad.polyphony = KIT_POLYPHONY;
    pad.level_db = CENTRE_DB;
    pad
}

fn grain_pad(name: &str, source: &Source, row: usize) -> Pad {
    let mut r = Region::whole(source);
    r.mode = PlayMode::Granular;
    r.grain = GrainParams {
        position: 0.1,
        spray: 0.05,
        size_ms: 60.0,
        density: 25.0,
        pitch_spray: 0.1,
        scan: 0.25,
        spread: 0.7,
        ..GrainParams::default()
    };
    let mut pad = kit_pad(name, r);
    pad.polyphony = 2;
    pad.amp = Ahdsr {
        attack_ms: 5.0,
        hold_ms: 0.0,
        decay_ms: 0.0,
        sustain: 1.0,
        release_ms: 250.0,
    };
    let (target, depth) = GRAIN_ROWS[row];
    pad.pressure = PressureRoute { target, depth };
    pad
}

impl Player {
    /// Builds every kit's engine view (and, if there is room, the Grains
    /// kit, which is added to `kits` for the screen and the log). The kits'
    /// own audio is handed over to the engine, so their `audio` is empty
    /// afterwards. `pad_notes[p]` is the note pad `p` sends. Call it before
    /// the sound starts: it allocates.
    #[cfg(test)]
    pub fn new(kits: &mut Vec<Kit>, pad_notes: &[u8; 16], rate: u32) -> Result<Player, String> {
        Player::with_user_kits(kits, pad_notes, rate, 0)
    }

    /// As [`Player::new`], with `user` empty kits after the others (as
    /// many as fit the engine), each added to `kits` as "Kit N" with no
    /// sounds ([`USER_KITS`]).
    pub fn with_user_kits(
        kits: &mut Vec<Kit>,
        pad_notes: &[u8; 16],
        rate: u32,
        user: usize,
    ) -> Result<Player, String> {
        if kits.is_empty() {
            return Err("no kits".to_string());
        }
        let mut views = Vec::with_capacity(kits.len() + 1 + user);
        let mut sounds = Vec::with_capacity(kits.len() + 1 + user);
        let mut projects = Vec::with_capacity(kits.len() + 1 + user);
        let mut first: Vec<(usize, Source)> = Vec::new();
        for (k, kit) in kits.iter_mut().enumerate() {
            let mut project = Project::new();
            let mut has = 0u128;
            for (p, &note) in pad_notes.iter().enumerate() {
                let Some(s) = kit.sounds.iter_mut().find(|s| s.note == note) else {
                    continue;
                };
                let source = Source::new(&s.name, rate, 2, floats(s))
                    .map_err(|e| format!("kit {} ({}), {}: {e:?}", k + 1, kit.name, s.name))?;
                s.audio = Vec::new();
                project.add_source(source.clone());
                project
                    .set_pad(p, Some(kit_pad(&s.name, Region::whole(&source))), None)
                    .map_err(|e| format!("kit {} ({}), {}: {e:?}", k + 1, kit.name, s.name))?;
                has |= 1 << p;
                if k == 0 {
                    first.push((p, source));
                }
            }
            views.push(Some(EngineView::build(&project, rate)));
            sounds.push(has);
            projects.push(project);
        }
        if kits.len() < MAX_KITS {
            let mut project = Project::new();
            let mut has = 0u128;
            let mut grain_sounds = Vec::new();
            for (p, source) in &first {
                project.add_source(source.clone());
                let name = kits[0]
                    .sounds
                    .iter()
                    .find(|s| s.note == pad_notes[*p])
                    .map_or(String::new(), |s| s.name.clone());
                project
                    .set_pad(*p, Some(grain_pad(&name, source, p / 4)), None)
                    .map_err(|e| format!("Grains, {name}: {e:?}"))?;
                has |= 1 << *p;
                grain_sounds.push(Sound {
                    note: pad_notes[*p],
                    name,
                    audio: Vec::new(),
                });
            }
            views.push(Some(EngineView::build(&project, rate)));
            sounds.push(has);
            projects.push(project);
            kits.push(Kit {
                name: format!("Grains: {}", kits[0].name),
                sounds: grain_sounds,
            });
        }
        let first_user = kits.len();
        for _ in 0..user.min(mpfree_sampler::engine::MAX_KITS.saturating_sub(kits.len())) {
            views.push(Some(EngineView::build(&Project::new(), rate)));
            sounds.push(0);
            projects.push(Project::new());
            kits.push(Kit {
                name: format!("Kit {}", kits.len() + 1),
                sounds: Vec::new(),
            });
        }
        let mut engine = Engine::with_kits(rate, POOL_VOICES, views.len());
        engine.set_full_voices(MAX_VOICES);
        for (k, view) in views.into_iter().enumerate() {
            engine.set_kit_view(k, view.expect("built above"));
        }
        let routes = projects.iter().map(pad_routes).collect();
        Ok(Player {
            engine,
            kit: 0,
            sounds,
            peak_grains: 0,
            projects,
            routes,
            users: first_user..kits.len(),
        })
    }

    /// The empty kit slots kept for New kit ([`Player::with_user_kits`]).
    pub fn user_kits(&self) -> std::ops::Range<usize> {
        self.users.clone()
    }

    #[cfg(test)]
    pub fn kits(&self) -> usize {
        self.engine.kits()
    }

    pub fn kit(&self) -> usize {
        self.kit
    }

    /// Which of kit `kit`'s 128 pads have a sound, bit p for pad p (none
    /// past the last kit).
    pub fn sounds(&self, kit: usize) -> u128 {
        self.sounds.get(kit).copied().unwrap_or(0)
    }

    /// Whether kit `kit`'s pad `pad` has a sound.
    pub fn has_sound(&self, kit: usize, pad: usize) -> bool {
        pad < PADS && self.sounds(kit) & (1 << pad) != 0
    }

    /// Which pads of page `page` (PAD BANK A = 0) of kit `kit` have a
    /// sound, as the 16 pads show them.
    pub fn page_sounds(&self, kit: usize, page: usize) -> [bool; 16] {
        std::array::from_fn(|p| self.has_sound(kit, page * 16 + p))
    }

    /// The pads play kit `k` from the next hit on (the armed track's).
    /// Every kit's clips go on playing, and so do sounds already playing;
    /// the master grain effect takes kit `k`'s settings. Never allocates
    /// or frees.
    pub fn select(&mut self, k: usize) {
        if k == self.kit || k >= self.engine.kits() {
            return;
        }
        self.kit = k;
        self.engine.set_master_kit(k);
    }

    /// Every kit's project, in kit order (the Grains kit last), for the
    /// Sample tab. The audio is shared with the engine's views, not copied.
    /// Afterwards the player holds none.
    pub fn take_projects(&mut self) -> Vec<Project> {
        std::mem::take(&mut self.projects)
    }

    /// Puts `swap`'s view in place for kit `swap.kit` in the engine and
    /// leaves the view it replaces in `swap`, to be freed off the audio
    /// thread. It is an edit of that kit, so its sounds already playing
    /// hear it ([`Engine::edit_kit_view`]: loop points, level, pan, filter
    /// and tune), and the next hit plays the new view; the pads with a
    /// sound are the view's (a chop adds some). Never allocates or frees.
    /// Returns false, leaving `swap` as it was, if there is no such kit.
    pub fn swap_view(&mut self, swap: &mut ViewSwap) -> bool {
        if swap.kit >= self.engine.kits() {
            return false;
        }
        let Some(view) = swap.view.take() else {
            return false;
        };
        if let Some(routes) = self.routes.get_mut(swap.kit) {
            std::mem::swap(routes, &mut swap.routes);
        }
        // A chop can put sounds on pads that had none (and undo take them
        // away); a pad of another kind plays too.
        if let Some(has) = self.sounds.get_mut(swap.kit) {
            let routes = &self.routes[swap.kit];
            *has = (0..PADS)
                .filter(|&p| view.has_pad(p) || routes.get(p).is_some_and(Option::is_some))
                .fold(0, |m, p| m | 1 << p);
        }
        swap.view = Some(self.engine.edit_kit_view(swap.kit, view));
        true
    }

    /// What a hit on kit `kit`'s pad `pad` does when the sampler plays
    /// nothing for it: an engine's voice or a note out ([`PadRoute`]).
    /// Never allocates.
    pub fn pad_route(&self, kit: usize, pad: usize) -> Option<&PadRoute> {
        self.routes.get(kit)?.get(pad)?.as_ref()
    }

    /// Every synth-voice pad (the Instrument kind) of every kit, as the
    /// kits' views have them: kit, pad and engine, kit by kit in pad
    /// order (`pad_synths.rs` gives each its own synth). Never allocates.
    pub fn voice_pads(
        &self,
    ) -> impl Iterator<Item = (usize, usize, crate::synth::Engine)> + Clone + '_ {
        self.routes.iter().enumerate().flat_map(|(k, r)| {
            r.iter().enumerate().filter_map(move |(p, x)| match x {
                Some(PadRoute::Voice(e, _)) => Some((k, p, *e)),
                _ => None,
            })
        })
    }

    /// Where the playing kit's voices are. Never allocates.
    pub fn playheads(&self) -> Playheads {
        let mut p = Playheads {
            kit: self.kit.min(usize::from(u16::MAX)) as u16,
            ..Playheads::default()
        };
        for (pad, layer, at) in self.engine.kit_playheads(self.kit) {
            let n = usize::from(p.len);
            if layer != 0 || n >= PLAYHEADS {
                continue;
            }
            p.pad[n] = pad;
            p.frame[n] = at.min(u64::from(u32::MAX)) as u32;
            p.len += 1;
        }
        p.marking = self.engine.marking().is_some() && self.engine.marking_kit() == self.kit;
        p.marks_made = self.engine.marks_made();
        let first = p.marks_made.saturating_sub(PLAYHEADS as u64);
        for i in first..p.marks_made {
            if let Some(at) = self.engine.mark(i) {
                p.marks[(i % PLAYHEADS as u64) as usize] = at.min(u64::from(u32::MAX)) as u32;
            }
        }
        p
    }

    /// Copies the grains `c` asks for (its kit, pad and layer) into it, with
    /// that layer's playhead and the grains playing on every pad of every
    /// kit. Never allocates.
    pub fn fill_cloud(&self, c: &mut Cloud) {
        c.live = self.engine.live_grains();
        (c.len, c.head, c.state) = (0, None, None);
        if c.kit >= self.engine.kits() {
            return;
        }
        c.len = self
            .engine
            .kit_grains_of(c.kit, c.pad, c.layer, &mut c.grains);
        c.state = self.engine.kit_cloud_of(c.kit, c.pad, c.layer);
        c.head = self
            .engine
            .kit_playheads(c.kit)
            .find(|&(pad, layer, _)| pad == c.pad && layer == c.layer)
            .map(|(_, _, at)| at);
    }

    /// Live chop on kit `kit`'s pad `pad` (`on`), or off: on plays source
    /// frames `start..end` of its layer `layer` once at `velocity` (as a
    /// tap on a slice does) and, while that plays, live hits
    /// ([`Player::live_hit_at`]) mark cuts in it instead of playing. Off
    /// stops it. A kit out of range does nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn live_chop(
        &mut self,
        kit: usize,
        pad: usize,
        layer: usize,
        start: u64,
        end: u64,
        on: bool,
        velocity: u8,
    ) {
        if kit >= self.engine.kits() || pad >= 128 {
            return;
        }
        // In order, after anything already queued.
        if on {
            self.audition(kit, pad, layer, start, end, velocity);
            self.engine.schedule_for(
                kit,
                0,
                Event::LiveChop {
                    pad: Some(pad as u8),
                },
            );
        } else {
            self.engine
                .schedule_for(kit, 0, Event::LiveChop { pad: None });
        }
    }

    /// Kit `kit`'s pad played live (by hand, or a latched arp): it marks
    /// a cut while live chop takes them on that kit, and plays otherwise
    /// ([`Player::hit_kit_at`]).
    pub fn live_hit_at(&mut self, kit: usize, pad: usize, velocity: u8, offset: u32) -> bool {
        if self.marking(kit) {
            return self.engine.schedule_for(kit, offset, Event::Mark);
        }
        self.hit_kit_at(kit, pad, velocity, offset)
    }

    /// Whether a live hit on kit `kit`'s pad `pad` waits for the launch
    /// line while the transport plays (a Loop pad, sampler R5.3b): never
    /// while live chop takes cuts on that kit, where a hit marks a cut at
    /// once. Never allocates.
    pub fn starts_on_line(&self, kit: usize, pad: usize) -> bool {
        !self.marking(kit) && self.engine.kit_starts_on_line(kit, pad)
    }

    /// Kit `kit`'s pad let go live: nothing while live chop takes cuts on
    /// that kit (the hit played nothing), else [`Player::release_kit_at`].
    pub fn live_release_at(&mut self, kit: usize, pad: usize, offset: u32) {
        if !self.marking(kit) {
            self.release_kit_at(kit, pad, offset);
        }
    }

    /// Whether live chop takes cuts on kit `kit` now.
    fn marking(&self, kit: usize) -> bool {
        self.engine.marking().is_some() && self.engine.marking_kit() == kit
    }

    /// Starts pad `pad`'s sound `offset` frames into the next render, on
    /// the kit the pads play. Returns false if it has no sound on that pad.
    pub fn hit_at(&mut self, pad: usize, velocity: u8, offset: u32) -> bool {
        self.hit_kit_at(self.kit, pad, velocity, offset)
    }

    /// [`Player::hit_at`] on kit `kit` (a clip's note on its track's kit).
    pub fn hit_kit_at(&mut self, kit: usize, pad: usize, velocity: u8, offset: u32) -> bool {
        if !self.has_sound(kit, pad) {
            return false;
        }
        let velocity = engine_velocity(velocity);
        if velocity > 0 {
            self.engine.schedule_for(
                kit,
                offset,
                Event::NoteOn {
                    pad: pad as u8,
                    velocity,
                },
            );
        }
        true
    }

    /// Plays `note` on the playing kit's keygroups, `offset` frames into
    /// the next render, as a key of a keyboard would (the Play tab's
    /// keyboard). Returns false at velocity 0.
    pub fn key_at(&mut self, note: u8, velocity: u8, offset: u32) -> bool {
        let velocity = engine_velocity(velocity);
        if velocity == 0 {
            return false;
        }
        self.engine.schedule_for(
            self.kit,
            offset,
            Event::Key {
                note: note.min(127),
                velocity,
            },
        );
        true
    }

    /// The key `note` was let go, `offset` frames into the next render.
    pub fn key_off_at(&mut self, note: u8, offset: u32) {
        self.engine.schedule_for(
            self.kit,
            offset,
            Event::KeyOff {
                note: note.min(127),
            },
        );
    }

    /// The pad was let go, `offset` frames into the next render, on the
    /// kit the pads play. One-shot sounds ignore it; a Grains pad stops
    /// making grains and fades out.
    pub fn release_at(&mut self, pad: usize, offset: u32) {
        self.release_kit_at(self.kit, pad, offset);
    }

    /// Plays kit `kit`'s take kept on the Sample page (the one its view
    /// holds, `EngineView::build_with_take`) at full velocity (`down`), or
    /// lets it go, `offset` frames into the next render.
    pub fn take_at(&mut self, kit: usize, down: bool, offset: u32) {
        let ev = if down {
            Event::TakeOn { velocity: 127 }
        } else {
            Event::TakeOff
        };
        self.engine.schedule_for(kit, offset, ev);
    }

    /// [`Player::release_at`] on kit `kit`.
    pub fn release_kit_at(&mut self, kit: usize, pad: usize, offset: u32) {
        if pad < PADS {
            self.engine
                .schedule_for(kit, offset, Event::NoteOff { pad: pad as u8 });
        }
    }

    /// Live values on kit `kit`'s pads (the scene crossfader's), applied
    /// before the next render: each replaces the pad's own until it is NaN
    /// or the kit's next view comes.
    pub fn pad_live(&mut self, kit: usize, values: &[(u8, PadField, f32)]) {
        for &(pad, field, value) in values {
            self.engine
                .apply_for(kit, Event::PadLive { pad, field, value });
        }
    }

    /// A step's parameter lock on kit `kit`'s pad `pad`'s `field` (`Some`),
    /// or its end (`None`), `offset` frames into the next render: the
    /// clips' locks on a kit pad's sound (`Owner::Pad`, on the kit of the
    /// lock's track). False for no such pad.
    pub fn pad_lock(
        &mut self,
        kit: usize,
        pad: usize,
        field: PadField,
        value: Option<f32>,
        offset: u32,
    ) -> bool {
        pad < PADS
            && self.engine.schedule_for(
                kit,
                offset,
                Event::PadLock {
                    pad: pad as u8,
                    field,
                    value: value.unwrap_or(f32::NAN),
                },
            )
    }

    /// Plays source frames `start..end` of pad `pad`'s layer `layer` once,
    /// from the next render, as the pad made from that slice will play it
    /// (a tap on the Chop page), on kit `kit`; returns whether it went to
    /// the engine.
    pub fn audition(
        &mut self,
        kit: usize,
        pad: usize,
        layer: usize,
        start: u64,
        end: u64,
        velocity: u8,
    ) -> bool {
        if pad >= PADS || layer > usize::from(u8::MAX) {
            return false;
        }
        let clip = |f: u64| u32::try_from(f).unwrap_or(u32::MAX);
        let velocity = engine_velocity(velocity);
        velocity > 0
            && self.engine.schedule_for(
                kit,
                0,
                Event::Audition {
                    pad: pad as u8,
                    layer: layer as u8,
                    start: clip(start),
                    end: clip(end),
                    velocity,
                },
            )
    }

    /// Pad pressure, 0 to 127, `offset` frames into the next render, on the
    /// kit the pads play.
    pub fn pressure_at(&mut self, pad: usize, value: u8, offset: u32) {
        self.pressure_kit_at(self.kit, pad, value, offset);
    }

    /// [`Player::pressure_at`] on kit `kit` (a clip's recorded pressure).
    pub fn pressure_kit_at(&mut self, kit: usize, pad: usize, value: u8, offset: u32) {
        if pad < PADS {
            self.engine.schedule_for(
                kit,
                offset,
                Event::Pressure {
                    pad: pad as u8,
                    value: value.min(127),
                },
            );
        }
    }

    /// Pad pressure, 0 to 127, from the next render on, on the kit the pads
    /// play.
    pub fn pressure(&mut self, pad: usize, value: u8) {
        if pad < PADS {
            self.engine.schedule_for(
                self.kit,
                0,
                Event::Pressure {
                    pad: pad as u8,
                    value,
                },
            );
        }
    }

    /// The project tempo, for warped pads (cheap when it hasn't moved:
    /// call it every period).
    pub fn set_tempo(&mut self, bpm: f64) {
        self.engine.set_tempo(bpm);
    }

    /// Where the transport is, in beats, at the start of the coming
    /// render (None while it is stopped): a pad's LFO synced to the tempo,
    /// with Retrigger off, keeps to the beats (sampler R5 part 4). Every
    /// period, with the tempo.
    pub fn set_beat(&mut self, beat: Option<f64>) {
        self.engine.set_beat(beat);
    }

    /// Fades every sound out (over a few milliseconds; a Grains pad over its
    /// release).
    pub fn stop_all(&mut self) {
        self.engine.apply(Event::AllOff);
    }

    /// The pads whose hit keeps a latched sound playing, as bits (pad `k`
    /// is bit `k`) of the kit the pads play: their lights stay on.
    pub fn latched_pads(&self) -> u128 {
        self.engine.kit_latched_pads(self.kit)
    }

    pub fn playing(&self) -> usize {
        self.engine.active_voices()
    }

    /// Load shedding (`shed.rs`): the voice the engine would give up
    /// first, and giving it up. No allocation.
    pub fn cull_candidate(&self) -> Option<mpfree_node::Cull> {
        self.engine.cull_candidate()
    }

    pub fn cull(&mut self) -> bool {
        self.engine.cull()
    }

    /// Hits the engine has played (the tests).
    #[cfg(test)]
    pub fn hits(&self) -> u64 {
        self.engine.stats().hits
    }

    pub fn counters(&self) -> Counters {
        let s = self.engine.stats();
        Counters {
            steals: s.steals,
            dropped_events: s.dropped_events,
            peak_grains: self.peak_grains,
            skipped_grains: s.skipped_grains,
            lane_parts: s.lane_parts,
            lane_late: s.lane_late,
        }
    }

    /// Renders the next period into `left` and `right`, scaled by `master`.
    /// Returns the largest absolute sample, before clamping to −1..1 (as the
    /// old mixer did).
    #[cfg(test)]
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32], master: f32) -> f32 {
        self.render_dry(left, right);
        master_out(left, right, master)
    }

    /// The engine's sound alone, every pad in one pair, before the level
    /// (the tests; the pad player renders with [`Player::render_routed`]).
    #[cfg(test)]
    pub fn render_dry(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.engine.render(left, right);
        self.peak_grains = self.peak_grains.max(self.engine.live_grains());
    }

    /// The next `frames` frames with each pad of every kit apart, for the
    /// mixer: pad `p` of kit `k` adds its sound into `out.pad(k, p)` (see
    /// [`Engine::render_routed`]). The pads add up to what
    /// [`Player::render_dry`] renders. (The tests; the audio loop renders
    /// with [`Player::render_routed_until`].)
    #[cfg(test)]
    pub fn render_routed(&mut self, frames: usize, out: &mut impl PadOut) {
        self.engine.render_routed(frames, out);
        self.peak_grains = self.peak_grains.max(self.engine.live_grains());
    }

    /// The next `frames` frames with each pad apart, as `render_routed`
    /// gives them, waiting for the lane's voices (once split) until
    /// `deadline` at most: past it their sound is left out of
    /// that part of the period, and no event is lost (see
    /// [`Engine::render_routed_until`]). The audio loop renders with this,
    /// with the period's deadline (cores.rs).
    pub fn render_routed_until(&mut self, frames: usize, out: &mut impl PadOut, deadline: Instant) {
        self.engine.render_routed_until(frames, out, deadline);
        self.peak_grains = self.peak_grains.max(self.engine.live_grains());
    }

    /// Splits the engine's voices over two cores: half render on a lane
    /// (a worker thread, pinned and real-time as `config` asks) while the
    /// audio thread renders the rest (see [`Engine::split`]). Allocates
    /// and starts a thread: call it before the sound starts. Returns what
    /// the lane got (threaded, pinned, FIFO), or None with fewer than 2
    /// voices. The audio loop calls it with `--sound-cores two` (cores.rs).
    pub fn split(&mut self, config: LaneConfig) -> Option<LaneSetup> {
        self.engine.split(config).cloned()
    }

    /// The lane's counters once split (posted, joined, late, busy, worker
    /// allocations); [`Player::counters`] has the engine's lane parts and
    /// late parts.
    // Not called: the Dev page shows the engine's lane parts and late parts
    // (`counters`, cores.rs).
    #[allow(dead_code)]
    pub fn lane_stats(&self) -> Option<LaneStats> {
        self.engine.lane_stats()
    }
}

/// What the sampler records from (steps CAP and RESAMP): the line input,
/// or a point in the mix taken by the mixer's capture point
/// ([`crate::mixing::SAMPLER_CAPTURE`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecSource {
    /// The line input, as it comes in (before the mixer).
    #[default]
    Line,
    /// A strip of the mixer at a capture point.
    Mix(StripId, CapturePoint),
}

/// The audio thread's end of the sampler's input (steps CAP and RESAMP):
/// the rolling buffer's writer and the bar lines. It travels boxed, so the
/// channels move a pointer and the audio thread never frees the buffer.
pub struct RecFeed(pub Feed);

impl RecFeed {
    /// One period: marks the bar lines the transport's `block` crosses
    /// (from bar 1 on, not the count-in, each with its bar's number) at
    /// their frames in the buffer,
    /// `latency` frames later (a point in the mix hears the block that
    /// much late), then appends the period's input. Never allocates, locks
    /// or waits.
    pub fn period(&mut self, block: &Block, left: &[f32], right: &[f32], latency: u32) {
        let base = self.0.audio.written() + u64::from(latency);
        let mut bar = None;
        for span in block.spans() {
            if !span.state.rolling() {
                continue;
            }
            let ticks = span.meter.bar_ticks();
            for (tick, offset) in span.lines(ticks) {
                if tick >= 0 {
                    // The bar the line starts, numbered from 1 as the
                    // transport shows it (Sample Recall's bar numbers).
                    let bar = u32::try_from(tick / ticks.max(1) + 1).unwrap_or(u32::MAX);
                    self.0.bars.mark_bar(base + u64::from(offset), bar);
                }
            }
            bar = Some(frames_between(0, ticks, span.bpm, span.rate));
        }
        self.0.bars.set_bar(bar);
        let n = left.len().min(right.len()).min(PERIOD);
        self.0.audio.push_planar(&left[..n], &right[..n]);
    }

    /// The input comes from another source from now on: Skip back and
    /// takes stop here.
    pub fn restart(&mut self) {
        self.0.restart();
    }

    /// Frames appended since it started.
    pub fn written(&self) -> u64 {
        self.0.audio.written()
    }
}

impl PartialEq for RecFeed {
    /// A feed is only ever itself: there is one writer per buffer.
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl std::fmt::Debug for RecFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RecFeed({} frames)", self.written())
    }
}

/// The level, then the clip to full scale. Returns the peak before the
/// clip (above 1.0 means the period clipped). The pad player's mixer does
/// this now (the Level is its master fader, Master safety its limit).
#[cfg(test)]
pub fn master_out(left: &mut [f32], right: &mut [f32], master: f32) -> f32 {
    let mut peak = 0f32;
    for s in left.iter_mut().chain(right.iter_mut()) {
        *s *= master;
        peak = peak.max(s.abs());
        *s = s.clamp(-1.0, 1.0);
    }
    peak
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{self, Mixer};

    const NOTES: [u8; 16] = [
        0x25, 0x24, 0x2A, 0x52, 0x28, 0x26, 0x2E, 0x2C, 0x30, 0x2F, 0x2D, 0x2B, 0x31, 0x37, 0x33,
        0x35,
    ];

    fn tone(frames: usize, f: f32) -> Vec<i16> {
        (0..frames)
            .flat_map(|i| {
                let x = (i as f32 * f).sin();
                [(x * 12000.0) as i16, (x * -9000.0) as i16]
            })
            .collect()
    }

    fn kits() -> Vec<Kit> {
        let a = kit::build(&[
            (NOTES[0], "Kick", &tone(3000, 0.01)[..]),
            (NOTES[5], "Snare", &tone(2000, 0.07)[..]),
        ]);
        let b = kit::build(&[(NOTES[0], "Clap", &tone(1500, 0.03)[..])]);
        kit::parse_kits(&kit::build_kits(&[("House", a), ("Trap", b)])).unwrap()
    }

    fn run(p: &mut Player, periods: usize) -> (Vec<f32>, f32) {
        let (mut l, mut r) = ([0f32; 128], [0f32; 128]);
        let mut out = Vec::new();
        let mut peak = 0f32;
        for _ in 0..periods {
            peak = peak.max(p.render(&mut l, &mut r, 0.5));
            out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
        }
        (out, peak)
    }

    fn run_mixer(m: &mut Mixer, periods: usize) -> Vec<f32> {
        let (mut l, mut r) = ([0f32; 128], [0f32; 128]);
        let mut out = Vec::new();
        for _ in 0..periods {
            m.render(&mut l, &mut r, 0.5);
            out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
        }
        out
    }

    #[test]
    fn kits_play_as_the_old_mixer_played_them() {
        for (velocity, tolerance) in [(127u8, 1e-4f32), (100, 1e-2), (40, 1e-2)] {
            let mut mixer = Mixer::new(kits());
            let mut ks = kits();
            let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
            // Two hits, the second on its exact frame in the second period.
            assert!(mixer.hit_at(NOTES[0], velocity, 3).is_some());
            assert!(p.hit_at(0, velocity, 3));
            let (want, got) = (run_mixer(&mut mixer, 1), run(&mut p, 1).0);
            mixer.hit_at(NOTES[5], velocity, 77);
            p.hit_at(5, velocity, 77);
            let want: Vec<f32> = want.into_iter().chain(run_mixer(&mut mixer, 30)).collect();
            let got: Vec<f32> = got.into_iter().chain(run(&mut p, 30).0).collect();
            for (k, (a, b)) in want.iter().zip(&got).enumerate() {
                assert!(
                    (a - b).abs() <= tolerance * a.abs().max(0.01),
                    "velocity {velocity}, sample {k}: {a} vs {b}"
                );
            }
            assert!(want.iter().any(|&x| x.abs() > 0.1));
            assert_eq!((mixer.playing(), p.playing()), (0, 0));
        }
    }

    /// Sixteen pad channels, cleared after each block as the mixer clears its own.
    struct Channels([[[f32; 128]; 2]; 16]);

    impl PadOut for Channels {
        fn pad(&mut self, _kit: usize, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
            let [l, r] = self.0.get_mut(pad)?;
            Some((&mut l[..], &mut r[..]))
        }
    }

    #[test]
    fn routed_pads_land_on_their_own_channels_and_add_up_to_the_dry_render() {
        let (mut ka, mut kb) = (kits(), kits());
        let mut dry = Player::new(&mut ka, &NOTES, 44_100).unwrap();
        let mut routed = Player::new(&mut kb, &NOTES, 44_100).unwrap();
        let mut ch = Channels([[[0.0; 128]; 2]; 16]);
        let (mut l, mut r) = ([0f32; 128], [0f32; 128]);
        let mut peaks = [0f32; 16];
        for period in 0..30 {
            if period == 1 {
                for p in [&mut dry, &mut routed] {
                    assert!(p.hit_at(0, 110, 7));
                    assert!(p.hit_at(5, 90, 64));
                }
            }
            dry.render_dry(&mut l, &mut r);
            routed.render_routed(128, &mut ch);
            for k in 0..128 {
                let sum: f32 = ch.0.iter().map(|c| c[0][k]).sum();
                assert!(
                    (sum - l[k]).abs() < 1e-6,
                    "period {period}, frame {k}: {sum} vs {}",
                    l[k]
                );
            }
            for (pad, c) in ch.0.iter_mut().enumerate() {
                peaks[pad] =
                    peaks[pad].max(c[0].iter().chain(&c[1]).fold(0f32, |m, x| m.max(x.abs())));
                *c = [[0.0; 128]; 2];
            }
        }
        for (pad, peak) in peaks.iter().enumerate() {
            assert_eq!(*peak > 0.05, pad == 0 || pad == 5, "pad {pad}: peak {peak}");
        }
        assert_eq!((dry.playing(), routed.playing()), (0, 0));
    }

    #[test]
    fn a_split_player_plays_as_one_core_does_and_counts_its_lane() {
        let (mut ka, mut kb) = (kits(), kits());
        let mut one = Player::new(&mut ka, &NOTES, 44_100).unwrap();
        let mut two = Player::new(&mut kb, &NOTES, 44_100).unwrap();
        assert_eq!(one.lane_stats(), None);
        let setup = two
            .split(LaneConfig {
                name: "test-pads".into(),
                ..LaneConfig::default()
            })
            .unwrap();
        assert!(setup.threaded.is_ok(), "{setup:?}");
        let (mut ca, mut cb) = (
            Channels([[[0.0; 128]; 2]; 16]),
            Channels([[[0.0; 128]; 2]; 16]),
        );
        let far = || Instant::now() + std::time::Duration::from_secs(5);
        for period in 0..40 {
            if period % 3 == 1 {
                for p in [&mut one, &mut two] {
                    assert!(p.hit_at(0, 110, 7));
                    assert!(p.hit_at(5, 90, 64));
                }
            }
            one.render_routed(128, &mut ca);
            two.render_routed_until(128, &mut cb, far());
            for (a, b) in ca.0.iter_mut().zip(cb.0.iter_mut()) {
                for k in 0..128 {
                    assert!((a[0][k] - b[0][k]).abs() < 1e-5, "period {period}");
                    assert!((a[1][k] - b[1][k]).abs() < 1e-5, "period {period}");
                }
                *a = [[0.0; 128]; 2];
                *b = [[0.0; 128]; 2];
            }
        }
        let c = two.counters();
        assert!(c.lane_parts > 0 && c.lane_late == 0, "{c:?}");
        assert_eq!(two.lane_stats().unwrap().posted, c.lane_parts);
        assert_eq!(
            (one.counters().lane_parts, one.playing()),
            (0, two.playing())
        );
    }

    #[test]
    fn a_slice_plays_once_on_its_own_kit() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        // Kick (pad 1) is 3000 frames: frames 1000 to 1500 of it.
        assert!(!p.audition(0, 128, 0, 1000, 1500, 127));
        assert!(!p.audition(0, 0, 0, 1000, 1500, 0));
        assert!(p.audition(0, 0, 0, 1000, 1500, 127));
        let (out, _) = run(&mut p, 8);
        let last = out.iter().rposition(|x| x.abs() > 1e-6).unwrap() / 2;
        assert!(
            (490..500).contains(&last),
            "the slice is 500 frames (its sine may touch zero at the end): {last}"
        );
        assert_eq!(p.playing(), 0);
        // Every kit is in the engine: kit 2's clap (1500 frames) plays its
        // slice too, whichever kit the pads play.
        assert!(p.audition(1, 0, 0, 1000, 1500, 127));
        run(&mut p, 1);
        assert_eq!(p.playing(), 1);
        run(&mut p, 8);
        assert_eq!(p.playing(), 0);
    }

    #[test]
    fn a_pad_without_a_sound_reports_it() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        assert!(!p.hit_at(1, 127, 0));
        assert!(!p.hit_at(16, 127, 0));
        assert!(p.hit_at(5, 127, 0));
        p.select(1);
        assert!(!p.hit_at(5, 127, 0), "the second kit has nothing on pad 6");
    }

    #[test]
    fn switching_kits_lets_playing_sounds_finish() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        p.hit_at(0, 127, 0);
        run(&mut p, 1);
        p.select(1);
        p.select(9); // out of range: ignored
        assert_eq!(p.kit(), 1);
        p.hit_at(0, 127, 0);
        run(&mut p, 1);
        assert_eq!(p.playing(), 2, "the kick keeps going under the clap");
        run(&mut p, 30);
        assert_eq!(p.playing(), 0);
        // And back: the first kit's view is intact.
        p.select(0);
        assert!(p.hit_at(5, 127, 0));
        let (out, _) = run(&mut p, 2);
        assert!(out.iter().any(|&x| x != 0.0));
    }

    #[test]
    fn stop_fades_everything_out_quickly() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        for _ in 0..5 {
            p.hit_at(0, 127, 0);
        }
        run(&mut p, 2);
        p.stop_all();
        let (out, _) = run(&mut p, 4); // 5 ms release, within 4 periods
        assert_eq!(p.playing(), 0);
        assert!(out[out.len() - 256..].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn one_pad_hit_again_and_again_keeps_to_its_own_voices() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        for _ in 0..MAX_VOICES + 5 {
            p.hit_at(0, 127, 0);
            run(&mut p, 1);
        }
        run(&mut p, 1);
        assert_eq!(p.playing(), usize::from(KIT_POLYPHONY));
        assert_eq!(p.counters().steals, 0, "the pool never filled");
    }

    #[test]
    fn steals_past_the_voice_limit_and_clamps_but_reports_the_peak() {
        let tone = tone(3000, 0.01);
        let sounds: Vec<(u8, &str, &[i16])> =
            NOTES.iter().map(|&n| (n, "Tone", &tone[..])).collect();
        let mut ks = kit::parse_kits(&kit::build_kits(&[("Full", kit::build(&sounds))])).unwrap();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        // Every pad hit once more than it holds, all at once: the kits'
        // plain hits fill the whole pool, past the 24 full voices.
        for k in 0..16 {
            for _ in 0..=KIT_POLYPHONY {
                p.hit_at(k, 127, 0);
            }
        }
        let (out, peak) = run(&mut p, 2);
        assert_eq!(p.playing(), POOL_VOICES);
        const { assert!(POOL_VOICES > MAX_VOICES) };
        assert!(p.counters().steals >= 5);
        assert!(peak > 1.0, "{peak}");
        assert!(out.iter().all(|&x| (-1.0..=1.0).contains(&x)));
    }

    #[test]
    fn the_grains_kit_plays_the_first_kit_as_clouds_while_held() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        assert_eq!((ks.len(), p.kits()), (3, 3));
        assert_eq!(ks[2].name, "Grains: House");
        let names: Vec<(u8, &str)> = ks[2]
            .sounds
            .iter()
            .map(|s| (s.note, s.name.as_str()))
            .collect();
        assert_eq!(names, [(NOTES[0], "Kick"), (NOTES[5], "Snare")]);
        assert!(
            ks.iter()
                .all(|k| k.sounds.iter().all(|s| s.audio.is_empty())),
            "audio handed to the engine"
        );
        p.select(2);
        assert!(p.hit_at(5, 127, 0));
        p.pressure(5, 127); // pad 6 is on the second row: longer grains
        run(&mut p, 100);
        assert!(p.counters().peak_grains >= 2, "{:?}", p.counters());
        assert_eq!(p.playing(), 1, "still sounding while held");
        p.release_at(5, 0);
        run(&mut p, 150); // the 250 ms release and the last grains
        assert_eq!(p.playing(), 0);
    }

    #[test]
    fn user_kits_wait_empty_after_grains_and_play_once_they_have_sounds() {
        let mut ks = kits();
        let mut p = Player::with_user_kits(&mut ks, &NOTES, 44_100, USER_KITS).unwrap();
        // Two kits, Grains, then the empty ones New kit hands out.
        assert_eq!(p.user_kits(), 3..3 + USER_KITS);
        assert_eq!((ks.len(), p.kits()), (3 + USER_KITS, 3 + USER_KITS));
        assert_eq!(ks[3].name, "Kit 4");
        assert!(p
            .user_kits()
            .all(|k| p.sounds(k) == 0 && ks[k].sounds.is_empty()));
        let mut projects = p.take_projects();
        assert_eq!(projects.len(), 3 + USER_KITS);
        // A user kit plays nothing until a sound goes on it; then its
        // view swaps in like any kit's.
        p.select(3);
        assert!(!p.hit_at(0, 127, 0));
        let kick = projects[0].pad(0).unwrap().clone();
        let source = projects[0].sources()[0].clone();
        projects[3].add_source(source);
        projects[3].set_pad(16, Some(kick), None).unwrap();
        let mut swap = ViewSwap {
            kit: 3,
            view: Some(EngineView::build(&projects[3], 44_100)),
            routes: pad_routes(&projects[3]),
        };
        assert!(p.swap_view(&mut swap));
        assert!(p.has_sound(3, 16), "B1 of the new kit");
        assert!(p.hit_kit_at(3, 16, 127, 0));
    }

    #[test]
    fn sixteen_kits_leave_no_room_for_grains() {
        let one = kit::build(&[(NOTES[0], "Kick", &tone(100, 0.01)[..])]);
        let names: Vec<String> = (0..MAX_KITS).map(|k| format!("Kit {k}")).collect();
        let all: Vec<(&str, Vec<u8>)> = names.iter().map(|n| (n.as_str(), one.clone())).collect();
        let mut ks = kit::parse_kits(&kit::build_kits(&all)).unwrap();
        let p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        assert_eq!((ks.len(), p.kits()), (MAX_KITS, MAX_KITS));
    }

    #[test]
    fn an_edited_view_swaps_in_without_allocating_and_the_old_one_goes_back() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        let mut projects = p.take_projects();
        assert_eq!(projects.len(), 3, "two kits and Grains");
        assert!(p.take_projects().is_empty());
        let (whole, _) = {
            p.hit_at(0, 127, 0);
            run(&mut p, 30)
        };
        // The kick's start moves 1000 frames on: the sound starts later in the audio.
        projects[0]
            .edit_pad(0, None, |pad| pad.layers[0].region.start = 1000)
            .unwrap();
        let revision = projects[0].revision();
        let mut swap = Box::new(ViewSwap {
            kit: 0,
            view: Some(EngineView::build(&projects[0], 44_100)),
            routes: pad_routes(&projects[0]),
        });
        let (ok, calls) = crate::sys::alloc_count::calls(|| p.swap_view(&mut swap));
        assert!(ok);
        assert_eq!(calls, 0, "the swap allocates and frees nothing");
        assert_ne!(
            swap.view.as_ref().unwrap().revision,
            revision,
            "the old view comes back in the box"
        );
        p.hit_at(0, 127, 0);
        let (trimmed, _) = run(&mut p, 30);
        assert_ne!(whole, trimmed);
        assert_eq!(&whole[2000..2100], &trimmed[..100], "plays from frame 1000");
        // A kit not playing takes it in its place; out of range gives it back.
        let mut other = Box::new(ViewSwap {
            kit: 1,
            view: Some(EngineView::build(&projects[1], 44_100)),
            routes: pad_routes(&projects[1]),
        });
        assert!(p.swap_view(&mut other));
        assert!(other.view.is_some());
        let mut none = Box::new(ViewSwap {
            kit: 7,
            view: Some(EngineView::build(&projects[1], 44_100)),
            routes: pad_routes(&projects[1]),
        });
        assert!(!p.swap_view(&mut none));
        assert_eq!(
            none.view.as_ref().map(|v| v.revision),
            Some(projects[1].revision())
        );
        p.select(1);
        assert!(p.hit_at(0, 127, 0));
    }

    #[test]
    fn an_edit_reaches_the_sound_still_playing_and_another_kit_does_not() {
        let play = |edit: bool, kit: usize| {
            let mut ks = kits();
            let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
            let mut projects = p.take_projects();
            p.hit_at(0, 127, 0);
            run(&mut p, 4);
            if edit {
                projects[kit]
                    .edit_pad(0, None, |pad| pad.level_db -= 20.0)
                    .unwrap();
                let mut swap = ViewSwap {
                    kit,
                    view: Some(EngineView::build(&projects[kit], 44_100)),
                    routes: pad_routes(&projects[kit]),
                };
                assert!(p.swap_view(&mut swap));
            }
            run(&mut p, 18).0
        };
        let peak = |x: &[f32]| x.iter().fold(0f32, |m, s| m.max(s.abs()));
        let (as_it_was, edited) = (play(false, 0), play(true, 0));
        // 20 dB down once it has glided there (a 10 ms glide: frames 1500
        // to 2300 of the rest; the sound ends at 2488).
        let window = 2 * 1500..2 * 2300;
        let ratio = peak(&edited[window.clone()]) / peak(&as_it_was[window]);
        assert!((0.07..0.13).contains(&ratio), "{ratio}");
        // An edit of the kit not playing doesn't reach what plays.
        assert_eq!(play(true, 1), as_it_was);
    }

    #[test]
    fn a_chopped_kit_plays_its_new_pads_once_its_view_swaps_in() {
        use mpfree_sampler::model::ChopSet;
        let mut kits = kits();
        let mut player = Player::new(&mut kits, &NOTES, 44_100).unwrap();
        let mut projects = player.take_projects();
        let empty = (0..16)
            .find(|&p| !player.has_sound(0, p))
            .expect("a pad without a sound in kit 1");
        assert!(!player.hit_at(empty, 100, 0));
        let from = (0..16).find(|&p| player.has_sound(0, p)).unwrap();
        let project = &mut projects[0];
        let source = project
            .source(project.pad(from).unwrap().layers[0].region.source)
            .unwrap()
            .clone();
        let c = ChopSet::grid("chop", &source, 2).unwrap();
        let template = project.pad(from).cloned();
        project
            .chop_to_pads(c, 0, 1, empty, template.as_ref())
            .unwrap();
        let mut swap = ViewSwap {
            kit: 0,
            view: Some(EngineView::build(project, 44_100)),
            routes: pad_routes(project),
        };
        assert!(player.swap_view(&mut swap));
        assert!(player.has_sound(0, empty) && player.hit_at(empty, 100, 0));
        // Undone, the pad is silent again.
        project.undo();
        let mut swap = ViewSwap {
            kit: 0,
            view: Some(EngineView::build(project, 44_100)),
            routes: pad_routes(project),
        };
        assert!(player.swap_view(&mut swap));
        assert!(!player.hit_at(empty, 100, 0));
    }

    /// Sampler R5.3b: a pad set to start on the launch line (a Loop pad)
    /// says so on the audio thread once its kit's view comes; a pad with
    /// no sound and another kit's pad don't.
    #[test]
    fn a_loop_pad_starts_on_the_line() {
        let mut kits = kits();
        let mut player = Player::new(&mut kits, &NOTES, 44_100).unwrap();
        let mut projects = player.take_projects();
        let pad = (0..16).find(|&p| player.has_sound(0, p)).unwrap();
        let empty = (0..16).find(|&p| !player.has_sound(0, p)).unwrap();
        assert!(!player.starts_on_line(0, pad));
        let project = &mut projects[0];
        let mut d = project.pad(pad).cloned().unwrap();
        d.on_launch = true;
        project.set_pad(pad, Some(d), None).unwrap();
        let mut swap = ViewSwap {
            kit: 0,
            view: Some(EngineView::build(project, 44_100)),
            routes: pad_routes(project),
        };
        assert!(player.swap_view(&mut swap));
        assert!(player.starts_on_line(0, pad));
        assert!(!player.starts_on_line(0, empty) && !player.starts_on_line(1, pad));
    }

    /// R3 part 2b: a kit's pads past 16 (PAD BANK B to H) hold and play
    /// sounds like its first 16.
    #[test]
    fn a_kit_plays_its_pads_past_16() {
        let mut kits = kits();
        let mut player = Player::new(&mut kits, &NOTES, 44_100).unwrap();
        let mut projects = player.take_projects();
        let loud = |p: &mut Player| run(p, 8).0.iter().any(|s| s.abs() > 0.01);
        assert!(!player.has_sound(0, 20) && !player.hit_kit_at(0, 20, 100, 0));
        let from = (0..16).find(|&p| player.has_sound(0, p)).unwrap();
        let copy = projects[0].pad(from).cloned();
        projects[0].set_pad(20, copy, None).unwrap();
        let mut swap = ViewSwap {
            kit: 0,
            view: Some(EngineView::build(&projects[0], 44_100)),
            routes: pad_routes(&projects[0]),
        };
        assert!(player.swap_view(&mut swap));
        assert!(player.has_sound(0, 20));
        assert_eq!(
            (player.sounds(0) >> 16) & 0xFFFF,
            1 << 4,
            "bank B: B5 alone"
        );
        assert!(player.page_sounds(0, 1)[4]);
        assert!(!player.page_sounds(0, 1)[0]);
        assert!(player.hit_kit_at(0, 20, 100, 0));
        assert!(loud(&mut player));
        player.release_kit_at(0, 20, 0);
        // Past the last pad: nothing.
        assert!(!player.has_sound(0, 128) && !player.hit_kit_at(0, 128, 100, 0));
    }

    #[test]
    fn keys_play_the_kits_keygroups_once_it_is_mapped_to_keys() {
        let mut kits = kits();
        let mut player = Player::new(&mut kits, &NOTES, 44_100).unwrap();
        let mut projects = player.take_projects();
        let loud = |p: &mut Player| run(p, 8).0.iter().any(|s| s.abs() > 0.01);
        // No keygroups yet: a key plays nothing.
        assert!(player.key_at(60, 100, 0));
        assert!(!loud(&mut player));
        let pads: Vec<usize> = (0..16).filter(|&p| player.has_sound(0, p)).collect();
        let roots: Vec<(usize, u8)> = pads.iter().map(|&p| (p, 48 + p as u8)).collect();
        projects[0].map_to_keys(&roots).unwrap();
        let mut swap = ViewSwap {
            kit: 0,
            view: Some(EngineView::build(&projects[0], 44_100)),
            routes: pad_routes(&projects[0]),
        };
        assert!(player.swap_view(&mut swap));
        assert!(player.key_at(60, 100, 0));
        assert!(loud(&mut player));
        player.key_off_at(60, 0);
        assert!(!player.key_at(60, 0, 0), "velocity 0 plays nothing");
    }

    #[test]
    fn playheads_follow_the_playing_kits_voices() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        assert_eq!(p.playheads().len, 0);
        p.hit_at(0, 127, 0);
        p.hit_at(5, 127, 64);
        run(&mut p, 4);
        let (heads, calls) = crate::sys::alloc_count::calls(|| p.playheads());
        assert_eq!(calls, 0);
        assert_eq!((heads.kit, heads.len), (0, 2));
        let kick: Vec<u64> = heads.of(0).collect();
        let snare: Vec<u64> = heads.of(5).collect();
        assert_eq!((kick.len(), snare.len()), (1, 1));
        assert!(kick[0] > snare[0] && kick[0] <= 512, "{kick:?} {snare:?}");
        assert_eq!(heads.of(3).count(), 0);
    }

    /// Only 3 tracks left (UNDO took the rest away): every place from
    /// track 4 on plays kit 1 again; the tracks left keep theirs.
    #[test]
    fn a_place_past_the_tracks_plays_kit_1_again() {
        let mut k = TrackKits::new(3);
        assert!(k.set(1, 2) && k.set(3, 1) && k.set(9, 3));
        assert!(k.forget_past(3));
        assert_eq!((k.entry(1), k.entry(3), k.entry(9)), (2, 0, 0));
        assert!(!k.forget_past(3), "nothing more to forget");
    }

    /// Every track plays kit 1 until a pick; a track picks a kit or the
    /// synth; its kit's home is the first track that plays it there, and
    /// a track that plays something else (its own instrument) is passed
    /// over. The project's part lists only the picks.
    #[test]
    fn each_track_plays_its_own_entry_and_a_kit_lives_on_its_first_track() {
        let mut k = TrackKits::new(3);
        assert!((0..mpfree_seq::MAX_TRACKS).all(|t| k.entry(t) == 0 && k.kit_of(t) == Some(0)));
        assert_eq!(k.synth(), 3);
        assert!(k.set(1, 2) && k.set(2, 2) && k.set(4, 3) && k.set(5, 1));
        assert!(!k.set(6, 4), "past the synth");
        assert!(!k.set(mpfree_seq::MAX_TRACKS, 1), "past the tracks");
        assert_eq!((k.kit_of(1), k.kit_of(4)), (Some(2), None));
        assert!(k.plays_synth(4) && !k.plays_synth(1));
        let homes = k.homes(|_| true);
        assert_eq!(
            &homes[..4],
            &[0, 6, 2, 0],
            "kit 1 on the pads, kits 2 and 3 on tracks 6 and 2"
        );
        // Track 2 has its own instrument: kit 3 lives on track 3.
        assert_eq!(k.homes(|t| t != 1)[2], 3);
        assert_eq!(
            k.text(),
            format!(
                "{}\ntrack 2 kit 3\ntrack 3 kit 3\ntrack 5 synth\ntrack 6 kit 2\n",
                crate::project::TRACK_KITS_FORMAT
            )
        );
        assert_eq!(
            crate::project::track_kits_parse(&k.text()),
            Ok(vec![(1, Some(2)), (2, Some(2)), (4, None), (5, Some(1))])
        );
        assert_eq!(TrackKits::new(3).text().lines().count(), 1);
    }

    /// Every kit is in the engine: hits on two kits in one period both
    /// sound, each from its own kit's pads.
    #[test]
    fn two_kits_sound_at_once() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        assert!(p.kits() >= 2);
        let pad_of = |p: &Player, k: usize| (0..16).find(|&i| p.has_sound(k, i)).unwrap();
        let (a, b) = (pad_of(&p, 0), pad_of(&p, 1));
        assert!(p.hit_kit_at(0, a, 100, 0));
        assert!(p.hit_kit_at(1, b, 100, 0));
        run(&mut p, 1);
        assert_eq!(p.playing(), 2, "one voice on each kit");
    }

    #[test]
    fn live_chop_marks_cuts_instead_of_playing_and_the_playheads_carry_them() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        // Kick (pad 1, 3000 frames) from frame 200 to its end.
        p.live_chop(0, 0, 0, 200, 3000, true, 127);
        run(&mut p, 1);
        let heads = p.playheads();
        assert!(heads.marking);
        // The snare hit 10 frames into the next period marks a cut there
        // (128 + 10 frames from 200) and plays nothing.
        assert!(p.live_hit_at(0, 5, 127, 10));
        p.live_release_at(0, 5, 20);
        run(&mut p, 1);
        let heads = p.playheads();
        assert_eq!(heads.marks_made, 1);
        assert_eq!(heads.mark(0), Some(200 + 128 + 10));
        assert_eq!(heads.of(5).count(), 0, "the snare didn't play");
        // Off stops the sound; the snare plays again.
        p.live_chop(0, 0, 0, 0, 0, false, 127);
        run(&mut p, 2);
        assert!(!p.playheads().marking);
        assert_eq!(p.playing(), 0);
        assert!(p.live_hit_at(0, 5, 127, 0));
        run(&mut p, 1);
        assert_eq!(p.playheads().of(5).count(), 1);
        assert_eq!(p.playheads().marks_made, 1);
    }

    #[test]
    fn the_cloud_holds_the_named_pads_grains_without_allocating() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        p.select(2);
        assert!(p.hit_at(5, 127, 0));
        run(&mut p, 40);
        let mut c = Box::new(Cloud {
            kit: 2,
            pad: 5,
            ..Cloud::default()
        });
        let ((), calls) = crate::sys::alloc_count::calls(|| p.fill_cloud(&mut c));
        assert_eq!(calls, 0, "allocations or frees on the audio thread's path");
        assert!(c.len > 0 && c.len == c.grains().len(), "{}", c.len);
        assert!(c.live >= c.len);
        assert!(c
            .grains()
            .iter()
            .all(|g| (0.0..=1.0).contains(&g.position) && g.level >= 0.0));
        assert!(c.head.is_some(), "the voice's reading place");
        // Another pad, layer or kit has none.
        for (kit, pad, layer) in [(2, 0, 0), (2, 5, 1), (0, 5, 0)] {
            (c.kit, c.pad, c.layer) = (kit, pad, layer);
            p.fill_cloud(&mut c);
            assert_eq!((c.len, c.head), (0, None), "{kit} {pad} {layer}");
            assert!(c.live > 0);
        }
        p.release_at(5, 0);
        run(&mut p, 150);
        (c.kit, c.pad, c.layer) = (2, 5, 0);
        p.fill_cloud(&mut c);
        assert_eq!((c.len, c.head, c.live), (0, None, 0));
    }

    #[test]
    fn velocity_maps_to_a_linear_level() {
        assert_eq!(engine_velocity(0), 0);
        assert_eq!(engine_velocity(127), 127);
        for v in 1..=127u8 {
            let level = (f32::from(engine_velocity(v)) / 127.0).powi(2);
            let want = f32::from(v) / 127.0;
            assert!(
                (level - want).abs() <= 0.01,
                "velocity {v}: {level} vs {want}"
            );
        }
    }

    #[test]
    fn the_feed_marks_each_bar_line_at_its_frame_and_appends_the_input() {
        use mpfree_sampler::record::input;
        use mpfree_transport::{Command, Transport};

        let rate = 44_100;
        let mut t = Transport::new(rate, 120.0).unwrap();
        let (feed, inp) = input(10, rate);
        let mut feed = RecFeed(feed);
        let mut block = Block::new();
        // Play from frame 1 000; a 4/4 bar at 120 BPM is 88 200 frames.
        t.command(1_000, Command::Play);
        let (l, r) = ([0.25f32; PERIOD], [-0.25f32; PERIOD]);
        for _ in 0..1_500 {
            assert!(t.process(PERIOD as u32, &mut block));
            feed.period(&block, &l, &r, 0);
        }
        assert_eq!(feed.written(), 1_500 * PERIOD as u64);
        let lines: Vec<u64> = (0..4).map_while(|n| inp.bars.back(n)).collect();
        assert_eq!(lines, [1_000 + 2 * 88_200, 1_000 + 88_200, 1_000]);
        assert_eq!(inp.bars.bar_frames(), Some(88_200.0));
        assert_eq!(inp.last_frames(1).unwrap(), [0.25, -0.25]);
        // Stopped: no bar length, and no more lines.
        t.command(t.frame(), Command::Stop);
        for _ in 0..800 {
            t.process(PERIOD as u32, &mut block);
            feed.period(&block, &l, &r, 0);
        }
        assert_eq!(inp.bars.bar_frames(), None);
        assert_eq!(inp.bars.count(), 3);
    }

    /// The Pads tab's pad types go to the audio thread with the kit's
    /// view: a MIDI out pad's channel as MIDI counts it (1 to 16 is 0 to
    /// 15), an Instrument pad's engine by its id; a Sample pad, and an
    /// engine there is none of, route nothing.
    #[test]
    fn pad_routes_name_each_pads_voice_or_note_out() {
        use crate::synth::Engine;
        use mpfree_sampler::model::PadKind;
        let mut p = Project::new();
        let blank = |kind| {
            let src = Source::new("t", 44_100, 1, vec![0.0; 16]).unwrap();
            let mut pad = Pad::new("x", Layer::new(Region::whole(&src)));
            pad.layers.clear();
            pad.kind = kind;
            pad
        };
        let pads = [
            PadKind::MidiOut {
                port: "Volca Kick".into(),
                channel: 10,
                note: 36,
            },
            PadKind::Instrument {
                engine: "fm".into(),
                note: 24,
            },
            PadKind::Instrument {
                engine: "theremin".into(),
                note: 24,
            },
        ];
        for (i, kind) in pads.into_iter().enumerate() {
            p.set_pad(i, Some(blank(kind)), None).unwrap();
        }
        let routes = pad_routes(&p);
        assert_eq!(routes.len(), PADS);
        assert_eq!(
            routes[..4],
            [
                Some(PadRoute::Midi {
                    port: "Volca Kick".into(),
                    channel: 9,
                    note: 36
                }),
                Some(PadRoute::Voice(Engine::Fm, 24)),
                None,
                None
            ]
        );
    }
}
