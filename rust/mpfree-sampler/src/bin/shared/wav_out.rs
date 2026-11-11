//! WAV output shared by the workstation tools (`mpfree-chop`, `mpfree-grain`).

use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// 16-bit stereo PCM WAV.
pub fn write_wav(
    path: &Path,
    rate: u32,
    left: &[f32],
    right: &[f32],
) -> Result<(), Box<dyn Error>> {
    let frames = left.len().min(right.len());
    let data_bytes = u32::try_from(frames * 4)?;
    let mut w = BufWriter::new(File::create(path)?);
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_bytes).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?; // PCM
    w.write_all(&2u16.to_le_bytes())?;
    w.write_all(&rate.to_le_bytes())?;
    w.write_all(&(rate * 4).to_le_bytes())?;
    w.write_all(&4u16.to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data_bytes.to_le_bytes())?;
    let q = |x: f32| (x.clamp(-1.0, 1.0) * 32767.0).round() as i16;
    for i in 0..frames {
        w.write_all(&q(left[i]).to_le_bytes())?;
        w.write_all(&q(right[i]).to_le_bytes())?;
    }
    w.flush()?;
    Ok(())
}
