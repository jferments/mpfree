//! Split (`Engine::split`), the engine still never allocates or frees while
//! it renders: not on the audio thread, and not on the lane's worker,
//! which renders the second half of the voices. A busy session: loops,
//! layers, chokes, mute groups, stealing, granular pads filling the grain
//! budget, the master effect, tempo moves, tape stops, views swapped, mixed
//! and routed blocks, and parts the lane is late for.
//!
//! This file installs mpfree-prof's counting allocator (per thread), which
//! is also what the lane reads to count its worker's allocations.

use std::time::{Duration, Instant};

use mpfree_lanes::Config;
use mpfree_prof::alloc::{self, CountingAlloc};
use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event, PadOut};
use mpfree_sampler::model::*;

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const RATE: u32 = 44_100;

/// Twelve pad channels, made before counting.
struct Channels(Vec<[Vec<f32>; 2]>);

impl PadOut for Channels {
    fn pad(&mut self, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
        let [l, r] = self.0.get_mut(pad)?;
        Some((&mut l[..], &mut r[..]))
    }
}

fn kit(s: &Source) -> Project {
    let mut p = Project::new();
    p.add_source(s.clone());
    for i in 0..16usize {
        let mut r = Region::whole(s);
        r.mode = [
            PlayMode::OneShot,
            PlayMode::Gate,
            PlayMode::Loop,
            PlayMode::Toggle,
        ][i % 4];
        r.sustain_loop = Some(Loop {
            start: 5000,
            end: 9000,
            crossfade: 200,
            mode: if i % 3 == 0 {
                LoopMode::PingPong
            } else {
                LoopMode::Forward
            },
        });
        r.reverse = i % 5 == 4;
        r.tune_cents = (i as i32 - 8) * 50;
        r.warp = (i % 4 == 1).then(|| Warp {
            bpm: 100.0,
            method: WarpMethod::Repitch,
            pins: Vec::new(),
        });
        let mut pad = Pad::new("p", Layer::new(r.clone()));
        pad.layers.push(Layer::new(r));
        pad.select = [LayerSelect::All, LayerSelect::Cycle, LayerSelect::Random][i % 3];
        pad.polyphony = 2;
        pad.mute_group = (i % 4 == 0).then_some(1);
        pad.chokes = vec![(i + 1) % 16];
        pad.wow = if i % 2 == 1 { 0.8 } else { 0.0 };
        pad.filter = Filter {
            kind: FilterKind::LowPass,
            poles: 4,
            cutoff_hz: 1000.0,
            resonance: 0.5,
            env_octaves: 3.0,
        };
        pad.amp = Ahdsr {
            attack_ms: 2.0,
            hold_ms: 1.0,
            decay_ms: 50.0,
            sustain: 0.6,
            release_ms: 30.0,
        };
        p.set_pad(i, Some(pad), None).unwrap();
    }
    for i in 16..20usize {
        let mut r = Region::whole(s);
        r.mode = PlayMode::Granular;
        r.reverse = i == 17;
        r.grain = GrainParams {
            spray: 0.6,
            size_ms: 300.0,
            density: 1000.0,
            pitch_spray: 5.0,
            spread: 1.0,
            scan: 1.0,
            ..GrainParams::default()
        };
        let mut pad = Pad::new("g", Layer::new(r));
        pad.polyphony = 2;
        pad.pressure = PressureRoute {
            target: PressureTarget::GrainDensity,
            depth: 1.0,
        };
        p.set_pad(i, Some(pad), None).unwrap();
    }
    p
}

