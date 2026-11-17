//! The time engine's model: a project holds tracks, a track holds clips in
//! 16 slots, a clip holds notes. Plain data, edited only on the control
//! thread and compiled into a read-only play view by `mpfree-play`.
//!
//! Everything is a clip: a step pattern, a recorded phrase and (later) a
//! looper take are the same thing, and the step grid is only a view. A clip
//! has any length, so clips of different lengths drift against each other
//! (polymeter). Recorded ticks stay exact; quantise is a clip setting
//! applied on playback, so it can be changed or undone later.
//!
//! A note has a chance to play on each pass. [`steps`] is the step
//! sequencer's view of a clip: 16 steps to a page, each step the notes of
//! one sound nearest its grid line, with velocity, length, nudge and
//! chance.
//!
//! A clip also holds **parameter locks** ([`Lock`]): a sound parameter set
//! on one step, the Elektron way. A lock sits on a step like a note (it
//! shares its note's tick, so it moves with the step's nudge) and lasts
//! until the track's next note without a lock on that parameter; the
//! player (`mpfree-play`) sends it on its frame, before the note-on. A lock
//! can also sit on a step with no note (a trigless lock). Locks name their
//! parameter without the track ([`mpfree_param::RelAddr`]), so a clip
//! copied to another track locks that track's sound.
//!
//! A note can also have a **trig condition** ([`Cond`]: play on pass A of
//! every B, only during a fill, only if the track's last conditional note
//! played, and so on) and a **ratchet** (it plays several times, evenly
//! spread over its length). The player (`mpfree-play`) decides them.
//!
//! **Scenes** ([`scenes`]) are the rows of the launch grid: launched
//! together, with mutes, follow actions and stored values for the morph.
//! A clip can have a follow action too ([`Clip::follow`]).
//!
//! A note can carry the **pad pressure** played on it ([`Touch`]): a run
//! of thinned values in the clip's [`Clip::touch`], timed from the note's
//! start, so quantise, swing and edits that move the note take its
//! pressure with it.
//!
//! The **arrangement** ([`arrange`]) puts clips on a timeline, linked to
//! their slots until made unique; scenes and the arrangement convert both
//! ways.
//!
//! The note editors (the Grid View and List Edit) make their edits with
//! [`notes`], Timing Correct among them.
//!
//! Audio clips come in a later roadmap step.
#![forbid(unsafe_code)]

pub mod arrange;
pub mod loops;
pub mod notes;
pub mod save;
pub mod scenes;
pub mod steps;

pub use arrange::{Arrangement, Marker, MorphRows, Placement, Source};
pub use scenes::{Follow, FollowAction, Scene, SCENES};

pub use mpfree_moves;
use mpfree_moves::Moves;
pub use mpfree_param;
use mpfree_param::{OnTrack, RelAddr};
use mpfree_transport::time::{Swing, Tick, PPQ};

/// Clip slots per track, one per pad.
pub const SLOTS: usize = 16;
/// Tracks in a project at most, as on Akai's MPC (Jay, 14 Nov 2026: up to
/// 128). Engines keep a little state per track in fixed tables of this
/// size, and do work only for the tracks a project has.
pub const MAX_TRACKS: usize = 128;
/// The shortest clip: a 1/32 note.
pub const MIN_CLIP_TICKS: Tick = (PPQ / 8) as Tick;
/// The longest clip: 256 bars of 4/4.
pub const MAX_CLIP_TICKS: Tick = 256 * 4 * PPQ as Tick;
/// Notes in one clip.
pub const MAX_NOTES: usize = 8192;
/// Parameter locks in one clip.
pub const MAX_LOCKS: usize = 8192;
/// Pressure values in one clip, every note's together ([`Clip::touch`]).
pub const MAX_TOUCH_POINTS: usize = 65536;
/// The looper's loops, which a Loop track's cells name ([`Clip::looped`]):
/// `mpfree_looper::MAX_LOOPS`.
pub const LOOPS: usize = 8;

/// One note in a clip, in the clip's own ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    /// Start, from the clip's start: `0 <= tick < clip length`.
    pub tick: Tick,
    /// Length in the clip's ticks, at least 1. It may run past the clip's
    /// end; it is cut if the clip or the transport jumps first.
    pub len: Tick,
    /// Pad (0 to 15) or MIDI note (0 to 127); the destination decides.
    pub note: u8,
    /// 1 to 127.
    pub velocity: u8,
    /// The chance it plays on each pass, in percent: 1 to 100 (always).
    pub chance: u8,
    /// When it plays: [`Cond::Always`], or a trig condition. Checked
    /// before the chance.
    pub cond: Cond,
    /// It plays this many times, 1 to [`MAX_RATCHET`], evenly spread over
    /// its length, each hit lasting `len / ratchet`. 1 is a plain note.
    pub ratchet: u8,
    /// The pad pressure played on it, if any: its values in the clip's
    /// [`Clip::touch`]. Edits that copy the note (`..note`) keep it.
    pub touch: Touch,
}

/// Where a note's pressure is in its clip: values `first..first + len` of
/// [`Clip::touch`]. `len` 0 is none ([`Touch::NONE`]). A clip only ever
/// adds pressure ([`Clip::add_touch`]), so this stays valid while notes
/// are taken out, put back (undo) or moved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Touch {
    pub first: u32,
    pub len: u32,
}

impl Touch {
    pub const NONE: Touch = Touch { first: 0, len: 0 };

    pub fn is_none(self) -> bool {
        self.len == 0
    }

    /// The indices in [`Clip::touch`].
    pub fn range(self) -> std::ops::Range<usize> {
        self.first as usize..self.first as usize + self.len as usize
    }
}

/// One pressure value of a note: `value` (0 to 127, the pad's raw
/// pressure, before any instrument's curve) from `at` clip ticks after the
/// note's start on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TouchPoint {
    pub at: u32,
    pub value: u8,
}

/// The most hits a ratchet makes.
pub const MAX_RATCHET: u8 = 8;

