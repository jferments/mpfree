use mpfree_mod::lfo;
use mpfree_mod::*;
use mpfree_node::{Ctx, Curve, NodeEvent, ParamInfo, Timed, Unit};
use mpfree_param::{Owner, ParamAddr, Strip};

const RATE: u32 = 48_000;
const CUT: ParamInfo = ParamInfo {
    id: 3,
    name: "Cutoff",
    group: "Filter",
    min: 20.0,
    max: 20000.0,
    default: 1000.0,
    unit: Unit::Hz,
    curve: Curve::Log,
    readonly: false,
};
const MIX: ParamInfo = ParamInfo {
    id: 5,
    name: "Mix",
    group: "Out",
    min: 0.0,
    max: 100.0,
    default: 50.0,
    unit: Unit::Percent,
    curve: Curve::Linear,
    readonly: false,
};
const MODE: ParamInfo = ParamInfo {
    id: 6,
    name: "Mode",
    group: "Filter",
    min: 0.0,
    max: 2.0,
    default: 0.0,
    unit: Unit::None,
    curve: Curve::Choice(&["LP", "HP", "BP"]),
    readonly: false,
};

fn cut() -> ParamAddr {
    ParamAddr::new(Owner::Insert(Strip::Track(0), 1), 3)
}
fn mix() -> ParamAddr {
    ParamAddr::new(Owner::Insert(Strip::Track(0), 1), 5)
}
fn mode() -> ParamAddr {
    ParamAddr::new(Owner::Insert(Strip::Track(0), 1), 6)
}

/// (absolute frame, address, value) for every event sent.
type Log = Vec<(u64, ParamAddr, f32)>;

/// Runs `frames` frames in blocks of `block`, with `events` at absolute
/// frames, and logs what comes out.
fn run(
    e: &mut ModEngine,
    frames: u64,
    block: usize,
    events: &[(u64, ModEvent)],
    audio: Option<&[f32]>,
) -> Log {
    let mut log = Log::new();
    let mut at = 0u64;
    while at < frames {
        let len = block.min((frames - at) as usize);
        let evs: Vec<ModTimed> = events
            .iter()
            .filter(|(f, _)| *f >= at && *f < at + len as u64)
            .map(|(f, ev)| ModTimed::new((f - at) as u32, *ev))
            .collect();
        let ctx = Ctx {
            rate: RATE,
            frame: at,
            bpm: 120.0,
            beats: at as f64 * 2.0 / RATE as f64,
            playing: true,
            sidechain: None,
        };
        let slice = audio.map(|a| &a[at as usize..at as usize + len]);
        let follow = [slice.map(|s| (s, s)), None, None, None];
        e.process(&ctx, len, &evs, &follow, &mut |addr, t: Timed| {
            let NodeEvent::Param { id, value } = t.ev else {
                panic!("only params")
            };
            assert_eq!(id, addr.param);
            assert!((t.offset as usize) < len);
            log.push((at + t.offset as u64, addr, value));
        });
        at += len as u64;
    }
    log
}

fn last(log: &Log, a: ParamAddr) -> f32 {
    log.iter().rev().find(|x| x.1 == a).map(|x| x.2).unwrap()
}

fn engine() -> ModEngine {
    let mut e = ModEngine::new(RATE, DEFAULT_TICK);
    e.add_target(cut(), CUT, 1000.0).unwrap();
    e.add_target(mix(), MIX, 50.0).unwrap();
    e.add_target(mode(), MODE, 0.0).unwrap();
    e
}

#[test]
fn a_target_is_sent_once_and_then_only_when_it_changes() {
    let mut e = engine();
    let log = run(&mut e, 4800, 128, &[], None);
    assert_eq!(log.iter().filter(|x| x.1 == cut()).count(), 1);
    assert_eq!(last(&log, cut()), 1000.0);
    let log = run(
        &mut e,
        4800,
        128,
        &[(
            100,
            ModEvent::Set {
                addr: cut(),
                value: 2000.0,
            },
        )],
        None,
    );
    assert_eq!(
        log.iter().filter(|x| x.1 == cut()).collect::<Vec<_>>(),
        [&(100, cut(), 2000.0)]
    );
}

