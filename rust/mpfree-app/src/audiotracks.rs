//! The audio tracks in the pad player: `mpfree-audiotracks`' [`AudioHost`]
//! on the transport, recording the line input onto the Song view's audio
//! lanes (layout spec 4c).
//!
//! - [`TrackHost::period`] runs the tracks once a period on the
//!   transport's block and tempo map, after the looper (which reads the
//!   line input), and gives what they play to the mixer's "Audio" track
//!   (`mixing::AUDIO`). Nothing in it allocates: the host's worker makes
//!   and drops every big buffer.
//! - Audio tracks play and record in song mode only (layout spec 4c): the
//!   host sends `SetActive` as song mode turns on and off. Monitoring goes
//!   on either way. REC records an armed track as the transport records.
//! - [`TrackHost::action`] answers the Song view's [`AudioAction`]s: arm,
//!   "+ Audio", the track panel (input, monitor, mute, solo, level and
//!   pan), Split (at the playhead), Erase, Undo and the Q-Links. Each
//!   answer is a line for the log; a refusal says why.
//! - [`TrackHost::bring`] puts a sound from the Sample page on a track
//!   (its Slice tool's Put on Audio track, the one Sample page's step 6):
//!   the first empty track, from the playhead's bar, a region a slice, as
//!   one undo step.
//! - [`TrackHost::fill`] puts the lanes, the waveforms, the memory gauge
//!   and the counters into the time engine's [`TimeState`], for the Song
//!   view. [`TrackHost::take_changed`] says when they changed, checked a
//!   few times a second without allocating.
//! - Each track's level bar (layout spec 4c: 3 px along the header's
//!   bottom edge) follows its sound, and the line input while it is
//!   armed, measured each period without allocating
//!   ([`TrackHost::take_levels`]); the screen gets the eight numbers on
//!   their own, not a whole song state.
//! - Recording from inside (the instrument redesign's step 9): a track
//!   can record an instrument's strip, a submix or the master, before or
//!   after its effects ([`TrackHost::set_inside`]). Each period, before
//!   the tracks run, [`TrackHost::feed_inside`] copies the mixer's capture
//!   point for it (`MixHost::audio_take`) into the engine and keeps the
//!   engine's latency for it up to date. A track recording from inside
//!   never plays its input through, so nothing is heard twice.
//! - Waveforms are read a take at a time with [`AudioHost::wave`], at a
//!   small fixed cost a period; a take that records is read again as it
//!   grows.
//! - Projects (`project.rs`, part `audiotracks`): a save reads every take
//!   over a few periods ([`AudioHost::save`]); opening a project or New
//!   empties the tracks ([`TrackHost::clear`], once nothing records) and
//!   the project's part goes in when it has been read
//!   ([`TrackHost::open`]).
//! - Recording to files (the audio tracks rethink, phase 3a): once the
//!   screen has recovered what a cut-off run left in the library's work
//!   folder, [`TrackHost::record_to`] has every take recorded to a file of
//!   its own there as it records (`NAME_ID.wav`, NAME the track's name,
//!   kept up to date by [`TrackHost::sync`]). A save copies the finished
//!   files into the project, and [`TrackHost::saved_into`] tells the host
//!   where they went. What happens to a take's file goes to the log
//!   ([`TrackHost::take_file_line`]).
//!
//! The memory is set aside when MPFree starts ([`SECONDS`] of stereo). The
//! takes also stay in RAM until streaming plays them from their files
//! (phase 3b).

use mpfree_audiotracks::copying::{ClipWant, CopyState};
use mpfree_audiotracks::host::{AudioHost, CueAudio, ImportRefused, TakeWave, MAX_COLUMNS};
use mpfree_audiotracks::save::{Saved, TrackFile};
use mpfree_audiotracks::streaming::StreamConfig;
use mpfree_audiotracks::{
    Answer, Change, ClipPlay, ClipRegion, Command, FollowTempo, Input, LoopPlay, LoopPlayer,
    Monitor, Plays, RecordingId, Region, Retime, RetimeRefused, SongStep, TakeInfo, MAX_CHANGES,
    MAX_CLIP_REGIONS, MAX_REGIONS, MAX_TAKES, MAX_TRACKS, PEAK_BLOCK,
};
use mpfree_play::{AudioAnswer, AudioPlays};
use mpfree_seq::{RecId, TrackId};
use mpfree_stretch::sched::Priority;
use mpfree_time_ui::song::{
    audio_blocks, input_chip, take_lanes, AudioLane, ClipBlock, ClipPiece, MixFrom, MONITORS,
};
use mpfree_time_ui::view::{
    audio_color, track_color, AudioAction, RegionId, StretchNow, TimeState, TransportInfo,
};
use mpfree_time_ui::widgets::CELL_WAVE;
use mpfree_transport::{Block, TempoMap, Tick};
use mpfree_ui::qlink::QLinkCard;

use crate::mixing::MixHost;
use crate::time::{AudioNeed, CopyNeed, CueKey};
use crate::PERIOD;
use mpfree_fx::ids::mixer as mx;

/// Put in a cell under way ([`TrackHost::bring_to_cell`]): the sound
/// copying in as a take of its own, for a clip in project track `track`.
struct CellSound {
    track: usize,
    name: String,
    frames: usize,
    bpm: f64,
}

/// Prefs "Audio track memory" (layout spec 4c): the stereo seconds every
/// take shares, 60 s by default (about 21 MB at 44.1 kHz). It is set aside,
/// and kept in RAM, when MPFree starts, so a new size applies from the next
/// start, like Loop memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioMemory {
    Sec30,
    #[default]
    Min1,
    Min2,
    Min5,
    Min10,
}

impl AudioMemory {
    pub const ALL: [AudioMemory; 5] = [
        AudioMemory::Sec30,
        AudioMemory::Min1,
        AudioMemory::Min2,
        AudioMemory::Min5,
        AudioMemory::Min10,
    ];

    pub fn seconds(self) -> u32 {
        match self {
            AudioMemory::Sec30 => 30,
            AudioMemory::Min1 => 60,
            AudioMemory::Min2 => 120,
            AudioMemory::Min5 => 300,
            AudioMemory::Min10 => 600,
        }
    }

    /// The kept setting's value: the seconds.
    pub fn parse(v: &str) -> Option<AudioMemory> {
        let n = v.trim().parse::<u32>().ok()?;
        AudioMemory::ALL.into_iter().find(|m| m.seconds() == n)
    }

    /// What it costs, in whole megabytes, at `rate` (stereo f32).
    pub fn megabytes(self, rate: u32) -> usize {
        (rate as usize * self.seconds() as usize * 2 * 4).div_ceil(1_000_000)
    }

    /// "1:00".
    pub fn time(self) -> String {
        format!("{}:{:02}", self.seconds() / 60, self.seconds() % 60)
    }

    /// Prefs' value for it: "1:00 stereo, 22 MB".
    pub fn row(self, rate: u32) -> String {
        format!("{} stereo, {} MB", self.time(), self.megabytes(rate))
    }
}
/// Periods between checks for a change the Song view should show (about
/// 23 ms at 128 frames).
const CHECK_EVERY: u64 = 8;
/// Periods after "+ Track"'s Audio track in which the audio track that
/// comes is armed (about 2.7 s at 128 frames and 48 kHz): an add the time
/// engine refused arms nothing later.
const ARM_WINDOW: u64 = 1000;
/// Periods between new reads of a recording take's waveform (about a
/// quarter of a second).
const GROW_EVERY: u64 = 86;
/// Periods between looks at the frames takes on their files played as
/// silence (about a second): a stick falling behind gives a line a
/// second at most.
const DROPOUTS_EVERY: u64 = 344;

/// Room for each audio track's name in the lines (bytes; a longer name is
/// cut short there).
const NAME_ROOM: usize = 64;

/// What a take's file is named for on an engine track whose project track
/// has no name, as its lane says.
const AUDIO_N: [&str; MAX_TRACKS] = [
    "Audio 1", "Audio 2", "Audio 3", "Audio 4", "Audio 5", "Audio 6", "Audio 7", "Audio 8",
];

/// Which project track each of the audio engine's tracks plays (the audio
/// tracks rethink, phase 1a: an audio track is a track in the project's
/// one track list, [`mpfree_seq::TrackKind::Audio`]): the track's lasting
/// id, or [`TrackId::NONE`] for a free one. The engine's tracks are fixed
/// arrays, so a track added or taken away never allocates on the audio
/// thread; this table is the only place the two meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slots(pub [TrackId; MAX_TRACKS]);

impl Default for Slots {
    fn default() -> Slots {
        Slots([TrackId::NONE; MAX_TRACKS])
    }
}

impl Slots {
    /// The engine track that plays project track `id`.
    /// A report about engine track `p.track` made about the project
    /// track it plays, by its lasting id (`Subject::audio_track`); one
    /// about no track, or an engine track playing none, is about the
    /// whole system.
    pub fn name_problem(&self, p: mpfree_audiotracks::files::Problem) -> mpfree_status::Report {
        match p.track.and_then(|s| self.0.get(s)) {
            Some(id) if id.is_given() => p.report.about(mpfree_status::Subject::audio_track(id.0)),
            _ => p.report,
        }
    }

    pub fn slot_of(&self, id: TrackId) -> Option<usize> {
        if !id.is_given() {
            return None;
        }
        self.0.iter().position(|&s| s == id)
    }

    /// The project track engine track `slot` plays.
    pub fn id_of(&self, slot: usize) -> Option<TrackId> {
        self.0.get(slot).copied().filter(|id| id.is_given())
    }

    /// The first engine track that plays nothing.
    pub fn free(&self) -> Option<usize> {
        self.0.iter().position(|s| !s.is_given())
    }
}

/// Why the audio tracks' own Undo, and Redo, stop at a song edit's step
/// (the Arrange refactor's 3.4): the song's UNDO and REDO move them.
const SONG_UNDO: &str =
    "the next undo is the song edit that moved the regions: UNDO takes it back, with the song";
const SONG_REDO: &str = "the next redo is the song edit that moved the regions: SHIFT + UNDO \
                         brings it back, with the song";

/// Where an audio track records from inside the mix (the instrument
/// redesign's step 9): a strip (an instrument's, a submix or the master)
/// and whether after its effects, before its fader (the Song view panel's
/// Mix, so the strip's level never changes the take); or after its fader
/// (`after_fader`, which implies after its effects: the input list's
/// Pads and Mix › a strip, phase 4's board 4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inside {
    pub strip: mpfree_fx::StripId,
    pub after_effects: bool,
    pub after_fader: bool,
}

impl Inside {
    /// Where the mixer takes it.
    pub fn point(self) -> mpfree_fx::CapturePoint {
        use mpfree_fx::CapturePoint;
        if self.after_fader {
            CapturePoint::AfterFader
        } else if self.after_effects {
            CapturePoint::BeforeFader
        } else {
            CapturePoint::BeforeInserts
        }
    }
}

/// How the input list names the pads' instrument ("Pads · Kit 1"): a name
/// [`TrackHost::offer`] keeps, as the strip's own would lose the pads.
pub const PADS_NAME: &str = "Pads \u{b7} ";

/// What the input list offered track `track` (phase 4, board 4), by the
/// list's `Select` index ([`mpfree_time_ui::inputs`]): where each choice
/// is inside the mix, and why the mixer refuses those it does.
#[derive(Clone, Debug, Default)]
struct Offered {
    track: usize,
    places: Vec<(usize, String, Inside)>,
    refused: Vec<(usize, String)>,
}

/// A strip the input list offers, as the app knows it: the mixer's
/// strip, its name, and the project track whose instrument plays into it.
#[derive(Clone, Debug, PartialEq)]
pub struct InputSource {
    pub strip: mpfree_fx::StripId,
    pub name: String,
    pub owner: Option<usize>,
}

/// The audio tracks, their host and what the Song view was last given.
const NO_CLIP_REGION: ClipRegion = ClipRegion {
    start: 0,
    end: 0,
    rec: RecordingId::NONE,
    src: 0,
    phase: 0.0,
    gain: 1.0,
    fade_in: 0,
    fade_out: 0,
    into: 0,
    left: 0,
    follow: FollowTempo::Off,
    keep_pitch: false,
    bpm: 0.0,
};

/// A clip region's Follow tempo in the engine's words.
fn follow_in(f: mpfree_seq::FollowTempo) -> FollowTempo {
    match f {
        mpfree_seq::FollowTempo::Off => FollowTempo::Off,
        mpfree_seq::FollowTempo::Repitch => FollowTempo::Repitch,
        mpfree_seq::FollowTempo::Stretch => FollowTempo::Stretch,
    }
}

/// A song region's Follow tempo in the clip's words.
fn follow_out(f: FollowTempo) -> mpfree_seq::FollowTempo {
    match f {
        FollowTempo::Off => mpfree_seq::FollowTempo::Off,
        FollowTempo::Repitch => mpfree_seq::FollowTempo::Repitch,
        FollowTempo::Stretch => mpfree_seq::FollowTempo::Stretch,
    }
}

/// Song region `r` of a take made at `made_at` as an audio clip's region
/// on the same ticks: its Follow tempo kept, and its tempo as 0 when it is
/// its take's. A region with no tempo while its take has one (set by
/// hand) plays at its recorded rate whatever it says, so it goes as Off,
/// playing the same.
fn clip_region_of(r: &Region, rec: RecId, made_at: f64) -> mpfree_seq::ClipRegion {
    let none = r.bpm == 0.0 && made_at != 0.0;
    mpfree_seq::ClipRegion {
        start: r.start,
        end: r.end,
        rec,
        src: r.src,
        phase: r.phase,
        gain: r.gain,
        fade_in: r.fade_in,
        fade_out: r.fade_out,
        into: r.into,
        left: r.left,
        follow: if none {
            mpfree_seq::FollowTempo::Off
        } else {
            follow_out(r.follow)
        },
        keep_pitch: r.keep_pitch && !none,
        bpm: if r.bpm == made_at { 0.0 } else { r.bpm },
    }
}

/// The one scheduler for stretches made ahead (`mpfree_stretch::sched`,
/// the audio tracks rethink, phase 5b), as the program configures it at
/// its start: its worker where the sampler's sound jobs run, off the
/// sound cores at idle priority, and each job's memory promised in the
/// shared memory check ([`crate::memory::reserve`]), beside every load's,
/// until the job ends.
pub fn stretch_config() -> mpfree_stretch::sched::Config {
    mpfree_stretch::sched::Config {
        on_start: std::sync::Arc::new(mpfree_lanes::os::idle_job),
        promise: |need| {
            crate::memory::reserve(crate::memory::free_bytes, need)
                .map(mpfree_stretch::sched::Hold::new)
        },
        ..mpfree_stretch::sched::Config::default()
    }
}

/// A clip's region in the engine's words, naming its recording by the
/// same lasting id.
/// Audio clip `c` as the host wants it for its tempo copies, at
/// `priority` and `bpm` (0: the tempo at the playhead). None for a clip
/// with no audio regions.
fn clip_want(c: &mpfree_seq::Clip, priority: Priority, bpm: f64) -> Option<ClipWant> {
    let a = c.audio.as_ref()?;
    let first = clip_region_in(a.regions.first()?);
    let mut regions = [first; MAX_CLIP_REGIONS];
    for (d, r) in regions.iter_mut().zip(&a.regions) {
        *d = clip_region_in(r);
    }
    Some(ClipWant {
        regions,
        n: a.regions.len().min(MAX_CLIP_REGIONS),
        len: c.length,
        last_frame: a.last_frame,
        priority,
        bpm,
    })
}

fn clip_region_in(r: &mpfree_seq::ClipRegion) -> ClipRegion {
    ClipRegion {
        start: r.start,
        end: r.end,
        rec: RecordingId(r.rec.0),
        src: r.src,
        phase: r.phase,
        gain: r.gain,
        fade_in: r.fade_in,
        fade_out: r.fade_out,
        into: r.into,
        left: r.left,
        follow: follow_in(r.follow),
        keep_pitch: r.keep_pitch,
        bpm: r.bpm,
    }
}

/// What the player's `p` says, in the engine's words: a clip's regions
/// copied into `room` (the first [`MAX_CLIP_REGIONS`]), each naming its
/// recording by the same lasting id.
fn plays_in<'r>(p: &AudioPlays, room: &'r mut [ClipRegion; MAX_CLIP_REGIONS]) -> Plays<'r> {
    match *p {
        AudioPlays::Nothing => Plays::Nothing,
        AudioPlays::Song => Plays::Song,
        AudioPlays::Clip {
            regions,
            origin,
            len,
            last_frame,
            gain,
            clip,
            stamp,
        } => {
            let n = regions.len().min(MAX_CLIP_REGIONS);
            for (d, r) in room.iter_mut().zip(regions) {
                *d = clip_region_in(r);
            }
            Plays::Clip(ClipPlay {
                regions: &room[..n],
                origin,
                len,
                last_frame,
                gain,
                clip: clip.0,
                stamp,
            })
        }
        // A loop cell (phase 4b): the track player's loops play it; the
        // region player hears nothing.
        AudioPlays::Loop { .. } => loop_plays(p),
    }
}

/// What a track plays as the loops see it (phase 4b): a loop as it is,
/// anything else as nothing (the loop player only tells a loop from what
/// isn't one), so no regions are copied.
fn loop_plays(p: &AudioPlays) -> Plays<'static> {
    match *p {
        AudioPlays::Loop {
            id,
            origin,
            len,
            clip,
            stamp,
        } => Plays::Loop(LoopPlay {
            id: id.0,
            origin,
            len,
            clip: clip.0,
            stamp,
        }),
        _ => Plays::Nothing,
    }
}

/// Looks between asking again for a waiting cue's audio that couldn't be
/// held (about a quarter of a second).
pub const WAIT_RETRY: u32 = 64;

/// A cue waiting for its audio, as the host holds it: the time engine's
/// key, the host's number, what it needs, and what the host last said
/// (None: asked, no answer yet).
struct Waiting {
    key: CueKey,
    cue: u32,
    needs: Vec<(RecordingId, i64, i64)>,
    held: Option<bool>,
}

/// A cue waiting for its tempo copies (5b.4), as the time engine lists
/// it: the keys of the copies it needs, and what the engine was last told
/// (None: nothing yet).
struct CopyWaiting {
    key: CueKey,
    keys: Vec<mpfree_audiotracks::copies::CopyKey>,
    told: Option<bool>,
}

/// What Clear copies says (System status' Memory, 5c.4): how many of the
/// audio tracks' tempo copies and the sampler's Warps went.
pub fn cleared_line(tracks: usize, warps: usize) -> String {
    let n = tracks + warps;
    match n {
        0 => "Nothing to clear: every copy is playing or about to".to_string(),
        1 => "1 copy cleared: it is made again when it's next needed".to_string(),
        n => format!("{n} copies cleared: each is made again when it's next needed"),
    }
}

pub struct TrackHost {
    pub host: AudioHost,
    rate: u32,
    /// The line input and the tracks' sound, interleaved stereo.
    input: Box<[f32; 2 * PERIOD]>,
    out: Box<[f32; 2 * PERIOD]>,
    /// What each track records from (the engine keeps it too; this is for
    /// the header's chip).
    inputs: [Input; MAX_TRACKS],
    /// The tracks recording from inside the mix ([`TrackHost::set_inside`]),
    /// the name each was given, the latency last sent to the engine for
    /// it, and whether its strip has gone from the mix (it records
    /// silence, said once).
    inside: [Option<Inside>; MAX_TRACKS],
    inside_name: [String; MAX_TRACKS],
    inside_lat: [Option<u32>; MAX_TRACKS],
    inside_lost: [bool; MAX_TRACKS],
    /// What each track recorded from inside the mix last, for the panel's
    /// Mix, and what the mix offers, by name, as the app last gave it
    /// ([`TrackHost::offer`]).
    inside_last: [Option<Inside>; MAX_TRACKS],
    sources: Vec<(String, mpfree_fx::StripId)>,
    /// What the input list offered last (phase 4, board 4), so a pick
    /// names one ([`AudioAction::InputPick`]).
    offered: Option<Offered>,
    /// Periods a track recording from inside got silence because its
    /// strip wasn't in the mix (counted, and said in the log).
    pub inside_silent: u64,
    /// Each track's level (0 to 2) and balance (-1 to 1), monitor mode,
    /// mute and solo, as last sent.
    volume: [f32; MAX_TRACKS],
    pan: [f32; MAX_TRACKS],
    monitor: [Monitor; MAX_TRACKS],
    /// The track whose panel is open over the lanes, and the track whose
    /// take lanes show.
    panel: Option<usize>,
    takes_open: Option<usize>,
    /// A region just moved: its track and new start, until the engine has
    /// it and the screen can pick it again ([`TrackHost::take_repick`]).
    repick: Option<(usize, Tick)>,
    /// The last `SetActive` sent.
    active: bool,
    /// The input latency last sent (frames), None before the first.
    latency: Option<u32>,
    /// Waveforms read so far, a take each, and the take being read.
    waves: Vec<TakeWave>,
    reading: Option<u16>,
    /// The period of the last waveform asked for while recording.
    grown_at: u64,
    periods: u64,
    /// A take was made whole on the stick since last asked
    /// ([`TrackHost::take_joined`]).
    joined: bool,
    /// What the lanes showed at the last check, and whether that changed.
    seen: u64,
    changed: bool,
    /// The regions' spans as last handed to the time engine
    /// ([`TrackHost::spans_if_changed`]), as a fingerprint.
    spans_seen: u64,
    /// The recordings' tempos as last handed to the time engine
    /// ([`TrackHost::tempos_if_changed`]), as a fingerprint.
    tempos_seen: u64,
    /// A project opening: the tracks empty once nothing records, then the
    /// project's part (if it has come) goes in.
    clearing: bool,
    pending: Option<Saved>,
    /// A line for the log from the open.
    note: Option<String>,
    /// What the open couldn't put back, for the screen
    /// ([`TrackHost::take_left_out`]).
    left_out: Option<String>,
    /// Each track's level bar, 0 to 1 (see [`meter`]), and the bars last
    /// given to the screen.
    meter: [f32; MAX_TRACKS],
    meter_shown: [f32; MAX_TRACKS],
    /// Which project track each engine track plays (phase 1a), and where
    /// that track is in the track list as last found ([`TrackHost::sync`]).
    pub slots: Slots,
    place: [Option<usize>; MAX_TRACKS],
    /// Room for what each track's clips play this period, at its start
    /// and at each change: the player's regions, copied in the engine's
    /// words ([`TrackHost::period`]).
    clip_room: Box<[[[ClipRegion; MAX_CLIP_REGIONS]; 1 + MAX_CHANGES]; MAX_TRACKS]>,
    /// Each engine track's project track's name as last found, for the
    /// hint and log lines ([`TrackHost::who`]); room made at start, so
    /// [`TrackHost::sync`] never allocates. A track gone keeps its name.
    names: [String; MAX_TRACKS],
    /// Each engine track silenced as last sent: its project track muted,
    /// left out by a solo, or gone (taken away by UNDO, its recordings
    /// kept for a REDO).
    silent: [bool; MAX_TRACKS],
    /// The period "+ Track"'s Audio track was picked in: the next audio
    /// track given an engine track (within [`ARM_WINDOW`]) is armed
    /// (layout spec 4c).
    arm_next: Option<u64>,
    /// The folder to record takes into, until the host takes it
    /// ([`TrackHost::record_to`]: it waits while a track records).
    files_dir: Option<std::path::PathBuf>,
    /// Frames each take on its file has played as silence, as last said
    /// in the log ([`TrackHost::take_file_line`]).
    silent_seen: [u64; MAX_TAKES],
    /// The recordings the project's audio clips, and the clips its undo
    /// steps could bring back, name, sorted ([`TrackHost::keep_named`]).
    named: Vec<RecId>,
    /// Cues waiting for their audio ([`TrackHost::hold_waits`]), the
    /// host's number for the next, and looks since short ones were last
    /// asked again.
    waits: Vec<Waiting>,
    next_cue: u32,
    /// The cues waiting for their tempo copies (5b.4).
    copy_waits: Vec<CopyWaiting>,
    waits_looked: u32,
    /// Put in a cell, the sound copying in.
    cell_sound: Option<CellSound>,
    /// The frame the last period ended on, for a ringing clip's playhead.
    frame_now: u64,
    /// The recordings the project opened last listed, with the take each
    /// was ("take 4"), for a clip whose audio is missing.
    opened: Vec<(RecId, String)>,
    /// The song's meter, for the bar a dropout was at.
    time_sig: mpfree_transport::time::Meter,
    /// Loops on audio tracks (phase 4b): where each
    /// track's loops sound this period and how loud
    /// ([`TrackHost::plan_loops`]).
    loops: LoopPlayer,
}

/// Playing takes from their files, for Dev's Performance page.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StreamNumbers {
    /// Takes on their files, of all takes.
    pub on_file: usize,
    pub takes: usize,
    /// Seconds of takes on their files that played as silence, the drive
    /// being late, since the start.
    pub silent_s: f64,
    /// Starts of regions of takes on their files held in the cache, of
    /// those wanted (fewer if the cache is too small for them all).
    pub heads_held: usize,
    pub heads_wanted: usize,
}

impl TrackHost {
    /// The tracks at `rate` with `seconds` of stereo memory, and the
    /// host's worker thread. Allocates: call it before the sound starts.
    pub fn new(rate: u32, seconds: u32) -> TrackHost {
        let mut host = AudioHost::new(rate, seconds);
        // Clip launch mode is the default: the tracks wait for song mode.
        host.command(Command::SetActive(false));
        TrackHost {
            host,
            rate,
            input: Box::new([0.0; 2 * PERIOD]),
            out: Box::new([0.0; 2 * PERIOD]),
            inputs: [Input::Left; MAX_TRACKS],
            inside: [None; MAX_TRACKS],
            inside_name: Default::default(),
            inside_lat: [None; MAX_TRACKS],
            inside_lost: [false; MAX_TRACKS],
            inside_last: [None; MAX_TRACKS],
            sources: Vec::new(),
            offered: None,
            inside_silent: 0,
            volume: [1.0; MAX_TRACKS],
            pan: [0.0; MAX_TRACKS],
            monitor: [Monitor::Auto; MAX_TRACKS],
            panel: None,
            takes_open: None,
            repick: None,
            active: false,
            latency: None,
            waves: Vec::with_capacity(mpfree_audiotracks::MAX_TAKES),
            reading: None,
            grown_at: 0,
            periods: 0,
            joined: false,
            seen: 0,
            spans_seen: 0,
            tempos_seen: 0,
            changed: false,
            clearing: false,
            pending: None,
            note: None,
            left_out: None,
            meter: [0.0; MAX_TRACKS],
            meter_shown: [0.0; MAX_TRACKS],
            slots: Slots::default(),
            names: std::array::from_fn(|_| String::with_capacity(NAME_ROOM)),
            place: [None; MAX_TRACKS],
            clip_room: Box::new(
                [[[NO_CLIP_REGION; MAX_CLIP_REGIONS]; 1 + MAX_CHANGES]; MAX_TRACKS],
            ),
            silent: [false; MAX_TRACKS],
            arm_next: None,
            files_dir: None,
            silent_seen: [0; MAX_TAKES],
            named: Vec::new(),
            waits: Vec::new(),
            next_cue: 0,
            copy_waits: Vec::new(),
            waits_looked: 0,
            cell_sound: None,
            frame_now: 0,
            opened: Vec::new(),
            time_sig: mpfree_transport::time::Meter::default(),
            loops: LoopPlayer::new(),
        }
    }

    /// Play takes from their files (Dev's "Audio tracks from their
    /// files"), through a cache as big as `seconds` of stereo (Audio track
    /// memory): call it at start, before [`TrackHost::record_to`]. The
    /// cache's size in bytes, or None if refused. The stream layer starts
    /// on the host's worker; if it can't, the log says so and takes play
    /// from memory.
    pub fn stream(&mut self, seconds: u32) -> Option<usize> {
        let cfg = StreamConfig::for_memory(seconds, self.rate);
        self.host.stream(cfg).then(|| cfg.cache_bytes())
    }

    /// Whether takes play from their files.
    pub fn streams(&self) -> bool {
        self.host.streams()
    }