/// A trig condition, the Elektron way: when a note plays. Passes count
/// from the clip's start on its track (pass 1 starts there), by song
/// position. A note is *conditional* when its condition is `Every`,
/// `Fill`, `First` or their opposites, or it is `Always` with a chance
/// below 100 %: whether it played is its track's last result, which `Pre`
/// and `Nei` read ([`Cond::sets_last`]). Notes on `Pre` or `Nei` do not
/// set it, so a chain of them all follow the same note.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Cond {
    #[default]
    Always,
    /// On pass `a` of every `b`: `2 <= b <= 8`, `1 <= a <= b` (1:2 plays
    /// on passes 1, 3, 5 ...).
    Every { a: u8, b: u8 },
    /// Only while fill is on.
    Fill,
    /// Only while fill is off.
    NotFill,
    /// Only if the track's last conditional note played.
    Pre,
    /// Only if the track's last conditional note did not play.
    NotPre,
    /// Only if the last conditional note of the track to the left played.
    Nei,
    /// Only if the last conditional note of the track to the left did not
    /// play (or there is no track to the left).
    NotNei,
    /// Only on the clip's first pass.
    First,
    /// On every pass but the first.
    NotFirst,
}

impl Cond {
    /// Whether a note with this condition and `chance` (percent) is
    /// conditional: whether it played becomes its track's last result.
    /// Not for `Pre` and `Nei` (and their opposites), which read it.
    pub fn sets_last(self, chance: u8) -> bool {
        match self {
            Cond::Always => chance < 100,
            Cond::Pre | Cond::NotPre | Cond::Nei | Cond::NotNei => false,
            _ => true,
        }
    }

    /// In range.
    pub fn ok(self) -> bool {
        match self {
            Cond::Every { a, b } => (2..=8).contains(&b) && (1..=b).contains(&a),
            _ => true,
        }
    }

    /// Every condition in the order a wheel steps through them: always,
    /// then 1:2 to 8:8 (by the number of passes, then the pass), then
    /// FILL, !FILL, PRE, !PRE, NEI, !NEI, 1ST and !1ST.
    pub fn all() -> Vec<Cond> {
        let mut out = vec![Cond::Always];
        for b in 2..=8 {
            out.extend((1..=b).map(|a| Cond::Every { a, b }));
        }
        out.extend([
            Cond::Fill,
            Cond::NotFill,
            Cond::Pre,
            Cond::NotPre,
            Cond::Nei,
            Cond::NotNei,
            Cond::First,
            Cond::NotFirst,
        ]);
        out
    }

    /// The condition `d` places on in [`Cond::all`], stopping at either
    /// end (one out of range steps from always).
    pub fn step(self, d: i32) -> Cond {
        let all = Cond::all();
        let at = all.iter().position(|&c| c == self).unwrap_or(0) as i32;
        all[(at + d).clamp(0, all.len() as i32 - 1) as usize]
    }

    /// The condition alone (fill, the pass, the last results given):
    /// whether the note plays, before its chance. `pass` counts from 1 at
    /// the clip's start (0 and below before it); `pre` and `nei` are the
    /// track's and its left neighbour's last results (`nei` None: no
    /// neighbour).
    pub fn holds(self, pass: i64, fill: bool, pre: bool, nei: Option<bool>) -> bool {
        match self {
            Cond::Always => true,
            Cond::Every { a, b } => (pass - 1).rem_euclid(i64::from(b.max(1))) == i64::from(a) - 1,
            Cond::Fill => fill,
            Cond::NotFill => !fill,
            Cond::Pre => pre,
            Cond::NotPre => !pre,
            Cond::Nei => nei == Some(true),
            Cond::NotNei => nei != Some(true),
            Cond::First => pass == 1,
            Cond::NotFirst => pass != 1,
        }
    }
}

impl std::fmt::Display for Cond {
    /// As the Steps screen shows it: "", "1:2", "FILL", "!FILL", "PRE",
    /// "!PRE", "NEI", "!NEI", "1ST", "!1ST".
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cond::Always => Ok(()),
            Cond::Every { a, b } => write!(f, "{a}:{b}"),
            Cond::Fill => f.write_str("FILL"),
            Cond::NotFill => f.write_str("!FILL"),
            Cond::Pre => f.write_str("PRE"),
            Cond::NotPre => f.write_str("!PRE"),
            Cond::Nei => f.write_str("NEI"),
            Cond::NotNei => f.write_str("!NEI"),
            Cond::First => f.write_str("1ST"),
            Cond::NotFirst => f.write_str("!1ST"),
        }
    }
}

impl Note {
    /// A note that always plays. New code should build notes with this, so
    /// that fields added later take their defaults.
    pub fn new(tick: Tick, len: Tick, note: u8, velocity: u8) -> Note {
        Note {
            tick,
            len,
            note,
            velocity,
            chance: 100,
            cond: Cond::Always,
            ratchet: 1,
            touch: Touch::NONE,
        }
    }

    /// Whether a clip `length` ticks long whose pad pressure is `touch`
    /// ([`Clip::touch`]) can hold this note: [`Clip::add_note`]'s rule,
    /// less the note count. A play view checks a recorded note with it,
    /// so the view and the clip decide alike.
    pub fn fits(&self, length: Tick, touch: &[TouchPoint]) -> bool {
        (0..length).contains(&self.tick)
            && (1..=MAX_CLIP_TICKS).contains(&self.len)
            && self.note <= 127
            && (1..=127).contains(&self.velocity)
            && (1..=100).contains(&self.chance)
            && self.cond.ok()
            && (1..=MAX_RATCHET).contains(&self.ratchet)
            && (self.touch.is_none() || touch.get(self.touch.range()).is_some_and(run_ok))
    }

    /// Whether it is conditional ([`Cond::sets_last`]): whether it played
    /// is what `Cond::Pre` and `Cond::Nei` read.
    pub fn conditional(&self) -> bool {
        self.cond.sets_last(self.chance)
    }
}

/// How fast a clip plays against the song: `num / den` clip ticks per song
/// tick, from 1/8 to 4 (1/8, 1/4, 1/2, 3/4, 1, 3/2, 2, 4 and the rest that
/// these numbers make).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Speed {
    pub num: u8,
    pub den: u8,
}

impl Speed {
    pub const NORMAL: Speed = Speed { num: 1, den: 1 };

    pub fn new(num: u8, den: u8) -> Option<Speed> {
        let ok = |x: u8| matches!(x, 1 | 2 | 3 | 4 | 8);
        let ratio = f64::from(num) / f64::from(den.max(1));
        (ok(num) && ok(den) && (0.125..=4.0).contains(&ratio)).then_some(Speed { num, den })
    }

