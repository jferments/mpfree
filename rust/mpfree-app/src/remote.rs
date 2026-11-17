//! Remote control by MIDI Learn (phase 2 of the MIDI plan, board 8): a
//! control learned on the transport (`Owner::Transport`, its ids in
//! [`mpfree_param::transport`]) presses the MPC's PLAY, STOP, PLAY START,
//! REC, OVERDUB or TAP TEMPO, plays and stops on one button, or sets the
//! tempo.
//!
//! A press goes through `App::remote_press`, so a controller's button does
//! exactly what the MPC's own does on the page showing (REC on the Play
//! page records the loop, PLAY while playing is FILL). This module only
//! turns a learned control's values into presses and a tempo: a press when
//! the value goes past the middle, a let-go when it comes back under.
//!
//! The MPC's own buttons never come here, and nothing here sends MIDI.

use mpfree_node::{Curve, ParamInfo, Unit};
use mpfree_play::mpfree_param::transport;

/// The tempo a learned knob sets, over the app's range (`App::set_tempo`).
/// Shown and read in whole BPM; set to a tenth.
pub const TEMPO: ParamInfo = ParamInfo {
    id: transport::TEMPO,
    name: "Tempo",
    group: "Transport",
    min: 30.0,
    max: 300.0,
    default: 120.0,
    unit: Unit::None,
    curve: Curve::Stepped,
    readonly: false,
};

/// PLAY and STOP on one control: on while the transport plays.
pub const PLAYING: ParamInfo = ParamInfo {
    id: transport::PLAYING,
    name: "PLAY and STOP",
    group: "Transport",
    min: 0.0,
    max: 1.0,
    default: 0.0,
    unit: Unit::None,
    curve: Curve::Toggle,
    readonly: false,
};

/// Where the transport stands, for Learn to read and to act on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Now {
    pub rolling: bool,
    pub bpm: f64,
}

/// A transport parameter described with its value now; None for the
/// presses, which have no value to read.
pub fn describe(id: u16, now: Now) -> Option<(ParamInfo, f32)> {
    match id {
        transport::TEMPO => Some((TEMPO, now.bpm as f32)),
        transport::PLAYING => Some((PLAYING, if now.rolling { 1.0 } else { 0.0 })),
        _ => None,
    }
}

/// What a learned control's value does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    Nothing,
    /// Press (true) or let go of one of the MPC's buttons, by its
    /// transport id.
    Press(u16, bool),
    /// Press and let go at once: PLAY and STOP's PLAY or STOP.
    Tap(u16),
    /// Set the tempo, in BPM.
    Tempo(f64),
}

/// The presses learned controls hold down now, so a value that stays on
/// one side of the middle (a knob turned past it) presses once.
#[derive(Debug, Default)]
pub struct Remote {
    held: [bool; transport::ALL.len()],
}

impl Remote {
    pub fn new() -> Remote {
        Remote::default()
    }

    /// A learned control set transport parameter `id` to `value` (0 to 1
    /// over its range).
    pub fn write(&mut self, id: u16, value: f32, now: Now) -> Act {
        match id {
            transport::TEMPO => {
                let bpm = f64::from(TEMPO.from_unit(value));
                Act::Tempo(((bpm * 10.0).round() / 10.0).clamp(30.0, 300.0))
            }
            transport::PLAYING => match (value >= 0.5, now.rolling) {
                (true, false) => Act::Tap(transport::PLAY),
                (false, true) => Act::Tap(transport::STOP),
                _ => Act::Nothing,
            },
            id if transport::is_press(id) => {
                let Some(held) = self.held.get_mut(usize::from(id)) else {
                    return Act::Nothing;
                };
                let down = value >= 0.5;
                if down == *held {
                    return Act::Nothing;
                }
                *held = down;
                Act::Press(id, down)
            }
            _ => Act::Nothing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOPPED: Now = Now {
        rolling: false,
        bpm: 120.0,
    };
    const PLAYING_NOW: Now = Now {
        rolling: true,
        bpm: 120.0,
    };

    #[test]
    fn a_button_presses_once_and_lets_go_once() {
        let mut r = Remote::new();
        assert_eq!(
            r.write(transport::REC, 1.0, STOPPED),
            Act::Press(transport::REC, true)
        );
        // A knob turned on past the middle: still one press.
        assert_eq!(r.write(transport::REC, 0.8, STOPPED), Act::Nothing);
        assert_eq!(
            r.write(transport::REC, 0.0, STOPPED),
            Act::Press(transport::REC, false)
        );
        assert_eq!(r.write(transport::REC, 0.2, STOPPED), Act::Nothing);
        // Each button keeps its own.
        assert_eq!(
            r.write(transport::TAP_TEMPO, 1.0, STOPPED),
            Act::Press(transport::TAP_TEMPO, true)
        );
        assert_eq!(
            r.write(transport::PLAY_START, 1.0, STOPPED),
            Act::Press(transport::PLAY_START, true)
        );
    }

    #[test]
    fn play_and_stop_plays_when_stopped_and_stops_when_playing() {
        let mut r = Remote::new();
        let p = transport::PLAYING;
        assert_eq!(r.write(p, 1.0, STOPPED), Act::Tap(transport::PLAY));
        assert_eq!(r.write(p, 1.0, PLAYING_NOW), Act::Nothing, "plays already");
        assert_eq!(r.write(p, 0.0, PLAYING_NOW), Act::Tap(transport::STOP));
        assert_eq!(r.write(p, 0.0, STOPPED), Act::Nothing, "stopped already");
        assert_eq!(describe(p, PLAYING_NOW).map(|d| d.1), Some(1.0));
        assert_eq!(describe(p, STOPPED).map(|d| d.1), Some(0.0));
    }

    #[test]
    fn a_knob_sets_the_tempo_over_the_apps_range() {
        let mut r = Remote::new();
        let t = transport::TEMPO;
        assert_eq!(r.write(t, 0.0, STOPPED), Act::Tempo(30.0));
        assert_eq!(r.write(t, 1.0, STOPPED), Act::Tempo(300.0));
        let Act::Tempo(mid) = r.write(t, 90.0 / 270.0, STOPPED) else {
            panic!("a tempo");
        };
        assert!((mid - 120.0).abs() < 0.05, "{mid}");
        let (info, v) = describe(
            t,
            Now {
                bpm: 98.5,
                ..STOPPED
            },
        )
        .unwrap();
        assert_eq!(v, 98.5);
        // Read to the nearest BPM, well inside a picked-up fader's window.
        assert!((info.to_unit(v) - 68.5 / 270.0).abs() <= 0.5 / 270.0 + 1e-6);
        assert_eq!(mpfree_mod::text::value_text(&info, 120.0), "120");
    }

    #[test]
    fn presses_have_no_value_to_read() {
        for id in [
            transport::PLAY,
            transport::STOP,
            transport::PLAY_START,
            transport::REC,
            transport::OVERDUB,
            transport::TAP_TEMPO,
        ] {
            assert!(transport::is_press(id));
            assert_eq!(describe(id, STOPPED), None);
        }
        assert_eq!(Remote::new().write(99, 1.0, STOPPED), Act::Nothing);
    }
}
