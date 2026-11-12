//! Pad links (a hit on one pad also plays others, as the MPC's
//! Simultaneous Play) and Latch (a pad plays on after it is lifted, until
//! its next hit).

use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{Engine, EngineView, Event, KILL_FRAMES};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

fn dc(frames: usize) -> Source {
    Source::new("dc", RATE, 1, vec![0.5; frames]).unwrap()
}

/// A pad whose one layer plays all of `s` in `mode`, releasing over 10 ms.
fn pad(s: &Source, mode: PlayMode) -> Pad {
    let mut r = Region::whole(s);
    r.mode = mode;
    let mut p = Pad::new("p", Layer::new(r));
    p.amp = Ahdsr {
        attack_ms: 0.0,
        hold_ms: 0.0,
        decay_ms: 0.0,
        sustain: 1.0,
        release_ms: 10.0,
    };
    p
}

fn engine(pads: &[(usize, Pad)], s: &Source) -> Engine {
    let mut p = Project::new();
    p.add_source(s.clone());
    for (i, pad) in pads {
        p.set_pad(*i, Some(pad.clone()), None).unwrap();
    }
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(&p, RATE));
    e
}

fn run(e: &mut Engine, frames: usize) -> Vec<f32> {
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    l
}

fn hit(e: &mut Engine, pad: u8) {
    e.apply(Event::NoteOn { pad, velocity: 100 });
}

fn lift(e: &mut Engine, pad: u8) {
    e.apply(Event::NoteOff { pad });
}

/// Voices sounding on pads 0 to `n` - 1.
fn voices(e: &Engine, n: u8) -> Vec<usize> {
    (0..n).map(|p| e.pad_voices(p)).collect()
}

/// Long enough for a 10 ms release (441 frames) or a choke to end.
const RELEASED: usize = 1024;

#[test]
fn a_hit_plays_its_links_and_lifting_it_lets_them_go() {
    let s = dc(200_000);
    let mut a = pad(&s, PlayMode::Gate);
    a.links = vec![1, 2];
    let held = pad(&s, PlayMode::Gate);
    let shot = pad(&s, PlayMode::OneShot);
    let mut e = engine(&[(0, a), (1, held), (2, shot)], &s);
    hit(&mut e, 0);
    let l = run(&mut e, 256);
    assert_eq!(voices(&e, 3), [1, 1, 1], "the pad and both links");
    let one = {
        let mut solo = engine(&[(0, pad(&s, PlayMode::Gate))], &s);
        hit(&mut solo, 0);
        run(&mut solo, 256)
    };
    assert!(
        (l[200] - 3.0 * one[200]).abs() < 1e-5,
        "three sounds at the same velocity: {} vs 3 × {}",
        l[200],
        one[200]
    );
    lift(&mut e, 0);
    run(&mut e, RELEASED);
    assert_eq!(
        voices(&e, 3),
        [0, 0, 1],
        "the gate pads let go with the pad hit; the one-shot plays on"
    );
}

#[test]
fn a_linked_pad_hit_itself_is_its_own() {
    let s = dc(200_000);
    let mut a = pad(&s, PlayMode::Gate);
    a.links = vec![1];
    let b = pad(&s, PlayMode::Gate);
    let mut e = engine(&[(0, a), (1, b)], &s);
    hit(&mut e, 1);
    hit(&mut e, 0);
    run(&mut e, 256);
    assert_eq!(voices(&e, 2), [1, 2]);
    lift(&mut e, 0);
    run(&mut e, RELEASED);
    assert_eq!(voices(&e, 2), [0, 1], "pad 1's own hit is still held");
    lift(&mut e, 1);
    run(&mut e, RELEASED);
    assert_eq!(voices(&e, 2), [0, 0]);
}

