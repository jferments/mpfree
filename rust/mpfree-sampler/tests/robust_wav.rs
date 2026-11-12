//! Robustness: `Source::parse_wav` answers every damaged WAV file (a bad
//! stick, a half-copied file, a WAV from another program) with an error or a
//! sound it can play: never a panic, and never a sample outside -1..1 or
//! not finite, which would blow up the mix.

#[path = "../../test-support/mutate.rs"]
mod mutate;

use mpfree_sampler::PreparedSample as Source;
use mutate::Mutator;

/// A valid WAV: `tag` 1 (PCM) or 3 (float), `bits`, `channels`, a few frames,
/// with an extra chunk before the data now and then.
fn wav(tag: u16, bits: u16, channels: u16, frames: usize, extra: bool) -> Vec<u8> {
    let rate = 44_100u32;
    let align = channels * bits / 8;
    let mut data = Vec::new();
    for f in 0..frames * usize::from(channels) {
        let x = ((f as f32) * 0.37).sin() * 0.8;
        match (tag, bits) {
            (3, 32) => data.extend(x.to_le_bytes()),
            (1, 16) => data.extend(((x * 32767.0) as i16).to_le_bytes()),
            (1, 24) => data.extend(&((x * 8_388_607.0) as i32).to_le_bytes()[..3]),
            _ => data.extend(((x * 2_147_483_647.0) as i32).to_le_bytes()),
        }
    }
    let mut body = b"WAVE".to_vec();
    body.extend(b"fmt ");
    body.extend(16u32.to_le_bytes());
    body.extend(tag.to_le_bytes());
    body.extend(channels.to_le_bytes());
    body.extend(rate.to_le_bytes());
    body.extend((rate * u32::from(align)).to_le_bytes());
    body.extend(align.to_le_bytes());
    body.extend(bits.to_le_bytes());
    if extra {
        body.extend(b"LIST");
        body.extend(5u32.to_le_bytes());
        body.extend(b"INFOx\0");
    }
    body.extend(b"data");
    body.extend((data.len() as u32).to_le_bytes());
    body.extend(&data);
    if data.len() % 2 == 1 {
        body.push(0); // a chunk's pad byte
    }
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

fn seeds() -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    for (tag, bits) in [(1, 16), (1, 24), (1, 32), (3, 32)] {
        for channels in [1, 2] {
            v.push(wav(tag, bits, channels, 40, false));
            v.push(wav(tag, bits, channels, 7, true));
        }
    }
    v
}

#[test]
fn the_seeds_are_valid() {
    for (i, s) in seeds().iter().enumerate() {
        let src = Source::parse_wav(s).unwrap_or_else(|e| panic!("seed {i}: {e:?}"));
        assert!(src.frames() > 0, "seed {i}");
    }
}

#[test]
fn damaged_wavs_are_refused_or_safe_to_play() {
    let seeds = seeds();
    let mut m = Mutator::new(1);
    let (mut ok, mut refused) = (0, 0);
    for case in 0..30_000 {
        let input = m.mutate(&seeds[case % seeds.len()]);
        match Source::parse_wav(&input) {
            Ok(src) => {
                ok += 1;
                assert!(src.frames() > 0, "case {case}: no frames");
                for (i, f) in src.frame_slice().iter().enumerate() {
                    for x in f {
                        assert!(
                            x.is_finite() && (-1.0..=1.0).contains(x),
                            "case {case}, frame {i}: sample {x}"
                        );
                    }
                }
            }
            Err(_) => refused += 1,
        }
    }
    // Both answers happen, so the damage reaches past the header checks.
    assert!(ok > 1_000 && refused > 1_000, "ok {ok}, refused {refused}");
}

#[test]
fn random_bytes_are_refused() {
    let mut m = Mutator::new(2);
    for case in 0..5_000 {
        let n = m.below(512);
        let mut input = m.bytes(n);
        if case % 2 == 0 && input.len() >= 12 {
            input[..4].copy_from_slice(b"RIFF");
            input[8..12].copy_from_slice(b"WAVE");
        }
        assert!(Source::parse_wav(&input).is_err(), "case {case}");
    }
}
