//! The XY pad's hand on the arp (the XY controller, phase 2): a layout of
//! yours holds the rate, the gate, the swing, the octaves, the
//! Modifier's push or a phrase's speed where the pad puts them, until it
//! lets go.
//!
//! - **The settings as set never change while it holds**
//!   ([`crate::Engine::settings`]): the steps play the held ones, and the
//!   Modifier still pushes them ([`crate::Engine::live`]). Let go and the
//!   arp plays as set again.
//! - **The Modifier's push** held by the pad wins over its controller (the
//!   mod wheel, pressure, a pedal), whichever it is, even Off: the pad is
//!   then the controller, pushing by the Modifier's amounts. The
//!   controller's moves wait, and it has the push back when the pad lets
//!   go, where it is by then.
//! - **A change to a held setting as set takes it from the pad** (a
//!   Q-Link turn, the panel, a learned knob): the pad's moves on it are
//!   kept until it lets go, as a turn takes a synth's knob from the pad.
//! - **Swing held** plays the arp's own swing while the transport swings,
//!   as a Q-Link turn of it does.
//!
//! Design: `docs/dev/design/xy.md`, "The arp on the pad".

use crate::engine::{Rate, Settings};
use crate::phrase::Speed;

/// What the pad's hand can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandOn {
    /// The rate, as its place in [`Rate::ALL`] (0 is 1/4).
    Rate,
    /// The gate, 1 to 100 %.
    Gate,
    /// The swing, 50 to 75 %.
    Swing,
    /// The octaves, 1 to 4.
    Octaves,
    /// The Modifier's push, 0 to 1.
    Modifier,
    /// A phrase's speed, as its place in [`Speed::ALL`] (0 is half).
    Speed,
}

impl HandOn {
    /// Every one, in the Q-Links' order, then the Modifier and a
    /// phrase's speed.
    pub const ALL: [HandOn; 6] = [
        HandOn::Rate,
        HandOn::Gate,
        HandOn::Swing,
        HandOn::Octaves,
        HandOn::Modifier,
        HandOn::Speed,
    ];

    fn at(self) -> usize {
        self as usize
    }

    /// `value` brought into this one's range and steps.
    fn clamp(self, value: f32) -> f32 {
        let v = if value.is_finite() { value } else { 0.0 };
        match self {
            HandOn::Rate => v.round().clamp(0.0, (Rate::ALL.len() - 1) as f32),
            HandOn::Gate => v.round().clamp(1.0, 100.0),
            HandOn::Swing => v.round().clamp(50.0, 75.0),
            HandOn::Octaves => v.round().clamp(1.0, 4.0),
            HandOn::Modifier => v.clamp(0.0, 1.0),
            HandOn::Speed => v.round().clamp(0.0, (Speed::ALL.len() - 1) as f32),
        }
    }

    /// This one's value in `s`, as the pad holds it.
    fn of(self, s: &Settings) -> f32 {
        match self {
            HandOn::Rate => Rate::ALL.iter().position(|r| *r == s.rate).unwrap_or(0) as f32,
            HandOn::Gate => f32::from(s.gate),
            HandOn::Swing => f32::from(s.swing),
            HandOn::Octaves => f32::from(s.octaves),
            HandOn::Modifier => 0.0,
            HandOn::Speed => Speed::ALL.iter().position(|v| *v == s.speed).unwrap_or(1) as f32,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Slot {
    #[default]
    Off,
    /// The pad holds it at this value.
    Holds(f32),
    /// A change as set took it from the pad: its moves are kept until it
    /// lets go.
    Lost,
}

/// Where the pad's hand holds the arp. See the module docs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hand {
    s: [Slot; 6],
}

impl Hand {
    /// Nothing held.
    pub fn is_empty(&self) -> bool {
        self.s.iter().all(|s| *s == Slot::Off)
    }

    /// Where the pad holds `on` now: None when it doesn't (or a change as
    /// set took it).
    pub fn held(&self, on: HandOn) -> Option<f32> {
        match self.s[on.at()] {
            Slot::Holds(v) => Some(v),
            _ => None,
        }
    }

    /// The pad holds `on` at `value`, or lets go of it (`None`). A setting
    /// a change took from the pad keeps it until it lets go. True when
    /// what plays may have changed.
    pub(crate) fn hold(&mut self, on: HandOn, value: Option<f32>) -> bool {
        let slot = &mut self.s[on.at()];
        let new = match (value, *slot) {
            (None, _) => Slot::Off,
            (Some(_), Slot::Lost) => Slot::Lost,
            (Some(v), _) => Slot::Holds(on.clamp(v)),
        };
        let changed = new != *slot;
        *slot = new;
        changed
    }

    /// `set` with the held settings over it.
    pub fn over(&self, set: &Settings) -> Settings {
        let mut s = *set;
        if let Some(v) = self.held(HandOn::Rate) {
            s.rate = Rate::ALL[v as usize];
        }
        if let Some(v) = self.held(HandOn::Gate) {
            s.gate = v as u8;
        }
        if let Some(v) = self.held(HandOn::Swing) {
            s.swing = v as u8;
        }
        if let Some(v) = self.held(HandOn::Octaves) {
            s.octaves = v as u8;
        }
        if let Some(v) = self.held(HandOn::Speed) {
            s.speed = Speed::ALL[v as usize];
        }
        s
    }

    /// The settings as set went from `old` to `new`: each held one they
    /// changed is taken from the pad.
    pub(crate) fn turned(&mut self, old: &Settings, new: &Settings) {
        for on in HandOn::ALL {
            if on != HandOn::Modifier
                && matches!(self.s[on.at()], Slot::Holds(_))
                && on.of(old) != on.of(new)
            {
                self.s[on.at()] = Slot::Lost;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hand_holds_over_the_settings_and_a_change_as_set_takes_one() {
        let set = Settings::default();
        let mut h = Hand::default();
        assert!(h.is_empty());
        assert_eq!(h.over(&set), set, "nothing held: as set");
        assert!(h.hold(HandOn::Gate, Some(80.4)));
        assert!(h.hold(HandOn::Rate, Some(99.0)));
        assert!(h.hold(HandOn::Modifier, Some(0.25)));
        let s = h.over(&set);
        assert_eq!(s.gate, 80, "a whole percent");
        assert_eq!(s.rate, *Rate::ALL.last().unwrap(), "kept to the rates");
        assert_eq!(s.octaves, set.octaves);
        assert_eq!(h.held(HandOn::Modifier), Some(0.25));
        // A turn of the gate as set takes it from the pad; the rate,
        // unchanged, stays held.
        let turned = Settings { gate: 30, ..set };
        h.turned(&set, &turned);
        assert_eq!(h.held(HandOn::Gate), None);
        assert!(!h.hold(HandOn::Gate, Some(90.0)), "kept until it lets go");
        assert_eq!(h.over(&turned).gate, 30);
        assert_eq!(h.over(&turned).rate, *Rate::ALL.last().unwrap());
        // A phrase's speed held: double, kept to the speeds.
        assert!(h.hold(HandOn::Speed, Some(7.0)));
        assert_eq!(h.over(&turned).speed, Speed::Double);
        // Let go, and the pad can hold it again.
        assert!(h.hold(HandOn::Gate, None));
        assert!(h.hold(HandOn::Gate, Some(90.0)));
        for on in HandOn::ALL {
            h.hold(on, None);
        }
        assert!(h.is_empty());
    }
}
