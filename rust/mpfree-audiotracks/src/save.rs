//! Audio tracks saved with a project.
//!
//! The text ([`to_text`], [`from_text`]) is the audio tracks' part of a
//! project (`parts/audiotracks.txt`): each track's input, monitor and
//! regions, and each take's place, named by its audio file. Each take's audio is a
//! WAV file ([`wav`], [`from_wav`]: 32-bit float, mono or stereo, as
//! recorded) that the project keeps with its samples.
//!
//! ```text
//! mpfree-audiotracks 2
//! rate=44100
//! track=0 id=9 input=left monitor=auto
//! region start=0 end=3840 take=3 src=0 gain=1 fade_in=0 fade_out=0 into=0 left=0
//! take=3 rec=5b2f0d9c41e8a7763c0f12ab9e4d8c01 track=0 group=1 pass=0 bpm=120 anchor=-12.5 frames=44100 channels=1 file=Vocal_5b2f0d9c41e8a7763c0f12ab9e4d8c01.wav
//! ```
//!
//! A line is `key=value` words; `file=` comes last and takes the rest of
//! the line. Unknown keys and lines are skipped, so a newer build's text
//! still loads. A region line belongs to the track line above it; it has
//! `phase=` after `src=` where it starts between frames of its take
//! ([`crate::Region::phase`]).
//!
//! A track line's `id` is the lasting id of its track in the project's
//! track list (the audio tracks rethink, phase 1: an audio track is a
//! track like any other); its `track` number only ties the takes to it in
//! this file. A track's level, pan, mute and solo aren't here: the level
//! and pan are its mixer strip's, and mute and solo its track's. That is
//! `mpfree-audiotracks 2`. An `mpfree-audiotracks 1` part, saved before
//! audio tracks were tracks, is refused in plain words: mpfree is early
//! enough that correctness comes before older files (Jay, 22 Nov).
//!
//! The take lines are the project's table of recordings (phase 1c): a
//! take line's `rec` is the recording's lasting id
//! ([`crate::RecordingId`], 32 hex digits), saved and never given to
//! another; its `take` number is only the engine place the regions above
//! name. Every recording is the project's own, its file in the project's
//! `samples/`, at the part's `rate`; recordings from outside the project,
//! and files at their own rates, come with streaming (the rethink's
//! phase 3b).
//!
//! A take's `file` is the name the host gives it as it saves
//! ([`crate::files`], phase 3a): a take recorded to a file of its own is
//! named for its track and its id (`NAME_ID.wav`), and its finished file
//! is copied in whole ([`TakeFile::path`], no audio read); a take opened
//! keeps its file's name; any other is `take_ID.wav`, written from its
//! samples.
//!
//! Saving: [`crate::AudioTracks::save_into`] fills a [`Saved::room`] made
//! off the audio thread with the settings, regions and takes without
//! allocating ([`crate::AudioTracks::save_file`] makes its own room: not
//! on the audio thread), and [`crate::AudioTracks::read_take`] reads each
//! take's audio a piece at a time without allocating. Opening: [`crate::AudioTracks::load_take`]
//! puts each take back (its audio copies in over a few periods) and
//! [`crate::AudioTracks::load_track`] each track.

use mpfree_transport::time::Tick;

use crate::{Input, Monitor, RecordingId, Region, MAX_REGIONS, MAX_TAKES, MAX_TRACKS};

/// The first line of the text.
pub const FORMAT: &str = "mpfree-audiotracks 2";
/// The first line of a part saved before audio tracks were tracks, which
/// is refused ([`OLDER`]).
pub const FORMAT_1: &str = "mpfree-audiotracks 1";

/// Why an [`FORMAT_1`] part is refused, in the musician's words.
pub const OLDER: &str =
    "saved by an older mpfree, before audio tracks were tracks; this mpfree doesn't open them";

/// A track as saved.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackFile {
    /// Its place among the engine's tracks: in the text, only what ties
    /// the takes to it.
    pub index: usize,
    /// The lasting id of its track in the project's track list, which the
    /// app fills in: 0 for none, as in what
    /// [`crate::AudioTracks::save_into`] gives. The text has no `id` for
    /// 0. Which tracks need one is the app's to check.
    pub id: u32,
    pub input: Input,
    pub monitor: Monitor,
    /// Where the track records from inside the mix, in the app's words
    /// (the instrument redesign's step 9): kept for the app, which fills
    /// it in. Empty: the line input. No spaces.
    pub from: String,
    /// In order, never overlapping.
    pub regions: Vec<Region>,
}

