//! The time engine's model: a project holds tracks, a track holds clips in
//! rows (64, the grid showing 16 at a time), a clip holds notes, and the
//! song's blocks name clips by their lasting ids. Plain data, edited only on the control
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
//! spread over its length, with a velocity ramp across the hits if set).
//! The player (`mpfree-play`) decides them. A step can be **off**
//! ([`Note::on`]: it keeps everything and is left out), and a note can be
//! **tied** to the same note before it or **slide** into it legato
//! ([`Join`]).
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
//! **Mutate** ([`mutate`]) makes a variation of a clip's notes and keeps
//! the clip as it was beside it ([`Clip::original`]), for Restore.
//!
//! Audio clips come in a later roadmap step.
#![forbid(unsafe_code)]

pub mod arrange;
pub mod loops;
pub mod mutate;
pub mod notes;
pub mod save;
pub mod scenes;
pub mod steps;
pub mod time_edit;

pub use arrange::{Arrangement, Marker, MorphRows, Placement, SongClips, TempoPoint};
pub use mutate::{Amount, Original, Pitches};
pub use scenes::{
    Follow, FollowAction, ListStep, Scene, SceneSettings, SongList, TrackProgram, MAX_SCENE_NAME,
    SCENES,
};
pub use time_edit::TimeEdit;

pub use mpfree_moves;
use mpfree_moves::Moves;
pub use mpfree_param;
use mpfree_param::{OnTrack, RelAddr};
use mpfree_transport::time::{Swing, Tick, PPQ};

/// The rows the Clips grid shows: one bank of clip slots per track, one
/// per pad. A project keeps up to [`ROWS`]; the grid shows these until
/// scene banks come (the Arrange refactor's phase 5), and what looks for
/// a free row on the grid (recording, a capture, Song → scenes) and a row
/// dragged stay within them.
pub const SLOTS: usize = 16;
/// The most rows a project keeps clips and scenes in (the Arrange
/// refactor's 1.2): four banks of [`SLOTS`]. A track keeps its clips by
/// row number ([`Rows`]), so rows past its last clip cost nothing.
pub const ROWS: usize = 4 * SLOTS;

/// The rows of the bank of [`SLOTS`] that holds `row`: 0 to 15 for the
/// grid's rows. Follow actions move within a clip's or a scene's bank.
pub fn bank(row: usize) -> std::ops::Range<usize> {
    let first = row / SLOTS * SLOTS;
    first..first + SLOTS
}
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
    /// The step is on. A step tapped off keeps its note, velocity, length,
    /// settings and locks, and the player leaves it out, with its locks,
    /// as if its chance had failed (but it sets no last result for `Pre`
    /// and `Nei`). Sequencer S6, for the Play page's step grid.
    pub on: bool,
    /// A ratchet's velocity ramp: its last hit's velocity less its first's,
    /// -[`MAX_RAMP`] to [`MAX_RAMP`]; the hits between step evenly, each
    /// kept to 1 to 127. 0 is every hit at the note's velocity, and a note
    /// with no ratchet has nothing to ramp (Elektron's retrig velocity
    /// curve; S6).
    pub ramp: i8,
    /// How it starts: with its own attack, or joined to the note before it
    /// ([`Join`]; S6).
    pub join: Join,
}

/// How a note starts ([`Note::join`]): with its own attack, or joined to
/// the note before it on its track, as a 303 ties and slides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Join {
    /// Its own attack.
    #[default]
    Attack,
    /// It continues the same note (pitch) sounding on its track as it
    /// starts, or ending on its tick: no new attack, that note lasts to
    /// this one's end. With nothing to continue, it plays as usual.
    Tie,
    /// Legato into it: the notes of its track sounding as it starts, or
    /// ending on its tick, end one tick after it starts, so the two overlap
    /// and a synth in mono or legato mode with glide slides there. Onto the
    /// same pitch it ties.
    Slide,
}

impl Join {
    /// As saved and shown: "", "tie", "slide".
    pub fn word(self) -> &'static str {
        match self {
            Join::Attack => "",
            Join::Tie => "tie",
            Join::Slide => "slide",
        }
    }
}

/// The velocity of hit `hit` (from 0) of `hits` ratchet hits that start at
/// `velocity` and ramp by `ramp` ([`Note::ramp`]) to the last: evenly
/// stepped, rounded, kept to 1 to 127. The first hit, a ramp of 0 or a
/// note with no ratchet keeps `velocity`.
pub fn ramped(velocity: u8, ramp: i8, hit: u8, hits: u8) -> u8 {
    let n = i32::from(hits.max(1));
    if n < 2 || ramp == 0 {
        return velocity;
    }
    let k = i32::from(hit).min(n - 1);
    let by = (f32::from(ramp) * k as f32 / (n - 1) as f32).round() as i32;
    (i32::from(velocity) + by).clamp(1, 127) as u8
}

/// Where a note's pressure is in its clip: values `first..first + len` of
/// [`Clip::touch`]. `len` 0 is none ([`Touch::NONE`]). A clip only ever
/// adds pressure ([`Clip::add_touch`]), so this stays valid while notes
/// are taken out, put back (undo) or moved; saving renumbers the runs as
/// it leaves out values no note names ([`TouchKeep`]), and so does a
/// clip that frees them in memory (`mpfree_play::Takes::reclaim_touch`,
/// which renumbers the notes its undo and redo hold too).
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

/// Which of a clip's pressure values ([`Clip::touch`]) are still named,
/// and where each goes once the rest are dropped (the refactor's B4).
/// Notes only ever add pressure, so values a note took out (or an undone
/// take) no longer names stay behind; saving writes only the values the
/// notes name, and a clip filling up drops the rest in memory too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TouchKeep {
    /// Each value's new index, or `u32::MAX` where it is dropped.
    to: Vec<u32>,
    kept: usize,
}

impl TouchKeep {
    /// What a clip holding `len` pressure values keeps of them for `runs`
    /// (every note's [`Touch`] that is to keep its pressure). A run past
    /// the values keeps nothing.
    pub fn of(len: usize, runs: impl IntoIterator<Item = Touch>) -> TouchKeep {
        let mut keep = vec![false; len];
        for t in runs {
            if let Some(k) = keep.get_mut(t.range()) {
                k.fill(true);
            }
        }
        let mut kept = 0;
        let to = keep
            .iter()
            .map(|&k| {
                if k {
                    kept += 1;
                    kept as u32 - 1
                } else {
                    u32::MAX
                }
            })
            .collect();
        TouchKeep { to, kept }
    }

