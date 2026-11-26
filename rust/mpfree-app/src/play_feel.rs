//! Prefs › Playing: how the pads and knobs feel (layout spec section 5).
//! The custom velocity curve, the pad threshold, and how far a turn of a
//! Q-Link or the data wheel goes. Pure rules; the main loop applies
//! [`Turns::event`] to what the MPC sends before anything else reads it.
//!
//! **Read-only for the MPC:** nothing here sends anything to it. The
//! defaults (Normal, Normal, Off, threshold 1) leave every event as the
//! MPC sent it.

use mpfree_controller::mpc_one::ControlEvent;
use std::time::Instant;

/// The custom velocity curve: a bend and the softest and loudest notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bend {
    /// -100 (hard: loud notes need hard hits) to +100 (soft); 0 is
    /// straight.
    pub shape: i8,
    /// The velocity the lightest hit plays (1 to 127).
    pub softest: u8,
    /// The velocity the hardest hit plays (1 to 127).
    pub loudest: u8,
}

impl Default for Bend {
    fn default() -> Bend {
        Bend {
            shape: 0,
            softest: 1,
            loudest: 127,
        }
    }
}

impl Bend {
    pub const SHAPE_STEP: i8 = 10;
    pub const VELOCITY_STEP: u8 = 4;

    /// The velocity a hit at `v` plays: the bend (+50 bends as far as
    /// Soft's, -50 a little further than Hard's), then spread from the
    /// softest to the loudest note.
    pub fn apply(self, v: u8) -> u8 {
        let x = f32::from(v.clamp(1, 127) - 1) / 126.0;
        let y = x.powf(2f32.powf(-f32::from(self.shape.clamp(-100, 100)) / 50.0));
        let lo = f32::from(self.softest.min(self.loudest));
        let hi = f32::from(self.loudest.max(self.softest));
        (lo + y * (hi - lo)).round().clamp(1.0, 127.0) as u8
    }

    /// The shape as the dial says it: "+30", "0", "-20".
    pub fn shape_text(self) -> String {
        match self.shape {
            0 => "0".to_string(),
            s => format!("{s:+}"),
        }
    }
}

/// How far a turn goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Speed {
    /// Half as far.
    Slow,
    /// As the MPC sends it (the default).
    #[default]
    Normal,
    /// Twice as far.
    Fast,
}

impl Speed {
    pub const ALL: [Speed; 3] = [Speed::Slow, Speed::Normal, Speed::Fast];
    pub fn name(self) -> &'static str {
        ["Slow", "Normal", "Fast"][self as usize]
    }
    pub fn key(self) -> &'static str {
        ["slow", "normal", "fast"][self as usize]
    }
    pub fn parse(v: &str) -> Option<Speed> {
        Speed::ALL.into_iter().find(|s| s.key() == v.trim())
    }
    fn factor(self) -> f32 {
        [0.5, 1.0, 2.0][self as usize]
    }
}

/// Whether a quick turn of the wheel goes further.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpeedUp {
    /// Every click goes as far (the default).
    #[default]
    Off,
    /// A quick spin goes up to three times as far.
    Some,
    /// Up to six times.
    More,
}

impl SpeedUp {
    pub const ALL: [SpeedUp; 3] = [SpeedUp::Off, SpeedUp::Some, SpeedUp::More];
    pub fn name(self) -> &'static str {
        ["Off", "Some", "More"][self as usize]
    }
    pub fn key(self) -> &'static str {
        ["off", "some", "more"][self as usize]
    }
    pub fn parse(v: &str) -> Option<SpeedUp> {
        SpeedUp::ALL.into_iter().find(|s| s.key() == v.trim())
    }
    /// The most a click is multiplied by, at the quickest spin.
    fn most(self) -> f32 {
        [1.0, 3.0, 6.0][self as usize]
    }
}

/// Clicks further apart than this go as far as a slow turn's.
const QUICK_MS: f32 = 80.0;

