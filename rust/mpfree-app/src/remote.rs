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
//! And the Perform pads (the Play, Pads and Sequencer redesign's board 7,
//! Learn a key; `Owner::Perform`, its ids in
//! [`mpfree_play::mpfree_param::perform`]): a learned key, pad or
//! footswitch plays one Perform pad as a hit on it does, whatever the
//! MPC's pads show, and lets go of what it pressed ([`PerformKeys`]).
//!
//! Play's own controls work this way too (the Learn section's Play card,
//! Jay's pick on 18 Nov 2026; `Owner::Play`, its ids in
//! [`mpfree_play::mpfree_param::play`]): a learned knob turns the Swing or
//! the Filter (track 1's, which Play calls Drums) as Play's Q-Links do, or
//! moves the XY pad across or up as a finger does ([`PlayMoves`] says how,
//! once a period). The XY pad's effect plays while the pad is away from
//! where it rests: across in the middle, up at the bottom. Play's own
//! touches stay as they are.
//!
//! A set's controls (the Learn section's Show card, live shows B1;
//! `Owner::Show`, its ids in [`mpfree_play::mpfree_param::show`]) are
//! presses: [`Remote::show_write`] gives each once as it goes down, for the
//! set player.
//!
//! The MPC's own buttons never come here, and nothing here sends MIDI.

use mpfree_node::{Curve, ParamInfo, Unit};
use mpfree_play::mpfree_param::clips::{self, Clips};
use mpfree_play::mpfree_param::looper::{self, Looper, Which};
use mpfree_play::mpfree_param::{perform, play, show, transport};

use crate::perform_pads::PadAction;

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
    /// Play's Swing, 50 to 75 %.
    pub swing: u8,
    /// The Drums filter, -100 to 100 (`PlayLevels::drums_filter`).
    pub drums_filter: f32,
    /// Where the XY pad stands, 0 to 1 across and up
    /// (`MixHost::xy_at`).
    pub xy: (f32, f32),
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
    /// Pressed: Next loop, Undo, Redo, Capture, Clear, Snap or Use my
    /// tap.
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

/// Play's Swing as a learned knob turns it, in whole percents.
pub const SWING: ParamInfo = ParamInfo {
    id: play::SWING,
    name: "Swing",
    group: "Play",
    min: 50.0,
    max: 75.0,
    default: 50.0,
    unit: Unit::Percent,
    curve: Curve::Stepped,
    readonly: false,
};

/// Play's Filter, track 1's DJ filter: -100 (low-pass) to 100
/// (high-pass), 0 off.
pub const FILTER: ParamInfo = ParamInfo {
    id: play::FILTER,
    name: "Filter",
    group: "Play",
    min: -100.0,
    max: 100.0,
    default: 0.0,
    unit: Unit::Percent,
    curve: Curve::Linear,
    readonly: false,
};

/// The XY pad across, 0 (left) to 1 (right).
pub const XY_ACROSS: ParamInfo = ParamInfo {
    id: play::XY_ACROSS,
    name: "XY across",
    group: "Play",
    min: 0.0,
    max: 1.0,
    default: 0.5,
    unit: Unit::None,
    curve: Curve::Linear,
    readonly: false,
};

/// The XY pad up, 0 (bottom) to 1 (top).
pub const XY_UP: ParamInfo = ParamInfo {
    id: play::XY_UP,
    name: "XY up",
    default: 0.0,
    ..XY_ACROSS
};

/// One of Play's controls described with its value now.
pub fn describe_play(id: u16, now: Now) -> Option<(ParamInfo, f32)> {
    match id {
        play::SWING => Some((SWING, f32::from(now.swing))),
        play::FILTER => Some((FILTER, now.drums_filter)),
        play::XY_ACROSS => Some((XY_ACROSS, now.xy.0)),
        play::XY_UP => Some((XY_UP, now.xy.1)),
        _ => None,
    }
}

/// How near where it rests (across in the middle, up at the bottom) the
/// XY pad counts as resting: a 7-bit knob's step, so a knob's middle (63
/// or 64) is the middle.
pub const XY_REST: f32 = 1.0 / 127.0;
/// A Drums filter this near 0 is off, so a knob's middle is off.
const FILTER_OFF: f32 = 1.0;

/// Play's controls as learned controls set them this period (the last
/// value each was given, 0 to 1 over its range), done once a period
/// ([`PlayMoves::take`]), so the XY pad's two ways move together.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayMoves([Option<f32>; play::KNOBS]);

