//! The time engine in the pad player: the transport, the sequencer
//! (`mpfree-play`'s player and recorder), take undo and a metronome click,
//! run in the audio loop.
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
//! Every pad note also goes into the capture history, so CAPTURE
//! ([`TimeAction::Capture`]: the Arrange page's Capture button) keeps the
//! last phrase in the armed track even when nothing was recording, as an
//! undoable take (`mpfree-capture`).
//!
//! Edits (launches, takes, arming, a capture) compile a new play view in
//! the loop. That allocates, but only on an edit, not every period; moving
//! it to a control thread is a later step. Recording is not an edit of
//! that kind: each recorded or erased note or lock goes into the clip and
//! into the play view in place, in room set aside when the view was
//! compiled ([`RECORD_ROOM`]), so a recording period neither compiles nor
//! allocates. The screens hear about those notes at most four times a
//! second ([`TimeHost::take_changed`]). Every track plays the current
//! kit.

use mpfree_capture::{keep, Capture, Now, Pending};
use mpfree_play::{Cue, Event, Launch, Out, PlayView, Player, Recorder, Sink, Takes, TrackArps};
use mpfree_seq::steps::{Change, Grid, Step};
use mpfree_seq::{Clip, Project, Track, SLOTS};
use mpfree_time_ui::view::{bars_label, clips_after, TimeAction, TimeState, TransportInfo};
use mpfree_transport::time::Tick;
use mpfree_transport::{Block, Command, Mark, State, Transport};
use std::f32::consts::TAU;

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
}

/// The player's notes for one period, in a fixed table.
struct Collect {
    out: [Out; MAX_OUT],
    n: usize,
    dropped: u64,
}

