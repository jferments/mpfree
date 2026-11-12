//! Files from a real encoder (FFmpeg's; tests/data/make.py makes them),
//! decoded and checked sample for sample against the signal they hold.

use std::io::Cursor;

use mpfree_flac::{decode, find_frame, overview, Assign, Decoder, Error, StreamInfo};

const N: i64 = 4800;

fn hash32(x: u64) -> u32 {
    let mut x = x as u32;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    x
}

fn noise(n: i64, ch: i64, amp: i64) -> i64 {
    i64::from(hash32((n * 2 + ch + 1) as u64)) % (2 * amp + 1) - amp
}

fn tri(n: i64, period: i64, amp: i64) -> i64 {
    let p = n % period;
    let half = period / 2;
    let v = if p < half { p } else { period - p };
    v * 2 * amp / half - amp
}

/// The signal make.py writes: sample `n` of channel `ch` at `bits` bits.
fn sample(n: i64, ch: i64, bits: u32) -> i64 {
    let full = (1i64 << (bits - 1)) - 1;
    let seg = n * 8 / N;
    let mut v = match seg {
        0 => 0,
        1 => tri(n, 97 + 13 * ch, full / 2),
        2 => tri(n, 211, full / 3) + noise(n, ch, full / 64),
        3 => noise(n, ch, full),
        4 => {
            let p = n % 150 - 75;
            full * 2 * (5625 - p * p) / 5625 - full
        }
        5 => {
            if (n / 3) % 2 == 0 {
                full
            } else {
                -full - 1
            }
        }
        6 => {
            if (n / 40) % 2 == 0 {
                full / 4
            } else {
                -(full / 4)
            }
        }
        _ => noise(n, ch, 3),
    };
    if ch == 1 && seg != 3 && seg != 5 {
        v = ((v * 3) >> 2) + noise(n, 7, 2);
    }
    v
}