    /// Takes on their files and what they missed, while takes play from
    /// their files. Does not allocate.
    pub fn stream_numbers(&self) -> Option<StreamNumbers> {
        if !self.host.streams() {
            return None;
        }
        let tr = &self.host.tracks;
        Some(StreamNumbers {
            on_file: tr.takes().filter(|k| tr.on_file(k.id)).count(),
            takes: tr.takes().count(),
            silent_s: tr.stats().missed as f64 / f64::from(self.rate),
            heads_held: self.host.heads_held,
            heads_wanted: self.host.heads_wanted,
        })
    }

    /// Record every take to a file of its own in `dir` (the library's work
    /// folder for the audio tracks, after the screen has recovered what a
    /// cut-off run left there), from the next armed track on. If a track
    /// records now, it starts once nothing does. Does not allocate.
    pub fn record_to(&mut self, dir: std::path::PathBuf) {
        self.files_dir = Some(dir);
        self.tend_files();
    }

    /// The SD card opened, its files under `lease.root()` (its mount
    /// point): a drive of its own, so takes there play from an I/O thread
    /// of their own and the recorder there waits only for that drive's
    /// reads; and the opening takes there read on, until it is revoked
    /// (`AudioHost::own_device`). Allocates the message: between periods.
    pub fn own_drive(&mut self, lease: std::sync::Arc<mpfree_stream::storage::Lease>) {
        self.host.own_device(lease);
    }

    fn tend_files(&mut self) {
        if let Some(dir) = self.files_dir.take() {
            let cfg = mpfree_audiotracks::files::WriterConfig::default();
            match self.host.record_to(dir, cfg) {
                // The drive is looked at for System status from now on.
                Ok(()) => self.host.watch_drive(drive_room),
                Err(dir) => self.files_dir = Some(dir),
            }
        }
    }

    /// The next report for System status about the takes' files, the
    /// drive and streaming (phase 3b step 5d), about the audio track by
    /// its lasting id when it is about one. Does not allocate.
    pub fn take_problem(&mut self) -> Option<mpfree_status::Report> {
        let p = self.host.take_problem()?;
        Some(self.slots.name_problem(p))
    }

    /// A project save finished into `samples` (the project's `samples/`):
    /// the takes it copied a file of are kept from there from now on, and
    /// their copies in the work folder go. Not for an autosave.
    pub fn saved_into(&self, samples: std::path::PathBuf) {
        self.host.saved_into(samples);
    }

    /// Whether a take that carried on on the stick was made whole there
    /// since last asked (its first part back from the SD card, B2c): the
    /// project's copy of it has silence for that part until the next save,
    /// so the work is unsaved.
    pub fn take_joined(&mut self) -> bool {
        std::mem::take(&mut self.joined)
    }

    /// The same SD card is in again, as `mark` says (its marker and volume
    /// serial): the takes that carried on on the stick when it went get
    /// their first parts back from it, looked for in `places` too
    /// (`AudioHost::join_card`). Allocates the message: between periods.
    pub fn join_card(
        &self,
        mark: mpfree_stream::storage::CardMark,
        places: Vec<std::path::PathBuf>,
    ) {
        self.host.join_card(mark, places);
    }

    /// The next line for the log about a take's file (it couldn't be made,
    /// it is at risk, a save took a take from memory, it couldn't be
    /// opened to play from), naming the track as the musician named it;
    /// and, about once a second while it happens, the takes on their
    /// files that played silence because the drive was late. Allocates
    /// only when there is one.
    pub fn take_file_line(&mut self) -> Option<String> {
        if let Some(e) = self.host.take_event() {
            self.joined |= matches!(e, mpfree_audiotracks::files::FileEvent::Joined { .. });
            return Some(e.line(&self.who(e.track())));
        }
        if self.host.streams() && self.periods.is_multiple_of(DROPOUTS_EVERY) {
            return self.dropout_line();
        }
        None
    }

    /// "audio tracks: the drive fell behind: 0.3 s of Vocal went silent
    /// at bar 34" for each take whose silent frames grew since the last.
    fn dropout_line(&mut self) -> Option<String> {
        let mut line: Option<String> = None;
        for id in 0..MAX_TAKES as u16 {
            let i = usize::from(id);
            let Some((now, tick)) = self.host.tracks.missed(id) else {
                // Gone: a take given this id later starts from none.
                self.silent_seen[i] = 0;
                continue;
            };
            // Less than before: a new take under the same id.
            let seen = Some(self.silent_seen[i]).filter(|&s| s <= now).unwrap_or(0);
            self.silent_seen[i] = now;
            let Some(k) = self.host.tracks.take(id).filter(|_| now > seen) else {
                continue;
            };
            let secs = (now - seen) as f64 / f64::from(self.rate);
            let bar = self.time_sig.position(tick.floor() as Tick).bar;
            let who = self.who(k.track);
            match &mut line {
                None => {
                    line = Some(format!(
                        "audio tracks: the drive fell behind: {secs:.1} s of {who} went silent at bar {bar}"
                    ));
                }
                Some(l) => l.push_str(&format!(", {secs:.1} s of {who} at bar {bar}")),
            }
        }
        line
    }

    /// A project opening, or New: which project track each engine track
    /// plays from now on (`project::audio_slots`). Set as the tracks are
    /// emptied ([`TrackHost::clear`]). Does not allocate.
    pub fn bind(&mut self, slots: Slots) {
        self.slots = slots;
        self.place = [None; MAX_TRACKS];
        self.changed = true;
    }

    /// Each period, before [`TrackHost::period`]: the audio tracks of the
    /// project's track list (`project`) found again, and each engine
    /// track silenced as its project track is (`silenced`, a bit a track:
    /// muted or left out by a solo, `TimeHost::silenced`), since mute and
    /// solo are the track's. A few times a second, and at once with `now`
    /// (after the tracks change), the table follows the track list: a new audio
    /// track gets a free engine track, with its settings as new; one gone
    /// (UNDO took it away) lets go of its engine track if nothing is on
    /// it, or keeps it, silent, for a REDO. Does not allocate.
    pub fn sync(&mut self, project: &mpfree_seq::Project, silenced: u128, now: bool) {
        let look = now || self.periods.is_multiple_of(CHECK_EVERY);
        if look && !self.clearing && self.pending.is_none() {
            self.place = [None; MAX_TRACKS];
            for (i, t) in project.tracks.iter().enumerate() {
                if !t.is_audio() {
                    continue;
                }
                let s = match self.slots.slot_of(t.id) {
                    Some(s) => s,
                    None => match self.free_slot() {
                        Some(s) => {
                            self.slots.0[s] = t.id;
                            self.fresh(s);
                            // Added from "+ Track": armed, ready to record.
                            let periods = self.periods;
                            if self.arm_next.is_some_and(|at| periods - at < ARM_WINDOW) {
                                self.arm_next = None;
                                let _ = self.host.command(Command::Arm(s, true));
                            }
                            s
                        }
                        // More audio tracks than engine tracks: + Track
                        // refuses first, so only a broken project.
                        None => continue,
                    },
                };
                self.place[s] = Some(i);
                self.keep_name(s, &t.name);
            }
            for s in 0..MAX_TRACKS {
                if self.place[s].is_some() || !self.slots.0[s].is_given() {
                    continue;
                }
                // Its track gone (Undo of the add): an arm goes, so REC
                // never records a track that isn't there; once empty the
                // engine track is free.
                let tr = &self.host.tracks;
                if tr.armed(s) && tr.recording(s).is_none() {
                    let _ = self.host.command(Command::Arm(s, false));
                }
                if self.empty(s) {
                    self.slots.0[s] = TrackId::NONE;
                    self.changed = true;
                }
            }
        }
        for s in 0..MAX_TRACKS {
            let quiet = self.slots.0[s].is_given()
                && self.place[s].is_none_or(|i| i < 128 && silenced & (1 << i) != 0);
            if quiet != self.silent[s] && self.host.command(Command::Mute(s, quiet)) {
                self.silent[s] = quiet;
                self.changed = true;
            }
        }
        if look && !self.clearing && self.pending.is_none() {
            self.keep_takes();
        }
    }

    /// The recordings the project's audio clips name, with the clips its
    /// undo steps could bring back and the clip COPY holds (the audio
    /// tracks rethink, phase 2: [`crate::time::TimeHost::recordings`]),
    /// sorted. Their takes are kept from the next look on, and a take no
    /// longer named is let go: forgotten if nothing else uses it.
    pub fn keep_named(&mut self, names: Vec<RecId>) {
        debug_assert!(names.windows(2).all(|w| w[0] < w[1]));
        self.named = names;
    }

    /// Where the project's audio clips' regions start in their
    /// recordings, for the host to hold in the cache while takes stream
    /// ([`AudioHost::clip_starts`]): so a clip launched with the transport
    /// stopped starts from memory at PLAY (no cue waits for it).
    pub fn clip_starts(&mut self, project: &mpfree_seq::Project) {
        let mut starts: Vec<(RecordingId, u64)> = project
            .tracks
            .iter()
            .filter(|t| t.is_audio())
            .flat_map(|t| t.slots.iter().map(|(_, c)| c))
            .filter_map(|c| c.audio.as_ref())
            .flat_map(|a| a.regions.iter())
            .map(|r| (RecordingId(r.rec.0), r.src.max(0) as u64))
            .collect();
        starts.sort_unstable();
        starts.dedup();
        self.host.clip_starts(starts);
    }

    /// The project's audio clips with a region on Stretch, on the pads and
    /// the song's own, for their tempo copies to be made ahead (the audio
    /// tracks rethink, phase 5b: [`AudioHost::clip_copies`]). Each is
    /// Prepared: it may be launched at any time.
    pub fn clip_copies(&mut self, project: &mpfree_seq::Project) {
        let pads = project
            .tracks
            .iter()
            .filter(|t| t.is_audio())
            .flat_map(|t| t.slots.iter().map(|(_, c)| c));
        let wants: Vec<ClipWant> = pads
            .chain(project.arrangement.clips.iter())
            .filter(|c| {
                c.audio.as_ref().is_some_and(|a| {
                    a.regions
                        .iter()
                        .any(|r| r.follow == mpfree_seq::FollowTempo::Stretch)
                })
            })
            .filter_map(|c| clip_want(c, Priority::Prepared, 0.0))
            .collect();
        self.host.clip_copies(wants);
    }

    /// The tempo copies the cues waiting for them need (5b.4), as the time
    /// engine lists them (`TimeHost::copy_needs`, asked again when its
    /// `needs_rev` rises): each launched clip goes to the host at Urgent at
    /// the tempo at its line, and the keys each cue needs are kept to tell
    /// the engine when they are ready ([`TrackHost::copy_answers`]). A
    /// region whose copy has no key (its take not here, or past the speeds
    /// a copy is made at) needs nothing.
    pub fn hold_copies(&mut self, needs: &[CopyNeed], project: &mpfree_seq::Project) {
        let mut wants: Vec<ClipWant> = Vec::new();
        let mut waits: Vec<CopyWaiting> = Vec::new();
        let mut seen: Vec<(CueKey, usize, usize)> = Vec::new();
        for n in needs {
            let w = match waits.iter().position(|w| w.key == n.cue) {
                Some(i) => i,
                None => {
                    waits.push(CopyWaiting {
                        key: n.cue,
                        keys: Vec::new(),
                        told: None,
                    });
                    waits.len() - 1
                }
            };
            if seen.contains(&(n.cue, n.track, n.slot)) {
                continue;
            }
            seen.push((n.cue, n.track, n.slot));
            let Some(want) = project
                .clip(n.track, n.slot)
                .and_then(|c| clip_want(c, Priority::Urgent, n.bpm))
            else {
                continue;
            };
            let regions: Vec<usize> = needs
                .iter()
                .filter(|m| (m.cue, m.track, m.slot) == (n.cue, n.track, n.slot))
                .map(|m| m.region)
                .collect();
            let play = ClipPlay {
                regions: &want.regions[..want.n],
                origin: 0.0,
                len: want.len,
                last_frame: want.last_frame,
                gain: 1.0,
                clip: 0,
                stamp: 0,
            };
            let keys = &mut waits[w].keys;
            self.host.tracks.clip_copy_keys(&play, n.bpm, &mut |i, k| {
                if regions.contains(&i) && !keys.contains(&k) {
                    keys.push(k);
                }
            });
            wants.push(want);
        }
        // What the engine was told stands for a cue needing the same.
        for w in &mut waits {
            w.told = self
                .copy_waits
                .iter()
                .find(|o| o.key == w.key && o.keys == w.keys)
                .and_then(|o| o.told);
        }
        self.copy_waits = waits;
        self.host.launch_copies(wants);
    }

    /// Whether the copies each cue waiting for them needs are ready, for
    /// the time engine (`TimeHost::copies_ready`), told when it changes:
    /// call it every look.
    pub fn copy_answers(&mut self, mut f: impl FnMut(CueKey, bool)) {
        let tracks = &self.host.tracks;
        for w in &mut self.copy_waits {
            let ready = w.keys.iter().all(|k| tracks.has_copy(k));
            if w.told != Some(ready) {
                w.told = Some(ready);
                f(w.key, ready);
            }
        }
    }

    /// The cues waiting for their audio, as the time engine lists them
    /// (`TimeHost::audio_needs`, asked again when its `needs_rev` rises):
    /// each new key's audio goes to the host to hold, a key whose needs
    /// changed is asked again, and a key gone (its cues landed or were
    /// cancelled) lets its hold go. The answers come through
    /// [`TrackHost::wait_answers`].
    pub fn hold_waits(&mut self, needs: &[AudioNeed]) {
        let mut keys: Vec<CueKey> = Vec::new();
        for n in needs {
            if !keys.contains(&n.cue) {
                keys.push(n.cue);
            }
        }
        let host = &mut self.host;
        self.waits.retain(|w| {
            let stays = keys.contains(&w.key);
            if !stays {
                host.let_cue_go(w.cue);
            }
            stays
        });
        for key in keys {
            let wanted: Vec<(RecordingId, i64, i64)> = needs
                .iter()
                .filter(|n| n.cue == key)
                .map(|n| (RecordingId(n.rec.0), n.from, n.to))
                .collect();
            match self.waits.iter_mut().find(|w| w.key == key) {
                Some(w) if w.needs == wanted => {}
                Some(w) => {
                    w.needs = wanted;
                    w.held = None;
                    self.host.hold_cue(w.cue, &w.needs);
                }
                None => {
                    let cue = self.next_cue;
                    self.next_cue = self.next_cue.wrapping_add(1);
                    self.host.hold_cue(cue, &wanted);
                    self.waits.push(Waiting {
                        key,
                        cue,
                        needs: wanted,
                        held: None,
                    });
                }
            }
        }
    }

    /// A new time engine (a project opened): no cue of the last one waits
    /// any more, and what was held for them goes; it is told the
    /// recordings' tempos afresh.
    pub fn forget_waits(&mut self) {
        self.tempos_seen = 0;
        for w in self.waits.drain(..) {
            self.host.let_cue_go(w.cue);
        }
        if !self.copy_waits.is_empty() {
            self.copy_waits.clear();
            self.host.launch_copies(Vec::new());
        }
    }

    /// What the host says about the waiting cues' audio, for the time
    /// engine (`TimeHost::audio_held`, `TimeHost::audio_given_back`):
    /// call it every look. A cue whose audio couldn't be held is asked
    /// again every [`WAIT_RETRY`] looks, until its cues land or are
    /// cancelled.
    pub fn wait_answers(&mut self, mut f: impl FnMut(CueKey, CueAudio)) {
        while let Some((cue, a)) = self.host.cue_audio() {
            if let Some(w) = self.waits.iter_mut().find(|w| w.cue == cue) {
                w.held = Some(a == CueAudio::Held);
                f(w.key, a);
            }
        }
        self.waits_looked += 1;
        if self.waits_looked >= WAIT_RETRY {
            self.waits_looked = 0;
            for w in self.waits.iter_mut().filter(|w| w.held == Some(false)) {
                w.held = None;
                self.host.hold_cue(w.cue, &w.needs);
            }
        }
    }

    /// Each take kept while [`TrackHost::keep_named`]'s recordings name
    /// it, and let go once they don't ([`Command::Keep`]): told at each
    /// look, so takes that come in later (an open) are kept too. Nothing
    /// allocates.
    fn keep_takes(&mut self) {
        for id in 0..MAX_TAKES as u16 {
            let Some(k) = self.host.tracks.take(id) else {
                continue;
            };
            let named = self.named.binary_search(&RecId(k.rec.0)).is_ok();
            if named != self.host.tracks.kept(id) {
                let _ = self.host.command(Command::Keep {
                    take: id,
                    keep: named,
                });
            }
        }
    }

    /// Each engine track's sound of the last period into its project
    /// track's own mixer channel (the audio tracks rethink, phase 1b), whose
    /// strip gives it its level, pan, effects and sends; and which tracks
    /// those are ([`MixHost::audio_tracks`]), so their strips show. Added
    /// into the channels' inputs for the next block; nothing allocates.
    /// The panel's level and pan follow the strips' (the screen sees them
    /// change at the next look, as for any setting).
    pub fn play_into(&mut self, mix: &mut MixHost) {
        let mut bits = 0u128;
        for s in 0..MAX_TRACKS {
            let Some(p) = self.place[s].filter(|&p| p < 128) else {
                mix.point_audio_out(s, None);
                continue;
            };
            bits |= 1 << p;
            let ch = mix.track_channel(p);
            // The looper's pads leave it out (`MixHost::pads_wet`).
            mix.point_audio_out(s, Some(ch));
            let strip = mpfree_fx::StripId::Track(ch);
            self.volume[s] = from_db(mix.value(strip, mx::VOLUME), LEVEL_MAX_DB);
            self.pan[s] = mix.value(strip, mx::PAN) / 100.0;
            let out = self.host.tracks.track_out(s, PERIOD);
            if let Some((l, r)) = mix.mixer.track_input(ch) {
                for (f, fr) in out.chunks_exact(2).enumerate().take(PERIOD) {
                    l[f] += fr[0];
                    r[f] += fr[1];
                }
            }
        }
        mix.audio_tracks = bits;
    }

    /// The mixer screen's strips of the audio tracks (layout spec 3a):
    /// named after the track, in its colour, with a wave mark and its
    /// input; M and S are the track's mute and solo (`project`'s), and it
    /// is dimmed while the project silences it (`silenced`). An audio
    /// track's strip has no freeze: there is no loop to take.
    pub fn dress_strips(
        &self,
        st: &mut mpfree_mixer_ui::MixState,
        project: &mpfree_seq::Project,
        silenced: u128,
        mix: &MixHost,
    ) {
        for s in 0..MAX_TRACKS {
            let Some(p) = self.place[s] else {
                continue;
            };
            let Some(t) = project.tracks.get(p) else {
                continue;
            };
            let id = mpfree_fx::StripId::Track(mix.track_channel(p));
            for v in st.strips.iter_mut().filter(|v| v.id == id) {
                v.name = t.name.clone();
                v.color = Some(track_color(p));
                v.audio = Some(match self.inside[s] {
                    Some(_) => "Mix".to_string(),
                    None => input_chip(self.inputs[s]).to_string(),
                });
                v.mute = t.mute;
                v.solo = t.solo;
                v.audible = p >= 128 || silenced >> p & 1 == 0;
                v.freezable = false;
            }
        }
    }

    /// Loops on audio tracks (phase 4, board 6; layout spec 3a, No Loop
    /// strip): each audio strip's input
    /// in words ("Line L", "Mix › Bass", after the wave mark) and a tag
    /// for what its track plays now, "loop · row 2" and the loop's name or
    /// "take · row 3" and the clip's (`plays`: the clip each track plays,
    /// by its place in the track list, and whether it is a loop cell). The
    /// Loop strip goes: each loop plays through its own track's channel.
    pub fn dress_loop_strips(
        &self,
        st: &mut mpfree_mixer_ui::MixState,
        project: &mpfree_seq::Project,
        plays: impl Fn(usize) -> Option<(bool, mpfree_seq::ClipId)>,
        mix: &MixHost,
    ) {
        for s in 0..MAX_TRACKS {
            let Some(p) = self.place[s] else {
                continue;
            };
            let Some(t) = project.tracks.get(p) else {
                continue;
            };
            let id = mpfree_fx::StripId::Track(mix.track_channel(p));
            let input = match self.inside[s] {
                Some(_) if self.inside_name[s].starts_with(PADS_NAME) => {
                    self.inside_name[s].clone()
                }
                Some(_) => format!("Mix \u{203a} {}", self.inside_name[s]),
                None => format!("Line {}", input_chip(self.inputs[s])),
            };
            let tag = plays(p).and_then(|(looped, clip)| {
                let (slot, c) = t.slots.iter().find(|(_, c)| c.id == clip)?;
                let row = match project.scenes.get(slot).filter(|sc| !sc.name.is_empty()) {
                    Some(sc) => sc.name.clone(),
                    None => format!("row {}", slot + 1),
                };
                let kind = if looped { "loop" } else { "take" };
                Some((format!("{kind} \u{b7} {row}"), c.name.clone()))
            });
            for v in st.strips.iter_mut().filter(|v| v.id == id) {
                v.audio = Some(input.clone());
                v.tag.clone_from(&tag);
            }
        }
        // The Loop strip is the looper's own: loops play on their tracks.
        let looper = mpfree_fx::StripId::Track(crate::mixing::LOOP);
        st.strips.retain(|v| v.id != looper);
    }

    /// The audio tracks that sound, for Stems by Track (export.rs): each
    /// with something on it that the project doesn't silence (its mute, a
    /// solo elsewhere: `silenced`), by its place in `project`, with its
    /// own channel's strip and its name.
    pub fn heard(
        &self,
        project: &mpfree_seq::Project,
        silenced: u128,
        mix: &MixHost,
    ) -> Vec<(usize, mpfree_fx::StripId, String)> {
        let mut out: Vec<(usize, mpfree_fx::StripId, String)> = (0..MAX_TRACKS)
            .filter_map(|s| {
                let p = self.place[s].filter(|&p| p < 128 && silenced >> p & 1 == 0)?;
                if self.host.tracks.regions(s).is_empty() {
                    return None;
                }
                let t = project.tracks.get(p)?;
                let name = if t.name.trim().is_empty() {
                    format!("Track {}", p + 1)
                } else {
                    t.name.clone()
                };
                Some((p, mpfree_fx::StripId::Track(mix.track_channel(p)), name))
            })
            .collect();
        out.sort_by_key(|(p, _, _)| *p);
        out
    }

    /// "+ Track"'s Audio track was picked: the audio track it adds is
    /// armed once it has an engine track.
    pub fn arm_next(&mut self) {
        self.arm_next = Some(self.periods);
    }

    /// An engine track no project track plays, ready for a new one: none
    /// while a project opens.
    pub fn free_slot(&self) -> Option<usize> {
        if self.clearing || self.pending.is_some() {
            return None;
        }
        (0..MAX_TRACKS).find(|&s| !self.slots.0[s].is_given() && self.empty(s))
    }

    /// Engine track `s` holds nothing: no region, take or recording, and
    /// isn't armed.
    fn empty(&self, s: usize) -> bool {
        let tr = &self.host.tracks;
        tr.regions(s).is_empty()
            && tr.recording(s).is_none()
            && !tr.armed(s)
            && !tr.takes().any(|k| k.track == s)
    }

    /// Engine track `s`, just given to a new audio track: its settings as
    /// a new track's.
    fn fresh(&mut self, s: usize) {
        for c in [
            Command::SetInput(s, Input::Left),
            Command::SetMonitor(s, Monitor::Auto),
            Command::SetLevel(s, 1.0),
            Command::SetPan(s, 0.0),
            Command::Solo(s, false),
            Command::SetInside(s, None),
        ] {
            let _ = self.host.command(c);
        }
        self.inputs[s] = Input::Left;
        self.monitor[s] = Monitor::Auto;
        self.volume[s] = 1.0;
        self.pan[s] = 0.0;
        self.inside[s] = None;
        self.inside_last[s] = None;
        self.inside_lat[s] = None;
        self.changed = true;
    }

    /// Where engine track `s`'s project track is in the track list, as
    /// last found.
    pub fn place(&self, s: usize) -> Option<usize> {
        self.place.get(s).copied().flatten()
    }

