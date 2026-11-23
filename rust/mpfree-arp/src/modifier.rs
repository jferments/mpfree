//! The modifier: one controller pushes the arp while you play.
//!
//! As the MPC's "Arpeggiator modifiers" do (MPC 3.10.1 user guide, pp. 83
//! and 84), a controller (the mod wheel, pressure, an expression pedal,
//! breath, or a knob learned with MIDI Learn) moves some of the arp's
//! settings away from where they are set, by amounts the player chooses:
//! the wheel all the way up can make the rate two steps faster and add an
//! octave, and let back down the arp plays as set again. The settings
//! themselves never change; the engine plays the pushed ones
//! ([`crate::Engine::live`]).
//!
//! Each amount is what the controller adds at its top (1.0), and the push
//! grows in a straight line from 0. Rate, octaves and rolls move in whole
//! steps, with some hold either side of each step ([`HOLD`]), so a
//! controller resting near a step's edge (a pad's pressure is never quite
//! still) doesn't flick between two rates. Gate, swing and the accent
//! chance move smoothly. Amounts can be negative for rate, octaves, gate
//! and swing: a push can slow the arp down, take octaves away, shorten
//! the notes or straighten the swing.
//!
//! Design: `docs/dev/design/arpeggiator.md`, "Controller tricks".

use crate::engine::{Rate, Settings};

/// What pushes the arp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModSource {
    /// Nothing: the arp plays as set.
    #[default]
    Off,
    /// The mod wheel, MIDI CC 1.
    ModWheel,
    /// The highest pressure (aftertouch) of the pads or keys held on this
    /// arp now: pads, a keyboard's aftertouch or its channel pressure.
    /// Latched notes, once let go, press nothing.
    Pressure,
    /// An expression (foot) pedal, MIDI CC 11.
    Expression,
    /// A breath controller, MIDI CC 2.
    Breath,
    /// A knob learned with MIDI Learn to the arp's Modifier setting.
    Knob,
}

impl ModSource {
    /// Every source, in the order a chooser offers them.
    pub const ALL: [ModSource; 6] = [
        ModSource::Off,
        ModSource::ModWheel,
        ModSource::Pressure,
        ModSource::Expression,
        ModSource::Breath,
        ModSource::Knob,
    ];

    /// The name a screen shows.
    pub const fn label(self) -> &'static str {
        match self {
            ModSource::Off => "Off",
            ModSource::ModWheel => "Mod wheel",
            ModSource::Pressure => "Pressure",
            ModSource::Expression => "Expression pedal",
            ModSource::Breath => "Breath",
            ModSource::Knob => "Learned knob",
        }
    }
}

/// The modifier's settings: which controller, and how far its top pushes
/// each setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modifier {
    pub source: ModSource,
    /// Rate steps along [`Rate::ALL`] (1/4 to 1/32T), -4 to 4: positive
    /// is faster. From 1/16, +2 reaches 1/32.
    pub rate: i8,
    /// Octaves added to the order's, -3 to 3 (the result stays 1 to 4).
    pub octaves: i8,
    /// Percent added to the gate, -99 to 99 (the result stays 1 to 100).
    pub gate: i8,
    /// Percent added to the swing, -25 to 25 (the result stays 50 to 75).
    pub swing: i8,
    /// Hits added to every step, 0 to 7: a step becomes a quick roll (a
    /// step's own ratchets and these come to 8 at most).
    pub rolls: u8,
    /// The chance, 0 to 100 %, that a step is accented: it plays at the
    /// accent velocity ([`Settings::accent`]).
    pub accents: u8,
}

impl Default for Modifier {
    /// Off, with amounts ready for when a controller is picked: two rate
    /// steps faster and an octave more at the top, like a synth's arp
    /// "intensity".
    fn default() -> Self {
        Modifier {
            source: ModSource::Off,
            rate: 2,
            octaves: 1,
            gate: 0,
            swing: 0,
            rolls: 0,
            accents: 0,
        }
    }
}

impl Modifier {
    pub const RATE: (i8, i8) = (-4, 4);
    pub const OCTAVES: (i8, i8) = (-3, 3);
    pub const GATE: (i8, i8) = (-99, 99);
    pub const SWING: (i8, i8) = (-25, 25);
    pub const ROLLS: u8 = 7;
    pub const ACCENTS: u8 = 100;

    /// Brings every amount into its range.
    pub fn clamped(mut self) -> Self {
        self.rate = self.rate.clamp(Self::RATE.0, Self::RATE.1);
        self.octaves = self.octaves.clamp(Self::OCTAVES.0, Self::OCTAVES.1);
        self.gate = self.gate.clamp(Self::GATE.0, Self::GATE.1);
        self.swing = self.swing.clamp(Self::SWING.0, Self::SWING.1);
        self.rolls = self.rolls.min(Self::ROLLS);
        self.accents = self.accents.min(Self::ACCENTS);
        self
    }

    /// Nothing can push the arp: no controller, or every amount 0.
    pub fn is_off(&self) -> bool {
        self.source == ModSource::Off
            || (self.rate == 0
                && self.octaves == 0
                && self.gate == 0
                && self.swing == 0
                && self.rolls == 0
                && self.accents == 0)
    }
}

