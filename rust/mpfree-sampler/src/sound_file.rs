//! Any sound file people bring, known by what it holds rather than by its
//! name: WAV ([`wav::read`]), AIFF ([`aiff::read`]) or FLAC (decoded by
//! `mpfree-flac`, every frame's check sum checked).
//!
//! Nothing here touches the disk: the caller reads the bytes.

use crate::model::{ModelError, Source};
use crate::{aiff, wav};

/// The sound in a file's bytes, named `name`, and a note when something
/// was left out (a file cut short, stray bytes between FLAC frames).
pub fn read(name: &str, b: &[u8]) -> Result<(Source, Option<String>), String> {
    match b.get(..4) {
        Some(b"RIFF") => wav::read(name, b).map(|s| (s, None)),
        Some(b"FORM") => aiff::read(name, b),
        Some(b"fLaC") => flac(name, b),
        Some(x) if &x[..3] == b"ID3" => flac(name, b),
        _ => Err(NOT_SOUND.into()),
    }
}

const NOT_SOUND: &str = "not a WAV, AIFF or FLAC file";

fn flac(name: &str, b: &[u8]) -> Result<(Source, Option<String>), String> {
    use mpfree_flac::{decode, Error, StreamInfo};
    let said = |e: Error| match e {
        // An ID3 tag in front of something else (an MP3, say).
        Error::NotFlac => NOT_SOUND.to_string(),
        e => e.to_string(),
    };
    let (info, _) = StreamInfo::read(b).map_err(said)?;
    if info.channels > 2 {
        return Err(format!("{} channels (mono or stereo only)", info.channels));
    }
    let d = decode(b, wav::MAX_SAMPLES).map_err(said)?;
    let mut notes = Vec::new();
    if d.cut_short {
        notes.push(format!(
            "the file is cut short: its first {} frames",
            d.frames()
        ));
    }
    if d.skipped > 0 {
        notes.push(format!(
            "{} stray bytes between frames stepped over",
            d.skipped
        ));
    }
    let s = Source::new(name, info.rate, info.channels, d.samples).map_err(|e| match e {
        ModelError::BadSource("rate") => format!("{} Hz (8 to 192 kHz only)", info.rate),
        e => format!("{e}"),
    })?;
    Ok((s, (!notes.is_empty()).then(|| notes.join("; "))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flac_file(name: &str) -> Vec<u8> {
        let path = format!(
            "{}/../mpfree-flac/tests/data/{name}.flac",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn a_flac_loads_bit_for_bit_and_is_kept_as_a_wav_of_its_own_bits() {
        for (name, bits, enc) in [
            ("s16-stereo-auto", 16, wav::Encoding::Pcm16),
            ("s24-stereo-mid", 24, wav::Encoding::Pcm24),
            ("s24-mono-wasted", 24, wav::Encoding::Pcm16),
        ] {
            let (s, note) = read("Loop", &flac_file(name)).unwrap();
            assert_eq!(note, None, "{name}");
            assert_eq!(
                (s.rate(), s.frames(), s.name.as_str()),
                (44_100, 4800, "Loop")
            );
            // Every sample a whole step of the file's bits, so a saved
            // project keeps it as a WAV of those bits, the same sound.
            assert_eq!(wav::encoding(&s), enc, "{name}: {bits}-bit");
            let back = wav::read("Loop", &wav::bytes(&s)).unwrap();
            assert_eq!(back.id(), s.id());
        }
    }

    #[test]
    fn files_are_known_by_what_they_hold_and_what_was_left_out_is_said() {
        // A WAV and an AIFF of the same sound are the same sound.
        let src = Source::new("t", 48_000, 1, vec![0.5, -0.25, 0.0]).unwrap();
        let (w, _) = read("t", &wav::bytes(&src)).unwrap();
        assert_eq!(w.id(), src.id());
        let mut aiff = b"FORM\0\0\0\x2EAIFFCOMM\0\0\0\x12\0\x01\0\0\0\x03\0\x10".to_vec();
        aiff.extend_from_slice(&[0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0]); // 48 kHz
        aiff.extend_from_slice(b"SSND\0\0\0\x0E\0\0\0\0\0\0\0\0\x40\0\xE0\0\0\0");
        let (a, _) = read("t", &aiff).unwrap();
        assert_eq!(a.id(), src.id());
        // A FLAC cut short, and one with stray bytes after a tag.
        let f = flac_file("s16-stereo-auto");
        let (cut, note) = read("t", &f[..f.len() * 3 / 4]).unwrap();
        assert!(cut.frames() < 4800);
        assert_eq!(
            note.unwrap(),
            format!("the file is cut short: its first {} frames", cut.frames())
        );
        let (info, start) = mpfree_flac::StreamInfo::read(&f).unwrap();
        assert_eq!(info.channels, 2);
        let mut stray = f[..start].to_vec();
        stray.extend_from_slice(&[1, 2, 3]);
        stray.extend_from_slice(&f[start..]);
        assert_eq!(
            read("t", &stray).unwrap().1.as_deref(),
            Some("3 stray bytes between frames stepped over")
        );
        // What it can't take, it says.
        assert_eq!(
            read("t", b"ID3\x04\0\0\0\0\0\0\xFF\xFB").unwrap_err(),
            NOT_SOUND
        );
        assert_eq!(read("t", b"OggS....").unwrap_err(), NOT_SOUND);
        assert_eq!(read("t", b"").unwrap_err(), NOT_SOUND);
        let mut broken = f.clone();
        broken[start + 2000] ^= 1;
        assert!(read("t", &broken)
            .unwrap_err()
            .starts_with("a damaged frame 0."));
    }
}