    /// Song ticks for `ticks` of the clip's own ticks.
    pub fn to_song(self, ticks: f64) -> f64 {
        ticks * f64::from(self.den) / f64::from(self.num)
    }

    /// Clip ticks for `ticks` song ticks.
    pub fn to_clip(self, ticks: f64) -> f64 {
        ticks * f64::from(self.num) / f64::from(self.den)
    }
}

impl Default for Speed {
    fn default() -> Self {
        Speed::NORMAL
    }
}

/// Non-destructive quantise: on playback each note moves `strength`
/// percent of the way to the nearest `grid` line. The notes keep their
/// recorded ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quantise {
    pub grid: u32,
    pub strength: u8,
}

impl Quantise {
    pub fn new(grid: u32, strength: u8) -> Option<Quantise> {
        (grid > 0 && grid <= PPQ * 4 && strength <= 100).then_some(Quantise { grid, strength })
    }

    /// Where a tick plays.
    pub fn apply(self, tick: f64) -> f64 {
        let g = f64::from(self.grid);
        let target = (tick / g).round() * g;
        tick + (target - tick) * f64::from(self.strength) / 100.0
    }
}

/// When a launched clip starts, and a stop or a mute lands (launch
/// quantise, sequencer S4): at once, on the next line of a note grid, or
/// on the next line of a number of bars counted from the song's start.
/// The project has one ([`Project::launch`], 1 bar for a new one), and a
/// clip can have its own ([`Clip::launch`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LaunchGrid {
    /// At once: the next tick the playhead plays.
    Now,
    /// The next line of a grid this many ticks apart (1/16 is 240).
    Ticks(u32),
    /// The next line of this many bars.
    Bars(u8),
}

impl Default for LaunchGrid {
    fn default() -> Self {
        LaunchGrid::Bars(1)
    }
}

/// The launch grids the screens step through, finest first: at once, 1/16
/// to 1/2, then 1 to 8 bars.
pub const LAUNCH_GRIDS: [LaunchGrid; 9] = [
    LaunchGrid::Now,
    LaunchGrid::Ticks(PPQ / 4),
    LaunchGrid::Ticks(PPQ / 2),
    LaunchGrid::Ticks(PPQ),
    LaunchGrid::Ticks(PPQ * 2),
    LaunchGrid::Bars(1),
    LaunchGrid::Bars(2),
    LaunchGrid::Bars(4),
    LaunchGrid::Bars(8),
];

impl LaunchGrid {
    /// In range: a note grid of a tick to a whole note, or 1 to 16 bars.
    pub fn ok(self) -> bool {
        match self {
            LaunchGrid::Now => true,
            LaunchGrid::Ticks(t) => (1..=PPQ * 4).contains(&t),
            LaunchGrid::Bars(n) => (1..=16).contains(&n),
        }
    }

    /// The distance between its lines, in song ticks, with bars of `bar`
    /// ticks: 1 for at once, so the next line is the next tick.
    pub fn line(self, bar: Tick) -> Tick {
        match self {
            LaunchGrid::Now => 1,
            LaunchGrid::Ticks(t) => Tick::from(t.max(1)),
            LaunchGrid::Bars(n) => bar.max(1) * Tick::from(n.max(1)),
        }
    }

    /// Its first line at or after song tick `t`, with bars of `bar` ticks.
    pub fn next(self, t: Tick, bar: Tick) -> Tick {
        let line = self.line(bar);
        t.div_euclid(line) * line + if t.rem_euclid(line) == 0 { 0 } else { line }
    }
}

/// A parameter lock: `addr` set to `value` on a step, from `tick` until
/// the track's next note without a lock on it (see the crate docs).
///
/// `value` is in the parameter's own units, as its `ParamInfo` gives them
/// (the resolver in `mpfree-mod` clamps it). It is always finite: a clip
/// refuses a lock that is not, so locks compare as equal or not like notes
/// do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lock {
    /// From the clip's start, `0 <= tick < clip length`: the tick of the
    /// step's note, so the lock moves with its nudge (the step's line for
    /// a step with no note).
    pub tick: Tick,
    /// The step row it sits on: a pad or note number, or None for the
    /// whole step (a chord on a melodic track). Where it shows and what
    /// step edits carry it with; how long it lasts depends on `addr`.
    pub sound: Option<u8>,
    /// The parameter, on the clip's track.
    pub addr: RelAddr,
    pub value: f32,
}

impl Eq for Lock {}

impl Lock {
    pub fn new(tick: Tick, sound: Option<u8>, addr: RelAddr, value: f32) -> Lock {
        Lock {
            tick,
            sound,
            addr,
            value,
        }
    }

    /// Whether a clip `length` ticks long can hold this lock:
    /// [`Clip::add_lock`]'s rule, less the lock count.
    pub fn fits(&self, length: Tick) -> bool {
        (0..length).contains(&self.tick)
            && self.sound.is_none_or(|s| s <= 127)
            && self.value.is_finite()
    }

    /// Whether this lock and `other` hold the same place: the same
    /// parameter on the same tick and row. A clip keeps one lock in each
    /// place, and a recorded lock takes the place of the one there
    /// ([`crate::Clip::locks`]).
    pub fn same_place(&self, other: &Lock) -> bool {
        (self.tick, self.sound, self.addr) == (other.tick, other.sound, other.addr)
    }