/// A take as saved.
#[derive(Clone, Debug, PartialEq)]
pub struct TakeFile {
    /// Its place in the engine, which the regions name.
    pub id: u16,
    /// Its lasting id: always given in the text; [`RecordingId::NONE`] for
    /// a new take (a sound brought in, a stretched copy), which gets one
    /// as it loads.
    pub rec: RecordingId,
    pub track: usize,
    pub group: u16,
    pub pass: u16,
    /// The tempo it was recorded at (0: not known).
    pub bpm: f64,
    pub anchor: f64,
    pub frames: usize,
    pub channels: usize,
    /// Its audio's file in the project; empty until named.
    pub file: String,
    /// Where its audio is as a finished file that holds all of it, when it
    /// has one (not in the text): a take recorded to its file and saved
    /// there, or one opened from a project. A save copies that file
    /// whole, and `audio` is then empty.
    pub path: Option<std::path::PathBuf>,
    /// Interleaved, `frames * channels` long (empty in the text, and when
    /// `path` gives it).
    pub audio: Vec<f32>,
}

/// The audio tracks' part of a project.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    /// The device rate the takes were recorded at.
    pub rate: u32,
    pub tracks: Vec<TrackFile>,
    pub takes: Vec<TakeFile>,
}

/// Room for a take's file name in a [`Saved::room`] (`NAME_ID.wav` is at
/// most 85 bytes: [`crate::files::NAME_MAX`] and the id's 32 digits).
pub const NAME_ROOM: usize = 96;

impl Saved {
    /// The takes, recorded at [`Saved::rate`], as at `rate`, so the song
    /// plays at its own speed and pitch: each take's audio converted
    /// through the windowed-sinc resampler ([`mpfree_stretch::convert_rate`],
    /// flat to 20 kHz from 48 to 44.1 kHz), its frames with it, and the
    /// frame counts of the regions playing it (where each starts in it,
    /// its fades) scaled to match. A converted take is a copy of its own:
    /// no file holds it yet (no `path` or name) and it gets a new lasting
    /// id when it goes in, as a stretched copy does. Takes with no audio
    /// here (not read) are left as they are, so read them all first. Heavy
    /// work for long takes (a minute of 48 kHz stereo is about 300 million
    /// multiplies): off the audio and screen threads.
    pub fn convert(&mut self, rate: u32) {
        let from = self.rate;
        if from == rate || from == 0 || rate == 0 {
            return;
        }
        let scale = |n: f64| (n * f64::from(rate) / f64::from(from)).round();
        for k in self.takes.iter_mut().filter(|k| !k.audio.is_empty()) {
            k.audio = mpfree_stretch::convert_rate(&k.audio, k.channels, from, rate);
            k.frames = k.audio.len() / k.channels.max(1);
            k.path = None;
            k.file.clear();
            k.rec = RecordingId::NONE;
            for g in self.tracks.iter_mut().flat_map(|t| &mut t.regions) {
                if g.take == k.id {
                    let x = (g.src as f64 + f64::from(g.phase)) * f64::from(rate) / f64::from(from);
                    g.src = x.round() as i64;
                    g.phase = (x - x.round()) as f32;
                    g.fade_in = scale(f64::from(g.fade_in)) as u32;
                    g.fade_out = scale(f64::from(g.fade_out)) as u32;
                    g.into = scale(f64::from(g.into)) as u32;
                    g.left = scale(f64::from(g.left)) as u32;
                }
            }
        }
        self.rate = rate;
    }

