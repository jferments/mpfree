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
//! - **programs:** a program change per track (sequencer S5, the
//!   Elektron's program change per pattern): launching the scene sends
//!   each track's on the scene's line, to the outside gear its track plays
//!   on, unless the clip the track starts there sends one of its own.
//!
//! A clip can have a follow action of its own: after some bars, its track
//! plays another clip (the next slot down, any other ...) or stops.
//!
//! The **song list** (sequencer S4, the MPC's song mode) plays scenes in
//! an order, each so many times ("Intro ×2, Verse ×4, Drop ×1"), then
//! stops or goes round again: a [`SongList`] of [`ListStep`]s, kept with
//! the arrangement (`Arrangement::list`). A scene can come in it more than
//! once, which follow actions can't say, so the list plays on its own,
//! in place of the scenes' follow actions while it plays.
//!
//! A linked list (the Arrange refactor's phase 4b, plan D8: one song, two
//! views) has no steps of its own: it is the song's sections, each a
//! marker's, with its scene and passes on the marker
//! (`arrange::song_sections`), so the list and the Song view show the
//! same thing. A list saved before is not linked: it keeps its own steps.
//!
//! These are plain values, checked by [`Scene::check`],
//! [`Follow::check`] and [`SongList::check`]; `mpfree-perform` carries
//! them out.

use mpfree_param::ParamAddr;

use crate::{ClipProgram, ModelError, MAX_TRACKS, ROWS};

/// Scenes in a project: one per row it keeps ([`ROWS`], the Arrange
/// refactor's 1.2), kept as a list as long as the last one set up.
pub const SCENES: usize = ROWS;
/// The longest a follow action waits, in bars.
pub const MAX_FOLLOW_BARS: u16 = 999;
/// Values one scene can store for the morph.
pub const MAX_SCENE_VALUES: usize = 1024;
/// Steps a song list holds.
pub const MAX_LIST_STEPS: usize = 128;
/// The most times a step plays its scene.
pub const MAX_TIMES: u16 = 99;
/// The most characters a scene's name has (the scene editor's Rename).
pub const MAX_SCENE_NAME: usize = 24;

/// One step of the song list: scene `row` played `times` times, one
/// after another (a pass is the scene's length, `arrange::scene_length`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ListStep {
    pub row: usize,
    pub times: u16,
}

/// The song list: see the module notes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SongList {
    /// Its own steps, when it isn't linked (none when it is).
    pub steps: Vec<ListStep>,
    /// After the last step the first plays again; otherwise every track
    /// stops.
    pub looped: bool,
    /// Linked to the song (the Arrange refactor's phase 4b): its steps are
    /// the song's sections (`arrange::song_sections`), not its own.
    pub linked: bool,
}

impl SongList {
    /// No more than [`MAX_LIST_STEPS`] steps, each a scene in range played
    /// 1 to [`MAX_TIMES`] times, and none when it is linked.
    pub fn check(&self) -> Result<(), ModelError> {
        if self.steps.len() > MAX_LIST_STEPS
            || (self.linked && !self.steps.is_empty())
            || self
                .steps
                .iter()
                .any(|s| s.row >= SCENES || s.times == 0 || s.times > MAX_TIMES)
        {
            return Err(ModelError::BadScene);
        }
        Ok(())
    }

    /// The step after `step`: the next, or the first again when the list
    /// goes round; None after the last (the list stops).
    pub fn after(&self, step: usize) -> Option<usize> {
        if step + 1 < self.steps.len() {
            Some(step + 1)
        } else if self.looped && !self.steps.is_empty() {
            Some(0)
        } else {
            None
        }
    }
}

/// The program change a scene sends one track as it starts (sequencer
/// S5): see the module notes. A scene holds at most one a track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrackProgram {
    pub track: usize,
    pub program: ClipProgram,
}

/// A scene's programs in order of track, one a track, each in range.
fn check_programs(programs: &[TrackProgram]) -> Result<(), ModelError> {
    let in_order = programs.windows(2).all(|w| w[0].track < w[1].track);
    if !in_order
        || programs
            .iter()
            .any(|p| p.track >= MAX_TRACKS || !p.program.ok())
    {
        return Err(ModelError::BadScene);
    }
    Ok(())
}