#[test]
fn a_macro_moves_its_targets_through_their_ranges() {
    let mut e = engine();
    e.map(
        0,
        MacroMap {
            target: cut(),
            from: 0.2,
            to: 0.8,
            curve: MapCurve::Linear,
        },
    )
    .unwrap();
    e.map(
        0,
        MacroMap {
            target: mix(),
            from: 1.0,
            to: 0.0,
            curve: MapCurve::Exp,
        },
    )
    .unwrap();
    let log = run(
        &mut e,
        256,
        64,
        &[(
            10,
            ModEvent::Set {
                addr: ParamAddr::macro_value(0),
                value: 0.5,
            },
        )],
        None,
    );
    assert!((CUT.to_unit(last(&log, cut())) - 0.5).abs() < 1e-5);
    assert!((last(&log, mix()) - 75.0).abs() < 1e-3, "1 - 0.5² of 100 %");
    assert!(
        log.iter().any(|x| x.0 == 10 && x.1 == cut()),
        "on the set's own frame"
    );
    assert_eq!(e.macro_of(cut()), Some(0));
    // Mapping it to another macro moves it.
    e.map(
        3,
        MacroMap {
            target: cut(),
            from: 0.0,
            to: 1.0,
            curve: MapCurve::Linear,
        },
    )
    .unwrap();
    assert_eq!(e.macro_of(cut()), Some(3));
    assert_eq!(e.macro_maps(0).count(), 1);
    assert!(e.unmap(cut()));
    assert_eq!(
        e.map(
            0,
            MacroMap {
                target: ParamAddr::macro_value(1),
                from: 0.0,
                to: 1.0,
                curve: MapCurve::Linear
            }
        ),
        Err(ModError::Refused)
    );
    // A switch can be on a macro: it jumps.
    e.map(
        1,
        MacroMap {
            target: mode(),
            from: 0.0,
            to: 1.0,
            curve: MapCurve::Linear,
        },
    )
    .unwrap();
    let log = run(
        &mut e,
        64,
        64,
        &[(
            0,
            ModEvent::Set {
                addr: ParamAddr::macro_value(1),
                value: 0.8,
            },
        )],
        None,
    );
    assert_eq!(last(&log, mode()), 2.0);
}

#[test]
fn a_lock_replaces_everything_until_unlocked_and_lands_on_its_frame() {
    let mut e = engine();
    e.map(
        0,
        MacroMap {
            target: mix(),
            from: 0.0,
            to: 1.0,
            curve: MapCurve::Linear,
        },
    )
    .unwrap();
    let log = run(
        &mut e,
        512,
        128,
        &[
            (
                0,
                ModEvent::Set {
                    addr: ParamAddr::macro_value(0),
                    value: 0.3,
                },
            ),
            (
                64,
                ModEvent::Lock {
                    addr: mix(),
                    value: 90.0,
                },
            ),
            (300, ModEvent::Unlock { addr: mix() }),
        ],
        None,
    );
    let mixes: Vec<_> = log
        .iter()
        .filter(|x| x.1 == mix())
        .map(|x| (x.0, x.2))
        .collect();
    assert_eq!(mixes.len(), 3, "{mixes:?}");
    assert_eq!(
        mixes[1],
        (64, 90.0),
        "the lock on a tick frame goes out on that frame"
    );
    assert_eq!(mixes[2].0, 300);
    assert!(
        (mixes[2].1 - 30.0).abs() < 1e-3,
        "back to the macro's value"
    );
}

#[test]
fn a_lock_on_a_parameter_no_one_registered_passes_through() {
    let mut e = engine();
    let other = ParamAddr::new(Owner::Inst(2), 9);
    let log = run(
        &mut e,
        128,
        128,
        &[(
            5,
            ModEvent::Lock {
                addr: other,
                value: 0.25,
            },
        )],
        None,
    );
    assert!(log.contains(&(5, other, 0.25)));
    assert_eq!(e.stats().unregistered, 1);
}

