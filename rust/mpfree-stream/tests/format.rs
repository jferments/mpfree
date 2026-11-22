//! Reading sound files in their own format: each kind of WAV and AIFF the
//! layer streams is built here byte by byte, and its layout and decoded
//! samples are checked exactly.

use std::io::Cursor;

use mpfree_stream::format::{decode, layout, Container, Endian, Layout, Sample};

fn chunk(id: &[u8; 4], body: &[u8], be: bool) -> Vec<u8> {
    let mut c = id.to_vec();
    let n = body.len() as u32;
    c.extend_from_slice(&if be { n.to_be_bytes() } else { n.to_le_bytes() });
    c.extend_from_slice(body);
    if body.len() % 2 == 1 {
        c.push(0);
    }
    c
}

fn fmt(tag: u16, channels: u16, rate: u32, bits: u16) -> Vec<u8> {
    let align = channels * bits / 8;
    let mut f = Vec::new();
    f.extend_from_slice(&tag.to_le_bytes());
    f.extend_from_slice(&channels.to_le_bytes());
    f.extend_from_slice(&rate.to_le_bytes());
    f.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    f.extend_from_slice(&align.to_le_bytes());
    f.extend_from_slice(&bits.to_le_bytes());
    f
}

fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = chunks.concat();
    let mut f = b"RIFF".to_vec();
    f.extend_from_slice(&(4 + body.len() as u32).to_le_bytes());
    f.extend_from_slice(b"WAVE");
    f.extend_from_slice(&body);
    f
}

fn read(b: &[u8]) -> std::io::Result<Layout> {
    layout(&mut Cursor::new(b))
}

fn audio(l: &Layout, file: &[u8]) -> Vec<f32> {
    let from = l.data_at as usize;
    let to = l.byte_of(l.frames) as usize;
    let mut out = vec![9.0; l.frames as usize * usize::from(l.channels)];
    let n = decode(l.sample, l.endian, &file[from..to], &mut out);
    assert_eq!(n, out.len());
    out
}

#[test]
fn wav_pcm_of_every_size_decodes_exactly() {
    // 16-bit stereo: -32768, 32767, 0, 16384.
    let pcm16: Vec<u8> = [-32768i16, 32767, 0, 16384]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let f = riff(&[
        chunk(b"fmt ", &fmt(1, 2, 44_100, 16), false),
        chunk(b"data", &pcm16, false),
    ]);
    let l = read(&f).unwrap();
    assert_eq!(
        (
            l.container,
            l.data_at,
            l.frames,
            l.channels,
            l.rate,
            l.sample
        ),
        (Container::Wav, 44, 2, 2, 44_100, Sample::I16)
    );
    assert_eq!(audio(&l, &f), vec![-1.0, 32767.0 / 32768.0, 0.0, 0.5]);

    // 24-bit mono: -8388608, 4194304, -1.
    let pcm24: Vec<u8> = [-8_388_608i32, 4_194_304, -1]
        .iter()
        .flat_map(|v| v.to_le_bytes()[..3].to_vec())
        .collect();
    let f = riff(&[
        chunk(b"fmt ", &fmt(1, 1, 48_000, 24), false),
        chunk(b"data", &pcm24, false),
    ]);
    let l = read(&f).unwrap();
    assert_eq!((l.frames, l.sample, l.block_align()), (3, Sample::I24, 3));
    assert_eq!(audio(&l, &f), vec![-1.0, 0.5, -1.0 / 8_388_608.0]);

    // 32-bit and 8-bit.
    let pcm32: Vec<u8> = [i32::MIN, 1 << 30]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let f = riff(&[
        chunk(b"fmt ", &fmt(1, 1, 44_100, 32), false),
        chunk(b"data", &pcm32, false),
    ]);
    let l = read(&f).unwrap();
    assert_eq!(audio(&l, &f), vec![-1.0, 0.5]);
    let f = riff(&[
        chunk(b"fmt ", &fmt(1, 1, 22_050, 8), false),
        chunk(b"data", &[0, 128, 192], false),
    ]);
    let l = read(&f).unwrap();
    assert_eq!((l.sample, l.frames), (Sample::U8, 3));
    assert_eq!(audio(&l, &f), vec![-1.0, 0.0, 0.5]);
}

