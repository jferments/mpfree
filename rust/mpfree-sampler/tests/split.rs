//! `Engine::split`: half the voices render on a lane (another core) while
//! the audio thread renders the other half. The output is the same with
//! the lane on its own thread or run in place, and the same as unsplit but
//! for rounding; chokes, mute groups, polyphony and stealing still see
//! every voice; a lane that misses its deadline drops its part and loses
//! no event.

use std::time::{Duration, Instant};

use mpfree_lanes::Config;
use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event, PadOut};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
const PAIRS: usize = 16;
const BLOCK: usize = 128;

/// Sixteen pad channels, as the mixer's; pad 5 has none (it plays unheard).
struct Channels {
    left: Vec<Vec<f32>>,
    right: Vec<Vec<f32>>,
}

impl Channels {
    fn new(block: usize) -> Channels {
        Channels {
            left: vec![vec![0.0; block]; PAIRS],
            right: vec![vec![0.0; block]; PAIRS],
        }
    }

    fn clear(&mut self) {
        for c in self.left.iter_mut().chain(self.right.iter_mut()) {
            c.fill(0.0);
        }
    }

    fn take(&self) -> Vec<f32> {
        self.left
            .iter()
            .zip(&self.right)
            .flat_map(|(l, r)| l.iter().chain(r))
            .copied()
            .collect()
    }
}

impl PadOut for Channels {
    fn pad(&mut self, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
        if pad >= PAIRS || pad == 5 {
            return None;
        }
        Some((&mut self.left[pad][..], &mut self.right[pad][..]))
    }
}

fn noise(frames: usize, seed: u32) -> Source {
    let mut x = seed;
    let data = (0..frames * 2).map(|_| {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    });
    Source::new("noise", RATE, 2, data.collect()).unwrap()
}

/// Pads 0 to 7: a one-shot, a filtered loop, a reversed gate pad, a
/// granular pad, a pad of two voices at most, an unheard pad, and two
/// pads in one mute group.
fn kit() -> Project {
    let s = noise(30_000, 7);
    let mut p = Project::new();
    p.add_source(s.clone());
    let one = Region::whole(&s);
    let mut lp = Region::whole(&s);
    lp.mode = PlayMode::Loop;
    lp.sustain_loop = Some(Loop {
        start: 2000,
        end: 6000,
        crossfade: 300,
        mode: LoopMode::Forward,
    });
    let mut gate = Region::whole(&s);
    gate.mode = PlayMode::Gate;
    gate.reverse = true;
    gate.tune_cents = 700;
    let mut grains = Region::whole(&s);
    grains.mode = PlayMode::Granular;
    grains.grain = GrainParams {
        density: 200.0,
        size_ms: 40.0,
        spray: 0.4,
        ..GrainParams::default()
    };
    p.set_pad(0, Some(Pad::new("one", Layer::new(one.clone()))), None)
        .unwrap();
    let mut pad = Pad::new("loop", Layer::new(lp));
    pad.filter = Filter {
        kind: FilterKind::LowPass,
        poles: 4,
        cutoff_hz: 1500.0,
        resonance: 0.4,
        env_octaves: 2.0,
    };
    p.set_pad(1, Some(pad), None).unwrap();
    let mut pad = Pad::new("gate", Layer::new(gate));
    pad.amp = Ahdsr {
        attack_ms: 3.0,
        hold_ms: 0.0,
        decay_ms: 40.0,
        sustain: 0.5,
        release_ms: 20.0,
    };
    p.set_pad(2, Some(pad), None).unwrap();
    p.set_pad(3, Some(Pad::new("grains", Layer::new(grains))), None)
        .unwrap();
    let mut pad = Pad::new("two", Layer::new(one.clone()));
    pad.polyphony = 2;
    p.set_pad(4, Some(pad), None).unwrap();
    p.set_pad(5, Some(Pad::new("unheard", Layer::new(one.clone()))), None)
        .unwrap();
    for k in [6, 7] {
        let mut pad = Pad::new("group", Layer::new(one.clone()));
        pad.mute_group = Some(1);
        p.set_pad(k, Some(pad), None).unwrap();
    }
    p
}

/// A deadline the lane always makes, so a busy test machine can't make it
/// late (the plain renders wait no longer than the block lasts).
fn far() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn engine(p: &Project, voices: usize) -> Engine {
    let mut e = Engine::new(RATE, voices);
    e.set_view(EngineView::build(p, RATE));
    e
}