    /// One period: the line input (`line`) in, the tracks' sound out into
    /// `out` (overwritten). `song_on` is song mode: the tracks record and
    /// play their song regions only then. `plays` says what a project
    /// track (by its place in the track list) played in the period, as the
    /// arranger's player has it ([`mpfree_play::Player::audio`]): its
    /// clips, its song or nothing; None plays every track's song, as
    /// before clips. (The app runs [`TrackHost::period_with_loops`]; the
    /// tests run the tracks alone with this.)
    #[cfg(test)]
    pub fn period<'a>(
        &mut self,
        block: &Block,
        map: &TempoMap,
        song_on: bool,
        line: (&[f32; PERIOD], &[f32; PERIOD]),
        plays: Option<&dyn Fn(usize) -> AudioAnswer<'a>>,
    ) {
        self.period_inner(block, map, song_on, line, plays, None);
    }

    /// Loops on audio tracks (phase 4b), before the
    /// looper runs: where each track's loops sound this period, from what
    /// the arranger's player says each plays (`plays`, as for
    /// [`TrackHost::period`]), for the loop host to tell the looper
    /// ([`TrackHost::loop_player`]). Does not allocate.
    pub fn plan_loops<'a>(&mut self, block: &Block, plays: &dyn Fn(usize) -> AudioAnswer<'a>) {
        let none = Change {
            frame: 0,
            tick: 0.0,
            plays: Plays::Nothing,
            muted: false,
            ring: false,
        };
        let mut changes = [[none; MAX_CHANGES]; MAX_TRACKS];
        let mut answers = [Answer::default(); MAX_TRACKS];
        for (s, (ch, ans)) in changes.iter_mut().zip(answers.iter_mut()).enumerate() {
            let Some(p) = self.place[s] else {
                continue;
            };
            let a = plays(p);
            let mut k = 0;
            for (c, slot) in a.changes().iter().zip(ch.iter_mut()) {
                *slot = Change {
                    frame: c.frame,
                    tick: c.tick,
                    plays: loop_plays(&c.plays),
                    muted: c.muted,
                    ring: c.ring,
                };
                k += 1;
            }
            let ch: &[Change] = ch;
            *ans = Answer {
                start: loop_plays(&a.start),
                muted: a.muted,
                changes: &ch[..k],
                superseded: a.superseded,
            };
        }
        self.loops.plan(block, &answers);
    }

    /// Where each track's loops sound this period
    /// ([`TrackHost::plan_loops`]).
    pub fn loop_player(&self) -> &LoopPlayer {
        &self.loops
    }

    /// Before [`TrackHost::plan_loops`]: the loops heard off any track
    /// now (the Loop strip's, by id: `LoopHost::moving_ids`). One that
    /// opens on a track carries on there unfaded ([`LoopPlayer::moving`]).
    pub fn loops_moving(&mut self, ids: &[u128]) {
        self.loops.moving(ids);
    }

    /// After [`TrackHost::plan_loops`]: Listen is heard through the
    /// project track in place `place` this period (`LoopHost::listen_track`;
    /// None: through none), its frames given as `LISTEN`'s by the source
    /// [`TrackHost::period_with_loops`] reads (phase 4e.2).
    pub fn loop_listen(&mut self, place: Option<usize>) {
        let s = place.and_then(|p| (0..MAX_TRACKS).find(|&s| self.place(s) == Some(p)));
        self.loops.listen(s);
    }

    /// After [`TrackHost::plan_loops`]: the free cells' loops this period
    /// (phase 4e.4), each (project track, lasting id; 0 for none), heard
    /// through their tracks at full gain on every frame: the looper
    /// launches and fades them on the free clock (`LoopPlayer::free`).
    pub fn loop_free(&mut self, cells: &[(usize, u128)]) {
        for &(p, id) in cells.iter().filter(|c| c.1 != 0) {
            if let Some(s) = (0..MAX_TRACKS).find(|&s| self.place(s) == Some(p)) {
                self.loops.free(s, id);
            }
        }
    }

    /// A new lasting id for a loop (phase 4b: `mpfree_seq::LoopId`), from
    /// the audio tracks' id maker: never 0, never given twice.
    pub fn fresh_loop_id(&mut self) -> u128 {
        self.host.tracks.fresh_rec().0
    }

    /// A new recording's id for a loop's layer (phase 4c.2), from the same
    /// id maker as takes': never 0, never given twice.
    pub fn fresh_recording_id(&mut self) -> u128 {
        self.host.tracks.fresh_rec().0
    }

    /// [`TrackHost::period`] with the tracks' loops (phase 4b): planned
    /// before the looper ran
    /// ([`TrackHost::plan_loops`]), and each loop's sound in the period by
    /// its id (`src`: the loop host's [`crate::looping::LoopHost::track_loop`]),
    /// into its track before the track's level, pan, mute and solo.
    pub fn period_with_loops<'a, 's>(
        &mut self,
        block: &Block,
        map: &TempoMap,
        song_on: bool,
        line: (&[f32; PERIOD], &[f32; PERIOD]),
        plays: &dyn Fn(usize) -> AudioAnswer<'a>,
        src: &dyn Fn(u128) -> Option<&'s [f32]>,
    ) {
        self.period_inner(block, map, song_on, line, Some(plays), Some(src));
    }

    fn period_inner<'a, 's>(
        &mut self,
        block: &Block,
        map: &TempoMap,
        song_on: bool,
        line: (&[f32; PERIOD], &[f32; PERIOD]),
        plays: Option<&dyn Fn(usize) -> AudioAnswer<'a>>,
        src: Option<&dyn Fn(u128) -> Option<&'s [f32]>>,
    ) {
        if song_on != self.active && self.host.command(Command::SetActive(song_on)) {
            self.active = song_on;
        }
        self.frame_now = block.start_frame + u64::from(block.frames);
        for f in 0..PERIOD {
            self.input[2 * f] = line.0[f];
            self.input[2 * f + 1] = line.1[f];
        }
        self.out.fill(0.0);
        // Each track's answer in the engine's words: its project track's,
        // or its song for one with none.
        let none = Change {
            frame: 0,
            tick: 0.0,
            plays: Plays::Nothing,
            muted: false,
            ring: false,
        };
        let mut changes = [[none; MAX_CHANGES]; MAX_TRACKS];
        let mut answers = [Answer::default(); MAX_TRACKS];
        let mut n = 0;
        if let Some(plays) = plays {
            n = MAX_TRACKS;
            let rows = self.clip_room.iter_mut().zip(changes.iter_mut());
            for (s, ((rooms, ch), ans)) in rows.zip(answers.iter_mut()).enumerate() {
                let Some(p) = self.place[s] else {
                    continue;
                };
                let a = plays(p);
                let (first, rest) = rooms.split_at_mut(1);
                let start = plays_in(&a.start, &mut first[0]);
                let mut k = 0;
                for ((c, room), slot) in a.changes().iter().zip(rest.iter_mut()).zip(ch.iter_mut())
                {
                    *slot = Change {
                        frame: c.frame,
                        tick: c.tick,
                        plays: plays_in(&c.plays, room),
                        muted: c.muted,
                        ring: c.ring,
                    };
                    k += 1;
                }
                let ch: &[Change] = ch;
                *ans = Answer {
                    start,
                    muted: a.muted,
                    changes: &ch[..k],
                    superseded: a.superseded,
                };
            }
        }
        // The engine's sum is no one's since phase 1b: each track plays
        // through its own channel (`play_into`).
        match src {
            Some(src) => self.host.period_with_loops(
                block,
                map,
                &self.input[..],
                &mut self.out[..],
                &answers[..n],
                &self.loops,
                src,
            ),
            None => self.host.period_with(
                block,
                map,
                &self.input[..],
                &mut self.out[..],
                &answers[..n],
            ),
        }
        if let Some(sp) = block.spans().last() {
            self.time_sig = sp.meter;
        }
        self.measure();
        self.periods += 1;
        self.tend_open();
        if self.files_dir.is_some() {
            self.tend_files();
        }
        self.take_wave();
        if self.periods.is_multiple_of(CHECK_EVERY) {
            self.tend_waves();
            let now = self.signature();
            if now != self.seen {
                self.seen = now;
                self.changed = true;
            }
        }
    }

    /// Track `t` records from inside the mix (`from`, with the name the
    /// screen gives its strip), or the line input again (None). Refused
    /// while the track records. The engine has the latency from the next
    /// [`TrackHost::feed_inside`], which runs before the tracks, so a
    /// recording that starts in the same period already takes from inside.
    pub fn set_inside(&mut self, t: usize, from: Option<(Inside, &str)>) -> Result<String, String> {
        let n = self.track_no(t)?;
        if self.host.tracks.recording(t).is_some() {
            return Err(format!(
                "{n} is recording: change what it records after it stops"
            ));
        }
        let line = match from {
            None => {
                self.send(Command::SetInside(t, None))?;
                self.inside[t] = None;
                format!("{n} records {}", input_words(self.inputs[t]))
            }
            Some((f, name)) => {
                self.inside[t] = Some(f);
                self.inside_last[t] = Some(f);
                self.inside_name[t] = name.to_string();
                match (f.after_fader, f.after_effects) {
                    (true, _) => format!("{n} records {name}, after its fader"),
                    (false, true) => format!("{n} records {name}, after its effects"),
                    (false, false) => format!("{n} records {name}, before its effects"),
                }
            }
        };
        self.inside_lat[t] = None;
        self.inside_lost[t] = false;
        self.changed = true;
        Ok(line)
    }

    /// The save [`AudioHost::save`] started, once it is read, with where
    /// each track records from inside the mix (step 9) written in: a
    /// track with only that set is saved too. Each track is named by its
    /// project track's id (phase 1a); an engine track whose project track
    /// isn't in `project`'s track list (UNDO took it away) is left out,
    /// with its takes, as the track is. Mute and solo are the project
    /// track's, saved with it, so they are never written here.
    pub fn take_saved(&mut self, project: &mpfree_seq::Project) -> Option<Saved> {
        let mut saved = self.host.take_saved()?;
        let ids = self.slots;
        let alive = |s: usize| {
            ids.id_of(s).is_some_and(|id| {
                project
                    .track_with(id)
                    .is_some_and(|i| project.tracks[i].is_audio())
            })
        };
        for t in 0..MAX_TRACKS {
            let Some(f) = self.inside[t] else { continue };
            let from = inside_text(f);
            match saved.tracks.iter_mut().find(|x| x.index == t) {
                Some(x) => x.from = from,
                None => {
                    let Some(s) = self.host.tracks.settings(t) else {
                        continue;
                    };
                    saved.tracks.push(TrackFile {
                        index: t,
                        id: 0,
                        input: s.input,
                        monitor: s.monitor,
                        from,
                        regions: Vec::new(),
                    });
                }
            }
        }
        saved.tracks.retain(|x| alive(x.index));
        saved.takes.retain(|k| alive(k.track));
        for x in &mut saved.tracks {
            x.id = ids.id_of(x.index).map_or(0, |id| id.0);
        }
        saved.tracks.sort_by_key(|x| x.index);
        Some(saved)
    }

    /// What the panel's Mix offers (step 9), by name, from the mixer
    /// (`MixHost::inside_sources`), before an action that needs it: the
    /// tracks recording from inside take their names from it again, so a
    /// new sound on a strip shows. Allocates: for the screen's taps.
    pub fn offer(&mut self, sources: Vec<(String, mpfree_fx::StripId)>) {
        for t in 0..MAX_TRACKS {
            let Some(f) = self.inside[t] else { continue };
            if self.inside_name[t].starts_with(PADS_NAME) {
                continue;
            }
            if let Some((name, _)) = sources.iter().find(|s| s.1 == f.strip) {
                if *name != self.inside_name[t] {
                    self.inside_name[t] = name.clone();
                    self.changed = true;
                }
            }
        }
        self.sources = sources;
    }

    /// The panel's Mix (`step` 0) and From steps: record from the mix,
    /// the source picked last (or the first offered), or the one `step`
    /// away from the source now, round the list.
    fn mix_step(&mut self, t: usize, step: i32) -> Result<String, String> {
        self.track_no(t)?;
        let n = self.sources.len();
        if n == 0 {
            return Err("the mix offers nothing to record".into());
        }
        let at = |s: mpfree_fx::StripId| self.sources.iter().position(|x| x.1 == s);
        let (k, after) = match (self.inside[t], step) {
            (Some(f), _) => {
                let k = at(f.strip).unwrap_or(0) as i64 + i64::from(step);
                (k.rem_euclid(n as i64) as usize, f.after_effects)
            }
            (None, _) => match self.inside_last[t] {
                Some(f) if at(f.strip).is_some() => (at(f.strip).unwrap_or(0), f.after_effects),
                _ => (0, true),
            },
        };
        if step == 0 && self.inside[t].is_some() {
            return Ok(String::new());
        }
        let (name, strip) = self.sources[k].clone();
        let f = Inside {
            strip,
            after_effects: after,
            after_fader: false,
        };
        self.set_inside(t, Some((f, &name)))
    }

    /// A new audio track made for a loop (4e.4's last resort) records what
    /// that loop recorded (phase 4e.5), so the loop's overdubs, which
    /// record its track's input, carry on from the same sound: the pads'
    /// instrument after its fader (`pads`, as the input list names it),
    /// the line input (both sides), a mix after its fader, or another
    /// track's input as it was. Nothing when `p` has no engine track here
    /// or the pads' instrument isn't known.
    pub fn records_as(
        &mut self,
        p: usize,
        source: crate::looping::LoopSource,
        pads: Option<InputSource>,
    ) -> Result<String, String> {
        use crate::looping::{LoopSource, TrackFrom};
        use mpfree_fx::CapturePoint;
        let s = (0..MAX_TRACKS)
            .find(|&s| self.place[s] == Some(p))
            .ok_or("that track has no engine track")?;
        let inside = |strip, point| Inside {
            strip,
            after_effects: point != CapturePoint::BeforeInserts,
            after_fader: point == CapturePoint::AfterFader,
        };
        match source {
            LoopSource::Pads => {
                let pads = pads.ok_or("the pads' instrument isn't known yet")?;
                let name = format!("{PADS_NAME}{}", pads.name);
                let at = inside(pads.strip, CapturePoint::AfterFader);
                self.set_inside(s, Some((at, &name)))
            }
            LoopSource::Line => self.set_line(s, Input::Stereo),
            LoopSource::Mix(strip) => {
                let name = crate::mixing::name(strip);
                self.set_inside(s, Some((inside(strip, CapturePoint::AfterFader), &name)))
            }
            LoopSource::Track(t) => match t.from {
                TrackFrom::Line(i) => self.set_line(s, i),
                TrackFrom::Mix(strip, point) => {
                    let name = crate::mixing::name(strip);
                    self.set_inside(s, Some((inside(strip, point), &name)))
                }
            },
        }
    }

    /// What a loop recorded into project track `p`'s cell records (audio
    /// tracks phase 4b): the track's own input, as a
    /// take on it would (inside the mix at its point, or its line pick),
    /// and its own channel, which is never in it. None when `p` is not an
    /// audio track here.
    pub fn loop_input(&self, p: usize, mix: &MixHost) -> Option<crate::looping::TrackInput> {
        use crate::looping::{TrackFrom, TrackInput};
        let s = (0..MAX_TRACKS).find(|&s| self.place[s] == Some(p))?;
        let from = match self.inside[s] {
            Some(f) => TrackFrom::Mix(f.strip, f.point()),
            None => TrackFrom::Line(self.inputs[s]),
        };
        Some(TrackInput {
            channel: mpfree_fx::StripId::Track(mix.track_channel(p)),
            from,
        })
    }

    /// What track `t`'s input list offers (the audio tracks rethink's
    /// phase 4, board 4): the line input's picks, the pads' instrument
    /// (`pads`, after its fader; None while they play MIDI out), each
    /// strip of `sources` after its fader (not its own `channel`, the
    /// pads' or the master) and the master before its effects, each dim
    /// with the mixer's reason where `refusal` gives one for a recording
    /// into `channel`. Kept, so a pick names one ([`AudioAction::InputPick`]).
    pub fn input_choices(
        &mut self,
        t: usize,
        sources: &[InputSource],
        pads: Option<InputSource>,
        channel: mpfree_fx::StripId,
        refusal: impl Fn(
            (mpfree_fx::StripId, mpfree_fx::CapturePoint),
        ) -> Option<mpfree_fx::CaptureRefusal>,
    ) -> Result<mpfree_time_ui::inputs::InputChoices, String> {
        use mpfree_fx::{CapturePoint, StripId};
        use mpfree_time_ui::inputs::{Choice, InputChoices, MASTER, PADS, STRIP};
        let n = self.track_no(t)?;
        let name_of = |s: StripId| {
            sources
                .iter()
                .find(|x| x.strip == s)
                .map_or_else(|| crate::mixing::name(s), |x| x.name.clone())
        };
        let why = |name: &str, r: mpfree_fx::CaptureRefusal| {
            use mpfree_fx::CaptureRefusal as No;
            match r {
                No::NotInMix => format!("{name} isn't in the mix"),
                No::Itself => format!("{name} is {n}'s own: it would record itself"),
                No::PastItsWayIn => format!("{n} plays into {name}, so it would record itself"),
                No::Through(b) => format!(
                    "{n} plays into {name} through {}, so it would record itself",
                    name_of(b)
                ),
                No::Sends(r) => format!(
                    "{n} sends to {}, which plays into {name}, so it would record its own echoes",
                    crate::mixing::name(StripId::Return(r))
                ),
            }
        };
        let mut offered = Offered {
            track: t,
            ..Offered::default()
        };
        let after_fader = |strip| Inside {
            strip,
            after_effects: true,
            after_fader: true,
        };
        let picked_inside = self.inside[t];
        let mut picked = None;
        let mut choice = |k: usize, src: &InputSource, at: Inside, offered: &mut Offered| {
            let refused = refusal((at.strip, at.point())).map(|r| why(&src.name, r));
            if let Some(r) = &refused {
                offered.refused.push((k, r.clone()));
            }
            offered.places.push((k, src.name.clone(), at));
            if picked_inside == Some(at) {
                picked = Some(k);
            }
            Choice {
                name: src.name.clone(),
                owner: src.owner,
                color: None,
                refused,
            }
        };
        let pads_choice = pads.as_ref().map(|p| {
            let named = InputSource {
                name: format!("{PADS_NAME}{}", p.name),
                ..p.clone()
            };
            choice(PADS, &named, after_fader(p.strip), &mut offered)
        });
        let pads_why = match &pads_choice {
            None => "The pads play MIDI out: nothing here to record".to_string(),
            Some(c) => c.refused.clone().unwrap_or_default(),
        };
        let mut strips = Vec::new();
        // The first dim strip's reason, short, for the line under them.
        let mut first_dim = None;
        let listed = sources.iter().filter(|x| {
            x.strip != channel
                && x.strip != StripId::Master
                && pads.as_ref().is_none_or(|p| p.strip != x.strip)
        });
        for (i, src) in listed.enumerate() {
            let at = after_fader(src.strip);
            if first_dim.is_none() {
                first_dim = refusal((at.strip, at.point()));
            }
            strips.push(choice(STRIP + i, src, at, &mut offered));
        }
        let master_src = InputSource {
            strip: StripId::Master,
            name: "Master".to_string(),
            owner: None,
        };
        let master_at = Inside {
            strip: StripId::Master,
            after_effects: false,
            after_fader: false,
        };
        let master = choice(MASTER, &master_src, master_at, &mut offered);
        // The lines under the strips and beside Master.
        let dim: Vec<&str> = strips
            .iter()
            .filter(|c| c.refused.is_some())
            .map(|c| c.name.as_str())
            .collect();
        let strips_why = match dim.as_slice() {
            [] => String::new(),
            [one] => {
                use mpfree_fx::CaptureRefusal as No;
                let r = match first_dim {
                    Some(No::Through(b)) => format!("{n} plays into it through {}", name_of(b)),
                    Some(No::Sends(r)) => format!(
                        "{n} sends to {}, which plays into it",
                        crate::mixing::name(StripId::Return(r))
                    ),
                    Some(No::NotInMix) => "it isn't in the mix".to_string(),
                    _ => format!("{n} plays into it"),
                };
                format!("{one} is dim: {r}. Pick a strip {n} doesn't feed.")
            }
            [rest @ .., last] => format!(
                "{} and {last} are dim: {n} feeds them. Pick a strip {n} doesn't feed.",
                rest.join(", ")
            ),
        };
        let master_why = match refusal((StripId::Master, CapturePoint::BeforeInserts)) {
            None => String::new(),
            Some(mpfree_fx::CaptureRefusal::Through(b)) => format!(
                "{n} plays into Master through {}: pick another input, or send {n} straight to Master.",
                name_of(b)
            ),
            Some(mpfree_fx::CaptureRefusal::Sends(r)) => format!(
                "{n} sends to {}, which plays into Master: pick another input, or turn that send down.",
                crate::mixing::name(StripId::Return(r))
            ),
            Some(r) => format!("{}.", why("Master", r)),
        };
        self.offered = Some(offered);
        Ok(InputChoices {
            track: t,
            name: n,
            line: picked_inside.is_none().then_some(self.inputs[t]),
            picked,
            pads: pads_choice,
            pads_why,
            strips,
            strips_why,
            master,
            master_why,
        })
    }

    /// A pick on the input list offered last ([`TrackHost::input_choices`]):
    /// a line input pick, or a place inside the mix; refused, with the
    /// mixer's reason, where it refused it, or while the track records.
    fn input_pick(&mut self, t: usize, k: usize) -> Result<String, String> {
        use mpfree_time_ui::inputs::{LINES, PADS};
        let n = self.track_no(t)?;
        if k < PADS {
            let input = LINES.get(k).ok_or("no such input")?.1;
            return self.set_line(t, input);
        }
        let offered = self
            .offered
            .as_ref()
            .filter(|o| o.track == t)
            .ok_or_else(|| format!("{n}'s input list has closed: open it again"))?;
        if let Some((_, why)) = offered.refused.iter().find(|x| x.0 == k) {
            return Err(why.clone());
        }
        let (_, name, at) = offered
            .places
            .iter()
            .find(|x| x.0 == k)
            .cloned()
            .ok_or("that input has gone from the list")?;
        if self.inside[t] == Some(at) {
            return Ok(String::new());
        }
        self.set_inside(t, Some((at, &name)))
    }

    /// Track `t` records line input `input` (the panel's chips, the input
    /// list's Line): refused while it records.
    fn set_line(&mut self, t: usize, input: Input) -> Result<String, String> {
        let n = self.track_no(t)?;
        if self.host.tracks.recording(t).is_some() {
            return Err(format!("{n} is recording: change its input after it stops"));
        }
        if self.inside[t].is_some() {
            self.set_inside(t, None)?;
        }
        self.send(Command::SetInput(t, input))?;
        self.inputs[t] = input;
        self.changed = true;
        Ok(format!("{n} records {}", input_words(input)))
    }

    /// What track `t` records from inside the mix, and the name it was
    /// given; None while it records the line input.
    #[cfg(test)]
    pub fn inside(&self, t: usize) -> Option<(Inside, &str)> {
        let f = (*self.inside.get(t)?)?;
        Some((f, &self.inside_name[t]))
    }

    /// Before each [`TrackHost::period`]: each track recording from inside
    /// gets the mixer's block for it ([`crate::mixing::MixHost::audio_take`]),
    /// and the engine its latency when that changes (never while the track
    /// records: the engine refuses it then, and the take keeps the latency
    /// it started with). The tracks run before the mixer, so the block is
    /// the last period's: one period later than the capture's own latency.
    /// A track whose strip has gone records silence, said once and counted
    /// ([`TrackHost::inside_silent`]). Real-time safe but for that line.
    pub fn feed_inside(&mut self, mix: &mut crate::mixing::MixHost) {
        for t in 0..MAX_TRACKS {
            let from = self.inside[t];
            let Some(into) = self.host.tracks.inside_input(t) else {
                continue;
            };
            let n = (2 * PERIOD).min(into.len());
            let got = mix.audio_take(t, from, &mut into[..n]);
            if from.is_none() {
                continue;
            }
            let Some(latency) = got else {
                self.inside_silent += 1;
                if !self.inside_lost[t] {
                    self.inside_lost[t] = true;
                    self.note = Some(format!(
                        "{}: {} isn't in the mix, so it records silence",
                        self.who(t),
                        self.inside_name[t]
                    ));
                }
                continue;
            };
            self.inside_lost[t] = false;
            let latency = latency + PERIOD as u32;
            if self.inside_lat[t] != Some(latency)
                && self.host.tracks.recording(t).is_none()
                && self.host.command(Command::SetInside(t, Some(latency)))
            {
                self.inside_lat[t] = Some(latency);
            }
        }
    }

    /// The level bars: each track's peak this period (its sound after its
    /// level, pan, mute and solo, and while it is armed the input it
    /// records), falling back at [`METER_FALL`] of the bar a second.
    fn measure(&mut self) {
        let fall = METER_FALL * PERIOD as f32 / self.rate as f32;
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        for t in 0..MAX_TRACKS {
            let mut p = peak(self.host.tracks.track_out(t, PERIOD));
            if self.host.tracks.armed(t) && self.inside[t].is_some() {
                if let Some(inside) = self.host.tracks.inside_input(t) {
                    p = p.max(peak(&inside[..(2 * PERIOD).min(inside.len())]));
                }
            } else if self.host.tracks.armed(t) {
                let (l, r) = match self.inputs[t] {
                    Input::Left => (true, false),
                    Input::Right => (false, true),
                    Input::Stereo | Input::Sum => (true, true),
                };
                for f in 0..PERIOD {
                    if l {
                        p = p.max(self.input[2 * f].abs());
                    }
                    if r {
                        p = p.max(self.input[2 * f + 1].abs());
                    }
                }
            }
            self.meter[t] = meter(p).max(self.meter[t] - fall);
        }
    }

    /// The level bars, when one has moved by a step since they were last
    /// taken (a still song sends nothing). Cheap: call it a few times a
    /// second while the Song view shows.
    pub fn take_levels(&mut self) -> Option<[f32; MAX_TRACKS]> {
        let moved = self
            .meter
            .iter()
            .zip(&self.meter_shown)
            .any(|(a, b)| (a - b).abs() >= METER_STEP);
        moved.then(|| {
            self.meter_shown = self.meter;
            self.meter
        })
    }

    /// The line input's latency, in frames: the looper's "Latency to make
    /// up for" (layout spec 4c: one line input latency, shared with the
    /// looper). Sent to the tracks when it changes; call it every period.
    pub fn follow_latency(&mut self, frames: u32) {
        if self.latency != Some(frames) && self.host.command(Command::SetLatency(frames)) {
            self.latency = Some(frames);
        }
    }

    /// The region just moved, once the engine has it where it went: pick
    /// it again, as its index may have changed. Cheap: call it as often
    /// as the change check.
    pub fn take_repick(&mut self) -> Option<RegionId> {
        let (track, at) = self.repick?;
        let index = self
            .host
            .tracks
            .regions(track)
            .iter()
            .position(|r| r.start == at)?;
        self.repick = None;
        Some(RegionId { track, index })
    }

    /// A project opening, or New: every recording ends now, a save under
    /// way stops, and the tracks are emptied (every take forgotten) as
    /// soon as nothing records; then the project's part, if one comes
    /// ([`TrackHost::open`]), goes in. Does not allocate.
    pub fn clear(&mut self) {
        self.host.cancel_save();
        for t in 0..MAX_TRACKS {
            if self.host.tracks.armed(t) {
                self.host.command(Command::Arm(t, false));
            }
        }
        if let Some(old) = self.pending.take() {
            self.host.drop_saved(old);
        }
        self.clearing = true;
        self.cell_sound = None;
        self.tend_open();
    }

    /// An opened project's audio tracks, read from its files: they go in
    /// once the tracks are empty ([`TrackHost::clear`] comes first). What
    /// is left of `saved` is dropped on the host's worker.
    pub fn open(&mut self, saved: Saved) {
        if let Some(old) = self.pending.replace(saved) {
            self.host.drop_saved(old);
        }
        self.tend_open();
    }

    /// True while a project's audio tracks are going in: emptying, waiting
    /// for the part, takes copying in, or takes waiting for their files to
    /// open. A save waits for it.
    pub fn opening(&self) -> bool {
        self.clearing
            || self.pending.is_some()
            || self.host.tracks.loading() > 0
            || self.host.opening() > 0
    }

    /// A line for the log from the open, once.
    pub fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }

    /// What the open couldn't put back (takes or tracks with no room), a
    /// few words for the screen, once: the project isn't saved over.
    pub fn take_left_out(&mut self) -> Option<String> {
        self.left_out.take()
    }

    /// The open's next step, each period: empty the tracks once nothing
    /// records, then put the waiting part in.
    fn tend_open(&mut self) {
        if self.clearing {
            // The host's clear: a fit or read still coming for the old
            // takes is dropped, never let near the project's.
            if !self.host.clear() {
                return;
            }
            self.clearing = false;
            // The waveforms were the old takes'; the ids come back.
            self.host.cancel_wave();
            self.reading = None;
            while let Some(w) = self.waves.pop() {
                self.host.drop_wave(w);
            }
            (self.panel, self.takes_open, self.repick) = (None, None, None);
            self.resync();
            self.changed = true;
        }
        let Some(mut saved) = self.pending.take() else {
            return;
        };
        // Only the engine tracks a project track plays. Level and pan
        // are the mixer strip's and mute and solo the project track's, so
        // the engine's own stay as the clear left them: unity and off.
        let ids = self.slots;
        saved.tracks.retain(|t| ids.id_of(t.index).is_some());
        saved.takes.retain(|k| ids.id_of(k.track).is_some());
        // Each recording as its track's take lane numbers it, for a clip
        // whose take's file doesn't open.
        self.opened = saved
            .takes
            .iter()
            .map(|k| {
                let mut mine: Vec<_> = saved.takes.iter().filter(|o| o.track == k.track).collect();
                mine.sort_by_key(|o| (o.group, o.pass, o.id));
                let n = mine.iter().position(|o| o.id == k.id).map_or(0, |n| n + 1);
                (RecId(k.rec.0), format!("take {n}"))
            })
            .collect();
        self.silent = [false; MAX_TRACKS];
        let (tracks, takes) = (saved.tracks.len(), saved.takes.len());
        let inside: Vec<(usize, Inside)> = saved
            .tracks
            .iter()
            .filter_map(|x| Some((x.index, parse_inside(&x.from)?)))
            .filter(|(t, _)| *t < MAX_TRACKS)
            .collect();
        let r = self.host.load(saved);
        self.resync();
        // Where tracks record from inside the mix: named as the mixer
        // names the strip until the panel offers the mix again, and the
        // engine has the latency from the next period.
        for (t, f) in inside {
            self.inside[t] = Some(f);
            self.inside_last[t] = Some(f);
            self.inside_name[t] = crate::mixing::name(f.strip);
            self.inside_lat[t] = None;
        }
        self.changed = true;
        self.note = Some(match r {
            Ok(()) if tracks == 0 => return,
            Ok(()) => format!(
                "project: {tracks} audio track{} and {takes} take{} in",
                plural(tracks),
                plural(takes)
            ),
            Err(e) => {
                self.left_out = Some("part of its audio tracks".to_string());
                format!("project: {e} (a take left out may not fit in Prefs > Audio track memory)")
            }
        });
    }

    /// What this side keeps of each track's settings, from the tracks
    /// themselves (after a clear or a load).
    fn resync(&mut self) {
        for t in 0..MAX_TRACKS {
            if let Some(s) = self.host.tracks.settings(t) {
                self.inputs[t] = s.input;
                self.monitor[t] = s.monitor;
                self.volume[t] = s.level;
                self.pan[t] = s.pan;
            }
            if !self.host.tracks.records_inside(t) {
                self.inside[t] = None;
                self.inside_lat[t] = None;
            }
        }
    }

    /// True once after the lanes, the waveforms or the counters changed.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// Engine track `t` has a lane: a project track plays it (phase 1a:
    /// an audio track's lane shows whether it holds anything or not).
    fn shows(&self, t: usize) -> bool {
        self.place(t).is_some()
    }

    /// A waveform read: into the list.
    fn take_wave(&mut self) {
        if let Some(w) = self.host.take_wave() {
            self.reading = None;
            match self.waves.iter().position(|x| x.id == w.id) {
                Some(i) => self
                    .host
                    .drop_wave(std::mem::replace(&mut self.waves[i], w)),
                // Room for every take was made at the start, so this never
                // allocates.
                None if self.waves.len() < self.waves.capacity() => self.waves.push(w),
                None => self.host.drop_wave(w),
            }
            self.changed = true;
        }
    }

    /// Ask for a take's waveform: the next take with none, or a recording
    /// one grown since it was read. One at a time, checked every
    /// [`CHECK_EVERY`] periods.
    fn tend_waves(&mut self) {
        // Waves of takes no longer there (forgotten, or a cleared project).
        let mut i = 0;
        while i < self.waves.len() {
            if self.host.tracks.take(self.waves[i].id).is_some() {
                i += 1;
            } else {
                self.host.drop_wave(self.waves.swap_remove(i));
                self.changed = true;
            }
        }
        if self.reading.is_some() {
            return;
        }
        let mut want = None;
        for k in self.host.tracks.takes() {
            match self.waves.iter().find(|w| w.id == k.id) {
                None if k.frames > 0 => {
                    want = Some(k);
                    break;
                }
                Some(w)
                    if k.recording
                        && w.frames < k.frames
                        && self.periods - self.grown_at >= GROW_EVERY =>
                {
                    want = Some(k);
                }
                _ => {}
            }
        }
        if let Some(k) = want {
            if k.recording {
                self.grown_at = self.periods;
            }
            if self.host.wave(k.id, columns(&k)) {
                self.reading = Some(k.id);
            }
        }
    }

    /// A cheap fingerprint of what the lanes show, made without allocating.
    fn signature(&self) -> u64 {
        let tr = &self.host.tracks;
        let mut h = Fnv::new();
        for s in 0..MAX_TRACKS {
            h.add(self.place[s].map_or(u64::MAX, |i| i as u64));
        }
        h.add(self.panel.map_or(u64::MAX, |t| t as u64));
        h.add(self.takes_open.map_or(u64::MAX, |t| t as u64));
        for t in 0..MAX_TRACKS {
            h.add(u64::from(tr.armed(t)));
            h.add(u64::from(self.volume[t].to_bits()));
            h.add(u64::from(self.pan[t].to_bits()));
            h.add(self.inputs[t] as u64);
            h.add(self.monitor[t] as u64);
            h.add(u64::from(self.silent[t]));
            h.add(tr.recording(t).map_or(u64::MAX, u64::from));
            for r in tr.regions(t) {
                h.add(r.start as u64);
                h.add(r.end as u64);
                h.add(u64::from(r.take));
                h.add(r.src as u64);
                h.add(u64::from(r.gain.to_bits()));
                h.add(u64::from(r.fade_in));
                h.add(u64::from(r.fade_out));
                h.add(u64::from(r.into));
                h.add(u64::from(r.left));
            }
        }
        // The history UNDO on Arrange may follow.
        let (done, redo) = tr.history();
        h.add(done as u64);
        h.add(redo as u64);
        // The copies (the clip cards' "Making its copy: 40 %", phase 5c):
        // how many are held and under way, and how far each is, in
        // percent; and the cells' ringing playheads, in steps of 1/64.
        let (_, held, busy) = self.host.copy_memory();
        h.add(held as u64);
        h.add(busy as u64);
        h.add(self.host.copies_playing() as u64);
        self.host.copy_progress(&mut |p| h.add((p * 100.0) as u64));
        for t in 0..MAX_TRACKS {
            if let Some((clip, f)) = tr.ringing(t, self.frame_now) {
                h.add(u64::from(clip));
                h.add((f * 64.0) as u64);
            }
            // What each playing clip plays on Stretch (the cells' "repitch
            // now", "rejoins 9.1", phase 5c.2).
            tr.on_stretch_each(t, &mut |clip, s| {
                h.add(u64::from(clip));
                h.add(u64::from(s.repitch) | u64::from(s.drift) << 1);
                h.add(s.join.map_or(u64::MAX, f64::to_bits));
            });
        }
        let s = tr.stats();
        h.add(u64::from(s.refused));
        h.add(u64::from(s.dropped));
        h.add(u64::from(s.full));
        h.add(self.host.fits_failed);
        // The legend's free time, to the second.
        h.add((tr.memory().used / (2 * self.rate as usize).max(1)) as u64);
        h.0
    }

    /// The Song view's audio part of `st`: the lanes, the waveforms, the
    /// memory and the counters. Allocates (a few times a second at most).
    pub fn fill(&self, st: &mut TimeState, map: &TempoMap) {
        let tr = &self.host.tracks;
        let takes: Vec<TakeInfo> = tr.takes().collect();
        st.audio = (0..MAX_TRACKS)
            .filter(|&t| self.shows(t))
            .map(|t| AudioLane {
                track: t,
                // In its project track's place, with its name and colour
                // (phase 1).
                row: self.place(t).unwrap_or(usize::MAX),
                name: self
                    .place(t)
                    .and_then(|i| st.tracks.get(i))
                    .map_or_else(|| format!("Audio {}", t + 1), |i| i.name.clone()),
                color: self.place(t).map_or_else(|| audio_color(t), track_color),
                armed: tr.armed(t),
                input: self.inputs[t],
                mix: self.inside[t].map(|f| MixFrom {
                    name: self.inside_name[t].clone(),
                    after_effects: f.after_effects,
                }),
                level: self.meter[t],
                volume: self.volume[t],
                pan: self.pan[t],
                monitor: self.monitor[t],
                // The project track's (phase 1a).
                mute: self
                    .place(t)
                    .and_then(|i| st.tracks.get(i))
                    .is_some_and(|i| i.mute),
                solo: self
                    .place(t)
                    .and_then(|i| st.tracks.get(i))
                    .is_some_and(|i| i.solo),
                blocks: {
                    let mut blocks = audio_blocks(t, tr.regions(t), &takes, map, self.rate);
                    // Where each one's tempo copy stands (5c.3: its Tempo
                    // card's line).
                    for (b, r) in blocks.iter_mut().zip(tr.regions(t)) {
                        b.copy = self.host.song_copy_state(r, map);
                    }
                    blocks
                },
                takes: take_lanes(t, tr.regions(t), &takes, map, self.rate),
                recording: tr
                    .recording(t)
                    .and_then(|id| tr.take(id))
                    .map(|k| k.anchor.floor() as Tick),
                // [`TrackHost::fill_blocks`]'s.
                clips: Vec::new(),
            })
            .collect();
        st.waves = self.waves.clone();
        st.audio_panel = self.panel;
        st.audio_takes = self.takes_open;
        st.audio_free = (!st.audio.is_empty()).then(|| {
            let m = tr.memory();
            let per_sec = 2.0 * self.rate as f64;
            (
                m.capacity.saturating_sub(m.used) as f64 / per_sec,
                m.capacity as f64 / per_sec,
            )
        });
        // The tempo copies, for System status' Memory (phase 5c.4).
        let (_, held, making) = self.host.copy_memory();
        let mut whose: Vec<String> = Vec::new();
        self.host.copies_making(&mut |rec| {
            if let Some(t) = tr.take_of(rec).and_then(|id| tr.take(id)) {
                let name = self.who(t.track);
                if !whose.contains(&name) {
                    whose.push(name);
                }
            }
        });
        st.audio_copies = mpfree_time_ui::view::AudioCopies {
            held,
            playing: self.host.copies_playing(),
            making,
            whose,
        };
        let s = tr.stats();
        st.audio_counts = [
            ("refused", s.refused),
            ("dropped", s.dropped),
            ("cut short", s.full),
            ("fits failed", self.host.fits_failed as u32),
        ]
        .iter()
        .filter(|c| c.1 > 0)
        .map(|&(w, n)| (w.to_string(), n))
        .collect();
    }

    /// The tempo each recording was made at (0: none, or it changed as it
    /// recorded), by its lasting id, when any changed since the last call:
    /// for the time engine, where a clip region following the tempo at its
    /// recording's is in it (phase 5,
    /// [`crate::time::TimeHost::set_rec_tempos`]). Allocates only when
    /// something changed.
    pub fn tempos_if_changed(&mut self) -> Option<Vec<(RecId, f64)>> {
        let tempos = || {
            self.host.tracks.takes().filter(|k| !k.recording).map(|k| {
                let made_at = if k.varied { 0.0 } else { k.bpm };
                (RecId(k.rec.0), made_at)
            })
        };
        let mut h = Fnv::new();
        for (rec, bpm) in tempos() {
            h.add(rec.0 as u64 ^ (rec.0 >> 64) as u64);
            h.add(bpm.to_bits());
        }
        if h.0 == self.tempos_seen {
            return None;
        }
        self.tempos_seen = h.0;
        Some(tempos().collect())
    }

    /// Each project audio track's regions as the spans of song ticks they
    /// cover, in order, by the track's lasting id, when they changed since
    /// the last call (or `project`'s audio tracks did): for the time
    /// engine, which refuses a block over a region (phase 2, step 2d,
    /// [`crate::time::TimeHost::set_audio_spans`]). Regions that touch
    /// make one span. Allocates only when something changed.
    pub fn spans_if_changed(
        &mut self,
        project: &mpfree_seq::Project,
    ) -> Option<crate::time::AudioSpans> {
        let tr = &self.host.tracks;
        let audio = || {
            project
                .tracks
                .iter()
                .filter(|t| t.is_audio())
                .map(|t| (t.id, self.slots.slot_of(t.id)))
        };
        let mut h = Fnv::new();
        for (id, slot) in audio() {
            h.add(u64::from(id.0));
            for r in slot.map_or(&[][..], |s| tr.regions(s)) {
                h.add(r.start as u64);
                h.add(r.end as u64);
            }
        }
        if h.0 == self.spans_seen {
            return None;
        }
        self.spans_seen = h.0;
        Some(
            audio()
                .map(|(id, slot)| {
                    let mut spans: Vec<(Tick, Tick)> = Vec::new();
                    for r in slot.map_or(&[][..], |s| tr.regions(s)) {
                        match spans.last_mut() {
                            Some(last) if last.1 >= r.start => last.1 = last.1.max(r.end),
                            _ => spans.push((r.start, r.end)),
                        }
                    }
                    (id, spans)
                })
                .collect(),
        )
    }

    /// The audio clips' cells and cards (audio tracks phase 2, layout spec
    /// 4f and 4j): each clip's small wave over one pass (from its takes'
    /// waveforms), its sound's length in seconds, why its audio is missing
    /// if it is, how far a once clip still sounding past its stop has got,
    /// the tempo each region was made at, and where its tempo copies for
    /// the tempo now stand (phase 5c). Into `st` as
    /// [`crate::time::TimeHost::state`] made it of `project`.
    pub fn fill_cells(&self, st: &mut TimeState, project: &mpfree_seq::Project) {
        let tr = &self.host.tracks;
        let bpm = st.transport.bpm;
        for (i, t) in project.tracks.iter().enumerate() {
            if !t.is_audio() {
                continue;
            }
            let at = self.slots.slot_of(t.id);
            let ring = at.and_then(|s| tr.ringing(s, self.frame_now));
            for (row, c) in t.slots.shown().enumerate() {
                let Some((c, a)) = c.as_ref().and_then(|c| Some((c, c.audio.as_ref()?))) else {
                    continue;
                };
                let Some(info) = st
                    .tracks
                    .get_mut(i)
                    .and_then(|x| x.slots.get_mut(row))
                    .and_then(|x| x.as_mut())
                    .and_then(|x| x.audio.as_mut())
                else {
                    continue;
                };
                let per_tick = f64::from(self.rate) * 60.0
                    / (a.bpm.max(1.0) * f64::from(mpfree_transport::time::PPQ));
                info.secs = a.regions.last().map_or(0.0, |r| {
                    (r.start as f64 * per_tick + (a.last_frame - r.src).max(0) as f64)
                        / f64::from(self.rate)
                });
                let gone = a
                    .regions
                    .iter()
                    .find(|r| tr.take_of(RecordingId(r.rec.0)).is_none())
                    .map(|r| r.rec);
                info.missing = gone.map(|rec| match self.opened.iter().find(|o| o.0 == rec) {
                    Some((_, take)) => {
                        format!("{take}'s file wasn't found when the project opened")
                    }
                    None => "its recording isn't in the project".to_string(),
                });
                info.wave = self.clip_wave(a, c.length, per_tick);
                info.ring = ring.filter(|r| r.0 == c.id.0).map(|r| r.1);
                info.tempos = a
                    .regions
                    .iter()
                    .map(|r| {
                        if r.bpm != 0.0 {
                            return r.bpm;
                        }
                        tr.take_of(RecordingId(r.rec.0))
                            .and_then(|id| tr.take(id))
                            .filter(|k| !k.varied)
                            .map_or(0.0, |k| k.bpm)
                    })
                    .collect();
                info.copy = self.clip_copy_state(c, bpm);
                // Playing on Stretch: as made, Repitch for want of its
                // copy, or drifting on Keep pitch (phase 5c.2).
                let now = at.map(|s| tr.on_stretch(s, c.id.0)).unwrap_or_default();
                info.stretch_now = match (now.drift, now.join) {
                    (true, join) => StretchNow::Drift(join.map(|j| j.round() as Tick)),
                    (false, _) if now.repitch => StretchNow::Repitch,
                    _ => StretchNow::AsMade,
                };
            }
        }
    }

    /// Where audio clip `c`'s tempo copies for `bpm` stand (phase 5c):
    /// `CopyState::None` unless a region is on Stretch.
    fn clip_copy_state(&self, c: &mpfree_seq::Clip, bpm: f64) -> CopyState {
        let stretch = c.audio.as_ref().is_some_and(|a| {
            a.regions
                .iter()
                .any(|r| r.follow == mpfree_seq::FollowTempo::Stretch)
        });
        let Some(want) = clip_want(c, Priority::Prepared, bpm).filter(|_| stretch) else {
            return CopyState::None;
        };
        let play = ClipPlay {
            regions: &want.regions[..want.n],
            origin: 0.0,
            len: want.len,
            last_frame: want.last_frame,
            gain: 1.0,
            clip: 0,
            stamp: 0,
        };
        self.host.clip_copy_state(&play, bpm)
    }

    /// The oldest sound put on an audio track's lane since the last ask
    /// (a take punched out, a take lane swiped, a region moved, a sound
    /// brought in), for the time engine to cut the song's blocks there
    /// ([`crate::time::TimeHost::audio_took`]): its track's lasting id,
    /// the song ticks it covers and its undo step, for
    /// [`TrackHost::tie_step`]. Ones on an engine track no project track
    /// plays are passed over.
    pub fn take_took(&mut self) -> Option<(TrackId, (Tick, Tick), u32)> {
        while let Some(t) = self.host.tracks.take_took() {
            if let Some(id) = self.slots.id_of(t.track).filter(|_| self.shows(t.track)) {
                return Some((id, (t.start, t.end), t.step));
            }
        }
        None
    }

    /// Undo step `step` (a [`TrackHost::take_took`]'s) becomes one of song
    /// edit `song`, the cut of the blocks under it, so either UNDO takes
    /// back both ([`Command::TieStep`]).
    pub fn tie_step(
        &mut self,
        step: u32,
        song: mpfree_transport::TransactionId,
    ) -> Result<(), String> {
        self.send(Command::TieStep { step, song })
    }

    /// Rec song put blocks of audio clips on audio tracks' lanes over
    /// `spans` (each track's lasting id, and song ticks), as song edit
    /// `song`: the regions there go (Joshi 23 Nov: the launch replaces
    /// them), as steps of that song edit, so the song's UNDO takes them
    /// back with the blocks and the audio tracks' UNDO stops at them
    /// (phase 2, step 2d, [`Command::EraseSong`]). Only where a region
    /// plays. How many spans it erased, and how many over regions it
    /// couldn't (the queue full: it stops there).
    pub fn replace_under(
        &mut self,
        song: mpfree_transport::TransactionId,
        spans: &[(TrackId, Tick, Tick)],
    ) -> (usize, usize) {
        let (mut done, mut left) = (0, 0);
        for &(id, from, to) in spans {
            let Some(track) = self.slots.slot_of(id) else {
                continue;
            };
            let under = self
                .host
                .tracks
                .regions(track)
                .iter()
                .any(|r| r.start < to && r.end > from);
            if !under || from >= to {
                continue;
            }
            let c = Command::EraseSong {
                song,
                track,
                from,
                to,
            };
            if left > 0 || self.send(c).is_err() {
                left += 1;
                continue;
            }
            done += 1;
        }
        (done, left)
    }

    /// The blocks of audio clips on the Song view's audio lanes (phase 2,
    /// step 2d, layout spec 4c): for each, where its clip's passes start
    /// and how long one is, whether it is a once clip, and what one pass
    /// plays, take by take, with the frames along `map` at the block's
    /// first pass. Into `st` after [`TrackHost::fill`] made its lanes of
    /// `project`; a block whose clip or takes are gone draws its header
    /// without a wave. A loop cell's block (phase 4e.3) names its loop,
    /// whose wave the screen has from the looper.
    pub fn fill_blocks(&self, st: &mut TimeState, project: &mpfree_seq::Project, map: &TempoMap) {
        for (index, p) in project.arrangement.placements.iter().enumerate() {
            let Some(lane) = st.audio.iter_mut().find(|l| l.row == p.track) else {
                continue;
            };
            let Some(c) = project.block_clip(p) else {
                continue;
            };
            let origin = p.origin(c);
            let pass = c.length.max(1);
            if let Some(id) = c.loop_id() {
                lane.clips.push(ClipBlock {
                    index,
                    origin,
                    pass,
                    once: false,
                    pieces: Vec::new(),
                    looped: Some(id.0),
                });
                continue;
            }
            let Some(a) = c.audio.as_ref() else {
                continue;
            };
            // The first pass the block plays some of.
            let base = origin + (p.start - origin).div_euclid(pass) * pass;
            let pieces = a
                .regions
                .iter()
                .filter_map(|r| {
                    let er = self.played(r)?;
                    let (from, to) = ((base + r.start) as f64, (base + r.end) as f64);
                    Some(ClipPiece {
                        start: r.start,
                        end: r.end,
                        take: er.take,
                        src: r.src,
                        // Its recording's frames under it: along the song,
                        // or following the tempo at its own.
                        frames: er.src_pos(map, from, to, self.rate) - f64::from(r.phase),
                    })
                })
                .collect();
            lane.clips.push(ClipBlock {
                index,
                origin,
                pass,
                once: c.once,
                pieces,
                looped: None,
            });
        }
    }

    /// Clip region `r` as the track player plays it: the take it names
    /// here, and its tempo resolved (0, its take's: as the player's
    /// `Set::fill`). None when its take isn't here.
    fn played(&self, r: &mpfree_seq::ClipRegion) -> Option<Region> {
        let tr = &self.host.tracks;
        let take = tr.take_of(RecordingId(r.rec.0))?;
        let info = tr.take(take)?;
        let made_at = if info.varied { 0.0 } else { info.bpm };
        Some(Region {
            start: r.start,
            end: r.end,
            take,
            src: r.src,
            phase: r.phase,
            gain: r.gain,
            fade_in: r.fade_in,
            fade_out: r.fade_out,
            into: r.into,
            left: r.left,
            follow: follow_in(r.follow),
            keep_pitch: r.keep_pitch,
            bpm: if r.bpm == 0.0 { made_at } else { r.bpm },
        })
    }

    /// An audio clip's small wave over one pass of `length` clip ticks
    /// (`per_tick` frames a tick at the tempo it was made at): each
    /// column's peak, 0 to 255, from its takes' waveforms read so far
    /// (silent where none is yet), with the regions' and clip's gains.
    fn clip_wave(&self, a: &mpfree_seq::AudioClip, length: Tick, per_tick: f64) -> [u8; CELL_WAVE] {
        let mut out = [0u8; CELL_WAVE];
        let length = length.max(1) as f64;
        for (c, v) in out.iter_mut().enumerate() {
            let (t0, t1) = (
                length * c as f64 / CELL_WAVE as f64,
                length * (c + 1) as f64 / CELL_WAVE as f64,
            );
            let mut peak = 0f32;
            for r in &a.regions {
                let (a0, a1) = (t0.max(r.start as f64), t1.min(r.end as f64));
                if a0 >= a1 {
                    continue;
                }
                let Some((w, er)) = self.played(r).and_then(|er| {
                    let w = self.waves.iter().find(|w| w.id == er.take)?;
                    Some((w, er))
                }) else {
                    continue;
                };
                // Following the tempo, at its own tempo.
                let per_tick = er.per_tick(self.rate).unwrap_or(per_tick);
                let f0 = r.src as f64 + (a0 - r.start as f64) * per_tick;
                let f1 = r.src as f64 + (a1 - r.start as f64) * per_tick;
                let fpc = w.frames_per_column.max(1) as f64;
                let (k0, k1) = (
                    (f0.max(0.0) / fpc) as usize,
                    (f1.max(0.0) / fpc).ceil() as usize,
                );
                for &(lo, hi) in w.columns.iter().take(k1.max(k0 + 1)).skip(k0) {
                    peak = peak.max(lo.abs().max(hi.abs()) * r.gain);
                }
            }
            *v = ((peak * a.gain).min(1.0) * 255.0).round() as u8;
        }
        out
    }

    /// To a cell (audio tracks phase 2): the audio clip that plays what
    /// region `r` plays in the song, and the project track it goes on.
    /// The clip holds one region of the region's recording on its own
    /// ticks, with the region's gain and fades (and how much of a cut
    /// region lies either side, so its fades stay exactly its share); it
    /// is as long as the region, made at the tempo the song (`map`) has at
    /// the region's start, its sound ending where the region's does, and
    /// named after its track and bars ("Vocal 5–8", in `meter`'s bars);
    /// with the words for where it was ("Vocal, bars 5 to 8"). Err says
    /// why not: no such region, its take still recording, or its
    /// track not in the project.
    pub fn region_clip(
        &self,
        r: RegionId,
        map: &TempoMap,
        meter: mpfree_transport::time::Meter,
    ) -> Result<(usize, mpfree_seq::Clip, String), String> {
        let tr = &self.host.tracks;
        let reg = region(tr, r)?;
        let take = tr
            .take(reg.take)
            .filter(|t| !t.recording)
            .ok_or("that region's take is still recording")?;
        let track = self
            .place(r.track)
            .ok_or_else(|| format!("{} is not in the track list", self.who(r.track)))?;
        let length = reg.end - reg.start;
        let frames = reg.frames_to(map, reg.start as f64, reg.end as f64, self.rate);
        let bar = meter.bar_ticks().max(1);
        let (first, last) = (reg.start / bar + 1, (reg.end + bar - 1) / bar);
        let who = self.who(r.track);
        let (name, place) = if last > first {
            (
                format!("{who} {first}–{last}"),
                format!("{who}, bars {first} to {last}"),
            )
        } else {
            (format!("{who} {first}"), format!("{who}, bar {first}"))
        };
        let made_at = if take.varied { 0.0 } else { take.bpm };
        let mut one = clip_region_of(&reg, RecId(take.rec.0), made_at);
        (one.start, one.end) = (0, length);
        // Following the tempo (phase 5), it was made at its own tempo,
        // and its sound ends where its recording's does at that tempo.
        let follows = reg.per_tick(self.rate).is_some();
        let mut clip = mpfree_seq::Clip::new(&name, length);
        clip.audio = Some(mpfree_seq::AudioClip {
            regions: vec![one],
            bpm: if follows {
                reg.bpm
            } else {
                map.bpm_at(reg.start)
            },
            gain: 1.0,
            last_frame: reg.src + reg.src_len(frames, self.rate).max(1),
        });
        clip.check()
            .map_err(|_| format!("{place} can't be a clip: it is longer than a clip holds"))?;
        Ok((track, clip, place))
    }

    /// Clear copies (System status' Memory, 5c.4): the tempo copies that
    /// aren't playing or about to go, to be made again when next wanted.
    /// How many went.
    pub fn clear_copies(&mut self) -> usize {
        self.host.clear_copies()
    }

    /// A recording no clip will name (a sound Put in a cell whose cell
    /// went meanwhile): forgotten, its memory back.
    pub fn forget_recording(&mut self, rec: RecId) {
        if let Some(id) = self.host.tracks.take_of(RecordingId(rec.0)) {
            let _ = self.host.command(Command::Forget(id));
        }
    }

    /// What a touch on the Song view's audio lanes asked for. `t` is the
    /// transport now, `line_open` whether the line input is on. Ok is a
    /// line for the log (may be empty); Err says why nothing happened.
    pub fn action(
        &mut self,
        a: AudioAction,
        t: &TransportInfo,
        line_open: bool,
    ) -> Result<String, String> {
        let tr = &self.host.tracks;
        let line = match a {
            AudioAction::Arm { track, on } => {
                if track >= MAX_TRACKS {
                    return Err(format!("there is no audio track {}", track + 1));
                }
                self.send(Command::Arm(track, on))?;
                let n = self.who(track);
                match (on, line_open) {
                    (false, _) => format!("{n} disarmed"),
                    (true, _) if self.inside[track].is_some() => format!(
                        "{n} armed: REC records {} in song mode",
                        self.inside_name[track]
                    ),
                    (true, true) => format!("{n} armed: REC records it in song mode"),
                    (true, false) => format!(
                        "{n} armed, but the line input is off: turn on Prefs > Line input to record"
                    ),
                }
            }
            // An audio track is a track (phase 1a): mute and solo are the
            // track's (`main::time_action`).
            AudioAction::Mute { .. } | AudioAction::Solo { .. } => {
                return Err("an audio track's mute and solo are its track's".into())
            }
            AudioAction::Split(r) => {
                let reg = region(tr, r)?;
                let at = t.position;
                if !(reg.start < at && at < reg.end) {
                    return Err(
                        "split: move the playhead inside the region (Split cuts at the playhead)"
                            .into(),
                    );
                }
                self.send(Command::Split { track: r.track, at })?;
                format!("{}: region split", self.who(r.track))
            }
            AudioAction::Erase(r) => {
                let reg = region(tr, r)?;
                self.send(Command::Erase {
                    track: r.track,
                    from: reg.start,
                    to: reg.end,
                })?;
                format!("{}: region erased (Undo brings it back)", self.who(r.track))
            }
            AudioAction::Undo => {
                if (0..MAX_TRACKS).any(|t| tr.recording(t).is_some()) {
                    return Err("undo waits until the recording stops".into());
                }
                if tr.history().0 == 0 {
                    return Err("nothing to undo".into());
                }
                if tr.undo_is_song().is_some() {
                    return Err(SONG_UNDO.into());
                }
                self.send(Command::Undo)?;
                "audio tracks: undone (SHIFT + Undo redoes)".into()
            }
            AudioAction::Redo => {
                if (0..MAX_TRACKS).any(|t| tr.recording(t).is_some()) {
                    return Err("redo waits until the recording stops".into());
                }
                if tr.history().1 == 0 {
                    return Err("nothing to redo".into());
                }
                if tr.redo_is_song().is_some() {
                    return Err(SONG_REDO.into());
                }
                self.send(Command::Redo)?;
                "audio tracks: redone".into()
            }
            AudioAction::Gain(r) => {
                region(tr, r)?;
                "Q3 turns the region's gain, SHIFT for fine steps".into()
            }
            // The clip goes to the time engine's cells: the app builds it
            // with `region_clip` and puts it in (`main::time_action`).
            AudioAction::ToCell(_) => return Err("To a cell goes through the clips".into()),
            // Following the tempo (phase 5): a step of the region edits'.
            AudioAction::Follow {
                region: r,
                follow,
                keep_pitch,
            } => {
                let reg = region(tr, r)?;
                self.send(Command::SetFollow {
                    track: r.track,
                    at: reg.start,
                    follow,
                    keep_pitch,
                })?;
                let how = match follow {
                    FollowTempo::Off => "Off",
                    FollowTempo::Repitch => "Repitch",
                    FollowTempo::Stretch if keep_pitch => "Stretch, keeping its pitch",
                    FollowTempo::Stretch => "Stretch",
                };
                format!(
                    "{}: Follow tempo {how} (Undo takes it back)",
                    self.who(r.track)
                )
            }
            AudioAction::Tempo {
                region: r,
                centi_bpm,
            } => {
                let reg = region(tr, r)?;
                let take = tr.take(reg.take).ok_or("that region's take is gone")?;
                let bpm = match centi_bpm {
                    0 if take.varied => 0.0,
                    0 => take.bpm,
                    c => f64::from(c) / 100.0,
                };
                if !(bpm == 0.0 || mpfree_transport::time::valid_bpm(bpm)) {
                    return Err(format!("MPFree doesn't play at {bpm} BPM"));
                }
                self.send(Command::SetTempo {
                    track: r.track,
                    at: reg.start,
                    bpm,
                })?;
                let said = if centi_bpm == 0 {
                    "its take's".to_string()
                } else {
                    format!("{bpm} BPM")
                };
                format!(
                    "{}: region's tempo {said} (Undo takes it back)",
                    self.who(r.track)
                )
            }
            AudioAction::Fades(r) => {
                region(tr, r)?;
                "Q4 turns the region's fades, SHIFT for fine steps".into()
            }
            AudioAction::Turn {
                track: t,
                region: index,
                knob,
                ticks,
                fine,
            } => {
                if t >= MAX_TRACKS {
                    return Err(format!("there is no audio track {}", t + 1));
                }
                // Q3 and Q4 need a region: the level and pan do not.
                let reg = match index {
                    Some(index) => Some(region(tr, RegionId { track: t, index })?),
                    None => None,
                };
                let picked = || {
                    reg.ok_or_else(|| {
                        format!("Q{}: pick a region of {} first", knob + 1, self.who(t))
                    })
                };
                match knob {
                    // The level and pan are the track's mixer strip's
                    // (phase 1b): the app moves the strip (main.rs's
                    // `time_action`), and the engine track stays at unity
                    // and centre, so they never count twice.
                    0 | 1 => {
                        return Err(format!(
                            "{}: its level and pan are its mixer strip's",
                            self.who(t)
                        ))
                    }
                    2 => {
                        let reg = picked()?;
                        let step = if fine { 0.1 } else { 0.5 };
                        let gain = from_db(to_db(reg.gain) + step * ticks as f32, GAIN_MAX_DB);
                        self.send(Command::SetGain {
                            track: t,
                            at: reg.start,
                            gain,
                        })?;
                    }
                    3 => {
                        let reg = picked()?;
                        let ms = if fine { 1.0 } else { 5.0 };
                        let step = (self.rate as f32 * ms / 1000.0) as i64;
                        let now = i64::from(reg.fade_in.max(reg.fade_out));
                        let v =
                            (now + step * i64::from(ticks)).clamp(0, i64::from(u32::MAX)) as u32;
                        // The engine keeps each fade to half the region.
                        self.send(Command::SetFades {
                            track: t,
                            at: reg.start,
                            fade_in: v,
                            fade_out: v,
                        })?;
                    }
                    _ => return Err(format!("Q-Link {} has nothing to turn here", knob + 1)),
                }
                self.changed = true;
                String::new()
            }
            AudioAction::Panel(t) => {
                if t >= MAX_TRACKS {
                    return Err(format!("there is no audio track {}", t + 1));
                }
                self.panel = if self.panel == Some(t) { None } else { Some(t) };
                self.changed = true;
                String::new()
            }
            AudioAction::ClosePanel => {
                self.panel = None;
                self.changed = true;
                String::new()
            }
            // The app opens the Mix tab at the track's strip (main.rs's
            // `time_action`): nothing here.
            AudioAction::Strip(_) => String::new(),
            AudioAction::Input { track, input } => self.set_line(track, input)?,
            // The app answers with what the mixer offers (main.rs).
            AudioAction::Inputs(_) => String::new(),
            AudioAction::InputPick { track, k } => self.input_pick(track, k)?,
            AudioAction::Mix { track, step } => self.mix_step(track, step)?,
            AudioAction::MixEffects { track, after } => {
                let n = self.track_no(track)?;
                let Some(f) = self.inside[track] else {
                    return Err(format!("{n} records the line input: pick Mix first"));
                };
                if f.after_effects == after && !f.after_fader {
                    return Ok(String::new());
                }
                let name = self.inside_name[track].clone();
                let f = Inside {
                    after_effects: after,
                    after_fader: false,
                    ..f
                };
                self.set_inside(track, Some((f, &name)))?
            }
            AudioAction::Monitor { track, monitor } => {
                let n = self.track_no(track)?;
                self.send(Command::SetMonitor(track, monitor))?;
                self.monitor[track] = monitor;
                self.changed = true;
                format!("{n} monitor: {}", monitor_words(monitor))
            }
            AudioAction::Takes(t) => {
                let n = self.track_no(t)?;
                if self.takes_open == Some(t) {
                    self.takes_open = None;
                    self.changed = true;
                    return Ok(String::new());
                }
                let count = tr.takes().filter(|k| k.track == t).count();
                if count == 0 {
                    return Err(format!("{n} has no takes yet: REC records one"));
                }
                // The lanes go under the track's lane, where the panel
                // may sit.
                self.takes_open = Some(t);
                self.panel = None;
                self.changed = true;
                format!(
                    "{n}: {count} take{} under the lane: swipe across one to play that stretch, tap it to use it all",
                    if count == 1 { "" } else { "s" }
                )
            }
            AudioAction::Comp {
                take,
                from,
                to,
                fine,
            } => {
                let k = tr
                    .take(take)
                    .ok_or_else(|| "that take is gone".to_string())?;
                if tr.recording(k.track).is_some() {
                    return Err(format!(
                        "{}: comping waits until the recording stops",
                        self.who(k.track)
                    ));
                }
                let (from, to) = if fine {
                    (from, to)
                } else {
                    snap_span(from, to)
                };
                let name = take_name(tr, take);
                self.send(Command::Comp { take, from, to })?;
                format!(
                    "{}: {name} plays from {} to {}",
                    self.who(k.track),
                    bar_beat(from, t),
                    bar_beat(to, t)
                )
            }
            AudioAction::Move {
                region: r,
                to,
                fine,
            } => {
                let reg = region(tr, r)?;
                if tr.recording(r.track).is_some() {
                    return Err(format!(
                        "{}: moving waits until the recording stops",
                        self.who(r.track)
                    ));
                }
                let to = if fine { to } else { snap(to) }.max(0);
                if to == reg.start {
                    return Ok(String::new());
                }
                self.send(Command::Move {
                    track: r.track,
                    at: reg.start,
                    to,
                })?;
                self.repick = Some((r.track, to));
                format!(
                    "{}: region moved to {} (Undo puts it back)",
                    self.who(r.track),
                    bar_beat(to, t)
                )
            }
            AudioAction::UseTake(take) => {
                let k = tr
                    .take(take)
                    .ok_or_else(|| "that take is gone".to_string())?;
                if tr.recording(k.track).is_some() {
                    return Err(format!(
                        "{}: comping waits until the recording stops",
                        self.who(k.track)
                    ));
                }
                let name = take_name(tr, take);
                // The engine keeps it to where the take has audio.
                self.send(Command::Comp {
                    take,
                    from: Tick::MIN,
                    to: Tick::MAX,
                })?;
                format!(
                    "{}: {name} plays everywhere it has audio",
                    self.who(k.track)
                )
            }
            // The time engine keeps the pick (`TimeControl`).
            AudioAction::Pick(_) => String::new(),
        };
        Ok(line)
    }

    /// The audio track a sound from the Sample page goes on
    /// ([`TrackHost::bring`]): the first of the track list's audio tracks
    /// that is empty, else the first with nothing from the bar the
    /// playhead is in (`t`) on. None: none has room (the app adds one).
    pub fn bring_track(&self, t: &TransportInfo) -> Option<usize> {
        let tr = &self.host.tracks;
        let bar = t.meter.bar_ticks().max(1);
        let at = t.position.div_euclid(bar) * bar;
        let mut listed: [usize; MAX_TRACKS] = std::array::from_fn(|s| s);
        listed.sort_by_key(|&s| self.place(s).unwrap_or(usize::MAX));
        let listed = listed.into_iter().filter(|&s| self.place(s).is_some());
        let empty = |k: usize| tr.regions(k).is_empty() && !tr.armed(k);
        let free = |k: usize| !tr.armed(k) && tr.regions(k).iter().all(|r| r.end <= at);
        listed
            .clone()
            .find(|&k| empty(k))
            .or_else(|| listed.clone().find(|&k| free(k)))
    }

    /// A sound from the Sample page (its Slice tool's Put on Audio track,
    /// the one Sample page's step 6), as the MPC's Audio Track from
    /// Samples: on the track [`TrackHost::bring_track`] picks, from the
    /// bar the playhead is in (`t`), a region a slice, as one undo step.
    /// Ok is a line for the log; Err says why not. Call it between
    /// periods (it drops the sound's name and cuts; the audio goes to the
    /// tracks, or to their worker).
    pub fn bring(&mut self, s: Sound, t: &TransportInfo) -> Result<String, String> {
        let Sound {
            name,
            channels,
            audio,
            cuts,
            bpm,
            ..
        } = s;
        let tr = &self.host.tracks;
        let refuse = |h: &mut TrackHost, audio: Vec<f32>, why: String| {
            h.host.drop_audio(audio);
            Err(why)
        };
        if (0..MAX_TRACKS).any(|t| tr.recording(t).is_some()) {
            return refuse(
                self,
                audio,
                "a sound goes on an audio track once the recording stops".into(),
            );
        }
        let bar = t.meter.bar_ticks().max(1);
        let at = t.position.div_euclid(bar) * bar;
        let Some(track) = self.bring_track(t) else {
            let why = format!(
                "every audio track has regions from bar {} on: move the playhead past them, erase some in the Song view, or add an audio track",
                at / bar + 1
            );
            return refuse(self, audio, why);
        };
        let frames = audio.len() / channels.max(1);
        let secs = frames as f64 / f64::from(self.rate);
        let slices = 1 + cuts
            .iter()
            .filter(|&&c| 0 < c && (c as usize) < frames)
            .count();
        match self.host.import(track, at, channels, audio, cuts, bpm) {
            Ok(_) => {
                self.changed = true;
                let what = match slices {
                    1 => name,
                    n => format!("{name}'s {n} slices"),
                };
                // Audio tracks play in song mode only (layout spec 4c).
                let song = if self.active {
                    ""
                } else {
                    "it plays in song mode, and "
                };
                Ok(format!(
                    "{what} on {} from bar {}; {song}the Song view's Undo takes it off",
                    self.who(track),
                    at / bar + 1
                ))
            }
            Err(ImportRefused::Busy) => Err("another sound is still going onto its track: a moment".into()),
            Err(ImportRefused::NoTake) => Err(format!(
                "the audio tracks hold {} takes, the most: forget some takes first",
                mpfree_audiotracks::MAX_TAKES
            )),
            Err(ImportRefused::NoRoom) => Err(format!(
                "{secs:.1} s is more than the audio tracks' memory has left: erase takes, or more memory in Prefs > Audio track memory"
            )),
        }
    }

    /// Put in a cell (audio tracks phase 2, the Sample page's Put on Audio
    /// track with In a cell): sound `s`, whole, as a take of its own on
    /// project track `track` (an audio track) that the song doesn't play.
    /// Once it is in, [`TrackHost::tend_cell_sound`] gives the clip that
    /// plays it, for the clips to put in the track's first empty cell.
    /// Ok is a line for the log; Err says why not (another sound still
    /// coming in, no take or memory left, not an audio track). Call it
    /// between periods, as [`TrackHost::bring`].
    pub fn bring_to_cell(
        &mut self,
        s: Sound,
        track: usize,
        project: &mpfree_seq::Project,
    ) -> Result<String, String> {
        let Sound {
            name,
            channels,
            audio,
            bpm,
            ..
        } = s;
        let engine = project
            .tracks
            .get(track)
            .filter(|t| t.is_audio())
            .and_then(|t| self.slots.slot_of(t.id));
        let Some(engine) = engine.filter(|_| self.cell_sound.is_none()) else {
            self.host.drop_audio(audio);
            return Err(if self.cell_sound.is_some() {
                "another sound is still going into its cell: a moment".into()
            } else {
                format!("there is no audio track {}", track + 1)
            });
        };
        let frames = audio.len() / channels.max(1);
        let secs = frames as f64 / f64::from(self.rate);
        match self.host.import_for_clip(engine, channels, audio, bpm) {
            Ok(_) => {
                let who = self.who(engine);
                self.cell_sound = Some(CellSound {
                    track,
                    name: name.clone(),
                    frames,
                    bpm,
                });
                Ok(format!("{name} going into {who}"))
            }
            Err(ImportRefused::Busy) => {
                Err("another sound is still going onto its track: a moment".into())
            }
            Err(ImportRefused::NoTake) => Err(format!(
                "the audio tracks hold {} takes, the most: forget some takes first",
                mpfree_audiotracks::MAX_TAKES
            )),
            Err(ImportRefused::NoRoom) => Err(format!(
                "{secs:.1} s is more than the audio tracks' memory has left: erase takes, or more memory in Prefs > Audio track memory"
            )),
        }
    }

    /// Each loop: the sound Put in a cell has come in. Some once it has:
    /// the project track and the clip that plays it (or why not). A sound
    /// with a tempo loops, made at its tempo, as long as it is to the
    /// nearest beat; one with none plays once, made at `bpm` (the
    /// project's), its length up to the next beat, and rings to its end.
    pub fn tend_cell_sound(
        &mut self,
        bpm: f64,
    ) -> Option<Result<(usize, mpfree_seq::Clip), String>> {
        self.cell_sound.as_ref()?;
        let id = match self.host.imported() {
            Some(id) => id,
            None if self.host.importing().is_some() => return None,
            None => {
                let c = self.cell_sound.take()?;
                return Some(Err(format!("{} didn't come in: try again", c.name)));
            }
        };
        let rec = self.host.tracks.take(id).map(|t| RecId(t.rec.0));
        let c = self.cell_sound.take()?;
        let Some(rec) = rec else {
            return Some(Err(format!("{} didn't come in: try again", c.name)));
        };
        let secs = c.frames as f64 / f64::from(self.rate);
        let ppq = f64::from(mpfree_transport::time::PPQ);
        let (made, beats, once) = if mpfree_transport::time::valid_bpm(c.bpm) {
            (c.bpm, (secs * c.bpm / 60.0).round().max(1.0), false)
        } else {
            (bpm, (secs * bpm / 60.0 - 1e-9).ceil().max(1.0), true)
        };
        let length = (beats * ppq) as Tick;
        let mut clip = mpfree_seq::Clip::new(&c.name, length);
        clip.audio = Some(mpfree_seq::AudioClip::of(
            rec,
            0,
            c.frames.max(1) as i64,
            length,
            made,
        ));
        clip.once = once;
        if clip.check().is_err() {
            let _ = self.host.command(Command::Forget(id));
            return Some(Err(format!("{} is longer than a clip holds", c.name)));
        }
        Some(Ok((c.track, clip)))
    }

    /// Track `t`'s number as the screen writes it, if there is one.
    /// What the lines call engine track `t` ([`TrackHost::who`]), or why
    /// there is no such track.
    fn track_no(&self, t: usize) -> Result<String, String> {
        if t < MAX_TRACKS {
            Ok(self.who(t))
        } else {
            Err(format!("there is no audio track {}", t + 1))
        }
    }

    /// What the hint and log lines call engine track `s`: its project
    /// track's name, as the musician named it, else "Audio N" (N the
    /// engine track's number, as its lane says without a name).
    fn who(&self, s: usize) -> String {
        match self.names.get(s).map(|n| n.trim()) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => format!("Audio {}", s + 1),
        }
    }

    /// Keeps `name` as engine track `s`'s name without allocating: the
    /// room was made at start ([`NAME_ROOM`]), and a longer name is cut
    /// short on a character's edge.
    fn keep_name(&mut self, s: usize, name: &str) {
        let Some(room) = self.names.get_mut(s) else {
            return;
        };
        let mut end = name.len().min(room.capacity());
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        if room.as_str() != &name[..end] {
            room.clear();
            room.push_str(&name[..end]);
        }
        // Its next takes' files are named for it too, "Audio N" as its
        // lane says when it has no name.
        let file = match name.trim() {
            "" => AUDIO_N.get(s).copied().unwrap_or("Audio"),
            n => n,
        };
        self.host.set_name(s, file);
    }

    /// Before the audio tracks' own Undo (`undo`) or Redo: a song edit's
    /// step next that the song's UNDO, or REDO, no longer reaches
    /// (`reaches`) is forgotten, with the steps only reachable through
    /// it, and the line says why there is nothing to take back (None:
    /// Undo goes on as usual). See [`Command::ForgetSong`].
    pub fn forget_song_step(
        &mut self,
        undo: bool,
        reaches: impl FnOnce(mpfree_transport::TransactionId) -> bool,
    ) -> Option<String> {
        let tr = &self.host.tracks;
        let song = if undo {
            tr.undo_is_song()
        } else {
            tr.redo_is_song()
        }?;
        if tr.busy() || reaches(song) {
            return None;
        }
        self.send(Command::ForgetSong(song)).ok()?;
        Some(if undo {
            "nothing to undo: the song edit that moved the regions can no longer be undone".into()
        } else {
            "nothing to redo: the song edit that moved the regions can no longer be redone".into()
        })
    }

    /// Forget the steps Redo could bring back (`redo`), else let go of
    /// those Undo could take back made at or before `through`, for the
    /// one UNDO across the engines (audio tracks phase 4d.3,
    /// [`Command::ForgetRedo`], [`Command::RetireThrough`]): from the next
    /// period. Says why not (the queue is full).
    pub fn forget_steps(
        &mut self,
        redo: bool,
        through: mpfree_transport::TransactionId,
    ) -> Result<(), String> {
        self.send(if redo {
            Command::ForgetRedo
        } else {
            Command::RetireThrough(through)
        })
    }

    /// Read loop layer recording `rec`'s file back, as this run wrote it
    /// (phase 4d.4): its audio comes back whole
    /// ([`TrackHost::layer_read_back`]). False with no worker.
    pub fn read_layer(&mut self, rec: u128) -> bool {
        self.host.layer_read(RecordingId(rec))
    }

    /// A layer's file read back ([`TrackHost::read_layer`]): its audio,
    /// interleaved stereo, or why not. Give the audio back with
    /// [`TrackHost::drop_audio`].
    pub fn layer_read_back(&mut self) -> Option<(u128, Result<Vec<f32>, String>)> {
        self.host.layer_read_back().map(|(r, a)| (r.0, a))
    }

    /// Audio done with, dropped on the worker.
    pub fn drop_audio(&self, audio: Vec<f32>) {
        self.host.drop_audio(audio);
    }

    fn send(&mut self, c: Command) -> Result<(), String> {
        if self.host.command(c) {
            Ok(())
        } else {
            Err("the audio tracks' queue is full; try again".into())
        }
    }

    /// The status line's part.
    pub fn describe(&self) -> String {
        let tr = &self.host.tracks;
        let m = tr.memory();
        let s = tr.stats();
        // Sound put on a lane the song's blocks never heard of (never:
        // the app asks every period), shown only if any.
        let lost = match tr.took_lost() {
            0 => String::new(),
            n => format!(", {n} recordings not checked against the song's blocks"),
        };
        // While takes play from their files: how many are on them, what
        // played as silence because the drive was late (since the start),
        // and the region starts held (the stream test reads these, D3).
        let files = self.stream_numbers().map_or_else(String::new, |n| {
            format!(
                "; from their files: {} of {} takes, {:.2} s went silent, {} of {} region starts held",
                n.on_file, n.takes, n.silent_s, n.heads_held, n.heads_wanted
            )
        });
        let times = timings_line(&self.host.timings());
        format!(
            "audio tracks: {} takes, {:.0} of {:.0} s used, {} refused, {} dropped, {} cut short{lost}{}{files}{times}",
            tr.takes().count(),
            m.used as f64 / (2.0 * self.rate as f64),
            m.capacity as f64 / (2.0 * self.rate as f64),
            s.refused,
            s.dropped,
            s.full,
            if self.active { "" } else { " (song mode off)" }
        )
    }
}