#[test]
fn wav_float_and_extensible_files_and_chunks_before_the_audio() {
    let fl: Vec<u8> = [0.25f32, -0.75, f32::NAN, 1.5]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    // A LIST chunk of odd length (padded) and a bext before the audio.
    let f = riff(&[
        chunk(b"LIST", b"INFOISFT\x05\0\0\0mpfre", false),
        chunk(b"fmt ", &fmt(3, 2, 96_000, 32), false),
        chunk(b"bext", &[0u8; 602], false),
        chunk(b"data", &fl, false),
    ]);
    let l = read(&f).unwrap();
    assert_eq!((l.sample, l.frames, l.rate), (Sample::F32, 2, 96_000));
    assert_eq!(audio(&l, &f), vec![0.25, -0.75, 0.0, 1.5], "NaN becomes 0");

    let f64s: Vec<u8> = [0.5f64, -0.125]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let f = riff(&[
        chunk(b"fmt ", &fmt(3, 1, 44_100, 64), false),
        chunk(b"data", &f64s, false),
    ]);
    let l = read(&f).unwrap();
    assert_eq!(audio(&l, &f), vec![0.5, -0.125]);

    // WAVE_FORMAT_EXTENSIBLE, 24-bit PCM sub-format.
    let mut ext = fmt(0xFFFE, 2, 48_000, 24);
    ext.extend_from_slice(&22u16.to_le_bytes()); // cbSize
    ext.extend_from_slice(&24u16.to_le_bytes()); // valid bits
    ext.extend_from_slice(&3u32.to_le_bytes()); // channel mask
    ext.extend_from_slice(&1u16.to_le_bytes()); // sub-format: PCM
    ext.extend_from_slice(&[0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]);
    let f = riff(&[
        chunk(b"fmt ", &ext, false),
        chunk(b"data", &[0u8; 12], false),
    ]);
    let l = read(&f).unwrap();
    assert_eq!((l.sample, l.channels, l.frames), (Sample::I24, 2, 2));
}

#[test]
fn rf64_sizes_come_from_ds64_and_a_recording_reads_to_its_end() {
    // RF64: data size FFFFFFFF, the real size in ds64.
    let mut ds64 = Vec::new();
    ds64.extend_from_slice(&0u64.to_le_bytes()); // RIFF size (unused here)
    ds64.extend_from_slice(&16u64.to_le_bytes()); // data size
    ds64.extend_from_slice(&4u64.to_le_bytes()); // frames
    ds64.extend_from_slice(&0u32.to_le_bytes()); // table length
    let mut f = b"RF64\xff\xff\xff\xffWAVE".to_vec();
    f.extend(chunk(b"ds64", &ds64, false));
    f.extend(chunk(b"fmt ", &fmt(3, 1, 44_100, 32), false));
    f.extend_from_slice(b"data\xff\xff\xff\xff");
    let at = f.len() as u64;
    f.extend(std::iter::repeat_n(0u8, 24)); // 6 frames there, 4 said
    let l = read(&f).unwrap();
    assert_eq!(
        (l.container, l.data_at, l.frames, l.cut_short),
        (Container::Rf64, at, 4, false)
    );

    // A plain RIFF with data size FFFFFFFF (a recording copied off after a
    // cut): read to the end, whole frames only, not "cut short".
    let mut f = riff(&[chunk(b"fmt ", &fmt(1, 2, 44_100, 16), false)]);
    f.extend_from_slice(b"data\xff\xff\xff\xff");
    f.extend(std::iter::repeat_n(0u8, 4 * 10 + 3));
    let l = read(&f).unwrap();
    assert_eq!((l.frames, l.cut_short), (10, false));
}

#[test]
fn a_file_shorter_than_its_header_says_is_read_to_its_end() {
    let mut f = riff(&[
        chunk(b"fmt ", &fmt(1, 2, 44_100, 16), false),
        chunk(b"data", &[0u8; 400], false),
    ]);
    f.truncate(44 + 4 * 25 + 2);
    let l = read(&f).unwrap();
    assert_eq!((l.frames, l.cut_short), (25, true));
}

