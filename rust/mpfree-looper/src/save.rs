//! Loops saved with a project.
//!
//! A saved loop is its settings, each layer's mix, and each layer's audio
//! as it plays: through the loop's edits and their seam fades, at gain 1,
//! the loop's length. [`crate::Looper::loop_file`] gives a playing loop's
//! settings and mixes, [`crate::Looper::read_layer`] its layers' audio a
//! piece at a time, and [`crate::Looper::load`] puts a loop back.
//!
//! The text ([`to_text`], [`from_text`]) is the looper's part of a project
//! (`parts/looper.txt`); each layer's audio is a WAV file ([`wav`],
//! [`from_wav`]) the project keeps with its samples, named in the text.
//!
//! ```text
//! mpfree-looper 1
//! rate=44100 selected=0
//! loop=0 frames=88200 ticks=3840 origin=0 level=1 muted=0 bars=1 after=play fade_old=1
//! layer level=1 pan=0 muted=0 solo=0 faded=1 file=3f0c...e1.wav
//! ```
//!
//! A line is `key=value` words; `file=` comes last and takes the rest of
//! the line. Unknown keys and lines are skipped, so a newer build's text
//! still loads.

use mpfree_transport::time::Tick;

use crate::{After, MAX_LAYERS, MAX_LOOPS};

/// The first line of the text.
pub const FORMAT: &str = "mpfree-looper 1";

/// A loop as saved: its settings and its layers in play order.
#[derive(Clone, Debug, PartialEq)]
pub struct LoopFile {
    /// Frames in one pass, and its length in ticks and start in the song.
    pub frames: usize,
    pub ticks: Tick,
    pub origin: Tick,
    pub level: f32,
    pub muted: bool,
    /// The bars setting for the next first pass (None: free length).
    pub bars: Option<u32>,
    pub after: After,
    pub fade_old: f32,
    pub layers: Vec<LayerFile>,
}

/// A layer as saved.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerFile {
    pub level: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    /// How far Fade old had turned it down (1: not at all).
    pub faded: f32,
    /// Its audio's file in the project (`samples/...`); empty until named.
    pub file: String,
    /// Interleaved stereo, the loop's `frames` long, as it plays (empty in
    /// the text).
    pub audio: Vec<f32>,
}

/// The looper's part of a project: the device rate the loops were made
/// at, the loop selected, and each loop by number.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub rate: u32,
    pub selected: usize,
    pub loops: Vec<(usize, LoopFile)>,
}

fn flag(b: bool) -> u8 {
    u8::from(b)
}

/// The text for `saved` (the layers' audio is not in it: each layer's
/// `file` names it).
pub fn to_text(saved: &Saved) -> String {
    let mut s = format!(
        "{FORMAT}\nrate={} selected={}\n",
        saved.rate, saved.selected
    );
    for (n, l) in &saved.loops {
        let bars = l.bars.map_or("free".to_string(), |b| b.to_string());
        let after = match l.after {
            After::Play => "play",
            After::Overdub => "overdub",
        };
        s.push_str(&format!(
            "loop={n} frames={} ticks={} origin={} level={} muted={} bars={bars} after={after} fade_old={}\n",
            l.frames,
            l.ticks,
            l.origin,
            l.level,
            flag(l.muted),
            l.fade_old
        ));
        for y in &l.layers {
            s.push_str(&format!(
                "layer level={} pan={} muted={} solo={} faded={} file={}\n",
                y.level,
                y.pan,
                flag(y.muted),
                flag(y.solo),
                y.faded,
                y.file
            ));
        }
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

/// The part from its text. The layers come back without audio: load each
/// one's `file`.
pub fn from_text(text: &str) -> Result<Saved, String> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, l)) if l.trim_end() == FORMAT => {}
        _ => return Err(format!("not a looper part (no \"{FORMAT}\")")),
    }
    let mut saved = Saved {
        rate: 0,
        selected: 0,
        loops: Vec::new(),
    };
    for (i, line) in lines {
        let n = i + 1;
        let (kv, file) = words(line.trim_end());
        if let Some(r) = get(&kv, "rate") {
            saved.rate = r.parse().map_err(|_| format!("line {n}: rate={r}"))?;
            saved.selected = get(&kv, "selected")
                .and_then(|v| v.parse().ok())
                .filter(|&s: &usize| s < MAX_LOOPS)
                .unwrap_or(0);
        } else if get(&kv, "loop").is_some() {
            let lp: usize = num(&kv, "loop", n)?;
            if lp >= MAX_LOOPS || saved.loops.iter().any(|(k, _)| *k == lp) {
                return Err(format!("line {n}: loop {lp} is past the last or twice"));
            }
            let frames: usize = num(&kv, "frames", n)?;
            let ticks: Tick = num(&kv, "ticks", n)?;
            if frames == 0 || ticks <= 0 {
                return Err(format!("line {n}: an empty loop"));
            }
            let bars = match get(&kv, "bars") {
                Some("free") | None => None,
                Some(b) => Some(
                    b.parse::<u32>()
                        .map_err(|_| format!("line {n}: bars={b}"))?
                        .max(1),
                ),
            };
            saved.loops.push((
                lp,
                LoopFile {
                    frames,
                    ticks,
                    origin: num(&kv, "origin", n)?,
                    level: level(&kv, "level", n, 0.0, 2.0)?,
                    muted: bit(&kv, "muted"),
                    bars,
                    after: match get(&kv, "after") {
                        Some("overdub") => After::Overdub,
                        _ => After::Play,
                    },
                    fade_old: level(&kv, "fade_old", n, 0.0, 1.0).unwrap_or(1.0),
                    layers: Vec::new(),
                },
            ));
        } else if line.starts_with("layer") {
            let Some((_, l)) = saved.loops.last_mut() else {
                return Err(format!("line {n}: a layer before any loop"));
            };
            if l.layers.len() >= MAX_LAYERS - 1 {
                return Err(format!("line {n}: more than {} layers", MAX_LAYERS - 1));
            }
            let file = file.unwrap_or("").to_string();
            if file.is_empty() {
                return Err(format!("line {n}: a layer with no file"));
            }
            l.layers.push(LayerFile {
                level: level(&kv, "level", n, 0.0, 2.0)?,
                pan: level(&kv, "pan", n, -1.0, 1.0)?,
                muted: bit(&kv, "muted"),
                solo: bit(&kv, "solo"),
                faded: level(&kv, "faded", n, 0.0, 1.0).unwrap_or(1.0),
                file,
                audio: Vec::new(),
            });
        }
    }
    if let Some((n, _)) = saved.loops.iter().find(|(_, l)| l.layers.is_empty()) {
        return Err(format!("loop {n} has no layers"));
    }
    Ok(saved)
}

