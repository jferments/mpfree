//! Prepared copies (disk streaming's P3, step 4): a set's files at
//! another rate made once, at the engine's rate, so a show plays them
//! without converting as it goes.
//!
//! Copies are made only when needed:
//!
//! | Original | Prepared copy |
//! |---|---|
//! | any format at the engine's rate | none: the original plays |
//! | integer at another rate | 24-bit at the engine's rate, dithered; float instead if its resampled peaks would pass full scale: never clipped |
//! | float at another rate | 32-bit float at the engine's rate |
//!
//! A set can ask for float copies throughout. A resampled copy is a new
//! version, not a lossless one, so each is named by what made it: the
//! original's content hash and format, the target rate and format, the
//! resampler's version and quality, and the dither ([`Recipe::name`]).
//! A newer resampler never reuses an older copy, and the same recipe always
//! makes the same file (the dither is seeded from the name), so a copy
//! already there is used as it is. Copies are made with the same
//! resampler that plays stems at another rate ([`crate::resample`]), a
//! block at a time, and published whole or not at all
//! ([`crate::publish`]).

use std::io;
use std::path::Path;

use crate::format::{decode, Layout, Sample};
use crate::fsio::Fs;
use crate::publish::{hex, publish, Published, Sink};
use crate::resample::{Rate, CUTOFF, ZEROS};

/// The resampler's version and quality, in every resampled copy's name.
/// Change it whenever the resampler's output changes.
pub const RESAMPLER: &str = "rs1-k24";
/// The dither on 24-bit copies: triangular, one step either side, seeded
/// from the copy's name.
pub const DITHER: &str = "tpdf1";

/// Engine frames made at a time.
const BLOCK: u64 = 16_384;

/// What a set's file becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The original plays as it is.
    Original,
    /// 24-bit at `rate`, dithered, or 32-bit float if its resampled peaks
    /// would pass full scale (decided as it is made, the same every time).
    Int24 { rate: u32 },
    /// 32-bit float at `rate`.
    Float { rate: u32 },
}

/// What a file at `layout` becomes for an engine at `engine` frames a
/// second; `float` when the set asks for float copies.
pub fn target(layout: &Layout, engine: u32, float: bool) -> Target {
    if layout.rate == engine {
        return Target::Original;
    }
    match layout.sample {
        Sample::F32 | Sample::F64 => Target::Float { rate: engine },
        _ if float => Target::Float { rate: engine },
        _ => Target::Int24 { rate: engine },
    }
}

fn sample_word(s: Sample) -> &'static str {
    match s {
        Sample::U8 => "u8",
        Sample::I8 => "i8",
        Sample::I16 => "i16",
        Sample::I24 => "i24",
        Sample::I32 => "i32",
        Sample::F32 => "f32",
        Sample::F64 => "f64",
    }
}

/// What makes a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipe {
    /// SHA-256 of the original file.
    pub original: [u8; 32],
    /// The original's layout.
    pub from: Layout,
    pub target: Target,
}

impl Recipe {
    /// The copy's file name: what made it, so a different recipe never
    /// finds this copy. None for [`Target::Original`].
    pub fn name(&self) -> Option<String> {
        let (rate, form, dither) = match self.target {
            Target::Original => return None,
            Target::Int24 { rate } => (rate, "i24", DITHER),
            Target::Float { rate } => (rate, "f32", "none"),
        };
        Some(format!(
            "{}-{}{}x{}-to-{}{}-{}-{}.wav",
            &hex(&self.original)[..32],
            self.from.rate,
            sample_word(self.from.sample),
            self.from.channels,
            rate,
            form,
            RESAMPLER,
            dither,
        ))
    }
}

/// A copy made (or found already made).
#[derive(Clone, Debug, PartialEq)]
pub struct Made {
    pub file: Published,
    /// How its samples are stored: [`Sample::I24`] or [`Sample::F32`].
    pub sample: Sample,
    pub frames: u64,
    pub rate: u32,
    /// The loudest sample after resampling, before dither; None for a
    /// copy found already made.
    pub peak: Option<f32>,
}

/// Makes a copy's engine frames a block at a time from the original's.
struct Convert<'a> {
    original: &'a Path,
    l: Layout,
    rate: Rate,
    raw: Vec<u8>,
    floats: Vec<f32>,
    out: Vec<f32>,
}

impl Convert<'_> {
    /// Engine frames `first..first + n`, interleaved, reading the
    /// original's frames they need through `fs`.
    fn block(&mut self, fs: &mut dyn Fs, first: u64, n: u64) -> io::Result<&[f32]> {
        let l = &self.l;
        let ch = usize::from(l.channels);
        let (a, b) = self.rate.span(first, n);
        let bytes = (b - a) as usize * l.block_align();
        self.raw.resize(bytes, 0);
        let got = fs.read_at(self.original, l.byte_of(a), &mut self.raw)?;
        if got < bytes {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("{}: shorter than its header says", self.original.display()),
            ));
        }
        self.floats.resize((b - a) as usize * ch, 0.0);
        let decoded = decode(l.sample, l.endian, &self.raw, &mut self.floats);
        self.out.resize(n as usize * ch, 0.0);
        self.rate
            .run(ch, &self.floats[..decoded], a, first, &mut self.out);
        Ok(&self.out)
    }
}

/// A small, fixed noise source for the dither: the same name, the same
/// noise, the same copy.
struct Noise(u64);

