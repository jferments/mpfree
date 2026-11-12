//! Keyboards into tracks, the clock source, MIDI Learn and CV/gate, end
//! to end on bytes.

use std::cell::RefCell;

use mpfree_midi::cv::{Calibration, CvAssign, CvChange, CvEngine, Priority, Scale};
use mpfree_midi::input::{ChanFilter, In, InputMode, InputRouter, Listen, PortFilter};
use mpfree_midi::learn::{Encoding, Input, LearnMap, Mode, Takeover};
use mpfree_midi::output::{OutQueue, OutRoute, OutputRouter};
use mpfree_midi::{Msg, PortId, DIN, MAX_PORTS};
use mpfree_param::{OnTrack, ParamAddr};

const KEYS: PortId = PortId(1);

fn feed(r: &mut InputRouter, port: PortId, bytes: &[u8]) -> Vec<In> {
    let mut v = Vec::new();
    r.feed_all(port, bytes, 0, |e| v.push(e));
    v
}

fn to_tracks(evs: &[In]) -> Vec<(u8, Msg)> {
    evs.iter()
        .filter_map(|e| match *e {
            In::Track { track, msg, .. } => Some((track, msg)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_keyboard_plays_the_selected_track_and_a_held_note_ends_where_it_began() {
    let mut r = InputRouter::new();
    r.set_selected(Some(2));
    r.set_listen(
        2,
        Some(Listen {
            transpose: 12,
            ..Listen::default()
        }),
    );
    // Note on, with running status for a second note.
    let ev = feed(&mut r, KEYS, &[0x90, 60, 100, 64, 90]);
    assert_eq!(
        to_tracks(&ev),
        vec![
            (
                2,
                Msg::NoteOn {
                    ch: 0,
                    note: 72,
                    vel: 100
                }
            ),
            (
                2,
                Msg::NoteOn {
                    ch: 0,
                    note: 76,
                    vel: 90
                }
            )
        ]
    );
    assert!(ev
        .iter()
        .any(|e| matches!(e, In::Seen { port, .. } if *port == KEYS)));
    // The user picks another track while the keys are down.
    r.set_selected(Some(5));
    let ev = feed(&mut r, KEYS, &[0x80, 60, 0, 0x90, 64, 0]);
    assert_eq!(
        to_tracks(&ev),
        vec![
            (
                2,
                Msg::NoteOff {
                    ch: 0,
                    note: 72,
                    vel: 0
                }
            ),
            (
                2,
                Msg::NoteOff {
                    ch: 0,
                    note: 76,
                    vel: 64
                }
            )
        ]
    );
    assert_eq!(r.held(), 0);
    let ev = feed(&mut r, KEYS, &[0x90, 50, 1]);
    assert_eq!(
        to_tracks(&ev),
        vec![(
            5,
            Msg::NoteOn {
                ch: 0,
                note: 50,
                vel: 1
            }
        )]
    );
}

#[test]
fn per_track_mode_splits_by_port_and_channel() {
    let mut r = InputRouter::new();
    r.set_mode(InputMode::PerTrack);
    r.set_listen(
        0,
        Some(Listen {
            port: PortFilter::One(DIN),
            channel: ChanFilter::One(9),
            transpose: 0,
        }),
    );
    r.set_listen(
        1,
        Some(Listen {
            port: PortFilter::Any,
            channel: ChanFilter::One(0),
            transpose: 0,
        }),
    );
    r.set_listen(
        3,
        Some(Listen {
            port: PortFilter::One(KEYS),
            channel: ChanFilter::Omni,
            transpose: -12,
        }),
    );
    let ev = feed(&mut r, DIN, &[0x99, 36, 127, 0x90, 60, 80, 0xB0, 1, 64]);
    assert_eq!(
        to_tracks(&ev),
        vec![
            (
                0,
                Msg::NoteOn {
                    ch: 9,
                    note: 36,
                    vel: 127
                }
            ),
            (
                1,
                Msg::NoteOn {
                    ch: 0,
                    note: 60,
                    vel: 80
                }
            ),
            (
                1,
                Msg::Cc {
                    ch: 0,
                    cc: 1,
                    value: 64
                }
            ),
        ]
    );
    let ev = feed(&mut r, KEYS, &[0x90, 60, 80]);
    assert_eq!(
        to_tracks(&ev),
        vec![
            (
                1,
                Msg::NoteOn {
                    ch: 0,
                    note: 60,
                    vel: 80
                }
            ),
            (
                3,
                Msg::NoteOn {
                    ch: 0,
                    note: 48,
                    vel: 80
                }
            )
        ]
    );
    // Unplugging the keyboard ends its notes on both tracks.
    let mut v = Vec::new();
    r.port_gone(KEYS, 7, |e| v.push(e));
    assert_eq!(
        to_tracks(&v),
        vec![
            (
                1,
                Msg::NoteOff {
                    ch: 0,
                    note: 60,
                    vel: 64
                }
            ),
            (
                3,
                Msg::NoteOff {
                    ch: 0,
                    note: 48,
                    vel: 64
                }
            )
        ]
    );
    assert_eq!(r.held(), 2); // the DIN notes are still down
    assert_eq!(r.stats.unrouted, 0);
}

#[test]
fn only_the_clock_source_reaches_the_clock() {
    let mut r = InputRouter::new();
    r.set_selected(Some(0));
    r.set_clock_source(Some(DIN));
    let rate = 48_000;
    let mut clock = mpfree_midi_clock::ClockIn::new(rate);
    // 120 BPM from the DIN jack: a pulse every 1000 frames, Song
    // Position 4 (a quarter note in), Continue, with a note in between.
    let mut frame = 0u64;
    let mut send = |r: &mut InputRouter, port, bytes: &[u8], frame| {
        r.feed_all(port, bytes, frame, |e| {
            if let In::ClockByte { byte, frame } = e {
                clock.byte(frame, byte);
            }
        })
    };
    send(&mut r, DIN, &[0xF2, 4, 0], frame);
    send(&mut r, DIN, &[0xFB], frame);
    for i in 0..48 {
        // A keyboard on another port sends its own clock: ignored.
        send(&mut r, KEYS, &[0xF8], frame + 500);
        if i == 10 {
            send(&mut r, DIN, &[0x90, 60, 100], frame);
        }
        send(&mut r, DIN, &[0xF8], frame);
        frame += 1000;
    }
    let st = clock.state(frame);
    assert!(st.playing);
    let bpm = st.bpm.unwrap();
    assert!((bpm - 120.0).abs() < 0.01, "{bpm}");
    // 960 ticks (Song Position 4 sixteenths) + 47 pulses after the first.
    assert_eq!(st.tick, 960 + 47 * 40);
    assert_eq!(r.stats.foreign_clock, 48);
}

#[test]
fn tracks_out_with_their_own_channels() {
    let mut out = OutputRouter::new();
    let mut q: Vec<OutQueue> = (0..MAX_PORTS).map(|_| OutQueue::new()).collect();
    out.set_route(
        0,
        Some(OutRoute {
            port: DIN,
            channel: 0,
        }),
        0,
        &mut q,
    );
    out.set_route(
        1,
        Some(OutRoute {
            port: DIN,
            channel: 9,
        }),
        0,
        &mut q,
    );
    out.send(
        1,
        Msg::NoteOn {
            ch: 0,
            note: 36,
            vel: 100,
        },
        0,
        &mut q,
    );
    out.send(
        0,
        Msg::NoteOn {
            ch: 0,
            note: 48,
            vel: 100,
        },
        64,
        &mut q,
    );
    out.send(
        1,
        Msg::NoteOff {
            ch: 0,
            note: 36,
            vel: 0,
        },
        32,
        &mut q,
    );
    q[0].sort();
    let mut buf = [0u8; 32];
    let mut last = 0;
    let n = q[0].write_bytes(&mut buf, false, &mut last);
    assert_eq!(&buf[..n], &[0x99, 36, 100, 0x89, 36, 0, 0x90, 48, 100]);
}

fn addr(p: u16) -> ParamAddr {
    ParamAddr::new(OnTrack::Inst.on(3), p)
}

fn writes(map: &mut LearnMap, port: PortId, m: Msg) -> Vec<Input> {
    let mut v = Vec::new();
    map.input(port, &m, |e| v.push(e));
    v
}

#[test]
fn learn_a_knob_then_turn_it() {
    let mut map = LearnMap::new();
    let cutoff = addr(12);
    map.arm(cutoff);
    assert_eq!(
        writes(
            &mut map,
            KEYS,
            Msg::Cc {
                ch: 2,
                cc: 74,
                value: 10
            }
        ),
        vec![Input::Learned(0)]
    );
    assert_eq!(map.armed(), None);
    let w = writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 2,
            cc: 74,
            value: 127,
        },
    );
    assert!(matches!(w[..], [Input::Write(x)] if x.target == cutoff && x.value == 1.0));
    // The same CC on another channel or port does nothing.
    assert!(writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 3,
            cc: 74,
            value: 1
        }
    )
    .is_empty());
    assert!(writes(
        &mut map,
        DIN,
        Msg::Cc {
            ch: 2,
            cc: 74,
            value: 1
        }
    )
    .is_empty());
    // Upside down, half range.
    map.set_range(0, 0.75, 0.25);
    let w = writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 2,
            cc: 74,
            value: 0,
        },
    );
    assert!(matches!(w[..], [Input::Write(x)] if x.value == 0.75));
}