/// `audio` (interleaved stereo) as a WAV file: 32-bit float, at `rate`.
/// The sampler reads it too.
pub fn wav(audio: &[f32], rate: u32) -> Vec<u8> {
    let data = 4 * audio.len();
    let mut b = Vec::with_capacity(44 + data);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 8).to_le_bytes());
    b.extend_from_slice(&8u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data as u32).to_le_bytes());
    for v in audio {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

/// A WAV file [`wav`] wrote: its audio (interleaved stereo) and rate.
/// Other kinds of WAV are refused.
pub fn from_wav(b: &[u8]) -> Result<(Vec<f32>, u32), String> {
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
                if fmt.map(|(t, c, _, bits)| (t, c, bits)) != Some((3, 2, 32)) {
                    return Err("not 32-bit float stereo".into());
                }
                let rate = fmt.map_or(0, |f| f.2);
                let audio = b[start..end]
                    .chunks_exact(4)
                    .map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]]))
                    .map(|v| if v.is_finite() { v } else { 0.0 })
                    .collect();
                return Ok((audio, rate));
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
        let layer = |level: f32, file: &str| LayerFile {
            level,
            pan: -0.25,
            muted: false,
            solo: true,
            faded: 0.88,
            file: file.to_string(),
            audio: Vec::new(),
        };
        Saved {
            rate: 44_100,
            selected: 2,
            loops: vec![
                (
                    0,
                    LoopFile {
                        frames: 88_200,
                        ticks: 3840,
                        origin: -960,
                        level: 0.7,
                        muted: true,
                        bars: None,
                        after: After::Overdub,
                        fade_old: 0.9,
                        layers: vec![layer(1.0, "a b.wav"), layer(0.1, "c.wav")],
                    },
                ),
                (
                    2,
                    LoopFile {
                        frames: 7,
                        ticks: 1,
                        origin: 0,
                        level: 1.0,
                        muted: false,
                        bars: Some(4),
                        after: After::Play,
                        fade_old: 1.0,
                        layers: vec![layer(1.0 / 3.0, "d.wav")],
                    },
                ),
            ],
        }
    }

    #[test]
    fn the_text_comes_back_as_it_was() {
        let s = saved();
        let t = to_text(&s);
        assert!(t.starts_with("mpfree-looper 1\n"));
        assert_eq!(from_text(&t), Ok(s));
    }

    #[test]
    fn unknown_words_and_lines_are_skipped() {
        let t = to_text(&saved())
            .replace("rate=", "colour=red rate=")
            .replace("\nlayer ", "\nsomething new\nlayer warp=on ");
        assert_eq!(from_text(&t), Ok(saved()));
    }

    #[test]
    fn bad_text_is_refused_with_its_line() {
        assert!(from_text("hello").is_err());
        let t = to_text(&saved());
        let e = from_text(&t.replace("frames=88200", "frames=lots")).unwrap_err();
        assert!(e.contains("line 3"), "{e}");
        assert!(from_text(&t.replace("loop=2", "loop=0")).is_err());
        assert!(from_text(&t.replace("loop=2", "loop=8")).is_err());
        assert!(from_text(&t.replace(" file=d.wav", "")).is_err());
        assert!(from_text(&t.replace("ticks=1 ", "ticks=0 ")).is_err());
        // A loop with no layers.
        let cut = t.lines().take(6).collect::<Vec<_>>().join("\n");
        assert!(from_text(&cut).unwrap_err().contains("loop 2"));
        // Out-of-range mixes are kept in range.
        let s = from_text(&t.replace("level=0.7", "level=9")).unwrap();
        assert_eq!(s.loops[0].1.level, 2.0);
    }

    #[test]
    fn a_wav_comes_back_sample_for_sample() {
        let audio: Vec<f32> = (0..2000).map(|i| (i as f32 * 0.37).sin()).collect();
        let b = wav(&audio, 48_000);
        assert_eq!(from_wav(&b), Ok((audio, 48_000)));
        assert!(from_wav(&b[..100]).is_err());
        assert!(from_wav(b"RIFF\0\0\0\0WAVE").is_err());
        let mut mono = b.clone();
        mono[22] = 1;
        assert!(from_wav(&mono).is_err());
    }
}