/// The pad threshold's range: 1 plays every hit.
pub const THRESHOLD_MIN: u8 = 1;
pub const THRESHOLD_MAX: u8 = 40;

/// How the pads and knobs feel, as Prefs sets it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Feel {
    pub qlink: Speed,
    pub wheel: Speed,
    pub speed_up: SpeedUp,
    /// Hits softer than this (raw velocity) play nothing.
    pub threshold: u8,
}

/// What the main loop keeps between events: the parts of a click a slow
/// speed hasn't turned yet, when the wheel last clicked, and the pads
/// whose hit was under the threshold (their let-go and pressure are left
/// out too).
#[derive(Debug, Default)]
pub struct Turns {
    qlink_rest: [f32; 4],
    wheel_rest: f32,
    wheel_at: Option<Instant>,
    under: [bool; 16],
}

impl Turns {
    /// `e` as the rest of the app should see it: a turn sped up or
    /// slowed (None while a slow turn hasn't made a whole click), a hit
    /// under the threshold left out with its let-go and pressure.
    pub fn event(
        &mut self,
        e: Option<ControlEvent>,
        feel: &Feel,
        now: Instant,
    ) -> Option<ControlEvent> {
        match e? {
            ControlEvent::PadDown { pad, velocity } if (pad as usize) < 16 => {
                let under = velocity < feel.threshold;
                self.under[pad as usize] = under;
                (!under).then_some(ControlEvent::PadDown { pad, velocity })
            }
            ControlEvent::PadUp { pad } if (pad as usize) < 16 && self.under[pad as usize] => {
                self.under[pad as usize] = false;
                None
            }
            ControlEvent::PadPressure { pad, .. }
                if (pad as usize) < 16 && self.under[pad as usize] =>
            {
                None
            }
            ControlEvent::QLinkTurn { knob, delta } if (knob as usize) < 4 => {
                let rest = &mut self.qlink_rest[knob as usize];
                let delta = scaled(rest, delta, feel.qlink.factor());
                (delta != 0).then_some(ControlEvent::QLinkTurn { knob, delta })
            }
            ControlEvent::DataWheel { delta } => {
                let gap = self
                    .wheel_at
                    .replace(now)
                    .map_or(f32::MAX, |at| now.duration_since(at).as_secs_f32() * 1000.0);
                let quick = ((QUICK_MS - gap) / QUICK_MS).clamp(0.0, 1.0);
                let factor = feel.wheel.factor() * (1.0 + quick * (feel.speed_up.most() - 1.0));
                let delta = scaled(&mut self.wheel_rest, delta, factor);
                (delta != 0).then_some(ControlEvent::DataWheel { delta })
            }
            other => Some(other),
        }
    }
}

