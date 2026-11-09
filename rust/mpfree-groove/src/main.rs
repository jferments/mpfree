//! Host-only offline WAV drum pattern; no ALSA, network, MPC, or device access.
#![forbid(unsafe_code)]

use mpfree_core::float_to_s32;
use mpfree_groove::{GrooveConfig, GrooveEngine, Pattern};
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};

const BLOCK: usize = 128;

fn put_u16(writer: &mut impl Write, value: u16) -> std::io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}
fn put_u32(writer: &mut impl Write, value: u32) -> std::io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if !(2..=4).contains(&args.len()) {
        return Err("Usage: mpfree-groove <output.wav> [bpm=120] [bars=2]".into());
    }
    let bpm = if args.len() > 2 { args[2].parse::<f64>()? } else { 120.0 };
    let bars = if args.len() > 3 { args[3].parse::<u32>()? } else { 2 };
    let config = GrooveConfig { bpm, bars, ..GrooveConfig::default() };
    config.validate().map_err(|error| format!("invalid groove configuration: {error:?}"))?;
    let frames = config.duration_frames()?;
    if frames > u64::from(config.sample_rate) * 60 {
        return Err("offline demo limited to 60 seconds; choose fewer bars or a faster tempo".into());
    }
    let data_bytes = frames.checked_mul(8).ok_or("WAV payload overflow")?;
    let payload_u32 = u32::try_from(data_bytes).map_err(|_| "WAV exceeds RIFF 32-bit size limit")?;
    let riff_size = 36u32.checked_add(payload_u32).ok_or("RIFF header size overflow")?;

    let mut engine = GrooveEngine::new(&Pattern::basic(), config)?;
    let file = File::create(&args[1])?;
    let mut writer = BufWriter::new(file);
    writer.write_all(b"RIFF")?;
    put_u32(&mut writer, riff_size)?;
    writer.write_all(b"WAVEfmt ")?;
    put_u32(&mut writer, 16)?;
    put_u16(&mut writer, 1)?; // uncompressed signed integer PCM
    put_u16(&mut writer, 2)?; // stereo
    put_u32(&mut writer, config.sample_rate)?;
    put_u32(&mut writer, config.sample_rate * 8)?;
    put_u16(&mut writer, 8)?; // frame byte width
    put_u16(&mut writer, 32)?;
    writer.write_all(b"data")?;
    put_u32(&mut writer, payload_u32)?;

    let mut l = [0.0f32; BLOCK];
    let mut r = [0.0f32; BLOCK];
    let mut remaining = frames;
    while remaining != 0 {
        let n = usize::try_from(remaining.min(BLOCK as u64))?;
        engine.render(&mut l[..n], &mut r[..n])?;
        for (&left, &right) in l[..n].iter().zip(r[..n].iter()) {
            writer.write_all(&float_to_s32(left).to_le_bytes())?;
            writer.write_all(&float_to_s32(right).to_le_bytes())?;
        }
        remaining -= n as u64;
    }
    writer.flush()?;

    println!(
        "Rust MPFree groove: {} frames, {} triggered events, {} prepared sample bytes, \
         {} stolen voices; 44,100 Hz stereo S32_LE WAV: {}",
        engine.rendered_frames(),
        engine.triggered_events(),
        engine.prepared_sample_bytes(),
        engine.stolen_voices(),
        args[1]
    );
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("mpfree-groove: {err}");
        std::process::exit(1);
    }
}
