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

mod audio;
mod control;
mod follow;
pub mod link;

use mpfree_play::{Event, Out, Sink, TrackArps};
use mpfree_seq::mpfree_moves::Input;
use mpfree_seq::steps::{Change, Step};
use mpfree_seq::{Clip, Project, Track};
use mpfree_time_ui::view::{TimeAction, TimeState, TransportInfo};
use mpfree_transport::time::Tick;
use mpfree_transport::{Block, Command, Transport};
use std::f32::consts::TAU;

pub use audio::TimeAudio;
pub use control::TimeControl;
pub use follow::Follow;
use link::Tracks;

/// Tracks in a new project.
pub const TRACKS: usize = 8;
/// Bars in a clip made by tapping an empty slot (without a loop set).
pub const NEW_CLIP_BARS: i64 = 2;
/// Notes the sequencer can start or stop in one period.
const MAX_OUT: usize = 512;
/// Notes, and locks, the armed track's clips can take while recording
/// before the view is compiled again (which allocates): room set aside
/// in the clip and in the play view each time it compiles.
pub const RECORD_ROOM: usize = 1024;
/// The Play page's steps: sixteenths.
pub const STEP_TICKS: Tick = 240;
/// Pad hits kept for the screen between two periods (more are dropped).
const MAX_HITS: usize = 16;

/// What the Play page's pattern shows: the armed track's clip (the one
/// playing, else its first), each sound's steps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepsData {
    pub track: usize,
    pub track_name: String,
    pub clip: Option<StepClip>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepClip {
    pub slot: usize,
    pub name: String,
    /// Steps in the clip (16 a bar).
    pub steps: usize,
    /// The clip's length and a step's, in song ticks (after its speed).
    pub song_length: f64,
    pub song_step: f64,
    /// Where it was launched, while it plays (for the playhead).
    pub origin: Option<Tick>,
    /// Every pad (0 to 15), and any other note the clip holds, with its
    /// steps (None: empty).
    pub rows: Vec<(u8, Vec<Option<Step>>)>,
}

impl StepClip {
    /// The steps of `pitch` (empty when the clip has no such row).
    pub fn row(&self, pitch: u8) -> &[Option<Step>] {
        self.rows
            .iter()
            .find(|(p, _)| *p == pitch)
            .map_or(&[], |(_, r)| r.as_slice())
    }

