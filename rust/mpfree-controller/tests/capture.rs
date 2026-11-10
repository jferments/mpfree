//! Tests built from the 2026-11-09 passive capture on the owner's MPC One
//! (IN messages on cable 1, "MPC Studio Live Private").

use mpfree_controller::midi::{Message, Parser, SYSEX_CAPACITY};
use mpfree_controller::mpc_one::{self, button, ControlEvent, Port};

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .flat_map(|w| {
            (0..w.len())
                .step_by(2)
                .map(move |i| u8::from_str_radix(&w[i..i + 2], 16).unwrap())
        })
        .collect()
}

fn events(bytes: &[u8]) -> Vec<ControlEvent> {
    let mut p = Parser::new();
    let mut out = Vec::new();
    for &b in bytes {
        if let Some(m) = p.feed(b) {
            if let Some(e) = mpc_one::decode(&m) {
                out.push(e);
            }
        }
    }
    assert_eq!(p.stray_bytes, 0);
    out
}

#[test]
fn pad1_hit_with_pressure_and_release() {
    let ev = events(&hex("992571 a9254a a92532 a92508 a92500 892500"));
    assert_eq!(
        ev,
        vec![
            ControlEvent::PadDown {
                pad: 0,
                velocity: 0x71
            },
            ControlEvent::PadPressure {
                pad: 0,
                pressure: 0x4a
            },
            ControlEvent::PadPressure {
                pad: 0,
                pressure: 0x32
            },
            ControlEvent::PadPressure {
                pad: 0,
                pressure: 0x08
            },
            ControlEvent::PadPressure {
                pad: 0,
                pressure: 0x00
            },
            ControlEvent::PadUp { pad: 0 },
        ]
    );
}

#[test]
fn all_sixteen_pads_map_in_order() {
    for (i, &note) in mpc_one::PAD_NOTES.iter().enumerate() {
        let ev = events(&[0x99, note, 0x7f, 0x89, note, 0x00]);
        assert_eq!(
            ev,
            vec![
                ControlEvent::PadDown {
                    pad: i as u8,
                    velocity: 0x7f
                },
                ControlEvent::PadUp { pad: i as u8 }
            ]
        );
    }
    assert_eq!(
        events(&hex("99247f")),
        vec![ControlEvent::PadDown {
            pad: 1,
            velocity: 0x7f
        }]
    );
    assert_eq!(
        events(&hex("99526d")),
        vec![ControlEvent::PadDown {
            pad: 3,
            velocity: 0x6d
        }]
    );
}

#[test]
fn pad_note_on_velocity_zero_is_release() {
    assert_eq!(events(&hex("992500")), vec![ControlEvent::PadUp { pad: 0 }]);
}

#[test]
fn unknown_pad_note_is_ignored() {
    assert_eq!(events(&hex("99007f")), vec![]);
}

#[test]
fn qlinks_turn_relative() {
    let ev = events(&hex("b01001 b01001 b0117f b0127e b01302"));
    assert_eq!(
        ev,
        vec![
            ControlEvent::QLinkTurn { knob: 0, delta: 1 },
            ControlEvent::QLinkTurn { knob: 0, delta: 1 },
            ControlEvent::QLinkTurn { knob: 1, delta: -1 },
            ControlEvent::QLinkTurn { knob: 2, delta: -2 },
            ControlEvent::QLinkTurn { knob: 3, delta: 2 },
        ]
    );
}

#[test]
fn qlink_touch_and_release() {
    let ev = events(&hex("90547f 805400 90577f 805700"));
    assert_eq!(
        ev,
        vec![
            ControlEvent::QLinkTouch {
                knob: 0,
                touched: true
            },
            ControlEvent::QLinkTouch {
                knob: 0,
                touched: false
            },
            ControlEvent::QLinkTouch {
                knob: 3,
                touched: true
            },
            ControlEvent::QLinkTouch {
                knob: 3,
                touched: false
            },
        ]
    );
}

