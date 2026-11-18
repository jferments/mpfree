//! Nothing in a voice decays through the denormals. A filter left ringing
//! on silence (a sound with a quiet tail, a held pad past its last note)
//! reaches exact zeros instead: before, its state crept toward zero
//! through numbers under 1.2e-38, which the CPU handles many times more
//! slowly (16 filtered pads ringing out cost about 40 times as much on
//! the workstation).

use mpfree_sampler::dsp::{Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

/// A tenth of a second of stereo noise, then `secs` seconds of silence.
fn noise_then_silence(secs: f32) -> Source {
    let loud = RATE as usize / 10;
    let quiet = (secs * RATE as f32) as usize;
    let mut x = 9u32;
    let data = (0..(loud + quiet) * 2)
        .map(|i| {
            if i < loud * 2 {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            } else {
                0.0
            }
        })
        .collect();
    Source::new("tail", RATE, 2, data).unwrap()
}

fn subnormal(x: f32) -> bool {
    x != 0.0 && x.abs() < f32::MIN_POSITIVE
}

#[test]
fn a_filter_ringing_out_on_silence_reaches_exact_zeros() {
    let s = noise_then_silence(4.9);
    let mut checked = 0;
    for kind in [
        FilterKind::LowPass,
        FilterKind::HighPass,
        FilterKind::BandPass,
    ] {
        for poles in [2, 4] {
            for cutoff_hz in [40.0, 1000.0, 18_000.0] {
                for resonance in [0.0, 0.95] {
                    let mut p = Project::new();
                    p.add_source(s.clone());
                    let mut r = Region::whole(&s);
                    r.mode = PlayMode::Gate;
                    let mut pad = Pad::new("tail", Layer::new(r));
                    pad.filter = Filter {
                        kind,
                        poles,
                        cutoff_hz,
                        resonance,
                        env_octaves: 0.0,
                        drive_db: 0.0,
                    };
                    p.set_pad(0, Some(pad), None).unwrap();
                    let mut e = Engine::new(RATE, 4);
                    e.set_view(EngineView::build(&p, RATE));
                    e.apply(Event::NoteOn {
                        pad: 0,
                        velocity: 100,
                    });
                    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
                    let mut out = Vec::new();
                    for _ in 0..(5 * RATE as usize) / 128 - 1 {
                        e.render(&mut l, &mut r);
                        out.extend_from_slice(&l);
                        out.extend_from_slice(&r);
                    }
                    let what =
                        format!("{kind:?} {poles}-pole {cutoff_hz} Hz, resonance {resonance}");
                    assert!(e.active_voices() == 1, "{what}: still held");
                    assert!(
                        !out.iter().any(|&x| subnormal(x)),
                        "{what}: a denormal in the output"
                    );
                    // The last half second is silence, exactly.
                    let tail = &out[out.len() - RATE as usize..];
                    assert!(tail.iter().all(|&x| x == 0.0), "{what}: not yet silent");
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 36);
}
