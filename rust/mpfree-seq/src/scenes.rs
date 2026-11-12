//! Scenes and follow actions (roadmap T9).
//!
//! A **scene** is a row of the launch grid: scene `k` is slot `k` of every
//! track, so there are as many scenes as slots ([`SCENES`]). Launching it
//! plays every track's clip in that row from the next bar line, and stops
//! a track whose cell is empty (a scene is a whole state of the song).
//! A scene can also hold:
//!
//! - **mutes:** the tracks muted while it plays, switched on the same bar
//!   line (the MPC's Track Mute, as an event mute in the sequencer). A
//!   scene with none leaves the mutes as they are.
//! - **a follow action:** after it has played some bars, another scene
//!   launches (the next, the first, any ...), so scenes can chain into a
//!   song without an arrangement.
//! - **values:** parameter values stored with it, for the scene morph
//!   (T10): two scenes on the crossfader morph every parameter they hold
//!   between their values.
//!
//! A clip can have a follow action of its own: after some bars, its track
//! plays another clip (the next slot down, any other ...) or stops.
//!
//! These are plain values, checked by [`Scene::check`] and
//! [`Follow::check`]; `mpfree-perform` carries them out.

use mpfree_param::ParamAddr;

use crate::{ModelError, MAX_TRACKS, SLOTS};

/// Scenes in a project: one per row of slots.
pub const SCENES: usize = SLOTS;
/// The longest a follow action waits, in bars.
pub const MAX_FOLLOW_BARS: u16 = 999;
/// Values one scene can store for the morph.
pub const MAX_SCENE_VALUES: usize = 1024;

/// What a follow action does when it comes due.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FollowAction {
    /// Keep playing as before.
    Stay,
    /// Play the same again from its start.
    Again,
    /// The next one down (a clip: the next filled slot on its track;
    /// a scene: the next row), round to the first after the last.
    Next,
    /// The one before, round to the last before the first.
    Previous,
    First,
    Last,
    /// Any, this one included.
    Any,
    /// Any but this one.
    Other,
    /// Stop (a clip: its track; a scene: every track).
    Stop,
}

/// After `bars` bars (counted from where it started), `action` with a
/// chance of `chance` % and otherwise `other`. Ableton's A and B.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Follow {
    pub bars: u16,
    pub action: FollowAction,
    pub other: FollowAction,
    /// 0 to 100: how often `action` is picked over `other`.
    pub chance: u8,
}

impl Follow {
    /// After `bars` bars, always `action`.
    pub fn after(bars: u16, action: FollowAction) -> Follow {
        Follow {
            bars,
            action,
            other: FollowAction::Stay,
            chance: 100,
        }
    }

    pub fn check(&self) -> Result<(), ModelError> {
        if self.bars == 0 || self.bars > MAX_FOLLOW_BARS || self.chance > 100 {
            return Err(ModelError::BadFollow);
        }
        Ok(())
    }
}

/// A row of the launch grid, and what it holds besides its clips.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    /// Empty: shown as its number.
    pub name: String,
    /// Tracks muted while it plays (bit `t` is track `t`), switched when
    /// it starts; `None` leaves them as they are.
    pub mutes: Option<u16>,
    pub follow: Option<Follow>,
    /// Parameter values (in each parameter's own units) for the morph.
    pub values: Vec<(ParamAddr, f32)>,
}

impl Scene {
    pub fn named(name: &str) -> Scene {
        Scene {
            name: name.to_string(),
            ..Scene::default()
        }
    }

    /// Whether track `t` is muted while this scene plays, or `None` when
    /// the scene leaves it as it is.
    pub fn mutes_track(&self, t: usize) -> Option<bool> {
        let m = self.mutes?;
        Some(t < 16 && m & (1 << t) != 0)
    }

    pub fn check(&self) -> Result<(), ModelError> {
        if let Some(f) = &self.follow {
            f.check()?;
        }
        if self
            .mutes
            .is_some_and(|m| MAX_TRACKS < 16 && m >> MAX_TRACKS != 0)
        {
            return Err(ModelError::BadScene);
        }
        if self.values.len() > MAX_SCENE_VALUES || self.values.iter().any(|v| !v.1.is_finite()) {
            return Err(ModelError::BadScene);
        }
        // One value a parameter.
        for (i, a) in self.values.iter().enumerate() {
            if self.values[..i].iter().any(|b| b.0 == a.0) {
                return Err(ModelError::BadScene);
            }
        }
        Ok(())
    }