#[test]
fn a_hand_beats_a_lock_and_lets_go_to_the_lock_it_holds_then() {
    let mut e = engine();
    let log = run(
        &mut e,
        512,
        128,
        &[
            // A lane's value arrives as a lock.
            (
                64,
                ModEvent::Lock {
                    addr: mix(),
                    value: 90.0,
                },
            ),
            // The hand takes over...
            (
                100,
                ModEvent::Touch {
                    addr: mix(),
                    value: 20.0,
                },
            ),
            // ...while the lane goes on underneath, unheard.
            (
                160,
                ModEvent::Lock {
                    addr: mix(),
                    value: 70.0,
                },
            ),
            (
                200,
                ModEvent::Touch {
                    addr: mix(),
                    value: 25.0,
                },
            ),
            (250, ModEvent::Untouch { addr: mix() }),
            (300, ModEvent::Unlock { addr: mix() }),
        ],
        None,
    );
    let mixes: Vec<_> = log
        .iter()
        .filter(|x| x.1 == mix())
        .map(|x| (x.0, x.2))
        .collect();
    assert_eq!(
        mixes,
        [
            (0, 50.0),
            (64, 90.0),
            (100, 20.0),
            (200, 25.0),
            (250, 70.0),
            (300, 50.0)
        ]
    );
    assert!(!e.is_touched(mix()) && !e.is_locked(mix()));

    // An unlock while the hand holds it changes nothing until it lets go.
    let log = run(
        &mut e,
        512,
        128,
        &[
            (
                0,
                ModEvent::Lock {
                    addr: mix(),
                    value: 80.0,
                },
            ),
            (
                10,
                ModEvent::Touch {
                    addr: mix(),
                    value: 40.0,
                },
            ),
            (20, ModEvent::Unlock { addr: mix() }),
        ],
        None,
    );
    assert_eq!(last(&log, mix()), 40.0);
    assert!(e.is_touched(mix()) && !e.is_locked(mix()));
    assert!(
        !e.idle_targets().any(|a| a == mix()),
        "a held target is not idle"
    );
    let log = run(
        &mut e,
        128,
        128,
        &[(5, ModEvent::Untouch { addr: mix() })],
        None,
    );
    assert_eq!(log, [(5, mix(), 50.0)], "back to the base");
}

#[test]
fn a_hand_on_a_macro_moves_its_targets_and_the_matrix_still_moves_a_held_value() {
    let mut e = engine();
    e.map(
        0,
        MacroMap {
            target: cut(),
            from: 0.0,
            to: 1.0,
            curve: MapCurve::Linear,
        },
    )
    .unwrap();
    e.add_row(Row {
        source: Source::Lfo(0),
        via: None,
        target: mix(),
        depth: 0.25,
    })
    .unwrap();
    let mac = ParamAddr::macro_value(0);
    let log = run(
        &mut e,
        RATE as u64,
        128,
        &[
            (
                0,
                ModEvent::Lock {
                    addr: mac,
                    value: 0.0,
                },
            ),
            (
                1000,
                ModEvent::Touch {
                    addr: mac,
                    value: 1.0,
                },
            ),
            (
                1000,
                ModEvent::Touch {
                    addr: mix(),
                    value: 50.0,
                },
            ),
        ],
        None,
    );
    let cuts: Vec<_> = log
        .iter()
        .filter(|x| x.1 == cut())
        .map(|x| (x.0, x.2))
        .collect();
    assert_eq!(
        cuts,
        [(0, 20.0), (1000, 20000.0)],
        "the macro follows the hand"
    );
    // 1 Hz sine around the held 50 %: 25 % to 75 %.
    let vals: Vec<f32> = log
        .iter()
        .filter(|x| x.1 == mix() && x.0 >= 1000)
        .map(|x| x.2)
        .collect();
    let (lo, hi) = vals
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    assert!(
        (lo - 25.0).abs() < 0.2 && (hi - 75.0).abs() < 0.2,
        "{lo} {hi}"
    );
}

#[test]
fn a_hand_on_a_parameter_no_one_registered_passes_through() {
    let mut e = engine();
    let other = ParamAddr::new(Owner::Inst(2), 9);
    let log = run(
        &mut e,
        128,
        128,
        &[
            (
                5,
                ModEvent::Touch {
                    addr: other,
                    value: 0.25,
                },
            ),
            (9, ModEvent::Untouch { addr: other }),
        ],
        None,
    );
    assert_eq!(
        log.iter().filter(|x| x.1 == other).collect::<Vec<_>>(),
        [&(5, other, 0.25)]
    );
    assert_eq!(e.stats().unregistered, 1);
}