/// The status line's timings (the SD card plan's B2a, for the device
/// rounds' logs): each drive's reads that have run, and the recording's
/// syncs, as "; stick reads: median 2.0 ms, 1 in 100 40 ms, 1 in 1,000
/// 300 ms, worst 1.20 s, of 1234, 0 waiting; syncs on the card: …".
/// Empty while nothing has been read or synced.
fn timings_line(t: &mpfree_audiotracks::streaming::Timings) -> String {
    let mut line = String::new();
    for d in t.reads.iter().filter(|d| d.reads.count > 0) {
        let drive = if d.root.is_some() { "card" } else { "stick" };
        line += &format!("; {drive} reads: {}, {} waiting", d.reads, d.queue);
    }
    if let Some(s) = t.syncs.as_ref().filter(|s| s.times.count > 0) {
        let drive = if s.on_card { "the card" } else { "the stick" };
        line += &format!("; syncs on {drive}: {}", s.times);
    }
    line
}

/// The track level's top: +6 dB. A region's gain goes to the engine's
/// 4 (+12 dB). Below -60 dB is off.
const LEVEL_MAX_DB: f32 = 6.0;
const GAIN_MAX_DB: f32 = 12.0;
const FLOOR_DB: f32 = -60.0;

/// A level as dB, with off as the floor.
fn to_db(v: f32) -> f32 {
    if v <= 0.0 {
        FLOOR_DB
    } else {
        (20.0 * v.log10()).max(FLOOR_DB)
    }
}