    /// How many values it keeps.
    pub fn kept(&self) -> usize {
        self.kept
    }

    /// Whether it keeps every value (dropping would change nothing).
    pub fn keeps_all(&self) -> bool {
        self.kept == self.to.len()
    }

    /// Run `t` renumbered for the kept values; none when it isn't kept
    /// whole.
    pub fn run(&self, t: Touch) -> Touch {
        if t.is_none() {
            return Touch::NONE;
        }
        match self.to.get(t.range()) {
            Some(to) if to.iter().all(|&i| i != u32::MAX) => Touch {
                first: to[0],
                len: t.len,
            },
            _ => Touch::NONE,
        }
    }

    /// The values kept, in order.
    pub fn kept_of<'a>(&'a self, touch: &'a [TouchPoint]) -> impl Iterator<Item = TouchPoint> + 'a {
        touch
            .iter()
            .zip(&self.to)
            .filter(|(_, &i)| i != u32::MAX)
            .map(|(p, _)| *p)
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
/// The most a ratchet's velocity ramps ([`Note::ramp`]), either way: from
/// 1 to 127.
pub const MAX_RAMP: i8 = 126;

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
    pub const fn new(tick: Tick, len: Tick, note: u8, velocity: u8) -> Note {
        Note {
            tick,
            len,
            note,
            velocity,
            chance: 100,
            cond: Cond::Always,
            ratchet: 1,
            touch: Touch::NONE,
            on: true,
            ramp: 0,
            join: Join::Attack,
        }
    }

    /// The velocity of hit `hit` (from 0) of its ratchet, with its ramp
    /// ([`ramped`]).
    pub fn hit_velocity(&self, hit: u8) -> u8 {
        ramped(self.velocity, self.ramp, hit, self.ratchet)
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
            && (-MAX_RAMP..=MAX_RAMP).contains(&self.ramp)
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

/// The program change a clip sends its track's outside gear when it
/// starts (sequencer S5, the MPC's program change per sequence): on the
/// tick it starts, after the clip before it ends its notes and before its
/// own first note. `program` is 0 to 127 (shown 1 to 128); `bank`, when
/// set, goes first as bank select (CC 0 its top seven bits, CC 32 the
/// rest), 0 to [`ClipProgram::MAX_BANK`] (shown 1 up). Only a track on
/// MIDI Out sends it; on the pads it does nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipProgram {
    pub program: u8,
    pub bank: Option<u16>,
}

impl ClipProgram {
    /// The last bank select, CC 0 and CC 32 both at 127.
    pub const MAX_BANK: u16 = 16383;

    /// In range: a program of 0 to 127, a bank of 0 to [`Self::MAX_BANK`].
    pub fn ok(self) -> bool {
        self.program < 128 && self.bank.is_none_or(|b| b <= Self::MAX_BANK)
    }
}

/// What a clip's notes stand for (the instrument redesign's step 5c):
/// pad hits or MIDI notes. A note's number is a pad's (a kit's pad, 1 to
/// 128 as 0 to 127, or a pad of the synth's layout) in a clip of hits,
/// which every clip was until step 5c and every kit's still is, and a
/// pitch in a clip of notes, which a take on an instrument that plays
/// notes makes. An old clip of hits on a synth stays hits and plays
/// through the pads' layout as it always did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Holds {
    /// Pad hits (the default, and every clip saved before step 5c).
    #[default]
    Hits,
    /// MIDI notes: each note's number is its pitch.
    Notes,
}

impl Holds {
    /// The word a saved clip's `holds` line has.
    pub fn word(self) -> &'static str {
        match self {
            Holds::Hits => "hits",
            Holds::Notes => "notes",
        }
    }

    /// [`Holds::word`] read back; None for a word this build doesn't know.
    pub fn from_word(word: &str) -> Option<Holds> {
        match word {
            "hits" => Some(Holds::Hits),
            "notes" => Some(Holds::Notes),
            _ => None,
        }
    }
}

/// The order a clip plays its steps in (sequencer S6, the Deluge's and
/// the Hapax's play direction): its 1/16 steps ([`DIRECTION_STEP`] clip
/// ticks), each step's notes and locks moving with it, their place within
/// the step kept. Pass 0 is the one the clip started in.
///
/// - **Forward**: as written.
/// - **Reverse**: the last step first, every pass.
/// - **Ping-pong**: forward, then back, the end steps once each turn
///   (0 1 2 3 2 1 0 1 ...), the turns running on across passes.
/// - **Random**: every step once a pass, in an order of its own each pass,
///   which the clip's seed and the pass pick, so a pass plays the same
///   every time it is played (an export too).
///
/// Only a clip of two whole steps or more plays it ([`Clip::direction_steps`]):
/// another plays forward. Its moves (FX lanes) play forward whatever it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    #[default]
    Forward,
    Reverse,
    PingPong,
    Random,
}

/// The step a clip's [`Direction`] moves, in clip ticks: a 1/16.
pub const DIRECTION_STEP: Tick = (PPQ / 4) as Tick;

impl Direction {
    /// In the order the clip card's Direction row steps through them.
    pub const ALL: [Direction; 4] = [
        Direction::Forward,
        Direction::Reverse,
        Direction::PingPong,
        Direction::Random,
    ];

    /// Its word in a saved clip (`direction reverse`).
    pub fn word(self) -> &'static str {
        match self {
            Direction::Forward => "forward",
            Direction::Reverse => "reverse",
            Direction::PingPong => "pingpong",
            Direction::Random => "random",
        }
    }

    pub fn from_word(w: &str) -> Option<Direction> {
        Direction::ALL.into_iter().find(|d| d.word() == w)
    }

    /// The clip step a clip of `n` steps plays as its step `s` (0 to
    /// `n - 1`) of pass `pass` (0: the pass it started in; earlier ones
    /// count down), `seed` picking Random's orders. A pure function, so the
    /// player and the recorder agree, and it never allocates.
    pub fn step(self, s: u32, pass: i64, n: u32, seed: u64) -> u32 {
        if n < 2 || s >= n {
            return s.min(n.saturating_sub(1));
        }
        match self {
            Direction::Forward => s,
            Direction::Reverse => n - 1 - s,
            Direction::PingPong => {
                let period = 2 * i64::from(n) - 2;
                let q = (pass * i64::from(n) + i64::from(s)).rem_euclid(period) as u32;
                if q < n {
                    q
                } else {
                    period as u32 - q
                }
            }
            Direction::Random => shuffle(s, n, mix64(seed ^ mix64(pass as u64))),
        }
    }

    /// The step of pass `pass` that plays clip step `k`: the inverse of
    /// [`Direction::step`] in that pass, for the recorder. Ping-pong plays
    /// a step twice in some passes (going and coming back): this is the
    /// first; and none in others: then `k` itself.
    pub fn played_at(self, k: u32, pass: i64, n: u32, seed: u64) -> u32 {
        match self {
            Direction::Forward => k,
            Direction::Reverse if n >= 2 && k < n => n - 1 - k,
            Direction::Reverse => k,
            Direction::Random if n >= 2 && k < n => {
                unshuffle(k, n, mix64(seed ^ mix64(pass as u64)))
            }
            Direction::Random => k,
            Direction::PingPong => (0..n)
                .find(|&s| self.step(s, pass, n, seed) == k)
                .unwrap_or(k),
        }
    }
}

