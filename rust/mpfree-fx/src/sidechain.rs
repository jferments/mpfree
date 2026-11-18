//! Which effects can listen to another track's sound (their sidechain),
//! and the control that says they do (layout spec 4b, "Sidechain": Jay,
//! 18 Nov 2026). On the screens the word is sidechain, never key.
//!
//! The mixer feeds an insert another strip's signal ([`crate::Mixer::set_key`],
//! the strip after its inserts, before its fader and mute); the effect
//! only listens to it while its own control says so: the Compressor's and
//! the Gate's Sidechain switch, the Ducker's Source on Sidechain, the
//! Vocoder's Carrier on Input (there the track's own sound is carried and
//! the strip fed in is the voice). The effect page shows that control as
//! one row, Off or the track picked, and picking a track sets both.

use crate::ids;

/// How an effect's control and its sidechain go together on its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// "Sidechain: Off | Kick ›": the control is off (its own sound) or
    /// on (the track picked). The Compressor, the Gate.
    Switch,
    /// "Source: Kick › | Tempo": the track picked, or the tempo. The
    /// Ducker.
    Source,
    /// The Vocoder's Carrier: on Input the track picked is the voice
    /// ("Voice: Mic ›" in the Carrier heading); Saw and Noise need none.
    Voice,
}

/// An effect's sidechain control.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listens {
    /// The control's parameter id.
    pub param: u16,
    /// Its value while the effect listens to the sidechain.
    pub on: f32,
    /// Its value for "Off" (its own sound) or "Tempo"; None for the
    /// Vocoder, whose other carriers are its own buttons.
    pub off: Option<f32>,
    pub row: Row,
    /// What it does while it listens and nothing is picked, for the red
    /// note in its section ("Nothing picked: it won't duck").
    pub nothing: &'static str,
}

impl Listens {
    /// Whether control value `v` has the effect listening.
    pub fn listening(&self, v: f32) -> bool {
        (v - self.on).abs() < 0.5
    }
}

/// The sidechain control of effect `id` (a `NodeInfo::id`), if it has one.
pub fn listens(id: &str) -> Option<Listens> {
    Some(match id {
        "mpfree.fx.compressor" => Listens {
            param: ids::compressor::SIDECHAIN,
            on: 1.0,
            off: Some(0.0),
            row: Row::Switch,
            nothing: "Nothing picked: it hears its own track",
        },
        "mpfree.fx.gate" => Listens {
            param: ids::gate::KEY,
            on: 1.0,
            off: Some(0.0),
            row: Row::Switch,
            nothing: "Nothing picked: it hears its own track",
        },
        "mpfree.fx.ducker" => Listens {
            param: ids::ducker::SOURCE,
            on: 0.0,
            off: Some(1.0),
            row: Row::Source,
            nothing: "Nothing picked: it won't duck",
        },
        "mpfree.fx.vocoder" => Listens {
            param: ids::vocoder::CARRIER,
            on: 2.0,
            off: None,
            row: Row::Voice,
            nothing: "Nothing picked: it is silent",
        },
        _ => return None,
    })
}

/// Why a strip can't feed another's sidechain, in a musician's words
/// ([`crate::Mixer::sidechain_refusal`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// It is the strip itself.
    Itself,
    /// It comes after the strip in the mix (a submix feeding a track).
    Later,
    /// A submix numbered higher (submixes hear lower-numbered ones only).
    HigherSubmix,
    /// It listens to this strip already (through its own sidechains), so
    /// each would wait for the other.
    Loop,
}

impl Refusal {
    /// The line under its tile in the sidechain list.
    pub fn text(self) -> &'static str {
        match self {
            Refusal::Itself => "this strip",
            Refusal::Later => "comes after it in the mix",
            Refusal::HigherSubmix => "a later submix",
            Refusal::Loop => "it listens to this one",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry;

    #[test]
    fn each_effect_that_listens_names_a_control_it_has() {
        for id in [
            "mpfree.fx.compressor",
            "mpfree.fx.gate",
            "mpfree.fx.ducker",
            "mpfree.fx.vocoder",
        ] {
            let l = listens(id).expect(id);
            let n = registry::make(id).expect(id);
            let p = n
                .params()
                .iter()
                .find(|p| p.id == l.param)
                .unwrap_or_else(|| panic!("{id}: no control {}", l.param));
            assert!(!p.readonly, "{id}");
            assert!(p.min <= l.on && l.on <= p.max, "{id}");
            if let Some(off) = l.off {
                assert!(p.min <= off && off <= p.max, "{id}");
                assert!(!l.listening(off), "{id}");
            }
            assert!(l.listening(l.on), "{id}");
        }
        assert!(listens("mpfree.fx.delay").is_none());
    }

    /// The Ducker starts on Sidechain (so adding one asks for a track);
    /// the Compressor and the Gate start on their own sound.
    #[test]
    fn only_the_ducker_starts_listening() {
        for (id, starts) in [
            ("mpfree.fx.compressor", false),
            ("mpfree.fx.gate", false),
            ("mpfree.fx.ducker", true),
            ("mpfree.fx.vocoder", false),
        ] {
            let l = listens(id).unwrap();
            let n = registry::make(id).unwrap();
            assert_eq!(l.listening(n.param(l.param)), starts, "{id}");
        }
    }
}