    /// The step playing at song position `pos`, while the clip plays.
    pub fn playing(&self, pos: Tick) -> Option<usize> {
        let origin = self.origin?;
        if self.song_length <= 0.0 || self.song_step <= 0.0 || pos < origin {
            return None;
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
}

/// A pad hit while the transport rolls: where it fell, for the Play page's
/// timing (Drum view).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HitInfo {
    pub pad: u8,
    pub velocity: u8,
    /// Song position, ticks, with the fraction.
    pub tick: f64,
    pub bpm: f64,
    pub bar_ticks: Tick,
}

/// Where the sequencer's notes go: the sampler's pads.
pub trait Pads {
    fn hit_at(&mut self, pad: usize, velocity: u8, offset: u32) -> bool;
    fn release_at(&mut self, pad: usize, offset: u32);
    fn stop_all(&mut self);
    /// A clip's recorded pressure on `pad`'s sounding note, the raw value
    /// 0 to 127, `offset` frames into the period.
    fn pressure_at(&mut self, pad: usize, value: u8, offset: u32);
    /// A track that plays somewhere other than the pads (one on MIDI
    /// OUT) is offered each event the clips play first: notes, a panic's
    /// `AllOff`, pressure, and the locks and lanes. True when it took the
    /// event, so neither the pads nor the mixer's mod engine get it.
    /// False leaves it to go on as usual (an `AllOff` should, so the pads
    /// stop too). The arps' notes, the live pads among them, go to
    /// [`Pads::live`] instead, which sees the whole event and says what
    /// records. By default nothing is taken.
    fn out(&mut self, o: &Out) -> bool {
        let _ = o;
        false
    }
    /// A note one of the per-track arps played ([`TimeHost::period_arps`]):
    /// the pads through their track's arp, a latched arp, or a clip that
    /// plays through its arp. It is live playing, so the pad player gives
    /// it the pads' own treatment (velocity curve, flashes). Returns the
    /// note to record as heard on the armed track (a pad number), or None.
    /// By default it plays as a clip's note and records as it is.
    fn live(&mut self, o: &Out) -> Option<Event> {
        match o.event {
            Event::NoteOn { note, velocity } => {
                self.hit_at(usize::from(note), velocity, o.offset);
                Some(o.event)
            }
            Event::NoteOff { note } => {
                self.release_at(usize::from(note), o.offset);
                Some(o.event)
            }
            Event::AllOff => {
                self.stop_all();
                None
            }
            Event::Pressure { note, value } => {
                self.pressure_at(usize::from(note), value, o.offset);
                None
            }
            Event::Lock { .. } | Event::Lane { .. } | Event::Unlock { .. } => None,
        }
    }
}

impl Pads for crate::sampler::Player {
    fn hit_at(&mut self, pad: usize, velocity: u8, offset: u32) -> bool {
        crate::sampler::Player::hit_at(self, pad, velocity, offset)
    }
    fn release_at(&mut self, pad: usize, offset: u32) {
        crate::sampler::Player::release_at(self, pad, offset)
    }
    fn stop_all(&mut self) {
        crate::sampler::Player::stop_all(self)
    }
    fn pressure_at(&mut self, pad: usize, value: u8, offset: u32) {
        crate::sampler::Player::pressure_at(self, pad, value, offset)
    }
}

/// The player's notes for one period, in a fixed table.
struct Collect {
    out: [Out; MAX_OUT],
    /// Which of `out` an arp played (live playing, see [`Pads::live`]).
    arp: [bool; MAX_OUT],
    n: usize,
    dropped: u64,
}

impl Collect {
    fn push(&mut self, out: Out, arp: bool) {
        if self.n < MAX_OUT {
            self.out[self.n] = out;
            self.arp[self.n] = arp;
            self.n += 1;
        } else {
            self.dropped += 1;
        }
    }
}

impl Sink for Collect {
    fn send(&mut self, out: Out) {
        self.push(out, false);
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
}

/// The click's level (about -12 dBFS) and decay (a quarter of its level
/// after 15 ms).
const CLICK_GAIN: f32 = 0.25;
const CLICK_MS: f32 = 15.0;

impl Click {
    fn new(rate: u32) -> Click {
        Click {
            rate: rate as f32,
            starts: [(0, false); 8],
            n: 0,
            phase: 0.0,
            freq: 0.0,
            env: 0.0,
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
                let s = CLICK_GAIN * self.env * (self.phase * TAU).sin();
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
    control: TimeControl,
    audio: TimeAudio,
    /// Click while counting in and recording.
    pub metronome: bool,
}

/// A step's parameter lock (`value`) or its end (`None`), `offset` frames
/// into the period.
pub type LockOut = (u32, mpfree_play::mpfree_param::ParamAddr, Option<f32>);

/// A move lane's value, `offset` frames into the period (a lane's end
/// comes as a [`LockOut`] unlock).
pub type LaneOut = (u32, mpfree_play::mpfree_param::ParamAddr, f32);

/// Locks kept for the mixer in one period (more are dropped).
const MAX_LOCKS: usize = 256;

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

    /// Stopped at bar 1 with `project` (an opened one), track 1 armed when
    /// it has a track. Builds and compiles everything here (allocates):
    /// the app makes it on the screen thread and swaps it in.
    pub fn with_project(rate: u32, bpm: f64, project: Project) -> Result<TimeHost, String> {
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
            control,
            audio,
            metronome: true,
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
    #[allow(dead_code)] // The app thread's control thread takes it (step 3).
    pub fn split(self) -> (TimeControl, TimeAudio) {
        let mut audio = self.audio;
        audio.metronome = self.metronome;
        (self.control, audio)
    }

    /// The audio half takes what the control half sent, and passes `r` on.
    fn sent<T>(&mut self, r: T) -> T {
        self.audio.receive();
        r
    }

    /// The armed track (the one the pads, the arp panel and recording
    /// follow), if any.
    pub fn armed(&self) -> Option<usize> {
        self.control.armed()
    }

    /// The project the clips are in (for a save: `project.rs`).
    pub fn project(&self) -> &Project {
        &self.control.project
    }

    /// "Clips through the arp" for `track` (see `Track::through_arp`).
    /// The track's arp follows from the next [`TimeHost::period_arps`],
    /// which clears it if it holds anything, so no note is left hanging.
    /// False if there is no such track.
    // The arp panel calls it (the arp thread's side); until then only
    // tests do.
    #[allow(dead_code)]
    pub fn set_through_arp(&mut self, track: usize, on: bool) -> bool {
        let r = self.control.set_through_arp(track, on);
        self.sent(r)
    }

    /// What the Play page's pattern shows. Allocates: called on a change.
    pub fn steps(&self) -> StepsData {
        self.control.steps()
    }

    /// True once after the steps the Play page shows changed.
    pub fn take_steps_changed(&mut self) -> bool {
        self.control.take_steps_changed()
    }

    /// A touch on the Play page's pattern, as one undoable edit (a
    /// Q-Link turned through many values joins one). A line for the log.
    pub fn step_action(&mut self, a: StepAction) -> Result<String, String> {
        let r = self.control.step_action(a);
        self.sent(r)
    }

    /// The pad hits of the last period that fell while the transport
    /// rolled (for the Play page's timing); [`TimeHost::clear_hits`]
    /// after reading.
    pub fn hits(&self) -> &[HitInfo] {
        self.audio.hits()
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
        let sent = self.control.command(cmd);
        self.audio.receive();
        sent && self.audio.counts().refused == refused
    }

    /// Follow the app's tempo (TAP TEMPO, the menu). Does nothing while
    /// a followed clock arrives ([`TimeHost::followed_bpm`]).
    pub fn follow_tempo(&mut self, bpm: f64) {
        self.audio.follow_tempo(bpm);
    }

    /// Follow another device's MIDI clock with these options (the
    /// project's MIDI setup: follow start/stop, follow position), or not
    /// (None). Cheap to call every period: only a change starts afresh.
    pub fn follow(&mut self, follow: Option<Follow>) {
        self.audio.follow(follow);
    }

    /// Every byte read from the followed port this period, before
    /// [`TimeHost::advance`], with `frame` = [`TimeHost::next_frame`]
    /// (or earlier, when the bytes have their own time). Anything but
    /// clock, Start, Continue, Stop and Song Position is skipped; the
    /// options given to [`TimeHost::follow`] say what follows.
    pub fn clock_in(&mut self, frame: u64, bytes: &[u8]) {
        self.audio.clock_in(frame, bytes);
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

    /// Carry out a touch on the time engine's screens. Returns a line for
    /// the log.
    pub fn act(&mut self, a: TimeAction) -> Result<String, String> {
        let r = self.control.act(a);
        self.sent(r)
    }

    /// What live to song put into the song when the transport stopped
    /// (or why it could not), once, for the log.
    pub fn take_song_note(&mut self) -> Option<String> {
        self.control.take_song_note()
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
        let r = self.control.morph(a, modu, mixer);
        self.sent(r)
    }

    /// A clip made elsewhere (the Sample page's chop to a phrase) into the
    /// first empty slot of the armed track (track 1 if none is armed), as
    /// a take that UNDO on the Clips page takes away again. It isn't
    /// launched: a tap on the Clips page plays it like any clip. Returns a
    /// line for the log.
    pub fn put_clip(&mut self, clip: Clip) -> Result<String, String> {
        let r = self.control.put_clip(clip);
        self.sent(r)
    }

    /// A pad note as pressed, before any arp, in the coming period at
    /// `offset`. When the armed track's clips play through its arp, this
    /// is what it records, and what the capture history keeps.
    // The app calls it once its pads play through the per-track arps (the
    // arp thread's side); until then only tests do.
    #[allow(dead_code)]
    pub fn pad_pressed(&mut self, offset: u32, event: Event) {
        self.audio.pad_pressed(offset, event);
    }

    /// A pad note as heard (after the arp) in the coming period, at
    /// `offset`: recorded into the armed track while the transport
    /// records, and kept in the capture history always, unless the armed
    /// track's clips play through its arp (then [`TimeHost::pad_pressed`]
    /// gives those). It lights the pattern's step either way.
    pub fn pad(&mut self, offset: u32, event: Event) {
        self.audio.pad(offset, event);
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
        let r = self.control.set_record_touch(track, on);
        self.sent(r)
    }

    /// Pressure values lost: past the recorder's tables, or a clip with
    /// no room left (should stay 0). The status line says it
    /// ([`TimeHost::describe`]); tests read it here.
    #[allow(dead_code)]
    pub fn touch_dropped(&self) -> u64 {
        self.audio.touch_dropped() + self.control.touch_lost()
    }

    /// A knob moved in the coming period, at `offset`: parameter `addr` is
    /// now `value`. While the transport records and `addr` is on the armed
    /// track, it becomes a parameter lock on the step it belongs to (see
    /// `mpfree_play::Recorder`).
    // No knob is mapped to a parameter in the pad player yet (that needs
    // the mod resolver); the Q-Links call this once one is.
    #[allow(dead_code)]
    pub fn knob(&mut self, offset: u32, addr: mpfree_play::mpfree_param::ParamAddr, value: f32) {
        self.audio.knob(offset, addr, value);
    }

    /// A Perform FX move in the coming period, at `offset`: the XY pad,
    /// the DJ knob or a pad's Hold moving a parameter (`Input::Move`, the
    /// first value starting a gesture), or letting go (`Input::End`).
    /// While the transport records it becomes a move in the armed track's
    /// clip; while it rolls the move log keeps it for CAPTURE.
    // The Perform FX page calls it once the pad player runs it (the FX
    // thread's side); until then only tests do.
    #[allow(dead_code)]
    pub fn gesture(&mut self, offset: u32, input: Input) {
        self.audio.gesture(offset, input);
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
    pub fn toggle_mute(&mut self, t: usize) -> Result<String, String> {
        let r = self.control.toggle_mute(t);
        self.sent(r)
    }

    /// Whether track `t` is muted (None: no such track), for TRACK MUTE's
    /// pad lights.
    pub fn muted(&self, t: usize) -> Option<bool> {
        self.control.muted(t)
    }

    /// Whether track `t` has a clip launched (playing, or waiting for the
    /// transport), for TRACK MUTE's pad lights.
    pub fn launched(&self, t: usize) -> bool {
        self.control.launched(t)
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
        self.audio.run(frames, pads, arps);
        self.control.poll();
        self.audio.receive();
    }

    /// Add the click to the rendered period; returns the peak afterwards.
    pub fn mix_click(&mut self, left: &mut [f32], right: &mut [f32]) -> f32 {
        self.audio.mix_click(left, right)
    }

    /// The transport's description of the last period (for the looper).
    pub fn block(&self) -> &Block {
        self.audio.block()
    }

    /// The song position as bar.beat.tick, for log lines.
    pub fn position_text(&self) -> String {
        self.audio.position_text()
    }

    pub fn info(&self) -> TransportInfo {
        self.audio.info()
    }

    /// True once after the clips, launches or takes changed. Notes and
    /// locks being recorded are told at most every quarter second.
    pub fn take_changed(&mut self) -> bool {
        self.control.take_changed()
    }

    /// Everything the screens show.
    pub fn state(&self) -> TimeState {
        self.control.state()
    }

    /// Notes and locks lost because a table was full (should stay 0). The
    /// status line says it ([`TimeHost::describe`]); tests read it here.
    #[allow(dead_code)]
    pub fn dropped(&self) -> u64 {
        self.audio.counts().dropped + self.control.dropped()
    }

    /// The time engine's part of the status line, with what passing
    /// between the halves lost, if anything.
    pub fn describe(&self) -> String {
        self.control.describe()
    }
}

/// "take 3" or "capture 2", for log lines.
fn take_name(row: Option<(bool, u32)>) -> String {
    match row {
        Some((true, n)) => format!("capture {n}"),
        Some((false, n)) => format!("take {n}"),
        None => "take".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_play::PlayView;
    use mpfree_seq::SLOTS;
    use mpfree_time_ui::view::{AudioAction, RegionId};
    use mpfree_transport::State;

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
        assert_eq!(h.control.project.clip(0, 1), Some(&clip));
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
        // A full track says so.
        for s in 0..SLOTS {
            h.control.project.tracks[2].slots[s] = Some(Clip::new("full", 960));
        }
        assert!(h.put_clip(clip).unwrap_err().contains("no empty slot"));
    }

    const RATE: u32 = 44_100;
    const PERIOD: u32 = 128;

    #[derive(Default)]
    struct Fake {
        hits: Vec<(u64, usize, u8)>,
        frame: u64,
        stops: usize,
        /// (absolute frame, pad, value)
        pressure: Vec<(u64, usize, u8)>,
    }

    impl Pads for Fake {
        fn hit_at(&mut self, pad: usize, velocity: u8, offset: u32) -> bool {
            self.hits
                .push((self.frame + u64::from(offset), pad, velocity));
            true
        }
        fn release_at(&mut self, _pad: usize, _offset: u32) {}
        fn stop_all(&mut self) {
            self.stops += 1;
        }
        fn pressure_at(&mut self, pad: usize, value: u8, offset: u32) {
            self.pressure
                .push((self.frame + u64::from(offset), pad, value));
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
            lanes += h.audio.sink.out[..h.audio.sink.n]
                .iter()
                .filter(|o| matches!(o.event, Event::Lane { addr, .. } if addr == xy))
                .count();
        }
        assert!(lanes > 20, "{lanes}");
        assert_eq!(h.act(TimeAction::Undo).unwrap(), "undo: take 1");
        assert_eq!(lane(&h), None);
        assert_eq!(h.dropped(), 0);
    }

    /// Pad pressure while recording goes on the pad's note in the clip
    /// (the Record list's "Pressure" row is on for a new track), plays
    /// back to the pads on the next pass, and is left out with the row off.
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
        assert_eq!(h.act(TimeAction::Undo).unwrap(), "undo: capture 1");
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
        assert_eq!(h.act(TimeAction::Undo).unwrap(), "undo: take 1");
        pads.hits.clear();
        run(&mut h, &mut pads, 6 * BAR, &[]);
        assert!(pads.hits.is_empty());
        assert_eq!(h.act(TimeAction::Redo).unwrap(), "redo: take 1");
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
        assert_eq!(h.act(TimeAction::Undo).unwrap(), "undo: take 1");
        let clip = h.control.project.tracks[2].slots[0].as_ref().unwrap();
        assert!(clip.notes.is_empty() && clip.locks.is_empty());
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
        assert!(h.act(TimeAction::LaunchScene(SLOTS)).is_err());
        assert_eq!(h.dropped(), 0);
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
        h.control.project.scene_mut(1).unwrap().name = "Drop".into();
        // Row 6 has a clip too: three rows, bars 1, 2 and 3.
        assert_eq!(
            h.act(TimeAction::ScenesToSong).unwrap(),
            "scenes → song: 3 rows from bar 1"
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
        assert_eq!(
            h.act(TimeAction::SongToScenes).unwrap(),
            "song → scenes: rows 3 to 5"
        );
        let p = &h.control.project;
        assert_eq!(p.tracks[0].slots[2], p.tracks[0].slots[0]);
        assert_eq!(p.tracks[1].slots[3], p.tracks[1].slots[1]);
        assert_eq!(p.tracks[0].slots[4], p.tracks[0].slots[5]);
        assert_eq!(p.scene(3).unwrap().name, "Drop");
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
        h.act(TimeAction::LaunchScene(0)).unwrap();
        h.act(TimeAction::RecSong(true)).unwrap();
        assert!(h.state().song_rec);
        h.command(Command::Play);
        run(&mut h, &mut pads, BAR / 2, &[]);
        h.act(TimeAction::LaunchScene(1)).unwrap();
        run(&mut h, &mut pads, 3 * BAR - 1000, &[]);
        h.command(Command::Stop);
        run(&mut h, &mut pads, 3 * BAR, &[]);
        let note = h.take_song_note().unwrap();
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
        let note = h.take_song_note().unwrap();
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

    /// The note editor on the focused clip: Edit opens it; Timing Correct
    /// with nothing selected moves every note to the 1/16 grid; a tap
    /// selects a note and Delete takes it out; each is one take that Undo
    /// puts back; ‹ Clips closes it.
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
            h.act(TimeAction::Edit(E::Button(7))).unwrap(),
            "edit: timing correct moved 3 notes"
        );
        assert_eq!(ticks(&h), [0, 960, 1920]);
        h.act(TimeAction::Edit(E::Note(1))).unwrap();
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
        assert_eq!(
            h.act(TimeAction::Edit(E::Button(0))).unwrap(),
            "edit: closed"
        );
        assert!(h.state().edit.is_none());
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
        assert_eq!(h.act(TimeAction::Undo).unwrap(), "undo: capture 1");
        let st = h.state();
        assert!(st.clip(0, 0).is_none() && st.launch(0).is_none());
        assert_eq!(
            st.capture.as_deref(),
            Some("2 bars"),
            "it can be kept again"
        );
        assert_eq!(h.act(TimeAction::Redo).unwrap(), "redo: capture 1");
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
        assert_eq!(h.act(TimeAction::Undo).unwrap(), "undo: capture 1");
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
        fn hit_at(&mut self, pad: usize, velocity: u8, offset: u32) -> bool {
            self.pads.hit_at(pad, velocity, offset)
        }
        fn release_at(&mut self, pad: usize, offset: u32) {
            self.pads.release_at(pad, offset)
        }
        fn stop_all(&mut self) {
            self.pads.stop_all()
        }
        fn pressure_at(&mut self, pad: usize, value: u8, offset: u32) {
            self.pads.pressure_at(pad, value, offset)
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

    /// Following another device's clock as the app will (the MIDI
    /// thread's M4): each period, the leader port's bytes read at its top
    /// go to `clock_in` at `next_frame`, and the app's tempo to
    /// `follow_tempo`. The leader runs at 100 BPM, the transport at 120.
    /// A one-bar clip with a note on every beat plays on the leader's
    /// beats from the third bar on, about half a period after them (bytes
    /// read once a period come up to a period late); the app's tempo is ignored while
    /// the clock arrives; the screens and `followed_bpm` show the leader's;
    /// the status line has nothing to add; and following allocates
    /// nothing.
    #[test]
    fn a_clip_plays_on_the_beats_of_a_followed_clock() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake {
            hits: Vec::with_capacity(1000),
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
            h.clock_in(p, &buf[..n]);
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
        assert_eq!(pads.hits.len(), 32, "a note a beat, from the first");
        // About half a period late: the bytes are read up to a period
        // after they came (58 to 72 frames when written).
        let half = i64::from(PERIOD) / 2;
        assert!(
            late.iter().all(|&d| (half - 32..=half + 32).contains(&d)),
            "frames after the leader's beats: {late:?}"
        );
        let bpm = h.followed_bpm().unwrap();
        assert!((bpm - 100.0).abs() < 0.1, "{bpm}");
        assert_eq!(h.info().bpm, bpm);
        assert!(!h.describe().contains("following"), "{}", h.describe());
        // A loop of its own takes the transport away from the leader's
        // position as it wraps: it jumps back, and the status line says so.
        // (Twice: the leader's pulse is on the loop's end, so the first
        // jump lands there and wraps again.)
        let bar = h.info().position / 3840 * 3840;
        h.command(Command::SetLoop(Some((bar, bar + 3840))));
        while pads.frame < beat(40) {
            period(&mut h, &mut pads);
        }
        assert!(
            h.describe()
                .contains("following a clock: 2 jumps to its position, 0 times"),
            "{}",
            h.describe()
        );
        // Not following: the app's tempo applies again.
        // (Playing, it takes effect on the next sixteenth line.)
        h.follow(None);
        assert_eq!(h.followed_bpm(), None);
        for _ in 0..100 {
            period(&mut h, &mut pads);
        }
        assert_eq!(h.info().bpm, 120.0);
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
}
