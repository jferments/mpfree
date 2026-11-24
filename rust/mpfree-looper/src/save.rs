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
//! A loop set to start when you play ([`crate::Starts::Play`]) adds
//! `starts=play` to its line; one that starts on the bar leaves it out.
//!
//! A loop at another speed ([`crate::Edit::Speed`]) adds `speed=` and
//! `octaves=` to its line (its speed, and the octaves its audio plays
//! faster than it is, [`crate::Status::rate`]); a loop as recorded leaves
//! them out.
//!
//! A loop on an audio track (audio tracks phase 4, step 4b) adds `id=` to
//! its line: its lasting id as its host gave it (`mpfree_seq::LoopId`), 32
//! hex digits, never 0. The looper keeps it only in the text.
//!
//! A free loop ([`crate::Clock::Free`]) adds `clock=free` to its line, and
//! the part then ends with the free clock's line: its exact tempo, its
//! meter and its bar 1, which the free loops' origins count from
//! ([`ClockFile`]). A song loop leaves `clock=` out.
//!
//! ```text
//! free-clock bpm=93.40000000000001 beats=4 unit=4 top=0
//! ```
//!
//! Format 2 (`mpfree-looper 2`, audio tracks phase 4c, written only for
//! loops kept as the looper holds them: [`LoopFile::raw`]) keeps each
//! layer's audio as written, not as it plays, with how it plays it, so a
//! loop reopens exactly and its edits can be taken further. Its loop line
//! adds `seam=`, `region=` (or `-`), `captured=` and `cut=`; each layer
//! line adds `size=` (frames of its audio, which its file holds),
//! `valid=` (the first frame written and how many), `view=` (its pieces,
//! `start:len` with `:b` for one played backwards, joined by `+`) and
//! `replace=` (a Replace layer's fades in and out, or `-`):
//!
//! ```text
//! mpfree-looper 2
//! rate=44100 selected=0
//! loop=0 frames=88200 ticks=3840 origin=0 level=1 muted=0 bars=1 after=play fade_old=1 seam=256 region=1536,1536 captured=1536,89736 cut=88200
//! layer level=1 pan=0 muted=0 solo=0 faded=1 size=92808 valid=0,92808 view=1536:88200 replace=- file=3f0c...e1.wav
//! ```
//!
//! A line is `key=value` words; `file=` comes last and takes the rest of
//! the line. Unknown keys and lines are skipped, so a newer build's text
//! still loads.

use mpfree_transport::time::Tick;
use mpfree_transport::Meter;

use crate::{After, Clock, FreeClock, Starts, MAX_LAYERS, MAX_LOOPS};

/// The first line of the text.
pub const FORMAT: &str = "mpfree-looper 1";
/// The first line of format 2: loops kept as the looper holds them.
pub const FORMAT_2: &str = "mpfree-looper 2";

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
    /// When its next first pass starts.
    pub starts: Starts,
    pub fade_old: f32,
    /// Its speed ([`crate::Status::speed`]) and the octaves its layers'
    /// audio plays faster than it is ([`crate::Status::rate`]).
    pub speed: i8,
    pub rate: i8,
    /// Its lasting id, its host's (`mpfree_seq::LoopId`), 0 for none:
    /// the looper keeps it for the host in the text, nothing more.
    pub id: u128,
    /// The clock it keeps time with: a free loop's `ticks` and `origin`
    /// are the free clock's ([`Saved::free_clock`]).
    pub clock: Clock,
    pub layers: Vec<LayerFile>,
    /// Its shape as the looper holds it (format 2), with each layer's
    /// [`LayerFile::raw`]; None: its layers as they play (format 1).
    pub raw: Option<RawShape>,
}

/// A loop's shape as the looper holds it (format 2), beyond its length,
/// place and speed: what its layers' views need to play the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawShape {
    /// Frames of its loop point's crossfade.
    pub seam: usize,
    /// The first pass's loop in the audio it kept (its first frame, and
    /// the one on bar 1) while layer 0 plays that region alone.
    pub region: Option<(usize, usize)>,
    /// The frames of the first pass's audio that were its loop, as
    /// recorded (for Move, Resize and the smart end).
    pub captured: (usize, usize),
    /// Frames of the first pass that hold its sound.
    pub cut: usize,
}

