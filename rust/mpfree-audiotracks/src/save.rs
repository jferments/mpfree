//! Audio tracks saved with a project.
//!
//! The text ([`to_text`], [`from_text`]) is the audio tracks' part of a
//! project (`parts/audiotracks.txt`): each track's settings and regions,
//! and each take's place, named by its audio file. Each take's audio is a
//! WAV file ([`wav`], [`from_wav`]: 32-bit float, mono or stereo, as
//! recorded) that the project keeps with its samples.
//!
//! ```text
//! mpfree-audiotracks 1
//! rate=44100
//! track=0 input=left monitor=auto level=1 pan=0 muted=0 solo=0
//! region start=0 end=3840 take=3 src=0 gain=1 fade_in=0 fade_out=0
//! take=3 track=0 group=1 pass=0 bpm=120 anchor=-12.5 frames=44100 channels=1 file=3f0c.wav
//! ```
//!
//! A line is `key=value` words; `file=` comes last and takes the rest of
//! the line. Unknown keys and lines are skipped, so a newer build's text
//! still loads. A region line belongs to the track line above it.
//!
//! Saving: [`crate::AudioTracks::save_file`] gives the settings, regions
//! and takes (allocates: not on the audio thread), and
//! [`crate::AudioTracks::read_take`] reads each take's audio a piece at a
//! time without allocating. Opening: [`crate::AudioTracks::load_take`]
//! puts each take back (its audio copies in over a few periods) and
//! [`crate::AudioTracks::load_track`] each track.

use mpfree_transport::time::Tick;

use crate::{Input, Monitor, Region, MAX_REGIONS, MAX_TAKES, MAX_TRACKS};

/// The first line of the text.
pub const FORMAT: &str = "mpfree-audiotracks 1";

/// A track as saved.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackFile {
    pub index: usize,
    pub input: Input,
    pub monitor: Monitor,
    pub level: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    /// In order, never overlapping.
    pub regions: Vec<Region>,
}

/// A take as saved.
#[derive(Clone, Debug, PartialEq)]
pub struct TakeFile {
    pub id: u16,
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
    /// Interleaved, `frames * channels` long (empty in the text).
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

fn flag(b: bool) -> u8 {
    u8::from(b)
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
/// `file` names it).
pub fn to_text(saved: &Saved) -> String {
    let mut s = format!("{FORMAT}\nrate={}\n", saved.rate);
    for t in &saved.tracks {
        s.push_str(&format!(
            "track={} input={} monitor={} level={} pan={} muted={} solo={}\n",
            t.index,
            input_word(t.input),
            monitor_word(t.monitor),
            t.level,
            t.pan,
            flag(t.muted),
            flag(t.solo)
        ));
        for r in &t.regions {
            s.push_str(&format!(
                "region start={} end={} take={} src={} gain={} fade_in={} fade_out={}\n",
                r.start, r.end, r.take, r.src, r.gain, r.fade_in, r.fade_out
            ));
        }
    }
    for k in &saved.takes {
        s.push_str(&format!(
            "take={} track={} group={} pass={} bpm={} anchor={} frames={} channels={} file={}\n",
            k.id, k.track, k.group, k.pass, k.bpm, k.anchor, k.frames, k.channels, k.file
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

fn bit(kv: &[(&str, &str)], key: &str) -> bool {
    get(kv, key) == Some("1")
}

/// The part from its text. The takes come back without audio: load each
/// one's `file`. Checks that regions are in order and name saved takes of
/// their own track.
pub fn from_text(text: &str) -> Result<Saved, String> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, l)) if l.trim_end() == FORMAT => {}
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
            saved.tracks.push(TrackFile {
                index,
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
                level: level(&kv, "level", n, 0.0, 2.0)?,
                pan: level(&kv, "pan", n, -1.0, 1.0)?,
                muted: bit(&kv, "muted"),
                solo: bit(&kv, "solo"),
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
                gain: level(&kv, "gain", n, 0.0, 4.0)?,
                fade_in: num(&kv, "fade_in", n).unwrap_or(0),
                fade_out: num(&kv, "fade_out", n).unwrap_or(0),
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
            saved.takes.push(TakeFile {
                id,
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
    let ch = channels.clamp(1, 2) as u16;
    let data = 4 * audio.len();
    let mut b = Vec::with_capacity(44 + data);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes());
    b.extend_from_slice(&ch.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 4 * u32::from(ch)).to_le_bytes());
    b.extend_from_slice(&(4 * ch).to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data as u32).to_le_bytes());
    for v in audio {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
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

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Saved {
        let region = |start, end, take| Region {
            start,
            end,
            take,
            src: -7,
            gain: 0.5,
            fade_in: 10,
            fade_out: 0,
        };
        let take = |id, track, file: &str| TakeFile {
            id,
            track,
            group: 2,
            pass: id,
            bpm: 97.5,
            anchor: -12.345_678_9,
            frames: 44_100,
            channels: 1 + id as usize % 2,
            file: file.to_string(),
            audio: Vec::new(),
        };
        Saved {
            rate: 44_100,
            tracks: vec![
                TrackFile {
                    index: 0,
                    input: Input::Stereo,
                    monitor: Monitor::Off,
                    level: 0.7,
                    pan: -0.25,
                    muted: true,
                    solo: false,
                    regions: vec![region(0, 1920, 0), region(1920, 3840, 1)],
                },
                TrackFile {
                    index: 3,
                    input: Input::Left,
                    monitor: Monitor::Auto,
                    level: 1.0,
                    pan: 0.0,
                    muted: false,
                    solo: true,
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
        assert!(t.starts_with("mpfree-audiotracks 1\n"));
        assert_eq!(from_text(&t), Ok(s));
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
        assert!(from_text(&t.replace("track=3 input", "track=8 input")).is_err());
        assert!(from_text(&t.replace(" file=d.wav", "")).is_err());
        assert!(from_text(&t.replace("start=1920", "start=1000")).is_err());
        assert!(from_text(&t.replace("take=1 src", "take=5 src")).is_err());
        let s = from_text(&t.replace("level=0.7", "level=9")).unwrap();
        assert_eq!(s.tracks[0].level, 2.0);
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
