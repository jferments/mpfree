//! Tables from the outside: WAV files (single cycles and Serum-style
//! multi-frame tables) and any pitched sample, such as a pad's.
//!
//! All of this allocates and belongs on the control thread.

use mpfree_analysis::{root_pitch, yin, PitchSettings};

use super::table::{Table, TableError, FRAME, MAX_FRAMES};

/// The most samples a WAV import reads (256 frames of 2048).
pub const MAX_WAV_SAMPLES: usize = MAX_FRAMES * FRAME;
/// The longest file taken as one single cycle.
pub const MAX_CYCLE: usize = 4096;

/// A decoded WAV: mono samples, the rate, and the frame size a `clm `
/// chunk declares (Serum writes `<!>2048 ...`).
#[derive(Clone, Debug, PartialEq)]
pub struct Wav {
    pub samples: Vec<f32>,
    pub rate: u32,
    pub frame_len: Option<usize>,
}

fn u16_at(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(i..i + 2)?.try_into().ok()?))
}
fn u32_at(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

/// Reads a RIFF WAVE: PCM 8/16/24/32 or float 32/64, any channel count
/// (mixed to mono). WAVE_FORMAT_EXTENSIBLE is read by its sub-format.
pub fn decode_wav(data: &[u8]) -> Result<Wav, TableError> {
    let bad = TableError::Wav;
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(bad("not a RIFF WAVE file"));
    }
    let mut fmt = None;
    let mut pcm: Option<&[u8]> = None;
    let mut frame_len = None;
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let len = u32_at(data, pos + 4).ok_or(bad("chunk"))? as usize;
        let start = pos + 8;
        // A truncated last chunk is read as far as it goes.
        let end = start.saturating_add(len).min(data.len());
        let body = &data[start..end];
        match id {
            b"fmt " => {
                let mut tag = u16_at(body, 0).ok_or(bad("fmt"))?;
                let ch = u16_at(body, 2).ok_or(bad("fmt"))?;
                let rate = u32_at(body, 4).ok_or(bad("fmt"))?;
                let bits = u16_at(body, 14).ok_or(bad("fmt"))?;
                if tag == 0xFFFE {
                    tag = u16_at(body, 24).ok_or(bad("extensible fmt"))?;
                }
                fmt = Some((tag, ch, rate, bits));
            }
            b"data" => pcm = Some(body),
            b"clm " => {
                let text = String::from_utf8_lossy(body);
                if let Some(rest) = text.strip_prefix("<!>") {
                    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                    frame_len = digits
                        .parse::<usize>()
                        .ok()
                        .filter(|&n| (16..=8192).contains(&n));
                }
            }
            _ => {}
        }
        pos = start.saturating_add(len).saturating_add(len & 1);
    }
    let (tag, ch, rate, bits) = fmt.ok_or(bad("no fmt chunk"))?;
    let pcm = pcm.ok_or(bad("no data chunk"))?;
    if ch == 0 || rate == 0 {
        return Err(bad("no channels"));
    }
    let bytes = usize::from(bits / 8);
    let ok = matches!((tag, bits), (1, 8 | 16 | 24 | 32) | (3, 32 | 64));
    if !ok {
        return Err(bad("only PCM 8/16/24/32-bit or float 32/64-bit"));
    }
    let ch = usize::from(ch);
    let stride = bytes * ch;
    let frames = (pcm.len() / stride).min(MAX_WAV_SAMPLES + 1);
    let read = |b: &[u8]| -> f32 {
        match (tag, bits) {
            (1, 8) => (f32::from(b[0]) - 128.0) / 128.0,
            (1, 16) => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0,
            (1, 24) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
            (1, 32) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
            (3, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            _ => f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32,
        }
    };
    let samples = (0..frames)
        .map(|f| {
            let s: f32 = (0..ch)
                .map(|c| read(&pcm[f * stride + c * bytes..]))
                .sum::<f32>()
                / ch as f32;
            if s.is_finite() {
                s
            } else {
                0.0
            }
        })
        .collect();
    Ok(Wav {
        samples,
        rate,
        frame_len,
    })
}