/// A layer as the looper holds it (format 2): its audio as written,
/// [`LayerFile::audio`] holding `size` frames, and how it plays it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawLayer {
    pub size: usize,
    /// The first frame written, and how many (wrapping round `size`).
    pub valid: (usize, usize),
    /// Its view's pieces: first frame, frames, backwards.
    pub pieces: Vec<(u32, u32, bool)>,
    /// A Replace layer's fades in and out, in frames.
    pub replace: Option<(u32, u32)>,
    /// The recording that holds its audio, its host's (`mpfree_seq::RecId`,
    /// audio tracks phase 4c), 0 for none: the looper keeps it for the
    /// host in the text, nothing more.
    pub rec: u128,
    /// For a layer a merge or a fit made (audio tracks phase 4c.5), the
    /// recordings it was made from, in play order (`from=`, 32 hex digits
    /// each joined by `+`); empty for one recorded. The host's, as `rec`.
    pub from: Vec<u128>,
}

/// The free clock as saved, for its free loops: its tempo, exactly, its
/// meter and its bar 1 (the tick its free loops' origins count from). A
/// project opens it stopped there ([`crate::Looper::open_free_clock`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockFile {
    pub bpm: f64,
    pub meter: Meter,
    pub top: Tick,
}

impl ClockFile {
    /// The free clock `c` as saved.
    pub fn of(c: &FreeClock) -> ClockFile {
        ClockFile {
            bpm: c.bpm,
            meter: c.meter,
            top: c.top,
        }
    }
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
    /// the text); with `raw`, its `size` frames as written.
    pub audio: Vec<f32>,
    /// How it plays its audio (format 2); None: as it plays (format 1).
    pub raw: Option<RawLayer>,
}

/// The looper's part of a project: the device rate the loops were made
/// at, the loop selected, and each loop by number.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub rate: u32,
    pub selected: usize,
    pub loops: Vec<(usize, LoopFile)>,
    /// The free clock, when a free loop is saved.
    pub free_clock: Option<ClockFile>,
}

fn flag(b: bool) -> u8 {
    u8::from(b)
}