    /// Room for the most a save can hold: [`MAX_TRACKS`] tracks of
    /// [`MAX_REGIONS`] regions each and [`MAX_TAKES`] takes, each with room
    /// for a [`NAME_ROOM`] file name, so that
    /// [`crate::AudioTracks::save_into`] fills it on the audio thread
    /// without allocating. Allocates (about 120 KB): made on a worker.
    pub fn room(rate: u32) -> Saved {
        Saved {
            rate,
            tracks: (0..MAX_TRACKS)
                .map(|index| TrackFile {
                    index,
                    id: 0,
                    input: Input::Left,
                    monitor: Monitor::Auto,
                    from: String::new(),
                    regions: Vec::with_capacity(MAX_REGIONS),
                })
                .collect(),
            takes: (0..MAX_TAKES)
                .map(|id| TakeFile {
                    id: id as u16,
                    rec: RecordingId::NONE,
                    track: 0,
                    group: 0,
                    pass: 0,
                    bpm: 0.0,
                    anchor: 0.0,
                    frames: 0,
                    channels: 1,
                    file: String::with_capacity(NAME_ROOM),
                    path: None,
                    audio: Vec::new(),
                })
                .collect(),
        }
    }
}

fn input_word(i: Input) -> &'static str {
    match i {
        Input::Left => "left",
        Input::Right => "right",
        Input::Stereo => "stereo",
        Input::Sum => "sum",
    }
}

fn monitor_word(m: Monitor) -> &'static str {
    match m {
        Monitor::Off => "off",
        Monitor::Auto => "auto",
        Monitor::In => "in",
    }
}

/// The text for `saved` (the takes' audio is not in it: each take's
/// `file` names it). Every take must have its lasting id.
pub fn to_text(saved: &Saved) -> String {
    let mut s = format!("{FORMAT}\nrate={}\n", saved.rate);
    for t in &saved.tracks {
        s.push_str(&format!(
            "track={}{} input={} monitor={}\n",
            t.index,
            if t.id != 0 {
                format!(" id={}", t.id)
            } else {
                String::new()
            },
            input_word(t.input),
            monitor_word(t.monitor),
        ));
        if !t.from.is_empty() && !t.from.contains(char::is_whitespace) {
            s.pop();
            s.push_str(&format!(" from={}\n", t.from));
        }
        for r in &t.regions {
            // Where in its first frame it starts, only when not on it.
            let phase = if r.phase == 0.0 {
                String::new()
            } else {
                format!(" phase={}", r.phase)
            };
            s.push_str(&format!(
                "region start={} end={} take={} src={}{phase} gain={} fade_in={} fade_out={} into={} left={}\n",
                r.start, r.end, r.take, r.src, r.gain, r.fade_in, r.fade_out, r.into, r.left
            ));
        }
    }
    for k in &saved.takes {
        s.push_str(&format!(
            "take={} rec={} track={} group={} pass={} bpm={} anchor={} frames={} channels={} file={}\n",
            k.id, k.rec, k.track, k.group, k.pass, k.bpm, k.anchor, k.frames, k.channels, k.file
        ));
    }
    s
}

/// The `key=value` words of a line, and the `file=` rest if any.
fn words(line: &str) -> (Vec<(&str, &str)>, Option<&str>) {
    let (head, file) = match line.find(" file=") {
        Some(i) => (&line[..i], Some(&line[i + 6..])),
        None => (line, None),
    };
    let kv = head
        .split_whitespace()
        .filter_map(|w| w.split_once('='))
        .collect();
    (kv, file)
}

fn get<'a>(kv: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
    kv.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn num<T: std::str::FromStr>(kv: &[(&str, &str)], key: &str, line: usize) -> Result<T, String> {
    let v = get(kv, key).ok_or_else(|| format!("line {line}: no {key}"))?;
    v.parse()
        .map_err(|_| format!("line {line}: {key}={v} is not a number"))
}

fn level(kv: &[(&str, &str)], key: &str, line: usize, lo: f32, hi: f32) -> Result<f32, String> {
    let v: f32 = num(kv, key, line)?;
    if !v.is_finite() {
        return Err(format!("line {line}: {key} is not a number"));
    }
    Ok(v.clamp(lo, hi))
}

