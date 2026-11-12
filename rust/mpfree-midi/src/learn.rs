//! MIDI Learn: any knob, fader, key or pedal on an external device moves
//! any parameter.
//!
//! The user touches a parameter on screen ([`LearnMap::arm`]), then moves
//! a control on their device: the next CC, note, pitch bend or pressure
//! from an external port takes it ([`LearnMap::input`] returns
//! [`Input::Learned`]). From then on that control writes the parameter
//! ([`Input::Write`], a value from 0 to 1 over the parameter's range,
//! which the app sets the way the Q-Links do).
//!
//! Targets are `mpfree_param::ParamAddr`, so a learned control reaches
//! the same things the Q-Links, macros and the mod matrix do.
//!
//! Each mapping has:
//! - a range (`lo` to `hi`, 0 to 1; `lo > hi` turns it upside down);
//! - a [`Mode`]: absolute (a fader or knob with an end stop), relative (an
//!   endless encoder, in one of the three usual encodings), toggle or
//!   momentary (a button or a key);
//! - a [`Takeover`] for absolute controls: jump, or pickup, which waits
//!   until the control passes the parameter's value before it moves it,
//!   so nothing leaps when the knob and the screen disagree.
//!
//! The MPC's own pads and knobs never come here (the app gives only
//! external ports). This is control-thread code: it may allocate.
//! Mappings save as lines for `mpfree-store` ([`LearnMap::save`]).

use std::collections::HashMap;

use mpfree_param::ParamAddr;

use crate::input::PortFilter;
use crate::msg::{Channel, Msg};
use crate::port::PortId;

/// What on the device moves it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Cc(u8),
    Note(u8),
    PitchBend,
    ChanPressure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Control {
    pub port: PortFilter,
    pub channel: Channel,
    pub source: Source,
}

impl Control {
    /// The control a message comes from, if it can be learned.
    pub fn of(port: PortId, m: &Msg) -> Option<Control> {
        let source = match *m {
            Msg::Cc { cc, .. } => Source::Cc(cc),
            Msg::NoteOn { note, .. } | Msg::NoteOff { note, .. } => Source::Note(note),
            Msg::PitchBend { .. } => Source::PitchBend,
            Msg::ChanPressure { .. } => Source::ChanPressure,
            _ => return None,
        };
        Some(Control {
            port: PortFilter::One(port),
            channel: m.channel(),
            source,
        })
    }

    fn matches(&self, port: PortId, m: &Msg) -> bool {
        Control::of(port, m).is_some_and(|c| {
            c.source == self.source && c.channel == self.channel && self.port.matches(port)
        })
    }
}

/// How an endless encoder says which way it turned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// 1 to 63 up, 127 down to 65 down (two's complement).
    TwosComplement,
    /// 65 to 127 up, 63 down to 1 down (offset by 64).
    Offset64,
    /// 1 to 63 up, 65 to 127 down (bit 6 is the sign).
    SignBit,
}