#[test]
fn an_lfo_row_sweeps_around_the_knob() {
    let mut e = engine();
    e.add_row(Row {
        source: Source::Lfo(0),
        via: None,
        target: mix(),
        depth: 0.25,
    })
    .unwrap();
    // 1 Hz sine from 50 %: 25 % to 75 % over a second.
    let log = run(&mut e, RATE as u64, 128, &[], None);
    let vals: Vec<f32> = log.iter().filter(|x| x.1 == mix()).map(|x| x.2).collect();
    let (lo, hi) = vals
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    assert!(
        (lo - 25.0).abs() < 0.1 && (hi - 75.0).abs() < 0.1,
        "{lo} {hi}"
    );
    assert!(vals.len() > 1000, "every tick: {}", vals.len());
    assert_eq!(
        e.mod_range(mix())
            .map(|(a, b)| ((a * 100.0).round(), (b * 100.0).round())),
        Some((25.0, 75.0))
    );
    // Rows clamp to the knob's travel, and switches refuse rows.
    e.add_row(Row {
        source: Source::Lfo(0),
        via: None,
        target: mix(),
        depth: 1.0,
    })
    .unwrap();
    let log = run(&mut e, RATE as u64, 128, &[], None);
    assert!(log
        .iter()
        .filter(|x| x.1 == mix())
        .all(|x| (0.0..=100.0).contains(&x.2)));
    assert_eq!(
        e.add_row(Row {
            source: Source::Lfo(0),
            via: None,
            target: mode(),
            depth: 0.5
        }),
        Err(ModError::Refused)
    );
    assert_eq!(
        e.add_row(Row {
            source: Source::Lfo(9),
            via: None,
            target: mix(),
            depth: 0.5
        }),
        Err(ModError::Refused)
    );
    assert!(e.remove_row(1).is_some());
}

#[test]
fn via_scales_a_row_by_a_host_source() {
    let mut e = engine();
    e.add_row(Row {
        source: Source::Lfo(0),
        via: Some(Source::Host(0)),
        target: mix(),
        depth: 0.5,
    })
    .unwrap();
    let log = run(&mut e, RATE as u64 / 2, 128, &[], None);
    assert!(
        log.iter().filter(|x| x.1 == mix()).all(|x| x.2 == 50.0),
        "mod wheel at 0: no movement"
    );
    let log = run(
        &mut e,
        RATE as u64,
        128,
        &[(0, ModEvent::Source { n: 0, value: 1.0 })],
        None,
    );
    assert!(log.iter().filter(|x| x.1 == mix()).any(|x| x.2 > 90.0));
    assert!(log.iter().filter(|x| x.1 == mix()).any(|x| x.2 < 10.0));
}

#[test]
fn the_morph_glides_between_scenes_and_a_turn_edits_the_nearer_one() {
    let mut e = engine();
    e.capture_scene(0);
    let log = run(
        &mut e,
        64,
        64,
        &[(
            0,
            ModEvent::Set {
                addr: mix(),
                value: 100.0,
            },
        )],
        None,
    );
    assert_eq!(last(&log, mix()), 100.0, "one scene alone moves nothing");
    e.capture_scene(1);
    assert_eq!(
        e.scene_changes().collect::<Vec<_>>(),
        [(mix(), 50.0, 100.0)]
    );
    let log = run(
        &mut e,
        64,
        64,
        &[(
            0,
            ModEvent::Set {
                addr: ParamAddr::morph_pos(0),
                value: 0.0,
            },
        )],
        None,
    );
    assert_eq!(last(&log, mix()), 50.0);
    let log = run(
        &mut e,
        64,
        64,
        &[(
            0,
            ModEvent::Set {
                addr: ParamAddr::morph_pos(0),
                value: 0.5,
            },
        )],
        None,
    );
    assert_eq!(last(&log, mix()), 75.0);
    // At 0.8 a turn edits B.
    let log = run(
        &mut e,
        64,
        64,
        &[
            (
                0,
                ModEvent::Set {
                    addr: ParamAddr::morph_pos(0),
                    value: 0.8,
                },
            ),
            (
                1,
                ModEvent::Set {
                    addr: mix(),
                    value: 0.0,
                },
            ),
        ],
        None,
    );
    assert!(
        (last(&log, mix()) - 10.0).abs() < 1e-3,
        "A 50, B 0, at 0.8: {}",
        last(&log, mix())
    );
    // An LFO on the morph sweeps every scene parameter.
    e.add_row(Row {
        source: Source::Lfo(1),
        via: None,
        target: ParamAddr::morph_pos(0),
        depth: 0.2,
    })
    .unwrap();
    let log = run(&mut e, RATE as u64, 128, &[], None);
    assert!(log.iter().filter(|x| x.1 == mix()).count() > 100);
}

