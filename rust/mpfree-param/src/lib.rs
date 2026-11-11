//! One way to name any parameter in MPFree.
//!
//! A [`ParamAddr`] says *whose* parameter it is (an [`Owner`]) and *which*
//! one (`param`, the owner's stable parameter id: a node's
//! `mpfree_node::ParamInfo::id`, a mixer strip's `mpfree_fx::ids::mixer`
//! id, a mod source's id in `mpfree-mod`). Parameter locks, macros, the mod
//! matrix, morph scenes and Q-Link bindings all name their targets this
//! way, so one target can be locked on a step, moved by a macro and swept
//! by an LFO at once, and a project file names it the same way everywhere.
//!
//! The crate has no dependencies and every type is `Copy` and 8 bytes or
//! less, so the sequencer, the effects, the synths and the screens can all
//! use it without pulling each other in, and it is free to pass on the
//! audio thread.
//!
//! # Track-relative addresses
//!
//! A clip can move to another track, so what it stores is a [`RelAddr`]:
//! the address without the track ([`OnTrack`]). The player fills in the
//! playing track ([`RelAddr::on`]) when it compiles the clip.
//!
//! # Text form
//!
//! For project files, logs and tests, every address has a short text form
//! that [`Display`] writes and [`FromStr`] reads back. Indexes count from
//! 0, as in the code; screens show their own names.
//!
//! | Address | Text |
//! |---|---|
//! | track 3's instrument, parameter 12 | `t3.inst.12` |
//! | track 3, sampler pad 5, parameter 2 | `t3.pad5.2` |
//! | track 3's mixer strip, volume (0) | `t3.mix.0` |
//! | bus 2, the insert with slot id 7, parameter 4 | `bus2.fx7.4` |
//! | input 0 / pad channel 9 / return 1 / master strip | `in0.mix.0` / `ps9.mix.0` / `ret1.mix.0` / `master.mix.0` |
//! | track 3's arpeggiator / sequencer settings | `t3.arp.1` / `t3.seq.0` |
//! | macro 2's value | `macro2.0` |
//! | mod source 4's parameter 1 (an LFO's rate) | `mod4.1` |
//! | morph 0's position | `morph0.0` |
//!
//! A [`RelAddr`] leaves the track out: `inst.12`, `pad5.2`, `mix.0`,
//! `fx7.4`, `arp.1`, `seq.0`.
#![forbid(unsafe_code)]

use std::fmt;
use std::str::FromStr;

/// A mixer strip. Mirrors `mpfree_fx::mixer::StripId` (which uses
/// `usize`); this crate has no dependency on the mixer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Strip {
    Input(u8),
    /// A pad channel of the mixer's pad pool (not a sampler pad's sound;
    /// that is [`Owner::Pad`]).
    Pad(u8),
    Track(u8),
    Bus(u8),
    Return(u8),
    Master,
}

/// Whose parameter it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Owner {
    /// A track's instrument node (Poly, FM, ...).
    Inst(u8),
    /// A sampler pad's sound on a drum track: `(track, pad)`. A value set
    /// here applies to the voice that starts on that frame.
    Pad(u8, u8),
    /// A mixer strip's own controls (volume, pan, sends ...).
    Strip(Strip),
    /// An insert on a strip, named by its slot id: a number the chain
    /// gives the insert when it is made and keeps across reordering,
    /// replacing and saving, so a lock or a macro follows the effect, not
    /// the position.
    Insert(Strip, u16),
    /// A track's arpeggiator settings.
    Arp(u8),
    /// A track's sequencer settings (swing, length ...).
    Seq(u8),
    /// A macro knob. Parameter 0 is its value, 0 to 1.
    Macro(u8),
    /// A modulation source (an LFO, an envelope follower). Its parameter
    /// ids are defined by `mpfree-mod`.
    Mod(u8),
    /// A morph between two scenes. Parameter 0 is its position, 0 to 1.
    Morph(u8),
}

/// What on a track a [`RelAddr`] names, without the track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OnTrack {
    Inst,
    Pad(u8),
    /// The track's mixer strip.
    Strip,
    /// An insert on the track's mixer strip, by slot id.
    Insert(u16),
    Arp,
    Seq,
}