/// What learned controls do to Play this period.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayActs {
    /// Set the swing, 50 to 75 %.
    pub swing: Option<u8>,
    /// Set the Drums filter, -100 to 100.
    pub filter: Option<f32>,
    /// Move the XY pad: across, up, and whether it is held away from
    /// where it rests (as a finger down: its effect plays).
    pub xy: Option<(f32, f32, bool)>,
}

impl PlayMoves {
    /// A learned control set Play's control `id` to `value` (0 to 1).
    pub fn set(&mut self, id: u16, value: f32) {
        if let Some(v) = self.0.get_mut(usize::from(id)) {
            *v = Some(value.clamp(0.0, 1.0));
        }
    }

    /// What this period's moves do, with Play standing at `now`; the moves
    /// are spent. The XY pad's other way stays where it is.
    pub fn take(&mut self, now: Now) -> PlayActs {
        let [swing, filter, across, up] = std::mem::take(&mut self.0);
        let swing = swing
            .map(|v| SWING.from_unit(v).round().clamp(50.0, 75.0) as u8)
            .filter(|&a| a != now.swing);
        let filter = filter
            .map(|v| {
                let p = FILTER.from_unit(v);
                if p.abs() < FILTER_OFF {
                    0.0
                } else {
                    p
                }
            })
            .filter(|&p| p != now.drums_filter);
        let xy = (across.is_some() || up.is_some()).then(|| {
            let mut x = across.unwrap_or(now.xy.0);
            let mut y = up.unwrap_or(now.xy.1);
            if (x - 0.5).abs() < XY_REST {
                x = 0.5;
            }
            if y < XY_REST {
                y = 0.0;
            }
            (x, y, x != 0.5 || y != 0.0)
        });
        PlayActs { swing, filter, xy }
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
    /// Use my tap.
    looper_held: u16,
    /// The mute last asked of each loop and when, until the looper has
    /// it ([`MUTE_ASK_S`]).
    mute_asked: [Option<(bool, f64)>; 8],
    /// A set's controls held down now, bit `id` (the Show card's).
    show_held: u8,
    /// Next and Previous patch held down now, bit `id - NEXT_PATCH`.
    patch_held: u8,
}

/// What a set's learned control says while no set is open.
pub const NO_SET: &str = "No set is open: the Show controls work while a set plays";

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
    /// Next loop, Undo, Redo, Capture, Clear, Snap and Use my tap press
    /// once each time they go down; a mute is on from the middle up, and does something only
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
                let bit = 1u16 << (id & 15);
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

    /// A learned control set a set's control `id` ([`show`]) to `value`:
    /// the press, as it goes down (from the middle up), once until it
    /// comes up again. The set player cleans a footswitch's bounce
    /// (`show::DEBOUNCE_MS`) and ignores presses while a song changes.
    pub fn show_write(&mut self, id: u16, value: f32) -> Option<u16> {
        show::name(id)?;
        let bit = 1u8 << (id & 7);
        let held = self.show_held & bit != 0;
        match (value >= 0.5, held) {
            (true, false) => {
                self.show_held |= bit;
                Some(id)
            }
            (false, true) => {
                self.show_held &= !bit;
                None
            }
            _ => None,
        }
    }

    /// A learned control set Next or Previous patch (`id`, [`play`]) to
    /// `value`: its step (1 or -1) and whether it waits for your hands, as
    /// it goes down (from the middle up), once until it comes up again.
    pub fn patch_write(&mut self, id: u16, value: f32) -> Option<(i32, bool)> {
        let step = play::patch_step(id)?;
        let bit = 1u8 << (id - play::NEXT_PATCH);
        let held = self.patch_held & bit != 0;
        match (value >= 0.5, held) {
            (true, false) => {
                self.patch_held |= bit;
                Some(step)
            }
            (false, true) => {
                self.patch_held &= !bit;
                None
            }
            _ => None,
        }
    }

