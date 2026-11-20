//! A region's loudest frame (`Region::peak`), what Normalize brings to
//! 0 dBFS: only between its start and end, and only on the channels it
//! plays.

use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

/// Stereo: left peaks at 0.5 (frame 100), right at -0.8 (frame 300),
/// and frame 500 is (0.4, 0.4), whose mono sum is the loudest.
fn stereo() -> Source {
    let mut data = vec![0.0f32; 1000 * 2];
    data[100 * 2] = 0.5;
    data[300 * 2 + 1] = -0.8;
    data[500 * 2] = 0.45;
    data[500 * 2 + 1] = 0.45;
    data[900 * 2] = 0.95;
    Source::new("stereo", RATE, 2, data).unwrap()
}

#[test]
fn the_peak_is_the_loudest_frame_on_the_channels_the_region_plays() {
    let s = stereo();
    let mut r = Region::whole(&s);
    r.end = 800;
    let peak = |r: &Region| r.peak(&s);
    assert_eq!(peak(&r), 0.8, "stereo: the louder side, by size");
    r.channels = ChannelPick::Left;
    assert_eq!(peak(&r), 0.5);
    r.channels = ChannelPick::Right;
    assert_eq!(peak(&r), 0.8);
    r.channels = ChannelPick::MonoSum;
    assert_eq!(peak(&r), 0.45, "(0.45 + 0.45) / 2 beats (0 - 0.8) / 2");
    // Only inside the region: frame 900 is past its end, frame 100
    // before its start.
    r.channels = ChannelPick::Stereo;
    r.start = 200;
    r.end = 1000;
    assert_eq!(peak(&r), 0.95);
    r.end = 400;
    assert_eq!(peak(&r), 0.8);
    r.start = 301;
    assert_eq!(peak(&r), 0.0, "silence");
    // Its gain, fades and reverse don't count.
    let mut g = Region::whole(&s);
    g.gain_db = -12.0;
    g.reverse = true;
    g.fade_in.frames = 950;
    assert_eq!(g.peak(&s), 0.95);
}

#[test]
fn a_mono_sound_peaks_on_its_one_channel_whatever_the_pick() {
    let mut data = vec![0.0f32; 64];
    data[10] = -0.25;
    data[20] = 0.125;
    let s = Source::new("mono", RATE, 1, data).unwrap();
    let mut r = Region::whole(&s);
    for pick in [
        ChannelPick::Stereo,
        ChannelPick::MonoSum,
        ChannelPick::Left,
        ChannelPick::Right,
    ] {
        r.channels = pick;
        assert_eq!(r.peak(&s), 0.25, "{pick:?}");
    }
    // A region past the sound's end reads what there is.
    r.start = 15;
    r.end = 100;
    assert_eq!(r.peak(&s), 0.125);
    r.start = 200;
    assert_eq!(r.peak(&s), 0.0);
}

/// A small random number source, the same every run.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    /// From 0 to 1.
    fn unit(&mut self) -> f32 {
        self.next() as f32 / u32::MAX as f32
    }
    fn below(&mut self, n: usize) -> usize {
        self.next() as usize % n.max(1)
    }
}