fn file(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/{name}.flac", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Every frame of a stream as whole numbers, channel by channel, and how
/// each frame's channels were coded.
fn frames(b: &[u8]) -> (StreamInfo, Vec<Vec<i64>>, Vec<Assign>) {
    let (info, mut at) = StreamInfo::read(b).unwrap();
    let mut dec = Decoder::new(info);
    let mut chans = vec![Vec::new(); usize::from(info.channels)];
    let mut assigns = Vec::new();
    while at < b.len() {
        let (h, len) = dec.frame(&b[at..]).unwrap();
        assert_eq!(h.first, chans[0].len() as u64, "each frame knows its place");
        for (c, out) in chans.iter_mut().enumerate() {
            out.extend_from_slice(dec.channel(c));
        }
        assigns.push(h.assign);
        at += len;
    }
    (info, chans, assigns)
}

#[test]
fn every_file_decodes_to_the_signal_it_holds_bit_for_bit() {
    // (file, channels, bits, wasted bits, a channel coding it must use)
    let cases = [
        ("s16-stereo-auto", 2, 16, 0, None),
        ("s16-stereo-left-fixed", 2, 16, 0, Some(Assign::LeftSide)),
        ("s16-stereo-right-lpc32", 2, 16, 0, Some(Assign::SideRight)),
        ("s16-mono-verbatim", 1, 16, 0, Some(Assign::Independent)),
        ("s24-stereo-mid", 2, 24, 0, Some(Assign::MidSide)),
        ("s24-mono-wasted", 1, 24, 8, None),
        ("s32-stereo", 2, 32, 0, None),
    ];
    for (name, channels, bits, wasted, assign) in cases {
        let b = file(name);
        let (info, chans, assigns) = frames(&b);
        assert_eq!(
            (info.rate, info.channels, info.bits, info.total),
            (44_100, channels, bits, N as u64),
            "{name}"
        );
        for (c, got) in chans.iter().enumerate() {
            assert_eq!(got.len(), N as usize, "{name}");
            for (n, &v) in got.iter().enumerate() {
                let want = sample(n as i64, c as i64, u32::from(bits) - wasted) << wasted;
                assert_eq!(v, want, "{name}: channel {c}, sample {n}");
            }
        }
        if let Some(a) = assign {
            assert!(assigns.contains(&a), "{name}: {assigns:?}");
        }
        // The whole file at once: the same samples, as -1 to 1.
        let d = decode(&b, 1 << 20).unwrap();
        assert!(!d.cut_short && d.skipped == 0, "{name}");
        assert_eq!(d.frames(), N as usize);
        let scale = (1u64 << (bits - 1)) as f64;
        for (i, &x) in d.samples.iter().enumerate() {
            let want = chans[i % usize::from(channels)][i / usize::from(channels)];
            assert_eq!(x, (want as f64 / scale) as f32, "{name}: {i}");
        }
        assert!(d.samples.iter().all(|x| (-1.0..=1.0).contains(x)));
    }
}

/// Where each frame of `b` starts.
fn frame_starts(b: &[u8]) -> Vec<usize> {
    let (info, mut at) = StreamInfo::read(b).unwrap();
    let mut dec = Decoder::new(info);
    let mut starts = Vec::new();
    while at < b.len() {
        starts.push(at);
        at += dec.frame(&b[at..]).unwrap().1;
    }
    starts
}

#[test]
fn tags_cuts_damage_and_stray_bytes_are_each_dealt_with_and_said() {
    let b = file("s16-stereo-auto");
    let whole = decode(&b, 1 << 20).unwrap();
    let starts = frame_starts(&b);
    assert_eq!(starts.len(), 5, "4800 frames in blocks of 1152");

    // An ID3v2 tag in front (with a footer) and an ID3v1 tag at the end.
    let mut tagged = b"ID3\x04\x00\x10\x00\x00\x00\x05hello".to_vec();
    tagged.extend_from_slice(b"3DI\x04\x00\x10\x00\x00\x00\x05");
    tagged.extend_from_slice(&b);
    tagged.extend_from_slice(b"TAG");
    tagged.extend_from_slice(&[0x20; 125]);
    assert_eq!(decode(&tagged, 1 << 20).unwrap(), whole);

    // Cut short in the fourth frame: the three before it, and it says so.
    let cut = decode(&b[..starts[3] + 100], 1 << 20).unwrap();
    assert!(cut.cut_short);
    assert_eq!(cut.frames(), 3 * 1152);
    assert_eq!(&cut.samples[..], &whole.samples[..3 * 1152 * 2]);
    // Cut exactly between frames: STREAMINFO's length says it's short.
    let cut = decode(&b[..starts[2]], 1 << 20).unwrap();
    assert!(cut.cut_short);
    assert_eq!(cut.frames(), 2 * 1152);

    // A byte changed in the third frame's audio: an error saying where,
    // never noise.
    let mut damaged = b.clone();
    damaged[starts[2] + 300] ^= 0x10;
    assert_eq!(
        decode(&damaged, 1 << 20),
        Err(Error::Damaged {
            ms: 2 * 1152 * 1000 / 44_100
        })
    );
    assert_eq!(
        Error::Damaged { ms: 52_245 }.to_string(),
        "a damaged frame 52.24 s in"
    );

    // Stray bytes between two frames are stepped over and counted.
    let mut stray = b[..starts[1]].to_vec();
    stray.extend_from_slice(&[0xFF, 0x00, 0x12, 0x34, 0xFF, 0xF8, 0x00]);
    stray.extend_from_slice(&b[starts[1]..]);
    let d = decode(&stray, 1 << 20).unwrap();
    assert_eq!((d.skipped, d.cut_short), (7, false));
    assert_eq!(d.samples, whole.samples);

    // Too long for the caller, not FLAC at all, nothing in it.
    assert_eq!(
        decode(&b, 4800 * 2 - 1),
        Err(Error::TooLong {
            frames: 4800,
            max: 4799
        })
    );
    assert_eq!(decode(b"RIFF....WAVE", 1 << 20), Err(Error::NotFlac));
    assert_eq!(decode(&b[..starts[0]], 1 << 20), Err(Error::Empty));
    assert_eq!(decode(&b[..20], 1 << 20), Err(Error::Short));
    assert_eq!(find_frame(&b[starts[1] - 3..], &whole.info), Some(3));
}

#[test]
fn the_browsers_picture_takes_each_column_from_the_frame_at_its_place() {
    for (name, block) in [("s16-stereo-auto", 1152), ("s24-stereo-mid", 2048)] {
        let b = file(name);
        let (info, chans, _) = frames(&b);
        let full = (1u64 << (info.bits - 1)) as f32;
        for columns in [1, 7, 60, 300] {
            let pic = overview(&mut Cursor::new(&b), columns, 512).unwrap();
            assert_eq!(pic.len(), columns);
            let per = (N as usize).div_ceil(columns);
            for (col, &(lo, hi)) in pic.iter().enumerate() {
                let want = col * per;
                if want >= N as usize {
                    assert_eq!((lo, hi), (0.0, 0.0));
                    continue;
                }
                // From `want` to 512 frames on, or the end of its frame.
                let end = (want + 512).min((want / block + 1) * block).min(N as usize);
                let (mut a, mut z) = (0f32, 0f32);
                for ch in &chans {
                    for &v in &ch[want..end] {
                        a = a.min(v as f32 / full);
                        z = z.max(v as f32 / full);
                    }
                }
                assert_eq!((lo, hi), (a, z), "{name}, {columns} columns: {col}");
            }
        }
    }
    assert_eq!(overview(&mut Cursor::new(b"RIFF....WAVE"), 10, 512), None);
}

#[test]
fn damaged_and_cut_files_are_errors_or_what_they_hold_never_a_panic() {
    let b = file("s24-stereo-mid");
    for at in (0..b.len()).step_by(37) {
        for bit in [0x01, 0x80] {
            let mut d = b.clone();
            d[at] ^= bit;
            if let Ok(x) = decode(&d, 1 << 20) {
                assert!(x.frames() <= 4800 + 4608, "{at}");
            }
            let _ = overview(&mut Cursor::new(&d), 50, 512);
        }
        let _ = decode(&b[..at], 1 << 20);
        let _ = overview(&mut Cursor::new(&b[..at]), 50, 512);
    }
}
