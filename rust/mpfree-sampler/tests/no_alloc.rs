//! The engine's real-time calls (render, apply, schedule) and the rolling
//! recording buffer's writer must not allocate
//! or free. A counting global allocator checks that across a busy session:
//! loops, layers, chokes, stealing, filters, sample-accurate events,
//! slice auditions,
//! granular pads, pad pressure, and swapping prebuilt views (the pad player
//! switches kits that way).
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code.
//!
//! Only the test's own thread is watched: the test harness's main thread
//! can allocate at any moment (it did on a busy CI runner), and that is
//! not the engine's doing.
//!
//! The master granular effect runs throughout: views move its mix up and
//! back down, and freeze turns on and off on exact frames. Every fourth
//! block renders routed instead, each pad into its own pair (pads 12 and
//! up, the granular ones among them, have none and play unheard).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event, PadField, PadOut, ProgramField};
use mpfree_sampler::model::*;
use mpfree_sampler::record::{bar_lines, rolling};

struct Counting;
thread_local! {
    // A const-initialised Cell needs no allocation or destructor, so it is
    // safe to read from inside the allocator.
    static WATCH: Cell<bool> = const { Cell::new(false) };
}
static CALLS: AtomicUsize = AtomicUsize::new(0);

fn count() {
    if WATCH.with(Cell::get) {
        CALLS.fetch_add(1, Ordering::Relaxed);
    }
}
fn watch(on: bool) {
    WATCH.with(|w| w.set(on));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        count();
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Twelve pad channels, made before the watch.
struct Channels(Vec<[Vec<f32>; 2]>);

impl PadOut for Channels {
    fn pad(&mut self, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
        let [l, r] = self.0.get_mut(pad)?;
        Some((&mut l[..], &mut r[..]))
    }
}

#[test]
fn render_and_events_never_allocate() {
    let rate = 44_100;
    let s = Source::new(
        "noise",
        rate,
        2,
        (0..40_000)
            .map(|i| ((i * 7919) % 2000) as f32 / 1000.0 - 1.0)
            .collect(),
    )
    .unwrap();
    let mut p = Project::new();
    p.add_source(s.clone());
    for i in 0..16usize {
        let mut r = Region::whole(&s);
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
        // Some at the project tempo: repitched, or stretched (no copy is
        // made here, so those repitch too).
        r.warp = match i % 4 {
            1 => Some(Warp {
                bpm: 100.0,
                method: WarpMethod::Repitch,
                pins: Vec::new(),
            }),
            3 => Some(Warp::new(140.0)),
            _ => None,
        };
        let mut pad = Pad::new("p", Layer::new(r.clone()));
        pad.layers.push(Layer::new(r));
        pad.select = [LayerSelect::All, LayerSelect::Cycle, LayerSelect::Random][i % 3];
        pad.polyphony = 2;
        pad.mute_group = (i % 4 == 0).then_some(1);
        // Some are keygroups too, played by notes below.
        pad.keys = (i % 4 == 2).then_some(Keys {
            lo: 40,
            hi: 80,
            root: 60,
        });
        pad.chokes = vec![(i + 1) % 16];
        // Half of them wobble like worn tape.
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
        pad.filter_env = Ahdsr {
            attack_ms: 1.0,
            hold_ms: 0.0,
            decay_ms: 80.0,
            sustain: 0.1,
            release_ms: 30.0,
        };
        let targets = [
            PressureTarget::Off,
            PressureTarget::Level,
            PressureTarget::Pitch,
            PressureTarget::Filter,
        ];
        pad.pressure = PressureRoute {
            target: targets[i % 4],
            depth: if i % 2 == 0 { 0.8 } else { -0.6 },
        };
        p.set_pad(i, Some(pad), None).unwrap();
    }
    // Four granular pads: busy clouds, one reversed, one synced, so the grain
    // budget fills and voices are stolen while grains play.
    for i in 16..20usize {
        let mut r = Region::whole(&s);
        r.mode = PlayMode::Granular;
        r.reverse = i == 17;
        r.tune_cents = (i as i32 - 18) * 700;
        r.grain = GrainParams {
            spray: 0.6,
            size_ms: 300.0,
            density: 1000.0,
            pitch_spray: 5.0,
            spread: 1.0,
            scan: 1.0,
            ..GrainParams::default()
        };
        if i == 19 {
            r.grain.window = GrainWindow::Exponential { decay: 6.0 };
            r.grain.sync = Some(TempoSync {
                frames_per_beat: 22_050.0,
                grains_per_beat: Some(8.0),
                size_beats: Some(0.25),
            });
        }
        let mut pad = Pad::new("g", Layer::new(r));
        pad.polyphony = 2;
        pad.filter = Filter {
            kind: FilterKind::LowPass,
            poles: 2,
            cutoff_hz: 3000.0,
            resonance: 0.2,
            env_octaves: 1.0,
        };
        let targets = [
            PressureTarget::GrainDensity,
            PressureTarget::GrainSize,
            PressureTarget::GrainPosition,
            PressureTarget::GrainSpray,
        ];
        pad.pressure = PressureRoute {
            target: targets[i - 16],
            depth: 1.0,
        };
        p.set_pad(i, Some(pad), None).unwrap();
    }
    let mut e = Engine::new(rate, 24);
    let mut warps = mpfree_sampler::warp::Warps::new(120.0);
    e.set_view(EngineView::build_warped(&p, rate, &mut warps));
    // A second view, swapped in and out while voices play: only the views
    // move, nothing is built or dropped.
    let mut q = Project::new();
    q.add_source(s.clone());
    for i in 0..16usize {
        let mut r = Region::whole(&s);
        r.start = (i as u64) * 1000;
        q.set_pad(i, Some(Pad::new("k", Layer::new(r))), None)
            .unwrap();
    }
    // The spare view plays 16 Levels of cutoff (its pads have no filter,
    // so the levels bring one in).
    q.set_pads_play(PadsPlay::Levels {
        pad: 2,
        by: Levels::Cutoff,
    })
    .unwrap();
    let mut spare = Some(EngineView::build(&q, rate));
    let mut l = vec![0.0f32; 128];
    let mut r = vec![0.0f32; 128];
    let mut ch = Channels(
        (0..12)
            .map(|_| [vec![0.0f32; 128], vec![0.0f32; 128]])
            .collect(),
    );
    e.render(&mut l, &mut r); // warm up outside the watch
                              // The rolling "skip back" buffer the audio thread fills every period.
    let (mut rec, reader) = rolling(2, 44_100 * 2);
    // And the transport's bar lines in its frames, for takes in time.
    let (mut bars, bar_reader) = bar_lines();
    // The master effect is made and put in before the watch; inside it, a
    // second view brings its mix up and back down.
    assert!(e.set_master_fx(Engine::make_master_fx(rate)).is_none());
    p.set_master(
        MasterFx {
            mix: 0.7,
            grain: GrainParams {
                density: 200.0,
                size_ms: 150.0,
                spray: 0.3,
                pitch_spray: 2.0,
                ..MasterFx::default().grain
            },
        },
        None,
    )
    .unwrap();
    // The wet views play 16 Levels of decay and of start.
    p.set_pads_play(PadsPlay::Levels {
        pad: 0,
        by: Levels::Decay,
    })
    .unwrap();
    let mut wet = Some(EngineView::build_warped(&p, rate, &mut warps));
    p.set_pads_play(PadsPlay::Levels {
        pad: 1,
        by: Levels::Start,
    })
    .unwrap();
    let mut wet_start = Some(EngineView::build_warped(&p, rate, &mut warps));
    p.set_pads_play(PadsPlay::Pads).unwrap();
    p.set_master(MasterFx::default(), None).unwrap();
    let dry = EngineView::build_warped(&p, rate, &mut warps);

    // The counter itself works: one Vec is one allocation and one free.
    watch(true);
    drop(std::hint::black_box(vec![0u8; 16]));
    watch(false);
    assert_eq!(CALLS.swap(0, Ordering::SeqCst), 2);

    watch(true);
    for block in 0..2000u32 {
        let pad = (block * 7 % 16) as u8;
        // The transport's tempo, moving now and then.
        e.set_tempo(120.0 + f64::from(block / 100 % 5) * 7.5);
        if block % 6 == 0 {
            let note = 38 + (block / 6 % 46) as u8;
            e.schedule(block % 128, Event::Key { note, velocity: 90 });
            e.schedule(
                (block + 64) % 128,
                Event::KeyOff {
                    note: note.wrapping_sub(12),
                },
            );
        }
        if block % 200 == 10 {
            wet = Some(e.set_view(wet.take().unwrap()));
        }
        if block % 300 == 150 {
            wet_start = Some(e.set_view(wet_start.take().unwrap()));
        }
        if block % 64 == 0 {
            e.schedule(
                block % 128,
                Event::Freeze {
                    on: block % 128 == 0,
                },
            );
        }
        // Tape stops and starts, at once and over a few blocks.
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
            block * 31 % 200,
            Event::NoteOff {
                pad: (pad + 5) % 16,
            },
        );
        let g = 16 + (block % 4) as u8;
        if block % 3 == 0 {
            e.schedule(
                block % 100,
                Event::NoteOn {
                    pad: g,
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
                pad: (block % 20) as u8,
                value: (block * 37 % 128) as u8,
            },
        );
        if block % 7 == 3 {
            e.schedule(
                block % 128,
                Event::Audition {
                    pad: (block % 20) as u8,
                    layer: 0,
                    start: 1000 + block,
                    end: 6000,
                    velocity: 100,
                },
            );
        }
        // Live chop: on for the audition, marks, and off again.
        if block % 7 == 3 {
            e.schedule(
                block % 128,
                Event::LiveChop {
                    pad: Some((block % 20) as u8),
                },
            );
        }
        if block % 3 == 1 {
            e.schedule(block * 29 % 128, Event::Mark);
            let _ = (e.marking(), e.mark(e.marks_made().saturating_sub(1)));
        }
        if block % 70 == 69 {
            e.schedule(0, Event::LiveChop { pad: None });
        }
        // Live settings: knobs turned, p-locks on hits, cleared again, and
        // the program's offsets.
        e.schedule(
            block * 13 % 128,
            Event::PadLive {
                pad: (block % 20) as u8,
                field: PadField::ALL[(block % 8) as usize],
                value: live_value(block),
            },
        );
        if block % 9 == 4 {
            e.schedule(
                block % 128,
                Event::ProgramLive {
                    field: ProgramField::ALL[(block % 3) as usize],
                    value: (block % 7) as f32 - 3.0,
                },
            );
        }
        if block % 11 == 5 {
            e.apply(Event::PadLive {
                pad: (block % 20) as u8,
                field: PadField::ALL[(block / 11 % 8) as usize],
                value: f32::NAN,
            });
        }
        if block % 500 == 377 {
            e.stop_all();
        }
        if block % 300 == 299 {
            e.apply(Event::AllOff);
        }
        if block % 50 == 25 {
            spare = Some(e.set_view(spare.take().unwrap()));
        }
        if block % 4 == 1 {
            e.render_routed(128, &mut ch);
            for [cl, cr] in &mut ch.0 {
                cl.fill(0.0);
                cr.fill(0.0);
            }
        } else {
            e.render(&mut l, &mut r);
        }
        if block % 16 == 0 {
            bars.mark(rec.written());
        }
        bars.set_bar(Some(88_200.0));
        rec.push_planar(&l, &r);
    }
    // Then only the granular pads, held long enough to fill the grain budget.
    for pad in 16..20u8 {
        e.apply(Event::NoteOn { pad, velocity: 100 });
        e.apply(Event::NoteOn { pad, velocity: 100 });
    }
    for block in 0..200u32 {
        for pad in 16..20u8 {
            e.schedule(
                block % 64,
                Event::Pressure {
                    pad,
                    value: (block * 5 % 128) as u8,
                },
            );
        }
        if block % 4 == 1 {
            e.render_routed(128, &mut ch);
        } else {
            e.render(&mut l, &mut r);
        }
        rec.push_planar(&l, &r);
    }
    let spawned_before_dry = e.master_fx().unwrap().stats().spawned;
    let _kept = e.set_view(dry); // dry again, so the master hands its grains back
    e.apply(Event::AllOff);
    for _ in 0..100 {
        e.render(&mut l, &mut r);
    }
    watch(false);
    // Voices that end drop their handle to the audio; the project still holds
    // it, so that is a counter decrement, not a free.
    assert_eq!(
        CALLS.load(Ordering::SeqCst),
        0,
        "allocations or frees while rendering"
    );
    assert!(e.stats().hits > 2000);
    assert!(
        e.stats().skipped_grains > 0,
        "the grain budget never filled; the test is not busy enough"
    );
    assert_eq!(reader.written(), 2200 * 128);
    assert_eq!(bar_reader.count(), 2000 / 16);
    assert_eq!(e.live_grains(), 0, "every grain handed back after AllOff");
    assert!(l.iter().chain(r.iter()).all(|x| x.is_finite()));
    assert!(
        spawned_before_dry > 100,
        "the master effect made grains: {spawned_before_dry}"
    );
}

/// A value for a live setting: in range for some fields, out of range
/// (clamped) for others, the way a careless caller might send it.
fn live_value(block: u32) -> f32 {
    (block * 37 % 2000) as f32 - 500.0
}