/// Sampler plan, phase 1 step 5: Normalize's peak from the loudness
/// summary ([`Region::peak_from`]) is the frame-by-frame peak
/// ([`Region::peak`]) to the bit, on every kind of sound and stretch:
/// floats and 16 bits, one side and two, sides that cancel in Mono sum,
/// past full scale, silence and near silence, values on the summary's
/// rounding edges, and regions that start and end anywhere.
#[test]
fn the_peak_from_the_summary_is_the_frame_by_frame_peak_exactly() {
    use mpfree_analysis::PeakPyramid;
    let mut rng = Rng(0x9E37_79B9);
    let frames = 50_011; // more than 12 of the summary's largest blocks, and not whole ones
    type Make = fn(&mut Rng, usize, usize) -> f32;
    let kinds: [(&str, u8, Make); 9] = [
        ("noise with spikes", 2, |r, _, _| {
            let x = (r.unit() - 0.5) * 0.2;
            if r.below(5000) == 0 {
                x * 8.0
            } else {
                x
            }
        }),
        ("past full scale", 2, |r, i, _| {
            if i % 9001 == 17 {
                1.0 + r.unit()
            } else {
                (r.unit() - 0.5) * 1.6
            }
        }),
        ("sides that cancel", 2, |r, i, ch| {
            let x = ((i as f32 * 0.01).sin() * 0.7 + (r.unit() - 0.5) * 1e-4) * 0.5;
            if ch == 0 {
                x
            } else {
                -x
            }
        }),
        ("silence", 2, |_, _, _| 0.0),
        ("near silence", 2, |r, i, _| {
            if i % 777 == 0 {
                (r.unit() - 0.5) * 3e-5
            } else {
                0.0
            }
        }),
        ("rounding edges", 2, |r, _, _| {
            // Just either side of where the summary's rounding turns.
            let m = r.below(32_000) as f32;
            let x = (m + 0.5) / 32_767.0;
            let x = match r.below(3) {
                0 => x,
                1 => f32::from_bits(x.to_bits() + 1),
                _ => f32::from_bits(x.to_bits() - 1),
            };
            if r.below(2) == 0 {
                x
            } else {
                -x
            }
        }),
        ("one quiet spot", 2, |r, i, _| {
            if i == 31_337 {
                0.3
            } else {
                (r.unit() - 0.5) * 0.01
            }
        }),
        ("mono noise", 1, |r, _, _| (r.unit() - 0.5) * 0.9),
        ("mono near silence", 1, |r, i, _| {
            if i % 501 == 0 {
                r.unit() * 2e-5
            } else {
                0.0
            }
        }),
    ];
    for (name, ch, make) in kinds {
        let chs = usize::from(ch);
        let floats: Vec<f32> = (0..frames * chs)
            .map(|i| make(&mut rng, i / chs, i % chs))
            .collect();
        let sixteen: Vec<f32> = floats
            .iter()
            .map(|x| ((x.clamp(-1.0, 32_767.0 / 32_768.0) * 32_768.0).round() + 0.0) / 32_768.0)
            .collect();
        for (bits, data) in [("floats", floats), ("16 bits", sixteen)] {
            let s = Source::new(name, RATE, ch, data).unwrap();
            // (Silence held as floats is held as 16 bits: every sample is
            // a 16-bit value.)
            assert!(s.data().is_16_bit() || bits == "floats", "{name}");
            let summary = PeakPyramid::build_from(s.data(), chs).unwrap();
            let mut r = Region::whole(&s);
            for trial in 0..200 {
                (r.start, r.end) = match trial {
                    0 => (0, s.frames()),
                    1 => (0, 1),
                    2 => (s.frames() - 1, s.frames()),
                    3 => (4095, 4097),
                    _ => {
                        let a = rng.below(frames) as u64;
                        let len = if trial % 3 == 0 {
                            rng.below(300)
                        } else {
                            rng.below(frames)
                        };
                        (a, (a + 1 + len as u64).min(s.frames()))
                    }
                };
                for pick in [
                    ChannelPick::Stereo,
                    ChannelPick::MonoSum,
                    ChannelPick::Left,
                    ChannelPick::Right,
                ] {
                    r.channels = pick;
                    let want = r.peak(&s);
                    let got = r.peak_from(&s, &summary).unwrap();
                    assert_eq!(
                        got.to_bits(),
                        want.to_bits(),
                        "{name}, {bits}, {pick:?}, {}..{}: {got} for {want}",
                        r.start,
                        r.end
                    );
                }
            }
        }
    }
}

/// A summary of another sound is no summary of this one.
#[test]
fn the_peak_from_a_summary_of_another_sound_is_none() {
    use mpfree_analysis::PeakPyramid;
    let s = stereo();
    let r = Region::whole(&s);
    let other = PeakPyramid::build(&[0.0; 999 * 2], 2).unwrap();
    assert_eq!(r.peak_from(&s, &other), None);
    let mono = PeakPyramid::build(&[0.0; 1000], 1).unwrap();
    assert_eq!(r.peak_from(&s, &mono), None);
    let own = PeakPyramid::build_from(s.data(), 2).unwrap();
    assert_eq!(r.peak_from(&s, &own), Some(0.95));
}