/// The steps a clip `length` ticks long with `direction` moves
/// ([`Clip::direction_steps`]).
pub fn direction_steps(direction: Direction, length: Tick) -> Option<u32> {
    let n = length / DIRECTION_STEP;
    (direction != Direction::Forward && length % DIRECTION_STEP == 0 && n >= 2)
        .then(|| u32::try_from(n).ok())
        .flatten()
}

/// splitmix64's finish: spreads a word over every bit.
pub(crate) fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `s`'s place in an order of 0 to `n - 1` that `key` picks: a
/// four-round Feistel network over the smallest even number of bits that
/// holds `n`, walked until it lands under `n` (its cycles always come
/// back under it). No allocation, so the audio thread can call it.
fn shuffle(s: u32, n: u32, key: u64) -> u32 {
    let bits = (32 - (n - 1).leading_zeros()).max(2);
    let half = bits.div_ceil(2);
    let mask = (1u32 << half) - 1;
    let round = |i: u64, r: u32| mix64(key ^ (i << 32) ^ u64::from(r)) as u32 & mask;
    let mut x = s;
    loop {
        let (mut l, mut r) = (x >> half, x & mask);
        for i in 0..4u64 {
            (l, r) = (r, l ^ round(i, r));
        }
        x = (l << half) | r;
        if x < n {
            return x;
        }
    }
}