impl OnTrack {
    /// The owner on track `t`.
    pub const fn on(self, t: u8) -> Owner {
        match self {
            OnTrack::Inst => Owner::Inst(t),
            OnTrack::Pad(p) => Owner::Pad(t, p),
            OnTrack::Strip => Owner::Strip(Strip::Track(t)),
            OnTrack::Insert(uid) => Owner::Insert(Strip::Track(t), uid),
            OnTrack::Arp => Owner::Arp(t),
            OnTrack::Seq => Owner::Seq(t),
        }
    }
}

impl Owner {
    /// The track this owner belongs to and what on it, or `None` for an
    /// owner that is not on a track (a bus, a macro ...).
    pub const fn split(self) -> Option<(u8, OnTrack)> {
        match self {
            Owner::Inst(t) => Some((t, OnTrack::Inst)),
            Owner::Pad(t, p) => Some((t, OnTrack::Pad(p))),
            Owner::Strip(Strip::Track(t)) => Some((t, OnTrack::Strip)),
            Owner::Insert(Strip::Track(t), uid) => Some((t, OnTrack::Insert(uid))),
            Owner::Arp(t) => Some((t, OnTrack::Arp)),
            Owner::Seq(t) => Some((t, OnTrack::Seq)),
            _ => None,
        }
    }

    /// The track, for an owner on a track.
    pub const fn track(self) -> Option<u8> {
        match self.split() {
            Some((t, _)) => Some(t),
            None => None,
        }
    }

    /// The same owner moved to track `t`; an owner not on a track stays.
    pub const fn with_track(self, t: u8) -> Owner {
        match self.split() {
            Some((_, on)) => on.on(t),
            None => self,
        }
    }

    /// The mixer strip this owner lives on, for strips and inserts.
    pub const fn strip(self) -> Option<Strip> {
        match self {
            Owner::Strip(s) | Owner::Insert(s, _) => Some(s),
            _ => None,
        }
    }
}

/// A parameter: whose, and which.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParamAddr {
    pub owner: Owner,
    pub param: u16,
}

impl ParamAddr {
    pub const fn new(owner: Owner, param: u16) -> ParamAddr {
        ParamAddr { owner, param }
    }

    /// The same parameter on track `t` (see [`Owner::with_track`]).
    pub const fn with_track(self, t: u8) -> ParamAddr {
        ParamAddr {
            owner: self.owner.with_track(t),
            param: self.param,
        }
    }

    /// The address without its track, for a clip to store; `None` when the
    /// owner is not on a track.
    pub const fn rel(self) -> Option<RelAddr> {
        match self.owner.split() {
            Some((_, on)) => Some(RelAddr {
                on,
                param: self.param,
            }),
            None => None,
        }
    }

    /// A macro's value.
    pub const fn macro_value(n: u8) -> ParamAddr {
        ParamAddr::new(Owner::Macro(n), 0)
    }

    /// A morph's position.
    pub const fn morph_pos(n: u8) -> ParamAddr {
        ParamAddr::new(Owner::Morph(n), 0)
    }
}

/// A parameter on "this track", for clips (see the crate docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelAddr {
    pub on: OnTrack,
    pub param: u16,
}

impl RelAddr {
    pub const fn new(on: OnTrack, param: u16) -> RelAddr {
        RelAddr { on, param }
    }

    /// The address on track `t`.
    pub const fn on(self, t: u8) -> ParamAddr {
        ParamAddr {
            owner: self.on.on(t),
            param: self.param,
        }
    }
}

// ---- Text form -------------------------------------------------------------

impl fmt::Display for Strip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Strip::Input(n) => write!(f, "in{n}"),
            Strip::Pad(n) => write!(f, "ps{n}"),
            Strip::Track(n) => write!(f, "t{n}"),
            Strip::Bus(n) => write!(f, "bus{n}"),
            Strip::Return(n) => write!(f, "ret{n}"),
            Strip::Master => f.write_str("master"),
        }
    }
}