/// The text for `saved` (the layers' audio is not in it: each layer's
/// `file` names it).
pub fn to_text(saved: &Saved) -> String {
    let format = if saved.loops.iter().any(|(_, l)| l.raw.is_some()) {
        FORMAT_2
    } else {
        FORMAT
    };
    let mut s = format!(
        "{format}\nrate={} selected={}\n",
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
        if l.starts == Starts::Play {
            s.pop();
            s.push_str(" starts=play\n");
        }
        if l.speed != 0 || l.rate != 0 {
            s.pop();
            s.push_str(&format!(" speed={} octaves={}\n", l.speed, l.rate));
        }
        if l.id != 0 {
            s.pop();
            s.push_str(&format!(" id={:032x}\n", l.id));
        }
        if l.clock == Clock::Free {
            s.pop();
            s.push_str(" clock=free\n");
        }
        if let Some(r) = &l.raw {
            s.pop();
            let region = r
                .region
                .map_or("-".to_string(), |(a, b)| format!("{a},{b}"));
            s.push_str(&format!(
                " seam={} region={region} captured={},{} cut={}\n",
                r.seam, r.captured.0, r.captured.1, r.cut
            ));
        }
        for y in &l.layers {
            let raw = y.raw.as_ref().map_or(String::new(), |r| {
                let view: Vec<String> = r
                    .pieces
                    .iter()
                    .map(|&(a, n, b)| format!("{a}:{n}{}", if b { ":b" } else { "" }))
                    .collect();
                let replace = r
                    .replace
                    .map_or("-".to_string(), |(i, o)| format!("{i},{o}"));
                let rec = match r.rec {
                    0 => String::new(),
                    id => format!(" rec={id:032x}"),
                };
                let from = match r.from.is_empty() {
                    true => String::new(),
                    false => {
                        let ids: Vec<String> =
                            r.from.iter().map(|id| format!("{id:032x}")).collect();
                        format!(" from={}", ids.join("+"))
                    }
                };
                format!(
                    "{rec} size={} valid={},{} view={} replace={replace}{from}",
                    r.size,
                    r.valid.0,
                    r.valid.1,
                    view.join("+")
                )
            });
            s.push_str(&format!(
                "layer level={} pan={} muted={} solo={} faded={}{raw} file={}\n",
                y.level,
                y.pan,
                flag(y.muted),
                flag(y.solo),
                y.faded,
                y.file
            ));
        }
    }
    if let Some(c) = &saved.free_clock {
        s.push_str(&format!(
            "free-clock bpm={} beats={} unit={} top={}\n",
            c.bpm, c.meter.beats, c.meter.unit, c.top
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

/// A small whole number from -`most` to `most`, 0 when it isn't there.
fn small(kv: &[(&str, &str)], key: &str, line: usize, most: i8) -> Result<i8, String> {
    match get(kv, key) {
        None => Ok(0),
        Some(v) => v
            .parse::<i8>()
            .ok()
            .filter(|x| x.abs() <= most)
            .ok_or_else(|| format!("line {line}: {key}={v}")),
    }
}

fn bit(kv: &[(&str, &str)], key: &str) -> bool {
    get(kv, key) == Some("1")
}

/// `a,b`, two whole numbers.
fn pair(v: &str) -> Option<(usize, usize)> {
    let (a, b) = v.split_once(',')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// A format 2 layer line's [`RawLayer`]: its audio's size, the frames
/// written within it, a view of at most [`crate::view::MAX_PIECES`] pieces each
/// starting inside its audio, and a Replace's fades. Refused, with the
/// line, when any of it doesn't hold.
fn raw_layer(kv: &[(&str, &str)], n: usize) -> Result<RawLayer, String> {
    let size: usize = num(kv, "size", n)?;
    let bad = |what: &str| format!("line {n}: {what}");
    let valid = get(kv, "valid")
        .and_then(pair)
        .ok_or_else(|| bad("no valid"))?;
    if size == 0 || size > u32::MAX as usize || valid.0 >= size.max(1) || valid.1 > size {
        return Err(bad("its audio's frames"));
    }
    let view = get(kv, "view").ok_or_else(|| bad("no view"))?;
    let mut pieces = Vec::new();
    for p in view.split('+') {
        let mut w = p.split(':');
        let start = w.next().and_then(|x| x.parse::<u32>().ok());
        let len = w.next().and_then(|x| x.parse::<u32>().ok());
        let back = match w.next() {
            None => false,
            Some("b") => true,
            Some(_) => return Err(bad("view")),
        };
        match (start, len) {
            (Some(a), Some(l)) if (a as usize) < size && l > 0 && w.next().is_none() => {
                pieces.push((a, l, back))
            }
            _ => return Err(bad("view")),
        }
    }
    if pieces.is_empty() || pieces.len() > crate::view::MAX_PIECES {
        return Err(bad("view"));
    }
    let replace = match get(kv, "replace") {
        Some("-") | None => None,
        Some(v) => {
            let (i, o) = pair(v).ok_or_else(|| bad("replace"))?;
            Some((
                u32::try_from(i).map_err(|_| bad("replace"))?,
                u32::try_from(o).map_err(|_| bad("replace"))?,
            ))
        }
    };
    // The recordings a merge or a fit made it from (4c.5).
    let mut from = Vec::new();
    if let Some(v) = get(kv, "from") {
        for h in v.split('+') {
            match Some(h)
                .filter(|h| h.len() == 32 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                .and_then(|h| u128::from_str_radix(h, 16).ok())
            {
                Some(id) if id != 0 && from.len() < MAX_LAYERS => from.push(id),
                _ => return Err(bad("from")),
            }
        }
    }
    Ok(RawLayer {
        size,
        valid,
        pieces,
        replace,
        rec: hex_id(kv, "rec", n)?,
        from,
    })
}

/// A lasting id's word (`id=`, `rec=`): 32 hex digits, never 0; 0 when
/// there is none.
fn hex_id(kv: &[(&str, &str)], key: &str, n: usize) -> Result<u128, String> {
    match get(kv, key) {
        None => Ok(0),
        Some(h) => match Some(h)
            .filter(|h| h.len() == 32 && h.bytes().all(|b| b.is_ascii_hexdigit()))
            .and_then(|h| u128::from_str_radix(h, 16).ok())
        {
            Some(id) if id != 0 => Ok(id),
            _ => Err(format!("line {n}: {key}={h}")),
        },
    }
}

/// The part from its text. The layers come back without audio: load each
/// one's `file`.
pub fn from_text(text: &str) -> Result<Saved, String> {
    let mut lines = text.lines().enumerate();
    let two = match lines.next() {
        Some((_, l)) if l.trim_end() == FORMAT => false,
        Some((_, l)) if l.trim_end() == FORMAT_2 => true,
        _ => return Err(format!("not a looper part (no \"{FORMAT}\")")),
    };
    let mut saved = Saved {
        rate: 0,
        selected: 0,
        loops: Vec::new(),
        free_clock: None,
    };
    for (i, line) in lines {
        let n = i + 1;
        let (kv, file) = words(line.trim_end());
        if line.starts_with("free-clock ") {
            let bpm: f64 = num(&kv, "bpm", n)?;
            let meter = Meter::new(num(&kv, "beats", n)?, num(&kv, "unit", n)?)
                .ok_or_else(|| format!("line {n}: not a meter"))?;
            if !(bpm.is_finite() && bpm > 0.0) {
                return Err(format!("line {n}: bpm={bpm}"));
            }
            saved.free_clock = Some(ClockFile {
                bpm,
                meter,
                top: num(&kv, "top", n)?,
            });
        } else if let Some(r) = get(&kv, "rate") {
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
                    starts: match get(&kv, "starts") {
                        Some("play") => Starts::Play,
                        _ => Starts::Bar,
                    },
                    fade_old: level(&kv, "fade_old", n, 0.0, 1.0).unwrap_or(1.0),
                    speed: small(&kv, "speed", n, 1)?,
                    rate: small(&kv, "octaves", n, 2)?,
                    id: hex_id(&kv, "id", n)?,
                    clock: match get(&kv, "clock") {
                        Some("free") => Clock::Free,
                        _ => Clock::Song,
                    },
                    layers: Vec::new(),
                    raw: if two {
                        Some(RawShape {
                            seam: num(&kv, "seam", n)?,
                            region: match get(&kv, "region") {
                                Some("-") | None => None,
                                Some(v) => {
                                    Some(pair(v).ok_or_else(|| format!("line {n}: region={v}"))?)
                                }
                            },
                            captured: get(&kv, "captured")
                                .and_then(pair)
                                .ok_or_else(|| format!("line {n}: no captured"))?,
                            cut: num(&kv, "cut", n)?,
                        })
                    } else {
                        None
                    },
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
            let raw = match two {
                true => Some(raw_layer(&kv, n)?),
                false => None,
            };
            l.layers.push(LayerFile {
                level: level(&kv, "level", n, 0.0, 2.0)?,
                pan: level(&kv, "pan", n, -1.0, 1.0)?,
                muted: bit(&kv, "muted"),
                solo: bit(&kv, "solo"),
                faded: level(&kv, "faded", n, 0.0, 1.0).unwrap_or(1.0),
                file,
                audio: Vec::new(),
                raw,
            });
        }
    }
    if let Some((n, _)) = saved.loops.iter().find(|(_, l)| l.layers.is_empty()) {
        return Err(format!("loop {n} has no layers"));
    }
    if saved.free_clock.is_none() {
        if let Some((n, _)) = saved.loops.iter().find(|(_, l)| l.clock == Clock::Free) {
            return Err(format!("loop {n} is free, and there is no free clock"));
        }
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
            raw: None,
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
                        starts: Starts::Play,
                        fade_old: 0.9,
                        speed: -1,
                        rate: -2,
                        id: 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210,
                        clock: Clock::Free,
                        layers: vec![layer(1.0, "a b.wav"), layer(0.1, "c.wav")],
                        raw: None,
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
                        starts: Starts::Bar,
                        fade_old: 1.0,
                        speed: 0,
                        rate: 0,
                        id: 0,
                        clock: Clock::Song,
                        layers: vec![layer(1.0 / 3.0, "d.wav")],
                        raw: None,
                    },
                ),
            ],
            free_clock: Some(ClockFile {
                // (Every bit of it comes back.)
                bpm: std::f64::consts::PI * 30.0,
                meter: Meter { beats: 7, unit: 8 },
                top: -3360,
            }),
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
        assert!(from_text(&t.replace("speed=-1", "speed=3")).is_err());
        // A loop's id: 32 hex digits, never 0.
        let id = "id=0123456789abcdeffedcba9876543210";
        assert!(t.contains(id), "{t}");
        for bad in [
            "id=0",
            "id=00000000000000000000000000000000",
            "id=0123456789abcdeffedcba987654321",
            "id=+123456789abcdeffedcba9876543210",
            "id=0123456789abcdeffedcba987654321g",
        ] {
            assert!(from_text(&t.replace(id, bad)).is_err(), "{bad}");
        }
        // Out-of-range mixes are kept in range.
        let s = from_text(&t.replace("level=0.7", "level=9")).unwrap();
        assert_eq!(s.loops[0].1.level, 2.0);
    }

    /// Format 2 (audio tracks phase 4c): a loop kept as the looper holds
    /// it comes back as it was, its shape and each layer's view of its
    /// audio; a view, written frames or a Replace that doesn't hold is
    /// refused with its line, and a format 1 text still reads.
    #[test]
    fn a_loop_as_the_looper_holds_it_comes_back_as_it_was() {
        let mut s = saved();
        let l = &mut s.loops[0].1;
        l.raw = Some(RawShape {
            seam: 512,
            region: Some((8192, 9000)),
            captured: (8192, 96_392),
            cut: 88_200,
        });
        l.layers[0].raw = Some(RawLayer {
            size: 112_776,
            valid: (0, 112_776),
            pieces: vec![(9000, 79_392, false), (8192, 808, false)],
            replace: None,
            rec: 0x0123_4567_89ab_cdef_0000_0000_0000_0001,
            from: Vec::new(),
        });
        l.layers[1].raw = Some(RawLayer {
            size: 88_200,
            valid: (20_000, 30_000),
            pieces: vec![(88_199, 88_200, true)],
            replace: Some((256, 512)),
            rec: 0x0123_4567_89ab_cdef_0000_0000_0000_0003,
            // Made by a merge of two recordings (4c.5).
            from: vec![
                0x0123_4567_89ab_cdef_0000_0000_0000_0001,
                0x0123_4567_89ab_cdef_0000_0000_0000_0002,
            ],
        });
        let t = to_text(&s);
        assert!(t.starts_with("mpfree-looper 2\n"), "{t}");
        assert!(t.contains(" seam=512 region=8192,9000 captured=8192,96392 cut=88200"));
        assert!(
            t.contains("view=9000:79392+8192:808 replace=- file=a b.wav"),
            "{t}"
        );
        assert!(t.contains("valid=20000,30000 view=88199:88200:b replace=256,512"));
        assert!(t.contains(" rec=0123456789abcdef0000000000000001 size=112776 "));
        assert!(
            t.contains(" replace=256,512 from=0123456789abcdef0000000000000001+0123456789abcdef0000000000000002 file="),
            "{t}"
        );
        assert_eq!(t.matches(" from=").count(), 1, "none for a layer recorded");
        // Loop 2 kept as it plays, in a format 2 text: no shape, and its
        // layer needs one, so it is refused.
        let only = |s: &Saved| {
            let mut s = s.clone();
            s.loops.truncate(1);
            s
        };
        assert_eq!(from_text(&to_text(&only(&s))), Ok(only(&s)));
        assert!(from_text(&t).is_err(), "a layer with no view in format 2");
        let t = to_text(&only(&s));
        for (from, to) in [
            ("view=9000:79392+8192:808", "view=200000:5"),
            ("view=9000:79392+8192:808", "view=9000:0"),
            ("view=9000:79392+8192:808", "view=9000:79392:x"),
            ("view=9000:79392+8192:808", "view="),
            ("valid=20000,30000", "valid=20000,99999"),
            ("valid=20000,30000", "valid=88200,1"),
            ("replace=256,512", "replace=256"),
            ("size=88200", "size=0"),
            (" cut=88200", ""),
            ("region=8192,9000", "region=8192"),
            ("rec=0123456789abcdef0000000000000001", "rec=0"),
            ("rec=0123456789abcdef0000000000000001", "rec=12"),
            ("+0123456789abcdef0000000000000002", "+12"),
            (
                "+0123456789abcdef0000000000000002",
                "+00000000000000000000000000000000",
            ),
        ] {
            assert!(t.contains(from), "{from}");
            let e = from_text(&t.replace(from, to)).unwrap_err();
            assert!(e.starts_with("line "), "{to}: {e}");
        }
        assert_eq!(
            from_text(&t.replace("region=8192,9000", "region=-"))
                .unwrap()
                .loops[0]
                .1
                .raw
                .unwrap()
                .region,
            None
        );
        // Format 1 still reads, with nothing kept as the looper holds it.
        let one = from_text(&to_text(&saved())).unwrap();
        assert!(one.loops.iter().all(|(_, l)| l.raw.is_none()));
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
