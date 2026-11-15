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
//! - [`TrackHost::fill`] puts the lanes, the waveforms, the memory gauge
//!   and the counters into the time engine's [`TimeState`], for the Song
//!   view. [`TrackHost::take_changed`] says when they changed, checked a
//!   few times a second without allocating.
//! - Each track's level bar (layout spec 4c: 3 px along the header's
//!   bottom edge) follows its sound, and the line input while it is
//!   armed, measured each period without allocating
//!   ([`TrackHost::take_levels`]); the screen gets the eight numbers on
//!   their own, not a whole song state.
//! - Waveforms are read a take at a time with [`AudioHost::wave`], at a
//!   small fixed cost a period; a take that records is read again as it
//!   grows.
//! - Projects (`project.rs`, part `audiotracks`): a save reads every take
//!   over a few periods ([`AudioHost::save`]); opening a project or New
//!   empties the tracks ([`TrackHost::clear`], once nothing records) and
//!   the project's part goes in when it has been read
//!   ([`TrackHost::open`]).
//!
//! The memory is set aside when MPFree starts ([`SECONDS`] of stereo). The
//! takes stay in RAM: streaming them from the stick waits for the stick
//! speed test (rule zero: it needs Jay's go).

use mpfree_audiotracks::host::{AudioHost, TakeWave, MAX_COLUMNS};
use mpfree_audiotracks::save::Saved;
use mpfree_audiotracks::{Command, Input, Monitor, TakeInfo, MAX_TRACKS, PEAK_BLOCK};
use mpfree_time_ui::song::{audio_blocks, take_lanes, AudioLane, MONITORS};
use mpfree_time_ui::view::{audio_color, AudioAction, RegionId, TimeState, TransportInfo};
use mpfree_transport::{Block, TempoMap, Tick};
use mpfree_ui::qlink::QLinkCard;

use crate::PERIOD;

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
/// Periods between new reads of a recording take's waveform (about a
/// quarter of a second).
const GROW_EVERY: u64 = 86;