#[test]
fn pickup_waits_for_the_knob_to_reach_the_value() {
    let mut map = LearnMap::new();
    let t = addr(1);
    map.arm(t);
    writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 0,
            cc: 20,
            value: 0,
        },
    );
    map.set_takeover(0, Takeover::Pickup);
    map.observe(t, 0.5); // the screen says 0.5
    assert!(writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 0,
            cc: 20,
            value: 10
        }
    )
    .is_empty());
    assert!(writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 0,
            cc: 20,
            value: 40
        }
    )
    .is_empty());
    // It jumps past 0.5 (63.5) between two messages: that counts.
    let w = writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 0,
            cc: 20,
            value: 70,
        },
    );
    assert!(matches!(w[..], [Input::Write(x)] if (x.value - 70.0 / 127.0).abs() < 1e-6));
    // Picked up: it follows from now on, until the screen moves it.
    assert_eq!(
        writes(
            &mut map,
            KEYS,
            Msg::Cc {
                ch: 0,
                cc: 20,
                value: 0
            }
        )
        .len(),
        1
    );
    map.observe(t, 0.9);
    assert!(writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 0,
            cc: 20,
            value: 1
        }
    )
    .is_empty());
}

#[test]
fn encoders_buttons_and_keys() {
    let mut map = LearnMap::new();
    let (a, b, c) = (addr(1), addr(2), addr(3));
    map.arm(a);
    writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 0,
            cc: 16,
            value: 1,
        },
    );
    map.set_mode(0, Mode::Relative(Encoding::TwosComplement));
    map.observe(a, 0.5);
    let w = writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 0,
            cc: 16,
            value: 127,
        },
    ); // one step down
    assert!(matches!(w[..], [Input::Write(x)] if (x.value - (0.5 - 1.0 / 128.0)).abs() < 1e-6));
    assert_eq!(Encoding::Offset64.delta(66), 2);
    assert_eq!(Encoding::SignBit.delta(66), -2);

    map.arm(b);
    writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 0,
            cc: 64,
            value: 127,
        },
    ); // sustain pedal
    map.set_mode(1, Mode::Toggle);
    let on = writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 0,
            cc: 64,
            value: 127,
        },
    );
    writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 0,
            cc: 64,
            value: 0,
        },
    );
    let off = writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 0,
            cc: 64,
            value: 127,
        },
    );
    assert!(matches!(on[..], [Input::Write(x)] if x.value == 1.0));
    assert!(matches!(off[..], [Input::Write(x)] if x.value == 0.0));

    // A key learns as momentary; its release does not learn anything.
    map.arm(c);
    assert!(writes(
        &mut map,
        KEYS,
        Msg::NoteOff {
            ch: 0,
            note: 30,
            vel: 0
        }
    )
    .is_empty());
    assert_eq!(
        writes(
            &mut map,
            KEYS,
            Msg::NoteOn {
                ch: 0,
                note: 30,
                vel: 9
            }
        ),
        vec![Input::Learned(2)]
    );
    let held = writes(
        &mut map,
        KEYS,
        Msg::NoteOn {
            ch: 0,
            note: 30,
            vel: 9,
        },
    );
    let up = writes(
        &mut map,
        KEYS,
        Msg::NoteOff {
            ch: 0,
            note: 30,
            vel: 0,
        },
    );
    assert!(matches!(held[..], [Input::Write(x)] if x.value == 1.0));
    assert!(matches!(up[..], [Input::Write(x)] if x.value == 0.0));
}