    /// A controller went away (`MusicIn::take_unplugged`): its controls
    /// held down will never come up. The looper's keys and presses let go
    /// without acting: a key held then clears nothing, and its next press
    /// is a press again; the looper itself is left as it is. The Clips
    /// grid's presses let go too (they act as they go down). The
    /// transport's presses held down come back, each once, for the caller
    /// to let their buttons up ([`Act::Press`] with `down` false), so a
    /// learned REC isn't left held.
    pub fn let_go(&mut self) -> impl Iterator<Item = u16> {
        self.loop_keys = [None; 9];
        self.looper_held = 0;
        self.show_held = 0;
        self.patch_held = 0;
        self.clips_held.clear();
        std::mem::take(&mut self.held)
            .into_iter()
            .zip(0u16..)
            .filter_map(|(down, id)| down.then_some(id))
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

/// What a key learned on a Perform pad does (the Play, Pads and
/// Sequencer redesign's board 7, Learn a key; `Owner::Perform`, its ids
/// in [`mpfree_play::mpfree_param::perform`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PerformAct {
    Nothing,
    /// Pressed: page `page`'s pad `pad` (0 is page A, pad 1) plays
    /// `action`, as a hit on the pad does.
    Down {
        page: u8,
        pad: u8,
        action: PadAction,
    },
    /// Let go: what it pressed lets go, as the pad's let-go does.
    Up(PadAction),
}

/// A Perform pad a learned key holds down: its id, when it went down, in
/// seconds, whether its hold has cleared its loop, and what it pressed.
type PerformHeld = (u16, f64, bool, PadAction);

/// The Perform pads learned keys hold down. Each lets go of what it
/// pressed (the pad changed or the page turned since changes nothing), a
/// loop key held a second clears its loop as the pad does
/// ([`PerformKeys::holds`]), and an unplugged controller lets go of all
/// of them ([`PerformKeys::let_go`]), so no throw stays held.
#[derive(Debug, Default)]
pub struct PerformKeys {
    held: Vec<PerformHeld>,
}

impl PerformKeys {
    /// A learned key set Perform parameter `id` to `value` (0 to 1) at
    /// `t` seconds: past the middle it presses the pad (`action_of` says
    /// what the pad does now), under it lets go. A value that stays on
    /// one side presses or lets go once.
    pub fn write(
        &mut self,
        id: u16,
        value: f32,
        t: f64,
        action_of: impl Fn(usize, usize) -> PadAction,
    ) -> PerformAct {
        let Some((page, pad)) = perform::of(id) else {
            return PerformAct::Nothing;
        };
        let at = self.held.iter().position(|h| h.0 == id);
        match (value >= 0.5, at) {
            (true, None) => {
                let action = action_of(usize::from(page), usize::from(pad));
                self.held.push((id, t, false, action));
                PerformAct::Down { page, pad, action }
            }
            (false, Some(i)) => PerformAct::Up(self.held.swap_remove(i).3),
            _ => PerformAct::Nothing,
        }
    }

    /// The loop keys learned keys have held for `hold` seconds by `t`,
    /// once each hold: each clears its loop, as the pad held does.
    pub fn holds(&mut self, t: f64, hold: f64) -> Vec<PadAction> {
        let mut out = Vec::new();
        for h in &mut self.held {
            if !h.2 && matches!(h.3, PadAction::LoopKey(_)) && t - h.1 >= hold {
                h.2 = true;
                out.push(h.3);
            }
        }
        out
    }

    /// A controller went away (`MusicIn::take_unplugged`): what its keys
    /// held lets go now, as their let-go would (a throw stops, a macro
    /// goes back), since they will never come up.
    pub fn let_go(&mut self) -> Vec<PadAction> {
        self.held.drain(..).map(|h| h.3).collect()
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
        swing: 50,
        drums_filter: 0.0,
        xy: (0.5, 0.0),
    };
    const PLAYING_NOW: Now = Now {
        rolling: true,
        ..STOPPED
    };

    /// A set's control presses once as it goes down, and again only after
    /// it has come up; an unplugged controller lets go of it.
    #[test]
    fn a_sets_control_presses_once_a_press() {
        let mut r = Remote::new();
        assert_eq!(r.show_write(show::GO, 1.0), Some(show::GO));
        assert_eq!(r.show_write(show::GO, 1.0), None, "still down");
        assert_eq!(r.show_write(show::NEXT_SONG, 0.8), Some(show::NEXT_SONG));
        assert_eq!(r.show_write(show::GO, 0.0), None);
        assert_eq!(r.show_write(show::GO, 0.6), Some(show::GO));
        assert_eq!(r.show_write(99, 1.0), None, "no such control");
        let _ = r.let_go().count();
        assert_eq!(r.show_write(show::NEXT_SONG, 1.0), Some(show::NEXT_SONG));
    }

    /// Next and Previous patch press once a press, each saying whether
    /// it waits for the hands; the knobs aren't presses.
    #[test]
    fn next_and_previous_patch_press_once_a_press() {
        let mut r = Remote::new();
        assert_eq!(r.patch_write(play::NEXT_PATCH, 1.0), Some((1, false)));
        assert_eq!(r.patch_write(play::NEXT_PATCH, 0.9), None, "still down");
        assert_eq!(
            r.patch_write(play::PREVIOUS_PATCH_HANDS_UP, 1.0),
            Some((-1, true))
        );
        assert_eq!(r.patch_write(play::NEXT_PATCH, 0.0), None);
        assert_eq!(r.patch_write(play::NEXT_PATCH, 1.0), Some((1, false)));
        assert_eq!(r.patch_write(play::SWING, 1.0), None);
        let _ = r.let_go().count();
        assert_eq!(r.patch_write(play::NEXT_PATCH, 1.0), Some((1, false)));
        // The knobs' moves still go by id.
        let mut m = PlayMoves::default();
        m.set(play::XY_UP, 1.0);
        m.set(play::NEXT_PATCH, 1.0);
        assert_eq!(m.take(STOPPED).xy, Some((0.5, 1.0, true)));
    }

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