    /// The pad whose notes end this lock, for a sampler pad's parameter
    /// (`OnTrack::Pad`); None when any note of the track ends it.
    pub fn scope(addr: RelAddr) -> Option<u8> {
        match addr.on {
            OnTrack::Pad(p) => Some(p),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Length in the clip's own ticks.
    pub length: Tick,
    pub speed: Speed,
    /// Overrides the track's and the transport's swing.
    pub swing: Option<Swing>,
    pub quantise: Option<Quantise>,
    /// Kept in the order they were written; the play view sorts them.
    pub notes: Vec<Note>,
    /// Parameter locks, in the order they were written.
    pub locks: Vec<Lock>,
    /// Recorded parameter moves (automation): Perform FX gestures, as
    /// lanes of points on the clip's ticks (`mpfree-moves`).
    pub moves: Moves,
    /// Every note's pad pressure, one run a note ([`Note::touch`]), each
    /// run's values in time order. Only added to, never taken out of.
    pub touch: Vec<TouchPoint>,
    /// After some bars, its track plays another clip or stops
    /// ([`scenes`]).
    pub follow: Option<Follow>,
    /// A Loop track's cell: the looper's loop it launches (0 to
    /// [`LOOPS`] - 1). Such a clip has no notes, locks, pressure or moves,
    /// and is as long as its loop ([`loops`]).
    pub looped: Option<u8>,
    /// Its own launch quantise; None: the project's ([`Project::launch`]).
    pub launch: Option<LaunchGrid>,
    /// Legato: launched while another clip plays on its track, it takes
    /// over that clip's place in its pass instead of starting from its
    /// top (sequencer S4).
    pub legato: bool,
}

/// A clip's settings that are not its notes: how fast it plays, its own
/// swing and quantise, and what its track does after it (the clip
/// settings card, sequencer S3). Read with [`Clip::settings`], changed as
/// one with [`Clip::set_settings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipSettings {
    pub speed: Speed,
    pub swing: Option<Swing>,
    pub quantise: Option<Quantise>,
    pub follow: Option<Follow>,
    /// Its launch quantise (None: the project's) and legato (S4).
    pub launch: Option<LaunchGrid>,
    pub legato: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    ClipLength,
    TooManyNotes,
    /// A note's tick, length, note number, velocity, chance, condition or
    /// ratchet is out of range.
    BadNote(usize),
    BadSpeed,
    BadSwing,
    BadQuantise,
    TooManyTracks,
    BadSlot,
    /// A note to take out is not in the clip (see [`steps::Edit`]).
    MissingNote,
    TooManyLocks,
    /// A lock's tick, sound or value is out of range.
    BadLock(usize),
    /// A lock to take out is not in the clip.
    MissingLock,
    /// Pressure values past [`MAX_TOUCH_POINTS`], or a run out of order
    /// or out of range.
    BadTouch,
    /// The clip's moves are not sound for its length, or not the ones an
    /// edit expected to change (see [`steps::Edit`]).
    BadMoves,
    /// A follow action waits 0 bars or more than 999, or its chance is
    /// over 100.
    BadFollow,
    /// A scene's mutes, values or count are out of range.
    BadScene,
    /// A placement is empty, out of range, overlaps another on its track,
    /// or names an empty slot; or the arrangement is out of order.
    BadPlacement,
    /// No room for it (song to scenes: no run of empty rows long enough).
    NoRoom,
    /// A launch quantise out of range ([`LaunchGrid::ok`]).
    BadLaunch,
    /// A loop cell names no loop or holds notes, or a clip is on a track
    /// of the other kind (a loop cell on a note track, a note clip on a
    /// Loop track).
    BadLoop,
}

impl Clip {
    /// Its speed, swing, quantise, follow action, launch quantise and
    /// legato.
    pub fn settings(&self) -> ClipSettings {
        ClipSettings {
            speed: self.speed,
            swing: self.swing,
            quantise: self.quantise,
            follow: self.follow,
            launch: self.launch,
            legato: self.legato,
        }
    }

    /// Change its speed, swing, quantise, follow action, launch quantise
    /// and legato, all or none: `BadSpeed`, `BadSwing`, `BadQuantise`,
    /// `BadLaunch` or the follow action's own refusal leaves the clip as it
    /// was.
    pub fn set_settings(&mut self, s: ClipSettings) -> Result<(), ModelError> {
        if Speed::new(s.speed.num, s.speed.den).is_none() {
            return Err(ModelError::BadSpeed);
        }
        if s.swing
            .is_some_and(|w| Swing::new(w.grid, w.amount).is_none())
        {
            return Err(ModelError::BadSwing);
        }
        if s.quantise
            .is_some_and(|q| Quantise::new(q.grid, q.strength).is_none())
        {
            return Err(ModelError::BadQuantise);
        }
        if let Some(f) = &s.follow {
            f.check()?;
        }
        if s.launch.is_some_and(|g| !g.ok()) {
            return Err(ModelError::BadLaunch);
        }
        self.speed = s.speed;
        self.swing = s.swing;
        self.quantise = s.quantise;
        self.follow = s.follow;
        self.launch = s.launch;
        self.legato = s.legato;
        Ok(())
    }

    /// An empty clip of `length` ticks at normal speed.
    pub fn new(name: &str, length: Tick) -> Clip {
        Clip {
            name: name.to_string(),
            length,
            speed: Speed::NORMAL,
            swing: None,
            quantise: None,
            notes: Vec::new(),
            locks: Vec::new(),
            moves: Moves::new(),
            follow: None,
            touch: Vec::new(),
            looped: None,
            launch: None,
            legato: false,
        }
    }

    /// A Loop track's cell for loop `n` (0 based), `ticks` long: named
    /// after its loop ("Loop 2"), with no notes.
    pub fn new_loop(n: u8, ticks: Tick) -> Clip {
        Clip {
            looped: Some(n),
            ..Clip::new(&format!("Loop {}", u32::from(n) + 1), loops::length(ticks))
        }
    }

    /// Add one note's pressure run (values in time order, 0 to 127) and
    /// say where it went, for the note's [`Note::touch`]. An empty run is
    /// [`Touch::NONE`]. A run out of order, or one that would take the clip
    /// past [`MAX_TOUCH_POINTS`], is refused and the clip is unchanged. It
    /// allocates only past the room reserved in `touch`.
    pub fn add_touch(&mut self, points: &[TouchPoint]) -> Result<Touch, ModelError> {
        if points.is_empty() {
            return Ok(Touch::NONE);
        }
        if !touch_fits(self.touch.len(), points) {
            return Err(ModelError::BadTouch);
        }
        let first = self.touch.len() as u32;
        self.touch.extend_from_slice(points);
        Ok(Touch {
            first,
            len: points.len() as u32,
        })
    }

    /// Note `n`'s pressure run (empty if it has none or it is out of
    /// range).
    pub fn touch_of(&self, n: &Note) -> &[TouchPoint] {
        self.touch.get(n.touch.range()).unwrap_or(&[])
    }

