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

use mpfree_audiotracks::host::{AudioHost, ImportRefused, TakeWave, MAX_COLUMNS};
use mpfree_audiotracks::save::{Saved, TrackFile};
use mpfree_audiotracks::streaming::StreamConfig;
use mpfree_audiotracks::{
    Command, Input, Monitor, Retime, RetimeRefused, SongStep, TakeInfo, MAX_REGIONS, MAX_TAKES,
    MAX_TRACKS, PEAK_BLOCK,
};
use mpfree_seq::TrackId;
use mpfree_time_ui::song::{audio_blocks, input_chip, take_lanes, AudioLane, MixFrom, MONITORS};
use mpfree_time_ui::view::{
    audio_color, track_color, AudioAction, RegionId, TimeState, TransportInfo,
};
use mpfree_transport::{Block, TempoMap, Tick};
use mpfree_ui::qlink::QLinkCard;

use crate::mixing::MixHost;
use crate::PERIOD;
use mpfree_fx::ids::mixer as mx;

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
/// and whether after its effects. Before its fader either way, so the
/// strip's level never changes the take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inside {
    pub strip: mpfree_fx::StripId,
    pub after_effects: bool,
}

/// The audio tracks, their host and what the Song view was last given.
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
    /// What the lanes showed at the last check, and whether that changed.
    seen: u64,
    changed: bool,
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
    /// The song's meter, for the bar a dropout was at.
    time_sig: mpfree_transport::time::Meter,
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
            seen: 0,
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
            silent: [false; MAX_TRACKS],
            arm_next: None,
            files_dir: None,
            silent_seen: [0; MAX_TAKES],
            time_sig: mpfree_transport::time::Meter::default(),
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

    /// The next line for the log about a take's file (it couldn't be made,
    /// it is at risk, a save took a take from memory, it couldn't be
    /// opened to play from), naming the track as the musician named it;
    /// and, about once a second while it happens, the takes on their
    /// files that played silence because the drive was late. Allocates
    /// only when there is one.
    pub fn take_file_line(&mut self) -> Option<String> {
        if let Some(e) = self.host.take_event() {
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
    /// `out` (overwritten). `song_on` is song mode.
    pub fn period(
        &mut self,
        block: &Block,
        map: &TempoMap,
        song_on: bool,
        line: (&[f32; PERIOD], &[f32; PERIOD]),
    ) {
        if song_on != self.active && self.host.command(Command::SetActive(song_on)) {
            self.active = song_on;
        }
        for f in 0..PERIOD {
            self.input[2 * f] = line.0[f];
            self.input[2 * f + 1] = line.1[f];
        }
        self.out.fill(0.0);
        // The engine's sum is no one's since phase 1b: each track plays
        // through its own channel (`play_into`).
        self.host
            .period(block, map, &self.input[..], &mut self.out[..]);
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
                let when = if f.after_effects { "after" } else { "before" };
                format!("{n} records {name}, {when} its effects")
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
        };
        self.set_inside(t, Some((f, &name)))
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
            }
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
                blocks: audio_blocks(t, tr.regions(t), &takes, map, self.rate),
                takes: take_lanes(t, tr.regions(t), &takes, map, self.rate),
                recording: tr
                    .recording(t)
                    .and_then(|id| tr.take(id))
                    .map(|k| k.anchor.floor() as Tick),
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
            AudioAction::Input { track, input } => {
                let n = self.track_no(track)?;
                if tr.recording(track).is_some() {
                    return Err(format!("{n} is recording: change its input after it stops"));
                }
                if self.inside[track].is_some() {
                    self.set_inside(track, None)?;
                }
                self.send(Command::SetInput(track, input))?;
                self.inputs[track] = input;
                self.changed = true;
                format!("{n} records {}", input_words(input))
            }
            AudioAction::Mix { track, step } => self.mix_step(track, step)?,
            AudioAction::MixEffects { track, after } => {
                let n = self.track_no(track)?;
                let Some(f) = self.inside[track] else {
                    return Err(format!("{n} records the line input: pick Mix first"));
                };
                if f.after_effects == after {
                    return Ok(String::new());
                }
                let name = self.inside_name[track].clone();
                let f = Inside {
                    after_effects: after,
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
        reaches: impl FnOnce(u32) -> bool,
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
        format!(
            "audio tracks: {} takes, {:.0} of {:.0} s used, {} refused, {} dropped, {} cut short{}",
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
/// known), named `name`.
#[derive(PartialEq)]
pub struct Sound {
    pub name: String,
    pub channels: usize,
    pub audio: Vec<f32>,
    pub cuts: Vec<u64>,
    pub bpm: f64,
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
fn input_words(i: Input) -> &'static str {
    match i {
        Input::Left => "the left input (mono)",
        Input::Right => "the right input (mono)",
        Input::Sum => "left and right summed (mono)",
        Input::Stereo => "both inputs (stereo)",
    }
}

/// Where a track records from inside the mix, as the project saves it:
/// the strip as the mixer's part writes it, then `:before` or `:after`
/// its effects (`t12:after`).
fn inside_text(f: Inside) -> String {
    let when = if f.after_effects { "after" } else { "before" };
    format!("{}:{when}", crate::project::strip_text(f.strip))
}

/// [`inside_text`] read back; None for the line input (empty) or words
/// it doesn't know.
fn parse_inside(w: &str) -> Option<Inside> {
    let (strip, when) = w.split_once(':')?;
    Some(Inside {
        strip: crate::project::parse_strip(strip)?,
        after_effects: match when {
            "after" => true,
            "before" => false,
            _ => return None,
        },
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

    fn retime(&mut self, song: u32, r: Retime) -> bool {
        // There is room: asked first (refused).
        self.moves(&r) && self.send(Command::Retime { song, retime: r }).is_ok()
    }

    fn step(&self, song: u32, undo: bool) -> SongStep {
        self.host.tracks.song_step(song, undo)
    }

    fn song_undo(&mut self, song: u32, undo: bool) {
        let c = if undo {
            Command::SongUndo(song)
        } else {
            Command::SongRedo(song)
        };
        // There is room: asked first (busy).
        let _ = self.send(c);
    }
}

/// How full and writable the drive holding `dir` is, for System status
/// (`statvfs`, reads only; it may wait on a failing drive, so the audio
/// tracks call it on a thread of their own).
fn drive_room(dir: &std::path::Path) -> Option<mpfree_audiotracks::files::Room> {
    crate::sys::drive_room(dir)
        .map(|(writable, free)| mpfree_audiotracks::files::Room { writable, free })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_transport::{Command as T, State, Transport};
    use std::time::{Duration, Instant};

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
            self.h.period(&self.b, &map, self.song, (&l, &r));
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
                after_effects: false
            })
        );
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
                    gain: 1.0,
                    fade_in: 0,
                    fade_out: 0,
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
