//! The time engine in the pad player: the transport, the sequencer
//! (`mpfree-play`'s player and recorder), take undo and a metronome click,
//! run in the audio loop.
//!
//! It is two halves that speak only through two queues ([`link`]), as
//! docs/dev/design/time-threads.md plans. The audio half
//! ([`audio::TimeAudio`]) has the transport, the player and the recorder,
//! the view it plays and the click; it runs once a period, allocates and
//! frees nothing. The control half ([`control::TimeControl`]) has the
//! project and everything that edits it, and compiles every view. For
//! now [`TimeHost`] runs both on the audio thread: after each call on the
//! control half the audio half takes what it sent, and after each period
//! the control half takes the period's news. The app thread moves the
//! control half to its own thread next (the plan's step 3).
//!
//! Each period the loop first runs the transport ([`TimeHost::advance`]),
//! so the arp can follow its clock ([`TimeHost::arp_clock`]), hands
//! [`TimeHost::pad`] the pad notes it plays (after the arp, with their
//! offsets), then calls [`TimeHost::period`]:
//! the transport runs, the clips play their notes on the kit's pads through
//! [`Pads`] (the sampler), the armed track records, and recorded and erased
//! notes go into the clips and the take history. After the sampler renders,
//! [`TimeHost::mix_click`] adds the click while counting in and recording.
//!
//! One arp per track ([`TrackArps`], which the app owns with the arp
//! panel) sits between the player and the pads when the loop calls
//! [`TimeHost::period_arps`]. A track with "Clips through the arp" on
//! (`Track::through_arp`) plays its clips through its arp, and records the
//! pads as pressed ([`TimeHost::pad_pressed`]); with it off, the clips play
//! as recorded and the track records what the arp played
//! ([`TimeHost::pad`]).
//!
//! Pad pressure comes in through [`TimeHost::pad_pressure`]: while the
//! armed track records and records pressure (`Track::record_touch`, the
//! Record list's "Pressure" row), it is thinned onto the pads' notes
//! and kept in the clip with them; clips play it back to the pads
//! ([`Pads::pressure_at`]) as the raw value, which each instrument's
//! pressure curve shapes.
//!
//! Perform FX gestures (the XY pad, the DJ knob, a pad's Hold) come in
//! through [`TimeHost::gesture`]: while recording they become moves in the
//! armed track's clip (FX moves, played back as lane values), and the
//! move log keeps them always, so CAPTURE keeps the moves made over its
//! phrase too.
//!
//! Every pad note, and its pressure, also goes into the capture history,
//! so CAPTURE ([`TimeAction::Capture`]: the Arrange page's Capture button)
//! keeps the last phrase in the armed track even when nothing was
//! recording, as an undoable take (`mpfree-capture`). A track that leaves
//! pressure out (`Track::record_touch` off) keeps the notes without it.
//!
//! Edits (launches, takes, arming, a capture) compile a new play view on
//! the control half, which sends it boxed; the audio half swaps it in and
//! sends the old one back to be freed. Recording is not an edit of that
//! kind: each recorded or erased note, lock or gesture is a numbered
//! change (`mpfree_play::Rec`) the audio half makes to the view it plays
//! in place, in room set aside when the view was compiled
//! ([`RECORD_ROOM`]), and sends on; the control half makes it to the clip,
//! each deciding alike, so a recording period neither compiles nor
//! allocates. Each view says how many changes and how many started cues it
//! includes (`PlayView::set_includes`); one compiled before the newest
//! changes gets them before it plays (`mpfree_play::Recent`). The screens
//! hear about recorded notes at most four times a second
//! ([`TimeHost::take_changed`]). Every track plays the current kit.

mod adding;
mod apart;
mod audio;
mod audio_wait;
mod compose;
mod control;
mod follow;
pub mod link;
mod offline;
mod pad_wait;
pub mod strips;
mod timing;

use mpfree_play::mpfree_param::{Owner, ParamAddr, RelAddr};
use mpfree_play::room::{self, Table};
use mpfree_play::{Event, Out, TrackArps};
use mpfree_seq::mpfree_moves::Input;
use mpfree_seq::steps::{Change, Step};
use mpfree_seq::{Clip, Project, Track};
use mpfree_time_ui::view::{TimeAction, TimeState, TransportInfo};
use mpfree_transport::time::Tick;
use mpfree_transport::{Block, Command, Transport};
use std::f32::consts::TAU;

pub use apart::Say;
use apart::{Apart, Told};
pub use audio::TimeAudio;
pub use audio_wait::{AudioNeed, CueKey};
pub use control::EXACT;
pub use control::{edits_steps, MuteLight, PadTap, SongAudio, TimeControl};
pub use follow::Follow;
use link::Tracks;
pub use mpfree_play::AheadCounts;
pub use pad_wait::{DuePads, PadFrom, PadWait, MAX_WAITS};
pub use timing::{Allowed, Answer, TimingChange};

/// Each project audio track's regions as the spans of song ticks they
/// cover, in order, by the track's lasting id (phase 2, step 2d).
pub type AudioSpans = Vec<(mpfree_seq::TrackId, Vec<(Tick, Tick)>)>;

/// Rec song's song edit that put blocks of audio clips in: its number,
/// and each block's track and song ticks (phase 2, step 2d).
pub type SongPlaced = (u32, Vec<(mpfree_seq::TrackId, Tick, Tick)>);

/// Tracks in a new project.
pub const TRACKS: usize = 8;

/// A new project's clips: [`TRACKS`] empty tracks, an empty song, and the
/// song list linked to the song (the Arrange refactor's 4b.4: the list is
/// the song's parts). [`TimeHost::new`], for tests, leaves it unlinked.
pub fn new_project() -> Project {
    let mut project = Project::new();
    for i in 0..TRACKS {
        project
            .tracks
            .push(Track::new(&format!("Track {}", i + 1), i as u16));
    }
    project.arrangement.list.linked = true;
    project
}
/// Bars in a clip made by tapping an empty slot (without a loop set).
pub const NEW_CLIP_BARS: i64 = 2;
/// Notes, and locks, the armed track's clips can take while recording
/// before the view is compiled again (which allocates): room set aside
/// in the clip and in the play view each time it compiles.
pub const RECORD_ROOM: usize = 1024;
/// The other tracks a take can record besides the armed one (SHIFT + a
/// track's row in the REC chip's target list): 16 armed tracks in all,
/// each with a recorder of its own.
pub const MAX_ALSO: usize = 15;
/// The Play page's steps: sixteenths.
pub const STEP_TICKS: Tick = 240;
/// The most pads a pen (STEP SEQ's note or chord on a melodic track)
/// holds.
pub const MAX_PEN: usize = 8;
/// The row of the whole line: every sound of a step as one (a melodic
/// track's notes and chords). `pad` in a [`StepAction`] may be it.
pub use mpfree_seq::steps::LINE;
/// Pad hits kept for the screen between two periods (more are dropped).
const MAX_HITS: usize = 16;

/// What the Play page's pattern shows: the selected track's clip (the one
/// picked to edit, else the one playing, else its first), each sound's
/// steps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepsData {
    pub track: usize,
    pub track_name: String,
    pub clip: Option<StepClip>,
    /// The clip was picked to edit apart from what plays (the Sequencer's
    /// clip chip, refactor C2).
    pub picked: bool,
    /// The track's clip that plays, when it isn't the one shown: its
    /// name.
    pub playing: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepClip {
    pub slot: usize,
    pub name: String,
    /// Steps in the clip (16 a bar at its first step size).
    pub steps: usize,
    /// A step, in the clip's ticks: its step size (refactor D1).
    pub step: Tick,
    /// The clip's length and a step's, in song ticks (after its speed).
    pub song_length: f64,
    pub song_step: f64,
    /// Where it was launched, while it plays (for the playhead).
    pub origin: Option<Tick>,
    /// The order its play direction plays its steps in, while it plays
    /// with one (sequencer S6): the playhead lights the step heard.
    pub order: Option<mpfree_play::StepOrder>,
    /// Every pad (0 to 15), and any other note the clip holds, with its
    /// steps (None: empty); and [`LINE`], the whole line, each step's
    /// notes as one.
    pub rows: Vec<(u8, Vec<Option<Step>>)>,
    /// The same rows' locks: how many each step holds (its row's own and
    /// the whole step's), for the step pads' lock colour.
    pub locks: Vec<(u8, Vec<usize>)>,
    /// Each row's own locks: row, step, parameter and value, for the
    /// Q-Links' cards of a held step.
    pub lock_values: Vec<(u8, usize, RelAddr, f32)>,
    /// Its instrument locks made for another instrument (step sequencer
    /// refactor F1, `mpfree_seq::inst`): kept, not played. None while
    /// they play.
    pub silent: Option<SilentLocks>,
}

/// A clip's instrument locks that don't play on its track's instrument
/// now (step sequencer refactor F1): what they were made for, which
/// controls, and what the track plays now.
#[derive(Clone, Debug, PartialEq)]
pub struct SilentLocks {
    pub made_for: mpfree_seq::inst::Inst,
    /// The controls (on `made_for`) whose locks don't play.
    pub params: Vec<u16>,
    /// The name of the instrument the track plays now ("FM").
    pub now: String,
}

impl SilentLocks {
    /// What a silent lane on control `id` says: "made for Poly's Cutoff,
    /// not playing on FM".
    pub fn line(&self, id: u16) -> String {
        format!(
            "made for {}, not playing on {}",
            self.made_for.control(id),
            self.now
        )
    }
}

/// An instrument a track plays, as the app knows it (step sequencer
/// refactor F1): its node's info and controls, both static.
pub type InstRef = (
    &'static mpfree_node::NodeInfo,
    &'static [mpfree_node::ParamInfo],
);

impl StepClip {
    /// The steps of `pitch` (empty when the clip has no such row).
    pub fn row(&self, pitch: u8) -> &[Option<Step>] {
        self.rows
            .iter()
            .find(|(p, _)| *p == pitch)
            .map_or(&[], |(_, r)| r.as_slice())
    }

    /// The lock of `addr` on step `step` of row `pitch`, if it has one.
    pub fn lock_value(&self, pitch: u8, step: usize, addr: RelAddr) -> Option<f32> {
        self.lock_values
            .iter()
            .find(|&&(p, i, a, _)| (p, i, a) == (pitch, step, addr))
            .map(|l| l.3)
    }

    /// How many locks each step of `pitch` holds (empty with no such row).
    pub fn locks(&self, pitch: u8) -> &[usize] {
        self.locks
            .iter()
            .find(|(p, _)| *p == pitch)
            .map_or(&[], |(_, r)| r.as_slice())
    }

    /// The step playing at song position `pos`, while the clip plays: with
    /// a play direction, the step its order plays there, from its line to
    /// the next.
    pub fn playing(&self, pos: Tick) -> Option<usize> {
        let origin = self.origin?;
        if self.song_length <= 0.0 || self.song_step <= 0.0 || pos < origin {
            return None;
        }
        if let Some(order) = self.order {
            // A play direction moves by 1/16s whatever the step size
            // (`DIRECTION_STEP`): the step lit is the one holding the
            // 1/16 heard.
            let dir = mpfree_seq::DIRECTION_STEP;
            let song_dir = self.song_step * dir as f64 / self.step.max(1) as f64;
            let big = ((pos - origin) as f64 / song_dir).floor() as i64;
            let tick = order.step(big) as Tick * dir;
            return Some(((tick / self.step.max(1)) as usize).min(self.steps.saturating_sub(1)));
        }
        let at = ((pos - origin) as f64).rem_euclid(self.song_length);
        Some(((at / self.song_step) as usize).min(self.steps.saturating_sub(1)))
    }
}

/// A touch on the Play page's pattern.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepAction {
    /// Tap step `step` of `pad`: on (at `velocity`) or off. With no clip,
    /// a one-bar clip is made in the armed track's first empty slot and
    /// launched.
    Tap { pad: u8, step: usize, velocity: u8 },
    /// A Q-Link on a step: velocity, length, nudge or chance.
    Change {
        pad: u8,
        step: usize,
        change: Change,
    },
    /// Q-Links turned while several step pads are held (STEP SEQ): the
    /// steps of `pad` whose bits are set in `steps` (step 0 the lowest)
    /// all take `change`, and a whole turn is one undo.
    ChangeSteps {
        pad: u8,
        steps: u128,
        change: Change,
    },
    /// A knob turned while step pads are held (STEP SEQ): the Q-Links'
    /// Sound bank, or a knob on the Synth tab. The steps of `pad` whose
    /// bits are set in `steps` take a lock of `addr` at `value` (a step
    /// with no note: a lock with no note), or with `None` lose it. A whole
    /// turn is one undo.
    Lock {
        pad: u8,
        steps: u128,
        addr: RelAddr,
        value: Option<f32>,
    },
    /// ERASE pressed while step pads are held: the steps of `pad` whose
    /// bits are set in `steps` lose their locks; their notes stay.
    ClearLocks { pad: u8, steps: u128 },
    /// ERASE + a lane's name: every step of `pad` comes out of the clip.
    ClearPad { pad: u8 },
    /// ERASE + a step pad (one step) or + a PAD BANK (a page), with STEP
    /// SEQ on: steps `from` up to `to` of `pad` are cleared, locks too.
    ClearSteps { pad: u8, from: usize, to: usize },
    /// COPY with STEP SEQ on: `count` steps of `pad` from step `from` go
    /// over the steps from `to`, locks too (a step, or a page of 16).
    CopySteps {
        pad: u8,
        from: usize,
        to: usize,
        count: usize,
    },
    /// STEP SEQ on a melodic track (the synth on the pads): the pen, the
    /// first `n` of `keys` (pad, velocity), goes on step `step` of the
    /// whole line in place of what it held (`Grid::enter`); `n` 0 clears
    /// the step. With no clip, one is made and launched, as `Tap` does.
    Enter {
        step: usize,
        keys: [(u8, u8); MAX_PEN],
        n: u8,
    },
    /// Keys of a keyboard played while step pads of the line are held:
    /// the first `n` of `keys` (pad, velocity) go on each of `steps` (a bit
    /// a step, step 0 the lowest) as `Enter` does. The keys of one chord,
    /// played one after another, join one undo (`steps::enter_key`).
    Keys {
        steps: u128,
        keys: [(u8, u8); MAX_PEN],
        n: u8,
    },
    /// The pattern's - / + on Play (Jay after live test 3: 32, 64 and 128
    /// steps): make the clip `steps` steps long. With `copy`, new steps
    /// repeat the ones before them; steps that go take their notes along.
    /// One take, so UNDO puts the old length and notes back.
    Length { steps: usize, copy: bool },
    /// The length a step at a time (refactor D2: the head's length tapped,
    /// then the wheel or - and +): `steps` more (or fewer, negative), from
    /// the clip's own length, up to [`MAX_STEPS`] and down to one step.
    /// New steps are empty; steps that go take their notes along. One take
    /// each, so UNDO puts the old length and notes back.
    LengthBy { steps: i32 },
    /// A Velocity or Chance lane's stroke under sound `pad` (refactor
    /// D3): step `from + k` takes `values[k]`, 0 leaving it as it is. One
    /// edit, so one UNDO takes the whole stroke back. Empty steps are left
    /// out.
    Values {
        pad: u8,
        from: usize,
        kind: NoteValue,
        values: [u8; 16],
    },
}

/// A note setting a lane under a sound shows and sets, step by step (the
/// Sequencer's Velocity and Chance lanes, refactor D3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NoteValue {
    Velocity,
    Chance,
}

impl NoteValue {
    pub const ALL: [NoteValue; 2] = [NoteValue::Velocity, NoteValue::Chance];

    pub fn name(self) -> &'static str {
        match self {
            NoteValue::Velocity => "Velocity",
            NoteValue::Chance => "Chance",
        }
    }

    /// Its least and most.
    pub fn range(self) -> (u8, u8) {
        match self {
            NoteValue::Velocity => (1, 127),
            NoteValue::Chance => (1, 100),
        }
    }

    /// A step's value.
    pub fn of(self, s: &Step) -> u8 {
        match self {
            NoteValue::Velocity => s.velocity,
            NoteValue::Chance => s.chance,
        }
    }

    /// The change that sets it to `v`.
    pub fn change(self, v: u8) -> Change {
        match self {
            NoteValue::Velocity => Change::Velocity(v),
            NoteValue::Chance => Change::Chance(v),
        }
    }

    /// `v` as the lane writes it: "96", "50 %".
    pub fn text(self, v: u8) -> String {
        match self {
            NoteValue::Velocity => v.to_string(),
            NoteValue::Chance => format!("{v} %"),
        }
    }
}

/// The most steps the Sequencer's head makes a pattern, doubling or a
/// step at a time (Jay after live test 3: up to 128).
pub const MAX_STEPS: usize = 128;

/// A pad hit while the transport rolls: where it fell, for the Play page's
/// timing (the Beats view's timing strip).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HitInfo {
    pub pad: u8,
    pub velocity: u8,
    /// Song position, ticks, with the fraction.
    pub tick: f64,
    pub bpm: f64,
    pub bar_ticks: Tick,
}

/// Most notes one live note records as: a pad of chords' notes
/// (`mpfree_arp::scale::pads::MAX_PAD_NOTES`).
pub const HEARD_NOTES: usize = 4;

/// What the armed track records of one live note ([`Pads::live`]): its pad,
/// which a take of pad hits records and CAPTURE's history and the Play
/// page's timing keep, and its notes as written (before the track's live
/// transpose), which a take of notes records (the instrument redesign's
/// step 5c). A pad of chords gives several notes; a note no pad plays (an
/// arp's octave up) gives no pad. Fixed size, made on the audio thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heard {
    pub pad: Option<Event>,
    notes: [Event; HEARD_NOTES],
    n: u8,
}

impl Heard {
    /// Nothing records.
    pub const NONE: Heard = Heard {
        pad: None,
        notes: [Event::AllOff; HEARD_NOTES],
        n: 0,
    };

    /// The trigger group its notes record in: its pad's
    /// ([`mpfree_seq::groups::pad_group`]), so a pad of chords records one
    /// trigger with one chance; 0 (each note on its own) for notes no pad
    /// plays.
    pub fn group(&self) -> u8 {
        match self.pad {
            Some(
                Event::NoteOn { note, .. } | Event::NoteOff { note } | Event::Pressure { note, .. },
            ) => mpfree_seq::groups::pad_group(note),
            _ => 0,
        }
    }

    /// A host with no instruments: the pad, and the same number as its
    /// note.
    pub fn pad(event: Event) -> Heard {
        Heard::of(Some(event), event, &[note_of(event)])
    }

    /// `kind` (a note-on with its velocity, a note-off, a pressure) on each
    /// of `notes`, recorded as `pad` in a take of pad hits.
    pub fn of(pad: Option<Event>, kind: Event, notes: &[u8]) -> Heard {
        let mut h = Heard { pad, ..Heard::NONE };
        for &note in notes.iter().take(HEARD_NOTES) {
            h.notes[usize::from(h.n)] = match kind {
                Event::NoteOn { velocity, .. } => Event::NoteOn { note, velocity },
                Event::NoteOff { .. } => Event::NoteOff { note },
                Event::Pressure { value, .. } => Event::Pressure { note, value },
                e => e,
            };
            h.n += 1;
        }
        h
    }

    /// The notes a take of notes records, in order.
    pub fn notes(&self) -> &[Event] {
        &self.notes[..usize::from(self.n)]
    }
}

/// The note number of a note's start, end or pressure (0 for the rest).
fn note_of(e: Event) -> u8 {
    match e {
        Event::NoteOn { note, .. } | Event::NoteOff { note } | Event::Pressure { note, .. } => note,
        _ => 0,
    }
}

/// The notes each of a track's 16 pads plays (`mpfree_play::PadRows`):
/// the step grid shows and edits a clip of notes through them, and a
/// track's arp that works in notes plays a clip of pad hits by them.
pub use mpfree_play::PadRows;

const _: () = assert!(HEARD_NOTES == mpfree_play::PAD_NOTES);

/// Where the sequencer's notes go: the sampler's pads. Each note names the
/// track it came from (`Out::track`), so a host can play each track's own
/// kit or instrument.
pub trait Pads {
    fn hit_at(&mut self, track: u16, pad: usize, velocity: u8, offset: u32) -> bool;
    fn release_at(&mut self, track: u16, pad: usize, offset: u32);
    fn stop_all(&mut self);
    /// A clip's recorded pressure on `pad`'s sounding note, the raw value
    /// 0 to 127, `offset` frames into the period.
    fn pressure_at(&mut self, track: u16, pad: usize, value: u8, offset: u32);
    /// A track that plays somewhere other than the pads (one on MIDI
    /// OUT) is offered each event the clips play first: notes, a panic's
    /// `AllOff`, pressure, and the locks and lanes. True when it took the
    /// event, so neither the pads nor the mixer's mod engine get it.
    /// False leaves it to go on as usual (an `AllOff` should, so the pads
    /// stop too). The arps' notes, the live pads among them, go to
    /// [`Pads::live`] instead, which sees the whole event and says what
    /// records. A clip of notes' notes ([`Out::notes`], instrument
    /// redesign step 5c) come here too: a host with instruments takes
    /// them, as the pad methods would read each note as a pad. By
    /// default nothing is taken, and they play as pads.
    fn out(&mut self, o: &Out) -> bool {
        let _ = o;
        false
    }
    /// An event of a track that plays ahead (sequencer S5, a track on a
    /// MIDI port whose gear needs more than the sound's lead), handed over
    /// before its period: `o.offset` counts from this period's first
    /// frame and may be past its end. Only MIDI out takes it; true when it
    /// did. By default nothing is. Also a lock, a lane's value or their
    /// end on the track's own MIDI out control (`mpfree_play::is_gear_control`)
    /// that the forecast missed, in its own period, or that went ahead and
    /// wasn't played (its unlock): false leaves it to the mod engine.
    fn out_ahead(&mut self, o: &Out) -> bool {
        let _ = o;
        false
    }
    /// An event the real pass played that went out ahead already
    /// ([`Pads::out_ahead`]): what [`Pads::out`] does besides MIDI out
    /// (CV out, the monitor, the pads' flashes) happens now.
    fn out_sent(&mut self, o: &Out) {
        let _ = o;
    }
    /// A clip's parameter lock (`Some`) or its end (`None`) on a kit
    /// pad's sound (`Owner::Pad`) or an instrument (`Owner::Inst`),
    /// `offset` frames into the period, before the note it plays with.
    /// True when it was taken; otherwise it goes on to the mixer's mod
    /// engine, which plays the strips' and inserts' locks and counts the
    /// rest. By default nothing is taken.
    fn lock_at(
        &mut self,
        addr: mpfree_play::mpfree_param::ParamAddr,
        value: Option<f32>,
        offset: u32,
    ) -> bool {
        let _ = (addr, value, offset);
        false
    }
    /// The mixer track (its number among the mixer's tracks) where track
    /// `track`'s instrument sounds, for a clip's locks on "this track's
    /// strip" ([`strips`]): another track's instrument it plays (step 4a)
    /// is heard on that instrument's strip. None where the host doesn't
    /// say (a kit, the synth, MIDI out): its own strip. By default None.
    fn sounds_on(&self, track: u16) -> Option<u8> {
        let _ = track;
        None
    }
    /// A note one of the per-track arps played ([`TimeHost::period_arps`]):
    /// the pads through their track's arp, a latched arp, or a clip that
    /// plays through its arp. It is live playing, so the pad player gives
    /// it the pads' own treatment (velocity curve, flashes). Returns what
    /// the armed track records of it as heard: its pad, and its notes for
    /// a take of notes ([`Heard`]). By default it plays as a clip's note
    /// and records as it is.
    fn live(&mut self, o: &Out) -> Heard {
        match o.event {
            Event::NoteOn { note, velocity } => {
                self.hit_at(o.track, usize::from(note), velocity, o.offset);
                Heard::pad(o.event)
            }
            Event::NoteOff { note } => {
                self.release_at(o.track, usize::from(note), o.offset);
                Heard::pad(o.event)
            }
            Event::AllOff => {
                self.stop_all();
                Heard::NONE
            }
            Event::Pressure { note, value } => {
                self.pressure_at(o.track, usize::from(note), value, o.offset);
                Heard::NONE
            }
            Event::Lock { .. }
            | Event::Lane { .. }
            | Event::Unlock { .. }
            | Event::Program { .. } => Heard::NONE,
        }
    }
}

impl Pads for crate::sampler::Player {
    fn hit_at(&mut self, _track: u16, pad: usize, velocity: u8, offset: u32) -> bool {
        crate::sampler::Player::hit_at(self, pad, velocity, offset)
    }
    fn release_at(&mut self, _track: u16, pad: usize, offset: u32) {
        crate::sampler::Player::release_at(self, pad, offset)
    }
    fn stop_all(&mut self) {
        crate::sampler::Player::stop_all(self)
    }
    fn pressure_at(&mut self, _track: u16, pad: usize, value: u8, offset: u32) {
        crate::sampler::Player::pressure_at(self, pad, value, offset)
    }
    fn lock_at(
        &mut self,
        addr: mpfree_play::mpfree_param::ParamAddr,
        value: Option<f32>,
        offset: u32,
    ) -> bool {
        match (addr.owner, crate::sampler::PadParam::of(addr.param)) {
            (mpfree_play::mpfree_param::Owner::Pad(_, pad), Some(p)) => {
                self.pad_param_lock(self.kit(), usize::from(pad), p, value, offset)
            }
            _ => false,
        }
    }
}

/// The metronome: a short sine blip on each beat, higher on the bar.
#[derive(Clone, Copy, Debug)]
struct Click {
    rate: f32,
    /// Starts in this period: (offset, accent).
    starts: [(u32, bool); 8],
    n: usize,
    phase: f32,
    freq: f32,
    env: f32,
    /// The click's level, as a gain.
    pub gain: f32,
}

/// The click's level by default (-24 dBFS; it was -12 until live test 3,
/// "far too loud") and decay (a quarter of its level after 15 ms). Play's
/// options set the level (`Settings::metronome_db`).
pub const CLICK_DB: f32 = -24.0;
const CLICK_MS: f32 = 15.0;

/// A level in dB as a gain.
pub fn click_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

impl Click {
    fn new(rate: u32) -> Click {
        Click {
            rate: rate as f32,
            starts: [(0, false); 8],
            n: 0,
            phase: 0.0,
            freq: 0.0,
            env: 0.0,
            gain: click_gain(CLICK_DB),
        }
    }

    fn schedule(&mut self, offset: u32, accent: bool) {
        if self.n < self.starts.len() {
            self.starts[self.n] = (offset, accent);
            self.n += 1;
        }
    }

    /// Add the click to `left` and `right` (keeping them within ±1);
    /// returns the peak afterwards.
    fn mix(&mut self, left: &mut [f32], right: &mut [f32]) -> f32 {
        let decay = 0.25f32.powf(1.0 / (CLICK_MS * 1e-3 * self.rate));
        let mut peak = 0f32;
        let mut next = 0;
        for f in 0..left.len().min(right.len()) {
            while next < self.n && self.starts[next].0 as usize == f {
                self.freq = if self.starts[next].1 { 1760.0 } else { 1320.0 };
                self.phase = 0.0;
                self.env = 1.0;
                next += 1;
            }
            if self.env > 1e-4 {
                let s = self.gain * self.env * (self.phase * TAU).sin();
                self.phase = (self.phase + self.freq / self.rate).fract();
                self.env *= decay;
                left[f] = (left[f] + s).clamp(-1.0, 1.0);
                right[f] = (right[f] + s).clamp(-1.0, 1.0);
            }
            peak = peak.max(left[f].abs()).max(right[f].abs());
        }
        self.n = 0;
        peak
    }
}

/// The time engine's two halves on one thread, for the pad player's audio
/// loop until the control half has its own (see the module notes). Each
/// call goes to the half that owns it; a call on the control half is
/// followed by the audio half taking what it sent, and a period by the
/// control half taking the period's news, so the two never differ for
/// long enough to show.
pub struct TimeHost {
    control: Half,
    audio: TimeAudio,
    /// Click while counting in and recording (off by default since live
    /// test 3; Play's options and SHIFT + PLAY START turn it on).
    pub metronome: bool,
    /// The click's level, as a gain ([`click_gain`]).
    pub click_gain: f32,
    /// The device the transport follows, for what is said while it waits.
    leader: Option<String>,
    /// Each track's Quantise grid before SHIFT + TC turned it off (0:
    /// none yet), so SHIFT + TC turns the same one back on.
    quantise_was: Vec<u32>,
    /// Where PLAY (bar 1 for PLAY START) last started the transport from
    /// stopped, for SHIFT + PLAY to play from again. None until then.
    play_from: Option<Tick>,
    /// The count-in's bars before SHIFT + OVERDUB turned it off (0: none
    /// yet), so SHIFT + OVERDUB gives the same back.
    count_in_was: u8,
}

/// Where the control half runs: here, on the facade's thread with the
/// audio half (together, the default), or on a thread of its own (apart,
/// `--time-thread`; Arrange 2.6b, [`apart`]).
enum Half {
    Here(Box<TimeControl>),
    Apart(Box<Apart>),
}

/// Tests reach into the control half together; apart it is on its thread.
#[cfg(test)]
impl std::ops::Deref for Half {
    type Target = TimeControl;
    fn deref(&self) -> &TimeControl {
        match self {
            Half::Here(c) => c,
            Half::Apart(_) => panic!("the control half runs on its own thread"),
        }
    }
}

#[cfg(test)]
impl std::ops::DerefMut for Half {
    fn deref_mut(&mut self) -> &mut TimeControl {
        match self {
            Half::Here(c) => c,
            Half::Apart(_) => panic!("the control half runs on its own thread"),
        }
    }
}

/// A step's parameter lock (`value`) or its end (`None`), `offset` frames
/// into the period.
pub type LockOut = (u32, mpfree_play::mpfree_param::ParamAddr, Option<f32>);

/// A move lane's value, `offset` frames into the period (a lane's end
/// comes as a [`LockOut`] unlock).
pub type LaneOut = (u32, mpfree_play::mpfree_param::ParamAddr, f32);

/// The grid SHIFT + TC turns Quantise on at the first time: 1/16.
pub const QUANTISE_ON: u32 = 240;

/// Locks and unlocks kept for the mixer in one period: every lock and
/// unlock the period's table lets in (refactor A2: an unlock is never
/// lost; mpfree_play::room), and room again for the ends a lock on "this
/// track's strip" sends on the old strip when its instrument moved
/// ([`strips`]: one at most for each lock held, fewer than
/// `room::LOCK_ENDS`).
const MAX_LOCKS: usize = room::VALUES + 2 * room::LOCK_ENDS;
/// Move values kept for the mod engine in one period: every value the
/// period's table lets in.
const MAX_LANES: usize = room::VALUES;

impl TimeHost {
    /// Stopped at bar 1 with an empty project of [`TRACKS`] tracks, track 1
    /// armed.
    pub fn new(rate: u32, bpm: f64) -> Result<TimeHost, String> {
        let mut project = Project::new();
        for i in 0..TRACKS {
            project
                .tracks
                .push(Track::new(&format!("Track {}", i + 1), i as u16));
        }
        TimeHost::with_project(rate, bpm, project)
    }

    /// As the app starts: a new project ([`new_project`]).
    pub fn new_project(rate: u32, bpm: f64) -> Result<TimeHost, String> {
        TimeHost::with_project(rate, bpm, new_project())
    }

    /// Stopped at bar 1 with `project` (an opened one), track 1 armed when
    /// it has a track. Builds and compiles everything here (allocates):
    /// the app makes it on the screen thread and swaps it in.
    pub fn with_project(rate: u32, bpm: f64, mut project: Project) -> Result<TimeHost, String> {
        // Every track and every clip has its lasting id from here on (a
        // new project's, or one saved before they had ids, gets 1, 2, 3
        // ...).
        project.give_track_ids();
        project.give_clip_ids();
        project
            .check()
            .map_err(|e| format!("the clips don't hold together: {e:?}"))?;
        let transport =
            Box::new(Transport::new(rate, bpm).map_err(|e| format!("transport: {e:?}"))?);
        let armed = (!project.tracks.is_empty()).then_some(0);
        let tracks = Tracks::of(&project, armed);
        let info = TransportInfo::of(&transport);
        let (to_audio, from_control) = link::link(link::TO_AUDIO);
        let (to_control, from_audio) = link::link(link::TO_CONTROL);
        let audio = TimeAudio::new(transport, rate, tracks, to_control, from_control);
        let control = TimeControl::new(project, info, rate, to_audio, from_audio, tracks)?;
        let mut host = TimeHost {
            control: Half::Here(Box::new(control)),
            audio,
            metronome: false,
            click_gain: click_gain(CLICK_DB),
            leader: None,
            quantise_was: vec![0; mpfree_seq::MAX_TRACKS],
            play_from: None,
            count_in_was: 0,
        };
        host.audio.receive();
        Ok(host)
    }

    /// The two halves, linked, for a host that runs the control half on
    /// its own thread (the plan's step 3). The audio loop keeps the
    /// [`TimeAudio`] and calls it as this facade does: the pads' input,
    /// [`TimeAudio::advance`], then [`TimeAudio::run`] (after
    /// [`TimeAudio::dress_arps`] with the per-track arps), the hits, locks
    /// and click. The control thread keeps the [`TimeControl`], calls
    /// [`TimeControl::poll`] every few ms and after each screen or MIDI
    /// event, and makes the edits.
    ///
    /// The queue to the audio half holds [`link::TO_AUDIO`] messages and
    /// the one back [`link::TO_CONTROL`] (a busy second of periods). Neither half
    /// ever waits: a full queue hands the message back to its sender,
    /// which counts it ([`link::Counts::lost`] in [`TimeAudio::counts`] for the
    /// audio half's, [`TimeControl::lost`] for the control half's; a view
    /// waits and is sent again). [`TimeControl::describe`] puts every
    /// count in the status line once one is not 0.
    /// Together only (apart, the control half is on its thread already).
    #[allow(dead_code)] // The tests' threads take it.
    pub fn split(self) -> (TimeControl, TimeAudio) {
        let mut audio = self.audio;
        audio.metronome = self.metronome;
        audio.set_click_gain(self.click_gain);
        match self.control {
            Half::Here(c) => (*c, audio),
            Half::Apart(_) => panic!("split: the control half runs on its own thread"),
        }
    }

    /// The control half onto a thread of its own (apart, the app's
    /// `--time-thread`; Arrange 2.6b, [`apart`]). The host comes back
    /// apart, or together with why the thread didn't start.
    pub fn into_apart(self) -> (TimeHost, Option<String>) {
        let TimeHost {
            control,
            audio,
            metronome,
            click_gain,
            leader,
            quantise_was,
            play_from,
            count_in_was,
        } = self;
        let (control, why) = match control {
            Half::Here(c) => match Apart::start(c) {
                Ok(a) => (Half::Apart(Box::new(a)), None),
                Err((c, why)) => (Half::Here(c), Some(why)),
            },
            apart => (apart, None),
        };
        let host = TimeHost {
            control,
            audio,
            metronome,
            click_gain,
            leader,
            quantise_was,
            play_from,
            count_in_was,
        };
        (host, why)
    }

    /// The control half back on this thread (together), after what was
    /// asked of it so far. None if its thread was lost.
    #[allow(dead_code)] // Tests; a later step may switch modes while running.
    pub fn into_together(self) -> Option<TimeHost> {
        let TimeHost {
            control,
            mut audio,
            metronome,
            click_gain,
            leader,
            quantise_was,
            play_from,
            count_in_was,
        } = self;
        let control = match control {
            Half::Apart(a) => Half::Here(a.stop()?),
            here => here,
        };
        audio.receive();
        Some(TimeHost {
            control,
            audio,
            metronome,
            click_gain,
            leader,
            quantise_was,
            play_from,
            count_in_was,
        })
    }

    /// The control half runs on its own thread.
    pub fn apart(&self) -> bool {
        matches!(self.control, Half::Apart(_))
    }

    /// `f` on the control half: here, or apart borrowed back from its
    /// thread for the call ([`Apart::lend`], counted): the same answer
    /// either way.
    fn with<R>(&mut self, f: impl FnOnce(&mut TimeControl) -> R) -> R {
        match &mut self.control {
            Half::Here(c) => f(c),
            Half::Apart(a) => a.lend(f),
        }
    }

    /// The asks' answers since the last call, with how the caller wants
    /// each said (apart; together each call answers at once).
    pub fn take_said(&mut self) -> Vec<(Say, Result<String, String>)> {
        match &mut self.control {
            Half::Here(_) => Vec::new(),
            Half::Apart(a) => a.take_said(),
        }
    }

    /// The audio half takes what the control half sent, and passes `r` on.
    fn sent<T>(&mut self, r: T) -> T {
        self.audio.receive();
        r
    }

    /// The armed track (the one REC records), if any.
    pub fn armed(&self) -> Option<usize> {
        match &self.control {
            Half::Here(c) => c.armed(),
            Half::Apart(a) => a.light.armed,
        }
    }

    /// The selected track (the one the Sequencer edits, and the pads and
    /// the arp panel follow): the armed track unless a track was picked
    /// while REC's arm is pinned (refactor C1).
    pub fn selected(&self) -> Option<usize> {
        match &self.control {
            Half::Here(c) => c.selected(),
            Half::Apart(a) => a.light.selected,
        }
    }

    /// SHIFT + TC (the MPC's TC On/Off): the armed track's Quantise while
    /// recording turns off, or back on at the grid it had (1/16 the first
    /// time), as the Record tab's Quantise row sets it. What to say.
    pub fn toggle_record_quantise(&mut self) -> Result<String, String> {
        let t = self.armed().ok_or("pick a track first")?;
        let track = self
            .project()
            .tracks
            .get(t)
            .ok_or_else(|| format!("no track {}", t + 1))?;
        let name = track.name.clone();
        let grid = match track.record_quantise {
            Some(g) => {
                if let Some(w) = self.quantise_was.get_mut(t) {
                    *w = g;
                }
                None
            }
            None => Some(
                self.quantise_was
                    .get(t)
                    .copied()
                    .filter(|&g| g > 0)
                    .unwrap_or(QUANTISE_ON),
            ),
        };
        self.act(TimeAction::RecordQuantise { track: t, grid })?;
        Ok(match grid {
            Some(g) => format!(
                "Quantise on: {name} records each note on the nearest {}   ·   SHIFT + TC: off",
                mpfree_time_ui::view::grid_name(g)
            ),
            None => {
                format!("Quantise off: {name} records the notes as played   ·   SHIFT + TC: on")
            }
        })
    }

    /// SHIFT + PLAY (the MPC's play from the last start): the transport
    /// plays from where PLAY last started it; while playing, the playhead
    /// jumps back there. Before PLAY has started anything, it plays from
    /// where it is, as PLAY does. What to say.
    pub fn play_back(&mut self) -> Result<String, String> {
        let info = self.info();
        let Some(at) = self.play_from else {
            self.act(TimeAction::Transport(Command::Play))?;
            return Ok("SHIFT + PLAY: playing from here (PLAY hasn't started anything yet)".into());
        };
        let place = control::bar_beat(at, info.meter);
        if !self.command(Command::Locate(at)) {
            return Err("transport: command queue full".into());
        }
        if info.state.rolling() {
            return Ok(format!("SHIFT + PLAY: back to {place}, where PLAY started"));
        }
        if !self.command(Command::Play) {
            return Err("transport: command queue full".into());
        }
        Ok(format!(
            "SHIFT + PLAY: playing from {place}, where PLAY last started"
        ))
    }

    /// SHIFT + OVERDUB (the MPC's Count In on or off): the count-in off,
    /// or back to the bars it had (1 bar the first time). What to say.
    pub fn toggle_count_in(&mut self) -> Result<String, String> {
        let now = self.info().count_in;
        let bars = if now > 0 {
            self.count_in_was = now;
            0
        } else {
            self.count_in_was.max(1)
        };
        if !self.command(Command::SetCountIn(bars)) {
            return Err("transport: command queue full".into());
        }
        Ok(match bars {
            0 => "Count-in off: REC starts recording at once   ·   SHIFT + OVERDUB: on".into(),
            1 => "Count-in on: 1 bar before recording   ·   SHIFT + OVERDUB: off".into(),
            n => format!("Count-in on: {n} bars before recording   ·   SHIFT + OVERDUB: off"),
        })
    }

    /// The other armed tracks, a bit each, without the armed one
    /// (`TimeAction::ArmAlso`).
    pub fn also(&self) -> u128 {
        match &self.control {
            Half::Here(c) => c.also(),
            Half::Apart(a) => a.light.also,
        }
    }

    /// The Loop cell armed on the grid, while it is still empty: the next
    /// loop recorded lands there (loop-tracks.md part 4).
    pub fn loop_cell(&self) -> Option<(usize, usize)> {
        match &self.control {
            Half::Here(c) => c.loop_cell(),
            Half::Apart(a) => a.light.loop_cell,
        }
    }

    /// The note editor is open (the Sequencer tab shows its Notes view).
    pub fn editing(&self) -> bool {
        match &self.control {
            Half::Here(c) => c.editing(),
            Half::Apart(a) => a.light.editing,
        }
    }

    /// The note editor is open with Pads write on (piano roll plan 1C.2).
    pub fn pads_write(&self) -> bool {
        match &self.control {
            Half::Here(c) => c.pads_write(),
            Half::Apart(a) => a.light.pads_write,
        }
    }

    /// The loop playing (or launched) on Loop track `t`, for OVERDUB with
    /// a cell of it armed.
    pub fn loop_on(&self, t: usize) -> Option<usize> {
        let mask = mpfree_seq::loops::on_track(self.project(), t);
        let plan = self.loop_plan();
        (0..mpfree_seq::LOOPS).find(|&n| plan.on[n] && mask & (1u8 << n) != 0)
    }

    /// Each track's live transpose (sequencer S4) as the audio half has
    /// it: where the pads become notes (`SeqPads`), its notes shift by it.
    pub fn transposes(&self) -> [mpfree_seq::Transpose; mpfree_seq::MAX_TRACKS] {
        self.audio.transposes()
    }

    /// The project the clips are in (for a save: `project.rs`). Apart, the
    /// copy published after the last change.
    pub fn project(&self) -> &Project {
        match &self.control {
            Half::Here(c) => &c.project,
            Half::Apart(a) => &a.heavy.project,
        }
    }

    /// "Clips through the arp" for `track` (see `Track::through_arp`).
    /// The track's arp follows from the next [`TimeHost::period_arps`],
    /// which clears it if it holds anything, so no note is left hanging.
    /// False if there is no such track.
    // The arp panel calls it (the arp thread's side); until then only
    // tests do.
    #[allow(dead_code)]
    pub fn set_through_arp(&mut self, track: usize, on: bool) -> bool {
        let r = self.with(|c| c.set_through_arp(track, on));
        self.sent(r)
    }

    /// Cues that launch audio wait until the start of their audio is held
    /// in memory (`on`: takes stream from their files), or go as they are
    /// (see `TimeControl::set_audio_waits`).
    pub fn set_audio_waits(&mut self, on: bool) {
        self.with(|c| c.set_audio_waits(on));
        self.sent(());
    }

    /// What the cues waiting for their audio need held (see
    /// `TimeControl::audio_needs`). Ask again when the state's
    /// `audio_waits.needs_rev` rises.
    pub fn audio_needs(&mut self) -> Vec<AudioNeed> {
        self.with(|c| c.audio_needs().to_vec())
    }

    /// The audio `key`'s cues need is held (`ok`), or not yet (see
    /// `TimeControl::audio_held`). False when nothing waits under `key`.
    pub fn audio_held(&mut self, key: CueKey, ok: bool) -> bool {
        let r = self.with(|c| c.audio_held(key, ok));
        self.sent(r)
    }

    /// The audio held for `key` was given back before its cues landed (see
    /// `TimeControl::audio_given_back`).
    pub fn audio_given_back(&mut self, key: CueKey) -> bool {
        let r = self.with(|c| c.audio_given_back(key));
        self.sent(r)
    }

    /// To a cell and Put in a cell (audio tracks phase 2): `clip` in audio
    /// track `track`'s first empty cell, as an edit UNDO takes away (see
    /// `TimeControl::put_audio_clip`). The row it went in, or why not.
    pub fn put_audio_clip(&mut self, track: usize, clip: Clip) -> Result<usize, String> {
        let r = self.with(|c| c.put_audio_clip(track, clip));
        self.sent(r)
    }

    /// Each audio track's regions as song tick spans (phase 2, step 2d):
    /// a block is never dropped, moved or made longer over one (see
    /// `TimeControl::set_audio_spans`).
    pub fn set_audio_spans(&mut self, spans: AudioSpans) {
        self.with(|c| c.set_audio_spans(spans));
    }

    /// Audio track `track` armed or disarmed by its own arm: the Clip tab
    /// follows it (see `TimeControl::focus_audio`).
    pub fn focus_audio(&mut self, track: usize, on: bool) {
        self.with(|c| c.focus_audio(track, on));
    }

    /// Put in a cell's cell (audio tracks phase 2,
    /// `mpfree_time_ui::view::pick_cell`), as (track, row); None: no audio
    /// track has an empty cell.
    pub fn cell_for_sound(&self) -> Option<(usize, usize)> {
        let cells = self.project().tracks.iter().map(|t| {
            let empty = t.slots.shown().position(|c| c.is_none());
            (t.is_audio(), empty)
        });
        mpfree_time_ui::view::pick_cell(self.armed(), cells)
    }

    /// Fit to tempo's end (audio tracks phase 2): clip `slot` of `track`,
    /// if still clip `id`, plays `audio`, as one edit UNDO takes back
    /// (see `TimeControl::fit_clip`). Its name, or why not.
    pub fn fit_clip(
        &mut self,
        track: usize,
        slot: usize,
        id: mpfree_seq::ClipId,
        audio: mpfree_seq::AudioClip,
    ) -> Result<String, String> {
        let r = self.with(|c| c.fit_clip(track, slot, id, audio));
        self.sent(r)
    }

    /// A track's MIDI effects, as the chain editor's Notes stage set them
    /// (see `TimeControl::set_note_fx`). False if there is no such note
    /// track.
    pub fn set_note_fx(&mut self, track: usize, fx: mpfree_notefx::TrackFx) -> bool {
        let r = self.with(|c| c.set_note_fx(track, fx));
        self.sent(r)
    }

    /// The mod engine's value for a MIDI effect setting (`addr` is an
    /// `Owner::NoteFx` address): a macro, an LFO or a step's lock moving it
    /// (`Some`, in the setting's own steps), or letting go (`None`). Its
    /// track's arp chain takes it from the next period; the project keeps
    /// the track's own value. Never allocates.
    pub fn modulate_note_fx(&mut self, addr: ParamAddr, value: Option<f32>) {
        let Owner::NoteFx(track, id) = addr.owner else {
            return;
        };
        let Ok(k) = u8::try_from(addr.param) else {
            return;
        };
        let value = match value {
            Some(v) if v.is_finite() => Some(v.round() as i32),
            Some(_) => return,
            None => None,
        };
        self.audio
            .modulate_note_fx(usize::from(track), id, k, value);
    }

    /// Track `track`'s MIDI effects as they play now, with the mod
    /// engine's values on them (for MIDI out's CC scale and CC smooth).
    /// Never allocates.
    pub fn note_fx_now(&self, track: usize) -> mpfree_notefx::TrackFx {
        self.audio.note_fx_now(track)
    }

    /// The tracks' MIDI effects as the sound loop has them, once after
    /// each change (for the mod engine's view of their settings).
    pub fn note_fx_moved(&mut self) -> Option<&[mpfree_notefx::TrackFx; mpfree_seq::MAX_TRACKS]> {
        self.audio
            .take_note_fx_moved()
            .then(|| self.audio.note_fx())
    }

    /// What the Play page's pattern shows. Allocates: called on a change.
    pub fn steps(&self) -> StepsData {
        match &self.control {
            Half::Here(c) => c.steps(),
            Half::Apart(a) => a.heavy.steps.clone(),
        }
    }

    /// True once after the steps the Play page shows changed.
    pub fn take_steps_changed(&mut self) -> bool {
        match &mut self.control {
            Half::Here(c) => c.take_steps_changed(),
            Half::Apart(a) => a.take_steps_changed(),
        }
    }

    /// A touch on the Play page's pattern, as one undoable edit (a
    /// Q-Link turned through many values joins one), made to the playing
    /// clip in place when it can be (`TimeControl::in_place`). A line for
    /// the log.
    pub fn step_action(&mut self, a: StepAction) -> Result<String, String> {
        let audio = &mut self.audio;
        let mut run = move |c: &mut TimeControl| {
            let at = std::time::Instant::now();
            let was = c.costs;
            let r = c.step_action(a);
            audio.receive();
            // From the action to the audio half having made it: what a
            // tap or a turn costs the audio thread while both halves run
            // on it (apart, the wait for the control half is the lend's).
            let took = at.elapsed();
            let c = &mut c.costs;
            if c.steps_in_place > was.steps_in_place {
                c.slowest_in_place = c.slowest_in_place.max(took);
            } else if c.steps_compiled > was.steps_compiled {
                c.slowest_step_compiled = c.slowest_step_compiled.max(took);
            }
            r
        };
        match &mut self.control {
            Half::Here(c) => run(c),
            Half::Apart(ap) => ap.lend(run),
        }
    }

    /// [`TimeHost::step_action`] for a tap or a turn that only says what
    /// it did, as [`TimeHost::act_then`] makes an action: together the
    /// answer at once, apart None and the answer later, so a step edit
    /// that has to compile never costs the audio thread.
    pub fn step_action_then(&mut self, a: StepAction, say: Say) -> Option<Result<String, String>> {
        let Half::Apart(ap) = &mut self.control else {
            return Some(self.step_action(a));
        };
        ap.ask(Box::new(move |c| {
            let at = std::time::Instant::now();
            let was = c.costs;
            let r = c.step_action(a);
            // Apart, the edit's own time on the editing side's thread.
            let took = at.elapsed();
            let costs = &mut c.costs;
            if costs.steps_in_place > was.steps_in_place {
                costs.slowest_in_place = costs.slowest_in_place.max(took);
            } else if costs.steps_compiled > was.steps_compiled {
                costs.slowest_step_compiled = costs.slowest_step_compiled.max(took);
            }
            Some(Told::Said(say, r))
        }));
        None
    }

    /// The pad hits of the last period that fell while the transport
    /// rolled (for the Play page's timing); [`TimeHost::clear_hits`]
    /// after reading.
    pub fn hits(&self) -> &[HitInfo] {
        self.audio.hits()
    }

    /// Every note started in the period just run ([`TimeAudio::take_notes`]):
    /// its frame, track, note and velocity, for the looper's smart loop
    /// end; then forgets them. The newest frame of a note lost for want
    /// of room, if any was.
    pub fn take_notes(&mut self, f: impl FnMut(u64, u16, u8, u8)) -> Option<u64> {
        self.audio.take_notes(f)
    }

    pub fn clear_hits(&mut self) {
        self.audio.clear_hits();
    }

    /// The parameter locks of the last period, for `MixHost::lock` (the
    /// next period starts a new list; [`TimeHost::clear_locks`] after
    /// handing them on).
    pub fn locks(&self) -> &[LockOut] {
        self.audio.locks()
    }

    /// The move lanes' values of the last period, for `MixHost::lane`
    /// (cleared with the locks).
    pub fn lanes(&self) -> &[LaneOut] {
        self.audio.lanes()
    }

    pub fn clear_locks(&mut self) {
        self.audio.clear_locks();
    }

    /// A transport command, from the next period on. False if a queue on
    /// the way was full (counted either way).
    pub fn command(&mut self, cmd: Command) -> bool {
        let refused = self.audio.counts().refused;
        let sent = self.with(|c| c.command(cmd));
        self.audio.receive();
        sent && self.audio.counts().refused == refused
    }

    /// While on, a chance roll counts frames from the one the transport
    /// last started on, so the same range played again from its start
    /// rolls the same: an export with replays turns it on for each pass.
    pub fn anchor_dice(&mut self, on: bool) {
        self.audio.anchor_dice(on);
    }

    /// A transport command on frame `at` of the transport's clock (see
    /// [`TimeHost::next_frame`]), from the coming period on: exact to the
    /// frame, where [`TimeHost::command`] waits for the next period. The
    /// export stops the song where its range ends with it. False if the
    /// transport's queue was full.
    pub fn command_at(&mut self, at: u64, cmd: Command) -> bool {
        self.audio.command_at(at, cmd)
    }

    /// Follow the app's tempo (TAP TEMPO, the menu), the project's: the
    /// whole song's, or while song mode plays the song's tempo points
    /// the tempo before the first. Does nothing while a followed clock
    /// arrives ([`TimeHost::followed_bpm`]).
    pub fn follow_tempo(&mut self, bpm: f64) {
        if self.audio.followed_bpm().is_none() {
            match &mut self.control {
                Half::Here(c) => c.project_tempo(bpm),
                // Apart, only a change goes (it does nothing otherwise).
                Half::Apart(a) if a.tempo != Some(bpm) => {
                    a.tempo = Some(bpm);
                    a.ask(Box::new(move |c| {
                        c.project_tempo(bpm);
                        None
                    }));
                }
                Half::Apart(_) => {}
            }
            self.audio.receive();
        }
        self.audio.follow_tempo(bpm);
    }

    /// A scene's tempo that landed on its line since last asked (the
    /// Arrange refactor's 5.2b), taken: the app makes it its tempo
    /// (`App::set_tempo`), so the tempo chip, Note Repeat and the arps
    /// follow, and the project saves it.
    pub fn take_scene_tempo(&mut self) -> Option<f64> {
        self.audio.take_scene_tempo()
    }

    /// Song mode plays the song's tempo points: the tempo turned sets the
    /// point in effect at the playhead (`TempoEdit::AtPlayhead`), not the
    /// project's tempo.
    pub fn song_tempo_plays(&self) -> bool {
        match &self.control {
            Half::Here(c) => c.song_tempo_plays(),
            Half::Apart(a) => a.light.song_tempo_plays,
        }
    }

    /// While song mode plays the song's tempo points, the tempo of the
    /// point in effect at the playhead (None before the first): what the
    /// tempo turned steps from.
    pub fn tempo_here(&self) -> Option<f64> {
        match &self.control {
            Half::Here(c) => c.tempo_here(),
            Half::Apart(a) => a.light.tempo_here,
        }
    }

    /// Follow another device's MIDI clock with these options (the
    /// project's MIDI setup: follow start/stop, follow position), or not
    /// (None). Cheap to call every period: only a change starts afresh.
    pub fn follow(&mut self, follow: Option<Follow>) {
        self.audio.follow(follow);
    }

    /// The bytes read from the followed port at the top of this period of
    /// `period` frames, before [`TimeHost::advance`]: they came during the
    /// period before, so they go to [`TimeHost::clock_in`] stamped half a
    /// period earlier than [`TimeHost::next_frame`]
    /// ([`follow::read_stamp`]), on average when they came.
    pub fn clock_read(&mut self, period: u32, bytes: &[u8]) -> Option<String> {
        self.clock_in(follow::read_stamp(self.next_frame(), period), bytes)
    }

    /// Every byte read from the followed port this period, before
    /// [`TimeHost::advance`], with `frame` when they came, at most
    /// [`TimeHost::next_frame`]. Anything but
    /// clock, Start, Continue, Stop and Song Position is skipped; the
    /// options given to [`TimeHost::follow`] say what follows. When the
    /// leader's start began a ready take, the clip it records into is
    /// made: a line for the log.
    pub fn clock_in(&mut self, frame: u64, bytes: &[u8]) -> Option<String> {
        self.audio.clock_in(frame, bytes);
        if !self.audio.took() {
            return None;
        }
        let r = self.with(|c| {
            let r = c.take_started();
            // No clip to record into (no empty slot): it plays without
            // recording, the punch-in taken back on its frame.
            if r.is_err() {
                c.command(Command::Record);
            }
            r
        });
        Some(
            self.sent(r)
                .unwrap_or_else(|e| format!("rec: {e}, so it only plays")),
        )
    }

    /// Start the song on frame `at` (see [`TimeHost::next_frame`]): the
    /// looper's Starts: When I play, whose first note starts the song so
    /// bar 1 is where you started. Only while it is stopped: back to the
    /// start of the bar it stopped in (and `into` ticks after it, where a
    /// loop closed by smart loop end's Auto already is), the tempo `bpm`
    /// if given (the whole
    /// song takes it), then PLAY on `at`, with no count-in. With a take
    /// ready (`take`, as for [`TimeHost::take_waits`]) it records from
    /// there too, a punch-in with no count-in, into the clip REC readies
    /// (with no clip to record into it only plays, and the line says so).
    /// Ok: a line for the log, if there is more to say. Err: why it can't
    /// start (the song plays or counts in, or another device's clock
    /// leads), counted ([`TimeHost::starts_refused`]).
    pub fn start_at(
        &mut self,
        at: u64,
        bpm: Option<f64>,
        take: Option<bool>,
        into: Tick,
    ) -> Result<Option<String>, String> {
        // The clip REC readies comes first, so with none to record into
        // the song starts without the take, and no punch-in needs taking
        // back. Only when the start will go in.
        let ready = match take {
            Some(_) if self.audio.start_refusal(bpm, take, into).is_none() => {
                Some(self.with(|c| c.take_started_on_note()))
            }
            _ => None,
        };
        let take = take.filter(|_| ready.as_ref().is_some_and(Result::is_ok));
        self.audio.start_at(at, bpm, take, into)?;
        Ok(ready.map(|r| {
            self.sent(r)
                .unwrap_or_else(|e| format!("rec: {e}, so it only plays"))
        }))
    }

    /// Starts on a note refused so far ([`TimeHost::start_at`]); the
    /// looper's status line counts them with its own (an export running).
    #[allow(dead_code)]
    pub fn starts_refused(&self) -> u64 {
        self.audio.starts_refused()
    }

    /// The song's timeline as it is now, for a timing change to be made
    /// against ([`TimeAudio::timeline`]).
    pub fn timeline(&self) -> u64 {
        self.audio.timeline()
    }

    /// Whether timing change `c` may be offered now: Safe, Ask (with
    /// `audio`, recorded audio playing at its own speed, at a new tempo)
    /// or Refused ([`TimeAudio::allowed`]).
    pub fn allowed(&self, c: &TimingChange, audio: bool) -> Allowed {
        self.audio.allowed(c, audio)
    }

    /// Timing change `c`, on its frame: its ticket, or why not
    /// ([`TimeAudio::timing_change`]). Called on the audio loop's thread,
    /// as [`TimeHost::start_at`] is.
    pub fn timing_change(&mut self, c: TimingChange) -> Result<u32, &'static str> {
        self.audio.timing_change(c)
    }

    /// Take back the timing change with `ticket` if it still waits.
    pub fn cancel_timing(&mut self, ticket: u32, why: &'static str) -> bool {
        self.audio.cancel_timing(ticket, why)
    }

    /// What became of the timing change with `ticket`, if it is one of
    /// the last few.
    pub fn timing_answer(&self, ticket: u32) -> Option<Answer> {
        self.audio.timing_answer(ticket)
    }

    /// Following another device's Start and Stop while it is stopped:
    /// PLAY waits for its Start, and a ready take records from it.
    pub fn clock_waits(&self) -> bool {
        self.audio.clock_waits()
    }

    /// A take the app has ready (REC pressed while stopped), adding (true)
    /// or replacing, every period before [`TimeHost::clock_in`]: while
    /// [`TimeHost::clock_waits`], the leader's start records it.
    pub fn take_waits(&mut self, take: Option<bool>) {
        self.audio.take_waits(take);
    }

    /// The device the transport follows, by name, for what is said while
    /// it waits (None: "the leader"). Cheap to call every period.
    pub fn set_leader(&mut self, name: Option<&str>) {
        if self.leader.as_deref() != name {
            self.leader = name.map(str::to_string);
        }
    }

    /// While PLAY waits for another device's Start
    /// ([`TimeHost::clock_waits`]): that device's name, for what is said
    /// ("the leader" when it isn't known).
    pub fn waiting_for(&self) -> Option<&str> {
        self.clock_waits()
            .then(|| self.leader.as_deref().unwrap_or("the leader"))
    }

    /// The first frame of the coming period on the transport's frame
    /// clock, which [`TimeHost::clock_in`] counts in.
    pub fn next_frame(&self) -> u64 {
        self.audio.next_frame()
    }

    /// The followed clock's tempo while it arrives, for the app's tempo
    /// (the chip and TAP TEMPO's start). None while not following, or
    /// with no clock arriving.
    pub fn followed_bpm(&self) -> Option<f64> {
        self.audio.followed_bpm()
    }

    /// The followed device's clock stopped arriving while it played, and
    /// no pulse or Stop has come since: System status's "The clock MPFree
    /// follows stopped".
    pub fn clock_lost(&self) -> bool {
        self.audio.clock_lost()
    }

    /// Carry out a touch on the time engine's screens. Returns a line for
    /// the log.
    pub fn act(&mut self, a: TimeAction) -> Result<String, String> {
        self.play_from(&a);
        let waits = self.audio.clock_waits();
        let r = self.with(|c| {
            c.clock_waits = waits;
            c.act(a)
        });
        self.sent(r)
    }

    /// [`TimeHost::act`] with the audio tracks, for a song time edit, UNDO
    /// and REDO while they hold regions or a song edit's step: their
    /// regions move with the song in the same period, or nothing moves
    /// anywhere (the Arrange refactor's 3.4,
    /// [`TimeControl::act_with_audio`]). Apart, the control half is
    /// borrowed back for it ([`Apart::lend`]), so the edit and the regions
    /// go together.
    pub fn act_with_audio(
        &mut self,
        a: TimeAction,
        audio: &mut dyn SongAudio,
    ) -> Result<String, String> {
        self.play_from(&a);
        let waits = self.audio.clock_waits();
        let r = self.with(|c| {
            c.clock_waits = waits;
            c.act_with_audio(a, audio)
        });
        self.sent(r)
    }

    /// A take, or an import placing regions, ended on audio track
    /// `track`'s lane over song ticks `span`: the blocks under it are cut
    /// there, as a song edit of its own, in the same period (audio tracks
    /// phase 2, step 2d; [`TimeControl::audio_took`]). Its number, to tag
    /// the take's undo step with, and the line to say; None when no block
    /// was there.
    pub fn audio_took(
        &mut self,
        track: mpfree_seq::TrackId,
        span: (Tick, Tick),
    ) -> Result<Option<(u32, String)>, String> {
        let r = self.with(|c| c.audio_took(track, span));
        self.sent(r)
    }

    /// Whether the song's UNDO (`undo`), or its REDO, still reaches song
    /// edit `n` ([`TimeControl::song_reaches`]).
    pub fn song_reaches(&mut self, n: u32, undo: bool) -> bool {
        self.with(|c| c.song_reaches(n, undo))
    }

    /// Where PLAY starts from, for SHIFT + PLAY ([`TimeHost::play_back`]).
    fn play_from(&mut self, a: &TimeAction) {
        let info = self.info();
        if info.state == mpfree_transport::State::Stopped {
            match a {
                TimeAction::Transport(Command::Play) => self.play_from = Some(info.position),
                TimeAction::Transport(Command::PlayStart) => self.play_from = Some(0),
                _ => {}
            }
        }
    }

    /// [`TimeHost::act`] for the screen's and the buttons' touches, which
    /// only say what they did: together the answer (`Some`), for the caller
    /// to say as `say` asks; apart None at once, the action sent to the
    /// control half's thread, and its answer later from
    /// [`TimeHost::take_said`] with `say`, so a compile never costs the
    /// audio thread (Arrange 2.6b).
    pub fn act_then(&mut self, a: TimeAction, say: Say) -> Option<Result<String, String>> {
        if !self.apart() {
            return Some(self.act(a));
        }
        self.play_from(&a);
        let waits = self.audio.clock_waits();
        if let Half::Apart(ap) = &mut self.control {
            ap.ask(Box::new(move |c| {
                c.clock_waits = waits;
                Some(Told::Said(say, c.act(a)))
            }));
        }
        None
    }

    /// What live to song put into the song when the transport stopped
    /// (or why it could not), once, for the log; apart, also what a take
    /// did to Rec song and what the loops did on the grid.
    pub fn take_later_note(&mut self) -> Option<String> {
        match &mut self.control {
            Half::Here(c) => c.take_later_note(),
            Half::Apart(a) => a.take_note(),
        }
    }

    /// The morph bar on the mixer's mod engine (`modu`): the rows, the
    /// crossfader (which loads the rows on its first move), snap back,
    /// keep and saving a row. A line for the log.
    pub fn morph(
        &mut self,
        a: TimeAction,
        modu: &mut crate::modmix::ModMix,
        mixer: &mpfree_fx::Mixer,
    ) -> Result<String, String> {
        let r = self.with(|c| c.morph(a, modu, mixer));
        self.sent(r)
    }

    /// A clip made elsewhere (the Sample page's chop to a phrase) into the
    /// first empty slot of the armed track (track 1 if none is armed), as
    /// a take that UNDO on the Clips page takes away again. It isn't
    /// launched: a tap on the Clips page plays it like any clip. Returns a
    /// line for the log.
    pub fn put_clip(&mut self, clip: Clip) -> Result<String, String> {
        let r = self.with(|c| c.put_clip(clip));
        self.sent(r)
    }

    /// Clips for several tracks at once (a MIDI file's tracks), each into
    /// its track's first empty slot, as one take that one UNDO on the
    /// Clips page takes away; all or none. Returns a line for the log.
    pub fn put_clips(&mut self, clips: Vec<(usize, Clip)>) -> Result<String, String> {
        let r = self.with(|c| c.put_clips(clips));
        self.sent(r)
    }

    /// A pad note as pressed, before any arp, in the coming period at
    /// `offset`. When the armed track's clips play through its arp, this
    /// is what it records, and what the capture history keeps. A clip of
    /// notes has no notes from it: [`TimeHost::pad_pressed_notes`] gives
    /// them.
    pub fn pad_pressed(&mut self, offset: u32, event: Event) {
        self.audio
            .pad_pressed(offset, &Heard::of(Some(event), event, &[]));
    }

    /// [`TimeHost::pad_pressed`] with the notes the pad plays, as written
    /// (`PadRows`): a take of notes on a track whose clips play through
    /// its arp records them (the instrument redesign's step 5d).
    pub fn pad_pressed_notes(&mut self, offset: u32, event: Event, notes: &[u8]) {
        self.audio
            .pad_pressed(offset, &Heard::of(Some(event), event, notes));
    }

    /// Whether a live hit on a pad that starts on the launch line waits
    /// for it now ([`TimeAudio::pads_wait`], sampler R5.3b).
    pub fn pads_wait(&self) -> bool {
        self.audio.pads_wait()
    }

    /// Whether something played on `from` waits for its line.
    pub fn waits_on(&self, from: PadFrom) -> bool {
        self.audio.waits_on(from)
    }

    /// `w` waits for its launch line; false when too many wait.
    pub fn wait_pad(&mut self, w: PadWait) -> bool {
        self.audio.wait_pad(w)
    }

    /// Whether `w` lets go of a pad whose hit STOP dropped while it was
    /// held: it plays nothing.
    pub fn lets_go_of_dropped(&mut self, w: PadWait) -> bool {
        self.audio.lets_go_of_dropped(w)
    }

    /// The pads of bank `page` whose hit waits for its line (they blink).
    pub fn hits_waiting(&self, page: usize) -> u16 {
        self.audio.hits_waiting(page)
    }

    /// What waited and plays in the period [`TimeHost::advance`] ran
    /// ([`TimeAudio::due_pads`]); returns how many hits STOP dropped.
    pub fn due_pads(&mut self, due: &mut DuePads) -> usize {
        self.audio.due_pads(due)
    }

    /// A pad note as heard (after the arp) in the coming period, at
    /// `offset`: recorded into the armed track while the transport
    /// records, and kept in the capture history always, unless the armed
    /// track's clips play through its arp (then [`TimeHost::pad_pressed`]
    /// gives those). It lights the pattern's step either way.
    pub fn pad(&mut self, offset: u32, event: Event) {
        self.audio.pad(offset, event);
    }

    /// A key a route played on `track`, which isn't armed (the I/O view's
    /// Routing), in the coming period at `offset`: the pad that plays its
    /// note there, its end or its pressure; `AllOff` ends every note held
    /// there. CAPTURE's history keeps it, and CAPTURE puts it in that
    /// track; nothing records it.
    pub fn routed_played(&mut self, track: u8, offset: u32, event: Event) {
        self.audio.routed(track, offset, event);
    }

    /// A note by its number as heard on `track`, another armed track (the
    /// pads' arp's notes on a synth they play on their own, sampler step
    /// 15c): its recorder takes it into a clip of notes while the
    /// transport records, as the armed track takes its arp's notes; a clip
    /// of pad hits has no pad for it. CAPTURE's history keeps it.
    pub fn routed_note(&mut self, track: u8, offset: u32, event: Event) {
        self.audio.routed_note(track, offset, event);
    }

    /// Pad `pad`'s pressure (0 to 127, raw) in the coming period, at
    /// `offset`: recorded onto the pad's held note while the armed track
    /// records and records pressure (`Track::record_touch`), and kept on
    /// it in the capture history always. The pads' notes go in through
    /// [`TimeHost::pad`] or [`TimeHost::pad_pressed`] first, so a value on
    /// their offset comes after them.
    pub fn pad_pressure(&mut self, offset: u32, pad: u8, value: u8) {
        self.audio.pad_pressure(offset, pad, value);
    }

    /// "Record pad pressure" for `track` (`Track::record_touch`). False if
    /// there is no such track.
    #[allow(dead_code)]
    pub fn set_record_touch(&mut self, track: usize, on: bool) -> bool {
        let r = self.with(|c| c.set_record_touch(track, on));
        self.sent(r)
    }

    /// Pressure values lost: past the recorder's tables, or a clip with
    /// no room left (should stay 0). The status line says it
    /// ([`TimeHost::describe`]); tests read it here.
    #[allow(dead_code)]
    pub fn touch_dropped(&self) -> u64 {
        let lost = match &self.control {
            Half::Here(c) => c.touch_lost(),
            Half::Apart(a) => a.light.touch_lost,
        };
        self.audio.touch_dropped() + lost
    }

    /// Live erase (ERASE held on a pad while the transport plays, the
    /// MPC's way): from the coming period, while the transport records or
    /// plays, the armed track's notes of pad `pad` are erased as the
    /// playhead reaches them, with their locks, into the take being
    /// recorded, or while only playing a take of their own for the hold
    /// (`mpfree_play::ERASE_TAKES`), so UNDO brings them back; `on` false
    /// stops it. On a clip of pad hits they are the pad's; on a clip of
    /// notes, every note the pad plays on that track (refactor E4).
    pub fn erase_sound(&mut self, pad: u8, on: bool) {
        self.audio.erase_sound(pad, on);
    }

    /// Live erase off for every sound (ERASE let go).
    pub fn erase_none(&mut self) {
        self.audio.erase_none();
    }

    /// Whether live erase is erasing any sound.
    pub fn erasing(&self) -> bool {
        self.audio.erasing() != 0
    }

    /// Whether live erase holds pad `pad`.
    pub fn erases(&self, pad: u8) -> bool {
        1u128
            .checked_shl(u32::from(pad))
            .is_some_and(|b| self.audio.erasing() & b != 0)
    }

    /// A knob moved in the coming period, at `offset`: parameter `addr` is
    /// now `value`. While the transport records and `addr` is on the armed
    /// track, it becomes a parameter lock on the step it belongs to (see
    /// `mpfree_play::Recorder`). The main loop calls it for the screen's
    /// knob turns while recording (`record_knob` in main.rs).
    pub fn knob(&mut self, offset: u32, addr: mpfree_play::mpfree_param::ParamAddr, value: f32) {
        self.audio.knob(offset, addr, value);
    }

    /// A Perform FX move in the coming period, at `offset`: the XY pad,
    /// the DJ knob or a pad's Hold moving a parameter (`Input::Move`, the
    /// first value starting a gesture), or letting go (`Input::End`).
    /// While the transport records it becomes a move in the armed track's
    /// clip; while it rolls the move log keeps it for CAPTURE. The main
    /// loop calls it for the keys' wheel, pressure and controllers on the
    /// armed track's gear (`expr_rec`, thinned to one value a period at
    /// most); the Perform FX page will once the pad player runs it.
    pub fn gesture(&mut self, offset: u32, input: Input) {
        self.audio.gesture(offset, input);
    }

    /// A setting the XY pad moved (`Input::Move`) or let go of
    /// (`Input::End`) in the coming period, at `offset`, for the song's
    /// own lanes: while Rec song records it becomes a move there
    /// (`SongRecorder::moved`), else nothing (`xy_rec.rs` says which
    /// moves go here).
    pub fn song_gesture(&mut self, offset: u32, input: Input) {
        self.audio.song_gesture(offset, input);
    }

    /// Whether Rec song is armed (it records while the transport plays).
    pub fn rec_song(&self) -> bool {
        self.song_rec_armed()
    }

    /// FILL on or off from `offset` in the coming period: notes with the
    /// FILL and !FILL trig conditions follow it from that frame on. For
    /// the FILL button (hold for a fill, double-press to latch): PLAY
    /// while the transport plays (control map decision 9).
    pub fn set_fill(&mut self, on: bool, offset: u32) {
        self.audio.set_fill(on, offset);
    }

    /// TRACK MUTE's keys (control map decision 5): mute track `t` now, or
    /// let it play again. Its clip keeps its place, so it comes back in
    /// time; a scene's mutes set it again when the scene starts.
    #[cfg(test)]
    pub fn toggle_mute(&mut self, t: usize) -> Result<String, String> {
        self.mute_or_solo(t, false, false)
    }

    /// TRACK MUTE's pad (sequencer S4): track `t`'s mute flips, or with
    /// `solo` its solo; with `wait` (Perform, or SHIFT in Make) the change
    /// joins the mute queue and lands on the launch line while the
    /// transport rolls ([`TimeControl::mute_or_solo`]).
    pub fn mute_or_solo(&mut self, t: usize, wait: bool, solo: bool) -> Result<String, String> {
        let r = self.with(|c| c.mute_or_solo(t, wait, solo));
        self.sent(r)
    }

    /// [`TimeHost::mute_or_solo`] as [`TimeHost::act_then`] makes an
    /// action: together the answer, apart None and the answer later.
    pub fn mute_or_solo_then(
        &mut self,
        t: usize,
        wait: bool,
        solo: bool,
        say: Say,
    ) -> Option<Result<String, String>> {
        match &mut self.control {
            Half::Here(_) => Some(self.mute_or_solo(t, wait, solo)),
            Half::Apart(a) => {
                a.ask(Box::new(move |c| {
                    Some(Told::Said(say, c.mute_or_solo(t, wait, solo)))
                }));
                None
            }
        }
    }

    /// Pad `pad` of track `t`'s kit tapped as a mute key: at once, or
    /// waiting in the mute queue ([`TimeControl::pad_mute_or_solo`]; the
    /// instrument redesign's step 10).
    pub fn pad_mute_or_solo(
        &mut self,
        t: usize,
        pad: usize,
        wait: bool,
        solo: bool,
    ) -> Result<PadTap, String> {
        let r = self.with(|c| c.pad_mute_or_solo(t, pad, wait, solo));
        self.sent(r)
    }

    /// Pad `pad` of track `t` muted or let play at once, from a pattern
    /// row's speaker ([`TimeControl::pad_mute_now`]).
    pub fn pad_mute_now(&mut self, t: usize, pad: usize, mute: bool) -> Result<String, String> {
        let r = self.with(|c| c.pad_mute_now(t, pad, mute));
        self.sent(r)
    }

    /// What Rec song last put on audio tracks' lanes, once: its song edit's
    /// number and the blocks' spans by lasting id, for the audio tracks to
    /// replace the regions under them as a step tagged with the number
    /// (audio tracks phase 2, step 2d; [`TimeControl::take_song_placed`]).
    /// Then [`TimeHost::tie_audio`] if they made one.
    pub fn take_song_placed(&mut self) -> Option<SongPlaced> {
        match &mut self.control {
            Half::Here(c) => c.take_song_placed(),
            Half::Apart(a) => a.take_placed(),
        }
    }

    /// The audio tracks made a step tied to song edit `n`: the song's UNDO
    /// and REDO of it go through them ([`TimeControl::tie_audio`]). False
    /// when the song's history no longer holds `n`.
    pub fn tie_audio(&mut self, n: u32) -> bool {
        let r = self.with(|c| c.tie_audio(n));
        self.sent(r)
    }

    /// An old project's kit 1 pad mutes and solos set on each track in
    /// `tracks` ([`TimeControl::carry_pad_mutes`]): how many took them.
    pub fn carry_pad_mutes(&mut self, tracks: u128, mutes: u128, solos: u128) -> usize {
        let r = self.with(|c| c.carry_pad_mutes(tracks, mutes, solos));
        self.sent(r)
    }

    /// Track `t`'s pad mutes and solos, a bit each by kit pad, as the
    /// project has them (apart, its published copy).
    pub fn pad_mutes(&self, t: usize) -> Option<(u128, u128)> {
        self.project()
            .tracks
            .get(t)
            .map(|tr| (tr.pad_mutes, tr.pad_solos))
    }

    /// Whether a change waits for track `t`'s pad `pad`, for its light.
    /// Apart, the published copy has the selected track's pads only (the
    /// keys light those), so another track's say none wait.
    pub fn pad_waiting(&self, t: usize, pad: usize) -> bool {
        match &self.control {
            Half::Here(c) => c.pad_waiting(t, pad),
            Half::Apart(a) => {
                a.light.selected == Some(t)
                    && a.light.pad_waiting.get(pad).copied().unwrap_or(false)
            }
        }
    }

    /// Whether track `t` is muted (None: no such track).
    #[allow(dead_code)]
    pub fn muted(&self, t: usize) -> Option<bool> {
        match &self.control {
            Half::Here(c) => c.muted(t),
            Half::Apart(a) => a.light.muted.get(t).copied().flatten(),
        }
    }

    /// The recordings the project's audio clips, the clips UNDO or REDO
    /// could bring back and the clip COPY holds name, sorted, each once
    /// ([`TimeControl::recordings`]; apart, as last published).
    pub fn recordings(&self) -> Vec<mpfree_seq::RecId> {
        match &self.control {
            Half::Here(c) => c.recordings(),
            Half::Apart(a) => a.heavy.recordings.clone(),
        }
    }

    /// The tracks silent now, muted or left out by a solo, a bit a track
    /// ([`TimeControl::silenced`]): a track gone silent ends what its kit's
    /// Instrument and MIDI out pads hold, which no strip of its mutes.
    pub fn silenced(&self) -> u128 {
        match &self.control {
            Half::Here(c) => c.silenced(),
            Half::Apart(a) => a.light.silenced,
        }
    }

    /// The tracks muted and soloed once the mute queue lands, a bit a
    /// track ([`TimeControl::mute_bits`]), for MIDI Learn.
    pub fn mute_bits(&self) -> (u128, u128) {
        match &self.control {
            Half::Here(c) => c.mute_bits(),
            Half::Apart(a) => a.light.mute_bits,
        }
    }

    /// What TRACK MUTE's pad for track `t` shows (None: no such track).
    pub fn mute_light(&self, t: usize) -> Option<MuteLight> {
        match &self.control {
            Half::Here(c) => c.mute_light(t),
            Half::Apart(a) => a.light.mute_light.get(t).copied().flatten(),
        }
    }

    /// The mute queue waiting, if any.
    #[allow(dead_code)]
    pub fn mute_wait(&self) -> Option<&mpfree_time_ui::view::MuteWait> {
        match &self.control {
            Half::Here(c) => c.mute_wait(),
            Half::Apart(a) => a.heavy.mute_wait.as_ref(),
        }
    }

    /// The project is in Perform, not Make (sequencer S4): the app follows
    /// it each period (`App::set_perform`).
    pub fn perform(&self) -> bool {
        match &self.control {
            Half::Here(c) => c.perform(),
            Half::Apart(a) => a.light.perform,
        }
    }

    /// Whether a take records (the notes, their count-in, or a loop's
    /// pass), each period: in Perform a take turns Rec song on for its
    /// length ([`TimeControl::take_follows`]). A line for the log.
    pub fn take_follows(&mut self, taking: bool) -> Option<String> {
        let r = match &mut self.control {
            Half::Here(c) => c.take_follows(taking),
            // Apart, only a change goes (it does nothing otherwise); its
            // line comes back for the log ([`TimeHost::take_later_note`]).
            Half::Apart(a) if a.taking != Some(taking) => {
                a.taking = Some(taking);
                a.ask(Box::new(move |c| c.take_follows(taking).map(Told::Note)));
                None
            }
            Half::Apart(_) => None,
        };
        self.sent(r)
    }

    /// Which loops the Loop tracks have on, and the bar line each change
    /// lands on ([`TimeControl::loop_plan`]): for the looper, before its
    /// period, so a launch lands on its line's own frame.
    pub fn loop_plan(&self) -> mpfree_seq::loops::LoopPlan {
        match &self.control {
            Half::Here(c) => c.loop_plan(),
            Half::Apart(a) => a.light.loop_plan,
        }
    }

    /// Loops on audio tracks (Dev's switch, phase 4b), for this start
    /// (see `TimeControl::set_loops_on`).
    pub fn set_loops_on(&mut self, on: bool) {
        self.with(|c| c.set_loops_on(on));
        self.sent(());
    }

    /// [`TimeHost::loops_are`] with each loop's lasting id (0: none yet),
    /// for loops on audio tracks (see `TimeControl::loops_are_on`).
    pub fn loops_are_on(
        &mut self,
        now: &[mpfree_seq::loops::LoopNow; mpfree_seq::LOOPS],
        ids: &[u128; mpfree_seq::LOOPS],
    ) -> Vec<String> {
        let r = match &mut self.control {
            Half::Here(c) => c.loops_are_on(now, ids),
            // Apart, sent; the lines come back for the log
            // ([`TimeHost::take_later_note`]).
            Half::Apart(a) => {
                a.loops_are_on(*now, *ids);
                Vec::new()
            }
        };
        self.sent(r)
    }

    /// What the looper says of its loops ([`TimeControl::loops_are`]):
    /// recorded loops land on Loop tracks, cells follow their loops.
    pub fn loops_are(
        &mut self,
        now: &[mpfree_seq::loops::LoopNow; mpfree_seq::LOOPS],
    ) -> Vec<String> {
        let r = match &mut self.control {
            Half::Here(c) => c.loops_are(now),
            // Apart, sent; the lines come back for the log
            // ([`TimeHost::take_later_note`]).
            Half::Apart(a) => {
                a.loops_are(*now);
                Vec::new()
            }
        };
        self.sent(r)
    }

    /// The project's tracks, for TRACK MUTE's banks.
    pub fn track_count(&self) -> usize {
        match &self.control {
            Half::Here(c) => c.track_count(),
            Half::Apart(a) => a.light.track_count,
        }
    }

    /// REDOs done so far, and whether REDO has anything to do: a track
    /// that comes back with a REDO gets its kit back
    /// ([`crate::sampler::TrackKits::follow`]).
    pub fn redos(&self) -> (u64, bool) {
        match &self.control {
            Half::Here(c) => (c.redos(), c.can_redo()),
            Half::Apart(a) => (a.light.redos, a.light.can_redo),
        }
    }

    /// Whether track `t` has a clip launched (playing, or waiting for the
    /// transport); TRACK MUTE's pad lights have it in [`MuteLight`].
    #[cfg(test)]
    pub fn launched(&self, t: usize) -> bool {
        match &self.control {
            Half::Here(c) => c.launched(t),
            Half::Apart(a) => t < 128 && a.light.launched & (1 << t) != 0,
        }
    }

    /// FILL, as of the last period, for the button's light.
    #[allow(dead_code)]
    pub fn fill(&self) -> bool {
        self.audio.fill()
    }

    /// Run the transport for the coming period, before the app's process,
    /// so the arp can follow it ([`TimeHost::arp_clock`]).
    /// [`TimeHost::period`] then plays and records on the same block.
    pub fn advance(&mut self, frames: u32) -> bool {
        self.audio.advance(frames)
    }

    /// The arp's clock for the coming period, from the first span of the
    /// block [`TimeHost::advance`] made: None while stopped, so the arp
    /// runs on its own tempo. The count-in counts as playing. `jumped` is
    /// set by a start or jump before the first span, or by one later in the
    /// last period (the arp follows the first span only, so it catches up
    /// one period later).
    // The per-track arps (TimeHost::period_arps) take the clock from the
    // block's spans themselves (ArpHost); App::process's single arp used
    // this. Kept for a host that drives one Engine directly.
    #[allow(dead_code)]
    pub fn arp_clock(&mut self) -> Option<mpfree_arp::Clock> {
        self.audio.arp_clock()
    }

    /// Run one period: the clips play into `pads`, every track as
    /// recorded (no arp), and the armed track records. Runs the transport
    /// first unless [`TimeHost::advance`] already did. For previews and
    /// tests; the pad player calls [`TimeHost::period_arps`].
    pub fn period(&mut self, frames: u32, pads: &mut impl Pads) {
        self.run(frames, pads, None);
    }

    /// Each track's ahead in frames for the coming periods (sequencer S5,
    /// `MidiOut::ahead`): its clips' notes go to MIDI out that far before
    /// their period ([`Pads::out_ahead`]). All 0 plays nothing ahead.
    pub fn set_ahead(&mut self, frames: &[u32]) {
        self.audio.set_ahead(frames);
    }

    /// The instrument each track plays now, by place (step sequencer
    /// refactor F1): a clip's instrument locks play only on the kind they
    /// were made for. Cheap when nothing changed; told every period.
    pub fn set_instruments(&mut self, plays: &[Option<InstRef>]) {
        self.with(|c| c.set_instruments(plays));
    }

    /// The tracks whose instrument plays notes (a track's own synth, or
    /// another track's it plays, but not the Drums engine), one bit each:
    /// a new clip REC makes for a take on one holds notes (step 5c), its
    /// clips through its arp too (step 5d).
    pub fn set_notes_tracks(&mut self, tracks: u128) {
        self.with(|c| c.set_notes_tracks(tracks));
    }

    /// The scales the note editor's Scale option offers (piano roll plan
    /// 2B.1): the synth pads' scale, if any, and the owner's own.
    pub fn set_scale_offers(
        &mut self,
        pads: Option<mpfree_seq::Scale>,
        own: &[(String, u16, u16)],
    ) {
        self.with(|c| c.set_scale_offers(pads, own));
    }

    /// The notes each of track `track`'s 16 pads plays: a clip of notes
    /// shows each pad's root on the step grid's pad rows, and a tap on a
    /// pad's row edits its notes (step 5c).
    pub fn set_pad_rows(&mut self, track: usize, rows: PadRows) {
        self.with(|c| c.set_pad_rows(track, rows));
    }

    /// What the look-ahead did, for the log.
    pub fn ahead_counts(&self) -> AheadCounts {
        self.audio.ahead_counts()
    }

    /// [`TimeHost::period`] with the per-track arps between the player and
    /// `pads`: a track whose clips play through its arp ("Clips through the
    /// arp") plays them into its arp; the others play as recorded; every
    /// arp's notes (the live pads' too, fed with [`TrackArps::pad`] before
    /// this) go to `pads` with them, in frame order. Each arp first takes
    /// its track's destination and switch.
    pub fn period_arps(&mut self, frames: u32, pads: &mut impl Pads, arps: &mut TrackArps) {
        self.audio.dress_arps(arps);
        self.run(frames, pads, Some(arps));
    }

    /// The audio half's period, then the control half's share of it, then
    /// anything that sent back (a view compiled for a cue that started, or
    /// for a recording with no room left in place).
    fn run(&mut self, frames: u32, pads: &mut impl Pads, arps: Option<&mut TrackArps>) {
        self.audio.metronome = self.metronome;
        self.audio.set_click_gain(self.click_gain);
        self.audio.run(frames, pads, arps);
        match &mut self.control {
            Half::Here(c) => {
                c.poll();
                self.audio.receive();
            }
            // Apart, the thread polls; the newest copy comes here.
            Half::Apart(a) => a.take_shown(),
        }
    }

    /// Add the click to the rendered period; returns the peak afterwards.
    pub fn mix_click(&mut self, left: &mut [f32], right: &mut [f32]) -> f32 {
        self.audio.mix_click(left, right)
    }

    /// The transport's description of the last period (for the looper).
    pub fn block(&self) -> &Block {
        self.audio.block()
    }

    /// The transport's tempo map (for the audio tracks, which place their
    /// regions on song ticks).
    pub fn tempo_map(&self) -> &mpfree_transport::TempoMap {
        self.audio.tempo_map()
    }

    /// What audio track `track` (in the track list) played in the last
    /// period: its clips, its song or nothing, each change on its frame
    /// (audio tracks phase 2, step 2b). Call after [`TimeHost::period`].
    pub fn audio(&self, track: usize) -> mpfree_play::AudioAnswer<'_> {
        self.audio.audio(track)
    }

    /// Song mode is on: the song plays on tracks with no clip launched.
    /// Audio tracks record only then (layout spec 4c), and play their song
    /// regions only then; their clips play in both.
    pub fn song_on(&self) -> bool {
        match &self.control {
            Half::Here(c) => c.song_on(),
            Half::Apart(a) => a.light.song_on,
        }
    }

    /// The song list's step playing (together only: the rows played
    /// offline, [`offline`]).
    fn list_playing(&self) -> Option<mpfree_perform::ListPlaying> {
        match &self.control {
            Half::Here(c) => c.list_playing(),
            Half::Apart(_) => None,
        }
    }

    /// Rec song is armed (together only, as [`TimeHost::list_playing`]).
    fn song_rec_armed(&self) -> bool {
        match &self.control {
            Half::Here(c) => c.song_rec_armed(),
            Half::Apart(_) => false,
        }
    }

    /// The song position as bar.beat.tick, for log lines.
    pub fn position_text(&self) -> String {
        self.audio.position_text()
    }

    pub fn info(&self) -> TransportInfo {
        self.audio.info()
    }

    /// What `track` plays now (takes.rs: a frozen strip follows it).
    pub fn now_playing(&self, track: usize) -> Option<mpfree_play::NowPlaying> {
        self.audio.now_playing(track)
    }

    /// True once after the clips, launches or takes changed. Notes and
    /// locks being recorded are told at most every quarter second.
    pub fn take_changed(&mut self) -> bool {
        match &mut self.control {
            Half::Here(c) => c.take_changed(),
            Half::Apart(a) => a.take_changed(),
        }
    }

    /// Everything the screens show.
    pub fn state(&self) -> TimeState {
        match &self.control {
            Half::Here(c) => c.state(),
            Half::Apart(a) => a.heavy.state.clone(),
        }
    }

    /// Notes and locks that gave way (should stay 0): lost because a
    /// table was full, left out of a moment too busy ([`TimeHost::busy`]),
    /// or endings lost ([`TimeHost::ends_lost`]). The status line says
    /// each ([`TimeHost::describe`]); tests read it here.
    #[allow(dead_code)]
    pub fn dropped(&self) -> u64 {
        let c = self.audio.counts();
        c.dropped
            + c.busy
            + u64::from(c.ends_lost)
            + match &self.control {
                Half::Here(c) => c.dropped(),
                Half::Apart(a) => a.light.dropped,
            }
            + self.audio.note_mods_dropped()
    }

    /// New notes, locks and values left out of moments too busy for them
    /// (refactor A2): something new gave way so nothing hangs.
    #[allow(dead_code)]
    pub fn busy(&self) -> u64 {
        self.audio.counts().busy
    }

    /// Endings (note-offs, unlocks, panics) that found no room: a bug,
    /// which must stay 0 (refactor A2).
    #[allow(dead_code)]
    pub fn ends_lost(&self) -> u64 {
        u64::from(self.audio.counts().ends_lost)
    }

    /// The time engine's part of the status line, with what passing
    /// between the halves lost, if anything.
    pub fn describe(&self) -> String {
        match &self.control {
            Half::Here(c) => c.describe(),
            Half::Apart(a) => format!("{}; {}", a.heavy.describe, a.describe()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_compose::State as PreviewState;
    use mpfree_play::PlayView;
    use mpfree_play::Replaced;
    use mpfree_seq::{ROWS, SLOTS};
    use mpfree_time_ui::adding::{AddAction, ChordsAction, PartAsk};
    use mpfree_time_ui::continuing::{ContinueAction, ContinueAsk, Continuing};
    use mpfree_time_ui::view::{AudioAction, RegionId};
    use mpfree_transport::time::Tick;
    use mpfree_transport::State;
    use std::sync::Arc;

    #[test]
    fn the_morph_bar_runs_the_scene_morph_on_the_mixers_mod_engine() {
        use mpfree_fx::{ids::mixer as mx, StripId};
        use mpfree_seq::mpfree_param::{Owner, ParamAddr, Strip};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut mix = crate::mixing::MixHost::new(RATE, 0.0, 6.0);
        let lp = crate::mixing::LOOP;
        let vol = ParamAddr::new(Owner::Strip(Strip::Track(lp as u8)), mx::VOLUME);
        let was = mix.mixer.param(StripId::Track(lp), mx::VOLUME);
        h.control
            .project
            .scene_mut(0)
            .unwrap()
            .set_value(vol, -24.0)
            .unwrap();
        h.control
            .project
            .scene_mut(1)
            .unwrap()
            .set_value(vol, -2.0)
            .unwrap();
        let run = |mix: &mut crate::mixing::MixHost, from: u64| {
            let z = [0f32; crate::PERIOD];
            let (mut l, mut r) = ([0f32; crate::PERIOD], [0f32; crate::PERIOD]);
            for i in from..from + 40 {
                let ctx = mpfree_fx::mpfree_node::Ctx::new(RATE, i * crate::PERIOD as u64);
                mix.period(&ctx, (&z, &z), (&z, &z), (&z, &z), &mut l, &mut r);
            }
        };
        // The crossfader's first move loads rows 1 and 2.
        let line = h
            .morph(TimeAction::Morph(1.0), &mut mix.modu, &mix.mixer)
            .unwrap();
        assert_eq!(line, "morph: rows 1 and 2, 1 parameters");
        assert!(h.state().morph.on);
        run(&mut mix, 0);
        assert!((mix.mixer.param(StripId::Track(lp), mx::VOLUME) + 2.0).abs() < 0.05);
        // Other rows while morphing: on A and B at once.
        h.control
            .project
            .scene_mut(3)
            .unwrap()
            .set_value(vol, -12.0)
            .unwrap();
        h.morph(
            TimeAction::MorphRows { a: 0, b: 3 },
            &mut mix.modu,
            &mix.mixer,
        )
        .unwrap();
        run(&mut mix, 40);
        assert!((mix.mixer.param(StripId::Track(lp), mx::VOLUME) + 12.0).abs() < 0.05);
        assert_eq!(
            h.morph(TimeAction::SnapBack, &mut mix.modu, &mix.mixer)
                .unwrap(),
            "morph: snapped back"
        );
        run(&mut mix, 80);
        assert!((mix.mixer.param(StripId::Track(lp), mx::VOLUME) - was).abs() < 0.05);
        assert!(!h.state().morph.on);
        // Save: row 6 stores the mixer as it is.
        let line = h
            .morph(TimeAction::SaveScene(5), &mut mix.modu, &mix.mixer)
            .unwrap();
        assert!(line.starts_with("morph: row 6 stores"), "{line}");
        // Snap back with nothing loaded does nothing.
        assert_eq!(
            h.morph(TimeAction::SnapBack, &mut mix.modu, &mix.mixer)
                .unwrap(),
            ""
        );
    }

    #[test]
    fn a_clip_put_from_elsewhere_lands_in_the_first_empty_slot_and_undo_takes_it() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut clip = Clip::new("Break phrase", 4 * 960);
        for k in 0..4u8 {
            clip.add_note(mpfree_seq::Note::new(i64::from(k) * 960, 480, k, 127))
                .unwrap();
        }
        // Slot 1 of track 1 is taken: the phrase goes in slot 2.
        h.control.project.tracks[0].slots[0] = Some(Clip::new("taken", 960));
        let line = h.put_clip(clip.clone()).unwrap();
        assert_eq!(line, "phrase: 4 notes in slot 2 of Track 1");
        // A new clip, with a lasting id of its own.
        let put = h.control.project.clip(0, 1).unwrap();
        assert!(put.id.is_given());
        assert_eq!(put.copied(), clip);
        assert!(h.control.launches.is_empty(), "not launched");
        // The armed track takes it.
        h.act(TimeAction::Arm(Some(2))).unwrap();
        assert!(h
            .put_clip(clip.clone())
            .unwrap()
            .ends_with("slot 1 of Track 3"));
        // UNDO on the Clips page takes the last one away, then the first.
        h.act(TimeAction::Undo).unwrap();
        assert!(h.control.project.clip(2, 0).is_none());
        h.act(TimeAction::Undo).unwrap();
        assert!(h.control.project.clip(0, 1).is_none());
        // Bank A full, it goes in bank B (scene banks, 5.6); all 64
        // rows full, the track says so.
        for s in 0..SLOTS {
            h.control.project.tracks[2].slots[s] = Some(Clip::new("full", 960));
        }
        assert!(h
            .put_clip(clip.clone())
            .unwrap()
            .ends_with("slot 17 of Track 3"));
        for s in 0..ROWS {
            h.control.project.tracks[2].slots[s] = Some(Clip::new("full", 960));
        }
        assert!(h.put_clip(clip).unwrap_err().contains("no empty slot"));
    }

    /// A phrase chopped to one sliced pad (sampler R4): played, each hit
    /// on the pad has its Slice lock just before it, so each plays its
    /// own slice.
    #[test]
    fn a_phrase_on_one_pad_plays_each_hit_with_its_slice_lock_first() {
        use mpfree_play::mpfree_param::{OnTrack, RelAddr};
        use mpfree_sampler::engine::PadField;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let slice = RelAddr::new(OnTrack::Pad(4), PadField::Slice as u16);
        let mut clip = Clip::new("Break 92 phrase", 3840);
        for k in 0..4u8 {
            let at = i64::from(k) * 960;
            clip.add_note(mpfree_seq::Note::new(at, 480, 4, 127))
                .unwrap();
            clip.add_lock(mpfree_seq::Lock::new(at, Some(4), slice, f32::from(k)))
                .unwrap();
        }
        assert!(h.put_clip(clip).unwrap().ends_with("slot 1 of Track 1"));
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        let mut pads = Fake::default();
        run(&mut h, &mut pads, BAR - 1000, &[]);
        let hits: Vec<_> = pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        assert_eq!(hits.len(), 4, "{hits:?}");
        assert!(hits.iter().all(|&(_, p)| p == 4));
        let at = slice.on(0);
        let locks: Vec<_> = pads.locks.iter().filter(|l| l.2 == at).collect();
        assert_eq!(locks.len(), 4, "{:?}", pads.locks);
        for (k, l) in locks.iter().enumerate() {
            assert_eq!(
                (l.0, l.1, l.3),
                (hits[k].0, k, Some(k as f32)),
                "slice {} reaches the pad before hit {}",
                k + 1,
                k + 1
            );
        }
    }

    #[test]
    fn clips_for_several_tracks_go_in_as_one_take_or_not_at_all() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut drums = Clip::new("Drums", 4 * 960);
        drums
            .add_note(mpfree_seq::Note::new(0, 240, 1, 100))
            .unwrap();
        let bass = Clip::new("Bass", 4 * 960);
        h.control.project.tracks[1].slots[0] = Some(Clip::new("taken", 960));
        let line = h
            .put_clips(vec![(0, drums.clone()), (1, bass.clone())])
            .unwrap();
        assert_eq!(line, "2 clips with 1 notes, from slot 1 of Track 1");
        let put = |t, s| h.control.project.clip(t, s).map(Clip::copied);
        assert_eq!(put(0, 0), Some(drums.clone()));
        assert_eq!(put(1, 1), Some(bass.clone()));
        let ids = |t, s| h.control.project.clip(t, s).unwrap().id;
        assert!(ids(0, 0).is_given() && ids(1, 1).is_given() && ids(0, 0) != ids(1, 1));
        assert!(h.control.launches.is_empty(), "not launched");
        // One UNDO takes both.
        h.act(TimeAction::Undo).unwrap();
        assert!(h.control.project.clip(0, 0).is_none());
        assert!(h.control.project.clip(1, 1).is_none());
        // A track with no empty slot: none goes in.
        for s in 0..ROWS {
            h.control.project.tracks[1].slots[s] = Some(Clip::new("full", 960));
        }
        let e = h.put_clips(vec![(0, drums), (1, bass)]).unwrap_err();
        assert!(e.contains("no empty slot"), "{e}");
        assert!(h.control.project.clip(0, 0).is_none());
    }

    /// REC on a fresh project (found by the UI audit): the armed track gets
    /// a clip to record into, launched, so what the pads play is kept.
    #[test]
    fn rec_with_nothing_to_record_into_makes_the_armed_tracks_clip() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        assert_eq!(h.control.armed, Some(0), "track 1 armed");
        assert_eq!(
            h.act(TimeAction::Transport(Command::Record)).unwrap(),
            "transport: Record, new clip in slot 1 of track 1"
        );
        assert!(h.control.project.clip(0, 0).is_some());
        assert!(h
            .control
            .launches
            .iter()
            .any(|l| l.track == 0 && l.slot == 0));
        // Pressed again before the transport moves: no second clip.
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        assert!(h.control.project.clip(0, 1).is_none());
        // A track with a clip but none playing: that clip is launched.
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.control.project.tracks[0].slots[3] = Some(Clip::new("beat", 3840));
        assert!(h
            .act(TimeAction::Transport(Command::Record))
            .unwrap()
            .ends_with("slot 4 of track 1 launched"));
        assert!(h.control.project.clip(0, 0).is_none());
    }

    /// ERASE + a tap (Jay after live test 3): a lane's steps or a whole
    /// clip go as one take, and UNDO brings them back; a clip the song
    /// plays stays, since the song's edits have no undo yet.
    #[test]
    fn the_patterns_length_doubles_with_its_steps_and_undo_puts_it_back() {
        use mpfree_seq::Note;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut beat = Clip::new("beat", 3840);
        beat.notes = vec![Note::new(0, 100, 0, 100), Note::new(960, 100, 2, 100)];
        h.control.project.tracks[0].slots[0] = Some(beat);
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        let clip = |h: &TimeHost| {
            let c = h.control.project.clip(0, 0).unwrap();
            let mut ticks: Vec<_> = c.notes.iter().map(|n| n.tick).collect();
            ticks.sort();
            (c.length, ticks)
        };
        // Twice the steps: the new bar repeats the first.
        assert_eq!(
            h.step_action(StepAction::Length {
                steps: 32,
                copy: true
            })
            .unwrap(),
            "steps: beat is 32 steps (2 bars) · the new steps repeat the ones before them"
        );
        assert_eq!(clip(&h), (7680, vec![0, 960, 3840, 4800]));
        h.step_action(StepAction::Length {
            steps: 64,
            copy: true,
        })
        .unwrap();
        assert_eq!(clip(&h).0, 4 * 3840);
        assert_eq!(clip(&h).1.len(), 8);
        // Half: the steps that go take their notes; UNDO brings them back.
        h.step_action(StepAction::Length {
            steps: 16,
            copy: false,
        })
        .unwrap();
        assert_eq!(clip(&h), (3840, vec![0, 960]));
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(clip(&h).0, 4 * 3840, "UNDO: four bars again");
        assert_eq!(clip(&h).1.len(), 8);
        h.act(TimeAction::Undo).unwrap();
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(clip(&h), (3840, vec![0, 960]), "back to the start");
        // The same length again is no take.
        assert_eq!(
            h.step_action(StepAction::Length {
                steps: 16,
                copy: true
            })
            .unwrap(),
            ""
        );
    }

    /// Several step pads held and turned (STEP SEQ): every turn of them
    /// is one undo; COPY of a step is one too.
    #[test]
    fn held_steps_turn_as_one_undo_and_copy_is_one_take() {
        use mpfree_seq::Note;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut beat = Clip::new("beat", 3840);
        beat.notes = vec![Note::new(0, 100, 0, 100), Note::new(960, 100, 0, 90)];
        h.control.project.tracks[0].slots[0] = Some(beat);
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        let velocities = |h: &TimeHost| -> Vec<(i64, u8)> {
            let mut v: Vec<_> = h.control.project.clip(0, 0).unwrap().notes.clone();
            v.sort_by_key(|n| n.tick);
            v.iter().map(|n| (n.tick, n.velocity)).collect()
        };
        for v in [80, 60, 40] {
            h.step_action(StepAction::ChangeSteps {
                pad: 0,
                steps: (1 << 0) | (1 << 4),
                change: Change::Velocity(v),
            })
            .unwrap();
        }
        assert_eq!(velocities(&h), [(0, 40), (960, 40)]);
        let line = h
            .step_action(StepAction::CopySteps {
                pad: 0,
                from: 4,
                to: 12,
                count: 1,
            })
            .unwrap();
        assert_eq!(line, "steps: step 5 of pad 1 to step 13 copied");
        assert_eq!(velocities(&h), [(0, 40), (960, 40), (2880, 40)]);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(velocities(&h), [(0, 40), (960, 40)], "UNDO: the copy goes");
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(
            velocities(&h),
            [(0, 100), (960, 90)],
            "UNDO: the whole turn, both steps"
        );
    }

    /// A held step's lock on its pad's sound: the turns are one undo, and
    /// played, the lock reaches the pads on its note's frame, before the
    /// note, and ends on the pad's next note without it.
    #[test]
    fn held_step_locks_reach_the_pads_before_their_note_and_end_at_the_next() {
        use mpfree_play::mpfree_param::{OnTrack, RelAddr};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        for step in [0, 4] {
            h.step_action(StepAction::Tap {
                pad: 2,
                step,
                velocity: 100,
            })
            .unwrap();
        }
        let track = h.control.armed().expect("an armed track") as u8;
        let tune = RelAddr::new(OnTrack::Pad(2), 2);
        let lines: Vec<String> = [100.0, 300.0, 500.0]
            .iter()
            .map(|&v| {
                h.step_action(StepAction::Lock {
                    pad: 2,
                    steps: 1,
                    addr: tune,
                    value: Some(v),
                })
                .unwrap()
            })
            .collect();
        assert_eq!(
            lines,
            ["steps: step 1 of pad 3: lock pad2.2 100", "", ""],
            "one line a turn"
        );
        let lock = |h: &TimeHost| h.steps().clip.and_then(|c| c.lock_value(2, 0, tune));
        assert_eq!(lock(&h), Some(500.0));
        h.command(Command::Play);
        // Two passes of the one-bar clip.
        run(&mut h, &mut pads, 2 * BAR - 1000, &[]);
        let hits: Vec<_> = pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        assert_eq!(hits.len(), 4, "{hits:?}");
        let (on, off) = (Some(500.0), None);
        let at = tune.on(track);
        assert_eq!(
            pads.locks,
            vec![
                (hits[0].0, 0, at, on),
                (hits[1].0, 1, at, off),
                (hits[2].0, 2, at, on),
                (hits[3].0, 3, at, off)
            ],
            "each pass: the lock before step 1's note, its end before step 5's"
        );
        assert_eq!(h.dropped(), 0);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(lock(&h), None, "UNDO: the whole turn");
        assert_eq!(h.steps().clip.unwrap().row(2).iter().flatten().count(), 2);
    }

    /// With a play direction (S6) the step playhead lights the step heard,
    /// as the player plays it: a reversed bar from its last step, ping
    /// pong turning at the end and running on into the next pass. Forward
    /// is as before.
    #[test]
    fn the_step_playhead_follows_the_play_direction() {
        use mpfree_seq::Direction;
        use mpfree_time_ui::view::CLIP_DIRECTION;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.step_action(StepAction::Tap {
            pad: 2,
            step: 0,
            velocity: 100,
        })
        .unwrap();
        let track = h.control.armed().expect("an armed track");
        let slot = h.steps().clip.expect("a clip").slot;
        let direct = |h: &mut TimeHost, d: Direction| {
            let mut settings = h.control.project.clip(track, slot).unwrap().settings();
            settings.direction = d;
            h.act(TimeAction::ClipSettings {
                track,
                slot,
                row: CLIP_DIRECTION as u8,
                settings,
            })
            .unwrap();
        };
        h.command(Command::Play);
        // Into step 3 (a step is 5,512.5 frames at 120 BPM).
        run(&mut h, &mut pads, 2 * 5_512 + 2_000, &[]);
        let clip = h.steps().clip.expect("the clip");
        let origin = clip.origin.expect("it plays");
        let pos = h.info().position;
        assert_eq!(clip.playing(pos), Some(2), "forward: step 3");
        assert_eq!(clip.playing(origin + 5 * 240), Some(5));
        direct(&mut h, Direction::Reverse);
        let clip = h.steps().clip.expect("the clip");
        assert_eq!(clip.playing(pos), Some(13), "reverse: step 14");
        // Passes count from the one the launch started in (its origin is
        // a bar line long before, as for any clip launched while stopped).
        let start = h.control.launches[0].start;
        assert_eq!((start - origin).rem_euclid(3840), 0);
        let at = |k: i64| start + k * 240;
        let lit: Vec<_> = [0, 1, 15, 16].map(|k| clip.playing(at(k))).into();
        assert_eq!(lit, [Some(15), Some(14), Some(0), Some(15)]);
        // The light moves on the step's line, not half a step before.
        assert_eq!(clip.playing(at(1) - 1), Some(15));
        direct(&mut h, Direction::PingPong);
        let clip = h.steps().clip.expect("the clip");
        let lit: Vec<_> = [0, 15, 16, 29, 30, 31].map(|k| clip.playing(at(k))).into();
        assert_eq!(
            lit,
            [Some(0), Some(15), Some(14), Some(1), Some(0), Some(1)]
        );
        // Random: every step once a pass, as the player plays them.
        direct(&mut h, Direction::Random);
        let clip = h.steps().clip.expect("the clip");
        let mut pass: Vec<usize> = (0..16).filter_map(|k| clip.playing(at(k))).collect();
        assert_ne!(pass, (0..16).collect::<Vec<_>>(), "shuffled");
        pass.sort_unstable();
        assert_eq!(pass, (0..16).collect::<Vec<_>>());
    }

    /// The first step on an empty track is one take: the new pattern, its
    /// note and its launch. UNDO leaves the slot empty and stopped, REDO
    /// brings the pattern back playing, and a step past the pattern's end
    /// makes nothing. If the play view can't be built after an edit, the
    /// edit is taken back from the clip, the takes and the launches
    /// (refactor plan A1).
    #[test]
    fn the_first_step_its_pattern_and_its_launch_are_one_take() {
        use mpfree_play::Launch;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let tap = |step| StepAction::Tap {
            pad: 0,
            step,
            velocity: 100,
        };
        let playing = |h: &TimeHost| {
            h.control
                .launches
                .iter()
                .any(|l| (l.track, l.slot) == (0, 0))
        };
        let notes = |h: &TimeHost| h.control.project.clip(0, 0).map(|c| c.notes.len());
        // A view that can't be built: two clips launched on track 2 (a
        // track plays one at a time).
        let two = |h: &mut TimeHost| {
            for slot in 0..2 {
                h.control.project.tracks[1].slots[slot] = Some(Clip::new("x", 3840));
                h.control.launches.push(Launch {
                    track: 1,
                    slot,
                    origin: 0,
                    start: 0,
                });
            }
        };
        two(&mut h);
        let launches = h.control.launches.clone();
        let said = h.step_action(tap(0)).unwrap_err();
        assert!(said.ends_with("the edit was taken back"), "{said}");
        assert_eq!(notes(&h), None, "no pattern");
        assert_eq!(h.control.launches, launches);
        assert_eq!(h.act(TimeAction::Undo).unwrap(), "undo: nothing to undo");
        h.control.launches.clear();
        h.control.project.tracks[1].slots = Default::default();
        // A step past the pattern's end makes nothing at all.
        assert_eq!(h.step_action(tap(16)).unwrap(), "");
        assert_eq!(notes(&h), None);
        assert_eq!(h.act(TimeAction::Undo).unwrap(), "undo: nothing to undo");
        // The first step: the pattern, its note and its launch.
        let said = h.step_action(tap(0)).unwrap();
        assert!(said.starts_with("steps: new clip Pattern 1"), "{said}");
        assert_eq!(notes(&h), Some(1));
        assert!(playing(&h));
        // One UNDO takes all three back.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(notes(&h), None, "the slot is empty");
        assert!(!playing(&h), "and stopped");
        assert_eq!(h.act(TimeAction::Undo).unwrap(), "undo: nothing to undo");
        // REDO brings the pattern back playing.
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(notes(&h), Some(1));
        assert!(playing(&h));
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        assert_eq!(
            pads.hits.first().map(|&(_, pad, _)| pad),
            Some(0),
            "it plays"
        );
        // An edit the view can't follow (a new length compiles) is taken
        // back: the clip as it was, and UNDO takes back the first step.
        two(&mut h);
        let clip = h.control.project.clip(0, 0).cloned();
        let said = h
            .step_action(StepAction::Length {
                steps: 32,
                copy: true,
            })
            .unwrap_err();
        assert!(said.ends_with("the edit was taken back"), "{said}");
        assert_eq!(h.control.project.clip(0, 0).cloned(), clip);
        h.control.launches.retain(|l| l.track != 1);
        h.control.project.tracks[1].slots = Default::default();
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(notes(&h), None, "UNDO took back the first step");
    }

    /// Taps, turns and locks on the pattern playing go into the view the
    /// audio half plays, in place: no compile, nothing allocated or freed
    /// on the audio half, and the view is just what a compile of the
    /// project gives, each time (the order of notes on one tick too).
    #[test]
    fn step_edits_go_into_the_playing_view_in_place_as_a_compile_would() {
        use mpfree_play::mpfree_param::{OnTrack, RelAddr};
        use mpfree_seq::steps::Change;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        // The first tap makes the pattern and launches it: that compiles.
        let tap = |pad, step, velocity| StepAction::Tap {
            pad,
            step,
            velocity,
        };
        h.step_action(tap(0, 0, 100)).unwrap();
        assert_eq!(h.control.costs.steps_compiled, 1);
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        let compiles = h.control.costs.compiles;
        let tune = RelAddr::new(OnTrack::Pad(2), 2);
        let (s5, s9, s13) = (1u128 << 4, 1u128 << 8, 1u128 << 12);
        let edits = [
            tap(2, 4, 100),
            tap(2, 8, 90),
            // On the tick of pad 3's step 9: after it, as a compile puts it.
            tap(0, 8, 80),
            StepAction::Change {
                pad: 2,
                step: 4,
                change: Change::Velocity(60),
            },
            StepAction::Change {
                pad: 2,
                step: 8,
                change: Change::Nudge(30),
            },
            StepAction::ChangeSteps {
                pad: 2,
                steps: s5 | s9,
                change: Change::Chance(50),
            },
            // Step 13 has no note: a lock with no note.
            StepAction::Lock {
                pad: 2,
                steps: s5 | s13,
                addr: tune,
                value: Some(300.0),
            },
            StepAction::Lock {
                pad: 2,
                steps: s5 | s13,
                addr: tune,
                value: Some(500.0),
            },
            StepAction::CopySteps {
                pad: 2,
                from: 4,
                to: 6,
                count: 1,
            },
            StepAction::ClearLocks { pad: 2, steps: s5 },
            StepAction::ClearSteps {
                pad: 2,
                from: 8,
                to: 9,
            },
            tap(2, 4, 100),
        ];
        for (i, a) in edits.into_iter().enumerate() {
            // The control half alone, then the audio half takes what it sent.
            let line = h.control.step_action(a);
            assert!(line.is_ok(), "{a:?}: {line:?}");
            let ((), allocs) = crate::sys::alloc_count::calls(|| h.audio.receive());
            assert_eq!(allocs, 0, "{a:?}: the audio half allocated or freed");
            assert_eq!(
                *h.audio.view,
                h.control.view().unwrap(),
                "{a:?}: the view in place is what a compile gives"
            );
            // A little play between the edits.
            run(&mut h, &mut pads, BAR / 2 + (i as u64 + 1) * BAR / 8, &[]);
        }
        assert_eq!(h.control.costs.compiles, compiles, "nothing compiled");
        assert_eq!(h.control.costs.steps_in_place, edits.len() as u64);
        assert_eq!(h.audio.counts().missed, 0);
        // Through the facade too, timed for the status line.
        h.step_action(tap(5, 0, 100)).unwrap();
        let c = h.control.costs;
        assert_eq!((c.compiles, c.steps_in_place), (compiles, 13));
        assert!(c.slowest_in_place > std::time::Duration::ZERO);
        assert!(
            h.describe().contains("; step edits: 13 in place (slowest "),
            "{}",
            h.describe()
        );
    }

    /// In song mode a tap on a clip the song plays goes into the one copy
    /// every block of it plays, on its own lane and on another track's, in
    /// place (Arrange 2.3): no compile, nothing allocated or freed on the
    /// audio half, and the view just what a compile gives.
    #[test]
    fn a_step_edit_in_song_mode_goes_into_every_block_in_place() {
        use mpfree_seq::arrange::Placement;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let tap = |pad, step, velocity| StepAction::Tap {
            pad,
            step,
            velocity,
        };
        // A pattern on track 0, stopped, then in the song: two blocks on
        // its own lane and one on track 1's.
        h.step_action(tap(0, 0, 100)).unwrap();
        h.act(TimeAction::StopAll).unwrap();
        let p = &mut h.control.project;
        for (track, start) in [(0, 0), (0, 2 * 3840), (1, 3840)] {
            let mut b = Placement::of(&p.tracks, 0, 0, start, 3840);
            b.track = track;
            p.arrangement.place(b).unwrap();
        }
        h.act(TimeAction::SongMode(true)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        assert_eq!(h.steps().clip.map(|c| c.slot), Some(0));
        let compiles = h.control.costs.compiles;
        let edits = [tap(0, 4, 100), tap(0, 8, 90), tap(0, 4, 100)];
        for (i, a) in edits.into_iter().enumerate() {
            let line = h.control.step_action(a);
            assert!(line.is_ok(), "{a:?}: {line:?}");
            let ((), allocs) = crate::sys::alloc_count::calls(|| h.audio.receive());
            assert_eq!(allocs, 0, "{a:?}: the audio half allocated or freed");
            assert_eq!(
                *h.audio.view,
                h.control.view().unwrap(),
                "{a:?}: the view in place is what a compile gives"
            );
            run(&mut h, &mut pads, BAR / 2 + (i as u64 + 1) * BAR / 4, &[]);
        }
        assert_eq!(h.control.costs.compiles, compiles, "nothing compiled");
        assert_eq!(h.control.costs.steps_in_place, edits.len() as u64);
        assert_eq!(h.audio.counts().missed, 0);
    }

    /// A step edit the view can't take in place compiles, as before: a new
    /// pattern, the pattern's length, a big erase, and any edit while
    /// recording, when the audio half edits the view itself.
    #[test]
    fn step_edits_that_need_more_than_the_playing_view_compile() {
        use mpfree_seq::Note;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut clip = Clip::new("busy", 128 * STEP_TICKS);
        clip.notes = (0..128)
            .map(|i| Note::new(i * STEP_TICKS, 100, 1, 100))
            .collect();
        h.control.project.tracks[0].slots[0] = Some(clip);
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        let ways = |h: &TimeHost| {
            (
                h.control.costs.steps_in_place,
                h.control.costs.steps_compiled,
            )
        };
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 3,
            velocity: 100,
        })
        .unwrap();
        assert_eq!(ways(&h), (1, 0), "a tap");
        h.step_action(StepAction::Length {
            steps: 16,
            copy: false,
        })
        .unwrap();
        assert_eq!(ways(&h), (1, 1), "the length");
        h.act(TimeAction::Undo).unwrap();
        // Pad 2's 128 steps: past `MAX_IN_PLACE`.
        h.step_action(StepAction::ClearPad { pad: 1 }).unwrap();
        assert_eq!(ways(&h), (1, 2), "a big erase");
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        run(&mut h, &mut pads, BAR / 4, &[]);
        assert_eq!(h.info().state, State::Recording);
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 5,
            velocity: 100,
        })
        .unwrap();
        assert_eq!(ways(&h), (1, 3), "while recording");
        let mut fresh = h.control.view().unwrap();
        fresh.set_armed(h.control.armed);
        assert_eq!(*h.audio.view, fresh);
    }

    /// A lock turned on a long clip goes in place: its changes cost the
    /// clip's locks, not its notes (refactor B3), so each Q-Link detent
    /// needs no compile. Two steps' notes changed on the same clip are past
    /// the work allowed in place, and compile.
    #[test]
    fn a_lock_turned_on_a_long_clip_goes_in_place() {
        use mpfree_play::mpfree_param::{OnTrack, RelAddr};
        use mpfree_seq::steps::Change;
        use mpfree_seq::Note;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut clip = Clip::new("long", 128 * STEP_TICKS);
        // 64 notes on each of 128 steps: as many as a note change in
        // place may search.
        clip.notes = (0..128)
            .flat_map(|i| (0..64).map(move |n| Note::new(i * STEP_TICKS, 100, n, 100)))
            .collect();
        assert_eq!(clip.notes.len(), mpfree_play::IN_PLACE_WORK);
        h.control.project.tracks[0].slots[0] = Some(clip);
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        let ways = |h: &TimeHost| {
            (
                h.control.costs.steps_in_place,
                h.control.costs.steps_compiled,
            )
        };
        let compiles = h.control.costs.compiles;
        let tune = RelAddr::new(OnTrack::Pad(2), 2);
        for (i, value) in [300.0, 320.0, 340.0].into_iter().enumerate() {
            h.step_action(StepAction::Lock {
                pad: 2,
                steps: 1 << 4,
                addr: tune,
                value: Some(value),
            })
            .unwrap();
            run(&mut h, &mut pads, BAR / 2 + (i as u64 + 1) * BAR / 8, &[]);
        }
        assert_eq!(ways(&h), (3, 0), "each detent in place");
        assert_eq!(h.control.costs.compiles, compiles, "nothing compiled");
        assert_eq!(h.audio.counts().missed, 0);
        assert_eq!(
            *h.audio.view,
            h.control.view().unwrap(),
            "the view in place is what a compile gives"
        );
        h.step_action(StepAction::ChangeSteps {
            pad: 2,
            steps: (1 << 4) | (1 << 8),
            change: Change::Velocity(60),
        })
        .unwrap();
        assert_eq!(ways(&h), (3, 1), "two steps' notes");
    }

    /// Refactor C3: a track selected apart from the arm (picked while
    /// REC's arm is pinned elsewhere) has room in the view for its step
    /// edits, so a tap, a velocity change and a lock detent on its playing
    /// clip go in place as the armed track's do: nothing compiled, nothing
    /// missed, and the view in place is what a compile gives. The armed
    /// track's clip keeps its room for recording.
    #[test]
    fn step_edits_on_a_track_selected_apart_from_the_arm_go_in_place() {
        use mpfree_play::mpfree_param::{OnTrack, RelAddr};
        use mpfree_seq::steps::Change;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        for t in [0, 2] {
            h.control.project.tracks[t].slots[0] = Some(Clip::new("Beat", 16 * STEP_TICKS));
            h.act(TimeAction::Launch { track: t, slot: 0 }).unwrap();
        }
        h.act(TimeAction::Arm(Some(0))).unwrap();
        h.act(TimeAction::Select(2)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        let ways = |h: &TimeHost| {
            (
                h.control.costs.steps_in_place,
                h.control.costs.steps_compiled,
            )
        };
        let compiles = h.control.costs.compiles;
        let tune = RelAddr::new(OnTrack::Pad(1), 2);
        let edits = [
            StepAction::Tap {
                pad: 1,
                step: 4,
                velocity: 100,
            },
            StepAction::Change {
                pad: 1,
                step: 4,
                change: Change::Velocity(60),
            },
            StepAction::Lock {
                pad: 1,
                steps: 1 << 4,
                addr: tune,
                value: Some(300.0),
            },
        ];
        for (i, a) in edits.into_iter().enumerate() {
            h.step_action(a).unwrap();
            run(&mut h, &mut pads, BAR / 4 + (i as u64 + 1) * BAR / 8, &[]);
        }
        assert_eq!(ways(&h), (3, 0), "each edit in place");
        assert_eq!(h.control.costs.compiles, compiles, "nothing compiled");
        assert_eq!(h.audio.counts().missed, 0);
        assert_eq!(*h.audio.view, h.control.view().unwrap());
        let clip = h.project().clip(2, 0).unwrap();
        assert_eq!((clip.notes.len(), clip.locks.len()), (1, 1));
        assert!(h.project().clip(0, 0).unwrap().notes.is_empty());
        // Selecting another track compiles once, for its room; selecting
        // the armed track again needs none.
        h.act(TimeAction::Select(3)).unwrap();
        assert_eq!(h.control.costs.compiles, compiles + 1);
        h.act(TimeAction::Select(0)).unwrap();
        assert_eq!(h.control.costs.compiles, compiles + 1);
    }

    /// Refactor C2: the clip chip picks which of the selected track's
    /// clips the Sequencer edits, without starting it: what plays keeps
    /// playing, the edit goes to the picked clip (one take UNDO takes
    /// back) with nothing compiled, and another track selected, What
    /// plays or the clip taken away goes back to the clip that plays.
    #[test]
    fn a_clip_picked_to_edit_takes_the_steps_and_nothing_starts() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        for (s, name) in ["Pattern 1", "Pattern 2", "Pattern 3"]
            .into_iter()
            .enumerate()
        {
            h.control.project.tracks[0].slots[s] = Some(Clip::new(name, 16 * STEP_TICKS));
        }
        h.control.project.tracks[1].slots[0] = Some(Clip::new("Bass", 16 * STEP_TICKS));
        h.control.project.give_clip_ids();
        h.act(TimeAction::Arm(Some(0))).unwrap();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        let said = h.act(TimeAction::EditClip(Some(2))).unwrap();
        assert_eq!(said, "editing Pattern 3 on Track 1 (Pattern 1 plays)");
        let steps = h.steps();
        assert_eq!(steps.clip.as_ref().map(|c| c.slot), Some(2));
        assert!(steps.picked);
        assert_eq!(steps.playing.as_deref(), Some("Pattern 1"));
        assert_eq!(h.state().clip_pick, Some(2));
        // A step goes into Pattern 3; Pattern 1 plays on, untouched, and
        // nothing compiles (the picked clip isn't in what plays).
        let compiles = h.control.costs.compiles;
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 4,
            velocity: 100,
        })
        .unwrap();
        run(&mut h, &mut pads, BAR / 2, &[]);
        assert_eq!(h.project().clip(0, 2).unwrap().notes.len(), 1);
        assert!(h.project().clip(0, 0).unwrap().notes.is_empty());
        assert_eq!(h.control.launches.len(), 1);
        assert_eq!(h.control.launches[0].slot, 0, "Pattern 1 still plays");
        assert_eq!(h.control.costs.compiles, compiles, "nothing compiled");
        assert_eq!(*h.audio.view, h.control.view().unwrap());
        h.act(TimeAction::Undo).unwrap();
        assert!(
            h.project().clip(0, 2).unwrap().notes.is_empty(),
            "UNDO takes it back"
        );
        assert_eq!(
            h.steps().clip.as_ref().map(|c| c.slot),
            Some(2),
            "and the pick stays"
        );
        // Launching another clip keeps the pick.
        h.act(TimeAction::Launch { track: 0, slot: 1 }).unwrap();
        run(&mut h, &mut pads, BAR * 2, &[]);
        assert_eq!(h.steps().clip.as_ref().map(|c| c.slot), Some(2));
        // What plays: the clip that plays again.
        h.act(TimeAction::EditClip(None)).unwrap();
        let steps = h.steps();
        assert_eq!(steps.clip.as_ref().map(|c| c.slot), Some(1));
        assert!(!steps.picked);
        assert_eq!(steps.playing, None);
        assert_eq!(h.state().clip_pick, None);
        // Another track selected forgets the pick; back on track 1 the
        // Sequencer shows what plays.
        h.act(TimeAction::EditClip(Some(2))).unwrap();
        h.act(TimeAction::Arm(Some(1))).unwrap();
        assert_eq!(h.state().clip_pick, None);
        h.act(TimeAction::Arm(Some(0))).unwrap();
        assert_eq!(h.steps().clip.as_ref().map(|c| c.slot), Some(1));
        // The picked clip dragged to another row: the pick goes with it
        // (kept by the clip's lasting id, not its row).
        h.act(TimeAction::EditClip(Some(2))).unwrap();
        h.act(TimeAction::DropClip {
            from: (0, 2),
            to: (0, 5),
            copy: false,
        })
        .unwrap();
        assert_eq!(h.state().clip_pick, Some(5));
        assert_eq!(h.steps().clip.as_ref().map(|c| c.slot), Some(5));
        // A picked clip taken away: the Sequencer goes back to what plays.
        h.act(TimeAction::DeleteClip { track: 0, slot: 5 }).unwrap();
        assert_eq!(h.state().clip_pick, None);
        assert_eq!(h.steps().clip.as_ref().map(|c| c.slot), Some(1));
        // An empty slot, or nothing selected, can't be picked.
        assert!(h.act(TimeAction::EditClip(Some(9))).is_err());
        h.act(TimeAction::Arm(None)).unwrap();
        assert!(h.act(TimeAction::EditClip(Some(0))).is_err());
    }

    /// The step size (refactor D1) cuts the same clip into steps of
    /// another size: its length and every note's time stay, nothing
    /// compiles, a tap lands on the new grid, and UNDO puts the size back.
    #[test]
    fn the_step_size_cuts_the_clip_into_other_steps_and_no_note_moves() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        // 13 sixteenths, notes on 1/16 steps 1 and 3 and one nudged later.
        let mut beat = Clip::new("Bass", 13 * STEP_TICKS);
        for (tick, pitch) in [(0, 36), (480, 36), (600, 38)] {
            beat.notes
                .push(mpfree_seq::Note::new(tick, 120, pitch, 100));
        }
        h.control.project.tracks[0].slots[0] = Some(beat);
        h.control.project.give_clip_ids();
        h.act(TimeAction::Arm(Some(0))).unwrap();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        let before = h.project().clip(0, 0).unwrap().notes.clone();
        let compiles = h.control.costs.compiles;
        let said = h.act(TimeAction::StepSize(480)).unwrap();
        assert_eq!(said, "steps: Bass in 1/8 steps (7)");
        let clip = h.project().clip(0, 0).unwrap();
        assert_eq!((clip.step, clip.length), (480, 13 * STEP_TICKS));
        assert_eq!(clip.notes, before, "no note moves");
        assert_eq!(h.control.costs.compiles, compiles, "nothing compiled");
        assert_eq!(*h.audio.view, h.control.view().unwrap());
        let steps = h.steps().clip.unwrap();
        assert_eq!((steps.steps, steps.step), (7, 480));
        // Step 2 of 1/8 holds the note on 1/16 step 3 and the one 120
        // ticks after it, nudged.
        let kick = steps.row(36)[1].unwrap();
        assert_eq!((kick.nudge, kick.len), (0, 120));
        assert_eq!(steps.row(38)[1].map(|s| s.nudge), Some(120));
        // A tap on 1/8 step 4 puts a note on tick 1440.
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 3,
            velocity: 100,
        })
        .unwrap();
        let added: Vec<_> = h
            .project()
            .clip(0, 0)
            .unwrap()
            .notes
            .iter()
            .filter(|n| !before.contains(n))
            .map(|n| n.tick)
            .collect();
        assert_eq!(added, [1440]);
        // Triplets: 3,120 ticks is 10 steps of 1/8T, the last one short.
        h.act(TimeAction::StepSize(320)).unwrap();
        assert_eq!(h.steps().clip.unwrap().steps, 10);
        // The head's length counts the clip's steps (12 triplet eighths
        // are a bar); the clip card's Length row still counts sixteenths.
        let said = h
            .step_action(StepAction::Length {
                steps: 12,
                copy: false,
            })
            .unwrap();
        assert!(said.contains("is 12 steps (1 bar)"), "{said}");
        assert_eq!(h.project().clip(0, 0).unwrap().length, 16 * STEP_TICKS);
        h.act(TimeAction::ClipLength {
            track: 0,
            slot: 0,
            steps: 8,
            copy: false,
        })
        .unwrap();
        assert_eq!(h.project().clip(0, 0).unwrap().length, 8 * STEP_TICKS);
        h.act(TimeAction::Undo).unwrap();
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.project().clip(0, 0).unwrap().length, 13 * STEP_TICKS);
        // UNDO: the triplets go, then the tap, then the eighths.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.project().clip(0, 0).unwrap().step, 480);
        h.act(TimeAction::Undo).unwrap();
        h.act(TimeAction::Undo).unwrap();
        let clip = h.project().clip(0, 0).unwrap();
        assert_eq!((clip.step, &clip.notes), (240, &before));
        assert_eq!(h.steps().clip.unwrap().steps, 13);
        // Not a step size; a track with no clip.
        assert!(h.act(TimeAction::StepSize(200)).is_err());
        h.act(TimeAction::Arm(Some(1))).unwrap();
        assert!(h.act(TimeAction::StepSize(480)).is_err());
    }

    /// Refactor D2: the length a step at a time, from the clip's own
    /// length: fewer steps take their notes (UNDO brings them back), more
    /// are empty, at the clip's step size; it stops at one step and at
    /// 128, and a longer (recorded) clip is never cut by a step more.
    #[test]
    fn the_length_goes_a_step_at_a_time_and_a_long_clip_is_never_cut() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut beat = Clip::new("Bass", 16 * STEP_TICKS);
        for step in [0, 13, 14] {
            beat.notes
                .push(mpfree_seq::Note::new(step * STEP_TICKS, 120, 36, 100));
        }
        h.control.project.tracks[0].slots[0] = Some(beat);
        h.control.project.give_clip_ids();
        h.act(TimeAction::Arm(Some(0))).unwrap();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        let before = h.project().clip(0, 0).unwrap().notes.clone();
        let by = |h: &mut TimeHost, steps| h.step_action(StepAction::LengthBy { steps });
        for _ in 0..3 {
            by(&mut h, -1).unwrap();
        }
        let clip = h.project().clip(0, 0).unwrap();
        assert_eq!(clip.length, 13 * STEP_TICKS);
        assert_eq!(clip.notes.len(), 1, "steps 14 and 15 went with theirs");
        let said = by(&mut h, 1).unwrap();
        assert_eq!(
            said,
            "steps: Bass is 14 steps (1 bar) · the new steps are empty"
        );
        let said = by(&mut h, -1).unwrap();
        assert!(
            said.ends_with("13 steps (1 bar) · UNDO brings the rest back"),
            "{said}"
        );
        // Five takes: UNDO five times, and the notes are back.
        for _ in 0..5 {
            h.act(TimeAction::Undo).unwrap();
        }
        let clip = h.project().clip(0, 0).unwrap();
        assert_eq!((clip.length, &clip.notes), (16 * STEP_TICKS, &before));
        // At 1/8 steps: one more is an eighth, and the bars follow.
        h.act(TimeAction::StepSize(480)).unwrap();
        let said = by(&mut h, 1).unwrap();
        assert!(said.contains("is 9 steps (2 bars)"), "{said}");
        assert_eq!(h.project().clip(0, 0).unwrap().length, 9 * 480);
        // Down to one step, and no further.
        assert!(by(&mut h, -100).unwrap().contains("is 1 step (1 bar)"));
        assert_eq!(by(&mut h, -1).unwrap_err(), "steps: 1 step is the fewest");
        // Up to 128, and no further.
        by(&mut h, 500).unwrap();
        assert_eq!(h.steps().clip.unwrap().steps, MAX_STEPS);
        assert!(by(&mut h, 1).unwrap_err().contains("128 steps is the most"));
        // A recorded clip of 200 sixteenths: a step more is refused, a
        // step fewer takes one step, never down to 128.
        h.act(TimeAction::StepSize(240)).unwrap();
        h.step_action(StepAction::Length {
            steps: 200,
            copy: false,
        })
        .unwrap();
        assert!(by(&mut h, 1).is_err());
        by(&mut h, -1).unwrap();
        assert_eq!(h.project().clip(0, 0).unwrap().length, 199 * STEP_TICKS);
        // No clip: nothing.
        h.act(TimeAction::Arm(Some(1))).unwrap();
        assert_eq!(by(&mut h, 1), Ok(String::new()));
    }

    /// A Velocity or Chance lane's stroke (refactor D3): each step it
    /// crossed gets its own value in one edit, so one UNDO takes the
    /// whole stroke back; steps it left alone and steps with no note stay
    /// as they were.
    #[test]
    fn a_lanes_stroke_sets_each_step_its_own_value_in_one_undo() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        for step in [0, 4, 8] {
            h.step_action(StepAction::Tap {
                pad: 0,
                step,
                velocity: 100,
            })
            .unwrap();
        }
        let before = h.steps().clip.unwrap();
        let mut values = [0u8; 16];
        values[0] = 40;
        values[1] = 90; // no note there: nothing
        values[4] = 127;
        let said = h
            .step_action(StepAction::Values {
                pad: 0,
                from: 0,
                kind: NoteValue::Velocity,
                values,
            })
            .unwrap();
        assert_eq!(said, "steps: Velocity of pad 1 on 2 steps");
        let vel = |h: &TimeHost| {
            let c = h.steps().clip.unwrap();
            [0, 4, 8].map(|i| c.row(0)[i].map(|s| s.velocity))
        };
        assert_eq!(vel(&h), [Some(40), Some(127), Some(100)]);
        assert!(h.steps().clip.unwrap().row(0)[1].is_none());
        // Chance the same way, its own edit.
        let mut chance = [0u8; 16];
        chance[8] = 25;
        h.step_action(StepAction::Values {
            pad: 0,
            from: 0,
            kind: NoteValue::Chance,
            values: chance,
        })
        .unwrap();
        assert_eq!(h.steps().clip.unwrap().row(0)[8].unwrap().chance, 25);
        // UNDO twice: the chance, then the whole stroke.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.steps().clip.unwrap().row(0)[8].unwrap().chance, 100);
        assert_eq!(vel(&h), [Some(40), Some(127), Some(100)]);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.steps().clip.unwrap(), before);
    }

    /// A play direction moves by 1/16s whatever the step size (refactor
    /// D1): the playhead lights the step of the 1/16 heard. Reversed, 8
    /// sixteenths as 4 eighths light 4, 4, 3, 3 ... from the start.
    #[test]
    fn a_play_direction_lights_the_step_heard_at_any_step_size() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut clip = Clip::new("Hats", 8 * STEP_TICKS);
        clip.notes.push(mpfree_seq::Note::new(0, 120, 36, 100));
        clip.direction = mpfree_seq::Direction::Reverse;
        clip.step = 480;
        h.control.project.tracks[0].slots[0] = Some(clip);
        h.control.project.give_clip_ids();
        h.act(TimeAction::Arm(Some(0))).unwrap();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        let c = h.steps().clip.unwrap();
        assert!(c.order.is_some());
        let o = c.origin.unwrap();
        let lit: Vec<_> = (0..8).map(|k| c.playing(o + k * STEP_TICKS + 10)).collect();
        let want: Vec<_> = [3, 3, 2, 2, 1, 1, 0, 0].into_iter().map(Some).collect();
        assert_eq!(lit, want);
    }

    /// A change the view can't make in place (here, a note it erases that
    /// the project changed behind its back) asks for one compile, which brings it and
    /// the changes after it; the status line counts it.
    #[test]
    fn a_step_edit_the_view_cannot_take_compiles_once_and_is_counted() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 0,
            velocity: 100,
        })
        .unwrap();
        let slot = h.control.launches[0].slot;
        // Step 1's note, softer in the project than in the view.
        let clip = h.control.project.tracks[0].slots[slot].as_mut().unwrap();
        clip.notes[0].velocity = 50;
        let compiles = h.control.costs.compiles;
        let edits = [
            StepAction::Change {
                pad: 0,
                step: 0,
                change: mpfree_seq::steps::Change::Velocity(60),
            },
            StepAction::Tap {
                pad: 0,
                step: 8,
                velocity: 100,
            },
        ];
        for a in edits {
            h.control.step_action(a).unwrap();
        }
        h.audio.receive();
        assert_eq!(h.audio.counts().missed, 1, "step 1's erase");
        assert_ne!(*h.audio.view, h.control.view().unwrap());
        run(&mut h, &mut pads, u64::from(PERIOD), &[]);
        assert_eq!(h.control.costs.compiles, compiles + 1, "one compile");
        assert_eq!(*h.audio.view, h.control.view().unwrap());
        assert!(
            h.describe().contains(", 1 step edits compiled again"),
            "{}",
            h.describe()
        );
    }

    /// A step edit reaches what plays whole or not at all (refactor A3):
    /// a queue that fills half way through it, or a view other than the
    /// one it was made on, leaves the playing view as it was (not half the
    /// edit), and the compiled view brings it.
    #[test]
    fn a_step_edit_reaches_the_playing_view_whole_or_not_at_all() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let tap = |step| StepAction::Tap {
            pad: 0,
            step,
            velocity: 100,
        };
        h.step_action(tap(0)).unwrap();
        run(&mut h, &mut pads, u64::from(PERIOD), &[]);
        assert_eq!(*h.audio.view, h.control.view().unwrap());
        let in_place = h.control.costs.steps_in_place;
        // Room in the queue for one of the edit's three messages (the
        // note out, the softer note in, the end).
        h.audio.receive();
        h.control.fill_queue_but(1);
        let before = h.audio.view.clone();
        h.control
            .step_action(StepAction::Change {
                pad: 0,
                step: 0,
                change: mpfree_seq::steps::Change::Velocity(60),
            })
            .unwrap();
        h.audio.receive();
        assert_eq!(
            *h.audio.view, *before,
            "not the note out without the note in"
        );
        assert_eq!(h.audio.counts().missed, 0, "nothing came to miss");
        let to = pads.frame + 2 * u64::from(PERIOD);
        run(&mut h, &mut pads, to, &[]);
        assert_eq!(
            *h.audio.view,
            h.control.view().unwrap(),
            "the compiled view brings it"
        );
        assert_eq!(h.audio.view_n, h.control.view_sent());
        // Made on a view the audio half doesn't play (as when it sent the
        // newest back to compile again): left out, and compiled.
        h.audio.view_n = h.audio.view_n.wrapping_add(1);
        let before = h.audio.view.clone();
        h.control.step_action(tap(4)).unwrap();
        assert_eq!(
            h.control.costs.steps_in_place,
            in_place + 1,
            "sent in place"
        );
        h.audio.receive();
        assert_eq!(*h.audio.view, *before);
        assert_eq!(h.audio.counts().missed, 1);
        let to = pads.frame + 2 * u64::from(PERIOD);
        run(&mut h, &mut pads, to, &[]);
        assert_eq!(*h.audio.view, h.control.view().unwrap());
        assert_eq!(h.audio.view_n, h.control.view_sent());
        // On the view it was made on, whole, with no compile.
        let compiles = h.control.costs.compiles;
        h.control.step_action(tap(8)).unwrap();
        h.audio.receive();
        assert_eq!(*h.audio.view, h.control.view().unwrap());
        assert_eq!(h.control.costs.compiles, compiles);
        assert_eq!(h.audio.counts().missed, 1);
    }

    #[test]
    fn erase_takes_a_lanes_steps_or_a_clip_and_undo_brings_them_back() {
        use mpfree_seq::Note;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut beat = Clip::new("beat", 3840);
        beat.notes = vec![
            Note::new(0, 100, 0, 100),
            Note::new(960, 100, 2, 100),
            Note::new(1920, 100, 0, 90),
        ];
        h.control.project.tracks[0].slots[0] = Some(beat.clone());
        h.control.project.tracks[0].slots[1] = Some(Clip::new("fill", 960));
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        // A lane's name: pad 1's steps go, pad 3's stay.
        assert_eq!(
            h.step_action(StepAction::ClearPad { pad: 0 }).unwrap(),
            "steps: pad 1's steps erased"
        );
        // The pads' notes, in time order.
        let notes = |h: &TimeHost| -> Vec<u8> {
            let mut v: Vec<_> = h.control.project.clip(0, 0).unwrap().notes.clone();
            v.sort_by_key(|n| n.tick);
            v.iter().map(|n| n.note).collect()
        };
        assert_eq!(notes(&h), [2]);
        // An empty lane: nothing to do, no take.
        assert_eq!(h.step_action(StepAction::ClearPad { pad: 9 }).unwrap(), "");
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(notes(&h), [0, 2, 0], "UNDO: the lane is back");
        // A clip: gone, with its launch; UNDO puts it back, REDO takes it again.
        assert!(h
            .act(TimeAction::DeleteClip { track: 0, slot: 0 })
            .unwrap()
            .contains("deleted"));
        assert!(h.control.project.clip(0, 0).is_none());
        assert!(h.control.launches.iter().all(|l| l.track != 0));
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(notes(&h), [0, 2, 0], "UNDO: the clip is back");
        assert_eq!(h.control.project.clip(0, 0).unwrap().name, beat.name);
        h.act(TimeAction::Redo).unwrap();
        assert!(h.control.project.clip(0, 0).is_none());
        // An empty slot says so.
        assert!(h
            .act(TimeAction::DeleteClip { track: 0, slot: 0 })
            .unwrap_err()
            .contains("empty"));
        // A clip the song plays goes to the song's own clips (Arrange
        // 1.3): the song plays on, and UNDO puts it back in its cell.
        let p = &mut h.control.project;
        p.give_clip_ids();
        let block = mpfree_seq::arrange::Placement::of(&p.tracks, 0, 1, 0, 960);
        p.arrangement.placements.push(block);
        let said = h.act(TimeAction::DeleteClip { track: 0, slot: 1 }).unwrap();
        assert!(said.contains("the song keeps playing it"), "{said}");
        let p = &h.control.project;
        assert!(p.clip(0, 1).is_none());
        let b = &p.arrangement.placements[0];
        assert_eq!(p.block_clip(b).map(|c| c.name.as_str()), Some("fill"));
        h.act(TimeAction::Undo).unwrap();
        let p = &h.control.project;
        assert_eq!(p.clip(0, 1).map(|c| c.name.as_str()), Some("fill"));
        assert!(p.arrangement.clips.is_empty());
    }

    /// The clip settings card (sequencer S3): a row turned is one take
    /// however far it turns, the clip plays the new settings, UNDO puts
    /// them back; Length is Beats' length, for the focused clip only.
    #[test]
    fn the_clip_card_sets_a_clip_and_undo_takes_it_back() {
        use mpfree_seq::{ClipSettings, Speed};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.control.project.tracks[0].slots[0] = Some(Clip::new("beat", 3840));
        h.control.project.tracks[0].slots[1] = Some(Clip::new("fill", 960));
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        let st = h.state();
        assert_eq!(st.focus(), Some((0, 0)));
        let was = st.clip(0, 0).unwrap().settings;
        let speed = |num, den| TimeAction::ClipSettings {
            track: 0,
            slot: 0,
            row: 1,
            settings: ClipSettings {
                speed: Speed::new(num, den).unwrap(),
                ..was
            },
        };
        let compiles = h.control.costs.compiles;
        let said = h.act(speed(1, 2)).unwrap();
        assert!(said.contains("beat · Speed ×1/2"), "{said}");
        assert_eq!(h.control.costs.compiles, compiles + 1, "it plays at once");
        h.act(speed(1, 4)).unwrap();
        let st = h.state();
        assert_eq!(st.done.len(), 1, "the row turned twice: one take");
        assert_eq!(st.done[0].short(), "settings 1");
        assert_eq!(st.clip(0, 0).unwrap().speed, Speed::new(1, 4).unwrap());
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.state().clip(0, 0).unwrap().settings, was);
        // Length: Beats' length, the focused clip's.
        let said = h
            .act(TimeAction::ClipLength {
                track: 0,
                slot: 0,
                steps: 12,
                copy: false,
            })
            .unwrap();
        assert!(said.contains("12 steps"), "{said}");
        assert_eq!(h.control.project.clip(0, 0).unwrap().length, 12 * 240);
        assert!(h
            .act(TimeAction::ClipLength {
                track: 0,
                slot: 1,
                steps: 4,
                copy: false,
            })
            .unwrap_err()
            .contains("focused clip changed"));
        // An empty slot says so.
        assert!(h
            .act(TimeAction::ClipSettings {
                track: 0,
                slot: 5,
                row: 1,
                settings: was,
            })
            .unwrap_err()
            .contains("empty"));
    }

    /// COPY + a clip, then COPY + an empty slot (sequencer S3): a copy
    /// goes there as a take UNDO takes away; a full slot, a Loop track or
    /// nothing copied says why. SHIFT + Length's › doubles the clip with
    /// its notes repeated.
    #[test]
    fn copy_and_paste_a_clip_and_double_it() {
        use mpfree_seq::Note;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut beat = Clip::new("beat", 3840);
        beat.notes = vec![Note::new(0, 100, 0, 100), Note::new(960, 100, 2, 100)];
        h.control.project.tracks[0].slots[0] = Some(beat.clone());
        assert!(h
            .act(TimeAction::PasteClip { track: 1, slot: 0 })
            .unwrap_err()
            .contains("nothing copied"));
        let said = h.act(TimeAction::CopyClip { track: 0, slot: 0 }).unwrap();
        assert!(said.contains("beat copied"), "{said}");
        assert_eq!(h.state().clipboard.as_deref(), Some("beat"));
        assert!(h.state().done.is_empty(), "a copy is no take");
        let said = h.act(TimeAction::PasteClip { track: 1, slot: 2 }).unwrap();
        assert!(said.contains("a copy of beat in"), "{said}");
        // A copy: the same clip, but a clip of its own with its own id.
        let pasted = h.control.project.clip(1, 2).unwrap();
        assert_eq!(pasted.copied(), beat.copied());
        assert_ne!(pasted.id, h.control.project.clip(0, 0).unwrap().id);
        assert_eq!(h.state().done.last().unwrap().short(), "paste 1");
        assert!(h
            .act(TimeAction::PasteClip { track: 0, slot: 0 })
            .unwrap_err()
            .contains("holds a clip"));
        h.act(TimeAction::Undo).unwrap();
        assert!(h.control.project.clip(1, 2).is_none(), "UNDO takes it away");
        // Doubled, the notes repeat in the new bar.
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.act(TimeAction::ClipLength {
            track: 0,
            slot: 0,
            steps: 32,
            copy: true,
        })
        .unwrap();
        let c = h.control.project.clip(0, 0).unwrap();
        assert_eq!(c.length, 7680);
        let mut ticks: Vec<_> = c.notes.iter().map(|n| n.tick).collect();
        ticks.sort();
        assert_eq!(ticks, [0, 960, 3840, 4800]);
    }

    /// Jay, 14 Nov: drag and drop on the Clips grid. A playing clip
    /// dropped elsewhere keeps playing where it lands; onto another clip
    /// the two change places; a row moves; UNDO takes each back, the
    /// launch with it.
    #[test]
    fn a_dropped_clip_keeps_playing_where_it_lands_and_undo_takes_it_back() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        for (t, s, name) in [(0, 0, "Beat"), (0, 1, "Fill"), (1, 0, "Bass")] {
            h.control.project.tracks[t].slots[s] = Some(Clip::new(name, 1920));
        }
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        let playing = |h: &TimeHost| {
            h.control
                .launches
                .iter()
                .map(|l| (l.track, l.slot))
                .collect::<Vec<_>>()
        };
        assert_eq!(playing(&h), [(0, 0)]);
        let drop = |from, to, copy| TimeAction::DropClip { from, to, copy };
        let said = h.act(drop((0, 0), (0, 3), false)).unwrap();
        assert!(said.contains("Beat moved to Track 1 / 4"), "{said}");
        assert_eq!(playing(&h), [(0, 3)], "it plays on where it landed");
        let said = h.act(drop((0, 3), (0, 1), false)).unwrap();
        assert!(said.contains("Beat and Fill changed places"), "{said}");
        assert_eq!(playing(&h), [(0, 1)]);
        let said = h.act(drop((1, 0), (0, 1), true)).unwrap_err();
        assert!(said.contains("a copy goes in an empty slot"), "{said}");
        let said = h
            .act(TimeAction::DropRow {
                from: 1,
                to: 0,
                copy: false,
            })
            .unwrap();
        assert!(said.contains("row 2 moved to row 1"), "{said}");
        assert_eq!(h.control.project.clip(0, 0).unwrap().name, "Beat");
        assert_eq!(playing(&h), [(0, 0)]);
        let said = h.act(TimeAction::Undo).unwrap();
        assert!(said.contains("move 3 · row 2"), "{said}");
        assert_eq!(playing(&h), [(0, 1)]);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(playing(&h), [(0, 3)]);
        let said = h.act(TimeAction::Undo).unwrap();
        assert!(said.contains("move 1 · Track 1 / 1"), "{said}");
        assert_eq!(playing(&h), [(0, 0)]);
        assert_eq!(h.control.project.clip(0, 1).unwrap().name, "Fill");
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(playing(&h), [(0, 3)], "redo: it moves again, playing");
    }

    /// Jay, 14 Nov: drag and drop on the Song view. A block moves along
    /// and across lanes, gets longer, a section moves; each is an edit
    /// UNDO takes back, and a refusal changes nothing.
    #[test]
    fn song_drags_move_blocks_and_sections_and_undo_takes_them_back() {
        use mpfree_seq::arrange::Placement;
        const BAR: i64 = 3840;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        for (t, name) in [(0, "Beat"), (1, "Bass")] {
            h.control.project.tracks[t].slots[0] = Some(Clip::new(name, BAR));
        }
        h.control.project.give_clip_ids();
        let tracks = h.control.project.tracks.clone();
        let song = &mut h.control.project.arrangement;
        song.place(Placement::of(&tracks, 0, 0, 0, 2 * BAR))
            .unwrap();
        song.place(Placement::of(&tracks, 1, 0, 2 * BAR, 2 * BAR))
            .unwrap();
        song.mark(0, "Intro").unwrap();
        song.mark(2 * BAR, "Verse").unwrap();
        let start = h.control.project.arrangement.clone();
        let mv = |index, track, start, copy| TimeAction::MovePlacement {
            index,
            track,
            start,
            copy,
        };
        let said = h.act(mv(0, 0, 4 * BAR, false)).unwrap();
        assert!(said.contains("Beat moved to bar 5"), "{said}");
        assert_eq!(h.control.state().selected, Some(0), "it stays picked");
        // Onto the bass's block: refused, nothing changes.
        let before = h.control.project.arrangement.clone();
        let said = h.act(mv(0, 1, 2 * BAR, false)).unwrap_err();
        assert!(said.contains("would run into another block"), "{said}");
        assert_eq!(h.control.project.arrangement, before);
        // Onto an audio track's lane: refused, and it says why.
        let dest = h.control.project.tracks[0].dest;
        h.control
            .project
            .tracks
            .push(mpfree_seq::Track::new_audio("Audio 1", dest));
        let audio = h.control.project.tracks.len() - 1;
        let said = h.act(mv(0, audio, 0, true)).unwrap_err();
        assert!(said.contains("Audio 1 is an audio track"), "{said}");
        assert_eq!(h.control.project.arrangement, before);
        h.control.project.tracks.pop();
        // A copy onto the bass's lane stays linked to the beat and plays
        // through the bass (Arrange 1.3).
        let said = h.act(mv(0, 1, 6 * BAR, true)).unwrap();
        assert!(
            said.contains("a copy of Beat at Track 2, bar 7, playing through Track 2"),
            "{said}"
        );
        let p = &h.control.project;
        let copy = p.arrangement.at(1, 6 * BAR).unwrap();
        assert_eq!(
            p.arrangement.placements[copy].clip,
            p.clip(0, 0).unwrap().id
        );
        assert!(p.arrangement.clips.is_empty(), "no copy of the clip");
        let said = h
            .act(TimeAction::StretchPlacement {
                index: 0,
                length: 3 * BAR,
            })
            .unwrap();
        assert!(said.contains("Beat is 3 bars long now"), "{said}");
        // A beat on (the wheel).
        let said = h.act(mv(0, 0, 4 * BAR + 960, false)).unwrap();
        assert!(said.contains("bar 5 beat 2"), "{said}");
        let said = h
            .act(TimeAction::MoveSection {
                marker: 1,
                gap: 0,
                copy: false,
            })
            .unwrap();
        assert!(said.contains("Verse moved to bar 1"), "{said}");
        let st = h.control.state();
        assert_eq!(st.section, Some(0), "the section stays picked");
        assert_eq!(st.markers[0].1, "Verse");
        let said = h.act(TimeAction::Undo).unwrap();
        assert!(said.contains("move 5 · a section of the song"), "{said}");
        assert_eq!(h.control.state().markers[0].1, "Intro");
        for _ in 0..4 {
            h.act(TimeAction::Undo).unwrap();
        }
        assert_eq!(h.control.project.arrangement, start, "every edit undone");
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(h.control.project.arrangement.placements[0].start, 4 * BAR);
        // Deleting a block is an edit too.
        let said = h.act(TimeAction::DeletePlacement(0)).unwrap();
        assert!(said.contains("UNDO puts it back"), "{said}");
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.control.project.arrangement.placements.len(), 2);
    }

    /// The Arrange refactor's phase 6: Split at the playhead (the picked
    /// block, or every block across it), Duplicate, trim from the left
    /// and slide are each an edit UNDO takes back, and a refusal says why
    /// and changes nothing.
    #[test]
    fn split_duplicate_trim_and_slide_are_song_edits_undo_takes_back() {
        use mpfree_seq::arrange::Placement;
        const BAR: i64 = 3840;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        for (t, name) in [(0, "Beat"), (1, "Bass")] {
            h.control.project.tracks[t].slots[0] = Some(Clip::new(name, 2 * BAR));
        }
        h.control.project.give_clip_ids();
        let tracks = h.control.project.tracks.clone();
        let song = &mut h.control.project.arrangement;
        song.place(Placement::of(&tracks, 0, 0, 0, 4 * BAR))
            .unwrap();
        song.place(Placement::of(&tracks, 1, 0, 0, 2 * BAR))
            .unwrap();
        let start = h.control.project.arrangement.clone();
        let blocks = |h: &TimeHost| -> Vec<(usize, Tick, Tick, Tick)> {
            h.control
                .project
                .arrangement
                .placements
                .iter()
                .map(|p| (p.track, p.start, p.length, p.offset))
                .collect()
        };
        // The playhead at bar 2 beat 3 (a bar and a half in).
        h.command(Command::Locate(BAR + BAR / 2));
        run(&mut h, &mut pads, u64::from(PERIOD), &[]);
        assert_eq!(h.info().position, BAR + BAR / 2);
        // Split with nothing picked: both lanes.
        let said = h.act(TimeAction::SplitPlacement(None)).unwrap();
        assert!(said.contains("2 blocks split at bar 2 beat 3"), "{said}");
        assert_eq!(
            blocks(&h),
            [
                (0, 0, BAR + BAR / 2, 0),
                (0, BAR + BAR / 2, 2 * BAR + BAR / 2, BAR + BAR / 2),
                (1, 0, BAR + BAR / 2, 0),
                (1, BAR + BAR / 2, BAR / 2, BAR + BAR / 2),
            ]
        );
        let said = h.act(TimeAction::SplitPlacement(None)).unwrap_err();
        assert!(
            said.contains("no block plays across bar 2 beat 3"),
            "{said}"
        );
        let said = h.act(TimeAction::SplitPlacement(Some(0))).unwrap_err();
        assert!(said.contains("move the playhead inside Beat"), "{said}");
        let said = h.act(TimeAction::Undo).unwrap();
        assert!(said.contains("split"), "{said}");
        assert_eq!(h.control.project.arrangement, start);
        // Split the picked block alone: the part after is picked.
        h.act(TimeAction::SplitPlacement(Some(0))).unwrap();
        assert_eq!(h.control.state().selected, Some(1));
        assert_eq!(blocks(&h).len(), 3);
        h.act(TimeAction::Undo).unwrap();
        // Duplicate the bass: a copy after it, picked; again, the copy's
        // copy; the beat's has no room.
        let said = h.act(TimeAction::DuplicatePlacement(1)).unwrap();
        assert!(said.contains("a copy of Bass at bar 3"), "{said}");
        assert_eq!(blocks(&h)[2], (1, 2 * BAR, 2 * BAR, 0));
        assert_eq!(h.control.state().selected, Some(2));
        h.act(TimeAction::DuplicatePlacement(1)).unwrap_err();
        let said = h.act(TimeAction::DuplicatePlacement(9)).unwrap_err();
        assert!(said.contains("no such block"), "{said}");
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.control.project.arrangement, start);
        // Trim the beat from the left to bar 2: what plays stays put.
        let said = h
            .act(TimeAction::TrimPlacement {
                index: 0,
                start: BAR,
            })
            .unwrap();
        assert!(said.contains("Beat starts at bar 2 now"), "{said}");
        assert_eq!(blocks(&h)[0], (0, BAR, 3 * BAR, BAR));
        let said = h
            .act(TimeAction::TrimPlacement {
                index: 0,
                start: 4 * BAR,
            })
            .unwrap_err();
        assert!(said.contains("can't start at bar 5"), "{said}");
        // Slide what it plays a beat later.
        let said = h
            .act(TimeAction::SlidePlacement {
                index: 0,
                by: BAR / 4,
            })
            .unwrap();
        assert!(said.contains("slid 1 beat later"), "{said}");
        assert_eq!(blocks(&h)[0], (0, BAR, 3 * BAR, BAR - BAR / 4));
        h.act(TimeAction::Undo).unwrap();
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.control.project.arrangement, start, "every edit undone");
    }

    /// Several blocks picked (phase 6): moved, copied, duplicated and
    /// deleted as one song edit each, which one UNDO takes back; a
    /// refusal says why and changes nothing.
    #[test]
    fn several_blocks_are_one_song_edit_and_undo_takes_them_back() {
        use mpfree_seq::arrange::Placement;
        const BAR: i64 = 3840;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        for (t, name) in [(0, "Beat"), (1, "Bass")] {
            h.control.project.tracks[t].slots[0] = Some(Clip::new(name, BAR));
        }
        h.control.project.give_clip_ids();
        let tracks = h.control.project.tracks.clone();
        let song = &mut h.control.project.arrangement;
        song.place(Placement::of(&tracks, 0, 0, 0, BAR)).unwrap();
        song.place(Placement::of(&tracks, 0, 0, 4 * BAR, BAR))
            .unwrap();
        song.place(Placement::of(&tracks, 1, 0, BAR, BAR)).unwrap();
        let start = h.control.project.arrangement.clone();
        let blocks = |h: &TimeHost| -> Vec<(usize, Tick)> {
            h.control
                .project
                .arrangement
                .placements
                .iter()
                .map(|p| (p.track, p.start))
                .collect()
        };
        // The beat's first block and the bass's, two bars on.
        let two = vec![0, 2];
        let said = h
            .act(TimeAction::MovePlacements {
                set: two.clone(),
                lanes: 0,
                by: 2 * BAR,
                copy: false,
            })
            .unwrap();
        assert!(said.contains("2 blocks moved 2 bars later"), "{said}");
        assert_eq!(blocks(&h), [(0, 2 * BAR), (0, 4 * BAR), (1, 3 * BAR)]);
        let said = h.act(TimeAction::Undo).unwrap();
        assert!(said.contains("2 blocks in the song"), "{said}");
        assert_eq!(h.control.project.arrangement, start);
        // Copies four bars on: the beat's would land on its next block.
        let said = h
            .act(TimeAction::MovePlacements {
                set: two.clone(),
                lanes: 0,
                by: 4 * BAR,
                copy: true,
            })
            .unwrap_err();
        assert!(said.contains("don't fit there"), "{said}");
        assert_eq!(h.control.project.arrangement, start, "nothing changed");
        let said = h
            .act(TimeAction::MovePlacements {
                set: two.clone(),
                lanes: 0,
                by: 8 * BAR,
                copy: true,
            })
            .unwrap();
        assert!(said.contains("copies of 2 blocks 8 bars later"), "{said}");
        assert_eq!(blocks(&h).len(), 5);
        h.act(TimeAction::Undo).unwrap();
        // A 1/16 earlier would start before the song: a 1/16 later.
        h.act(TimeAction::MovePlacements {
            set: two.clone(),
            lanes: 0,
            by: -BAR / 16,
            copy: false,
        })
        .unwrap_err();
        let said = h
            .act(TimeAction::MovePlacements {
                set: two.clone(),
                lanes: 0,
                by: BAR / 16,
                copy: false,
            })
            .unwrap();
        assert!(said.contains("2 blocks moved 1/16 later"), "{said}");
        h.act(TimeAction::Undo).unwrap();
        // Duplicate: right after the pair, from bar 3.
        let said = h.act(TimeAction::DuplicatePlacements(two.clone())).unwrap();
        assert!(
            said.contains("copies of 2 blocks right after them"),
            "{said}"
        );
        assert_eq!(
            blocks(&h),
            [(0, 0), (0, 2 * BAR), (0, 4 * BAR), (1, BAR), (1, 3 * BAR)]
        );
        h.act(TimeAction::Undo).unwrap();
        // Delete both: one UNDO brings both back.
        let said = h.act(TimeAction::DeletePlacements(two)).unwrap();
        assert!(said.contains("2 blocks deleted"), "{said}");
        assert_eq!(blocks(&h), [(0, 4 * BAR)]);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.control.project.arrangement, start, "every edit undone");
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(blocks(&h), [(0, 4 * BAR)]);
    }

    /// Picking several blocks (6.5): SHIFT + a tap adds or takes out one,
    /// a box picks or adds; one left is the block selected. The blocks
    /// moved, copied or split stay picked where they are now; any other
    /// pick, an UNDO or a delete lets them go.
    #[test]
    fn several_blocks_are_picked_and_stay_picked_where_an_edit_puts_them() {
        use mpfree_seq::arrange::Placement;
        const BAR: i64 = 3840;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        for (t, name) in [(0, "Beat"), (1, "Bass")] {
            h.control.project.tracks[t].slots[0] = Some(Clip::new(name, 2 * BAR));
        }
        h.control.project.give_clip_ids();
        let tracks = h.control.project.tracks.clone();
        let song = &mut h.control.project.arrangement;
        song.place(Placement::of(&tracks, 0, 0, 0, 2 * BAR))
            .unwrap();
        song.place(Placement::of(&tracks, 0, 0, 4 * BAR, BAR))
            .unwrap();
        song.place(Placement::of(&tracks, 1, 0, BAR, 2 * BAR))
            .unwrap();
        let picked = |h: &TimeHost| {
            let st = h.control.state();
            (st.selected, st.picks.clone())
        };
        h.act(TimeAction::SelectPlacement(Some(0))).unwrap();
        // SHIFT + a tap on the bass's: both picked, none selected.
        h.act(TimeAction::PickPlacement(2)).unwrap();
        assert_eq!(picked(&h), (None, vec![0, 2]));
        // Again: taken out, the beat's alone is the block selected.
        h.act(TimeAction::PickPlacement(2)).unwrap();
        assert_eq!(picked(&h), (Some(0), vec![]));
        h.act(TimeAction::PickPlacement(9)).unwrap_err();
        // A box adds the beat's second block; one without SHIFT picks
        // only its own.
        h.act(TimeAction::PickPlacements {
            set: vec![1],
            add: true,
        })
        .unwrap();
        assert_eq!(picked(&h), (None, vec![0, 1]));
        h.act(TimeAction::PickPlacements {
            set: vec![0, 2],
            add: false,
        })
        .unwrap();
        assert_eq!(picked(&h), (None, vec![0, 2]));
        // Moved four bars on, they stay picked there.
        h.act(TimeAction::MovePlacements {
            set: vec![0, 2],
            lanes: 0,
            by: 8 * BAR,
            copy: false,
        })
        .unwrap();
        let st = h.control.state();
        let at: Vec<_> = st
            .picks
            .iter()
            .map(|&i| (st.placements[i].track, st.placements[i].start))
            .collect();
        assert_eq!(at, [(0, 8 * BAR), (1, 9 * BAR)]);
        // A refused move (to before the song) keeps them picked.
        h.act(TimeAction::MovePlacements {
            set: st.picks.clone(),
            lanes: 0,
            by: -20 * BAR,
            copy: false,
        })
        .unwrap_err();
        assert_eq!(h.control.state().picks, st.picks);
        // Split at bar 10 beat 1: the beat's block alone plays across it
        // (the bass's starts there); the part after is the block selected.
        h.command(Command::Locate(9 * BAR));
        run(&mut h, &mut pads, u64::from(PERIOD), &[]);
        assert_eq!(h.info().position, 9 * BAR);
        let said = h
            .act(TimeAction::SplitPlacements(st.picks.clone()))
            .unwrap();
        assert!(said.contains("1 block split at"), "{said}");
        let st = h.control.state();
        let after = st
            .selected
            .map(|i| (st.placements[i].track, st.placements[i].start));
        assert_eq!(after, Some((0, 9 * BAR)));
        assert!(st.picks.is_empty());
        // UNDO lets go of any pick.
        h.act(TimeAction::PickPlacements {
            set: vec![0, 1],
            add: false,
        })
        .unwrap();
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(picked(&h), (None, vec![]));
        // Duplicate several: the copies are picked; Split where none
        // plays says why.
        h.act(TimeAction::PickPlacements {
            set: vec![0, 1],
            add: false,
        })
        .unwrap();
        h.act(TimeAction::DuplicatePlacements(vec![0, 1])).unwrap();
        let st = h.control.state();
        assert_eq!(st.picks.len(), 2);
        assert!(st.picks.iter().all(|&i| st.placements[i].start >= 4 * BAR));
        h.command(Command::Locate(100 * BAR));
        run(&mut h, &mut pads, 2 * u64::from(PERIOD), &[]);
        let said = h
            .act(TimeAction::SplitPlacements(st.picks.clone()))
            .unwrap_err();
        assert!(said.contains("plays across"), "{said}");
        // Delete them: nothing stays picked.
        h.act(TimeAction::DeletePlacements(st.picks.clone()))
            .unwrap();
        assert_eq!(picked(&h), (None, vec![]));
    }

    /// A section moves with its tempo points (Arrange 3.2), one UNDO
    /// takes the whole move back, and a refusal says why in plain words.
    #[test]
    fn a_section_moves_with_its_tempo_and_undo_takes_it_back() {
        use mpfree_seq::arrange::Placement;
        const BAR: i64 = 3840;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.control.project.tracks[0].slots[0] = Some(Clip::new("Beat", BAR));
        h.control.project.give_clip_ids();
        let tracks = h.control.project.tracks.clone();
        let song = &mut h.control.project.arrangement;
        song.place(Placement::of(&tracks, 0, 0, 0, 4 * BAR))
            .unwrap();
        song.mark(0, "Intro").unwrap();
        song.mark(2 * BAR, "Verse").unwrap();
        song.set_tempo(2 * BAR, 90.0, None).unwrap();
        let start = h.control.project.arrangement.clone();
        let said = h
            .act(TimeAction::MoveSection {
                marker: 1,
                gap: 0,
                copy: false,
            })
            .unwrap();
        assert!(said.contains("Verse moved to bar 1"), "{said}");
        let song = &h.control.project.arrangement;
        let tempo: Vec<_> = song.tempo.iter().map(|p| (p.at, p.bpm)).collect();
        // The Verse's 90 BPM from bar 1, the Intro's 120 after it, and
        // past the song's end the 90 it had there.
        assert_eq!(tempo, [(0, 90.0), (2 * BAR, 120.0), (4 * BAR, 90.0)]);
        assert_eq!(song.placements.len(), 1, "the beat stays one block");
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.control.project.arrangement, start);
        // The most tempo points a song holds, the Verse starting between
        // two: refused, nothing changes.
        let song = &mut h.control.project.arrangement;
        song.tempo.clear();
        for k in 0..mpfree_seq::arrange::MAX_TEMPO_POINTS as i64 {
            let bpm = if k % 2 == 0 { 100.0 } else { 110.0 };
            song.set_tempo(k * 240, bpm, None).unwrap();
        }
        song.markers[1].at = 3 * 240 + 120;
        let before = h.control.project.arrangement.clone();
        let said = h
            .act(TimeAction::MoveSection {
                marker: 1,
                gap: 0,
                copy: false,
            })
            .unwrap_err();
        // The Verse's start and the place it left each need a point.
        assert_eq!(
            said,
            "song: Verse can't go there: the song would need more than 31 tempo points"
        );
        assert_eq!(h.control.project.arrangement, before);
    }

    /// Bars put in and taken out (Arrange 3.3): everything after moves,
    /// tempo points and markers too, each is one UNDO, and the undo list
    /// names it.
    #[test]
    fn bars_go_in_and_come_out_and_undo_takes_each_back() {
        use mpfree_seq::arrange::Placement;
        use mpfree_time_ui::BarsEdit;
        const BAR: i64 = 3840;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        for (t, name) in [(0, "Beat"), (1, "Bass")] {
            h.control.project.tracks[t].slots[0] = Some(Clip::new(name, BAR));
        }
        h.control.project.give_clip_ids();
        let tracks = h.control.project.tracks.clone();
        let song = &mut h.control.project.arrangement;
        song.place(Placement::of(&tracks, 0, 0, 0, 4 * BAR))
            .unwrap();
        song.place(Placement::of(&tracks, 1, 0, 4 * BAR, 2 * BAR))
            .unwrap();
        song.mark(0, "Intro").unwrap();
        song.mark(4 * BAR, "Verse").unwrap();
        song.set_tempo(4 * BAR, 100.0, None).unwrap();
        let start = h.control.project.arrangement.clone();
        let bars = |h: &mut TimeHost, e| h.act(TimeAction::SongBars(e));
        // Two bars for a bridge before the Verse.
        let said = bars(
            &mut h,
            BarsEdit::Insert {
                at: 4 * BAR,
                bars: 2,
            },
        )
        .unwrap();
        assert_eq!(
            said,
            "song: 2 empty bars at bar 5; everything after moved on (UNDO takes them out)"
        );
        let song = &h.control.project.arrangement;
        assert_eq!(song.markers[1].at, 6 * BAR, "the Verse moved on");
        assert_eq!(song.tempo[0].at, 6 * BAR, "and its tempo");
        assert_eq!(song.placements[1].start, 6 * BAR, "and the bass");
        let after_insert = song.clone();
        // A bar out of the Intro's beat.
        let said = bars(
            &mut h,
            BarsEdit::Delete {
                from: BAR,
                to: 2 * BAR,
            },
        )
        .unwrap();
        assert_eq!(
            said,
            "song: bar 2 taken out; everything after moved back (UNDO puts them back)"
        );
        let song = &h.control.project.arrangement;
        assert_eq!(song.placements[0].length, 3 * BAR, "the beat a bar shorter");
        assert_eq!(song.markers[1].at, 5 * BAR);
        // No bars at all: refused, nothing changes.
        let before = h.control.project.arrangement.clone();
        let said = bars(
            &mut h,
            BarsEdit::Delete {
                from: 2 * BAR,
                to: 2 * BAR,
            },
        )
        .unwrap_err();
        assert_eq!(said, "song: there are no bars to take out at bar 3");
        assert_eq!(h.control.project.arrangement, before);
        let st = h.control.state();
        let row = st.done.last().unwrap();
        assert_eq!(row.word(), "bars");
        assert_eq!(mpfree_time_ui::view::take_title(&st, row), "Bars 2 · bar 2");
        let said = h.act(TimeAction::Undo).unwrap();
        assert!(said.contains("the song's bars at bar 2"), "{said}");
        assert_eq!(h.control.project.arrangement, after_insert);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.control.project.arrangement, start, "both undone");
        // + Bar with the Verse picked, twice: two empty bars before it,
        // the Verse still picked, and one UNDO for both.
        let said = bars(&mut h, BarsEdit::RoomBefore(1)).unwrap();
        assert_eq!(
            said,
            "song: 1 empty bar at bar 5; everything after moved on (UNDO takes them out)"
        );
        assert_eq!(h.control.state().section, Some(1), "the Verse stays picked");
        let said = bars(&mut h, BarsEdit::RoomBefore(1)).unwrap();
        assert!(said.starts_with("song: 1 empty bar at bar 6;"), "{said}");
        let song = &h.control.project.arrangement;
        assert_eq!(
            (
                song.markers[1].at,
                song.placements[1].start,
                song.tempo[0].at
            ),
            (6 * BAR, 6 * BAR, 6 * BAR)
        );
        let st = h.control.state();
        let row = st.done.last().unwrap();
        assert_eq!(
            row.song,
            Some(mpfree_play::takes::SongChange::Bars {
                at: 4 * BAR,
                len: 2 * BAR,
                inserted: true
            }),
            "the undo list says 2 bars put in"
        );
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.control.project.arrangement, start, "one UNDO for both");
        // Delete bars on the Verse: its bars out, with its bass and
        // marker (what would follow keeps the tempo it had); UNDO puts
        // them back.
        let said = bars(&mut h, BarsEdit::DeleteSection(1)).unwrap();
        assert_eq!(
            said,
            "song: bars 5 to 6 taken out; everything after moved back (UNDO puts them back)"
        );
        let song = &h.control.project.arrangement;
        assert_eq!((song.placements.len(), song.markers.len()), (1, 1));
        let said = bars(&mut h, BarsEdit::DeleteSection(1)).unwrap_err();
        assert!(said.starts_with("song: "), "{said}");
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.control.project.arrangement, start);
    }

    /// Markers added, renamed, moved and taken away (Arrange 3.3): only the
    /// markers change, each is one UNDO, and names are kept.
    #[test]
    fn markers_are_added_renamed_moved_and_taken_away_each_one_undo() {
        use mpfree_seq::arrange::Placement;
        use mpfree_time_ui::MarkerEdit;
        const BAR: i64 = 3840;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.control.project.tracks[0].slots[0] = Some(Clip::new("Beat", BAR));
        h.control.project.give_clip_ids();
        let tracks = h.control.project.tracks.clone();
        let song = &mut h.control.project.arrangement;
        song.place(Placement::of(&tracks, 0, 0, 0, 8 * BAR))
            .unwrap();
        song.mark(0, "Intro").unwrap();
        let start = h.control.project.arrangement.clone();
        let marker = |h: &mut TimeHost, e| h.act(TimeAction::SongMarker(e));
        let names = |h: &TimeHost| -> Vec<(i64, String)> { h.control.state().markers.clone() };
        let said = marker(&mut h, MarkerEdit::Add(4 * BAR)).unwrap();
        assert_eq!(said, "song: marker A at bar 5 (UNDO takes it away)");
        assert_eq!(h.control.state().section, Some(1), "picked");
        let said = marker(&mut h, MarkerEdit::Add(4 * BAR)).unwrap_err();
        assert_eq!(said, "song: there's a marker at bar 5 already");
        let said = marker(
            &mut h,
            MarkerEdit::Rename {
                index: 1,
                name: " Bridge ".into(),
            },
        )
        .unwrap();
        assert_eq!(said, "song: A is Bridge now (UNDO gives its name back)");
        let said = marker(
            &mut h,
            MarkerEdit::Rename {
                index: 1,
                name: "  ".into(),
            },
        )
        .unwrap_err();
        assert_eq!(said, "song: Bridge needs a name");
        let said = marker(
            &mut h,
            MarkerEdit::Move {
                index: 1,
                at: 6 * BAR,
            },
        )
        .unwrap();
        assert!(said.starts_with("song: Bridge moved to bar 7;"), "{said}");
        assert_eq!(
            names(&h),
            [(0, "Intro".to_string()), (6 * BAR, "Bridge".to_string())]
        );
        let said = marker(&mut h, MarkerEdit::Move { index: 1, at: 0 }).unwrap_err();
        assert_eq!(said, "song: Bridge can't go onto another marker at bar 1");
        let said = marker(&mut h, MarkerEdit::Delete(1)).unwrap();
        assert_eq!(
            said,
            "song: marker Bridge taken away; its bars join Intro (UNDO puts it back)"
        );
        assert_eq!(
            h.control.project.arrangement.placements, start.placements,
            "only the markers changed"
        );
        let said = h.act(TimeAction::Undo).unwrap();
        assert!(said.contains("the song's marker at bar 7"), "{said}");
        assert_eq!(names(&h)[1].1, "Bridge", "back with its name");
        for _ in 0..3 {
            h.act(TimeAction::Undo).unwrap();
        }
        assert_eq!(h.control.project.arrangement, start, "every edit undone");
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(names(&h)[1], (4 * BAR, "A".to_string()));
    }

    #[test]
    fn plus_track_adds_an_armed_track_up_to_128_and_undo_takes_it_away() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        assert_eq!(h.control.project.tracks.len(), TRACKS);
        h.act(TimeAction::Arm(Some(2))).unwrap();
        let said = h.act(TimeAction::AddTrack).unwrap();
        assert!(said.contains("Track 9 added"), "{said}");
        assert_eq!(h.control.project.tracks.len(), TRACKS + 1);
        assert_eq!(h.control.project.tracks[TRACKS].name, "Track 9");
        assert_eq!(h.control.armed, Some(TRACKS));
        // A clip made on it and launched, then both undone: the clip, then
        // the track; the armed track comes back inside the project.
        h.act(TimeAction::NewClip {
            track: TRACKS,
            slot: 0,
        })
        .unwrap();
        h.act(TimeAction::DeleteClip {
            track: TRACKS,
            slot: 0,
        })
        .unwrap();
        h.act(TimeAction::Undo).unwrap();
        assert!(h.control.project.clip(TRACKS, 0).is_some());
        let said = h.act(TimeAction::Undo).unwrap();
        assert!(said.contains("new track 9"), "{said}");
        assert_eq!(h.control.project.tracks.len(), TRACKS);
        assert_eq!(h.control.armed, Some(TRACKS - 1));
        assert!(h.control.launches.iter().all(|l| l.track < TRACKS));
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(h.control.project.tracks.len(), TRACKS + 1);
        // Up to 128, then it says why not.
        while h.control.project.tracks.len() < mpfree_seq::MAX_TRACKS {
            h.act(TimeAction::AddTrack).unwrap();
        }
        assert_eq!(h.control.armed, Some(127));
        assert!(h
            .act(TimeAction::AddTrack)
            .unwrap_err()
            .contains("128 tracks"));
    }

    /// Refactor C1: a track selected apart from the arm (a track picked
    /// while REC's arm is pinned) is the one the Play page's steps show, a
    /// step tap edits and the note editor opens, while REC still records
    /// the armed track. Arming a track selects it again, and so does
    /// selecting the armed track; a pick on its own arms; a Loop track
    /// has no steps to select.
    #[test]
    fn a_track_selected_apart_from_the_arm_is_the_one_the_steps_edit() {
        use mpfree_time_ui::editor::EditAction as E;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        assert_eq!((h.armed(), h.selected()), (Some(0), Some(0)));
        let said = h.act(TimeAction::Select(2)).unwrap();
        assert_eq!(said, "selected: Track 3 (REC stays on Track 1, pinned)");
        assert_eq!((h.armed(), h.selected()), (Some(0), Some(2)));
        let st = h.state();
        assert_eq!(
            (st.armed, st.track_pick, st.selected()),
            (Some(0), Some(2), Some(2))
        );
        assert_eq!(h.steps().track, 2);
        assert_eq!(h.steps().track_name, "Track 3");
        // A step tapped goes into Track 3, which gets its first clip;
        // Track 1 stays empty.
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 0,
            velocity: 100,
        })
        .unwrap();
        let notes = |h: &TimeHost, t: usize| -> usize {
            h.project().tracks[t]
                .slots
                .clips()
                .map(|c| c.notes.len())
                .sum()
        };
        assert_eq!((notes(&h, 2), notes(&h, 0)), (1, 0));
        assert_eq!(h.steps().clip.map(|c| c.slot), Some(0));
        // The recorder still takes live notes for the armed track alone.
        h.audio.receive();
        assert_eq!(h.audio.armed(), Some(0));
        // The note editor opens on the selected track's clip.
        assert_eq!(
            h.act(TimeAction::Edit(E::Open)).unwrap(),
            "edit: Pattern 1 on Track 3"
        );
        // Arming a track selects it too.
        h.act(TimeAction::Arm(Some(1))).unwrap();
        assert_eq!((h.armed(), h.selected()), (Some(1), Some(1)));
        assert_eq!(h.state().track_pick, None);
        // Selecting the armed track is the arm's selection again.
        h.act(TimeAction::Select(3)).unwrap();
        assert_eq!(h.act(TimeAction::Select(1)).unwrap(), "selected: Track 2");
        assert_eq!((h.state().track_pick, h.selected()), (None, Some(1)));
        // A pick on its own (the app found no pin) arms and selects.
        h.act(TimeAction::Pick(4)).unwrap();
        assert_eq!((h.armed(), h.selected()), (Some(4), Some(4)));
        // Letting go of the arm leaves nothing selected apart from it.
        h.act(TimeAction::Select(5)).unwrap();
        h.act(TimeAction::Arm(None)).unwrap();
        assert_eq!((h.armed(), h.selected()), (None, None));
        // No such track, and a Loop track, select nothing.
        h.act(TimeAction::Arm(Some(0))).unwrap();
        assert!(h.act(TimeAction::Select(99)).is_err());
        h.act(TimeAction::AddLoopTrack).unwrap();
        let loops = h.project().tracks.len() - 1;
        h.act(TimeAction::Arm(Some(0))).unwrap();
        let said = h.act(TimeAction::Select(loops)).unwrap();
        assert!(
            said.ends_with("plays loops: the Sequencer stays on Track 1"),
            "{said}"
        );
        assert_eq!(h.selected(), Some(0));
    }

    /// Refactor C1: the armed and the selected track are kept by their
    /// lasting ids, never by their places: a track added before them, one
    /// taken away from between them, UNDO taking away the armed track and
    /// REDO bringing it back each leave the arm and the selection on their
    /// own tracks (or, for a track that is gone, the arm on the last track
    /// and the selection on the armed one); a project opened starts with
    /// track 1 armed and selected.
    #[test]
    fn the_arm_and_the_selection_stay_on_their_own_tracks() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.act(TimeAction::Arm(Some(2))).unwrap();
        h.act(TimeAction::Select(4)).unwrap();
        let id = |h: &TimeHost, t: Option<usize>| t.map(|t| h.project().tracks[t].id);
        let (armed, picked) = (id(&h, h.armed()), id(&h, h.selected()));
        let names = |h: &TimeHost| {
            let name = |t: Option<usize>| t.map(|t| h.project().tracks[t].name.clone());
            (name(h.armed()), name(h.selected()))
        };
        let both = (Some("Track 3".to_string()), Some("Track 5".to_string()));
        assert_eq!(names(&h), both);
        // A track taken away from before both (as a delete would): each
        // keeps its track, one place up.
        h.control.project.tracks.remove(1);
        h.control.compile().unwrap();
        assert_eq!((h.armed(), h.selected()), (Some(1), Some(3)));
        assert_eq!(names(&h), both);
        // A track put in at the start: one place down.
        let mut first = Track::new("New", 0);
        h.control.project.give_track_id(&mut first);
        h.control.project.tracks.insert(0, first);
        h.control.compile().unwrap();
        assert_eq!((h.armed(), h.selected()), (Some(2), Some(4)));
        assert_eq!((id(&h, h.armed()), id(&h, h.selected())), (armed, picked));
        // The selected track gone: the selection is the armed track.
        h.control.project.tracks.remove(4);
        h.control.compile().unwrap();
        assert_eq!((h.armed(), h.selected()), (Some(2), Some(2)));
        assert_eq!(h.state().track_pick, None);
        // + Track arms and selects the new track; with another track
        // selected apart from it, UNDO takes the armed track away: the arm
        // goes to the last track, and the selection stays on its own.
        h.act(TimeAction::AddTrack).unwrap();
        let added = h.project().tracks.len() - 1;
        assert_eq!((h.armed(), h.selected()), (Some(added), Some(added)));
        h.act(TimeAction::Select(0)).unwrap();
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.project().tracks.len(), added);
        assert_eq!((h.armed(), h.selected()), (Some(added - 1), Some(0)));
        assert_eq!(names(&h).1.as_deref(), Some("New"));
        // REDO brings the track back with its id; the arm, not an undo
        // step, stays where it went, and the selection on its track.
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(h.project().tracks.len(), added + 1);
        assert_eq!((h.armed(), h.selected()), (Some(added - 1), Some(0)));
        // A project opened: track 1 armed and selected, whatever its ids.
        let mut p = h.project().clone();
        p.tracks.swap(0, 3);
        let mut opened = TimeHost::with_project(RATE, 120.0, p).unwrap();
        assert_eq!((opened.armed(), opened.selected()), (Some(0), Some(0)));
        assert_eq!(opened.state().track_pick, None);
        assert_eq!(opened.steps().track, 0);
        opened.act(TimeAction::Select(1)).unwrap();
        assert_eq!(opened.steps().track, 1);
    }

    const RATE: u32 = 44_100;
    const PERIOD: u32 = 128;

    #[derive(Default)]
    struct Fake {
        hits: Vec<(u64, usize, u8)>,
        /// The track of each of `hits`.
        hit_tracks: Vec<u16>,
        /// (absolute frame, pad) of each release.
        releases: Vec<(u64, usize)>,
        frame: u64,
        stops: usize,
        /// (absolute frame, pad, value)
        pressure: Vec<(u64, usize, u8)>,
        /// The pads' sound locks taken: (absolute frame, hits before it,
        /// address, value or None for its end).
        locks: Vec<(
            u64,
            usize,
            mpfree_play::mpfree_param::ParamAddr,
            Option<f32>,
        )>,
        /// (track, mixer track): where a track's instrument sounds
        /// ([`Pads::sounds_on`]); a track not here, on its own strip.
        sounds: Vec<(u16, u8)>,
        /// A live pad plays these notes above its number (a chord of an
        /// instrument's pad map, step 5c); empty: its number alone.
        chord: Vec<u8>,
    }

    impl Pads for Fake {
        fn sounds_on(&self, track: u16) -> Option<u8> {
            self.sounds
                .iter()
                .find(|(t, _)| *t == track)
                .map(|&(_, s)| s)
        }
        fn hit_at(&mut self, track: u16, pad: usize, velocity: u8, offset: u32) -> bool {
            self.hits
                .push((self.frame + u64::from(offset), pad, velocity));
            self.hit_tracks.push(track);
            true
        }
        fn release_at(&mut self, _track: u16, pad: usize, offset: u32) {
            self.releases.push((self.frame + u64::from(offset), pad));
        }
        fn stop_all(&mut self) {
            self.stops += 1;
        }
        fn pressure_at(&mut self, _track: u16, pad: usize, value: u8, offset: u32) {
            self.pressure
                .push((self.frame + u64::from(offset), pad, value));
        }
        fn lock_at(
            &mut self,
            addr: mpfree_play::mpfree_param::ParamAddr,
            value: Option<f32>,
            offset: u32,
        ) -> bool {
            use mpfree_play::mpfree_param::Owner;
            if !matches!(addr.owner, Owner::Pad(..)) {
                return false;
            }
            let at = self.frame + u64::from(offset);
            self.locks.push((at, self.hits.len(), addr, value));
            true
        }
        // As the default, but a pad's notes are its chord's.
        fn live(&mut self, o: &Out) -> Heard {
            let (note, play) = match o.event {
                Event::NoteOn { note, velocity } => (
                    note,
                    self.hit_at(o.track, usize::from(note), velocity, o.offset),
                ),
                Event::NoteOff { note } => {
                    self.release_at(o.track, usize::from(note), o.offset);
                    (note, true)
                }
                _ => return Heard::NONE,
            };
            let notes: Vec<u8> = if self.chord.is_empty() {
                vec![note]
            } else {
                self.chord.iter().map(|&k| note + k).collect()
            };
            if !play {
                Heard::NONE
            } else if o.notes {
                // An arp working in notes (step 5d): a note, no pad.
                Heard::of(None, o.event, &[note])
            } else {
                Heard::of(Some(o.event), o.event, &notes)
            }
        }
    }

    /// Run until frame `to`, pressing `presses` (absolute frame, pad, velocity, frames held) on the way.
    fn run(h: &mut TimeHost, pads: &mut Fake, to: u64, presses: &[(u64, u8, u8, u64)]) {
        while pads.frame < to {
            let end = pads.frame + u64::from(PERIOD);
            for &(at, pad, v, held) in presses {
                if (pads.frame..end).contains(&at) {
                    h.pad(
                        (at - pads.frame) as u32,
                        Event::NoteOn {
                            note: pad,
                            velocity: v,
                        },
                    );
                }
                if (pads.frame..end).contains(&(at + held)) {
                    h.pad(
                        (at + held - pads.frame) as u32,
                        Event::NoteOff { note: pad },
                    );
                }
            }
            h.period(PERIOD, pads);
            pads.frame = end;
        }
    }

    /// Frames per bar at 120 BPM.
    const BAR: u64 = 88_200;

    /// Outside gear on every track (sequencer S5's tests): `Pads::out`
    /// takes every clip event, as MIDI out does for a track on a port.
    /// `midi` is what MIDI out was handed: the frame it plays on, the
    /// first frame of the period it was handed over in, the event.
    #[derive(Default)]
    struct Gear {
        frame: u64,
        midi: Vec<(u64, u64, Event)>,
        /// Real-pass events that had gone out ahead already.
        sent: usize,
    }

    impl Pads for Gear {
        fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
            true
        }
        fn release_at(&mut self, _: u16, _: usize, _: u32) {}
        fn stop_all(&mut self) {}
        fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        fn out(&mut self, o: &Out) -> bool {
            self.midi
                .push((self.frame + u64::from(o.offset), self.frame, o.event));
            o.track != mpfree_play::ALL
        }
        fn out_ahead(&mut self, o: &Out) -> bool {
            self.out(o)
        }
        fn out_sent(&mut self, _: &Out) {
            self.sent += 1;
        }
    }

    impl Gear {
        /// Periods until frame `to`.
        fn run(&mut self, h: &mut TimeHost, to: u64) {
            while self.frame < to {
                h.period(PERIOD, self);
                self.frame += u64::from(PERIOD);
            }
        }

        /// The notes as they will play: (frame, event), in frame order
        /// (the order handed over within a frame), before frame `to`.
        fn notes(&self, to: u64) -> Vec<(u64, Event)> {
            let mut v: Vec<(u64, Event)> = self
                .midi
                .iter()
                .filter(|m| m.0 < to)
                .map(|&(f, _, e)| (f, e))
                .collect();
            v.sort_by_key(|m| m.0);
            v
        }

        /// Notes still on at the end, on gear `ahead` frames slow: each
        /// note-on not followed by its note-off (or an All off) in the
        /// order they leave, as MIDI out sends them: each its ahead before
        /// its frame, never before the period it was handed over in, and
        /// in the order handed over when two leave together.
        fn hanging(&self, ahead: u64) -> Vec<u8> {
            let mut left: Vec<(u64, Event)> = self
                .midi
                .iter()
                .map(|&(f, at, e)| (f.saturating_sub(ahead).max(at), e))
                .collect();
            left.sort_by_key(|m| m.0);
            let mut on = [0i32; 128];
            for (_, e) in left {
                match e {
                    Event::NoteOn { note, .. } => on[usize::from(note)] += 1,
                    Event::NoteOff { note } => {
                        on[usize::from(note)] = (on[usize::from(note)] - 1).max(0)
                    }
                    Event::AllOff => on = [0; 128],
                    _ => {}
                }
            }
            (0..128u8).filter(|&n| on[usize::from(n)] > 0).collect()
        }
    }

    /// A bar of 16ths on two pads, some overlapping, on the armed track.
    fn gear_host() -> TimeHost {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        for (pad, steps) in [
            (2u8, &[0usize, 3, 4, 8, 11, 12, 14][..]),
            (5, &[2, 6, 10, 15]),
        ] {
            for &step in steps {
                h.step_action(StepAction::Tap {
                    pad,
                    step,
                    velocity: 100,
                })
                .unwrap();
            }
        }
        h
    }

    /// Sequencer S5: a track on slow gear hands its clips' notes to MIDI
    /// out its ahead before their frame, each once, on the frames the
    /// real pass plays them; the real pass then strikes each off.
    #[test]
    fn a_track_on_slow_gear_hands_its_notes_over_ahead_each_once() {
        const AHEAD: u64 = 1_000;
        let play = |ahead: u32| {
            let mut h = gear_host();
            h.set_ahead(&[ahead]);
            h.command(Command::Play);
            let mut g = Gear::default();
            g.run(&mut h, 2 * BAR + 4 * AHEAD);
            (g, h.ahead_counts())
        };
        let (plain, none) = play(0);
        let (gear, counts) = play(AHEAD as u32);
        assert_eq!(none.ahead, 0);
        assert_eq!(
            gear.notes(2 * BAR),
            plain.notes(2 * BAR),
            "the same notes, frames"
        );
        assert!(plain.notes(2 * BAR).len() >= 22, "two bars of notes");
        // Once the forecast runs (a period and the ahead after PLAY), each
        // is handed over in the period that holds its frame less the ahead.
        for &(f, at, e) in gear
            .midi
            .iter()
            .filter(|m| m.0 >= AHEAD + 2 * u64::from(PERIOD))
        {
            assert!(
                at <= f - AHEAD && f - AHEAD < at + u64::from(PERIOD),
                "{e:?} on {f} handed over at {at}"
            );
        }
        assert!(counts.ahead > 20, "{counts:?}");
        assert_eq!((counts.late, counts.wrong, counts.dropped), (0, 0, 0));
        assert_eq!(
            gear.sent as u64,
            counts.ahead - gear.midi.iter().filter(|m| m.0 >= gear.frame).count() as u64
        );
        assert!(
            gear.hanging(AHEAD).len() <= 1,
            "at most the note sounding at the end"
        );
    }

    /// Sequencer S5: what the forecast can't know. A step added just ahead
    /// of the playhead goes out late, when the real pass plays it; STOP
    /// leaves no note on, though notes went out ahead of it.
    #[test]
    fn slow_gear_gets_a_late_edit_late_and_no_note_left_on_at_stop() {
        const AHEAD: u64 = 3_000;
        let mut h = gear_host();
        h.set_ahead(&[AHEAD as u32]);
        h.command(Command::Play);
        let mut g = Gear::default();
        // To 2,000 frames before step 8 of the second bar (a step is
        // 5,512.5 frames), so its frame is in what went out ahead already.
        g.run(&mut h, BAR + 7 * 5_513 - 2_000);
        h.step_action(StepAction::Tap {
            pad: 9,
            step: 7,
            velocity: 90,
        })
        .unwrap();
        g.run(&mut h, BAR + 8 * 5_513);
        let late = g
            .midi
            .iter()
            .find(|m| {
                m.2 == Event::NoteOn {
                    note: 9,
                    velocity: 90,
                }
            })
            .copied()
            .expect("the new step played");
        assert!(
            late.1 + u64::from(PERIOD) > late.0,
            "handed over in its own period: {late:?}"
        );
        assert!(h.ahead_counts().late >= 1, "{:?}", h.ahead_counts());
        // STOP 1,500 frames before step 13, whose notes went out ahead:
        // they end, and so does the note sounding.
        g.run(&mut h, BAR + 12 * 5_513 - 1_500);
        h.command(Command::Stop);
        g.run(&mut h, 2 * BAR + 4 * AHEAD);
        assert_eq!(g.hanging(AHEAD), Vec::<u8>::new(), "{:?}", h.ahead_counts());
        assert!(
            h.ahead_counts().wrong >= 1,
            "notes ahead of the stop: {:?}",
            h.ahead_counts()
        );
    }

    /// Sequencer S5's controls ahead: a lock on the track's own MIDI out
    /// control goes ahead with its note, once; one taken off its step
    /// after it went ahead is put back (its unlock) when the real pass
    /// doesn't play it.
    #[test]
    fn slow_gear_gets_a_lock_ahead_once_and_its_unlock_when_the_lock_is_taken_off() {
        use mpfree_play::mpfree_param::{OnTrack, Owner, ParamAddr, RelAddr};
        const AHEAD: u64 = 3_000;
        const STEP: f64 = 5_512.5;
        let mut h = gear_host();
        let cc = RelAddr::new(OnTrack::Midi, 74);
        let addr = ParamAddr::new(Owner::Midi(0), 74);
        h.step_action(StepAction::Lock {
            pad: 2,
            steps: 1 << 8,
            addr: cc,
            value: Some(100.0),
        })
        .unwrap();
        h.set_ahead(&[AHEAD as u32]);
        h.command(Command::Play);
        let mut g = Gear::default();
        let locks = |g: &Gear| -> Vec<(u64, u64, Event)> {
            g.midi
                .iter()
                .filter(|m| matches!(m.2, Event::Lock { .. } | Event::Unlock { .. }))
                .copied()
                .collect()
        };
        // The first pass: the lock goes out ahead, with step 9's note.
        g.run(&mut h, BAR);
        let first = locks(&g);
        let step_9 = (8.0 * STEP).round() as u64;
        let lock = first
            .iter()
            .find(|m| m.2 == Event::Lock { addr, value: 100.0 })
            .copied()
            .expect("the lock");
        assert!(lock.0.abs_diff(step_9) <= 1, "{first:?}");
        assert!(lock.1 + AHEAD <= lock.0, "ahead: {lock:?}");
        let note = g
            .midi
            .iter()
            .find(|m| m.0 == lock.0 && matches!(m.2, Event::NoteOn { note: 2, .. }))
            .expect("step 9's note");
        assert_eq!(note.1, lock.1, "handed over with its note");
        assert_eq!(
            first.iter().filter(|m| m.2 == lock.2).count(),
            1,
            "once: {first:?}"
        );
        assert_eq!(h.ahead_counts().late, 0, "{:?}", h.ahead_counts());
        // The second pass: taken off its step 2,000 frames before it, after
        // it went ahead. It isn't played, so its unlock follows.
        let at = BAR + step_9 - 2_000;
        g.run(&mut h, at);
        h.step_action(StepAction::Lock {
            pad: 2,
            steps: 1 << 8,
            addr: cc,
            value: None,
        })
        .unwrap();
        let before = g.midi.len();
        g.run(&mut h, 2 * BAR);
        let second: Vec<_> = g.midi[before..]
            .iter()
            .filter(|m| matches!(m.2, Event::Lock { .. } | Event::Unlock { .. }))
            .copied()
            .collect();
        assert!(
            matches!(second[..], [(_, _, Event::Unlock { addr: a })] if a == addr),
            "only its unlock: {second:?}"
        );
        // In the period that held the lock's frame, from its start.
        assert!(second[0].0 + u64::from(PERIOD) > BAR + step_9, "{second:?}");
        assert!(h.ahead_counts().wrong >= 1, "{:?}", h.ahead_counts());
    }

    /// Perform FX gestures at frames: (frame, value or None for letting go).
    fn gestures(
        h: &mut TimeHost,
        pads: &mut Fake,
        to: u64,
        addr: mpfree_play::mpfree_param::ParamAddr,
        moves: &[(u64, Option<f32>)],
    ) {
        use mpfree_seq::mpfree_moves::Map;
        while pads.frame < to {
            let end = pads.frame + u64::from(PERIOD);
            for &(at, v) in moves {
                if (pads.frame..end).contains(&at) {
                    let input = match v {
                        Some(value) => Input::Move {
                            addr,
                            value,
                            map: Map::linear(0.0, 1.0),
                        },
                        None => Input::End { addr },
                    };
                    h.gesture((at - pads.frame) as u32, input);
                }
            }
            h.period(PERIOD, pads);
            pads.frame = end;
        }
    }

    /// The XY pad moved while recording becomes a move in the armed
    /// track's clip, plays back as lane values, and undoes with its take.
    #[test]
    fn a_perform_fx_gesture_while_recording_becomes_a_move() {
        use mpfree_play::mpfree_param::{Owner, ParamAddr, Strip};
        use mpfree_seq::mpfree_moves::Target;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::SetLoop(Some((0, 3840))));
        h.act(TimeAction::NewClip { track: 0, slot: 0 }).unwrap();
        let xy = ParamAddr::new(Owner::Insert(Strip::Master, 1), 0);
        let moves: Vec<(u64, Option<f32>)> = (0..20)
            .map(|i| (1000 + i * 2000, Some(i as f32 / 20.0)))
            .chain([(45_000, None)])
            .collect();
        gestures(&mut h, &mut pads, BAR - 1000, xy, &moves);
        let lane = |h: &TimeHost| {
            h.control
                .project
                .clip(0, 0)
                .unwrap()
                .moves
                .lane(Target::Fixed(xy))
                .map(|l| l.points.len())
        };
        assert!(lane(&h).is_some_and(|n| n >= 2), "{:?}", lane(&h));
        assert_eq!(*h.audio.view, {
            let mut v =
                PlayView::compile_cued(&h.control.project, &h.control.launches, &h.control.cues)
                    .unwrap();
            v.set_armed(h.control.armed);
            v.make_room(RECORD_ROOM, RECORD_ROOM);
            v
        });
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        // The next pass plays it as lane values.
        let mut lanes = 0;
        while pads.frame < 2 * BAR {
            h.period(PERIOD, &mut pads);
            pads.frame += u64::from(PERIOD);
            lanes += h
                .audio
                .sink
                .events()
                .iter()
                .filter(|o| matches!(o.event, Event::Lane { addr, .. } if addr == xy))
                .count();
        }
        assert!(lanes > 20, "{lanes}");
        assert_eq!(h.act(TimeAction::Undo).unwrap(), "undo: take 1");
        assert_eq!(lane(&h), None);
        assert_eq!(h.dropped(), 0);
    }

    /// An XY pad gesture across a looping clip's end while recording (the
    /// XY plan's phase 3, 3.1): its moves land at the clip's end and go
    /// on from its start, one lane.
    #[test]
    fn an_xy_gesture_across_the_clips_loop_point_records_on_both_sides() {
        use mpfree_play::mpfree_param::{Owner, ParamAddr};
        use mpfree_seq::mpfree_moves::{Kind, Target};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.act(TimeAction::NewClip { track: 0, slot: 0 }).unwrap();
        let len = h.control.project.clip(0, 0).unwrap().length;
        assert_eq!(len, 7680, "a new clip: two bars");
        let knob = ParamAddr::new(Owner::Inst(0), 5);
        // From a quarter bar before the clip's end to a quarter bar into
        // its next pass, every 50 ms, then let go.
        let moves: Vec<(u64, Option<f32>)> = (0..20)
            .map(|i| (BAR * 7 / 4 + i * 2205, Some(0.2 + 0.03 * i as f32)))
            .chain([(BAR * 9 / 4 + 2205, None)])
            .collect();
        gestures(&mut h, &mut pads, BAR * 5 / 2, knob, &moves);
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        let clip = h.control.project.clip(0, 0).unwrap();
        assert_eq!(clip.length, len, "the clip looped");
        let lane = clip
            .moves
            .lane(Target::for_track(knob, 0))
            .expect("the knob's lane, as the track's own");
        let ticks: Vec<u32> = lane
            .points
            .iter()
            .filter(|p| p.kind != Kind::End)
            .map(|p| p.tick)
            .collect();
        assert!(
            ticks.iter().any(|&t| (6_720..7_680).contains(&t)) && ticks.iter().any(|&t| t < 960),
            "both sides of the loop point: {ticks:?}"
        );
        assert!(
            ticks.iter().all(|&t| !(1_100..6_700).contains(&t)),
            "nothing in between: {ticks:?}"
        );
        assert_eq!(h.dropped(), 0);
    }

    /// Live erase (sequencer S1, ERASE held on a pad while recording): the
    /// pad's notes come out of the clip as the playhead passes them, the
    /// other pads' stay; let go, nothing more is erased.
    #[test]
    fn live_erase_takes_a_pads_notes_out_while_recording() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::SetLoop(Some((0, 3840))));
        h.act(TimeAction::NewClip { track: 0, slot: 0 }).unwrap();
        run(
            &mut h,
            &mut pads,
            BAR - 1000,
            &[(22_969, 5, 100, 2000), (26_414, 6, 100, 2000)],
        );
        let pads_in = |h: &TimeHost| {
            let mut n: Vec<u8> = h
                .control
                .project
                .clip(0, 0)
                .unwrap()
                .notes
                .iter()
                .map(|n| n.note)
                .collect();
            n.sort();
            n
        };
        assert_eq!(pads_in(&h), [5, 6]);
        assert!(!h.erasing());
        // The second pass with ERASE + pad 5 held.
        h.erase_sound(5, true);
        assert!(h.erasing() && h.erases(5) && !h.erases(6));
        run(&mut h, &mut pads, 2 * BAR - 1000, &[]);
        assert_eq!(pads_in(&h), [6], "pad 5's note came out");
        h.erase_sound(5, false);
        assert!(!h.erasing());
        h.erase_sound(6, true);
        h.erase_none();
        run(&mut h, &mut pads, 3 * BAR - 1000, &[]);
        assert_eq!(pads_in(&h), [6], "let go: nothing more");
        assert_eq!(h.dropped(), 0);
    }

    /// Live erase while only playing (the button combos of 19 Nov): with no
    /// take recording, the hold is a take of its own, "erase 1", which UNDO
    /// brings back, leaving the recorded take as it was.
    #[test]
    fn live_erase_while_only_playing_is_a_take_of_its_own() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::SetLoop(Some((0, 3840))));
        h.act(TimeAction::NewClip { track: 0, slot: 0 }).unwrap();
        run(
            &mut h,
            &mut pads,
            BAR - 1000,
            &[(22_969, 5, 100, 2000), (26_414, 6, 100, 2000)],
        );
        let pads_in = |h: &TimeHost| {
            let mut n: Vec<u8> = h
                .control
                .project
                .clip(0, 0)
                .unwrap()
                .notes
                .iter()
                .map(|n| n.note)
                .collect();
            n.sort();
            n
        };
        assert_eq!(pads_in(&h), [5, 6]);
        // Punch out: it plays on.
        h.command(Command::Record);
        run(&mut h, &mut pads, BAR + 1000, &[]);
        assert_eq!(h.info().state, State::Playing);
        h.erase_sound(5, true);
        run(&mut h, &mut pads, 2 * BAR - 1000, &[]);
        h.erase_sound(5, false);
        assert_eq!(pads_in(&h), [6], "pad 5's note came out while playing");
        let said = h.act(TimeAction::Undo).unwrap();
        assert!(said.starts_with("undo: erase 1"), "{said}");
        assert_eq!(pads_in(&h), [5, 6], "UNDO brought it back");
        let said = h.act(TimeAction::Undo).unwrap();
        assert!(said.starts_with("undo: take 1"), "{said}");
        assert_eq!(h.dropped(), 0);
    }

    /// Record quantise (sequencer S3): with the Record list's Quantise at
    /// 1/16, a note played between two lines goes in on the nearest; off,
    /// the notes go in as played.
    #[test]
    fn record_quantise_puts_the_recorded_notes_on_the_grid() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::SetLoop(Some((0, 3840))));
        h.act(TimeAction::NewClip { track: 0, slot: 0 }).unwrap();
        let said = h
            .act(TimeAction::RecordQuantise {
                track: 0,
                grid: Some(240),
            })
            .unwrap();
        assert_eq!(
            said,
            "record quantise: Track 1 records each note on the nearest 1/16"
        );
        assert_eq!(h.state().tracks[0].record_quantise, Some(240));
        // Ticks 1000 and 1150: nearest 960 and 1200.
        run(
            &mut h,
            &mut pads,
            BAR - 1000,
            &[(22_969, 5, 100, 2000), (26_414, 6, 100, 2000)],
        );
        let clip = h.control.project.clip(0, 0).unwrap();
        let mut ticks: Vec<_> = clip.notes.iter().map(|n| (n.note, n.tick)).collect();
        ticks.sort();
        assert_eq!(ticks, [(5, 960), (6, 1200)]);
        assert_eq!(
            h.act(TimeAction::RecordQuantise {
                track: 0,
                grid: None
            })
            .unwrap(),
            "record quantise: Track 1 records the notes as played"
        );
        assert_eq!(h.state().tracks[0].record_quantise, None);
        assert!(h
            .act(TimeAction::RecordQuantise {
                track: 0,
                grid: Some(0)
            })
            .is_err());
    }

    /// Pad pressure while recording goes on the pad's note in the clip
    /// (the Record list's "Pressure" row is on for a new track), plays
    /// back to the pads on the next pass, and is left out with the row off.
    /// Refactor B4b: a clip past half its pressure room frees what no
    /// note names once the recording stops, the view is compiled again
    /// with the new numbering, and the note still plays its pressure.
    #[test]
    fn unused_pressure_is_freed_once_the_recording_stops() {
        use mpfree_seq::TouchPoint;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::SetLoop(Some((0, 3840))));
        h.act(TimeAction::NewClip { track: 0, slot: 0 }).unwrap();
        // Pressure no note names, as erased notes whose takes are gone
        // leave behind: past half the room.
        let chunk: Vec<TouchPoint> = (0..1000)
            .map(|k| TouchPoint {
                at: k,
                value: (k % 128) as u8,
            })
            .collect();
        let clip = h.control.project.tracks[0].slots[0].as_mut().unwrap();
        for _ in 0..33 {
            clip.add_touch(&chunk).unwrap();
        }
        assert!(clip.touch.len() > mpfree_play::TOUCH_RECLAIM_AT);
        h.control.compile().unwrap();
        h.audio.receive();
        // A note pressed harder and let up, recorded.
        let squeeze = [(12_000u64, 40u8), (20_000, 90), (30_000, 20)];
        while pads.frame < BAR - 1000 {
            let end = pads.frame + u64::from(PERIOD);
            let here = |at: u64| {
                (pads.frame..end)
                    .contains(&at)
                    .then(|| (at - pads.frame) as u32)
            };
            if let Some(o) = here(10_000) {
                h.pad(
                    o,
                    Event::NoteOn {
                        note: 5,
                        velocity: 100,
                    },
                );
            }
            for &(at, value) in &squeeze {
                if let Some(o) = here(at) {
                    h.pad_pressure(o, 5, value);
                }
            }
            if let Some(o) = here(40_000) {
                h.pad(o, Event::NoteOff { note: 5 });
            }
            h.period(PERIOD, &mut pads);
            pads.frame = end;
        }
        assert_eq!(h.control.touch_freed(), (0, 0), "not while recording");
        // Recording off: the next period frees the 33,000.
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        let to = pads.frame + u64::from(PERIOD);
        run(&mut h, &mut pads, to, &[]);
        assert_eq!(h.control.touch_freed(), (33_000, 1));
        let clip = h.control.project.clip(0, 0).unwrap();
        assert_eq!(clip.touch.len(), 3);
        let values: Vec<u8> = clip
            .touch_of(&clip.notes[0])
            .iter()
            .map(|p| p.value)
            .collect();
        assert_eq!(values, [40, 90, 20]);
        assert_eq!(*h.audio.view, h.control.view().unwrap(), "compiled again");
        // It plays back, and UNDO still takes the note out.
        pads.pressure.clear();
        run(&mut h, &mut pads, 2 * BAR - 1000, &[]);
        let back: Vec<(usize, u8)> = pads.pressure.iter().map(|p| (p.1, p.2)).collect();
        assert_eq!(back, [(5, 40), (5, 90), (5, 20)]);
        h.act(TimeAction::Undo).unwrap();
        assert!(h.control.project.clip(0, 0).unwrap().notes.is_empty());
    }

    #[test]
    fn pad_pressure_while_recording_goes_on_its_note_and_plays_back() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::SetLoop(Some((0, 3840))));
        h.act(TimeAction::NewClip { track: 0, slot: 0 }).unwrap();
        assert!(h.state().tracks[0].pressure);
        // A pad held while pressed harder, then let up, and its pressure.
        let play = |h: &mut TimeHost,
                    pads: &mut Fake,
                    to: u64,
                    pad: u8,
                    held: (u64, u64),
                    squeeze: &[(u64, u8)]| {
            while pads.frame < to {
                let end = pads.frame + u64::from(PERIOD);
                let here = |at: u64| {
                    (pads.frame..end)
                        .contains(&at)
                        .then(|| (at - pads.frame) as u32)
                };
                if let Some(o) = here(held.0) {
                    h.pad(
                        o,
                        Event::NoteOn {
                            note: pad,
                            velocity: 100,
                        },
                    );
                }
                for &(at, value) in squeeze {
                    if let Some(o) = here(at) {
                        h.pad_pressure(o, pad, value);
                    }
                }
                if let Some(o) = here(held.1) {
                    h.pad(o, Event::NoteOff { note: pad });
                }
                h.period(PERIOD, pads);
                pads.frame = end;
            }
        };
        let squeeze = [(12_000u64, 40u8), (20_000, 90), (30_000, 20)];
        play(&mut h, &mut pads, BAR - 1000, 5, (10_000, 40_000), &squeeze);
        let clip = h.control.project.clip(0, 0).unwrap();
        assert_eq!(clip.notes.len(), 1);
        let values: Vec<u8> = clip
            .touch_of(&clip.notes[0])
            .iter()
            .map(|p| p.value)
            .collect();
        assert_eq!(values, [40, 90, 20]);
        // The view took it in place, as a compile would have.
        assert_eq!(*h.audio.view, {
            let mut v =
                PlayView::compile_cued(&h.control.project, &h.control.launches, &h.control.cues)
                    .unwrap();
            v.set_armed(h.control.armed);
            v.make_room(RECORD_ROOM, RECORD_ROOM);
            v
        });
        // Recording off: the next pass plays it back to the pads, each value
        // a bar after it was played (to a tick).
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        pads.pressure.clear();
        run(&mut h, &mut pads, 2 * BAR - 1000, &[]);
        let back: Vec<(usize, u8)> = pads.pressure.iter().map(|p| (p.1, p.2)).collect();
        assert_eq!(back, [(5, 40), (5, 90), (5, 20)]);
        for (p, &(at, _)) in pads.pressure.iter().zip(&squeeze) {
            assert!(p.0.abs_diff(at + BAR) <= 23, "{} vs {}", p.0, at + BAR);
        }
        // With the row off, a take keeps the notes but not their pressure.
        assert_eq!(
            h.act(TimeAction::RecordPressure {
                track: 0,
                on: false
            })
            .unwrap(),
            "pad pressure: Track 1 leaves it out"
        );
        assert!(!h.state().tracks[0].pressure);
        run(&mut h, &mut pads, 2 * BAR, &[]);
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        let s2 = 2 * BAR;
        play(
            &mut h,
            &mut pads,
            3 * BAR - 1000,
            6,
            (s2 + 10_000, s2 + 30_000),
            &[(s2 + 15_000, 99)],
        );
        let clip = h.control.project.clip(0, 0).unwrap();
        let six = clip.notes.iter().find(|n| n.note == 6).unwrap();
        assert!(six.touch.is_none() && clip.touch_of(six).is_empty());
        assert_eq!(h.touch_dropped(), 0);
        assert_eq!(h.dropped(), 0);
    }

    /// CAPTURE keeps the moves made over the phrase it keeps.
    #[test]
    fn capture_keeps_the_moves_made_over_its_phrase() {
        use mpfree_play::mpfree_param::{OnTrack, Owner, ParamAddr, RelAddr};
        use mpfree_seq::mpfree_moves::Target;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        let knob = ParamAddr::new(Owner::Inst(0), 5);
        // A pad in bar 2 and a gesture over it.
        let notes = [(BAR + 1000, 4, 100, 2000)];
        let mut moves: Vec<(u64, Option<f32>)> = (0..10)
            .map(|i| (BAR + 3000 + i * 1500, Some(0.1 * i as f32)))
            .collect();
        moves.push((BAR + 20_000, None));
        while pads.frame < 3 * BAR {
            let end = pads.frame + u64::from(PERIOD);
            for &(at, pad, v, held) in &notes {
                if (pads.frame..end).contains(&at) {
                    h.pad(
                        (at - pads.frame) as u32,
                        Event::NoteOn {
                            note: pad,
                            velocity: v,
                        },
                    );
                }
                if (pads.frame..end).contains(&(at + held)) {
                    h.pad(
                        (at + held - pads.frame) as u32,
                        Event::NoteOff { note: pad },
                    );
                }
            }
            for &(at, v) in &moves {
                if (pads.frame..end).contains(&at) {
                    let input = match v {
                        Some(value) => Input::Move {
                            addr: knob,
                            value,
                            map: mpfree_seq::mpfree_moves::Map::linear(0.0, 1.0),
                        },
                        None => Input::End { addr: knob },
                    };
                    h.gesture((at - pads.frame) as u32, input);
                }
            }
            h.period(PERIOD, &mut pads);
            pads.frame = end;
        }
        assert!(h.state().done.is_empty(), "nothing was recorded");
        let line = h.act(TimeAction::Capture).unwrap();
        assert_eq!(
            line,
            "capture 1: 1 notes and 1 move, 1 bar, new clip in slot 1 of Track 1"
        );
        let target = Target::Track(RelAddr::new(OnTrack::Inst, 5));
        let points = h
            .control
            .project
            .clip(0, 0)
            .unwrap()
            .moves
            .lane(target)
            .map(|l| l.points.clone());
        let points = points.expect("the move went into the kept clip");
        // On the clip's ticks: bar 2 is its bar, from about tick 125.
        assert!(points[0].tick.abs_diff(130) <= 2, "{points:?}");
        assert_eq!(
            points.last().map(|p| p.kind),
            Some(mpfree_seq::mpfree_moves::Kind::End)
        );
        assert_eq!(
            h.act(TimeAction::Undo).unwrap(),
            "undo: capture 1 · Track 1 / 1"
        );
        assert!(h.control.project.clip(0, 0).is_none());
    }

    /// The arp's clock follows the transport: None while stopped, the
    /// transport's anchor while playing (a whole tick on an exact frame),
    /// `jumped` once at the start and after a locate, and the swing.
    #[test]
    fn the_arp_clock_follows_the_transport() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        assert!(h.advance(PERIOD));
        assert_eq!(h.arp_clock(), None, "stopped: the arp runs on its own");
        h.period(PERIOD, &mut pads);
        h.command(Command::Play);
        let mut clocks = Vec::new();
        for _ in 0..40 {
            h.advance(PERIOD);
            clocks.push(h.arp_clock().expect("playing"));
            h.period(PERIOD, &mut pads);
        }
        assert!(
            clocks[0].jumped && clocks[1..].iter().all(|c| !c.jumped),
            "{clocks:?}"
        );
        for c in &clocks {
            assert!(c.playing && c.bpm == 120.0 && c.bar_ticks == 3840 && c.swing.is_none());
            // The anchor's tick and frame agree with the song position at the period's start.
            let tick = c.anchor_tick as f64
                + c.frames_since_anchor as f64 * 120.0 * 960.0 / (60.0 * f64::from(RATE));
            assert!(tick >= 0.0);
        }
        let k = clocks.len() - 1;
        let at = |c: &mpfree_arp::Clock| {
            c.anchor_tick as f64
                + c.frames_since_anchor as f64 * 120.0 * 960.0 / (60.0 * f64::from(RATE))
        };
        assert!(
            (at(&clocks[k])
                - at(&clocks[0])
                - k as f64 * f64::from(PERIOD) * 960.0 * 2.0 / f64::from(RATE))
            .abs()
                < 1e-6
        );
        // A locate while playing: jumped once, from the new place.
        h.command(Command::Locate(3840 * 4));
        h.advance(PERIOD);
        let c = h.arp_clock().unwrap();
        assert!(c.jumped && (at(&c) - 15_360.0).abs() < 1e-6, "{c:?}");
        h.period(PERIOD, &mut pads);
        h.advance(PERIOD);
        assert!(!h.arp_clock().unwrap().jumped);
        h.period(PERIOD, &mut pads);
        // Swing on the transport goes to the arp.
        h.command(Command::SetSwing(mpfree_transport::Swing {
            grid: 240,
            amount: 62,
        }));
        h.period(PERIOD, &mut pads);
        h.advance(PERIOD);
        assert_eq!(h.arp_clock().unwrap().swing, Some((240, 62)));
        h.period(PERIOD, &mut pads);
        // Stopped again: None.
        h.command(Command::Stop);
        h.period(PERIOD, &mut pads);
        h.advance(PERIOD);
        assert_eq!(h.arp_clock(), None);
    }

    /// Plays the periods up to frame `to` as the sound loop does, with
    /// what waited for the launch line and played in each (sampler
    /// R5.3b), at its frame.
    /// Periods until frame `to`, the waits that play kept in `got` by
    /// their frame; returns how many hits STOP dropped.
    fn run_waits(
        h: &mut TimeHost,
        pads: &mut Fake,
        to: u64,
        got: &mut Vec<(u64, PadWait)>,
    ) -> usize {
        let mut due = DuePads::default();
        let mut dropped = 0;
        while pads.frame < to {
            h.advance(PERIOD);
            dropped += h.due_pads(&mut due);
            got.extend(
                due.as_slice()
                    .iter()
                    .map(|&(w, at)| (pads.frame + u64::from(at), w)),
            );
            h.period(PERIOD, pads);
            pads.frame += u64::from(PERIOD);
        }
        dropped
    }

    /// Sampler R5.3b: a Loop pad's live hit made while the song plays
    /// waits for the project's launch line and plays on its frame, its
    /// let-go on the line after; the Launch setting moves the line, at
    /// once plays what waits a frame apart and lets hits play at once,
    /// and a stop drops the hits still waiting with their let-gos.
    #[test]
    fn a_loop_pads_hit_waits_for_the_launch_line_and_plays_on_it() {
        use mpfree_seq::LaunchGrid;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let hit = |p| PadWait {
            from: PadFrom::Pad(p),
            down: true,
            velocity: 99,
            kit_pad: p,
        };
        let up = |p| PadWait {
            from: PadFrom::Pad(p),
            down: false,
            velocity: 0,
            kit_pad: p,
        };
        let mut got = Vec::new();
        run_waits(&mut h, &mut pads, u64::from(PERIOD), &mut got);
        assert!(!h.pads_wait(), "stopped: a hit plays at once");
        h.command(Command::Play);
        // The song's first frame.
        let start = pads.frame;
        run_waits(&mut h, &mut pads, start + BAR / 4, &mut got);
        assert!(h.pads_wait());
        // A quarter into bar 1: it starts on bar 2 and is let go on bar 3.
        assert!(h.wait_pad(hit(2)) && h.wait_pad(up(2)));
        assert!(h.waits_on(PadFrom::Pad(2)) && !h.waits_on(PadFrom::Pad(3)));
        run_waits(&mut h, &mut pads, start + 2 * BAR + BAR / 4, &mut got);
        assert_eq!(got, [(start + BAR, hit(2)), (start + 2 * BAR, up(2))]);
        assert!(!h.waits_on(PadFrom::Pad(2)));
        // Launch on the beat: the next beat line.
        h.act(TimeAction::LaunchGrid(LaunchGrid::Ticks(960)))
            .unwrap();
        let next = pads.frame + u64::from(PERIOD);
        run_waits(&mut h, &mut pads, next, &mut got);
        got.clear();
        let from = pads.frame;
        assert!(h.wait_pad(hit(3)));
        run_waits(&mut h, &mut pads, from + BAR / 2, &mut got);
        let beat = BAR / 4;
        let line = start + (from - start).div_ceil(beat) * beat;
        assert_eq!(got, [(line, hit(3))]);
        // At once: what waits plays now, a frame apart, and nothing waits
        // after.
        got.clear();
        assert!(h.wait_pad(hit(5)) && h.wait_pad(up(5)));
        h.act(TimeAction::LaunchGrid(LaunchGrid::Now)).unwrap();
        let at = pads.frame;
        run_waits(&mut h, &mut pads, at + u64::from(PERIOD), &mut got);
        assert_eq!(got, [(at, hit(5)), (at + 1, up(5))]);
        assert!(!h.pads_wait());
        // A stop drops the hits still waiting, with their let-gos (pad 4's
        // made, pad 7's still held), and plays the let-go of pad 6's hit,
        // which sounded.
        h.act(TimeAction::LaunchGrid(LaunchGrid::Bars(1))).unwrap();
        let next = pads.frame + u64::from(PERIOD);
        run_waits(&mut h, &mut pads, next, &mut got);
        got.clear();
        assert!(h.pads_wait() && h.wait_pad(hit(6)));
        let bar = start + (pads.frame - start).div_ceil(BAR) * BAR;
        let dropped = run_waits(&mut h, &mut pads, bar + u64::from(PERIOD), &mut got);
        assert_eq!((dropped, &got[..]), (0, &[(bar, hit(6))][..]));
        got.clear();
        for w in [up(6), hit(4), up(4), hit(7)] {
            assert!(h.wait_pad(w));
        }
        assert_eq!(h.hits_waiting(0), 1 << 4 | 1 << 7);
        h.command(Command::Stop);
        let at = pads.frame;
        let dropped = run_waits(&mut h, &mut pads, at + u64::from(PERIOD), &mut got);
        assert_eq!((dropped, &got[..]), (2, &[(at, up(6))][..]));
        assert!(!h.pads_wait() && !h.waits_on(PadFrom::Pad(4)));
        assert_eq!(h.hits_waiting(0), 0);
        assert!(h.lets_go_of_dropped(up(7)) && !h.lets_go_of_dropped(up(4)));
    }

    /// Sampler R5.3b: a Loop pad's hit made during a count-in waits for
    /// the first line after it, the record start, not a count-in's line.
    #[test]
    fn a_loop_pads_hit_in_a_count_in_waits_for_the_record_start() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let hit = PadWait {
            from: PadFrom::Pad(1),
            down: true,
            velocity: 99,
            kit_pad: 1,
        };
        let mut got = Vec::new();
        assert!(h.command(Command::SetCountIn(2)));
        assert!(h.command(Command::Record));
        // The count-in's first frame: two bars before the record start.
        let start = pads.frame;
        run_waits(&mut h, &mut pads, start + BAR / 4, &mut got);
        assert_eq!(h.info().state, State::CountIn);
        assert!(h.pads_wait() && h.wait_pad(hit));
        run_waits(&mut h, &mut pads, start + 3 * BAR, &mut got);
        assert_eq!(h.info().state, State::Recording);
        assert_eq!(got, [(start + 2 * BAR, hit)]);
    }

    #[test]
    fn a_new_clip_records_the_pads_and_plays_them_back() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::SetLoop(Some((0, 2 * 3840))));
        assert!(h
            .act(TimeAction::NewClip { track: 2, slot: 0 })
            .unwrap()
            .contains("recording"));
        let st = h.state();
        assert_eq!(st.armed, Some(2));
        assert_eq!(
            st.clip(2, 0).map(|c| c.length),
            Some(2 * 3840),
            "as long as the loop"
        );
        // Record two hits in the first pass; the second pass plays them.
        run(
            &mut h,
            &mut pads,
            2 * BAR,
            &[(1000, 4, 100, 2000), (BAR + 500, 9, 80, 2000)],
        );
        assert_eq!(h.info().state, State::Recording);
        assert!(h.take_changed());
        let st = h.state();
        assert_eq!(st.done.len(), 1);
        assert_eq!(st.done[0].added, 2);
        assert!(
            pads.hits.is_empty(),
            "nothing plays the notes back while they are being played"
        );
        run(&mut h, &mut pads, 4 * BAR, &[]);
        // Notes are kept in whole ticks (960 to a beat: 23 frames at 120 BPM).
        let back: Vec<(u64, usize)> = pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        assert_eq!(back.len(), 2, "{back:?}");
        for ((f, p), (want, pad)) in back.iter().zip([(2 * BAR + 1000, 4), (3 * BAR + 500, 9)]) {
            assert!(f.abs_diff(want) <= 23 && *p == pad, "{back:?}");
        }
        // Undo takes the take out; the next pass is silent.
        assert_eq!(
            h.act(TimeAction::Undo).unwrap(),
            "undo: take 1 · Track 3 / 1"
        );
        pads.hits.clear();
        run(&mut h, &mut pads, 6 * BAR, &[]);
        assert!(pads.hits.is_empty());
        assert_eq!(
            h.act(TimeAction::Redo).unwrap(),
            "redo: take 1 · Track 3 / 1"
        );
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        run(&mut h, &mut pads, 8 * BAR, &[]);
        assert_eq!(pads.hits.len(), 2, "redo brings both back");
        assert_eq!(h.dropped(), 0);
    }

    /// A knob turned while a pad is held and the track records becomes a
    /// parameter lock on that pad's note, and goes with its take.
    #[test]
    fn a_knob_turned_while_recording_locks_the_held_pads_note() {
        use mpfree_play::mpfree_param::{OnTrack, RelAddr};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.act(TimeAction::NewClip { track: 2, slot: 0 }).unwrap();
        let press = [(1000, 4, 100, 2000)];
        run(&mut h, &mut pads, 1536, &press);
        let cutoff = RelAddr::new(OnTrack::Inst, 1);
        h.knob(0, cutoff.on(2), 0.25);
        h.knob(64, cutoff.on(2), 0.6);
        // Another track's knob is not recorded here.
        h.knob(100, cutoff.on(5), 0.9);
        run(&mut h, &mut pads, BAR, &press);
        let clip = h.control.project.tracks[2].slots[0].as_ref().unwrap();
        assert_eq!(clip.notes.len(), 1);
        let locks: Vec<_> = clip
            .locks
            .iter()
            .map(|l| (l.tick, l.sound, l.addr, l.value))
            .collect();
        assert_eq!(locks, vec![(clip.notes[0].tick, None, cutoff, 0.6)]);
        // Played back, the lock goes out for the mixer's mod engine on its
        // note's frame (the clip's only note is locked, so it never ends).
        let mut out = Vec::new();
        for _ in 0..2 * BAR / u64::from(PERIOD) {
            h.period(PERIOD, &mut pads);
            out.extend_from_slice(h.locks());
        }
        assert!(
            out.iter()
                .any(|&(_, a, v)| a == cutoff.on(2) && v == Some(0.6)),
            "{out:?}"
        );
        assert_eq!(h.dropped(), 0);
        assert_eq!(
            h.act(TimeAction::Undo).unwrap(),
            "undo: take 1 · Track 3 / 1"
        );
        let clip = h.control.project.tracks[2].slots[0].as_ref().unwrap();
        assert!(clip.notes.is_empty() && clip.locks.is_empty());
    }

    /// Step sequencer refactor F1: a clip's lock on its synth's control
    /// plays only while the track plays the kind of synth it was made
    /// for. Made on Poly, it is left out of what plays on FM, kept, and
    /// the Steps say why; a new lock (or a recorded move) for FM stays
    /// out while it is there; back on Poly it plays; taken off, the clip
    /// forgets Poly and FM's lock goes in.
    #[test]
    fn a_synth_lock_plays_only_on_the_synth_it_was_made_for() {
        use mpfree_node::Node;
        use mpfree_play::mpfree_param::{OnTrack, RelAddr};
        use mpfree_seq::steps::LINE;
        let poly = crate::synth::engine_node("mpfree.poly", RATE).unwrap();
        let fm = crate::synth::engine_node("mpfree.fm", RATE).unwrap();
        let of = |n: &dyn Node| -> Option<InstRef> { Some((n.info(), n.params())) };
        let cutoff = poly.params().iter().find(|p| p.name == "Cutoff").unwrap();
        let addr = RelAddr::new(OnTrack::Inst, cutoff.id);
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.control.project.tracks[0].slots[0] = Some(Clip::new("Bass", 3840));
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        h.set_instruments(&[of(&*poly)]);
        let lock = |value| StepAction::Lock {
            pad: LINE,
            steps: 1,
            addr,
            value,
        };
        h.step_action(lock(Some(0.4))).unwrap();
        let made = |h: &TimeHost| {
            let c = h.control.project.tracks[0].slots[0].as_ref().unwrap();
            c.made_for.clone().map(|m| (m.node, m.name, m.params))
        };
        assert_eq!(
            made(&h),
            Some((
                "mpfree.poly".into(),
                "Poly".into(),
                vec![(cutoff.id, "Cutoff".into())]
            ))
        );
        let played = |h: &mut TimeHost, pads: &mut Fake| {
            let mut out = Vec::new();
            for _ in 0..BAR / u64::from(PERIOD) {
                h.period(PERIOD, pads);
                out.extend_from_slice(h.locks());
            }
            out.iter().any(|&(_, a, _)| a == addr.on(0))
        };
        assert!(played(&mut h, &mut pads), "on Poly it plays");
        h.set_instruments(&[of(&*fm)]);
        assert!(!played(&mut h, &mut pads), "on FM it doesn't");
        let c = h.control.project.tracks[0].slots[0].as_ref().unwrap();
        assert_eq!(c.locks.len(), 1, "kept");
        let silent = h.steps().clip.unwrap().silent.unwrap();
        assert_eq!(silent.params, vec![cutoff.id]);
        assert_eq!(
            silent.line(cutoff.id),
            "made for Poly's Cutoff, not playing on FM"
        );
        assert_eq!(
            h.step_action(lock(Some(0.7))),
            Err("steps: this pattern's Poly locks don't play on FM: take them off first".into())
        );
        // A move of FM's control recorded into it stays out, counted.
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        run(&mut h, &mut pads, 3 * BAR, &[]);
        assert_eq!(h.info().state, State::Recording);
        h.knob(0, addr.on(0), 0.9);
        run(&mut h, &mut pads, 3 * BAR + BAR / 2, &[]);
        h.command(Command::Stop);
        run(&mut h, &mut pads, 4 * BAR, &[]);
        let c = h.control.project.tracks[0].slots[0].as_ref().unwrap();
        assert_eq!(c.locks.len(), 1);
        assert_eq!(h.control.inst_refused(), 1);
        assert!(h
            .describe()
            .contains("1 recorded moves left out of patterns"));
        h.command(Command::Play);
        h.set_instruments(&[of(&*poly)]);
        assert!(played(&mut h, &mut pads), "back on Poly it plays");
        assert_eq!(h.steps().clip.unwrap().silent, None);
        // Taken off (allowed on any synth), the clip forgets Poly.
        h.set_instruments(&[of(&*fm)]);
        h.step_action(lock(None)).unwrap();
        assert_eq!(made(&h), None);
        h.step_action(lock(Some(0.7))).unwrap();
        assert_eq!(made(&h).map(|m| m.1), Some("FM".to_string()));
    }

    /// Instrument redesign, after step 4d: a clip's lock on "this track's
    /// strip" goes where the track's instrument sounds (strips.rs). Track
    /// 2 plays track 3's instrument, heard on track 3's strip: its lock
    /// and the lock's end go there, and track 3's own lock on the same
    /// control waits behind it (no lasting ids here, so by track), then
    /// takes over. A lock on an effect of track 2's strip stays there.
    #[test]
    fn a_lock_on_this_tracks_strip_goes_where_its_instrument_sounds() {
        use mpfree_play::mpfree_param::{track_strip, OnTrack, Owner, ParamAddr, RelAddr, Strip};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let vol = RelAddr::new(OnTrack::Strip, 0);
        let fx = RelAddr::new(OnTrack::Insert(1), 2);
        // Each track: pad 2 on steps 1 and 9, step 1 locked; step 9's note,
        // with no lock, ends it.
        for (track, value) in [(1, 0.25), (2, 0.75)] {
            h.act(TimeAction::Arm(Some(track))).unwrap();
            for step in [0, 8] {
                h.step_action(StepAction::Tap {
                    pad: 2,
                    step,
                    velocity: 100,
                })
                .unwrap();
            }
            h.step_action(StepAction::Lock {
                pad: 2,
                steps: 1,
                addr: vol,
                value: Some(value),
            })
            .unwrap();
        }
        h.act(TimeAction::Arm(Some(1))).unwrap();
        h.step_action(StepAction::Lock {
            pad: 2,
            steps: 1,
            addr: fx,
            value: Some(0.5),
        })
        .unwrap();
        let Strip::Track(three) = track_strip(2) else {
            unreachable!()
        };
        let mut pads = Fake {
            sounds: vec![(1, three)],
            ..Fake::default()
        };
        h.command(Command::Play);
        let mut out = Vec::new();
        // Past step 9, before the clip comes round again.
        while pads.frame < 3 * BAR / 4 {
            h.period(PERIOD, &mut pads);
            out.extend(h.locks().iter().map(|&(_, a, v)| (a, v)));
            h.clear_locks();
            pads.frame += u64::from(PERIOD);
        }
        let on_three = ParamAddr::new(Owner::Strip(Strip::Track(three)), 0);
        let fx_two = fx.on(1);
        assert_eq!(
            out,
            vec![
                // Step 1: track 2's lock on track 3's strip; its effect
                // lock on its own; track 3's own lock waits.
                (on_three, Some(0.25)),
                (fx_two, Some(0.5)),
                // Step 9: track 2's end hands the control to track 3, whose
                // end lets it go.
                (on_three, Some(0.75)),
                (fx_two, None),
                (on_three, None),
            ]
        );
        let c = h.control.strips;
        assert_eq!((c.moved, c.waited, c.full), (1, 1, 0));
        assert!(h
            .describe()
            .contains("this track's strip: 1 locks went to their instrument's strip, 1 waited"));
        assert_eq!(h.dropped(), 0);
    }

    /// Per-track arps with track `track`'s arp playing up in sixteenths.
    fn arp_up(track: usize) -> TrackArps {
        use mpfree_arp::{Mode, Order, Rate, Settings};
        let mut arps = TrackArps::new(RATE, 120.0);
        arps.engine(track).unwrap().set_settings(Settings {
            mode: Mode::Arp,
            order: Order::Up,
            rate: Rate::Sixteenth,
            gate: 50,
            ..Settings::default()
        });
        arps
    }

    /// Records one bar into a new clip on track 3 (a two-bar loop) through
    /// the per-track arps: the pads pressed the chord 4, 7, 9 at frame
    /// 1000 and held it to the bar line, while the app's arp let pads 20
    /// and 21 be heard. Then plays the next pass and returns its pad hits
    /// as (frame, pad).
    fn record_a_chord(h: &mut TimeHost, arps: &mut TrackArps) -> Vec<(u64, usize)> {
        let mut pads = Fake::default();
        h.command(Command::SetLoop(Some((0, 2 * 3840))));
        h.act(TimeAction::NewClip { track: 2, slot: 0 }).unwrap();
        let on = |note| Event::NoteOn {
            note,
            velocity: 100,
        };
        while pads.frame < 4 * BAR {
            let f = pads.frame;
            let at = |x: u64| {
                (f..f + u64::from(PERIOD))
                    .contains(&x)
                    .then(|| (x - f) as u32)
            };
            if let Some(o) = at(1000) {
                for note in [4, 7, 9] {
                    h.pad_pressed(o, on(note));
                }
                h.pad(o, on(20));
            }
            if let Some(o) = at(1000 + 5512) {
                h.pad(o, Event::NoteOff { note: 20 });
                h.pad(o, on(21));
            }
            if let Some(o) = at(BAR - 100) {
                for note in [4, 7, 9] {
                    h.pad_pressed(o, Event::NoteOff { note });
                }
                h.pad(o, Event::NoteOff { note: 21 });
            }
            h.period_arps(PERIOD, &mut pads, arps);
            pads.frame += u64::from(PERIOD);
        }
        assert!(
            pads.hits.iter().all(|&(f, _, _)| f >= 2 * BAR),
            "the first pass only records"
        );
        assert_eq!(h.dropped() + arps.dropped(), 0);
        pads.hits
            .iter()
            .filter(|&&(f, _, _)| f >= 2 * BAR)
            .map(|&(f, p, _)| (f, p))
            .collect()
    }

    /// Step 5c: REC's new clip on a track whose instrument plays notes
    /// holds notes, and the take records the notes the pads played (a
    /// pad's chord, each note), with the pad's pressure on each; on a
    /// track that doesn't it holds pad hits and records the pad, as
    /// before. Through the arp it holds notes too (step 5d).
    #[test]
    fn a_take_on_a_track_that_plays_notes_records_its_notes() {
        use mpfree_seq::Holds;
        let take = |track: usize, through: bool| {
            let mut h = TimeHost::new(RATE, 120.0).unwrap();
            let mut pads = Fake {
                chord: vec![60, 64],
                ..Fake::default()
            };
            let mut arps = TrackArps::new(RATE, 120.0);
            h.control.project.tracks[track].through_arp = through;
            h.set_notes_tracks(1 << 1);
            h.command(Command::SetLoop(Some((0, 3840))));
            h.act(TimeAction::NewClip { track, slot: 0 }).unwrap();
            let holds = h.project().clip(track, 0).unwrap().holds;
            while pads.frame < BAR - 1000 {
                let at = pads.frame;
                if at == 10 * u64::from(PERIOD) {
                    arps.pad(
                        track,
                        0,
                        Event::NoteOn {
                            note: 4,
                            velocity: 100,
                        },
                    );
                }
                if at == 20 * u64::from(PERIOD) {
                    h.pad_pressure(5, 4, 70);
                }
                if at == 40 * u64::from(PERIOD) {
                    arps.pad(track, 0, Event::NoteOff { note: 4 });
                }
                h.period_arps(PERIOD, &mut pads, &mut arps);
                pads.frame += u64::from(PERIOD);
            }
            let clip = h.project().clip(track, 0).unwrap().clone();
            let mut notes: Vec<(u8, Vec<u8>)> = clip
                .notes
                .iter()
                .map(|n| (n.note, clip.touch_of(n).iter().map(|p| p.value).collect()))
                .collect();
            notes.sort();
            assert_eq!(h.dropped(), 0);
            (holds, notes)
        };
        assert_eq!(
            take(1, false),
            (Holds::Notes, vec![(64, vec![70]), (68, vec![70])],),
            "the chord's notes, each with the pad's pressure"
        );
        assert_eq!(
            take(0, false),
            (Holds::Hits, vec![(4, vec![70])]),
            "a track that plays no notes records the pad"
        );
        // Through the arp, pads as pressed: none here, so the take records
        // nothing but makes a clip of notes (step 5d).
        assert_eq!(take(1, true).0, Holds::Notes);
    }

    /// Step 5c: on a clip of notes the step grid's pad rows show the
    /// notes the track's pads play, named by the pad, so the Play page and
    /// the step pads read it as they read a clip of hits: a tap on pad 3's
    /// row adds pad 3's note, the Note knob moves a step to another pad's
    /// note, and a note no pad plays stays on a row of its own. A new
    /// pattern on a track that plays notes holds notes; on one that
    /// doesn't it holds pad hits, as before.
    #[test]
    fn a_clip_of_notes_shows_its_pads_notes_on_the_pad_rows() {
        use mpfree_seq::steps::Change;
        use mpfree_seq::Holds;
        let mut rows = PadRows::PADS;
        for p in 0..16 {
            rows.set(p, &[60 + p as u8]);
        }
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.set_notes_tracks(1);
        h.set_pad_rows(0, rows);
        let tap = |pad, step| StepAction::Tap {
            pad,
            step,
            velocity: 100,
        };
        let said = h.step_action(tap(2, 0)).unwrap();
        assert!(said.contains("step 1 of note D4"), "{said}");
        let clip = h.project().clip(0, 0).unwrap().clone();
        assert_eq!(clip.holds, Holds::Notes, "a new pattern holds notes");
        let notes = |h: &TimeHost| -> Vec<(u8, Tick)> {
            let mut v: Vec<_> = h
                .project()
                .clip(0, 0)
                .unwrap()
                .notes
                .iter()
                .map(|n| (n.note, n.tick))
                .collect();
            v.sort();
            v
        };
        assert_eq!(notes(&h), vec![(62, 0)], "pad 3's note");
        // The grid names the row by the pad.
        let shown = |h: &TimeHost| {
            let s = h.steps();
            let c = s.clip.expect("the pattern");
            c.rows
                .iter()
                .filter(|(p, r)| *p != LINE && r.iter().any(Option::is_some))
                .map(|(p, r)| (*p, r.iter().position(Option::is_some).unwrap()))
                .collect::<Vec<_>>()
        };
        assert_eq!(shown(&h), vec![(2, 0)]);
        // The Note knob on pad 3's step, to pad 6: the step plays pad 6's
        // note, on pad 6's row.
        h.step_action(StepAction::Change {
            pad: 2,
            step: 0,
            change: Change::Note(5),
        })
        .unwrap();
        assert_eq!(notes(&h), vec![(65, 0)]);
        assert_eq!(shown(&h), vec![(5, 0)]);
        // A note no pad plays (from a take, or a pad map changed since)
        // keeps a row of its own, under its note.
        h.step_action(tap(0, 2)).unwrap();
        let mut up = rows;
        for p in 0..16 {
            up.set(p, &[72 + p as u8]);
        }
        h.set_pad_rows(0, up);
        assert!(h.control.take_steps_changed(), "new rows show again");
        let mut other = shown(&h);
        other.sort();
        assert_eq!(other, vec![(60, 2), (65, 0)]);
        // A second tap on the same row takes the note back off.
        h.set_pad_rows(0, rows);
        h.step_action(tap(0, 2)).unwrap();
        assert_eq!(notes(&h), vec![(65, 0)]);
        // A track that plays no notes: a pattern of pad hits, by pad.
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.set_pad_rows(0, rows);
        let said = h.step_action(tap(2, 0)).unwrap();
        assert!(said.contains("step 1 of pad 3"), "{said}");
        let clip = h.project().clip(0, 0).unwrap();
        assert_eq!(clip.holds, Holds::Hits);
        assert_eq!(clip.notes[0].note, 2);
    }

    /// Step 5c with a pad of chords: on a clip of notes a pad's row stands
    /// for the whole chord, as a pad hit plays it on a clip of hits. A tap
    /// puts all its notes on the step and takes them all off again; a
    /// change takes every note; the Note knob puts the step on another
    /// pad's notes; keys entered on the line are their pads' chords.
    #[test]
    fn a_pad_of_chords_steps_its_whole_chord_on_a_clip_of_notes() {
        use mpfree_seq::steps::Change;
        let mut rows = PadRows::PADS;
        for p in 0..16 {
            rows.set(p, &[60 + p as u8]);
        }
        // Pad 1 plays C major, its root last.
        rows.set(0, &[64, 67, 60]);
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.set_notes_tracks(1);
        h.set_pad_rows(0, rows);
        let notes = |h: &TimeHost| -> Vec<(u8, u8)> {
            let mut v: Vec<_> = h
                .project()
                .clip(0, 0)
                .map(|c| c.notes.iter().map(|n| (n.note, n.velocity)).collect())
                .unwrap_or_default();
            v.sort();
            v
        };
        let tap = |pad, step| StepAction::Tap {
            pad,
            step,
            velocity: 100,
        };
        let said = h.step_action(tap(0, 0)).unwrap();
        assert!(said.contains("note C4"), "the root names it: {said}");
        assert_eq!(notes(&h), vec![(60, 100), (64, 100), (67, 100)]);
        // Shown on pad 1's row.
        let s = h.steps();
        let row = &s.clip.as_ref().unwrap().rows;
        assert!(row.iter().any(|(p, r)| *p == 0 && r[0].is_some()));
        // Every note takes a change, and one UNDO takes it back.
        h.step_action(StepAction::Change {
            pad: 0,
            step: 0,
            change: Change::Velocity(50),
        })
        .unwrap();
        assert_eq!(notes(&h), vec![(60, 50), (64, 50), (67, 50)]);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(notes(&h), vec![(60, 100), (64, 100), (67, 100)]);
        // The Note knob to pad 6: the step plays pad 6's note.
        h.step_action(StepAction::Change {
            pad: 0,
            step: 0,
            change: Change::Note(5),
        })
        .unwrap();
        assert_eq!(notes(&h), vec![(65, 100)]);
        // And back to pad 1: the chord again.
        h.step_action(StepAction::Change {
            pad: 5,
            step: 0,
            change: Change::Note(0),
        })
        .unwrap();
        assert_eq!(notes(&h), vec![(60, 100), (64, 100), (67, 100)]);
        // A second tap takes the whole chord off.
        h.step_action(tap(0, 0)).unwrap();
        assert_eq!(notes(&h), vec![]);
        // Keys on the line: pad 1 and pad 3 give the chord and D4.
        let mut keys = [(0, 0); MAX_PEN];
        keys[0] = (0, 90);
        keys[1] = (2, 80);
        h.step_action(StepAction::Enter {
            step: 1,
            keys,
            n: 2,
        })
        .unwrap();
        assert_eq!(notes(&h), vec![(60, 90), (62, 80), (64, 90), (67, 90)]);
    }

    /// Trigger groups: a pad's step on a clip of notes is the pad's
    /// trigger (its number plus one). A tap, a change, the Note knob and
    /// keys on the line all give its notes the pad's group, so a chance on
    /// the step rolls once for the whole chord; a chord written before
    /// groups (each note on its own) becomes one trigger with the next
    /// change to its step.
    #[test]
    fn a_pad_of_chords_step_is_one_trigger() {
        use mpfree_seq::steps::Change;
        use mpfree_seq::Note;
        let mut rows = PadRows::PADS;
        for p in 0..16 {
            rows.set(p, &[60 + p as u8]);
        }
        rows.set(0, &[64, 67, 60]);
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.set_notes_tracks(1);
        h.set_pad_rows(0, rows);
        let on = |h: &TimeHost, tick: Tick| -> Vec<(u8, u8, u8)> {
            let c = h.project().clip(0, 0).unwrap();
            assert!(c.groups_ok());
            let mut v: Vec<_> = c
                .notes
                .iter()
                .filter(|n| n.tick == tick)
                .map(|n| (n.note, n.group, n.chance))
                .collect();
            v.sort();
            v
        };
        let tap = |pad, step| StepAction::Tap {
            pad,
            step,
            velocity: 100,
        };
        let change = |pad, step, change| StepAction::Change { pad, step, change };
        h.step_action(tap(0, 0)).unwrap();
        assert_eq!(on(&h, 0), vec![(60, 1, 100), (64, 1, 100), (67, 1, 100)]);
        h.step_action(change(0, 0, Change::Chance(50))).unwrap();
        assert_eq!(on(&h, 0), vec![(60, 1, 50), (64, 1, 50), (67, 1, 50)]);
        // The Note knob to pad 6 and back: each pad's group, the step's
        // chance kept.
        h.step_action(change(0, 0, Change::Note(5))).unwrap();
        assert_eq!(on(&h, 0), vec![(65, 6, 50)]);
        h.step_action(change(5, 0, Change::Note(0))).unwrap();
        assert_eq!(on(&h, 0), vec![(60, 1, 50), (64, 1, 50), (67, 1, 50)]);
        // Keys on the line: pad 1's chord and pad 3's note, each its pad's.
        let mut keys = [(0, 0); MAX_PEN];
        keys[0] = (0, 90);
        keys[1] = (2, 80);
        h.step_action(StepAction::Enter {
            step: 1,
            keys,
            n: 2,
        })
        .unwrap();
        assert_eq!(
            on(&h, 240),
            vec![(60, 1, 100), (62, 3, 100), (64, 1, 100), (67, 1, 100)]
        );
        // A chord from before groups on step 3: one trigger after a change.
        let c = h.control.project.tracks[0].slots[0].as_mut().unwrap();
        for pitch in [60, 64, 67] {
            c.notes.push(Note::new(480, 120, pitch, 70));
        }
        assert_eq!(on(&h, 480), vec![(60, 0, 100), (64, 0, 100), (67, 0, 100)]);
        h.step_action(change(0, 2, Change::Velocity(60))).unwrap();
        assert_eq!(on(&h, 480), vec![(60, 1, 100), (64, 1, 100), (67, 1, 100)]);
    }

    /// Pads as pressed with no notes to give a clip of notes (one through
    /// the arp) aren't recorded into it, and each is counted.
    #[test]
    fn pads_as_pressed_into_a_clip_of_notes_are_counted_not_recorded() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut arps = TrackArps::new(RATE, 120.0);
        h.control.project.tracks[1].through_arp = true;
        h.command(Command::SetLoop(Some((0, 3840))));
        h.act(TimeAction::NewClip { track: 1, slot: 0 }).unwrap();
        h.control.project.tracks[1].slots[0].as_mut().unwrap().holds = mpfree_seq::Holds::Notes;
        h.control.compile().unwrap();
        for i in 0..40u64 {
            if i == 10 {
                h.pad_pressed(
                    0,
                    Event::NoteOn {
                        note: 4,
                        velocity: 100,
                    },
                );
            }
            if i == 20 {
                h.pad_pressed(0, Event::NoteOff { note: 4 });
            }
            h.period_arps(PERIOD, &mut pads, &mut arps);
            pads.frame += u64::from(PERIOD);
        }
        assert!(h.project().clip(1, 0).unwrap().notes.is_empty());
        assert_eq!(h.dropped(), 1);
    }

    /// Step 5d: pads as pressed on a track whose clips play through its
    /// arp record their notes into a clip of notes, each with the pad's
    /// pressure.
    #[test]
    fn pads_as_pressed_into_a_clip_of_notes_record_their_notes() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut arps = TrackArps::new(RATE, 120.0);
        h.control.project.tracks[1].through_arp = true;
        h.set_notes_tracks(1 << 1);
        h.command(Command::SetLoop(Some((0, 3840))));
        h.act(TimeAction::NewClip { track: 1, slot: 0 }).unwrap();
        for i in 0..40u64 {
            if i == 10 {
                let on = Event::NoteOn {
                    note: 4,
                    velocity: 100,
                };
                h.pad_pressed_notes(0, on, &[64, 67, 60]);
            }
            if i == 15 {
                h.pad_pressure(0, 4, 70);
            }
            if i == 20 {
                h.pad_pressed_notes(0, Event::NoteOff { note: 4 }, &[64, 67, 60]);
            }
            h.period_arps(PERIOD, &mut pads, &mut arps);
            pads.frame += u64::from(PERIOD);
        }
        let clip = h.project().clip(1, 0).unwrap().clone();
        assert_eq!(clip.holds, mpfree_seq::Holds::Notes);
        let mut notes: Vec<(u8, Vec<u8>)> = clip
            .notes
            .iter()
            .map(|n| (n.note, clip.touch_of(n).iter().map(|p| p.value).collect()))
            .collect();
        notes.sort();
        assert_eq!(
            notes,
            vec![(60, vec![70]), (64, vec![70]), (67, vec![70])],
            "the pad's chord, each note with its pressure"
        );
        // One trigger: pad 5's group.
        assert!(clip.notes.iter().all(|n| n.group == 5), "{:?}", clip.notes);
        assert_eq!(h.dropped(), 0);
    }

    /// Step 5d, as the plan proves it: the pads recorded while a clip
    /// plays the same synth on another track, both through arps that work
    /// in notes. The take holds the pads' notes and never the clip's,
    /// whether the recording track's clips play through its arp (the pads
    /// as pressed, one trigger) or not (what its arp played, each note on
    /// its own).
    #[test]
    fn a_take_of_the_pads_leaves_out_a_clip_playing_the_same_synth() {
        use mpfree_seq::{Holds, Note};
        for through in [false, true] {
            let mut h = TimeHost::new(RATE, 120.0).unwrap();
            let mut pads = Fake::default();
            let mut arps = TrackArps::new(RATE, 120.0);
            let mut rows = PadRows::PADS;
            rows.set(4, &[64, 67, 60]);
            for t in 0..2 {
                arps.set_on_notes(t, true);
                arps.set_pads(t, &rows);
            }
            // Track 1 plays D3 all through, through its arp.
            h.control.project.tracks[0].through_arp = true;
            let mut c = Clip::new("D3", 3840);
            c.holds = Holds::Notes;
            c.notes = vec![Note::new(0, 3840, 50, 100)];
            h.control.project.tracks[0].slots[0] = Some(c);
            h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
            // Track 2 records the pads: pad 5, a C major chord.
            h.control.project.tracks[1].through_arp = through;
            h.set_notes_tracks(0b11);
            h.command(Command::SetLoop(Some((0, 3840))));
            h.act(TimeAction::NewClip { track: 1, slot: 0 }).unwrap();
            let on = Event::NoteOn {
                note: 4,
                velocity: 100,
            };
            let off = Event::NoteOff { note: 4 };
            for i in 0..40u64 {
                for (at, e) in [(10, on), (20, off)] {
                    if i == at {
                        h.pad_pressed_notes(0, e, rows.notes(4));
                        for &n in rows.notes(4) {
                            let e = match e {
                                Event::NoteOn { velocity, .. } => {
                                    Event::NoteOn { note: n, velocity }
                                }
                                _ => Event::NoteOff { note: n },
                            };
                            arps.pad(1, 0, e);
                        }
                    }
                }
                h.period_arps(PERIOD, &mut pads, &mut arps);
                pads.frame += u64::from(PERIOD);
            }
            assert!(pads.hits.iter().any(|&(_, n, _)| n == 50), "the clip plays");
            let clip = h.project().clip(1, 0).unwrap().clone();
            assert_eq!(clip.holds, Holds::Notes, "through {through}");
            let mut notes: Vec<(u8, u8)> = clip.notes.iter().map(|n| (n.note, n.group)).collect();
            notes.sort();
            // The pads as pressed are pad 5's trigger; what an arp played
            // came with no pad, each note on its own.
            let group = if through { 5 } else { 0 };
            assert_eq!(
                notes,
                vec![(60, group), (64, group), (67, group)],
                "through {through}"
            );
            assert_eq!(h.dropped(), 0);
        }
    }

    /// A track's MIDI effects go from the project to the sound loop: the
    /// control half sends the effects and, the first time, a chain to run
    /// them in; the live pads through the track's arp then play through
    /// them, as do edits made later. A chain stays when its effects go.
    #[test]
    fn a_tracks_midi_effects_reach_the_pads_from_the_project() {
        use mpfree_notefx::{Fx, Slot};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut arps = TrackArps::new(RATE, 120.0);
        let transpose = |semitones| Slot {
            fx: Fx::Transpose { semitones },
            on: true,
        };
        h.control.project.tracks[1].note_fx.slots[0] = Some(transpose(2));
        // A period for an edit to cross to the audio half, then a press.
        let press = |h: &mut TimeHost, pads: &mut Fake, arps: &mut TrackArps| {
            h.period_arps(PERIOD, pads, arps);
            pads.frame += u64::from(PERIOD);
            arps.pad(
                1,
                0,
                Event::NoteOn {
                    note: 4,
                    velocity: 100,
                },
            );
            arps.pad(1, 10, Event::NoteOff { note: 4 });
            for _ in 0..4 {
                h.period_arps(PERIOD, pads, arps);
                pads.frame += u64::from(PERIOD);
            }
            pads.hits.last().map(|&(_, pad, _)| pad)
        };
        assert_eq!(press(&mut h, &mut pads, &mut arps), Some(6));
        assert!(arps.has_chain(1) && !arps.has_chain(0));
        // As the chain editor's Notes stage sets them.
        let mut fx = h.project().tracks[1].note_fx;
        fx.slots[0] = Some(transpose(-3));
        assert!(h.set_note_fx(1, fx));
        assert_eq!(press(&mut h, &mut pads, &mut arps), Some(1));
        h.control.project.tracks[1].note_fx.slots[0] = None;
        assert_eq!(press(&mut h, &mut pads, &mut arps), Some(4));
        assert!(arps.has_chain(1));
        assert_eq!((h.dropped(), arps.dropped()), (0, 0));
        // A Loop track plays no notes, so it has no MIDI effects.
        h.control
            .project
            .tracks
            .push(Track::new_loops("Loops A", 8));
        let loops = h.project().tracks.len() - 1;
        assert!(!h.set_note_fx(loops, fx));
        assert!(!h.set_note_fx(loops + 1, fx), "no such track");
    }

    /// The mod engine's value on a MIDI effect setting (a macro, an LFO, a
    /// step's lock) plays on the track's chain from the next period, over
    /// the project's own, which stays as it was; letting go brings the own
    /// value back. An effect that leaves its place takes the value with
    /// it, even if a new one gets its id.
    #[test]
    fn the_mod_engines_value_moves_a_midi_effect_setting_until_it_lets_go() {
        use mpfree_notefx::Fx;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut arps = TrackArps::new(RATE, 120.0);
        let mut fx = mpfree_notefx::TrackFx::default();
        fx.add(Fx::Transpose { semitones: 2 });
        let id = fx.ids[0];
        assert!(h.set_note_fx(1, fx));
        let press = |h: &mut TimeHost, pads: &mut Fake, arps: &mut TrackArps| {
            h.period_arps(PERIOD, pads, arps);
            pads.frame += u64::from(PERIOD);
            arps.pad(
                1,
                0,
                Event::NoteOn {
                    note: 4,
                    velocity: 100,
                },
            );
            arps.pad(1, 10, Event::NoteOff { note: 4 });
            for _ in 0..4 {
                h.period_arps(PERIOD, pads, arps);
                pads.frame += u64::from(PERIOD);
            }
            pads.hits.last().map(|&(_, pad, _)| pad)
        };
        assert_eq!(press(&mut h, &mut pads, &mut arps), Some(6));
        // The sound loop has the effects with their ids, once.
        assert_eq!(h.note_fx_moved().map(|f| f[1].ids[0]), Some(id));
        assert!(h.note_fx_moved().is_none());
        let addr = ParamAddr::new(Owner::NoteFx(1, id), 0);
        h.modulate_note_fx(addr, Some(7.4));
        assert_eq!(press(&mut h, &mut pads, &mut arps), Some(11));
        assert_eq!(
            h.project().tracks[1].note_fx,
            fx,
            "the project keeps its own"
        );
        // As they play now (MIDI out's view): with the mod engine's value.
        assert_eq!(h.note_fx_now(1).value(id, 0), Some(7));
        assert_eq!(h.note_fx_now(99), mpfree_notefx::TrackFx::default());
        // Kept in range; a setting or an effect the track hasn't got is not.
        h.modulate_note_fx(addr, Some(99.0));
        assert_eq!(press(&mut h, &mut pads, &mut arps), Some(52));
        h.modulate_note_fx(ParamAddr::new(Owner::NoteFx(1, id + 1), 0), Some(-3.0));
        h.modulate_note_fx(ParamAddr::new(Owner::NoteFx(1, id), 4), Some(-3.0));
        assert_eq!(press(&mut h, &mut pads, &mut arps), Some(52));
        h.modulate_note_fx(addr, None);
        assert_eq!(press(&mut h, &mut pads, &mut arps), Some(6));
        // Held again, then the effect leaves and another takes its id.
        h.modulate_note_fx(addr, Some(-4.0));
        assert_eq!(press(&mut h, &mut pads, &mut arps), Some(0));
        let mut other = mpfree_notefx::TrackFx::default();
        other.add(Fx::Transpose { semitones: 1 });
        assert_eq!(other.ids[0], id);
        other.slots[0] = Some(mpfree_notefx::Slot {
            fx: Fx::Chance { percent: 100 },
            on: true,
        });
        assert!(h.set_note_fx(1, other));
        assert_eq!(
            press(&mut h, &mut pads, &mut arps),
            Some(4),
            "nothing held now"
        );
        assert_eq!((h.dropped(), arps.dropped()), (0, 0));
    }

    /// A track going silent on its line ends what its MIDI effects have
    /// on there: a pad held through the track's chain lets go on the bar
    /// line the mute waited for, not when the finger lifts.
    #[test]
    fn a_mute_landing_ends_what_the_tracks_midi_effects_hold() {
        use mpfree_notefx::{Fx, Slot};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut arps = TrackArps::new(RATE, 120.0);
        h.control.project.tracks[1].note_fx.slots[0] = Some(Slot {
            fx: Fx::Transpose { semitones: 2 },
            on: true,
        });
        let period = |h: &mut TimeHost, pads: &mut Fake, arps: &mut TrackArps| {
            h.period_arps(PERIOD, pads, arps);
            pads.frame += u64::from(PERIOD);
        };
        assert!(h.command(Command::Play));
        period(&mut h, &mut pads, &mut arps);
        arps.pad(
            1,
            0,
            Event::NoteOn {
                note: 4,
                velocity: 100,
            },
        );
        while pads.frame < BAR / 2 {
            period(&mut h, &mut pads, &mut arps);
        }
        assert_eq!(pads.hits.last().map(|&(_, pad, _)| pad), Some(6));
        assert!(pads.releases.is_empty());
        h.mute_or_solo(1, true, false).unwrap();
        while pads.frame < BAR + BAR / 2 {
            period(&mut h, &mut pads, &mut arps);
        }
        assert_eq!(h.muted(1), Some(true));
        let off: Vec<(u64, usize)> = pads.releases.clone();
        assert_eq!(off.len(), 1, "{off:?}");
        assert_eq!(off[0].1, 6);
        assert!(off[0].0.abs_diff(BAR) <= 1, "{off:?} at the bar, {BAR}");
        assert_eq!((h.dropped(), arps.dropped()), (0, 0));
    }

    /// STOP while stopped (the transport's panic) silences everything,
    /// every track's arp included, and costs only what sounds. In the
    /// 12 Nov live test each such STOP lost 512 notes in the time engine
    /// and 1 025 in the arps, when each of the 16 arps ended all 128 notes.
    #[test]
    fn a_panic_through_the_arps_loses_nothing() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut arps = arp_up(0);
        let period = |h: &mut TimeHost, pads: &mut Fake, arps: &mut TrackArps| {
            h.period_arps(PERIOD, pads, arps);
            pads.frame += u64::from(PERIOD);
        };
        assert!(h.command(Command::Play));
        for note in [4, 7] {
            arps.pad(
                0,
                0,
                Event::NoteOn {
                    note,
                    velocity: 100,
                },
            );
        }
        for _ in 0..200 {
            period(&mut h, &mut pads, &mut arps);
        }
        assert!(!pads.hits.is_empty(), "the arp plays");
        assert!(h.command(Command::Stop));
        for _ in 0..20 {
            period(&mut h, &mut pads, &mut arps);
        }
        assert_eq!(pads.stops, 0, "the first STOP lets sounds ring");
        assert!(h.command(Command::Stop));
        for _ in 0..20 {
            period(&mut h, &mut pads, &mut arps);
        }
        assert_eq!(pads.stops, 1, "the second silences everything");
        assert_eq!((h.dropped(), arps.dropped()), (0, 0));
        assert!(arps.engine_at(0).idle() && arps.engine_at(0).sounding() == 0);
        // Again and again, as on the MPC: still nothing lost.
        for _ in 0..5 {
            assert!(h.command(Command::Stop));
            period(&mut h, &mut pads, &mut arps);
        }
        assert_eq!((pads.stops, h.dropped(), arps.dropped()), (6, 0, 0));
    }

    /// The pads' side of the stress tests (refactor A2): what sounds, by
    /// track and pad, and the locks the mixer holds, so a note or a lock
    /// left hanging shows.
    #[derive(Default)]
    struct Ledger {
        sounding: std::collections::HashMap<(u16, usize), u32>,
        hits: usize,
        /// Releases of nothing sounding.
        stray: usize,
        stops: usize,
        held: std::collections::HashSet<ParamAddr>,
        most_locks: usize,
    }

    impl Ledger {
        /// The locks and unlocks the period sent the mixer.
        fn take_locks(&mut self, h: &mut TimeHost) {
            self.most_locks = self.most_locks.max(h.locks().len());
            for &(_, addr, value) in h.locks() {
                if value.is_some() {
                    self.held.insert(addr);
                } else {
                    self.held.remove(&addr);
                }
            }
            h.clear_locks();
        }
    }

    impl Pads for Ledger {
        fn hit_at(&mut self, track: u16, pad: usize, _velocity: u8, _offset: u32) -> bool {
            *self.sounding.entry((track, pad)).or_default() += 1;
            self.hits += 1;
            true
        }
        fn release_at(&mut self, track: u16, pad: usize, _offset: u32) {
            match self.sounding.get_mut(&(track, pad)) {
                Some(n) if *n > 1 => *n -= 1,
                Some(_) => {
                    self.sounding.remove(&(track, pad));
                }
                None => self.stray += 1,
            }
        }
        fn stop_all(&mut self) {
            self.sounding.clear();
            self.stops += 1;
        }
        fn pressure_at(&mut self, _track: u16, _pad: usize, _value: u8, _offset: u32) {}
    }

    /// 16 tracks, far past what one period has room for, within the
    /// player's own tables (256 notes sounding, 256 locks held). Slot 2
    /// holds 16 long notes on each track, all on one tick. Slot 1 plays 16
    /// notes on every sixteenth, each ending as the next starts (256
    /// endings and 256 starts on one frame), with 16 locks a track (256
    /// held at once) locked three times on every step (768 values), and
    /// all ending together. Slot 2 plays first; `through` tracks play through
    /// their arps.
    fn crowd(through: usize) -> TimeHost {
        use mpfree_play::mpfree_param::OnTrack;
        use mpfree_seq::{Lock, Note};
        let mut project = Project::new();
        for t in 0..16u16 {
            let mut track = Track::new(&format!("Track {}", t + 1), t);
            track.through_arp = usize::from(t) < through;
            let mut crowd = Clip::new("Crowd", 3840);
            for step in 0..16 {
                let at = step * 240;
                for n in 0..16u8 {
                    crowd.add_note(Note::new(at, 240, n, 100)).unwrap();
                }
                for cc in 0..16u16 {
                    let addr = RelAddr::new(OnTrack::Midi, cc);
                    for sound in [None, Some(0), Some(1)] {
                        crowd
                            .add_lock(Lock::new(at, sound, addr, f32::from(cc) / 16.0))
                            .unwrap();
                    }
                }
            }
            let mut held = Clip::new("Held", 3840);
            for n in 64..80u8 {
                held.add_note(Note::new(0, 3 * 3840, n, 90)).unwrap();
            }
            track.slots[0] = Some(crowd);
            track.slots[1] = Some(held);
            project.tracks.push(track);
        }
        let mut h = TimeHost::with_project(RATE, 120.0, project).unwrap();
        for track in 0..16 {
            h.act(TimeAction::Launch { track, slot: 1 }).unwrap();
        }
        h
    }

    /// Plays [`crowd`]: the held notes for two bars (retriggered on the
    /// loop), then every track switches to the crowd on the bar line while
    /// its notes still hold (256 endings, 256 starts and 768 lock values on
    /// one frame), two bars of it, then STOP. Returns the pads' side
    /// once everything has had time to end.
    fn play_the_crowd(h: &mut TimeHost, mut arps: Option<&mut TrackArps>) -> Ledger {
        let mut pads = Ledger::default();
        let mut until = |h: &mut TimeHost, pads: &mut Ledger, frame: u64| {
            while h.audio.block().start_frame + u64::from(PERIOD) < frame {
                match arps.as_deref_mut() {
                    Some(a) => h.period_arps(PERIOD, pads, a),
                    None => h.period(PERIOD, pads),
                }
                pads.take_locks(h);
                assert_eq!(h.ends_lost(), 0, "at frame {}", h.audio.block().start_frame);
            }
        };
        assert!(h.command(Command::Play));
        until(h, &mut pads, BAR + BAR / 2);
        for track in 0..16 {
            h.act(TimeAction::Launch { track, slot: 0 }).unwrap();
        }
        until(h, &mut pads, 4 * BAR);
        assert!(pads.held.len() > 100, "{} locks held", pads.held.len());
        assert!(h.command(Command::Stop));
        until(h, &mut pads, 5 * BAR);
        assert_eq!(pads.stops, 0, "one STOP lets go, it doesn't panic");
        pads
    }

    /// Every ending gets through a period far too busy (refactor A2): no
    /// note or lock is left hanging, no note-off comes for a note that
    /// never sounded, and what gave way was new notes and locks, counted.
    #[test]
    fn a_crowd_on_one_frame_leaves_nothing_hanging() {
        let mut h = crowd(0);
        let pads = play_the_crowd(&mut h, None);
        assert!(pads.hits > 5_000, "{} hits", pads.hits);
        assert!(
            pads.sounding.is_empty(),
            "left sounding: {:?}",
            pads.sounding
        );
        assert!(pads.held.is_empty(), "locks left held: {:?}", pads.held);
        assert_eq!(pads.stray, 0);
        assert!(pads.most_locks <= MAX_LOCKS, "{}", pads.most_locks);
        assert_eq!(h.ends_lost(), 0);
        assert!(h.busy() > 0, "the crowd was too big for a period");
        assert!(h
            .describe()
            .contains(" new notes or locks left out of moments too busy, 0 endings lost"));
    }

    /// The same crowd through the per-track arps, four tracks playing
    /// their clips through their arps: the endings get through the arps'
    /// table and into each arp, and a panic after it all still silences
    /// everything.
    #[test]
    fn a_crowd_through_the_arps_leaves_nothing_hanging() {
        use mpfree_arp::{Mode, Order, Rate, Settings};
        let mut h = crowd(4);
        let mut arps = TrackArps::new(RATE, 120.0);
        for t in 0..4 {
            arps.engine(t).unwrap().set_settings(Settings {
                mode: Mode::Arp,
                order: Order::Up,
                rate: Rate::Sixteenth,
                gate: 50,
                ..Settings::default()
            });
        }
        let mut pads = play_the_crowd(&mut h, Some(&mut arps));
        assert!(pads.hits > 5_000, "{} hits", pads.hits);
        assert!(
            pads.sounding.is_empty(),
            "left sounding: {:?}",
            pads.sounding
        );
        assert!(pads.held.is_empty(), "locks left held: {:?}", pads.held);
        assert_eq!(pads.stray, 0);
        assert_eq!((h.ends_lost(), arps.lost()), (0, 0));
        assert!(arps.refused() > 0, "the crowd was too big for a period");
        for t in 0..4 {
            assert_eq!(arps.engine_at(t).sounding(), 0, "track {}'s arp", t + 1);
        }
        // A panic after a full table still gets through.
        assert!(h.command(Command::Stop));
        for _ in 0..20 {
            h.period_arps(PERIOD, &mut pads, &mut arps);
        }
        assert_eq!((pads.stops, h.ends_lost(), arps.lost()), (1, 0, 0));
    }

    /// The live pads through a track's arp, while the transport plays: the
    /// arp steps on the transport's sixteenths (period_arps hands every arp
    /// the clock), its notes reach the pads as live playing, and the armed
    /// track records them as heard.
    #[test]
    fn the_pads_arp_steps_on_the_transport_grid_and_records_as_heard() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut arps = arp_up(0);
        // Its own tempo differs: the transport's clock puts it on the
        // transport's grid.
        arps.engine(0).unwrap().set_tempo(97.0);
        h.command(Command::SetLoop(Some((0, 2 * 3840))));
        h.act(TimeAction::NewClip { track: 0, slot: 0 }).unwrap();
        assert_eq!(h.armed(), Some(0));
        while pads.frame < 2 * BAR {
            let f = pads.frame;
            let at = |x: u64| {
                (f..f + u64::from(PERIOD))
                    .contains(&x)
                    .then(|| (x - f) as u32)
            };
            if let Some(o) = at(1000) {
                arps.pad(
                    0,
                    o,
                    Event::NoteOn {
                        note: 4,
                        velocity: 100,
                    },
                );
            }
            if let Some(o) = at(BAR / 2) {
                arps.pad(0, o, Event::NoteOff { note: 4 });
            }
            h.period_arps(PERIOD, &mut pads, &mut arps);
            pads.frame += u64::from(PERIOD);
        }
        let sixteenth = BAR as f64 / 16.0;
        let frames: Vec<u64> = pads.hits.iter().map(|&(f, _, _)| f).collect();
        assert!(frames.len() >= 6 && frames.len() <= 8, "{frames:?}");
        for &f in &frames {
            let steps = f as f64 / sixteenth;
            assert!(
                (steps - steps.round()).abs() * sixteenth <= 1.0,
                "on the transport's grid: {frames:?}"
            );
        }
        assert!(frames[0] >= 1000 && (frames[0] as f64) < 1000.0 + sixteenth);
        assert!(pads.hits.iter().all(|&(_, p, _)| p == 4));
        assert_eq!(h.dropped() + arps.dropped(), 0);
        let clip = h.control.project.tracks[0].slots[0].as_ref().unwrap();
        assert_eq!(
            clip.notes.len(),
            frames.len(),
            "every step recorded as heard"
        );
    }

    /// "Clips through the arp" on: the track records the chord as pressed
    /// (not what the app's arp played), and its clip plays back through
    /// the track's arp, one note per sixteenth.
    #[test]
    fn a_through_track_records_the_chord_and_plays_it_through_its_arp() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        assert!(h.set_through_arp(2, true));
        assert!(h.control.project.tracks[2].through_arp);
        let mut arps = arp_up(2);
        let hits = record_a_chord(&mut h, &mut arps);
        assert!(arps.through(2) && !arps.through(1));
        let clip = h.control.project.tracks[2].slots[0].as_ref().unwrap();
        let mut kept: Vec<u8> = clip.notes.iter().map(|n| n.note).collect();
        kept.sort();
        assert_eq!(kept, [4, 7, 9], "the chord, not the arp's notes");
        let pads: Vec<usize> = hits.iter().map(|&(_, p)| p).collect();
        assert!(pads.len() >= 12, "{hits:?}");
        assert_eq!(&pads[..6], &[4, 7, 9, 4, 7, 9], "up, one at a time");
        let sixteenth = BAR / 16;
        for w in hits.windows(2) {
            assert!(w[1].0.abs_diff(w[0].0 + sixteenth) <= 1, "{hits:?}");
        }
        assert!(hits[0].0 >= 2 * BAR + 1000 - 23 && hits[0].0 < 2 * BAR + 1000 + sixteenth);
        // A set_through_arp past the last track says so.
        assert!(!h.set_through_arp(99, true));
    }

    /// The switch off (the default, for drums): the track records what was
    /// heard, and its clip plays as recorded even though its arp is set
    /// to arpeggiate.
    #[test]
    fn a_track_with_the_switch_off_records_and_plays_what_was_heard() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut arps = arp_up(2);
        let hits = record_a_chord(&mut h, &mut arps);
        let clip = h.control.project.tracks[2].slots[0].as_ref().unwrap();
        let kept: Vec<u8> = clip.notes.iter().map(|n| n.note).collect();
        assert_eq!(kept, [20, 21]);
        let pads: Vec<usize> = hits.iter().map(|&(_, p)| p).collect();
        assert_eq!(pads, [20, 21], "{hits:?}");
        assert!(hits[0].0.abs_diff(2 * BAR + 1000) <= 23);
        assert!(hits[1].0.abs_diff(2 * BAR + 1000 + 5512) <= 23);
    }

    /// A clip launched while the music plays waits for the next bar line,
    /// and the track's clip plays until then.
    #[test]
    fn a_launch_while_playing_waits_for_the_bar_line() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |name: &str, notes: &[(i64, u8)]| {
            let mut c = Clip::new(name, 3840);
            c.notes = notes
                .iter()
                .map(|&(tick, note)| mpfree_seq::Note {
                    tick,
                    len: 100,
                    note,
                    velocity: 100,
                    chance: 100,
                    cond: mpfree_seq::Cond::Always,
                    ratchet: 1,
                    touch: mpfree_seq::Touch::NONE,
                    on: true,
                    ramp: 0,
                    join: mpfree_seq::Join::Attack,
                    group: 0,
                })
                .collect();
            c
        };
        h.control.project.tracks[0].slots[0] = Some(one_bar("A", &[(0, 4), (1920, 4)]));
        h.control.project.tracks[0].slots[1] = Some(one_bar("B", &[(0, 9)]));
        // Stopped: at once.
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        assert_eq!(h.state().launch(0).map(|l| l.slot), Some(0));
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR + BAR / 2, &[]);
        h.act(TimeAction::Launch { track: 0, slot: 1 }).unwrap();
        let st = h.state();
        assert_eq!(
            (st.launch(0).map(|l| l.slot), st.cues.len()),
            (Some(0), 1),
            "B waits for bar 3"
        );
        run(&mut h, &mut pads, 4 * BAR - 1000, &[]);
        let st = h.state();
        assert_eq!((st.launch(0).map(|l| l.slot), st.cues.len()), (Some(1), 0));
        // The view says it shows the one switch made on track 1.
        assert_eq!(
            h.audio.view.includes().map(|i| i.switches[..2].to_vec()),
            Some(vec![1, 0])
        );
        let hits: Vec<(u64, usize)> = pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        let want = [
            (0, 4),
            (BAR / 2, 4),
            (BAR, 4),
            (BAR + BAR / 2, 4),
            (2 * BAR, 9),
            (3 * BAR, 9),
        ];
        assert_eq!(hits.len(), want.len(), "{hits:?}");
        for ((f, p), (at, pad)) in hits.iter().zip(want) {
            assert!(f.abs_diff(at) <= 23 && *p == pad, "{hits:?}");
        }
        assert_eq!(h.dropped(), 0);
    }

    /// Launch on the Record tab (S4): a tapped clip waits for the next
    /// line of the project's launch quantise, a legato clip plays on from
    /// the place in the bar the one it replaces had reached, and a cue
    /// waiting when Launch changes keeps the line it had.
    #[test]
    fn a_launch_lands_on_the_launch_quantise_and_waiting_cues_keep_their_line() {
        use mpfree_seq::LaunchGrid;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |name: &str, notes: &[(i64, u8)]| {
            let mut c = Clip::new(name, 3840);
            c.notes = notes
                .iter()
                .map(|&(tick, note)| mpfree_seq::Note::new(tick, 100, note, 100))
                .collect();
            c
        };
        h.control.project.tracks[0].slots[0] = Some(one_bar("A", &[(0, 4), (1920, 4)]));
        let mut b = one_bar("B", &[(0, 9), (1920, 10)]);
        b.legato = true;
        h.control.project.tracks[0].slots[1] = Some(b);
        assert!(h.act(TimeAction::LaunchGrid(LaunchGrid::Bars(0))).is_err());
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR + BAR / 8, &[]);
        // A quarter note: B lands on bar 2's second beat, a quarter into
        // its pass as A was, so its first note is its half-bar one.
        assert_eq!(
            h.act(TimeAction::LaunchGrid(LaunchGrid::Ticks(960)))
                .unwrap(),
            "launch: a tapped clip, a stop or a scene starts on the next 1/4"
        );
        assert_eq!(h.state().launch, LaunchGrid::Ticks(960));
        h.act(TimeAction::Launch { track: 0, slot: 1 }).unwrap();
        assert_eq!(h.state().cue_line(0), LaunchGrid::Ticks(960));
        run(&mut h, &mut pads, 2 * BAR + BAR / 8, &[]);
        assert_eq!(h.state().launch(0).map(|l| l.slot), Some(1));
        // Back to a bar: A waits for bar 4. Launch goes to 4 bars while it
        // waits, and A keeps its bar line.
        h.act(TimeAction::LaunchGrid(LaunchGrid::Bars(1))).unwrap();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.act(TimeAction::LaunchGrid(LaunchGrid::Bars(4))).unwrap();
        let st = h.state();
        assert_eq!(st.cue_lines, vec![(0, LaunchGrid::Bars(1))]);
        assert_eq!(st.cue_line(0), LaunchGrid::Bars(1));
        run(&mut h, &mut pads, 4 * BAR - 1000, &[]);
        let st = h.state();
        assert_eq!((st.launch(0).map(|l| l.slot), st.cues.len()), (Some(0), 0));
        assert!(st.cue_lines.is_empty());
        let hits: Vec<(u64, usize)> = pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        let want = [
            (0, 4),
            (BAR / 2, 4),
            (BAR, 4),
            (BAR + BAR / 2, 10),
            (2 * BAR, 9),
            (2 * BAR + BAR / 2, 10),
            (3 * BAR, 4),
            (3 * BAR + BAR / 2, 4),
        ];
        assert_eq!(hits.len(), want.len(), "{hits:?}");
        for ((f, p), (at, pad)) in hits.iter().zip(want) {
            assert!(f.abs_diff(at) <= 23 && *p == pad, "{hits:?}");
        }
        // A new tap while one waits lands on the line Launch gives now.
        h.act(TimeAction::Launch { track: 0, slot: 1 }).unwrap();
        h.act(TimeAction::LaunchGrid(LaunchGrid::Bars(1))).unwrap();
        assert_eq!(h.state().cue_line(0), LaunchGrid::Bars(4));
        h.act(TimeAction::Launch { track: 0, slot: 1 }).unwrap();
        assert_eq!(h.state().cue_line(0), LaunchGrid::Bars(1));
        assert_eq!(h.dropped(), 0);
    }

    /// Live transpose (sequencer S4): the track and every track that
    /// follows move together, a track that doesn't follow stays, and past
    /// two octaves is refused. The audio half has it for the pads, and
    /// the screen's state shows it.
    #[test]
    fn transpose_moves_the_track_and_the_tracks_that_follow_it() {
        use mpfree_seq::Transpose;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let five = Transpose::semitones(5);
        assert_eq!(
            h.act(TimeAction::Transpose { track: 0, to: five }).unwrap(),
            "transpose: Track 1 +5 st"
        );
        assert_eq!(
            h.act(TimeAction::FollowsTranspose { track: 2, on: true })
                .unwrap(),
            "follows transpose: Track 3 moves with the armed track's transpose"
        );
        let key = Transpose {
            by: -3,
            in_key: true,
        };
        assert_eq!(
            h.act(TimeAction::Transpose { track: 0, to: key }).unwrap(),
            "transpose: Track 1 -3 in key, with Track 3"
        );
        let st = h.state();
        assert_eq!(st.tracks[0].transpose, key);
        assert_eq!(st.tracks[1].transpose, Transpose::NONE);
        assert!(st.tracks[2].follows);
        assert_eq!(st.tracks[2].transpose, key);
        let mut pads = Fake::default();
        run(&mut h, &mut pads, 512, &[]);
        let audio = h.transposes();
        assert_eq!((audio[0], audio[1], audio[2]), (key, Transpose::NONE, key));
        assert!(h
            .act(TimeAction::Transpose {
                track: 0,
                to: Transpose::semitones(25)
            })
            .is_err());
        assert!(h
            .act(TimeAction::Transpose {
                track: 99,
                to: five
            })
            .is_err());
        assert_eq!(
            h.act(TimeAction::FollowsTranspose {
                track: 2,
                on: false
            })
            .unwrap(),
            "follows transpose: Track 3 keeps its own"
        );
        assert_eq!(h.dropped(), 0);
    }

    /// A row's scene button: stopped, the row plays at once; playing, it
    /// waits for the bar line, where track 2 goes quiet (the scene mutes
    /// it) and track 1 switches row. Row 2 follows on to row 1 after a bar.
    #[test]
    fn a_scene_plays_its_row_with_its_mutes_and_follows_on() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |note: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            c
        };
        for (t, base) in [(0, 4), (1, 30)] {
            h.control.project.tracks[t].slots[0] = Some(one_bar(base));
            h.control.project.tracks[t].slots[1] = Some(one_bar(base + 1));
        }
        let s1 = h.control.project.scene_mut(1).unwrap();
        s1.mutes = Some(0b10);
        s1.follow = Some(mpfree_seq::Follow::after(
            1,
            mpfree_seq::FollowAction::Previous,
        ));
        h.control.project.scene_mut(0).unwrap().mutes = Some(0);
        assert_eq!(h.act(TimeAction::LaunchScene(0)).unwrap(), "scene 1");
        let st = h.state();
        assert_eq!((st.launches.len(), st.scene_playing), (2, Some(0)));
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.act(TimeAction::LaunchScene(1)).unwrap();
        assert_eq!(h.state().scene_cued, Some(1));
        run(&mut h, &mut pads, 4 * BAR - 1000, &[]);
        let hits: Vec<(u64, usize)> = pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        // Bar 1: row 1. Bar 2: row 2, track 2 muted. Bar 3: back to row 1
        // (the follow), track 2 heard again. Bar 4: row 1.
        let want = [
            (0, 4),
            (0, 30),
            (BAR, 5),
            (2 * BAR, 4),
            (2 * BAR, 30),
            (3 * BAR, 4),
            (3 * BAR, 30),
        ];
        assert_eq!(hits.len(), want.len(), "{hits:?}");
        for ((f, p), (at, pad)) in hits.iter().zip(want) {
            assert!(f.abs_diff(at) <= 23 && *p == pad, "{hits:?}");
        }
        let st = h.state();
        assert_eq!(
            (st.scene_playing, st.scene_cued, st.tracks[1].mute),
            (Some(0), None, false)
        );
        assert_eq!(h.act(TimeAction::StopAll).unwrap(), "stop all");
        run(&mut h, &mut pads, 6 * BAR, &[]);
        assert!(h.state().launches.is_empty());
        assert!(h.act(TimeAction::LaunchScene(mpfree_seq::ROWS)).is_err());
        assert_eq!(h.dropped(), 0);
    }

    /// The Arrange refactor's 5.2b: a scene's tempo comes in on its line,
    /// on the frame its clips start, and the app takes it once; a scene
    /// with none leaves the tempo be. A tempo turned after a launch is
    /// the newer and wins; while song mode plays the song's tempo points
    /// they lead; stopped, it comes in at once.
    #[test]
    fn a_scenes_tempo_comes_in_on_its_line() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |note: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            c
        };
        for row in 0..3 {
            h.control.project.tracks[0].slots[row] = Some(one_bar(4 + row as u8));
        }
        let p = &mut h.control.project;
        p.scene_mut(1).unwrap().set_bpm(Some(150.0)).unwrap();
        p.scene_mut(2).unwrap().set_bpm(Some(90.0)).unwrap();
        // The app's side: its tempo each period, taking what landed.
        let mut app = 120.0;
        let mut took = vec![];
        let period = |h: &mut TimeHost, pads: &mut Fake, app: &mut f64, took: &mut Vec<_>| {
            if let Some(bpm) = h.take_scene_tempo() {
                took.push((pads.frame, bpm));
                *app = bpm;
            }
            h.follow_tempo(*app);
            h.period(PERIOD, pads);
            pads.frame += u64::from(PERIOD);
        };
        h.act(TimeAction::LaunchScene(0)).unwrap();
        h.command(Command::Play);
        while pads.frame < BAR / 2 {
            period(&mut h, &mut pads, &mut app, &mut took);
        }
        // Scene 2 at 150 BPM, on bar 2: bar 2 is two seconds in, bar 3
        // 1.6 s after it.
        h.act(TimeAction::LaunchScene(1)).unwrap();
        let mut heard = vec![];
        while pads.frame < BAR + BAR * 4 / 5 + BAR / 2 {
            period(&mut h, &mut pads, &mut app, &mut took);
            heard.push((h.info().position, h.info().bpm));
        }
        let hits: Vec<(u64, usize)> = pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        let want = [(0, 4), (BAR, 5), (BAR + BAR * 4 / 5, 5)];
        assert_eq!(hits.len(), want.len(), "{hits:?}");
        for ((f, p), (at, pad)) in hits.iter().zip(want) {
            assert!(f.abs_diff(at) <= 23 && *p == pad, "{hits:?}");
        }
        assert!(heard.iter().filter(|x| x.0 < 3840).all(|x| x.1 == 120.0));
        assert!(heard.iter().filter(|x| x.0 > 3840).all(|x| x.1 == 150.0));
        assert_eq!(took.len(), 1, "{took:?}");
        assert!(took[0].0.abs_diff(BAR) <= u64::from(2 * PERIOD));
        assert_eq!((took[0].1, app), (150.0, 150.0));
        // Scene 1, with none: the tempo stays.
        h.act(TimeAction::LaunchScene(0)).unwrap();
        for _ in 0..BAR / u64::from(PERIOD) {
            period(&mut h, &mut pads, &mut app, &mut took);
        }
        assert_eq!((h.info().bpm, took.len()), (150.0, 1));
        // Scene 3 at 90, then the tempo turned to 100 before its line:
        // the turn is the newer.
        h.act(TimeAction::LaunchScene(2)).unwrap();
        app = 100.0;
        for _ in 0..2 * BAR / u64::from(PERIOD) {
            period(&mut h, &mut pads, &mut app, &mut took);
        }
        assert_eq!(h.state().scene_playing, Some(2));
        assert_eq!((h.info().bpm, took.len()), (100.0, 1));
        // Song mode with a tempo point: the song's tempo leads.
        h.act(TimeAction::SongTempo(mpfree_time_ui::TempoEdit::Add(0)))
            .unwrap();
        h.act(TimeAction::SongMode(true)).unwrap();
        h.act(TimeAction::LaunchScene(1)).unwrap();
        for _ in 0..2 * BAR / u64::from(PERIOD) {
            period(&mut h, &mut pads, &mut app, &mut took);
        }
        assert_eq!(h.state().scene_playing, Some(1));
        assert_eq!(took.len(), 1);
        assert!(h.song_tempo_plays());
        h.act(TimeAction::SongMode(false)).unwrap();
        // Stopped: at once.
        h.command(Command::Stop);
        period(&mut h, &mut pads, &mut app, &mut took);
        h.act(TimeAction::LaunchScene(2)).unwrap();
        period(&mut h, &mut pads, &mut app, &mut took);
        period(&mut h, &mut pads, &mut app, &mut took);
        assert_eq!((h.info().bpm, app, took.len()), (90.0, 90.0, 2));
        assert_eq!(h.dropped(), 0);
    }

    /// A scene that mutes a track it gives no cue (an empty cell, nothing
    /// launched): in song mode that track plays the song, so its mute
    /// waits in the mute queue and lands on the scene's bar line with the
    /// scene's cue, not at the tap. Every 16th of the song plays up to the
    /// line and none after it, at every period size, and with the track
    /// soloed.
    #[test]
    fn a_scenes_mute_on_a_track_playing_the_song_lands_on_its_line() {
        use mpfree_seq::arrange::Placement;
        use mpfree_seq::Note;
        let sixteenths = |pad: u8| {
            let mut c = Clip::new("16ths", 3840);
            c.notes = (0..16).map(|k| Note::new(k * 240, 60, pad, 100)).collect();
            c
        };
        let one_bar = |pad: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![Note::new(0, 100, pad, 100)];
            c
        };
        let cases = [1u32, 7, 64, 128, 1000, 4096].map(|p| (p, false));
        for (period, solo) in cases.into_iter().chain([(128, true)]) {
            let mut h = TimeHost::new(RATE, 120.0).unwrap();
            let mut pads = Fake::default();
            let by = |h: &mut TimeHost, pads: &mut Fake, to: u64| {
                while pads.frame < to {
                    h.period(period, pads);
                    pads.frame += u64::from(period);
                }
            };
            let p = &mut h.control.project;
            p.tracks[0].slots[1] = Some(one_bar(9));
            p.tracks[1].slots[2] = Some(sixteenths(30));
            p.give_clip_ids();
            p.arrangement
                .placements
                .push(Placement::of(&p.tracks, 1, 2, 0, 8 * 3840));
            p.scene_mut(1).unwrap().mutes = Some(0b10);
            if solo {
                p.tracks[0].solo = true;
                p.tracks[1].solo = true;
            }
            h.act(TimeAction::SongMode(true)).unwrap();
            h.command(Command::Play);
            by(&mut h, &mut pads, BAR / 2 + 1000);
            assert_eq!(h.act(TimeAction::LaunchScene(1)).unwrap(), "scene 2");
            let st = h.state();
            assert!(!st.tracks[1].mute, "period {period}: not at the tap");
            let q = h.mute_wait().expect("it waits in the mute queue");
            assert_eq!(q.want, vec![(1, true, solo)], "period {period}");
            by(&mut h, &mut pads, 2 * BAR - 1000);
            let song: Vec<u64> = pads
                .hits
                .iter()
                .filter(|h| h.1 == 30)
                .map(|h| h.0)
                .collect();
            assert_eq!(song.len(), 16, "period {period}, solo {solo}: {song:?}");
            for (k, at) in song.iter().enumerate() {
                let want = (k as u64 * BAR) / 16;
                assert!(at.abs_diff(want) <= 1, "period {period}: {song:?}");
            }
            let row: Vec<u64> = pads.hits.iter().filter(|h| h.1 == 9).map(|h| h.0).collect();
            assert_eq!(row.len(), 1, "period {period}: {:?}", pads.hits);
            assert!(row[0].abs_diff(BAR) <= 1, "the cue's line: {row:?}");
            assert!(h.state().tracks[1].mute, "period {period}: muted now");
            assert!(h.mute_wait().is_none(), "period {period}: landed");
            assert_eq!(h.dropped(), 0);
        }
    }

    /// Loop tracks: a loop cell launched at once while stopped; while
    /// playing, the loops switch on the cue's bar line, which the plan
    /// gives for that one period. TRACK MUTE, a scene and Stop all switch
    /// them too; a loop in no cell plays throughout.
    #[test]
    fn a_loop_cell_launches_its_loop_on_the_bar_line_as_a_clip_does() {
        use mpfree_seq::LOOPS;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        assert_eq!(h.loop_plan().on, [true; LOOPS], "every loop plays");
        let mut t = Track::new_loops("Loops A", 8);
        t.slots[0] = Some(Clip::new_loop(0, 3840));
        t.slots[1] = Some(Clip::new_loop(1, 3840));
        h.control.project.tracks.push(t);
        h.act(TimeAction::Launch { track: 8, slot: 0 }).unwrap();
        let p = h.loop_plan();
        assert_eq!(
            (&p.on[..3], p.at),
            (&[true, false, true][..], [None; LOOPS])
        );
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.act(TimeAction::Launch { track: 8, slot: 1 }).unwrap();
        assert_eq!(&h.loop_plan().on[..2], &[true, false], "it waits");
        // One period at a time to bar 2's line: the period with it says.
        let mut said = Vec::new();
        let step = |h: &mut TimeHost, pads: &mut Fake| {
            let to = pads.frame + u64::from(PERIOD);
            run(h, pads, to, &[]);
            h.loop_plan()
        };
        while pads.frame < BAR + 4 * u64::from(PERIOD) {
            let p = step(&mut h, &mut pads);
            if p.at[0].is_some() {
                said.push((pads.frame, p));
            }
        }
        let [(end, p)] = said[..] else {
            panic!("one period says the line: {said:?}")
        };
        assert!(end - u64::from(PERIOD) <= BAR && BAR < end, "{end}");
        assert_eq!(&p.on[..3], &[false, true, true]);
        assert_eq!((p.at[0], p.at[1], p.at[2]), (Some(3840), Some(3840), None));
        // TRACK MUTE: off at once, and back.
        h.toggle_mute(8).unwrap();
        assert_eq!(
            (&h.loop_plan().on[..2], h.loop_plan().at[1]),
            (&[false, false][..], None)
        );
        h.toggle_mute(8).unwrap();
        assert!(h.loop_plan().on[1]);
        // A scene: row 1's loop from the next line.
        assert_eq!(h.act(TimeAction::LaunchScene(0)).unwrap(), "scene 1");
        assert_eq!(&h.loop_plan().on[..2], &[false, true]);
        run(&mut h, &mut pads, 2 * BAR + 4 * u64::from(PERIOD), &[]);
        assert_eq!(&h.loop_plan().on[..2], &[true, false]);
        // Stop all: off on the next line.
        h.act(TimeAction::StopAll).unwrap();
        run(&mut h, &mut pads, 3 * BAR + 4 * u64::from(PERIOD), &[]);
        let p = h.loop_plan();
        assert_eq!(
            (&p.on[..2], p.on[2..].iter().all(|&on| on)),
            (&[false, false][..], true)
        );
        assert_eq!(h.dropped(), 0);
    }

    /// A Loop track's mute that waits: its loop goes off on the line, and
    /// the plan gives that line for the one period that has it.
    #[test]
    fn a_loop_tracks_mute_that_waits_turns_its_loop_off_on_the_line() {
        use mpfree_seq::LOOPS;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut t = Track::new_loops("Loops A", 8);
        t.slots[0] = Some(Clip::new_loop(0, 3840));
        h.control.project.tracks.push(t);
        h.act(TimeAction::Launch { track: 8, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.mute_or_solo(8, true, false).unwrap();
        assert!(h.loop_plan().on[0], "it plays until the line");
        let mut said = Vec::new();
        while pads.frame < BAR + 4 * u64::from(PERIOD) {
            let to = pads.frame + u64::from(PERIOD);
            run(&mut h, &mut pads, to, &[]);
            let p = h.loop_plan();
            if p.at[0].is_some() {
                said.push((pads.frame, p));
            }
        }
        let [(end, p)] = said[..] else {
            panic!("one period says the line: {said:?}")
        };
        assert!(end - u64::from(PERIOD) <= BAR && BAR < end, "{end}");
        assert_eq!((p.on[0], p.at[0], p.at[1]), (false, Some(3840), None));
        assert!(p.on[1..].iter().all(|&on| on), "loops in no cell play on");
        assert_eq!(h.muted(8), Some(true));
        assert_eq!(h.loop_plan().at, [None; LOOPS]);
        assert_eq!(h.dropped(), 0);
    }

    /// A loop recorded lands on a Loop track (a new one when every Loop
    /// track plays), launched; its cells follow its length and empty when
    /// it is cleared. The pads stay on note tracks, a Loop track takes no
    /// note clip, and PLAY still starts the Play page's pattern.
    #[test]
    fn a_recorded_loop_lands_on_a_loop_track_and_its_cells_follow_it() {
        use mpfree_seq::loops::LoopNow;
        use mpfree_seq::{LoopInput, LOOPS};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut now = [LoopNow::default(); LOOPS];
        assert!(h.loops_are(&now).is_empty());
        let pass = |ticks, origin| LoopNow {
            pass: true,
            ticks,
            origin,
        };
        now[0] = pass(7680, 0);
        assert_eq!(h.loops_are(&now), ["Loop 1 on Loops A, row 1, playing"]);
        assert!(h.loops_are(&now).is_empty(), "said once");
        let st = h.state();
        assert_eq!(
            (st.tracks.len(), st.tracks[8].loops),
            (9, Some(LoopInput::Pads))
        );
        let cell = st.clip(8, 0).map(|c| (c.name.clone(), c.length, c.notes));
        assert_eq!(cell, Some(("Loop 1".to_string(), 7680, 0)));
        assert_eq!(st.launch(8).map(|l| (l.slot, l.origin)), Some((0, 0)));
        // Loops A plays it, so the next one gets a Loop track of its own.
        now[1] = pass(3840, 3840);
        assert_eq!(h.loops_are(&now), ["Loop 2 on Loops B, row 1, playing"]);
        assert_eq!(&h.loop_plan().on[..3], &[true, true, true]);
        // The pads stay on their track; no note clip goes on a Loop track.
        let armed = h.act(TimeAction::Arm(Some(8))).unwrap();
        assert_eq!(
            armed,
            "armed: Loops A plays loops, the pads stay on Track 1"
        );
        assert_eq!(h.armed(), Some(0));
        assert!(h.act(TimeAction::NewClip { track: 8, slot: 3 }).is_err());
        let to_notes = TimeAction::DropClip {
            from: (8, 0),
            to: (0, 3),
            copy: false,
        };
        assert!(h.act(to_notes).is_err());
        let to_b = TimeAction::DropClip {
            from: (8, 0),
            to: (9, 3),
            copy: true,
        };
        assert!(h.act(to_b).is_err(), "a copy on another Loop track");
        // PLAY still starts the pattern with the loops launched.
        h.control.project.tracks[0].slots[0] = Some(Clip::new("Beat", 3840));
        let play = h.act(TimeAction::Transport(Command::Play)).unwrap();
        assert!(play.contains("the Play page's pattern"), "{play}");
        h.act(TimeAction::Transport(Command::Stop)).unwrap();
        // Half: its cell follows the length.
        now[0].ticks = 3840;
        assert!(h.loops_are(&now).is_empty());
        assert_eq!(h.state().clip(8, 0).map(|c| c.length), Some(3840));
        // Cleared: its cell empties and its launch goes; loop 2 plays on.
        now[0] = LoopNow::default();
        assert_eq!(h.loops_are(&now), ["Loop 1 cleared: its cells are empty"]);
        let st = h.state();
        assert!(st.clip(8, 0).is_none() && st.launch(8).is_none());
        assert_eq!(&h.loop_plan().on[..2], &[true, true]);
        // Its pass back (the looper's redo): back in the cell it left.
        now[0] = pass(3840, 0);
        assert_eq!(h.loops_are(&now), ["Loop 1 on Loops A, row 1, playing"]);
        // A loop recorded with row 3's scene playing: in row 3, on Loops A,
        // where nothing plays now.
        h.act(TimeAction::LaunchScene(2)).unwrap();
        now[2] = pass(3840, 7680);
        assert_eq!(h.loops_are(&now), ["Loop 3 on Loops A, row 3, playing"]);
        // Cleared and recorded anew (another pass): where a new loop lands.
        now[2] = LoopNow::default();
        h.loops_are(&now);
        h.act(TimeAction::LaunchScene(0)).unwrap();
        now[2] = pass(7680, 15360);
        assert_eq!(h.loops_are(&now), ["Loop 3 on Loops C, row 1, playing"]);
        assert_eq!(h.control.project.check(), Ok(()));
    }

    /// Song mode (loop-tracks.md part 5): a Loop track's blocks turn their
    /// loops on and off on their own ticks, as its clips' blocks play;
    /// a cell launched on the track takes it from the song.
    #[test]
    fn the_songs_loop_blocks_turn_their_loops_on_their_ticks() {
        use mpfree_seq::arrange::Placement;
        use mpfree_seq::LOOPS;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut t = Track::new_loops("Loops A", 8);
        t.slots[0] = Some(Clip::new_loop(0, 3840));
        t.slots[1] = Some(Clip::new_loop(1, 3840));
        h.control.project.tracks.push(t);
        h.control.project.give_clip_ids();
        let tracks = &h.control.project.tracks;
        // Loop 1 over bars 2 and 3, loop 2 over bar 4.
        h.control.project.arrangement.placements = vec![
            Placement::of(tracks, 8, 0, 3840, 7680),
            Placement::of(tracks, 8, 1, 11520, 3840),
        ];
        h.act(TimeAction::SongMode(true)).unwrap();
        run(&mut h, &mut pads, u64::from(PERIOD), &[]);
        assert_eq!(&h.loop_plan().on[..3], &[false, false, true]);
        h.command(Command::Play);
        let mut said = Vec::new();
        while pads.frame < 5 * BAR + 4 * u64::from(PERIOD) {
            let to = pads.frame + u64::from(PERIOD);
            run(&mut h, &mut pads, to, &[]);
            let p = h.loop_plan();
            if p.at[0].is_some() || p.at[1].is_some() {
                said.push((p.on[0], p.on[1], p.at[0], p.at[1]));
            }
        }
        assert_eq!(
            said,
            [
                (true, false, Some(3840), None),
                (false, true, Some(11520), Some(11520)),
                (false, false, None, Some(15360)),
            ]
        );
        // A cell launched on the track: the song leaves it.
        h.command(Command::Stop);
        h.command(Command::Locate(3840 + 100));
        let to = pads.frame + u64::from(PERIOD);
        run(&mut h, &mut pads, to, &[]);
        assert!(h.loop_plan().on[0], "stopped inside loop 1's block");
        h.act(TimeAction::Launch { track: 8, slot: 1 }).unwrap();
        let to = pads.frame + u64::from(PERIOD);
        run(&mut h, &mut pads, to, &[]);
        assert_eq!(&h.loop_plan().on[..2], &[false, true]);
        // Song mode off: only the launch.
        h.act(TimeAction::StopAll).unwrap();
        h.act(TimeAction::SongMode(false)).unwrap();
        let to = pads.frame + u64::from(PERIOD);
        run(&mut h, &mut pads, to, &[]);
        assert_eq!(&h.loop_plan().on[..2], &[false, false]);
        assert!(h.loop_plan().on[2..LOOPS].iter().all(|&on| on));
        // A loop's block stays on its lane, and says so.
        let off = h.act(TimeAction::MovePlacement {
            index: 0,
            track: 0,
            start: 0,
            copy: false,
        });
        assert!(off.unwrap_err().contains("stays on its own lane"));
        assert_eq!(h.control.project.arrangement.placements[0].track, 8);
        assert_eq!(h.dropped(), 0);
    }

    /// Loop tracks part 6: the same loop in two rows of its Loop track
    /// plays on through a scene change between them; a loop's cell copied
    /// onto another Loop track takes an empty loop for the copy there, off
    /// until launched, and its cell goes if the copy stops with nothing
    /// in.
    #[test]
    fn the_same_loop_in_two_rows_plays_on_and_a_copy_takes_a_new_loop() {
        use mpfree_seq::loops::LoopNow;
        use mpfree_seq::LOOPS;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut a = Track::new_loops("Loops A", 8);
        a.slots[0] = Some(Clip::new_loop(0, 3840));
        a.slots[1] = Some(Clip::new_loop(0, 3840));
        h.control.project.tracks.push(a);
        h.control
            .project
            .tracks
            .push(Track::new_loops("Loops B", 9));
        let pass = LoopNow {
            pass: true,
            ticks: 3840,
            origin: 0,
        };
        let mut now = [LoopNow::default(); LOOPS];
        now[0] = pass;
        assert!(h.loops_are(&now).is_empty(), "in its cells already");
        h.act(TimeAction::LaunchScene(0)).unwrap();
        h.command(Command::Play);
        let to = pads.frame + BAR;
        run(&mut h, &mut pads, to, &[]);
        assert!(h.loop_plan().on[0]);
        // Row 2's scene: loop 1 is in it too, so it never goes off.
        h.act(TimeAction::LaunchScene(1)).unwrap();
        while pads.frame < 3 * BAR {
            let to = pads.frame + u64::from(PERIOD);
            run(&mut h, &mut pads, to, &[]);
            assert!(h.loop_plan().on[0], "off at {}", pads.frame);
        }
        let slot = h
            .state()
            .launches
            .iter()
            .find(|l| l.track == 8)
            .map(|l| l.slot);
        assert_eq!(slot, Some(1), "row 2's cell plays it");

        // A copy onto Loops B row 3: Loop 4 there, off until launched.
        let copy = |into| TimeAction::LoopCopy {
            from: (8, 0),
            to: (9, 2),
            into,
            id: 0,
        };
        assert!(h.act(copy(0)).is_err(), "loop 1 is in cells");
        let bad = |from, to| TimeAction::LoopCopy {
            from,
            to,
            into: 3,
            id: 0,
        };
        assert!(h.act(bad((9, 0), (9, 2))).is_err(), "an empty cell");
        assert!(h.act(bad((8, 0), (8, 2))).is_err(), "its own Loop track");
        assert!(h.act(bad((8, 0), (0, 5))).is_err(), "a note track");
        let said = h.act(copy(3)).unwrap();
        assert!(
            said.starts_with("clips: Loop 1 copies into Loop 4 on Loops B / 3"),
            "{said}"
        );
        let cell = h.control.project.clip(9, 2).unwrap();
        assert_eq!(
            (cell.looped, cell.length, cell.name.as_str()),
            (Some(3), 3840, "Loop 4")
        );
        assert!(!h.loop_plan().on[3], "off until launched");
        assert!(h.act(copy(4)).unwrap_err().contains("has a loop"));
        // Nothing went in: its cell goes.
        let off = h.act(TimeAction::LoopCopyOff(3)).unwrap();
        assert_eq!(off, "clips: Loop 4's copy stopped: its cell is empty");
        assert!(h.control.project.clip(9, 2).is_none());
        // Again, and its first layer goes in: the cell stays, as it is.
        h.act(copy(3)).unwrap();
        now[3] = pass;
        assert!(h.loops_are(&now).is_empty(), "no landing: it has its cell");
        assert_eq!(h.act(TimeAction::LoopCopyOff(3)).unwrap(), "");
        assert_eq!(h.control.project.clip(9, 2).and_then(|c| c.looped), Some(3));
        assert!(!h.loop_plan().on[3]);
        h.act(TimeAction::Launch { track: 9, slot: 2 }).unwrap();
        let to = pads.frame + BAR;
        run(&mut h, &mut pads, to, &[]);
        assert!(h.loop_plan().on[3] && h.loop_plan().on[0], "both play");
        assert_eq!(h.control.project.check(), Ok(()));
        assert_eq!(h.dropped(), 0);
    }

    /// SHIFT + "+ Track" adds an empty Loop track (UNDO takes it away), its
    /// sound line steps its input, and a tap on an empty cell of it arms
    /// that cell: the next loop recorded lands there, playing, and the
    /// loop the track played stops (loop-tracks.md part 4).
    #[test]
    fn an_armed_loop_cell_takes_the_next_loop_recorded() {
        use mpfree_seq::loops::LoopNow;
        use mpfree_seq::{LoopInput, LOOPS};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.act(TimeAction::Arm(Some(2))).unwrap();
        let said = h.act(TimeAction::AddLoopTrack).unwrap();
        assert!(said.starts_with("Loops A added, a Loop track"), "{said}");
        let st = h.state();
        assert_eq!(
            (st.tracks.len(), st.tracks[8].loops),
            (9, Some(LoopInput::Pads))
        );
        assert_eq!(h.armed(), Some(2), "the pads stay where they were");
        // Its input steps along, and round.
        let said = h.act(TimeAction::LoopInput(8)).unwrap();
        assert_eq!(said, "Loops A records Line in");
        h.act(TimeAction::LoopInput(8)).unwrap();
        assert_eq!(h.state().tracks[8].loops, Some(LoopInput::Resample));
        h.act(TimeAction::LoopInput(8)).unwrap();
        assert_eq!(h.state().tracks[8].loops, Some(LoopInput::Pads));
        assert!(h.act(TimeAction::LoopInput(0)).is_err(), "a note track");
        // Only an empty cell of a Loop track arms.
        assert!(h.act(TimeAction::LoopCell(Some((0, 1)))).is_err());
        assert!(h.act(TimeAction::LoopCell(Some((8, 99)))).is_err());
        h.act(TimeAction::LoopCell(Some((8, 2)))).unwrap();
        let st = h.state();
        assert_eq!((h.loop_cell(), st.loop_cell), (Some((8, 2)), Some((8, 2))));
        let name = mpfree_time_ui::view::loop_cell_name(&st);
        assert_eq!(name.as_deref(), Some("Loops A · row 3"));
        // The next loop recorded lands there, playing, and the arm goes.
        let pass = |ticks| LoopNow {
            pass: true,
            ticks,
            origin: 0,
        };
        let mut now = [LoopNow::default(); LOOPS];
        now[0] = pass(7680);
        assert_eq!(h.loops_are(&now), ["Loop 1 on Loops A, row 3, playing"]);
        assert_eq!((h.loop_cell(), h.loop_on(8)), (None, Some(0)));
        // Row 1 armed: the next loop plays there, and loop 1 stops.
        h.act(TimeAction::LoopCell(Some((8, 0)))).unwrap();
        let full = h.act(TimeAction::LoopCell(Some((8, 2)))).unwrap_err();
        assert_eq!(full, "row 3 of Loops A holds a loop");
        assert_eq!(h.loop_cell(), Some((8, 0)), "the arm stays");
        now[1] = pass(3840);
        assert_eq!(h.loops_are(&now), ["Loop 2 on Loops A, row 1, playing"]);
        assert_eq!(h.loop_on(8), Some(1));
        assert_eq!(&h.loop_plan().on[..2], &[false, true]);
        // Let go, the next loop lands as before: Loops A plays, so on a
        // new Loops B.
        h.act(TimeAction::LoopCell(Some((8, 1)))).unwrap();
        h.act(TimeAction::LoopCell(None)).unwrap();
        assert_eq!(h.loop_cell(), None);
        now[2] = pass(3840);
        assert_eq!(h.loops_are(&now), ["Loop 3 on Loops B, row 1, playing"]);
        // An armed cell that fills (a loop dropped there) arms nothing.
        h.act(TimeAction::LoopCell(Some((9, 1)))).unwrap();
        h.act(TimeAction::DropClip {
            from: (9, 0),
            to: (9, 1),
            copy: false,
        })
        .unwrap();
        assert_eq!(h.loop_cell(), None);
        assert_eq!(h.control.project.check(), Ok(()));
    }

    /// Loops on audio tracks (Dev's switch, audio tracks phase 4b): an
    /// audio track's empty cell arms for the next loop recorded, which
    /// lands there as a loop cell naming its lasting id, launched from the
    /// loop's own start; the cell follows the loop's length, empties when
    /// it is cleared and comes back with a redo of its first pass. With no
    /// id yet, or one named already, the loop lands as before and the arm
    /// stays. With the switch off an audio track's cell doesn't arm.
    #[test]
    fn with_loops_on_an_armed_audio_cell_takes_the_next_loop_recorded() {
        use mpfree_seq::loops::LoopNow;
        use mpfree_seq::{LoopId, LOOPS};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.act(TimeAction::AddAudioTrack).unwrap();
        let audio = h.project().tracks.len() - 1;
        assert!(h.project().tracks[audio].is_audio());
        let off = h.act(TimeAction::LoopCell(Some((audio, 1)))).unwrap_err();
        assert_eq!(off, format!("track {} is not a Loop track", audio + 1));
        h.set_loops_on(true);
        assert!(
            h.act(TimeAction::LoopCell(Some((0, 1)))).is_err(),
            "a note track"
        );
        let said = h.act(TimeAction::LoopCell(Some((audio, 1)))).unwrap();
        assert_eq!(
            said,
            "loop cell: Audio 1 row 2 armed, the next loop recorded lands there"
        );
        assert_eq!(h.loop_cell(), Some((audio, 1)));
        let pass = |ticks, origin| LoopNow {
            pass: true,
            ticks,
            origin,
        };
        let mut now = [LoopNow::default(); LOOPS];
        let mut ids = [0u128; LOOPS];
        // No id yet: it lands as before, on a new Loop track, and the arm
        // stays for the next.
        now[0] = pass(3840, 0);
        assert_eq!(
            h.loops_are_on(&now, &ids),
            ["Loop 1 on Loops A, row 1, playing"]
        );
        assert_eq!(h.loop_cell(), Some((audio, 1)));
        // Loop 2 with its id: in the audio cell, launched from its start.
        ids[1] = 0xbeef;
        now[1] = pass(7680, 1920);
        assert_eq!(
            h.loops_are_on(&now, &ids),
            ["Loop 2 on Audio 1, row 2, playing"]
        );
        assert_eq!(h.loop_cell(), None, "the arm goes");
        let cell = h.project().tracks[audio].slots[1].as_ref().unwrap();
        assert_eq!(cell.loop_id(), Some(LoopId(0xbeef)));
        assert_eq!((cell.name.as_str(), cell.length), ("Loop 2", 7680));
        assert_eq!(cell.looped, None, "not a Loop track's cell");
        assert!(h.launched(audio));
        assert_eq!(h.project().loop_track(LoopId(0xbeef)), Some(audio));
        assert_eq!(h.control.project.check(), Ok(()));
        // Its length follows the loop.
        now[1] = pass(11_520, 1920);
        assert!(h.loops_are_on(&now, &ids).is_empty());
        let cell = h.project().tracks[audio].slots[1].as_ref().unwrap();
        assert_eq!(cell.length, 11_520);
        // Cleared: the cell empties. Its first pass back (a redo): back in
        // the cell.
        now[1] = LoopNow::default();
        assert_eq!(
            h.loops_are_on(&now, &ids),
            ["Loop 2 cleared: its cells are empty"]
        );
        assert!(h.project().tracks[audio].slots[1].is_none());
        now[1] = pass(11_520, 1920);
        assert_eq!(
            h.loops_are_on(&now, &ids),
            ["Loop 2 on Audio 1, row 2, playing"]
        );
        assert_eq!(h.project().loop_track(LoopId(0xbeef)), Some(audio));
        // A loop whose id a cell names already doesn't land there again.
        h.act(TimeAction::LoopCell(Some((audio, 2)))).unwrap();
        ids[2] = 0xbeef;
        now[2] = pass(3840, 0);
        let said = h.loops_are_on(&now, &ids);
        // (Loops A plays loop 1, so a new Loop track.)
        assert_eq!(said, ["Loop 3 on Loops B, row 1, playing"]);
        assert_eq!(h.loop_cell(), Some((audio, 2)), "the arm stays");
        assert_eq!(h.control.project.check(), Ok(()));
    }

    /// Loops on audio tracks (phase 4b): an audio track's loop cell copied
    /// onto another audio track's empty cell is a new loop, its cell naming
    /// the id the app gave it; refused with no id, one named already, a
    /// loop not free or the same track, and its cell goes again when the
    /// copy stops before anything went in.
    #[test]
    fn with_loops_on_a_loop_cell_copied_to_another_audio_track_is_a_new_loop() {
        use mpfree_seq::loops::LoopNow;
        use mpfree_seq::{LoopId, LOOPS};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.act(TimeAction::AddAudioTrack).unwrap();
        h.act(TimeAction::AddAudioTrack).unwrap();
        let b = h.project().tracks.len() - 1;
        let a = b - 1;
        h.set_loops_on(true);
        h.act(TimeAction::LoopCell(Some((a, 0)))).unwrap();
        let mut now = [LoopNow::default(); LOOPS];
        let mut ids = [0u128; LOOPS];
        (now[0], ids[0]) = (
            LoopNow {
                pass: true,
                ticks: 7680,
                origin: 0,
            },
            0xa1,
        );
        assert_eq!(
            h.loops_are_on(&now, &ids),
            ["Loop 1 on Audio 1, row 1, playing"]
        );
        let copy = |to, into, id| TimeAction::LoopCopy {
            from: (a, 0),
            to,
            into,
            id,
        };
        assert!(h.act(copy((b, 2), 3, 0)).is_err(), "no id");
        assert!(h.act(copy((b, 2), 3, 0xa1)).is_err(), "named already");
        assert!(h.act(copy((b, 2), 0, 0xb2)).is_err(), "loop 1 isn't free");
        assert!(h.act(copy((a, 2), 3, 0xb2)).is_err(), "the same track");
        assert!(h.act(copy((0, 2), 3, 0xb2)).is_err(), "a note track");
        let said = h.act(copy((b, 2), 3, 0xb2)).unwrap();
        assert!(said.contains("copies into Loop 4 on Audio 2 / 3"), "{said}");
        let cell = h.project().tracks[b].slots[2].as_ref().unwrap();
        assert_eq!(cell.loop_id(), Some(LoopId(0xb2)));
        assert_eq!(cell.length, 7680);
        assert!(!h.launched(b), "silent until launched");
        assert_eq!(h.project().loop_track(LoopId(0xb2)), Some(b));
        assert_eq!(h.project().loop_track(LoopId(0xa1)), Some(a));
        assert_eq!(h.control.project.check(), Ok(()));
        // The copy stopped before anything went in: its cell goes.
        let off = h.act(TimeAction::LoopCopyOff(3)).unwrap();
        assert!(off.contains("Loop 4's copy stopped"), "{off}");
        assert!(h.project().tracks[b].slots[2].is_none());
        assert_eq!(h.act(TimeAction::LoopCopyOff(3)).unwrap(), "");
    }

    /// A project saved before Loop tracks: its loops come in with it, each
    /// lands on a Loop track of its own in row 1, and all of them play, as
    /// they did. A project saved with Loop tracks keeps its cells.
    #[test]
    fn an_older_projects_loops_each_land_on_a_loop_track_of_their_own() {
        use mpfree_seq::loops::LoopNow;
        use mpfree_seq::LOOPS;
        let mut four = Project::new();
        for i in 0..4 {
            four.tracks.push(Track::new(&format!("Track {}", i + 1), i));
        }
        let old = mpfree_seq::save::to_text(&four);
        assert!(!old.contains("loops "));
        let read = mpfree_seq::save::from_text(&old).unwrap();
        let mut h = TimeHost::with_project(RATE, 120.0, read).unwrap();
        let mut now = [LoopNow::default(); LOOPS];
        for (n, l) in now.iter_mut().enumerate().take(3) {
            *l = LoopNow {
                pass: true,
                ticks: 3840 * (n as i64 + 1),
                origin: 0,
            };
        }
        assert_eq!(
            h.loops_are(&now),
            [
                "Loop 1 on Loops A, row 1, playing",
                "Loop 2 on Loops B, row 1, playing",
                "Loop 3 on Loops C, row 1, playing"
            ]
        );
        assert!(h.loop_plan().on.iter().all(|&on| on));
        // Saved now and opened again: the cells are there, nothing moves.
        let text = mpfree_seq::save::to_text(h.project());
        let mut again =
            TimeHost::with_project(RATE, 120.0, mpfree_seq::save::from_text(&text).unwrap())
                .unwrap();
        assert!(again.loops_are(&now).is_empty());
        assert_eq!(again.project(), h.project());
        // An opened project launches nothing: its loops wait for a launch.
        assert_eq!(&again.loop_plan().on[..4], &[false, false, false, true]);
    }

    /// TRACK MUTE's keys: track 2 muted halfway through bar 1 goes quiet
    /// from there, and unmuted in bar 3 it plays on in time.
    #[test]
    fn a_track_mute_key_silences_the_track_and_lets_it_back_in_time() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let beats = |note: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = (0..4)
                .map(|b| mpfree_seq::Note::new(b * 960, 100, note, 100))
                .collect();
            c
        };
        for (t, note) in [(0, 4), (1, 30)] {
            h.control.project.tracks[t].slots[0] = Some(beats(note));
            h.act(TimeAction::Launch { track: t, slot: 0 }).unwrap();
        }
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2 - 1000, &[]);
        assert_eq!(h.toggle_mute(1).unwrap(), "track mute: Track 2 muted");
        assert_eq!(
            (h.muted(1), h.muted(0), h.muted(TRACKS)),
            (Some(true), Some(false), None)
        );
        // Silent now: track 2. A solo on track 1 leaves every other out.
        assert_eq!(h.silenced(), 0b10);
        h.control.project.tracks[0].solo = true;
        assert_eq!(h.silenced(), (1u128 << TRACKS) - 2);
        h.control.project.tracks[0].solo = false;
        assert!(h.launched(1) && !h.launched(2));
        assert!(h.state().tracks[1].mute);
        run(&mut h, &mut pads, 2 * BAR - 1000, &[]);
        assert_eq!(h.toggle_mute(1).unwrap(), "track mute: Track 2 plays");
        run(&mut h, &mut pads, 3 * BAR - 1000, &[]);
        let at = |pad: usize| {
            pads.hits
                .iter()
                .filter(|&&(_, p, _)| p == pad)
                .map(|&(f, _, _)| (f + BAR / 8) / (BAR / 4))
                .collect::<Vec<u64>>()
        };
        // Track 1 on every beat; track 2 on beats 1 and 2, then bar 3.
        assert_eq!(at(4), (0..12).collect::<Vec<u64>>());
        assert_eq!(at(30), [0, 1, 8, 9, 10, 11]);
        assert!(h.toggle_mute(TRACKS).is_err());
        assert_eq!(h.dropped(), 0);
    }

    /// The mute queue (sequencer S4): taps that wait land together on the
    /// next bar line; a tap on a track waiting takes its change back; a
    /// solo silences the others from its line; STOP lands what waits.
    #[test]
    fn mutes_that_wait_land_together_on_the_launch_line() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let beats = |note: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = (0..4)
                .map(|b| mpfree_seq::Note::new(b * 960, 100, note, 100))
                .collect();
            c
        };
        for (t, note) in [(0, 4), (1, 30), (2, 31)] {
            h.control.project.tracks[t].slots[0] = Some(beats(note));
            h.act(TimeAction::Launch { track: t, slot: 0 }).unwrap();
        }
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2 - 1000, &[]);
        assert_eq!(
            h.mute_or_solo(1, true, false).unwrap(),
            "track mute: Track 2 muted on the next bar"
        );
        assert_eq!(
            h.mute_or_solo(0, true, false).unwrap(),
            "track mute: Track 1 muted on the next bar"
        );
        assert_eq!(
            h.mute_or_solo(0, true, false).unwrap(),
            "track mute: Track 1: its change taken back"
        );
        let l = h.mute_light(1).unwrap();
        assert!(l.waiting && !l.silent, "it plays until the line");
        // MIDI Learn reads a mute as it will land (sequencer S5).
        assert_eq!(h.mute_bits(), (1 << 1, 0));
        assert!(!h.mute_light(0).unwrap().waiting);
        let st = h.state();
        assert_eq!(st.mute_wait.as_ref().unwrap().want, vec![(1, true, false)]);
        assert_eq!(st.waiting_reading().as_deref(), Some("Track 2 at 2.1"));
        run(&mut h, &mut pads, BAR + BAR / 2 - 1000, &[]);
        assert_eq!(h.muted(1), Some(true), "landed on bar 2");
        assert!(h.mute_wait().is_none());
        assert_eq!(
            h.take_later_note().as_deref(),
            Some("track mute: at 2.1, Track 2 muted")
        );
        // A solo waits too: track 3 alone from bar 3.
        assert_eq!(
            h.mute_or_solo(2, true, true).unwrap(),
            "track mute: Track 3 solo on the next bar"
        );
        assert_eq!(h.mute_bits(), (1 << 1, 1 << 2));
        run(&mut h, &mut pads, 2 * BAR + BAR / 2 - 1000, &[]);
        assert!(h.mute_light(0).unwrap().silent && h.mute_light(2).unwrap().solo);
        // Its solo off waits, and STOP lands it.
        h.mute_or_solo(2, true, true).unwrap();
        assert!(h.mute_wait().is_some());
        h.command(Command::Stop);
        run(&mut h, &mut pads, 3 * BAR, &[]);
        assert!(h.mute_wait().is_none());
        assert!(!h.mute_light(2).unwrap().solo && !h.mute_light(0).unwrap().silent);
        let note = h.take_later_note().unwrap();
        assert!(
            note.starts_with("track mute: at 3.") && note.ends_with(", Track 3 solo off (STOP)"),
            "{note}"
        );
        let at = |pad: usize| {
            pads.hits
                .iter()
                .filter(|&&(_, p, _)| p == pad)
                .map(|&(f, _, _)| (f + BAR / 8) / (BAR / 4))
                .collect::<Vec<u64>>()
        };
        // Track 1 for two bars, then silent by the solo; track 2 for one
        // bar; track 3 throughout (to beat 2 of bar 3, where STOP was).
        assert_eq!(at(4), (0..8).collect::<Vec<u64>>());
        assert_eq!(at(30), [0, 1, 2, 3]);
        assert_eq!(at(31), (0..10).collect::<Vec<u64>>());
        // Stopped, a tap that would wait acts at once.
        assert_eq!(
            h.mute_or_solo(1, true, false).unwrap(),
            "track mute: Track 2 plays"
        );
        assert_eq!(h.dropped(), 0);
    }

    /// A track's pads in the mute queue (SHIFT + TRACK MUTE in Perform;
    /// step 10, a pad mute is the track's alone): a tap waits with the
    /// tracks' taps, a second takes it back, and a solo waits too. Each
    /// comes due in the period before its line, so the view compiled then
    /// plays the period that holds it; another track on the same pads
    /// plays on. STOP lands what waits, and stopped a tap acts at once.
    #[test]
    fn a_tracks_pad_mutes_wait_for_the_line_and_leave_its_hits_out_alone() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        // Tracks 1 and 2 hit A5 and B5; track 3 hits B15.
        for (t, hits) in [(0, [4u8, 20]), (1, [4, 20]), (2, [30, 30])] {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![
                mpfree_seq::Note::new(0, 100, hits[0], 100),
                mpfree_seq::Note::new(960, 100, hits[1], 100),
            ];
            h.control.project.tracks[t].slots[0] = Some(c);
            h.act(TimeAction::Launch { track: t, slot: 0 }).unwrap();
        }
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        let waits = |s: &str| Ok(PadTap::Waits(s.to_string()));
        assert_eq!(
            h.pad_mute_or_solo(0, 20, true, false),
            waits("pad mute: Track 1 B5 muted on the next bar")
        );
        assert_eq!(
            h.pad_mute_or_solo(0, 4, true, false),
            waits("pad mute: Track 1 A5 muted on the next bar")
        );
        assert_eq!(
            h.pad_mute_or_solo(0, 4, true, false),
            waits("pad mute: Track 1 A5: its change taken back")
        );
        assert!(h.pad_waiting(0, 20) && !h.pad_waiting(0, 4) && !h.pad_waiting(1, 20));
        // A track joins the same line.
        h.mute_or_solo(2, true, false).unwrap();
        assert_eq!(
            h.state().waiting_reading().as_deref(),
            Some("Track 3, Track 1 B5 at 2.1")
        );
        let (mut due, mut notes) = (None, Vec::new());
        while pads.frame < 2 * BAR + BAR / 2 {
            let to = pads.frame + u64::from(PERIOD);
            run(&mut h, &mut pads, to, &[]);
            if due.is_none() && h.pad_mutes(0) == Some((1 << 20, 0)) {
                due = Some(pads.frame);
            }
            notes.extend(h.take_later_note());
        }
        // The view has it from `at`: never after bar 2, and at most a
        // period and a tick before it (the period's ticks round up).
        let at = due.unwrap();
        let tick = BAR.div_ceil(3840);
        assert!(at <= BAR && BAR - at <= u64::from(PERIOD) + tick, "{at}");
        assert_eq!(h.pad_mutes(1), Some((0, 0)), "track 2's B5 untouched");
        assert!(!h.pad_waiting(0, 20) && h.mute_wait().is_none());
        assert_eq!(h.muted(2), Some(true), "the track landed on the line");
        assert!(
            notes[0].starts_with("pad mute: at 2.1, Track 1 B5 muted"),
            "{notes:?}"
        );
        let at = |track: u16, pad: usize| {
            pads.hits
                .iter()
                .zip(&pads.hit_tracks)
                .filter(|&(&(_, p, _), &t)| p == pad && t == track)
                .map(|(&(f, _, _), _)| (f + BAR / 8) / (BAR / 4))
                .collect::<Vec<u64>>()
        };
        assert_eq!(at(0, 4), [0, 4, 8], "track 1's A5 plays on");
        assert_eq!(at(0, 20), [1], "track 1's B5 left out from bar 2");
        assert_eq!(at(1, 20), [1, 5, 9], "track 2's B5 plays on");
        assert_eq!(at(2, 30), [0, 1], "track 3 muted from bar 2");
        // A solo waits, and STOP lands it.
        assert_eq!(
            h.pad_mute_or_solo(1, 4, true, true),
            waits("pad mute: Track 2 A5 solo on the next bar")
        );
        h.command(Command::Stop);
        let to = pads.frame + 4 * u64::from(PERIOD);
        run(&mut h, &mut pads, to, &[]);
        assert_eq!(h.pad_mutes(1), Some((0, 1 << 4)));
        let note = h.take_later_note().unwrap();
        assert!(note.ends_with("Track 2 A5 solo (STOP)"), "{note}");
        // Stopped, at once.
        assert_eq!(
            h.pad_mute_or_solo(0, 20, true, false),
            Ok(PadTap::Now("pad mute: Track 1 B5 plays".to_string()))
        );
        assert!(h.mute_wait().is_none());
        assert_eq!(h.pad_mutes(0), Some((0, 0)));
    }

    /// Song → scenes on a song in the exact set (plan D7): the rows,
    /// launched from the first, give the same notes on the same frames as
    /// the song, at several period sizes. The drums' four-bar block carries
    /// on over both section edges (bars 3 and 7), so its copies are turned
    /// round to where it had got to.
    #[test]
    fn a_song_in_the_exact_set_plays_the_same_from_its_rows() {
        use mpfree_seq::arrange::Placement;
        use mpfree_seq::Note;
        for period in [7u32, 128, 1000] {
            let by = |h: &mut TimeHost, pads: &mut Fake, to: u64| {
                while pads.frame < to {
                    h.period(period, pads);
                    pads.frame += u64::from(period);
                }
            };
            let mut h = TimeHost::new(RATE, 120.0).unwrap();
            let mut four = Clip::new("four", 4 * 3840);
            four.notes = (0..16)
                .map(|k| Note::new(k * 960, 100, 40 + k as u8, 100))
                .collect();
            let mut eighths = Clip::new("eighths", 3840);
            eighths.notes = (0..8)
                .map(|k| Note::new(k * 480, 200, 70 + k as u8, 90))
                .collect();
            let p = &mut h.control.project;
            p.tracks[0].slots[2] = Some(four);
            p.tracks[1].slots[0] = Some(eighths);
            p.give_clip_ids();
            let s = &mut p.arrangement;
            s.place(Placement::of(&p.tracks, 0, 2, 0, 8 * 3840))
                .unwrap();
            s.place(Placement::of(&p.tracks, 1, 0, 2 * 3840, 4 * 3840))
                .unwrap();
            // The song, in song mode from bar 1.
            let mut heard = Fake::default();
            h.act(TimeAction::SongMode(true)).unwrap();
            h.command(Command::Play);
            by(&mut h, &mut heard, 8 * BAR + 1000);
            assert_eq!(heard.hits.len(), 16 * 2 + 8 * 4, "period {period}");
            // Into rows 4 to 6 (rows 1 and 3 hold the clips), exactly.
            let said = h.act(TimeAction::SongToScenes).unwrap();
            assert_eq!(
                said,
                "song → scenes: rows 4 to 6, exact. UNDO takes it back."
            );
            // The rows alone, from the first, in a fresh host.
            let mut r = TimeHost::new(RATE, 120.0).unwrap();
            r.control.project = h.control.project.clone();
            r.control.project.arrangement = Default::default();
            let mut rows = Fake::default();
            r.act(TimeAction::LaunchScene(3)).unwrap();
            r.command(Command::Play);
            by(&mut r, &mut rows, 8 * BAR + 1000);
            assert_eq!(rows.hits, heard.hits, "period {period}");
            assert_eq!(rows.releases, heard.releases, "period {period}");
            assert_eq!((h.dropped(), r.dropped()), (0, 0));
        }
    }

    /// Scenes → song carries the scenes' mutes from row to row and ends a
    /// one-shot's block with its pass: the song plays the same notes on the
    /// same frames as the rows launched from the first.
    #[test]
    fn scenes_to_song_plays_what_the_rows_played_mutes_and_one_shots_too() {
        use mpfree_seq::{Follow, FollowAction, Note};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let beats = |pad: u8| {
            let mut c = Clip::new("beats", 3840);
            c.notes = (0..4)
                .map(|k| Note::new(k * 960, 100, pad + k as u8, 100))
                .collect();
            c
        };
        let p = &mut h.control.project;
        for row in 0..3 {
            p.tracks[0].slots[row] = Some(beats(10 + 10 * row as u8));
            p.tracks[1].slots[row] = Some(beats(50 + 10 * row as u8));
        }
        // Row 1: a one-shot on the drums, the bass muted, for 2 bars.
        // Row 2 leaves the mutes (the bass stays muted); row 3 unmutes.
        p.tracks[0].slots[0].as_mut().unwrap().once = true;
        let next = |bars| Some(Follow::after(bars, FollowAction::Next));
        let s = p.scene_mut(0).unwrap();
        s.mutes = Some(0b10);
        s.follow = next(2);
        p.scene_mut(1).unwrap().follow = next(1);
        let s = p.scene_mut(2).unwrap();
        s.mutes = Some(0);
        s.follow = Some(Follow::after(1, FollowAction::Stop));
        p.give_clip_ids();
        let mut song = TimeHost::new(RATE, 120.0).unwrap();
        song.control.project = h.control.project.clone();
        // The rows, from the first.
        let mut live = Fake::default();
        h.act(TimeAction::LaunchScene(0)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut live, 4 * BAR + 1000, &[]);
        let pads: Vec<usize> = live.hits.iter().map(|h| h.1).collect();
        assert_eq!(
            pads,
            [10, 11, 12, 13, 20, 21, 22, 23, 30, 70, 31, 71, 32, 72, 33, 73]
        );
        // The song they make.
        let said = song.act(TimeAction::ScenesToSong).unwrap();
        assert_eq!(
            said,
            "scenes → song: 3 rows from bar 1, exact. UNDO takes it back."
        );
        song.act(TimeAction::SongMode(true)).unwrap();
        let mut played = Fake::default();
        song.command(Command::Play);
        run(&mut song, &mut played, 4 * BAR + 1000, &[]);
        assert_eq!(played.hits, live.hits);
        assert_eq!(played.releases, live.releases);
        assert_eq!((h.dropped(), song.dropped()), (0, 0));
    }

    /// The conversion report card (Arrange 4.3): a conversion whose result
    /// plays differently opens it, each difference by track and bar in song
    /// order; its Done closes it and keeps what the conversion did, UNDO
    /// closes it and takes the conversion back, and another conversion
    /// replaces it (an exact one leaves it closed).
    #[test]
    fn a_conversion_that_plays_differently_opens_its_report_card() {
        use mpfree_seq::arrange::Placement;
        use mpfree_seq::{ClipProgram, Direction, ListStep, Note, TrackProgram};
        use mpfree_time_ui::view::{Report, ReportRow};
        const BAR: i64 = 3840;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut four = Clip::new("four", 4 * BAR);
        four.notes = (0..4).map(|k| Note::new(k * BAR, 100, 10, 100)).collect();
        four.direction = Direction::Random;
        let mut bass = Clip::new("bass", BAR);
        bass.notes = vec![Note::new(0, 100, 40, 100)];
        let p = &mut h.control.project;
        p.tracks[0].slots[2] = Some(four);
        p.tracks[1].slots[0] = Some(bass);
        p.give_clip_ids();
        let s = &mut p.arrangement;
        s.place(Placement::of(&p.tracks, 0, 2, 0, 8 * BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 1, 0, 2 * BAR, 6 * BAR))
            .unwrap();
        assert_eq!(h.state().report, None);
        // Song → scenes: the drums' Random order rolls again in each copy.
        let before = h.control.project.clone();
        let said = h.act(TimeAction::SongToScenes).unwrap();
        assert!(said.contains("2 differences"), "{said}");
        let random = "plays in Random order; the copy rolls its own order";
        let row = |bar| ReportRow {
            track: Some(0),
            bar,
            what: random.into(),
        };
        let want = Report {
            title: "SONG → SCENES".into(),
            made: "rows 4 to 5".into(),
            lead: "Launch row 4 and the rows play the song, except:".into(),
            rows: vec![row(1), row(3)],
        };
        assert_eq!(h.state().report.as_deref(), Some(&want));
        // Done: the card goes, the rows stay.
        let after = h.control.project.clone();
        assert_eq!(h.act(TimeAction::CloseReport).unwrap(), "");
        assert_eq!(h.state().report, None);
        assert_eq!(h.control.project, after);
        // UNDO with the card open: the card goes with the rows.
        h.act(TimeAction::Undo).unwrap();
        h.act(TimeAction::SongToScenes).unwrap();
        assert!(h.state().report.is_some());
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.state().report, None);
        let mut was = before.clone();
        was.next_clip = h.control.project.next_clip;
        assert_eq!(h.control.project, was);
        // REDO brings the rows back, not the card.
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(h.state().report, None);
        h.act(TimeAction::Undo).unwrap();
        // List → song with a scene's program change: its card replaces
        // the last one; an exact one after it leaves it closed.
        h.act(TimeAction::SongToScenes).unwrap();
        let p = &mut h.control.project;
        p.arrangement = Default::default();
        p.scene_mut(0).unwrap().programs = vec![TrackProgram {
            track: 0,
            program: ClipProgram {
                program: 11,
                bank: None,
            },
        }];
        p.arrangement.list.steps =
            vec![ListStep { row: 2, times: 1 }, ListStep { row: 0, times: 1 }];
        let said = h.act(TimeAction::ListToSong).unwrap();
        assert!(said.contains("1 difference"), "{said}");
        let st = h.state();
        let r = st.report.as_deref().unwrap();
        assert_eq!(
            (r.title.as_str(), r.made.as_str(), r.lead.as_str()),
            (
                "LIST → SONG",
                "bars 1 to 5",
                "The song plays the list, except:"
            )
        );
        assert_eq!(
            r.rows,
            [ReportRow {
                track: None,
                bar: 5,
                what: "the scene's program changes aren't in the song".into(),
            }]
        );
        h.control.project.arrangement.list.steps = vec![ListStep { row: 2, times: 1 }];
        let said = h.act(TimeAction::ListToSong).unwrap();
        assert!(said.ends_with(EXACT), "{said}");
        assert_eq!(h.state().report, None);
    }

    /// List → song plays the list offline through Rec song (Arrange 4.1,
    /// plan D7), so what only playing shows comes out right: a clip's
    /// follow action (track 2 moves to its next clip after a bar), a
    /// legato clip taking over its track's place (track 3, two bars into
    /// a three-bar clip), a one-shot (track 4), a scene's mute carried
    /// into the next (row 2 mutes track 1, and row 3 leaves it), a silent
    /// step (empty row 6) and a follow with a chance (track 5, the same
    /// rolls each time). The song gives the same notes on the same frames
    /// as the list at several period sizes, and the project converts the
    /// same way twice.
    #[test]
    fn list_to_song_plays_what_the_list_played_at_every_period_size() {
        use mpfree_seq::{Follow, FollowAction, ListStep, Note};
        let clip = |bars: i64, pad: u8| {
            let mut c = Clip::new("beats", bars * 3840);
            c.notes = (0..4 * bars)
                .map(|k| Note::new(k * 960, 100, pad + k as u8, 100))
                .collect();
            c
        };
        let mut p = TimeHost::new(RATE, 120.0).unwrap().project().clone();
        let t = &mut p.tracks;
        t[0].slots[0] = Some(clip(1, 10));
        t[0].slots[2] = Some(clip(1, 20));
        let mut a = clip(1, 30);
        a.follow = Some(Follow::after(1, FollowAction::Next));
        t[1].slots[0] = Some(a);
        t[1].slots[1] = Some(clip(1, 40));
        t[1].slots[2] = Some(clip(1, 50));
        t[2].slots[0] = Some(clip(3, 60));
        let mut legato = clip(4, 80);
        legato.legato = true;
        t[2].slots[1] = Some(legato);
        let mut once = clip(1, 100);
        once.length = 1920;
        once.notes.truncate(2);
        once.once = true;
        t[3].slots[0] = Some(once);
        let mut maybe = clip(1, 110);
        maybe.follow = Some(Follow {
            bars: 1,
            action: FollowAction::Next,
            other: FollowAction::Stay,
            chance: 50,
        });
        t[4].slots[0] = Some(maybe);
        t[4].slots[1] = Some(clip(1, 120));
        let s = p.scene_mut(0).unwrap();
        s.follow = Some(Follow::after(2, FollowAction::Next));
        let s = p.scene_mut(1).unwrap();
        s.mutes = Some(0b1);
        s.follow = Some(Follow::after(2, FollowAction::Next));
        p.arrangement.list.steps = [(0, 1), (1, 2), (5, 1), (2, 1)]
            .into_iter()
            .map(|(row, times)| ListStep { row, times })
            .collect();
        p.give_clip_ids();
        // 2 + 2 × 2 + 1 + 1 bars, and the last notes' ends.
        let to = 8 * BAR + 1000;
        let mut songs = Vec::new();
        for period in [7u32, 128, 1000] {
            let by = |h: &mut TimeHost, pads: &mut Fake, to: u64| {
                while pads.frame < to {
                    h.period(period, pads);
                    pads.frame += u64::from(period);
                }
            };
            let mut live = TimeHost::with_project(RATE, 120.0, p.clone()).unwrap();
            let mut heard = Fake::default();
            live.act(TimeAction::ListPlay(0)).unwrap();
            live.command(Command::Play);
            by(&mut live, &mut heard, to);
            let mut song = TimeHost::with_project(RATE, 120.0, p.clone()).unwrap();
            let said = song.act(TimeAction::ListToSong).unwrap();
            assert_eq!(
                said,
                "list → song: 4 steps from bar 1, exact. UNDO takes it back."
            );
            song.act(TimeAction::SongMode(true)).unwrap();
            let mut played = Fake::default();
            song.command(Command::Play);
            by(&mut song, &mut played, to);
            assert_eq!(played.hits, heard.hits, "period {period}");
            assert_eq!(played.releases, heard.releases, "period {period}");
            assert_eq!((live.dropped(), song.dropped()), (0, 0));
            songs.push(song.project().arrangement.clone());
            // What only playing shows: track 2's follow, track 3's legato
            // from two bars in (its third bar before its first), track 4's
            // one-shot once, and track 1 muted in row 3 (row 3's clip on
            // track 2 heard).
            let first = |pad: usize| heard.hits.iter().find(|h| h.1 == pad).map(|h| h.0);
            assert_eq!(first(40), Some(BAR));
            assert_eq!(first(88), Some(2 * BAR));
            assert_eq!(heard.hits.iter().filter(|h| h.1 == 100).count(), 1);
            assert_eq!((first(20), first(50)), (None, Some(7 * BAR)));
        }
        assert!(songs.windows(2).all(|w| w[0] == w[1]), "the same twice");
        let names: Vec<&str> = songs[0].markers.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Scene 1", "Scene 2", "Scene 6", "Scene 3"]);
    }

    /// Arrange 1.2 and 5.6: a clip and a scene in row 40, in bank C,
    /// save, open, launch and play; the scene's follow action moves within
    /// its bank (rows 32 to 47), never to bank A's; the grid shows all 64
    /// rows; a row dragged within bank A leaves row 40 alone, and a row
    /// copied above it moves it down one.
    #[test]
    fn a_clip_and_a_scene_in_row_40_save_open_launch_and_stay_put() {
        use mpfree_seq::{Follow, FollowAction, Note, ROWS};
        let beats = |pad: u8| {
            let mut c = Clip::new("beats", 3840);
            c.notes = (0..4)
                .map(|k| Note::new(k * 960, 100, pad + k as u8, 100))
                .collect();
            c
        };
        let mut p = TimeHost::new(RATE, 120.0).unwrap().project().clone();
        p.tracks[0].slots[0] = Some(beats(10));
        p.tracks[0].slots[40] = Some(beats(40));
        p.tracks[1].slots[41] = Some(beats(60));
        let next = Some(Follow::after(1, FollowAction::Next));
        let s = p.scene_mut(40).unwrap();
        s.name = "Far".into();
        s.follow = next;
        p.scene_mut(41).unwrap().follow = next;
        let read = mpfree_seq::save::from_text(&mpfree_seq::save::to_text(&p)).unwrap();
        assert_eq!(read, p);
        let mut h = TimeHost::with_project(RATE, 120.0, read).unwrap();
        assert_eq!(h.state().tracks[0].slots.len(), ROWS, "the grid's rows");
        let mut pads = Fake::default();
        h.act(TimeAction::LaunchScene(40)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, 3 * BAR - 1000, &[]);
        let hits: Vec<usize> = pads.hits.iter().map(|h| h.1).collect();
        assert_eq!(hits, [40, 41, 42, 43, 60, 61, 62, 63, 40, 41, 42, 43]);
        h.command(Command::Stop);
        // A row dragged and one copied on the grid: row 40 stays.
        h.act(TimeAction::DropRow {
            from: 0,
            to: 3,
            copy: false,
        })
        .unwrap();
        h.act(TimeAction::DropRow {
            from: 2,
            to: 0,
            copy: true,
        })
        .unwrap();
        let p = h.project();
        assert_eq!(p.clip(0, 41).map(|c| c.notes[0].note), Some(40));
        assert_eq!(p.scene(41).map(|s| s.name.as_str()), Some("Far"));
        assert!(h.act(TimeAction::LaunchScene(ROWS)).is_err());
    }

    /// Scene banks (the Arrange refactor's 5.6): rows 17 to 64 work as
    /// bank A's. Capture under a scene with bank A full lands in row 17;
    /// a row copied at row 30 moves the rows under it down, as far as row
    /// 64; the scene card opens on row 41; the morph takes rows 41 and 64.
    #[test]
    fn rows_past_bank_a_capture_copy_open_a_card_and_morph_as_bank_a() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |note: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            c
        };
        for row in 0..SLOTS {
            h.control.project.tracks[0].slots[row] = Some(one_bar(row as u8));
        }
        h.control.project.tracks[1].slots[40] = Some(one_bar(90));
        h.control.project.give_clip_ids();
        h.act(TimeAction::LaunchScene(3)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR + BAR / 4, &[]);
        let said = h.act(TimeAction::CaptureScene).unwrap();
        assert!(said.contains("into row 17"), "{said}");
        assert_eq!(h.project().clip(0, 16).map(|c| c.notes[0].note), Some(3));
        h.act(TimeAction::DropRow {
            from: 16,
            to: 30,
            copy: true,
        })
        .unwrap();
        let p = h.project();
        assert_eq!(p.clip(0, 30).map(|c| c.notes[0].note), Some(3));
        assert_eq!(p.clip(1, 41).map(|c| c.notes[0].note), Some(90), "down one");
        assert!(p.row_empty(40));
        h.act(TimeAction::SceneCard(Some(41))).unwrap();
        assert_eq!(h.state().scene_open, Some(41));
        h.act(TimeAction::SceneCard(None)).unwrap();
        assert!(h.act(TimeAction::SceneCard(Some(ROWS))).is_err());
        h.act(TimeAction::MorphRows { a: 41, b: 63 }).unwrap();
        assert_eq!((h.state().morph.a, h.state().morph.b), (41, 63));
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.project().clip(1, 40).map(|c| c.notes[0].note), Some(90));
    }

    /// Capture scene (Arrange 5.3): the clips playing now, one from row 1
    /// and one launched from row 2 over it, go as copies into the first
    /// empty row under the scene playing (row 3 has only a name, so row 4),
    /// as one take; what plays plays on, and UNDO takes the row back. In
    /// song mode a track with no launch gives the song's block under the
    /// playhead; with nothing playing there is nothing to keep.
    #[test]
    fn capture_scene_keeps_what_plays_as_copies_in_the_first_empty_row_under_it() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |note: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            c
        };
        h.control.project.tracks[0].slots[0] = Some(one_bar(4));
        h.control.project.tracks[1].slots[0] = Some(one_bar(30));
        h.control.project.tracks[0].slots[1] = Some(one_bar(5));
        h.control.project.scene_mut(2).unwrap().name = "Kept".into();
        h.control.project.give_clip_ids();
        assert!(h.act(TimeAction::CaptureScene).is_err(), "nothing plays");
        h.act(TimeAction::LaunchScene(0)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR, &[]);
        h.act(TimeAction::Launch { track: 0, slot: 1 }).unwrap();
        run(&mut h, &mut pads, 2 * BAR + BAR / 4, &[]);
        let names = (
            h.project().tracks[0].name.clone(),
            h.project().tracks[1].name.clone(),
        );
        let ids = |p: &Project| -> Vec<mpfree_seq::ClipId> {
            p.cell_clips().map(|(_, _, c)| c.id).collect()
        };
        let was = ids(h.project());
        let launches = h.control.launches.clone();
        assert_eq!(
            h.act(TimeAction::CaptureScene).unwrap(),
            format!(
                "capture: {} and {} into row 4, as copies (UNDO takes it back)",
                names.0, names.1
            )
        );
        let p = h.project();
        assert_eq!(p.clip(0, 3).map(|c| c.notes[0].note), Some(5));
        assert_eq!(p.clip(1, 3).map(|c| c.notes[0].note), Some(30));
        for t in 0..2 {
            let id = p.clip(t, 3).unwrap().id;
            assert!(id.is_given() && !was.contains(&id), "a clip of its own");
        }
        assert_eq!(p.scene(2).map(|s| s.name.as_str()), Some("Kept"));
        assert_eq!(h.control.launches, launches, "what plays plays on");
        assert_eq!(h.state().scene_focus, 3, "the scene editor shows it");
        h.act(TimeAction::Undo).unwrap();
        assert!(h.project().row_empty(3));
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(h.project().clip(1, 3).map(|c| c.notes[0].note), Some(30));
        // The next goes under the scene playing again: row 5.
        h.act(TimeAction::CaptureScene).unwrap();
        assert_eq!(h.project().clip(0, 4).map(|c| c.notes[0].note), Some(5));
        // In song mode, a track with no launch gives the song's block.
        h.act(TimeAction::StopAll).unwrap();
        h.command(Command::Stop);
        run(&mut h, &mut pads, 3 * BAR, &[]);
        h.act(TimeAction::ScenesToSong).unwrap();
        h.act(TimeAction::SongMode(true)).unwrap();
        h.command(Command::PlayStart);
        run(&mut h, &mut pads, 3 * BAR + BAR / 2, &[]);
        assert!(h.control.launches.is_empty(), "the song plays");
        let row = (0..SLOTS).find(|&r| h.project().row_empty(r)).unwrap();
        h.act(TimeAction::CaptureScene).unwrap();
        let p = h.project();
        assert_eq!(p.clip(0, row).map(|c| c.notes[0].note), Some(4));
        assert_eq!(p.clip(1, row).map(|c| c.notes[0].note), Some(30));
    }

    /// The scene card's Insert row and Delete row (5.2): an empty row goes
    /// in under a row and the rows under it move down one, or a row goes
    /// and the rows under it move up; their scenes, the song list's steps,
    /// what plays and the scene playing go with them, and UNDO takes each
    /// back. A clip the song plays stays the song's.
    #[test]
    fn insert_row_and_delete_row_move_the_rows_under_them_and_undo_takes_them_back() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |note: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            c
        };
        for r in 0..3 {
            h.control.project.tracks[0].slots[r] = Some(one_bar(4 + r as u8));
            h.control.project.tracks[1].slots[r] = Some(one_bar(30 + r as u8));
        }
        h.control.project.scene_mut(1).unwrap().name = "Verse".into();
        h.control.project.scene_mut(2).unwrap().name = "Drop".into();
        h.control.project.arrangement.list.steps = (0..3)
            .map(|row| mpfree_seq::ListStep { row, times: 1 })
            .collect();
        h.control.project.give_clip_ids();
        h.act(TimeAction::LaunchScene(2)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR, &[]);
        assert_eq!(h.state().scene_playing, Some(2));
        let note =
            |h: &TimeHost, t: usize, r: usize| h.project().clip(t, r).map(|c| c.notes[0].note);
        let steps = |h: &TimeHost| -> Vec<usize> {
            h.project()
                .arrangement
                .list
                .steps
                .iter()
                .map(|s| s.row)
                .collect()
        };
        let played =
            |h: &TimeHost| -> Vec<usize> { h.control.launches.iter().map(|l| l.slot).collect() };
        // Insert row under row 1: Verse and Drop down one, row 2 empty.
        assert_eq!(
            h.act(TimeAction::InsertRow(0)).unwrap(),
            "clips: an empty row 2 under row 1, the rows under it down one (UNDO takes it back)"
        );
        assert!(h.project().row_empty(1));
        assert_eq!((note(&h, 0, 2), note(&h, 1, 3)), (Some(5), Some(32)));
        assert_eq!(h.project().scene(3).map(|s| s.name.as_str()), Some("Drop"));
        assert_eq!(steps(&h), [0, 2, 3]);
        assert_eq!(played(&h), [3, 3], "Drop plays on, in its new row");
        assert_eq!(h.state().scene_playing, Some(3));
        h.act(TimeAction::Undo).unwrap();
        assert_eq!((note(&h, 0, 1), note(&h, 0, 2)), (Some(5), Some(6)));
        assert_eq!(steps(&h), [0, 1, 2]);
        assert_eq!(h.state().scene_playing, Some(2));
        // Delete row 2, Verse: Drop up one, its step too, Verse's gone.
        assert_eq!(
            h.act(TimeAction::DeleteRow(1)).unwrap(),
            "clips: row 2 (Verse) deleted, the rows under it up one (UNDO brings it back)"
        );
        assert_eq!((note(&h, 0, 1), note(&h, 1, 1)), (Some(6), Some(32)));
        assert!(h.project().row_empty(2));
        assert_eq!(h.project().scene(1).map(|s| s.name.as_str()), Some("Drop"));
        assert_eq!(steps(&h), [0, 1]);
        assert_eq!(played(&h), [1, 1]);
        assert_eq!(h.state().scene_playing, Some(1));
        h.act(TimeAction::Undo).unwrap();
        assert_eq!((note(&h, 0, 1), note(&h, 0, 2)), (Some(5), Some(6)));
        assert_eq!(h.project().scene(1).map(|s| s.name.as_str()), Some("Verse"));
        assert_eq!(steps(&h), [0, 1, 2]);
        assert_eq!(played(&h), [2, 2]);
        assert_eq!(h.state().scene_playing, Some(2));
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(note(&h, 0, 1), Some(6));
        assert_eq!(played(&h), [1, 1]);
        h.act(TimeAction::Undo).unwrap();
        // The playing row deleted: its clips stop.
        h.act(TimeAction::DeleteRow(2)).unwrap();
        assert_eq!(played(&h), Vec::<usize>::new());
        assert_eq!(h.state().scene_playing, None);
        h.act(TimeAction::Undo).unwrap();
        // No room under the grid's last row, row 64.
        h.control.project.tracks[0].slots[ROWS - 1] = Some(one_bar(9));
        h.control.project.give_clip_ids();
        assert!(h
            .act(TimeAction::InsertRow(0))
            .unwrap_err()
            .contains("in use"));
        h.control.project.tracks[0].slots[ROWS - 1] = None;
        // A row the song plays: its clips stay the song's.
        h.act(TimeAction::StopAll).unwrap();
        h.command(Command::Stop);
        run(&mut h, &mut pads, 2 * BAR, &[]);
        h.act(TimeAction::ScenesToSong).unwrap();
        let id = h.project().clip(0, 1).unwrap().id;
        assert!(h
            .act(TimeAction::DeleteRow(1))
            .unwrap()
            .contains("the song keeps playing its clips"));
        assert!(h.project().arrangement.plays(id));
        assert!(h.project().arrangement.clips.get(id).is_some());
    }

    /// Two one-bar rows become a song (Scenes → song), which plays in song
    /// mode; a clip launched on track 1 takes it over, and the song plays
    /// on there after it stops. The song goes back into rows (Song →
    /// scenes), and a placement is made unique and deleted.
    #[test]
    fn rows_become_a_song_that_plays_and_goes_back_into_rows() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |note: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            c
        };
        for (t, base) in [(0, 4), (1, 30)] {
            h.control.project.tracks[t].slots[0] = Some(one_bar(base));
            h.control.project.tracks[t].slots[1] = Some(one_bar(base + 1));
        }
        h.control.project.tracks[0].slots[5] = Some(one_bar(9));
        h.control.project.give_clip_ids();
        h.control.project.scene_mut(1).unwrap().name = "Drop".into();
        // Row 6 has a clip too: three rows, bars 1, 2 and 3.
        assert_eq!(
            h.act(TimeAction::ScenesToSong).unwrap(),
            "scenes → song: 3 rows from bar 1, exact. UNDO takes it back."
        );
        let st = h.state();
        assert_eq!(st.placements.len(), 5);
        let names: Vec<&str> = st.markers.iter().map(|m| m.1.as_str()).collect();
        assert_eq!(names, ["Scene 1", "Drop", "Scene 6"]);
        assert!(!st.song_on);
        h.act(TimeAction::SongMode(true)).unwrap();
        assert!(h.state().song_on);
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        // Track 1 plays row 1's clip in place of the song in bar 2, to its
        // stop on bar 3, where the song plays on (row 6).
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        run(&mut h, &mut pads, BAR + BAR / 2, &[]);
        h.act(TimeAction::StopClip(0)).unwrap();
        run(&mut h, &mut pads, 4 * BAR - 1000, &[]);
        let hits: Vec<(u64, usize)> = pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        let want = [(0, 4), (0, 30), (BAR, 4), (BAR, 31), (2 * BAR, 9)];
        assert_eq!(hits.len(), want.len(), "{hits:?}");
        for ((f, p), (at, pad)) in hits.iter().zip(want) {
            assert!(f.abs_diff(at) <= 23 && *p == pad, "{hits:?}");
        }
        // Back into rows: the first run of three empty rows is 3 to 5.
        let before = h.control.project.clone();
        let takes = h.state().done.len();
        assert_eq!(
            h.act(TimeAction::SongToScenes).unwrap(),
            "song → scenes: rows 3 to 5, exact. UNDO takes it back."
        );
        let p = &h.control.project;
        // Copies of the clips the song played, each with an id of its own.
        let same = |a: (usize, usize), b: (usize, usize)| {
            let (a, b) = (p.clip(a.0, a.1).unwrap(), p.clip(b.0, b.1).unwrap());
            a.copied() == b.copied() && a.id != b.id
        };
        assert!(same((0, 2), (0, 0)) && same((1, 3), (1, 1)) && same((0, 4), (0, 5)));
        assert_eq!(p.scene(3).unwrap().name, "Drop");
        // One edit: UNDO gives the project back exactly, REDO the rows.
        let after = p.clone();
        let st = h.state();
        assert_eq!(st.done.len(), takes + 1);
        assert_eq!(st.done.last().unwrap().rows, Some((2, 4)));
        let said = h.act(TimeAction::Undo).unwrap();
        assert!(said.ends_with(" · rows 3 to 5"), "{said}");
        assert!(said.starts_with("undo: song → scenes "), "{said}");
        // As before, but for the next clip id: UNDO never winds it back.
        assert!(h.control.project.next_clip > before.next_clip);
        let mut want = before.clone();
        want.next_clip = h.control.project.next_clip;
        assert_eq!(h.control.project, want);
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(h.control.project, after);
        // Refused (too few empty rows left in a run): nothing changes, and
        // there is nothing new to undo.
        let mut full = h.control.project.clone();
        for row in 6..ROWS {
            full.scene_mut(row).unwrap().name = format!("Taken {row}");
        }
        h.control.project = full.clone();
        let takes = h.state().done.len();
        assert!(h.act(TimeAction::SongToScenes).is_err());
        assert_eq!(h.control.project, full);
        assert_eq!(h.state().done.len(), takes);
        h.control.project = after;
        // A placement made unique keeps its clip when the slot changes.
        h.act(TimeAction::SelectPlacement(Some(0))).unwrap();
        assert_eq!(h.state().selected, Some(0));
        h.act(TimeAction::MakeUnique(0)).unwrap();
        assert!(h.state().placements[0].unique);
        h.act(TimeAction::DeletePlacement(4)).unwrap();
        let st = h.state();
        assert_eq!((st.placements.len(), st.selected), (4, None));
        assert!(h.act(TimeAction::DeletePlacement(9)).is_err());
        assert_eq!(h.state().placements.len(), 4);
        h.act(TimeAction::SongView(true)).unwrap();
        assert!(h.state().song_view);
        assert_eq!(h.dropped(), 0);
        // The selection's kind: a region picked drops the placement, and
        // a placement (or empty space) drops the region.
        use mpfree_time_ui::view::Pick;
        let r = RegionId { track: 1, index: 0 };
        h.act(TimeAction::SelectPlacement(Some(0))).unwrap();
        h.act(TimeAction::Audio(AudioAction::Pick(Some(r))))
            .unwrap();
        let st = h.state();
        assert_eq!(
            (st.selected, st.region, st.pick()),
            (None, Some(r), Some(Pick::Region(r)))
        );
        h.act(TimeAction::SelectPlacement(Some(1))).unwrap();
        assert_eq!(h.state().pick(), Some(Pick::Placement(1)));
        h.act(TimeAction::Audio(AudioAction::Pick(Some(r))))
            .unwrap();
        h.act(TimeAction::SelectPlacement(None)).unwrap();
        assert_eq!(h.state().pick(), None);
        // The rest is the audio tracks' engine: not wired here, and it
        // says so rather than doing nothing quietly.
        let e = h.act(TimeAction::Audio(AudioAction::Split(r))).unwrap_err();
        assert!(e.contains("not wired yet"), "{e}");
    }

    /// Rec song with the song nearly full: it warns, then stops on every
    /// track at the line that wouldn't fit, says so, and keeps what it
    /// recorded.
    #[test]
    fn rec_song_stops_on_a_line_the_song_has_no_room_for() {
        use mpfree_seq::arrange::{Placement, MAX_PLACEMENTS};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |note: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            c
        };
        let p = &mut h.control.project;
        for (t, base) in [(0, 4), (1, 30)] {
            p.tracks[t].slots[0] = Some(one_bar(base));
            p.tracks[t].slots[1] = Some(one_bar(base + 1));
        }
        p.give_clip_ids();
        // Room for the pass (a block and a stretch on each track) and one
        // block more: row 2's two don't fit.
        for k in 0..MAX_PLACEMENTS - 5 {
            p.arrangement.placements.push(Placement::of(
                &p.tracks,
                2,
                0,
                (1000 + k as i64) * 3840,
                3840,
            ));
        }
        h.act(TimeAction::LaunchScene(0)).unwrap();
        h.act(TimeAction::RecSong(true)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.act(TimeAction::LaunchScene(1)).unwrap();
        run(&mut h, &mut pads, 2 * BAR, &[]);
        let note = h.take_later_note().unwrap();
        assert!(
            note.starts_with("rec song: the song is 99% full (4095 of 4096 blocks)"),
            "{note}"
        );
        assert!(
            note.contains(
                "rec song: the song is full (4096 blocks), so recording stopped on every \
                 track at bar 2; what it recorded is kept"
            ),
            "{note}"
        );
        assert!(
            note.ends_with("rec song: 2 placements and 0 morph points into the song"),
            "{note}"
        );
        let st = h.state();
        assert!(!st.song_rec, "Rec song is off");
        let mine: Vec<(usize, Tick, Tick)> = st
            .placements
            .iter()
            .filter(|p| p.track < 2)
            .map(|p| (p.track, p.start, p.start + p.length))
            .collect();
        assert_eq!(mine, [(0, 0, 3840), (1, 0, 3840)]);
        assert_eq!(st.placements.len(), MAX_PLACEMENTS - 3);
        // Row 2 plays on: only the recording stopped.
        let row2 = |h: &&(u64, usize, u8)| (h.1 == 5 || h.1 == 31) && h.0 < 2 * BAR;
        assert_eq!(pads.hits.iter().filter(row2).count(), 2);
    }

    /// Rec song on, row 1 then row 2 launched on the grid, STOP: the song
    /// holds what was played, and plays it back the same in song mode.
    #[test]
    fn rec_song_records_the_grid_into_the_song() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |note: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            c
        };
        for (t, base) in [(0, 4), (1, 30)] {
            h.control.project.tracks[t].slots[0] = Some(one_bar(base));
            h.control.project.tracks[t].slots[1] = Some(one_bar(base + 1));
        }
        h.control.project.give_clip_ids();
        h.act(TimeAction::LaunchScene(0)).unwrap();
        h.act(TimeAction::RecSong(true)).unwrap();
        assert!(h.state().song_rec);
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.act(TimeAction::LaunchScene(1)).unwrap();
        run(&mut h, &mut pads, 3 * BAR - 1000, &[]);
        h.command(Command::Stop);
        run(&mut h, &mut pads, 3 * BAR, &[]);
        let note = h.take_later_note().unwrap();
        assert_eq!(
            note,
            "rec song: 4 placements and 0 morph points into the song"
        );
        let st = h.state();
        let placed: Vec<(usize, Tick, Tick)> = st
            .placements
            .iter()
            .map(|p| (p.track, p.start, p.start + p.length))
            .collect();
        // Bar 1 row 1, bars 2 and 3 row 2, to where it stopped.
        let end = st.placements[1].start + st.placements[1].length;
        assert!((2 * 3840 + 3000..3 * 3840).contains(&end), "{placed:?}");
        assert_eq!(
            placed,
            [(0, 0, 3840), (0, 3840, end), (1, 0, 3840), (1, 3840, end)]
        );
        let heard = std::mem::take(&mut pads.hits);
        // Back to the top with nothing launched, in song mode.
        h.act(TimeAction::RecSong(false)).unwrap();
        h.act(TimeAction::StopAll).unwrap();
        h.act(TimeAction::SongMode(true)).unwrap();
        h.command(Command::Locate(0));
        h.command(Command::Play);
        let from = pads.frame;
        run(&mut h, &mut pads, from + 3 * BAR - 1000, &[]);
        let again: Vec<(u64, usize)> = pads.hits.iter().map(|&(f, p, _)| (f - from, p)).collect();
        let heard: Vec<(u64, usize)> = heard.iter().map(|&(f, p, _)| (f, p)).collect();
        assert_eq!(again.len(), heard.len(), "{again:?} {heard:?}");
        for (a, b) in again.iter().zip(&heard) {
            assert!(
                a.0.abs_diff(b.0) <= 128 && a.1 == b.1,
                "{again:?} {heard:?}"
            );
        }
        assert_eq!(h.dropped(), 0);
    }

    /// "Back to song" (sequencer S4): every clip launched over the song
    /// stops on the launch line and the song plays its tracks from there;
    /// stopped, it is at once; with song mode off it turns it on.
    #[test]
    fn back_to_song_stops_the_clips_on_the_line_and_the_song_plays_on() {
        use mpfree_seq::arrange::Placement;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |note: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            c
        };
        h.control.project.tracks[0].slots[0] = Some(one_bar(4));
        h.control.project.tracks[0].slots[1] = Some(one_bar(9));
        let p = &mut h.control.project;
        p.give_clip_ids();
        p.arrangement
            .placements
            .push(Placement::of(&p.tracks, 0, 0, 0, 8 * 3840));
        h.act(TimeAction::SongMode(true)).unwrap();
        assert!(!h.state().over_song(), "the song alone");
        h.act(TimeAction::Launch { track: 0, slot: 1 }).unwrap();
        assert!(h.state().over_song(), "a clip over the song");
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        let live = pads.hits.last().unwrap().1;
        assert_eq!(pads.hits.len(), 1);
        let said = h.act(TimeAction::BackToSong).unwrap();
        assert_eq!(
            said,
            "back to song: every track plays the song from the next launch line"
        );
        let st = h.state();
        assert!(st.song_on);
        assert!(st.over_song(), "the clip plays to the line");
        run(&mut h, &mut pads, 2 * BAR - 1000, &[]);
        assert_eq!(pads.hits.len(), 2, "{:?}", pads.hits);
        let (at, song, _) = pads.hits[1];
        assert!(at.abs_diff(BAR) <= 128, "on the line: {at}");
        assert_ne!(song, live, "the song's clip, not the one launched");
        assert!(!h.state().over_song(), "nothing launched now");
        // Stopped, with song mode off: at once, and song mode is on.
        h.command(Command::Stop);
        run(&mut h, &mut pads, 2 * BAR, &[]);
        h.act(TimeAction::SongMode(false)).unwrap();
        h.act(TimeAction::Launch { track: 0, slot: 1 }).unwrap();
        assert_eq!(h.act(TimeAction::BackToSong).unwrap(), "back to song");
        let st = h.state();
        assert!(st.song_on && st.launches.is_empty() && st.cues.is_empty());
        assert_eq!(h.dropped(), 0);
    }

    /// The scene editor (sequencer S4, part 7): a scene's settings are
    /// takes, a row turned again joining its take; the editor follows a
    /// scene launched; the scene's follow action plays.
    #[test]
    fn the_scene_editor_sets_a_scene_and_undo_takes_it_back() {
        use mpfree_seq::{Follow, FollowAction, SceneSettings};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |name: &str, note: u8| {
            let mut c = Clip::new(name, 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            c
        };
        let t = &mut h.control.project.tracks[0];
        t.slots[0] = Some(one_bar("intro", 4));
        t.slots[1] = Some(one_bar("verse", 9));
        // Nothing playing: the editor shows scene 1, a plain row.
        let st = h.state();
        assert_eq!((st.scene_focus, &st.scene), (0, &SceneSettings::default()));
        let follow = |chance| SceneSettings {
            follow: Some(Follow {
                chance,
                ..Follow::after(1, FollowAction::Next)
            }),
            ..SceneSettings::default()
        };
        let set = |h: &mut TimeHost, card_row, settings| {
            h.act(TimeAction::SceneSettings {
                row: 0,
                card_row,
                settings: Box::new(settings),
            })
        };
        assert_eq!(
            set(&mut h, 1, follow(100)).unwrap(),
            "scene 1 · Follow Next (UNDO takes it back)"
        );
        let takes = h.state().done.len();
        set(&mut h, 3, follow(90)).unwrap();
        set(&mut h, 3, follow(80)).unwrap();
        assert_eq!(
            h.state().done.len(),
            takes + 1,
            "Chance turned twice: one take"
        );
        assert_eq!(
            h.state().done.last().unwrap().short(),
            format!("scene {}", takes + 1)
        );
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.state().scene, follow(100), "UNDO takes Chance back");
        // A name.
        let named = SceneSettings {
            name: "Intro".into(),
            ..follow(100)
        };
        assert_eq!(
            set(&mut h, mpfree_time_ui::view::SCENE_NAME, named).unwrap(),
            "scene 1 · named Intro (UNDO takes it back)"
        );
        assert_eq!(h.state().scenes[0], "Intro");
        assert!(set(
            &mut h,
            mpfree_time_ui::view::SCENE_NAME,
            SceneSettings {
                name: "x".repeat(mpfree_seq::MAX_SCENE_NAME + 1),
                ..follow(100)
            }
        )
        .is_err());
        // The editor's ‹ ›, then a scene launched: the editor follows it.
        h.act(TimeAction::SceneFocus(1)).unwrap();
        let st = h.state();
        assert_eq!((st.scene_focus, &st.scene), (1, &SceneSettings::default()));
        h.act(TimeAction::LaunchScene(0)).unwrap();
        assert_eq!(h.state().scene_focus, 0);
        // Scene 1 plays a bar, then follows on to scene 2.
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR + BAR / 2, &[]);
        assert_eq!(h.state().scene_playing, Some(1));
        let played: Vec<(u64, usize)> = pads.hits.iter().map(|h| (h.0, h.1)).collect();
        assert_eq!(played.len(), 2, "{played:?}");
        assert!(played[1].0.abs_diff(BAR) <= 128, "{played:?}");
        assert_ne!(played[0].1, played[1].1, "scene 2's clip");
        assert_eq!(h.dropped(), 0);
    }

    /// A one-shot clip (sequencer S4, the clip card's Mode: Once): it plays
    /// one pass, its track's stop waiting from its last bar, and stops on
    /// the tick the pass ends, though that is no bar line.
    #[test]
    fn a_one_shot_clip_plays_one_pass_and_its_track_stops() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        // Three beats, a note on each.
        let mut c = Clip::new("fill", 2880);
        c.notes = [0, 960, 1920]
            .iter()
            .map(|&t| mpfree_seq::Note::new(t, 100, 7, 100))
            .collect();
        c.once = true;
        h.control.project.tracks[0].slots[0] = Some(c);
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 8, &[]);
        let st = h.state();
        assert_eq!(
            st.cues,
            vec![mpfree_play::Cue {
                track: 0,
                slot: None
            }],
            "its stop waits"
        );
        assert_eq!(st.cue_lines, vec![(0, mpfree_seq::LaunchGrid::At(2880))]);
        run(&mut h, &mut pads, 2 * BAR, &[]);
        let played: Vec<u64> = pads.hits.iter().map(|h| h.0).collect();
        assert_eq!(played.len(), 3, "one pass: {played:?}");
        for (k, at) in [0, BAR / 4, BAR / 2].iter().enumerate() {
            assert!(played[k].abs_diff(*at) <= 128, "{k}: {played:?}");
        }
        let st = h.state();
        assert!(st.launches.is_empty() && st.cues.is_empty());
        assert_eq!(h.dropped(), 0);
    }

    /// The song list (sequencer S4): steps added after the pick play
    /// their rows their times, in order, the next one landing on the bar
    /// its step ends; after the last everything stops. Edits are takes
    /// UNDO takes back; List → song lays the list on the timeline; a scene
    /// launched by hand ends the list.
    #[test]
    fn the_song_list_plays_its_steps_in_order_and_edits_undo() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |name: &str, note: u8| {
            let mut c = Clip::new(name, 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            c
        };
        let t = &mut h.control.project.tracks[0];
        t.slots[0] = Some(one_bar("intro", 4));
        t.slots[1] = Some(one_bar("verse", 9));
        t.slots[3] = Some(one_bar("drop", 6));
        h.control.project.give_clip_ids();
        // Two steps: row 1, then the next row with a clip, row 2.
        assert_eq!(
            h.act(TimeAction::ListAdd).unwrap(),
            "song list: step 1 plays row 1"
        );
        assert_eq!(
            h.act(TimeAction::ListAdd).unwrap(),
            "song list: step 2 plays row 2"
        );
        h.act(TimeAction::ListTimes { step: 0, delta: 1 }).unwrap();
        let st = h.state();
        let rows: Vec<(String, Option<usize>, u16, i64, i64)> = st
            .list
            .iter()
            .map(|r| (r.name.clone(), r.scene, r.times, r.bars, r.starts))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("Row 1".into(), Some(0), 2, 2, 1),
                ("Row 2".into(), Some(1), 1, 1, 3)
            ]
        );
        assert_eq!(st.list_pick, Some(1));
        // Step 2's scene: the next row with a clip skips the empty one.
        h.act(TimeAction::ListScene { step: 1, delta: 1 }).unwrap();
        assert_eq!(h.state().list[1].scene, Some(3));
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.state().list[1].scene, Some(1), "UNDO takes the edit back");
        // Played from the top while stopped: row 1 twice, row 2, then
        // every track stops on bar 4.
        h.act(TimeAction::ListPlay(0)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        assert_eq!(h.state().list_next, Some((Some(1), 2 * 3840)));
        // In its last bar the next step waits for its line.
        run(&mut h, &mut pads, BAR + BAR / 2, &[]);
        let st = h.state();
        assert_eq!(st.list[0].playing.map(|p| p.0), Some(2), "pass 2 of 2");
        assert!(st.list[1].waiting);
        assert_eq!(st.scene_cued, Some(1));
        run(&mut h, &mut pads, 5 * BAR, &[]);
        let played: Vec<(u64, usize)> = pads.hits.iter().map(|h| (h.0, h.1)).collect();
        assert_eq!(played.len(), 3, "{played:?}");
        for (k, at) in [0, BAR, 2 * BAR].iter().enumerate() {
            assert!(played[k].0.abs_diff(*at) <= 128, "{k}: {played:?}");
        }
        assert_eq!(played[0].1, played[1].1);
        assert_ne!(played[1].1, played[2].1, "row 2's clip");
        let st = h.state();
        assert!(st.launches.is_empty(), "stopped after the last step");
        assert!(st.list.iter().all(|r| r.playing.is_none()));
        // Remove, and UNDO puts it back.
        h.act(TimeAction::ListRemove(1)).unwrap();
        assert_eq!(h.state().list.len(), 1);
        assert_eq!(h.state().list_pick, Some(0));
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.state().list.len(), 2);
        // Moved by its grip (Arrange 4b): picked where it went, and UNDO
        // takes it back.
        let scenes = |h: &TimeHost| h.state().list.iter().map(|r| r.scene).collect::<Vec<_>>();
        assert_eq!(
            h.act(TimeAction::ListMove { step: 1, to: 0 }).unwrap(),
            "song list: step 2 is step 1 now (UNDO takes it back)"
        );
        assert_eq!(scenes(&h), [Some(1), Some(0)]);
        assert_eq!(h.state().list_pick, Some(0));
        assert_eq!(
            h.act(TimeAction::ListMove { step: 0, to: 1 }).unwrap(),
            "song list: step 1 is already there"
        );
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(scenes(&h), [Some(0), Some(1)]);
        // List → song: three bars of blocks and a marker each step.
        h.command(Command::Stop);
        run(&mut h, &mut pads, 6 * BAR, &[]);
        let said = h.act(TimeAction::ListToSong).unwrap();
        assert_eq!(
            said,
            "list → song: 2 steps from bar 1, exact. UNDO takes it back."
        );
        let song = &h.control.project.arrangement;
        assert_eq!(song.markers.len(), 2);
        assert_eq!(song.end(), 3 * 3840);
        // A scene by hand ends the list.
        h.act(TimeAction::ListPlay(0)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, 7 * BAR, &[]);
        assert!(h.state().list.iter().any(|r| r.playing.is_some()));
        h.act(TimeAction::LaunchScene(3)).unwrap();
        let st = h.state();
        assert!(st.list_next.is_none() && st.list.iter().all(|r| !r.waiting));
        assert_eq!(h.dropped(), 0);
    }

    /// Arrange 4b.1: a linked song list shows the song's sections, each
    /// with its marker's name, its scene and its passes, and the bars
    /// before the first marker as "Song"; the one the song plays is lit
    /// with its pass, and picking one picks its scene. It needs no List →
    /// song; the loop switch works.
    #[test]
    fn a_linked_list_shows_the_songs_sections_and_their_passes() {
        use mpfree_seq::arrange::{scenes_to_song, FromScene, Placement};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let p = &mut h.control.project;
        for (row, name) in [(0, "intro"), (1, "verse")] {
            let mut c = Clip::new(name, 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, 4, 100)];
            p.tracks[0].slots[row] = Some(c);
        }
        p.give_clip_ids();
        p.scene_mut(1).unwrap().name = "Verse".into();
        // A bar of the song's own, then row 1 once and row 2 three times.
        let mut song = scenes_to_song(p, &[(0, 3840), (1, 3 * 3840)], 3840)
            .unwrap()
            .song;
        for (m, (row, passes)) in song.markers.iter_mut().zip([(0, 1), (1, 3)]) {
            m.scene = Some(FromScene::following(row, 3840));
            m.passes = passes;
        }
        song.place(Placement::of(&p.tracks, 0, 0, 0, 3840)).unwrap();
        song.list.linked = true;
        p.arrangement = song;
        p.check().unwrap();
        let rows = |h: &TimeHost| -> Vec<(String, Option<usize>, u16, i64, i64)> {
            h.state()
                .list
                .iter()
                .map(|r| (r.name.clone(), r.scene, r.times, r.bars, r.starts))
                .collect()
        };
        assert_eq!(
            rows(&h),
            vec![
                ("Song".into(), None, 1, 1, 1),
                ("Scene 1".into(), Some(0), 1, 1, 2),
                ("Verse".into(), Some(1), 3, 3, 3),
            ]
        );
        // Halfway through bar 4: the verse's second pass.
        h.act(TimeAction::SongMode(true)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, 3 * BAR + BAR / 2, &[]);
        let st = h.state();
        let lit: Vec<Option<u16>> = st.list.iter().map(|r| r.playing.map(|p| p.0)).collect();
        assert_eq!(lit, vec![None, None, Some(2)]);
        h.act(TimeAction::ListPick(Some(2))).unwrap();
        let st = h.state();
        assert_eq!((st.list_pick, st.scene_focus), (Some(2), 1));
        assert_eq!(
            h.act(TimeAction::ListPick(Some(3))).unwrap(),
            "song list: nothing picked"
        );
        // No List → song; the loop switch works (Play from here: 4b.4's
        // tests).
        let was = h.control.project.arrangement.clone();
        assert_eq!(
            h.act(TimeAction::ListToSong),
            Err("list → song: the list is the song's parts already".into())
        );
        assert_eq!(h.control.project.arrangement, was);
        h.act(TimeAction::ListLoop(true)).unwrap();
        assert!(h.state().list_looped);
        assert_eq!(h.dropped(), 0);
    }

    /// The grid Arrange 4b.2's tests edit a linked list on: Intro (row 1)
    /// a bar on track 1; Verse (row 2) two bars on track 1 and one on
    /// track 2; Chorus (row 3) a bar on track 1 and four on track 2. The
    /// song is empty and the list linked.
    fn linked_grid() -> TimeHost {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let p = &mut h.control.project;
        let clip = |name: &str, bars: Tick| {
            let mut c = Clip::new(name, bars * 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, 4, 100)];
            Some(c)
        };
        p.tracks[0].slots[0] = clip("intro", 1);
        p.tracks[0].slots[1] = clip("verse", 2);
        p.tracks[1].slots[1] = clip("verse bass", 1);
        p.tracks[0].slots[2] = clip("chorus", 1);
        p.tracks[1].slots[2] = clip("chorus bass", 4);
        p.give_clip_ids();
        for (row, name) in [(0, "Intro"), (1, "Verse"), (2, "Chorus")] {
            p.scene_mut(row).unwrap().name = name.into();
        }
        p.arrangement.list.linked = true;
        h
    }

    /// The song List → song lays for `steps` (row, times) on `h`'s grid,
    /// from an empty song.
    fn list_to_song(h: &TimeHost, steps: &[(usize, u16)]) -> mpfree_seq::Arrangement {
        let mut q = h.control.project.clone();
        q.arrangement = mpfree_seq::Arrangement::default();
        q.arrangement.list.steps = steps
            .iter()
            .map(|&(row, times)| mpfree_seq::ListStep { row, times })
            .collect();
        let mut r = TimeHost::with_project(RATE, 120.0, q).unwrap();
        r.act(TimeAction::ListToSong).unwrap();
        r.control.project.arrangement.clone()
    }

    /// `h`'s linked song is the one List → song lays for `steps`: the same
    /// blocks, and a marker where each step starts, named after its scene,
    /// carrying its scene and times; the list shows them.
    #[track_caller]
    fn laid_as(h: &TimeHost, steps: &[(usize, u16)]) {
        let song = &h.control.project.arrangement;
        let want = list_to_song(h, steps);
        assert_eq!(song.placements, want.placements, "{steps:?}");
        assert_eq!(song.markers.len(), steps.len(), "{steps:?}");
        for ((m, w), &(row, times)) in song.markers.iter().zip(&want.markers).zip(steps) {
            assert_eq!((m.at, &m.name), (w.at, &w.name), "{steps:?}");
            let from = m.scene.expect("made from its scene");
            assert_eq!((from.row, from.changed, m.passes), (row, false, times));
        }
        let rows: Vec<(Option<usize>, u16)> =
            h.state().list.iter().map(|r| (r.scene, r.times)).collect();
        let want: Vec<(Option<usize>, u16)> = steps.iter().map(|&(r, n)| (Some(r), n)).collect();
        assert_eq!(rows, want);
    }

    /// Arrange 4b.2: a linked list's edits move the song. Steps added, ×
    /// N, another scene, a step moved, a step taken out: after each the
    /// song is the one List → song lays for the new list (the same blocks,
    /// a marker where each step starts), each marker carrying its scene and
    /// times. Each edit is one UNDO, and REDO brings it back.
    #[test]
    fn a_linked_lists_edits_move_the_song_as_list_to_song_lays_it() {
        let mut h = linked_grid();
        let mut songs = vec![h.control.project.arrangement.clone()];
        // Each edit, the list after it (row, times) and what it says.
        type Edit<'a> = (TimeAction, &'a [(usize, u16)], &'a str);
        let edits: [Edit; 10] = [
            (
                TimeAction::ListAdd,
                &[(0, 1)],
                "song list: step 1 plays Intro, at bar 1; the song after it moved on (UNDO takes it out)",
            ),
            (
                TimeAction::ListAdd,
                &[(0, 1), (1, 1)],
                "song list: step 2 plays Verse, at bar 2; the song after it moved on (UNDO takes it out)",
            ),
            (
                TimeAction::ListTimes { step: 1, delta: 1 },
                &[(0, 1), (1, 2)],
                "song list: Verse ×2 (UNDO takes it back)",
            ),
            // After the Intro: room in the song for another Verse.
            (TimeAction::ListPick(Some(0)), &[(0, 1), (1, 2)], ""),
            (
                TimeAction::ListAdd,
                &[(0, 1), (1, 1), (1, 2)],
                "song list: step 2 plays Verse, at bar 2; the song after it moved on (UNDO takes it out)",
            ),
            // The Chorus is twice as long as the Verse it replaces.
            (
                TimeAction::ListScene { step: 1, delta: 1 },
                &[(0, 1), (2, 1), (1, 2)],
                "song list: step 2 plays Chorus now (UNDO takes it back)",
            ),
            (
                TimeAction::ListMove { step: 2, to: 0 },
                &[(1, 2), (0, 1), (2, 1)],
                "song list: Verse is step 1 now, and its bars moved with it (UNDO takes it back)",
            ),
            // Half as long: its second bar taken out.
            (
                TimeAction::ListScene { step: 0, delta: -1 },
                &[(0, 2), (0, 1), (2, 1)],
                "song list: step 1 plays Intro now (UNDO takes it back)",
            ),
            (
                TimeAction::ListRemove(1),
                &[(0, 2), (2, 1)],
                "song list: Intro taken out, and its bars with it; the song after it moved up (UNDO puts it back)",
            ),
            (
                TimeAction::ListMove { step: 0, to: 2 },
                &[(2, 1), (0, 2)],
                "song list: Intro is step 2 now, and its bars moved with it (UNDO takes it back)",
            ),
        ];
        for (a, steps, said) in edits {
            let pick = matches!(a, TimeAction::ListPick(_));
            let line = h.act(a.clone()).unwrap_or_else(|e| panic!("{a:?}: {e}"));
            if !pick {
                assert_eq!(line, said);
                songs.push(h.control.project.arrangement.clone());
            }
            laid_as(&h, steps);
        }
        // Each edit is one UNDO, back to the empty song; REDO brings the
        // first back.
        for want in songs.iter().rev().skip(1) {
            h.act(TimeAction::Undo).unwrap();
            assert_eq!(&h.control.project.arrangement, want);
        }
        h.act(TimeAction::Redo).unwrap();
        laid_as(&h, &[(0, 1)]);
        assert_eq!(h.dropped(), 0);
    }

    /// The audio tracks as a song time edit sees them, holding regions
    /// that every edit moves: the edits they were given, by song edit.
    #[derive(Default)]
    struct Regions(Vec<(u32, mpfree_audiotracks::Retime)>);

    impl SongAudio for Regions {
        fn busy(&self) -> Option<String> {
            None
        }
        fn refused(&self, _: &mpfree_audiotracks::Retime) -> Option<String> {
            None
        }
        fn moves(&self, _: &mpfree_audiotracks::Retime) -> bool {
            true
        }
        fn retime(&mut self, song: u32, r: mpfree_audiotracks::Retime) -> bool {
            self.0.push((song, r));
            true
        }
        fn step(&self, _: u32, _: bool) -> mpfree_audiotracks::SongStep {
            mpfree_audiotracks::SongStep::Next
        }
        fn song_undo(&mut self, _: u32, _: bool) {}
    }

    /// Arrange 4b.2: a linked list's edit that moves time in the song
    /// gives the audio tracks the same edit, so their regions move with
    /// the song; one that moves no time (another scene as long) gives them
    /// none.
    #[test]
    fn a_linked_lists_edits_move_the_audio_regions_with_the_song() {
        use mpfree_seq::TimeEdit;
        const B: Tick = 3840;
        let mut h = linked_grid();
        // And a Bridge (row 4): a bar, as long as the Intro.
        let mut c = Clip::new("bridge", B);
        c.notes = vec![mpfree_seq::Note::new(0, 100, 4, 100)];
        h.control.project.tracks[0].slots[3] = Some(c);
        h.control.project.give_clip_ids();
        h.control.project.scene_mut(3).unwrap().name = "Bridge".into();
        let mut audio = Regions::default();
        for a in [TimeAction::ListAdd, TimeAction::ListAdd] {
            h.act_with_audio(a, &mut audio).unwrap();
        }
        // Intro, Verse: the room for each went to the audio too.
        let given = |edits: &[TimeEdit]| -> Vec<mpfree_audiotracks::Retime> {
            edits.iter().filter_map(control::retime_of).collect()
        };
        let had = |audio: &Regions| audio.0.iter().map(|x| x.1).collect::<Vec<_>>();
        let mut want = vec![
            TimeEdit::Insert { at: 0, len: B },
            TimeEdit::Insert { at: B, len: 2 * B },
        ];
        assert_eq!(had(&audio), given(&want));
        h.act_with_audio(TimeAction::ListMove { step: 1, to: 0 }, &mut audio)
            .unwrap();
        want.push(TimeEdit::Move {
            from: B,
            to: 3 * B,
            at: 0,
        });
        h.act_with_audio(TimeAction::ListTimes { step: 0, delta: 1 }, &mut audio)
            .unwrap();
        want.push(TimeEdit::Insert {
            at: 2 * B,
            len: 2 * B,
        });
        h.act_with_audio(TimeAction::ListRemove(1), &mut audio)
            .unwrap();
        want.push(TimeEdit::Delete {
            from: 4 * B,
            to: 5 * B,
        });
        assert_eq!(had(&audio), given(&want));
        // The Verse to the Chorus, × 2 each: four bars to eight.
        h.act_with_audio(TimeAction::ListScene { step: 0, delta: 1 }, &mut audio)
            .unwrap();
        want.push(TimeEdit::Insert {
            at: 4 * B,
            len: 4 * B,
        });
        assert_eq!(had(&audio), given(&want));
        laid_as(&h, &[(2, 2)]);
        // The Bridge after it, then the Intro in its place: as long, so
        // no time moves and the audio is given nothing.
        h.act(TimeAction::ListAdd).unwrap();
        laid_as(&h, &[(2, 2), (3, 1)]);
        let n = audio.0.len();
        h.act_with_audio(TimeAction::ListScene { step: 1, delta: 1 }, &mut audio)
            .unwrap();
        laid_as(&h, &[(2, 2), (0, 1)]);
        assert_eq!(audio.0.len(), n, "{:?}", audio.0);
    }

    /// Arrange 4b.2: the Song row (the bars before the first part) stays
    /// first, has no scene and plays once, but can be taken out; × N on a
    /// part of the song's own puts a copy of its last pass after it,
    /// everything in it with it (its blocks and tempo), and – takes it
    /// out again. What is refused changes nothing.
    #[test]
    fn a_linked_lists_song_row_stays_first_and_a_part_of_its_own_repeats_whole() {
        use mpfree_seq::arrange::{Marker, Placement, TempoPoint};
        const B: Tick = 3840;
        let mut h = linked_grid();
        let p = &mut h.control.project;
        let song = &mut p.arrangement;
        // A bar of the song's own, then part A: the Verse's clip for two
        // bars, and 140 BPM from its second bar.
        song.place(Placement::of(&p.tracks, 0, 0, 0, B)).unwrap();
        song.place(Placement::of(&p.tracks, 0, 1, B, 2 * B))
            .unwrap();
        song.mark(B, "A").unwrap();
        song.tempo = vec![TempoPoint {
            at: 2 * B,
            bpm: 140.0,
            ramp: false,
        }];
        p.check().unwrap();
        let was = p.arrangement.clone();
        let rows = |h: &TimeHost| -> Vec<(String, Option<usize>, u16, i64)> {
            h.state()
                .list
                .iter()
                .map(|r| (r.name.clone(), r.scene, r.times, r.bars))
                .collect()
        };
        assert_eq!(
            rows(&h),
            [("Song".into(), None, 1, 1), ("A".into(), None, 1, 2)]
        );
        for (a, why) in [
            (TimeAction::ListMove { step: 1, to: 0 }, "row stays first"),
            (TimeAction::ListMove { step: 0, to: 2 }, "row stays first"),
            (TimeAction::ListTimes { step: 0, delta: 1 }, "add a marker"),
            (TimeAction::ListScene { step: 0, delta: 1 }, "add a marker"),
            (TimeAction::ListRemove(2), "no such step"),
        ] {
            let e = h.act(a.clone()).unwrap_err();
            assert!(e.contains(why), "{a:?}: {e}");
            assert_eq!(h.control.project.arrangement, was);
        }
        // A × 2: its two bars again after it, the tempo change with them.
        assert_eq!(
            h.act(TimeAction::ListTimes { step: 1, delta: 1 }).unwrap(),
            "song list: A ×2 (UNDO takes it back)"
        );
        assert_eq!(
            rows(&h),
            [("Song".into(), None, 1, 1), ("A".into(), None, 2, 4)]
        );
        let song = &h.control.project.arrangement;
        assert_eq!(
            song.markers,
            [Marker {
                passes: 2,
                ..Marker::new(B, "A")
            }]
        );
        assert_eq!(song.end(), 5 * B);
        let bpm: Vec<(Tick, f64)> = song.tempo.iter().map(|t| (t.at, t.bpm)).collect();
        assert_eq!(bpm, [(2 * B, 140.0), (3 * B, 120.0), (4 * B, 140.0)]);
        // A × 1: the copy out again, the song as it was.
        h.act(TimeAction::ListTimes { step: 1, delta: -1 }).unwrap();
        assert_eq!(h.control.project.arrangement, was);
        // The Song row taken out: A starts the song.
        h.act(TimeAction::ListRemove(0)).unwrap();
        assert_eq!(rows(&h), [("A".into(), None, 1, 2)]);
        assert_eq!(h.control.project.arrangement.markers[0].at, 0);
        for _ in 0..3 {
            h.act(TimeAction::Undo).unwrap();
        }
        assert_eq!(h.control.project.arrangement, was);
    }

    /// The linked list's rows as (name, scene, times, bars, changed).
    fn linked_rows(h: &TimeHost) -> Vec<(String, Option<usize>, u16, i64, bool)> {
        h.state()
            .list
            .iter()
            .map(|r| (r.name.clone(), r.scene, r.times, r.bars, r.changed))
            .collect()
    }

    /// Arrange 4b.3: an edit inside a part on the Song view makes it the
    /// song's own: changed in the song, × 1, its bars as they were, and
    /// the log says how to take it back. The list's edits leave it as it
    /// is: moved, repeated whole. Back to the scene makes it from its
    /// scene again, the song as List → song lays it, as one UNDO; on a
    /// part that follows its scene it says so and changes nothing.
    #[test]
    fn an_edit_inside_a_part_makes_it_the_songs_own_until_back_to_the_scene() {
        use mpfree_time_ui::{BarsEdit, MarkerEdit};
        const B: Tick = 3840;
        let mut h = linked_grid();
        for a in [
            TimeAction::ListAdd,
            TimeAction::ListAdd,
            TimeAction::ListAdd,
            TimeAction::ListTimes { step: 1, delta: 1 },
        ] {
            h.act(a).unwrap();
        }
        // Intro bar 1, Verse × 2 bars 2 to 5, Chorus bars 6 to 9.
        laid_as(&h, &[(0, 1), (1, 2), (2, 1)]);
        let laid = h.control.project.arrangement.clone();
        for k in [0, 1] {
            let e = h.act(TimeAction::ListBack(k)).unwrap_err();
            assert!(e.ends_with("follows its scene already"), "{e}");
        }
        assert_eq!(h.control.project.arrangement, laid);
        // The Verse's bass taken out on the Song view.
        let bass = laid
            .placements
            .iter()
            .position(|p| p.track == 1 && p.start == B)
            .unwrap();
        assert_eq!(
            h.act(TimeAction::DeletePlacement(bass)).unwrap(),
            "song: Verse is the song's own now: Back to the scene on the song list puts the scene's clips back"
        );
        let own = h.control.project.arrangement.clone();
        assert_eq!(
            linked_rows(&h),
            [
                ("Intro".into(), Some(0), 1, 1, false),
                ("Verse".into(), Some(1), 1, 4, true),
                ("Chorus".into(), Some(2), 1, 4, false),
            ]
        );
        // Moved and back, the Chorus put before it and taken out again,
        // × 2 (its whole four bars again) and back: as it was each time,
        // still the song's own.
        for (a, back) in [
            (
                TimeAction::ListMove { step: 1, to: 0 },
                TimeAction::ListMove { step: 0, to: 2 },
            ),
            (
                TimeAction::ListMove { step: 2, to: 1 },
                TimeAction::ListMove { step: 1, to: 3 },
            ),
            (
                TimeAction::ListTimes { step: 1, delta: 1 },
                TimeAction::ListTimes { step: 1, delta: -1 },
            ),
        ] {
            h.act(a.clone()).unwrap();
            assert!(
                linked_rows(&h).iter().any(|r| r.0 == "Verse" && r.4),
                "{a:?}"
            );
            h.act(back).unwrap();
            assert_eq!(h.control.project.arrangement, own, "{a:?}");
        }
        h.act(TimeAction::ListTimes { step: 1, delta: 1 }).unwrap();
        assert_eq!(linked_rows(&h)[1], ("Verse".into(), Some(1), 2, 8, true));
        h.act(TimeAction::Undo).unwrap();
        // Back to the scene: two passes fit its four bars, as before.
        assert_eq!(
            h.act(TimeAction::ListBack(1)).unwrap(),
            "song list: Verse plays its scene again ×2 (UNDO takes it back)"
        );
        assert_eq!(h.control.project.arrangement, laid);
        laid_as(&h, &[(0, 1), (1, 2), (2, 1)]);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.control.project.arrangement, own);
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(h.control.project.arrangement, laid);
        // A bar of room put in it on the Song view: five bars, the song's
        // own; back to the scene, two passes again, the bar taken out.
        h.act(TimeAction::SongBars(BarsEdit::Insert {
            at: 2 * B,
            bars: 1,
        }))
        .unwrap();
        assert_eq!(linked_rows(&h)[1], ("Verse".into(), Some(1), 1, 5, true));
        assert_eq!(
            h.act(TimeAction::ListBack(1)).unwrap(),
            "song list: Verse plays its scene again ×2; the song after it moved up (UNDO takes it back)"
        );
        assert_eq!(h.control.project.arrangement, laid);
        // The Song row, and a part with no scene, have none to go back to.
        h.act(TimeAction::SongMarker(MarkerEdit::Move {
            index: 0,
            at: B / 2,
        }))
        .unwrap();
        let e = h.act(TimeAction::ListBack(0)).unwrap_err();
        assert!(e.contains("add a marker"), "{e}");
        assert_eq!(h.dropped(), 0);
    }

    /// Arrange 4b.3: the Song view's edits move the list. A part dragged
    /// along the ruler moves as its grip does, and back again it is as it
    /// was (a round trip); a copy is a part of its own that still follows
    /// its scene; a marker put inside a part makes the bars before it the
    /// song's own, and the bars after it a part of their own.
    #[test]
    fn the_song_views_section_edits_move_the_linked_list() {
        use mpfree_time_ui::MarkerEdit;
        const B: Tick = 3840;
        let three = || {
            let mut h = linked_grid();
            for _ in 0..3 {
                h.act(TimeAction::ListAdd).unwrap();
            }
            h
        };
        let mut h = three();
        let mut g = three();
        let laid = h.control.project.arrangement.clone();
        h.act(TimeAction::MoveSection {
            marker: 2,
            gap: 0,
            copy: false,
        })
        .unwrap();
        g.act(TimeAction::ListMove { step: 2, to: 0 }).unwrap();
        assert_eq!(h.control.project.arrangement, g.control.project.arrangement);
        laid_as(&h, &[(2, 1), (0, 1), (1, 1)]);
        h.act(TimeAction::ListMove { step: 0, to: 3 }).unwrap();
        assert_eq!(h.control.project.arrangement, laid);
        // The Chorus (bars 4 to 7) copied after itself: its blocks join
        // their copies, and both parts hold what the Chorus plays.
        h.act(TimeAction::MoveSection {
            marker: 2,
            gap: 3,
            copy: true,
        })
        .unwrap();
        assert_eq!(
            linked_rows(&h),
            [
                ("Intro".into(), Some(0), 1, 1, false),
                ("Verse".into(), Some(1), 1, 2, false),
                ("Chorus".into(), Some(2), 1, 4, false),
                ("Chorus".into(), Some(2), 1, 4, false),
            ]
        );
        // A marker half way through the copy (bars 8 to 11).
        assert_eq!(
            h.act(TimeAction::SongMarker(MarkerEdit::Add(9 * B))).unwrap(),
            "song: Chorus is the song's own now: Back to the scene on the song list puts the scene's clips back"
        );
        let rows = linked_rows(&h);
        assert_eq!(rows[3], ("Chorus".into(), Some(2), 1, 2, true));
        assert_eq!(
            (rows[4].1, rows[4].2, rows[4].3, rows[4].4),
            (None, 1, 2, false)
        );
        assert_eq!(rows.len(), 5);
        assert_eq!(h.dropped(), 0);
    }

    /// Arrange 4b.3: Scenes → song on a linked list puts the rows after
    /// its last part, past the silence a part that follows its scene may
    /// end with, so the part isn't cut.
    #[test]
    fn scenes_to_song_on_a_linked_list_goes_after_its_last_part() {
        use mpfree_seq::arrange::{FromScene, Placement};
        const B: Tick = 3840;
        let mut h = linked_grid();
        let p = &mut h.control.project;
        // The Intro's bar, then three bars of its silence.
        let song = &mut p.arrangement;
        song.place(Placement::of(&p.tracks, 0, 0, 0, B)).unwrap();
        song.mark(0, "Intro").unwrap();
        song.markers[0].scene = Some(FromScene::following(0, 4 * B));
        h.act(TimeAction::ScenesToSong).unwrap();
        let song = &h.control.project.arrangement;
        assert_eq!(song.markers[1].at, 4 * B);
        assert_eq!(song.markers[0].scene, Some(FromScene::following(0, 4 * B)));
    }

    /// Arrange 4b.3: Rec song over a part that follows its scene makes it
    /// the song's own when the recording goes in.
    #[test]
    fn rec_song_over_a_part_makes_it_the_songs_own() {
        const B: Tick = 3840;
        let mut h = linked_grid();
        let mut pads = Fake::default();
        h.act(TimeAction::ListAdd).unwrap();
        h.act(TimeAction::ListAdd).unwrap();
        // The Chorus played over the Verse (bars 2 and 3), recorded.
        h.command(Command::Locate(B));
        h.act(TimeAction::LaunchScene(2)).unwrap();
        h.act(TimeAction::RecSong(true)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.command(Command::Stop);
        run(&mut h, &mut pads, BAR, &[]);
        let note = h.take_later_note().unwrap();
        assert_eq!(
            note,
            "rec song: Verse is the song's own now: Back to the scene on the song list puts the scene's clips back"
        );
        assert!(linked_rows(&h)[1].4);
        assert!(!linked_rows(&h)[0].4);
    }

    /// The grid Arrange 4b.4's tests play, its list (unlinked) the steps
    /// row 1 once, row 2 twice and row 3 once:
    ///
    /// - track 1: a bar in each row (pads 10, 20, 30);
    /// - track 2: row 1's bar moves to row 2's after a bar (a follow), and
    ///   row 3 has a bar (pads 40, 50, 60);
    /// - track 3, which plays elsewhere: two bars in row 1 (so row 1 is
    ///   two bars), none in row 2, a bar in row 3 (pads 70, 80);
    /// - scene 1 hears every track and sends track 3 program 6; scene 2
    ///   mutes track 1 (still muted in row 3, which leaves the mutes as
    ///   they are) and sends no program; scene 3 sends program 10.
    fn section_grid() -> mpfree_seq::Project {
        use mpfree_seq::{ClipProgram, Follow, FollowAction, ListStep, Note, TrackProgram};
        let clip = |bars: i64, pad: u8| {
            let mut c = Clip::new("beats", bars * 3840);
            c.notes = (0..4 * bars)
                .map(|k| Note::new(k * 960, 100, pad + k as u8, 100))
                .collect();
            Some(c)
        };
        let mut p = TimeHost::new(RATE, 120.0).unwrap().project().clone();
        let t = &mut p.tracks;
        for (row, pad) in [(0, 10), (1, 20), (2, 30)] {
            t[0].slots[row] = clip(1, pad);
        }
        t[1].slots[0] = clip(1, 40).map(|mut c| {
            c.follow = Some(Follow::after(1, FollowAction::Next));
            c
        });
        t[1].slots[1] = clip(1, 50);
        t[1].slots[2] = clip(1, 60);
        t[2].slots[0] = clip(2, 70);
        t[2].slots[2] = clip(1, 80);
        let program = |n: u8| {
            vec![TrackProgram {
                track: 2,
                program: ClipProgram {
                    program: n,
                    bank: None,
                },
            }]
        };
        let s = p.scene_mut(0).unwrap();
        s.mutes = Some(0);
        s.programs = program(5);
        p.scene_mut(1).unwrap().mutes = Some(0b1);
        p.scene_mut(2).unwrap().programs = program(9);
        p.arrangement.list.steps = [(0, 1), (1, 2), (2, 1)]
            .into_iter()
            .map(|(row, times)| ListStep { row, times })
            .collect();
        p.give_clip_ids();
        p
    }

    /// [`section_grid`] linked: its song made by the list's edits, row 1
    /// bars 1 and 2, row 2 bars 3 and 4, row 3 bar 5.
    fn section_song() -> TimeHost {
        let mut p = section_grid();
        p.arrangement.list.steps.clear();
        p.arrangement.list.linked = true;
        let mut h = TimeHost::with_project(RATE, 120.0, p).unwrap();
        for a in [
            TimeAction::ListAdd,
            TimeAction::ListAdd,
            TimeAction::ListAdd,
            TimeAction::ListTimes { step: 1, delta: 1 },
        ] {
            h.act(a).unwrap();
        }
        // Its blocks are the scenes' clips with no scene's mutes: track 1
        // plays on through rows 2 and 3 (List → song leaves them out).
        let song = &h.control.project.arrangement;
        let parts: Vec<(Tick, Option<usize>, u16)> = song
            .markers
            .iter()
            .map(|m| (m.at, m.scene.map(|f| f.row), m.passes))
            .collect();
        assert_eq!(
            parts,
            [
                (0, Some(0), 1),
                (2 * 3840, Some(1), 2),
                (4 * 3840, Some(2), 1)
            ]
        );
        let track_1: Vec<Tick> = song
            .placements
            .iter()
            .filter(|b| b.track == 0)
            .map(|b| b.start)
            .collect();
        assert_eq!(track_1, [0, 2 * 3840, 4 * 3840]);
        h
    }

    /// Arrange 4b.4 (plan D8, playing the list plays the song): a linked
    /// song played in song mode gives what the list played live gives,
    /// the same notes and program changes on the same frames at several
    /// period sizes: each section brings its scene's mutes (carried into
    /// the next section, as the list carries them) and program changes as
    /// it starts, though its blocks were made with none. Looped, the song
    /// goes round as the list does, the first section's mutes and program
    /// changes in again on the jump.
    #[test]
    fn a_linked_song_plays_what_its_list_played_at_every_period_size() {
        for (period, looped) in [
            (7u32, false),
            (128, false),
            (1000, false),
            (7, true),
            (128, true),
            (1000, true),
        ] {
            let to = if looped { 11 * BAR } else { 5 * BAR + 1000 };
            let by = |h: &mut TimeHost, r: &mut Routed, to: u64| {
                while r.pads.frame < to {
                    h.period(period, r);
                    r.pads.frame += u64::from(period);
                }
            };
            let routed = || Routed {
                track: 2,
                ..Routed::default()
            };
            let mut live = TimeHost::with_project(RATE, 120.0, section_grid()).unwrap();
            let mut heard = routed();
            live.act(TimeAction::ListLoop(looped)).unwrap();
            live.act(TimeAction::ListPlay(0)).unwrap();
            live.command(Command::Play);
            by(&mut live, &mut heard, to);
            let mut song = section_song();
            song.act(TimeAction::SongMode(true)).unwrap();
            song.act(TimeAction::ListLoop(looped)).unwrap();
            // A period stopped: the first section comes in at once.
            song.period(period, &mut routed());
            let mut played = routed();
            song.command(Command::Play);
            by(&mut song, &mut played, to);
            let p = &played.pads;
            let why = format!("period {period}, looped {looped}");
            assert_eq!(p.hits, heard.pads.hits, "{why}");
            assert_eq!(p.releases, heard.pads.releases, "{why}");
            assert_eq!(played.taken, heard.taken, "{why}");
            assert_eq!((live.dropped(), song.dropped()), (0, 0));
            // What the mutes and programs show: track 2's follow, track 1
            // muted from row 2 on (heard again round the loop), and the
            // programs on their bars.
            let first = |pad: usize, from: u64| {
                let mut hits = p.hits.iter().filter(|h| h.0 >= from);
                hits.find(|h| h.1 == pad).map(|h| h.0)
            };
            assert_eq!((first(10, 0), first(50, 0)), (Some(0), Some(BAR)));
            assert_eq!(first(20, 0), None);
            assert_eq!((first(30, 0), first(60, 0)), (None, Some(4 * BAR)));
            let programs: Vec<(u64, u8)> = played
                .taken
                .iter()
                .filter_map(|&(f, e)| match e {
                    Event::Program { program, .. } => Some((f, program)),
                    _ => None,
                })
                .collect();
            if looped {
                assert_eq!(first(10, 2 * BAR), Some(5 * BAR), "{why}");
                assert_eq!(
                    programs,
                    [
                        (0, 5),
                        (4 * BAR, 9),
                        (5 * BAR, 5),
                        (9 * BAR, 9),
                        (10 * BAR, 5)
                    ],
                    "{why}"
                );
            } else {
                assert_eq!(first(10, 2 * BAR), None, "{why}");
                assert_eq!(programs, [(0, 5), (4 * BAR, 9)], "{why}");
            }
        }
    }

    /// Arrange 5.2b: the scenes' tempi come in where their rows start,
    /// whether the list plays the rows live or a linked song plays its
    /// sections in song mode, on the same frames; and List → song writes
    /// them into the song as tempo points.
    #[test]
    fn the_lists_scenes_bring_their_tempo_live_and_from_a_linked_song() {
        let tempi = |p: &mut mpfree_seq::Project| {
            p.scene_mut(1).unwrap().set_bpm(Some(150.0)).unwrap();
            p.scene_mut(2).unwrap().set_bpm(Some(90.0)).unwrap();
        };
        // Each period as the app runs it, with the tempo it takes: the
        // frames each new tempo was first heard on.
        let play = |h: &mut TimeHost| {
            let mut pads = Fake::default();
            let mut app = 120.0;
            let mut changes = vec![];
            h.command(Command::Play);
            while pads.frame < 5 * BAR {
                if let Some(bpm) = h.take_scene_tempo() {
                    app = bpm;
                }
                h.follow_tempo(app);
                h.period(PERIOD, &mut pads);
                pads.frame += u64::from(PERIOD);
                let bpm = h.info().bpm;
                if changes.last().is_none_or(|c: &(u64, f64)| c.1 != bpm) {
                    changes.push((pads.frame, bpm));
                }
            }
            (changes, app)
        };
        // Row 2 starts two seconds in at 120 BPM; row 3 two bars later at
        // 150, 3.2 s on.
        let want = [(2 * BAR, 150.0), (2 * BAR + 2 * BAR * 4 / 5, 90.0)];
        let near = |got: &[(u64, f64)]| {
            got.len() == 3
                && got[0].1 == 120.0
                && got[1..]
                    .iter()
                    .zip(want)
                    .all(|(g, w)| g.1 == w.1 && g.0.abs_diff(w.0) <= u64::from(PERIOD))
        };
        let mut p = section_grid();
        tempi(&mut p);
        let mut live = TimeHost::with_project(RATE, 120.0, p).unwrap();
        live.act(TimeAction::ListPlay(0)).unwrap();
        let (got, app) = play(&mut live);
        assert!(near(&got), "live: {got:?}");
        assert_eq!(app, 90.0);
        let mut song = section_song();
        tempi(&mut song.control.project);
        song.act(TimeAction::SongMode(true)).unwrap();
        let (got, app) = play(&mut song);
        assert!(near(&got), "linked song: {got:?}");
        assert_eq!(app, 90.0);
        // List → song: a tempo point where each row with one starts.
        let mut p = section_grid();
        tempi(&mut p);
        let mut h = TimeHost::with_project(RATE, 120.0, p).unwrap();
        h.act(TimeAction::ListToSong).unwrap();
        let points: Vec<(Tick, f64)> = h
            .control
            .project
            .arrangement
            .tempo
            .iter()
            .map(|t| (t.at, t.bpm))
            .collect();
        assert_eq!(points, [(2 * 3840, 150.0), (4 * 3840, 90.0)]);
    }

    /// Arrange 4b.5: Link on a list that isn't linked. Song from the list
    /// lays the list's steps at the song's start as the linked list's
    /// edits make them, the old song after them (a "Song" marker where its
    /// unmarked bars start), and the audio tracks get the same room; List
    /// from the song leaves the song as it is and reads its parts. Each is
    /// one UNDO, and a linked list can't be linked again.
    #[test]
    fn link_makes_the_song_from_the_list_or_the_list_from_the_song() {
        use mpfree_seq::arrange::Placement;
        use mpfree_seq::TimeEdit;
        use mpfree_time_ui::view::LinkFrom;
        const B: Tick = 3840;
        // An old song: a bar of row 3's clip on track 1, no marker.
        let mut p = section_grid();
        p.arrangement
            .place(Placement::of(&p.tracks, 0, 2, 0, B))
            .unwrap();
        let mut h = TimeHost::with_project(RATE, 120.0, p).unwrap();
        let was = h.control.project.arrangement.clone();
        let mut audio = Regions::default();
        let said = h
            .act_with_audio(TimeAction::ListLink(LinkFrom::List), &mut audio)
            .unwrap();
        assert_eq!(
            said,
            "song list: linked to the song: its 3 steps are the song's first 5 bars, the old song after them (UNDO takes it back)"
        );
        let given = |edits: &[TimeEdit]| -> Vec<mpfree_audiotracks::Retime> {
            edits.iter().filter_map(control::retime_of).collect()
        };
        let had: Vec<_> = audio.0.iter().map(|x| x.1).collect();
        assert_eq!(had, given(&[TimeEdit::Insert { at: 0, len: 5 * B }]));
        // The steps' sections are the ones the list's edits make; the old
        // bar after them, marked "Song".
        let song = &h.control.project.arrangement;
        let made = section_song();
        let laid = &made.control.project.arrangement;
        assert!(song.list.linked && song.list.steps.is_empty());
        assert_eq!(&song.markers[..3], &laid.markers[..]);
        assert_eq!(
            (song.markers[3].at, song.markers[3].name.as_str()),
            (5 * B, "Song")
        );
        let before: Vec<&Placement> = song.placements.iter().filter(|b| b.start < 5 * B).collect();
        assert_eq!(before, laid.placements.iter().collect::<Vec<_>>());
        let after: Vec<(usize, Tick)> = song
            .placements
            .iter()
            .filter(|b| b.start >= 5 * B)
            .map(|b| (b.track, b.start))
            .collect();
        assert_eq!(after, [(0, 5 * B)]);
        let rows: Vec<(String, Option<usize>, u16)> = h
            .state()
            .list
            .iter()
            .map(|r| (r.name.clone(), r.scene, r.times))
            .collect();
        assert_eq!(
            rows,
            [
                ("Scene 1".into(), Some(0), 1),
                ("Scene 2".into(), Some(1), 2),
                ("Scene 3".into(), Some(2), 1),
                ("Song".into(), None, 1),
            ]
        );
        assert_eq!(
            h.act(TimeAction::ListLink(LinkFrom::Song)),
            Err("song list: it is linked to the song already".into())
        );
        // One UNDO back to the list of its own; REDO links it again.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.control.project.arrangement, was);
        h.act(TimeAction::Redo).unwrap();
        assert!(h.control.project.arrangement.list.linked);
        h.act(TimeAction::Undo).unwrap();
        // List from the song: the song as it is, its parts the list, each
        // × 1 (List → song's markers are plain).
        h.act(TimeAction::ListToSong).unwrap();
        let laid = h.control.project.arrangement.clone();
        let said = h.act(TimeAction::ListLink(LinkFrom::Song)).unwrap();
        assert_eq!(
            said,
            "song list: linked to the song: its 3 marked parts are the list (UNDO takes it back)"
        );
        let song = &h.control.project.arrangement;
        assert!(song.list.linked && song.list.steps.is_empty());
        assert_eq!(
            (&song.placements, &song.markers),
            (&laid.placements, &laid.markers)
        );
        let rows: Vec<(String, u16, i64)> = h
            .state()
            .list
            .iter()
            .map(|r| (r.name.clone(), r.times, r.bars))
            .collect();
        assert_eq!(
            rows,
            [
                ("Song".into(), 1, 1),
                ("Scene 1".into(), 1, 2),
                ("Scene 2".into(), 1, 2),
                ("Scene 3".into(), 1, 1)
            ]
        );
        assert_eq!(h.dropped(), 0);
    }

    /// Arrange 4b.4: Play from here on a linked list stops the clips
    /// launched over the song and plays the song from the part's first
    /// bar, song mode on and its scene's mutes in at once; stopped, it
    /// starts the transport. Loop the list loops the song, first part to
    /// last (the transport's loop, the musician's kept aside and given
    /// back), and the first part's mutes and program changes come in on
    /// the loop's jump; a new clip is as long as the musician's loop.
    #[test]
    fn play_from_here_plays_the_song_from_a_part_and_loop_the_list_goes_round() {
        const B: Tick = 3840;
        let mut h = section_song();
        let mut r = Routed {
            track: 2,
            ..Routed::default()
        };
        let by = |h: &mut TimeHost, r: &mut Routed, to: u64| {
            while r.pads.frame < to {
                h.period(PERIOD, r);
                r.pads.frame += u64::from(PERIOD);
            }
        };
        let more = |h: &mut TimeHost, r: &mut Routed, periods: u64| {
            let to = r.pads.frame + periods * u64::from(PERIOD);
            by(h, r, to);
        };
        let hits_from = |r: &Routed, from: u64| -> Vec<(u64, usize)> {
            let hits = r.pads.hits.iter().filter(|h| h.0 >= from);
            hits.map(|h| (h.0 - from, h.1)).collect()
        };
        // A clip launched over the song (song mode off), half a bar in.
        h.act(TimeAction::Launch { track: 1, slot: 2 }).unwrap();
        h.command(Command::Play);
        by(&mut h, &mut r, BAR / 2);
        let said = h.act(TimeAction::ListPlay(1)).unwrap();
        assert_eq!(said, "song list: the song from Scene 2, bar 3");
        let st = h.state();
        assert!(st.song_on && st.launches.is_empty() && st.cues.is_empty());
        assert_eq!(st.list_pick, Some(1));
        let from = r.pads.frame;
        by(&mut h, &mut r, from + 3 * BAR + 1000);
        // Track 2's row 2 from bar 3, its row 3 from bar 5; track 1 muted.
        let pads: Vec<usize> = hits_from(&r, from).iter().map(|h| h.1).collect();
        assert_eq!(pads, [50, 51, 52, 53, 50, 51, 52, 53, 60, 61, 62, 63]);
        assert_eq!(hits_from(&r, from)[8].0, 2 * BAR);
        assert!(r.taken.contains(&(
            from + 2 * BAR,
            Event::Program {
                program: 9,
                bank: None
            }
        )));
        // Stopped: from bar 5, the transport started.
        h.command(Command::Stop);
        more(&mut h, &mut r, 2);
        h.act(TimeAction::ListPlay(2)).unwrap();
        more(&mut h, &mut r, 1);
        assert!(h.info().state.rolling());
        // Loop the list: the song's five bars, the musician's loop kept.
        h.command(Command::SetLoop(Some((B, 3 * B))));
        more(&mut h, &mut r, 1);
        h.act(TimeAction::ListLoop(true)).unwrap();
        more(&mut h, &mut r, 2);
        assert_eq!(h.info().loop_range, Some((0, 5 * B)));
        h.command(Command::Stop);
        more(&mut h, &mut r, 2);
        h.act(TimeAction::ListPlay(2)).unwrap();
        let from = r.pads.frame;
        r.taken.clear();
        by(&mut h, &mut r, from + 4 * BAR - 1000);
        // Bar 5, then round to bar 1: row 1's mutes (every track heard)
        // and program on the jump, then row 2's mutes on bar 3 again.
        let hits = hits_from(&r, from);
        let first = |pad: usize| hits.iter().find(|h| h.1 == pad).map(|h| h.0);
        assert_eq!(
            (first(30), first(10), first(40)),
            (Some(0), Some(BAR), Some(BAR))
        );
        assert_eq!(first(20), None, "muted again on bar 3");
        assert_eq!(first(50).map(|f| f > BAR), Some(true));
        let programs: Vec<(u64, u8)> = r
            .taken
            .iter()
            .filter_map(|&(f, e)| match e {
                Event::Program { program, .. } => Some((f - from, program)),
                _ => None,
            })
            .collect();
        assert_eq!(programs, [(0, 9), (BAR, 5)]);
        // A new clip is as long as the musician's loop (two bars).
        h.act(TimeAction::NewClip { track: 3, slot: 0 }).unwrap();
        assert_eq!(h.control.project.clip(3, 0).map(|c| c.length), Some(2 * B));
        // Loop the list off: the musician's loop back; song mode off: no
        // list loop either.
        h.act(TimeAction::ListLoop(false)).unwrap();
        more(&mut h, &mut r, 2);
        assert_eq!(h.info().loop_range, Some((B, 3 * B)));
        h.act(TimeAction::ListLoop(true)).unwrap();
        h.act(TimeAction::SongMode(false)).unwrap();
        more(&mut h, &mut r, 2);
        assert_eq!(h.info().loop_range, Some((B, 3 * B)));
        assert_eq!(h.dropped(), 0);
    }

    /// The song's parts follow their rows (parts.rs): a clip copied into
    /// the Verse's row, deleted from it, or moved to the Intro's row, and
    /// the parts made from those rows are made again, so the song is the
    /// one List → song lays for the grid as it is now. Each is one UNDO
    /// with the clip, and REDO brings both back.
    #[test]
    fn the_songs_parts_take_in_what_their_rows_hold_and_one_undo_takes_both_back() {
        let mut h = linked_grid();
        h.act(TimeAction::ListAdd).unwrap();
        h.act(TimeAction::ListAdd).unwrap();
        h.act(TimeAction::ListTimes { step: 1, delta: 1 }).unwrap();
        let steps = [(0, 1), (1, 2)];
        laid_as(&h, &steps);
        let mut songs = vec![h.control.project.arrangement.clone()];
        // A copy of the Intro's clip into the Verse's row, on track 3.
        let said = h
            .act(TimeAction::DropClip {
                from: (0, 0),
                to: (2, 1),
                copy: true,
            })
            .unwrap();
        assert!(
            said.contains("; the song's parts from row 2 with it"),
            "{said}"
        );
        laid_as(&h, &steps);
        let copy = h.control.project.clip(2, 1).unwrap().id;
        assert!(h.control.project.arrangement.plays(copy));
        assert_eq!(h.state().done.last().unwrap().followed, Some(1));
        songs.push(h.control.project.arrangement.clone());
        // Deleted from the row: it leaves the parts, so the song keeps no
        // copy of it.
        let said = h.act(TimeAction::DeleteClip { track: 2, slot: 1 }).unwrap();
        assert!(!said.contains("keeps playing it"), "{said}");
        assert!(
            said.contains("the song's parts from row 2 with it"),
            "{said}"
        );
        laid_as(&h, &steps);
        assert!(!h.control.project.arrangement.plays(copy));
        assert!(h.control.project.arrangement.clips.is_empty());
        songs.push(h.control.project.arrangement.clone());
        // The Verse's bass moved to the Intro's row: both parts change.
        let said = h
            .act(TimeAction::DropClip {
                from: (1, 1),
                to: (1, 0),
                copy: false,
            })
            .unwrap();
        assert!(
            said.contains("the song's parts from their rows with it"),
            "{said}"
        );
        laid_as(&h, &steps);
        songs.push(h.control.project.arrangement.clone());
        // One UNDO each takes the clip and the parts back, REDO both again.
        for k in (0..3).rev() {
            h.act(TimeAction::Undo).unwrap();
            assert_eq!(h.control.project.arrangement, songs[k], "undo to {k}");
            laid_as(&h, &steps);
        }
        assert!(h.control.project.clip(2, 1).is_none());
        for (k, song) in songs.iter().enumerate().skip(1) {
            h.act(TimeAction::Redo).unwrap();
            assert_eq!(h.control.project.arrangement, *song, "redo to {k}");
        }
        assert_eq!(h.dropped(), 0);
    }

    /// The scene editor's row goes with its row when the row is dragged
    /// on the grid, as its scene does.
    #[test]
    fn the_scene_editors_row_follows_a_dragged_row() {
        let mut h = linked_grid();
        h.act(TimeAction::SceneFocus(1)).unwrap();
        h.act(TimeAction::DropRow {
            from: 1,
            to: 3,
            copy: false,
        })
        .unwrap();
        let st = h.state();
        assert_eq!((st.scene_focus, st.scene.name.as_str()), (2, "Verse"));
        h.act(TimeAction::DropRow {
            from: 0,
            to: 0,
            copy: true,
        })
        .unwrap();
        let st = h.state();
        assert_eq!((st.scene_focus, st.scene.name.as_str()), (3, "Verse"));
    }

    /// What doesn't follow its row (parts.rs): a part changed in the song
    /// keeps what it holds; a part keeps its bars when its row gets a
    /// longer clip, the blocks cut at its end; a list that isn't linked
    /// leaves the song alone; a new clip REC puts in a row is no take, so
    /// its parts are a take of their own.
    #[test]
    fn parts_keep_their_bars_and_changed_parts_and_unlinked_lists_stay_as_they_are() {
        let mut h = linked_grid();
        h.act(TimeAction::ListAdd).unwrap();
        h.act(TimeAction::ListAdd).unwrap();
        // A four-bar clip into the one-bar Intro's row: the Intro stays a
        // bar, and the Verse where it was.
        let mut long = Clip::new("long", 4 * 3840);
        long.notes = vec![mpfree_seq::Note::new(0, 100, 4, 100)];
        let ok = h.control.project.put_new_clip(3, 3, long).unwrap();
        let said = h
            .act(TimeAction::DropClip {
                from: (3, 3),
                to: (3, 0),
                copy: false,
            })
            .unwrap();
        assert!(said.contains("from row 1 with it"), "{said}");
        let song = &h.control.project.arrangement;
        let ats: Vec<Tick> = song.markers.iter().map(|m| m.at).collect();
        assert_eq!(ats, vec![0, 3840]);
        let b = song.placements.iter().find(|p| p.clip == ok).unwrap();
        assert_eq!((b.start, b.length), (0, 3840), "cut at the Intro's end");
        // The Verse changed in the song: a clip into its row leaves it.
        h.control.project.arrangement.markers[1]
            .scene
            .as_mut()
            .unwrap()
            .changed = true;
        let was = h.control.project.arrangement.clone();
        let said = h
            .act(TimeAction::DropClip {
                from: (0, 0),
                to: (2, 1),
                copy: true,
            })
            .unwrap();
        assert!(!said.contains("the song's parts"), "{said}");
        assert_eq!(h.control.project.arrangement, was);
        // REC on track 3 puts a new clip in its first empty cell, the
        // Intro's row: the Intro takes it in, as a take of its own.
        h.act(TimeAction::DeleteClip { track: 3, slot: 0 }).unwrap();
        h.act(TimeAction::Arm(Some(3))).unwrap();
        let before = h.control.project.arrangement.clone();
        let said = h.act(TimeAction::Transport(Command::Record)).unwrap();
        assert!(
            said.ends_with("new clip in slot 1 of track 4; the song's parts from row 1 with it"),
            "{said}"
        );
        let id = h.control.project.clip(3, 0).unwrap().id;
        assert!(h.control.project.arrangement.plays(id));
        let last = *h.state().done.last().unwrap();
        assert_eq!(
            last.song,
            Some(mpfree_play::SongChange::Followed { row: 0 })
        );
        h.act(TimeAction::Transport(Command::Stop)).unwrap();
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.control.project.arrangement, before);
        // Not linked: nothing follows.
        h.control.project.arrangement.list.linked = false;
        let was = h.control.project.arrangement.clone();
        h.act(TimeAction::DeleteClip { track: 0, slot: 0 }).unwrap();
        assert_eq!(h.control.project.arrangement.placements, was.placements);
        assert_eq!(h.dropped(), 0);
    }

    /// Save state and Reload clip (sequencer S4): the saved state comes
    /// back on the launch line while the clip plays, at once while the
    /// transport is stopped; UNDO takes a reload back; asked again while
    /// it waits, it is taken back; a deleted clip forgets its state.
    #[test]
    fn reload_clip_lands_on_the_launch_line_and_undo_takes_it_back() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let one_bar = |note: u8| {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            c
        };
        h.control.project.tracks[0].slots[0] = Some(one_bar(4));
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        let reload = || TimeAction::Reload { track: 0, slot: 0 };
        let pitch = |h: &TimeHost| h.control.project.clip(0, 0).unwrap().notes[0].note;
        assert!(h
            .act(reload())
            .unwrap_err()
            .contains("no saved state: Save state on the Clip card first"));
        assert_eq!(
            h.act(TimeAction::SaveState { track: 0, slot: 0 }).unwrap(),
            "save state: beat kept (Reload clip brings it back)"
        );
        assert_eq!(h.state().saved, [(0, 0)]);
        assert_eq!(h.act(reload()).unwrap(), "reload: beat is as saved already");
        // Mangled: another note.
        let mangle = |h: &mut TimeHost| {
            h.control.project.tracks[0].slots[0] = Some(one_bar(9));
            h.control.compile().unwrap();
        };
        mangle(&mut h);
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        assert_eq!(pads.hits.len(), 1);
        let mangled = pads.hits[0].1;
        assert_eq!(
            h.act(reload()).unwrap(),
            "reload: beat reloads at bar 2 (Reload clip again: taken back)"
        );
        let st = h.state();
        assert_eq!(st.reloads, [(0, 0, 3840)]);
        assert_eq!(pitch(&h), 9, "not yet");
        run(&mut h, &mut pads, 2 * BAR - 1000, &[]);
        assert_eq!(pads.hits.len(), 2, "{:?}", pads.hits);
        let (at, pad, _) = pads.hits[1];
        assert!(at.abs_diff(BAR) <= 128, "on the line: {at}");
        assert_ne!(pad, mangled, "the saved note");
        assert_eq!(
            h.take_later_note().unwrap(),
            "reload: beat back as saved on bar 2"
        );
        let st = h.state();
        assert!(st.reloads.is_empty());
        assert_eq!(pitch(&h), 4);
        assert_eq!(st.done.last().unwrap().short(), "reload 1");
        // UNDO: mangled again; stopped, a reload is at once.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(pitch(&h), 9);
        h.command(Command::Stop);
        run(&mut h, &mut pads, 2 * BAR, &[]);
        assert_eq!(
            h.act(reload()).unwrap(),
            "reload: beat is back as saved (UNDO takes it back)"
        );
        assert_eq!(pitch(&h), 4);
        // Taken back while it waits.
        mangle(&mut h);
        h.command(Command::Play);
        run(&mut h, &mut pads, 2 * BAR + BAR / 2, &[]);
        assert!(h.act(reload()).unwrap().contains("reloads at"));
        assert_eq!(h.act(reload()).unwrap(), "reload: beat's reload taken back");
        assert!(h.state().reloads.is_empty());
        // STOP lands what waits.
        h.act(reload()).unwrap();
        h.command(Command::Stop);
        run(&mut h, &mut pads, 3 * BAR, &[]);
        assert_eq!(
            h.take_later_note().unwrap(),
            "reload: beat back as saved (STOP)"
        );
        assert_eq!(pitch(&h), 4);
        // A deleted clip forgets its saved state.
        h.act(TimeAction::DeleteClip { track: 0, slot: 0 }).unwrap();
        assert!(h.state().saved.is_empty());
        assert_eq!(h.dropped(), 0);
    }

    /// Mutate and Restore (sequencer S6) on the clip card: a variation at
    /// once while the clip plays, each a take UNDO takes back; a second
    /// Mutate varies the original; Restore brings it back; the Takes list
    /// names them.
    #[test]
    fn mutate_varies_the_clip_and_restore_brings_it_back() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut c = Clip::new("hats", 3840);
        c.notes = (0..16)
            .map(|s| mpfree_seq::Note::new(s * 240, 120, 42, 90))
            .collect();
        let plain = c.clone();
        h.control.project.tracks[0].slots[0] = Some(c);
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        let mutate = |more| TimeAction::Mutate {
            track: 0,
            slot: 0,
            more,
            melodic: false,
        };
        let restore = || TimeAction::Restore { track: 0, slot: 0 };
        let clip = |h: &TimeHost| h.control.project.clip(0, 0).unwrap().clone();
        assert!(h
            .act(restore())
            .unwrap_err()
            .contains("hats plays its original: Mutate it first"));
        assert_eq!(
            h.act(mutate(false)).unwrap(),
            "mutate: hats · variant 1 (‹ on Mutate restores; UNDO takes it back)"
        );
        let v1 = clip(&h);
        assert_ne!(v1.notes, plain.notes);
        assert_eq!(v1.variation(), Some(1));
        assert_eq!(h.state().clip(0, 0).unwrap().variation, Some(1));
        // Edited since, then mutated again: from the original.
        h.control.project.tracks[0].slots[0]
            .as_mut()
            .unwrap()
            .notes
            .truncate(2);
        h.act(mutate(true)).unwrap();
        let v2 = clip(&h);
        assert_eq!((v2.notes.len(), v2.variation()), (16, Some(2)));
        assert_ne!(v2.notes, v1.notes, "a new variation");
        let st = h.state();
        let row = st.done.last().unwrap();
        assert_eq!(row.short(), "mutate 2");
        assert_eq!(
            h.act(restore()).unwrap(),
            "restore: hats is back as it was before Mutate (UNDO takes it back)"
        );
        assert_eq!(clip(&h), plain);
        assert_eq!(h.state().done.last().unwrap().short(), "restore 3");
        // UNDO: the variation again, its original kept.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(clip(&h), v2);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(clip(&h).variation(), Some(1));
        // It plays on, nothing dropped.
        run(&mut h, &mut pads, 2 * BAR, &[]);
        assert!(!pads.hits.is_empty());
        assert_eq!(h.dropped(), 0);
        // Not while recording.
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        run(&mut h, &mut pads, 2 * BAR + BAR / 4, &[]);
        assert_eq!(h.info().state, State::Recording);
        assert!(h
            .act(mutate(false))
            .unwrap_err()
            .contains("not while recording"));
    }

    /// Convert to notes (step sequencer refactor E1) on the clip card's
    /// Melody row: › previews what it would make (the hint line says it,
    /// the buttons are Cancel and Convert); Convert makes the clip of pad
    /// hits the notes it plays now, its lock on its pad's root and the
    /// original Restore brings back converted too, as one take UNDO takes
    /// back and REDO does again. The pads changed since the preview, a
    /// playing transport, any reason it can't and a kit's clip each change
    /// nothing; another action closes the preview.
    #[test]
    fn convert_to_notes_previews_then_converts_in_one_undo() {
        use mpfree_seq::{Holds, Lock, Note};
        use mpfree_time_ui::view::{card_buttons, clip_card_hint, clip_rows, CLIP_MELODY};
        // Pad 1 a chord (root last), the others one note each, none shared.
        let mut rows = PadRows::PADS;
        for p in 1..16 {
            rows.set(p, &[40 + p as u8]);
        }
        rows.set(0, &[64, 67, 60]);
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.set_notes_tracks(1);
        h.set_pad_rows(0, rows);
        let mut c = Clip::new("riff", 3840);
        for (tick, pad) in [(0, 0), (480, 2), (960, 5), (1440, 2)] {
            c.add_note(Note::new(tick, 120, pad, 100)).unwrap();
        }
        let strip = RelAddr {
            on: mpfree_play::mpfree_param::OnTrack::Strip,
            param: 1,
        };
        c.locks.push(Lock::new(480, Some(2), strip, 0.25));
        h.control.project.tracks[0].slots[0] = Some(c);
        let hits = h.project().clip(0, 0).unwrap().clone();
        let melody = |h: &TimeHost| clip_rows(&h.state())[CLIP_MELODY].value.clone();
        let names = |st: &TimeState| -> Vec<String> {
            card_buttons(st).into_iter().map(|b| b.label).collect()
        };
        assert_eq!(melody(&h), "pads");
        // Mutated first: it keeps the original Restore brings back.
        h.act(TimeAction::Mutate {
            track: 0,
            slot: 0,
            more: true,
            melodic: false,
        })
        .unwrap();
        let mutated = h.project().clip(0, 0).unwrap().clone();
        let preview = || TimeAction::ConvertPreview { track: 0, slot: 0 };
        let convert = || TimeAction::Convert { track: 0, slot: 0 };
        // The preview: nothing changes yet.
        let line = h.act(preview()).unwrap();
        assert!(
            line.starts_with(
                "Convert to notes: 4 pad hits become 6 notes. Keeps what riff plays now"
            ),
            "{line}"
        );
        let st = h.state();
        assert_eq!(names(&st), ["Cancel", "Convert"]);
        assert_eq!(clip_card_hint(&st), line, "the hint line says it");
        assert_eq!(h.project().clip(0, 0).unwrap(), &mutated);
        // The pads change: Convert refuses, and the preview closes.
        let mut moved = rows;
        moved.set(2, &[70]);
        h.set_pad_rows(0, moved);
        assert_eq!(
            h.act(convert()).unwrap_err(),
            "convert to notes: the pads changed since the preview: look again"
        );
        assert_eq!(h.project().clip(0, 0).unwrap(), &mutated);
        assert!(h.state().converting.is_none());
        h.set_pad_rows(0, rows);
        // Another action closes it too.
        h.act(preview()).unwrap();
        h.act(TimeAction::Arm(Some(0))).unwrap();
        assert!(h.state().converting.is_none());
        assert_eq!(names(&h.state())[0], "Save state");
        // While the transport rolls: STOP first, and the preview stays.
        h.act(preview()).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        assert_eq!(names(&h.state()), ["Cancel"]);
        assert_eq!(
            h.act(convert()).unwrap_err(),
            "convert to notes: press STOP first, then Convert"
        );
        h.command(Command::Stop);
        run(&mut h, &mut pads, BAR / 2, &[]);
        assert_eq!(names(&h.state()), ["Cancel", "Convert"]);
        // Convert: the notes the pads play, one take.
        let done = h.state().done.len();
        assert_eq!(
            h.act(convert()).unwrap(),
            "Converted: 4 pad hits are 6 notes (UNDO takes it back)"
        );
        let made = h.project().clip(0, 0).unwrap().clone();
        assert_eq!((made.holds, made.id), (Holds::Notes, mutated.id));
        assert_eq!(made.notes.len(), mutated.notes.len() + 2);
        assert!(made.notes.iter().all(|n| n.note >= 40), "pitches, not pads");
        assert_eq!(made.locks[0].sound, Some(42), "on pad 3's note");
        let st = h.state();
        assert_eq!(st.done.len(), done + 1);
        assert_eq!(st.done.last().unwrap().short(), "convert 2");
        assert!(st.converting.is_none());
        assert_eq!(melody(&h), "notes");
        // UNDO brings the clip that follows the pads back; REDO converts.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.project().clip(0, 0).unwrap(), &mutated);
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(h.project().clip(0, 0).unwrap(), &made);
        // Restore: the original, converted, never pad hits.
        h.act(TimeAction::Restore { track: 0, slot: 0 }).unwrap();
        let back = h.project().clip(0, 0).unwrap();
        assert_eq!(back.holds, Holds::Notes);
        let mut pitches: Vec<(Tick, u8)> = back.notes.iter().map(|n| (n.tick, n.note)).collect();
        pitches.sort();
        assert_eq!(
            pitches,
            [(0, 60), (0, 64), (0, 67), (480, 42), (960, 45), (1440, 42)]
        );
        assert_eq!(hits.notes.len(), 4);
        // A clip of notes, a kit's clip: no preview.
        assert!(h
            .act(preview())
            .unwrap_err()
            .contains("riff uses notes already"));
        let mut kit = Clip::new("beat", 3840);
        kit.add_note(Note::new(0, 120, 3, 100)).unwrap();
        h.control.project.tracks[1].slots[0] = Some(kit);
        assert!(h
            .act(TimeAction::ConvertPreview { track: 1, slot: 0 })
            .unwrap_err()
            .contains("beat is a kit's clip"));
        // Every reason it can't, at once; only Cancel; Convert refuses.
        let mut odd = hits.clone();
        // A chance alone converts now (trigger groups), but one pad hit
        // twice on a step at two chances can't: as notes they are one
        // trigger.
        odd.notes.push(Note {
            chance: 50,
            ..odd.notes[0]
        });
        odd.locks.push(Lock::new(
            960,
            Some(5),
            RelAddr {
                on: mpfree_play::mpfree_param::OnTrack::Pad(5),
                param: 2,
            },
            0.5,
        ));
        h.control.project.tracks[0].slots[1] = Some(odd.clone());
        let line = h
            .act(TimeAction::ConvertPreview { track: 0, slot: 1 })
            .unwrap();
        assert!(
            line.starts_with(
                "Convert to notes can't keep what riff plays yet: Pad A1 is hit twice on step 1 with a different chance or condition"
            ),
            "{line}"
        );
        assert!(line.contains("; Pad A6's Tune on step 5: "), "{line}");
        assert_eq!(
            h.act(TimeAction::Convert { track: 0, slot: 1 })
                .unwrap_err(),
            "convert to notes: the preview named what to fix first"
        );
        assert_eq!(h.project().clip(0, 1).unwrap(), &odd);
        assert_eq!(h.dropped(), 0);
    }

    /// Perform's Rec song with REC (sequencer S4): a take turns Rec song
    /// on for its length and what it played goes into the song when it
    /// ends; "+ Rec song" taken off skips one take; Rec song off during
    /// a take that turned it on is off for that take; in Make a take
    /// changes nothing.
    #[test]
    fn in_perform_a_take_brings_rec_song_for_its_length() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut c = Clip::new("beat", 3840);
        c.notes = vec![mpfree_seq::Note::new(0, 100, 4, 100)];
        h.control.project.tracks[0].slots[0] = Some(c);
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        // Make: a take is only a take.
        assert_eq!(h.take_follows(true), None);
        assert!(!h.state().song_rec && !h.state().song_with_take);
        assert_eq!(h.take_follows(false), None);
        h.act(TimeAction::Perform(true)).unwrap();
        assert!(h.state().song_with_take, "the REC chip says + Rec song");
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        assert_eq!(
            h.take_follows(true).as_deref(),
            Some("rec song: on with REC (Perform)")
        );
        assert!(h.state().song_rec && h.state().song_with_take);
        run(&mut h, &mut pads, 2 * BAR + BAR / 2, &[]);
        let line = h.take_follows(false).unwrap();
        assert!(
            line.starts_with("rec song: off with the take, ") && line.contains("placement"),
            "{line}"
        );
        let st = h.state();
        assert!(!st.song_rec && st.song_with_take, "ready for the next take");
        assert!(
            !st.placements.is_empty(),
            "the take's stretch is in the song"
        );
        // Taken off for the next take: it records no song.
        assert_eq!(
            h.act(TimeAction::RecSongWithTake(false)).unwrap(),
            "rec song: off for the next take"
        );
        assert!(!h.state().song_with_take);
        assert_eq!(h.take_follows(true), None);
        assert!(!h.state().song_rec);
        assert_eq!(h.take_follows(false), None);
        assert!(h.state().song_with_take, "back with the take after it");
        // Off by hand during a take that turned it on: off for that take.
        h.take_follows(true).unwrap();
        h.act(TimeAction::RecSong(false)).unwrap();
        assert!(!h.state().song_rec && !h.state().song_with_take);
        run(&mut h, &mut pads, 3 * BAR, &[]);
        assert_eq!(h.take_follows(false), None);
        // "+ Rec song" put back during a take turns it on for the rest.
        h.act(TimeAction::RecSongWithTake(false)).unwrap();
        h.take_follows(true);
        assert_eq!(
            h.act(TimeAction::RecSongWithTake(true)).unwrap(),
            "rec song: on with this take"
        );
        assert!(h.state().song_rec);
        // Rec song on by hand stays on after a take.
        h.take_follows(false).unwrap();
        h.act(TimeAction::RecSong(true)).unwrap();
        assert_eq!(h.take_follows(true), None);
        assert_eq!(h.take_follows(false), None);
        assert!(h.state().song_rec);
        // In Make "+ Rec song" is refused.
        h.act(TimeAction::Perform(false)).unwrap();
        assert!(h.act(TimeAction::RecSongWithTake(true)).is_err());
        assert_eq!(h.dropped(), 0);
    }

    /// Rec song keeps the morph bar's moves made on the mixer's mod engine:
    /// the crossfader's positions, and the rows they morph between, go into
    /// the song's morph lane when the transport stops.
    #[test]
    fn rec_song_keeps_the_morph_bar_moves_made_on_the_mixer() {
        use mpfree_seq::arrange::MORPH_LANE;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut mix = crate::mixing::MixHost::new(RATE, 0.0, 6.0);
        let mut pads = Fake::default();
        let mut c = Clip::new("beat", 3840);
        c.notes = vec![mpfree_seq::Note::new(0, 100, 4, 100)];
        h.control.project.tracks[0].slots[0] = Some(c);
        h.act(TimeAction::LaunchScene(0)).unwrap();
        h.act(TimeAction::RecSong(true)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.morph(TimeAction::Morph(0.5), &mut mix.modu, &mix.mixer)
            .unwrap();
        run(&mut h, &mut pads, BAR + BAR / 2, &[]);
        h.morph(
            TimeAction::MorphRows { a: 0, b: 2 },
            &mut mix.modu,
            &mix.mixer,
        )
        .unwrap();
        h.morph(TimeAction::Morph(1.0), &mut mix.modu, &mix.mixer)
            .unwrap();
        run(&mut h, &mut pads, 2 * BAR - 1000, &[]);
        h.command(Command::Stop);
        run(&mut h, &mut pads, 2 * BAR, &[]);
        let note = h.take_later_note().unwrap();
        assert!(note.starts_with("rec song: 1 placements and "), "{note}");
        let song = &h.control.project.arrangement;
        let rows: Vec<(usize, usize)> = song.morphs.iter().map(|m| (m.a, m.b)).collect();
        assert_eq!(rows, [(0, 1), (0, 2)]);
        // Held half way from the first move, all the way after the second.
        let lane = song.lanes.lane(MORPH_LANE).unwrap();
        let at = |frames: u64| frames as f64 * 960.0 * 2.0 / f64::from(RATE);
        assert!((lane.value_at(at(BAR)).unwrap() - 0.5).abs() < 1e-3);
        assert!((lane.value_at(at(2 * BAR - 1500)).unwrap() - 1.0).abs() < 1e-3);
    }

    /// Rec song keeps the XY pad's moves that go into the song (XY phase
    /// 3, `xy_rec.rs`): a setting's gesture goes into the song's lane for
    /// it when the transport stops, as one song edit UNDO takes back.
    #[test]
    fn rec_song_keeps_the_xy_pads_moves() {
        use mpfree_play::mpfree_param::{Owner, ParamAddr, Strip};
        use mpfree_seq::mpfree_moves::{Map, Target};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut c = Clip::new("beat", 3840);
        c.notes = vec![mpfree_seq::Note::new(0, 100, 4, 100)];
        h.control.project.tracks[0].slots[0] = Some(c);
        h.act(TimeAction::LaunchScene(0)).unwrap();
        h.act(TimeAction::RecSong(true)).unwrap();
        assert!(h.rec_song());
        h.command(Command::Play);
        let wet = ParamAddr::new(Owner::Insert(Strip::Master, 1), 0);
        let map = Map::linear(0.0, 1.0);
        let mv = |value: f32| Input::Move {
            addr: wet,
            value,
            map,
        };
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.song_gesture(0, mv(0.25));
        run(&mut h, &mut pads, BAR, &[]);
        h.song_gesture(0, mv(0.75));
        run(&mut h, &mut pads, BAR + BAR / 2, &[]);
        h.song_gesture(0, Input::End { addr: wet });
        run(&mut h, &mut pads, 2 * BAR - 1000, &[]);
        h.command(Command::Stop);
        run(&mut h, &mut pads, 2 * BAR, &[]);
        let note = h.take_later_note().unwrap();
        assert!(note.contains(", 1 XY pad move"), "{note}");
        let lane = |h: &TimeHost| {
            h.control
                .project
                .arrangement
                .lanes
                .lane(Target::Fixed(wet))
                .cloned()
        };
        let l = lane(&h).expect("the song's lane for it");
        let at = |frames: u64| frames as f64 * 960.0 * 2.0 / f64::from(RATE);
        assert_eq!(l.value_at(at(BAR / 2 - 2000)), None, "free before the move");
        assert!((l.value_at(at(BAR / 2 + 300)).unwrap() - 0.25).abs() < 0.02);
        assert!((l.value_at(at(BAR + 2000)).unwrap() - 0.75).abs() < 0.02);
        assert_eq!(
            l.value_at(at(BAR + BAR / 2 + 2000)),
            None,
            "free after the lift"
        );
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(lane(&h), None, "UNDO takes it back with the song edit");
    }

    /// SHIFT + TC (the MPC's TC On/Off): the armed track's Quantise while
    /// recording turns on at 1/16, off, and back on at the grid it had.
    #[test]
    fn shift_tc_turns_the_armed_tracks_quantise_off_and_back_on() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let line = h.toggle_record_quantise().unwrap();
        assert!(line.starts_with("Quantise on: Track 1"), "{line}");
        assert_eq!(h.state().tracks[0].record_quantise, Some(QUANTISE_ON));
        h.act(TimeAction::RecordQuantise {
            track: 0,
            grid: Some(120),
        })
        .unwrap();
        let line = h.toggle_record_quantise().unwrap();
        assert!(line.starts_with("Quantise off: Track 1"), "{line}");
        assert_eq!(h.state().tracks[0].record_quantise, None);
        h.toggle_record_quantise().unwrap();
        assert_eq!(
            h.state().tracks[0].record_quantise,
            Some(120),
            "the grid it had"
        );
    }

    /// SHIFT + PLAY: before PLAY has started anything it plays from where
    /// it is; then it plays again from where PLAY last started, and while
    /// playing jumps back there. SHIFT + OVERDUB: the count-in off, and
    /// back to the bars it had (1 the first time).
    #[test]
    fn shift_play_goes_back_to_where_play_started_and_shift_overdub_toggles_the_count_in() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let run = |h: &mut TimeHost, pads: &mut Fake, frames: u64| {
            let to = pads.frame + frames;
            while pads.frame < to {
                h.period(PERIOD, pads);
                pads.frame += u64::from(PERIOD);
            }
        };
        let bar = 4 * mpfree_transport::PPQ as i64;
        let line = h.play_back().unwrap();
        assert!(line.contains("playing from here"), "{line}");
        run(&mut h, &mut pads, 4096);
        assert_eq!(h.info().state, State::Playing);
        assert!(h.command(Command::StopAndRewind));
        run(&mut h, &mut pads, 4096);
        // PLAY from bar 3, on into bar 4.
        assert!(h.command(Command::Locate(2 * bar)));
        run(&mut h, &mut pads, 4096);
        h.act(TimeAction::Transport(Command::Play)).unwrap();
        run(&mut h, &mut pads, 100_000);
        assert!(h.info().position > 3 * bar);
        assert_eq!(
            h.play_back().unwrap(),
            "SHIFT + PLAY: back to bar 3, where PLAY started"
        );
        run(&mut h, &mut pads, 4096);
        let at = h.info().position;
        assert!((2 * bar..2 * bar + 960).contains(&at), "{at}");
        // Stopped: from bar 3 again, playing.
        assert!(h.command(Command::Stop));
        run(&mut h, &mut pads, 50_000);
        assert_eq!(h.info().state, State::Stopped);
        assert_eq!(
            h.play_back().unwrap(),
            "SHIFT + PLAY: playing from bar 3, where PLAY last started"
        );
        run(&mut h, &mut pads, 4096);
        assert_eq!(h.info().state, State::Playing);
        assert!((2 * bar..2 * bar + 960).contains(&h.info().position));

        // The count-in.
        assert_eq!(h.info().count_in, 0);
        assert_eq!(
            h.toggle_count_in().unwrap(),
            "Count-in on: 1 bar before recording   ·   SHIFT + OVERDUB: off"
        );
        run(&mut h, &mut pads, 4096);
        assert_eq!(h.info().count_in, 1);
        assert!(h.command(Command::SetCountIn(2)));
        run(&mut h, &mut pads, 4096);
        assert!(h.toggle_count_in().unwrap().starts_with("Count-in off"));
        run(&mut h, &mut pads, 4096);
        assert_eq!(h.info().count_in, 0);
        assert!(h
            .toggle_count_in()
            .unwrap()
            .starts_with("Count-in on: 2 bars"));
        run(&mut h, &mut pads, 4096);
        assert_eq!(h.info().count_in, 2, "the bars it had");
    }

    /// The note editor on the focused clip: Edit opens it; Timing Correct
    /// with nothing selected moves every note to the 1/16 grid; a tap
    /// selects a note and Delete takes it out; each is one take that Undo
    /// puts back; ‹ Steps closes it.
    #[test]
    fn the_note_editor_edits_the_focused_clip_through_the_take_history() {
        use mpfree_seq::Note;
        use mpfree_time_ui::editor::EditAction as E;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        assert_eq!(
            h.act(TimeAction::Edit(E::Open)).unwrap_err(),
            "edit: Track 1 has no clip"
        );
        let mut c = Clip::new("Beat", 3840);
        c.notes = vec![
            Note::new(10, 100, 0, 100),
            Note::new(970, 100, 1, 90),
            Note::new(1930, 100, 0, 110),
        ];
        h.control.project.tracks[0].slots[0] = Some(c.clone());
        assert_eq!(
            h.act(TimeAction::Edit(E::Open)).unwrap(),
            "edit: Beat on Track 1"
        );
        assert!(h.state().edit.is_some());
        // The clip's notes in time order (an undo may put them back in
        // another order, which plays the same).
        let sorted = |h: &TimeHost| -> Vec<Note> {
            let mut n = h.control.project.clip(0, 0).unwrap().notes.clone();
            n.sort_by_key(|n| (n.tick, n.note));
            n
        };
        let ticks = |h: &TimeHost| -> Vec<Tick> { sorted(h).iter().map(|n| n.tick).collect() };
        assert_eq!(
            h.act(TimeAction::Edit(E::Correct)).unwrap(),
            "edit: timing correct moved 3 notes"
        );
        assert_eq!(ticks(&h), [0, 960, 1920]);
        let rev = h.state().edit.unwrap().rev;
        h.act(TimeAction::Edit(E::Note { index: 1, rev })).unwrap();
        assert_eq!(
            h.act(TimeAction::Edit(E::Button(6))).unwrap(),
            "edit: 1 notes deleted"
        );
        assert_eq!(ticks(&h), [0, 1920]);
        // Each edit is a take of its own.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(ticks(&h), [0, 960, 1920]);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(sorted(&h), c.notes);
        // ERASE + a drag (piano roll plan 1B.5): the notes it crossed in
        // one take, which one UNDO brings back; on an old picture,
        // refused with nothing erased.
        h.take_changed();
        let rev = h.state().edit.unwrap().rev;
        assert_eq!(
            h.act(TimeAction::Edit(E::Sweep {
                notes: vec![0, 2],
                rev
            }))
            .unwrap(),
            "edit: 2 notes erased"
        );
        assert_eq!(sorted(&h).len(), 1);
        let said = h
            .act(TimeAction::Edit(E::Sweep {
                notes: vec![0],
                rev,
            }))
            .unwrap_err();
        assert!(
            said.contains("the clip changed under your finger"),
            "{said}"
        );
        assert_eq!(sorted(&h).len(), 1);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(sorted(&h), c.notes);
        // The clip's end handle (piano roll plan 1A.2): the Steps' length
        // edit, one take; no shorter than just after the last note start.
        let length = |h: &TimeHost| h.control.project.clip(0, 0).unwrap().length;
        assert_eq!(
            h.act(TimeAction::Edit(E::Length { to: 7680 })).unwrap(),
            "edit: the clip is 32 steps long"
        );
        assert_eq!(length(&h), 7680);
        assert_eq!(
            h.act(TimeAction::Edit(E::Length { to: 960 })).unwrap(),
            "edit: a note or lock starts on step 9: the clip can't end before it"
        );
        assert_eq!(length(&h), 7680);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(length(&h), 3840);
        assert_eq!(sorted(&h), c.notes);
        // A finger on a note and the wheel (piano roll plan 1C.3): a
        // hold's detents are one take, its later ones naming no picture;
        // a hold's first on an old picture is refused.
        h.take_changed();
        let e = h.state().edit.unwrap();
        let (rev, at) = (e.rev, e.clip.notes.iter().position(|n| n.tick == 970));
        let held = |index, ticks, turn, first| {
            TimeAction::Edit(E::Held {
                index,
                rev,
                ticks,
                fine: false,
                turn,
                first,
            })
        };
        let vels = |h: &TimeHost| -> Vec<u8> { sorted(h).iter().map(|n| n.velocity).collect() };
        h.act(held(at.unwrap(), 5, 1, true)).unwrap();
        h.act(held(mpfree_time_ui::editor::HELD_PICKS, 3, 1, false))
            .unwrap();
        assert_eq!(vels(&h), [100, 98, 110]);
        let said = h.act(held(0, 5, 2, true)).unwrap_err();
        assert!(
            said.contains("the clip changed under your finger"),
            "{said}"
        );
        assert_eq!(vels(&h), [100, 98, 110]);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(sorted(&h), c.notes, "one UNDO, the whole hold");
        assert_eq!(
            h.act(TimeAction::Edit(E::Button(0))).unwrap(),
            "edit: closed"
        );
        assert!(h.state().edit.is_none());
    }

    /// A tap on a note carries the clip's revision: after something else
    /// changed the clip (here a step tapped on Steps), a tap made on the
    /// old picture is refused and erases nothing; the picks follow their
    /// notes to where they now are, and when a picked note goes they are
    /// cleared and the legend says so (piano roll plan 0A.1).
    #[test]
    fn the_note_editor_refuses_a_tap_on_an_old_picture_and_its_picks_follow() {
        use mpfree_time_ui::editor::EditAction as E;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let tap = |step| StepAction::Tap {
            pad: 0,
            step,
            velocity: 100,
        };
        // Three kicks on steps 1, 5 and 9, then the editor on them.
        for step in [0, 4, 8] {
            h.step_action(tap(step)).unwrap();
        }
        h.act(TimeAction::Edit(E::Open)).unwrap();
        let notes = |h: &TimeHost| h.control.project.clip(0, 0).unwrap().notes.clone();
        let info = |h: &mut TimeHost| {
            h.take_changed();
            *h.state().edit.unwrap()
        };
        let old = info(&mut h);
        assert_eq!(old.clip.notes.len(), 3);
        // Step 1 taken off on Steps: the notes move up.
        let mid = info(&mut h);
        let gone = notes(&h)[0];
        h.step_action(tap(0)).unwrap();
        assert!(!notes(&h).contains(&gone));
        // ERASE + a tap made on the picture before: refused, nothing
        // erased.
        let before = notes(&h);
        let said = h
            .act(TimeAction::Edit(E::Erase {
                index: 0,
                rev: mid.rev,
            }))
            .unwrap_err();
        assert!(
            said.contains("the clip changed under your finger"),
            "{said}"
        );
        assert_eq!(notes(&h), before);
        // The new picture's erase takes the note it showed.
        let now = info(&mut h);
        assert!(now.rev > mid.rev);
        let hit = now.clip.notes[1];
        h.act(TimeAction::Edit(E::Erase {
            index: 1,
            rev: now.rev,
        }))
        .unwrap();
        assert!(!notes(&h).contains(&hit));
        assert_eq!(notes(&h).len(), 1);
        // The same for a move and a tap on an old picture (piano roll
        // plan 1B.1: every action naming a note carries its picture).
        for a in [
            E::Tap {
                index: 0,
                rev: mid.rev,
            },
            E::Drop {
                index: 0,
                rev: mid.rev,
                ticks: 240,
                notes: 0,
                scale: false,
            },
            // Plan 1B.2: a handle and a note tapped while one is held.
            E::Resize {
                index: 0,
                rev: mid.rev,
                start: false,
                ticks: 240,
            },
            E::Add {
                index: 0,
                rev: mid.rev,
            },
        ] {
            assert!(h.act(TimeAction::Edit(a)).is_err());
            assert_eq!(notes(&h).len(), 1);
        }
        // Picks follow: two kicks back, the last one picked...
        for step in [0, 12] {
            h.step_action(tap(step)).unwrap();
        }
        let st = info(&mut h);
        assert_eq!(st.editor.tool, mpfree_time_ui::editor::Tool::Select);
        let last = st
            .clip
            .notes
            .iter()
            .position(|n| n.tick == 12 * 240)
            .unwrap();
        let picked = st.clip.notes[last];
        h.act(TimeAction::Edit(E::Note {
            index: last,
            rev: st.rev,
        }))
        .unwrap();
        // ...then the first note taken off on Steps: the pick follows its
        // note to its new place.
        h.step_action(tap(0)).unwrap();
        let st = info(&mut h);
        assert!(!st.editor.cleared);
        assert_eq!(st.editor.sel.len(), 1);
        assert_eq!(st.clip.notes[st.editor.sel[0]], picked);
        // The picked note itself taken off: the picks are cleared, and the
        // legend says why.
        h.step_action(tap(12)).unwrap();
        let st = info(&mut h);
        assert!(st.editor.sel.is_empty());
        assert!(st.editor.cleared);
        let grid = mpfree_time_ui::view::note_grid(&st, &h.state());
        assert!(
            grid.legend.contains("the clip changed: picks cleared"),
            "{}",
            grid.legend
        );
    }

    /// Piano roll plan 0C.1: an editor edit on the playing clip goes into
    /// the view in place, as a step edit does (no compile, the view just
    /// what a compile gives, timed for the status line); a stroke along
    /// the velocity lane, sent once as the finger lifts, is one take; and the
    /// screens get one shared copy of the clip, the same while it is
    /// unchanged, a new one when it changed.
    #[test]
    fn the_note_editor_edits_the_playing_clip_in_place_and_shares_one_copy() {
        use mpfree_time_ui::editor::EditAction as E;
        use std::sync::Arc;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let tap = |step| StepAction::Tap {
            pad: 0,
            step,
            velocity: 100,
        };
        for step in [0, 4, 8] {
            h.step_action(tap(step)).unwrap();
        }
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.act(TimeAction::Edit(E::Open)).unwrap();
        let info = |h: &mut TimeHost| {
            h.take_changed();
            *h.state().edit.unwrap()
        };
        let a = info(&mut h);
        assert!(Arc::ptr_eq(&a.clip, &info(&mut h).clip), "the same copy");
        let compiles = h.control.costs.compiles;
        h.act(TimeAction::Edit(E::Button(4))).unwrap();
        let takes = h.state().done.len();
        for v in [64, 127] {
            // A stroke along the velocity lane over the picked notes.
            let e = info(&mut h);
            let set = e.editor.sel.iter().map(|&i| (i, v)).collect();
            let lane = E::Lane {
                field: mpfree_seq::notes::Field::Velocity,
                rev: e.rev,
                set,
            };
            h.act(TimeAction::Edit(lane)).unwrap();
            h.audio.receive();
            assert_eq!(
                *h.audio.view,
                h.control.view().unwrap(),
                "in place, as a compile gives"
            );
        }
        assert_eq!(h.state().done.len(), takes + 2, "a take a stroke");
        let c = h.control.costs;
        assert_eq!(
            (c.compiles, c.notes_in_place, c.notes_compiled),
            (compiles, 2, 0)
        );
        assert!(
            h.describe().contains("; note edits: 2 in place (slowest "),
            "{}",
            h.describe()
        );
        let b = info(&mut h);
        assert!(!Arc::ptr_eq(&a.clip, &b.clip), "changed: a new copy");
        assert!(b.clip.notes.iter().all(|n| n.velocity == 127));
        // A change from Steps: a new copy, then the same one again.
        h.step_action(tap(12)).unwrap();
        let d = info(&mut h);
        assert!(!Arc::ptr_eq(&b.clip, &d.clip));
        assert_eq!(d.clip.notes.len(), 4);
        assert!(Arc::ptr_eq(&d.clip, &info(&mut h).clip));
    }

    /// Piano roll plan 2B.1: the clip's scale is set from the note editor
    /// as a clip setting, a take each (UNDO takes it back, no note moves),
    /// and saved with the clip; the editor offers the pads' scale it was
    /// given; a drag with the scale's flag moves by its notes.
    #[test]
    fn the_note_editor_sets_the_clips_scale_and_undo_takes_it_back() {
        use mpfree_seq::mpfree_scale::{DORIAN, MAJOR};
        use mpfree_seq::Scale;
        use mpfree_time_ui::editor::EditAction as E;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 0,
            velocity: 100,
        })
        .unwrap();
        // A clip of pad hits takes no scale (its numbers are pads).
        h.act(TimeAction::Edit(E::Open)).unwrap();
        let line = h
            .act(TimeAction::Edit(E::Scale(Some(Scale::new(0, MAJOR)))))
            .unwrap();
        assert_eq!(line, "edit: a clip of pads has no scale");
        // A clip of notes, which a synth's track plays, does.
        h.control.project.tracks[0].slots[0].as_mut().unwrap().holds = mpfree_seq::Holds::Notes;
        h.act(TimeAction::Edit(E::Open)).unwrap();
        for (tick, note) in [(960, 60), (1920, 64)] {
            h.act(TimeAction::Edit(E::Draw {
                tick,
                note,
                len: 240,
            }))
            .unwrap();
        }
        h.set_scale_offers(Some(Scale::new(2, DORIAN)), &[("Mine".into(), 0b11, 0b11)]);
        h.take_changed();
        let edit = |h: &mut TimeHost| {
            h.take_changed();
            h.state().edit.unwrap()
        };
        assert_eq!(edit(&mut h).editor.offers.pads, Some(Scale::new(2, DORIAN)));
        assert_eq!(edit(&mut h).editor.offers.own.len(), 1);
        let line = h
            .act(TimeAction::Edit(E::Scale(Some(Scale::new(0, MAJOR)))))
            .unwrap();
        assert_eq!(line, "edit: scale C Major (UNDO takes it back)");
        h.act(TimeAction::Edit(E::Root(2))).unwrap();
        let e = edit(&mut h);
        assert_eq!(e.clip.scale, Some(Scale::new(2, MAJOR)));
        let mut notes: Vec<u8> = e.clip.notes.iter().map(|n| n.note).collect();
        notes.sort_unstable();
        assert_eq!(notes, [0, 60, 64], "no note moved");
        let saved = mpfree_seq::save::to_text(h.project());
        assert!(saved.contains("\nscale D 0 2 4 5 7 9 11\n"), "{saved}");
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(edit(&mut h).clip.scale, Some(Scale::new(0, MAJOR)));
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(edit(&mut h).clip.scale, None);
        h.act(TimeAction::Redo).unwrap();
        // C major: the C a scale step up is D.
        let e = edit(&mut h);
        let c = e.clip.notes.iter().position(|n| n.note == 60).unwrap();
        h.act(TimeAction::Edit(E::Drop {
            index: c,
            rev: e.rev,
            ticks: 0,
            notes: 1,
            scale: true,
        }))
        .unwrap();
        let mut notes: Vec<u8> = edit(&mut h).clip.notes.iter().map(|n| n.note).collect();
        notes.sort_unstable();
        assert_eq!(notes, [0, 62, 64]);
    }

    /// Piano roll plans 2B.2 and 2B.3: with the Chord chip on, a note
    /// drawn puts in the chord, picked, and one UNDO takes the whole chord
    /// back; the inspector names the picked chord. Strum opened on it
    /// turns on the screen's preview alone (the clip and its revision
    /// stay), and Done is one take.
    #[test]
    fn the_note_editor_draws_chords_and_a_tools_done_is_one_take() {
        use mpfree_time_ui::editor::{
            ChordShape, EditAction as E, ToolAct, ToolKind, STRUM_ROW, STRUM_TICKS,
        };
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        // A track that plays notes: its clip holds notes (plan 4.2).
        h.set_notes_tracks(1);
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 0,
            velocity: 100,
        })
        .unwrap();
        h.act(TimeAction::Edit(E::Open)).unwrap();
        h.act(TimeAction::Edit(E::Draw {
            tick: 960,
            note: 48,
            len: 240,
        }))
        .unwrap();
        let line = h
            .act(TimeAction::Edit(E::Chord {
                shape: ChordShape::Triad,
                minor: false,
            }))
            .unwrap();
        assert!(
            line.starts_with("edit: drawing chords: Major triad"),
            "{line}"
        );
        let edit = |h: &mut TimeHost| {
            h.take_changed();
            h.state().edit.unwrap()
        };
        let line = h
            .act(TimeAction::Edit(E::Draw {
                tick: 1920,
                note: 62,
                len: 960,
            }))
            .unwrap();
        assert!(line.starts_with("edit: chord D drawn"), "{line}");
        let e = edit(&mut h);
        let mut picked: Vec<u8> = e.editor.sel.iter().map(|&i| e.clip.notes[i].note).collect();
        picked.sort_unstable();
        assert_eq!(picked, [62, 66, 69], "D F# A, picked");
        assert_eq!(
            mpfree_time_ui::view::note_cards(&e)
                .map(|(head, _)| head)
                .as_deref(),
            Some("3 notes · D")
        );
        // Strum opens on the picks; two turns of Q1 change its preview,
        // not the clip.
        h.act(TimeAction::Edit(E::Row {
            row: STRUM_ROW,
            delta: 1,
        }))
        .unwrap();
        let rev = edit(&mut h).rev;
        for turn in 1..=2 {
            h.act(TimeAction::Edit(E::Knob {
                k: 0,
                ticks: 2,
                fine: false,
                turn,
            }))
            .unwrap();
        }
        let e = edit(&mut h);
        let at = |e: &mpfree_time_ui::view::EditInfo, n: u8| {
            e.clip.notes.iter().find(|x| x.note == n).map(|x| x.tick)
        };
        assert_eq!(e.editor.open_tool.map(|t| t.kind), Some(ToolKind::Strum));
        assert_eq!((e.rev, at(&e, 69)), (rev, Some(1920)), "nothing went in");
        let (p, _) = e.editor.preview(&e.clip).unwrap();
        let p69 = p.notes.iter().find(|x| x.note == 69).map(|x| x.tick);
        assert_eq!(p69, Some(1920 + 8 * STRUM_TICKS));
        // Done: one take.
        let line = h.act(TimeAction::Edit(E::Tool(ToolAct::Done))).unwrap();
        assert_eq!(line, "edit: 1 chord strummed");
        let e = edit(&mut h);
        assert_eq!(e.editor.open_tool, None);
        assert_eq!(at(&e, 69), Some(1920 + 8 * STRUM_TICKS));
        h.act(TimeAction::Undo).unwrap();
        let e = edit(&mut h);
        assert_eq!(
            (at(&e, 62), at(&e, 66), at(&e, 69)),
            (Some(1920), Some(1920), Some(1920))
        );
        // One UNDO takes the whole chord back.
        h.act(TimeAction::Undo).unwrap();
        let e = edit(&mut h);
        assert_eq!((at(&e, 62), at(&e, 66), at(&e, 69)), (None, None, None));
        assert_eq!(at(&e, 48), Some(960));
    }

    /// Piano roll plan 3A.1: pressure drawn on the Pressure lane is one
    /// take a stroke, and UNDO brings the old pressure back. Once the
    /// clip's pressure is past half its room, the next period tidies it
    /// (refactor B4b): the picks stay, and UNDO and REDO still find each
    /// stroke's pressure.
    /// The note editor's lock lanes (piano roll plan 3A.2): a stroke
    /// sets the lock where each note plays, as the Steps' lock lanes do
    /// (notes in one step share a whole-step lock), one take that UNDO
    /// takes back; ERASE + a tap takes it off; the picks stay.
    #[test]
    fn locks_set_on_notes_are_the_steps_locks_and_one_take() {
        use mpfree_play::mpfree_param::{OnTrack, RelAddr};
        use mpfree_time_ui::editor::EditAction as E;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 0,
            velocity: 100,
        })
        .unwrap();
        h.act(TimeAction::Edit(E::Open)).unwrap();
        // A second note on step 1, and one on step 5.
        for (tick, note) in [(0, 7), (960, 0)] {
            h.act(TimeAction::Edit(E::Draw {
                tick,
                note,
                len: 120,
            }))
            .unwrap();
        }
        let edit = |h: &mut TimeHost| {
            h.take_changed();
            h.state().edit.unwrap()
        };
        let cutoff = RelAddr::new(OnTrack::Inst, 3);
        let locks = |h: &mut TimeHost| -> Vec<(i64, Option<u8>, f32)> {
            let mut v: Vec<_> = edit(h)
                .clip
                .locks
                .iter()
                .filter(|l| l.addr == cutoff)
                .map(|l| (l.tick, l.sound, l.value))
                .collect();
            v.sort_by_key(|l| l.0);
            v
        };
        let e = edit(&mut h);
        let at = |t: i64| e.clip.notes.iter().position(|n| n.tick == t).unwrap();
        let (first, later) = (at(0), at(960));
        let picks = e.editor.sel.clone();
        let line = h
            .act(TimeAction::Edit(E::Lock {
                rev: e.rev,
                addr: cutoff,
                set: vec![(first, 800.0), (later, 1200.0)],
            }))
            .unwrap();
        assert_eq!(line, "edit: 2 locks set");
        assert_eq!(
            locks(&mut h),
            [(0, None, 800.0), (960, None, 1200.0)],
            "a whole-step lock each"
        );
        assert_eq!(edit(&mut h).editor.sel, picks, "the picks stay");
        // ERASE + a tap on step 5's bar.
        let e = edit(&mut h);
        let line = h
            .act(TimeAction::Edit(E::Unlock {
                rev: e.rev,
                addr: cutoff,
                notes: vec![later],
            }))
            .unwrap();
        assert_eq!(line, "edit: lock taken off");
        assert_eq!(locks(&mut h), [(0, None, 800.0)]);
        // UNDO, UNDO: each a take.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(locks(&mut h).len(), 2);
        h.act(TimeAction::Undo).unwrap();
        assert!(locks(&mut h).is_empty());
        // A stale picture: refused.
        let rev = edit(&mut h).rev;
        assert!(h
            .act(TimeAction::Edit(E::Lock {
                rev: rev - 1,
                addr: cutoff,
                set: vec![(first, 1.0)],
            }))
            .is_err());
    }

    /// Pads write (piano roll plan 1C.2) through the time engine: two
    /// pads held together write one chord at the cursor, one UNDO takes
    /// it back, and the cursor steps once, when the last lets go; the
    /// switch is off whenever the editor opens.
    #[test]
    fn a_chord_on_the_pads_is_one_cursor_step_and_one_undo() {
        use mpfree_time_ui::editor::EditAction as E;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 0,
            velocity: 100,
        })
        .unwrap();
        h.act(TimeAction::Edit(E::Open)).unwrap();
        assert!(!h.pads_write(), "off as it opens");
        let pad = |key: u8, down: bool| {
            TimeAction::Edit(E::Pad {
                key,
                hit: key + 4,
                notes: Vec::new(),
                velocity: 100,
                down,
            })
        };
        let edit = |h: &mut TimeHost| {
            h.take_changed();
            h.state().edit.unwrap()
        };
        // Off: a pad picks its notes and writes nothing.
        h.act(pad(0, true)).unwrap();
        h.act(pad(0, false)).unwrap();
        assert_eq!(edit(&mut h).clip.notes.len(), 1);
        h.act(TimeAction::Edit(E::PadsWrite)).unwrap();
        assert!(h.pads_write());
        // Step 2 of the clip: the cursor a 1/16 on.
        h.act(TimeAction::Edit(E::Cursor(1))).unwrap();
        let at = edit(&mut h).editor.cursor;
        h.act(pad(0, true)).unwrap();
        h.act(pad(1, true)).unwrap();
        h.act(pad(0, false)).unwrap();
        assert_eq!(edit(&mut h).editor.cursor, at, "a pad still held");
        h.act(pad(1, false)).unwrap();
        let e = edit(&mut h);
        let mut written: Vec<(i64, u8)> = e
            .clip
            .notes
            .iter()
            .filter(|n| n.tick == at)
            .map(|n| (n.tick, n.note))
            .collect();
        written.sort_unstable();
        assert_eq!(written, [(at, 4), (at, 5)]);
        assert!(e.editor.cursor > at, "the cursor stepped once");
        // One UNDO takes the whole chord back.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(edit(&mut h).clip.notes.len(), 1);
        // Closed and opened again: off.
        h.act(TimeAction::Edit(E::Button(0))).unwrap();
        h.act(TimeAction::Edit(E::Open)).unwrap();
        assert!(!h.pads_write());
    }

    /// Show (piano roll plan 4.1): the editor's other clips go to the
    /// screen with their notes, one copy while unchanged; a faint note's
    /// tap opens its clip, selects its track, and leaves the clip it left
    /// showing in its place.
    #[test]
    fn show_sends_other_clips_and_a_faint_notes_tap_opens_its_clip() {
        use mpfree_time_ui::editor::EditAction as E;
        let mut h = host_with_a_clip();
        h.act(TimeAction::AddTrack).unwrap();
        let mut line = Clip::new("line", 3840);
        line.notes = vec![mpfree_seq::Note::new(0, 480, 1, 90)];
        h.control.project.tracks[1].slots[0] = Some(line);
        h.control.project.give_clip_ids();
        h.act(TimeAction::Arm(Some(0))).unwrap();
        h.act(TimeAction::Edit(E::Open)).unwrap();
        let edit = |h: &mut TimeHost| {
            h.take_changed();
            h.state().edit.unwrap()
        };
        let e = edit(&mut h);
        assert_eq!((e.editor.track, e.editor.slot), (0, 0));
        assert!(e.ghosts.is_empty());
        h.act(TimeAction::Edit(E::Show { track: 1, slot: 0 }))
            .unwrap();
        let e = edit(&mut h);
        assert_eq!(e.editor.show, [(1, 0)]);
        assert_eq!(e.ghosts.len(), 1);
        assert_eq!(
            (
                e.ghosts[0].clip.name.as_str(),
                e.ghosts[0].track_name.as_str()
            ),
            ("line", h.state().tracks[1].name.as_str())
        );
        // Unchanged: the same copy.
        h.act(TimeAction::Edit(E::Cursor(1))).unwrap();
        let again = edit(&mut h);
        assert!(std::sync::Arc::ptr_eq(
            &e.ghosts[0].clip,
            &again.ghosts[0].clip
        ));
        // The faint note's tap: that clip opens, its track selected, the
        // beat behind it.
        h.act(TimeAction::Edit(E::Switch { track: 1, slot: 0 }))
            .unwrap();
        let e = edit(&mut h);
        assert_eq!((e.editor.track, e.editor.slot), (1, 0));
        assert_eq!(e.editor.show, [(0, 0)]);
        assert_eq!(e.ghosts[0].clip.name, "beat");
        assert_eq!(h.state().track_pick, Some(1));
        // A clip that's gone shows no more, and can't be opened.
        h.act(TimeAction::DeleteClip { track: 0, slot: 0 }).unwrap();
        assert!(edit(&mut h).ghosts.is_empty());
        assert!(h
            .act(TimeAction::Edit(E::Switch { track: 0, slot: 0 }))
            .is_err());
    }

    #[test]
    fn pressure_drawn_is_one_take_a_stroke_and_survives_a_tidy() {
        use mpfree_seq::TouchPoint;
        use mpfree_time_ui::editor::EditAction as E;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 0,
            velocity: 100,
        })
        .unwrap();
        h.act(TimeAction::Edit(E::Open)).unwrap();
        h.act(TimeAction::Edit(E::Draw {
            tick: 960,
            note: 48,
            len: 960,
        }))
        .unwrap();
        let edit = |h: &mut TimeHost| {
            h.take_changed();
            h.state().edit.unwrap()
        };
        // The pressure of the note at 960, and whether it is picked.
        let run_of = |e: &mpfree_time_ui::view::EditInfo| -> (Vec<(u32, u8)>, bool) {
            let i = e.clip.notes.iter().position(|n| n.tick == 960).unwrap();
            let run = e.clip.touch_of(&e.clip.notes[i]);
            (
                run.iter().map(|p| (p.at, p.value)).collect(),
                e.editor.sel.contains(&i),
            )
        };
        let stroke = |h: &mut TimeHost, points: Vec<(i64, u8)>, end: i64| {
            let rev = edit(h).rev;
            h.act(TimeAction::Edit(E::Pressure { rev, points, end }))
        };
        let one = stroke(&mut h, vec![(960, 30), (1200, 90)], 1920).unwrap();
        assert_eq!(one, "edit: pressure drawn on 1 note");
        assert_eq!(run_of(&edit(&mut h)), (vec![(0, 30), (240, 90)], true));
        stroke(&mut h, vec![(1440, 10)], 1920).unwrap();
        assert_eq!(
            run_of(&edit(&mut h)),
            (vec![(0, 30), (240, 90), (480, 10)], true)
        );
        // UNDO: the old pressure.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(run_of(&edit(&mut h)).0, [(0, 30), (240, 90)]);
        // A stale picture: refused, nothing drawn.
        let rev = edit(&mut h).rev;
        assert!(h
            .act(TimeAction::Edit(E::Pressure {
                rev: rev - 1,
                points: vec![(960, 1)],
                end: 1000,
            }))
            .is_err());
        // Past half the room with pressure nothing names (as old strokes
        // whose takes are gone leave); a stroke, then a period, tidies.
        let chunk: Vec<TouchPoint> = (0..1000).map(|k| TouchPoint { at: k, value: 1 }).collect();
        let clip = h.control.project.tracks[0].slots[0].as_mut().unwrap();
        for _ in 0..33 {
            clip.add_touch(&chunk).unwrap();
        }
        // UNDO left nothing picked (the picked note went): pick it.
        let e = edit(&mut h);
        if !run_of(&e).1 {
            let i = e.clip.notes.iter().position(|n| n.tick == 960).unwrap();
            h.act(TimeAction::Edit(E::Note {
                index: i,
                rev: e.rev,
            }))
            .unwrap();
        }
        let three = stroke(&mut h, vec![(1680, 64)], 1920).unwrap();
        assert_eq!(three, "edit: pressure drawn on 1 note");
        let rev = edit(&mut h).rev;
        run(&mut h, &mut pads, u64::from(PERIOD), &[]);
        let (freed, times) = h.control.touch_freed();
        assert!(freed >= 33_000 && times == 1, "{freed} {times}");
        let e = edit(&mut h);
        assert!(e.clip.touch.len() < 100, "{}", e.clip.touch.len());
        assert!(e.rev > rev, "the picture moved on");
        assert_eq!(
            run_of(&e),
            (vec![(0, 30), (240, 90), (720, 64)], true),
            "the picks stay"
        );
        // UNDO and REDO find each stroke's pressure.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(run_of(&edit(&mut h)).0, [(0, 30), (240, 90)]);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(run_of(&edit(&mut h)).0, []);
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(run_of(&edit(&mut h)).0, [(0, 30), (240, 90)]);
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(run_of(&edit(&mut h)).0, [(0, 30), (240, 90), (720, 64)]);
    }

    /// Piano roll plan 0C.2: the copy of the clip the screen reads carries
    /// the answer to the editor's last touch, numbered: taken with the new
    /// revision and where its notes landed, refused with why, or nothing
    /// changed.
    #[test]
    fn the_note_editors_copy_answers_each_touch() {
        use mpfree_time_ui::editor::EditAction as E;
        use mpfree_time_ui::view::EditOutcome as O;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        for step in [0, 4, 8] {
            h.step_action(StepAction::Tap {
                pad: 0,
                step,
                velocity: 100,
            })
            .unwrap();
        }
        let answer = |h: &mut TimeHost| {
            h.take_changed();
            h.state().edit.unwrap().answer
        };
        h.act(TimeAction::Edit(E::Open)).unwrap();
        let a = answer(&mut h);
        assert_eq!((a.n, a.outcome), (1, O::Unchanged));
        let rev = h.state().edit.unwrap().rev;
        // A pick: nothing changed.
        h.act(TimeAction::Edit(E::Note { index: 1, rev })).unwrap();
        assert_eq!(answer(&mut h).outcome, O::Unchanged);
        // A move: taken, at a new revision, the note where it landed.
        h.act(TimeAction::Edit(E::Drop {
            index: 1,
            rev,
            ticks: 240,
            notes: 2,
            scale: false,
        }))
        .unwrap();
        let a = answer(&mut h);
        let st = h.state().edit.unwrap();
        assert_eq!(a.n, 3);
        assert_eq!(a.from, rev);
        let O::Taken { rev: now, landed } = a.outcome else {
            panic!("{a:?}")
        };
        assert_eq!(now, st.rev);
        assert!(now > rev);
        assert_eq!(landed.len(), 1);
        let n = st.clip.notes[landed[0]];
        assert_eq!((n.tick, n.note), (4 * STEP_TICKS + 240, 2));
        // A tap on the old picture: refused, and why.
        h.act(TimeAction::Edit(E::Note { index: 0, rev }))
            .unwrap_err();
        let a = answer(&mut h);
        assert_eq!(a.n, 4);
        assert!(
            matches!(&a.outcome, O::Refused(why) if why.contains("the clip changed")),
            "{a:?}"
        );
    }

    /// COPY + a note copies the picked notes, apart from the clip, and
    /// COPY + empty space pastes them, here or in another track's clip,
    /// as one take: their pressure comes along, and a lock another track
    /// may not have the setting of is left out and said (piano roll plan
    /// 1B.4).
    #[test]
    fn the_note_editor_copies_notes_into_another_tracks_clip() {
        use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
        use mpfree_seq::{Lock, TouchPoint};
        use mpfree_time_ui::editor::EditAction as E;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        for step in [0, 4] {
            h.step_action(StepAction::Tap {
                pad: 0,
                step,
                velocity: 100,
            })
            .unwrap();
        }
        let p = |at, value| TouchPoint { at, value };
        let cut = RelAddr::new(OnTrack::Inst, 1);
        let level = RelAddr::new(OnTrack::Strip, 0);
        {
            let c = h.control.project.tracks[0].slots[0].as_mut().unwrap();
            let i = c.notes.iter().position(|n| n.tick == 0).unwrap();
            c.notes[i].touch = c.add_touch(&[p(0, 30), p(60, 90)]).unwrap();
            let note = c.notes[i].note;
            c.add_lock(Lock::new(0, Some(note), cut, 900.0)).unwrap();
            c.add_lock(Lock::new(0, Some(note), level, 0.25)).unwrap();
        }
        h.control.project.tracks[1].slots[0] = Some(Clip::new("Chorus", 3840));
        h.act(TimeAction::Edit(E::Open)).unwrap();
        let why = h.act(TimeAction::Edit(E::Paste { tick: 0 })).unwrap_err();
        assert!(why.contains("nothing copied yet"), "{why}");
        let st = h.state().edit.unwrap();
        let first = st.clip.notes.iter().position(|n| n.tick == 0).unwrap();
        let said = h
            .act(TimeAction::Edit(E::Copy {
                index: first,
                rev: st.rev,
            }))
            .unwrap();
        let name = st.clip.name.clone();
        assert_eq!(
            h.state().notes_copied.as_deref(),
            Some(format!("1 note from {name}").as_str()),
            "{said}"
        );
        assert_eq!(h.state().edit.unwrap().editor.sel, [first], "picked");
        // Into track 2's clip: the pressure comes, the synth's lock not.
        h.act(TimeAction::Arm(Some(1))).unwrap();
        h.act(TimeAction::Edit(E::Open)).unwrap();
        // Not pad hits into a clip of notes.
        h.control.project.tracks[1].slots[0].as_mut().unwrap().holds = mpfree_seq::Holds::Notes;
        let why = h.act(TimeAction::Edit(E::Paste { tick: 480 })).unwrap_err();
        assert!(
            why.contains("Chorus holds notes and the copy is pad hits"),
            "{why}"
        );
        // Its pressure nearly full of runs nothing names: taken back first.
        {
            let c = h.control.project.tracks[1].slots[0].as_mut().unwrap();
            c.holds = mpfree_seq::Holds::Hits;
            c.touch = vec![p(0, 1); mpfree_seq::MAX_TOUCH_POINTS - 1];
        }
        let takes = h.state().done.len();
        let said = h.act(TimeAction::Edit(E::Paste { tick: 480 })).unwrap();
        let track = h.control.project.tracks[1].name.clone();
        assert!(
            said.starts_with(&format!("paste: 1 note from {name} into Chorus on {track}")),
            "{said}"
        );
        assert!(
            said.ends_with(&format!(
                "1 lock left out, as {track} may not have its setting: 1 on the instrument"
            )),
            "{said}"
        );
        let c = h.control.project.clip(1, 0).unwrap().clone();
        assert_eq!(c.notes.len(), 1);
        assert_eq!(c.notes[0].tick, 480);
        assert_eq!(c.touch_of(&c.notes[0]), [p(0, 30), p(60, 90)]);
        assert_eq!(c.touch.len(), 2, "the room nothing named was taken back");
        assert_eq!(
            c.locks,
            [Lock::new(480, Some(c.notes[0].note), level, 0.25)]
        );
        let st = h.state();
        assert_eq!(st.done.len(), takes + 1, "one take");
        assert_eq!(st.edit.unwrap().editor.sel, [0], "the pasted notes picked");
        h.act(TimeAction::Undo).unwrap();
        assert!(h.control.project.clip(1, 0).unwrap().notes.is_empty());
        // Back on track 1, both locks come with a paste.
        h.act(TimeAction::Arm(Some(0))).unwrap();
        h.act(TimeAction::Edit(E::Open)).unwrap();
        let said = h.act(TimeAction::Edit(E::Paste { tick: 1920 })).unwrap();
        assert!(!said.contains("left out"), "{said}");
        let c = h.control.project.clip(0, 0).unwrap();
        assert_eq!(c.locks.iter().filter(|l| l.tick == 1920).count(), 2);
    }

    /// A lock on the synth pastes into another track's clip when that
    /// track plays the same synth (refactor F1), not when it plays
    /// another, nor into a clip whose synth locks were made for another.
    #[test]
    fn a_synth_lock_pastes_onto_a_track_that_plays_the_same_synth() {
        use mpfree_node::Node;
        use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
        use mpfree_seq::Lock;
        use mpfree_time_ui::editor::EditAction as E;
        let poly = crate::synth::engine_node("mpfree.poly", RATE).unwrap();
        let fm = crate::synth::engine_node("mpfree.fm", RATE).unwrap();
        let of = |n: &dyn Node| -> Option<InstRef> { Some((n.info(), n.params())) };
        let cutoff = poly.params().iter().find(|p| p.name == "Cutoff").unwrap();
        let cut = RelAddr::new(OnTrack::Inst, cutoff.id);
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.set_instruments(&[of(&*poly), of(&*poly), of(&*fm)]);
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 0,
            velocity: 100,
        })
        .unwrap();
        {
            let c = h.control.project.tracks[0].slots[0].as_mut().unwrap();
            let note = c.notes[0].note;
            c.add_lock(Lock::new(0, Some(note), cut, 0.7)).unwrap();
        }
        for t in [1, 2] {
            h.control.project.tracks[t].slots[0] = Some(Clip::new("Verse", 3840));
        }
        h.act(TimeAction::Edit(E::Open)).unwrap();
        // The lock put in by hand gets its tag with the first action, so
        // the picture moves on once.
        let why = h.act(TimeAction::Edit(E::Paste { tick: 0 })).unwrap_err();
        assert!(why.contains("nothing copied yet"), "{why}");
        let st = h.state().edit.unwrap();
        h.act(TimeAction::Edit(E::Copy {
            index: 0,
            rev: st.rev,
        }))
        .unwrap();
        let paste = |h: &mut TimeHost, track: usize| {
            h.act(TimeAction::Arm(Some(track))).unwrap();
            h.act(TimeAction::Edit(E::Open)).unwrap();
            h.act(TimeAction::Edit(E::Paste { tick: 480 })).unwrap()
        };
        // Track 2 plays Poly too: the lock comes, and the clip's locks are
        // now made for Poly.
        let said = paste(&mut h, 1);
        assert!(!said.contains("left out"), "{said}");
        let c = h.control.project.clip(1, 0).unwrap();
        assert_eq!(c.locks.len(), 1);
        assert_eq!(c.locks[0].addr, cut);
        assert_eq!(
            c.made_for.as_ref().map(|m| m.node.as_str()),
            Some("mpfree.poly")
        );
        // Track 3 plays FM: left out, and said.
        let said = paste(&mut h, 2);
        let fm_track = h.control.project.tracks[2].name.clone();
        assert!(
            said.ends_with(&format!(
                "1 lock left out, as {fm_track} may not have its setting: \
                 1 on the instrument (made for Poly, {fm_track} plays FM)"
            )),
            "{said}"
        );
        assert!(h.control.project.clip(2, 0).unwrap().locks.is_empty());
        // A clip on track 2 whose synth locks were made for FM takes none
        // of Poly's.
        h.act(TimeAction::Undo).unwrap();
        h.act(TimeAction::Arm(Some(1))).unwrap();
        h.act(TimeAction::Undo).unwrap();
        {
            let c = h.control.project.tracks[1].slots[0].as_mut().unwrap();
            c.add_lock(Lock::new(960, None, cut, 0.1)).unwrap();
            c.made_for = Some(mpfree_seq::inst::Inst {
                node: "mpfree.fm".into(),
                name: "FM".into(),
                params: vec![],
            });
        }
        let said = paste(&mut h, 1);
        assert!(
            said.ends_with("1 on the instrument (Verse's are made for FM)"),
            "{said}"
        );
        assert_eq!(h.control.project.clip(1, 0).unwrap().locks.len(), 1);
    }

    /// The note inspector (piano roll plan 1C.1): every detent of a turn,
    /// whichever knob, goes in at once and joins one take; the next turn is
    /// another; UNDO takes a turn back whole. – / + nudge as the Nudge row.
    #[test]
    fn the_note_inspector_makes_one_take_a_turn() {
        use mpfree_time_ui::editor::EditAction as E;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        for step in [0, 4] {
            h.step_action(StepAction::Tap {
                pad: 0,
                step,
                velocity: 100,
            })
            .unwrap();
        }
        h.act(TimeAction::Edit(E::Open)).unwrap();
        h.act(TimeAction::Edit(E::Choose(0))).unwrap();
        assert_eq!(h.state().edit.unwrap().editor.sel.len(), 2, "All");
        let notes = |h: &TimeHost| {
            let mut v: Vec<(Tick, u8, u8)> = h
                .control
                .project
                .clip(0, 0)
                .unwrap()
                .notes
                .iter()
                .map(|n| (n.tick, n.note, n.velocity))
                .collect();
            v.sort_unstable();
            v
        };
        let before = notes(&h);
        let takes = h.state().done.len();
        for (k, ticks) in [(0, 1), (3, -10), (0, 1)] {
            h.act(TimeAction::Edit(E::Knob {
                k,
                ticks,
                fine: false,
                turn: 1,
            }))
            .unwrap();
        }
        assert_eq!(h.state().done.len(), takes + 1, "one take a turn");
        let turned = notes(&h);
        let pad = before[0].1;
        assert_eq!(turned, [(480, pad, 90), (1440, pad, 90)]);
        h.act(TimeAction::Edit(E::Knob {
            k: 2,
            ticks: 1,
            fine: false,
            turn: 2,
        }))
        .unwrap();
        assert_eq!(h.state().done.len(), takes + 2, "the next turn, another");
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(notes(&h), turned);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(notes(&h), before, "the whole turn back");
        // – / +: a snap later, the picks kept.
        h.act(TimeAction::Edit(E::Choose(0))).unwrap();
        h.act(TimeAction::Edit(E::Nudge {
            steps: 1,
            fine: false,
        }))
        .unwrap();
        assert_eq!(notes(&h)[0].0, 240);
        assert_eq!(h.state().edit.unwrap().editor.sel.len(), 2);
    }

    /// An editor edit that what plays can't follow (the play view would not
    /// build) is taken back from the clip and the takes, as a step edit is.
    #[test]
    fn a_note_editor_edit_the_play_view_cant_follow_is_taken_back() {
        use mpfree_play::Launch;
        use mpfree_time_ui::editor::EditAction as E;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.step_action(StepAction::Tap {
            pad: 0,
            step: 0,
            velocity: 100,
        })
        .unwrap();
        h.act(TimeAction::Edit(E::Open)).unwrap();
        // Draw picked.
        h.act(TimeAction::Edit(E::Button(2))).unwrap();
        // A view that can't be built: two clips launched on track 2.
        for slot in 0..2 {
            h.control.project.tracks[1].slots[slot] = Some(Clip::new("x", 3840));
            h.control.launches.push(Launch {
                track: 1,
                slot,
                origin: 0,
                start: 0,
            });
        }
        // Another track armed: the edit compiles rather than going into the
        // view in place (plan 0C.1).
        h.control.armed = Some(2);
        let clip = h.control.project.clip(0, 0).cloned();
        let ed = h.state().edit.map(|e| e.editor);
        let said = h
            .act(TimeAction::Edit(E::Empty { tick: 960, note: 3 }))
            .unwrap_err();
        assert_eq!(h.control.costs.notes_compiled, 1);
        assert!(said.ends_with("the edit was taken back"), "{said}");
        assert_eq!(h.control.project.clip(0, 0).cloned(), clip);
        assert_eq!(h.state().edit.map(|e| e.editor), ed, "the editor as it was");
        // UNDO takes back the step, not the drawn note.
        h.control.launches.retain(|l| l.track != 1);
        h.control.project.tracks[1].slots = Default::default();
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.control.project.clip(0, 0).map(|c| c.notes.len()), None);
    }

    /// Pads played over bars 2 and 3 without recording; Capture keeps them
    /// as a new clip in the armed track that plays on in its place, and
    /// Undo takes it back.
    #[test]
    fn capture_keeps_what_was_played_without_recording() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        assert_eq!(
            h.act(TimeAction::Capture).unwrap(),
            "capture: nothing new to keep"
        );
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.take_changed();
        run(
            &mut h,
            &mut pads,
            3 * BAR + BAR / 2,
            &[(BAR + 1000, 4, 100, 2000), (2 * BAR + 500, 9, 80, 2000)],
        );
        assert!(h.take_changed(), "the Capture button changed");
        assert_eq!(h.state().capture.as_deref(), Some("2 bars"));
        assert!(
            pads.hits.is_empty() && h.state().done.is_empty(),
            "nothing was recorded or played back"
        );
        let line = h.act(TimeAction::Capture).unwrap();
        assert_eq!(
            line,
            "capture 1: 2 notes, 2 bars, new clip in slot 1 of Track 1"
        );
        let st = h.state();
        assert_eq!(st.capture, None);
        assert_eq!(
            (
                st.clip(0, 0).map(|c| (c.name.as_str(), c.notes)),
                st.launch(0).is_some()
            ),
            (Some(("Capture 1", 2)), true)
        );
        // It plays on in its place: it was played over bars 2 and 3, so bar
        // 4 is its first bar again. Kept halfway through bar 4, it plays
        // its second bar's note in bar 5 and its first bar's in bar 6.
        run(&mut h, &mut pads, 6 * BAR, &[]);
        let back: Vec<(u64, usize)> = pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        assert_eq!(back.len(), 2, "{back:?}");
        for ((f, p), (want, pad)) in back.iter().zip([(4 * BAR + 500, 9), (5 * BAR + 1000, 4)]) {
            assert!(f.abs_diff(want) <= 23 && *p == pad, "{back:?}");
        }
        assert_eq!(
            h.act(TimeAction::Undo).unwrap(),
            "undo: capture 1 · Track 1 / 1"
        );
        let st = h.state();
        assert!(st.clip(0, 0).is_none() && st.launch(0).is_none());
        assert_eq!(
            st.capture.as_deref(),
            Some("2 bars"),
            "it can be kept again"
        );
        assert_eq!(
            h.act(TimeAction::Redo).unwrap(),
            "redo: capture 1 · Track 1 / 1"
        );
        assert_eq!(h.dropped(), 0);
    }

    /// Several armed tracks record in one take (SHIFT + a track's row in
    /// the REC chip's list; the MPC 3.10.1 guide's Rec Arm Behavior): REC
    /// readies a clip on each, the keys a route plays on each armed track
    /// record into its own clip, a routed track that isn't armed only
    /// plays, and one UNDO takes the take back on every track. The arm
    /// can't change while the take records. A track that records pressure
    /// keeps its keys' pressure on their notes (refactor E3); one that
    /// doesn't leaves it out, as the armed track's pads.
    #[test]
    fn armed_tracks_record_their_routed_keys_in_one_take() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        // Track 3 doesn't record pressure (a track does unless told).
        assert!(h.set_record_touch(2, false));
        h.act(TimeAction::Arm(Some(0))).unwrap();
        assert_eq!(
            h.act(TimeAction::ArmAlso(1 << 1 | 1 << 2 | 1 << 0))
                .unwrap(),
            "armed too: Track 2, Track 3",
            "the armed track is left out of the others"
        );
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        let said = h.act(TimeAction::Transport(Command::Record)).unwrap();
        assert!(
            said.contains("new clip in slot 1 of track 1")
                && said.contains("new clip in slot 1 of track 2")
                && said.contains("new clip in slot 1 of track 3"),
            "{said}"
        );
        run(&mut h, &mut pads, BAR / 2 + 4 * u64::from(PERIOD), &[]);
        assert!(
            h.act(TimeAction::ArmAlso(0)).is_err(),
            "the arm stays while the take records"
        );
        let on = |note| Event::NoteOn { note, velocity: 90 };
        // (frame, track, event): Tracks 2 and 3 armed, Track 4 not.
        let keys = [
            (BAR + 2000, 1, on(5)),
            (BAR + 4500, 1, Event::Pressure { note: 5, value: 40 }),
            (BAR + 6000, 1, Event::NoteOff { note: 5 }),
            (BAR + 3000, 2, on(7)),
            (BAR + 5000, 2, Event::Pressure { note: 7, value: 70 }),
            (BAR + 9000, 2, Event::NoteOff { note: 7 }),
            (BAR + 4000, 3, on(1)),
            (BAR + 5000, 3, Event::NoteOff { note: 1 }),
        ];
        let pressed = [(BAR + 1000, 4, 100, 2000)];
        while pads.frame < 2 * BAR {
            let end = pads.frame + u64::from(PERIOD);
            for &(at, track, ev) in &keys {
                if (pads.frame..end).contains(&at) {
                    h.routed_played(track, (at - pads.frame) as u32, ev);
                }
            }
            run(&mut h, &mut pads, end, &pressed);
        }
        let notes = |h: &TimeHost, t: usize| {
            h.control.project.tracks[t]
                .slots
                .clips()
                .map(|c| c.notes.iter().map(|n| n.note).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        };
        assert_eq!(notes(&h, 0), vec![vec![4]]);
        assert_eq!(notes(&h, 1), vec![vec![5]]);
        assert_eq!(notes(&h, 2), vec![vec![7]]);
        assert!(
            notes(&h, 3).is_empty(),
            "a track that isn't armed only plays"
        );
        // Track 2 records pressure: its note keeps it. Track 3 doesn't.
        let touch = |h: &TimeHost, t: usize| {
            let c = h.control.project.tracks[t].slots.clips().next().unwrap();
            let pts = c.touch_of(&c.notes[0]);
            pts.iter().map(|p| p.value).collect::<Vec<u8>>()
        };
        assert_eq!(touch(&h, 1), vec![40]);
        assert_eq!(touch(&h, 2), Vec::<u8>::new());
        assert_eq!(h.touch_dropped(), 0, "nothing lost");
        let line = h.describe();
        assert!(line.contains("0 pressure values lost"), "{line}");
        assert!(!line.contains("left out of a take"), "{line}");
        let st = h.state();
        assert_eq!(st.done.len(), 1, "one take");
        assert_eq!(st.done[0].added, 3);
        assert!(h.act(TimeAction::Undo).unwrap().starts_with("undo: take 1"));
        for t in 0..3 {
            assert_eq!(notes(&h, t), vec![Vec::<u8>::new()], "track {}", t + 1);
        }
        assert_eq!(h.dropped(), 0);
    }

    /// An armed track with no clip playing and no empty slot doesn't
    /// record the take, and REC says so; the others still record.
    #[test]
    fn an_armed_track_with_no_room_is_said_and_the_others_record() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.act(TimeAction::Arm(Some(0))).unwrap();
        h.act(TimeAction::ArmAlso(1 << 1 | 1 << 2)).unwrap();
        for s in 0..ROWS {
            h.control.project.tracks[2].slots[s] = Some(Clip::new("full", 3840));
        }
        let said = h.act(TimeAction::Transport(Command::Record)).unwrap();
        assert!(
            said.contains("new clip in slot 1 of track 2")
                && said.contains("Track 3 has no clip playing and no empty slot"),
            "{said}"
        );
        assert_eq!(h.control.also_full(), 1);
        run(&mut h, &mut pads, BAR, &[]);
        assert_eq!(h.info().state, State::Recording);
        let line = h.describe();
        assert!(
            line.contains("1 armed tracks left out of a take with no room for a clip"),
            "{line}"
        );
    }

    /// Keys a route plays on tracks that aren't armed go into CAPTURE's
    /// phrase with the pads' notes. CAPTURE puts each track's notes in a
    /// clip of its own, all one take that one UNDO takes back, and says
    /// which track it left out.
    #[test]
    fn capture_keeps_routed_keys_in_their_own_tracks_as_one_take() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        let on = |note| Event::NoteOn { note, velocity: 90 };
        // (frame, track, event): Track 3 in bars 2 and 3, Track 2 in bar
        // 2, and a track there isn't.
        let keys = [
            (BAR + 2000, 2, on(5)),
            (BAR + 6000, 2, Event::NoteOff { note: 5 }),
            (2 * BAR + 100, 2, on(6)),
            (2 * BAR + 4000, 2, Event::NoteOff { note: 6 }),
            (BAR + 3000, 1, on(7)),
            (BAR + 9000, 1, Event::NoteOff { note: 7 }),
            (BAR + 4000, 40, on(1)),
            (BAR + 5000, 40, Event::NoteOff { note: 1 }),
        ];
        let pressed = [(BAR + 1000, 4, 100, 2000)];
        while pads.frame < 3 * BAR + BAR / 2 {
            let end = pads.frame + u64::from(PERIOD);
            for &(at, track, ev) in &keys {
                if (pads.frame..end).contains(&at) {
                    h.routed_played(track, (at - pads.frame) as u32, ev);
                }
            }
            let start = pads.frame;
            run(&mut h, &mut pads, end, &pressed);
            assert_eq!(pads.frame, end, "one period from {start}");
        }
        assert_eq!(h.state().capture.as_deref(), Some("2 bars"));
        assert!(h.state().done.is_empty(), "nothing was recorded");
        assert_eq!(
            h.act(TimeAction::Capture).unwrap(),
            "capture 1: 1 notes, 2 bars, new clip in slot 1 of Track 1; \
             1 notes, new clip in slot 1 of Track 2; \
             2 notes, new clip in slot 1 of Track 3; \
             track 41 left out: there is no such track"
        );
        let st = h.state();
        for (t, n) in [(0, 1), (1, 1), (2, 2)] {
            assert_eq!(
                (
                    st.clip(t, 0).map(|c| (c.name.as_str(), c.notes)),
                    st.launch(t).is_some()
                ),
                (Some(("Capture 1", n)), true),
                "track {}",
                t + 1
            );
        }
        assert_eq!(st.done.len(), 1, "one take");
        assert_eq!(
            h.act(TimeAction::Undo).unwrap(),
            "undo: capture 1 · Track 1 / 1"
        );
        let st = h.state();
        assert!((0..3).all(|t| st.clip(t, 0).is_none() && st.launch(t).is_none()));
        assert_eq!(
            st.capture.as_deref(),
            Some("2 bars"),
            "it can be kept again"
        );
        h.act(TimeAction::Redo).unwrap();
        let st = h.state();
        assert!((0..3).all(|t| st.clip(t, 0).is_some()));
        assert_eq!(h.dropped(), 0);
    }

    /// A routed note whose end never came stops holding CAPTURE's phrase
    /// open once its track's notes all end (`AllOff`): the next note after
    /// the quiet starts a phrase of its own.
    #[test]
    fn a_routed_all_off_ends_a_held_note_for_capture() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        let keys = [
            (
                BAR + 1000,
                Event::NoteOn {
                    note: 5,
                    velocity: 90,
                },
            ),
            (2 * BAR, Event::AllOff),
            (
                5 * BAR + 100,
                Event::NoteOn {
                    note: 6,
                    velocity: 90,
                },
            ),
            (5 * BAR + 4000, Event::NoteOff { note: 6 }),
        ];
        while pads.frame < 6 * BAR + BAR / 2 {
            let end = pads.frame + u64::from(PERIOD);
            for &(at, ev) in &keys {
                if (pads.frame..end).contains(&at) {
                    h.routed_played(1, (at - pads.frame) as u32, ev);
                }
            }
            run(&mut h, &mut pads, end, &[]);
        }
        assert_eq!(
            h.act(TimeAction::Capture).unwrap(),
            "capture 1: 1 notes, 1 bar, new clip in slot 1 of Track 2"
        );
        assert_eq!(h.dropped(), 0);
    }

    /// The moves made over a phrase go with the armed track's notes; when
    /// only routed keys played, they stay out, and the line says so.
    #[test]
    fn capture_of_routed_keys_alone_leaves_the_moves_out_and_says_so() {
        use mpfree_play::mpfree_param::{Owner, ParamAddr};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        let knob = ParamAddr::new(Owner::Inst(0), 5);
        let keys = [
            (
                BAR + 1000,
                Event::NoteOn {
                    note: 5,
                    velocity: 90,
                },
            ),
            (BAR + 9000, Event::NoteOff { note: 5 }),
        ];
        let mut moves: Vec<(u64, Option<f32>)> = (0..5)
            .map(|i| (BAR + 3000 + i * 1500, Some(0.2 * i as f32)))
            .collect();
        moves.push((BAR + 12_000, None));
        while pads.frame < 3 * BAR {
            let end = pads.frame + u64::from(PERIOD);
            for &(at, ev) in &keys {
                if (pads.frame..end).contains(&at) {
                    h.routed_played(1, (at - pads.frame) as u32, ev);
                }
            }
            for &(at, v) in &moves {
                if (pads.frame..end).contains(&at) {
                    let input = match v {
                        Some(value) => Input::Move {
                            addr: knob,
                            value,
                            map: mpfree_seq::mpfree_moves::Map::linear(0.0, 1.0),
                        },
                        None => Input::End { addr: knob },
                    };
                    h.gesture((at - pads.frame) as u32, input);
                }
            }
            h.period(PERIOD, &mut pads);
            pads.frame = end;
        }
        assert_eq!(
            h.act(TimeAction::Capture).unwrap(),
            "capture 1: 1 notes, 1 bar, new clip in slot 1 of Track 2; \
             1 move left out: it goes with the armed track's notes"
        );
        let clip = h.control.project.clip(1, 0).expect("Track 2's clip");
        assert!(clip.moves.is_empty(), "no move went into Track 2's clip");
        assert!(h.control.project.clip(0, 0).is_none());
        assert_eq!(h.dropped(), 0);
    }

    /// Pad pressure played without recording goes on its note when
    /// CAPTURE keeps it, and plays back with it; a track that leaves
    /// pressure out keeps the note without it.
    #[test]
    fn capture_keeps_the_pressure_played_on_its_notes() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::Play);
        // Pad 5 held over bar 2, squeezed harder then let up. The same
        // value twice is one; a value after letting go is no one's.
        let (on, off) = (BAR + 1_000, BAR + 30_000);
        let squeeze = [
            (BAR + 2_000, 40u8),
            (BAR + 2_100, 40),
            (BAR + 10_000, 90),
            (BAR + 20_000, 20),
            (BAR + 40_000, 60),
        ];
        while pads.frame < 2 * BAR + BAR / 2 {
            let end = pads.frame + u64::from(PERIOD);
            let here = |at: u64| {
                (pads.frame..end)
                    .contains(&at)
                    .then(|| (at - pads.frame) as u32)
            };
            if let Some(o) = here(on) {
                let event = Event::NoteOn {
                    note: 5,
                    velocity: 100,
                };
                h.pad(o, event);
            }
            for &(at, value) in &squeeze {
                if let Some(o) = here(at) {
                    h.pad_pressure(o, 5, value);
                }
            }
            if let Some(o) = here(off) {
                h.pad(o, Event::NoteOff { note: 5 });
            }
            h.period(PERIOD, &mut pads);
            pads.frame = end;
        }
        assert!(pads.pressure.is_empty(), "nothing played back yet");
        assert_eq!(
            h.act(TimeAction::Capture).unwrap(),
            "capture 1: 1 notes with 3 pressure values, 1 bar, new clip in slot 1 of Track 1"
        );
        let clip = h.control.project.clip(0, 0).unwrap();
        let kept: Vec<(u32, u8)> = clip
            .touch_of(&clip.notes[0])
            .iter()
            .map(|p| (p.at, p.value))
            .collect();
        // Timed from the note's start, in ticks (23 frames a tick).
        let tick = |frames: u64| (frames as f64 * 1920.0 / f64::from(RATE)).round() as u32;
        assert_eq!(
            kept,
            [(tick(1_000), 40), (tick(9_000), 90), (tick(19_000), 20)]
        );
        // It plays on in its place, with its pressure, every bar.
        run(&mut h, &mut pads, 4 * BAR, &[]);
        let back: Vec<(usize, u8)> = pads.pressure.iter().map(|p| (p.1, p.2)).collect();
        assert_eq!(back, [(5, 40), (5, 90), (5, 20)]);
        assert!(pads.pressure[0].0.abs_diff(3 * BAR + 2_000) <= 23);
        // Undone, then kept by a track that leaves pressure out.
        assert_eq!(
            h.act(TimeAction::Undo).unwrap(),
            "undo: capture 1 · Track 1 / 1"
        );
        h.act(TimeAction::RecordPressure {
            track: 0,
            on: false,
        })
        .unwrap();
        let line = h.act(TimeAction::Capture).unwrap();
        assert!(line.contains(": 1 notes, 1 bar"), "{line}");
        let clip = h.control.project.clip(0, 0).unwrap();
        assert!(clip.touch.is_empty() && clip.notes[0].touch.is_none());
        assert_eq!((h.dropped(), h.touch_dropped()), (0, 0));
    }

    /// Recording puts each note into the clip and the play view in place:
    /// once the take has begun, recording periods (a new pass replacing the
    /// last one, sixteen notes a second) neither compile nor allocate, the
    /// view is what compiling again would give, and the screens hear about
    /// the notes at most four times a second.
    #[test]
    fn recording_neither_compiles_nor_allocates_on_every_note() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake {
            hits: Vec::with_capacity(100_000),
            hit_tracks: Vec::with_capacity(100_000),
            releases: Vec::with_capacity(100_000),
            ..Fake::default()
        };
        h.command(Command::SetLoop(Some((0, 2 * 3840))));
        h.act(TimeAction::NewClip { track: 2, slot: 0 }).unwrap();
        let presses: Vec<(u64, u8, u8, u64)> = (0..200u64)
            .map(|i| (1000 + i * 2756, (i % 16) as u8, 60 + (i % 60) as u8, 1000))
            .collect();
        // The first notes begin the take (its entry in the history).
        run(&mut h, &mut pads, BAR / 2, &presses);
        assert_eq!(h.info().state, State::Recording);
        h.take_changed();
        let mut told = 0;
        let ((), allocs) = crate::sys::alloc_count::calls(|| {
            while pads.frame < 6 * BAR {
                let to = pads.frame + u64::from(PERIOD);
                run(&mut h, &mut pads, to, &presses);
                told += usize::from(h.take_changed());
            }
        });
        assert_eq!(allocs, 0, "allocations or frees while recording");
        let notes = h.state().clip(2, 0).map(|c| c.notes).unwrap();
        assert!(notes > 40, "{notes} notes in the clip");
        assert!(!pads.hits.is_empty(), "the clip plays what it recorded");
        let mut fresh =
            PlayView::compile_cued(&h.control.project, &h.control.launches, &h.control.cues)
                .unwrap();
        fresh.set_armed(h.control.armed);
        assert_eq!(
            *h.audio.view, fresh,
            "the view in place is what compiling gives"
        );
        // Every recorded note was a numbered change.
        assert!(
            h.control.includes.recorded >= notes as u64,
            "{:?}",
            h.control.includes
        );
        // 5.5 bars of 2 s: the notes at most 4 times a second (44), and the
        // Capture button once a bar; one each note would be 176.
        assert!((10..=52).contains(&told), "told the screens {told} times");
    }

    #[test]
    fn a_slot_with_a_clip_is_not_made_again() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.act(TimeAction::NewClip { track: 0, slot: 3 }).unwrap();
        assert!(h.act(TimeAction::NewClip { track: 0, slot: 3 }).is_err());
        assert!(h.act(TimeAction::NewClip { track: 99, slot: 0 }).is_err());
    }

    #[test]
    fn the_click_counts_in_and_follows_recording_only() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        assert!(!h.metronome, "off by default (live test 3)");
        h.metronome = true;
        let mut pads = Fake::default();
        let (mut l, mut r) = ([0f32; PERIOD as usize], [0f32; PERIOD as usize]);
        let mut onsets = Vec::new();
        let mut frame = 0u64;
        let mut prev = 0f32;
        let mut step =
            |h: &mut TimeHost, pads: &mut Fake, frame: &mut u64, onsets: &mut Vec<u64>| {
                h.period(PERIOD, pads);
                l.fill(0.0);
                r.fill(0.0);
                h.mix_click(&mut l, &mut r);
                // An onset: sound after silence (a blip dies away within 0.1 s).
                for (f, &s) in l.iter().enumerate() {
                    if s != 0.0 && prev == 0.0 {
                        onsets.push(*frame + f as u64);
                    }
                    prev = s;
                }
                pads.frame += u64::from(PERIOD);
                *frame += u64::from(PERIOD);
            };
        // Playing: no click.
        h.command(Command::Play);
        while frame < BAR {
            step(&mut h, &mut pads, &mut frame, &mut onsets);
        }
        assert!(onsets.is_empty(), "{onsets:?}");
        h.command(Command::Stop);
        h.command(Command::SetCountIn(1));
        h.command(Command::Record);
        let start = frame;
        while frame < start + 2 * BAR {
            step(&mut h, &mut pads, &mut frame, &mut onsets);
        }
        // One bar of count-in and one of recording: eight beats, half a second apart.
        assert_eq!(onsets.len(), 8, "{onsets:?}");
        for w in onsets.windows(2) {
            assert!((w[1] - w[0]).abs_diff(22_050) <= 1, "{onsets:?}");
        }
        assert!(l.iter().chain(&r).all(|s| s.abs() <= 1.0));
    }

    #[test]
    fn the_transport_follows_the_apps_tempo() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.follow_tempo(96.0);
        h.period(PERIOD, &mut pads);
        assert_eq!(h.info().bpm, 96.0);
        h.follow_tempo(96.0);
        h.period(PERIOD, &mut pads);
        assert_eq!(h.info().bpm, 96.0);
    }

    /// Tempo ramps in the app. The Song view's tempo lane edits the
    /// song's points (UNDO takes them back; the wheel's and the tempo's
    /// turns on one point are one undo). Song mode plays them: the
    /// project's tempo before the first, a ramp gliding on the
    /// transport's staircase, then the last point's; off, the project's
    /// tempo throughout. Turning the tempo sets the point in effect at the
    /// playhead, or before the first adds one at the playhead's bar. The
    /// project's tempo moves the song's before its first point only.
    #[test]
    fn song_mode_plays_the_songs_tempo_points() {
        use mpfree_time_ui::TempoEdit::{Add, AtPlayhead, Bpm, Delete, Ramp};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let tempo = |h: &mut TimeHost, e| h.act(TimeAction::SongTempo(e));
        // Bar 3 at 100 gliding to 140 on bar 5.
        tempo(&mut h, Add(2 * 3840)).unwrap();
        assert_eq!(
            (h.state().tempo_points, h.state().tempo_pick),
            (vec![(7680, 120.0, false)], Some(0))
        );
        tempo(
            &mut h,
            Bpm {
                index: 0,
                bpm: 101.0,
            },
        )
        .unwrap();
        tempo(
            &mut h,
            Bpm {
                index: 0,
                bpm: 100.0,
            },
        )
        .unwrap();
        tempo(&mut h, Add(4 * 3840)).unwrap();
        tempo(
            &mut h,
            Bpm {
                index: 1,
                bpm: 140.0,
            },
        )
        .unwrap();
        tempo(&mut h, Ramp { index: 0, on: true }).unwrap();
        let points = vec![(7680, 100.0, true), (15_360, 140.0, false)];
        assert_eq!(h.state().tempo_points, points);
        // Add, the wheel's two turns, add, a turn, the ramp.
        assert_eq!(h.state().done.len(), 5);
        let period = |h: &mut TimeHost, pads: &mut Fake, bpm: f64| {
            h.follow_tempo(bpm);
            h.period(PERIOD, pads);
            pads.frame += u64::from(PERIOD);
        };

        // Song mode off: the project's tempo throughout.
        period(&mut h, &mut pads, 120.0);
        assert_eq!(h.tempo_map().segments().len(), 1);
        assert!(!h.song_tempo_plays() && h.state().tempo_here.is_none());
        h.act(TimeAction::SongMode(true)).unwrap();
        period(&mut h, &mut pads, 120.0);
        let map = h.control.project.arrangement.tempo_map(120.0).unwrap();
        assert_eq!(*h.tempo_map(), map);
        assert!(h.song_tempo_plays());

        // Playing: 120 for two bars, gliding up over bars 3 and 4, then 140.
        assert!(h.command(Command::Play));
        let mut heard: Vec<(Tick, f64)> = Vec::new();
        while h.info().position < 6 * 3840 {
            period(&mut h, &mut pads, 120.0);
            let i = h.info();
            heard.push((i.position, i.bpm));
        }
        assert!(heard.iter().filter(|h| h.0 < 7680).all(|h| h.1 == 120.0));
        // (A period that ends on a line takes its tempo in the next.)
        let ramp: Vec<f64> = heard
            .iter()
            .filter(|h| h.0 > 7680 && h.0 <= 15_360)
            .map(|h| h.1)
            .collect();
        assert!(ramp.windows(2).all(|w| w[1] >= w[0]));
        assert!(ramp[0] > 100.0 && ramp[0] < 102.0, "{}", ramp[0]);
        let last = *ramp.last().unwrap();
        assert!(last > 138.0 && last < 140.0, "{last}");
        // A step every 1/16 through the ramp.
        let mut steps = ramp.clone();
        steps.dedup();
        assert_eq!(steps.len(), 32);
        assert!(heard.iter().filter(|h| h.0 > 15_360).all(|h| h.1 == 140.0));
        assert_eq!(h.state().tempo_here, Some(140.0));

        // The tempo turned in bar 6: the point on bar 5 takes it, and the
        // turns are one undo.
        let done = h.state().done.len();
        tempo(&mut h, AtPlayhead(141.0)).unwrap();
        tempo(&mut h, AtPlayhead(142.0)).unwrap();
        assert_eq!(h.state().tempo_points[1], (15_360, 142.0, false));
        assert_eq!(h.state().done.len(), done + 1);
        for _ in 0..50 {
            period(&mut h, &mut pads, 120.0);
        }
        assert_eq!(h.info().bpm, 142.0);
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.state().tempo_points, points);
        for _ in 0..50 {
            period(&mut h, &mut pads, 120.0);
        }
        assert_eq!(h.info().bpm, 140.0);

        // Back to bar 1: the project's tempo moves the song's before its
        // first point; turning the tempo there adds a point on bar 1.
        assert!(h.command(Command::StopAndRewind));
        period(&mut h, &mut pads, 90.0);
        period(&mut h, &mut pads, 90.0);
        assert_eq!((h.info().position, h.info().bpm), (0, 90.0));
        assert_eq!(h.tempo_map().segments()[1].bpm, 100.0);
        assert_eq!(h.state().tempo_base, 90.0);
        tempo(&mut h, AtPlayhead(95.0)).unwrap();
        assert_eq!(h.state().tempo_points[0], (0, 95.0, false));
        period(&mut h, &mut pads, 90.0);
        assert_eq!(h.info().bpm, 95.0);
        tempo(&mut h, Delete(0)).unwrap();

        // Song mode off: the project's tempo throughout again.
        h.act(TimeAction::SongMode(false)).unwrap();
        period(&mut h, &mut pads, 90.0);
        period(&mut h, &mut pads, 90.0);
        assert_eq!(h.tempo_map().segments().len(), 1);
        assert_eq!(h.info().bpm, 90.0);
        assert_eq!(h.state().tempo_points, points, "the points stay");
    }

    /// The looper's Starts: When I play (start_at): the song starts on
    /// the frame asked, from the start of the bar it stopped in, with no
    /// count-in; a tempo given is the whole song's, exactly, and the app's
    /// tempo following it asks for nothing more; it is refused while the
    /// song plays or counts in (counted, with why); and a take ready
    /// records from the same frame.
    #[test]
    fn a_note_starts_the_song_on_its_frame_from_the_bar_it_stopped_in() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        // Played into bar 2 (a bar is 88 200 frames at 120 BPM), stopped.
        assert!(h.command(Command::Play));
        while pads.frame < 120_000 {
            h.period(PERIOD, &mut pads);
            pads.frame += u64::from(PERIOD);
        }
        assert!(h.command(Command::Stop));
        h.period(PERIOD, &mut pads);
        pads.frame += u64::from(PERIOD);
        assert_eq!(h.info().state, State::Stopped);
        let bar = 4 * mpfree_transport::PPQ as i64;
        let at = h.next_frame() + 40;
        // The app's tempo moves with it, in the same period (4c's way).
        assert_eq!(h.start_at(at, Some(117.3456), None, 0), Ok(None));
        h.follow_tempo(117.3456);
        h.period(PERIOD, &mut pads);
        let span = *h.block().spans().last().unwrap();
        assert_eq!(span.state, State::Playing);
        assert_eq!(span.frame, at, "on the frame asked");
        assert_eq!(span.ticks().start, bar, "from bar 2's start");
        assert_eq!(h.info().bpm, 117.3456);
        assert_eq!(h.tempo_map().segments().len(), 1, "the whole song's tempo");
        // Nothing more asked for: no tempo change comes.
        for _ in 0..20 {
            h.follow_tempo(117.3456);
            h.period(PERIOD, &mut pads);
            assert!(
                !h.block()
                    .markers()
                    .iter()
                    .any(|m| matches!(m.mark, mpfree_transport::Mark::Tempo(_))),
                "asked again"
            );
        }
        // A real change still goes in (on the next 1/16 line), asked for
        // once: until that line it counts as asked.
        let mut asked = 0;
        for _ in 0..60 {
            let room = h.audio.queue_room();
            h.follow_tempo(100.0);
            asked += room - h.audio.queue_room().min(room);
            h.period(PERIOD, &mut pads);
        }
        assert_eq!(h.info().bpm, 100.0);
        assert_eq!(asked, 1, "asked again before the line");
        // Refused while it plays, and while it counts in.
        let r = h.start_at(h.next_frame(), None, None, 0);
        assert_eq!(r, Err("the song is playing already".to_string()));
        assert!(h.command(Command::StopAndRewind));
        h.period(PERIOD, &mut pads);
        assert!(h.command(Command::SetCountIn(1)));
        assert!(h.command(Command::Record));
        h.period(PERIOD, &mut pads);
        assert_eq!(h.info().state, State::CountIn);
        let r = h.start_at(h.next_frame(), None, None, 0);
        assert_eq!(r, Err("the song is counting in".to_string()));
        assert_eq!(h.starts_refused(), 2);
        // A take ready: it records from the note, with no count-in.
        assert!(h.command(Command::StopAndRewind));
        h.period(PERIOD, &mut pads);
        let at = h.next_frame();
        let line = h.start_at(at, None, Some(true), 0).unwrap().unwrap();
        assert!(
            line.starts_with("rec: the take started on your note"),
            "{line}"
        );
        h.period(PERIOD, &mut pads);
        let span = *h.block().spans().last().unwrap();
        assert_eq!((span.state, span.frame), (State::Recording, at));
        assert_eq!(span.ticks().start, 0);
        assert!(h.info().overdub, "adding");
        // No clip to record into (here, an armed track that has gone):
        // the song starts on the note's frame without the take, with no
        // punch-in before it to take back.
        assert!(h.command(Command::StopAndRewind));
        h.period(PERIOD, &mut pads);
        h.control.armed = Some(h.control.project.tracks.len());
        let at = h.next_frame() + 40;
        let line = h.start_at(at, None, Some(false), 0).unwrap().unwrap();
        assert!(line.ends_with("so it only plays"), "{line}");
        for _ in 0..8 {
            h.period(PERIOD, &mut pads);
            assert!(
                h.block()
                    .spans()
                    .iter()
                    .all(|s| s.state != State::Recording),
                "recorded"
            );
        }
        assert_eq!(h.info().state, State::Playing);
        // A queue without room for every command refuses the start and
        // leaves nothing behind (no Locate, no tempo).
        assert!(h.command(Command::Stop));
        h.period(PERIOD, &mut pads);
        let pos = h.info().position;
        let bpm = h.info().bpm;
        let far = h.next_frame() + 1_000_000;
        while h.audio.queue_room() > 2 {
            assert!(h.audio.command_at(far, Command::Stop));
        }
        let r = h.start_at(h.next_frame(), Some(90.0), None, 0);
        assert_eq!(r, Err("the transport's queue is full".to_string()));
        assert_eq!(h.audio.queue_room(), 2, "nothing queued");
        h.period(PERIOD, &mut pads);
        assert_eq!((h.info().position, h.info().bpm), (pos, bpm));
    }

    /// The frames a tick span takes at `bpm`.
    fn frames_of(ticks: f64, bpm: f64) -> f64 {
        ticks * 60.0 * f64::from(RATE) / (bpm * 960.0)
    }

    /// One timing change (smart loop end, step 0C), made while the song
    /// plays: it waits for its frame and lands there in one piece. A tempo
    /// alone takes over on its frame with no jump, so the notes sounding go
    /// on and nothing replays or is skipped. With a position it is a jump
    /// on its frame: each sounding note ends once there, and the song plays
    /// on from the position at the new tempo. Stopped, one with PLAY starts
    /// the song on its frame.
    #[test]
    fn a_timing_change_lands_on_its_frame_in_one_piece() {
        use mpfree_transport::Mark;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        // A note held through bar 1 (pad 1), a short hit on every beat
        // (pad 2), in a two-bar clip.
        let mut clip = Clip::new("Held and hits", 2 * 3840);
        clip.add_note(mpfree_seq::Note::new(0, 3840, 1, 100))
            .unwrap();
        for k in 0..8 {
            clip.add_note(mpfree_seq::Note::new(k * 960, 120, 2, 100))
                .unwrap();
        }
        h.put_clip(clip).unwrap();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        assert!(h.command(Command::Play));
        let mut pads = Fake::default();
        run(&mut h, &mut pads, BAR / 4, &[]);
        // Just into beat 2: pad 1 held, beat 2's hit sounding.
        let at = h.next_frame() + 3 * u64::from(PERIOD) + 17;
        let c = TimingChange {
            at,
            tempo: Some(100.0),
            position: None,
            play: false,
            take: None,
            made_against: h.timeline(),
        };
        assert_eq!(h.allowed(&c, false), Allowed::Safe);
        assert_eq!(
            h.allowed(&c, true),
            Allowed::Ask("recorded audio would play at its old tempo")
        );
        let t = h.timing_change(c).unwrap();
        assert_eq!(h.timing_answer(t), Some(Answer::Waiting));
        // The first tick not yet played on `at` takes that frame, and the
        // song goes on from it at 100 BPM.
        let now = (0i64..)
            .find(|&k| frames_of(k as f64, 120.0).round() as u64 >= at)
            .unwrap();
        let on = |tick: i64| at as f64 + frames_of((tick - now) as f64, 100.0);
        let (hits, releases) = (pads.hits.len(), pads.releases.len());
        let mut marks = Vec::new();
        let end = on(3840) as u64 + 2 * u64::from(PERIOD);
        while pads.frame < end {
            h.period(PERIOD, &mut pads);
            marks.extend(h.block().markers().iter().map(|m| (m.frame, m.mark)));
            pads.frame += u64::from(PERIOD);
        }
        assert_eq!(h.timing_answer(t), Some(Answer::Committed(at)));
        assert_eq!(h.info().bpm, 100.0);
        assert!(marks.contains(&(at, Mark::Tempo(100.0))), "{marks:?}");
        assert!(
            !marks.iter().any(|m| matches!(m.1, Mark::Located { .. })),
            "no jump: {marks:?}"
        );
        let near = |f: u64, want: f64| (f as f64 - want).abs() <= 1.0;
        let ended = |pads: &Fake, from: usize, pad: usize| -> Vec<u64> {
            pads.releases[from..]
                .iter()
                .filter(|r| r.1 == pad)
                .map(|r| r.0)
                .collect()
        };
        // Pad 1 sounds on and ends once, on bar 2 in the new timing; beat
        // 2's hit ends on its own tick, the next beats land where 100 BPM
        // puts them, and nothing replays.
        let e1 = ended(&pads, releases, 1);
        assert!(e1.len() == 1 && near(e1[0], on(3840)), "{e1:?}");
        let e2 = ended(&pads, releases, 2);
        assert!(near(e2[0], on(1080)), "{e2:?}");
        let after: Vec<(u64, usize)> = pads.hits[hits..].iter().map(|&(f, p, _)| (f, p)).collect();
        assert_eq!(after.len(), 3, "{after:?}");
        for (k, &(f, p)) in after.iter().enumerate() {
            assert!(p == 2 && near(f, on(1920 + 960 * k as i64)), "{after:?}");
        }

        // A tempo and a position: a jump to bar 1 on its frame, while beat
        // 5's hit sounds.
        let at = h.next_frame() + 2 * u64::from(PERIOD) + 5;
        let t = h
            .timing_change(TimingChange {
                at,
                tempo: Some(120.0),
                position: Some(0),
                play: false,
                take: None,
                made_against: h.timeline(),
            })
            .unwrap();
        let (hits, releases) = (pads.hits.len(), pads.releases.len());
        let mut jumps = Vec::new();
        let end = at + frames_of(1000.0, 120.0) as u64;
        while pads.frame < end {
            h.period(PERIOD, &mut pads);
            for m in h.block().markers() {
                if let Mark::Located { to, .. } = m.mark {
                    jumps.push((m.frame, to));
                }
            }
            pads.frame += u64::from(PERIOD);
        }
        assert_eq!(h.timing_answer(t), Some(Answer::Committed(at)));
        assert_eq!(jumps, vec![(at, 0)]);
        assert_eq!(h.info().bpm, 120.0);
        // Beat 5's hit ends once, on the jump; bar 1 plays from there:
        // pad 1 and beat 1 on the jump's frame (beat 1 ending on its own
        // tick), beat 2 a beat later.
        let e2 = ended(&pads, releases, 2);
        assert!(
            e2.len() == 2 && e2[0] == at && near(e2[1], at as f64 + frames_of(120.0, 120.0)),
            "{e2:?}"
        );
        let after: Vec<(u64, usize)> = pads.hits[hits..].iter().map(|&(f, p, _)| (f, p)).collect();
        assert_eq!(after.len(), 3, "{after:?}");
        assert!(after[..2].contains(&(at, 1)) && after[..2].contains(&(at, 2)));
        assert!(
            after[2].1 == 2 && near(after[2].0, at as f64 + frames_of(960.0, 120.0)),
            "{after:?}"
        );
        assert_eq!(h.audio.timing_counts().committed, 2);

        // Stopped: PLAY, a tempo and bar 1 on one frame.
        assert!(h.command(Command::Stop));
        h.period(PERIOD, &mut pads);
        pads.frame += u64::from(PERIOD);
        let at = h.next_frame() + 2 * u64::from(PERIOD) + 44;
        let t = h
            .timing_change(TimingChange {
                at,
                tempo: Some(110.0),
                position: Some(0),
                play: true,
                take: None,
                made_against: h.timeline(),
            })
            .unwrap();
        let mut started = None;
        for _ in 0..4 {
            h.period(PERIOD, &mut pads);
            pads.frame += u64::from(PERIOD);
            let s = h.block().spans().iter().find(|s| s.state == State::Playing);
            started = started.or(s.map(|s| (s.frame, s.ticks().start)));
        }
        assert_eq!(h.timing_answer(t), Some(Answer::Committed(at)));
        assert_eq!(started, Some((at, 0)), "from bar 1, on its frame");
        assert_eq!(h.info().bpm, 110.0);
    }

    /// Anything that moves time before a timing change's frame cancels
    /// it, saying why, and one made against an older timeline is refused
    /// for the same reason: a newer change, a locate, a tempo, STOP, a
    /// song edit that moves time (not one that doesn't), its maker taking
    /// it back, another project.
    #[test]
    fn a_timing_change_is_cancelled_by_anything_that_moves_time_first() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        assert!(h.command(Command::Play));
        run(&mut h, &mut pads, BAR / 2, &[]);
        let change = |h: &TimeHost| TimingChange {
            at: h.next_frame() + 10 * u64::from(PERIOD),
            tempo: Some(90.0),
            position: None,
            play: false,
            take: None,
            made_against: h.timeline(),
        };
        let a = h.timing_change(change(&h)).unwrap();
        let b = h.timing_change(change(&h)).unwrap();
        assert_eq!(
            h.timing_answer(a),
            Some(Answer::Cancelled("a newer change came"))
        );
        assert_eq!(h.timing_answer(b), Some(Answer::Waiting));
        assert!(h.cancel_timing(b, "the loop changed"));
        assert!(!h.cancel_timing(b, "the loop changed"));
        assert_eq!(
            h.timing_answer(b),
            Some(Answer::Cancelled("the loop changed"))
        );
        for (cmd, why) in [
            (Command::Locate(3840), "the song moved"),
            (Command::SetTempo(130.0), "the tempo changed"),
            (Command::Stop, "the song stopped"),
        ] {
            let old = change(&h);
            let t = h.timing_change(change(&h)).unwrap();
            assert!(h.command(cmd));
            assert_eq!(h.timing_answer(t), Some(Answer::Cancelled(why)));
            assert_eq!(h.timing_change(old), Err(why), "made before it");
            h.period(PERIOD, &mut pads);
            pads.frame += u64::from(PERIOD);
        }
        // Song edits: one that doesn't move time leaves it waiting; song
        // mode does, once its view comes.
        let t = h.timing_change(change(&h)).unwrap();
        h.act(TimeAction::Arm(Some(1))).unwrap();
        h.period(PERIOD, &mut pads);
        assert_eq!(h.timing_answer(t), Some(Answer::Waiting));
        h.act(TimeAction::SongMode(true)).unwrap();
        h.period(PERIOD, &mut pads);
        assert_eq!(
            h.timing_answer(t),
            Some(Answer::Cancelled("the song was edited"))
        );
        // Another project's engine: another timeline, and the ticket is
        // none of its own.
        let other = TimeHost::new(RATE, 120.0).unwrap();
        assert_ne!(other.timeline(), h.timeline());
        assert_eq!(other.timing_answer(t), None);
        let mut other = other;
        assert_eq!(other.timing_change(change(&h)), Err("the song changed"));
        assert!(h.audio.timing_counts().cancelled >= 6);
    }

    /// When a timing change can't be made, or can't land when its frame
    /// comes, nothing of it goes in: refused while the song records,
    /// counts in or follows the song's tempo points, for a frame already
    /// played, and when its frame comes with no room in the transport's
    /// queue for all of it.
    #[test]
    fn a_timing_change_lands_whole_or_not_at_all() {
        use mpfree_time_ui::TempoEdit::Add;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        assert!(h.command(Command::Play));
        run(&mut h, &mut pads, BAR / 2, &[]);
        let change = |h: &TimeHost, at: u64| TimingChange {
            at,
            tempo: Some(90.0),
            position: None,
            play: false,
            take: None,
            made_against: h.timeline(),
        };
        let past = change(&h, h.next_frame() - 1);
        assert_eq!(
            h.allowed(&past, false),
            Allowed::Refused("its frame has played")
        );
        assert_eq!(h.timing_change(past), Err("its frame has played"));
        // No room for it when its frame comes: refused, nothing changed.
        let at = h.next_frame() + 2 * u64::from(PERIOD);
        let t = h.timing_change(change(&h, at)).unwrap();
        let far = h.next_frame() + 10_000_000;
        while h.audio.queue_room() > 0 {
            assert!(h.audio.command_at(far, Command::ToggleOverdub));
        }
        assert_eq!(h.timing_answer(t), Some(Answer::Waiting));
        for _ in 0..4 {
            h.period(PERIOD, &mut pads);
            pads.frame += u64::from(PERIOD);
            assert!(!h
                .block()
                .markers()
                .iter()
                .any(|m| matches!(m.mark, mpfree_transport::Mark::Located { .. })));
        }
        assert_eq!(
            h.timing_answer(t),
            Some(Answer::Refused("the transport's queue is full"))
        );
        assert_eq!(h.info().bpm, 120.0);
        // Room in the queue but not in its period: fifteen commands due by
        // its frame would leave its second command for the next period, so
        // it is refused whole.
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        assert!(h.command(Command::Play));
        h.period(PERIOD, &mut pads);
        let at = h.next_frame() + 50;
        let c = TimingChange {
            position: Some(0),
            ..change(&h, at)
        };
        let t = h.timing_change(c).unwrap();
        for _ in 0..15 {
            assert!(h.audio.command_at(at - 1, Command::SetCountIn(0)));
        }
        h.period(PERIOD, &mut pads);
        assert_eq!(
            h.timing_answer(t),
            Some(Answer::Refused("too many commands land in its period"))
        );
        assert_eq!(h.info().bpm, 120.0);
        // Recording (a punch-in): refused.
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        assert!(h.command(Command::Play));
        h.period(PERIOD, &mut pads);
        assert!(h.command(Command::Record));
        h.period(PERIOD, &mut pads);
        assert_eq!(h.info().state, State::Recording);
        let c = change(&h, h.next_frame());
        assert_eq!(
            h.allowed(&c, false),
            Allowed::Refused("the song is recording")
        );
        assert_eq!(h.timing_change(c), Err("the song is recording"));
        // The song's tempo points lead: a tempo change while it plays is
        // refused; a position alone is not.
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.act(TimeAction::SongTempo(Add(2 * 3840))).unwrap();
        h.act(TimeAction::SongMode(true)).unwrap();
        h.follow_tempo(120.0);
        h.period(PERIOD, &mut pads);
        assert!(h.command(Command::Play));
        h.period(PERIOD, &mut pads);
        let c = change(&h, h.next_frame());
        assert_eq!(
            h.allowed(&c, false),
            Allowed::Refused("the song's tempo points lead")
        );
        assert_eq!(h.timing_change(c), Err("the song's tempo points lead"));
        assert_eq!(h.audio.timing_counts().refused, 1);
        let c = TimingChange {
            tempo: None,
            position: Some(3840),
            ..c
        };
        assert_eq!(h.allowed(&c, true), Allowed::Safe);
    }

    fn xorshift(mut x: u64) -> impl FnMut() -> u64 {
        move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        }
    }

    /// The pads played at random into the audio half: presses and
    /// let-gos, pressure while a pad is held, and now and then a knob on
    /// the armed track. The same for a given seed, whichever host runs.
    struct Hands<F: FnMut() -> u64> {
        next: F,
        /// The next press or let-go, absolute frame.
        at: u64,
        held: Option<u8>,
    }

    impl<F: FnMut() -> u64> Hands<F> {
        fn play(&mut self, a: &mut TimeAudio, frame: u64, frames: u32) {
            use mpfree_play::mpfree_param::{OnTrack, RelAddr};
            let end = frame + u64::from(frames);
            while self.at < end {
                let offset = (self.at.max(frame) - frame) as u32;
                match self.held.take() {
                    Some(note) => a.pad(offset, Event::NoteOff { note }),
                    None => {
                        let note = ((self.next)() % 16) as u8;
                        let velocity = 1 + ((self.next)() % 127) as u8;
                        a.pad(offset, Event::NoteOn { note, velocity });
                        self.held = Some(note);
                        if (self.next)().is_multiple_of(4) {
                            let v = ((self.next)() % 100) as f32 / 100.0;
                            a.knob(offset, RelAddr::new(OnTrack::Inst, 1).on(0), v);
                        }
                    }
                }
                self.at += 1 + (self.next)() % (BAR / 16);
            }
            if let Some(note) = self.held {
                a.pad_pressure(frames - 1, note, ((self.next)() % 128) as u8);
            }
        }
    }

    /// Recording into a new clip on track 1 with `hands`, `frames` at a
    /// time, until frame `to`. With `late`, the halves run apart: the
    /// control half wakes every 0 to `late.0` frames (at random, from
    /// seed `late.1`), and half the times it wakes it compiles a view
    /// first (arming track 1 again, which plays the same) from what it
    /// knew when it last woke, as a control thread running late would.
    /// Every view the audio half swaps in must play just as the one it
    /// replaces (it caught up in place), and whenever the control half has
    /// caught up the view playing must be a fresh compile of its project.
    /// Returns the host, the pads, and how many views caught up changes.
    fn record_apart(
        frames: u32,
        to: u64,
        seed: u64,
        late: Option<(u64, u64)>,
    ) -> (TimeHost, Fake, u64) {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.act(TimeAction::NewClip { track: 0, slot: 0 }).unwrap();
        let mut hands = Hands {
            next: xorshift(seed),
            at: 500,
            held: None,
        };
        let Some((late, seed)) = late else {
            while pads.frame < to {
                hands.play(&mut h.audio, pads.frame, frames);
                h.period(frames, &mut pads);
                pads.frame += u64::from(frames);
            }
            return (h, pads, 0);
        };
        let mut next = xorshift(seed);
        let (mut wake, mut caught_up, mut caught) = (0, false, 0);
        while pads.frame < to {
            hands.play(&mut h.audio, pads.frame, frames);
            // The audio half's period starts by taking what was sent.
            let before = (*h.audio.view).clone();
            let was = &*h.audio.view as *const PlayView;
            let missing = h.audio.recent.last();
            h.audio.receive();
            if !std::ptr::eq(was, &*h.audio.view) {
                assert_eq!(*h.audio.view, before, "a late view caught up in place");
                let included = h.audio.view.includes().unwrap().recorded;
                caught += u64::from(included < missing);
            }
            if caught_up {
                assert_eq!(*h.audio.view, h.control.view().unwrap());
            }
            h.audio.run(frames, &mut pads, None);
            pads.frame += u64::from(frames);
            caught_up = false;
            if pads.frame >= wake {
                if next().is_multiple_of(2) {
                    h.control.act(TimeAction::Arm(Some(0))).unwrap();
                }
                h.control.poll();
                caught_up = true;
                wake = pads.frame + next() % late;
            }
        }
        h.control.poll();
        h.audio.receive();
        assert_eq!(*h.audio.view, h.control.view().unwrap());
        (h, pads, caught)
    }

    /// The control half up to 20 ms late, at period sizes from 16 to 4096
    /// frames: what the pads, pressure and a knob record into a looping
    /// clip, and what it plays back, are just what they are with the
    /// halves in step. Nothing recorded is lost or doubled, and no view is
    /// sent back or compiled again.
    #[test]
    fn recording_with_the_control_half_late_loses_and_doubles_nothing() {
        let late = u64::from(RATE) / 50;
        let mut caught = 0;
        for (i, frames) in [16u32, 64, 128, 256, 1024, 4096].into_iter().enumerate() {
            let seed = 0x5eed_2b00 + i as u64;
            let (step, step_pads, _) = record_apart(frames, 3 * BAR, seed, None);
            let (apart, apart_pads, n) =
                record_apart(frames, 3 * BAR, seed, Some((late, seed ^ 0xa5a5)));
            caught += n;
            let clip = |h: &TimeHost| h.control.project.clip(0, 0).cloned().unwrap();
            let (want, got) = (clip(&step), clip(&apart));
            assert!(
                want.notes.len() > 20 && !want.touch.is_empty() && !want.locks.is_empty(),
                "{frames}: {want:?}"
            );
            assert_eq!(got, want, "{frames} frames");
            assert_eq!(apart_pads.hits, step_pads.hits, "{frames} frames");
            assert_eq!(apart_pads.pressure, step_pads.pressure, "{frames} frames");
            assert_eq!(apart.state().done, step.state().done, "{frames} frames");
            let c = apart.audio.counts();
            assert_eq!(
                (
                    apart.dropped(),
                    apart.touch_dropped(),
                    c.lost,
                    c.behind,
                    c.kept,
                    c.refused
                ),
                (0, 0, 0, 0, 0, 0),
                "{frames} frames"
            );
            assert!(
                !apart.describe().contains("between the halves"),
                "{}",
                apart.describe()
            );
        }
        assert!(caught > 100, "views that caught up: {caught}");
    }

    /// Every follow action at 300 BPM, with the control half up to 20 ms
    /// late: a 1-bar clip's follow lands on its bar line, playing just
    /// what it plays with the halves in step.
    #[test]
    fn follow_actions_land_on_their_bar_lines_with_the_control_half_late() {
        use mpfree_seq::{Follow, FollowAction as A, Note};
        let bar = u64::from(RATE) * 4 / 5;
        let play = |action: A, frames: u32, late: Option<u64>| {
            let mut h = TimeHost::new(RATE, 300.0).unwrap();
            let mut pads = Fake::default();
            for slot in 0..3 {
                let mut c = Clip::new("one bar", 3840);
                c.notes = vec![Note::new(0, 100, slot as u8, 100)];
                c.follow = Some(Follow::after(1, action));
                h.control.project.tracks[0].slots[slot] = Some(c);
            }
            h.act(TimeAction::Launch { track: 0, slot: 1 }).unwrap();
            h.command(Command::Play);
            let mut next = xorshift(0xf011_0000 + frames as u64);
            let mut wake = 0;
            while pads.frame < 6 * bar - 1000 {
                match late {
                    None => h.period(frames, &mut pads),
                    Some(late) => {
                        h.audio.run(frames, &mut pads, None);
                        if pads.frame >= wake {
                            h.control.poll();
                            wake = pads.frame + next() % late;
                        }
                    }
                }
                pads.frame += u64::from(frames);
            }
            let c = h.audio.counts();
            assert_eq!(
                (c.lost, c.behind, c.kept, c.refused, h.dropped()),
                (0, 0, 0, 0, 0)
            );
            pads.hits
                .iter()
                .map(|&(f, p, _)| (f, p))
                .collect::<Vec<_>>()
        };
        for action in [
            A::Stay,
            A::Again,
            A::Next,
            A::Previous,
            A::First,
            A::Last,
            A::Any,
            A::Other,
            A::Stop,
        ] {
            for frames in [64, 128, 512] {
                let step = play(action, frames, None);
                let apart = play(action, frames, Some(u64::from(RATE) / 50));
                assert_eq!(apart, step, "{action:?} at {frames} frames");
                // One note on each bar line (Stop: the first bar's only).
                let bars = if action == A::Stop { 1 } else { 6 };
                assert_eq!(step.len(), bars, "{action:?}: {step:?}");
                for (i, &(f, _)) in step.iter().enumerate() {
                    assert!(f.abs_diff(i as u64 * bar) <= 1, "{action:?}: {step:?}");
                }
            }
        }
    }

    /// The timing contract (Arrange 2.5) through the halves: a launch the
    /// control half decides before it has heard the audio half play a bar
    /// line (as a control half on its own thread can be behind) starts at
    /// once, in phase with that line, and Dev counts it; from there on it
    /// plays as if the launch had come in time for the line. Decided with
    /// every period heard, the same launch at the same moment waits for the
    /// next line, as before.
    #[test]
    fn a_launch_decided_before_the_control_half_heard_a_line_lands_in_phase_with_it() {
        use mpfree_seq::Note;
        let bar = u64::from(RATE) * 4 / 5;
        let line = 2 * bar;
        // `act` launches B at the first period from that frame; the
        // control half hears nothing from `deaf` until then.
        let play = |frames: u32, act: u64, deaf: u64| {
            let mut h = TimeHost::new(RATE, 300.0).unwrap();
            let mut pads = Fake::default();
            let mut a = Clip::new("A", 3840);
            a.notes = (0..4).map(|i| Note::new(i * 960, 100, 0, 100)).collect();
            // B: two bars, so a pass out of phase would put its second
            // bar's note (3) elsewhere.
            let mut b = Clip::new("B", 2 * 3840);
            b.notes = (0..8)
                .flat_map(|i| {
                    [
                        Note::new(i * 960, 100, 1, 100),
                        Note::new(i * 960 + 480, 100, 2, 100),
                    ]
                })
                .chain([Note::new(3840, 100, 3, 100)])
                .collect();
            h.control.project.tracks[0].slots[0] = Some(a);
            h.control.project.tracks[0].slots[1] = Some(b);
            h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
            h.command(Command::Play);
            let mut acted = None;
            while pads.frame < 5 * bar {
                if acted.is_none() && pads.frame >= act {
                    h.act(TimeAction::Launch { track: 0, slot: 1 }).unwrap();
                    acted = Some(pads.frame);
                }
                if (deaf..act).contains(&pads.frame) {
                    h.audio.run(frames, &mut pads, None);
                } else {
                    h.period(frames, &mut pads);
                }
                pads.frame += u64::from(frames);
            }
            let (c, q) = (h.audio.counts(), h.audio.cue_counts());
            assert_eq!(
                (
                    c.lost,
                    c.behind,
                    c.kept,
                    c.refused,
                    q.stale,
                    q.lost,
                    h.dropped()
                ),
                (0, 0, 0, 0, 0, 0, 0)
            );
            let hits: Vec<_> = pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
            (hits, acted.unwrap_or(0), q.late, h.control.describe())
        };
        for frames in [64u32, 128, 512] {
            let f = u64::from(frames);
            // The period that holds the line.
            let holds = line / f * f;
            // In time: launched a period before it, everything heard.
            let (in_time, _, late, _) = play(frames, holds - f, u64::MAX);
            assert_eq!(late, 0);
            for held in 1..=4u64 {
                let what = format!("{frames} frames, {held} periods unheard");
                // Launched after the line, the last `held` periods unheard.
                let act = holds + f;
                let (hits, from, late, said) = play(frames, act, act - held * f);
                assert_eq!(late, 1, "{what}");
                assert!(
                    said.contains("1 started after their line"),
                    "{what}: {said}"
                );
                // From the period it came on, as in time; before it, A.
                let after = |h: &[(u64, usize)]| -> Vec<_> {
                    h.iter().filter(|e| e.0 >= from).copied().collect()
                };
                assert_eq!(after(&hits), after(&in_time), "{what}");
                assert!(
                    after(&hits).iter().any(|e| e.1 == 3),
                    "{what}: B's second bar"
                );
                assert!(
                    hits.iter()
                        .all(|&(at, p)| (at < from) == (p == 0) || at < line),
                    "{what}"
                );
                // Everything heard, the same launch waits for bar 4.
                let (step, _, late, _) = play(frames, act, u64::MAX);
                assert_eq!(late, 0, "{what}");
                let b = step.iter().find(|e| e.1 != 0).map(|e| e.0);
                assert_eq!(b, Some(3 * bar), "{what}");
            }
        }
    }

    /// Arrange 2.6: with the control half on a thread of its own, busy with
    /// everything a musician does to a song while it plays (launches,
    /// scenes, stops, step taps, undo and redo, capture, mutes, song mode
    /// on and off), the audio half allocates and frees nothing in any
    /// whole period: taking what the control half sent, playing, and
    /// sending back what it no longer plays.
    #[test]
    fn the_audio_half_allocates_nothing_in_any_period_while_the_control_half_works_on_its_own_thread(
    ) {
        use mpfree_seq::Note;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;
        /// Pads that only count, so the test itself allocates nothing.
        #[derive(Default)]
        struct Count(u64);
        impl Pads for Count {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                self.0 += 1;
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }
        let mut h = TimeHost::new(RATE, 140.0).unwrap();
        // Eight tracks, two rows of 4-bar clips of 64 notes each.
        for t in 0..8 {
            if t > 0 {
                h.act(TimeAction::AddTrack).unwrap();
            }
            for row in 0..2 {
                let mut c = Clip::new("busy", 4 * 3840);
                c.notes = (0..64)
                    .map(|i| Note::new(i * 240, 120, ((t + row + i as usize) % 16) as u8, 100))
                    .collect();
                h.control.project.tracks[t].slots[row] = Some(c);
            }
        }
        h.control.project.give_clip_ids();
        h.act(TimeAction::ScenesToSong).unwrap();
        h.command(Command::Play);
        let (mut control, mut audio) = h.split();
        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        // The control half's actions so far, for the audio loop to wait on.
        let acts = Arc::new(AtomicUsize::new(0));
        let acted = acts.clone();
        let thread = std::thread::spawn(move || {
            let tap = |step| StepAction::Tap {
                pad: 3,
                step,
                velocity: 100,
            };
            let mut k = 0usize;
            let mut lines = 0usize;
            while !stop.load(Ordering::Relaxed) {
                let r = match k % 12 {
                    0 => control.act(TimeAction::Launch {
                        track: k % 8,
                        slot: (k / 8) % 2,
                    }),
                    1 => control.act(TimeAction::LaunchScene((k / 12) % 2)),
                    2 => control.step_action(tap(k % 16)),
                    3 => control.act(TimeAction::Undo),
                    4 => control.act(TimeAction::Redo),
                    5 => control.act(TimeAction::Capture),
                    6 => control.mute_or_solo(k % 8, k % 24 < 12, false),
                    7 => control.act(TimeAction::StopClip(k % 8)),
                    8 => control.act(TimeAction::SongMode(k % 24 < 12)),
                    9 => control.step_action(tap((k + 5) % 16)),
                    10 => control.act(TimeAction::Uncue(k % 8)),
                    _ => control.act(TimeAction::Launch {
                        track: (k + 3) % 8,
                        slot: 1,
                    }),
                };
                lines += usize::from(r.is_ok());
                k += 1;
                acted.store(k, Ordering::Relaxed);
                control.poll();
                std::thread::sleep(std::time::Duration::from_micros(300));
            }
            control.poll();
            (control, k, lines)
        });
        let mut pads = Count::default();
        let mut worst = (0, 0);
        // At least 4,000 periods, and on until the control half has made
        // ACTIONS actions: counted, not timed, since a busy machine slows
        // both sides (on a loaded CI runner 4,000 periods once saw only 959
        // actions). The deadline only stops a hang.
        const ACTIONS: usize = 1000;
        let t0 = std::time::Instant::now();
        let mut period = 0;
        while period < 4000 || acts.load(Ordering::Relaxed) < ACTIONS {
            assert!(
                t0.elapsed() < std::time::Duration::from_secs(300),
                "{} of {ACTIONS} actions after {period} periods",
                acts.load(Ordering::Relaxed)
            );
            let ((), n) = crate::sys::alloc_count::calls(|| audio.run(PERIOD, &mut pads, None));
            if n > worst.1 {
                worst = (period, n);
            }
            period += 1;
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        done.store(true, Ordering::Relaxed);
        let (mut control, actions, ok) = thread.join().unwrap();
        // What the audio half sent back last is freed there.
        audio.run(PERIOD, &mut pads, None);
        control.poll();
        assert_eq!(worst, (0, 0), "(period, allocations and frees) at worst");
        assert!(
            actions >= ACTIONS && ok > actions / 4,
            "{actions} actions, {ok} ok"
        );
        assert!(pads.0 > 200, "it played: {} hits", pads.0);
        assert!(
            control.costs.compiles > 100,
            "{} views",
            control.costs.compiles
        );
        let c = audio.counts();
        assert_eq!((c.lost, c.kept, c.ends_lost, control.lost()), (0, 0, 0, 0));
    }

    /// Arrange 2.6b: periods, a millisecond apart, until the editing side
    /// on its own thread has answered `n` asks: the answers, oldest first.
    fn answers(h: &mut TimeHost, pads: &mut Fake, n: usize) -> Vec<(Say, Result<String, String>)> {
        let mut said = Vec::new();
        let t0 = std::time::Instant::now();
        while said.len() < n {
            assert!(t0.elapsed().as_secs() < 10, "{} of {n} answers", said.len());
            h.period(PERIOD, pads);
            pads.frame += u64::from(PERIOD);
            said.extend(h.take_said());
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(said.len(), n, "{said:?}");
        said
    }

    /// Periods, a millisecond apart, until `done` holds of what the facade
    /// reads (apart, the copy the editing side publishes).
    fn until(h: &mut TimeHost, pads: &mut Fake, done: impl Fn(&TimeHost) -> bool) {
        let t0 = std::time::Instant::now();
        while !done(h) {
            assert!(t0.elapsed().as_secs() < 10, "the copy never followed");
            h.period(PERIOD, pads);
            pads.frame += u64::from(PERIOD);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// A host with a one-bar clip of four notes on track 1, row 1.
    fn host_with_a_clip() -> TimeHost {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut c = Clip::new("beat", 3840);
        c.notes = (0..4)
            .map(|i| mpfree_seq::Note::new(i * 960, 100, 3, 100))
            .collect();
        h.control.project.tracks[0].slots[0] = Some(c);
        h.control.project.give_clip_ids();
        h
    }

    /// Arrange 2.6b: apart, the screen's actions go to the editing side's
    /// thread and return at once; their answers come back later, each with
    /// how its caller wants it said, the same as together answers them on
    /// the spot, and the published copy comes to match the project and
    /// the selection together has. Back together, nothing was lost.
    #[test]
    fn apart_the_screens_actions_answer_as_together_and_the_copy_follows() {
        let note = Say::Note {
            ok: "time: ",
            err: "time: ",
        };
        let actions = vec![
            (TimeAction::AddTrack, note.clone()),
            (TimeAction::Launch { track: 0, slot: 0 }, note.clone()),
            (TimeAction::Arm(Some(1)), note.clone()),
            (TimeAction::Launch { track: 0, slot: 3 }, note.clone()),
            (TimeAction::Uncue(0), note.clone()),
            (TimeAction::Undo, Say::Undo),
            (TimeAction::Redo, Say::Undo),
            (TimeAction::DeleteClip { track: 0, slot: 0 }, Say::Tell),
            (TimeAction::Undo, Say::Undo),
            (TimeAction::DeleteClip { track: 0, slot: 7 }, Say::Tell),
        ];
        let mut together = host_with_a_clip();
        let mut pads = Fake::default();
        let mut want = Vec::new();
        for (a, say) in actions.clone() {
            let r = together
                .act_then(a, say.clone())
                .expect("together answers at once");
            want.push((say, r));
            together.period(PERIOD, &mut pads);
            pads.frame += u64::from(PERIOD);
        }
        assert!(
            want.iter().any(|(_, r)| r.is_err()),
            "an empty slot: {want:?}"
        );
        let (mut h, why) = host_with_a_clip().into_apart();
        assert_eq!(why, None);
        assert!(h.apart());
        let mut pads = Fake::default();
        for (a, say) in actions {
            assert_eq!(h.act_then(a, say), None, "apart, the answer comes later");
        }
        assert_eq!(answers(&mut h, &mut pads, want.len()), want);
        until(&mut h, &mut pads, |h| {
            h.project() == together.project()
                && (h.armed(), h.selected(), h.track_count(), h.redos())
                    == (
                        together.armed(),
                        together.selected(),
                        together.track_count(),
                        together.redos(),
                    )
                && h.steps() == together.steps()
        });
        assert!(h.take_changed(), "the screens are told");
        assert!(
            h.describe().contains("10 asks, 0 lends"),
            "{}",
            h.describe()
        );
        let h = h.into_together().expect("the control half comes back");
        assert!(!h.apart());
        assert_eq!(h.project(), together.project());
    }

    /// Piano roll plan 0B (on Arrange 2.6b): apart, the note editor's
    /// touches go to the editing side's thread as the screen sends them
    /// (`time_action`'s `act_then`) and answer as together does, in the
    /// same order and words: a pick, a move, a tap made on the picture
    /// from before the move's answer came back (refused: the clip changed
    /// under the finger), a stroke along the velocity lane, Draw, a drawn note, ERASE + a
    /// tap, UNDO. The copy the screen reads comes to match together's:
    /// the clip, its revision, the picks and the last touch's answer.
    #[test]
    fn apart_the_note_editor_answers_as_together_and_its_copy_follows() {
        use mpfree_time_ui::editor::EditAction as E;
        use mpfree_time_ui::view::EditOutcome as O;
        let note = Say::Note {
            ok: "time: ",
            err: "time: ",
        };
        // Together: each touch made on the picture the screen has after
        // the touch before it was answered, but the stale tap, made on
        // the picture before the move.
        let mut together = host_with_a_clip();
        let info = |h: &mut TimeHost| {
            h.take_changed();
            *h.state().edit.expect("the editor is open")
        };
        let mut sent: Vec<(TimeAction, Say)> = Vec::new();
        let mut want = Vec::new();
        let mut touch = |h: &mut TimeHost, a: E, say: &Say| {
            let a = TimeAction::Edit(a);
            let r = h.act_then(a.clone(), say.clone()).expect("together");
            sent.push((a, say.clone()));
            want.push((say.clone(), r));
        };
        touch(&mut together, E::Open, &note);
        let first = info(&mut together);
        let rev = first.rev;
        touch(&mut together, E::Note { index: 1, rev }, &note);
        let drop = E::Drop {
            index: 1,
            rev,
            ticks: 240,
            notes: 2,
            scale: false,
        };
        touch(&mut together, drop, &note);
        touch(&mut together, E::Tap { index: 0, rev }, &note);
        // A stroke along the velocity lane over the picks (plan 2A.1).
        let now = info(&mut together);
        let lane = E::Lane {
            field: mpfree_seq::notes::Field::Velocity,
            rev: now.rev,
            set: now.editor.sel.iter().map(|&i| (i, 64)).collect(),
        };
        assert!(!now.editor.sel.is_empty());
        touch(&mut together, lane, &note);
        touch(&mut together, E::Button(2), &note);
        touch(&mut together, E::Empty { tick: 480, note: 5 }, &note);
        let drawn = info(&mut together);
        let index = drawn
            .clip
            .notes
            .iter()
            .position(|n| (n.tick, n.note) == (480, 5))
            .expect("the drawn note");
        let rev = drawn.rev;
        touch(&mut together, E::Erase { index, rev }, &note);
        touch(&mut together, E::Note { index: 0, rev }, &note);
        // COPY + a note, then COPY + empty roll (plan 1B.4): a paste; then
        // a Q-Link turn of two detents on the pasted notes (plan 1C.1), one
        // take. The first UNDO takes the turn back, the second the paste,
        // the third the erase.
        let rev = info(&mut together).rev;
        touch(&mut together, E::Copy { index: 0, rev }, &note);
        touch(&mut together, E::Paste { tick: 2000 }, &note);
        assert!(info(&mut together)
            .clip
            .notes
            .iter()
            .any(|n| n.tick == 2000));
        for ticks in [1, 1] {
            let k = E::Knob {
                k: 0,
                ticks,
                fine: true,
                turn: 1,
            };
            touch(&mut together, k, &note);
        }
        assert!(info(&mut together)
            .clip
            .notes
            .iter()
            .any(|n| n.tick == 2008));
        for _ in 0..3 {
            let r = together
                .act_then(TimeAction::Undo, Say::Undo)
                .expect("together");
            sent.push((TimeAction::Undo, Say::Undo));
            want.push((Say::Undo, r));
        }
        let refused = |r: &Result<String, String>| {
            r.as_ref()
                .is_err_and(|why| why.contains("the clip changed under your finger"))
        };
        assert_eq!(
            want.iter().filter(|(_, r)| refused(r)).count(),
            2,
            "the tap from before the move and the pick from before the erase: {want:?}"
        );
        // UNDO took the paste and the erase back: the drawn note is there
        // again.
        let end = info(&mut together);
        assert!(end.clip.notes.iter().any(|n| (n.tick, n.note) == (480, 5)));
        assert!(!end.clip.notes.iter().any(|n| n.tick == 2000));
        // Apart: every touch sent at once, as a quick hand does.
        let (mut h, why) = host_with_a_clip().into_apart();
        assert_eq!(why, None);
        let mut pads = Fake::default();
        for (a, say) in sent {
            assert_eq!(h.act_then(a, say), None, "apart, the answer comes later");
        }
        assert_eq!(answers(&mut h, &mut pads, want.len()), want);
        until(&mut h, &mut pads, |h| {
            h.project() == together.project()
                && h.state().edit.as_deref() == Some(&end)
                && h.editing()
        });
        let a = h.state().edit.expect("open").answer;
        assert_eq!(a, end.answer);
        assert!(
            matches!(a.outcome, O::Taken { .. }),
            "the last editor touch was the turn: {a:?}"
        );
        // A new copy for something else (a track added) shares the clip
        // while it is unchanged (plan 0C.1), so the roll isn't built again.
        let shown = h.state().edit.expect("open").clip;
        let tracks = h.project().tracks.len();
        assert_eq!(h.act_then(TimeAction::AddTrack, note.clone()), None);
        assert!(answers(&mut h, &mut pads, 1)[0].1.is_ok());
        until(&mut h, &mut pads, |h| {
            h.project().tracks.len() == tracks + 1
        });
        let again = h.state().edit.expect("open").clip;
        assert!(std::sync::Arc::ptr_eq(&shown, &again), "the same copy");
        let h = h.into_together().expect("the control half comes back");
        assert_eq!(h.project().tracks.len(), tracks + 1);
    }

    /// Arrange 2.6b: apart, a call that isn't an ask borrows the control
    /// half back for the call (a lend): its answer is the same as
    /// together's, and what the facade reads straight after (the armed
    /// track, the steps, the mutes) already shows it. Lends are counted.
    #[test]
    fn apart_a_lent_call_answers_at_once_and_reads_see_it() {
        let (mut h, why) = host_with_a_clip().into_apart();
        assert_eq!(why, None);
        let mut pads = Fake::default();
        assert!(h.act(TimeAction::Arm(Some(0))).is_ok());
        assert_eq!(h.armed(), Some(0));
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        let before = h.steps();
        assert!(before.clip.is_some(), "the launched clip's steps");
        let r = h.step_action(StepAction::Tap {
            pad: 5,
            step: 2,
            velocity: 100,
        });
        assert!(r.is_ok(), "{r:?}");
        assert_ne!(h.steps(), before, "read straight after the tap");
        assert!(h.take_steps_changed());
        h.mute_or_solo(0, false, false).unwrap();
        assert_eq!(h.muted(0), Some(true), "read straight after the mute");
        // A copy published before a lend never comes after it: the facade
        // never sees an older state than the lend left.
        for k in 0..200 {
            let t = k % 3;
            h.act(TimeAction::Select(t)).unwrap();
            h.period(PERIOD, &mut pads);
            pads.frame += u64::from(PERIOD);
            assert_eq!(h.selected(), Some(t), "after lend {k}");
        }
        h.act(TimeAction::Select(0)).unwrap();
        assert!(h.command(Command::Play));
        until(&mut h, &mut pads, |h| h.info().state.rolling());
        while !pads_heard(&pads) {
            h.period(PERIOD, &mut pads);
            pads.frame += u64::from(PERIOD);
        }
        assert!(
            h.describe().contains("0 asks, 206 lends"),
            "{}",
            h.describe()
        );
        // The project, read from the copy, comes to have the tap.
        until(&mut h, &mut pads, |h| {
            h.project().clip(0, 0).map(|c| c.notes.len()) == Some(5)
        });
    }

    /// Whether the fake pads were told anything (a muted track still
    /// plays to the pads' mixer strip, silent there).
    fn pads_heard(p: &Fake) -> bool {
        !p.hits.is_empty() || p.frame > 2 * BAR
    }

    /// Arrange 2.6b: apart, the facade's period (the audio half's period
    /// and taking the newest copy, the copy it replaces sent back to be
    /// freed on the editing side's thread) allocates and frees nothing, in
    /// any period, while the editing side compiles launch after launch on
    /// its own thread; the asks themselves are made between periods.
    #[test]
    fn apart_the_facades_period_allocates_nothing_while_the_editing_side_compiles() {
        /// Pads that only count, so the test itself allocates nothing.
        #[derive(Default)]
        struct Count(u64);
        impl Pads for Count {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                self.0 += 1;
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }
        let mut h = TimeHost::new(RATE, 140.0).unwrap();
        for t in 0..4 {
            if t > 0 {
                h.act(TimeAction::AddTrack).unwrap();
            }
            for row in 0..2 {
                let mut c = Clip::new("busy", 4 * 3840);
                c.notes = (0..64)
                    .map(|i| {
                        mpfree_seq::Note::new(
                            i * 240,
                            120,
                            ((t + row + i as usize) % 16) as u8,
                            100,
                        )
                    })
                    .collect();
                h.control.project.tracks[t].slots[row] = Some(c);
            }
        }
        h.control.project.give_clip_ids();
        h.command(Command::Play);
        let (mut h, why) = h.into_apart();
        assert_eq!(why, None);
        let note = Say::Note { ok: "", err: "" };
        let mut pads = Count::default();
        let mut worst = (0, 0);
        let mut asked = 0;
        for period in 0..3000 {
            if period % 3 == 0 {
                let k = period / 3;
                let a = match k % 4 {
                    0 => TimeAction::Launch {
                        track: k % 4,
                        slot: (k / 4) % 2,
                    },
                    1 => TimeAction::Undo,
                    2 => TimeAction::Redo,
                    _ => TimeAction::StopClip(k % 4),
                };
                assert_eq!(h.act_then(a, note.clone()), None);
                asked += 1;
                let _ = h.take_said();
            }
            let ((), n) = crate::sys::alloc_count::calls(|| h.period(PERIOD, &mut pads));
            if n > worst.1 {
                worst = (period, n);
            }
            if period % 10 == 0 {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        assert_eq!(worst, (0, 0), "(period, allocations and frees) at worst");
        let h = h.into_together().expect("back");
        assert!(asked > 900);
        assert!(
            h.control.costs.compiles > 100,
            "{}",
            h.control.costs.compiles
        );
        assert!(pads.0 > 20, "it played: {} hits", pads.0);
        assert_eq!(h.dropped(), 0);
    }

    /// Arrange 2.6b: apart, what comes back without being asked reaches
    /// the app as together: live to song's and the loops' lines, the mute
    /// queue's landings and the steps changing; and the inputs the app
    /// gives every period go only when they change.
    #[test]
    fn apart_the_editing_sides_news_reach_the_app() {
        let (mut h, why) = host_with_a_clip().into_apart();
        assert_eq!(why, None);
        let mut pads = Fake::default();
        // The tempo, every period: one ask for the change only.
        for _ in 0..20 {
            h.follow_tempo(100.0);
            h.take_follows(false);
            h.period(PERIOD, &mut pads);
        }
        assert!(h.describe().contains("2 asks"), "{}", h.describe());
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        // The editing side has heard the transport start (its copy of the
        // screens' state says so).
        until(&mut h, &mut pads, |h| {
            h.state().transport.state.rolling() && h.info().bpm == 100.0
        });
        // A mute waiting for its line lands on it, and its light follows.
        h.mute_or_solo(0, true, false).unwrap();
        assert!(h.mute_light(0).is_some_and(|l| l.waiting));
        until(&mut h, &mut pads, |h| h.muted(0) == Some(true));
        assert!(h.mute_light(0).is_some_and(|l| !l.waiting && l.silent));
        let h = h.into_together().expect("back");
        assert_eq!(h.dropped(), 0);
    }

    /// Arrange 4.1, on the desktop: what Scenes → song and List → song
    /// cost the thread they run on (the audio thread together) in the big
    /// song's rows (16 tracks, 2 rows of 4-bar clips of 1,000 notes):
    /// the rows once (8 bars), and a list of both rows 25 times each (200
    /// bars). `cargo test --release -p mpfree-app --bin mpfree --
    /// --ignored print_what_the_conversions_cost --nocapture`.
    #[test]
    #[ignore]
    fn print_what_the_conversions_cost() {
        use mpfree_seq::{ListStep, Note};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let len = 16 * 960i64;
        for t in 0..16 {
            if t > 0 {
                h.act(TimeAction::AddTrack).unwrap();
            }
            for row in 0..2usize {
                let mut c = Clip::new("c", len);
                c.notes = (0..1000i64)
                    .map(|k| {
                        let pitch = 36 + (k * 7 + row as i64) % 48;
                        Note::new(k * len / 1000, 8, pitch as u8, 100)
                    })
                    .collect();
                h.control.project.tracks[t].slots[row] = Some(c);
            }
        }
        h.control.project.give_clip_ids();
        h.control.project.arrangement.list.steps = vec![
            ListStep { row: 0, times: 25 },
            ListStep { row: 1, times: 25 },
        ];
        for (name, a) in [
            ("scenes → song, 8 bars", TimeAction::ScenesToSong),
            ("list → song, 200 bars", TimeAction::ListToSong),
        ] {
            let mut worst = 0f64;
            for _ in 0..5 {
                let t0 = std::time::Instant::now();
                h.act(a.clone()).unwrap();
                worst = worst.max(t0.elapsed().as_secs_f64() * 1000.0);
                h.act(TimeAction::Undo).unwrap();
            }
            println!("{name}: {worst:.1} ms at worst");
        }
    }

    /// Arrange 2.6c, on the desktop: what each action costs the facade's
    /// thread (the audio thread in the app) in the big song in song mode
    /// while it plays, together and apart: the call and the period after
    /// it, mean and worst of 20. Apart, the screen's actions go as asks
    /// ([`TimeHost::act_then`], as `time_action` sends them), and a step
    /// tap both as an ask and lent. `cargo test --release -p mpfree-app --bin mpfree --
    /// --ignored print_what_each_action_costs --nocapture`.
    #[test]
    #[ignore]
    fn print_what_each_action_costs_the_audio_thread_in_the_big_song() {
        use mpfree_seq::{Note, Placement};
        for apart in [false, true] {
            let mut h = TimeHost::new(RATE, 120.0).unwrap();
            let len = 16 * 960i64;
            for t in 0..16 {
                if t > 0 {
                    h.act(TimeAction::AddTrack).unwrap();
                }
                for row in 0..2usize {
                    let mut c = Clip::new("c", len);
                    c.notes = (0..1000i64)
                        .map(|k| {
                            let pitch = 36 + (k * 7 + row as i64) % 48;
                            Note::new(k * len / 1000, 8, pitch as u8, 100)
                        })
                        .collect();
                    h.control.project.tracks[t].slots[row] = Some(c);
                }
            }
            h.control.project.give_clip_ids();
            for t in 0..16 {
                for k in 0..32i64 {
                    let b =
                        Placement::of(&h.control.project.tracks, t, (k % 2) as usize, k * len, len);
                    h.control.project.arrangement.place(b).unwrap();
                }
            }
            h.act(TimeAction::SongMode(true)).unwrap();
            h.command(Command::Play);
            if apart {
                h = h.into_apart().0;
            }
            let mut pads = Fake::default();
            for _ in 0..50 {
                h.period(PERIOD, &mut pads);
            }
            let note = Say::Note { ok: "", err: "" };
            let mut rows: Vec<(&str, f64, f64)> = Vec::new();
            let mut time = |name, h: &mut TimeHost, f: &mut dyn FnMut(&mut TimeHost)| {
                let (mut worst, mut sum) = (0f64, 0f64);
                for _ in 0..20 {
                    let t0 = std::time::Instant::now();
                    f(h);
                    h.period(PERIOD, &mut pads);
                    let ms = t0.elapsed().as_secs_f64() * 1000.0;
                    (worst, sum) = (worst.max(ms), sum + ms);
                    // The rest of the period, and a few more, as the app
                    // has between touches.
                    for _ in 0..3 {
                        std::thread::sleep(std::time::Duration::from_millis(3));
                        h.period(PERIOD, &mut pads);
                        let _ = h.take_said();
                    }
                }
                rows.push((name, sum / 20.0, worst));
            };
            let mut k = 0;
            let act = |h: &mut TimeHost, a: TimeAction| {
                let _ = h.act_then(a, note.clone());
            };
            time("a period alone", &mut h, &mut |_| {});
            time("launch", &mut h, &mut |h| {
                k += 1;
                act(
                    h,
                    TimeAction::Launch {
                        track: k % 16,
                        slot: k % 2,
                    },
                );
            });
            time("scene", &mut h, &mut |h| {
                k += 1;
                act(h, TimeAction::LaunchScene(k % 2));
            });
            time("undo", &mut h, &mut |h| act(h, TimeAction::Undo));
            time("redo", &mut h, &mut |h| act(h, TimeAction::Redo));
            time("song mode off and on", &mut h, &mut |h| {
                act(h, TimeAction::SongMode(false));
                act(h, TimeAction::SongMode(true));
            });
            time("track mute", &mut h, &mut |h| {
                k += 1;
                let _ = h.mute_or_solo_then(k % 16, false, false, note.clone());
            });
            time("step tap", &mut h, &mut |h| {
                k += 1;
                let tap = StepAction::Tap {
                    pad: 3,
                    step: k % 16,
                    velocity: 100,
                };
                let _ = h.step_action_then(tap, note.clone());
            });
            time("step tap, lent apart", &mut h, &mut |h| {
                k += 1;
                let _ = h.step_action(StepAction::Tap {
                    pad: 3,
                    step: k % 16,
                    velocity: 100,
                });
            });
            time("the screens' state", &mut h, &mut |h| {
                std::hint::black_box(h.state());
            });
            let mode = if apart { "apart" } else { "together" };
            for (n, mean, worst) in &rows {
                eprintln!("{mode:9} {n:22} mean {mean:7.3} ms, worst {worst:7.3} ms");
            }
            eprintln!("{mode:9} {}", h.describe());
        }
    }

    /// Split, the halves run on two threads: the control half on its own,
    /// launching a clip and starting the transport, then polling every
    /// millisecond; the audio half here. The clip plays from the period
    /// the commands reach, on the bar lines, and nothing is lost.
    #[test]
    fn the_halves_split_onto_two_threads_play_what_the_control_half_launches() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut c = Clip::new("beat", 3840);
        c.notes = vec![mpfree_seq::Note::new(0, 100, 7, 100)];
        h.control.project.tracks[0].slots[0] = Some(c);
        let (mut control, mut audio) = h.split();
        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        let thread = std::thread::spawn(move || {
            control
                .act(TimeAction::Launch { track: 0, slot: 0 })
                .unwrap();
            control.act(TimeAction::Transport(Command::Play)).unwrap();
            while !stop.load(Ordering::Relaxed) {
                control.poll();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            control.poll();
            control
        });
        let mut pads = Fake::default();
        // Periods go by, a millisecond apart, while the control thread
        // starts.
        let t0 = std::time::Instant::now();
        while !audio.info().state.rolling() {
            assert!(t0.elapsed().as_secs() < 10, "never started");
            audio.run(PERIOD, &mut pads, None);
            pads.frame += u64::from(PERIOD);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let start = pads.frame - u64::from(PERIOD);
        let end = start + 2 * BAR - 1000;
        while pads.frame < end {
            audio.run(PERIOD, &mut pads, None);
            pads.frame += u64::from(PERIOD);
        }
        done.store(true, Ordering::Relaxed);
        let control = thread.join().unwrap();
        let hits: Vec<(u64, usize)> = pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(hits[1].0 - hits[0].0, BAR, "{hits:?}");
        assert_eq!(
            hits[0],
            (start, 7),
            "the first note on the first frame played"
        );
        assert!(control.launched(0));
        let c = audio.counts();
        assert_eq!(
            (
                c.dropped,
                c.lost,
                c.behind,
                c.kept,
                c.refused,
                control.lost()
            ),
            (0, 0, 0, 0, 0, 0)
        );
    }

    /// The pads, with one track playing somewhere else (as a track on MIDI
    /// OUT does): its events are taken, every `AllOff` is seen and left
    /// to the pads too.
    #[derive(Default)]
    struct Routed {
        pads: Fake,
        track: u16,
        /// (absolute frame, event) of what was taken.
        taken: Vec<(u64, Event)>,
        all_off: usize,
    }

    impl Pads for Routed {
        fn hit_at(&mut self, track: u16, pad: usize, velocity: u8, offset: u32) -> bool {
            self.pads.hit_at(track, pad, velocity, offset)
        }
        fn release_at(&mut self, track: u16, pad: usize, offset: u32) {
            self.pads.release_at(track, pad, offset)
        }
        fn stop_all(&mut self) {
            self.pads.stop_all()
        }
        fn pressure_at(&mut self, track: u16, pad: usize, value: u8, offset: u32) {
            self.pads.pressure_at(track, pad, value, offset)
        }
        fn out(&mut self, o: &Out) -> bool {
            if o.event == Event::AllOff {
                self.all_off += 1;
                return false;
            }
            if o.track != self.track {
                return false;
            }
            self.taken
                .push((self.pads.frame + u64::from(o.offset), o.event));
            true
        }
    }

    /// A track that plays elsewhere takes its clip's notes: they never
    /// reach the pads, while the other track's do; a panic's AllOff is
    /// offered to it and still stops the pads.
    #[test]
    fn a_track_playing_elsewhere_takes_its_clips_notes_from_the_pads() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        for (t, note) in [(0, 4), (1, 9)] {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            h.control.project.tracks[t].slots[0] = Some(c);
            h.act(TimeAction::Launch { track: t, slot: 0 }).unwrap();
        }
        let mut r = Routed {
            track: 1,
            ..Routed::default()
        };
        h.command(Command::Play);
        while r.pads.frame < 2 * BAR - 1000 {
            h.period(PERIOD, &mut r);
            r.pads.frame += u64::from(PERIOD);
        }
        let hits: Vec<(u64, usize)> = r.pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        assert_eq!(hits, vec![(0, 4), (BAR, 4)]);
        let ons: Vec<(u64, Event)> = r
            .taken
            .iter()
            .copied()
            .filter(|(_, e)| matches!(e, Event::NoteOn { .. }))
            .collect();
        let on = Event::NoteOn {
            note: 9,
            velocity: 100,
        };
        assert_eq!(ons, vec![(0, on), (BAR, on)]);
        let offs = r
            .taken
            .iter()
            .filter(|(_, e)| *e == Event::NoteOff { note: 9 })
            .count();
        assert_eq!((offs, r.taken.len()), (2, 4), "{:?}", r.taken);
        // STOP, then STOP again: the panic.
        h.command(Command::Stop);
        h.period(PERIOD, &mut r);
        h.command(Command::Stop);
        h.period(PERIOD, &mut r);
        assert!(r.all_off > 0);
        assert_eq!(r.pads.stops, 1);
        assert_eq!(h.dropped(), 0);
    }

    /// The clip card's Program row (sequencer S5): one take, saved with
    /// the clip, and its program change offered first to a track that
    /// plays elsewhere as the clip starts, before its first note, once
    /// while it loops; on the pads it changes nothing.
    #[test]
    fn the_clip_cards_program_goes_out_as_the_clip_starts() {
        use mpfree_seq::{ClipProgram, ClipSettings};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        for (t, note) in [(0, 4), (1, 9)] {
            let mut c = Clip::new("beat", 3840);
            c.notes = vec![mpfree_seq::Note::new(0, 100, note, 100)];
            h.control.project.tracks[t].slots[0] = Some(c);
            h.act(TimeAction::Launch { track: t, slot: 0 }).unwrap();
        }
        let program = Some(ClipProgram {
            program: 4,
            bank: Some(2),
        });
        for track in [0, 1] {
            let was = h.state().clip(track, 0).unwrap().settings;
            let said = h
                .act(TimeAction::ClipSettings {
                    track,
                    slot: 0,
                    row: 9,
                    settings: ClipSettings { program, ..was },
                })
                .unwrap();
            assert!(said.contains("beat · Program 5"), "{said}");
        }
        assert_eq!(h.state().clip(1, 0).unwrap().settings.program, program);
        let saved = mpfree_seq::save::to_text(&h.control.project);
        assert!(saved.contains("program 4 bank 2\n"), "{saved}");
        let mut r = Routed {
            track: 1,
            ..Routed::default()
        };
        h.command(Command::Play);
        while r.pads.frame < 2 * BAR - 1000 {
            h.period(PERIOD, &mut r);
            r.pads.frame += u64::from(PERIOD);
        }
        let on = Event::NoteOn {
            note: 9,
            velocity: 100,
        };
        let sent = Event::Program {
            program: 4,
            bank: Some(2),
        };
        let starts: Vec<(u64, Event)> = r
            .taken
            .iter()
            .copied()
            .filter(|(_, e)| !matches!(e, Event::NoteOff { .. }))
            .collect();
        assert_eq!(starts, vec![(0, sent), (0, on), (BAR, on)]);
        // The pads' track: its notes as before, nothing from its program.
        let hits: Vec<(u64, usize)> = r.pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        assert_eq!(hits, vec![(0, 4), (BAR, 4)]);
        assert_eq!(h.dropped(), 0);
    }

    /// The arps' notes (the live pads through the armed track's arp) are
    /// not offered to `out`, even on a track that plays elsewhere: they go
    /// to `Pads::live`, which sees the whole event and says what the
    /// armed track records (here the default: each note as played).
    #[test]
    fn the_arps_notes_go_to_live_not_out() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut arps = arp_up(0);
        let mut r = Routed {
            track: 0,
            ..Routed::default()
        };
        h.command(Command::SetLoop(Some((0, 2 * 3840))));
        h.act(TimeAction::NewClip { track: 0, slot: 0 }).unwrap();
        while r.pads.frame < 2 * BAR {
            let f = r.pads.frame;
            let at = |x: u64| {
                (f..f + u64::from(PERIOD))
                    .contains(&x)
                    .then(|| (x - f) as u32)
            };
            if let Some(o) = at(1000) {
                arps.pad(
                    0,
                    o,
                    Event::NoteOn {
                        note: 4,
                        velocity: 100,
                    },
                );
            }
            if let Some(o) = at(BAR / 2) {
                arps.pad(0, o, Event::NoteOff { note: 4 });
            }
            h.period_arps(PERIOD, &mut r, &mut arps);
            r.pads.frame += u64::from(PERIOD);
        }
        assert!(r.taken.is_empty(), "{:?}", r.taken);
        let ons = r.pads.hits.len();
        assert!((6..=8).contains(&ons), "{:?}", r.pads.hits);
        let clip = h.control.project.tracks[0].slots[0].as_ref().unwrap();
        assert_eq!(clip.notes.len(), ons, "every note recorded as played");
        assert_eq!(h.dropped() + arps.dropped(), 0);
    }

    /// Another device's clock: a pulse every `pulse` frames from frame
    /// `first`, and Start halfway before pulse `start`.
    struct Leader {
        first: f64,
        pulse: f64,
        start: usize,
        /// The next pulse, and whether Start went.
        k: usize,
        started: bool,
    }

    impl Leader {
        /// Its bytes due before frame `p`, in order, into `out`; how many.
        fn bytes(&mut self, p: u64, out: &mut [u8; 8]) -> usize {
            use mpfree_midi_clock::{CLOCK, START};
            let p = p as f64;
            let mut n = 0;
            while n < out.len() {
                let start_at = self.first + (self.start as f64 - 0.5) * self.pulse;
                if !self.started && self.k == self.start && start_at < p {
                    out[n] = START;
                    self.started = true;
                } else if self.first + self.k as f64 * self.pulse < p {
                    out[n] = CLOCK;
                    self.k += 1;
                } else {
                    break;
                }
                n += 1;
            }
            n
        }
    }

    /// Following another device's clock as the app does (the MIDI
    /// thread's M4): each period, the leader port's bytes read at its top
    /// go to `clock_read`, which stamps them half a period earlier, and the
    /// app's tempo to `follow_tempo`. The leader runs at 100 BPM, the
    /// transport at 120. A one-bar clip with a note on every beat plays on
    /// the leader's beats from the third bar on, within half a period of
    /// them (bytes read once a period come up to a period after they
    /// did); the app's tempo is ignored while the clock arrives; the screens and `followed_bpm` show the leader's;
    /// the status line has nothing to add; and following allocates
    /// nothing.
    #[test]
    fn a_clip_plays_on_the_beats_of_a_followed_clock() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake {
            hits: Vec::with_capacity(1000),
            hit_tracks: Vec::with_capacity(1000),
            releases: Vec::with_capacity(1000),
            ..Fake::default()
        };
        let mut c = Clip::new("Beats", 3840);
        c.notes = (0..4)
            .map(|i| mpfree_seq::Note {
                tick: i * 960,
                len: 100,
                note: 4,
                velocity: 100,
                chance: 100,
                cond: mpfree_seq::Cond::Always,
                ratchet: 1,
                touch: mpfree_seq::Touch::NONE,
                on: true,
                ramp: 0,
                join: mpfree_seq::Join::Attack,
                group: 0,
            })
            .collect();
        h.control.project.tracks[0].slots[0] = Some(c);
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.follow(Some(Follow {
            start_stop: true,
            position: true,
        }));
        let mut leader = Leader {
            first: 10_000.0,
            pulse: 60.0 * f64::from(RATE) / (100.0 * 24.0),
            start: 48,
            k: 0,
            started: false,
        };
        let (first, pulse) = (leader.first, leader.pulse);
        let beat = |j: usize| (first + (48 + 24 * j) as f64 * pulse).round() as u64;
        let mut buf = [0u8; 8];
        let mut period = |h: &mut TimeHost, pads: &mut Fake| {
            let p = h.next_frame();
            let n = leader.bytes(p, &mut buf);
            h.clock_read(PERIOD, &buf[..n]);
            h.follow_tempo(120.0);
            h.period(PERIOD, pads);
            pads.frame += u64::from(PERIOD);
        };
        while pads.frame < beat(8) {
            period(&mut h, &mut pads);
        }
        let ((), allocs) = crate::sys::alloc_count::calls(|| {
            while pads.frame < beat(32) {
                period(&mut h, &mut pads);
            }
        });
        assert_eq!(allocs, 0, "allocations while following");
        let late: Vec<i64> = (8..32)
            .map(|j| {
                let b = beat(j);
                let hit = pads
                    .hits
                    .iter()
                    .map(|&(f, _, _)| f)
                    .min_by_key(|f| f.abs_diff(b))
                    .unwrap();
                hit as i64 - b as i64
            })
            .collect();
        // A note a beat, from the first (beat 32's may have played too, a
        // little early, in the last period run).
        let mid = (beat(31) + beat(32)) / 2;
        let played = pads.hits.iter().filter(|h| h.0 < mid).count();
        assert_eq!(played, 32, "a note a beat, from the first");
        // On the beats, within half a period: the bytes are read up to a
        // period after they came, and stamped half a period back.
        let half = i64::from(PERIOD) / 2;
        assert!(
            late.iter().all(|&d| (-half..=half).contains(&d)),
            "frames after the leader's beats: {late:?}"
        );
        let mean = late.iter().sum::<i64>() / late.len() as i64;
        assert!(
            mean.abs() <= 16,
            "{mean} frames after the beats on average: {late:?}"
        );
        let bpm = h.followed_bpm().unwrap();
        assert!((bpm - 100.0).abs() < 0.1, "{bpm}");
        assert_eq!(h.info().bpm, bpm);
        assert!(!h.describe().contains("following"), "{}", h.describe());
        // The leader owns the timeline: a loop set while following its
        // position is held for later, so nothing wraps or jumps (a jump is
        // counted by follow.rs's tests), and the screens see it held.
        let bar = h.info().position / 3840 * 3840;
        h.command(Command::SetLoop(Some((bar, bar + 3840))));
        while pads.frame < beat(40) {
            period(&mut h, &mut pads);
        }
        assert!(!h.describe().contains("following"), "{}", h.describe());
        let info = h.info();
        assert_eq!(
            (info.loop_range, info.follows_position, info.playing_loop()),
            (Some((bar, bar + 3840)), true, None)
        );
        // A note a beat on and on, none pulled back by a loop.
        let a_beat = (24.0 * pulse).round() as i64;
        let gaps: Vec<i64> = pads.hits[31..]
            .windows(2)
            .map(|w| w[1].0 as i64 - w[0].0 as i64)
            .collect();
        assert!(gaps.len() >= 8, "{gaps:?}");
        assert!(
            gaps.iter().all(|g| (g - a_beat).abs() <= 64),
            "{gaps:?}, a beat {a_beat}"
        );
        // Not following: the app's tempo applies again (playing, on the
        // next sixteenth line), and the loop comes back.
        h.follow(None);
        assert_eq!(h.followed_bpm(), None);
        for _ in 0..100 {
            period(&mut h, &mut pads);
        }
        let info = h.info();
        assert_eq!(info.bpm, 120.0);
        assert_eq!(
            (info.follows_position, info.playing_loop()),
            (false, Some((bar, bar + 3840)))
        );
    }

    /// A project's clips, saved and read back, make an engine that plays
    /// them; clips that don't hold together are refused.
    #[test]
    fn an_opened_projects_clips_play() {
        let mut p = TimeHost::new(RATE, 120.0).unwrap().project().clone();
        let mut clip = Clip::new("Opened", 3840);
        clip.add_note(mpfree_seq::Note::new(0, 100, 7, 90)).unwrap();
        clip.add_note(mpfree_seq::Note::new(1920, 100, 3, 80))
            .unwrap();
        p.tracks[2].slots[1] = Some(clip);
        let read = mpfree_seq::save::from_text(&mpfree_seq::save::to_text(&p)).unwrap();
        assert_eq!(read, p);
        let mut h = TimeHost::with_project(RATE, 120.0, read).unwrap();
        // As saved, its clip given its lasting id as it opens.
        p.give_clip_ids();
        assert_eq!(h.project(), &p);
        assert_eq!(h.state().armed, Some(0));
        let mut pads = Fake::default();
        h.act(TimeAction::Launch { track: 2, slot: 1 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR - 1000, &[]);
        let hits: Vec<(u64, usize, u8)> = pads.hits.clone();
        assert_eq!(hits, vec![(0, 7, 90), (BAR / 2, 3, 80)]);
        // Too many tracks: refused, with why.
        let mut bad = p.clone();
        while bad.tracks.len() <= mpfree_seq::MAX_TRACKS {
            bad.tracks.push(mpfree_seq::Track::new("more", 0));
        }
        assert!(TimeHost::with_project(RATE, 120.0, bad).is_err());
        // An empty project has nothing armed.
        let none = TimeHost::with_project(RATE, 120.0, mpfree_seq::Project::new()).unwrap();
        assert_eq!(none.state().armed, None);
    }

    // ------------------------------- every synth path records notes (E4)

    /// Step sequencer refactor E4: a second armed track whose instrument
    /// plays notes gets a new clip of notes, and a key routed to it, which
    /// comes as the pad that plays its note there, records the notes that
    /// pad plays (a chord pad's every note), each with the key's pressure.
    /// A pad that plays no note there is counted, not recorded.
    #[test]
    fn a_second_armed_synth_track_records_the_notes_its_routed_pads_play() {
        use mpfree_seq::Holds;
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        // Track 2 plays a synth: its pad 6 is a C major chord, pad 3 D4.
        let mut rows = PadRows::PADS;
        rows.set(5, &[64, 67, 60]);
        rows.set(2, &[62]);
        h.set_notes_tracks(1 << 1);
        h.set_pad_rows(1, rows);
        h.act(TimeAction::Arm(Some(0))).unwrap();
        h.act(TimeAction::ArmAlso(1 << 1)).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        run(&mut h, &mut pads, BAR / 2 + 4 * u64::from(PERIOD), &[]);
        let on = |note| Event::NoteOn { note, velocity: 90 };
        let keys = [
            (BAR + 2000, on(5)),
            (BAR + 4500, Event::Pressure { note: 5, value: 40 }),
            (BAR + 6000, Event::NoteOff { note: 5 }),
            (BAR + 7000, on(2)),
            (BAR + 8000, Event::NoteOff { note: 2 }),
            // Past the 16 pads a track's pad map holds.
            (BAR + 9000, on(20)),
            (BAR + 9500, Event::NoteOff { note: 20 }),
        ];
        while pads.frame < 2 * BAR {
            let end = pads.frame + u64::from(PERIOD);
            for &(at, ev) in &keys {
                if (pads.frame..end).contains(&at) {
                    h.routed_played(1, (at - pads.frame) as u32, ev);
                }
            }
            run(&mut h, &mut pads, end, &[]);
        }
        let clip = h.control.project.tracks[1]
            .slots
            .clips()
            .next()
            .unwrap()
            .clone();
        assert_eq!(clip.holds, Holds::Notes);
        let mut notes: Vec<(u8, Vec<u8>)> = clip
            .notes
            .iter()
            .map(|n| (n.note, clip.touch_of(n).iter().map(|p| p.value).collect()))
            .collect();
        notes.sort();
        assert_eq!(
            notes,
            vec![(60, vec![40]), (62, vec![]), (64, vec![40]), (67, vec![40])]
        );
        assert_eq!(h.dropped(), 1, "pad 21 has no note: counted");
    }

    /// Refactor E4: CAPTURE keeps what a take would record. On a track
    /// whose instrument plays notes a new clip holds the notes played (a
    /// chord pad's every note); over a playing clip, the notes go in in
    /// that clip's kind, as an overdub would: the pad into a clip of pads,
    /// the chord into a clip of notes.
    #[test]
    fn capture_on_a_synth_track_keeps_what_a_take_there_would() {
        use mpfree_seq::Holds;
        let capture = |playing: Option<Holds>| {
            let mut h = TimeHost::new(RATE, 120.0).unwrap();
            let mut pads = Fake {
                chord: vec![0, 4, 7],
                ..Fake::default()
            };
            let mut arps = TrackArps::new(RATE, 120.0);
            h.set_notes_tracks(1);
            if let Some(holds) = playing {
                let mut c = Clip::new("beat", 3840);
                c.holds = holds;
                h.control.project.tracks[0].slots[0] = Some(c);
                h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
            }
            h.command(Command::Play);
            let on = Event::NoteOn {
                note: 4,
                velocity: 100,
            };
            let off = Event::NoteOff { note: 4 };
            while pads.frame < 3 * BAR + BAR / 2 {
                let end = pads.frame + u64::from(PERIOD);
                for (at, e) in [(BAR + 1000, on), (BAR + 3000, off)] {
                    if (pads.frame..end).contains(&at) {
                        arps.pad(0, (at - pads.frame) as u32, e);
                    }
                }
                h.period_arps(PERIOD, &mut pads, &mut arps);
                pads.frame += u64::from(PERIOD);
            }
            let line = h.act(TimeAction::Capture).unwrap();
            assert_eq!(h.dropped(), 0);
            let clip = h.project().clip(0, 0).unwrap();
            let mut notes: Vec<u8> = clip.notes.iter().map(|n| n.note).collect();
            notes.sort();
            (line, clip.holds, notes)
        };
        let (line, holds, notes) = capture(None);
        assert!(
            line.starts_with("capture 1: 3 notes")
                && line.ends_with("new clip in slot 1 of Track 1"),
            "{line}"
        );
        assert_eq!(
            (holds, notes),
            (Holds::Notes, vec![4, 8, 11]),
            "the pad's chord as notes"
        );
        let (line, holds, notes) = capture(Some(Holds::Hits));
        assert!(
            line.starts_with("capture 1: 1 notes")
                && line.ends_with("into the clip in slot 1 of Track 1"),
            "{line}"
        );
        assert_eq!(
            (holds, notes),
            (Holds::Hits, vec![4]),
            "the pad, as an overdub records it"
        );
        let (line, holds, notes) = capture(Some(Holds::Notes));
        assert!(line.starts_with("capture 1: 3 notes"), "{line}");
        assert_eq!((holds, notes), (Holds::Notes, vec![4, 8, 11]));
    }

    /// Refactor E4: live erase on a clip of notes takes out the notes the
    /// held pad plays (a chord pad's every note), not the note with the
    /// pad's number.
    #[test]
    fn live_erase_on_a_clip_of_notes_takes_out_the_notes_its_pad_plays() {
        use mpfree_seq::{Holds, Note};
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let mut rows = PadRows::PADS;
        rows.set(0, &[64, 67, 60]);
        rows.set(1, &[62]);
        h.set_notes_tracks(1);
        h.set_pad_rows(0, rows);
        let mut c = Clip::new("chords", 3840);
        c.holds = Holds::Notes;
        // Note 0 is a pitch here, not pad 1: it stays.
        c.notes = vec![
            Note::new(0, 100, 60, 100),
            Note::new(0, 100, 64, 100),
            Note::new(0, 100, 67, 100),
            Note::new(960, 100, 62, 100),
            Note::new(1920, 100, 0, 100),
        ];
        h.control.project.tracks[0].slots[0] = Some(c);
        h.command(Command::SetLoop(Some((0, 3840))));
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.erase_sound(0, true);
        assert!(h.erasing() && h.erases(0) && !h.erases(1));
        run(&mut h, &mut pads, 2 * BAR - 1000, &[]);
        h.erase_none();
        let mut notes: Vec<u8> = h
            .project()
            .clip(0, 0)
            .unwrap()
            .notes
            .iter()
            .map(|n| n.note)
            .collect();
        notes.sort();
        assert_eq!(notes, vec![0, 62], "pad 1's chord came out");
        assert_eq!(h.dropped(), 0);
    }

    /// Cues that wait for their audio while takes stream from their files
    /// (the audio tracks rethink's phase 2; `time/audio_wait.rs`).
    mod audio_waits {
        use super::*;
        use mpfree_play::Cue;
        use mpfree_seq::{AudioClip, ClipRegion, LaunchGrid, RecId};
        use mpfree_time_ui::view::AudioWaitCounts;

        /// A bar in ticks.
        const TICKS: i64 = 3840;
        const DRUMS: usize = 0;
        const VOCAL: usize = 1;
        /// A second at the tests' rate, in frames.
        const SECOND: i64 = RATE as i64;

        /// An audio clip `bars` long: recording `rec` from frame 0 on its
        /// first bar, and recording 9 from frame 100,000 on its second.
        fn audio_clip(name: &str, rec: u128, bars: i64) -> Clip {
            let mut c = Clip::new(name, bars * TICKS);
            let mut a = AudioClip::of(RecId(rec), 0, 200_000, TICKS, 120.0);
            if bars > 1 {
                a.regions
                    .push(ClipRegion::new(TICKS, 2 * TICKS, RecId(9), 100_000));
            }
            c.audio = Some(a);
            c
        }

        fn beat(name: &str) -> Clip {
            let mut c = Clip::new(name, TICKS);
            for k in 0..4 {
                c.add_note(mpfree_seq::Note::new(k * 960, 240, 36, 100))
                    .unwrap();
            }
            c
        }

        /// Drums playing their first clip and a vocal (an audio track) with
        /// Verse (2 bars) and Chorus (1 bar), a quarter of a bar into the
        /// song, with waits on or off.
        fn host(waits: bool) -> (TimeHost, Fake) {
            let mut p = Project::new();
            let mut drums = Track::new("Drums", 1);
            drums.slots[0] = Some(beat("Beat"));
            drums.slots[1] = Some(beat("Fill"));
            let mut vocal = Track::new_audio("Vocal", 2);
            vocal.slots[0] = Some(audio_clip("Verse", 1, 2));
            vocal.slots[1] = Some(audio_clip("Chorus", 2, 1));
            p.tracks.push(drums);
            p.tracks.push(vocal);
            let mut h = TimeHost::with_project(RATE, 120.0, p).unwrap();
            h.act(TimeAction::Launch {
                track: DRUMS,
                slot: 0,
            })
            .unwrap();
            h.set_audio_waits(waits);
            h.command(Command::Play);
            let mut pads = Fake::default();
            run(&mut h, &mut pads, BAR / 4, &[]);
            (h, pads)
        }

        /// The clip `track` plays and the song tick it started on.
        fn playing(h: &TimeHost, track: usize) -> Option<(usize, i64)> {
            h.control
                .launches
                .iter()
                .find(|l| l.track == track)
                .map(|l| (l.slot, l.start))
        }

        fn cued_in_view(h: &TimeHost, track: usize) -> Option<Option<usize>> {
            h.control.view().unwrap().cue(track)
        }

        fn counts(h: &TimeHost) -> (u64, u64, u64, u64, u64, u32) {
            let c = h.state().audio_waits;
            (
                c.kept,
                c.held,
                c.moved,
                c.cancelled,
                c.given_back,
                c.waiting,
            )
        }

        fn launch(h: &mut TimeHost, track: usize, slot: usize) {
            h.act(TimeAction::Launch { track, slot }).unwrap();
        }

        #[test]
        fn with_streaming_off_or_no_audio_a_cue_goes_as_it_did() {
            let (mut h, mut pads) = host(false);
            launch(&mut h, VOCAL, 0);
            assert!(h.audio_needs().is_empty());
            assert_eq!(cued_in_view(&h, VOCAL), Some(Some(0)));
            run(&mut h, &mut pads, BAR + BAR / 8, &[]);
            assert_eq!(playing(&h, VOCAL), Some((0, TICKS)));
            assert_eq!(h.state().audio_waits, AudioWaitCounts::default());
            // Waits on: a cue that launches no audio still goes at once.
            h.set_audio_waits(true);
            launch(&mut h, DRUMS, 1);
            assert!(h.audio_needs().is_empty());
            assert_eq!(cued_in_view(&h, DRUMS), Some(Some(1)));
            run(&mut h, &mut pads, 2 * BAR + BAR / 8, &[]);
            assert_eq!(playing(&h, DRUMS), Some((1, 2 * TICKS)));
            assert_eq!(counts(&h), (0, 0, 0, 0, 0, 0));
        }

        #[test]
        fn a_cue_held_in_time_lands_on_its_line() {
            let (mut h, mut pads) = host(true);
            launch(&mut h, VOCAL, 0);
            // Each region's first second, from its start.
            let needs = h.audio_needs();
            let key = needs[0].cue;
            let got: Vec<_> = needs.iter().map(|n| (n.cue, n.rec, n.from, n.to)).collect();
            assert_eq!(
                got,
                vec![
                    (key, RecId(1), 0, SECOND),
                    (key, RecId(9), 100_000, 100_000 + SECOND)
                ]
            );
            // Cued, as the screens show it, but out of the view.
            assert_eq!(
                h.control.cues,
                vec![Cue {
                    track: VOCAL,
                    slot: Some(0)
                }]
            );
            assert_eq!(cued_in_view(&h, VOCAL), None);
            assert_eq!(counts(&h), (1, 0, 0, 0, 0, 1));
            run(&mut h, &mut pads, BAR / 2, &[]);
            assert!(h.audio_held(key, true));
            assert_eq!(cued_in_view(&h, VOCAL), Some(Some(0)));
            run(&mut h, &mut pads, BAR + BAR / 8, &[]);
            assert_eq!(playing(&h, VOCAL), Some((0, TICKS)), "on its line");
            assert!(h.audio_needs().is_empty());
            assert_eq!(counts(&h), (1, 1, 0, 0, 0, 0));
            // A key no longer waiting answers false.
            assert!(!h.audio_held(key, true));
        }

        #[test]
        fn a_cue_not_held_moves_twice_then_is_cancelled() {
            let (mut h, mut pads) = host(true);
            launch(&mut h, VOCAL, 1);
            let key = h.audio_needs()[0].cue;
            assert!(h.audio_held(key, false), "not held yet: it waits");
            run(&mut h, &mut pads, BAR + BAR / 8, &[]);
            assert_eq!(playing(&h, VOCAL), None);
            assert_eq!(h.control.cues.len(), 1, "still cued");
            let said = h.take_later_note().unwrap();
            assert!(
                said.contains("audio not ready: Vocal: Chorus moves to bar 3"),
                "{said}"
            );
            run(&mut h, &mut pads, 2 * BAR + BAR / 8, &[]);
            let said = h.take_later_note().unwrap();
            assert!(said.contains("moves to bar 4"), "{said}");
            assert_eq!(counts(&h), (1, 0, 2, 0, 0, 1));
            run(&mut h, &mut pads, 3 * BAR + BAR / 8, &[]);
            let said = h.take_later_note().unwrap();
            assert!(
                said.contains("audio not ready: Vocal: Chorus cancelled after 2 moves"),
                "{said}"
            );
            assert!(h.control.cues.is_empty());
            assert!(h.audio_needs().is_empty());
            assert_eq!(counts(&h), (1, 0, 2, 1, 0, 0));
            run(&mut h, &mut pads, 5 * BAR, &[]);
            assert_eq!(playing(&h, VOCAL), None, "never launched");
        }

        #[test]
        fn a_hold_given_back_moves_the_cue_and_a_new_hold_lands_it() {
            let (mut h, mut pads) = host(true);
            launch(&mut h, VOCAL, 0);
            let key = h.audio_needs()[0].cue;
            assert!(h.audio_held(key, true));
            run(&mut h, &mut pads, BAR * 3 / 4, &[]);
            assert!(h.audio_given_back(key));
            assert_eq!(cued_in_view(&h, VOCAL), None, "out of the view again");
            let said = h.take_later_note().unwrap();
            assert!(said.contains("Vocal: Verse moves to bar 3"), "{said}");
            run(&mut h, &mut pads, BAR + BAR / 8, &[]);
            assert_eq!(playing(&h, VOCAL), None);
            assert!(h.audio_held(key, true));
            run(&mut h, &mut pads, 2 * BAR + BAR / 8, &[]);
            assert_eq!(playing(&h, VOCAL), Some((0, 2 * TICKS)));
            assert_eq!(counts(&h), (1, 2, 1, 0, 1, 0));
            // Nothing held waits under it now.
            assert!(!h.audio_given_back(key));
        }

        #[test]
        fn a_scenes_cues_wait_and_land_as_one() {
            let (mut h, mut pads) = host(true);
            h.act(TimeAction::LaunchScene(1)).unwrap();
            let needs = h.audio_needs();
            assert_eq!(needs.len(), 1, "the chorus's one region");
            let key = needs[0].cue;
            // The drums' cue launches no audio but waits with the scene.
            assert_eq!(cued_in_view(&h, DRUMS), None);
            assert_eq!(cued_in_view(&h, VOCAL), None);
            assert_eq!(h.state().scene_cued, Some(1));
            run(&mut h, &mut pads, BAR + BAR / 8, &[]);
            assert_eq!(playing(&h, DRUMS), Some((0, 0)), "the beat plays on");
            assert_eq!(playing(&h, VOCAL), None);
            let said = h.take_later_note().unwrap();
            assert!(said.contains("scene 2 moves to bar 3"), "{said}");
            assert!(h.audio_held(key, true));
            assert_eq!(cued_in_view(&h, DRUMS), Some(Some(1)));
            run(&mut h, &mut pads, 2 * BAR + BAR / 8, &[]);
            assert_eq!(playing(&h, DRUMS), Some((1, 2 * TICKS)));
            assert_eq!(playing(&h, VOCAL), Some((1, 2 * TICKS)));
            assert_eq!(h.state().scene_playing, Some(1));
            assert_eq!(counts(&h), (1, 1, 1, 0, 0, 0));
        }

        #[test]
        fn a_scene_cancelled_takes_all_its_cues_back() {
            let (mut h, mut pads) = host(true);
            h.act(TimeAction::LaunchScene(1)).unwrap();
            run(&mut h, &mut pads, 3 * BAR + BAR / 8, &[]);
            let said = h.take_later_note().unwrap();
            assert!(said.contains("scene 2 cancelled after 2 moves"), "{said}");
            assert!(h.control.cues.is_empty());
            assert_eq!(h.state().scene_cued, None);
            assert_eq!(playing(&h, DRUMS), Some((0, 0)));
            assert_eq!(playing(&h, VOCAL), None);
            // A launch after it isn't taken for the scene.
            launch(&mut h, DRUMS, 1);
            run(&mut h, &mut pads, 4 * BAR + BAR / 8, &[]);
            assert_eq!(playing(&h, DRUMS), Some((1, 4 * TICKS)));
            assert_eq!(h.state().scene_playing, None);
        }

        #[test]
        fn at_now_a_cue_lands_once_held_and_gives_up_after_two_bars() {
            let (mut h, mut pads) = host(true);
            for slot in 0..2 {
                h.control.project.tracks[VOCAL].slots[slot]
                    .as_mut()
                    .unwrap()
                    .launch = Some(LaunchGrid::Now);
            }
            launch(&mut h, VOCAL, 0);
            let key = h.audio_needs()[0].cue;
            run(&mut h, &mut pads, BAR / 2, &[]);
            assert_eq!(playing(&h, VOCAL), None);
            let at = h.state().transport.position;
            assert!(h.audio_held(key, true));
            run(&mut h, &mut pads, BAR / 2 + BAR / 8, &[]);
            let (slot, start) = playing(&h, VOCAL).unwrap();
            assert_eq!(slot, 0);
            assert!((at..at + 200).contains(&start), "{start} from {at}");
            // Never held: it gives up two bars after it was decided.
            launch(&mut h, VOCAL, 1);
            let decided = pads.frame;
            run(&mut h, &mut pads, decided + 2 * BAR - BAR / 8, &[]);
            assert_eq!(h.control.cues.len(), 1, "still waiting");
            run(&mut h, &mut pads, decided + 2 * BAR + BAR / 8, &[]);
            let said = h.take_later_note().unwrap();
            assert!(
                said.contains("Vocal: Chorus cancelled after 2 bars"),
                "{said}"
            );
            assert!(h.control.cues.is_empty());
            assert_eq!(playing(&h, VOCAL).map(|p| p.0), Some(0));
        }

        #[test]
        fn a_legato_cue_needs_its_audio_from_the_place_it_takes_over() {
            let (mut h, mut pads) = host(true);
            // The verse plays from bar 1; the chorus, legato on a 1/4 grid
            // and two bars long, takes over its place on the next beat.
            let mut chorus = audio_clip("Chorus", 2, 2);
            chorus.legato = true;
            chorus.launch = Some(LaunchGrid::Ticks(960));
            h.control.project.tracks[VOCAL].slots[1] = Some(chorus);
            h.control.project.give_clip_ids();
            h.control.launches.push(mpfree_play::Launch {
                track: VOCAL,
                slot: 0,
                origin: 0,
                start: 0,
            });
            h.control.compile().unwrap();
            launch(&mut h, VOCAL, 1);
            let needs = h.audio_needs();
            let got: Vec<_> = needs.iter().map(|n| (n.rec, n.from, n.to)).collect();
            // The next beat is tick 1,920, a second into the first region:
            // from there to its end; the second region from its start.
            assert_eq!(
                got,
                vec![
                    (RecId(2), SECOND, 2 * SECOND),
                    (RecId(9), 100_000, 100_000 + SECOND)
                ]
            );
            assert!(h.audio_held(needs[0].cue, true));
            run(&mut h, &mut pads, BAR / 2 + BAR / 8, &[]);
            assert_eq!(playing(&h, VOCAL).map(|p| p.0), Some(1));
        }

        #[test]
        fn a_loop_jump_aims_a_waiting_cue_at_the_next_line_without_a_move() {
            let (mut h, mut pads) = host(true);
            h.command(Command::SetLoop(Some((0, 2 * TICKS))));
            launch(&mut h, VOCAL, 0);
            let key = h.audio_needs()[0].cue;
            // Bar 2 plays without it (one move), then the loop goes back to
            // bar 1: it aims at bar 2 again, still one move.
            run(&mut h, &mut pads, 2 * BAR + BAR / 8, &[]);
            assert_eq!(counts(&h).2, 1);
            assert!(h.audio_held(key, true));
            run(&mut h, &mut pads, 3 * BAR + BAR / 8, &[]);
            assert_eq!(playing(&h, VOCAL), Some((0, TICKS)));
            assert_eq!(counts(&h), (1, 1, 1, 0, 0, 0));
        }
    }

    /// Audio clips in the grid (the audio tracks rethink's phase 2, step
    /// 2c): To a cell's and Put in a cell's first empty cell, and audio
    /// clips moved, copied and pasted between audio tracks only.
    mod audio_cells {
        use super::*;
        use mpfree_seq::{AudioClip, RecId, ROWS};

        const DRUMS: usize = 0;
        const VOCAL: usize = 1;
        const GUITAR: usize = 2;

        fn audio_clip(name: &str, rec: u128) -> Clip {
            let mut c = Clip::new(name, 3840);
            c.audio = Some(AudioClip::of(RecId(rec), 0, 88_200, 3840, 120.0));
            c
        }

        /// Drums with a beat, Vocal (audio) with Verse, Guitar (audio)
        /// empty.
        fn host() -> TimeHost {
            let mut p = Project::new();
            let mut drums = Track::new("Drums", 1);
            drums.slots[0] = Some(Clip::new("Beat", 3840));
            let mut vocal = Track::new_audio("Vocal", 2);
            vocal.slots[0] = Some(audio_clip("Verse", 1));
            p.tracks.push(drums);
            p.tracks.push(vocal);
            p.tracks.push(Track::new_audio("Guitar", 3));
            TimeHost::with_project(RATE, 120.0, p).unwrap()
        }

        fn name_at(h: &TimeHost, track: usize, slot: usize) -> Option<String> {
            h.control.project.clip(track, slot).map(|c| c.name.clone())
        }

        #[test]
        fn an_audio_clip_goes_in_its_tracks_first_empty_cell_and_undo_takes_it_out() {
            let mut h = host();
            assert_eq!(h.put_audio_clip(VOCAL, audio_clip("Hook", 2)), Ok(1));
            assert_eq!(h.put_audio_clip(VOCAL, audio_clip("Ad libs", 3)), Ok(2));
            assert_eq!(name_at(&h, VOCAL, 2).as_deref(), Some("Ad libs"));
            let ids: Vec<_> = (0..3)
                .filter_map(|s| h.control.project.clip(VOCAL, s).map(|c| c.id))
                .collect();
            assert!(ids[0] != ids[1] && ids[1] != ids[2], "each its own id");
            // UNDO takes the last one out; the cell is free again.
            h.act(TimeAction::Undo).unwrap();
            assert_eq!(name_at(&h, VOCAL, 2), None);
            assert_eq!(h.put_audio_clip(VOCAL, audio_clip("Ad libs", 3)), Ok(2));
            // A full track, a note track and a note clip are refused, and
            // nothing changes.
            for _ in 3..ROWS {
                h.put_audio_clip(VOCAL, audio_clip("More", 4)).unwrap();
            }
            let full = h.put_audio_clip(VOCAL, audio_clip("One more", 5));
            assert_eq!(full, Err("Vocal has no empty cell".to_string()));
            let drums = h.put_audio_clip(DRUMS, audio_clip("Hook", 2));
            assert_eq!(drums, Err("there is no audio track 1".to_string()));
            let notes = h.put_audio_clip(GUITAR, Clip::new("Notes", 3840));
            assert_eq!(notes, Err("Guitar takes audio clips only".to_string()));
            assert!(h.control.project.tracks[GUITAR]
                .slots
                .iter()
                .next()
                .is_none());
        }

        /// Phase 2, step 2d (the Arrange thread's half): Scenes → song and
        /// List → song play the rows offline through Rec song, and the
        /// copy keeps its audio clips as audio clips, so an audio clip's
        /// launch comes out as its block on its own track's lane, linked to
        /// the clip, with no audio tracks host attached.
        #[test]
        fn the_rows_played_offline_keep_their_audio_clips_blocks() {
            for a in [TimeAction::ScenesToSong, TimeAction::ListToSong] {
                let mut h = host();
                if a == TimeAction::ListToSong {
                    h.act(TimeAction::ListAdd).unwrap();
                }
                let verse = h.control.project.clip(VOCAL, 0).unwrap().id;
                let said = h.act(a.clone()).unwrap();
                assert!(
                    said.ends_with("exact. UNDO takes it back."),
                    "{a:?}: {said}"
                );
                let blocks: Vec<_> = h
                    .control
                    .project
                    .arrangement
                    .placements
                    .iter()
                    .map(|p| (p.track, p.start, p.length))
                    .collect();
                assert_eq!(blocks, [(DRUMS, 0, 3840), (VOCAL, 0, 3840)], "{a:?}");
                let p = &h.control.project.arrangement.placements[1];
                assert_eq!(p.clip, verse, "{a:?}: the Verse clip itself");
            }
        }

        /// The audio tracks, as far as a song edit tied to them goes: the
        /// steps UNDO and REDO took back and brought back.
        #[derive(Default)]
        struct Tied {
            undone: Vec<(u32, bool)>,
            regions: Vec<(mpfree_seq::TrackId, Vec<mpfree_seq::ClipRegion>)>,
        }

        impl SongAudio for Tied {
            fn busy(&self) -> Option<String> {
                None
            }
            fn refused(&self, _: &mpfree_audiotracks::Retime) -> Option<String> {
                None
            }
            fn moves(&self, _: &mpfree_audiotracks::Retime) -> bool {
                false
            }
            fn retime(&mut self, _: u32, _: mpfree_audiotracks::Retime) -> bool {
                false
            }
            fn step(&self, _: u32, _: bool) -> mpfree_audiotracks::SongStep {
                mpfree_audiotracks::SongStep::Next
            }
            fn song_undo(&mut self, song: u32, undo: bool) {
                self.undone.push((song, undo));
            }
            fn regions(&self) -> Vec<(mpfree_seq::TrackId, Vec<mpfree_seq::ClipRegion>)> {
                self.regions.clone()
            }
        }

        /// Phase 2, step 2d: what Rec song puts into the song is a song
        /// edit UNDO takes back; the blocks it put on audio lanes are handed
        /// to the audio tracks once, with the edit's number, to replace the
        /// regions under them; once they say they made that step, the
        /// song's UNDO and REDO take it back and bring it back too.
        #[test]
        fn rec_songs_blocks_are_one_undo_and_handed_to_the_audio_tracks() {
            let mut h = host();
            let mut pads = Fake::default();
            h.act(TimeAction::LaunchScene(0)).unwrap();
            h.act(TimeAction::RecSong(true)).unwrap();
            h.command(Command::Play);
            run(&mut h, &mut pads, 2 * BAR - 1000, &[]);
            h.command(Command::Stop);
            run(&mut h, &mut pads, 2 * BAR, &[]);
            let note = h.take_later_note().unwrap();
            assert!(note.starts_with("rec song: 2 placements"), "{note}");
            let lanes = |h: &TimeHost| -> Vec<usize> {
                let song = &h.control.project.arrangement;
                song.placements.iter().map(|p| p.track).collect()
            };
            assert_eq!(lanes(&h), [DRUMS, VOCAL]);
            let vocal = h.control.project.tracks[VOCAL].id;
            let st = h.state();
            let took = st.done.last().unwrap();
            assert_eq!(took.word(), "rec song");
            let n = took.number;
            let end = h.control.project.arrangement.placements[1].end();
            assert_eq!(h.take_song_placed(), Some((n, vec![(vocal, 0, end)])));
            assert_eq!(h.take_song_placed(), None, "once");
            // Untied, UNDO and REDO are the song's alone.
            let mut tied = Tied::default();
            h.act_with_audio(TimeAction::Undo, &mut tied).unwrap();
            assert!(lanes(&h).is_empty());
            h.act_with_audio(TimeAction::Redo, &mut tied).unwrap();
            assert_eq!(lanes(&h), [DRUMS, VOCAL]);
            assert!(tied.undone.is_empty());
            // Tied: the audio tracks' step goes back and comes back too.
            assert!(h.tie_audio(n));
            h.act_with_audio(TimeAction::Undo, &mut tied).unwrap();
            h.act_with_audio(TimeAction::Redo, &mut tied).unwrap();
            assert_eq!(tied.undone, [(n, true), (n, false)]);
            assert!(!h.tie_audio(n + 100), "no such edit");
        }

        /// Phase 2, step 2d: Song → scenes asks the audio tracks for the
        /// song's regions, through act_with_audio, and each section's part
        /// of a lane becomes an audio clip in the section's row; a section
        /// a clip can't hold is refused by track and bars, and nothing
        /// changes. Song → scenes through act alone has no regions.
        #[test]
        fn song_to_scenes_brings_the_songs_regions_in_as_audio_clips() {
            use mpfree_seq::ClipRegion;
            let bar = 3840;
            let mut h = host();
            let p = &mut h.control.project;
            let beat = mpfree_seq::Placement::of(&p.tracks, DRUMS, 0, 0, 2 * bar);
            p.arrangement.place(beat).unwrap();
            p.arrangement.mark(bar, "Chorus").unwrap();
            h.control.compile().unwrap();
            let guitar = h.control.project.tracks[GUITAR].id;
            let take = ClipRegion::new(bar / 2, 3 * bar / 2, RecId(5), 0);
            let mut tied = Tied {
                regions: vec![(guitar, vec![take])],
                ..Tied::default()
            };
            let said = h
                .act_with_audio(TimeAction::SongToScenes, &mut tied)
                .unwrap();
            assert!(said.starts_with("song → scenes: rows 2 to 3"), "{said}");
            let audio = |h: &TimeHost, row: usize| {
                let c = h.control.project.clip(GUITAR, row).unwrap();
                (c.name.clone(), c.audio.clone().unwrap().regions)
            };
            let (name, regions) = audio(&h, 1);
            assert_eq!(name, "Guitar 1");
            assert_eq!(regions.len(), 1);
            assert_eq!((regions[0].start, regions[0].end), (bar / 2, bar));
            let (name, regions) = audio(&h, 2);
            assert_eq!(name, "Guitar 2");
            assert_eq!((regions[0].start, regions[0].end), (0, bar / 2));
            assert_eq!(regions[0].src, regions[0].into as i64, "on from the cut");
            // One UNDO takes the rows away.
            h.act(TimeAction::Undo).unwrap();
            assert!(h.control.project.clip(GUITAR, 1).is_none());
            // 17 regions in the first section: refused by name, nothing
            // changes.
            tied.regions = vec![(
                guitar,
                (0..17)
                    .map(|k| ClipRegion::new(k * 200, k * 200 + 100, RecId(5), 0))
                    .collect(),
            )];
            let before = h.control.project.clone();
            let err = h
                .act_with_audio(TimeAction::SongToScenes, &mut tied)
                .unwrap_err();
            assert_eq!(
                err,
                "song → scenes: nothing changed: Guitar, bar 1: 17 regions, and a clip holds 16. Split the section, or bounce it first."
            );
            assert_eq!(h.control.project, before);
            // Through act alone: no regions asked for, only the Beat.
            h.act(TimeAction::SongToScenes).unwrap();
            assert!(h.control.project.clip(GUITAR, 1).is_none());
            assert!(h.control.project.clip(DRUMS, 1).is_some());
        }

        /// Phase 2, step 2d: a take over a block cuts the block at the
        /// take's edges, as a song edit of its own whose number the audio
        /// tracks tie their step to, so the song's UNDO and REDO take their
        /// step back and bring it back. No block there, or a note track:
        /// nothing to cut, nothing tied.
        #[test]
        fn a_take_over_a_block_cuts_it_and_its_undo_is_tied_to_the_takes() {
            let bar = 3840;
            let mut h = host();
            let p = &mut h.control.project;
            let mut verse = mpfree_seq::Placement::of(&p.tracks, VOCAL, 0, 0, 4 * bar);
            verse.offset = 0;
            p.arrangement.place(verse).unwrap();
            let beat = mpfree_seq::Placement::of(&p.tracks, DRUMS, 0, 0, 4 * bar);
            p.arrangement.place(beat).unwrap();
            h.control.compile().unwrap();
            let id = |h: &TimeHost, t: usize| h.control.project.tracks[t].id;
            let lane = |h: &TimeHost, t: usize| -> Vec<(Tick, Tick, Tick)> {
                let song = &h.control.project.arrangement;
                song.placements
                    .iter()
                    .filter(|p| p.track == t)
                    .map(|p| (p.start, p.length, p.offset))
                    .collect()
            };
            let before = h.control.project.arrangement.clone();
            // Nowhere near a block, or on a note track: nothing.
            assert_eq!(h.audio_took(id(&h, GUITAR), (0, bar)), Ok(None));
            assert_eq!(h.audio_took(id(&h, DRUMS), (0, bar)), Ok(None));
            assert_eq!(h.audio_took(id(&h, VOCAL), (4 * bar, 5 * bar)), Ok(None));
            assert_eq!(h.control.project.arrangement, before);
            // Bars 2 and 3 recorded over Vocal's four-bar Verse: bar 1 and
            // bar 4 stay, bar 4 playing on from where it was.
            let (n, said) = h
                .audio_took(id(&h, VOCAL), (bar, 3 * bar))
                .unwrap()
                .expect("a block was there");
            assert_eq!(
                said,
                "song: the recording on Vocal at bar 2 cut the blocks under it"
            );
            assert_eq!(lane(&h, VOCAL), [(0, bar, 0), (3 * bar, bar, 0)]);
            assert_eq!(lane(&h, DRUMS), [(0, 4 * bar, 0)], "another lane stays");
            let st = h.state();
            let took = st.done.last().unwrap();
            assert_eq!((took.number, took.word()), (n, "recorded over"));
            assert_eq!(
                took.song,
                Some(mpfree_play::SongChange::Recorded {
                    track: VOCAL,
                    start: bar
                })
            );
            let cut = h.control.project.arrangement.clone();
            // The song's UNDO takes the audio tracks' step back with it,
            // and REDO brings it back.
            let mut tied = Tied::default();
            h.act_with_audio(TimeAction::Undo, &mut tied).unwrap();
            assert_eq!(h.control.project.arrangement, before);
            h.act_with_audio(TimeAction::Redo, &mut tied).unwrap();
            assert_eq!(h.control.project.arrangement, cut);
            assert_eq!(tied.undone, [(n, true), (n, false)]);
        }

        /// A take inside a block when the song holds all the blocks it
        /// can: there's no room to cut it in two, so the recording wins and
        /// the whole block goes, said and counted.
        #[test]
        fn a_take_inside_a_block_of_a_full_song_takes_the_whole_block() {
            let bar = 3840;
            let mut h = host();
            let p = &mut h.control.project;
            let n = mpfree_seq::arrange::MAX_PLACEMENTS - 1;
            for k in 0..n {
                let b = mpfree_seq::Placement::of(&p.tracks, DRUMS, 0, k as Tick * bar, bar);
                p.arrangement.placements.push(b);
            }
            let verse = mpfree_seq::Placement::of(&p.tracks, VOCAL, 0, 0, 4 * bar);
            p.arrangement.placements.push(verse);
            h.control.compile().unwrap();
            let vocal = h.control.project.tracks[VOCAL].id;
            let (_, said) = h.audio_took(vocal, (bar, 2 * bar)).unwrap().unwrap();
            assert!(said.contains("took the whole block"), "{said}");
            let song = &h.control.project.arrangement;
            assert!(song.placements.iter().all(|p| p.track == DRUMS));
            assert_eq!(song.placements.len(), n);
            assert!(h
                .control
                .describe()
                .contains("1 recordings over a block took the whole block"));
        }

        /// Phase 2, step 2d: an audio clip's block moves between audio
        /// tracks' lanes only, never over a region (the spans the audio
        /// tracks hand over), and a note clip's block never onto an audio
        /// track's lane. A refusal changes nothing.
        #[test]
        fn an_audio_block_stays_on_audio_lanes_and_never_covers_a_region() {
            let mut h = host();
            let bar = 3840;
            let p = &mut h.control.project;
            let verse = mpfree_seq::Placement::of(&p.tracks, VOCAL, 0, 0, bar);
            let beat = mpfree_seq::Placement::of(&p.tracks, DRUMS, 0, 0, bar);
            p.arrangement.place(verse).unwrap();
            p.arrangement.place(beat).unwrap();
            h.control.compile().unwrap();
            let id = |t: usize| h.control.project.tracks[t].id;
            let spans = vec![
                (id(VOCAL), Vec::new()),
                (id(GUITAR), vec![(2 * bar, 3 * bar), (5 * bar, 6 * bar)]),
            ];
            h.set_audio_spans(spans);
            let mv = |index, track, start| TimeAction::MovePlacement {
                index,
                track,
                start,
                copy: false,
            };
            let song = |h: &TimeHost| h.control.project.arrangement.clone();
            let before = song(&h);
            // Drums' beat is placement 0, Vocal's Verse 1.
            let err = h.act(mv(1, DRUMS, 4 * bar)).unwrap_err();
            assert_eq!(
                err,
                "song: Verse stays where it was: an audio clip's block goes only on an audio track, and Drums isn't one"
            );
            let err = h.act(mv(0, VOCAL, 4 * bar)).unwrap_err();
            assert_eq!(
                err,
                "song: Beat stays where it was: Vocal is an audio track, and it takes blocks of audio clips only"
            );
            let err = h.act(mv(1, GUITAR, 2 * bar - 960)).unwrap_err();
            assert_eq!(
                err,
                "song: Verse can't go over the region at bar 3: a block and a region never overlap on a lane"
            );
            assert_eq!(song(&h), before, "nothing changed");
            // A clear stretch of Guitar's lane takes it, and it plays the
            // same clip.
            h.act(mv(1, GUITAR, 3 * bar)).unwrap();
            let moved = song(&h).placements[1].clone();
            assert_eq!((moved.track, moved.start), (GUITAR, 3 * bar));
            assert_eq!(moved.clip, before.placements[1].clip);
            // Made longer over Guitar's next recording: refused.
            let stretch = |length| TimeAction::StretchPlacement { index: 1, length };
            let err = h.act(stretch(3 * bar)).unwrap_err();
            assert_eq!(
                err,
                "song: Verse can't run over the region at bar 6: a block and a region never overlap on a lane"
            );
            h.act(stretch(2 * bar)).unwrap();
            assert_eq!(song(&h).placements[1].length, 2 * bar);
        }

        /// An audio clip's card (layout spec 4j): a length cut keeps the
        /// frames it cuts and a longer one brings them back, the gain and
        /// the name are clip settings, and a row turned is one take.
        #[test]
        fn an_audio_clips_card_sets_its_length_gain_and_name_a_row_a_take() {
            use mpfree_time_ui::view::AudioSet;
            let mut h = host();
            let set = |row, set| TimeAction::AudioClip {
                track: VOCAL,
                slot: 0,
                row,
                set,
            };
            let clip = |h: &TimeHost| h.control.project.clip(VOCAL, 0).unwrap().clone();
            let region = |h: &TimeHost| clip(h).audio.unwrap().regions[0];
            let first = region(&h);
            // A beat shorter: the region ends there, the beat's frames (half
            // a second at 120 BPM) kept in its left.
            let said = h.act(set(0, AudioSet::Length(3 * 960))).unwrap();
            assert_eq!(said, "clip: Verse · Length 3 beats (UNDO takes it back)");
            let r = region(&h);
            assert_eq!((clip(&h).length, r.end, r.left), (3 * 960, 3 * 960, 22_050));
            assert_eq!(clip(&h).audio.unwrap().last_frame, 66_150);
            // Back a beat: it grows back into what it was cut from.
            h.act(set(0, AudioSet::Length(3840))).unwrap();
            assert_eq!(region(&h), first);
            assert_eq!(clip(&h).audio.unwrap().last_frame, 88_200);
            // A bar longer: silence after; the turn is one take.
            h.act(set(0, AudioSet::Length(2 * 3840))).unwrap();
            assert_eq!((clip(&h).length, region(&h)), (2 * 3840, first));
            assert_eq!(h.state().done.len(), 1, "the row turned: one take");
            // Gain turned: another take; the name another.
            h.act(set(5, AudioSet::Gain(2.0))).unwrap();
            let said = h.act(set(5, AudioSet::Gain(1.5))).unwrap();
            assert_eq!(said, "clip: Verse · Gain +3.5 dB (UNDO takes it back)");
            h.act(set(6, AudioSet::Name(" Hook ".into()))).unwrap();
            assert_eq!(clip(&h).name, "Hook");
            assert_eq!(h.state().done.len(), 3);
            let st = h.state();
            assert_eq!(
                mpfree_time_ui::view::take_title(&st, &st.done[1]),
                "Settings 2 · Vocal / 1"
            );
            // Refused: an empty name, a gain out of range, a once clip's
            // length; nothing changes.
            assert!(h.act(set(6, AudioSet::Name("  ".into()))).is_err());
            assert!(h.act(set(5, AudioSet::Gain(5.0))).is_err());
            let mut s = clip(&h).settings();
            s.once = true;
            h.act(TimeAction::ClipSettings {
                track: VOCAL,
                slot: 0,
                row: 8,
                settings: s,
            })
            .unwrap();
            assert!(h.act(set(0, AudioSet::Length(3840))).is_err());
            assert_eq!(clip(&h).length, 2 * 3840);
            // UNDO takes each back in turn.
            h.act(TimeAction::Undo).unwrap();
            h.act(TimeAction::Undo).unwrap();
            assert_eq!(clip(&h).name, "Verse");
            h.act(TimeAction::Undo).unwrap();
            assert_eq!(clip(&h).audio.unwrap().gain, 1.0);
            h.act(TimeAction::Undo).unwrap();
            assert_eq!((clip(&h).length, region(&h)), (3840, first));
        }

        #[test]
        fn audio_clips_move_copy_and_paste_between_audio_tracks_only() {
            let mut h = host();
            let drop = |from, to, copy| TimeAction::DropClip { from, to, copy };
            // A copy onto another audio track names the same recording,
            // with an id of its own.
            h.act(drop((VOCAL, 0), (GUITAR, 2), true)).unwrap();
            let (a, b) = (
                h.control.project.clip(VOCAL, 0).unwrap().clone(),
                h.control.project.clip(GUITAR, 2).unwrap().clone(),
            );
            assert_eq!(
                (a.audio.as_ref(), a.name.as_str()),
                (b.audio.as_ref(), "Verse")
            );
            assert_ne!(a.id, b.id);
            // Moved within the audio tracks.
            h.act(drop((GUITAR, 2), (VOCAL, 4), false)).unwrap();
            assert_eq!(name_at(&h, VOCAL, 4).as_deref(), Some("Verse"));
            assert_eq!(name_at(&h, GUITAR, 2), None);
            // Never between an audio track and a note track, either way.
            let e = h.act(drop((VOCAL, 0), (DRUMS, 3), false)).unwrap_err();
            assert_eq!(
                e,
                "clips: Drums plays notes: an audio clip goes on an audio track"
            );
            let e = h.act(drop((DRUMS, 0), (GUITAR, 3), true)).unwrap_err();
            assert_eq!(
                e,
                "clips: Guitar is an audio track: it takes audio clips only"
            );
            assert_eq!(
                (name_at(&h, DRUMS, 3), name_at(&h, GUITAR, 3)),
                (None, None)
            );
            // UNDO twice: the move, then the copy.
            h.act(TimeAction::Undo).unwrap();
            h.act(TimeAction::Undo).unwrap();
            assert_eq!(
                (name_at(&h, VOCAL, 4), name_at(&h, GUITAR, 2)),
                (None, None)
            );
            // COPY and paste the same way.
            h.act(TimeAction::CopyClip {
                track: VOCAL,
                slot: 0,
            })
            .unwrap();
            h.act(TimeAction::PasteClip {
                track: GUITAR,
                slot: 5,
            })
            .unwrap();
            assert_eq!(name_at(&h, GUITAR, 5).as_deref(), Some("Verse"));
            let e = h.act(TimeAction::PasteClip {
                track: DRUMS,
                slot: 5,
            });
            assert_eq!(
                e.unwrap_err(),
                "paste: Drums plays notes: an audio clip goes on an audio track"
            );
            h.act(TimeAction::CopyClip {
                track: DRUMS,
                slot: 0,
            })
            .unwrap();
            let e = h.act(TimeAction::PasteClip {
                track: VOCAL,
                slot: 5,
            });
            assert_eq!(
                e.unwrap_err(),
                "paste: Vocal is an audio track: it takes audio clips only"
            );
        }
    }

    // ------------------------------------------------ Continue (phase 1)

    /// A host with a one-bar riff of eight notes (a clip of notes) on
    /// track 1, row 1.
    fn riff_host() -> TimeHost {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut c = Clip::new("riff", 3840);
        c.holds = mpfree_seq::Holds::Notes;
        c.notes = [60u8, 63, 67, 65, 63, 62, 63, 60]
            .iter()
            .enumerate()
            .map(|(i, &n)| mpfree_seq::Note::new(i as Tick * 480, 240, n, 96))
            .collect();
        h.control.project.tracks[0].slots[0] = Some(c);
        h.control.project.give_clip_ids();
        h
    }

    fn continue_ask(bars: u8) -> TimeAction {
        continue_on(0, bars)
    }

    fn continue_on(track: usize, bars: u8) -> TimeAction {
        TimeAction::Continue(ContinueAction::Ask(Box::new(ContinueAsk {
            track,
            slot: 0,
            settings: mpfree_compose::Settings {
                bars,
                ..mpfree_compose::Settings::default()
            },
            kind: mpfree_compose::InstrumentKind::OneNote,
            lanes: mpfree_compose::Lanes::default(),
            scope: mpfree_time_ui::continuing::Scope::All,
            locked: Vec::new(),
        })))
    }

    fn continuing(h: &TimeHost) -> Option<Arc<Continuing>> {
        h.state().continuing
    }

    /// The hits clip `c`, launched at song tick 0 and looping, makes
    /// from frame `from` to `to`, as (frame, note).
    fn clip_hits(c: &Clip, from: u64, to: u64) -> Vec<(u64, usize)> {
        let frame = |t: Tick| t as u64 * BAR / 3840;
        let mut v = Vec::new();
        let mut pass = 0;
        while frame(pass * c.length) < to {
            for n in c.notes.iter().filter(|n| n.on) {
                let f = frame(pass * c.length + n.tick);
                if (from..to).contains(&f) {
                    v.push((f, usize::from(n.note)));
                }
            }
            pass += 1;
        }
        v.sort();
        v
    }

    /// The fake pads' hits from frame `from` to `to`, as (frame, note).
    fn hits_in(pads: &Fake, from: u64, to: u64) -> Vec<(u64, usize)> {
        let mut v: Vec<(u64, usize)> = pads
            .hits
            .iter()
            .filter(|h| (from..to).contains(&h.0))
            .map(|h| (h.0, h.1))
            .collect();
        v.sort();
        v
    }

    /// The same notes, each within a frame of where it should be.
    fn same_hits(got: &[(u64, usize)], want: &[(u64, usize)]) -> bool {
        got.len() == want.len()
            && got
                .iter()
                .zip(want)
                .all(|(g, w)| g.1 == w.1 && g.0.abs_diff(w.0) <= 1)
    }

    /// Periods, a millisecond apart, until Continue has a result and the
    /// bar line it comes in on (or it came in at once): the line.
    fn result_in(h: &mut TimeHost, pads: &mut Fake) -> Option<Tick> {
        until(h, pads, |h| {
            continuing(h).is_some_and(|c| c.line.is_some() || c.heard)
        });
        continuing(h).unwrap().line
    }

    /// Composing tools phase 1: Continue on a playing clip. The result
    /// comes from the composer's own thread and comes in on the next bar
    /// line, in place of the clip and in time with it (its launch keeps
    /// its origin); nothing is written until Keep, which is one take that
    /// plays on unchanged; UNDO takes it all back.
    #[test]
    fn continue_is_heard_from_the_next_bar_line_kept_as_one_take_and_undone() {
        let mut h = riff_host();
        let mut pads = Fake::default();
        let riff = h.project().clip(0, 0).unwrap().clone();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        assert_eq!(
            h.act(continue_ask(1)).unwrap(),
            "continue: 1 more bar of riff, Minimal"
        );
        let c = continuing(&h).unwrap();
        assert_eq!(
            (c.state, c.heard, c.position),
            (PreviewState::Preparing, false, None)
        );
        let at = result_in(&mut h, &mut pads).expect("on a bar line");
        let c = continuing(&h).unwrap();
        assert_eq!(
            (c.state, c.heard, c.position),
            (PreviewState::Ready, false, Some((1, 1)))
        );
        assert_eq!(at % 3840, 0);
        let heard = c.clip.clone().unwrap();
        assert_eq!((heard.length, c.new_from), (2 * 3840, 3840));
        assert!(heard.notes.iter().any(|n| n.tick >= 3840), "new notes");
        assert_eq!(h.project().clip(0, 0).unwrap(), &riff, "nothing written");
        assert_eq!(c.made.as_ref().unwrap().plan.len(), 2, "each bar's job");

        // Up to the line the riff; from it the clip as heard, on its
        // origin: a 2-bar clip whose second bar is the new one.
        let line = at as u64 * BAR / 3840;
        run(&mut h, &mut pads, line + 2 * BAR, &[]);
        let c = continuing(&h).unwrap();
        assert_eq!(
            (c.state, c.heard, c.line),
            (PreviewState::Auditioning, true, None)
        );
        assert!(same_hits(
            &hits_in(&pads, 0, line),
            &clip_hits(&riff, 0, line)
        ));
        let after = hits_in(&pads, line, line + 2 * BAR);
        assert!(
            same_hits(&after, &clip_hits(&heard, line, line + 2 * BAR)),
            "{after:?}"
        );
        assert_eq!(
            h.project().clip(0, 0).unwrap(),
            &riff,
            "still nothing written"
        );

        // Keep: one take, playing the same notes on.
        assert_eq!(
            h.act(TimeAction::Continue(ContinueAction::Keep)).unwrap(),
            "continue: 1 more bar of riff kept (UNDO takes it back)"
        );
        assert!(continuing(&h).is_none());
        let kept = h.project().clip(0, 0).unwrap().clone();
        assert_eq!((kept.length, &kept.notes), (heard.length, &heard.notes));
        let st = h.state();
        let row = st.done.last().unwrap();
        assert_eq!(row.replaced, Some(Replaced::Continue { bars: 1 }));
        assert!(row.short().starts_with("continue"), "{}", row.short());
        let from = line + 2 * BAR;
        run(&mut h, &mut pads, from + 2 * BAR, &[]);
        assert!(same_hits(
            &hits_in(&pads, from, from + 2 * BAR),
            &clip_hits(&kept, from, from + 2 * BAR)
        ));

        // UNDO: the clip as it was, length and all.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.project().clip(0, 0).unwrap(), &riff);
        assert_eq!(h.dropped(), 0);
        assert!(
            h.describe().contains("continue: 1 asked, 1 made"),
            "{}",
            h.describe()
        );
    }

    /// Try again while a result plays: it plays on until the next one has
    /// come and its line; ‹ › step back and forth on bar lines; Back
    /// brings the clip back on the next one, as it was.
    #[test]
    fn try_again_and_back_change_the_music_on_bar_lines() {
        let mut h = riff_host();
        let mut pads = Fake::default();
        let riff = h.project().clip(0, 0).unwrap().clone();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        h.act(continue_ask(1)).unwrap();
        let at = result_in(&mut h, &mut pads).unwrap();
        run(&mut h, &mut pads, at as u64 * BAR / 3840 + BAR / 4, &[]);
        let first = continuing(&h).unwrap().clip.clone().unwrap();
        assert!(continuing(&h).unwrap().heard);

        assert_eq!(
            h.act(continue_ask(1)).unwrap(),
            "continue: trying 1 more bar of riff again, Minimal (the last plays on until it's ready)"
        );
        let c = continuing(&h).unwrap();
        assert_eq!((c.state, c.heard), (PreviewState::Preparing, true));
        until(&mut h, &mut pads, |h| {
            continuing(h).is_some_and(|c| c.position == Some((2, 2)))
        });
        let c = continuing(&h).unwrap();
        let second = c.clip.clone().unwrap();
        assert!(c.heard, "the first plays on");
        let at = c.line.expect("the second on the next line");
        run(&mut h, &mut pads, at as u64 * BAR / 3840 + BAR / 4, &[]);
        let c = continuing(&h).unwrap();
        assert_eq!(
            (c.state, c.heard, c.line),
            (PreviewState::Auditioning, true, None)
        );
        assert_eq!(c.clip.as_deref(), Some(&*second));

        // ‹: the first again, on the next line.
        let said = h
            .act(TimeAction::Continue(ContinueAction::Step { back: true }))
            .unwrap();
        assert!(
            said.starts_with("continue: the result shown, heard from bar "),
            "{said}"
        );
        assert!(h
            .act(TimeAction::Continue(ContinueAction::Step { back: true }))
            .unwrap_err()
            .contains("no earlier result"));
        let c = continuing(&h).unwrap();
        assert_eq!(c.position, Some((1, 2)));
        let at = c.line.unwrap();
        run(&mut h, &mut pads, at as u64 * BAR / 3840 + BAR / 4, &[]);
        assert_eq!(continuing(&h).unwrap().clip.as_deref(), Some(&*first));

        // Back: the clip as it was, from the next line.
        let said = h.act(TimeAction::Continue(ContinueAction::Back)).unwrap();
        assert!(
            said.starts_with("continue: back to the clip as it was, heard from bar "),
            "{said}"
        );
        let c = continuing(&h).unwrap();
        assert_eq!((c.state, c.heard), (PreviewState::Cancelling, true));
        let line = c.line.unwrap() as u64 * BAR / 3840;
        run(&mut h, &mut pads, line + 2 * BAR, &[]);
        assert!(continuing(&h).is_none());
        assert!(same_hits(
            &hits_in(&pads, line, line + 2 * BAR),
            &clip_hits(&riff, line, line + 2 * BAR)
        ));
        assert_eq!(h.project().clip(0, 0).unwrap(), &riff);
        assert!(h.state().done.is_empty(), "Back writes nothing");
    }

    /// Any change to the clip while its result plays stops the result at
    /// once (the clip plays as it now is) and says so; REC stops one
    /// playing at once, and it stays ready for after.
    #[test]
    fn an_edit_or_rec_stops_the_new_bars_at_once() {
        let mut h = riff_host();
        let mut pads = Fake::default();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        h.act(continue_ask(2)).unwrap();
        let at = result_in(&mut h, &mut pads).unwrap();
        run(&mut h, &mut pads, at as u64 * BAR / 3840 + BAR / 4, &[]);
        assert!(continuing(&h).unwrap().heard);

        // An edit (it compiles): the result goes, the clip plays as edited.
        h.control.project.tracks[0].slots[0].as_mut().unwrap().notes[0].note = 72;
        h.control.compile().unwrap();
        let c = continuing(&h).unwrap();
        assert_eq!((c.state, c.heard), (PreviewState::Invalidated, false));
        assert_eq!(c.words.as_deref(), Some("The clip changed: try again?"));
        let edited = h.project().clip(0, 0).unwrap().clone();
        let from = pads.frame + BAR;
        run(&mut h, &mut pads, from + BAR, &[]);
        assert!(same_hits(
            &hits_in(&pads, from, from + BAR),
            &clip_hits(&edited, from, from + BAR)
        ));
        assert_eq!(h.project().clip(0, 0).unwrap(), &edited, "nothing written");
        assert!(h
            .act(TimeAction::Continue(ContinueAction::Keep))
            .unwrap_err()
            .contains("continue: "));

        // Again, then REC: the clip comes back before the take starts.
        h.act(continue_ask(1)).unwrap();
        let at = result_in(&mut h, &mut pads).unwrap();
        run(&mut h, &mut pads, at as u64 * BAR / 3840 + BAR / 4, &[]);
        assert!(continuing(&h).unwrap().heard);
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        let c = continuing(&h).unwrap();
        assert_eq!((c.state, c.heard), (PreviewState::Ready, false));
        let said = h.take_later_note().unwrap();
        assert!(
            said.ends_with("continue: REC brought the clip back; the new bars stay ready"),
            "{said}"
        );
        let to = pads.frame + 2 * BAR;
        run(&mut h, &mut pads, to, &[]);
        assert!(!continuing(&h).unwrap().heard, "none while recording");
        assert!(h
            .act(continue_ask(1))
            .unwrap_err()
            .contains("not while recording"));
        assert!(
            h.describe().contains("1 stopped by REC"),
            "{}",
            h.describe()
        );
        // REC replaced the riff's notes as it passed them (no overdub): a
        // change, so the result that waited went too.
        assert_eq!(continuing(&h).unwrap().state, PreviewState::Invalidated);
        assert!(
            h.describe().contains("2 stopped by a change"),
            "{}",
            h.describe()
        );
    }

    /// Stopped, a result comes in at once; a clip of pad hits, a loop's
    /// cell or an empty slot are refused with why.
    #[test]
    fn stopped_a_result_comes_in_at_once_and_what_cant_be_carried_on_says_why() {
        let mut h = riff_host();
        let mut pads = Fake::default();
        h.act(continue_ask(4)).unwrap();
        assert_eq!(result_in(&mut h, &mut pads), None, "at once");
        let c = continuing(&h).unwrap();
        assert_eq!((c.state, c.heard), (PreviewState::Auditioning, true));
        assert_eq!(c.clip.as_ref().unwrap().length, 5 * 3840);
        h.act(TimeAction::Continue(ContinueAction::Back)).unwrap();
        assert!(continuing(&h).is_none(), "stopped, Back is at once too");

        let mut beat = Clip::new("beat", 3840);
        beat.notes = vec![mpfree_seq::Note::new(0, 100, 3, 100)];
        h.control.project.tracks[1].slots[0] = Some(beat);
        let ask = |track| continue_on(track, 1);
        assert_eq!(
            h.act(ask(1)).unwrap_err(),
            "continue: This clip plays pads, so there's no tune to carry on."
        );
        assert_eq!(
            h.act(ask(2)).unwrap_err(),
            "continue: slot 1 of track 3 is empty"
        );
    }

    /// Phase 1 step 4: the clip card's Continue opens the note editor on
    /// the clip with Continue open, which asks at once; Done keeps it as
    /// one take. Opened again while it is open, it says so; another tool
    /// open refuses it; a clip of pad hits says why.
    #[test]
    fn continue_from_the_clip_card_opens_the_note_editor_and_asks() {
        use mpfree_time_ui::editor::{EditAction as E, ToolAct, CHOP_ROW};
        let mut h = riff_host();
        let mut pads = Fake::default();
        let open = TimeAction::Continue(ContinueAction::Open { track: 0, slot: 0 });
        assert!(h.state().edit.is_none());
        assert_eq!(
            h.act(open.clone()).unwrap(),
            "continue: 4 more bars of riff, Minimal"
        );
        let st = h.state();
        let e = st.edit.as_ref().expect("the note editor opened");
        assert_eq!((e.editor.track, e.editor.slot), (0, 0));
        assert!(e.editor.continuing_open());
        until(&mut h, &mut pads, |h| {
            continuing(h).is_some_and(|c| c.heard)
        });
        assert_eq!(
            h.act(open.clone()).unwrap(),
            "continue: open already, in the note editor"
        );
        // Done: one take of four more bars, the tool closed, the editor
        // open on the kept clip.
        let heard = continuing(&h).unwrap().clip.clone().unwrap();
        h.act(TimeAction::Edit(E::Tool(ToolAct::Done))).unwrap();
        assert!(continuing(&h).is_none());
        assert_eq!(h.project().clip(0, 0).unwrap().notes, heard.notes);
        assert_eq!(h.project().clip(0, 0).unwrap().length, 5 * 3840);
        // Another tool open: refused, with which.
        h.act(TimeAction::Edit(E::Pick {
            from: 0,
            to: 5 * 3840,
            low: 0,
            high: 127,
        }))
        .unwrap();
        h.act(TimeAction::Edit(E::Row {
            row: CHOP_ROW,
            delta: 1,
        }))
        .unwrap();
        assert_eq!(
            h.act(open).unwrap_err(),
            "continue: Done or Cancel Chop in the note editor first"
        );
        h.act(TimeAction::Edit(E::Tool(ToolAct::Cancel))).unwrap();
        // A clip of pad hits: why.
        let mut hits = h.project().clip(0, 0).unwrap().clone();
        hits.holds = mpfree_seq::Holds::Hits;
        h.control.project.tracks[1].slots[0] = Some(hits);
        h.control.project.give_clip_ids();
        assert_eq!(
            h.act(TimeAction::Continue(ContinueAction::Open {
                track: 1,
                slot: 0
            }))
            .unwrap_err(),
            "continue: This clip plays pads, so there's no tune to carry on."
        );
    }

    /// Phase 1 step 3: Try again on picked bars or notes changes only
    /// them (picked notes keep their places, lengths and velocity), and a
    /// locked bar stays through a try on everything.
    #[test]
    fn continue_tries_picked_bars_and_notes_and_keeps_locked_bars() {
        use mpfree_seq::Note;
        use mpfree_time_ui::continuing::Again;
        use mpfree_time_ui::editor::{EditAction as E, ToolAct, CONTINUE_ROW};
        const BAR: Tick = 3840;
        let mut h = riff_host();
        let mut pads = Fake::default();
        h.act(TimeAction::Edit(E::Open)).unwrap();
        h.act(TimeAction::Edit(E::Row {
            row: CONTINUE_ROW,
            delta: 1,
        }))
        .unwrap();
        until(&mut h, &mut pads, |h| {
            continuing(h).is_some_and(|c| c.heard)
        });
        let heard = |h: &TimeHost| continuing(h).unwrap().clip.clone().unwrap();
        let in_bar = |c: &Clip, b: Tick| -> Vec<Note> {
            c.notes
                .iter()
                .copied()
                .filter(|n| (b * BAR - 60..(b + 1) * BAR - 60).contains(&n.tick))
                .collect()
        };
        let outside = |c: &Clip, b: Tick| -> Vec<Note> {
            c.notes
                .iter()
                .copied()
                .filter(|n| !(b * BAR - 60..(b + 1) * BAR - 60).contains(&n.tick))
                .collect()
        };
        let one = heard(&h);
        // Bar 3 picked on the ruler, then Try again: only bar 3 moves.
        assert_eq!(
            h.act(TimeAction::Edit(E::ContinueBar(2))).unwrap(),
            "continue: bar 3 picked: Try again remakes it alone"
        );
        assert_eq!(
            h.act(TimeAction::Edit(E::Tool(ToolAct::Option))).unwrap(),
            "continue: trying bar 3 again (the rest stays as it is)"
        );
        until(&mut h, &mut pads, |h| {
            continuing(h).is_some_and(|c| c.position == Some((2, 2)) && c.line.is_none())
        });
        let two = heard(&h);
        assert_eq!(outside(&two, 2), outside(&one, 2));
        assert_eq!(continuing(&h).unwrap().tries[1].again, Some(Again::Bar(2)));
        // Locked, then Try again on everything: bar 3 stays.
        assert_eq!(
            h.act(TimeAction::Edit(E::ContinueLock)).unwrap(),
            "continue: bar 3 locked: it stays through every try"
        );
        let picks = h.state().edit.unwrap().editor.continue_picks.clone();
        assert_eq!((picks.bars, picks.locked), (vec![], vec![2]));
        h.act(TimeAction::Edit(E::Tool(ToolAct::Option))).unwrap();
        until(&mut h, &mut pads, |h| {
            continuing(h).is_some_and(|c| c.position == Some((3, 3)) && c.line.is_none())
        });
        let three = heard(&h);
        assert_eq!(in_bar(&three, 2), in_bar(&two, 2));
        assert_eq!(continuing(&h).unwrap().tries[2].again, None);
        // A new note picked on the roll, then Try again: only it may move,
        // in its place.
        let pick = in_bar(&three, 3)[0];
        assert_eq!(
            h.act(TimeAction::Edit(E::ContinueNote(pick.tick))).unwrap(),
            "continue: 1 note picked: Try again gives it another"
        );
        h.act(TimeAction::Edit(E::Tool(ToolAct::Option))).unwrap();
        until(&mut h, &mut pads, |h| {
            continuing(h).is_some_and(|c| c.position == Some((4, 4)) && c.line.is_none())
        });
        let four = heard(&h);
        let others = |c: &Clip| -> Vec<Note> {
            c.notes
                .iter()
                .copied()
                .filter(|n| n.tick != pick.tick)
                .collect()
        };
        assert_eq!(others(&four), others(&three));
        let moved = four.notes.iter().find(|n| n.tick == pick.tick).unwrap();
        assert_eq!((moved.len, moved.velocity), (pick.len, pick.velocity));
        assert_eq!(
            continuing(&h).unwrap().tries[3].again,
            Some(Again::Notes(1))
        );
        // Only locked bars picked: nothing to try.
        h.act(TimeAction::Edit(E::ContinueBar(2))).unwrap();
        assert_eq!(
            h.act(TimeAction::Edit(E::Tool(ToolAct::Option))).unwrap(),
            "continue: the picked bars are locked: Unlock them to try them again"
        );
        // Done keeps the try heard; the picks go with the tool.
        h.act(TimeAction::Edit(E::Tool(ToolAct::Done))).unwrap();
        assert_eq!(h.project().clip(0, 0).unwrap().notes, four.notes);
    }

    /// Phase 1 step 2: Continue from the note editor. The kind of
    /// instrument comes from what the screen says the track plays, else
    /// from the clip's notes. Opening the tool asks at once; stopped, the
    /// result comes in at once and the editor's own clip stays as it is;
    /// a setting changed asks again; Done keeps it as one take, closes
    /// the tool and the editor follows the kept clip. Cancel goes back and
    /// closes it, and the editor opening again on the clip goes back too.
    #[test]
    fn continue_from_the_note_editor_keeps_and_goes_back() {
        use mpfree_compose::{InstrumentKind, Lanes};
        use mpfree_time_ui::continuing::Parts;
        use mpfree_time_ui::editor::{EditAction as E, ToolAct, CONTINUE_ROW};
        let mut h = riff_host();
        let mut pads = Fake::default();
        let riff = h.project().clip(0, 0).unwrap().clone();
        assert_eq!(h.control.part_kind(0, 0), InstrumentKind::OneNote);
        let parts = Parts {
            kinds: vec![Some(InstrumentKind::Bass)],
            lanes: Lanes::default(),
        };
        let said = h.act(TimeAction::Continue(ContinueAction::Parts(Arc::new(parts))));
        assert_eq!(said.unwrap(), "");
        assert_eq!(h.control.part_kind(0, 0), InstrumentKind::Bass);
        h.act(TimeAction::Edit(E::Open)).unwrap();
        let open = E::Row {
            row: CONTINUE_ROW,
            delta: 1,
        };
        assert_eq!(
            h.act(TimeAction::Edit(open.clone())).unwrap(),
            "continue: 4 more bars of riff, Minimal"
        );
        assert!(h.state().edit.unwrap().editor.continuing_open());
        until(&mut h, &mut pads, |h| {
            continuing(h).is_some_and(|c| c.heard)
        });
        let c = continuing(&h).unwrap();
        assert_eq!((c.position, c.line), (Some((1, 1)), None));
        let st = h.state();
        let e = st.edit.as_ref().unwrap();
        assert_eq!(*e.clip, riff, "the editor's clip is the clip as it is");
        // A setting asks again.
        assert_eq!(
            h.act(TimeAction::Edit(E::Row { row: 1, delta: -1 }))
                .unwrap(),
            "continue: 2 more bars: the next try is on its way"
        );
        until(&mut h, &mut pads, |h| {
            continuing(h).is_some_and(|c| c.position == Some((2, 2)))
        });
        let heard = continuing(&h).unwrap().clip.clone().unwrap();
        assert_eq!(heard.length, 3 * 3840);
        // Done: one take, the tool closed, the editor on the kept clip.
        let rev = h.state().edit.unwrap().rev;
        assert_eq!(
            h.act(TimeAction::Edit(E::Tool(ToolAct::Done))).unwrap(),
            "continue: 2 more bars of riff kept (UNDO takes it back)"
        );
        assert!(continuing(&h).is_none());
        let st = h.state();
        let e = st.edit.as_ref().unwrap();
        assert!(e.editor.open_tool.is_none());
        assert_eq!(*e.clip, *heard);
        assert!(e.rev > rev, "the editor's picture moved on");
        assert_eq!(
            st.done.last().unwrap().replaced,
            Some(Replaced::Continue { bars: 2 })
        );
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.project().clip(0, 0).unwrap(), &riff);
        // Cancel: back, the tool closed, nothing written.
        h.act(TimeAction::Edit(open.clone())).unwrap();
        until(&mut h, &mut pads, |h| {
            continuing(h).is_some_and(|c| c.heard)
        });
        assert_eq!(
            h.act(TimeAction::Edit(E::Tool(ToolAct::Cancel))).unwrap(),
            "continue: back to the clip as it was"
        );
        assert!(h.state().edit.unwrap().editor.open_tool.is_none());
        assert_eq!(h.project().clip(0, 0).unwrap(), &riff);
        // The editor opened again on the clip: back as well.
        h.act(TimeAction::Edit(open)).unwrap();
        until(&mut h, &mut pads, |h| {
            continuing(h).is_some_and(|c| c.heard)
        });
        h.act(TimeAction::Edit(E::Open)).unwrap();
        assert!(continuing(&h).is_none_or(|c| !c.heard && c.made.is_none()));
        assert_eq!(h.project().clip(0, 0).unwrap(), &riff);
    }

    /// Apart (Arrange 2.6b), Continue goes through the editing side's
    /// thread the same way: asked, heard, kept.
    #[test]
    fn continue_works_with_the_halves_apart() {
        let (mut h, why) = riff_host().into_apart();
        assert_eq!(why, None);
        let mut pads = Fake::default();
        assert_eq!(h.act_then(continue_ask(1), Say::Tell), None);
        let said = answers(&mut h, &mut pads, 1);
        assert_eq!(
            said[0].1.as_deref(),
            Ok("continue: 1 more bar of riff, Minimal")
        );
        until(&mut h, &mut pads, |h| {
            continuing(h).is_some_and(|c| c.heard)
        });
        assert_eq!(
            h.act_then(TimeAction::Continue(ContinueAction::Keep), Say::Tell),
            None
        );
        let said = answers(&mut h, &mut pads, 1);
        assert_eq!(
            said[0].1.as_deref(),
            Ok("continue: 1 more bar of riff kept (UNDO takes it back)")
        );
        until(&mut h, &mut pads, |h| {
            h.project().clip(0, 0).is_some_and(|c| c.length == 2 * 3840)
        });
    }

    // ------------------------------------------------ Add a part (S2.6)

    fn add_bass() -> TimeAction {
        use mpfree_compose::{InstrumentKind, Lanes, Relation};
        TimeAction::Add(AddAction::Ask(Box::new(PartAsk {
            row: 0,
            track: 1,
            part: mpfree_context::PartRole::Bass,
            relation: Relation::Support,
            settings: mpfree_compose::Settings::default(),
            kind: InstrumentKind::Bass,
            lanes: Lanes::default(),
            pads: [None; 4],
            bars: None,
            new_track: None,
        })))
    }

    fn adding(h: &TimeHost) -> Option<Arc<mpfree_time_ui::adding::Adding>> {
        h.state().adding
    }

    /// Composing tools S2.6: Add a part on a playing row. The try comes
    /// from the composer's thread and is heard in its empty cell from the
    /// next bar line, in time with the row; nothing is written until Keep,
    /// one take ("add part") that plays on unchanged; UNDO empties the
    /// cell again.
    #[test]
    fn add_a_part_is_heard_in_its_cell_from_the_next_bar_line_kept_and_undone() {
        use mpfree_time_ui::adding::AddState;
        let mut h = riff_host();
        let mut pads = Fake::default();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        assert_eq!(
            h.act(add_bass()).unwrap(),
            "add a part: bass on Support for row 1"
        );
        assert_eq!(adding(&h).unwrap().state, AddState::Making);
        until(&mut h, &mut pads, |h| {
            adding(h).is_some_and(|a| a.line.is_some() || a.state == AddState::Playing)
        });
        let a = adding(&h).unwrap();
        assert_eq!((a.state, a.position), (AddState::Ready, Some((1, 1))));
        let at = a.line.expect("on a bar line");
        assert_eq!(at % 3840, 0);
        let part = a.made.as_ref().unwrap().clip.clone();
        assert!(!part.notes.is_empty());
        assert!(h.project().clip(1, 0).is_none(), "nothing written");

        // From the line the part plays with the riff: its low notes.
        let line = at as u64 * BAR / 3840;
        run(&mut h, &mut pads, line + 2 * BAR, &[]);
        assert_eq!(adding(&h).unwrap().state, AddState::Playing);
        assert!(
            hits_in(&pads, 0, line).iter().all(|h| h.1 >= 60),
            "the riff alone"
        );
        let low =
            |pads: &Fake, from, to| hits_in(pads, from, to).iter().filter(|h| h.1 < 60).count();
        assert!(low(&pads, line, line + 2 * BAR) > 0, "the bass in its cell");
        assert!(h.project().clip(1, 0).is_none(), "still nothing written");

        // Keep: one take, playing on.
        assert_eq!(
            h.act(TimeAction::Add(AddAction::Keep)).unwrap(),
            format!(
                "add a part: {} kept in row 1 (UNDO takes it back)",
                part.name
            )
        );
        assert!(adding(&h).is_none());
        let kept = h.project().clip(1, 0).unwrap().clone();
        assert_eq!(kept.notes, part.notes);
        let st = h.state();
        assert_eq!(st.done.last().unwrap().word(), "add part");
        let from = line + 2 * BAR;
        run(&mut h, &mut pads, from + 2 * BAR, &[]);
        assert!(
            low(&pads, from, from + 2 * BAR) > 0,
            "the kept bass plays on"
        );

        // UNDO: the cell empty again.
        h.act(TimeAction::Undo).unwrap();
        assert!(h.project().clip(1, 0).is_none());
        assert_eq!(h.dropped(), 0);
        assert!(
            h.describe().contains("add a part: 1 asked, 1 made"),
            "{}",
            h.describe()
        );
    }

    /// Back while a try plays: the cell goes quiet on the next bar line
    /// and nothing is written. A filled cell, Rec song and recording are
    /// refused.
    #[test]
    fn add_a_part_back_quietens_the_cell_and_refuses_what_it_cant_do() {
        use mpfree_time_ui::adding::AddState;
        let mut h = riff_host();
        let mut pads = Fake::default();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        h.act(add_bass()).unwrap();
        until(&mut h, &mut pads, |h| {
            adding(h).is_some_and(|a| a.state == AddState::Playing)
        });
        let pos = h.state().transport.position;
        run(&mut h, &mut pads, (pos as u64 + 3840 / 4) * BAR / 3840, &[]);
        let said = h.act(TimeAction::Add(AddAction::Back)).unwrap();
        assert!(
            said.starts_with("add a part: back to the row as it was, heard from"),
            "{said}"
        );
        assert!(adding(&h).is_none(), "the panel closes");
        let pos = h.state().transport.position;
        let next = (pos / 3840 + 1) * 3840;
        let quiet = next as u64 * BAR / 3840 + BAR / 8;
        run(&mut h, &mut pads, quiet + BAR, &[]);
        assert!(hits_in(&pads, quiet, quiet + BAR).iter().all(|h| h.1 >= 60));
        assert!(h.project().clip(1, 0).is_none());
        // A filled cell.
        let mut ask = add_bass();
        if let TimeAction::Add(AddAction::Ask(a)) = &mut ask {
            a.track = 0;
        }
        assert!(h.act(ask).unwrap_err().contains("isn't empty"));
    }

    /// A blank clip (the Sequencer's empty clip, S2.6): the try plays in
    /// its place, Keep puts the part there, and UNDO brings the blank clip
    /// back.
    #[test]
    fn add_a_part_takes_a_blank_clips_place() {
        use mpfree_time_ui::adding::AddState;
        let mut h = riff_host();
        let mut pads = Fake::default();
        h.control.project.tracks[1].slots[0] = Some(Clip::new("Clip 1", 2 * 3840));
        h.control.project.give_clip_ids();
        let blank = h.project().clip(1, 0).unwrap().clone();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        h.act(add_bass()).unwrap();
        until(&mut h, &mut pads, |h| {
            adding(h).is_some_and(|a| a.state == AddState::Playing)
        });
        let from = (h.state().transport.position as u64 / 3840 + 1) * BAR;
        run(&mut h, &mut pads, from + 2 * BAR, &[]);
        assert!(
            hits_in(&pads, from, from + 2 * BAR)
                .iter()
                .any(|h| h.1 < 60),
            "the try plays in the blank clip's place"
        );
        assert_eq!(h.project().clip(1, 0), Some(&blank), "nothing written");
        h.act(TimeAction::Add(AddAction::Keep)).unwrap();
        let kept = h.project().clip(1, 0).unwrap().clone();
        assert!(!kept.notes.is_empty());
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.project().clip(1, 0), Some(&blank));
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(h.project().clip(1, 0).unwrap().notes, kept.notes);
    }

    /// Add a part on New track (S2.6b): the part for the track Keep adds,
    /// at the end (index [`TRACKS`] here), on a Bass synth.
    fn add_bass_on_new_track() -> TimeAction {
        let TimeAction::Add(AddAction::Ask(mut ask)) = add_bass() else {
            unreachable!("an ask")
        };
        ask.track = TRACKS;
        ask.new_track = Some(mpfree_time_ui::adding::NewTrack {
            name: "Bass".into(),
            sound: "mpfree.bass",
        });
        TimeAction::Add(AddAction::Ask(ask))
    }

    /// Composing tools S2.6b: Add a part on New track. The try plays on
    /// the track Keep would add (the play view has it, the project
    /// doesn't), from the next bar line in time with the row; Keep adds
    /// the track with the part as one take, which plays on; UNDO takes
    /// both away and REDO brings them back.
    #[test]
    fn a_part_on_a_new_track_plays_there_and_keep_adds_the_track_as_one_take() {
        use mpfree_time_ui::adding::AddState;
        let mut h = riff_host();
        let mut pads = Fake::default();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        assert_eq!(
            h.act(add_bass_on_new_track()).unwrap(),
            "add a part: bass on Support for row 1"
        );
        until(&mut h, &mut pads, |h| {
            adding(h).is_some_and(|a| a.state == AddState::Playing)
        });
        let part = adding(&h).unwrap().made.as_ref().unwrap().clip.clone();
        assert!(!part.notes.is_empty());
        assert_eq!(h.project().tracks.len(), TRACKS, "no track until Keep");
        let low =
            |pads: &Fake, from, to| hits_in(pads, from, to).iter().filter(|h| h.1 < 60).count();
        let from = (h.state().transport.position as u64 / 3840 + 1) * BAR;
        run(&mut h, &mut pads, from + 2 * BAR, &[]);
        assert!(
            low(&pads, from, from + 2 * BAR) > 0,
            "the bass plays on the new track"
        );
        assert_eq!(h.project().tracks.len(), TRACKS, "still no track");

        // Keep: the track and the part, one take, playing on.
        assert_eq!(
            h.act(TimeAction::Add(AddAction::Keep)).unwrap(),
            format!(
                "add a part: {} kept in row 1 on a new track, Bass (UNDO takes both back)",
                part.name
            )
        );
        assert!(adding(&h).is_none());
        assert_eq!(h.project().tracks.len(), TRACKS + 1);
        assert_eq!(h.project().tracks[TRACKS].name, "Bass");
        assert_eq!(h.project().clip(TRACKS, 0).unwrap().notes, part.notes);
        assert_eq!(h.state().done.last().unwrap().word(), "add part");
        let from = h.state().transport.position as u64 * BAR / 3840 + BAR / 8;
        run(&mut h, &mut pads, from + 2 * BAR, &[]);
        assert!(
            low(&pads, from, from + 2 * BAR) > 0,
            "the kept bass plays on"
        );

        // UNDO: the track goes with its part; REDO brings both.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.project().tracks.len(), TRACKS);
        h.act(TimeAction::Redo).unwrap();
        assert_eq!(h.project().tracks.len(), TRACKS + 1);
        assert_eq!(h.project().clip(TRACKS, 0).unwrap().notes, part.notes);
        assert_eq!(h.dropped(), 0);
    }

    /// A track added while New track tries (+ Track): Keep would add the
    /// track at another index than the one its synth plays on, so the
    /// tries go, saying why, and Keep and the old index are refused until
    /// Try again asks for the new one.
    #[test]
    fn a_track_added_while_a_new_track_tries_drops_the_tries() {
        use mpfree_time_ui::adding::AddState;
        let mut h = riff_host();
        let mut pads = Fake::default();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        h.act(add_bass_on_new_track()).unwrap();
        until(&mut h, &mut pads, |h| {
            adding(h).is_some_and(|a| a.state != AddState::Making)
        });
        h.act(TimeAction::AddTrack).unwrap();
        let a = adding(&h).unwrap();
        assert_eq!(a.words.as_deref(), Some("A track was added: Try again"));
        assert!(a.made.is_none(), "the tries went");
        assert!(h
            .act(TimeAction::Add(AddAction::Keep))
            .unwrap_err()
            .contains("nothing made"));
        assert_eq!(
            h.act(add_bass_on_new_track()).unwrap_err(),
            "add a part: a track was added: ask again"
        );
        // Try again for the track Keep would add now.
        let TimeAction::Add(AddAction::Ask(mut ask)) = add_bass_on_new_track() else {
            unreachable!()
        };
        ask.track = TRACKS + 1;
        h.act(TimeAction::Add(AddAction::Ask(ask))).unwrap();
        until(&mut h, &mut pads, |h| {
            adding(h).is_some_and(|a| a.state == AddState::Playing)
        });
        h.act(TimeAction::Add(AddAction::Keep)).unwrap();
        assert_eq!(h.project().tracks.len(), TRACKS + 2);
        assert_eq!(h.project().tracks[TRACKS + 1].name, "Bass");
        assert!(h.project().clip(TRACKS + 1, 0).is_some());
    }

    /// Chords picked on the panel: the try plays them at once, the next
    /// try is made over them, and Keep puts them on the row with the part
    /// as one take; the riff, kept before, is marked. One UNDO takes back
    /// both.
    #[test]
    fn chords_picked_on_the_panel_go_with_keep_as_one_take() {
        use mpfree_context::chord::{Chord, ChordAnswer, ChordTimeline, Heard, Quality};
        use mpfree_time_ui::adding::AddState;
        let mut h = riff_host();
        let mut pads = Fake::default();
        h.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 4, &[]);
        h.act(add_bass()).unwrap();
        until(&mut h, &mut pads, |h| {
            adding(h).is_some_and(|a| a.state != AddState::Making)
        });
        let before = adding(&h).unwrap().made.clone().unwrap();
        let ab = ChordTimeline::whole(
            3840,
            ChordAnswer::Chord(Chord::new(8, Quality::Major)),
            Heard::Set,
        );
        h.act(TimeAction::Add(AddAction::Line(ab.clone()))).unwrap();
        let a = adding(&h).unwrap();
        assert_eq!(a.picked.as_ref(), Some(&ab));
        let now = a.made.clone().unwrap();
        assert_ne!(now.clip.notes, before.clip.notes, "moved to fit");
        assert_eq!(
            now.clip.notes.iter().map(|n| n.tick).collect::<Vec<_>>(),
            before.clip.notes.iter().map(|n| n.tick).collect::<Vec<_>>(),
            "the rhythm stays"
        );
        assert!(
            h.project().scene(0).is_none_or(|s| s.chords.is_empty()),
            "nothing written"
        );
        h.act(TimeAction::Add(AddAction::Keep)).unwrap();
        assert_eq!(h.project().scene(0).unwrap().chords, ab.plain());
        let st = h.state();
        assert_eq!(st.done.last().unwrap().word(), "add part");
        assert!(st
            .unfit
            .iter()
            .any(|u| u.row == 0 && u.clips.iter().any(|c| c.0 == 0)));
        h.act(TimeAction::Undo).unwrap();
        assert!(h.project().clip(1, 0).is_none());
        assert!(h.project().scene(0).is_none_or(|s| s.chords.is_empty()));
    }

    /// The row's chords outside the panel: Use is one take, the kept
    /// clips it leaves off the chords are marked (drums never), and Fit
    /// moves the ones asked as one take that UNDO takes back.
    #[test]
    fn a_chord_change_marks_kept_clips_and_fit_moves_them_as_one_take() {
        use mpfree_context::chord::{Chord, ChordAnswer, ChordTimeline, Heard, Quality};
        let mut h = riff_host();
        // A bass under the riff on track 2, on C minor's chord tones.
        let mut bass = Clip::new("bass", 3840);
        bass.holds = mpfree_seq::Holds::Notes;
        bass.notes = [36u8, 43, 39, 36]
            .iter()
            .enumerate()
            .map(|(i, &n)| mpfree_seq::Note::new(i as Tick * 960, 900, n, 100))
            .collect();
        h.control.project.tracks[1].slots[0] = Some(bass.clone());
        h.control.project.give_clip_ids();
        let whole = |root, q| {
            ChordTimeline::whole(3840, ChordAnswer::Chord(Chord::new(root, q)), Heard::Set)
        };
        let cm = whole(0, Quality::Minor);
        // Heard as C minor, set to F sharp minor: no note of C minor in it.
        let said = h
            .act(TimeAction::Chords(ChordsAction::Set {
                row: 0,
                line: whole(6, Quality::Minor),
                shown: cm.clone(),
            }))
            .unwrap();
        assert!(said.contains("2 clips don't fit them"), "{said}");
        let st = h.state();
        assert_eq!(st.done.last().unwrap().word(), "chords");
        let u = st.unfit.iter().find(|u| u.row == 0).unwrap();
        assert_eq!(u.before, cm);
        let tracks: Vec<usize> = u.clips.iter().map(|c| c.0).collect();
        assert_eq!(tracks, vec![0, 1]);

        // Fit the bass only: one take; the riff stays marked.
        let said = h
            .act(TimeAction::Chords(ChordsAction::Fit {
                row: 0,
                tracks: vec![1],
            }))
            .unwrap();
        assert!(said.starts_with("chords: a clip fitted"), "{said}");
        let fitted = h.project().clip(1, 0).unwrap().clone();
        assert_ne!(fitted.notes, bass.notes);
        assert_eq!(
            fitted
                .notes
                .iter()
                .map(|n| (n.tick, n.len))
                .collect::<Vec<_>>(),
            bass.notes
                .iter()
                .map(|n| (n.tick, n.len))
                .collect::<Vec<_>>(),
            "the rhythm stays"
        );
        for n in &fitted.notes {
            assert!([6, 9, 1].contains(&(n.note % 12)), "{n:?}");
        }
        let st = h.state();
        assert_eq!(st.done.last().unwrap().word(), "fit");
        let u = st.unfit.iter().find(|u| u.row == 0).unwrap();
        assert_eq!(u.clips.iter().map(|c| c.0).collect::<Vec<_>>(), vec![0]);

        // UNDO: the bass as it was, marked again.
        h.act(TimeAction::Undo).unwrap();
        assert_eq!(h.project().clip(1, 0).unwrap().notes, bass.notes);
        assert_eq!(h.state().unfit[0].clips.len(), 2);
        // UNDO the chords: nothing marked.
        h.act(TimeAction::Undo).unwrap();
        assert!(h.state().unfit.is_empty());
    }

    /// The Chords tab hears its row without a try: the row's chords come
    /// from the composer's thread into the state, are heard again when
    /// the row's chords change (a set chord shows as set), and go when
    /// the tab closes. Nothing is written by hearing.
    #[test]
    fn the_chords_tab_hears_its_row_again_when_it_changes() {
        use mpfree_context::chord::{Chord, ChordAnswer, ChordTimeline, Heard, Quality};
        let mut h = riff_host();
        let mut pads = Fake::default();
        let before = h.project().clone();
        assert_eq!(
            h.act(TimeAction::Chords(ChordsAction::Hear { row: Some(0) }))
                .unwrap(),
            ""
        );
        until(&mut h, &mut pads, |h| h.state().row_chords.is_some());
        let (row, chords) = h.state().row_chords.unwrap();
        let heard = &chords.line;
        assert_eq!(row, 0);
        assert!(!heard.is_empty(), "the riff gives chords");
        assert!(heard.spans().iter().all(|s| s.heard != Heard::Set));
        assert_eq!(chords.heard, *heard, "nothing of its own yet");
        assert_eq!(h.project().scenes, before.scenes, "hearing writes nothing");

        // A chord set on the row: heard again, the set chord shown.
        let set = ChordTimeline::whole(
            3840,
            ChordAnswer::Chord(Chord::new(6, Quality::Minor)),
            Heard::Set,
        );
        h.act(TimeAction::Chords(ChordsAction::Set {
            row: 0,
            line: set,
            shown: heard.clone(),
        }))
        .unwrap();
        until(&mut h, &mut pads, |h| {
            h.state()
                .row_chords
                .is_some_and(|(_, c)| c.line.span_at(0).is_some_and(|s| s.heard == Heard::Set))
        });
        let (_, now) = h.state().row_chords.unwrap();
        assert_eq!(
            now.line.span_at(0).unwrap().answer,
            ChordAnswer::Chord(Chord::new(6, Quality::Minor))
        );
        assert_eq!(now.heard, chords.heard, "what the clips say stays");

        // Another row, then none: the old row's chords go at once.
        h.act(TimeAction::Chords(ChordsAction::Hear { row: Some(1) }))
            .unwrap();
        assert!(h.state().row_chords.is_none());
        h.act(TimeAction::Chords(ChordsAction::Hear { row: None }))
            .unwrap();
        run(&mut h, &mut pads, BAR, &[]);
        assert!(h.state().row_chords.is_none());
        assert!(h
            .act(TimeAction::Chords(ChordsAction::Hear {
                row: Some(mpfree_seq::SCENES)
            }))
            .is_err());
    }
}
