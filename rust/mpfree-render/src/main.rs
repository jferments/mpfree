//! Desktop-only PCM WAV reference. Never opens MPC/ALSA/MIDI/network devices.
#![forbid(unsafe_code)]

use mpfree_core::{float_to_s32, ClickConfig, ClickEngine};
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};

const RATE: u32 = 44_100;
const CHANNELS: u16 = 2;
const BYTES_PER_SAMPLE: u16 = 4;
const BLOCK: usize = 128;

fn u16le(out: &mut impl Write, n: u16) -> std::io::Result<()> {
    out.write_all(&n.to_le_bytes())
}
fn u32le(out: &mut impl Write, n: u32) -> std::io::Result<()> {
    out.write_all(&n.to_le_bytes())
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if !(2..=4).contains(&args.len()) {
        return Err("Usage: mpfree-render <output.wav> [seconds=4] [bpm=120]".into());
    }
    let seconds: f64 = if args.len() >= 3 {
        args[2].parse()?
    } else {
        4.0
    };
    let bpm: f64 = if args.len() >= 4 {
        args[3].parse()?
    } else {
        120.0
    };
    if !seconds.is_finite() || !(0.0..=60.0).contains(&seconds) || seconds == 0.0 {
        return Err("seconds must be finite and >0 and <=60".into());
    }
    let mut engine = ClickEngine::new();
    if !engine.configure(ClickConfig {
        sample_rate: RATE,
        bpm,
        beats_per_bar: 4,
    }) {
        return Err("tempo must be finite and in [20,400] BPM".into());
    }

    let frames = (seconds * f64::from(RATE)).round() as u32;
    let data_bytes = frames
        .checked_mul(u32::from(CHANNELS) * u32::from(BYTES_PER_SAMPLE))
        .ok_or("WAV data size overflow")?;
    let riff_size = 36u32
        .checked_add(data_bytes)
        .ok_or("WAV RIFF size overflow")?;

    let file = File::create(&args[1])?;
    let mut out = BufWriter::new(file);
    out.write_all(b"RIFF")?;
    u32le(&mut out, riff_size)?;
    out.write_all(b"WAVEfmt ")?;
    u32le(&mut out, 16)?; // linear PCM fmt chunk bytes
    u16le(&mut out, 1)?; // uncompressed integer PCM
    u16le(&mut out, CHANNELS)?;
    u32le(&mut out, RATE)?;
    u32le(
        &mut out,
        RATE * u32::from(CHANNELS) * u32::from(BYTES_PER_SAMPLE),
    )?;
    u16le(&mut out, CHANNELS * BYTES_PER_SAMPLE)?;
    u16le(&mut out, 32)?; // 32 bits per sample
    out.write_all(b"data")?;
    u32le(&mut out, data_bytes)?;

    let mut left = [0.0f32; BLOCK];
    let mut right = [0.0f32; BLOCK];
    let mut remaining = frames as usize;
    while remaining > 0 {
        let block = remaining.min(BLOCK);
        engine
            .render(&mut left[..block], &mut right[..block])
            .map_err(|_| "channel sizes do not match")?;
        for (&l, &r) in left[..block].iter().zip(right[..block].iter()) {
            out.write_all(&float_to_s32(l).to_le_bytes())?;
            out.write_all(&float_to_s32(r).to_le_bytes())?;
        }
        remaining -= block;
    }
    out.flush()?;
    println!(
        "Rust MPFree: {} frames; {} beat triggers; 44,100 Hz stereo S32_LE WAV: {}",
        engine.rendered_frames(),
        engine.beats_emitted(),
        args[1]
    );
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("mpfree-render: {err}");
        std::process::exit(1);
    }
}
