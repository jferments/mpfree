//! A sound whose samples are all 16-bit values (a 16-bit WAV or AIFF, and
//! anything cut from one) is held as 16 bits, at half the memory of the
//! floats it used to be held as, and plays, saves and reads back exactly as
//! those floats did.

use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_sampler::wav;

const RATE: u32 = 44_100;

/// `frames` of a decaying chord with noise on it, every sample a 16-bit
/// value: `channels` interleaved.
fn chord(frames: usize, channels: u8) -> Vec<f32> {
    let mut x = 7u32;
    (0..frames * usize::from(channels))
        .map(|i| {
            let f = (i / usize::from(channels)) as f32 / RATE as f32;
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
            let tone = (f * 220.0 * std::f32::consts::TAU).sin()
                + 0.5 * (f * 331.0 * std::f32::consts::TAU).sin();
            let y = (0.6 * tone + 0.1 * noise) * (-2.0 * f).exp();
            // (Plus 0, so a sample rounded to -0 is 0, as a WAV holds it.)
            ((y.clamp(-1.0, 1.0) * 32_767.0).round() + 0.0) / 32_768.0
        })
        .collect()
}

fn sixteen_bit(frames: usize, channels: u8) -> Source {
    Source::new("chord", RATE, channels, chord(frames, channels)).unwrap()
}

fn bits(x: &[f32]) -> Vec<u32> {
    x.iter().map(|x| x.to_bits()).collect()
}

/// What a case does to the pad.
type Shape = Box<dyn Fn(&mut Pad)>;

/// Pad 0 playing `s` as `shape` sets it up.
fn kit(s: &Source, shape: &dyn Fn(&mut Pad)) -> Project {
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut pad = Pad::new("pad", Layer::new(Region::whole(s)));
    shape(&mut pad);
    p.set_pad(0, Some(pad), None).unwrap();
    p
}

/// What pad 0 makes from a hit held `held` frames, over `frames`, in
/// blocks of 128.
fn play(p: &Project, held: usize, frames: usize) -> Vec<f32> {
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(p, RATE));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 100,
    });
    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
    let mut out = Vec::with_capacity(frames * 2);
    let mut at = 0;
    while at < frames {
        if at <= held && held < at + 128 {
            e.apply(Event::NoteOff { pad: 0 });
        }
        e.render(&mut l, &mut r);
        out.extend_from_slice(&l);
        out.extend_from_slice(&r);
        at += 128;
    }
    out
}

#[test]
fn a_16_bit_sound_is_held_as_16_bits() {
    let s = sixteen_bit(10_000, 2);
    assert!(s.data().is_16_bit());
    assert_eq!(s.data().bytes(), 10_000 * 2 * 2, "two bytes a sample");
    let floats = s.held_as_floats();
    assert!(!floats.data().is_16_bit());
    assert_eq!(floats.data().bytes(), 10_000 * 2 * 4);
    // The same sound: the same id, the same samples.
    assert_eq!(floats.id(), s.id());
    assert_eq!(floats.data().to_vec(), s.data().to_vec());

    // Read from a 16-bit WAV, it stays 16 bits; from a 24-bit or float one
    // whose samples aren't all 16-bit values, floats.
    let read = wav::read("x", &wav::bytes(&s)).unwrap();
    assert!(read.data().is_16_bit());
    assert_eq!(read.id(), s.id());
    let mut fine = s.data().to_vec();
    fine[1234] += 1.0 / 8_388_608.0;
    let fine = Source::new("fine", RATE, 2, fine).unwrap();
    assert!(!fine.data().is_16_bit());
    assert_eq!(wav::encoding(&fine), wav::Encoding::Pcm24);
    assert!(!wav::read("y", &wav::bytes(&fine))
        .unwrap()
        .data()
        .is_16_bit());
    // A negative zero isn't a 16-bit value: kept as it is.
    let neg = Source::new("neg", RATE, 1, vec![0.5, -0.0, 0.25]).unwrap();
    assert!(!neg.data().is_16_bit());
    assert_eq!(neg.data().get(1).to_bits(), (-0.0f32).to_bits());
}