/// Track `track`'s program in `programs`.
fn program_of(programs: &[TrackProgram], track: usize) -> Option<ClipProgram> {
    programs
        .iter()
        .find(|p| p.track == track)
        .map(|p| p.program)
}

/// Give track `track` the program `to` in `programs`, or none, keeping
/// them in order of track.
fn put_program(programs: &mut Vec<TrackProgram>, track: usize, to: Option<ClipProgram>) {
    let k = programs.partition_point(|p| p.track < track);
    let here = programs.get(k).is_some_and(|p| p.track == track);
    match (to, here) {
        (Some(program), true) => programs[k].program = program,
        (Some(program), false) => programs.insert(k, TrackProgram { track, program }),
        (None, true) => {
            programs.remove(k);
        }
        (None, false) => {}
    }
}

/// A scene's name, mutes, follow action and programs: what the scene
/// editor (the Record panel's Scene tab, sequencer S4 and S5) sets, apart
/// from its morph values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneSettings {
    pub name: String,
    pub mutes: Option<u128>,
    pub follow: Option<Follow>,
    /// The program changes it sends as it starts, in order of track.
    pub programs: Vec<TrackProgram>,
}

impl SceneSettings {
    /// Track `track`'s program change in the scene, if it has one.
    pub fn program(&self, track: usize) -> Option<ClipProgram> {
        program_of(&self.programs, track)
    }

    /// Track `track` gets the program change `to` (None: none).
    pub fn set_program(&mut self, track: usize, to: Option<ClipProgram>) {
        put_program(&mut self.programs, track, to);
    }

    pub fn check(&self) -> Result<(), ModelError> {
        if let Some(f) = &self.follow {
            f.check()?;
        }
        check_programs(&self.programs)?;
        if self
            .mutes
            .and_then(|m| m.checked_shr(MAX_TRACKS as u32))
            .is_some_and(|rest| rest != 0)
            || self.name.chars().count() > MAX_SCENE_NAME
            || self.name.chars().any(char::is_control)
        {
            return Err(ModelError::BadScene);
        }
        Ok(())
    }
}

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
    /// Tracks muted while it plays (bit `t` is track `t`, one bit for
    /// each of [`MAX_TRACKS`]), switched when it starts; `None` leaves
    /// them as they are.
    pub mutes: Option<u128>,
    pub follow: Option<Follow>,
    /// Parameter values (in each parameter's own units) for the morph.
    pub values: Vec<(ParamAddr, f32)>,
    /// Program changes it sends as it starts, in order of track, one a
    /// track (sequencer S5).
    pub programs: Vec<TrackProgram>,
}

impl Scene {
    pub fn named(name: &str) -> Scene {
        Scene {
            name: name.to_string(),
            ..Scene::default()
        }
    }

    /// What the scene editor sets (sequencer S4 and S5): its name, mutes,
    /// follow action and programs.
    pub fn settings(&self) -> SceneSettings {
        SceneSettings {
            name: self.name.clone(),
            mutes: self.mutes,
            follow: self.follow,
            programs: self.programs.clone(),
        }
    }

    /// Give it `s`, its values (the morph's) as they are. `BadScene`, and
    /// nothing changed, for a follow out of range, mutes past the last
    /// track, a name too long or with a line break, or programs out of
    /// order or range.
    pub fn set_settings(&mut self, s: &SceneSettings) -> Result<(), ModelError> {
        s.check()?;
        self.name.clone_from(&s.name);
        self.mutes = s.mutes;
        self.follow = s.follow;
        self.programs.clone_from(&s.programs);
        Ok(())
    }

    /// Track `track`'s program change in this scene, if it has one.
    pub fn program(&self, track: usize) -> Option<ClipProgram> {
        program_of(&self.programs, track)
    }

    /// Track `track` gets the program change `to` (None: none).
    pub fn set_program(&mut self, track: usize, to: Option<ClipProgram>) -> Result<(), ModelError> {
        if track >= MAX_TRACKS || to.is_some_and(|p| !p.ok()) {
            return Err(ModelError::BadScene);
        }
        put_program(&mut self.programs, track, to);
        Ok(())
    }