/// [`shuffle`] backwards: the `s` whose place is `k`.
fn unshuffle(k: u32, n: u32, key: u64) -> u32 {
    let bits = (32 - (n - 1).leading_zeros()).max(2);
    let half = bits.div_ceil(2);
    let mask = (1u32 << half) - 1;
    let round = |i: u64, r: u32| mix64(key ^ (i << 32) ^ u64::from(r)) as u32 & mask;
    let mut x = k;
    loop {
        let (mut l, mut r) = (x >> half, x & mask);
        for i in (0..4u64).rev() {
            (l, r) = (r ^ round(i, l), l);
        }
        x = (l << half) | r;
        if x < n {
            return x;
        }
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
    /// This song tick, or at once once it is past: where a one-shot
    /// clip's pass ends ([`Clip::once`]). Never a clip's or the project's
    /// own setting.
    At(Tick),
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
    /// `At` is only ever a cue's, never a setting.
    pub fn ok(self) -> bool {
        match self {
            LaunchGrid::Now => true,
            LaunchGrid::Ticks(t) => (1..=PPQ * 4).contains(&t),
            LaunchGrid::Bars(n) => (1..=16).contains(&n),
            LaunchGrid::At(_) => false,
        }
    }

    /// The distance between its lines, in song ticks, with bars of `bar`
    /// ticks: 1 for at once, so the next line is the next tick.
    /// `At`'s one line is 1 apart, as at once, past its tick.
    pub fn line(self, bar: Tick) -> Tick {
        match self {
            LaunchGrid::Now | LaunchGrid::At(_) => 1,
            LaunchGrid::Ticks(t) => Tick::from(t.max(1)),
            LaunchGrid::Bars(n) => bar.max(1) * Tick::from(n.max(1)),
        }
    }

    /// Its first line at or after song tick `t`, with bars of `bar` ticks.
    pub fn next(self, t: Tick, bar: Tick) -> Tick {
        if let LaunchGrid::At(at) = self {
            return at.max(t);
        }
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
    /// Its lasting id ([`ClipId`]), given by its project as it goes in;
    /// [`ClipId::NONE`] until then.
    pub id: ClipId,
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
    /// One-shot (sequencer S4): its track stops at the end of the pass it
    /// started in, in place of looping; a follow action of its own waits.
    pub once: bool,
    /// The program change it sends when it starts (sequencer S5); None
    /// sends none.
    pub program: Option<ClipProgram>,
    /// The order it plays its steps in (sequencer S6).
    pub direction: Direction,
    /// The length of a step on its step grid, in ticks: one of
    /// [`steps::SIZES`], [`steps::DEFAULT_SIZE`] (a 1/16) unless changed
    /// (step sequencer refactor D1). It changes what the Steps, Notes and
    /// STEP SEQ show and edit, never when a note plays.
    pub step: Tick,
    /// Its notes, locks, length and moves as they were before its first
    /// Mutate (sequencer S6, [`mutate`]): what Restore puts back and each
    /// Mutate varies. None until it is mutated, and again once restored.
    pub original: Option<Box<Original>>,
    /// Whether its notes are pad hits or MIDI notes (step 5c).
    pub holds: Holds,
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
    /// Its launch quantise (None: the project's), legato and one-shot
    /// (S4).
    pub launch: Option<LaunchGrid>,
    pub legato: bool,
    pub once: bool,
    /// Its program change (S5).
    pub program: Option<ClipProgram>,
    /// Its play direction (S6).
    pub direction: Direction,
    /// Its step size on the step grid (refactor D1).
    pub step: Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    ClipLength,
    TooManyNotes,
    /// A note's tick, length, note number, velocity, chance, condition,
    /// ratchet or ramp is out of range.
    BadNote(usize),
    BadSpeed,
    BadSwing,
    BadQuantise,
    TooManyTracks,
    /// A track's transpose past [`MAX_TRANSPOSE`].
    BadTranspose,
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
    /// A clip's program or bank is out of range ([`ClipProgram::ok`]).
    BadProgram,
    /// A clip's step size isn't one of [`steps::SIZES`].
    BadStep,
    /// A loop cell names no loop or holds notes, or a clip is on a track
    /// of the other kind (a loop cell on a note track, a note clip on a
    /// Loop track).
    BadLoop,
    /// A song's tempo point is before the start, out of range, too near
    /// another (a 1/16 at least) or one too many.
    BadTempo,
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
            once: self.once,
            program: self.program,
            direction: self.direction,
            step: self.step,
        }
    }

    /// Change its speed, swing, quantise, follow action, launch quantise,
    /// legato, program, direction and step size, all or none: `BadSpeed`,
    /// `BadSwing`, `BadQuantise`, `BadLaunch`, `BadProgram`, `BadStep` or
    /// the follow action's own refusal leaves the clip as it was.
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
        if s.program.is_some_and(|p| !p.ok()) {
            return Err(ModelError::BadProgram);
        }
        if !steps::SIZES.contains(&s.step) {
            return Err(ModelError::BadStep);
        }
        self.speed = s.speed;
        self.swing = s.swing;
        self.quantise = s.quantise;
        self.follow = s.follow;
        self.launch = s.launch;
        self.legato = s.legato;
        self.once = s.once;
        self.program = s.program;
        self.direction = s.direction;
        self.step = s.step;
        Ok(())
    }

    /// The number of [`DIRECTION_STEP`] steps its [`Direction`] moves:
    /// None when it plays forward, or its length isn't two whole steps or
    /// more (then it plays forward whatever its direction says).
    pub fn direction_steps(&self) -> Option<u32> {
        direction_steps(self.direction, self.length)
    }

    /// An empty clip of `length` ticks at normal speed.
    pub fn new(name: &str, length: Tick) -> Clip {
        Clip {
            id: ClipId::NONE,
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
            once: false,
            program: None,
            direction: Direction::Forward,
            step: steps::DEFAULT_SIZE,
            original: None,
            holds: Holds::Hits,
        }
    }

    /// A copy of it, with no id: a copy is a clip of its own, and gets a
    /// new id as it goes into a project ([`Project::put_new_clip`]).
    pub fn copied(&self) -> Clip {
        Clip {
            id: ClipId::NONE,
            ..self.clone()
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
        if !steps::SIZES.contains(&self.step) {
            return Err(ModelError::BadStep);
        }
        if self.program.is_some_and(|p| !p.ok()) {
            return Err(ModelError::BadProgram);
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
                || self.original.is_some()
        }) {
            return Err(ModelError::BadLoop);
        }
        if let Some(o) = &self.original {
            o.check(&self.touch)?;
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

/// A track's lasting number in its project (the instrument redesign's
/// step 2c): not its place in the list, which changes when tracks are
/// added, moved or taken away. A project gives each new track the next
/// number ([`Project::next_track`]) and never gives one twice, so
/// something that names a track (a knob's owner, say) can't reach another
/// track by mistake. [`TrackId::NONE`] (0) is a track that has none yet:
/// one just made, or read from a project saved before tracks had ids,
/// until [`Project::give_track_ids`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackId(pub u32);

impl TrackId {
    /// No id given yet.
    pub const NONE: TrackId = TrackId(0);

    /// Whether it is a given id (not [`TrackId::NONE`]).
    pub fn is_given(self) -> bool {
        self != TrackId::NONE
    }
}

/// A clip's lasting number in its project (the Arrange refactor's D1):
/// one id space for every kind of clip, note clips and Loop track cells
/// alike. A clip keeps its id when it moves (a drag to another cell, a
/// row dragged) and when UNDO or REDO brings it back; a copy is a clip of
/// its own and gets a new one. A project gives each new clip the next
/// number ([`Project::next_clip`]) and never gives one twice, so
/// something that names a clip can't reach another clip by mistake.
/// [`ClipId::NONE`] (0) is a clip that has none yet: one just made, or
/// read from a project saved before clips had ids, until
/// [`Project::give_clip_ids`]. A clip lives in one place, a cell or the
/// song's own clips ([`arrange::SongClips`]), and song blocks name it by
/// its id ([`Placement::clip`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClipId(pub u32);

impl ClipId {
    /// No id given yet.
    pub const NONE: ClipId = ClipId(0);

    /// Whether it is a given id (not [`ClipId::NONE`]).
    pub fn is_given(self) -> bool {
        self != ClipId::NONE
    }
}

/// Where a clip is in its project ([`Project::find_clip`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipAt {
    /// In a cell: track `track`, row `row`.
    Cell { track: usize, row: usize },
    /// Among the song's own clips ([`Arrangement::clips`]).
    Song,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// Its lasting id ([`TrackId`]), given by its project.
    pub id: TrackId,
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
    /// Its live transpose (sequencer S4): every note it plays shifted, as
    /// played on the pads' Transpose or its row; 0 plays as written. Only
    /// a melodic track is ever transposed: a drum kit's notes are its
    /// pads.
    pub transpose: Transpose,
    /// "Follows transpose": the pads' Transpose moves this track with the
    /// armed one, so a bass and its chords change key together.
    pub follows: bool,
    /// Its clips by row, 0 to [`ROWS`] - 1 ([`Rows`]).
    pub slots: Rows,
}

/// A track's clips by row number (the Arrange refactor's 1.2): row `r`'s
/// clip or none, for rows 0 to [`ROWS`] - 1, kept as a list as long as
/// its last clip, so rows past it cost nothing.
///
/// `rows[r]` reads row `r` (none past the last clip) and `rows[r] = ...`
/// writes it (a row past [`ROWS`] panics, as an index past an array's end
/// does). [`Rows::get`] and [`Rows::get_mut`] are the same, None past
/// [`ROWS`]. [`Rows::iter`] lists the clips with their rows;
/// [`Rows::shown`] the rows the grid shows, empty ones too. Two are equal
/// when they hold the same clips in the same rows.
#[derive(Clone, Default)]
pub struct Rows {
    cells: Vec<Option<Clip>>,
}

/// What an empty row reads as.
static NO_CLIP: Option<Clip> = None;

impl Rows {
    /// Row `row`: its clip or none. None past [`ROWS`]. Never allocates.
    pub fn get(&self, row: usize) -> Option<&Option<Clip>> {
        (row < ROWS).then(|| self.cells.get(row).unwrap_or(&NO_CLIP))
    }

    /// Row `row` to change. None past [`ROWS`]. Allocates when the row
    /// is past the last one kept.
    pub fn get_mut(&mut self, row: usize) -> Option<&mut Option<Clip>> {
        if row >= ROWS {
            return None;
        }
        if self.cells.len() <= row {
            self.cells.resize_with(row + 1, || None);
        }
        self.cells.get_mut(row)
    }

    /// Every clip with its row, in row order.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &Clip)> + '_ {
        self.cells
            .iter()
            .enumerate()
            .filter_map(|(r, c)| c.as_ref().map(|c| (r, c)))
    }

    /// Every clip with its row, in row order, to change.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (usize, &mut Clip)> + '_ {
        self.cells
            .iter_mut()
            .enumerate()
            .filter_map(|(r, c)| c.as_mut().map(|c| (r, c)))
    }

    /// Every clip, in row order.
    pub fn clips(&self) -> impl Iterator<Item = &Clip> + '_ {
        self.cells.iter().flatten()
    }

    /// Every clip, in row order, to change.
    pub fn clips_mut(&mut self) -> impl Iterator<Item = &mut Clip> + '_ {
        self.cells.iter_mut().flatten()
    }

    /// The rows the Clips grid shows ([`SLOTS`]), empty ones too, in order.
    pub fn shown(&self) -> impl Iterator<Item = &Option<Clip>> + '_ {
        (0..SLOTS).map(|r| &self[r])
    }

    /// The row of the clip with id `id`, if this track has it.
    pub fn row_of(&self, id: ClipId) -> Option<usize> {
        if !id.is_given() {
            return None;
        }
        self.iter().find(|(_, c)| c.id == id).map(|(r, _)| r)
    }

    /// The first empty row the grid shows; None when they are all full.
    pub fn first_free(&self) -> Option<usize> {
        self.shown().position(Option::is_none)
    }

    /// No clip in any row.
    pub fn is_empty(&self) -> bool {
        self.cells.iter().all(Option::is_none)
    }

    /// Every row emptied.
    pub fn clear(&mut self) {
        self.cells.clear();
    }

    /// Empty every row whose clip `gone` picks. How many were emptied.
    pub fn empty_where(&mut self, gone: impl Fn(&Clip) -> bool) -> usize {
        let mut n = 0;
        for cell in &mut self.cells {
            if cell.as_ref().is_some_and(&gone) {
                *cell = None;
                n += 1;
            }
        }
        n
    }
}

