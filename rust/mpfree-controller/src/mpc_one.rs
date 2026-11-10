//! MPC One (product code ACVA) internal controller mapping.
//!
//! Every constant here was measured on the owner's unit with
//! `collect-over-ssh.sh --midi-capture` while the factory app was running.
//! See `docs/hardware/MPC_ONE_3_6_0_CONTROLLER_PROTOCOL_20261109.md`.

use crate::midi::Message;

/// Akai SysEx manufacturer ID.
pub const AKAI_SYSEX_ID: u8 = 0x47;
/// MPC One device/product byte in Akai SysEx.
pub const DEVICE_ID: u8 = 0x46;

/// USB-MIDI cable / ALSA RawMIDI substream of the internal controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Port {
    /// Cable 0, "MPC Studio Live Public": pad RGB and keep-alive ping go here.
    Public = 0,
    /// Cable 1, "MPC Studio Live Private": all physical controls arrive here;
    /// button LEDs go here.
    Private = 1,
}

/// Pad note numbers for pads 1..=16 (index 0..=15), bottom-left to
/// top-right, row by row, as printed on the MPC One. Channel 10.
pub const PAD_NOTES: [u8; 16] = [
    0x25, 0x24, 0x2A, 0x52, 0x28, 0x26, 0x2E, 0x2C, 0x30, 0x2F, 0x2D, 0x2B, 0x31, 0x37, 0x33, 0x35,
];

/// Q-Link 1..=4 relative-turn CCs (channel 1).
pub const QLINK_TURN_CC: [u8; 4] = [16, 17, 18, 19];
/// Q-Link 1..=4 capacitive-touch notes (channel 1).
pub const QLINK_TOUCH_NOTES: [u8; 4] = [0x54, 0x55, 0x56, 0x57];
/// Data wheel relative-turn CC (channel 1).
pub const DATA_WHEEL_CC: u8 = 100;

/// Button note numbers confirmed by capture (channel 1). A button's LED is
/// driven by a CC with the same number on [`Port::Private`].
pub mod button {
    pub const FULL_LEVEL: u8 = 39;
    pub const SHIFT: u8 = 49;
    pub const STOP: u8 = 81;
    pub const PLAY: u8 = 82;
    /// LED only so far: blinks at the tempo in the factory app.
    pub const TAP_TEMPO: u8 = 53;
}

const PAD_CHANNEL: u8 = 9; // MIDI channel 10, zero-based
const CONTROL_CHANNEL: u8 = 0; // MIDI channel 1

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlEvent {
    /// `pad` is 0..=15 in [`PAD_NOTES`] order; velocity 1..=127.
    PadDown {
        pad: u8,
        velocity: u8,
    },
    PadUp {
        pad: u8,
    },
    /// Polyphonic aftertouch while the pad is held, 0..=127.
    PadPressure {
        pad: u8,
        pressure: u8,
    },
    /// `knob` 0..=3; positive is clockwise. Observed steps: ±1, ±2.
    QLinkTurn {
        knob: u8,
        delta: i8,
    },
    QLinkTouch {
        knob: u8,
        touched: bool,
    },
    /// Positive is clockwise.
    DataWheel {
        delta: i8,
    },
    /// Any other channel-1 note: `note` is the button number.
    Button {
        note: u8,
        pressed: bool,
    },
    /// Controller's reply to [`keepalive_ping`], echoing the token.
    KeepAliveReply {
        token: [u8; 4],
    },
}

/// Two's-complement 7-bit relative value: 1..=63 clockwise, 64..=127 counter.
pub fn relative_delta(value: u8) -> i8 {
    let v = value & 0x7F;
    if v < 64 {
        v as i8
    } else {
        (v as i16 - 128) as i8
    }
}

pub fn pad_index(note: u8) -> Option<u8> {
    PAD_NOTES.iter().position(|&n| n == note).map(|i| i as u8)
}

fn index_of(table: &[u8], v: u8) -> Option<u8> {
    table.iter().position(|&n| n == v).map(|i| i as u8)
}

