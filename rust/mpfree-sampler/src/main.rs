//! Offline, user-supplied WAV sampler exercise. No MPC or ALSA access.
#![forbid(unsafe_code)]
use mpfree_core::float_to_s32;
use mpfree_sampler::{PreparedSample, Sampler};
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        return Err("Usage: mpfree-sampler INPUT.wav OUTPUT.wav (44.1 kHz mono/stereo PCM16/24/32 or float32; host only)".into());
    }
    let prepared = PreparedSample::read_wav(Path::new(&args[1]))?;
    let mut sampler = Sampler::new(44_100)?;
    let sample_idx = sampler.add_sample(prepared)?;
    sampler.map_note(60, sample_idx)?;
    let _ = sampler.note_on(60, 110);

    let frames = 44_100usize * 2;
    let data_bytes = u32::try_from(frames * 8)?;
    let mut out = BufWriter::new(File::create(&args[2])?);
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_bytes).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&1u16.to_le_bytes())?;
    out.write_all(&2u16.to_le_bytes())?;
    out.write_all(&44_100u32.to_le_bytes())?;
    out.write_all(&(44_100u32 * 8).to_le_bytes())?;
    out.write_all(&8u16.to_le_bytes())?;
    out.write_all(&32u16.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data_bytes.to_le_bytes())?;
    let mut l = [0.0f32; 128];
    let mut r = [0.0f32; 128];
    let mut rendered = 0usize;
    while rendered < frames {
        if rendered == 44_100 {
            sampler.note_off(60); // 64-frame declicked release at 1 second
        }
        let until_release = if rendered < 44_100 { 44_100 - rendered } else { 128 };
        let count = (frames - rendered).min(128).min(until_release);
        sampler.render(&mut l[..count], &mut r[..count])?;
        for i in 0..count {
            out.write_all(&float_to_s32(l[i]).to_le_bytes())?;
            out.write_all(&float_to_s32(r[i]).to_le_bytes())?;
        }
        rendered += count;
    }
    out.flush()?;
    println!("MPFree sampler: 88200 frames; 44,100 Hz stereo S32_LE; samples={}; voice steals={}; output={}",
        sampler.sample_count(), sampler.voice_steals(), args[2]);
    Ok(())
}
fn main() {
    if let Err(err) = run() {
        eprintln!("mpfree-sampler: {err}");
        std::process::exit(1);
    }
}
