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
//!   "+ Audio", Split (at the playhead), Erase and Undo. Each answer is a
//!   line for the log; a refusal says why.
//! - [`TrackHost::fill`] puts the lanes, the waveforms, the memory gauge
//!   and the counters into the time engine's [`TimeState`], for the Song
//!   view. [`TrackHost::take_changed`] says when they changed, checked a
//!   few times a second without allocating.
//! - Waveforms are read a take at a time with [`AudioHost::wave`], at a
//!   small fixed cost a period; a take that records is read again as it
//!   grows.
//!
//! The memory is set aside when MPFree starts ([`SECONDS`] of stereo). The
//! takes stay in RAM: streaming them from the stick waits for the stick
//! speed test (rule zero: it needs Jay's go).

use mpfree_audiotracks::host::{AudioHost, TakeWave, MAX_COLUMNS};
use mpfree_audiotracks::{Command, Input, TakeInfo, MAX_TRACKS, PEAK_BLOCK};
use mpfree_time_ui::song::{audio_blocks, AudioLane};
use mpfree_time_ui::view::{audio_color, AudioAction, RegionId, TimeState, TransportInfo};
use mpfree_transport::{Block, TempoMap, Tick};
use mpfree_ui::qlink::QLinkCard;

use crate::PERIOD;

/// Seconds of stereo the takes share: layout spec 4c's default, 60 s
/// (about 21 MB at 44.1 kHz). Prefs "Audio track memory" comes later.
pub const SECONDS: u32 = 60;
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
    /// Each track's level (0 to 2) and balance (-1 to 1), as last sent.
    volume: [f32; MAX_TRACKS],
    pan: [f32; MAX_TRACKS],
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
            active: false,
            latency: None,
            waves: Vec::with_capacity(mpfree_audiotracks::MAX_TAKES),
            reading: None,
            grown_at: 0,
            periods: 0,
            seen: 0,
            changed: false,
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
        self.periods += 1;
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

    /// The line input's latency, in frames: the looper's "Latency to make
    /// up for" (layout spec 4c: one line input latency, shared with the
    /// looper). Sent to the tracks when it changes; call it every period.
    pub fn follow_latency(&mut self, frames: u32) {
        if self.latency != Some(frames) && self.host.command(Command::SetLatency(frames)) {
            self.latency = Some(frames);
        }
    }

    /// True once after the lanes, the waveforms or the counters changed.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    fn shows(&self, t: usize) -> bool {
        let tr = &self.host.tracks;
        t < self.shown || tr.armed(t) || !tr.regions(t).is_empty() || tr.recording(t).is_some()
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
        for t in 0..MAX_TRACKS {
            h.add(u64::from(tr.armed(t)));
            h.add(u64::from(self.volume[t].to_bits()));
            h.add(u64::from(self.pan[t].to_bits()));
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
                input: input_chip(self.inputs[t]).into(),
                level: 0.0,
                volume: self.volume[t],
                pan: self.pan[t],
                blocks: audio_blocks(t, tr.regions(t), &takes, map, self.rate),
                recording: tr
                    .recording(t)
                    .and_then(|id| tr.take(id))
                    .map(|k| k.anchor.floor() as Tick),
            })
            .collect();
        st.waves = self.waves.clone();
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
                "audio tracks: undone".into()
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
                region: r,
                knob,
                ticks,
                fine,
            } => {
                let reg = region(tr, r)?;
                let t = r.track;
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
                        let step = if fine { 0.1 } else { 0.5 };
                        let gain = from_db(to_db(reg.gain) + step * ticks as f32, GAIN_MAX_DB);
                        self.send(Command::SetGain {
                            track: t,
                            at: reg.start,
                            gain,
                        })?;
                    }
                    3 => {
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
            AudioAction::Panel(_) | AudioAction::Takes(_) => {
                return Err(format!(
                    "{a:?} is not built yet (the track panel and the take lanes come next)"
                ))
            }
            // The time engine keeps the pick (`TimeControl`).
            AudioAction::Pick(_) => String::new(),
        };
        Ok(line)
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

/// The Q-Link cards for the audio region picked in `st` (layout spec 4c):
/// Q1 the track's level, Q2 its pan, Q3 the region's gain, Q4 its fades.
/// None when no region is picked or it is not in the lanes.
pub fn region_cards(st: &TimeState) -> Option<(RegionId, [QLinkCard; 4])> {
    let r = st.region?;
    let lane = st.audio.iter().find(|l| l.track == r.track)?;
    let b = lane.blocks.get(r.index)?;
    let track = format!("AUDIO {}", r.track + 1);
    let pos = |v: f32, max: f32| (to_db(v) - FLOOR_DB) / (max - FLOOR_DB);
    let ms = (b.fade_in.max(b.fade_out) as f64 / RATE_FOR_TEXT * 1000.0).round();
    let mut pan = QLinkCard::new("Pan", &track, &pan_text(lane.pan), (lane.pan + 1.0) / 2.0);
    pan.bipolar = true;
    Some((
        r,
        [
            QLinkCard::new(
                "Level",
                &track,
                &db_text(lane.volume),
                pos(lane.volume, LEVEL_MAX_DB),
            ),
            pan,
            QLinkCard::new("Gain", "REGION", &db_text(b.gain), pos(b.gain, GAIN_MAX_DB)),
            QLinkCard::new(
                "Fades",
                "REGION",
                &format!("{ms:.0} ms"),
                (ms / 2000.0).clamp(0.0, 1.0) as f32,
            ),
        ],
    ))
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

/// Columns for a take's waveform: one for every block of its peak table,
/// at most [`MAX_COLUMNS`].
fn columns(k: &TakeInfo) -> usize {
    (k.frames * k.channels / PEAK_BLOCK).clamp(1, MAX_COLUMNS)
}

/// The header's input chip.
pub fn input_chip(i: Input) -> &'static str {
    match i {
        Input::Left => "L",
        Input::Right => "R",
        Input::Sum => "L+R",
        Input::Stereo => "ST",
    }
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
        assert_eq!(st.audio[0].input, "L");
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
            "audio tracks: undone"
        );
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
        assert!(r
            .h
            .action(AudioAction::Takes(0), &r.info(), true)
            .unwrap_err()
            .contains("not built yet"));
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
                region,
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
        let (_, cards) = region_cards(&st).unwrap();
        let shown: Vec<_> = cards.iter().map(|c| c.value.as_str()).collect();
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
            region,
            knob: 4,
            ticks: 1,
            fine: false,
        };
        assert!(r.h.action(a, &r.info(), true).is_err());
        assert_eq!(r.h.host.tracks.stats().refused, 0);
        // No region picked: no cards.
        st.region = None;
        assert!(region_cards(&st).is_none());
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