impl Sink for Collect {
    fn send(&mut self, out: Out) {
        if self.n < MAX_OUT {
            self.out[self.n] = out;
            self.n += 1;
        } else {
            self.dropped += 1;
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

pub struct TimeHost {
    // Boxed: the fixed tables are large.
    transport: Box<Transport>,
    player: Box<Player>,
    recorder: Box<Recorder>,
    capture: Box<Capture>,
    /// What CAPTURE would keep, as the screen last showed it.
    pending: Option<Pending>,
    block: Box<Block>,
    sink: Box<Collect>,
    view: PlayView,
    project: Project,
    launches: Vec<Launch>,
    /// Launches and stops waiting for the next bar line.
    cues: Vec<Cue>,
    armed: Option<usize>,
    takes: Takes,
    click: Click,
    /// Click while counting in and recording.
    pub metronome: bool,
    /// The clips, launches or takes changed since [`TimeHost::take_changed`].
    changed: bool,
    /// Recording changed the clips and takes since the screens last heard.
    recorded: bool,
    /// Frames since `changed` was last set for a recording.
    since_told: u64,
    /// Recorded changes are told this often, in frames (a quarter second).
    tell_every: u64,
    /// [`TimeHost::advance`] ran the transport for the coming period (and
    /// whether it could).
    advanced: Option<bool>,
    /// A start or jump after the first span of the last period: the next
    /// arp clock says it jumped.
    jump_pending: bool,
    /// Pad hits this period, for the screen ([`TimeHost::hits`]).
    hits: [HitInfo; MAX_HITS],
    n_hits: usize,
    /// The steps changed since [`TimeHost::take_steps_changed`].
    steps_changed: bool,
}

impl TimeHost {
    /// Stopped at bar 1 with an empty project of [`TRACKS`] tracks, track 1
    /// armed.
    pub fn new(rate: u32, bpm: f64) -> Result<TimeHost, String> {
        let transport =
            Box::new(Transport::new(rate, bpm).map_err(|e| format!("transport: {e:?}"))?);
        let mut project = Project::new();
        for i in 0..TRACKS {
            project
                .tracks
                .push(Track::new(&format!("Track {}", i + 1), i as u16));
        }
        let blank = Out {
            offset: 0,
            frame: 0,
            track: 0,
            dest: 0,
            event: Event::AllOff,
        };
        let mut host = TimeHost {
            transport,
            player: Box::new(Player::new()),
            recorder: Box::new(Recorder::new()),
            capture: Box::new(Capture::new()),
            pending: None,
            block: Box::new(Block::new()),
            sink: Box::new(Collect {
                out: [blank; MAX_OUT],
                n: 0,
                dropped: 0,
            }),
            view: PlayView::empty(),
            project,
            launches: Vec::new(),
            cues: Vec::new(),
            armed: Some(0),
            takes: Takes::default(),
            click: Click::new(rate),
            metronome: true,
            changed: true,
            recorded: false,
            since_told: 0,
            tell_every: u64::from(rate / 4),
            advanced: None,
            jump_pending: false,
            hits: [HitInfo::default(); MAX_HITS],
            n_hits: 0,
            steps_changed: true,
        };
        host.compile()?;
        Ok(host)
    }

    fn compile(&mut self) -> Result<(), String> {
        // An undone capture takes its clip, and so its launch, away.
        let project = &self.project;
        self.launches
            .retain(|l| project.clip(l.track, l.slot).is_some());
        self.cues.retain(|c| {
            c.slot
                .is_none_or(|slot| project.clip(c.track, slot).is_some())
        });
        let mut v = PlayView::compile_cued(&self.project, &self.launches, &self.cues)
            .map_err(|e| format!("clips: {e:?}"))?;
        v.set_armed(self.armed);
        // Room to record into the armed track's clips without allocating.
        v.make_room(RECORD_ROOM, RECORD_ROOM);
        if let Some(t) = self.armed {
            let slots = self
                .launches
                .iter()
                .filter(|l| l.track == t)
                .map(|l| l.slot);
            let cued = self
                .cues
                .iter()
                .filter(|c| c.track == t)
                .filter_map(|c| c.slot);
            for slot in slots.chain(cued) {
                if let Some(Some(c)) = self
                    .project
                    .tracks
                    .get_mut(t)
                    .and_then(|t| t.slots.get_mut(slot))
                {
                    c.notes.reserve(RECORD_ROOM);
                    c.locks.reserve(RECORD_ROOM);
                }
            }
        }
        self.view = v;
        self.changed = true;
        self.recorded = false;
        self.steps_changed = true;
        Ok(())
    }

    /// "Clips through the arp" for `track` (see `Track::through_arp`).
    /// The track's arp follows from the next [`TimeHost::period_arps`],
    /// which clears it if it holds anything, so no note is left hanging.
    /// False if there is no such track.
    // The arp panel calls it (the arp thread's side); until then only
    // tests do.
    #[allow(dead_code)]
    pub fn set_through_arp(&mut self, track: usize, on: bool) -> bool {
        let Some(t) = self.project.tracks.get_mut(track) else {
            return false;
        };
        if t.through_arp != on {
            t.through_arp = on;
            self.changed = true;
        }
        true
    }

    /// The clip the Play page's pattern edits: the armed track's (track 1
    /// if none is armed) launched clip, else its first clip.
    fn step_focus(&self) -> (usize, Option<usize>) {
        let track = self.armed.unwrap_or(0);
        let launched = self
            .launches
            .iter()
            .find(|l| l.track == track)
            .map(|l| l.slot);
        let first = || {
            self.project
                .tracks
                .get(track)?
                .slots
                .iter()
                .position(Option::is_some)
        };
        (track, launched.or_else(first))
    }

    /// What the Play page's pattern shows. Allocates: called on a change.
    pub fn steps(&self) -> StepsData {
        let (track, slot) = self.step_focus();
        let track_name = self
            .project
            .tracks
            .get(track)
            .map_or(String::new(), |t| t.name.clone());
        let grid = Grid::new(STEP_TICKS).expect("a sixteenth is a step size");
        let clip = slot.and_then(|slot| {
            let c = self.project.clip(track, slot)?;
            let mut pitches: Vec<u8> = (0..16).collect();
            for n in &c.notes {
                if !pitches.contains(&n.note) {
                    pitches.push(n.note);
                }
            }
            let origin = self
                .launches
                .iter()
                .find(|l| l.track == track && l.slot == slot)
                .map(|l| l.origin);
            Some(StepClip {
                slot,
                name: c.name.clone(),
                steps: grid.steps(c.length),
                song_length: c.speed.to_song(c.length as f64),
                song_step: c.speed.to_song(STEP_TICKS as f64),
                origin,
                rows: pitches.into_iter().map(|p| (p, grid.row(c, p))).collect(),
            })
        });
        StepsData {
            track,
            track_name,
            clip,
        }
    }

    /// True once after the steps the Play page shows changed.
    pub fn take_steps_changed(&mut self) -> bool {
        std::mem::take(&mut self.steps_changed)
    }

    /// A touch on the Play page's pattern, as one undoable edit (a
    /// Q-Link turned through many values joins one). A line for the log.
    pub fn step_action(&mut self, a: StepAction) -> Result<String, String> {
        let grid = Grid::new(STEP_TICKS).expect("a sixteenth is a step size");
        let (track, slot) = self.step_focus();
        let (slot, made) = match slot {
            Some(s) => (s, None),
            None => {
                let StepAction::Tap { .. } = a else {
                    return Ok(String::new());
                };
                let t = self.project.tracks.get_mut(track).ok_or("no such track")?;
                let s = t
                    .slots
                    .iter()
                    .position(Option::is_none)
                    .ok_or_else(|| format!("{} has no empty slot", t.name))?;
                let name = format!("Pattern {}", s + 1);
                t.slots[s] = Some(Clip::new(&name, grid.page_length()));
                let info = self.info();
                let launch = TimeAction::Launch { track, slot: s };
                (self.launches, self.cues) =
                    clips_after(&self.project, &self.launches, &self.cues, &launch, &info);
                (s, Some(name))
            }
        };
        let clip = self.project.clip(track, slot).ok_or("the clip is gone")?;
        let (edit, join, what) = match a {
            StepAction::Tap {
                pad,
                step,
                velocity,
            } => (
                grid.toggle(clip, pad, step, velocity),
                None,
                format!("step {} of pad {}", step + 1, pad + 1),
            ),
            StepAction::Change { pad, step, change } => (
                grid.change(clip, pad, step, change),
                Some(change.key(pad, step)),
                format!("step {} of pad {}: {change:?}", step + 1, pad + 1),
            ),
        };
        if edit.is_empty() && made.is_none() {
            return Ok(String::new());
        }
        self.takes
            .edit(&mut self.project, track, slot, &edit, join)
            .map_err(|e| format!("steps: {e:?}"))?;
        self.compile()?;
        let on = if edit.add.is_empty() { "off" } else { "on" };
        Ok(match (made, a) {
            (Some(name), _) => format!(
                "steps: new clip {name} in {}, {what} {on}",
                self.project.tracks[track].name
            ),
            (None, StepAction::Tap { .. }) => format!("steps: {what} {on}"),
            (None, _) => String::new(),
        })
    }

    /// The pad hits of the last period that fell while the transport
    /// rolled (for the Play page's timing); [`TimeHost::clear_hits`]
    /// after reading.
    pub fn hits(&self) -> &[HitInfo] {
        &self.hits[..self.n_hits]
    }

    pub fn clear_hits(&mut self) {
        self.n_hits = 0;
    }

    /// A transport command, from the next period on.
    pub fn command(&mut self, cmd: Command) -> bool {
        let at = self.transport.frame();
        self.transport.command(at, cmd)
    }

    /// Follow the app's tempo (TAP TEMPO, the menu).
    pub fn follow_tempo(&mut self, bpm: f64) {
        if (self.transport.bpm() - bpm).abs() > 1e-9 {
            self.command(Command::SetTempo(bpm));
        }
    }

    /// Carry out a touch on the time engine's screens. Returns a line for
    /// the log.
    pub fn act(&mut self, a: TimeAction) -> Result<String, String> {
        let info = self.info();
        match a {
            TimeAction::Transport(cmd) => {
                if !self.command(cmd) {
                    return Err("transport: command queue full".into());
                }
                Ok(format!("transport: {cmd:?}"))
            }
            TimeAction::Undo => match self.takes.undo(&mut self.project) {
                Some(_) => {
                    let row = self.takes.undone().last().map(|t| (t.captured, t.number));
                    if row.is_some_and(|(captured, _)| captured) {
                        // It can be kept again.
                        self.capture.forget_kept();
                        self.pending = self.capture.pending();
                    }
                    self.compile().map(|_| format!("undo: {}", take_name(row)))
                }
                None => Ok("undo: nothing to undo".into()),
            },
            TimeAction::Redo => match self.takes.redo(&mut self.project) {
                Some(_) => {
                    let row = self.takes.done().last().map(|t| (t.captured, t.number));
                    self.compile().map(|_| format!("redo: {}", take_name(row)))
                }
                None => Ok("redo: nothing to redo".into()),
            },
            TimeAction::Capture => self.keep(),
            TimeAction::Arm(track) => {
                self.armed = track.filter(|&t| t < self.project.tracks.len());
                self.compile()?;
                Ok(match self.armed {
                    Some(t) => format!("armed: {}", self.project.tracks[t].name),
                    None => "armed: none".into(),
                })
            }
            TimeAction::Launch { .. } | TimeAction::StopClip(_) | TimeAction::Uncue(_) => {
                // While the music plays, they wait for the next bar line.
                let after = clips_after(&self.project, &self.launches, &self.cues, &a, &info);
                let before = (
                    std::mem::replace(&mut self.launches, after.0),
                    std::mem::replace(&mut self.cues, after.1),
                );
                if let Err(e) = self.compile() {
                    (self.launches, self.cues) = before;
                    return Err(e);
                }
                Ok(format!("clips: {a:?}"))
            }
            TimeAction::NewClip { track, slot } => {
                let bar = info.meter.bar_ticks();
                let length = info.loop_range.map_or(NEW_CLIP_BARS * bar, |(a, b)| b - a);
                let t = self.project.tracks.get_mut(track).ok_or("no such track")?;
                let s = t.slots.get_mut(slot).ok_or("no such slot")?;
                if s.is_some() {
                    return Err(format!("slot {} of {} is not empty", slot + 1, t.name));
                }
                *s = Some(Clip::new(&format!("Clip {}", slot + 1), length));
                self.armed = Some(track);
                (self.launches, self.cues) =
                    clips_after(&self.project, &self.launches, &self.cues, &a, &info);
                self.compile()?;
                // Record into it, unless already recording or counting in.
                if !matches!(info.state, State::Recording | State::CountIn) {
                    self.command(Command::Record);
                }
                Ok(format!(
                    "new clip in slot {} of track {}, recording",
                    slot + 1,
                    track + 1
                ))
            }
            TimeAction::Chip(_) => Ok(String::new()),
        }
    }

    /// CAPTURE: keep the last phrase in the armed track (track 1 if none
    /// is armed): into its playing clip when it was played in time over
    /// it, else as a new clip in its first empty slot that plays on in its
    /// place. A line for the log.
    fn keep(&mut self) -> Result<String, String> {
        if self.capture.pending().is_none() {
            return Ok("capture: nothing new to keep".into());
        }
        let phrase = self.capture.phrase().ok_or("capture: nothing played")?;
        let t = &self.transport;
        let now = Now {
            position: t.position(),
            rolling: t.state().rolling(),
            meter: t.meter(),
            loop_range: t.loop_range(),
        };
        let track = self.armed.unwrap_or(0);
        let name = format!("Capture {}", self.takes.captures() + 1);
        let k = keep(
            &phrase,
            &mut self.project,
            &self.launches,
            &mut self.takes,
            track,
            now,
            &name,
        )
        .map_err(|e| format!("capture: {e:?}"))?;
        if let Some(l) = k.launch {
            self.launches.retain(|x| x.track != l.track);
            self.cues.retain(|x| x.track != l.track);
            self.launches.push(l);
        }
        self.capture.mark_kept(phrase.id);
        self.pending = self.capture.pending();
        self.compile()?;
        let into = if k.launch.is_some() {
            "new clip in"
        } else {
            "into the clip in"
        };
        Ok(format!(
            "capture {}: {} notes, {}, {into} slot {} of {}",
            k.take,
            k.notes,
            bars_label(k.bars),
            k.slot + 1,
            self.project.tracks[track].name
        ))
    }

    /// A clip made elsewhere (the Sample page's chop to a phrase) into the
    /// first empty slot of the armed track (track 1 if none is armed), as
    /// a take that UNDO on the Clips page takes away again. It isn't
    /// launched: a tap on the Clips page plays it like any clip. Returns a
    /// line for the log.
    pub fn put_clip(&mut self, clip: Clip) -> Result<String, String> {
        let track = self.armed.unwrap_or(0);
        let t = self.project.tracks.get(track).ok_or("no such track")?;
        let name = t.name.clone();
        let slot = (0..SLOTS)
            .find(|&s| t.slots[s].is_none())
            .ok_or_else(|| format!("{name} has no empty slot for the phrase"))?;
        let notes = clip.notes.len();
        self.takes
            .capture_clip(&mut self.project, track, slot, clip)
            .map_err(|e| format!("phrase refused: {e:?}"))?;
        self.compile()?;
        Ok(format!(
            "phrase: {notes} notes in slot {} of {name}",
            slot + 1
        ))
    }

    /// Whether the armed track records the pads as pressed (its clips
    /// play through its arp) rather than what its arp played.
    fn records_pressed(&self) -> bool {
        self.armed
            .and_then(|t| self.project.tracks.get(t))
            .is_some_and(|t| t.through_arp)
    }

    /// A pad note as pressed, before any arp, in the coming period at
    /// `offset`. When the armed track's clips play through its arp, this
    /// is what it records, and what the capture history keeps.
    // The app calls it once its pads play through the per-track arps (the
    // arp thread's side); until then only tests do.
    #[allow(dead_code)]
    pub fn pad_pressed(&mut self, offset: u32, event: Event) {
        if self.records_pressed() {
            self.recorder.input(offset, event);
            self.capture.input(offset, event);
        }
    }

    /// A pad note as heard (after the arp) in the coming period, at
    /// `offset`: recorded into the armed track while the transport
    /// records, and kept in the capture history always, unless the armed
    /// track's clips play through its arp (then [`TimeHost::pad_pressed`]
    /// gives those). It lights the pattern's step either way.
    pub fn pad(&mut self, offset: u32, event: Event) {
        if !self.records_pressed() {
            self.recorder.input(offset, event);
            self.capture.input(offset, event);
        }
        let Event::NoteOn { note, velocity } = event else {
            return;
        };
        if self.n_hits >= MAX_HITS {
            return;
        }
        let span = self
            .block
            .spans()
            .iter()
            .find(|s| offset >= s.offset && offset < s.offset + s.len.max(1));
        if let Some(s) = span.filter(|s| s.state.rolling()) {
            self.hits[self.n_hits] = HitInfo {
                pad: note,
                velocity,
                tick: s.tick_at(s.frame + u64::from(offset - s.offset)),
                bpm: s.bpm,
                bar_ticks: s.meter.bar_ticks(),
            };
            self.n_hits += 1;
        }
    }

    /// A knob moved in the coming period, at `offset`: parameter `addr` is
    /// now `value`. While the transport records and `addr` is on the armed
    /// track, it becomes a parameter lock on the step it belongs to (see
    /// `mpfree_play::Recorder`).
    // No knob is mapped to a parameter in the pad player yet (that needs
    // the mod resolver); the Q-Links call this once one is.
    #[allow(dead_code)]
    pub fn knob(&mut self, offset: u32, addr: mpfree_play::mpfree_param::ParamAddr, value: f32) {
        self.recorder.input(offset, Event::Lock { addr, value });
    }

    /// FILL on or off from `offset` in the coming period: notes with the
    /// FILL and !FILL trig conditions follow it from that frame on. For
    /// the FILL button (hold for a fill, double-press to latch).
    // No button is mapped to FILL yet: the pad player picks one under the
    // layout rules.
    #[allow(dead_code)]
    pub fn set_fill(&mut self, on: bool, offset: u32) {
        self.player.set_fill(on, offset);
    }

    /// FILL, as of the last period, for the button's light.
    #[allow(dead_code)]
    pub fn fill(&self) -> bool {
        self.player.fill()
    }

    /// Run the transport for the coming period, before the app's process,
    /// so the arp can follow it ([`TimeHost::arp_clock`]).
    /// [`TimeHost::period`] then plays and records on the same block.
    pub fn advance(&mut self, frames: u32) -> bool {
        let ok = self.transport.process(frames, &mut self.block);
        self.advanced = Some(ok);
        ok
    }

    /// The arp's clock for the coming period, from the first span of the
    /// block [`TimeHost::advance`] made: None while stopped, so the arp
    /// runs on its own tempo. The count-in counts as playing. `jumped` is
    /// set by a start or jump before the first span, or by one later in the
    /// last period (the arp follows the first span only, so it catches up
    /// one period later).
    // The arp thread's App::set_clock takes it: `app.set_clock(time.arp_clock())`
    // after `time.advance` in the audio loop. Until that lands only tests call it.
    pub fn arp_clock(&mut self) -> Option<mpfree_arp::Clock> {
        let jump = |m: &Mark| {
            matches!(
                m,
                Mark::Started { .. }
                    | Mark::CountIn { .. }
                    | Mark::Located { .. }
                    | Mark::Looped { .. }
            )
        };
        let first = self.block.spans().first().copied();
        let first_offset = first.map_or(0, |s| s.offset);
        let mut jumped = std::mem::take(&mut self.jump_pending);
        for m in self.block.markers() {
            if jump(&m.mark) {
                if m.offset <= first_offset {
                    jumped = true;
                } else {
                    self.jump_pending = true;
                }
            }
        }
        let span = first?;
        if !span.state.rolling() {
            return None;
        }
        Some(mpfree_arp::Clock {
            anchor_tick: span.anchor_tick,
            frames_since_anchor: span.frame.saturating_sub(span.anchor_frame),
            bpm: span.bpm,
            playing: true,
            jumped,
            swing: (span.swing.amount > 50).then_some((span.swing.grid, span.swing.amount)),
            bar_ticks: u32::try_from(span.meter.bar_ticks()).unwrap_or(4 * mpfree_arp::PPQ),
        })
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
    // The pad player calls it once the app owns the per-track arps (the
    // arp thread's side); until then only tests do.
    #[allow(dead_code)]
    pub fn period_arps(&mut self, frames: u32, pads: &mut impl Pads, arps: &mut TrackArps) {
        for (t, track) in self.project.tracks.iter().enumerate() {
            arps.set_through(t, track.through_arp);
            arps.set_dest(t, track.dest);
        }
        self.run(frames, pads, Some(arps));
    }

    fn run(&mut self, frames: u32, pads: &mut impl Pads, arps: Option<&mut TrackArps>) {
        let ok = match self.advanced.take() {
            Some(ok) => ok,
            None => self.transport.process(frames, &mut self.block),
        };
        if !ok {
            return;
        }
        self.capture.process(&self.block);
        // The Capture button shows what it would keep: a new phrase, or one
        // more bar of it, is a change for the screen (once a bar at most).
        let pending = self.capture.pending();
        if pending != self.pending {
            self.pending = pending;
            self.changed = true;
        }
        self.sink.n = 0;
        match arps {
            // The player's notes go through the per-track arps (a through
            // track's into its arp, the rest as they are), then to the pads.
            Some(arps) => {
                self.player
                    .process(&self.view, &self.block, &mut arps.through_sink());
                arps.finish(&self.block, &mut *self.sink);
            }
            None => self
                .player
                .process(&self.view, &self.block, &mut *self.sink),
        }
        for o in &self.sink.out[..self.sink.n] {
            match o.event {
                Event::NoteOn { note, velocity } => {
                    pads.hit_at(usize::from(note), velocity, o.offset);
                }
                Event::NoteOff { note } => pads.release_at(usize::from(note), o.offset),
                Event::AllOff => pads.stop_all(),
                // Parameter locks need the mod resolver (mpfree-mod), which
                // the pad player does not run yet.
                Event::Lock { .. } | Event::Unlock { .. } => {}
            }
        }
        self.recorder.process(&self.view, &self.block);
        // Cues that started on a bar line become the tracks' launches: the
        // view is compiled again.
        let mut compile = false;
        for i in 0..self.player.started().len() {
            self.player.started()[i].apply(&mut self.launches, &mut self.cues);
            compile = true;
        }
        // Recorded and erased notes and locks go into the clips and, in
        // place, into the view, unless it has no room left.
        let mut recorded = false;
        for i in 0..self.player.erased().len() {
            let e = self.player.erased()[i];
            if self.takes.erase(&mut self.project, &e) {
                recorded = true;
                compile |= !self.view.remove_note(e.track, e.slot, &e.note);
            }
        }
        for i in 0..self.player.erased_locks().len() {
            let e = self.player.erased_locks()[i];
            if self.takes.erase_lock(&mut self.project, &e) {
                recorded = true;
                compile |= !self.view.remove_lock(e.track, e.slot, &e.lock);
            }
        }
        for i in 0..self.recorder.taken().len() {
            let k = self.recorder.taken()[i];
            if self.takes.add(&mut self.project, &k).is_ok() {
                recorded = true;
                compile |= !self.view.add_note(k.track, k.slot, &k.note);
            }
        }
        for i in 0..self.recorder.taken_locks().len() {
            let k = self.recorder.taken_locks()[i];
            if let Ok(old) = self.takes.add_lock(&mut self.project, &k) {
                recorded = true;
                compile |= !self.view.add_lock(k.track, k.slot, &k.lock, old.as_ref());
            }
        }
        if compile {
            // The clips only gained or lost notes and locks, and cues became
            // launches of clips they had compiled with, so this compiles.
            let _ = self.compile();
        } else {
            self.recorded |= recorded;
            self.since_told += u64::from(frames);
            if self.recorded && self.since_told >= self.tell_every {
                self.changed = true;
                self.recorded = false;
                self.since_told = 0;
            }
        }
        if self.metronome {
            for span in self.block.spans() {
                if matches!(span.state, State::CountIn | State::Recording) {
                    let bar = span.meter.bar_ticks();
                    for (tick, offset) in span.lines(span.meter.beat_ticks()) {
                        self.click.schedule(offset, tick.rem_euclid(bar) == 0);
                    }
                }
            }
        }
    }

    /// Add the click to the rendered period; returns the peak afterwards.
    pub fn mix_click(&mut self, left: &mut [f32], right: &mut [f32]) -> f32 {
        self.click.mix(left, right)
    }

    /// The transport's description of the last period (for the looper).
    pub fn block(&self) -> &Block {
        &self.block
    }

    /// The song position as bar.beat.tick, for log lines.
    pub fn position_text(&self) -> String {
        let t = &self.transport;
        t.meter().position(t.position()).to_string()
    }

    pub fn info(&self) -> TransportInfo {
        TransportInfo::of(&self.transport)
    }

    /// True once after the clips, launches or takes changed. Notes and
    /// locks being recorded are told at most every quarter second.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// Everything the screens show.
    pub fn state(&self) -> TimeState {
        let mut st = TimeState::from_engine(
            self.info(),
            &self.project,
            &self.launches,
            self.armed,
            &self.takes,
        );
        st.capture = self.pending.map(|p| bars_label(p.bars));
        st.cues = self.cues.clone();
        st
    }

    /// Notes lost because a table was full (should stay 0).
    pub fn dropped(&self) -> u64 {
        self.sink.dropped
            + self.recorder.dropped()
            + self.player.stats().dropped
            + self.capture.dropped()
    }

    /// The time engine's part of the status line.
    pub fn describe(&self) -> String {
        let t = &self.transport;
        format!(
            "time: {:?} at {}, {:.1} BPM, {} takes, {} notes lost to full tables",
            t.state(),
            t.meter().position(t.position()),
            t.bpm(),
            self.takes.done().len(),
            self.dropped()
        )
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

    #[test]
    fn a_clip_put_from_elsewhere_lands_in_the_first_empty_slot_and_undo_takes_it() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut clip = Clip::new("Break phrase", 4 * 960);
        for k in 0..4u8 {
            clip.add_note(mpfree_seq::Note::new(i64::from(k) * 960, 480, k, 127))
                .unwrap();
        }
        // Slot 1 of track 1 is taken: the phrase goes in slot 2.
        h.project.tracks[0].slots[0] = Some(Clip::new("taken", 960));
        let line = h.put_clip(clip.clone()).unwrap();
        assert_eq!(line, "phrase: 4 notes in slot 2 of Track 1");
        assert_eq!(h.project.clip(0, 1), Some(&clip));
        assert!(h.launches.is_empty(), "not launched");
        // The armed track takes it.
        h.act(TimeAction::Arm(Some(2))).unwrap();
        assert!(h
            .put_clip(clip.clone())
            .unwrap()
            .ends_with("slot 1 of Track 3"));
        // UNDO on the Clips page takes the last one away, then the first.
        h.act(TimeAction::Undo).unwrap();
        assert!(h.project.clip(2, 0).is_none());
        h.act(TimeAction::Undo).unwrap();
        assert!(h.project.clip(0, 1).is_none());
        // A full track says so.
        for s in 0..SLOTS {
            h.project.tracks[2].slots[s] = Some(Clip::new("full", 960));
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
        let clip = h.project.tracks[2].slots[0].as_ref().unwrap();
        assert_eq!(clip.notes.len(), 1);
        let locks: Vec<_> = clip
            .locks
            .iter()
            .map(|l| (l.tick, l.sound, l.addr, l.value))
            .collect();
        assert_eq!(locks, vec![(clip.notes[0].tick, None, cutoff, 0.6)]);
        assert_eq!(h.act(TimeAction::Undo).unwrap(), "undo: take 1");
        let clip = h.project.tracks[2].slots[0].as_ref().unwrap();
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

    /// "Clips through the arp" on: the track records the chord as pressed
    /// (not what the app's arp played), and its clip plays back through
    /// the track's arp, one note per sixteenth.
    #[test]
    fn a_through_track_records_the_chord_and_plays_it_through_its_arp() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        assert!(h.set_through_arp(2, true));
        assert!(h.project.tracks[2].through_arp);
        let mut arps = arp_up(2);
        let hits = record_a_chord(&mut h, &mut arps);
        assert!(arps.through(2) && !arps.through(1));
        let clip = h.project.tracks[2].slots[0].as_ref().unwrap();
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
        let clip = h.project.tracks[2].slots[0].as_ref().unwrap();
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
                })
                .collect();
            c
        };
        h.project.tracks[0].slots[0] = Some(one_bar("A", &[(0, 4), (1920, 4)]));
        h.project.tracks[0].slots[1] = Some(one_bar("B", &[(0, 9)]));
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
        let mut fresh = PlayView::compile_cued(&h.project, &h.launches, &h.cues).unwrap();
        fresh.set_armed(h.armed);
        assert_eq!(h.view, fresh, "the view in place is what compiling gives");
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
}