    /// Add one note, for example a recorded one. A note out of range, or
    /// one more than [`MAX_NOTES`], is refused and the clip is unchanged.
    pub fn add_note(&mut self, note: Note) -> Result<(), ModelError> {
        if self.notes.len() >= MAX_NOTES {
            return Err(ModelError::TooManyNotes);
        }
        if !self.note_ok(&note) {
            return Err(ModelError::BadNote(self.notes.len()));
        }
        self.notes.push(note);
        Ok(())
    }

    /// Remove one note equal to `note`, for example one that replace
    /// recording erased. False if the clip has none.
    pub fn remove_note(&mut self, note: &Note) -> bool {
        match self.notes.iter().position(|n| n == note) {
            Some(i) => {
                self.notes.remove(i);
                true
            }
            None => false,
        }
    }

    /// Add one lock. A lock out of range (its tick, its sound, a value
    /// that is not finite), or one more than [`MAX_LOCKS`], is refused and
    /// the clip is unchanged.
    pub fn add_lock(&mut self, lock: Lock) -> Result<(), ModelError> {
        if self.locks.len() >= MAX_LOCKS {
            return Err(ModelError::TooManyLocks);
        }
        if !self.lock_ok(&lock) {
            return Err(ModelError::BadLock(self.locks.len()));
        }
        self.locks.push(lock);
        Ok(())
    }

    /// Remove one lock equal to `lock`. False if the clip has none.
    pub fn remove_lock(&mut self, lock: &Lock) -> bool {
        match self.locks.iter().position(|l| l == lock) {
            Some(i) => {
                self.locks.remove(i);
                true
            }
            None => false,
        }
    }

    fn lock_ok(&self, l: &Lock) -> bool {
        l.fits(self.length)
    }

    fn note_ok(&self, n: &Note) -> bool {
        n.fits(self.length, &self.touch)
    }

    /// Every value in range.
    pub fn check(&self) -> Result<(), ModelError> {
        if !(MIN_CLIP_TICKS..=MAX_CLIP_TICKS).contains(&self.length) {
            return Err(ModelError::ClipLength);
        }
        if self.notes.len() > MAX_NOTES {
            return Err(ModelError::TooManyNotes);
        }
        if self.touch.len() > MAX_TOUCH_POINTS {
            return Err(ModelError::BadTouch);
        }
        if Speed::new(self.speed.num, self.speed.den).is_none() {
            return Err(ModelError::BadSpeed);
        }
        if self
            .swing
            .is_some_and(|s| Swing::new(s.grid, s.amount).is_none())
        {
            return Err(ModelError::BadSwing);
        }
        if self
            .quantise
            .is_some_and(|q| Quantise::new(q.grid, q.strength).is_none())
        {
            return Err(ModelError::BadQuantise);
        }
        if self.launch.is_some_and(|g| !g.ok()) {
            return Err(ModelError::BadLaunch);
        }
        for (i, n) in self.notes.iter().enumerate() {
            if !self.note_ok(n) {
                return Err(ModelError::BadNote(i));
            }
        }
        if self.locks.len() > MAX_LOCKS {
            return Err(ModelError::TooManyLocks);
        }
        for (i, l) in self.locks.iter().enumerate() {
            if !self.lock_ok(l) {
                return Err(ModelError::BadLock(i));
            }
        }
        if let Some(f) = &self.follow {
            f.check()?;
        }
        if self.looped.is_some_and(|n| {
            usize::from(n) >= LOOPS
                || !self.notes.is_empty()
                || !self.locks.is_empty()
                || !self.touch.is_empty()
                || !self.moves.is_empty()
        }) {
            return Err(ModelError::BadLoop);
        }
        let len = u32::try_from(self.length).map_err(|_| ModelError::ClipLength)?;
        self.moves.check(len).map_err(|_| ModelError::BadMoves)
    }
}

/// A pressure run is sound: values 0 to 127, in time order.
fn run_ok(points: &[TouchPoint]) -> bool {
    points.iter().all(|p| p.value <= 127) && points.windows(2).all(|w| w[0].at <= w[1].at)
}

/// Whether a clip holding `had` pressure values takes a new run of
/// `points`: [`Clip::add_touch`]'s rule (sound, and no further than
/// [`MAX_TOUCH_POINTS`]). An empty run always fits; it adds nothing.
pub fn touch_fits(had: usize, points: &[TouchPoint]) -> bool {
    points.is_empty() || (had + points.len() <= MAX_TOUCH_POINTS && run_ok(points))
}

/// Whether a clip holding `had` pressure values has room for `points`,
/// several notes' runs one after another (each note that names one
/// checks its run's order when it goes in).
pub fn touch_room(had: usize, points: &[TouchPoint]) -> bool {
    had + points.len() <= MAX_TOUCH_POINTS && points.iter().all(|p| p.value <= 127)
}

/// Where a track's notes go: a sampler program, the arp in front of it,
/// MIDI out ... The host owns the list; a track holds an index into it.
pub type DestId = u16;

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub name: String,
    pub dest: DestId,
    /// Overrides the transport's swing for every clip on the track that has
    /// none of its own.
    pub swing: Option<Swing>,
    pub mute: bool,
    /// Soloed (sequencer S4): while any track is, the tracks that aren't
    /// are silent ([`Project::silent`]). A muted track stays silent soloed.
    pub solo: bool,
    /// "Clips through the arp": the track's clips play through its arp,
    /// as the live pads do, and recording keeps the pads as pressed (on:
    /// melodic tracks, which record the chord). Off (the default; drum
    /// tracks), the clips play as recorded and recording keeps what the
    /// arp played. Either way a part is never arpeggiated twice
    /// (`mpfree_play::TrackArps`).
    pub through_arp: bool,
    /// "Record pad pressure": while the track records, the pads' pressure
    /// goes into its notes ([`Note::touch`]). On by default.
    pub record_touch: bool,
    /// "Record quantise" (sequencer S3, the MPC's Timing Correct while
    /// recording): the notes recorded on this track go in on the nearest
    /// line of this grid, at full strength, in place of where they were
    /// played; none (the default) keeps them as played. The clip's own
    /// quantise ([`Clip::quantise`]) still applies on playback.
    pub record_quantise: Option<u32>,
    /// The track's MIDI effects (its Notes stage) and where its arp sits
    /// among them. They change what reaches the arp (the live pads, and the
    /// clips with [`Track::through_arp`] on) and what it plays, by the same
    /// rule: a part is never put through them twice.
    pub note_fx: mpfree_notefx::TrackFx,
    /// A note track (the pads play it and its clips hold notes), or a Loop
    /// track, whose cells launch the looper's loops ([`loops`]).
    pub kind: TrackKind,
    pub slots: [Option<Clip>; SLOTS],
}