/// How long a test renders parts with a deadline already gone before it
/// gives up waiting for late ones. A gone deadline doesn't make a part
/// late: `Lane::join` takes the lane's half if the lane finished it while
/// the audio thread rendered its own, and a quick lane did that for 50
/// parts running in about 1 run in 25 (and on CI). So the tests go on
/// until the parts they need came back late: 50 to about 130 parts of
/// 128 frames on a 4-core desktop, a fraction of a second.
const GIVE_UP: Duration = Duration::from_secs(20);

fn split(mut e: Engine, threaded: bool) -> Engine {
    let setup = e
        .split(Config {
            name: "test-lane".into(),
            threaded,
            ..Config::default()
        })
        .unwrap();
    assert_eq!(setup.threaded.is_ok(), threaded, "{setup:?}");
    e
}

/// A busy performance: hits on every pad on frames inside the blocks,
/// lifts, pressure, a tape stop and start, more hits than voices.
fn events(block: usize) -> Vec<(u32, Event)> {
    let on = |pad, velocity| Event::NoteOn { pad, velocity };
    let mut v = Vec::new();
    if block.is_multiple_of(3) {
        v.push((((block * 37) % BLOCK) as u32, on((block % 8) as u8, 100)));
    }
    if block % 5 == 1 {
        v.push((
            ((block * 11) % BLOCK) as u32,
            on(((block + 3) % 8) as u8, 70),
        ));
        v.push((((block * 11) % BLOCK) as u32 + 1, on(4, 90)));
    }
    if block % 7 == 2 {
        v.push((((block * 5) % BLOCK) as u32, Event::NoteOff { pad: 2 }));
        v.push((3, Event::NoteOff { pad: 1 }));
    }
    if block.is_multiple_of(4) {
        v.push((
            9,
            Event::Pressure {
                pad: 3,
                value: (block * 9 % 128) as u8,
            },
        ));
    }
    match block {
        40 => v.push((
            50,
            Event::Tape {
                stop: true,
                ms: 200,
            },
        )),
        60 => v.push((
            0,
            Event::Tape {
                stop: false,
                ms: 100,
            },
        )),
        _ => {}
    }
    v
}

/// Every pad pair of every block.
fn perform(e: &mut Engine, blocks: usize) -> Vec<f32> {
    let mut out = Vec::new();
    let mut ch = Channels::new(BLOCK);
    for b in 0..blocks {
        for (o, ev) in events(b) {
            assert!(e.schedule(o, ev));
        }
        ch.clear();
        e.render_routed_until(BLOCK, &mut ch, far());
        out.extend(ch.take());
    }
    out
}

#[test]
fn split_plays_the_same_on_its_own_core_or_in_place_and_as_unsplit() {
    let p = kit();
    let one = perform(&mut engine(&p, 8), 120);
    let mut inline = split(engine(&p, 8), false);
    let here = perform(&mut inline, 120);
    let mut threaded = split(engine(&p, 8), true);
    let there = perform(&mut threaded, 120);
    assert!(one.iter().any(|x| x.abs() > 0.1), "it plays");
    // Bit for bit the same with the lane on its own thread.
    assert!(here == there, "the lane's own core changes nothing");
    // The same as one pool but for the order the voices add up in.
    let worst = one
        .iter()
        .zip(&here)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(worst < 1e-5, "split differs from unsplit by {worst}");
    let st = threaded.stats();
    let lane = threaded.lane_stats().unwrap();
    assert!(st.lane_parts > 50, "{st:?}");
    assert_eq!((st.lane_late, lane.late), (0, 0));
    assert_eq!(lane.posted, st.lane_parts);
    assert_eq!(st.steals, engine_steals(&p), "the same voices are stolen");
    assert!(st.steals > 0, "the test fills the pool");
}

fn engine_steals(p: &Project) -> u64 {
    let mut e = engine(p, 8);
    perform(&mut e, 120);
    e.stats().steals
}

#[test]
fn mixed_render_and_the_master_effect_match_too() {
    let p = kit();
    let run = |e: &mut Engine| {
        let fx = Engine::make_master_fx(RATE);
        e.set_master_fx(fx);
        let mut out = Vec::new();
        for b in 0..80 {
            for (o, ev) in events(b) {
                e.schedule(o, ev);
            }
            let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
            e.render_until(&mut l, &mut r, far());
            out.extend(l.into_iter().chain(r));
        }
        out
    };
    let here = run(&mut split(engine(&p, 8), false));
    let there = run(&mut split(engine(&p, 8), true));
    assert!(here == there);
}