/// dB as a level, at most `max` dB; at the floor or under it is off.
fn from_db(db: f32, max: f32) -> f32 {
    let db = db.min(max);
    if db <= FLOOR_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// "-3.0 dB", or "off".
fn db_text(v: f32) -> String {
    if v <= 0.0 {
        "off".into()
    } else {
        format!("{:.1} dB", to_db(v))
    }
}

/// "C", "L15", "R40", as the mixer writes a pan.
fn pan_text(p: f32) -> String {
    let n = (p.abs() * 100.0).round() as i32;
    match n {
        0 => "C".into(),
        _ if p < 0.0 => format!("L{n}"),
        _ => format!("R{n}"),
    }
}

/// What the Q-Links turn on Arrange with an audio region picked or a
/// track panel open (layout spec 4c): see [`audio_knobs`].
#[derive(Clone, Debug, PartialEq)]
pub struct AudioKnobs {
    /// The audio track whose level and pan Q1 and Q2 turn.
    pub track: usize,
    /// The region (an index into the track's) whose gain and fades Q3 and
    /// Q4 turn: None when none of the track's is picked.
    pub region: Option<usize>,
    pub cards: [QLinkCard; 4],
}

impl AudioKnobs {
    /// The column's title: "Q-Links · Audio region" or "Q-Links · Audio 2".
    pub fn title(&self) -> String {
        match self.region {
            Some(_) => "Q-Links · Audio region".into(),
            None => format!("Q-Links · Audio {}", self.track + 1),
        }
    }
}

/// The Q-Link cards for `st` (layout spec 4c): with the track panel open,
/// that track's; else with an audio region picked, its track's. Q1 the
/// track's level, Q2 its pan, Q3 the region's gain, Q4 its fades (they
/// say "pick a region" when none of the track's is picked). None when
/// neither is so, or the track is not in the lanes.
pub fn audio_knobs(st: &TimeState) -> Option<AudioKnobs> {
    let t = st.audio_panel.or(st.region.map(|r| r.track))?;
    let lane = st.audio.iter().find(|l| l.track == t)?;
    let index = st.region.filter(|r| r.track == t).map(|r| r.index);
    let b = index.and_then(|i| lane.blocks.get(i));
    // Level and pan are the track's strip's (phase 1b, as drawn).
    let track = "ITS STRIP";
    let pos = |v: f32, max: f32| (to_db(v) - FLOOR_DB) / (max - FLOOR_DB);
    let mut pan = QLinkCard::new("Pan", track, &pan_text(lane.pan), (lane.pan + 1.0) / 2.0);
    pan.bipolar = true;
    let level = QLinkCard::new(
        "Level",
        track,
        &db_text(lane.volume),
        pos(lane.volume, LEVEL_MAX_DB),
    );
    let [gain, fades] = match b {
        Some(b) => {
            let ms = (b.fade_in.max(b.fade_out) as f64 / RATE_FOR_TEXT * 1000.0).round();
            [
                QLinkCard::new("Gain", "REGION", &db_text(b.gain), pos(b.gain, GAIN_MAX_DB)),
                QLinkCard::new(
                    "Fades",
                    "REGION",
                    &format!("{ms:.0} ms"),
                    (ms / 2000.0).clamp(0.0, 1.0) as f32,
                ),
            ]
        }
        None => [
            QLinkCard::new("Gain", "REGION", "pick one", 0.0),
            QLinkCard::new("Fades", "REGION", "pick one", 0.0),
        ],
    };
    Some(AudioKnobs {
        track: t,
        region: b.and(index),
        cards: [level, pan, gain, fades],
    })
}

/// The rate the cards turn frames into milliseconds at (the app's).
const RATE_FOR_TEXT: f64 = crate::RATE as f64;

/// Region `r` as the tracks have it now.
fn region(
    tr: &mpfree_audiotracks::AudioTracks,
    r: RegionId,
) -> Result<mpfree_audiotracks::Region, String> {
    if r.track >= MAX_TRACKS {
        return Err(format!("there is no audio track {}", r.track + 1));
    }
    tr.regions(r.track)
        .get(r.index)
        .copied()
        .ok_or_else(|| "that region is gone; pick it again".to_string())
}

/// How fast a level bar falls back: the whole bar in 1.5 s.
const METER_FALL: f32 = 1.0 / 1.5;
/// The smallest move of a bar worth sending: a pixel of a 96 px header.
const METER_STEP: f32 = 0.01;
/// The bottom of a level bar, in dB: -48 dB and below show nothing.
const METER_FLOOR_DB: f32 = -48.0;

/// A peak (0 to 1 is full scale) as a level bar, 0 to 1: dB from
/// [`METER_FLOOR_DB`] to 0 dB.
fn meter(peak: f32) -> f32 {
    if peak <= 0.0 {
        return 0.0;
    }
    ((20.0 * peak.log10() - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

/// "s" for a count other than one.
fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Columns for a take's waveform: one for every block of its peak table,
/// at most [`MAX_COLUMNS`].
fn columns(k: &TakeInfo) -> usize {
    (k.frames * k.channels / PEAK_BLOCK).clamp(1, MAX_COLUMNS)
}

/// The comp grid: a sixteenth note (layout spec 4c; SHIFT is free).
const GRID: Tick = mpfree_transport::time::PPQ as Tick / 4;

/// A tick to the nearest grid line.
fn snap(t: Tick) -> Tick {
    (t + GRID / 2).div_euclid(GRID) * GRID
}

/// A swipe's stretch on the grid: each end to the nearest line, at least
/// one step long.
fn snap_span(from: Tick, to: Tick) -> (Tick, Tick) {
    let (a, b) = (snap(from.min(to)), snap(from.max(to)));
    (a, b.max(a + GRID))
}

/// "take 3": the track's takes count from 1, oldest first, as the lanes
/// name them.
fn take_name(tr: &mpfree_audiotracks::AudioTracks, take: u16) -> String {
    let Some(k) = tr.take(take) else {
        return "the take".into();
    };
    let mut mine: Vec<TakeInfo> = tr.takes().filter(|t| t.track == k.track).collect();
    mine.sort_by_key(|t| (t.group, t.pass, t.id));
    match mine.iter().position(|t| t.id == take) {
        Some(n) => format!("take {}", n + 1),
        None => "the take".into(),
    }
}

/// A sound from the Sample page for an audio track ([`TrackHost::bring`]):
/// `audio` interleaved, `channels` 1 or 2, at the tracks' rate, cut into
/// slices at its frames `cuts` (rising), and its tempo `bpm` (0: not
/// known), named `name`. With `cell` it goes in a cell, whole, as a clip
/// (Put in a cell, [`TrackHost::bring_to_cell`]), not on the song.
#[derive(PartialEq)]
pub struct Sound {
    pub name: String,
    pub channels: usize,
    pub audio: Vec<f32>,
    pub cuts: Vec<u64>,
    pub bpm: f64,
    pub cell: bool,
}

/// Not the samples: a sound is long, and actions are logged by name.
impl std::fmt::Debug for Sound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Sound({:?}, {} frames, {} ch, {} cuts)",
            self.name,
            self.audio.len() / self.channels.max(1),
            self.channels,
            self.cuts.len()
        )
    }
}

/// A song tick as "5.2" (bar and beat, from 1) in `t`'s meter.
fn bar_beat(tick: Tick, t: &TransportInfo) -> String {
    let bar = t.meter.bar_ticks().max(1);
    let beat = t.meter.beat_ticks().max(1);
    format!(
        "{}.{}",
        tick.div_euclid(bar) + 1,
        tick.rem_euclid(bar) / beat + 1
    )
}

/// What an input records, for the log.
pub(crate) fn input_words(i: Input) -> &'static str {
    match i {
        Input::Left => "the left input (mono)",
        Input::Right => "the right input (mono)",
        Input::Sum => "left and right summed (mono)",
        Input::Stereo => "both inputs (stereo)",
    }
}