#[test]
fn a_split_engine_allocates_nothing_on_either_core() {
    assert!(alloc::installed());
    let s = Source::new(
        "noise",
        RATE,
        2,
        (0..40_000)
            .map(|i| ((i * 7919) % 2000) as f32 / 1000.0 - 1.0)
            .collect(),
    )
    .unwrap();
    let mut p = kit(&s);
    let mut warps = mpfree_sampler::warp::Warps::new(120.0);
    let mut e = Engine::new(RATE, 24);
    e.set_view(EngineView::build_warped(&p, RATE, &mut warps));
    let setup = e
        .split(Config {
            name: "split-test".into(),
            ..Config::default()
        })
        .unwrap();
    assert!(setup.threaded.is_ok(), "{setup:?}");
    assert!(e.set_master_fx(Engine::make_master_fx(RATE)).is_none());
    p.set_master(
        MasterFx {
            mix: 0.7,
            ..MasterFx::default()
        },
        None,
    )
    .unwrap();
    let mut wet = Some(EngineView::build_warped(&p, RATE, &mut warps));
    let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
    let mut ch = Channels(
        (0..12)
            .map(|_| [vec![0.0f32; 128], vec![0.0f32; 128]])
            .collect(),
    );
    // Warm up outside the count: the lane's first wake-ups.
    for pad in 0..4 {
        e.apply(Event::NoteOn { pad, velocity: 90 });
    }
    for _ in 0..20 {
        e.render(&mut l, &mut r);
    }
    let gone = Instant::now() - Duration::from_millis(1);

    let before = alloc::thread_count();
    for block in 0..2000u32 {
        let pad = (block * 7 % 16) as u8;
        e.set_tempo(120.0 + f64::from(block / 100 % 5) * 7.5);
        if block % 200 == 10 {
            wet = Some(e.set_view(wet.take().unwrap()));
        }
        if block % 150 == 40 {
            e.schedule(0, Event::Tape { stop: true, ms: 30 });
        }
        if block % 150 == 70 {
            e.schedule(
                block % 128,
                Event::Tape {
                    stop: false,
                    ms: (block % 3 * 10) as u16,
                },
            );
        }
        e.apply(Event::NoteOn {
            pad,
            velocity: (block % 127 + 1) as u8,
        });
        e.schedule(
            block % 128,
            Event::NoteOn {
                pad: (pad + 3) % 16,
                velocity: 90,
            },
        );
        e.schedule(
            block * 31 % 128,
            Event::NoteOff {
                pad: (pad + 5) % 16,
            },
        );
        if block % 3 == 0 {
            e.schedule(
                block % 100,
                Event::NoteOn {
                    pad: 16 + (block % 4) as u8,
                    velocity: 100,
                },
            );
        }
        if block % 5 == 0 {
            e.schedule(
                block % 90,
                Event::NoteOff {
                    pad: 16 + ((block + 1) % 4) as u8,
                },
            );
        }
        e.schedule(
            block * 17 % 128,
            Event::Pressure {
                pad: 16 + (block % 4) as u8,
                value: (block * 37 % 128) as u8,
            },
        );
        if block % 300 == 299 {
            e.apply(Event::AllOff);
        }
        match block % 4 {
            1 => {
                e.render_routed(128, &mut ch);
                for [cl, cr] in &mut ch.0 {
                    cl.fill(0.0);
                    cr.fill(0.0);
                }
            }
            // Now and then the lane is given no time at all.
            3 if block % 40 == 3 => {
                e.render_routed_until(128, &mut ch, gone);
                for [cl, cr] in &mut ch.0 {
                    cl.fill(0.0);
                    cr.fill(0.0);
                }
            }
            _ => e.render(&mut l, &mut r),
        }
    }
    // Then the granular pads held, two voices each, to fill the grain
    // budget, and every pad hit twice over, more voices than the pool has.
    for pad in 16..20u8 {
        e.apply(Event::NoteOn { pad, velocity: 100 });
        e.apply(Event::NoteOn { pad, velocity: 100 });
    }
    for block in 0..200u32 {
        if block % 50 == 0 {
            for pad in 0..16u8 {
                e.schedule(u32::from(pad), Event::NoteOn { pad, velocity: 80 });
                e.schedule(u32::from(pad) + 40, Event::NoteOn { pad, velocity: 80 });
            }
        }
        if block % 2 == 1 {
            e.render_routed(128, &mut ch);
        } else {
            e.render(&mut l, &mut r);
        }
    }
    e.apply(Event::AllOff);
    for _ in 0..100 {
        e.render(&mut l, &mut r);
    }
    let made = alloc::thread_count() - before;
    let st = e.stats();
    let lane = e.lane_stats().unwrap();
    assert_eq!(made, 0, "the audio thread allocated or freed");
    assert_eq!(
        lane.worker_allocs, 0,
        "the lane's worker allocated: {lane:?}"
    );
    assert!(st.lane_parts > 1000, "{st:?}");
    assert!(
        st.skipped_grains > 0,
        "the grain budget never filled: {st:?}"
    );
    assert!(st.steals > 0, "no voice was stolen: {st:?}");
    assert!(l.iter().chain(&r).all(|x| x.is_finite()));
}