/// Map one parsed message from [`Port::Private`] to a control event.
/// Returns `None` for anything not (yet) understood; callers may log it.
pub fn decode(msg: &Message<'_>) -> Option<ControlEvent> {
    match *msg {
        Message::Channel {
            status,
            data1,
            data2,
        } => {
            let kind = status & 0xF0;
            let ch = status & 0x0F;
            if ch == PAD_CHANNEL {
                let pad = pad_index(data1)?;
                return match kind {
                    0x90 if data2 > 0 => Some(ControlEvent::PadDown {
                        pad,
                        velocity: data2,
                    }),
                    0x90 | 0x80 => Some(ControlEvent::PadUp { pad }),
                    0xA0 => Some(ControlEvent::PadPressure {
                        pad,
                        pressure: data2,
                    }),
                    _ => None,
                };
            }
            if ch != CONTROL_CHANNEL {
                return None;
            }
            match kind {
                0xB0 => {
                    if data1 == DATA_WHEEL_CC {
                        Some(ControlEvent::DataWheel {
                            delta: relative_delta(data2),
                        })
                    } else {
                        let knob = index_of(&QLINK_TURN_CC, data1)?;
                        Some(ControlEvent::QLinkTurn {
                            knob,
                            delta: relative_delta(data2),
                        })
                    }
                }
                0x90 | 0x80 => {
                    let pressed = kind == 0x90 && data2 > 0;
                    if let Some(knob) = index_of(&QLINK_TOUCH_NOTES, data1) {
                        Some(ControlEvent::QLinkTouch {
                            knob,
                            touched: pressed,
                        })
                    } else {
                        Some(ControlEvent::Button {
                            note: data1,
                            pressed,
                        })
                    }
                }
                _ => None,
            }
        }
        // Keep-alive reply: F0 47 7F 46 53 00 05 t0 t1 t2 t3 01 F7
        Message::SysEx {
            bytes:
                [0xF0, AKAI_SYSEX_ID, 0x7F, DEVICE_ID, 0x53, 0x00, 0x05, t0, t1, t2, t3, 0x01, 0xF7],
            truncated: false,
        } => Some(ControlEvent::KeepAliveReply {
            token: [*t0, *t1, *t2, *t3],
        }),
        _ => None,
    }
}

/// Button LED: CC on channel 1 with the button's number. Observed values
/// 0..=4 (0 = off; 1 and 3 most common, read as dim/bright). Send on
/// [`Port::Private`].
pub fn button_led(button: u8, value: u8) -> (Port, [u8; 3]) {
    (
        Port::Private,
        [0xB0 | CONTROL_CHANNEL, button & 0x7F, value & 0x7F],
    )
}

/// Pad RGB color, 7-bit components. `pad` is 0..=15 in [`PAD_NOTES`] order.
/// Send on [`Port::Public`]. Returns `None` for an invalid pad index.
pub fn pad_rgb(pad: u8, r: u8, g: u8, b: u8) -> Option<(Port, [u8; 12])> {
    if pad >= 16 {
        return None;
    }
    Some((
        Port::Public,
        [
            0xF0,
            AKAI_SYSEX_ID,
            0x7F,
            DEVICE_ID,
            0x65,
            0x00,
            0x04,
            pad,
            r & 0x7F,
            g & 0x7F,
            b & 0x7F,
            0xF7,
        ],
    ))
}

/// The keep-alive the factory app sends about every 5 s on [`Port::Public`].
/// The controller answers with [`ControlEvent::KeepAliveReply`]. Whether the
/// controller requires it is unknown, so MPFree sends it until proven
/// otherwise. Token bytes must be 7-bit.
pub fn keepalive_ping(token: [u8; 4]) -> (Port, [u8; 12]) {
    let [a, b, c, d] = token.map(|t| t & 0x7F);
    (
        Port::Public,
        [
            0xF0,
            AKAI_SYSEX_ID,
            0x00,
            DEVICE_ID,
            0x52,
            0x00,
            0x04,
            a,
            b,
            c,
            d,
            0xF7,
        ],
    )
}

/// The token observed during the whole 2026-11-09 capture.
pub const OBSERVED_KEEPALIVE_TOKEN: [u8; 4] = [0x7D, 0x6D, 0x57, 0x35];
