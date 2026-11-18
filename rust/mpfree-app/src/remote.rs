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
//! The Clips grid works the same way (sequencer S5, "being driven";
//! `Owner::Clips`, its ids in [`mpfree_play::mpfree_param::clips`]): a
//! learned button launches a clip or a scene, stops a track or all of
//! them, goes back to the song or arms a track, as a tap on the grid does
//! ([`Remote::clips_write`] says which, the app finds the tap with
//! `mpfree_time_ui::view::clips_press`); a learned key, knob or fader
//! mutes or solos a track, on the launch line in Perform as TRACK MUTE's
//! pads do.
//!
//! The looper works the same way too (the looper redesign's footswitch,
//! step 5; `Owner::Looper`, its ids in
//! [`mpfree_play::mpfree_param::looper`]): a learned footswitch, button or
//! key is a loop's key as its pad is (pressed, it takes the loop's next
//! step; held a second, it clears the loop), the picked loop's key, Next
//! loop, Undo, Redo, Capture or Clear ([`Remote::looper_write`] says
//! which); a learned key, switch or fader mutes a loop or lets it play.
//! The MPC One has no footswitch jack: a footswitch comes in through a
//! MIDI controller or pedal.
//!
//! The MPC's own buttons never come here, and nothing here sends MIDI.

use mpfree_node::{Curve, ParamInfo, Unit};
use mpfree_play::mpfree_param::clips::{self, Clips};
use mpfree_play::mpfree_param::looper::{self, Looper, Which};
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
    /// The tracks muted, bit `t` for track `t`, as they will be once the
    /// mute queue lands (`TimeHost::mute_bits`).
    pub muted: u128,
    /// The tracks soloed, the same way.
    pub soloed: u128,
    /// The loop picked (0 is Loop 1).
    pub picked: usize,
    /// The loops muted, bit `n` for loop `n` (`LoopHost::mute_bits`).
    pub loops_muted: u8,
}

/// A track's mute as MIDI Learn reads and sets it: on while muted.
pub const MUTE: ParamInfo = ParamInfo {
    id: 0,
    name: "Mute",
    group: "Clips",
    min: 0.0,
    max: 1.0,
    default: 0.0,
    unit: Unit::None,
    curve: Curve::Toggle,
    readonly: false,
};

/// A track's solo: on while soloed.
pub const SOLO: ParamInfo = ParamInfo {
    name: "Solo",
    ..MUTE
};

/// A Clips grid parameter described with its value now: a track's mute or
/// solo. None for the presses, which have no value to read.
pub fn describe_clips(id: u16, now: Now) -> Option<(ParamInfo, f32)> {
    let on = |bits: u128, t: u8| if (bits >> t) & 1 == 1 { 1.0 } else { 0.0 };
    match clips::of(id)? {
        Clips::Mute(t) => Some((ParamInfo { id, ..MUTE }, on(now.muted, t))),
        Clips::Solo(t) => Some((ParamInfo { id, ..SOLO }, on(now.soloed, t))),
        _ => None,
    }
}

/// A loop's mute as MIDI Learn reads and sets it: on while muted.
pub const LOOP_MUTE: ParamInfo = ParamInfo {
    group: "Looper",
    ..MUTE
};

/// The loop a key or a mute is for, with `picked` picked.
fn loop_of(which: Which, picked: usize) -> usize {
    match which {
        Which::Picked => picked,
        Which::Loop(n) => usize::from(n),
    }
}

/// A looper parameter described with its value now: a loop's mute. None
/// for the presses, which have no value to read.
pub fn describe_looper(id: u16, now: Now) -> Option<(ParamInfo, f32)> {
    let Looper::Mute(w) = looper::of(id)? else {
        return None;
    };
    let n = loop_of(w, now.picked);
    let on = n < 8 && (now.loops_muted >> n) & 1 == 1;
    Some((ParamInfo { id, ..LOOP_MUTE }, if on { 1.0 } else { 0.0 }))
}

/// What a learned control on the looper does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LooperAct {
    Nothing,
    /// Loop `n`'s key went down: what its pad does (its next step).
    Key(usize),
    /// Loop `n` muted (on) or let play.
    Mute {
        n: usize,
        on: bool,
    },
    /// Pressed: Next loop, Undo, Redo, Capture or Clear.
    Press(Looper),
}