/// What a track is: see [`Track::kind`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackKind {
    #[default]
    Notes,
    /// A Loop track and what its new loops record.
    Loops(LoopInput),
}

/// What a Loop track's new loops record (recording from the grid, Loop
/// tracks part 4): the pads and the synth as heard (the looper's source
/// today), the line input, or the other loops as heard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopInput {
    #[default]
    Pads,
    Line,
    Resample,
}

impl LoopInput {
    pub const ALL: [LoopInput; 3] = [LoopInput::Pads, LoopInput::Line, LoopInput::Resample];

    /// As the screens say it in full.
    pub fn name(self) -> &'static str {
        match self {
            LoopInput::Pads => "Pads + synth",
            LoopInput::Line => "Line in",
            LoopInput::Resample => "Resample",
        }
    }

    /// As the Clips grid says it, under the track's name.
    pub fn short(self) -> &'static str {
        match self {
            LoopInput::Pads => "Pads",
            LoopInput::Line => "Line in",
            LoopInput::Resample => "Resample",
        }
    }

    /// As a saved project writes it.
    pub fn word(self) -> &'static str {
        match self {
            LoopInput::Pads => "pads",
            LoopInput::Line => "line",
            LoopInput::Resample => "resample",
        }
    }

    pub fn from_word(w: &str) -> Option<LoopInput> {
        LoopInput::ALL.into_iter().find(|i| i.word() == w)
    }
}

impl Track {
    pub fn new(name: &str, dest: DestId) -> Track {
        Track {
            name: name.to_string(),
            dest,
            swing: None,
            mute: false,
            solo: false,
            through_arp: false,
            record_touch: true,
            record_quantise: None,
            note_fx: mpfree_notefx::TrackFx::default(),
            kind: TrackKind::Notes,
            slots: Default::default(),
        }
    }

    /// An empty Loop track recording the pads and the synth. Its `dest`
    /// is unused: nothing plays notes into it.
    pub fn new_loops(name: &str, dest: DestId) -> Track {
        Track {
            kind: TrackKind::Loops(LoopInput::Pads),
            ..Track::new(name, dest)
        }
    }

    /// A Loop track's input; None for a note track.
    pub fn loops(&self) -> Option<LoopInput> {
        match self.kind {
            TrackKind::Notes => None,
            TrackKind::Loops(i) => Some(i),
        }
    }