/// Where a track records from inside the mix, as the project saves it:
/// the strip as the mixer's part writes it, then `:before` or `:after`
/// its effects, or `:fader` after its fader (`t12:after`, `b0:fader`).
fn inside_text(f: Inside) -> String {
    let when = match (f.after_fader, f.after_effects) {
        (true, _) => "fader",
        (false, true) => "after",
        (false, false) => "before",
    };
    format!("{}:{when}", crate::project::strip_text(f.strip))
}

/// [`inside_text`] read back; None for the line input (empty) or words
/// it doesn't know.
fn parse_inside(w: &str) -> Option<Inside> {
    let (strip, when) = w.split_once(':')?;
    let (after_effects, after_fader) = match when {
        "fader" => (true, true),
        "after" => (true, false),
        "before" => (false, false),
        _ => return None,
    };
    Some(Inside {
        strip: crate::project::parse_strip(strip)?,
        after_effects,
        after_fader,
    })
}

/// A monitor mode, for the log.
fn monitor_words(m: Monitor) -> String {
    let name = MONITORS.iter().find(|x| x.1 == m).map_or("?", |x| x.0);
    let what = match m {
        Monitor::Auto => "hear the input while armed, except during playback",
        Monitor::In => "always hear the input",
        Monitor::Off => "never hear the input",
    };
    format!("{name} ({what})")
}