#[test]
fn learned_controls_save_by_device_name() {
    let mut map = LearnMap::new();
    map.arm(addr(12));
    writes(
        &mut map,
        KEYS,
        Msg::Cc {
            ch: 2,
            cc: 74,
            value: 1,
        },
    );
    map.arm(addr(3));
    writes(&mut map, DIN, Msg::PitchBend { ch: 0, value: 0 });
    map.set_mode(1, Mode::Relative(Encoding::SignBit));
    map.set_takeover(1, Takeover::Pickup);
    let name = |p: PortId| {
        Some(if p == DIN {
            "MIDI".to_string()
        } else {
            "Keystep | 37".to_string()
        })
    };
    let text = map.save(name);
    assert_eq!(
        text,
        "learn = port Keystep / 37 | ch 3 | cc 74 | t3.inst.12 | 0.0000 1.0000 | absolute | jump\n\
         learn = port MIDI | ch 1 | bend | t3.inst.3 | 0.0000 1.0000 | relative-sign | pickup\n"
    );
    // Next time the keyboard came up as port 4.
    let (back, bad) = LearnMap::load(&format!("{text}learn = nonsense\n"), |n| match n {
        "MIDI" => Some(DIN),
        "Keystep / 37" => Some(PortId(4)),
        _ => None,
    });
    assert_eq!(bad, 1);
    let m: Vec<_> = back.mappings().copied().collect();
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].control.port, PortFilter::One(PortId(4)));
    assert_eq!(m[1].mode, Mode::Relative(Encoding::SignBit));
    assert_eq!(m[1].takeover, Takeover::Pickup);
    assert_eq!(m[0].target, addr(12));
}