impl Noise {
    fn from_name(name: &str) -> Noise {
        // FNV-1a.
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for b in name.bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        Noise(h | 1)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

const FULL: f64 = 8_388_608.0;

fn wav_header(sample: Sample, channels: u16, rate: u32, data: u32) -> Vec<u8> {
    let (tag, bits): (u16, u16) = match sample {
        Sample::F32 => (3, 32),
        _ => (1, 24),
    };
    let align = u32::from(channels) * u32::from(bits / 8);
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&tag.to_le_bytes());
    h.extend_from_slice(&channels.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * align).to_le_bytes());
    h.extend_from_slice(&(align as u16).to_le_bytes());
    h.extend_from_slice(&bits.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    h
}

/// Make the copy `recipe` names in `dir` (written through `fs`) from
/// `original` (read through `src`; its layout is `recipe.from`), or find
/// it already made. `job` makes its part file's name unique. A 24-bit
/// copy whose peaks would pass full scale is made as float. A copy too
/// long for a WAV file (4 GB) is refused.
pub fn make(
    src: &mut dyn Fs,
    original: &Path,
    fs: &mut dyn Fs,
    recipe: &Recipe,
    dir: &Path,
    job: u32,
) -> io::Result<Made> {
    let refuse = |why: String| io::Error::new(io::ErrorKind::InvalidInput, why);
    let name = recipe
        .name()
        .ok_or_else(|| refuse("the original plays as it is: no copy to make".into()))?;
    let l = recipe.from;
    let to = match recipe.target {
        Target::Int24 { rate } | Target::Float { rate } => rate,
        Target::Original => unreachable!("named above"),
    };
    let rate = Rate::new(l.rate, to, l.frames)
        .ok_or_else(|| refuse(format!("{} Hz to {to} Hz isn't a conversion", l.rate)))?;
    let frames = rate.frames();
    let ch = usize::from(l.channels);
    let size = |sample: Sample| {
        frames
            .checked_mul((ch * sample.bytes()) as u64)
            .filter(|&d| d <= u64::from(u32::MAX - 36))
    };
    // Already made (the same recipe makes the same file): keep it. Its
    // size says which way it was stored.
    let path = dir.join(&name);
    if fs.exists(&path) {
        let (hash, bytes) = crate::publish::hash_file(fs, &path)?;
        for sample in [Sample::I24, Sample::F32] {
            if size(sample).is_some_and(|d| bytes == 44 + d) {
                let file = Published {
                    path,
                    hash,
                    size: bytes,
                    existed: true,
                };
                return Ok(Made {
                    file,
                    sample,
                    frames,
                    rate: to,
                    peak: None,
                });
            }
        }
    }
    let mut conv = Convert {
        original,
        l,
        rate,
        raw: Vec::new(),
        floats: Vec::new(),
        out: Vec::new(),
    };
    // The peak decides a 24-bit copy's format, so it comes first: one
    // pass to find it (float copies need none), one to write.
    let mut peak = 0.0f32;
    if matches!(recipe.target, Target::Int24 { .. }) {
        let mut first = 0;
        while first < frames {
            let n = BLOCK.min(frames - first);
            peak = conv
                .block(src, first, n)?
                .iter()
                .fold(peak, |p, x| p.max(x.abs()));
            first += n;
        }
    }
    // Dither moves a sample at most one step: float if that could pass
    // the largest 24-bit value.
    let sample = match recipe.target {
        Target::Int24 { .. } if f64::from(peak) * FULL + 1.0 < FULL - 1.0 => Sample::I24,
        _ => Sample::F32,
    };
    let data = size(sample).ok_or_else(|| refuse(format!("{name}: too long for a WAV copy")))?;
    let mut noise = Noise::from_name(&name);
    let mut bytes = Vec::new();
    let mut write = |sink: &mut Sink| -> io::Result<()> {
        sink(&wav_header(sample, l.channels, to, data as u32))?;
        let mut first = 0;
        while first < frames {
            let n = BLOCK.min(frames - first);
            let out = conv.block(src, first, n)?;
            bytes.clear();
            match sample {
                Sample::I24 => {
                    for &x in out {
                        let d = noise.unit() - noise.unit();
                        let v = (f64::from(x) * FULL + d).round().clamp(-FULL, FULL - 1.0) as i32;
                        bytes.extend_from_slice(&v.to_le_bytes()[..3]);
                    }
                }
                _ => {
                    for &x in out {
                        bytes.extend_from_slice(&x.to_le_bytes());
                    }
                }
            }
            sink(&bytes)?;
            first += n;
        }
        Ok(())
    };
    let file = publish(fs, dir, &name, job, &mut write)?;
    let peak = matches!(recipe.target, Target::Int24 { .. }).then_some(peak);
    Ok(Made {
        file,
        sample,
        frames,
        rate: to,
        peak,
    })
}

/// The file bytes a copy of `recipe` takes, for the free-space check
/// before anything is written (0 for the original).
pub fn copy_bytes(recipe: &Recipe) -> u64 {
    let (rate, bytes) = match recipe.target {
        Target::Original => return 0,
        // A 24-bit copy may be made as float: count the larger.
        Target::Int24 { rate } | Target::Float { rate } => (rate, 4),
    };
    Rate::new(recipe.from.rate, rate, recipe.from.frames).map_or(0, |r| {
        44 + r.frames() * u64::from(recipe.from.channels) * bytes
    })
}

/// The quality the resampler works at, for the manual and the screen: the
/// zero crossings each side and the cutoff.
pub fn quality() -> (usize, f64) {
    (ZEROS, CUTOFF)
}