#[test]
fn links_go_one_way_and_one_step() {
    let s = dc(200_000);
    let mut a = pad(&s, PlayMode::Gate);
    a.links = vec![1];
    let mut b = pad(&s, PlayMode::Gate);
    b.links = vec![2, 0];
    let c = pad(&s, PlayMode::Gate);
    let mut e = engine(&[(0, a), (1, b), (2, c)], &s);
    hit(&mut e, 0);
    run(&mut e, 128);
    assert_eq!(
        voices(&e, 3),
        [1, 1, 0],
        "pad 1's own links are not followed"
    );
    lift(&mut e, 0);
    run(&mut e, RELEASED);
    hit(&mut e, 1);
    run(&mut e, 128);
    assert_eq!(
        voices(&e, 3),
        [1, 1, 1],
        "pad 1 plays its links, pad 0 among them"
    );
}

#[test]
fn a_link_to_itself_or_past_the_pads() {
    let s = dc(200_000);
    let mut a = pad(&s, PlayMode::Gate);
    a.links = vec![0];
    let mut e = engine(&[(0, a)], &s);
    hit(&mut e, 0);
    run(&mut e, 128);
    assert_eq!(e.pad_voices(0), 1, "a link to itself plays nothing more");

    let mut p = Project::new();
    p.add_source(s.clone());
    let mut far = pad(&s, PlayMode::Gate);
    far.links = vec![PADS];
    assert_eq!(p.set_pad(0, Some(far), None), Err(ModelError::BadPad));
}

#[test]
fn a_hit_and_its_links_dont_choke_each_other() {
    let s = dc(200_000);
    let mut a = pad(&s, PlayMode::Gate);
    a.links = vec![1];
    a.chokes = vec![1];
    a.mute_group = Some(3);
    let mut b = pad(&s, PlayMode::Gate);
    b.mute_group = Some(3);
    let mut e = engine(&[(0, a), (1, b)], &s);
    hit(&mut e, 0);
    run(&mut e, 256);
    assert_eq!(voices(&e, 2), [1, 1], "the pad and its link play together");
    hit(&mut e, 0);
    run(&mut e, usize::from(KILL_FRAMES) + 128);
    assert_eq!(
        voices(&e, 2),
        [1, 1],
        "the next hit chokes the last hit's sounds and plays its own"
    );
}

#[test]
fn with_the_pads_playing_notes_links_dont_apply() {
    let s = dc(200_000);
    let mut a = pad(&s, PlayMode::Gate);
    a.links = vec![1];
    a.keys = Some(Keys {
        lo: 0,
        hi: 127,
        root: 60,
    });
    let b = pad(&s, PlayMode::Gate);
    let mut p = Project::new();
    p.add_source(s.clone());
    p.set_pad(0, Some(a), None).unwrap();
    p.set_pad(1, Some(b), None).unwrap();
    p.set_pads_play(PadsPlay::Keys { from: 60 }).unwrap();
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(&p, RATE));
    hit(&mut e, 0);
    run(&mut e, 128);
    assert_eq!(
        voices(&e, 2),
        [1, 0],
        "pad 1 of the pads is note 60, the keygroup"
    );
}

#[test]
fn latch_holds_a_pad_until_its_next_hit() {
    let s = dc(200_000);
    let mut gate = pad(&s, PlayMode::Gate);
    gate.latch = true;
    let mut e = engine(&[(0, gate)], &s);
    hit(&mut e, 0);
    lift(&mut e, 0);
    let l = run(&mut e, 4 * RELEASED);
    assert!(l[4 * RELEASED - 1] > 0.1, "it plays on after the lift");
    assert_eq!(e.pad_voices(0), 1);
    hit(&mut e, 0);
    run(&mut e, RELEASED);
    assert_eq!(
        e.pad_voices(0),
        0,
        "the next hit lets it go, starting nothing"
    );
    lift(&mut e, 0);
    hit(&mut e, 0);
    run(&mut e, 128);
    assert_eq!(e.pad_voices(0), 1, "and the one after starts it again");
}