/// The part from its text. The takes come back without audio: load each
/// one's `file`. Checks that regions are in order and name saved takes of
/// their own track.
pub fn from_text(text: &str) -> Result<Saved, String> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, l)) if l.trim_end() == FORMAT => {}
        Some((_, l)) if l.trim_end() == FORMAT_1 => return Err(OLDER.into()),
        _ => return Err(format!("not an audio tracks part (no \"{FORMAT}\")")),
    }
    let mut saved = Saved {
        rate: 0,
        tracks: Vec::new(),
        takes: Vec::new(),
    };
    for (i, line) in lines {
        let n = i + 1;
        let line = line.trim_end();
        let (kv, file) = words(line);
        if line.starts_with("rate=") {
            saved.rate = num(&kv, "rate", n)?;
        } else if line.starts_with("track=") {
            let index: usize = num(&kv, "track", n)?;
            if index >= MAX_TRACKS || saved.tracks.iter().any(|t| t.index == index) {
                return Err(format!("line {n}: track {index} is past the last or twice"));
            }
            let id: u32 = match get(&kv, "id") {
                Some(_) => num(&kv, "id", n)?,
                None => 0,
            };
            if id != 0 && saved.tracks.iter().any(|t| t.id == id) {
                return Err(format!("line {n}: track id {id} twice"));
            }
            saved.tracks.push(TrackFile {
                index,
                id,
                input: match get(&kv, "input") {
                    Some("right") => Input::Right,
                    Some("stereo") => Input::Stereo,
                    Some("sum") => Input::Sum,
                    _ => Input::Left,
                },
                monitor: match get(&kv, "monitor") {
                    Some("off") => Monitor::Off,
                    Some("in") => Monitor::In,
                    _ => Monitor::Auto,
                },
                from: get(&kv, "from").unwrap_or("").to_string(),
                regions: Vec::new(),
            });
        } else if line.starts_with("region ") {
            let Some(t) = saved.tracks.last_mut() else {
                return Err(format!("line {n}: a region before any track"));
            };
            if t.regions.len() >= MAX_REGIONS {
                return Err(format!("line {n}: more than {MAX_REGIONS} regions"));
            }
            let r = Region {
                start: num::<Tick>(&kv, "start", n)?,
                end: num::<Tick>(&kv, "end", n)?,
                take: num(&kv, "take", n)?,
                src: num(&kv, "src", n)?,
                phase: match get(&kv, "phase") {
                    None => 0.0,
                    Some(_) => level(&kv, "phase", n, -0.5, 0.5)?,
                },
                gain: level(&kv, "gain", n, 0.0, 4.0)?,
                fade_in: num(&kv, "fade_in", n).unwrap_or(0),
                fade_out: num(&kv, "fade_out", n).unwrap_or(0),
                into: num(&kv, "into", n).unwrap_or(0),
                left: num(&kv, "left", n).unwrap_or(0),
            };
            if r.end <= r.start || t.regions.last().is_some_and(|p| p.end > r.start) {
                return Err(format!("line {n}: a region empty or out of order"));
            }
            t.regions.push(r);
        } else if line.starts_with("take=") {
            let id: u16 = num(&kv, "take", n)?;
            if id as usize >= MAX_TAKES || saved.takes.iter().any(|k| k.id == id) {
                return Err(format!("line {n}: take {id} is past the last or twice"));
            }
            let track: usize = num(&kv, "track", n)?;
            let channels: usize = num(&kv, "channels", n)?;
            let frames: usize = num(&kv, "frames", n)?;
            let anchor: f64 = num(&kv, "anchor", n)?;
            if track >= MAX_TRACKS
                || !(1..=2).contains(&channels)
                || frames == 0
                || !anchor.is_finite()
            {
                return Err(format!("line {n}: a bad take"));
            }
            let file = file.unwrap_or("").to_string();
            if file.is_empty() {
                return Err(format!("line {n}: a take with no file"));
            }
            let v = get(&kv, "rec").ok_or_else(|| format!("line {n}: no rec"))?;
            let rec = RecordingId::parse(v)
                .ok_or_else(|| format!("line {n}: rec={v} is not a recording id"))?;
            if saved.takes.iter().any(|k| k.rec == rec) {
                return Err(format!("line {n}: recording {rec} twice"));
            }
            saved.takes.push(TakeFile {
                id,
                rec,
                track,
                group: num(&kv, "group", n)?,
                pass: num(&kv, "pass", n)?,
                bpm: num(&kv, "bpm", n)
                    .ok()
                    .filter(|b: &f64| b.is_finite() && *b > 0.0)
                    .unwrap_or(0.0),
                anchor,
                frames,
                channels,
                file,
                path: None,
                audio: Vec::new(),
            });
        }
    }
    for t in &saved.tracks {
        for r in &t.regions {
            if !saved
                .takes
                .iter()
                .any(|k| k.id == r.take && k.track == t.index)
            {
                return Err(format!(
                    "track {}: a region plays take {}, which is not one of its takes",
                    t.index, r.take
                ));
            }
        }
    }
    Ok(saved)
}