    pub fn is_loops(&self) -> bool {
        self.loops().is_some()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Project {
    pub tracks: Vec<Track>,
    /// Scene `k` holds what row `k` of the slots has besides its clips;
    /// rows past the end are plain (see [`Project::scene`]).
    pub scenes: Vec<Scene>,
    /// The song: clips on a timeline ([`arrange`]).
    pub arrangement: Arrangement,
    /// Make (false, a new project's) or Perform (sequencer roadmap S4,
    /// Jay's decisions 4 and 5): how the app answers the hands, never
    /// what plays. Saved, so a set opens as it was left.
    pub perform: bool,
    /// When a launch, a stop or a scene lands (launch quantise, S4): the
    /// transport bar's "Launch 1 bar". A clip can have its own.
    pub launch: LaunchGrid,
}

impl Project {
    pub fn new() -> Project {
        Project::default()
    }

    /// Whether track `t` is silent: muted, or not soloed while another
    /// track is (sequencer S4's solo). False past the last track.
    pub fn silent(&self, t: usize) -> bool {
        let soloing = self.tracks.iter().any(|t| t.solo);
        self.tracks
            .get(t)
            .is_some_and(|tr| tr.mute || (soloing && !tr.solo))
    }

    /// Every track and clip in range.
    pub fn check(&self) -> Result<(), ModelError> {
        self.check_tracks()?;
        for t in &self.tracks {
            for c in t.slots.iter().flatten() {
                c.check()?;
                if c.looped.is_some() != t.is_loops() {
                    return Err(ModelError::BadLoop);
                }
            }
        }
        if self.scenes.len() > SCENES {
            return Err(ModelError::BadScene);
        }
        for s in &self.scenes {
            s.check()?;
        }
        if !self.launch.ok() {
            return Err(ModelError::BadLaunch);
        }
        self.arrangement.check(&self.tracks)
    }

    /// The tracks alone, not their clips: no more than [`MAX_TRACKS`],
    /// each swing in range. A play view checks this and then only the
    /// clips it compiles, so an edit costs what plays, not the whole
    /// project.
    pub fn check_tracks(&self) -> Result<(), ModelError> {
        if self.tracks.len() > MAX_TRACKS {
            return Err(ModelError::TooManyTracks);
        }
        if self.tracks.iter().any(|t| {
            t.swing
                .is_some_and(|s| Swing::new(s.grid, s.amount).is_none())
        }) {
            return Err(ModelError::BadSwing);
        }
        Ok(())
    }

    /// Scene `row`, or `None` when the project keeps nothing for it (a
    /// plain row: launching it still plays its clips).
    pub fn scene(&self, row: usize) -> Option<&Scene> {
        self.scenes.get(row)
    }

    /// Scene `row` to change, made (with the plain ones before it) if
    /// needed. `None` past the last row.
    pub fn scene_mut(&mut self, row: usize) -> Option<&mut Scene> {
        if row >= SCENES {
            return None;
        }
        while self.scenes.len() <= row {
            self.scenes.push(Scene::default());
        }
        self.scenes.get_mut(row)
    }

    /// Whether row `row` has a clip on any track.
    pub fn row_filled(&self, row: usize) -> bool {
        self.tracks
            .iter()
            .any(|t| t.slots.get(row).is_some_and(Option::is_some))
    }

    /// The clip in a slot, if there is one.
    pub fn clip(&self, track: usize, slot: usize) -> Option<&Clip> {
        self.tracks.get(track)?.slots.get(slot)?.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notes_pressure_is_a_run_in_its_clip() {
        let p = |at, value| TouchPoint { at, value };
        let mut c = Clip::new("pressed", 3840);
        assert_eq!(c.add_touch(&[]), Ok(Touch::NONE));
        let a = c.add_touch(&[p(0, 10), p(50, 40)]).unwrap();
        let b = c.add_touch(&[p(0, 127)]).unwrap();
        assert_eq!(
            (a, b),
            (Touch { first: 0, len: 2 }, Touch { first: 2, len: 1 })
        );
        // Out of order or out of range: refused, nothing added.
        assert_eq!(c.add_touch(&[p(9, 1), p(3, 1)]), Err(ModelError::BadTouch));
        assert_eq!(c.add_touch(&[p(0, 128)]), Err(ModelError::BadTouch));
        assert_eq!(c.touch.len(), 3);
        let n = Note {
            touch: a,
            ..Note::new(0, 240, 1, 100)
        };
        c.add_note(n).unwrap();
        assert_eq!(c.touch_of(&n), &[p(0, 10), p(50, 40)]);
        // Taking the note out keeps its pressure, so undo can put it back.
        assert!(c.remove_note(&n));
        assert_eq!(c.touch.len(), 3);
        c.add_note(n).unwrap();
        assert_eq!(c.check(), Ok(()));
        // A note naming pressure the clip does not have is refused.
        let stray = Note {
            touch: Touch { first: 2, len: 5 },
            ..Note::new(0, 240, 2, 100)
        };
        assert_eq!(c.add_note(stray), Err(ModelError::BadNote(1)));
        // No more than MAX_TOUCH_POINTS in a clip.
        let many = vec![p(0, 1); MAX_TOUCH_POINTS - 3];
        assert!(c.add_touch(&many).is_ok());
        assert_eq!(c.add_touch(&[p(0, 1)]), Err(ModelError::BadTouch));
        assert_eq!(c.check(), Ok(()));
        // A new track records pressure.
        assert!(Track::new("t", 0).record_touch);
    }

    #[test]
    fn clips_check_their_values() {
        let mut c = Clip::new("beat", 3840);
        c.notes.push(Note {
            tick: 0,
            len: 240,
            note: 0,
            velocity: 100,
            chance: 100,
            cond: Cond::Always,
            ratchet: 1,
            touch: Touch::NONE,
        });
        assert_eq!(c.check(), Ok(()));
        c.notes.push(Note {
            tick: 3840,
            len: 240,
            note: 0,
            velocity: 100,
            chance: 100,
            cond: Cond::Always,
            ratchet: 1,
            touch: Touch::NONE,
        });
        assert_eq!(c.check(), Err(ModelError::BadNote(1)));
        c.notes[1].tick = 100;
        c.notes[1].velocity = 0;
        assert_eq!(c.check(), Err(ModelError::BadNote(1)));
        c.notes.pop();
        c.length = 100;
        assert_eq!(c.check(), Err(ModelError::ClipLength));
        c.length = 3840;
        c.speed = Speed { num: 3, den: 8 };
        assert_eq!(c.check(), Ok(()));
        c.speed = Speed { num: 8, den: 1 };
        assert_eq!(c.check(), Err(ModelError::BadSpeed));
        assert!(
            Speed::new(1, 8).is_some() && Speed::new(4, 1).is_some() && Speed::new(5, 4).is_none()
        );
    }

    #[test]
    fn trig_conditions_and_ratchets() {
        let every = |a, b| Cond::Every { a, b };
        // 1:2 on odd passes, 2:2 on even ones, 3:4 on 3, 7, -1 ...
        let on = |c: Cond| -> Vec<i64> {
            (-3..=8)
                .filter(|&p| c.holds(p, false, false, None))
                .collect()
        };
        assert_eq!(on(every(1, 2)), vec![-3, -1, 1, 3, 5, 7]);
        assert_eq!(on(every(2, 2)), vec![-2, 0, 2, 4, 6, 8]);
        assert_eq!(on(every(3, 4)), vec![-1, 3, 7]);
        assert_eq!(on(Cond::First), vec![1]);
        assert_eq!(on(Cond::NotFirst).len(), 11);
        assert!(
            Cond::Fill.holds(1, true, false, None) && !Cond::NotFill.holds(1, true, false, None)
        );
        assert!(Cond::Pre.holds(1, false, true, None) && Cond::NotPre.holds(1, false, false, None));
        assert!(Cond::Nei.holds(1, false, false, Some(true)));
        assert!(
            !Cond::Nei.holds(1, false, false, None) && Cond::NotNei.holds(1, false, false, None)
        );
        // Which notes set the last result that PRE and NEI read.
        assert!(!Cond::Always.sets_last(100) && Cond::Always.sets_last(99));
        assert!(
            every(1, 2).sets_last(100) && Cond::Fill.sets_last(100) && Cond::First.sets_last(100)
        );
        assert!(!Cond::Pre.sets_last(50) && !Cond::NotNei.sets_last(100));
        let shown: Vec<String> = [
            every(3, 8),
            Cond::NotFill,
            Cond::Pre,
            Cond::First,
            Cond::Always,
        ]
        .iter()
        .map(|c| c.to_string())
        .collect();
        assert_eq!(shown, ["3:8", "!FILL", "PRE", "1ST", ""]);
        // The wheel's order: always, the 35 A:B conditions, then the rest.
        let all = Cond::all();
        assert_eq!(all.len(), 44);
        assert!(all.iter().all(|c| c.ok()));
        assert_eq!(Cond::Always.step(1), every(1, 2));
        assert_eq!(every(2, 2).step(1), every(1, 3));
        assert_eq!(every(8, 8).step(1), Cond::Fill);
        assert_eq!(Cond::NotFirst.step(1), Cond::NotFirst, "stops at the end");
        assert_eq!(Cond::Always.step(-1), Cond::Always);
        assert_eq!(Cond::Fill.step(-2), every(7, 8));
        // Out of range: refused by the clip.
        assert!(!every(0, 2).ok() && !every(3, 2).ok() && !every(1, 9).ok() && every(8, 8).ok());
        let mut c = Clip::new("c", 3840);
        let mut n = Note::new(0, 240, 36, 100);
        n.cond = every(1, 1);
        c.notes.push(n);
        assert_eq!(c.check(), Err(ModelError::BadNote(0)));
        c.notes[0].cond = every(1, 2);
        c.notes[0].ratchet = MAX_RATCHET + 1;
        assert_eq!(c.check(), Err(ModelError::BadNote(0)));
        c.notes[0].ratchet = MAX_RATCHET;
        assert_eq!(c.check(), Ok(()));
        c.notes[0].ratchet = 0;
        assert_eq!(c.check(), Err(ModelError::BadNote(0)));
    }

    #[test]
    fn recorded_notes_are_checked_as_they_are_added() {
        let mut c = Clip::new("take", 960);
        assert_eq!(
            c.add_note(Note {
                tick: 959,
                len: 2000,
                note: 3,
                velocity: 1,
                chance: 100,
                cond: Cond::Always,
                ratchet: 1,
                touch: Touch::NONE,
            }),
            Ok(())
        );
        assert_eq!(
            c.add_note(Note {
                tick: 960,
                len: 10,
                note: 3,
                velocity: 100,
                chance: 100,
                cond: Cond::Always,
                ratchet: 1,
                touch: Touch::NONE,
            }),
            Err(ModelError::BadNote(1))
        );
        assert_eq!(
            c.add_note(Note {
                tick: 0,
                len: 0,
                note: 3,
                velocity: 100,
                chance: 100,
                cond: Cond::Always,
                ratchet: 1,
                touch: Touch::NONE,
            }),
            Err(ModelError::BadNote(1))
        );
        assert_eq!(c.notes.len(), 1);
        c.notes = vec![
            Note {
                tick: 0,
                len: 1,
                note: 0,
                velocity: 1,
                chance: 100,
                cond: Cond::Always,
                ratchet: 1,
                touch: Touch::NONE,
            };
            MAX_NOTES
        ];
        assert_eq!(
            c.add_note(Note {
                tick: 0,
                len: 1,
                note: 0,
                velocity: 1,
                chance: 100,
                cond: Cond::Always,
                ratchet: 1,
                touch: Touch::NONE,
            }),
            Err(ModelError::TooManyNotes)
        );
        assert_eq!(c.check(), Ok(()));
        // Equal notes are the same note: removing one leaves the rest.
        assert!(c.remove_note(&Note {
            tick: 0,
            len: 1,
            note: 0,
            velocity: 1,
            chance: 100,
            cond: Cond::Always,
            ratchet: 1,
            touch: Touch::NONE,
        }));
        assert_eq!(c.notes.len(), MAX_NOTES - 1);
        assert!(!c.remove_note(&Note {
            tick: 5,
            len: 1,
            note: 0,
            velocity: 1,
            chance: 100,
            cond: Cond::Always,
            ratchet: 1,
            touch: Touch::NONE,
        }));
    }

    #[test]
    fn speed_and_quantise_map_ticks() {
        let s = Speed::new(2, 1).unwrap();
        assert_eq!(s.to_song(960.0), 480.0);
        assert_eq!(s.to_clip(480.0), 960.0);
        assert_eq!(Speed::new(3, 4).unwrap().to_song(3.0), 4.0);
        let q = Quantise::new(240, 50).unwrap();
        assert_eq!(q.apply(250.0), 245.0);
        assert_eq!(q.apply(110.0), 55.0);
        assert_eq!(Quantise::new(240, 100).unwrap().apply(130.0), 240.0);
        assert!(Quantise::new(0, 50).is_none() && Quantise::new(240, 101).is_none());
    }

    /// Launch quantise (S4): the next line of the grid, at once for Now,
    /// and only grids in range pass.
    #[test]
    fn launch_grids_find_their_next_line() {
        let bar = 3840;
        assert_eq!(LaunchGrid::default(), LaunchGrid::Bars(1));
        assert_eq!(LaunchGrid::Now.next(1234, bar), 1234);
        assert_eq!(LaunchGrid::Ticks(240).next(1234, bar), 1440);
        assert_eq!(LaunchGrid::Ticks(240).next(1440, bar), 1440);
        assert_eq!(LaunchGrid::Bars(1).next(1, bar), 3840);
        assert_eq!(LaunchGrid::Bars(2).next(3841, bar), 7680);
        assert_eq!(LaunchGrid::Bars(4).next(0, bar), 0);
        // Before the song's start (a count-in), lines count back from 0.
        assert_eq!(LaunchGrid::Bars(1).next(-100, bar), 0);
        assert_eq!(LaunchGrid::Bars(1).next(-3940, bar), -3840);
        assert!(LAUNCH_GRIDS.iter().all(|g| g.ok()));
        for bad in [
            LaunchGrid::Ticks(0),
            LaunchGrid::Ticks(PPQ * 4 + 1),
            LaunchGrid::Bars(0),
            LaunchGrid::Bars(17),
        ] {
            assert!(!bad.ok(), "{bad:?}");
        }
        let mut c = Clip::new("c", 960);
        let mut s = c.settings();
        s.launch = Some(LaunchGrid::Bars(0));
        assert_eq!(c.set_settings(s), Err(ModelError::BadLaunch));
        assert_eq!(c.launch, None);
        s.launch = Some(LaunchGrid::Now);
        s.legato = true;
        assert_eq!(c.set_settings(s), Ok(()));
        assert_eq!((c.launch, c.legato), (Some(LaunchGrid::Now), true));
        let mut p = Project::new();
        p.launch = LaunchGrid::Bars(99);
        assert_eq!(p.check(), Err(ModelError::BadLaunch));
    }

    #[test]
    fn projects_check_every_clip() {
        let mut p = Project::new();
        let mut t = Track::new("drums", 0);
        t.slots[3] = Some(Clip::new("bad", 10));
        p.tracks.push(t);
        assert_eq!(p.check(), Err(ModelError::ClipLength));
        p.tracks[0].slots[3] = Some(Clip::new("ok", 960));
        assert_eq!(p.check(), Ok(()));
        assert_eq!(p.clip(0, 3).map(|c| c.length), Some(960));
        assert!(p.clip(0, 4).is_none() && p.clip(1, 0).is_none() && p.clip(0, 99).is_none());
    }
}
