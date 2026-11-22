//! The browser reads MPFree's own recordings: finished takes (24-bit and
//! float with its `fact` chunk, mono and stereo), takes past 4 GiB as
//! RF64, and a take still being written (sizes `FFFFFFFF`).

use std::fs::File;
use std::io::Cursor;

use mpfree_library::audio::{overview, read_info, AudioFormat};
use mpfree_stream::fsio::{Fs, StdFs};
use mpfree_stream::rec::{header, RecFormat, RecId, Sizes, RIFF_MAX};
use mpfree_stream::testkit::{as_rf64, folder, record_take};

const FORMATS: [RecFormat; 4] = [
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
];

#[test]
fn finished_takes_and_their_rf64_form_read_back() {
    let dir = folder("library-recordings");
    for fmt in FORMATS {
        let path = record_take(&dir, fmt, 20_001, 3);
        let bytes = std::fs::read(&path).unwrap();
        for b in [bytes.clone(), as_rf64(&bytes)] {
            let i = read_info(&mut Cursor::new(&b)).unwrap();
            assert_eq!(i.format, AudioFormat::Wav);
            assert_eq!(
                (i.rate, i.channels, i.bits, i.float, i.frames),
                (fmt.rate, fmt.channels, fmt.bits(), fmt.float, 20_001)
            );
            let cols = overview(&mut Cursor::new(&b), 40).expect("a picture");
            assert_eq!(cols.len(), 40);
            assert!(cols.iter().any(|&(lo, hi)| hi > lo), "not silence");
        }
        let i = read_info(&mut File::open(&path).unwrap()).unwrap();
        assert_eq!(i.frames, 20_001);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_take_still_being_written_reads_to_the_end_of_its_file() {
    let fmt = FORMATS[0];
    let mut b = header(fmt, &RecId([1; 16]), Sizes::Open);
    b.extend(std::iter::repeat_n(0x11u8, 1000 * fmt.block_align() + 3));
    let i = read_info(&mut Cursor::new(&b)).unwrap();
    assert_eq!(i.frames, 1000);
}

#[test]
fn just_under_and_over_the_riff_limit_with_sparse_files() {
    let dir = folder("library-riff-limit");
    let fmt = FORMATS[0];
    let under = RIFF_MAX / fmt.block_align() as u64;
    for frames in [under, under + 1] {
        let p = dir.join(format!("{frames}.wav"));
        let sizes = Sizes::done(fmt, frames);
        let Sizes::Done { end, .. } = sizes else {
            unreachable!()
        };
        File::create(&p).unwrap().set_len(end).unwrap();
        StdFs::new()
            .write_at(&p, 0, &header(fmt, &RecId([2; 16]), sizes))
            .unwrap();
        let i = read_info(&mut File::open(&p).unwrap()).unwrap();
        assert_eq!(i.frames, frames);
        assert!(overview(&mut File::open(&p).unwrap(), 8).is_some());
        std::fs::remove_file(&p).unwrap();
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