/// `audio` (interleaved, `channels` of them) as a 32-bit float WAV at
/// `rate`.
pub fn wav(audio: &[f32], channels: usize, rate: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity(44 + 4 * audio.len());
    write_wav(&mut b, audio, channels, rate).expect("a Vec takes every write");
    b
}

/// [`wav`] written to `w` a piece at a time, without a copy of the whole
/// file (a project's save writes each take this way).
pub fn write_wav(
    w: &mut dyn std::io::Write,
    audio: &[f32],
    channels: usize,
    rate: u32,
) -> std::io::Result<()> {
    let ch = channels.clamp(1, 2) as u16;
    let data = 4 * audio.len();
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data as u32).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&3u16.to_le_bytes());
    h.extend_from_slice(&ch.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * 4 * u32::from(ch)).to_le_bytes());
    h.extend_from_slice(&(4 * ch).to_le_bytes());
    h.extend_from_slice(&32u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&(data as u32).to_le_bytes());
    w.write_all(&h)?;
    let mut buf = [0u8; 4096];
    for piece in audio.chunks(buf.len() / 4) {
        for (o, v) in buf.chunks_exact_mut(4).zip(piece) {
            o.copy_from_slice(&v.to_le_bytes());
        }
        w.write_all(&buf[..4 * piece.len()])?;
    }
    Ok(())
}

/// A WAV file [`wav`] wrote: its audio (interleaved), channels and rate.
/// Other kinds of WAV are refused.
pub fn from_wav(b: &[u8]) -> Result<(Vec<f32>, usize, u32), String> {
    let u16_at = |i: usize| b.get(i..i + 2).map(|x| u16::from_le_bytes([x[0], x[1]]));
    let u32_at = |i: usize| {
        b.get(i..i + 4)
            .map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
    };
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let mut fmt = None;
    let mut i = 12;
    while let (Some(id), Some(len)) = (b.get(i..i + 4), u32_at(i + 4)) {
        let (start, len) = (i + 8, len as usize);
        let end = start.checked_add(len).filter(|&e| e <= b.len());
        match id {
            b"fmt " => {
                let (Some(tag), Some(ch), Some(rate), Some(bits)) = (
                    u16_at(start),
                    u16_at(start + 2),
                    u32_at(start + 4),
                    u16_at(start + 14),
                ) else {
                    return Err("a short fmt chunk".into());
                };
                fmt = Some((tag, ch, rate, bits));
            }
            b"data" => {
                let Some(end) = end else {
                    return Err("the audio is cut short".into());
                };
                let Some((3, ch @ (1 | 2), rate, 32)) = fmt else {
                    return Err("not 32-bit float mono or stereo".into());
                };
                let audio = b[start..end]
                    .chunks_exact(4)
                    .map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]]))
                    .map(|v| if v.is_finite() { v } else { 0.0 })
                    .collect();
                return Ok((audio, ch as usize, rate));
            }
            _ => {}
        }
        i = start + len + (len & 1);
    }
    Err("no audio in the file".into())
}

