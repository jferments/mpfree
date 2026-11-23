//! The stem separation on the workstation: make the model file from
//! Deezer's checkpoint, and split a WAV file into four, to listen to or to
//! compare with Spleeter's own output (`docs/dev/design/stems.md`).
//!
//! ```text
//! cargo run --release -p mpfree-stems --example stems -- pack CHECKPOINT_DIR OUT.mpfstem
//! cargo run --release -p mpfree-stems --example stems -- split MODEL IN.wav OUT_DIR [--band spleeter|full] [--threads N]
//! ```
//!
//! `pack` reads the `4stems` folder from Spleeter's v1.4.0 release
//! (`4stems.tar.gz`). `split` reads 16-bit, 24-bit or 32-bit float WAV at
//! 44.1 kHz, mono or stereo, and writes `vocals.wav`, `drums.wav`,
//! `bass.wav` and `other.wav` (32-bit float, stereo) into OUT_DIR.

use mpfree_stems::{Band, Model, Separator, Settings, Sink, Stem};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let run = match args.first().map(String::as_str) {
        Some("pack") if args.len() == 3 => pack(&args[1], &args[2]),
        Some("split") if args.len() >= 4 => split(&args[1], &args[2], &args[3], &args[4..]),
        _ => Err("usage: stems pack CHECKPOINT_DIR OUT.mpfstem\n       \
                  stems split MODEL IN.wav OUT_DIR [--band spleeter|full] [--threads N]"
            .into()),
    };
    match run {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn pack(dir: &str, out: &str) -> Result<(), String> {
    let t = Instant::now();
    let mut file =
        std::io::BufWriter::new(std::fs::File::create(out).map_err(|e| format!("{out}: {e}"))?);
    let r = mpfree_stems::pack::pack(Path::new(dir), &mut file).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| format!("{out}: {e}"))?;
    println!(
        "wrote {out} in {:.1} s: {} kernel weights, {} of them below the smallest normal half (now zero)",
        t.elapsed().as_secs_f64(),
        r.kernels,
        r.flushed
    );
    Ok(())
}

fn split(model: &str, input: &str, out: &str, opts: &[String]) -> Result<(), String> {
    let mut settings = Settings::default();
    let mut it = opts.iter();
    while let Some(o) = it.next() {
        match (o.as_str(), it.next().map(String::as_str)) {
            ("--band", Some("spleeter")) => settings.band = Band::Spleeter,
            ("--band", Some("full")) => settings.band = Band::Full,
            ("--threads", Some(n)) => {
                settings.threads = n.parse().map_err(|_| format!("--threads {n}"))?
            }
            _ => return Err(format!("unknown option {o}")),
        }
    }
    let t = Instant::now();
    let model = Model::open(Path::new(model)).map_err(|e| e.to_string())?;
    let loaded = t.elapsed().as_secs_f64();
    let (rate, channels, data) = read_wav(input)?;
    std::fs::create_dir_all(out).map_err(|e| format!("{out}: {e}"))?;
    let mut sink = Files {
        files: Stem::ALL
            .iter()
            .map(|s| Wav::create(&format!("{out}/{}.wav", s.name().to_lowercase())))
            .collect::<Result<_, _>>()?,
    };
    let t = Instant::now();
    let mut sep =
        Separator::new(&model, &data, channels, rate, &settings).map_err(|e| e.to_string())?;
    while sep.step(&mut sink).map_err(|e| e.to_string())? {}
    let secs = t.elapsed().as_secs_f64();
    for f in sink.files {
        f.finish()?;
    }
    let st = sep.stats();
    let audio = data.len() as f64 / channels as f64 / f64::from(rate);
    println!(
        "{audio:.2} s of audio, {:?}, {} threads: model loaded in {loaded:.2} s; split in {secs:.2} s \
         (STFT {:.2}, U-Nets {:.2} with {:.2} unpacking, masks and inverse {:.2}, writing {:.2}); \
         {:.2} G multiply-adds; {} segments",
        settings.band,
        settings.threads,
        st.analyse.as_secs_f64(),
        st.nets.as_secs_f64(),
        st.unpack.as_secs_f64(),
        st.masks.as_secs_f64(),
        st.sink.as_secs_f64(),
        st.macs as f64 / 1e9,
        st.segments
    );
    Ok(())
}