/// How long a mute asked of a loop counts as where the loop stands, while
/// the looper takes it (a period or two): a fader moved past the middle
/// and back in that time lets the loop play again.
const MUTE_ASK_S: f64 = 0.25;

/// A learned loop key held down.
#[derive(Clone, Copy, Debug, PartialEq)]
struct KeyHeld {
    /// The loop it is the key of (the picked loop's key: the loop picked
    /// when it went down).
    n: usize,
    /// When it went down, in seconds.
    at: f64,
    /// Its hold has cleared the loop.
    cleared: bool,
}

/// What a learned control on the Clips grid does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipsAct {
    Nothing,
    /// Pressed: do what a tap on the grid does now.
    Press(Clips),
    /// Mute (or with `solo`, solo) track `track`, or let it play.
    Mute {
        track: usize,
        solo: bool,
        on: bool,
    },
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
    /// The Clips grid's presses held down now, by id.
    clips_held: Vec<u16>,
    /// The looper's keys held down: Loops 1 to 8's, then the picked
    /// loop's.
    loop_keys: [Option<KeyHeld>; 9],
    /// The looper's presses held down now, bit `id` for Next loop to
    /// Clear.
    looper_held: u8,
    /// The mute last asked of each loop and when, until the looper has
    /// it ([`MUTE_ASK_S`]).
    mute_asked: [Option<(bool, f64)>; 8],
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

    /// A learned control set Clips grid parameter `id` to `value` (0 to
    /// 1). A press goes past the middle once; a mute or solo is on from
    /// the middle up, and does something only when that isn't where the
    /// track stands (or will, once the mute queue lands).
    pub fn clips_write(&mut self, id: u16, value: f32, now: Now) -> ClipsAct {
        let Some(c) = clips::of(id) else {
            return ClipsAct::Nothing;
        };
        let on = value >= 0.5;
        if let Clips::Mute(t) | Clips::Solo(t) = c {
            let solo = matches!(c, Clips::Solo(_));
            let bits = if solo { now.soloed } else { now.muted };
            if ((bits >> t) & 1 == 1) == on {
                return ClipsAct::Nothing;
            }
            return ClipsAct::Mute {
                track: usize::from(t),
                solo,
                on,
            };
        }
        let held = self.clips_held.contains(&id);
        match (on, held) {
            (true, false) => {
                self.clips_held.push(id);
                ClipsAct::Press(c)
            }
            (false, true) => {
                self.clips_held.retain(|&h| h != id);
                ClipsAct::Nothing
            }
            _ => ClipsAct::Nothing,
        }
    }
}

impl Remote {
    /// A learned control set looper parameter `id` to `value` (0 to 1) at
    /// `t` seconds. A key goes down past the middle and lets go under it,
    /// as a pad does (its hold clears the loop: [`Remote::loop_holds`]);
    /// Next loop, Undo, Redo, Capture and Clear press once each time they
    /// go down; a mute is on from the middle up, and does something only
    /// when that isn't where the loop stands (or will, once the looper has
    /// the last one asked).
    pub fn looper_write(&mut self, id: u16, value: f32, now: Now, t: f64) -> LooperAct {
        let Some(l) = looper::of(id) else {
            return LooperAct::Nothing;
        };
        let on = value >= 0.5;
        match l {
            Looper::Key(w) => {
                let slot = match w {
                    Which::Loop(n) => usize::from(n),
                    Which::Picked => 8,
                };
                let Some(held) = self.loop_keys.get_mut(slot) else {
                    return LooperAct::Nothing;
                };
                match (on, held.is_some()) {
                    (true, false) => {
                        let n = loop_of(w, now.picked);
                        *held = Some(KeyHeld {
                            n,
                            at: t,
                            cleared: false,
                        });
                        LooperAct::Key(n)
                    }
                    (false, true) => {
                        *held = None;
                        LooperAct::Nothing
                    }
                    _ => LooperAct::Nothing,
                }
            }
            Looper::Mute(w) => {
                let n = loop_of(w, now.picked);
                let Some(asked) = self.mute_asked.get_mut(n) else {
                    return LooperAct::Nothing;
                };
                let is = (now.loops_muted >> n) & 1 == 1;
                let will = match *asked {
                    Some((a, at)) if a != is && t - at < MUTE_ASK_S => a,
                    _ => is,
                };
                if will == on {
                    return LooperAct::Nothing;
                }
                *asked = Some((on, t));
                LooperAct::Mute { n, on }
            }
            press => {
                let bit = 1u8 << (id & 7);
                let held = self.looper_held & bit != 0;
                match (on, held) {
                    (true, false) => {
                        self.looper_held |= bit;
                        LooperAct::Press(press)
                    }
                    (false, true) => {
                        self.looper_held &= !bit;
                        LooperAct::Nothing
                    }
                    _ => LooperAct::Nothing,
                }
            }
        }
    }