#[test]
fn chokes_and_polyphony_reach_across_the_halves() {
    let p = kit();
    let mut e = split(engine(&p, 8), true);
    let mut ch = Channels::new(BLOCK);
    let mut block = |e: &mut Engine| {
        ch.clear();
        e.render_routed_until(BLOCK, &mut ch, far());
    };
    // Pad 6 takes the first half's first voice; pad 7, of the same mute
    // group, lands in the second half (it plays fewer) and silences pad 6.
    e.schedule(
        0,
        Event::NoteOn {
            pad: 6,
            velocity: 100,
        },
    );
    block(&mut e);
    assert_eq!(e.pad_voices(6), 1);
    e.schedule(
        0,
        Event::NoteOn {
            pad: 7,
            velocity: 100,
        },
    );
    block(&mut e);
    assert_eq!((e.pad_voices(6), e.pad_voices(7)), (0, 1));
    // Three hits on the pad of two voices: its oldest is pushed out,
    // wherever it plays.
    for _ in 0..3 {
        e.schedule(
            0,
            Event::NoteOn {
                pad: 4,
                velocity: 100,
            },
        );
        block(&mut e);
    }
    assert_eq!(e.pad_voices(4), 2);
    assert!(e.stats().lane_parts > 0);
}

#[test]
fn a_late_lane_leaves_its_part_out_and_loses_no_event() {
    let p = kit();
    let mut e = split(engine(&p, 8), true);
    let mut ch = Channels::new(LONG);
    const LONG: usize = 1024;
    // Two loops, one in each half.
    for pad in [1, 3] {
        e.schedule(0, Event::NoteOn { pad, velocity: 100 });
    }
    ch.clear();
    e.render_routed_until(LONG, &mut ch, far());
    assert_eq!(e.active_voices(), 2);
    // A deadline already gone: the audio thread doesn't wait. That doesn't
    // make every part late (see `GIVE_UP`), so the parts go on until
    // one is.
    let late = Instant::now() - Duration::from_millis(1);
    let give_up = Instant::now() + GIVE_UP;
    let mut lates = 0;
    while lates == 0 && Instant::now() < give_up {
        ch.clear();
        e.render_routed_until(LONG, &mut ch, late);
        lates = e.stats().lane_late;
    }
    assert!(lates > 0, "the lane never missed a deadline");
    // Lifts while the lane's half is away wait in the queue, then apply.
    e.schedule(0, Event::NoteOff { pad: 1 });
    e.schedule(0, Event::NoteOff { pad: 3 });
    std::thread::sleep(Duration::from_millis(50));
    for _ in 0..200 {
        ch.clear();
        e.render_routed(LONG, &mut ch);
    }
    assert_eq!(e.active_voices(), 0, "both let go and ended");
}

#[test]
fn skipped_grains_never_go_back_while_the_lanes_half_is_away() {
    // A dense granular pad: eight voices of it ask for more grains than
    // the budget holds, in both halves.
    let s = noise(60_000, 3);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut grains = Region::whole(&s);
    grains.mode = PlayMode::Granular;
    grains.grain = GrainParams {
        density: 400.0,
        size_ms: 200.0,
        ..GrainParams::default()
    };
    p.set_pad(0, Some(Pad::new("dense", Layer::new(grains))), None)
        .unwrap();
    let mut e = split(engine(&p, 8), true);
    let mut ch = Channels::new(128);
    for _ in 0..8 {
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 100,
        });
    }
    for _ in 0..100 {
        ch.clear();
        e.render_routed_until(128, &mut ch, far());
    }
    let mut last = e.stats().skipped_grains;
    assert!(last > 0, "the budget filled");
    // Parts the lane is late for: its half stays away until the next part,
    // and its skipped grains still count as they were when it left. At
    // least 50 parts, and on until a few were late (see `GIVE_UP`).
    let late = Instant::now() - Duration::from_millis(1);
    let give_up = Instant::now() + GIVE_UP;
    let (mut parts, mut late_parts) = (0u32, 0u64);
    while (parts < 50 || late_parts < 3) && Instant::now() < give_up {
        ch.clear();
        let was = e.stats().lane_late;
        e.render_routed_until(128, &mut ch, late);
        let st = e.stats();
        assert!(
            st.skipped_grains >= last,
            "skipped grains went back from {last} to {}",
            st.skipped_grains
        );
        last = st.skipped_grains;
        late_parts += st.lane_late - was;
        parts += 1;
    }
    assert!(
        late_parts >= 3,
        "the lane missed a deadline {late_parts} times in {parts} parts"
    );
}