    /// Whether track `t` is muted while this scene plays, or `None` when
    /// the scene leaves it as it is.
    pub fn mutes_track(&self, t: usize) -> Option<bool> {
        let m = self.mutes?;
        Some(t < MAX_TRACKS && m & (1 << t) != 0)
    }

    pub fn check(&self) -> Result<(), ModelError> {
        if let Some(f) = &self.follow {
            f.check()?;
        }
        check_programs(&self.programs)?;
        if self
            .mutes
            .and_then(|m| m.checked_shr(MAX_TRACKS as u32))
            .is_some_and(|rest| rest != 0)
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

    /// The scene editor's settings: set together, the morph values left
    /// alone; a bad one changes nothing.
    #[test]
    fn scene_settings_set_name_mutes_follow_and_programs() {
        let a = ParamAddr::new(Owner::Inst(0), 1);
        let mut s = Scene::named("Verse");
        s.set_value(a, 0.5).unwrap();
        let pc = |program, bank| ClipProgram { program, bank };
        let want = SceneSettings {
            name: "Drop".into(),
            mutes: Some(0b10),
            follow: Some(Follow::after(8, FollowAction::Next)),
            programs: vec![TrackProgram {
                track: 3,
                program: pc(9, Some(1)),
            }],
        };
        s.set_settings(&want).unwrap();
        assert_eq!(s.settings(), want);
        assert_eq!(s.value(a), Some(0.5), "the morph's value stays");
        for bad in [
            SceneSettings {
                name: "x".repeat(MAX_SCENE_NAME + 1),
                ..want.clone()
            },
            SceneSettings {
                name: "a\nb".into(),
                ..want.clone()
            },
            SceneSettings {
                follow: Some(Follow::after(0, FollowAction::Next)),
                ..want.clone()
            },
            SceneSettings {
                programs: vec![TrackProgram {
                    track: MAX_TRACKS,
                    program: pc(9, None),
                }],
                ..want.clone()
            },
            SceneSettings {
                programs: vec![TrackProgram {
                    track: 0,
                    program: pc(128, None),
                }],
                ..want.clone()
            },
            SceneSettings {
                programs: vec![
                    TrackProgram {
                        track: 4,
                        program: pc(1, None),
                    },
                    TrackProgram {
                        track: 4,
                        program: pc(2, None),
                    },
                ],
                ..want.clone()
            },
        ] {
            assert!(s.set_settings(&bad).is_err(), "{bad:?}");
        }
        assert_eq!(s.settings(), want);
    }

    /// One program a track, kept in order of track; none takes it off.
    #[test]
    fn a_scenes_programs_stay_in_order_one_a_track() {
        let pc = |program| {
            Some(ClipProgram {
                program,
                bank: None,
            })
        };
        let mut s = SceneSettings::default();
        s.set_program(5, pc(50));
        s.set_program(1, pc(10));
        s.set_program(3, pc(30));
        s.set_program(3, pc(31));
        let tracks: Vec<_> = s.programs.iter().map(|p| p.track).collect();
        assert_eq!(tracks, [1, 3, 5]);
        assert_eq!(s.program(3), pc(31));
        assert_eq!(s.program(2), None);
        assert_eq!(s.check(), Ok(()));
        s.set_program(1, None);
        s.set_program(2, None);
        assert_eq!(s.programs.len(), 2);
        let mut sc = Scene::named("Verse");
        sc.set_settings(&s).unwrap();
        assert_eq!(sc.program(5), pc(50));
        assert_eq!(sc.set_program(MAX_TRACKS, pc(1)), Err(ModelError::BadScene));
        assert_eq!(sc.set_program(0, pc(128)), Err(ModelError::BadScene));
        sc.set_program(0, pc(7)).unwrap();
        assert_eq!(sc.programs[0].track, 0);
        assert_eq!(sc.check(), Ok(()));
        // Out of order: refused by the check (a file can't make one).
        sc.programs.swap(0, 1);
        assert_eq!(sc.check(), Err(ModelError::BadScene));
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
        // The last of 128 tracks has its bit too.
        s.mutes = Some(1 << 127 | 1);
        assert_eq!(s.mutes_track(127), Some(true));
        assert_eq!(s.mutes_track(126), Some(false));
        assert_eq!(s.mutes_track(128), Some(false));
        assert_eq!(s.check(), Ok(()));
    }
}