struct Files {
    files: Vec<Wav>,
}

impl Sink for Files {
    fn put(&mut self, stem: Stem, stereo: &[f32]) -> Result<(), String> {
        let i = Stem::ALL.iter().position(|&s| s == stem).unwrap_or(0);
        self.files[i].write(stereo)
    }
}

/// A WAV file's rate, channels and interleaved samples.
fn read_wav(path: &str) -> Result<(u32, usize, Vec<f32>), String> {
    let b = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(format!("{path}: not a WAV file"));
    }
    let u16le = |b: &[u8]| u16::from_le_bytes([b[0], b[1]]);
    let u32le = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let (mut fmt, mut channels, mut rate, mut bits) = (0u16, 0usize, 0u32, 0u16);
    let mut at = 12;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32le(&b[at + 4..]) as usize;
        let body = &b[at + 8..(at + 8 + len).min(b.len())];
        if id == b"fmt " && body.len() >= 16 {
            fmt = u16le(body);
            channels = usize::from(u16le(&body[2..]));
            rate = u32le(&body[4..]);
            bits = u16le(&body[14..]);
            if fmt == 0xfffe && body.len() >= 26 {
                fmt = u16le(&body[24..]);
            }
        } else if id == b"data" {
            let data: Vec<f32> = match (fmt, bits) {
                (1, 16) => body
                    .chunks_exact(2)
                    .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0)
                    .collect(),
                (1, 24) => body
                    .chunks_exact(3)
                    .map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0)
                    .collect(),
                (3, 32) => body
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect(),
                _ => {
                    return Err(format!(
                        "{path}: format {fmt} at {bits} bits isn't read here"
                    ))
                }
            };
            return Ok((rate, channels, data));
        }
        at += 8 + len + (len & 1);
    }
    Err(format!("{path}: no data chunk"))
}

/// A 32-bit float stereo WAV written as it comes; `finish` puts the sizes in.
struct Wav {
    f: std::io::BufWriter<std::fs::File>,
    path: String,
    bytes: u64,
}

impl Wav {
    fn create(path: &str) -> Result<Wav, String> {
        let mut h = Vec::with_capacity(44);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&36u32.to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16u32.to_le_bytes());
        h.extend_from_slice(&3u16.to_le_bytes());
        h.extend_from_slice(&2u16.to_le_bytes());
        h.extend_from_slice(&mpfree_stems::RATE.to_le_bytes());
        h.extend_from_slice(&(mpfree_stems::RATE * 8).to_le_bytes());
        h.extend_from_slice(&8u16.to_le_bytes());
        h.extend_from_slice(&32u16.to_le_bytes());
        h.extend_from_slice(b"data");
        h.extend_from_slice(&0u32.to_le_bytes());
        let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
        let mut f = std::io::BufWriter::new(file);
        f.write_all(&h).map_err(|e| format!("{path}: {e}"))?;
        Ok(Wav {
            f,
            path: path.to_string(),
            bytes: 0,
        })
    }

    fn write(&mut self, data: &[f32]) -> Result<(), String> {
        let mut b = Vec::with_capacity(data.len() * 4);
        data.iter()
            .for_each(|v| b.extend_from_slice(&v.to_le_bytes()));
        self.f
            .write_all(&b)
            .map_err(|e| format!("{}: {e}", self.path))?;
        self.bytes += b.len() as u64;
        Ok(())
    }

    fn finish(mut self) -> Result<(), String> {
        let e = |e: std::io::Error| format!("{}: {e}", self.path);
        let bytes = u32::try_from(self.bytes).map_err(|_| format!("{}: over 4 GB", self.path))?;
        self.f.seek(SeekFrom::Start(4)).map_err(e)?;
        self.f.write_all(&(36 + bytes).to_le_bytes()).map_err(e)?;
        self.f.seek(SeekFrom::Start(40)).map_err(e)?;
        self.f.write_all(&bytes.to_le_bytes()).map_err(e)?;
        self.f.flush().map_err(e)
    }
}