/// `delta` times `factor`, with what is left of a whole click carried in
/// `rest` to the next (dropped when the turn changes direction).
fn scaled(rest: &mut f32, delta: i8, factor: f32) -> i8 {
    if factor == 1.0 {
        *rest = 0.0;
        return delta;
    }
    if *rest != 0.0 && rest.signum() != f32::from(delta).signum() {
        *rest = 0.0;
    }
    let want = *rest + f32::from(delta) * factor;
    let whole = want.trunc();
    *rest = want - whole;
    whole.clamp(-127.0, 127.0) as i8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_straight_bend_plays_the_pad_and_the_ends_spread_it() {
        let b = Bend::default();
        for v in 1..=127 {
            assert_eq!(b.apply(v), v);
        }
        let soft = Bend {
            shape: 50,
            ..Bend::default()
        };
        assert!(soft.apply(64) > 64 && soft.apply(64) < 127);
        let hard = Bend {
            shape: -50,
            ..Bend::default()
        };
        assert!(hard.apply(64) < 64 && hard.apply(64) > 1);
        let narrow = Bend {
            shape: 30,
            softest: 10,
            loudest: 100,
        };
        assert_eq!((narrow.apply(1), narrow.apply(127)), (10, 100));
        // Softest over loudest: the two swap.
        let crossed = Bend {
            shape: 0,
            softest: 100,
            loudest: 20,
        };
        assert_eq!((crossed.apply(1), crossed.apply(127)), (20, 100));
        assert_eq!(soft.shape_text(), "+50");
        assert_eq!(hard.shape_text(), "-50");
        assert_eq!(b.shape_text(), "0");
    }

    #[test]
    fn the_defaults_leave_every_event_as_it_came() {
        let mut t = Turns::default();
        let feel = Feel {
            threshold: THRESHOLD_MIN,
            ..Feel::default()
        };
        let now = Instant::now();
        for e in [
            ControlEvent::PadDown {
                pad: 3,
                velocity: 1,
            },
            ControlEvent::PadUp { pad: 3 },
            ControlEvent::QLinkTurn { knob: 1, delta: -2 },
            ControlEvent::DataWheel { delta: 1 },
            ControlEvent::DataWheel { delta: 1 },
        ] {
            assert_eq!(t.event(Some(e), &feel, now), Some(e));
        }
    }

    #[test]
    fn a_hit_under_the_threshold_plays_nothing_nor_its_let_go() {
        let mut t = Turns::default();
        let feel = Feel {
            threshold: 10,
            ..Feel::default()
        };
        let now = Instant::now();
        let down = |velocity| Some(ControlEvent::PadDown { pad: 5, velocity });
        assert_eq!(t.event(down(9), &feel, now), None);
        let pressure = Some(ControlEvent::PadPressure {
            pad: 5,
            pressure: 40,
        });
        assert_eq!(t.event(pressure, &feel, now), None);
        assert_eq!(
            t.event(Some(ControlEvent::PadUp { pad: 5 }), &feel, now),
            None
        );
        assert_eq!(t.event(down(10), &feel, now), down(10));
        assert_eq!(t.event(pressure, &feel, now), pressure);
        let up = Some(ControlEvent::PadUp { pad: 5 });
        assert_eq!(t.event(up, &feel, now), up);
    }

    #[test]
    fn slow_turns_carry_half_clicks_and_fast_double_them() {
        let mut t = Turns::default();
        let slow = Feel {
            qlink: Speed::Slow,
            ..Feel::default()
        };
        let now = Instant::now();
        let turn = |delta| Some(ControlEvent::QLinkTurn { knob: 0, delta });
        assert_eq!(t.event(turn(1), &slow, now), None);
        assert_eq!(t.event(turn(1), &slow, now), turn(1));
        assert_eq!(t.event(turn(3), &slow, now), turn(1));
        // The half click left over goes when the turn changes direction.
        assert_eq!(t.event(turn(-1), &slow, now), None);
        assert_eq!(t.event(turn(-1), &slow, now), turn(-1));
        let fast = Feel {
            qlink: Speed::Fast,
            ..Feel::default()
        };
        assert_eq!(t.event(turn(-2), &fast, now), turn(-4));
    }

    #[test]
    fn a_quick_spin_of_the_wheel_goes_further_with_speed_up() {
        let feel = Feel {
            speed_up: SpeedUp::More,
            ..Feel::default()
        };
        let wheel = |delta| Some(ControlEvent::DataWheel { delta });
        let start = Instant::now();
        let mut t = Turns::default();
        // A slow turn: each click goes one.
        for n in 0..4u64 {
            let at = start + Duration::from_millis(200 * n);
            assert_eq!(t.event(wheel(1), &feel, at), wheel(1));
        }
        // A quick spin: clicks 10 ms apart go about five each.
        let mut went = 0i32;
        for n in 1..=10u64 {
            let at = start + Duration::from_millis(800 + 10 * n);
            if let Some(ControlEvent::DataWheel { delta }) = t.event(wheel(1), &feel, at) {
                went += i32::from(delta);
            }
        }
        assert!((40..=60).contains(&went), "{went}");
    }
}