/// The audio tracks, their host and what the Song view was last given.
pub struct TrackHost {
    pub host: AudioHost,
    rate: u32,
    /// The line input and the tracks' sound, interleaved stereo.
    input: Box<[f32; 2 * PERIOD]>,
    out: Box<[f32; 2 * PERIOD]>,
    /// Audio tracks whose lanes show even when empty ("+ Audio").
    shown: usize,
    /// What each track records from (the engine keeps it too; this is for
    /// the header's chip).
    inputs: [Input; MAX_TRACKS],
    /// Each track's level (0 to 2) and balance (-1 to 1), monitor mode,
    /// mute and solo, as last sent.
    volume: [f32; MAX_TRACKS],
    pan: [f32; MAX_TRACKS],
    monitor: [Monitor; MAX_TRACKS],
    mute: [bool; MAX_TRACKS],
    solo: [bool; MAX_TRACKS],
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
    /// Each track's level bar, 0 to 1 (see [`meter`]), and the bars last
    /// given to the screen.
    meter: [f32; MAX_TRACKS],
    meter_shown: [f32; MAX_TRACKS],
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
            shown: 0,
            inputs: [Input::Left; MAX_TRACKS],
            volume: [1.0; MAX_TRACKS],
            pan: [0.0; MAX_TRACKS],
            monitor: [Monitor::Auto; MAX_TRACKS],
            mute: [false; MAX_TRACKS],
            solo: [false; MAX_TRACKS],
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
            meter: [0.0; MAX_TRACKS],
            meter_shown: [0.0; MAX_TRACKS],
        }
    }

    /// One period: the line input (`line`) in, the tracks' sound out into
    /// `out` (overwritten). `song_on` is song mode.
    pub fn period(
        &mut self,
        block: &Block,
        map: &TempoMap,
        song_on: bool,
        line: (&[f32; PERIOD], &[f32; PERIOD]),
        out: (&mut [f32; PERIOD], &mut [f32; PERIOD]),
    ) {
        if song_on != self.active && self.host.command(Command::SetActive(song_on)) {
            self.active = song_on;
        }
        for f in 0..PERIOD {
            self.input[2 * f] = line.0[f];
            self.input[2 * f + 1] = line.1[f];
        }
        self.out.fill(0.0);
        self.host
            .period(block, map, &self.input[..], &mut self.out[..]);
        for f in 0..PERIOD {
            out.0[f] = self.out[2 * f];
            out.1[f] = self.out[2 * f + 1];
        }
        self.measure();
        self.periods += 1;
        self.tend_open();
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

    /// The level bars: each track's peak this period (its sound after its
    /// level, pan, mute and solo, and while it is armed the input it
    /// records), falling back at [`METER_FALL`] of the bar a second.
    fn measure(&mut self) {
        let fall = METER_FALL * PERIOD as f32 / self.rate as f32;
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        for t in 0..MAX_TRACKS {
            let mut p = peak(self.host.tracks.track_out(t, PERIOD));
            if self.host.tracks.armed(t) {
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
    /// for the part, or takes copying in. A save waits for it.
    pub fn opening(&self) -> bool {
        self.clearing || self.pending.is_some() || self.host.tracks.loading() > 0
    }

    /// A line for the log from the open, once.
    pub fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }

    /// The open's next step, each period: empty the tracks once nothing
    /// records, then put the waiting part in.
    fn tend_open(&mut self) {
        if self.clearing {
            if !self.host.tracks.clear() {
                return;
            }
            self.clearing = false;
            // The waveforms were the old takes'; the ids come back.
            self.host.cancel_wave();
            self.reading = None;
            while let Some(w) = self.waves.pop() {
                self.host.drop_wave(w);
            }
            (self.panel, self.takes_open, self.repick, self.shown) = (None, None, None, 0);
            self.resync();
            self.changed = true;
        }
        let Some(saved) = self.pending.take() else {
            return;
        };
        let (tracks, takes) = (saved.tracks.len(), saved.takes.len());
        self.shown = saved.tracks.iter().map(|t| t.index + 1).max().unwrap_or(0);
        let r = self.host.load(saved);
        self.resync();
        self.changed = true;
        self.note = Some(match r {
            Ok(()) if tracks == 0 => return,
            Ok(()) => format!(
                "project: {tracks} audio track{} and {takes} take{} in",
                plural(tracks),
                plural(takes)
            ),
            Err(e) => {
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
                self.mute[t] = s.mute;
                self.solo[t] = s.solo;
            }
        }
    }

    /// True once after the lanes, the waveforms or the counters changed.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    fn shows(&self, t: usize) -> bool {
        let tr = &self.host.tracks;
        t < self.shown
            || self.panel == Some(t)
            || tr.armed(t)
            || !tr.regions(t).is_empty()
            || tr.recording(t).is_some()
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
        h.add(self.shown as u64);
        h.add(self.panel.map_or(u64::MAX, |t| t as u64));
        h.add(self.takes_open.map_or(u64::MAX, |t| t as u64));
        for t in 0..MAX_TRACKS {
            h.add(u64::from(tr.armed(t)));
            h.add(u64::from(self.volume[t].to_bits()));
            h.add(u64::from(self.pan[t].to_bits()));
            h.add(self.inputs[t] as u64);
            h.add(self.monitor[t] as u64);
            h.add(u64::from(self.mute[t]) | u64::from(self.solo[t]) << 1);
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
                name: format!("Audio {}", t + 1),
                color: audio_color(t),
                armed: tr.armed(t),
                input: self.inputs[t],
                level: self.meter[t],
                volume: self.volume[t],
                pan: self.pan[t],
                monitor: self.monitor[t],
                mute: self.mute[t],
                solo: self.solo[t],
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
                let n = track + 1;
                match (on, line_open) {
                    (false, _) => format!("audio {n} disarmed"),
                    (true, true) => format!("audio {n} armed: REC records it in song mode"),
                    (true, false) => format!(
                        "audio {n} armed, but the line input is off: turn on Prefs > Line input to record"
                    ),
                }
            }
            AudioAction::Add => {
                let next = (0..MAX_TRACKS)
                    .find(|&t| !self.shows(t))
                    .ok_or_else(|| format!("all {MAX_TRACKS} audio tracks are showing"))?;
                self.shown = next + 1;
                self.changed = true;
                format!("audio {} added", next + 1)
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
                format!("audio {}: region split", r.track + 1)
            }
            AudioAction::Erase(r) => {
                let reg = region(tr, r)?;
                self.send(Command::Erase {
                    track: r.track,
                    from: reg.start,
                    to: reg.end,
                })?;
                format!("audio {}: region erased (Undo brings it back)", r.track + 1)
            }
            AudioAction::Undo => {
                if (0..MAX_TRACKS).any(|t| tr.recording(t).is_some()) {
                    return Err("undo waits until the recording stops".into());
                }
                if tr.history().0 == 0 {
                    return Err("nothing to undo".into());
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
                        format!("Q{}: pick a region of audio {} first", knob + 1, t + 1)
                    })
                };
                match knob {
                    0 => {
                        let step = if fine { 0.1 } else { 0.5 };
                        let v = from_db(to_db(self.volume[t]) + step * ticks as f32, LEVEL_MAX_DB);
                        self.send(Command::SetLevel(t, v))?;
                        self.volume[t] = v;
                    }
                    1 => {
                        let step = if fine { 0.01 } else { 0.02 };
                        let v = (self.pan[t] + step * ticks as f32).clamp(-1.0, 1.0);
                        // Snap to the middle near it, so centre is easy to find.
                        let v = if v.abs() < 0.005 { 0.0 } else { v };
                        self.send(Command::SetPan(t, v))?;
                        self.pan[t] = v;
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
            AudioAction::Input { track, input } => {
                let n = self.track_no(track)?;
                if tr.recording(track).is_some() {
                    return Err(format!(
                        "audio {n} is recording: change its input after it stops"
                    ));
                }
                self.send(Command::SetInput(track, input))?;
                self.inputs[track] = input;
                self.changed = true;
                format!("audio {n} records {}", input_words(input))
            }
            AudioAction::Monitor { track, monitor } => {
                let n = self.track_no(track)?;
                self.send(Command::SetMonitor(track, monitor))?;
                self.monitor[track] = monitor;
                self.changed = true;
                format!("audio {n} monitor: {}", monitor_words(monitor))
            }
            AudioAction::Mute { track, on } => {
                let n = self.track_no(track)?;
                self.send(Command::Mute(track, on))?;
                self.mute[track] = on;
                self.changed = true;
                format!("audio {n} {}", if on { "muted" } else { "unmuted" })
            }
            AudioAction::Solo { track, on } => {
                let n = self.track_no(track)?;
                self.send(Command::Solo(track, on))?;
                self.solo[track] = on;
                self.changed = true;
                format!(
                    "audio {n} {}",
                    if on {
                        "soloed (among the audio tracks)"
                    } else {
                        "unsoloed"
                    }
                )
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
                    return Err(format!("audio {n} has no takes yet: REC records one"));
                }
                // The lanes go under the track's lane, where the panel
                // may sit.
                self.takes_open = Some(t);
                self.panel = None;
                self.changed = true;
                format!(
                    "audio {n}: {count} take{} under the lane: swipe across one to play that stretch, tap it to use it all",
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
                        "audio {}: comping waits until the recording stops",
                        k.track + 1
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
                    "audio {}: {name} plays from {} to {}",
                    k.track + 1,
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
                        "audio {}: moving waits until the recording stops",
                        r.track + 1
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
                    "audio {}: region moved to {} (Undo puts it back)",
                    r.track + 1,
                    bar_beat(to, t)
                )
            }
            AudioAction::UseTake(take) => {
                let k = tr
                    .take(take)
                    .ok_or_else(|| "that take is gone".to_string())?;
                if tr.recording(k.track).is_some() {
                    return Err(format!(
                        "audio {}: comping waits until the recording stops",
                        k.track + 1
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
                    "audio {}: {name} plays everywhere it has audio",
                    k.track + 1
                )
            }
            // The time engine keeps the pick (`TimeControl`).
            AudioAction::Pick(_) => String::new(),
        };
        Ok(line)
    }

    /// Track `t`'s number as the screen writes it, if there is one.
    fn track_no(&self, t: usize) -> Result<usize, String> {
        if t < MAX_TRACKS {
            Ok(t + 1)
        } else {
            Err(format!("there is no audio track {}", t + 1))
        }
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
    let track = format!("AUDIO {}", t + 1);
    let pos = |v: f32, max: f32| (to_db(v) - FLOOR_DB) / (max - FLOOR_DB);
    let mut pan = QLinkCard::new("Pan", &track, &pan_text(lane.pan), (lane.pan + 1.0) / 2.0);
    pan.bipolar = true;
    let level = QLinkCard::new(
        "Level",
        &track,
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

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_transport::{Command as T, State, Transport};
    use std::time::{Duration, Instant};

    const RATE: u32 = 44_100;

    struct Rig {
        t: Transport,
        b: Block,
        h: TrackHost,
        song: bool,
        frame: u64,
        /// The line input's tone on while true.
        live: bool,
        heard: f32,
    }

    impl Rig {
        fn new() -> Rig {
            Rig {
                t: Transport::new(RATE, 120.0).unwrap(),
                b: Block::new(),
                h: TrackHost::new(RATE, 10),
                song: true,
                frame: 0,
                live: false,
                heard: 0.0,
            }
        }

        fn period(&mut self) {
            let mut l = [0.0f32; PERIOD];
            if self.live {
                for (j, x) in l.iter_mut().enumerate() {
                    let g = self.frame + j as u64;
                    *x = 0.5 * (std::f32::consts::TAU * 441.0 * g as f32 / RATE as f32).sin();
                }
            }
            let r = [0.0f32; PERIOD];
            let (mut ol, mut or) = ([0.0f32; PERIOD], [0.0f32; PERIOD]);
            self.t.process(PERIOD as u32, &mut self.b);
            let map = *self.t.tempo_map();
            self.h
                .period(&self.b, &map, self.song, (&l, &r), (&mut ol, &mut or));
            self.heard = ol.iter().chain(or.iter()).fold(0.0, |m, s| m.max(s.abs()));
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
            let mut st = TimeState::default();
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
        // "+ Audio" shows track 1's lane; arming it with the line input off
        // says where to turn it on.
        assert_eq!(
            r.h.action(AudioAction::Add, &r.info(), true).unwrap(),
            "audio 1 added"
        );
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

    #[test]
    fn split_erase_and_undo_edit_the_region_and_refusals_say_why() {
        let mut r = Rig::new();
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
        r.periods(2);
        assert_eq!(
            r.h.action(AudioAction::Split(reg), &r.info(), true)
                .unwrap(),
            "audio 1: region split"
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
        // Every lane added: "+ Audio" says so.
        for _ in 1..MAX_TRACKS {
            r.h.action(AudioAction::Add, &r.info(), true).unwrap();
        }
        assert!(r
            .h
            .action(AudioAction::Add, &r.info(), true)
            .unwrap_err()
            .contains("all 8"));
        assert_eq!(
            r.h.host.tracks.stats().refused,
            0,
            "nothing reached the engine to be refused"
        );
    }

    #[test]
    fn the_q_links_turn_level_pan_gain_and_fades_and_the_cards_say_them() {
        let mut r = Rig::new();
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
        assert_eq!(turn(&mut r, 0, -12, false), "", "turns write no log line");
        turn(&mut r, 1, 10, false);
        turn(&mut r, 1, 5, true);
        turn(&mut r, 2, 7, false);
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
        // The level goes off at the floor and stops at +6 dB.
        turn(&mut r, 0, -200, false);
        assert_eq!(r.h.volume[0], 0.0);
        turn(&mut r, 0, 400, false);
        assert!((to_db(r.h.volume[0]) - 6.0).abs() < 1e-3);
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
        // Without a region, Q3 and Q4 ask for one; Q1 still turns.
        let bare = |knob| AudioAction::Turn {
            track: 0,
            region: None,
            knob,
            ticks: 1,
            fine: false,
        };
        let err = r.h.action(bare(2), &r.info(), true).unwrap_err();
        assert!(err.contains("pick a region of audio 1"), "{err}");
        assert!(r.h.action(bare(3), &r.info(), true).is_err());
        assert!(r.h.action(bare(0), &r.info(), true).is_ok());
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
        let mut r = Rig::new();
        let act = |r: &mut Rig, a| {
            let out = r.h.action(a, &r.info(), true);
            r.periods(2);
            out
        };
        // The panel opens on an empty track: its lane shows while it is
        // open, and the header toggles it.
        act(&mut r, AudioAction::Panel(2)).unwrap();
        let st = r.state();
        assert_eq!(st.audio_panel, Some(2));
        assert_eq!(st.audio.iter().map(|l| l.track).collect::<Vec<_>>(), [2]);
        assert!(r.h.take_changed());
        let knobs = audio_knobs(&st).unwrap();
        assert_eq!((knobs.track, knobs.region), (2, None));
        assert_eq!(knobs.title(), "Q-Links · Audio 3");
        let shown: Vec<_> = knobs.cards.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(shown, ["0.0 dB", "C", "pick one", "pick one"]);
        act(&mut r, AudioAction::Panel(2)).unwrap();
        assert_eq!(r.state().audio_panel, None);
        assert!(r.state().audio.is_empty(), "closed: the empty lane goes");
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
        // Mute silences the track; Solo of another track does too.
        act(&mut r, AudioAction::Mute { track: 0, on: true }).unwrap();
        r.periods(4);
        assert_eq!(r.heard, 0.0, "muted");
        act(
            &mut r,
            AudioAction::Mute {
                track: 0,
                on: false,
            },
        )
        .unwrap();
        act(&mut r, AudioAction::Solo { track: 1, on: true }).unwrap();
        r.periods(4);
        assert_eq!(r.heard, 0.0, "another track soloed");
        act(&mut r, AudioAction::Solo { track: 0, on: true }).unwrap();
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
        let mut r = Rig::new();
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
        assert_eq!(line, "audio 1: take 1 plays from 1.2 to 1.4");
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
        assert_eq!(line, "audio 1: take 1 plays everywhere it has audio");
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
        let mut r = Rig::new();
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
        assert_eq!(line, "audio 1: region moved to 3.1 (Undo puts it back)");
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
        // A bar on audio 1, audio 2 muted and set to the right input.
        act(&mut r, AudioAction::Arm { track: 0, on: true });
        act(&mut r, AudioAction::Mute { track: 1, on: true });
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
        assert_eq!(r.h.host.tracks.regions(0), &first[..]);
        assert!(r.h.host.tracks.regions(2).is_empty());
        assert!(!r.h.host.tracks.armed(2));
        let st = r.state();
        assert_eq!(st.audio.iter().map(|l| l.track).collect::<Vec<_>>(), [0, 1]);
        assert!(st.audio[1].mute && st.audio[1].input == Input::Right);
        assert!(!st.audio[0].mute && !st.audio[0].armed);
        // The waveform is read again for the take that came back.
        r.waves();
        assert_eq!(r.state().waves.len(), 1);
        r.command(T::Locate(0));
        r.command(T::Play);
        r.periods(100);
        assert!(r.heard > 0.3, "plays back: {}", r.heard);
        r.command(T::Stop);
        r.periods(4);

        // New project: empty, every setting as at start, no line.
        r.h.clear();
        r.periods(2);
        assert!(!r.h.opening());
        assert_eq!(r.h.take_note(), None);
        assert_eq!(r.h.host.tracks.takes().count(), 0);
        assert!(r.state().audio.is_empty());
        assert!(!r.h.mute[1] && r.h.inputs[1] == Input::Left);
    }

    #[test]
    fn the_level_bars_follow_the_input_while_armed_and_the_sound_and_fall_back() {
        let mut r = Rig::new();
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
        let mut r = Rig::new();
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
}