#[test]
fn cv_pitch_and_gate_follow_a_mono_line() {
    let mut cv = CvEngine::new(48_000);
    let ch = RefCell::new(Vec::new());
    let mut emit = |c: CvChange| ch.borrow_mut().push(c);
    let pitch = CvAssign::Pitch {
        track: 0,
        scale: Scale::VoltsPerOctave(1.0),
        zero_note: 36,
        bend_semis: 2,
        priority: Priority::Last,
    };
    cv.set_assign(0, pitch, 0, &mut emit);
    cv.set_assign(
        1,
        CvAssign::Gate {
            track: 0,
            volts: 5.0,
            retrigger_ms: 2.0,
        },
        0,
        &mut emit,
    );
    cv.set_assign(
        2,
        CvAssign::Velocity {
            track: 0,
            max_volts: 8.0,
        },
        0,
        &mut emit,
    );
    let note = |n, v| Msg::NoteOn {
        ch: 0,
        note: n,
        vel: v,
    };
    cv.track_msg(0, note(48, 127), 10, &mut emit); // C3: 1 V, gate up
    cv.track_msg(0, note(55, 64), 20, &mut emit); // G3 while C3 is held: retrigger
    cv.end_period(128, &mut emit); // 2 ms = 96 frames: the gate is back up at 116
    cv.track_msg(0, Msg::PitchBend { ch: 0, value: 8191 }, 0, &mut emit);
    cv.track_msg(
        0,
        Msg::NoteOff {
            ch: 0,
            note: 55,
            vel: 0,
        },
        5,
        &mut emit,
    ); // back to C3, gate stays up
    cv.track_msg(
        0,
        Msg::NoteOff {
            ch: 0,
            note: 48,
            vel: 0,
        },
        9,
        &mut emit,
    ); // gate down, pitch stays
    cv.end_period(128, &mut emit);
    let at = |o: u8| -> Vec<(u32, f32)> {
        ch.borrow()
            .iter()
            .filter(|c| c.out == o)
            .map(|c| (c.offset, c.volts))
            .collect()
    };
    let bend = 8191.0 / 8192.0 * 2.0 / 12.0;
    let p = at(0);
    let want = [
        (0, 0.0),
        (10, 1.0),
        (20, 19.0 / 12.0),
        (0, 19.0 / 12.0 + bend),
        (5, 1.0 + bend),
    ];
    assert_eq!(p.len(), want.len(), "{p:?}");
    for (got, want) in p.iter().zip(want) {
        assert_eq!(got.0, want.0);
        assert!((got.1 - want.1).abs() < 1e-6, "{got:?} {want:?}");
    }
    assert_eq!(
        at(1),
        vec![(0, 0.0), (10, 5.0), (20, 0.0), (116, 5.0), (9, 0.0)]
    );
    assert_eq!(at(2), vec![(0, 0.0), (10, 8.0), (20, 64.0 / 127.0 * 8.0)]);
}