impl std::ops::Index<usize> for Rows {
    type Output = Option<Clip>;

    fn index(&self, row: usize) -> &Option<Clip> {
        self.cells.get(row).unwrap_or(&NO_CLIP)
    }
}

impl std::ops::IndexMut<usize> for Rows {
    fn index_mut(&mut self, row: usize) -> &mut Option<Clip> {
        assert!(row < ROWS, "row {row} past the {ROWS} rows a track keeps");
        self.get_mut(row).expect("checked")
    }
}

impl PartialEq for Rows {
    fn eq(&self, other: &Rows) -> bool {
        self.iter().eq(other.iter())
    }
}

impl std::fmt::Debug for Rows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// The furthest a track's transpose goes either way: two octaves of
/// semitones, or 24 notes of its key (`mpfree_scale::MAX_SHIFT`).
pub const MAX_TRANSPOSE: i8 = 24;

/// A track's live transpose (sequencer S4, Jay's card of 17 Nov): `by`
/// semitones, or with `in_key` `by` notes of the scale its pads play in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Transpose {
    pub by: i8,
    pub in_key: bool,
}

impl Transpose {
    /// As written.
    pub const NONE: Transpose = Transpose {
        by: 0,
        in_key: false,
    };

    pub fn semitones(by: i8) -> Transpose {
        Transpose { by, in_key: false }
    }

    /// Within [`MAX_TRANSPOSE`] either way.
    pub fn ok(self) -> bool {
        (-MAX_TRANSPOSE..=MAX_TRANSPOSE).contains(&self.by)
    }

