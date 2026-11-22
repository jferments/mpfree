//! The sampler loads MPFree's own recordings exactly: finished takes
//! (24-bit and float with its `fact` chunk, mono and stereo, with a pad
//! byte when odd) and takes past 4 GiB as RF64, whose `ds64` size keeps
//! the audio apart from chunks after it.

use mpfree_sampler::{sound_file, wav};
use mpfree_stream::rec::RecFormat;
use mpfree_stream::testkit::{as_rf64, folder, played, record_take};

#[test]
fn takes_and_their_rf64_form_load_exactly() {
    let dir = folder("sampler-recordings");
    for fmt in [
        RecFormat {
            float: true,
            channels: 2,
            rate: 48_000,
        },
        RecFormat {
            float: true,
            channels: 1,
            rate: 44_100,
        },
        RecFormat {
            float: false,
            channels: 2,
            rate: 48_000,
        },
        RecFormat {
            float: false,
            channels: 1,
            rate: 44_100,
        },
    ] {
        let path = record_take(&dir, fmt, 20_001, 5);
        let bytes = std::fs::read(&path).unwrap();
        for b in [bytes.clone(), as_rf64(&bytes)] {
            let (s, note) = sound_file::read("take", &b).unwrap();
            assert_eq!(note, None);
            assert_eq!(
                (s.rate(), u16::from(s.channels()), s.frames()),
                (fmt.rate, fmt.channels, 20_001)
            );
            let ch = usize::from(fmt.channels);
            for (i, x) in s.data().iter().enumerate() {
                assert_eq!(x, played(5, (i / ch) as u64, i % ch), "{fmt:?} sample {i}");
            }
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_rf64_data_size_stops_before_the_chunks_after_it() {
    let dir = folder("sampler-rf64-chunks");
    let fmt = RecFormat {
        float: false,
        channels: 1,
        rate: 44_100,
    };
    let path = record_take(&dir, fmt, 1001, 2);
    let mut b = as_rf64(&std::fs::read(&path).unwrap());
    // A smpl chunk after the audio (and its pad byte): root note 60.
    let mut smpl = vec![0u8; 36];
    smpl[12..16].copy_from_slice(&60u32.to_le_bytes());
    b.extend_from_slice(b"smpl");
    b.extend_from_slice(&(smpl.len() as u32).to_le_bytes());
    b.extend_from_slice(&smpl);
    let s = wav::read("take", &b).unwrap();
    assert_eq!(s.frames(), 1001, "the smpl chunk isn't read as audio");
    assert_eq!(wav::marks(&b).root, Some(60));
    std::fs::remove_dir_all(&dir).unwrap();
}