    /// A controller pulled out while its footswitch is down never sends
    /// the let-go: the key lets go without clearing its loop, and the next
    /// press is a press.
    #[test]
    fn an_unplugged_footswitch_lets_go_and_clears_nothing() {
        let mut r = Remote::new();
        let k1 = looper::key(0);
        assert_eq!(r.looper_write(k1, 1.0, STOPPED, 0.0), LooperAct::Key(0));
        assert_eq!(
            r.looper_write(looper::UNDO, 1.0, STOPPED, 0.1),
            LooperAct::Press(Looper::Undo)
        );
        assert_eq!(r.let_go().count(), 0, "no transport press held");
        assert!(r.loop_holds(5.0, 1.0).is_empty(), "nothing cleared");
        assert_eq!(r.looper_write(k1, 1.0, STOPPED, 6.0), LooperAct::Key(0));
        assert_eq!(
            r.looper_write(looper::UNDO, 1.0, STOPPED, 6.0),
            LooperAct::Press(Looper::Undo)
        );
    }

    /// A controller pulled out while a learned control holds REC down
    /// and another a Clips grid press: REC comes back to be let up, once;
    /// the Clips press lets go; and the next press of each is a press.
    #[test]
    fn an_unplugged_controller_lets_the_transport_and_the_clips_go() {
        let mut r = Remote::new();
        assert_eq!(
            r.write(transport::REC, 1.0, STOPPED),
            Act::Press(transport::REC, true)
        );
        assert_eq!(
            r.write(transport::TAP_TEMPO, 1.0, STOPPED),
            Act::Press(transport::TAP_TEMPO, true)
        );
        assert_eq!(
            r.write(transport::TAP_TEMPO, 0.0, STOPPED),
            Act::Press(transport::TAP_TEMPO, false)
        );
        let scene = clips::scene(1);
        assert_eq!(
            r.clips_write(scene, 1.0, STOPPED),
            ClipsAct::Press(Clips::Scene(1))
        );
        assert_eq!(r.let_go().collect::<Vec<_>>(), [transport::REC]);
        assert_eq!(r.let_go().count(), 0, "once");
        assert_eq!(
            r.write(transport::REC, 1.0, STOPPED),
            Act::Press(transport::REC, true)
        );
        assert_eq!(
            r.clips_write(scene, 1.0, STOPPED),
            ClipsAct::Press(Clips::Scene(1))
        );
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
            (looper::SNAP, Looper::Snap),
            (looper::USE_MY_TAP, Looper::UseMyTap),
        ] {
            assert_eq!(r.looper_write(id, 1.0, STOPPED, 0.0), LooperAct::Press(l));
            assert_eq!(r.looper_write(id, 0.8, STOPPED, 0.0), LooperAct::Nothing);
            assert_eq!(r.looper_write(id, 0.0, STOPPED, 0.0), LooperAct::Nothing);
            assert_eq!(r.looper_write(id, 1.0, STOPPED, 0.0), LooperAct::Press(l));
            assert_eq!(describe_looper(id, STOPPED), None);
        }
        assert_eq!(r.looper_write(9, 1.0, STOPPED, 0.0), LooperAct::Nothing);
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
    /// The Play card's knobs: the Swing in whole percents, the Drums
    /// filter off in a knob's middle, and only what changed.
    #[test]
    fn plays_swing_and_filter_turn_as_its_q_links_do() {
        let mut m = PlayMoves::default();
        m.set(play::SWING, 0.32);
        m.set(play::FILTER, 0.25);
        let a = m.take(STOPPED);
        assert_eq!(a.swing, Some(58));
        assert_eq!(a.filter, Some(-50.0));
        assert_eq!(a.xy, None);
        assert_eq!(m.take(STOPPED), PlayActs::default(), "spent");
        // A knob's middle (63 or 64 of 127) is the filter off; the swing
        // where it is already does nothing.
        for v in [63.0 / 127.0, 64.0 / 127.0] {
            m.set(play::FILTER, v);
            let now = Now {
                drums_filter: 30.0,
                ..STOPPED
            };
            assert_eq!(m.take(now).filter, Some(0.0));
        }
        m.set(play::SWING, 0.0);
        m.set(play::FILTER, 0.5);
        assert_eq!(m.take(STOPPED), PlayActs::default());
        // An id Play hasn't got is nothing.
        m.set(9, 1.0);
        assert_eq!(m.take(STOPPED), PlayActs::default());
    }

