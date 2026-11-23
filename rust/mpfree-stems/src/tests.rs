use super::*;
use crate::net::{Kind, Shape};

/// A small network, the same code as Spleeter's: 64-frame segments
/// (1.5 s), narrow layers.
const SMALL: Shape = Shape {
    filters: [3, 4, 5, 6, 7, 8],
    rows: 64,
};
/// Bins the small tests' networks see (to 1.4 kHz).
const BINS_SMALL: usize = 128;

fn noise(n: usize, seed: u32, level: f32) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((s >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * level
        })
        .collect()
}

/// Every weight zero, every batch norm the identity: each U-Net's mask is
/// sigmoid(0) = 1/2, so each stem's soft mask is 1/4.
fn quarters() -> Model {
    Model::build(SMALL, |n, kind| match kind {
        Kind::Scale => vec![1.0; n],
        _ => vec![0.0; n],
    })
}

/// The stems as they come, by stem: interleaved stereo.
#[derive(Default)]
struct Keep {
    stems: [Vec<f32>; 4],
    calls: usize,
}

impl Sink for Keep {
    fn put(&mut self, stem: Stem, stereo: &[f32]) -> Result<(), String> {
        let k = Stem::ALL.iter().position(|&s| s == stem).unwrap();
        // The stems come in order, a chunk each.
        assert_eq!(k, self.calls % 4, "stems out of order");
        self.calls += 1;
        self.stems[k].extend_from_slice(stereo);
        Ok(())
    }
}

fn separate(model: &Model, input: &[f32], channels: usize, extend: bool, threads: usize) -> Keep {
    let mut sep = Separator::at(model, input, channels, BINS_SMALL, extend, threads).unwrap();
    let mut keep = Keep::default();
    let mut last = 0.0;
    while sep.step(&mut keep).unwrap() {
        let p = sep.progress();
        assert!(p > last && p < 1.0, "progress {last} then {p}");
        last = p;
    }
    assert_eq!(sep.progress(), 1.0);
    assert!(
        !sep.step(&mut keep).unwrap(),
        "a finished separation does nothing"
    );
    keep
}

#[test]
fn with_equal_masks_each_stem_is_a_quarter_of_the_sound() {
    let model = quarters();
    // Three segments and a bit; stereo, the sides different.
    let frames = 4 * 44_100 + 333;
    let input = noise(frames * 2, 7, 0.5);
    let keep = separate(&model, &input, 2, true, 2);
    for (k, s) in keep.stems.iter().enumerate() {
        assert_eq!(s.len(), input.len(), "stem {k}");
        for (i, (&y, &x)) in s.iter().zip(&input).enumerate() {
            assert!(
                (y - x / 4.0).abs() < 2e-5,
                "stem {k} [{i}]: {y} vs {}",
                x / 4.0
            );
        }
    }
}

#[test]
fn spleeters_band_leaves_the_stems_silent_above_it() {
    let model = quarters();
    // A 5 kHz tone, well above the small networks' 1.4 kHz.
    let frames = 2 * 44_100;
    let tone: Vec<f32> = (0..frames)
        .flat_map(|i| {
            let v = (2.0 * std::f32::consts::PI * 5000.0 * i as f32 / 44_100.0).sin() * 0.5;
            [v, v]
        })
        .collect();
    let narrow = separate(&model, &tone, 2, false, 2);
    let full = separate(&model, &tone, 2, true, 2);
    // Away from the ends, where the tone's start and stop are clicks
    // across the whole band.
    let peak = |s: &[f32]| {
        let mid = &s[2 * stft::N..s.len() - 2 * stft::N];
        mid.iter().fold(0f32, |a, &b| a.max(b.abs()))
    };
    for k in 0..4 {
        assert!(
            peak(&narrow.stems[k]) < 1e-4,
            "stem {k}: {}",
            peak(&narrow.stems[k])
        );
        assert!(
            (peak(&full.stems[k]) - 0.125).abs() < 1e-3,
            "stem {k}: {}",
            peak(&full.stems[k])
        );
    }
}