#[test]
fn it_saves_as_its_floats_did() {
    let s = sixteen_bit(5_000, 1);
    let floats = s.held_as_floats();
    assert_eq!(wav::encoding(&s), wav::Encoding::Pcm16);
    assert_eq!(wav::bytes(&s), wav::bytes(&floats), "the same file");
}

#[test]
fn a_16_bit_sound_plays_as_its_floats_do() {
    let lp = |mode| Loop {
        start: 9_000,
        end: 30_000,
        crossfade: 2_000,
        mode,
    };
    let shapes: Vec<(&str, Shape)> = vec![
        ("one shot", Box::new(|_: &mut Pad| {})),
        (
            "left channel, tuned down",
            Box::new(|p: &mut Pad| {
                p.layers[0].region.channels = ChannelPick::Left;
                p.tune_cents = -500;
            }),
        ),
        (
            "mixed to mono, a semitone up",
            Box::new(|p: &mut Pad| {
                p.layers[0].region.channels = ChannelPick::MonoSum;
                p.tune_cents = 100;
            }),
        ),
        ("a fifth up", Box::new(|p: &mut Pad| p.tune_cents = 700)),
        (
            "an octave and a half up",
            Box::new(|p: &mut Pad| p.tune_cents = 1_900),
        ),
        (
            "Vintage, an octave up",
            Box::new(|p: &mut Pad| {
                p.vintage = true;
                p.tune_cents = 1_200;
            }),
        ),
        (
            "held in a crossfaded loop",
            Box::new(move |p: &mut Pad| {
                let r = &mut p.layers[0].region;
                r.mode = PlayMode::Loop;
                r.sustain_loop = Some(lp(LoopMode::Forward));
            }),
        ),
        (
            "held in a ping-pong loop",
            Box::new(move |p: &mut Pad| {
                let r = &mut p.layers[0].region;
                r.mode = PlayMode::Gate;
                r.sustain_loop = Some(lp(LoopMode::PingPong));
            }),
        ),
        (
            "reversed, faded",
            Box::new(|p: &mut Pad| {
                let r = &mut p.layers[0].region;
                r.reverse = true;
                r.fade_in = Fade {
                    frames: 3_000,
                    curve: FadeCurve::Log,
                };
                r.fade_out = Fade {
                    frames: 5_000,
                    curve: FadeCurve::Exp,
                };
            }),
        ),
        (
            "filtered, with an envelope",
            Box::new(|p: &mut Pad| {
                p.layers[0].region.mode = PlayMode::Gate;
                p.amp = Ahdsr {
                    attack_ms: 5.0,
                    release_ms: 200.0,
                    ..Ahdsr::OPEN
                };
                p.filter = Filter {
                    kind: FilterKind::LowPass,
                    poles: 4,
                    cutoff_hz: 900.0,
                    resonance: 0.7,
                    env_octaves: 2.0,
                    drive_db: 0.0,
                };
            }),
        ),
        (
            "as grains",
            Box::new(|p: &mut Pad| {
                let r = &mut p.layers[0].region;
                r.mode = PlayMode::Granular;
                r.grain.density = 30.0;
                r.grain.size_ms = 60.0;
                r.grain.spray = 0.3;
                r.grain.pitch = 5.0;
            }),
        ),
    ];
    for channels in [2, 1] {
        let data = chord(40_000, channels);
        let s = Source::new("chord", RATE, channels, data.clone()).unwrap();
        assert!(s.data().is_16_bit());
        // It reads as the floats it was made from, and the floats it is
        // compared with are those.
        let floats = s.held_as_floats();
        assert_eq!(bits(&s.data().to_vec()), bits(&data));
        assert_eq!(bits(&floats.data().to_vec()), bits(&data));
        // Copies for the reads an octave up, made here for both.
        s.make_octaves();
        floats.make_octaves();
        for (what, shape) in &shapes {
            let a = play(&kit(&s, shape.as_ref()), 20_000, 44_100);
            let b = play(&kit(&floats, shape.as_ref()), 20_000, 44_100);
            assert!(a.iter().any(|x| x.abs() > 0.01), "{what}: silent");
            let same = a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits());
            assert!(same, "{what} ({channels} channels): not the same sound");
        }
    }
}