    /// XY across and up move together, the other way where it is; the
    /// pad is held (its effect plays) until both are back where it rests.
    #[test]
    fn the_xy_pad_moves_both_ways_and_rests_in_the_middle_at_the_bottom() {
        let mut m = PlayMoves::default();
        m.set(play::XY_ACROSS, 0.9);
        m.set(play::XY_UP, 0.4);
        assert_eq!(m.take(STOPPED).xy, Some((0.9, 0.4, true)));
        // Only up moves: across stays where the pad has it.
        let now = Now {
            xy: (0.9, 0.4),
            ..STOPPED
        };
        m.set(play::XY_UP, 0.0);
        assert_eq!(m.take(now).xy, Some((0.9, 0.0, true)), "across still off");
        let now = Now {
            xy: (0.9, 0.0),
            ..STOPPED
        };
        m.set(play::XY_ACROSS, 64.0 / 127.0);
        assert_eq!(m.take(now).xy, Some((0.5, 0.0, false)), "resting: let go");
        m.set(play::XY_UP, 1.0 / 127.0);
        assert_eq!(m.take(STOPPED).xy, Some((0.5, 1.0 / 127.0, true)));
    }

    /// Learn reads each where it stands, in its own units.
    #[test]
    fn plays_controls_read_where_they_stand() {
        let now = Now {
            swing: 62,
            drums_filter: -40.0,
            xy: (0.25, 0.75),
            ..STOPPED
        };
        let read = |id| describe_play(id, now).map(|(i, v)| (i.id, i.to_unit(v)));
        assert_eq!(read(play::SWING), Some((play::SWING, 12.0 / 25.0)));
        assert_eq!(read(play::FILTER), Some((play::FILTER, 0.3)));
        assert_eq!(read(play::XY_ACROSS), Some((play::XY_ACROSS, 0.25)));
        assert_eq!(read(play::XY_UP), Some((play::XY_UP, 0.75)));
        assert_eq!(describe_play(4, now), None);
    }

    /// A key learned on a Perform pad presses it past the middle and lets
    /// go under it, of what it pressed (a page turned meanwhile changes
    /// nothing); a loop key held a second clears once; an unplugged
    /// controller lets go of everything it held.
    #[test]
    fn a_learned_key_plays_its_perform_pad_and_lets_go_of_what_it_pressed() {
        use crate::perform_pads::{Hold, Throw};
        let throw = PadAction::Throw(Throw::new(1, Hold::WhileHeld));
        let mut what = throw;
        let mut k = PerformKeys::default();
        let b2 = perform::pad(1, 1);
        assert_eq!(
            k.write(b2, 1.0, 0.0, |page, pad| {
                assert_eq!((page, pad), (1, 1));
                what
            }),
            PerformAct::Down {
                page: 1,
                pad: 1,
                action: throw
            }
        );
        assert_eq!(k.write(b2, 0.8, 0.1, |_, _| what), PerformAct::Nothing);
        // The pad changed while held: the key lets go of the throw.
        what = PadAction::Scene(2);
        assert_eq!(k.write(b2, 0.0, 0.2, |_, _| what), PerformAct::Up(throw));
        assert_eq!(k.write(b2, 0.0, 0.3, |_, _| what), PerformAct::Nothing);
        assert_eq!(k.write(128, 1.0, 0.3, |_, _| what), PerformAct::Nothing);
        // A loop key held a second clears its loop, once; a throw's
        // hold clears nothing.
        let a1 = perform::pad(0, 0);
        k.write(a1, 1.0, 1.0, |_, _| PadAction::LoopKey(0));
        k.write(b2, 1.0, 1.0, |_, _| throw);
        assert!(k.holds(1.5, 1.0).is_empty());
        assert_eq!(k.holds(2.0, 1.0), [PadAction::LoopKey(0)]);
        assert!(k.holds(3.0, 1.0).is_empty());
        // Unplugged: both let go, and the next press is a press again.
        let mut gone = k.let_go();
        gone.sort_by_key(|a| matches!(a, PadAction::LoopKey(_)));
        assert_eq!(gone, [throw, PadAction::LoopKey(0)]);
        assert!(matches!(
            k.write(b2, 1.0, 4.0, |_, _| throw),
            PerformAct::Down { .. }
        ));
    }
}