#[test]
fn data_wheel_three_clicks_each_way() {
    let ev = events(&hex("b06401 b06401 b06401 b0647f b0647f b0647f"));
    let deltas: Vec<i8> = ev
        .iter()
        .map(|e| match e {
            ControlEvent::DataWheel { delta } => *delta,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(deltas, vec![1, 1, 1, -1, -1, -1]);
}

#[test]
fn buttons_press_and_release() {
    let ev = events(&hex(
        "90317f 903100 90277f 902700 90527f 905200 90517f 905100",
    ));
    let b = |note, pressed| ControlEvent::Button { note, pressed };
    assert_eq!(
        ev,
        vec![
            b(button::SHIFT, true),
            b(button::SHIFT, false),
            b(button::FULL_LEVEL, true),
            b(button::FULL_LEVEL, false),
            b(button::PLAY, true),
            b(button::PLAY, false),
            b(button::STOP, true),
            b(button::STOP, false),
        ]
    );
}

#[test]
fn running_status_is_honoured() {
    // b0 10 01, then running status 10 7f.
    let ev = events(&hex("b01001 107f"));
    assert_eq!(
        ev,
        vec![
            ControlEvent::QLinkTurn { knob: 0, delta: 1 },
            ControlEvent::QLinkTurn { knob: 0, delta: -1 }
        ]
    );
}

#[test]
fn keepalive_reply_decodes_token() {
    let ev = events(&hex("f0477f465300057d6d573501f7"));
    assert_eq!(
        ev,
        vec![ControlEvent::KeepAliveReply {
            token: mpc_one::OBSERVED_KEEPALIVE_TOKEN
        }]
    );
}

#[test]
fn realtime_inside_message_does_not_break_it() {
    let ev = events(&hex("99 25 f8 71"));
    assert_eq!(
        ev,
        vec![ControlEvent::PadDown {
            pad: 0,
            velocity: 0x71
        }]
    );
}

#[test]
fn other_channels_are_ignored() {
    assert_eq!(events(&hex("b11001 91317f")), vec![]);
}

#[test]
fn stray_data_is_counted() {
    let mut p = Parser::new();
    assert!(p.feed(0x10).is_none());
    assert!(p.feed(0xF7).is_none());
    assert_eq!(p.stray_bytes, 2);
}

#[test]
fn long_sysex_is_truncated_but_terminated() {
    let mut p = Parser::new();
    assert!(p.feed(0xF0).is_none());
    for _ in 0..100 {
        assert!(p.feed(0x01).is_none());
    }
    match p.feed(0xF7) {
        Some(Message::SysEx { bytes, truncated }) => {
            assert!(truncated);
            assert_eq!(bytes.len(), SYSEX_CAPACITY);
            assert_eq!(bytes[0], 0xF0);
            assert_eq!(*bytes.last().unwrap(), 0xF7);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn sysex_interrupted_by_status_is_dropped() {
    let mut p = Parser::new();
    for &b in &hex("f0 47 7f 46") {
        assert!(p.feed(b).is_none());
    }
    assert!(p.feed(0x90).is_none());
    assert!(p.feed(0x31).is_none());
    assert_eq!(
        p.feed(0x7f),
        Some(Message::Channel {
            status: 0x90,
            data1: 0x31,
            data2: 0x7f
        })
    );
}

#[test]
fn two_byte_channel_messages() {
    let mut p = Parser::new();
    assert!(p.feed(0xC0).is_none());
    assert_eq!(
        p.feed(0x05),
        Some(Message::Channel {
            status: 0xC0,
            data1: 5,
            data2: 0
        })
    );
    assert_eq!(
        p.feed(0x06),
        Some(Message::Channel {
            status: 0xC0,
            data1: 6,
            data2: 0
        })
    );
}

#[test]
fn system_common_has_no_running_status() {
    let mut p = Parser::new();
    assert!(p.feed(0xF3).is_none());
    assert_eq!(
        p.feed(0x02),
        Some(Message::Common {
            status: 0xF3,
            data1: 2,
            data2: 0
        })
    );
    assert!(p.feed(0x03).is_none());
    assert_eq!(p.stray_bytes, 1);
    assert_eq!(
        p.feed(0xF6),
        Some(Message::Common {
            status: 0xF6,
            data1: 0,
            data2: 0
        })
    );
}

#[test]
fn relative_delta_edges() {
    assert_eq!(mpc_one::relative_delta(1), 1);
    assert_eq!(mpc_one::relative_delta(63), 63);
    assert_eq!(mpc_one::relative_delta(64), -64);
    assert_eq!(mpc_one::relative_delta(127), -1);
    assert_eq!(mpc_one::relative_delta(0), 0);
}

#[test]
fn output_builders_match_capture() {
    assert_eq!(
        mpc_one::button_led(button::PLAY, 3),
        (Port::Private, [0xB0, 82, 3])
    );
    let (port, bytes) = mpc_one::pad_rgb(0, 0x7f, 0, 0x10).unwrap();
    assert_eq!(port, Port::Public);
    assert_eq!(bytes.to_vec(), hex("f0477f4665000400 7f0010f7"));
    assert!(mpc_one::pad_rgb(16, 0, 0, 0).is_none());
    let (port, bytes) = mpc_one::keepalive_ping(mpc_one::OBSERVED_KEEPALIVE_TOKEN);
    assert_eq!(port, Port::Public);
    assert_eq!(bytes.to_vec(), hex("f0470046520004 7d6d5735 f7"));
}

#[test]
fn output_builders_mask_to_7_bits() {
    let (_, bytes) = mpc_one::pad_rgb(15, 0xff, 0x80, 0x81).unwrap();
    assert!(bytes[1..11].iter().all(|&b| b < 0x80));
    let (_, led) = mpc_one::button_led(0xff, 0xff);
    assert_eq!(led, [0xB0, 0x7f, 0x7f]);
}