#[test]
fn a_latched_loop_plays_its_tail_after_the_next_hit() {
    let frames = 4000;
    let s = dc(frames);
    let mut r = Region::whole(&s);
    r.mode = PlayMode::Loop;
    r.sustain_loop = Some(Loop {
        start: 1000,
        end: 2000,
        crossfade: 0,
        mode: LoopMode::Forward,
    });
    let mut looped = Pad::new("loop", Layer::new(r));
    looped.latch = true;
    let mut e = engine(&[(0, looped)], &s);
    hit(&mut e, 0);
    lift(&mut e, 0);
    run(&mut e, 3 * frames);
    assert_eq!(e.pad_voices(0), 1, "still looping, long after the lift");
    hit(&mut e, 0);
    run(&mut e, frames);
    assert_eq!(
        e.pad_voices(0),
        0,
        "out of the loop and to the region's end"
    );
}

#[test]
fn a_latched_grain_cloud_runs_until_the_next_hit() {
    let s = dc(200_000);
    let mut r = Region::whole(&s);
    r.mode = PlayMode::Granular;
    r.grain = GrainParams::default();
    let mut cloud = Pad::new("cloud", Layer::new(r));
    cloud.latch = true;
    cloud.amp.release_ms = 10.0;
    let mut e = engine(&[(0, cloud)], &s);
    hit(&mut e, 0);
    lift(&mut e, 0);
    run(&mut e, RATE as usize);
    assert!(
        e.live_grains() > 0,
        "grains still come a second after the lift"
    );
    hit(&mut e, 0);
    run(&mut e, RATE as usize);
    assert_eq!(e.pad_voices(0), 0);
    assert_eq!(e.live_grains(), 0);
}

#[test]
fn a_latched_one_shot_stops_on_the_next_hit() {
    let s = dc(200_000);
    let mut shot = pad(&s, PlayMode::OneShot);
    shot.latch = true;
    let mut e = engine(&[(0, shot)], &s);
    hit(&mut e, 0);
    lift(&mut e, 0);
    run(&mut e, 256);
    hit(&mut e, 0);
    run(&mut e, RELEASED);
    assert_eq!(e.pad_voices(0), 0);
}

#[test]
fn latched_keys_wait_for_the_same_note_again() {
    let s = dc(200_000);
    let mut k = pad(&s, PlayMode::Gate);
    k.latch = true;
    k.keys = Some(Keys {
        lo: 0,
        hi: 127,
        root: 60,
    });
    let mut e = engine(&[(0, k)], &s);
    e.apply(Event::Key {
        note: 60,
        velocity: 100,
    });
    e.apply(Event::Key {
        note: 64,
        velocity: 100,
    });
    e.apply(Event::KeyOff { note: 60 });
    e.apply(Event::KeyOff { note: 64 });
    run(&mut e, RELEASED);
    assert_eq!(e.pad_voices(0), 2, "both notes play on");
    e.apply(Event::Key {
        note: 64,
        velocity: 100,
    });
    run(&mut e, RELEASED);
    assert_eq!(e.pad_voices(0), 1, "64 again lets 64 go; 60 plays on");
}

#[test]
fn all_off_lets_latched_pads_go() {
    let s = dc(200_000);
    let mut gate = pad(&s, PlayMode::Gate);
    gate.latch = true;
    let mut e = engine(&[(0, gate)], &s);
    hit(&mut e, 0);
    lift(&mut e, 0);
    run(&mut e, 256);
    e.apply(Event::AllOff);
    run(&mut e, RELEASED);
    assert_eq!(e.pad_voices(0), 0);
}

#[test]
fn a_latched_link_plays_on_and_its_pad_hit_again_lets_it_go() {
    let s = dc(200_000);
    let mut a = pad(&s, PlayMode::Gate);
    a.links = vec![1];
    let mut b = pad(&s, PlayMode::Gate);
    b.latch = true;
    let mut e = engine(&[(0, a), (1, b)], &s);
    hit(&mut e, 0);
    lift(&mut e, 0);
    run(&mut e, RELEASED);
    assert_eq!(voices(&e, 2), [0, 1], "the latched link plays on");
    hit(&mut e, 0);
    run(&mut e, RELEASED);
    assert_eq!(
        voices(&e, 2),
        [1, 0],
        "the next hit plays pad 0 and lets the latched link go"
    );
}
