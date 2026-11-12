//! Live chop in the engine: while a pad's audition plays, a mark notes
//! where it is in its source, to the frame.

use mpfree_sampler::engine::{Engine, EngineView, Event, KILL_FRAMES, MAX_MARKS};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

fn engine(tune_cents: i32) -> Engine {
    let s = Source::new(
        "ramp",
        RATE,
        1,
        (0..20_000).map(|i| i as f32 * 1e-5).collect(),
    )
    .unwrap();
    let mut pad = Pad::new("loop", Layer::new(Region::whole(&s)));
    pad.tune_cents = tune_cents;
    let mut p = Project::new();
    p.add_source(s);
    p.set_pad(0, Some(pad), None).unwrap();
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&p, RATE));
    e
}

fn run(e: &mut Engine, frames: usize) {
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
}

fn start(e: &mut Engine) {
    e.apply(Event::Audition {
        pad: 0,
        layer: 0,
        start: 0,
        end: 20_000,
        velocity: 100,
    });
    e.apply(Event::LiveChop { pad: Some(0) });
}

#[test]
fn a_mark_is_where_the_audition_is_to_the_frame() {
    let mut e = engine(0);
    assert_eq!(e.marking(), None);
    start(&mut e);
    assert_eq!(e.marking(), Some(0));
    run(&mut e, 1024);
    assert!(e.schedule(37, Event::Mark));
    run(&mut e, 128);
    assert_eq!((e.marks_made(), e.mark(0)), (1, Some(1024 + 37)));
    // An octave up, the source goes by twice as fast.
    let mut e = engine(1200);
    start(&mut e);
    run(&mut e, 1024);
    e.schedule(37, Event::Mark);
    run(&mut e, 128);
    assert_eq!(e.mark(0), Some(2 * (1024 + 37)));
}

#[test]
fn marking_ends_with_the_audition_off_or_a_new_audition() {
    // The sound's end.
    let mut e = engine(0);
    start(&mut e);
    run(&mut e, 20_480);
    assert_eq!(e.marking(), None);
    e.apply(Event::Mark);
    assert_eq!(e.marks_made(), 0, "no sound, no cut");
    // Turned off: the audition stops too.
    let mut e = engine(0);
    start(&mut e);
    run(&mut e, 256);
    e.apply(Event::LiveChop { pad: None });
    run(&mut e, usize::from(KILL_FRAMES) + 128);
    assert_eq!((e.marking(), e.active_voices()), (None, 0));
    // A tap on a slice is an audition of its own: live chop is over.
    let mut e = engine(0);
    start(&mut e);
    e.apply(Event::Audition {
        pad: 0,
        layer: 0,
        start: 100,
        end: 5000,
        velocity: 100,
    });
    run(&mut e, 128);
    assert_eq!(e.marking(), None);
    assert_eq!(e.pad_voices(0), 1);
}

#[test]
fn the_last_marks_are_kept() {
    let mut e = engine(0);
    start(&mut e);
    for _ in 0..MAX_MARKS + 6 {
        e.schedule(0, Event::Mark);
        run(&mut e, 128);
    }
    let n = e.marks_made();
    assert_eq!(n, MAX_MARKS as u64 + 6);
    assert_eq!(e.mark(5), None, "pushed out");
    assert_eq!(e.mark(6), Some(6 * 128));
    assert_eq!(e.mark(n - 1), Some((n - 1) * 128));
    assert_eq!(e.mark(n), None);
}