impl fmt::Display for OnTrack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            OnTrack::Inst => f.write_str("inst"),
            OnTrack::Pad(p) => write!(f, "pad{p}"),
            OnTrack::Strip => f.write_str("mix"),
            OnTrack::Insert(uid) => write!(f, "fx{uid}"),
            OnTrack::Arp => f.write_str("arp"),
            OnTrack::Seq => f.write_str("seq"),
        }
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Owner::Strip(s) => write!(f, "{s}.mix"),
            Owner::Insert(s, uid) => write!(f, "{s}.fx{uid}"),
            Owner::Macro(n) => write!(f, "macro{n}"),
            Owner::Mod(n) => write!(f, "mod{n}"),
            Owner::Morph(n) => write!(f, "morph{n}"),
            o => {
                let (t, on) = o.split().expect("every other owner is on a track");
                write!(f, "t{t}.{on}")
            }
        }
    }
}

impl fmt::Display for ParamAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.owner, self.param)
    }
}

impl fmt::Display for RelAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.on, self.param)
    }
}

/// Why a text address did not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bad parameter address: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// `word` as `prefix` followed by a number that fits `T`.
fn num<T: FromStr>(word: &str, prefix: &str) -> Option<T> {
    let digits = word.strip_prefix(prefix)?;
    // Plain decimal only: no sign, no leading "+", nothing empty.
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn strip(word: &str) -> Option<Strip> {
    if word == "master" {
        return Some(Strip::Master);
    }
    // Longer prefixes first: "ps" before "p", "ret" before "t".
    if let Some(n) = num(word, "ps") {
        return Some(Strip::Pad(n));
    }
    if let Some(n) = num(word, "in") {
        return Some(Strip::Input(n));
    }
    if let Some(n) = num(word, "bus") {
        return Some(Strip::Bus(n));
    }
    if let Some(n) = num(word, "ret") {
        return Some(Strip::Return(n));
    }
    num(word, "t").map(Strip::Track)
}

fn on_track(word: &str) -> Option<OnTrack> {
    match word {
        "inst" => Some(OnTrack::Inst),
        "mix" => Some(OnTrack::Strip),
        "arp" => Some(OnTrack::Arp),
        "seq" => Some(OnTrack::Seq),
        _ => num(word, "pad")
            .map(OnTrack::Pad)
            .or_else(|| num(word, "fx").map(OnTrack::Insert)),
    }
}

/// The owner named by `words` (the text before the parameter, split at
/// dots).
fn owner(words: &[&str]) -> Option<Owner> {
    match *words {
        [w] => num(w, "macro")
            .map(Owner::Macro)
            .or_else(|| num(w, "mod").map(Owner::Mod))
            .or_else(|| num(w, "morph").map(Owner::Morph)),
        [s, what] => {
            let s = strip(s)?;
            if what == "mix" {
                return Some(Owner::Strip(s));
            }
            if let Some(uid) = num(what, "fx") {
                return Some(Owner::Insert(s, uid));
            }
            // Instrument, sampler pad, arp and sequencer: tracks only.
            let Strip::Track(t) = s else { return None };
            Some(on_track(what)?.on(t))
        }
        _ => None,
    }
}

impl FromStr for ParamAddr {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<ParamAddr, ParseError> {
        let bad = || ParseError(s.to_string());
        let (head, param) = s.rsplit_once('.').ok_or_else(bad)?;
        let param = num(param, "").ok_or_else(bad)?;
        let words: Vec<&str> = head.split('.').collect();
        Ok(ParamAddr {
            owner: owner(&words).ok_or_else(bad)?,
            param,
        })
    }
}

impl FromStr for RelAddr {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<RelAddr, ParseError> {
        let bad = || ParseError(s.to_string());
        let (on, param) = s.split_once('.').ok_or_else(bad)?;
        Ok(RelAddr {
            on: on_track(on).ok_or_else(bad)?,
            param: num(param, "").ok_or_else(bad)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_owner() -> Vec<Owner> {
        let strips = [
            Strip::Input(0),
            Strip::Pad(127),
            Strip::Track(3),
            Strip::Bus(7),
            Strip::Return(3),
            Strip::Master,
        ];
        let mut v = vec![
            Owner::Inst(0),
            Owner::Inst(63),
            Owner::Pad(3, 5),
            Owner::Arp(1),
            Owner::Seq(2),
            Owner::Macro(7),
            Owner::Mod(4),
            Owner::Morph(0),
        ];
        for s in strips {
            v.push(Owner::Strip(s));
            v.push(Owner::Insert(s, 0));
            v.push(Owner::Insert(s, u16::MAX));
        }
        v
    }

    #[test]
    fn small_and_copy() {
        assert!(
            std::mem::size_of::<ParamAddr>() <= 8,
            "{}",
            std::mem::size_of::<ParamAddr>()
        );
        assert!(std::mem::size_of::<RelAddr>() <= 6);
    }

    #[test]
    fn text_examples() {
        let cases = [
            (ParamAddr::new(Owner::Inst(3), 12), "t3.inst.12"),
            (ParamAddr::new(Owner::Pad(3, 5), 2), "t3.pad5.2"),
            (ParamAddr::new(Owner::Strip(Strip::Track(3)), 0), "t3.mix.0"),
            (
                ParamAddr::new(Owner::Insert(Strip::Bus(2), 7), 4),
                "bus2.fx7.4",
            ),
            (ParamAddr::new(Owner::Strip(Strip::Pad(9)), 0), "ps9.mix.0"),
            (
                ParamAddr::new(Owner::Strip(Strip::Master), 1),
                "master.mix.1",
            ),
            (
                ParamAddr::new(Owner::Insert(Strip::Return(1), 3), 0),
                "ret1.fx3.0",
            ),
            (ParamAddr::new(Owner::Arp(3), 1), "t3.arp.1"),
            (ParamAddr::macro_value(2), "macro2.0"),
            (ParamAddr::new(Owner::Mod(4), 1), "mod4.1"),
            (ParamAddr::morph_pos(0), "morph0.0"),
        ];
        for (a, s) in cases {
            assert_eq!(a.to_string(), s);
            assert_eq!(s.parse::<ParamAddr>(), Ok(a), "{s}");
        }
    }

    #[test]
    fn every_owner_round_trips() {
        for o in every_owner() {
            for p in [0u16, 1, 999, u16::MAX] {
                let a = ParamAddr::new(o, p);
                assert_eq!(a.to_string().parse::<ParamAddr>(), Ok(a), "{a}");
            }
        }
    }

    #[test]
    fn bad_text_is_refused() {
        for s in [
            "",
            "t3",
            "t3.inst",
            "t3.inst.",
            "t3.inst.x",
            "t3.inst.-1",
            "t3.inst.+1",
            "t3.inst.70000",
            "t300.inst.1",
            "bus2.inst.1",
            "master.pad1.0",
            "t3.fx.1",
            "macro.0",
            "foo.1",
            "t3.mix.0.1",
            "t.mix.0",
            "T3.mix.0",
        ] {
            assert!(s.parse::<ParamAddr>().is_err(), "{s:?} should not parse");
        }
        for s in ["", "inst", "inst.", "t3.inst.1", "pad.1", "mix.x"] {
            assert!(s.parse::<RelAddr>().is_err(), "{s:?} should not parse");
        }
    }

    #[test]
    fn tracks_come_off_and_go_back_on() {
        for o in every_owner() {
            let a = ParamAddr::new(o, 9);
            match a.rel() {
                Some(r) => {
                    let t = o.track().unwrap();
                    assert_eq!(r.on(t), a);
                    assert_eq!(r.to_string().parse::<RelAddr>(), Ok(r));
                    let moved = a.with_track(t.wrapping_add(5));
                    assert_eq!(moved.owner.track(), Some(t.wrapping_add(5)));
                    assert_eq!(moved.rel(), Some(r));
                }
                None => {
                    assert_eq!(a.with_track(5), a, "{a} is not on a track");
                    assert_eq!(o.track(), None);
                }
            }
        }
        // An insert on a track strip moves with the track; on a bus it stays.
        assert_eq!(
            ParamAddr::new(Owner::Insert(Strip::Track(1), 7), 2)
                .with_track(4)
                .to_string(),
            "t4.fx7.2"
        );
        assert_eq!(
            ParamAddr::new(Owner::Insert(Strip::Bus(1), 7), 2)
                .with_track(4)
                .to_string(),
            "bus1.fx7.2"
        );
        assert_eq!(
            "fx7.2".parse::<RelAddr>().unwrap().on(0).to_string(),
            "t0.fx7.2"
        );
    }
}