    /// Stores `value` for `addr`, in place of the one it had.
    pub fn set_value(&mut self, addr: ParamAddr, value: f32) -> Result<(), ModelError> {
        if !value.is_finite() {
            return Err(ModelError::BadScene);
        }
        if let Some(v) = self.values.iter_mut().find(|v| v.0 == addr) {
            v.1 = value;
        } else if self.values.len() == MAX_SCENE_VALUES {
            return Err(ModelError::BadScene);
        } else {
            self.values.push((addr, value));
        }
        Ok(())
    }

    /// The value it stores for `addr`.
    pub fn value(&self, addr: ParamAddr) -> Option<f32> {
        self.values.iter().find(|v| v.0 == addr).map(|v| v.1)
    }
}

/// Where a follow action goes from `at` (a slot or a row) among `filled`
/// (which are there to go to), with `roll` a random number for Any and
/// Other. `None`: stop. `Some(at)` with [`FollowAction::Stay`]: nothing
/// changes (the caller does nothing). No allocation.
pub fn follow_target(action: FollowAction, at: usize, filled: &[bool], roll: u64) -> Option<usize> {
    let have = || (0..filled.len()).filter(|&i| filled[i]);
    let n = have().count();
    if n == 0 {
        return None;
    }
    match action {
        FollowAction::Stay | FollowAction::Again => Some(at),
        FollowAction::Stop => None,
        FollowAction::First => have().next(),
        FollowAction::Last => have().next_back(),
        FollowAction::Next => have().find(|&i| i > at).or(have().next()),
        FollowAction::Previous => have().rfind(|&i| i < at).or(have().next_back()),
        FollowAction::Any => have().nth((roll % n as u64) as usize),
        FollowAction::Other => {
            let others = || have().filter(|&i| i != at);
            match others().count() {
                0 => Some(at),
                m => others().nth((roll % m as u64) as usize),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_param::Owner;

    #[test]
    fn follow_targets_wrap_and_skip_empty_cells() {
        let f = [true, false, true, true, false];
        use FollowAction::*;
        assert_eq!(follow_target(Next, 0, &f, 0), Some(2));
        assert_eq!(follow_target(Next, 3, &f, 0), Some(0), "round to the first");
        assert_eq!(
            follow_target(Previous, 0, &f, 0),
            Some(3),
            "round to the last"
        );
        assert_eq!(follow_target(Previous, 2, &f, 0), Some(0));
        assert_eq!(follow_target(First, 3, &f, 0), Some(0));
        assert_eq!(follow_target(Last, 0, &f, 0), Some(3));
        assert_eq!(follow_target(Again, 2, &f, 0), Some(2));
        assert_eq!(follow_target(Stop, 2, &f, 0), None);
        for roll in 0..20 {
            let o = follow_target(Other, 2, &f, roll).unwrap();
            assert!(o == 0 || o == 3);
            assert!(f[follow_target(Any, 2, &f, roll).unwrap()]);
        }
        assert_eq!(follow_target(Other, 0, &[true], 5), Some(0), "nothing else");
        assert_eq!(follow_target(Next, 0, &[false, false], 0), None);
    }

    #[test]
    fn scenes_check_their_values() {
        let mut s = Scene::named("Verse");
        assert_eq!(s.check(), Ok(()));
        s.follow = Some(Follow::after(0, FollowAction::Next));
        assert_eq!(s.check(), Err(ModelError::BadFollow));
        s.follow = Some(Follow {
            chance: 101,
            ..Follow::after(4, FollowAction::Next)
        });
        assert_eq!(s.check(), Err(ModelError::BadFollow));
        s.follow = Some(Follow::after(4, FollowAction::Next));
        let a = ParamAddr::new(Owner::Inst(0), 1);
        s.set_value(a, 0.5).unwrap();
        s.set_value(a, 0.7).unwrap();
        assert_eq!(s.values.len(), 1);
        assert_eq!(s.value(a), Some(0.7));
        assert_eq!(s.set_value(a, f32::NAN), Err(ModelError::BadScene));
        s.values.push((a, 0.1));
        assert_eq!(
            s.check(),
            Err(ModelError::BadScene),
            "one value a parameter"
        );
        s.values.pop();
        s.mutes = Some(0b101);
        assert_eq!(s.mutes_track(0), Some(true));
        assert_eq!(s.mutes_track(1), Some(false));
        assert_eq!(s.check(), Ok(()));
    }
}