/// FNV-1a over u64s.
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn add(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// The audio tracks as a song time edit sees them (the Arrange refactor's
/// 3.4): asked before the song changes, so their refusal changes nothing
/// anywhere, and sent the edit after, applied at the start of the next
/// period, the one the song's edit starts in.
impl crate::time::SongAudio for TrackHost {
    fn busy(&self) -> Option<String> {
        if self.opening() {
            return Some("the project's audio tracks are still going in".into());
        }
        // What the tracks say now must hold when the change applies.
        if self.host.tracks.busy() {
            return Some("the audio tracks are taking the last change; try again".into());
        }
        None
    }

    fn refused(&self, r: &Retime) -> Option<String> {
        if let Some(why) = self.busy() {
            return Some(why);
        }
        match self.host.tracks.retime_refused(r)? {
            RetimeRefused::Recording(t) => {
                Some(format!("{} is recording (stop it first)", self.who(t)))
            }
            RetimeRefused::Full(t) => Some(format!(
                "{} would be cut into more than {MAX_REGIONS} regions",
                self.who(t)
            )),
        }
    }

    fn moves(&self, r: &Retime) -> bool {
        let tr = &self.host.tracks;
        (0..MAX_TRACKS).any(|t| r.changes(tr.regions(t)))
    }

    fn retime(&mut self, song: mpfree_transport::TransactionId, r: Retime) -> bool {
        // There is room: asked first (refused).
        self.moves(&r) && self.send(Command::Retime { song, retime: r }).is_ok()
    }

    fn step(&self, song: mpfree_transport::TransactionId, undo: bool) -> SongStep {
        self.host.tracks.song_step(song, undo)
    }

    fn song_undo(&mut self, song: mpfree_transport::TransactionId, undo: bool) {
        let c = if undo {
            Command::SongUndo(song)
        } else {
            Command::SongRedo(song)
        };
        // There is room: asked first (busy).
        let _ = self.send(c);
    }

    fn regions(&self) -> Vec<(TrackId, Vec<mpfree_seq::ClipRegion>)> {
        let tr = &self.host.tracks;
        (0..MAX_TRACKS)
            .filter_map(|s| {
                let id = self.slots.id_of(s).filter(|_| self.shows(s))?;
                let regions: Vec<mpfree_seq::ClipRegion> = tr
                    .regions(s)
                    .iter()
                    .filter_map(|r| {
                        let take = tr.take(r.take)?;
                        let made_at = if take.varied { 0.0 } else { take.bpm };
                        Some(clip_region_of(r, RecId(take.rec.0), made_at))
                    })
                    .collect();
                (!regions.is_empty()).then_some((id, regions))
            })
            .collect()
    }
}

/// How full and writable the drive holding `dir` is, for System status
/// (`statvfs`, reads only; it may wait on a failing drive, so the audio
/// tracks call it on a thread of their own).
fn drive_room(dir: &std::path::Path) -> Option<mpfree_audiotracks::files::Room> {
    crate::sys::drive_room(dir).map(|(writable, free, total)| mpfree_audiotracks::files::Room {
        writable,
        free,
        total,
    })
}

/// Loops' closed layers' files (phase 4c.3) go through the audio tracks'
/// worker, to the folder takes record into.
impl crate::loops_on_tracks::LayerFiles for TrackHost {
    fn can_write(&self) -> bool {
        self.host.layer_files()
    }

    fn begin(&mut self, rec: u128, frames: usize) -> bool {
        self.host.layer_begin(RecordingId(rec), frames, 2)
    }

    fn buffer(&mut self) -> Option<Vec<f32>> {
        self.host.layer_buffer()
    }

    fn piece(&mut self, rec: u128, buf: Vec<f32>, len: usize) {
        self.host.layer_piece(RecordingId(rec), buf, len);
    }

    fn end(&mut self, rec: u128) {
        self.host.layer_end(RecordingId(rec));
    }

    fn drop_layer(&mut self, rec: u128) {
        self.host.layer_drop(RecordingId(rec));
    }

    fn written(&mut self) -> Option<(u128, Option<String>)> {
        self.host.layer_written().map(|(r, why)| (r.0, why))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_transport::{Command as T, State, Transport};
    use std::time::{Duration, Instant};

    /// The status line says each drive's read times that ran and the
    /// recording's syncs, for the device rounds' logs; nothing before any.
    #[test]
    fn the_status_line_says_each_drives_read_and_sync_times() {
        use mpfree_audiotracks::streaming::{Syncs, Timings};
        use mpfree_stream::coord::DriveTimes;
        use mpfree_stream::times::Summary;
        assert_eq!(timings_line(&Timings::default()), "");
        let times = Summary {
            count: 1234,
            median: 0.002,
            p99: 0.04,
            p999: 0.3,
            worst: 1.2,
        };
        let drive = |root: Option<&str>, count| DriveTimes {
            root: root.map(std::path::PathBuf::from),
            reads: Summary { count, ..times },
            queue: 2,
            speed: 1e7,
        };
        let t = Timings {
            reads: vec![drive(None, 0), drive(Some("/run/mpfree-sd"), 1234)],
            syncs: Some(Syncs {
                dir: "/run/mpfree-sd/MPFree/.work/audiotracks".into(),
                on_card: true,
                times,
            }),
        };
        let all = "median 2.0 ms, 1 in 100 40 ms, 1 in 1,000 300 ms, worst 1.20 s, of 1234";
        assert_eq!(
            timings_line(&t),
            format!("; card reads: {all}, 2 waiting; syncs on the card: {all}")
        );
    }

    /// A take's problem names the project track its engine track plays,
    /// by its lasting id, so System status can name it; with no track, or
    /// none playing there, it is about the whole system.
    #[test]
    fn a_take_s_problem_is_about_its_project_track() {
        use mpfree_audiotracks::files::Problem;
        use mpfree_status::{kinds, Report, Subject};
        let mut slots = Slots::default();
        slots.0[2] = TrackId(7);
        let at_risk = Report::began(&kinds::RECORDING_AT_RISK);
        let named = |track| {
            slots.name_problem(Problem {
                report: at_risk,
                track,
            })
        };
        assert_eq!(named(Some(2)).subject, Subject::audio_track(7));
        assert_eq!(named(Some(3)).subject, Subject::NONE);
        assert_eq!(named(None).subject, Subject::NONE);
        assert_eq!(named(Some(MAX_TRACKS + 4)).subject, Subject::NONE);
        assert_eq!(named(Some(2)).kind, &kinds::RECORDING_AT_RISK);
    }

    /// The drive's room as statvfs gives it: a folder that is there is
    /// on a drive that answers, one that isn't answers nothing.
    #[test]
    fn the_drive_s_room_is_read_for_a_folder_there_and_none_for_one_gone() {
        let here = std::env::temp_dir();
        let room = drive_room(&here).expect("the temp folder's drive answers");
        assert!(room.free > 0);
        assert!(room.total >= room.free, "{room:?}");
        assert!(drive_room(&here.join("mpfree-no-such-folder-here")).is_none());
    }

    const RATE: u32 = 44_100;

    struct Rig {
        t: Transport,
        b: Block,
        h: TrackHost,
        /// The project's track list: two note tracks, then the audio
        /// tracks [`Rig::add`] adds.
        project: mpfree_seq::Project,
        song: bool,
        frame: u64,
        /// The line input's tone on while true.
        live: bool,
        heard: f32,
    }

    impl Rig {
        fn new() -> Rig {
            let mut project = mpfree_seq::Project::new();
            for i in 0..2 {
                project
                    .tracks
                    .push(mpfree_seq::Track::new(&format!("Track {}", i + 1), i));
            }
            project.give_track_ids();
            Rig {
                t: Transport::new(RATE, 120.0).unwrap(),
                b: Block::new(),
                h: TrackHost::new(RATE, 10),
                project,
                song: true,
                frame: 0,
                live: false,
                heard: 0.0,
            }
        }

        /// A rig with `n` audio tracks in its track list, engine tracks 0
        /// to `n` - 1.
        fn with(n: usize) -> Rig {
            let mut r = Rig::new();
            for s in 0..n {
                assert_eq!(r.add(), s);
            }
            r
        }

        /// An audio track at the end of the track list, as + Track adds
        /// it: its engine track.
        fn add(&mut self) -> usize {
            let n = self.project.tracks.iter().filter(|t| t.is_audio()).count();
            let mut track = mpfree_seq::Track::new_audio(&format!("Audio {}", n + 1), 0);
            self.project.give_track_id(&mut track);
            let id = track.id;
            self.project.tracks.push(track);
            self.sync();
            self.h.slots.slot_of(id).expect("an engine track")
        }

        /// The tracks silent now, a bit a track (`TimeHost::silenced`).
        fn silenced(&self) -> u128 {
            (0..self.project.tracks.len())
                .filter(|&i| self.project.silent(i))
                .fold(0, |b, i| b | 1 << i)
        }

        fn sync(&mut self) {
            let silenced = self.silenced();
            self.h.sync(&self.project, silenced, true);
        }

        /// Engine track `s`'s project track muted or soloed, as TRACK MUTE
        /// or the track panel does it.
        fn mute(&mut self, s: usize, solo: bool, on: bool) {
            let i = self.h.place(s).expect("a project track");
            let t = &mut self.project.tracks[i];
            if solo {
                t.solo = on;
            } else {
                t.mute = on;
            }
            self.sync();
        }

        fn period(&mut self) {
            let silenced = self.silenced();
            self.h.sync(&self.project, silenced, false);
            let mut l = [0.0f32; PERIOD];
            if self.live {
                for (j, x) in l.iter_mut().enumerate() {
                    let g = self.frame + j as u64;
                    *x = 0.5 * (std::f32::consts::TAU * 441.0 * g as f32 / RATE as f32).sin();
                }
            }
            let r = [0.0f32; PERIOD];
            self.t.process(PERIOD as u32, &mut self.b);
            let map = *self.t.tempo_map();
            self.h.period(&self.b, &map, self.song, (&l, &r), None);
            // What the tracks played, each into its own channel.
            self.heard = (0..MAX_TRACKS)
                .flat_map(|s| self.h.host.tracks.track_out(s, PERIOD).iter())
                .fold(0.0, |m, s| m.max(s.abs()));
            self.frame += PERIOD as u64;
        }

        fn periods(&mut self, n: usize) {
            for _ in 0..n {
                self.period();
            }
        }

        fn command(&mut self, c: T) {
            assert!(self.t.command(self.frame, c));
        }

        fn info(&self) -> TransportInfo {
            TransportInfo {
                position: self.t.position(),
                ..Default::default()
            }
        }

        fn state(&self) -> TimeState {
            let mut st = TimeState::from_engine(
                TransportInfo::default(),
                &self.project,
                &[],
                None,
                &mpfree_play::Takes::new(4),
            );
            self.h.fill(&mut st, self.t.tempo_map());
            self.h
                .fill_blocks(&mut st, &self.project, self.t.tempo_map());
            st
        }

        /// Run until every take has its waveform.
        fn waves(&mut self) {
            let t0 = Instant::now();
            while self.h.waves.len() < self.h.host.tracks.takes().count()
                || self.h.reading.is_some()
            {
                assert!(t0.elapsed() < Duration::from_secs(20), "no waveform");
                self.period();
                std::thread::sleep(Duration::from_micros(100));
            }
        }
    }

    /// Rec song's blocks over regions (phase 2, step 2d, Joshi 23 Nov:
    /// replace): the regions under them go as steps of the song edit,
    /// which the audio tracks' own UNDO stops at and the song's UNDO and
    /// REDO take back and bring back. A span over no region, or on a
    /// track that isn't an audio track's, erases nothing.
    #[test]
    fn rec_songs_blocks_replace_the_regions_under_them_as_a_song_step() {
        let mut r = Rig::with(1);
        r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), false)
            .unwrap();
        r.periods(CHECK_EVERY as usize);
        r.live = true;
        r.command(T::Record);
        while r.t.position() < 2 * 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.live = false;
        r.periods(20);
        let tr = |r: &Rig| -> Vec<(Tick, Tick)> {
            r.h.host
                .tracks
                .regions(0)
                .iter()
                .map(|x| (x.start, x.end))
                .collect()
        };
        let before = tr(&r);
        assert_eq!(before.len(), 1);
        let (start, end) = (before[0].0, before[0].1);
        let id = r.h.slots.id_of(0).unwrap();
        let other = mpfree_seq::TrackId(id.0 + 100);
        let spans = [
            (id, 3840, 4 * 3840),
            (id, 10 * 3840, 11 * 3840),
            (other, 0, 3840),
        ];
        assert_eq!(
            r.h.replace_under(mpfree_transport::TransactionId(7), &spans),
            (1, 0),
            "one span over a region"
        );
        r.periods(2);
        assert_eq!(tr(&r), [(start, 3840)], "replaced from bar 2 on");
        assert_eq!(
            r.h.host.tracks.history().0,
            2,
            "the take, then the song's step"
        );
        // The audio tracks' UNDO stops at it; the song's takes it back.
        let refused = r.h.host.tracks.stats().refused;
        assert!(r.h.host.command(Command::Undo));
        r.periods(2);
        assert_eq!(r.h.host.tracks.stats().refused, refused + 1);
        assert_eq!(tr(&r), [(start, 3840)]);
        crate::time::SongAudio::song_undo(&mut r.h, mpfree_transport::TransactionId(7), true);
        r.periods(2);
        assert_eq!(tr(&r), [(start, end)], "back as recorded");
        crate::time::SongAudio::song_undo(&mut r.h, mpfree_transport::TransactionId(7), false);
        r.periods(2);
        assert_eq!(tr(&r), [(start, 3840)], "and replaced again");
    }

    /// A block of an audio clip on the Song view (phase 2, step 2d): its
    /// lane has where its clip's passes start, a pass's length, and the
    /// take each region plays with the frames it spans; a block whose
    /// clip is gone has none.
    #[test]
    fn an_audio_clips_block_gives_its_lane_its_wave() {
        let mut r = Rig::with(1);
        r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), false)
            .unwrap();
        r.periods(CHECK_EVERY as usize);
        r.live = true;
        r.command(T::Record);
        while r.t.position() < 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.live = false;
        r.periods(20);
        let k = r.h.host.tracks.takes().next().expect("a take");
        let mut riff = mpfree_seq::Clip::new("Riff", 3840);
        riff.audio = Some(mpfree_seq::AudioClip::of(
            RecId(k.rec.0),
            0,
            k.frames as i64,
            3840,
            120.0,
        ));
        let guitar = r.h.place(0).unwrap();
        let id = r.project.put_new_clip(guitar, 0, riff).unwrap();
        let song = &mut r.project.arrangement;
        // Two passes from bar 3.
        song.placements
            .push(mpfree_seq::Placement::new(guitar, id, 2 * 3840, 2 * 3840));
        let st = r.state();
        assert_eq!(st.audio[0].row, guitar);
        let clips = &st.audio[0].clips;
        assert_eq!(clips.len(), 1);
        let c = &clips[0];
        assert_eq!(
            (c.index, c.origin, c.pass, c.once),
            (0, 2 * 3840, 3840, false)
        );
        assert_eq!(c.pieces.len(), 1);
        let p = c.pieces[0];
        assert_eq!((p.start, p.end, p.take, p.src), (0, 3840, k.id, 0));
        // A bar at 120 BPM: two seconds.
        assert!((p.frames - 2.0 * RATE as f64).abs() < 1.0, "{}", p.frames);
        // Its clip gone: a block with no wave and no header.
        r.project.tracks[guitar].slots[0] = None;
        assert!(r.state().audio[0].clips.is_empty());
    }

    /// The scheduler's promise is the shared memory check's (5b.3): a job
    /// that wouldn't leave the floor free is refused with why, and one
    /// that starts holds its bytes in the same account as every load
    /// until it ends.
    #[test]
    fn the_stretch_schedulers_promise_is_the_shared_memory_check() {
        let c = stretch_config();
        let why = (c.promise)(u64::MAX / 4).unwrap_err();
        assert!(why.starts_with(crate::memory::SHORT), "{why}");
        let before = crate::memory::promised();
        let hold = (c.promise)(1000).expect("a small promise");
        assert_eq!(crate::memory::promised(), before + 1000);
        drop(hold);
        assert_eq!(crate::memory::promised(), before);
    }

    /// A clip on Stretch in the project (phase 5b.2): the tracks are told
    /// of it, and its copy for the tempo playing is made ahead on the
    /// scheduler, so it plays at its own pitch whenever it is launched.
    #[test]
    fn a_clip_on_stretch_gets_its_copy_made_ahead_at_the_tempo_playing() {
        let mut r = Rig::with(1);
        r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), false)
            .unwrap();
        r.periods(CHECK_EVERY as usize);
        r.live = true;
        r.command(T::Record);
        while r.t.position() < 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.live = false;
        r.periods(20);
        let k = r.h.host.tracks.takes().next().expect("a take");
        let mut audio = mpfree_seq::AudioClip::of(RecId(k.rec.0), 0, k.frames as i64, 3840, 120.0);
        audio.regions[0].follow = mpfree_seq::FollowTempo::Stretch;
        let mut riff = mpfree_seq::Clip::new("Riff", 3840);
        riff.audio = Some(audio);
        let guitar = r.h.place(0).unwrap();
        r.project.put_new_clip(guitar, 0, riff).unwrap();
        r.h.clip_copies(&r.project);
        r.command(T::TempoNow(90.0));
        let t0 = Instant::now();
        while r.h.host.copy_memory().1 == 0 {
            let c = r.h.host.copy_counts();
            assert!(t0.elapsed() < Duration::from_secs(30), "{c:?}");
            r.period();
            std::thread::sleep(Duration::from_micros(200));
        }
        let v: Vec<_> = r.h.host.tracks.copies().collect();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].key.rec, k.rec);
        assert_eq!(f64::from_bits(v[0].key.tempo), 90.0);
    }

    /// System status' Memory (5c.4): the time state says how many tempo
    /// copies the tracks hold, play and are making, and whose; Clear
    /// copies lets go of a clip's copy nothing plays or is about to play,
    /// and says so; it is made again.
    #[test]
    fn system_status_hears_of_the_tracks_copies_and_clear_copies_lets_a_clips_go() {
        let mut r = Rig::with(1);
        r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), false)
            .unwrap();
        r.periods(CHECK_EVERY as usize);
        r.live = true;
        r.command(T::Record);
        while r.t.position() < 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.live = false;
        r.periods(20);
        let k = r.h.host.tracks.takes().next().expect("a take");
        let mut audio = mpfree_seq::AudioClip::of(RecId(k.rec.0), 0, k.frames as i64, 3840, 120.0);
        audio.regions[0].follow = mpfree_seq::FollowTempo::Stretch;
        let mut riff = mpfree_seq::Clip::new("Riff", 3840);
        riff.audio = Some(audio);
        let guitar = r.h.place(0).unwrap();
        r.project.put_new_clip(guitar, 0, riff).unwrap();
        r.h.clip_copies(&r.project);
        r.command(T::TempoNow(90.0));
        assert_eq!(r.state().audio_copies, Default::default(), "none yet");
        let (t0, mut whose) = (Instant::now(), None);
        while r.h.host.copy_memory().1 == 0 {
            assert!(t0.elapsed() < Duration::from_secs(30));
            let c = r.state().audio_copies;
            if c.making > 0 {
                whose = Some(c.whose);
            }
            r.period();
            std::thread::sleep(Duration::from_micros(200));
        }
        assert_eq!(
            whose,
            Some(vec!["Audio 1".to_string()]),
            "whose, while made"
        );
        let c = r.state().audio_copies;
        assert_eq!((c.held, c.playing, c.making), (1, 0, 0));
        // The clip is only listed, not cued: its copy goes.
        assert_eq!(r.h.clear_copies(), 1);
        assert_eq!(r.state().audio_copies.held, 0);
        assert_eq!(
            cleared_line(1, 2),
            "3 copies cleared: each is made again when it's next needed"
        );
        assert_eq!(
            cleared_line(0, 0),
            "Nothing to clear: every copy is playing or about to"
        );
        let t0 = Instant::now();
        while r.h.host.copy_memory().1 == 0 {
            assert!(t0.elapsed() < Duration::from_secs(30), "made again");
            r.period();
            std::thread::sleep(Duration::from_micros(200));
        }
    }

    /// A launch waiting for its tempo copy (phase 5b.4): the host makes it
    /// at once, at the tempo at the cue's line, though the clip was never
    /// listed ahead, and the time engine is told it is ready once it is,
    /// and only then.
    #[test]
    fn a_launch_waiting_for_its_copy_gets_it_made_at_once_and_is_told_when_ready() {
        use crate::time::{CopyNeed, CueKey};
        let mut r = Rig::with(1);
        r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), false)
            .unwrap();
        r.periods(CHECK_EVERY as usize);
        r.live = true;
        r.command(T::Record);
        while r.t.position() < 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.live = false;
        r.periods(20);
        let k = r.h.host.tracks.takes().next().expect("a take");
        let mut audio = mpfree_seq::AudioClip::of(RecId(k.rec.0), 0, k.frames as i64, 3840, 120.0);
        audio.regions[0].follow = mpfree_seq::FollowTempo::Stretch;
        let mut riff = mpfree_seq::Clip::new("Riff", 3840);
        riff.audio = Some(audio);
        let guitar = r.h.place(0).unwrap();
        r.project.put_new_clip(guitar, 0, riff).unwrap();
        let key = CueKey::test(7);
        let need = CopyNeed {
            cue: key,
            track: guitar,
            slot: 0,
            region: 0,
            bpm: 90.0,
        };
        r.h.hold_copies(&[need], &r.project);
        let mut told = Vec::new();
        r.h.copy_answers(|k, ok| told.push((k, ok)));
        assert_eq!(told, vec![(key, false)], "not ready yet");
        r.command(T::TempoNow(90.0));
        let t0 = Instant::now();
        while r.h.host.copy_memory().1 == 0 {
            let c = r.h.host.copy_counts();
            assert!(t0.elapsed() < Duration::from_secs(30), "{c:?}");
            r.period();
            r.h.copy_answers(|k, ok| told.push((k, ok)));
            std::thread::sleep(Duration::from_micros(200));
        }
        r.h.copy_answers(|k, ok| told.push((k, ok)));
        assert_eq!(
            told,
            vec![(key, false), (key, true)],
            "told once, when ready"
        );
        let v: Vec<_> = r.h.host.tracks.copies().collect();
        assert_eq!(f64::from_bits(v[0].key.tempo), 90.0);
        // Its cue gone, nothing more is said of it.
        r.h.hold_copies(&[], &r.project);
        r.h.copy_answers(|k, ok| told.push((k, ok)));
        assert_eq!(told.len(), 2);
    }

    /// A loop cell's block (phase 4e.3, board 8): its lane draws it as a
    /// clip block naming its loop, its passes counted from its start less
    /// its offset, with no pieces (the screen has the loop's wave from
    /// the looper).
    #[test]
    fn a_loop_cells_block_names_its_loop_and_where_its_passes_start() {
        let mut r = Rig::with(1);
        let audio = r.h.place(0).unwrap();
        let id = mpfree_seq::LoopId(0xfeed);
        let clip = mpfree_seq::Clip::new_audio_loop(id, "Vox loop", 3 * 3840);
        let cid = r.project.put_new_clip(audio, 0, clip).unwrap();
        // From bar 6, a bar into its loop.
        let mut p = mpfree_seq::Placement::new(audio, cid, 5 * 3840, 4 * 3840);
        p.offset = 3840;
        r.project.arrangement.placements.push(p);
        let st = r.state();
        assert_eq!(st.audio[0].clips.len(), 1);
        let c = &st.audio[0].clips[0];
        assert_eq!(
            (c.index, c.origin, c.pass, c.once, c.looped),
            (0, 4 * 3840, 3 * 3840, false, Some(0xfeed))
        );
        assert!(c.pieces.is_empty());
    }

    /// Takes the project's audio clips name stay (the audio tracks
    /// rethink, phase 2): kept from the next look, so a Forget is refused
    /// with no region or undo step on them; let go once nothing names
    /// them, and forgotten then.
    #[test]
    fn takes_named_by_clips_are_kept_until_none_names_them() {
        let mut r = Rig::with(1);
        r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), false)
            .unwrap();
        r.periods(CHECK_EVERY as usize);
        r.live = true;
        r.command(T::Record);
        while r.t.position() < 960 {
            r.period();
        }
        r.command(T::Stop);
        r.live = false;
        r.periods(20);
        let k = r.h.host.tracks.takes().next().expect("a take");
        // Only a clip names it: its region erased, the history forgotten.
        assert!(r.h.host.command(Command::Erase {
            track: 0,
            from: 0,
            to: 100 * 3840,
        }));
        assert!(r.h.host.command(Command::ClearHistory));
        r.h.keep_named(vec![RecId(k.rec.0)]);
        r.periods(2 * CHECK_EVERY as usize);
        assert!(r.h.host.tracks.kept(k.id));
        assert!(r.h.host.command(Command::Forget(k.id)));
        r.periods(2);
        assert!(r.h.host.tracks.take(k.id).is_some(), "a kept take stays");
        r.h.keep_named(Vec::new());
        r.periods(2 * CHECK_EVERY as usize);
        assert!(r.h.host.tracks.take(k.id).is_none(), "let go: forgotten");
    }

    #[test]
    fn an_armed_track_records_the_line_input_in_song_mode_and_shows_its_lane() {
        let mut r = Rig::new();
        assert!(r.state().audio.is_empty(), "no lane before one is added");
        assert_eq!(r.state().audio_free, None);
        // An audio track in the track list has a lane; arming it with the
        // line input off says where to turn it on.
        assert_eq!(r.add(), 0);
        assert_eq!(r.h.place(0), Some(2), "after the two note tracks");
        let line =
            r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), false)
                .unwrap();
        assert!(line.contains("Prefs > Line input"), "{line}");
        r.periods(CHECK_EVERY as usize);
        assert!(r.h.take_changed());
        let st = r.state();
        assert_eq!(st.audio.len(), 1);
        assert!(st.audio[0].armed);
        assert_eq!(st.audio[0].input, Input::Left);
        assert_eq!(st.audio[0].name, "Audio 1");
        let (free, all) = st.audio_free.unwrap();
        assert!((all - 10.0).abs() < 0.5 && free <= all, "{free} of {all}");
        // Record two bars (4 s at 120 BPM), then stop.
        r.live = true;
        r.command(T::Record);
        r.periods(10);
        assert_eq!(r.t.state(), State::Recording);
        assert!(
            r.state().audio[0].recording.is_some(),
            "the red block grows"
        );
        while r.t.position() < 2 * 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.live = false;
        r.periods(20);
        let st = r.state();
        assert_eq!(st.audio[0].recording, None);
        assert_eq!(st.audio[0].blocks.len(), 1);
        let b = &st.audio[0].blocks[0];
        assert!(b.end - b.start > 3840, "{b:?}");
        r.waves();
        let st = r.state();
        assert_eq!(st.waves.len(), 1);
        assert!(
            st.waves[0].columns.iter().any(|c| c.1 > 0.4),
            "the tone is in the wave"
        );
        // It plays back from the start in song mode.
        r.command(T::Locate(0));
        r.command(T::Play);
        r.periods(200);
        assert!(r.heard > 0.3, "plays back: {}", r.heard);
        // Out of song mode the tracks are silent.
        r.song = false;
        r.periods(10);
        assert_eq!(r.heard, 0.0, "silent out of song mode");
        r.command(T::Stop);
        r.song = true;
        r.periods(4);
    }

    /// Phase 1a: an audio track taken out of the track list (Undo of the
    /// add) keeps its engine track while that holds a recording, silent,
    /// and gets it back when the track comes back; an armed one is
    /// disarmed, so REC records no track that isn't there; an empty one
    /// is freed for the next audio track, with a new track's settings.
    /// One added from "+ Track" is armed.
    #[test]
    fn an_engine_track_follows_its_track_away_and_back() {
        let mut r = Rig::with(2);
        r.periods(2);
        let (a, b) = (r.project.tracks[2].clone(), r.project.tracks[3].clone());
        assert_eq!(
            (r.h.slots.slot_of(a.id), r.h.slots.slot_of(b.id)),
            (Some(0), Some(1))
        );
        // Audio 2 holds a sound; Audio 1 is armed and never heard through.
        r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), true)
            .unwrap();
        let off = AudioAction::Monitor {
            track: 0,
            monitor: Monitor::Off,
        };
        r.h.action(off, &r.info(), true).unwrap();
        r.periods(2);
        r.h.bring(
            Sound {
                name: "Hit".into(),
                channels: 1,
                audio: vec![0.25; 4410],
                cuts: Vec::new(),
                bpm: 120.0,
                cell: false,
            },
            &TransportInfo::default(),
        )
        .unwrap();
        let t0 = Instant::now();
        while r.h.host.importing().is_some() {
            assert!(t0.elapsed() < Duration::from_secs(20), "never placed");
            r.period();
        }
        assert_eq!(r.h.host.tracks.regions(1).len(), 1, "on Audio 2");
        assert_eq!(r.h.host.tracks.settings(0).unwrap().monitor, Monitor::Off);
        // Audio 2 goes: its engine track stays its, silent, with no lane.
        r.project.tracks.pop();
        r.sync();
        r.periods(1);
        assert_eq!(r.h.slots.id_of(1), Some(b.id));
        assert_eq!(r.h.place(1), None);
        assert!(r.h.silent[1], "a track that's gone is silent");
        assert_eq!(r.state().audio.len(), 1);
        // It comes back (Redo): the same engine track, heard again.
        r.project.tracks.push(b.clone());
        r.sync();
        r.periods(1);
        assert_eq!(r.h.place(1), Some(3));
        assert!(!r.h.silent[1]);
        assert_eq!(r.state().audio.len(), 2);
        // Audio 1 goes: disarmed, then free, and the next audio track,
        // added from "+ Track", gets it as new, armed.
        r.project.tracks.remove(2);
        r.sync();
        r.periods(1);
        assert!(!r.h.host.tracks.armed(0), "disarmed");
        r.sync();
        assert_eq!(r.h.slots.id_of(0), None);
        assert_eq!(r.h.free_slot(), Some(0));
        r.h.arm_next();
        assert_eq!(r.add(), 0);
        r.periods(2);
        let new = r.h.host.tracks.settings(0).unwrap();
        assert_eq!(
            (new.level, new.monitor),
            (1.0, Monitor::Auto),
            "a new track's"
        );
        assert!(r.h.host.tracks.armed(0), "armed, ready to record");
        // Without "+ Track" (the Sample page's), not armed.
        assert_eq!(r.add(), 2);
        r.periods(2);
        assert!(!r.h.host.tracks.armed(2));
    }

    /// Phase 1b: each audio track plays into its own mixer channel (and
    /// only there), counts as a channel in use, and the looper's pads
    /// leave it out; its strip's fader and pan show back in the track
    /// (the panel, the lanes, the cards); and an export hears it from its
    /// strip, by its name, while it has something on it.
    #[test]
    fn each_audio_track_plays_into_its_own_mixer_channel() {
        use mpfree_fx::StripId;
        let mut r = Rig::with(2);
        let mut mix = MixHost::new(RATE, -6.0, -6.0);
        r.h.bring(
            Sound {
                name: "Hit".into(),
                channels: 1,
                audio: vec![0.25; 44_100],
                cuts: Vec::new(),
                bpm: 120.0,
                cell: false,
            },
            &TransportInfo::default(),
        )
        .unwrap();
        let t0 = Instant::now();
        while r.h.host.importing().is_some() {
            assert!(t0.elapsed() < Duration::from_secs(20), "never placed");
            r.period();
        }
        let s = (0..2)
            .find(|&s| !r.h.host.tracks.regions(s).is_empty())
            .unwrap();
        let other = 1 - s;
        let (p, q) = (r.h.place(s).unwrap(), r.h.place(other).unwrap());
        let (ch, quiet) = (mix.track_channel(p), mix.track_channel(q));
        assert_ne!(ch, quiet);
        mix.mixer.set_now(StripId::Track(ch), mx::VOLUME, -6.0);
        mix.mixer.set_now(StripId::Track(ch), mx::PAN, 50.0);
        r.command(T::Play);
        let peak = |mix: &mut MixHost, c: usize| {
            let (l, rr) = mix.mixer.track_input(c).unwrap();
            l.iter().chain(rr.iter()).fold(0f32, |m, x| m.max(x.abs()))
        };
        let (mut own, mut elsewhere) = (0f32, 0f32);
        for _ in 0..20 {
            r.period();
            r.h.play_into(&mut mix);
            own = own.max(peak(&mut mix, ch));
            elsewhere = elsewhere
                .max(peak(&mut mix, quiet))
                .max(peak(&mut mix, crate::mixing::AUDIO));
            let z = [0f32; PERIOD];
            let (mut l, mut rr) = (z, z);
            let ctx = mpfree_node::Ctx::new(RATE, r.frame);
            mix.period(&ctx, (&z, &z), (&z, &z), (&z, &z), &mut l, &mut rr);
        }
        assert!(own > 0.2, "{own}");
        assert_eq!(elsewhere, 0.0, "only its own channel");
        assert_eq!(mix.audio_tracks, 1 << p | 1 << q);
        assert_eq!(mix.channels_in_use() & (1 << p), 1 << p);
        // The strip's fader and pan, as the panel and cards show them.
        assert!((to_db(r.h.volume[s]) + 6.0).abs() < 1e-3);
        assert_eq!(r.h.pan[s], 0.5);
        let heard = r.h.heard(&r.project, r.silenced(), &mix);
        let name = r.project.tracks[p].name.clone();
        assert_eq!(heard, [(p, StripId::Track(ch), name)]);
        // Muted (silenced): no stem.
        r.mute(s, false, true);
        assert!(r.h.heard(&r.project, r.silenced(), &mix).is_empty());
    }

    /// Step 9's panel: Mix picks what the mix offers (the one picked last
    /// on the way back), From steps round it, Effects turns before or
    /// after, an input chip goes back to the line input, nothing changes
    /// while the track records, and the project keeps it, a track with
    /// only that set too.
    /// The audio tracks rethink's phase 4, board 4: the input list offers
    /// the line's picks, the pads' instrument, each strip but the track's
    /// own and the pads' (after its fader) and the master (before its
    /// effects), dim where the mixer refuses them with its reason in the
    /// track's words; a pick sets the input, a dim one is refused, and the
    /// line goes back to the line input.
    #[test]
    fn the_input_list_offers_the_mix_and_says_what_it_refuses() {
        use mpfree_fx::{CapturePoint, CaptureRefusal, StripId};
        use mpfree_time_ui::inputs::{LINE, MASTER, PADS, STRIP};
        let mut r = Rig::new();
        let act = |r: &mut Rig, a| r.h.action(a, &r.info(), true);
        for s in 0..2 {
            assert_eq!(r.add(), s);
        }
        let own = StripId::Track(40);
        let src = |name: &str, strip, owner| InputSource {
            strip,
            name: name.to_string(),
            owner,
        };
        let sources = vec![
            src("Drums", StripId::Track(0), Some(0)),
            src("Bass", StripId::Track(31), Some(1)),
            src("Bus A", StripId::Bus(0), None),
            src("Bus B", StripId::Bus(1), None),
            src("Master", StripId::Master, None),
        ];
        // Audio 2 plays into Bus A, which plays into the master.
        let refusal = |from: (StripId, CapturePoint)| match from {
            (StripId::Bus(0), _) => Some(CaptureRefusal::PastItsWayIn),
            (StripId::Master, _) => Some(CaptureRefusal::Through(StripId::Bus(0))),
            _ => None,
        };
        let pads = Some(src("Kit 1", StripId::Track(0), Some(0)));
        let c =
            r.h.input_choices(1, &sources, pads.clone(), own, refusal)
                .unwrap();
        assert_eq!((c.track, c.name.as_str()), (1, "Audio 2"));
        assert_eq!((c.line, c.picked), (Some(Input::Left), None));
        assert_eq!(
            c.pads.as_ref().map(|p| p.name.as_str()),
            Some("Pads · Kit 1")
        );
        assert_eq!(c.pads_why, "");
        let names: Vec<&str> = c.strips.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(
            names,
            ["Bass", "Bus A", "Bus B"],
            "not the pads', not the master"
        );
        assert_eq!(c.strips[0].owner, Some(1));
        assert_eq!(
            c.strips[1].refused.as_deref(),
            Some("Audio 2 plays into Bus A, so it would record itself")
        );
        assert_eq!(
            c.strips_why,
            "Bus A is dim: Audio 2 plays into it. Pick a strip Audio 2 doesn't feed."
        );
        assert_eq!(
            c.master_why,
            "Audio 2 plays into Master through Bus A: pick another input, or send \
             Audio 2 straight to Master."
        );
        assert!(c.master.refused.is_some());
        // A pick: after the strip's fader, as the list says.
        let line = act(&mut r, AudioAction::InputPick { track: 1, k: STRIP }).unwrap();
        assert_eq!(line, "Audio 2 records Bass, after its fader");
        let (f, name) = r.h.inside(1).unwrap();
        assert_eq!(
            (f.strip, f.point(), name),
            (StripId::Track(31), CapturePoint::AfterFader, "Bass")
        );
        // The same again: nothing to say.
        let line = act(&mut r, AudioAction::InputPick { track: 1, k: STRIP }).unwrap();
        assert_eq!(line, "");
        // A dim one is refused with the mixer's reason; the master too.
        let e = act(
            &mut r,
            AudioAction::InputPick {
                track: 1,
                k: STRIP + 1,
            },
        )
        .unwrap_err();
        assert!(e.contains("plays into Bus A"), "{e}");
        let e = act(
            &mut r,
            AudioAction::InputPick {
                track: 1,
                k: MASTER,
            },
        )
        .unwrap_err();
        assert!(e.contains("through Bus A"), "{e}");
        // The pads: named so, and an offer of the mix keeps the name.
        let line = act(&mut r, AudioAction::InputPick { track: 1, k: PADS }).unwrap();
        assert_eq!(line, "Audio 2 records Pads · Kit 1, after its fader");
        r.h.offer(vec![("Drums".into(), StripId::Track(0))]);
        assert_eq!(r.h.inside(1).unwrap().1, "Pads · Kit 1");
        // Offered again, the list knows what is picked.
        let c = r.h.input_choices(1, &sources, pads, own, refusal).unwrap();
        assert_eq!((c.line, c.picked), (None, Some(PADS)));
        // The pads on MIDI out: no Pads, and why.
        let c = r.h.input_choices(1, &sources, None, own, |_| None).unwrap();
        assert!(
            c.pads.is_none() && c.pads_why.contains("MIDI out"),
            "{}",
            c.pads_why
        );
        assert_eq!(c.strips.len(), 4, "Drums is a strip again");
        assert_eq!((c.strips_why.as_str(), c.master_why.as_str()), ("", ""));
        // A line pick goes back to the line input.
        let line = act(
            &mut r,
            AudioAction::InputPick {
                track: 1,
                k: LINE + 3,
            },
        )
        .unwrap();
        assert!(line.contains("both inputs (stereo)"), "{line}");
        assert!(r.h.inside(1).is_none());
        // Another track's pick needs its own list.
        let e = act(&mut r, AudioAction::InputPick { track: 0, k: STRIP }).unwrap_err();
        assert!(e.contains("open it again"), "{e}");
    }

    #[test]
    fn mix_and_from_pick_what_the_mix_offers_and_the_project_keeps_it() {
        use mpfree_fx::StripId;
        let mut r = Rig::new();
        let act = |r: &mut Rig, a| r.h.action(a, &r.info(), true);
        for s in 0..3 {
            assert_eq!(r.add(), s);
        }
        let e = act(&mut r, AudioAction::Mix { track: 0, step: 0 }).unwrap_err();
        assert!(e.contains("offers nothing"), "{e}");
        r.h.offer(vec![
            ("Drums".into(), StripId::Track(0)),
            ("FM · Pads".into(), StripId::Track(200)),
            ("Master".into(), StripId::Master),
        ]);
        let e = act(
            &mut r,
            AudioAction::MixEffects {
                track: 2,
                after: false,
            },
        )
        .unwrap_err();
        assert!(e.contains("pick Mix first"), "{e}");
        let line = act(&mut r, AudioAction::Mix { track: 2, step: 0 }).unwrap();
        assert_eq!(line, "Audio 3 records Drums, after its effects");
        let line = act(&mut r, AudioAction::Mix { track: 2, step: -1 }).unwrap();
        assert_eq!(
            line, "Audio 3 records Master, after its effects",
            "round the list"
        );
        let line = act(&mut r, AudioAction::Mix { track: 2, step: 2 }).unwrap();
        assert!(line.contains("FM · Pads"), "{line}");
        let line = act(
            &mut r,
            AudioAction::MixEffects {
                track: 2,
                after: false,
            },
        )
        .unwrap();
        assert_eq!(line, "Audio 3 records FM · Pads, before its effects");
        let (f, name) = r.h.inside(2).unwrap();
        assert_eq!(
            (f.strip, f.after_effects, name),
            (StripId::Track(200), false, "FM · Pads")
        );
        // The name follows the strip when the mix is offered again.
        r.h.offer(vec![("Poly · Pads".into(), StripId::Track(200))]);
        assert_eq!(r.h.inside(2).unwrap().1, "Poly · Pads");
        let st = r.state();
        let m = st.audio[2].mix.clone().unwrap();
        assert_eq!((m.name.as_str(), m.after_effects), ("Poly · Pads", false));
        let line = act(&mut r, AudioAction::Arm { track: 2, on: true }).unwrap();
        assert_eq!(line, "Audio 3 armed: REC records Poly · Pads in song mode");
        // The project keeps it: a track with only that set is saved too.
        assert!(r.h.host.save());
        let mut saved = None;
        for _ in 0..4000 {
            r.period();
            saved = r.h.take_saved(&r.project);
            if saved.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_micros(50));
        }
        let saved = saved.expect("saved");
        assert_eq!(
            saved.tracks.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![r.project.tracks[4].id.0],
            "named by its project track's id"
        );
        let t2 = saved
            .tracks
            .iter()
            .find(|t| t.index == 2)
            .expect("track 3 saved");
        assert_eq!(t2.from, "t200:before");
        assert!(t2.regions.is_empty());
        let text = mpfree_audiotracks::save::to_text(&saved);
        let back = mpfree_audiotracks::save::from_text(&text).unwrap();
        assert_eq!(back.tracks, saved.tracks);
        assert_eq!(
            parse_inside("t200:before"),
            Some(Inside {
                strip: StripId::Track(200),
                after_effects: false,
                after_fader: false,
            })
        );
        // After its fader (the input list's Pads and strips, phase 4).
        let f = Inside {
            strip: StripId::Bus(1),
            after_effects: true,
            after_fader: true,
        };
        assert_eq!(parse_inside(&inside_text(f)), Some(f));
        assert_eq!(f.point(), mpfree_fx::CapturePoint::AfterFader);
        assert_eq!(parse_inside(""), None);
        assert_eq!(parse_inside("t200:sideways"), None);
        // Opened again: it records from there, named as the mixer names
        // the strip until the panel offers the mix.
        r.h.clear();
        r.periods(4);
        assert!(
            r.h.inside(2).is_none(),
            "a new project records the line input"
        );
        r.h.open(back);
        r.periods(4);
        let (f, name) = r.h.inside(2).unwrap();
        assert_eq!((f.strip, f.after_effects), (StripId::Track(200), false));
        assert_eq!(name, crate::mixing::name(StripId::Track(200)));
        // An input chip goes back to the line input.
        act(
            &mut r,
            AudioAction::Input {
                track: 2,
                input: Input::Stereo,
            },
        )
        .unwrap();
        assert!(r.h.inside(2).is_none());
        // Mix again brings back the one picked last.
        let line = act(&mut r, AudioAction::Mix { track: 2, step: 0 }).unwrap();
        assert!(line.contains("before its effects"), "{line}");
    }

    #[test]
    fn split_erase_and_undo_edit_the_region_and_refusals_say_why() {
        let mut r = Rig::with(3);
        r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), true)
            .unwrap();
        r.live = true;
        r.command(T::Record);
        while r.t.position() < 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.periods(10);
        r.h.action(
            AudioAction::Arm {
                track: 0,
                on: false,
            },
            &r.info(),
            true,
        )
        .unwrap();
        let reg = RegionId { track: 0, index: 0 };
        // The playhead at the end of the take is outside the region's inside.
        r.command(T::Locate(0));
        r.periods(2);
        let e =
            r.h.action(AudioAction::Split(reg), &r.info(), true)
                .unwrap_err();
        assert!(e.contains("playhead inside"), "{e}");
        r.command(T::Locate(1920));
        // The line names the track as the musician named it.
        let p = r.project.tracks.iter().position(|t| t.is_audio()).unwrap();
        r.project.tracks[p].name = "Vocal".into();
        r.periods(CHECK_EVERY as usize);
        assert_eq!(
            r.h.action(AudioAction::Split(reg), &r.info(), true)
                .unwrap(),
            "Vocal: region split"
        );
        r.periods(2);
        assert_eq!(r.h.host.tracks.regions(0).len(), 2);
        r.h.action(
            AudioAction::Erase(RegionId { track: 0, index: 1 }),
            &r.info(),
            true,
        )
        .unwrap();
        r.periods(2);
        assert_eq!(r.h.host.tracks.regions(0).len(), 1);
        assert_eq!(
            r.h.action(AudioAction::Undo, &r.info(), true).unwrap(),
            "audio tracks: undone (SHIFT + Undo redoes)"
        );
        r.periods(2);
        assert_eq!(r.h.host.tracks.regions(0).len(), 2);
        // SHIFT + Undo brings the erase back, once.
        assert_eq!(
            r.h.action(AudioAction::Redo, &r.info(), true).unwrap(),
            "audio tracks: redone"
        );
        r.periods(2);
        assert_eq!(r.h.host.tracks.regions(0).len(), 1);
        assert_eq!(
            r.h.action(AudioAction::Redo, &r.info(), true).unwrap_err(),
            "nothing to redo"
        );
        r.h.action(AudioAction::Undo, &r.info(), true).unwrap();
        r.periods(2);
        assert_eq!(r.h.host.tracks.regions(0).len(), 2);
        // A region that is not there, a track past the last, and what is not
        // built yet are refused with a reason.
        let gone = RegionId { track: 0, index: 9 };
        assert!(r
            .h
            .action(AudioAction::Erase(gone), &r.info(), true)
            .unwrap_err()
            .contains("pick it again"));
        assert!(r
            .h
            .action(AudioAction::Arm { track: 8, on: true }, &r.info(), true)
            .is_err());
        assert!(r.h.action(AudioAction::Takes(8), &r.info(), true).is_err());
        // Mute and solo are the track list's now (phase 1a).
        let mute = AudioAction::Mute { track: 0, on: true };
        assert!(r.h.action(mute, &r.info(), true).is_err());
        // Every engine track given to an audio track: none is free.
        for s in 3..MAX_TRACKS {
            assert_eq!(r.add(), s);
        }
        assert_eq!(r.h.free_slot(), None);
        assert_eq!(
            r.h.host.tracks.stats().refused,
            0,
            "nothing reached the engine to be refused"
        );
    }

    /// Audio tracks phase 5, with no screen yet (5c): a region's Follow
    /// tempo and its own tempo are steps of the region edits, Undo takes
    /// each back; To a cell and Song → scenes carry them (a tempo that is
    /// its take's as 0), and the time engine hears the take's tempo.
    #[test]
    fn a_regions_follow_tempo_and_tempo_are_region_edits_and_clips_carry_them() {
        let mut r = Rig::with(3);
        r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), true)
            .unwrap();
        r.live = true;
        r.command(T::Record);
        while r.t.position() < 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.periods(10);
        r.h.action(
            AudioAction::Arm {
                track: 0,
                on: false,
            },
            &r.info(),
            true,
        )
        .unwrap();
        let p = r.project.tracks.iter().position(|t| t.is_audio()).unwrap();
        r.project.tracks[p].name = "Vocal".into();
        r.periods(CHECK_EVERY as usize);
        let reg = RegionId { track: 0, index: 0 };
        let first = r.h.host.tracks.regions(0)[0];
        assert_eq!((first.follow, first.bpm), (FollowTempo::Off, 120.0));
        let follow = AudioAction::Follow {
            region: reg,
            follow: FollowTempo::Repitch,
            keep_pitch: false,
        };
        assert_eq!(
            r.h.action(follow, &r.info(), true).unwrap(),
            "Vocal: Follow tempo Repitch (Undo takes it back)"
        );
        let tempo = |c| AudioAction::Tempo {
            region: reg,
            centi_bpm: c,
        };
        assert_eq!(
            r.h.action(tempo(9750), &r.info(), true).unwrap(),
            "Vocal: region's tempo 97.5 BPM (Undo takes it back)"
        );
        assert!(r.h.action(tempo(500), &r.info(), true).is_err());
        r.periods(2);
        let now = r.h.host.tracks.regions(0)[0];
        assert_eq!((now.follow, now.bpm), (FollowTempo::Repitch, 97.5));
        // To a cell: made at its own tempo, its sound ending where its
        // recording's does at that tempo (a bar at 97.5 BPM).
        let map = *r.t.tempo_map();
        let meter = mpfree_transport::time::Meter::default();
        let (_, clip, _) = r.h.region_clip(reg, &map, meter).unwrap();
        let a = clip.audio.unwrap();
        assert_eq!(a.bpm, 97.5);
        let c = a.regions[0];
        assert_eq!((c.follow, c.bpm), (mpfree_seq::FollowTempo::Repitch, 97.5));
        let bar = (4.0 * 60.0 / 97.5 * f64::from(RATE)).round() as i64;
        assert_eq!(a.last_frame, now.src + bar);
        // Back to its take's: the clip region says 0, its recording's.
        assert_eq!(
            r.h.action(tempo(0), &r.info(), true).unwrap(),
            "Vocal: region's tempo its take's (Undo takes it back)"
        );
        r.periods(2);
        let songs = crate::time::SongAudio::regions(&r.h);
        let c = songs[0].1[0];
        assert_eq!((c.follow, c.bpm), (mpfree_seq::FollowTempo::Repitch, 0.0));
        // The time engine is told the take's tempo once.
        let tempos = r.h.tempos_if_changed().unwrap();
        assert_eq!(tempos, [(c.rec, 120.0)]);
        assert_eq!(r.h.tempos_if_changed(), None);
        // Undo takes each back in turn.
        for _ in 0..3 {
            r.h.action(AudioAction::Undo, &r.info(), true).unwrap();
            r.periods(2);
        }
        assert_eq!(r.h.host.tracks.regions(0)[0], first);
    }

    #[test]
    fn the_q_links_turn_level_pan_gain_and_fades_and_the_cards_say_them() {
        let mut r = Rig::with(3);
        r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), true)
            .unwrap();
        r.live = true;
        r.command(T::Record);
        while r.t.position() < 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.periods(10);
        let region = RegionId { track: 0, index: 0 };
        let turn = |r: &mut Rig, knob, ticks, fine| {
            let a = AudioAction::Turn {
                track: 0,
                region: Some(0),
                knob,
                ticks,
                fine,
            };
            let line = r.h.action(a, &r.info(), true).unwrap();
            r.periods(2);
            line
        };
        // Q1 and Q2 are the track's strip's (phase 1b): the app moves the
        // strip, and the cards show it as the tracks mirror it.
        let a = AudioAction::Turn {
            track: 0,
            region: Some(0),
            knob: 0,
            ticks: 1,
            fine: false,
        };
        let e = r.h.action(a, &r.info(), true).unwrap_err();
        assert!(e.contains("its mixer strip's"), "{e}");
        (r.h.volume[0], r.h.pan[0]) = (from_db(-6.0, LEVEL_MAX_DB), 0.25);
        assert_eq!(turn(&mut r, 2, 7, false), "", "turns write no log line");
        turn(&mut r, 2, -5, true);
        turn(&mut r, 3, 4, false);
        let mut st = r.state();
        st.region = Some(region);
        let knobs = audio_knobs(&st).unwrap();
        assert_eq!((knobs.track, knobs.region), (0, Some(0)));
        assert_eq!(knobs.title(), "Q-Links · Audio region");
        let shown: Vec<_> = knobs.cards.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(shown, ["-6.0 dB", "R25", "3.0 dB", "20 ms"]);
        assert!(r.h.take_changed());
        // Gain and Fades point at their knobs; a fifth knob has nothing.
        assert!(r
            .h
            .action(AudioAction::Gain(region), &r.info(), true)
            .unwrap()
            .contains("Q3"));
        assert!(r
            .h
            .action(AudioAction::Fades(region), &r.info(), true)
            .unwrap()
            .contains("Q4"));
        let a = AudioAction::Turn {
            track: 0,
            region: Some(0),
            knob: 4,
            ticks: 1,
            fine: false,
        };
        assert!(r.h.action(a, &r.info(), true).is_err());
        // Without a region, Q3 and Q4 ask for one.
        let bare = |knob| AudioAction::Turn {
            track: 0,
            region: None,
            knob,
            ticks: 1,
            fine: false,
        };
        let err = r.h.action(bare(2), &r.info(), true).unwrap_err();
        assert!(err.contains("pick a region of Audio 1"), "{err}");
        assert!(r.h.action(bare(3), &r.info(), true).is_err());
        assert_eq!(r.h.host.tracks.stats().refused, 0);
        // No region picked and no panel open: no cards.
        st.region = None;
        assert!(audio_knobs(&st).is_none());
    }

    /// Layout spec 4c: the header opens the track's panel; its Input,
    /// Monitor, Mute and Solo reach the engine and show in the lane, and
    /// the Q-Links follow the panel's track.
    #[test]
    fn the_track_panel_sets_the_track_and_the_q_links_follow_it() {
        let mut r = Rig::with(3);
        let act = |r: &mut Rig, a| {
            let out = r.h.action(a, &r.info(), true);
            r.periods(2);
            out
        };
        // The panel opens on an empty track (every audio track has its
        // lane), and the header toggles it.
        act(&mut r, AudioAction::Panel(2)).unwrap();
        let st = r.state();
        assert_eq!(st.audio_panel, Some(2));
        assert_eq!(
            st.audio.iter().map(|l| l.track).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(r.h.take_changed());
        let knobs = audio_knobs(&st).unwrap();
        assert_eq!((knobs.track, knobs.region), (2, None));
        assert_eq!(knobs.title(), "Q-Links · Audio 3");
        let shown: Vec<_> = knobs.cards.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(shown, ["0.0 dB", "C", "pick one", "pick one"]);
        act(&mut r, AudioAction::Panel(2)).unwrap();
        assert_eq!(r.state().audio_panel, None);
        assert_eq!(r.state().audio.len(), 3, "closed: the empty lane stays");
        act(&mut r, AudioAction::Panel(1)).unwrap();
        act(&mut r, AudioAction::ClosePanel).unwrap();
        assert_eq!(r.state().audio_panel, None);
        assert!(act(&mut r, AudioAction::Panel(MAX_TRACKS)).is_err());

        // Monitor In hears the line input on an unarmed track, stopped
        // and in clip mode too; Off never does.
        r.song = false;
        r.live = true;
        act(&mut r, AudioAction::Panel(0)).unwrap();
        r.periods(4);
        assert_eq!(r.heard, 0.0, "Auto, unarmed: silent");
        let line = act(
            &mut r,
            AudioAction::Monitor {
                track: 0,
                monitor: Monitor::In,
            },
        )
        .unwrap();
        assert!(line.contains("In (always hear the input)"), "{line}");
        r.periods(4);
        assert!(r.heard > 0.1, "In: heard {}", r.heard);
        act(
            &mut r,
            AudioAction::Monitor {
                track: 0,
                monitor: Monitor::Off,
            },
        )
        .unwrap();
        act(&mut r, AudioAction::Arm { track: 0, on: true }).unwrap();
        r.periods(4);
        assert_eq!(r.heard, 0.0, "Off, armed: silent");
        act(
            &mut r,
            AudioAction::Monitor {
                track: 0,
                monitor: Monitor::Auto,
            },
        )
        .unwrap();
        r.periods(4);
        assert!(r.heard > 0.1, "Auto, armed and stopped: heard");
        // Its track's mute silences it; a solo of another track does too,
        // a note track's as well (solo is the project's); its own solo
        // brings it back.
        r.mute(0, false, true);
        r.periods(4);
        assert_eq!(r.heard, 0.0, "muted");
        r.mute(0, false, false);
        r.mute(1, true, true);
        r.periods(4);
        assert_eq!(r.heard, 0.0, "another track soloed");
        r.mute(1, true, false);
        r.project.tracks[0].solo = true;
        r.sync();
        r.periods(4);
        assert_eq!(r.heard, 0.0, "a note track soloed");
        r.project.tracks[0].solo = false;
        r.mute(1, true, true);
        r.mute(0, true, true);
        r.periods(4);
        assert!(r.heard > 0.1, "soloed too: heard");

        // The input: the lane's chip says it; it waits while recording.
        let line = act(
            &mut r,
            AudioAction::Input {
                track: 0,
                input: Input::Stereo,
            },
        )
        .unwrap();
        assert!(line.contains("both inputs (stereo)"), "{line}");
        let st = r.state();
        let lane = st.audio.iter().find(|l| l.track == 0).unwrap();
        assert_eq!(
            (lane.input, lane.monitor, lane.mute, lane.solo),
            (Input::Stereo, Monitor::Auto, false, true)
        );
        r.song = true;
        r.command(T::Record);
        r.periods(20);
        assert!(r.h.host.tracks.recording(0).is_some());
        let err = act(
            &mut r,
            AudioAction::Input {
                track: 0,
                input: Input::Left,
            },
        )
        .unwrap_err();
        assert!(err.contains("after it stops"), "{err}");
        r.command(T::Stop);
        r.periods(10);
        // A region picked on the panel's track gives Q3 and Q4 to it; one
        // on another track leaves them asking.
        let mut st = r.state();
        st.region = Some(RegionId { track: 0, index: 0 });
        assert_eq!(audio_knobs(&st).unwrap().region, Some(0));
        st.audio_panel = Some(1);
        st.audio.push(AudioLane {
            track: 1,
            ..st.audio[0].clone()
        });
        let knobs = audio_knobs(&st).unwrap();
        assert_eq!((knobs.track, knobs.region), (1, None));
        assert_eq!(r.h.host.tracks.stats().refused, 0);
    }

    /// Layout spec 4c: Takes opens the track's take lanes (newest first);
    /// a swipe across one plays that stretch, snapped to the grid unless
    /// fine, and a tap uses the take everywhere it has audio.
    #[test]
    fn take_lanes_open_under_the_track_and_a_swipe_comps_a_stretch() {
        let mut r = Rig::with(3);
        let act = |r: &mut Rig, a| {
            let out = r.h.action(a, &r.info(), true);
            r.periods(2);
            out
        };
        act(&mut r, AudioAction::Arm { track: 0, on: true }).unwrap();
        let e = act(&mut r, AudioAction::Takes(0)).unwrap_err();
        assert!(e.contains("no takes yet"), "{e}");
        // Two recordings over bar 1.
        r.live = true;
        for _ in 0..2 {
            r.command(T::Locate(0));
            r.periods(2);
            r.command(T::Record);
            while r.t.position() < 3840 {
                r.period();
            }
            r.command(T::Stop);
            r.periods(10);
        }
        act(
            &mut r,
            AudioAction::Arm {
                track: 0,
                on: false,
            },
        )
        .unwrap();
        let ids: Vec<u16> = {
            let mut k: Vec<TakeInfo> = r.h.host.tracks.takes().collect();
            k.sort_by_key(|t| (t.group, t.pass, t.id));
            k.iter().map(|t| t.id).collect()
        };
        assert_eq!(ids.len(), 2);
        let (old, new) = (ids[0], ids[1]);
        let plays = |r: &Rig| -> Vec<(Tick, Tick, u16)> {
            r.h.host
                .tracks
                .regions(0)
                .iter()
                .map(|g| (g.start, g.end, g.take))
                .collect()
        };
        let end = plays(&r)[0].1;
        assert_eq!(plays(&r), [(0, end, new)], "the newest plays");
        // Takes opens the lanes and closes the panel, which sat there.
        act(&mut r, AudioAction::Panel(0)).unwrap();
        let line = act(&mut r, AudioAction::Takes(0)).unwrap();
        assert!(line.contains("2 takes under the lane"), "{line}");
        let st = r.state();
        assert_eq!((st.audio_takes, st.audio_panel), (Some(0), None));
        let lane = &st.audio[0];
        let names: Vec<_> = lane.takes.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, ["take 2", "take 1"], "newest first");
        assert_eq!(lane.takes[0].used, [(0, end)]);
        assert!(lane.takes[1].used.is_empty());
        assert!(lane.takes[1].start <= 0 && lane.takes[1].end >= end - 1);
        // A swipe over beats 2 and 3 of take 1, snapped to sixteenths.
        let line = act(
            &mut r,
            AudioAction::Comp {
                take: old,
                from: 997,
                to: 2830,
                fine: false,
            },
        )
        .unwrap();
        assert_eq!(line, "Audio 1: take 1 plays from 1.2 to 1.4");
        assert_eq!(
            plays(&r),
            [(0, 960, new), (960, 2880, old), (2880, end, new)]
        );
        // With SHIFT it is free.
        act(
            &mut r,
            AudioAction::Comp {
                take: old,
                from: 3001,
                to: 3005,
                fine: true,
            },
        )
        .unwrap();
        assert!(plays(&r).contains(&(3001, 3005, old)));
        // Undo takes the edits back one at a time.
        act(&mut r, AudioAction::Undo).unwrap();
        act(&mut r, AudioAction::Undo).unwrap();
        assert_eq!(plays(&r), [(0, end, new)]);
        // A tap: the take everywhere it has audio.
        let line = act(&mut r, AudioAction::UseTake(old)).unwrap();
        assert_eq!(line, "Audio 1: take 1 plays everywhere it has audio");
        assert!(plays(&r).iter().all(|p| p.2 == old), "{:?}", plays(&r));
        assert!(r.h.take_changed());
        // Takes again closes them.
        act(&mut r, AudioAction::Takes(0)).unwrap();
        assert_eq!(r.state().audio_takes, None);
        // Comping waits for a recording to stop, and a gone take says so.
        act(&mut r, AudioAction::Arm { track: 0, on: true }).unwrap();
        r.command(T::Locate(0));
        r.command(T::Record);
        r.periods(20);
        let e = act(&mut r, AudioAction::UseTake(new)).unwrap_err();
        assert!(e.contains("waits until the recording stops"), "{e}");
        r.command(T::Stop);
        r.periods(10);
        assert!(act(&mut r, AudioAction::UseTake(999)).is_err());
        assert_eq!(r.h.host.tracks.stats().refused, 0);
    }

    /// Layout spec 4c: dragging a region moves it, snapped to sixteenths
    /// (free with SHIFT); the screen picks it again where it went.
    #[test]
    fn a_dragged_region_moves_on_the_grid_and_is_picked_again() {
        let mut r = Rig::with(3);
        let act = |r: &mut Rig, a| {
            let out = r.h.action(a, &r.info(), true);
            r.periods(2);
            out
        };
        act(&mut r, AudioAction::Arm { track: 0, on: true }).unwrap();
        r.live = true;
        r.command(T::Record);
        while r.t.position() < 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.periods(10);
        act(
            &mut r,
            AudioAction::Arm {
                track: 0,
                on: false,
            },
        )
        .unwrap();
        let reg = RegionId { track: 0, index: 0 };
        let len = {
            let g = r.h.host.tracks.regions(0)[0];
            g.end - g.start
        };
        assert_eq!(r.h.take_repick(), None);
        let line = act(
            &mut r,
            AudioAction::Move {
                region: reg,
                to: 7690,
                fine: false,
            },
        )
        .unwrap();
        assert_eq!(line, "Audio 1: region moved to 3.1 (Undo puts it back)");
        let g = r.h.host.tracks.regions(0)[0];
        assert_eq!((g.start, g.end), (7680, 7680 + len));
        assert_eq!(r.h.take_repick(), Some(reg));
        assert_eq!(r.h.take_repick(), None, "once");
        // With SHIFT it lands where it was let go.
        act(
            &mut r,
            AudioAction::Move {
                region: reg,
                to: 7701,
                fine: true,
            },
        )
        .unwrap();
        assert_eq!(r.h.host.tracks.regions(0)[0].start, 7701);
        // Where it already is: nothing to do.
        assert_eq!(
            act(
                &mut r,
                AudioAction::Move {
                    region: reg,
                    to: 7701,
                    fine: true,
                },
            ),
            Ok(String::new())
        );
        // Undo walks it back.
        act(&mut r, AudioAction::Undo).unwrap();
        act(&mut r, AudioAction::Undo).unwrap();
        assert_eq!(r.h.host.tracks.regions(0)[0].start, 0);
        let gone = RegionId { track: 0, index: 5 };
        assert!(act(
            &mut r,
            AudioAction::Move {
                region: gone,
                to: 0,
                fine: false
            }
        )
        .unwrap_err()
        .contains("gone"));
        assert_eq!(r.h.host.tracks.stats().refused, 0);
    }

    #[test]
    fn a_swipe_snaps_to_sixteenths_and_is_never_empty() {
        assert_eq!(snap_span(997, 2830), (960, 2880));
        assert_eq!(snap_span(2830, 997), (960, 2880), "either way round");
        assert_eq!(snap_span(250, 260), (240, 480), "one step at least");
        assert_eq!(snap_span(-130, 100), (-240, 0));
    }

    #[test]
    fn the_line_input_latency_follows_the_loopers_setting() {
        let mut r = Rig::new();
        assert_eq!(r.h.host.tracks.latency(), 0);
        r.h.follow_latency(384);
        r.period();
        assert_eq!(r.h.host.tracks.latency(), 384);
        // The same value is not sent again; a new one is.
        r.h.follow_latency(384);
        r.h.follow_latency(512);
        r.period();
        assert_eq!(r.h.host.tracks.latency(), 512);
        assert_eq!(r.h.host.tracks.stats().dropped, 0);
    }

    /// Steady play of a recorded take, with its waveform read, neither
    /// allocates nor frees on the audio thread.
    #[test]
    fn a_project_opening_empties_the_tracks_and_puts_its_part_in() {
        let mut r = Rig::new();
        let act = |r: &mut Rig, a| {
            r.h.action(a, &r.info(), true).unwrap();
            r.periods(2);
        };
        // A bar on audio 1, audio 2 muted (its track's mute) and set to
        // the right input, audio 3 empty.
        for s in 0..3 {
            assert_eq!(r.add(), s);
        }
        act(&mut r, AudioAction::Arm { track: 0, on: true });
        r.mute(1, false, true);
        r.periods(1);
        assert!(r.h.host.tracks.settings(1).unwrap().mute, "silenced");
        act(
            &mut r,
            AudioAction::Input {
                track: 1,
                input: Input::Right,
            },
        );
        r.live = true;
        r.command(T::Record);
        while r.t.position() < 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.live = false;
        act(
            &mut r,
            AudioAction::Arm {
                track: 0,
                on: false,
            },
        );
        r.periods(20);
        assert!(r.h.host.save());
        let t0 = Instant::now();
        let saved = loop {
            if let Some(s) = r.h.host.take_saved() {
                break s;
            }
            assert!(t0.elapsed() < Duration::from_secs(20), "no save");
            r.period();
            std::thread::sleep(Duration::from_micros(100));
        };
        assert_eq!((saved.tracks.len(), saved.takes.len()), (2, 1));
        // The recording's lasting id goes with the save (phase 1c).
        let rec = r.h.host.tracks.takes().next().unwrap().rec;
        assert!(rec.is_given());
        assert_eq!(saved.takes[0].rec, rec);
        let first = r.h.host.tracks.regions(0).to_vec();
        r.waves();

        // Another take records when the project opens: the recording ends,
        // the tracks empty once it has, and the part goes in.
        act(&mut r, AudioAction::Arm { track: 2, on: true });
        r.live = true;
        r.command(T::Record);
        r.periods(40);
        assert!(r.h.host.tracks.recording(2).is_some());
        r.h.clear();
        r.command(T::Stop);
        r.live = false;
        assert!(r.h.opening());
        r.h.open(saved);
        let t0 = Instant::now();
        while r.h.opening() {
            assert!(t0.elapsed() < Duration::from_secs(20), "never opened");
            r.period();
            std::thread::sleep(Duration::from_micros(100));
        }
        assert_eq!(
            r.h.take_note().as_deref(),
            Some("project: 2 audio tracks and 1 take in")
        );
        assert_eq!(r.h.host.tracks.takes().count(), 1);
        assert_eq!(r.h.host.tracks.takes().next().unwrap().rec, rec, "kept");
        assert_eq!(r.h.host.tracks.regions(0), &first[..]);
        assert!(r.h.host.tracks.regions(2).is_empty());
        assert!(!r.h.host.tracks.armed(2));
        let st = r.state();
        assert_eq!(
            st.audio.iter().map(|l| l.track).collect::<Vec<_>>(),
            [0, 1, 2],
            "every audio track in the list has its lane"
        );
        assert!(st.audio[1].mute && st.audio[1].input == Input::Right);
        assert!(!st.audio[0].mute && !st.audio[0].armed);
        r.periods(1);
        assert!(r.h.host.tracks.settings(1).unwrap().mute, "silenced again");
        // The waveform is read again for the take that came back.
        r.waves();
        assert_eq!(r.state().waves.len(), 1);
        r.command(T::Locate(0));
        r.command(T::Play);
        r.periods(100);
        assert!(r.heard > 0.3, "plays back: {}", r.heard);
        r.command(T::Stop);
        r.periods(4);

        // New project: no audio track, empty, every setting as at start,
        // no line.
        r.project.tracks.retain(|t| !t.is_audio());
        r.h.bind(Slots::default());
        r.h.clear();
        r.periods(2);
        assert!(!r.h.opening());
        assert_eq!(r.h.take_note(), None);
        assert_eq!(r.h.host.tracks.takes().count(), 0);
        assert!(r.state().audio.is_empty());
        assert!(!r.h.silent[1] && r.h.inputs[1] == Input::Left);
        assert!(!r.h.host.tracks.settings(1).unwrap().mute);
    }

    /// Phase 3b step 3: with Dev's "Audio tracks from their files" on, a
    /// project's takes open as their files with nothing read into memory,
    /// Dev's Performance page counts them, and frames the drive was late
    /// with are said in the log, with the track and the bar.
    #[test]
    fn takes_play_from_their_files_and_dropouts_are_said() {
        use mpfree_audiotracks::save::{wav, TakeFile};
        use mpfree_audiotracks::{RecordingId, Region};
        let dir = std::env::temp_dir().join(format!("mpfree-th-stream-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut r = Rig::new();
        // 10 s of stereo is under the least cache, 4 MiB.
        assert_eq!(r.h.stream(10), Some(4 << 20));
        assert!(r.h.stream(10).is_none(), "once");
        assert!(r.h.streams());
        assert_eq!(r.add(), 0);
        // 30 s of mono in the project's file: longer than the cache holds.
        let frames = 30 * RATE as usize;
        let audio: Vec<f32> = (0..frames)
            .map(|g| (g % 100) as f32 / 200.0 - 0.25)
            .collect();
        let path = dir.join("Audio-1_take.wav");
        std::fs::write(&path, wav(&audio, 1, RATE)).unwrap();
        // 30 s at 120 BPM: 60 beats.
        let end: Tick = 60 * mpfree_transport::time::PPQ as Tick;
        let saved = Saved {
            rate: RATE,
            tracks: vec![TrackFile {
                index: 0,
                id: 0,
                input: Input::Left,
                monitor: Monitor::Auto,
                from: String::new(),
                regions: vec![Region {
                    start: 0,
                    end,
                    take: 0,
                    src: 0,
                    phase: 0.0,
                    gain: 1.0,
                    fade_in: 0,
                    fade_out: 0,
                    into: 0,
                    left: 0,
                    bpm: 0.0,
                    follow: Default::default(),
                    keep_pitch: false,
                }],
            }],
            takes: vec![TakeFile {
                id: 0,
                rec: RecordingId(0x77),
                track: 0,
                group: 0,
                pass: 0,
                bpm: 120.0,
                anchor: 0.0,
                frames,
                channels: 1,
                file: "Audio-1_take.wav".into(),
                path: Some(path.clone()),
                audio: Vec::new(),
                varied: false,
            }],
        };
        r.h.clear();
        r.h.open(saved);
        let t0 = Instant::now();
        while r.h.opening() {
            assert!(t0.elapsed() < Duration::from_secs(20), "never opened");
            r.period();
            std::thread::sleep(Duration::from_micros(200));
        }
        assert_eq!(
            r.h.take_note().as_deref(),
            Some("project: 1 audio track and 1 take in")
        );
        assert!(r.h.host.tracks.on_file(0));
        assert_eq!(r.h.host.tracks.memory().used, 0, "nothing read in");
        assert_eq!(r.h.host.tracks.regions(0).len(), 1);
        assert_eq!(
            r.h.stream_numbers(),
            Some(StreamNumbers {
                on_file: 1,
                takes: 1,
                silent_s: 0.0,
                ..r.h.stream_numbers().unwrap()
            })
        );
        // The region's start is held in the cache.
        let t0 = Instant::now();
        while r.h.stream_numbers().unwrap().heads_held == 0 {
            assert!(t0.elapsed() < Duration::from_secs(20), "never held");
            r.period();
            std::thread::sleep(Duration::from_micros(200));
        }
        let n = r.h.stream_numbers().unwrap();
        assert_eq!((n.heads_held, n.heads_wanted), (1, 1));
        // Playing, then on at once to bar 11 (20 s in), which the cache
        // can't have yet: its first period plays silence.
        r.command(T::Locate(0));
        r.command(T::Play);
        r.periods(10);
        r.command(T::Locate(10 * 4 * mpfree_transport::time::PPQ as i64));
        r.periods(2);
        assert!(r.h.stream_numbers().unwrap().silent_s > 0.0);
        // The log says so within a second, naming the track and the bar.
        let mut line = None;
        for _ in 0..=DROPOUTS_EVERY {
            r.period();
            if let Some(l) = r.h.take_file_line() {
                line = Some(l);
                break;
            }
        }
        let line = line.expect("a line about the dropout");
        assert!(
            line.starts_with("audio tracks: the drive fell behind: ")
                && line.ends_with(" s of Audio 1 went silent at bar 11"),
            "{line}"
        );
        r.command(T::Stop);
        r.periods(4);
        drop(r);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_level_bars_follow_the_input_while_armed_and_the_sound_and_fall_back() {
        let mut r = Rig::with(3);
        r.periods(4);
        assert_eq!(r.h.take_levels(), None, "silence sends nothing");
        // Armed, the bar shows the input it records: a tone at 0.5 is
        // -6 dB, 7/8 of the bar.
        r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), true)
            .unwrap();
        r.live = true;
        r.periods(4);
        let l = r.h.take_levels().expect("the bar moved");
        assert!((l[0] - meter(0.5)).abs() < 0.01, "{}", l[0]);
        assert!((meter(0.5) - 0.875).abs() < 0.01);
        assert_eq!(&l[1..], &[0.0; MAX_TRACKS - 1]);
        r.periods(4);
        assert_eq!(r.h.take_levels(), None, "a steady tone sends nothing");
        let mut st = r.state();
        assert!((st.audio[0].level - l[0]).abs() < 0.01);
        // Another input hears nothing: the tone is on the left only.
        r.h.action(
            AudioAction::Input {
                track: 0,
                input: Input::Right,
            },
            &r.info(),
            true,
        )
        .unwrap();
        r.periods(8);
        // It falls back over about 1.5 s, not at once.
        let after = r.h.take_levels().expect("falling");
        assert!(after[0] > 0.5 && after[0] < l[0], "{}", after[0]);
        let t = (1.6 * RATE as f32 / PERIOD as f32) as usize;
        r.periods(t);
        assert_eq!(r.h.take_levels().unwrap()[0], 0.0);
        st = r.state();
        assert_eq!(st.audio[0].level, 0.0);
        // Unarmed and silent, a recorded take playing shows on its bar.
        r.h.action(
            AudioAction::Input {
                track: 0,
                input: Input::Left,
            },
            &r.info(),
            true,
        )
        .unwrap();
        r.command(T::Record);
        while r.t.position() < 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.live = false;
        r.h.action(
            AudioAction::Arm {
                track: 0,
                on: false,
            },
            &r.info(),
            true,
        )
        .unwrap();
        r.periods(t);
        let _ = r.h.take_levels();
        r.command(T::Locate(0));
        r.command(T::Play);
        r.periods(100);
        let l = r.h.take_levels().expect("playing");
        assert!(l[0] > 0.8, "{}", l[0]);
        r.command(T::Stop);
        r.periods(4);
    }

    #[test]
    fn steady_play_never_allocates() {
        let mut r = Rig::with(3);
        r.h.action(AudioAction::Arm { track: 0, on: true }, &r.info(), true)
            .unwrap();
        r.live = true;
        r.command(T::Record);
        while r.t.position() < 3840 {
            r.period();
        }
        r.command(T::Stop);
        r.live = false;
        r.periods(10);
        r.waves();
        r.command(T::Locate(0));
        r.command(T::Play);
        r.periods(10);
        let ((), calls) = crate::sys::alloc_count::calls(|| r.periods(2000));
        assert_eq!(calls, 0, "allocations or frees while playing");
        assert!(r.heard > 0.0 || r.t.position() > 3840);
    }

    /// The one Sample page's step 6: a sound from the Sample page goes on
    /// the first empty audio track from the bar the playhead is in, a
    /// region a slice, and its lane shows; the next waits for it, then
    /// takes the next empty track; with none empty, the first with nothing
    /// from the bar on; Undo takes the last off; more than the memory is
    /// refused and says where to get more.
    #[test]
    fn a_sound_from_the_sample_page_goes_on_an_empty_track_from_the_playheads_bar() {
        let mut r = Rig::new();
        r.periods(2);
        let none = r.h.bring_track(&TransportInfo::default());
        assert_eq!(none, None, "no audio track: the app adds one first");
        for s in 0..MAX_TRACKS {
            assert_eq!(r.add(), s);
        }
        let sound = |name: &str, frames: usize, cuts: Vec<u64>| Sound {
            name: name.into(),
            channels: 1,
            audio: (0..frames)
                .map(|g| {
                    (0.5 * (std::f64::consts::TAU * 441.0 * g as f64 / f64::from(RATE)).sin())
                        as f32
                })
                .collect(),
            cuts,
            bpm: 120.0,
            cell: false,
        };
        let bar = mpfree_transport::Meter::default().bar_ticks();
        let at = |b: i64| TransportInfo {
            position: (b - 1) * bar + 100,
            ..Default::default()
        };
        let half = RATE as usize / 2;
        let line =
            r.h.bring(sound("Break", 2 * half, vec![half as u64]), &at(3))
                .unwrap();
        assert_eq!(
            line,
            "Break's 2 slices on Audio 1 from bar 3; the Song view's Undo takes it off"
        );
        assert_eq!(
            r.h.bring(sound("Hit", 100, Vec::new()), &at(3)),
            Err("another sound is still going onto its track: a moment".into())
        );
        let t0 = Instant::now();
        while r.h.host.importing().is_some() {
            assert!(t0.elapsed() < Duration::from_secs(20), "never placed");
            r.period();
        }
        let regs = r.h.host.tracks.regions(0).to_vec();
        assert_eq!(regs.len(), 2, "{regs:?}");
        assert_eq!(regs[0].start, 2 * bar);
        assert_eq!(regs[1].start, regs[0].end);
        assert!((regs[1].src - half as i64).abs() <= 1, "{regs:?}");
        let take = r.h.host.tracks.take(regs[0].take).unwrap();
        assert_eq!((take.bpm, take.frames, take.channels), (120.0, 2 * half, 1));
        // The next goes on the next empty track, until none is empty.
        for k in 1..MAX_TRACKS {
            let line = r.h.bring(sound("Hit", 4410, Vec::new()), &at(3)).unwrap();
            assert_eq!(
                line,
                format!(
                    "Hit on Audio {} from bar 3; the Song view's Undo takes it off",
                    k + 1
                )
            );
            while r.h.host.importing().is_some() {
                r.period();
            }
            assert_eq!(r.h.host.tracks.regions(k).len(), 1);
        }
        assert_eq!(r.state().audio.len(), MAX_TRACKS);
        // None empty: the first with nothing from the bar on, else none.
        assert_eq!(
            r.h.bring(sound("Hit", 4410, Vec::new()), &at(1)),
            Err("every audio track has regions from bar 1 on: move the playhead past them, erase some in the Song view, or add an audio track".into())
        );
        let line = r.h.bring(sound("Hit", 4410, Vec::new()), &at(9)).unwrap();
        assert_eq!(
            line,
            "Hit on Audio 1 from bar 9; the Song view's Undo takes it off"
        );
        while r.h.host.importing().is_some() {
            r.period();
        }
        assert_eq!(r.h.host.tracks.regions(0).len(), 3);
        // Undo takes it off again.
        let undo = r.h.action(AudioAction::Undo, &r.info(), true);
        assert!(undo.is_ok(), "{undo:?}");
        r.periods(2);
        assert_eq!(r.h.host.tracks.regions(0).len(), 2);
        // Out of song mode it says it plays in song mode.
        r.song = false;
        r.periods(2);
        let line = r.h.bring(sound("Hit", 4410, Vec::new()), &at(12)).unwrap();
        assert_eq!(
            line,
            "Hit on Audio 1 from bar 12; it plays in song mode, and the Song view's Undo takes it off"
        );
        while r.h.host.importing().is_some() {
            r.period();
        }
        // More than the memory (10 s) is refused, and says where to get more.
        let e =
            r.h.bring(sound("Long", 12 * RATE as usize, Vec::new()), &at(20));
        assert_eq!(
            e,
            Err("12.0 s is more than the audio tracks' memory has left: erase takes, or more memory in Prefs > Audio track memory".into())
        );
        assert_eq!(r.h.host.importing(), None);
    }
}