/// A table from a WAV file's bytes. The frame size is the `clm ` chunk's;
/// without one, a file of whole 2048-sample frames is a multi-frame table
/// and a file of up to 4096 samples is one cycle.
pub fn table_from_wav(name: &str, data: &[u8]) -> Result<Table, TableError> {
    let w = decode_wav(data)?;
    let n = w.samples.len();
    if n == 0 {
        return Err(TableError::Empty);
    }
    if n > MAX_WAV_SAMPLES {
        return Err(TableError::TooManyFrames(n / w.frame_len.unwrap_or(FRAME)));
    }
    let frame_len = match w.frame_len {
        Some(f) => f,
        None if n % FRAME == 0 => FRAME,
        None if n <= MAX_CYCLE => n,
        None => return Err(TableError::NotFrames(n)),
    };
    Table::from_frames(name, &w.samples, frame_len)
}

/// Cuts a pitched sample (mono) into `frames` single cycles spread across
/// it, each starting on a rising zero crossing and as long as the pitch
/// measured there, and builds a table: scanning the position plays through
/// the sample's timbre at any pitch.
pub fn table_from_sample(
    name: &str,
    mono: &[f32],
    rate: u32,
    frames: usize,
) -> Result<Table, TableError> {
    let frames = frames.clamp(1, MAX_FRAMES);
    let set = PitchSettings {
        min_hz: 30.0,
        max_hz: 2_000.0,
        ..PitchSettings::default()
    };
    let root = root_pitch(mono, 1, rate, set)
        .ok()
        .flatten()
        .ok_or(TableError::NoPitch)?;
    let period = rate as f32 / root.hz;
    let window = (period * 4.0) as usize;
    let span = window + (rate as f32 / (root.hz / 1.5)) as usize + 2;
    if mono.len() < span + period as usize * 2 {
        return Err(TableError::NoPitch);
    }
    let peak = mono.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let last = mono.len() - span - period as usize * 2;
    let mut cycles = Vec::with_capacity(frames * FRAME);
    let mut kept = 0;
    for j in 0..frames {
        let at = if frames == 1 {
            last / 2
        } else {
            last * j / (frames - 1)
        };
        // The pitch here, near the root (a sample drifts).
        let p = match yin(
            &mono[at..],
            rate,
            window,
            root.hz / 1.5,
            root.hz * 1.5,
            0.25,
        ) {
            Some((hz, _)) if hz > 0.0 => rate as f32 / hz,
            _ => period,
        };
        let Some(start) = rising_zero(mono, at, p as usize + 2) else {
            continue;
        };
        if (start as f32 + p + 2.0) as usize >= mono.len() {
            continue;
        }
        let cyc: Vec<f32> = (0..FRAME)
            .map(|i| interp(mono, start as f32 + i as f32 * p / FRAME as f32))
            .collect();
        let rms = (cyc.iter().map(|v| v * v).sum::<f32>() / FRAME as f32).sqrt();
        if rms < peak * 0.01 {
            continue;
        }
        cycles.extend(cycles_seamless(cyc));
        kept += 1;
    }
    if kept == 0 {
        return Err(TableError::NoPitch);
    }
    Table::from_frames(name, &cycles, FRAME)
}

/// The first rising zero crossing at or after `from`, within `within`.
fn rising_zero(x: &[f32], from: usize, within: usize) -> Option<usize> {
    let end = (from + within.max(2)).min(x.len() - 1);
    (from..end).find(|&i| x[i] <= 0.0 && x[i + 1] > 0.0)
}

fn interp(x: &[f32], at: f32) -> f32 {
    let i = at as usize;
    let t = at - i as f32;
    let a = x[i.min(x.len() - 1)];
    let b = x[(i + 1).min(x.len() - 1)];
    a + (b - a) * t
}

