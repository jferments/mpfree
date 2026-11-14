//! Load shedding on the sampler engine (`Engine::cull_candidate`,
//! `Engine::cull`): the least audible voice by what it plays goes first,
//! with the steal's fade, and is counted.

use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

fn dc(name: &str, level: f32) -> Source {
    Source::new(name, RATE, 1, vec![level; 200_000]).unwrap()
}

fn gate(s: &Source, release_ms: f32) -> Pad {
    let mut r = Region::whole(s);
    r.mode = PlayMode::Gate;
    let mut pad = Pad::new(&s.name, Layer::new(r));
    pad.amp = Ahdsr {
        attack_ms: 0.0,
        hold_ms: 0.0,
        decay_ms: 0.0,
        sustain: 1.0,
        release_ms,
    };
    pad
}

fn engine(pads: Vec<Pad>, sources: Vec<Source>) -> Engine {
    let mut p = Project::new();
    for s in sources {
        p.add_source(s);
    }
    for (i, pad) in pads.into_iter().enumerate() {
        p.set_pad(i, Some(pad), None).unwrap();
    }
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&p, RATE));
    e
}

fn hit(e: &mut Engine, pad: u8) {
    e.apply(Event::NoteOn { pad, velocity: 127 });
}

fn run(e: &mut Engine, frames: usize) -> Vec<f32> {
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    l
}

#[test]
fn the_quietest_sound_goes_first_with_a_fade() {
    let (loud, quiet) = (dc("loud", 0.5), dc("quiet", 0.02));
    let pads = vec![gate(&loud, 500.0), gate(&quiet, 500.0)];
    let mut e = engine(pads, vec![loud, quiet]);
    hit(&mut e, 0);
    hit(&mut e, 1);
    // A sample's own level counts, not only its gain: the quiet sample
    // reads quiet once it has played a little.
    run(&mut e, 2048);
    assert_eq!(e.active_voices(), 2);
    let c = e.cull_candidate().unwrap();
    assert!(!c.releasing);
    assert!(c.level < 0.05, "{c:?}");
    let before = run(&mut e, 128)[127];
    assert!(e.cull());
    assert_eq!(e.stats().culls, 1);
    let after = run(&mut e, 1024);
    // Both were in: 0.52 x the centre gain; now only the loud one, after
    // the fade, with no step at its start.
    assert!(
        (after[0] - before).abs() < 0.002,
        "{before} -> {}",
        after[0]
    );
    let c = std::f32::consts::FRAC_1_SQRT_2;
    assert!((after[1023] - 0.5 * c).abs() < 1e-4, "{}", after[1023]);
    assert_eq!(e.active_voices(), 1);
    // The loud one is what is left to give up.
    assert!(e.cull_candidate().unwrap().level > 0.3);
}

#[test]
fn a_released_sound_counts_quieter_and_none_is_offered_twice() {
    let (a, b) = (dc("a", 0.5), dc("b", 0.3));
    let pads = vec![gate(&a, 2000.0), gate(&b, 2000.0)];
    let mut e = engine(pads, vec![a, b]);
    hit(&mut e, 0);
    hit(&mut e, 1);
    run(&mut e, 2048);
    // The louder one let go: still louder, but releasing, so it goes first.
    e.apply(Event::NoteOff { pad: 0 });
    run(&mut e, 512);
    let c = e.cull_candidate().unwrap();
    assert!(c.releasing, "{c:?}");
    assert!(c.level > 0.3, "{c:?}");
    assert!(e.cull());
    // Fading: not offered again; the held one is next.
    let next = e.cull_candidate().unwrap();
    assert!(!next.releasing);
    assert!(e.cull());
    assert_eq!(e.cull_candidate(), None);
    assert!(!e.cull());
    run(&mut e, 512);
    assert_eq!(e.active_voices(), 0);
    assert_eq!(e.stats().culls, 2);
}