impl Encoding {
    /// Steps turned, + up.
    pub fn delta(self, v: u8) -> i32 {
        let v = i32::from(v & 0x7F);
        match self {
            Encoding::TwosComplement => {
                if v < 64 {
                    v
                } else {
                    v - 128
                }
            }
            Encoding::Offset64 => v - 64,
            Encoding::SignBit => {
                if v < 64 {
                    v
                } else {
                    -(v - 64)
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Absolute,
    Relative(Encoding),
    /// Each press flips between `lo` and `hi`.
    Toggle,
    /// `hi` while held, `lo` when let go.
    Momentary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Takeover {
    Jump,
    Pickup,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mapping {
    pub control: Control,
    pub target: ParamAddr,
    pub lo: f32,
    pub hi: f32,
    pub mode: Mode,
    pub takeover: Takeover,
}

/// Steps of an endless encoder for the whole range.
pub const RELATIVE_STEPS: f32 = 128.0;
/// How near (0 to 1) a picked-up control has to come to the value.
pub const PICKUP_WINDOW: f32 = 1.5 / 127.0;

/// Set this parameter to `value` (0 to 1 over its range).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Write {
    pub target: ParamAddr,
    pub value: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    /// The armed parameter took this control; the mapping's index.
    Learned(usize),
    Write(Write),
}

#[derive(Default)]
struct State {
    /// Pickup: the control has reached the value.
    picked: bool,
    /// The control's last position (0 to 1), for pickup crossing.
    last_in: Option<f32>,
    /// Toggle: on.
    on: bool,
}

#[derive(Default)]
pub struct LearnMap {
    maps: Vec<(Mapping, State)>,
    armed: Option<ParamAddr>,
    /// Parameter values (0 to 1) as the app last reported or this wrote.
    values: HashMap<ParamAddr, f32>,
}

fn position(m: &Msg) -> Option<f32> {
    Some(match *m {
        Msg::Cc { value, .. } | Msg::ChanPressure { value, .. } => f32::from(value) / 127.0,
        Msg::NoteOn { .. } => 1.0,
        Msg::NoteOff { .. } => 0.0,
        Msg::PitchBend { value, .. } => (f32::from(value) + 8192.0) / 16383.0,
        _ => return None,
    })
}

impl LearnMap {
    pub fn new() -> LearnMap {
        LearnMap::default()
    }

    /// The next control moved takes `target`.
    pub fn arm(&mut self, target: ParamAddr) {
        self.armed = Some(target);
    }

    pub fn disarm(&mut self) {
        self.armed = None;
    }

    pub fn armed(&self) -> Option<ParamAddr> {
        self.armed
    }

    pub fn mappings(&self) -> impl Iterator<Item = &Mapping> {
        self.maps.iter().map(|(m, _)| m)
    }

    pub fn get(&self, i: usize) -> Option<&Mapping> {
        self.maps.get(i).map(|(m, _)| m)
    }

    /// Adds a mapping; one that has the same control is replaced (a
    /// control moves one thing). Returns its index.
    pub fn add(&mut self, m: Mapping) -> usize {
        if let Some(i) = self.maps.iter().position(|(x, _)| x.control == m.control) {
            self.maps[i] = (m, State::default());
            i
        } else {
            self.maps.push((m, State::default()));
            self.maps.len() - 1
        }
    }

    pub fn remove(&mut self, i: usize) -> Option<Mapping> {
        (i < self.maps.len()).then(|| self.maps.remove(i).0)
    }

    /// Every mapping onto `target` goes (the parameter was deleted).
    pub fn forget(&mut self, target: ParamAddr) {
        self.maps.retain(|(m, _)| m.target != target);
        self.values.remove(&target);
    }

    pub fn set_mode(&mut self, i: usize, mode: Mode) {
        if let Some((m, s)) = self.maps.get_mut(i) {
            m.mode = mode;
            *s = State::default();
        }
    }

    pub fn set_takeover(&mut self, i: usize, t: Takeover) {
        if let Some((m, s)) = self.maps.get_mut(i) {
            m.takeover = t;
            s.picked = false;
        }
    }

    pub fn set_range(&mut self, i: usize, lo: f32, hi: f32) {
        if let Some((m, _)) = self.maps.get_mut(i) {
            m.lo = lo.clamp(0.0, 1.0);
            m.hi = hi.clamp(0.0, 1.0);
        }
    }

    /// The parameter's value now (0 to 1), when something other than a
    /// learned control set it: a picked-up control lets go until it
    /// reaches the new value.
    pub fn observe(&mut self, target: ParamAddr, value: f32) {
        let old = self.values.insert(target, value);
        if old.is_some_and(|o| (o - value).abs() <= f32::EPSILON) {
            return;
        }
        for (m, s) in &mut self.maps {
            if m.target == target {
                s.picked = false;
            }
        }
    }

    /// One message from an external port.
    pub fn input(&mut self, port: PortId, msg: &Msg, mut out: impl FnMut(Input)) {
        if let Some(target) = self.armed {
            if let Some(control) = Control::of(port, msg) {
                // Learn on a press or a move, not on a key's release.
                if !matches!(msg, Msg::NoteOff { .. }) {
                    let mode = match control.source {
                        Source::Note(_) => Mode::Momentary,
                        _ => Mode::Absolute,
                    };
                    let i = self.add(Mapping {
                        control,
                        target,
                        lo: 0.0,
                        hi: 1.0,
                        mode,
                        takeover: Takeover::Jump,
                    });
                    self.armed = None;
                    out(Input::Learned(i));
                    return;
                }
            }
        }
        let Some(pos) = position(msg) else { return };
        for k in 0..self.maps.len() {
            if !self.maps[k].0.control.matches(port, msg) {
                continue;
            }
            if let Some(w) = self.apply(k, msg, pos) {
                self.values.insert(w.target, w.value);
                out(Input::Write(w));
            }
        }
    }

    fn apply(&mut self, k: usize, msg: &Msg, pos: f32) -> Option<Write> {
        let current = self.values.get(&self.maps[k].0.target).copied();
        let (m, s) = &mut self.maps[k];
        let span = |x: f32| m.lo + (m.hi - m.lo) * x;
        let value = match m.mode {
            Mode::Absolute => {
                let v = span(pos);
                if m.takeover == Takeover::Pickup && !s.picked {
                    let last = s.last_in.replace(v);
                    let reached = match (current, last) {
                        (None, _) => true,
                        (Some(c), last) => {
                            (v - c).abs() <= PICKUP_WINDOW
                                || last.is_some_and(|l| (l - c) * (v - c) <= 0.0)
                        }
                    };
                    if !reached {
                        return None;
                    }
                    s.picked = true;
                }
                s.last_in = Some(v);
                v
            }
            Mode::Relative(enc) => {
                let Msg::Cc { value, .. } = *msg else {
                    return None;
                };
                let d = enc.delta(value);
                if d == 0 {
                    return None;
                }
                let step = (m.hi - m.lo) / RELATIVE_STEPS;
                let (a, b) = if m.lo <= m.hi {
                    (m.lo, m.hi)
                } else {
                    (m.hi, m.lo)
                };
                (current.unwrap_or(m.lo) + step * d as f32).clamp(a, b)
            }
            Mode::Toggle => {
                if pos < 0.5 {
                    return None; // only presses flip it
                }
                s.on = !s.on;
                if s.on {
                    m.hi
                } else {
                    m.lo
                }
            }
            Mode::Momentary => {
                if pos >= 0.5 {
                    m.hi
                } else {
                    m.lo
                }
            }
        };
        Some(Write {
            target: m.target,
            value: value.clamp(0.0, 1.0),
        })
    }

    /// One line a mapping, for `mpfree-store`'s "midi" settings.
    /// `name_of` gives a port's name, so a mapping finds its device again
    /// whatever id it gets next time.
    pub fn save(&self, name_of: impl Fn(PortId) -> Option<String>) -> String {
        let mut s = String::new();
        for (m, _) in &self.maps {
            let port = match m.control.port {
                PortFilter::Any => "any".to_string(),
                PortFilter::One(p) => match name_of(p) {
                    Some(n) => format!("port {}", n.replace('|', "/")),
                    None => continue,
                },
            };
            let src = match m.control.source {
                Source::Cc(n) => format!("cc {n}"),
                Source::Note(n) => format!("note {n}"),
                Source::PitchBend => "bend".to_string(),
                Source::ChanPressure => "pressure".to_string(),
            };
            let mode = match m.mode {
                Mode::Absolute => "absolute",
                Mode::Relative(Encoding::TwosComplement) => "relative-2c",
                Mode::Relative(Encoding::Offset64) => "relative-64",
                Mode::Relative(Encoding::SignBit) => "relative-sign",
                Mode::Toggle => "toggle",
                Mode::Momentary => "momentary",
            };
            let take = match m.takeover {
                Takeover::Jump => "jump",
                Takeover::Pickup => "pickup",
            };
            s.push_str(&format!(
                "learn = {port} | ch {} | {src} | {} | {:.4} {:.4} | {mode} | {take}\n",
                m.control.channel + 1,
                m.target,
                m.lo,
                m.hi
            ));
        }
        s
    }

    /// Reads [`LearnMap::save`]'s lines; `id_of` turns a port name into
    /// its id (the app's `Ports::reserve`). Returns the map and how many
    /// lines it could not read.
    pub fn load(text: &str, mut id_of: impl FnMut(&str) -> Option<PortId>) -> (LearnMap, usize) {
        let mut map = LearnMap::new();
        let mut bad = 0;
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let ok = (|| {
                let rest = line
                    .strip_prefix("learn")?
                    .trim_start()
                    .strip_prefix('=')?
                    .trim();
                let f: Vec<&str> = rest.split('|').map(str::trim).collect();
                let [port, ch, src, target, range, mode, take] = f[..] else {
                    return None;
                };
                let port = if port == "any" {
                    PortFilter::Any
                } else {
                    PortFilter::One(id_of(port.strip_prefix("port ")?)?)
                };
                let ch: u8 = ch.strip_prefix("ch ")?.parse().ok()?;
                if !(1..=16).contains(&ch) {
                    return None;
                }
                let source = match src.split_once(' ') {
                    Some(("cc", n)) => Source::Cc(n.parse().ok().filter(|&n: &u8| n < 128)?),
                    Some(("note", n)) => Source::Note(n.parse().ok().filter(|&n: &u8| n < 128)?),
                    None if src == "bend" => Source::PitchBend,
                    None if src == "pressure" => Source::ChanPressure,
                    _ => return None,
                };
                let target: ParamAddr = target.parse().ok()?;
                let (lo, hi) = range.split_once(' ')?;
                let (lo, hi): (f32, f32) = (lo.parse().ok()?, hi.parse().ok()?);
                if !(0.0..=1.0).contains(&lo) || !(0.0..=1.0).contains(&hi) {
                    return None;
                }
                let mode = match mode {
                    "absolute" => Mode::Absolute,
                    "relative-2c" => Mode::Relative(Encoding::TwosComplement),
                    "relative-64" => Mode::Relative(Encoding::Offset64),
                    "relative-sign" => Mode::Relative(Encoding::SignBit),
                    "toggle" => Mode::Toggle,
                    "momentary" => Mode::Momentary,
                    _ => return None,
                };
                let takeover = match take {
                    "jump" => Takeover::Jump,
                    "pickup" => Takeover::Pickup,
                    _ => return None,
                };
                map.add(Mapping {
                    control: Control {
                        port,
                        channel: ch - 1,
                        source,
                    },
                    target,
                    lo,
                    hi,
                    mode,
                    takeover,
                });
                Some(())
            })();
            if ok.is_none() {
                bad += 1;
            }
        }
        (map, bad)
    }
}