#[test]
fn removing_a_target_keeps_the_others_mapped() {
    let mut e = engine();
    e.map(
        0,
        MacroMap {
            target: cut(),
            from: 0.0,
            to: 1.0,
            curve: MapCurve::Linear,
        },
    )
    .unwrap();
    e.add_row(Row {
        source: Source::Lfo(0),
        via: None,
        target: mode(),
        depth: 0.1,
    })
    .unwrap_err();
    e.add_row(Row {
        source: Source::Lfo(0),
        via: None,
        target: mix(),
        depth: 0.1,
    })
    .unwrap();
    assert!(e.remove_target(cut()));
    assert!(!e.is_target(cut()));
    assert_eq!(e.macro_maps(0).count(), 0);
    assert_eq!(e.rows().map(|r| r.target).collect::<Vec<_>>(), [mix()]);
    let log = run(&mut e, RATE as u64 / 4, 128, &[], None);
    assert!(
        log.iter().filter(|x| x.1 == mix()).count() > 10,
        "the row still finds its target"
    );
    assert!(!e.remove_target(ParamAddr::macro_value(0)));
    assert_eq!(e.idle_targets().collect::<Vec<_>>(), [mode()]);
}

#[test]
fn a_follower_opens_a_parameter_with_the_audio() {
    let mut e = engine();
    e.add_row(Row {
        source: Source::Follower(0),
        via: None,
        target: mix(),
        depth: -0.5,
    })
    .unwrap();
    let audio: Vec<f32> = (0..RATE as usize)
        .map(|i| if i < RATE as usize / 2 { 0.0 } else { 0.9 })
        .collect();
    let log = run(&mut e, RATE as u64, 128, &[], Some(&audio));
    let at = |f: u64| log.iter().rfind(|x| x.1 == mix() && x.0 <= f).unwrap().2;
    assert_eq!(at(20_000), 50.0);
    assert!(at(47_000) < 10.0, "ducked by the loud half: {}", at(47_000));
}

#[test]
fn the_same_events_whatever_the_block_size() {
    let setup = || {
        let mut e = engine();
        e.map(
            0,
            MacroMap {
                target: cut(),
                from: 0.1,
                to: 0.9,
                curve: MapCurve::S,
            },
        )
        .unwrap();
        e.add_row(Row {
            source: Source::Lfo(0),
            via: None,
            target: mix(),
            depth: 0.3,
        })
        .unwrap();
        e.add_row(Row {
            source: Source::Lfo(1),
            via: None,
            target: ParamAddr::macro_value(0),
            depth: 0.2,
        })
        .unwrap();
        e.add_row(Row {
            source: Source::Follower(0),
            via: None,
            target: cut(),
            depth: 0.1,
        })
        .unwrap();
        let ev = |f, ev| (f, ev);
        let events = vec![
            ev(
                0,
                ModEvent::Set {
                    addr: ParamAddr::new(Owner::Mod(0), lfo::ids::RATE),
                    value: 3.3,
                },
            ),
            ev(
                0,
                ModEvent::Set {
                    addr: ParamAddr::new(Owner::Mod(1), lfo::ids::SYNC),
                    value: 6.0,
                },
            ),
            ev(
                0,
                ModEvent::Set {
                    addr: ParamAddr::new(Owner::Mod(1), lfo::ids::SHAPE),
                    value: 5.0,
                },
            ),
            ev(
                777,
                ModEvent::Set {
                    addr: ParamAddr::macro_value(0),
                    value: 0.4,
                },
            ),
            ev(
                5000,
                ModEvent::Lock {
                    addr: mix(),
                    value: 10.0,
                },
            ),
            ev(5000, ModEvent::Trigger),
            ev(
                6000,
                ModEvent::Touch {
                    addr: mix(),
                    value: 80.0,
                },
            ),
            ev(9001, ModEvent::Unlock { addr: mix() }),
            ev(11_000, ModEvent::Untouch { addr: mix() }),
            ev(
                14_000,
                ModEvent::Touch {
                    addr: ParamAddr::macro_value(0),
                    value: 0.9,
                },
            ),
            ev(
                20_001,
                ModEvent::Untouch {
                    addr: ParamAddr::macro_value(0),
                },
            ),
            ev(
                12_345,
                ModEvent::Set {
                    addr: cut(),
                    value: 300.0,
                },
            ),
        ];
        (e, events)
    };
    let audio: Vec<f32> = (0..30_000)
        .map(|i| ((i as f32) * 0.02).sin() * if (i / 4000) % 2 == 0 { 0.7 } else { 0.01 })
        .collect();
    let (mut e, events) = setup();
    let want = run(&mut e, 30_000, 128, &events, Some(&audio));
    assert!(want.len() > 500);
    for block in [1, 7, 31, 32, 33, 100, 512, 8192] {
        let (mut e, events) = setup();
        let got = run(&mut e, 30_000, block, &events, Some(&audio));
        assert_eq!(got.len(), want.len(), "block {block}");
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(g, w, "block {block}");
        }
    }
}