#[test]
fn full_band_stems_add_up_to_the_sound() {
    let model = Model::synthetic_shaped(SMALL, 21);
    let frames = 3 * 44_100 + 5;
    let input = noise(frames * 2, 3, 0.7);
    for threads in [1, 3] {
        let keep = separate(&model, &input, 2, true, threads);
        let peak = input.iter().fold(0f32, |a, &b| a.max(b.abs()));
        for (i, &x) in input.iter().enumerate() {
            let sum: f32 = keep.stems.iter().map(|s| s[i]).sum();
            assert!(
                (sum - x).abs() < 1e-4 * peak,
                "[{i}] threads {threads}: {sum} vs {x}"
            );
        }
        // The masks differ, so the stems do.
        assert_ne!(keep.stems[0], keep.stems[1]);
    }
}

#[test]
fn the_threads_change_nothing() {
    let model = Model::synthetic_shaped(SMALL, 8);
    let input = noise(2 * 44_100 * 2, 4, 0.5);
    let one = separate(&model, &input, 2, true, 1);
    let four = separate(&model, &input, 2, true, 4);
    for k in 0..4 {
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&one.stems[k]), bits(&four.stems[k]), "stem {k}");
    }
}

/// Spleeter's way with a mono sound: both sides the same into the
/// networks. Their two output channels have their own weights, so the
/// stems' sides can differ.
#[test]
fn a_mono_sound_separates_as_stereo_with_both_sides_the_same() {
    let model = Model::synthetic_shaped(SMALL, 9);
    let mono = noise(44_100 + 11, 5, 0.5);
    let keep = separate(&model, &mono, 1, true, 2);
    let stereo: Vec<f32> = mono.iter().flat_map(|&v| [v, v]).collect();
    let both = separate(&model, &stereo, 2, true, 2);
    for k in 0..4 {
        assert_eq!(keep.stems[k].len(), mono.len() * 2);
        assert_eq!(keep.stems[k], both.stems[k], "stem {k}");
    }
}

#[test]
fn a_sound_of_nothing_or_one_frame_separates() {
    let model = Model::synthetic_shaped(SMALL, 2);
    let none = separate(&model, &[], 2, true, 2);
    assert!(none.stems.iter().all(|s| s.is_empty()));
    assert_eq!(none.calls, 0);
    let one = separate(&model, &[0.25, -0.25], 2, true, 2);
    assert!(one.stems.iter().all(|s| s.len() == 2));
}

#[test]
fn it_steps_four_networks_a_segment_and_counts_their_work() {
    let model = Model::synthetic_shaped(SMALL, 1);
    // 64-frame segments: (44100·2 + 4096) / 1024 = 91 frames, two segments.
    let input = noise(2 * 44_100 * 2, 6, 0.5);
    let mut sep = Separator::at(&model, &input, 2, BINS_SMALL, true, 2).unwrap();
    let mut keep = Keep::default();
    let mut steps = 1;
    while sep.step(&mut keep).unwrap() {
        steps += 1;
    }
    assert_eq!(steps, 2 * 6);
    let st = sep.stats();
    assert_eq!(st.segments, 2);
    assert!(st.macs > 0 && st.nets > Duration::ZERO);
}

#[test]
fn what_it_refuses() {
    let model = Model::synthetic_shaped(SMALL, 1);
    let s = Settings::default();
    assert_eq!(
        Separator::new(&model, &[], 2, 48_000, &s).err(),
        Some(Error::Rate(48_000))
    );
    assert_eq!(
        Separator::new(&model, &[], 3, RATE, &s).err(),
        Some(Error::Channels(3))
    );
    assert_eq!(
        Separator::new(&model, &[], 0, RATE, &s).err(),
        Some(Error::Channels(0))
    );

    struct Full;
    impl Sink for Full {
        fn put(&mut self, _: Stem, _: &[f32]) -> Result<(), String> {
            Err("the disk is full".into())
        }
    }
    let input = noise(44_100 * 2, 1, 0.5);
    let mut sep = Separator::at(&model, &input, 2, BINS_SMALL, true, 2).unwrap();
    let err = loop {
        match sep.step(&mut Full) {
            Ok(true) => {}
            Ok(false) => panic!("finished into a full disk"),
            Err(e) => break e,
        }
    };
    assert_eq!(err, Error::Sink("the disk is full".into()));
}

#[test]
fn spleeters_promise_is_about_76_mb_at_1024_bins_and_97_at_1536() {
    let model = Model::synthetic(1);
    let mb = |band| model.need(&Settings { band, threads: 2 }) as f64 / 1e6;
    let (narrow, full) = (mb(Band::Spleeter), mb(Band::Full));
    assert!((70.0..82.0).contains(&narrow), "{narrow} MB");
    assert!((90.0..105.0).contains(&full), "{full} MB");
}