/// How far past the middle between two steps a stepped push must go
/// before it moves, in steps: a controller resting on the edge stays put.
pub const HOLD: f32 = 0.15;

/// Where the stepped settings were last pushed, kept between pushes for
/// [`HOLD`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Steps {
    pub rate: i8,
    pub octaves: i8,
    pub rolls: i8,
}

/// The settings as the modifier pushes them, and its two extras.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pushed {
    /// The settings with rate, octaves, gate and swing pushed.
    pub settings: Settings,
    /// Hits added to every step.
    pub rolls: u8,
    /// The chance, in percent, that a step is accented.
    pub accents: u8,
}

/// The step a push of `target` steps lands on, from step `now`: it stays
/// on `now` until the target is more than half a step plus [`HOLD`] away.
fn stepped(target: f32, now: i8) -> i8 {
    if (target - f32::from(now)).abs() <= 0.5 + HOLD {
        now
    } else {
        target.round() as i8
    }
}

/// `base` as the modifier pushes it at `amount` (0 to 1), with `steps`
/// updated. A modifier that is off, or an amount of 0, gives `base` back
/// unchanged.
pub fn push(base: &Settings, amount: f32, steps: &mut Steps) -> Pushed {
    let m = base.modifier;
    let a = if m.source == ModSource::Off || !amount.is_finite() {
        0.0
    } else {
        amount.clamp(0.0, 1.0)
    };
    steps.rate = stepped(a * f32::from(m.rate), steps.rate);
    steps.octaves = stepped(a * f32::from(m.octaves), steps.octaves);
    steps.rolls = stepped(a * f32::from(m.rolls), steps.rolls).max(0);
    let mut s = *base;
    let at = Rate::ALL.iter().position(|r| *r == base.rate).unwrap_or(0) as i32;
    let last = Rate::ALL.len() as i32 - 1;
    s.rate = Rate::ALL[(at + i32::from(steps.rate)).clamp(0, last) as usize];
    s.octaves = (i32::from(base.octaves) + i32::from(steps.octaves)).clamp(1, 4) as u8;
    let add = |v: u8, by: i8, lo: f32, hi: f32| {
        (f32::from(v) + a * f32::from(by)).round().clamp(lo, hi) as u8
    };
    s.gate = add(base.gate, m.gate, 1.0, 100.0);
    s.swing = add(base.swing, m.swing, 50.0, 75.0);
    Pushed {
        settings: s,
        rolls: steps.rolls as u8,
        accents: (a * f32::from(m.accents)).round() as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(m: Modifier) -> Settings {
        Settings {
            modifier: Modifier {
                source: ModSource::ModWheel,
                ..m
            },
            ..Settings::default()
        }
    }

    fn at(s: &Settings, amount: f32) -> Pushed {
        push(s, amount, &mut Steps::default())
    }

    /// MPC 3.10.1 guide p. 84: from 1/8 with a Max Amount of 2 the
    /// controller reaches 1/8T and 1/16, "the next two higher time
    /// divisions"; an Extra Octaves of 2 on 1 octave reaches 3.
    #[test]
    fn the_top_of_the_controller_gives_the_whole_amount() {
        let s = Settings {
            rate: Rate::Eighth,
            octaves: 1,
            ..base(Modifier {
                rate: 2,
                octaves: 2,
                gate: 20,
                swing: 10,
                rolls: 3,
                accents: 40,
                ..Modifier::default()
            })
        };
        let p = at(&s, 1.0);
        assert_eq!(p.settings.rate, Rate::Sixteenth);
        assert_eq!(p.settings.octaves, 3);
        assert_eq!(p.settings.gate, 70);
        assert_eq!(p.settings.swing, 60);
        assert_eq!((p.rolls, p.accents), (3, 40));
        let mut s1 = Steps::default();
        let half = push(&s, 0.5, &mut s1);
        assert_eq!(half.settings.rate, Rate::EighthTriplet);
        assert_eq!(half.settings.gate, 60);
        assert_eq!(half.accents, 20);
        // At rest, the settings as set.
        let rest = at(&s, 0.0);
        assert_eq!(rest.settings, s);
        assert_eq!((rest.rolls, rest.accents), (0, 0));
    }

    #[test]
    fn off_or_a_bad_amount_pushes_nothing() {
        let mut s = base(Modifier {
            rate: 4,
            gate: 50,
            ..Modifier::default()
        });
        assert_eq!(at(&s, f32::NAN).settings, s);
        s.modifier.source = ModSource::Off;
        assert_eq!(at(&s, 1.0).settings, s);
        assert!(s.modifier.is_off());
        assert!(Modifier {
            source: ModSource::Pressure,
            rate: 0,
            octaves: 0,
            ..Modifier::default()
        }
        .is_off());
    }

    #[test]
    fn negative_amounts_slow_down_shorten_and_stay_in_range() {
        let s = Settings {
            rate: Rate::Eighth,
            octaves: 2,
            gate: 30,
            swing: 60,
            ..base(Modifier {
                rate: -4,
                octaves: -3,
                gate: -99,
                swing: -25,
                ..Modifier::default()
            })
        };
        let p = at(&s, 1.0);
        assert_eq!(p.settings.rate, Rate::Quarter, "no slower than 1/4");
        assert_eq!(p.settings.octaves, 1);
        assert_eq!(p.settings.gate, 1);
        assert_eq!(p.settings.swing, 50);
        let fast = Settings {
            rate: Rate::ThirtySecond,
            octaves: 4,
            ..base(Modifier {
                rate: 4,
                octaves: 3,
                ..Modifier::default()
            })
        };
        let p = at(&fast, 1.0);
        assert_eq!(p.settings.rate, Rate::ThirtySecondTriplet);
        assert_eq!(p.settings.octaves, 4);
    }

    /// A controller resting near a step's edge (pressure is never still)
    /// keeps the step it is on until it is clearly past the middle.
    #[test]
    fn a_stepped_push_holds_near_the_edge() {
        let s = base(Modifier {
            rate: 1,
            octaves: 0,
            ..Modifier::default()
        });
        let mut st = Steps::default();
        let rate = |a: f32, st: &mut Steps| push(&s, a, st).settings.rate;
        assert_eq!(rate(0.6, &mut st), Rate::Sixteenth);
        assert_eq!(rate(0.64, &mut st), Rate::Sixteenth);
        assert_eq!(rate(0.7, &mut st), Rate::SixteenthTriplet);
        // Wobbling back to just under the middle doesn't come down.
        assert_eq!(rate(0.45, &mut st), Rate::SixteenthTriplet);
        assert_eq!(rate(0.36, &mut st), Rate::SixteenthTriplet);
        assert_eq!(rate(0.3, &mut st), Rate::Sixteenth);
        // A jump goes straight there, and 0 is always the setting.
        let s4 = base(Modifier {
            rate: 4,
            ..Modifier::default()
        });
        let mut st = Steps::default();
        assert_eq!(
            push(&s4, 1.0, &mut st).settings.rate,
            Rate::ThirtySecondTriplet
        );
        assert_eq!(push(&s4, 0.0, &mut st).settings.rate, Rate::Sixteenth);
    }

    /// Any settings, any amounts and a controller wobbling anywhere: the
    /// pushed settings stay in their ranges, a push never moves a setting
    /// the other way from its amount, and back at 0 the arp plays exactly
    /// as set, whatever the steps held on the way.
    #[test]
    fn any_push_stays_in_range_and_comes_back_to_the_settings() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..2_000 {
            let m = Modifier {
                source: ModSource::ALL[1 + next(5) as usize],
                rate: next(9) as i8 - 4,
                octaves: next(7) as i8 - 3,
                gate: (next(199) as i32 - 99) as i8,
                swing: next(51) as i8 - 25,
                rolls: next(8) as u8,
                accents: next(101) as u8,
            };
            let s = Settings {
                rate: Rate::ALL[next(Rate::ALL.len() as u64) as usize],
                octaves: 1 + next(4) as u8,
                gate: 1 + next(100) as u8,
                swing: 50 + next(26) as u8,
                modifier: m,
                ..Settings::default()
            };
            let at = |r: Rate| Rate::ALL.iter().position(|x| *x == r).unwrap() as i32;
            let mut st = Steps::default();
            for _ in 0..20 {
                let a = next(1_001) as f32 / 1_000.0;
                let p = push(&s, a, &mut st);
                let q = p.settings;
                assert!((1..=4).contains(&q.octaves), "{q:?}");
                assert!((1..=100).contains(&q.gate), "{q:?}");
                assert!((50..=75).contains(&q.swing), "{q:?}");
                assert!(p.rolls <= Modifier::ROLLS && p.accents <= m.accents);
                let way = |d: i32, by: i8| d == 0 || d.signum() == i32::from(by.signum());
                assert!(way(at(q.rate) - at(s.rate), m.rate), "{s:?} {q:?}");
                assert!(way(i32::from(q.octaves) - i32::from(s.octaves), m.octaves));
                assert!(way(i32::from(q.gate) - i32::from(s.gate), m.gate));
                assert!(way(i32::from(q.swing) - i32::from(s.swing), m.swing));
                // Everything but the four pushed settings stays as set.
                let mut same = q;
                same.rate = s.rate;
                same.octaves = s.octaves;
                same.gate = s.gate;
                same.swing = s.swing;
                assert_eq!(same, s);
            }
            let rest = push(&s, 0.0, &mut st);
            assert_eq!(rest.settings, s);
            assert_eq!((rest.rolls, rest.accents), (0, 0));
            assert_eq!(st, Steps::default());
        }
    }

    #[test]
    fn amounts_are_clamped() {
        let m = Modifier {
            source: ModSource::Knob,
            rate: 9,
            octaves: -9,
            gate: 120,
            swing: -40,
            rolls: 30,
            accents: 200,
        }
        .clamped();
        assert_eq!(
            (m.rate, m.octaves, m.gate, m.swing, m.rolls, m.accents),
            (4, -3, 99, -25, 7, 100)
        );
    }
}