/// A sequence's notes back to back: the gate drops for GATE_GAP_MS
/// between them, so the envelope starts again; the pitch moves at once.
#[test]
fn cv_gate_drops_between_notes_that_touch() {
    let mut cv = CvEngine::new(48_000);
    let ch = RefCell::new(Vec::new());
    let mut emit = |c: CvChange| ch.borrow_mut().push(c);
    let pitch = CvAssign::Pitch {
        track: 0,
        scale: Scale::VoltsPerOctave(1.0),
        zero_note: 36,
        bend_semis: 0,
        priority: Priority::Last,
    };
    let gate = CvAssign::Gate {
        track: 0,
        volts: 5.0,
        retrigger_ms: 0.0,
    };
    cv.set_assign(0, pitch, 0, &mut emit);
    cv.set_assign(1, gate, 0, &mut emit);
    let on = |note| Msg::NoteOn {
        ch: 0,
        note,
        vel: 100,
    };
    let off = |note| Msg::NoteOff {
        ch: 0,
        note,
        vel: 0,
    };
    cv.track_msg(0, on(48), 0, &mut emit);
    cv.track_msg(0, off(48), 40, &mut emit);
    cv.track_msg(0, on(60), 40, &mut emit);
    cv.end_period(128, &mut emit);
    // The gap ends at frame 136, offset 8 in the next period, before
    // that note ends; the next starts a frame later, with no gap.
    cv.track_msg(0, off(60), 10, &mut emit);
    cv.track_msg(0, on(48), 11, &mut emit);
    cv.end_period(128, &mut emit);
    let at = |o: u8| -> Vec<(u32, f32)> {
        ch.borrow()
            .iter()
            .filter(|c| c.out == o)
            .map(|c| (c.offset, c.volts))
            .collect()
    };
    // 2 ms at 48 kHz is 96 frames.
    assert_eq!(
        at(1),
        vec![
            (0, 0.0),
            (0, 5.0),
            (40, 0.0),
            (8, 5.0),
            (10, 0.0),
            (11, 5.0)
        ]
    );
    assert_eq!(at(0), vec![(0, 0.0), (0, 1.0), (40, 2.0), (11, 1.0)]);
}