    /// The loops whose learned key has been held for `hold` seconds by
    /// `t`, once each hold: each is cleared, as a loop key's pad held
    /// clears it.
    pub fn loop_holds(&mut self, t: f64, hold: f64) -> Vec<usize> {
        let mut out = Vec::new();
        for k in self.loop_keys.iter_mut().flatten() {
            if !k.cleared && t - k.at >= hold {
                k.cleared = true;
                if !out.contains(&k.n) {
                    out.push(k.n);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOPPED: Now = Now {
        rolling: false,
        bpm: 120.0,
        muted: 0,
        soloed: 0,
        picked: 0,
        loops_muted: 0,
    };
    const PLAYING_NOW: Now = Now {
        rolling: true,
        ..STOPPED
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

    #[test]
    fn a_clips_button_presses_once_each_time_it_goes_down() {
        let mut r = Remote::new();
        let scene = clips::scene(2);
        assert_eq!(
            r.clips_write(scene, 1.0, STOPPED),
            ClipsAct::Press(Clips::Scene(2))
        );
        assert_eq!(r.clips_write(scene, 0.7, STOPPED), ClipsAct::Nothing);
        assert_eq!(r.clips_write(scene, 0.0, STOPPED), ClipsAct::Nothing);
        assert_eq!(
            r.clips_write(scene, 1.0, STOPPED),
            ClipsAct::Press(Clips::Scene(2))
        );
        // Each control keeps its own.
        let clip = clips::clip(3, 1);
        assert_eq!(
            r.clips_write(clip, 1.0, STOPPED),
            ClipsAct::Press(Clips::Clip(3, 1))
        );
        assert_eq!(
            r.clips_write(clips::STOP_ALL, 1.0, STOPPED),
            ClipsAct::Press(Clips::StopAll)
        );
        assert_eq!(
            r.clips_write(9, 1.0, STOPPED),
            ClipsAct::Nothing,
            "no such id"
        );
        assert_eq!(describe_clips(scene, STOPPED), None, "a press has no value");
    }

    #[test]
    fn a_mute_is_set_only_where_the_track_isnt() {
        let mut r = Remote::new();
        let now = Now {
            muted: 1 << 5,
            soloed: 1 << 127,
            ..PLAYING_NOW
        };
        assert_eq!(r.clips_write(clips::mute(5), 1.0, now), ClipsAct::Nothing);
        assert_eq!(
            r.clips_write(clips::mute(5), 0.0, now),
            ClipsAct::Mute {
                track: 5,
                solo: false,
                on: false
            }
        );
        assert_eq!(
            r.clips_write(clips::mute(4), 0.9, now),
            ClipsAct::Mute {
                track: 4,
                solo: false,
                on: true
            }
        );
        assert_eq!(r.clips_write(clips::solo(127), 1.0, now), ClipsAct::Nothing);
        assert_eq!(
            r.clips_write(clips::solo(127), 0.0, now),
            ClipsAct::Mute {
                track: 127,
                solo: true,
                on: false
            }
        );
        let (info, v) = describe_clips(clips::mute(5), now).unwrap();
        assert_eq!((info.name, info.id, v), ("Mute", clips::mute(5), 1.0));
        assert_eq!(describe_clips(clips::mute(4), now).unwrap().1, 0.0);
        assert_eq!(describe_clips(clips::solo(127), now).unwrap().1, 1.0);
    }

    /// A learned loop key goes down and lets go as its pad does: the
    /// picked loop's key is the loop picked when it went down, and a key
    /// held for the hold clears its loop once.
    #[test]
    fn a_loop_key_presses_on_its_way_down_and_its_hold_clears_once() {
        let mut r = Remote::new();
        let now = Now {
            picked: 4,
            ..STOPPED
        };
        let k3 = looper::key(2);
        assert_eq!(r.looper_write(k3, 1.0, now, 0.0), LooperAct::Key(2));
        assert_eq!(r.looper_write(k3, 0.9, now, 0.1), LooperAct::Nothing);
        assert_eq!(r.loop_holds(0.5, 1.0), Vec::<usize>::new());
        assert_eq!(r.loop_holds(1.0, 1.0), [2]);
        assert_eq!(r.loop_holds(2.0, 1.0), Vec::<usize>::new(), "once");
        assert_eq!(r.looper_write(k3, 0.0, now, 2.1), LooperAct::Nothing);
        // A short press clears nothing.
        assert_eq!(r.looper_write(k3, 1.0, now, 3.0), LooperAct::Key(2));
        assert_eq!(r.looper_write(k3, 0.0, now, 3.2), LooperAct::Nothing);
        assert!(r.loop_holds(9.0, 1.0).is_empty());
        // The picked loop's key: Loop 5 when it went down, and its hold
        // clears Loop 5 even if another is picked by then.
        let p = looper::PICKED_KEY;
        assert_eq!(r.looper_write(p, 1.0, now, 10.0), LooperAct::Key(4));
        assert_eq!(
            r.looper_write(looper::NEXT_LOOP, 1.0, now, 10.2),
            LooperAct::Press(Looper::NextLoop)
        );
        assert_eq!(r.loop_holds(11.0, 1.0), [4]);
        assert_eq!(r.looper_write(p, 0.0, now, 11.5), LooperAct::Nothing);
        assert_eq!(describe_looper(p, now), None, "a key has no value");
    }

    #[test]
    fn the_loopers_presses_go_once_each_time_they_go_down() {
        let mut r = Remote::new();
        for (id, l) in [
            (looper::NEXT_LOOP, Looper::NextLoop),
            (looper::UNDO, Looper::Undo),
            (looper::REDO, Looper::Redo),
            (looper::CAPTURE, Looper::Capture),
            (looper::CLEAR, Looper::Clear),
        ] {
            assert_eq!(r.looper_write(id, 1.0, STOPPED, 0.0), LooperAct::Press(l));
            assert_eq!(r.looper_write(id, 0.8, STOPPED, 0.0), LooperAct::Nothing);
            assert_eq!(r.looper_write(id, 0.0, STOPPED, 0.0), LooperAct::Nothing);
            assert_eq!(r.looper_write(id, 1.0, STOPPED, 0.0), LooperAct::Press(l));
            assert_eq!(describe_looper(id, STOPPED), None);
        }
        assert_eq!(r.looper_write(7, 1.0, STOPPED, 0.0), LooperAct::Nothing);
    }

    /// A loop's mute is on from the middle up and acts only where the
    /// loop isn't, counting a mute asked a moment ago that the looper
    /// hasn't taken yet.
    #[test]
    fn a_loop_mute_is_set_only_where_the_loop_isnt() {
        let mut r = Remote::new();
        let now = Now {
            picked: 1,
            loops_muted: 1 << 3,
            ..STOPPED
        };
        let m4 = looper::mute(3);
        assert_eq!(r.looper_write(m4, 1.0, now, 0.0), LooperAct::Nothing);
        assert_eq!(
            r.looper_write(m4, 0.2, now, 0.0),
            LooperAct::Mute { n: 3, on: false }
        );
        // Back past the middle before the looper took it: muted again.
        assert_eq!(r.looper_write(m4, 0.1, now, 0.01), LooperAct::Nothing);
        assert_eq!(
            r.looper_write(m4, 0.7, now, 0.02),
            LooperAct::Mute { n: 3, on: true }
        );
        // The picked loop's: Loop 2.
        assert_eq!(
            r.looper_write(looper::PICKED_MUTE, 1.0, now, 1.0),
            LooperAct::Mute { n: 1, on: true }
        );
        let (info, v) = describe_looper(m4, now).unwrap();
        assert_eq!(
            (info.name, info.group, info.id, v),
            ("Mute", "Looper", m4, 1.0)
        );
        assert_eq!(describe_looper(looper::PICKED_MUTE, now).unwrap().1, 0.0);
        assert_eq!(describe_looper(looper::mute(0), now).unwrap().1, 0.0);
        // Long after a mute the looper refused, the loop's own state wins.
        let mut r = Remote::new();
        assert_eq!(
            r.looper_write(looper::mute(0), 1.0, STOPPED, 0.0),
            LooperAct::Mute { n: 0, on: true }
        );
        assert_eq!(
            r.looper_write(looper::mute(0), 1.0, STOPPED, 1.0),
            LooperAct::Mute { n: 0, on: true }
        );
    }
}