/// Where a 32-bit float WAV's audio is, from its first bytes (at least
/// up to the `data` chunk's header): (byte offset of the audio, its
/// length in bytes as the header says, channels, rate). For streaming a
/// take file without reading it all.
pub fn wav_layout(b: &[u8]) -> Result<(u64, u64, usize, u32), String> {
    let u16_at = |i: usize| b.get(i..i + 2).map(|x| u16::from_le_bytes([x[0], x[1]]));
    let u32_at = |i: usize| {
        b.get(i..i + 4)
            .map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
    };
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let mut fmt = None;
    let mut i = 12;
    while let (Some(id), Some(len)) = (b.get(i..i + 4), u32_at(i + 4)) {
        let start = i + 8;
        match id {
            b"fmt " => {
                let (Some(tag), Some(ch), Some(rate), Some(bits)) = (
                    u16_at(start),
                    u16_at(start + 2),
                    u32_at(start + 4),
                    u16_at(start + 14),
                ) else {
                    return Err("a short fmt chunk".into());
                };
                fmt = Some((tag, ch, rate, bits));
            }
            b"data" => {
                let Some((3, ch @ (1 | 2), rate, 32)) = fmt else {
                    return Err("not 32-bit float mono or stereo".into());
                };
                return Ok((start as u64, u64::from(len), ch as usize, rate));
            }
            _ => {}
        }
        i = start + len as usize + (len as usize & 1);
    }
    Err("no audio header in the first bytes".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Saved {
        let region = |start, end, take| Region {
            start,
            end,
            take,
            src: -7,
            phase: 0.0,
            gain: 0.5,
            fade_in: 10,
            fade_out: 0,
            into: 3,
            left: 0,
        };
        let take = |id, track, file: &str| TakeFile {
            id,
            rec: RecordingId(0xabc0_0000_0000_0000_0000_0000_0000_0000 + u128::from(id)),
            track,
            group: 2,
            pass: id,
            bpm: 97.5,
            anchor: -12.345_678_9,
            frames: 44_100,
            channels: 1 + id as usize % 2,
            file: file.to_string(),
            path: None,
            audio: Vec::new(),
        };
        Saved {
            rate: 44_100,
            tracks: vec![
                TrackFile {
                    index: 0,
                    id: 12,
                    input: Input::Stereo,
                    monitor: Monitor::Off,
                    from: "t12:after".into(),
                    regions: vec![region(0, 1920, 0), region(1920, 3840, 1)],
                },
                TrackFile {
                    index: 3,
                    id: 4,
                    input: Input::Left,
                    monitor: Monitor::Auto,
                    from: String::new(),
                    regions: vec![],
                },
            ],
            takes: vec![
                take(0, 0, "a b.wav"),
                take(1, 0, "c.wav"),
                take(5, 3, "d.wav"),
            ],
        }
    }

    #[test]
    fn the_text_comes_back_as_it_was() {
        let s = saved();
        let t = to_text(&s);
        assert!(t.starts_with("mpfree-audiotracks 2\n"));
        assert!(t.contains("\ntrack=3 id=4 input=left "), "{t}");
        assert_eq!(from_text(&t), Ok(s));
    }

    /// Takes recorded at another rate come out at the one asked for: each
    /// read take converted (its sound the same tone at the new rate), its
    /// frames and the frame counts of its regions scaled (and where between
    /// frames a region starts), and no file or
    /// lasting id kept for it; a take not read is left as it was.
    #[test]
    fn takes_at_another_rate_are_converted_with_their_regions() {
        let mut s = saved();
        // Take 0: a second of a 441 Hz tone, mono. Take 1 isn't read.
        let tone = |rate: f64, frames: usize| -> Vec<f32> {
            (0..frames)
                .map(|n| (std::f64::consts::TAU * 441.0 * n as f64 / rate).sin() as f32 * 0.5)
                .collect()
        };
        s.takes[0].audio = tone(44_100.0, 44_100);
        s.tracks[0].regions[0].src = 4410;
        s.tracks[0].regions[0].phase = 0.25;
        s.tracks[0].regions[0].fade_out = 441;
        s.tracks[0].regions[0].left = 44_100;
        let before = s.clone();
        s.convert(48_000);
        assert_eq!(s.rate, 48_000);
        let k = &s.takes[0];
        assert_eq!((k.frames, k.audio.len()), (48_000, 48_000));
        assert!(k.path.is_none() && k.file.is_empty() && k.rec == RecordingId::NONE);
        assert_eq!(
            (k.anchor, k.bpm, k.channels),
            (before.takes[0].anchor, 97.5, 1)
        );
        // The same tone, away from the ends where the filter starts.
        let want = tone(48_000.0, 48_000);
        let worst = (1000..47_000)
            .map(|n| (k.audio[n] - want[n]).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "{worst}");
        let g = &s.tracks[0].regions[0];
        assert_eq!(
            (g.src, g.fade_in, g.fade_out, g.into, g.left),
            (4800, 11, 480, 3, 48_000)
        );
        // Its place between frames too: 4410.25 frames are 4800.27.
        let x = 4410.25 * 48_000.0 / 44_100.0;
        assert!(
            (g.src as f64 + f64::from(g.phase) - x).abs() < 1e-4,
            "{g:?}"
        );
        assert_eq!((g.start, g.end, g.gain), (0, 1920, 0.5));
        // Take 1 (no sound here) and its region stay as they were.
        assert_eq!(s.takes[1], before.takes[1]);
        assert_eq!(s.tracks[0].regions[1], before.tracks[0].regions[1]);
        // The same rate changes nothing.
        let again = s.clone();
        s.convert(48_000);
        assert_eq!(s, again);
    }

    /// A part saved before audio tracks were tracks is refused in plain
    /// words; a track's id is never there twice, and a track line holds
    /// no level, pan, mute or solo (its strip's and its track's).
    #[test]
    fn a_part_from_before_audio_tracks_were_tracks_is_refused() {
        let v2 = to_text(&saved());
        let old = v2.replace("mpfree-audiotracks 2", "mpfree-audiotracks 1");
        assert_eq!(from_text(&old), Err(OLDER.to_string()));
        assert_eq!(from_text(&v2.replace(" id=4", "")).unwrap().tracks[1].id, 0);
        assert!(from_text(&v2.replace(" id=4", " id=x")).is_err());
        assert!(from_text(&v2.replace(" id=4", " id=12")).is_err());
        for word in ["level=", "pan=", "muted=", "solo="] {
            assert!(!v2.contains(word), "{v2}");
        }
    }

    /// The take lines are the project's recordings: each keeps its lasting
    /// id, which every line has; an id is 32 hex digits, not 0, and never
    /// there twice.
    #[test]
    fn each_recording_keeps_its_lasting_id() {
        let t = to_text(&saved());
        assert!(
            t.contains("\ntake=5 rec=abc00000000000000000000000000005 track=3 "),
            "{t}"
        );
        let s = from_text(&t).unwrap();
        assert_eq!(s.takes[2].rec, RecordingId(0xabc << 116 | 5));
        let none = t.replace(" rec=abc00000000000000000000000000001", "");
        assert!(from_text(&none).unwrap_err().contains("no rec"));
        let one = "abc00000000000000000000000000001";
        for bad in [
            "abc",
            "zbc00000000000000000000000000001",
            "00000000000000000000000000000000",
        ] {
            let e = from_text(&t.replace(one, bad)).unwrap_err();
            assert!(e.contains("is not a recording id"), "{e}");
        }
        let e = from_text(&t.replace(one, "abc00000000000000000000000000000")).unwrap_err();
        assert!(e.contains("twice"), "{e}");
        assert_eq!(
            RecordingId::parse(one).map(|r| r.to_string()),
            Some(one.into())
        );
    }

    #[test]
    fn unknown_words_and_lines_are_skipped() {
        let t = to_text(&saved())
            .replace("rate=44100", "rate=44100 colour=red")
            .replace("\nregion ", "\nsomething new\nregion warp=on ");
        assert_eq!(from_text(&t), Ok(saved()));
    }

    #[test]
    fn bad_text_is_refused_with_its_line() {
        assert!(from_text("hello").is_err());
        let t = to_text(&saved());
        let e = from_text(&t.replace(
            "frames=44100 channels=1 file=a b.wav",
            "frames=lots channels=1 file=a b.wav",
        ))
        .unwrap_err();
        assert!(e.contains("line 7"), "{e}");
        assert!(from_text(&t.replace("track=3", "track=0")).is_err());
        assert!(from_text(&t.replace("track=3 id", "track=8 id")).is_err());
        assert!(from_text(&t.replace(" file=d.wav", "")).is_err());
        assert!(from_text(&t.replace("start=1920", "start=1000")).is_err());
        assert!(from_text(&t.replace("take=1 src", "take=5 src")).is_err());
    }

    #[test]
    fn a_wav_comes_back_sample_for_sample() {
        let audio: Vec<f32> = (0..2001).map(|i| (i as f32 * 0.37).sin()).collect();
        let b = wav(&audio, 1, 48_000);
        assert_eq!(from_wav(&b), Ok((audio.clone(), 1, 48_000)));
        let b = wav(&audio[..2000], 2, 44_100);
        assert_eq!(from_wav(&b), Ok((audio[..2000].to_vec(), 2, 44_100)));
        assert!(from_wav(&b[..100]).is_err());
        assert!(from_wav(b"RIFF\0\0\0\0WAVE").is_err());
        let mut pcm = b.clone();
        pcm[20] = 1;
        assert!(from_wav(&pcm).is_err());
    }
}