#[test]
fn cv_priority_hz_per_volt_and_calibration() {
    let mut cv = CvEngine::new(48_000);
    let ch = RefCell::new(Vec::new());
    let mut emit = |c: CvChange| ch.borrow_mut().push(c);
    let low = CvAssign::Pitch {
        track: 1,
        scale: Scale::VoltsPerOctave(1.0),
        zero_note: 0,
        bend_semis: 0,
        priority: Priority::Low,
    };
    let hz = CvAssign::Pitch {
        track: 1,
        scale: Scale::HzPerVolt {
            ref_volts: 1.0,
            ref_note: 48,
        },
        zero_note: 0,
        bend_semis: 0,
        priority: Priority::High,
    };
    cv.set_assign(0, low, 0, &mut emit);
    cv.set_assign(1, hz, 0, &mut emit);
    for n in [60, 48, 72] {
        cv.track_msg(
            1,
            Msg::NoteOn {
                ch: 0,
                note: n,
                vel: 100,
            },
            0,
            &mut emit,
        );
    }
    assert_eq!(cv.volts()[0], 4.0); // lowest held: C3 = 48 / 12
    assert_eq!(cv.volts()[1], 4.0); // highest held: two octaves over 1 V at 48
    cv.set_calibration(
        0,
        Calibration {
            gain: 1.01,
            offset: -0.02,
        },
        3,
        &mut emit,
    );
    assert!((cv.volts()[0] - (4.0 * 1.01 - 0.02)).abs() < 1e-6);
    assert_eq!(ch.borrow().last().unwrap().offset, 3);
    // Track 2's notes do not move track 1's outputs.
    let before = ch.borrow().len();
    cv.track_msg(
        2,
        Msg::NoteOn {
            ch: 0,
            note: 1,
            vel: 1,
        },
        0,
        &mut emit,
    );
    assert_eq!(ch.borrow().len(), before);
}

#[test]
fn cv_clock_reset_run_and_triggers() {
    let mut cv = CvEngine::new(48_000);
    let ch = RefCell::new(Vec::new());
    let mut emit = |c: CvChange| ch.borrow_mut().push(c);
    cv.set_assign(
        0,
        CvAssign::Clock {
            ppqn: 4,
            volts: 5.0,
            ms: 1.0,
        },
        0,
        &mut emit,
    );
    cv.set_assign(1, CvAssign::Run { volts: 5.0 }, 0, &mut emit);
    cv.set_assign(
        2,
        CvAssign::Reset {
            volts: 5.0,
            ms: 1.0,
        },
        0,
        &mut emit,
    );
    cv.set_assign(
        3,
        CvAssign::Trigger {
            track: 9,
            note: Some(36),
            volts: 10.0,
            ms: 5.0,
        },
        0,
        &mut emit,
    );
    cv.set_assign(4, CvAssign::Fixed { volts: -2.5 }, 0, &mut emit);
    ch.borrow_mut().clear();
    cv.started(true, 0, &mut emit);
    cv.clock(4, 0, &mut emit);
    cv.clock(24, 1, &mut emit); // another ppqn: no pulse here
    cv.track_msg(
        9,
        Msg::NoteOn {
            ch: 9,
            note: 38,
            vel: 100,
        },
        2,
        &mut emit,
    ); // not its note
    cv.track_msg(
        9,
        Msg::NoteOn {
            ch: 9,
            note: 36,
            vel: 100,
        },
        3,
        &mut emit,
    );
    cv.end_period(128, &mut emit); // nothing ends yet: 1 ms is 48 frames
    cv.stopped(10, &mut emit);
    cv.end_period(128, &mut emit);
    cv.end_period(128, &mut emit);
    let at = |o: u8| -> Vec<(u32, f32)> {
        ch.borrow()
            .iter()
            .filter(|c| c.out == o)
            .map(|c| (c.offset, c.volts))
            .collect()
    };
    assert_eq!(at(0), vec![(0, 5.0), (48, 0.0)]);
    assert_eq!(at(1), vec![(0, 5.0), (10, 0.0)]);
    assert_eq!(at(2), vec![(0, 5.0), (48, 0.0)]);
    assert_eq!(at(3), vec![(3, 10.0), (243 - 128, 0.0)]);
    assert_eq!(cv.volts()[4], -2.5);
}