    /// It moves nothing.
    pub fn is_none(self) -> bool {
        self.by == 0
    }
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
    /// A new track, with no id yet: its project gives it one
    /// ([`Project::give_track_id`]) as it goes in.
    pub fn new(name: &str, dest: DestId) -> Track {
        Track {
            id: TrackId::NONE,
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
            transpose: Transpose::NONE,
            follows: false,
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

    /// Whether `self` and `other` have the same settings and clips, their
    /// ids and names aside (a track nobody set up has a new track's).
    /// Never allocates. Every field is named, so a new one must be
    /// placed here.
    pub fn same_settings(&self, other: &Track) -> bool {
        let Track {
            id: _,
            name: _,
            dest,
            swing,
            mute,
            solo,
            through_arp,
            record_touch,
            record_quantise,
            note_fx,
            kind,
            transpose,
            follows,
            slots,
        } = self;
        *dest == other.dest
            && *swing == other.swing
            && *mute == other.mute
            && *solo == other.solo
            && *through_arp == other.through_arp
            && *record_touch == other.record_touch
            && *record_quantise == other.record_quantise
            && *note_fx == other.note_fx
            && *kind == other.kind
            && *transpose == other.transpose
            && *follows == other.follows
            && *slots == other.slots
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
    /// The id the next track gets ([`TrackId`]): past every id the
    /// project ever gave, so none is given twice, even to a track added
    /// after one UNDO took away. 0 before any was given (a project saved
    /// before tracks had ids).
    pub next_track: u32,
    /// The id the next clip gets ([`ClipId`]): past every id the project
    /// ever gave, so none is given twice, even to a clip made after an
    /// UNDO took one away. UNDO and REDO never wind it back. 0 before any
    /// was given (a project saved before clips had ids).
    pub next_clip: u32,
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

    /// Gives `track` an id, the next one, if it has none or another
    /// track of the project has its id (a copy, say): before it goes in.
    /// Never allocates.
    pub fn give_track_id(&mut self, track: &mut Track) {
        let taken = |id: TrackId| self.tracks.iter().any(|t| t.id == id);
        if !track.id.is_given() || taken(track.id) {
            let highest = self.tracks.iter().map(|t| t.id.0).max().unwrap_or(0);
            let next = self.next_track.max(highest.saturating_add(1)).max(1);
            track.id = TrackId(next);
            self.next_track = next.saturating_add(1);
        } else {
            self.next_track = self.next_track.max(track.id.0.saturating_add(1));
        }
    }

    /// Gives every track that has no id, or has one an earlier track has
    /// too, the next id, in track order; the rest keep theirs. How many
    /// were given. A project saved before tracks had ids gets 1, 2, 3 ...
    /// Never allocates.
    pub fn give_track_ids(&mut self) -> usize {
        let highest = self.tracks.iter().map(|t| t.id.0).max().unwrap_or(0);
        let mut next = self.next_track.max(highest.saturating_add(1)).max(1);
        let mut given = 0;
        for t in 0..self.tracks.len() {
            let id = self.tracks[t].id;
            if !id.is_given() || self.tracks[..t].iter().any(|e| e.id == id) {
                self.tracks[t].id = TrackId(next);
                next = next.saturating_add(1);
                given += 1;
            }
        }
        self.next_track = next;
        given
    }

    /// The next clip id, never given before: past [`Project::next_clip`]
    /// and every id a clip of the project has. Never allocates.
    pub fn new_clip_id(&mut self) -> ClipId {
        let next = self
            .next_clip
            .max(self.highest_clip_id().saturating_add(1))
            .max(1);
        self.next_clip = next.saturating_add(1);
        ClipId(next)
    }

    /// The highest id a clip has, in a cell or the song's (0 for none).
    fn highest_clip_id(&self) -> u32 {
        let song = self.arrangement.clips.iter().last().map_or(0, |c| c.id.0);
        self.cell_clips()
            .map(|(_, _, c)| c.id.0)
            .max()
            .unwrap_or(0)
            .max(song)
    }

    /// Every clip in a cell, as (track, row, clip), in track and row
    /// order.
    pub fn cell_clips(&self) -> impl Iterator<Item = (usize, usize, &Clip)> {
        self.tracks
            .iter()
            .enumerate()
            .flat_map(|(t, tr)| tr.slots.iter().map(move |(r, c)| (t, r, c)))
    }

    /// Put `clip`, a new clip or a copy, in the empty cell `row` of
    /// `track`, with a new id ([`Project::new_clip_id`]), whatever id it
    /// had. Its id, or `BadSlot` (changing nothing) for no such cell or one
    /// that holds a clip.
    pub fn put_new_clip(
        &mut self,
        track: usize,
        row: usize,
        mut clip: Clip,
    ) -> Result<ClipId, ModelError> {
        let free = self
            .tracks
            .get(track)
            .and_then(|t| t.slots.get(row))
            .is_some_and(Option::is_none);
        if !free {
            return Err(ModelError::BadSlot);
        }
        clip.id = self.new_clip_id();
        let id = clip.id;
        self.tracks[track].slots[row] = Some(clip);
        Ok(id)
    }

    /// Gives every clip in a cell that has no id, or has one an earlier
    /// clip has too (in track and row order), the next id, then the song's
    /// own clips the same way; the rest keep theirs. How many were given.
    /// A project saved before clips had ids gets 1, 2, 3 ... in track and
    /// row order. Allocates a list of the ids seen.
    pub fn give_clip_ids(&mut self) -> usize {
        let mut next = self
            .next_clip
            .max(self.highest_clip_id().saturating_add(1))
            .max(1);
        let mut seen = std::collections::HashSet::new();
        let mut given = 0;
        for tr in &mut self.tracks {
            for c in tr.slots.clips_mut() {
                if !c.id.is_given() || !seen.insert(c.id) {
                    c.id = ClipId(next);
                    seen.insert(c.id);
                    next = next.saturating_add(1);
                    given += 1;
                }
            }
        }
        given += self
            .arrangement
            .clips
            .give_ids(&mut next, |id| seen.contains(&id));
        self.next_clip = next;
        given
    }

    /// Where the clip with id `id` is now. None for [`ClipId::NONE`] or
    /// an id no clip of the project has.
    pub fn find_clip(&self, id: ClipId) -> Option<ClipAt> {
        if !id.is_given() {
            return None;
        }
        self.cell_clips()
            .find(|(_, _, c)| c.id == id)
            .map(|(track, row, _)| ClipAt::Cell { track, row })
            .or_else(|| self.arrangement.clips.get(id).map(|_| ClipAt::Song))
    }

    /// Where the track with id `id` is now.
    pub fn track_with(&self, id: TrackId) -> Option<usize> {
        if !id.is_given() {
            return None;
        }
        self.tracks.iter().position(|t| t.id == id)
    }

    /// Every track and clip in range, and no clip of the song's own with
    /// an id a cell's clip has (a clip lives in one place).
    pub fn check(&self) -> Result<(), ModelError> {
        self.check_tracks()?;
        for t in &self.tracks {
            for c in t.slots.clips() {
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
        let song = &self.arrangement.clips;
        if !song.is_empty() && self.cell_clips().any(|(_, _, c)| song.get(c.id).is_some()) {
            return Err(ModelError::BadPlacement);
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
        if self.tracks.iter().any(|t| !t.transpose.ok()) {
            return Err(ModelError::BadTranspose);
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

    /// Whether row `row` is empty: no clip on any track, and a plain scene
    /// (no name, mutes, follow action, morph values or program changes).
    /// The one test of a free row for everything that looks for room
    /// (Song → scenes, a copied row), so a row set up with only a name or
    /// mutes is never taken.
    pub fn row_empty(&self, row: usize) -> bool {
        !self.row_filled(row) && self.scene(row).is_none_or(|s| *s == Scene::default())
    }

    /// The clip in a slot, if there is one.
    pub fn clip(&self, track: usize, slot: usize) -> Option<&Clip> {
        self.tracks.get(track)?.slots.get(slot)?.as_ref()
    }
}

#[cfg(test)]
mod track_id_tests {
    use super::*;

    /// Instrument redesign step 2c: tracks get lasting ids in order, a
    /// copy or a track sharing an id gets a new one, and an id is never
    /// given twice, even after its track is gone.
    #[test]
    fn tracks_get_lasting_ids_that_are_never_given_twice() {
        let mut p = Project::new();
        for i in 0..3 {
            p.tracks.push(Track::new(&format!("Track {}", i + 1), i));
        }
        assert_eq!(p.give_track_ids(), 3);
        let ids: Vec<u32> = p.tracks.iter().map(|t| t.id.0).collect();
        assert_eq!((ids, p.next_track), (vec![1, 2, 3], 4));
        assert_eq!(p.give_track_ids(), 0, "they keep theirs");
        // A copy of track 2 and a new track: the next ids.
        let mut copy = p.tracks[1].clone();
        p.give_track_id(&mut copy);
        assert_eq!(copy.id, TrackId(4));
        p.tracks.push(copy);
        p.tracks.pop();
        let mut new = Track::new("new", 3);
        p.give_track_id(&mut new);
        assert_eq!(new.id, TrackId(5), "4 is never given again");
        // Two that share one (a damaged file): the later one gets a new id.
        p.tracks[2].id = TrackId(1);
        assert_eq!(p.give_track_ids(), 1);
        assert_eq!(p.tracks[2].id, TrackId(6));
        assert_eq!(p.track_with(TrackId(6)), Some(2));
        assert_eq!(p.track_with(TrackId::NONE), None);
        // A track read with its own id keeps it, and the next id moves past it.
        let mut q = Project::new();
        let mut t = Track::new("kept", 0);
        t.id = TrackId(40);
        q.give_track_id(&mut t);
        assert_eq!((t.id, q.next_track), (TrackId(40), 41));
    }

    /// Arrange 1.1: clips get lasting ids in track and row order, loop
    /// cells too; a new clip or a copy gets the next one, whatever id it
    /// carried; a clip sharing an id gets a new one; and an id is never
    /// given twice, even after its clip is gone.
    #[test]
    fn clips_get_lasting_ids_that_are_never_given_twice() {
        let mut p = Project::new();
        p.tracks.push(Track::new("Drums", 0));
        p.tracks.push(Track::new_loops("Loops", 1));
        p.tracks[0].slots[3] = Some(Clip::new("b", 3840));
        p.tracks[0].slots[1] = Some(Clip::new("a", 3840));
        p.tracks[1].slots[0] = Some(Clip::new_loop(0, 3840));
        assert_eq!(p.give_clip_ids(), 3);
        let ids: Vec<(usize, usize, u32)> =
            p.cell_clips().map(|(t, r, c)| (t, r, c.id.0)).collect();
        assert_eq!(ids, [(0, 1, 1), (0, 3, 2), (1, 0, 3)]);
        assert_eq!(p.next_clip, 4);
        assert_eq!(p.give_clip_ids(), 0, "they keep theirs");
        assert_eq!(
            p.find_clip(ClipId(2)),
            Some(ClipAt::Cell { track: 0, row: 3 })
        );
        assert_eq!(p.find_clip(ClipId::NONE), None);
        assert_eq!(p.find_clip(ClipId(9)), None);
        // A copy has no id until it goes in; then the next one.
        let copy = p.clip(0, 1).unwrap().copied();
        assert_eq!((copy.id, copy.name.as_str()), (ClipId::NONE, "a"));
        assert_eq!(p.put_new_clip(0, 2, copy), Ok(ClipId(4)));
        // Put in with the id of a clip there already, it still gets a new one.
        let twin = p.clip(0, 1).unwrap().clone();
        assert_eq!(p.put_new_clip(0, 5, twin.clone()), Ok(ClipId(5)));
        assert_eq!(p.put_new_clip(0, 5, twin), Err(ModelError::BadSlot));
        assert_eq!(
            p.put_new_clip(0, ROWS, Clip::new("x", 3840)),
            Err(ModelError::BadSlot)
        );
        assert_eq!(
            p.put_new_clip(7, 0, Clip::new("x", 3840)),
            Err(ModelError::BadSlot)
        );
        assert_eq!(p.next_clip, 6, "a refusal gives none");
        // Gone, its id is never given again.
        p.tracks[0].slots[5] = None;
        assert_eq!(p.new_clip_id(), ClipId(6));
        // Two that share one (a damaged file): the later one gets a new id.
        p.tracks[1].slots[0].as_mut().unwrap().id = ClipId(1);
        assert_eq!(p.give_clip_ids(), 1);
        assert_eq!(p.clip(1, 0).unwrap().id, ClipId(7));
        assert_eq!(p.clip(0, 1).unwrap().id, ClipId(1), "the first keeps it");
        // A clip read with a high id keeps it, and the next id moves past it.
        let mut q = Project::new();
        q.tracks.push(Track::new("t", 0));
        let mut c = Clip::new("kept", 3840);
        c.id = ClipId(40);
        q.tracks[0].slots[0] = Some(c);
        assert_eq!(q.give_clip_ids(), 0);
        assert_eq!((q.next_clip, q.new_clip_id()), (41, ClipId(41)));
    }

    /// Arrange 1.2: a track keeps its clips by row, up to ROWS; a row
    /// past its last clip reads empty; two tracks' rows are equal when
    /// they hold the same clips in the same rows; the grid's rows are
    /// the first SLOTS, and a free one is looked for there.
    #[test]
    fn a_track_keeps_its_clips_by_row_up_to_64() {
        let mut r = Rows::default();
        assert!(r.is_empty() && r[40].is_none() && r.get(40) == Some(&None));
        assert_eq!(r.get(ROWS), None);
        r[40] = Some(Clip::new("far", 3840));
        r[2] = Some(Clip::new("near", 3840));
        let rows: Vec<(usize, &str)> = r.iter().map(|(k, c)| (k, c.name.as_str())).collect();
        assert_eq!(rows, [(2, "near"), (40, "far")]);
        assert_eq!(r.shown().count(), SLOTS);
        assert_eq!(r.first_free(), Some(0));
        assert!(r.get_mut(ROWS).is_none());
        // An empty row written and left empty changes nothing.
        let mut same = r.clone();
        *same.get_mut(63).unwrap() = None;
        assert_eq!(same, r);
        assert_eq!(format!("{r:?}").matches("Clip {").count(), 2);
        // The grid full, past its rows is not a free one.
        for k in 0..SLOTS {
            r[k] = Some(Clip::new("x", 960));
        }
        assert_eq!(r.first_free(), None);
        assert_eq!(r.empty_where(|c| c.name == "x"), SLOTS);
        assert_eq!(r.clips().count(), 1);
        // Banks of SLOTS rows.
        assert_eq!((bank(0), bank(15), bank(40)), (0..16, 0..16, 32..48));
        // A project's checks and scenes go to row 63, no further.
        let mut p = Project::new();
        p.tracks.push(Track::new("t", 0));
        p.tracks[0].slots[63] = Some(Clip::new("last", 960));
        assert!(p.scene_mut(63).is_some() && p.scene_mut(ROWS).is_none());
        assert_eq!(p.check(), Ok(()));
        assert!(p.row_filled(63) && !p.row_empty(63));
    }

    #[test]
    #[should_panic(expected = "past the 64 rows")]
    fn a_row_past_64_is_not_a_row() {
        let mut r = Rows::default();
        r[ROWS] = Some(Clip::new("x", 960));
    }

    /// Two tracks with the same settings and clips match whatever their
    /// ids and names; any setting or clip tells them apart.
    #[test]
    fn same_settings_looks_past_the_id_and_the_name() {
        let mut a = Track::new("Track 3", 2);
        a.id = TrackId(9);
        let fresh = Track::new("", 2);
        assert!(a.same_settings(&fresh));
        let mut b = a.clone();
        b.mute = true;
        assert!(!b.same_settings(&fresh));
        let mut c = a.clone();
        c.slots[3] = Some(Clip::new("x", 3840));
        assert!(!c.same_settings(&fresh));
        assert!(!a.same_settings(&Track::new("", 3)), "another destination");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Play directions (S6): each a pass plays every step it should, and
    /// `played_at` finds where a step plays.
    #[test]
    fn play_directions_order_the_steps() {
        let order = |d: Direction, pass: i64, n: u32| -> Vec<u32> {
            (0..n).map(|s| d.step(s, pass, n, 7)).collect()
        };
        assert_eq!(order(Direction::Forward, 3, 4), [0, 1, 2, 3]);
        assert_eq!(order(Direction::Reverse, 3, 4), [3, 2, 1, 0]);
        // 0 1 2 3 2 1 | 0 1 2 3 2 1 ..., the turns across passes.
        assert_eq!(order(Direction::PingPong, 0, 4), [0, 1, 2, 3]);
        assert_eq!(order(Direction::PingPong, 1, 4), [2, 1, 0, 1]);
        assert_eq!(order(Direction::PingPong, 2, 4), [2, 3, 2, 1]);
        assert_eq!(order(Direction::PingPong, -1, 4), [2, 3, 2, 1]);
        assert_eq!(order(Direction::PingPong, 5, 2), [0, 1]);
        for n in [2u32, 3, 5, 16, 17, 64, 255, 256, 1000, 4096] {
            for pass in [-3i64, 0, 1, 2, 1 << 40] {
                let mut o = order(Direction::Random, pass, n);
                for (s, &k) in o.iter().enumerate() {
                    assert_eq!(Direction::Random.played_at(k, pass, n, 7), s as u32);
                    assert_eq!(
                        Direction::Reverse.played_at(n - 1 - s as u32, pass, n, 7),
                        s as u32
                    );
                }
                o.sort_unstable();
                assert!(o.iter().copied().eq(0..n), "n {n}, pass {pass}");
            }
        }
        // Another pass, or another seed, plays another order; the same
        // pass plays the same.
        let r = |pass, seed| -> Vec<u32> {
            (0..16)
                .map(|s| Direction::Random.step(s, pass, 16, seed))
                .collect()
        };
        assert_ne!(r(0, 7), r(1, 7));
        assert_ne!(r(0, 7), r(0, 8));
        assert_eq!(r(4, 9), r(4, 9));
        // Pass 1 of four steps plays 2 1 0 1: step 1 first where it comes
        // first, and step 3 not at all (it says where it would be).
        let at = |k| Direction::PingPong.played_at(k, 1, 4, 0);
        assert_eq!([at(0), at(1), at(2), at(3)], [2, 1, 0, 3]);
        // Only whole steps, two or more.
        assert_eq!(direction_steps(Direction::Reverse, 3840), Some(16));
        assert_eq!(direction_steps(Direction::Forward, 3840), None);
        assert_eq!(direction_steps(Direction::Reverse, 3841), None);
        assert_eq!(direction_steps(Direction::Reverse, DIRECTION_STEP), None);
        assert_eq!(Direction::Reverse.step(0, 0, 1, 0), 0);
    }

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
            on: true,
            ramp: 0,
            join: Join::Attack,
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
            on: true,
            ramp: 0,
            join: Join::Attack,
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
    fn a_ratchets_ramp_steps_its_hits_velocity_and_stays_in_range() {
        let mut n = Note::new(0, 240, 36, 100);
        n.ratchet = 4;
        n.ramp = -60;
        let hits: Vec<u8> = (0..4).map(|h| n.hit_velocity(h)).collect();
        assert_eq!(hits, vec![100, 80, 60, 40]);
        // Rounded, kept to 1 to 127, past the last hit the last's.
        assert_eq!(ramped(100, 50, 1, 3), 125);
        assert_eq!(ramped(100, 126, 2, 3), 127);
        assert_eq!(ramped(10, -126, 2, 3), 1);
        assert_eq!(ramped(90, 7, 9, 3), 97);
        // No ratchet or no ramp: the note's velocity.
        assert_eq!(ramped(90, -50, 0, 1), 90);
        assert_eq!(ramped(90, 0, 3, 4), 90);
        // A clip refuses a ramp out of range, and every step setting is
        // a plain note's by default.
        let mut c = Clip::new("c", 960);
        assert!(c.add_note(Note { ramp: -127, ..n }).is_err());
        assert!(c
            .add_note(Note {
                ramp: MAX_RAMP,
                ..n
            })
            .is_ok());
        let plain = Note::new(0, 1, 0, 1);
        assert!(plain.on && plain.ramp == 0 && plain.join == Join::Attack);
        assert_eq!(Join::default(), Join::Attack);
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
                on: true,
                ramp: 0,
                join: Join::Attack,
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
                on: true,
                ramp: 0,
                join: Join::Attack,
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
                on: true,
                ramp: 0,
                join: Join::Attack,
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
                on: true,
                ramp: 0,
                join: Join::Attack,
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
                on: true,
                ramp: 0,
                join: Join::Attack,
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
            on: true,
            ramp: 0,
            join: Join::Attack,
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
            on: true,
            ramp: 0,
            join: Join::Attack,
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
    fn a_clips_program_is_checked_with_its_settings() {
        let ok = ClipProgram {
            program: 127,
            bank: Some(ClipProgram::MAX_BANK),
        };
        assert!(ok.ok());
        let mut c = Clip::new("c", 960);
        let mut s = c.settings();
        for bad in [
            ClipProgram {
                program: 128,
                bank: None,
            },
            ClipProgram {
                program: 0,
                bank: Some(ClipProgram::MAX_BANK + 1),
            },
        ] {
            assert!(!bad.ok(), "{bad:?}");
            s.program = Some(bad);
            assert_eq!(c.set_settings(s), Err(ModelError::BadProgram));
            assert_eq!(c.program, None);
            c.program = Some(bad);
            assert_eq!(c.check(), Err(ModelError::BadProgram));
            c.program = None;
        }
        s.program = Some(ok);
        assert_eq!(c.set_settings(s), Ok(()));
        assert_eq!(c.settings().program, Some(ok));
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