fn aiff_file(
    aifc: Option<&[u8; 4]>,
    channels: u16,
    bits: u16,
    rate80: [u8; 10],
    data: &[u8],
) -> Vec<u8> {
    let frames = (data.len() / (usize::from(channels) * usize::from(bits.div_ceil(8)))) as u32;
    let mut comm = Vec::new();
    comm.extend_from_slice(&channels.to_be_bytes());
    comm.extend_from_slice(&frames.to_be_bytes());
    comm.extend_from_slice(&bits.to_be_bytes());
    comm.extend_from_slice(&rate80);
    if let Some(c) = aifc {
        comm.extend_from_slice(c);
        comm.extend_from_slice(&[0, 0]); // empty name
    }
    let mut ssnd = vec![0u8; 8]; // offset 0, block size 0
    ssnd.extend_from_slice(data);
    let body = [
        if aifc.is_some() {
            chunk(b"FVER", &[0xA2, 0x80, 0x51, 0x40], true)
        } else {
            Vec::new()
        },
        chunk(b"COMM", &comm, true),
        chunk(b"SSND", &ssnd, true),
    ]
    .concat();
    let mut f = b"FORM".to_vec();
    f.extend_from_slice(&(4 + body.len() as u32).to_be_bytes());
    f.extend_from_slice(if aifc.is_some() { b"AIFC" } else { b"AIFF" });
    f.extend_from_slice(&body);
    f
}

/// 44,100 as an 80-bit extended float.
const R44: [u8; 10] = [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0];
/// 48,000.
const R48: [u8; 10] = [0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0];

#[test]
fn aiff_big_endian_little_endian_and_float() {
    let be16: Vec<u8> = [-32768i16, 16384]
        .iter()
        .flat_map(|v| v.to_be_bytes())
        .collect();
    let f = aiff_file(None, 1, 16, R44, &be16);
    let l = read(&f).unwrap();
    assert_eq!(
        (l.container, l.rate, l.frames, l.sample, l.endian),
        (Container::Aiff, 44_100, 2, Sample::I16, Endian::Big)
    );
    assert_eq!(audio(&l, &f), vec![-1.0, 0.5]);

    let be24: Vec<u8> = [4_194_304i32, -8_388_608]
        .iter()
        .flat_map(|v| v.to_be_bytes()[1..].to_vec())
        .collect();
    let f = aiff_file(None, 2, 24, R48, &be24);
    let l = read(&f).unwrap();
    assert_eq!((l.rate, l.frames, l.channels), (48_000, 1, 2));
    assert_eq!(audio(&l, &f), vec![0.5, -1.0]);

    let le16: Vec<u8> = [16384i16, -16384]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let f = aiff_file(Some(b"sowt"), 2, 16, R44, &le16);
    let l = read(&f).unwrap();
    assert_eq!(l.endian, Endian::Little);
    assert_eq!(audio(&l, &f), vec![0.5, -0.5]);

    let fl: Vec<u8> = [0.25f32, -0.5]
        .iter()
        .flat_map(|v| v.to_be_bytes())
        .collect();
    let f = aiff_file(Some(b"fl32"), 1, 32, R44, &fl);
    let l = read(&f).unwrap();
    assert_eq!(l.sample, Sample::F32);
    assert_eq!(audio(&l, &f), vec![0.25, -0.5]);
}

#[test]
fn what_the_layer_does_not_stream_is_refused_with_its_reason() {
    let refused = |f: &[u8]| read(f).unwrap_err().to_string();
    let six = riff(&[
        chunk(b"fmt ", &fmt(1, 6, 48_000, 16), false),
        chunk(b"data", &[0u8; 24], false),
    ]);
    assert_eq!(refused(&six), "6 channels (mono or stereo only)");
    let slow = riff(&[
        chunk(b"fmt ", &fmt(1, 1, 4_000, 16), false),
        chunk(b"data", &[0u8; 4], false),
    ]);
    assert_eq!(refused(&slow), "4000 Hz (8 to 192 kHz only)");
    let adpcm = riff(&[
        chunk(b"fmt ", &fmt(2, 1, 44_100, 4), false),
        chunk(b"data", &[0u8; 4], false),
    ]);
    assert!(refused(&adpcm).contains("PCM or float only"));
    assert_eq!(refused(&riff(&[])), "WAV has no fmt chunk");
    assert!(refused(b"OggS\0\0\0\0\0\0\0\0").contains("not a WAV or AIFF"));
    let ima = aiff_file(Some(b"ima4"), 1, 16, R44, &[0u8; 4]);
    assert!(refused(&ima).contains("uncompressed only"));
}