/// Removes the step between a cut cycle's end and its start with a linear
/// tilt, so the loop has no click (and no buzz of high harmonics).
fn cycles_seamless(mut c: Vec<f32>) -> Vec<f32> {
    let n = c.len();
    let gap = c[0] - (c[n - 1] + (c[n - 1] - c[n - 2]));
    for (i, v) in c.iter_mut().enumerate() {
        *v += gap * i as f32 / n as f32;
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(samples: &[f32], clm: Option<&str>) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend(b"RIFF\0\0\0\0WAVE");
        d.extend(b"fmt ");
        d.extend(16u32.to_le_bytes());
        d.extend(3u16.to_le_bytes());
        d.extend(1u16.to_le_bytes());
        d.extend(48_000u32.to_le_bytes());
        d.extend((48_000u32 * 4).to_le_bytes());
        d.extend(4u16.to_le_bytes());
        d.extend(32u16.to_le_bytes());
        if let Some(c) = clm {
            d.extend(b"clm ");
            d.extend((c.len() as u32).to_le_bytes());
            d.extend(c.as_bytes());
            if c.len() % 2 == 1 {
                d.push(0);
            }
        }
        d.extend(b"data");
        d.extend(((samples.len() * 4) as u32).to_le_bytes());
        for s in samples {
            d.extend(s.to_le_bytes());
        }
        let n = (d.len() - 8) as u32;
        d[4..8].copy_from_slice(&n.to_le_bytes());
        d
    }

    fn saw(n: usize) -> Vec<f32> {
        (0..n).map(|i| 1.0 - 2.0 * i as f32 / n as f32).collect()
    }

    #[test]
    fn reads_a_single_cycle_and_a_serum_table() {
        let one = table_from_wav("one", &wav(&saw(600), None)).unwrap();
        assert_eq!(one.frames(), 1);
        let mut multi = saw(256);
        multi.extend(saw(256).iter().map(|v| -v));
        multi.extend(saw(256));
        let w = wav(
            &multi,
            Some("<!>256 10000000 wavetable (www.xferrecords.com)"),
        );
        assert_eq!(decode_wav(&w).unwrap().frame_len, Some(256));
        let t = table_from_wav("serum", &w).unwrap();
        assert_eq!(t.frames(), 3);
        let two = table_from_wav("two", &wav(&saw(4096), None)).unwrap();
        assert_eq!(two.frames(), 2, "whole 2048 frames win over one long cycle");
        assert_eq!(
            table_from_wav("long", &wav(&saw(5000), None)).unwrap_err(),
            TableError::NotFrames(5000)
        );
        assert!(matches!(
            table_from_wav("bad", b"RIFF....WAVEjunk"),
            Err(TableError::Wav(_))
        ));
    }

    #[test]
    fn a_pitched_sample_becomes_cycles() {
        let rate = 44_100;
        let hz = 220.0f32;
        // A saw at 220 Hz whose brightness falls: a one-pole low-pass closing.
        let mut x = Vec::new();
        let mut y = 0.0f32;
        for i in 0..rate as usize {
            let ph = (i as f32 * hz / rate as f32).fract();
            let k = 0.9 - 0.85 * i as f32 / rate as f32;
            y += k * ((1.0 - 2.0 * ph) - y);
            x.push(y * 0.5);
        }
        let t = table_from_sample("pad", &x, rate, 8).unwrap();
        assert_eq!(t.frames(), 8);
        // Brighter at the start: more energy above the fundamental.
        let bright = |f: usize| {
            let p = t.preview(f);
            p.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>()
        };
        assert!(bright(0) > bright(7) * 1.5, "{} {}", bright(0), bright(7));
        let mut rng = crate::dsp::Rng::new(7);
        let noise: Vec<f32> = (0..20_000).map(|_| rng.bipolar()).collect();
        assert_eq!(
            table_from_sample("noise", &noise, rate, 8).unwrap_err(),
            TableError::NoPitch
        );
    }
}
