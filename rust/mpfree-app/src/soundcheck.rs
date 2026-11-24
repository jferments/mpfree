//! Soundcheck (live shows B2b, layout spec 4p): plays a set through on
//! this MPC and keeps what it measured in each song's line of the set, so
//! the set check and the Show view's COLD LOAD pause use real numbers
//! instead of estimates. Pure, so every step is tested here; the screen
//! (gui.rs) feeds it the open's news, the audio loop's numbers once a
//! frame and the ending's news, and does what it answers ([`Do`]).
//!
//! For each song, three steps:
//! 1. **Opened cold**: the song opens all or nothing, as a set opens it,
//!    timed from the open to the song being in, with the memory it took.
//! 2. **Its busiest part**: the song list's step with the most clips
//!    playing at once (audio clips, which stream, and plugin tracks count
//!    twice; the first of a tie), from its first bar for up to 16 bars;
//!    with no song list, the song's first 16 bars. The CPU now and at its
//!    worst, the longest a launch waited for its audio, misses and xruns
//!    are kept as it plays.
//! 3. **The change into the next song**: the song ends by its own Ends,
//!    then the next song opens cold, timed from Go to that song being in.
//!    The set's last song has none.
//!
//! "Soundcheck this song" checks one song, and times its change into the
//! song after it (that song opens, but its part isn't played).
//!
//! A song's line is kept as soon as its change is timed (or after its part,
//! for the last song), so Stop keeps everything measured so far. A song
//! that doesn't open is passed over, and says so. Muted measures the same
//! with nothing leaving the outputs. At the end, or at Stop, the sound
//! comes back and the song that was open before opens again.

use crate::set_play::end_of;
use crate::song_end::End;
use mpfree_library::{Loc, Set, SetSong, Soundcheck as Kept};
use mpfree_time_ui::list::ListRow;
use mpfree_time_ui::view::TrackInfo;

/// The most bars of its busiest part a song plays.
pub const PART_BARS: i64 = 16;
/// How often the memory is read while a part plays, in ms.
pub const MEMORY_EVERY_MS: u64 = 250;
/// For the time left: a song's open, and its ending, until measured.
const OPEN_GUESS_MS: u64 = 5_000;
const ENDING_GUESS_MS: u64 = 5_000;
/// The tempo the time left assumes for a song whose tempo isn't known.
const TEMPO_GUESS: f64 = 120.0;

/// The part of a song Soundcheck plays.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Busiest {
    /// Its step's name ("Chorus 2"); empty for a song with no song list.
    pub name: String,
    /// Its step in the song list, which plays it from there (None: no
    /// song list, it plays from bar 1).
    pub step: Option<usize>,
    /// Its first bar, from 1.
    pub bar: i64,
    /// How many bars it plays: the step's, at most [`PART_BARS`].
    pub bars: i64,
}

/// A track as the busiest part weighs it: which scenes have a clip on it,
/// and whether it counts twice (an audio track, whose clips stream, or a
/// plugin's track).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackLoad {
    pub clips: Vec<bool>,
    pub heavy: bool,
}

/// The tracks as the busiest part weighs them.
pub fn loads(tracks: &[TrackInfo]) -> Vec<TrackLoad> {
    tracks
        .iter()
        .map(|t| TrackLoad {
            clips: t.slots.iter().map(Option::is_some).collect(),
            heavy: t.audio || crate::plugins::Plugins::is_plugin(&t.sound),
        })
        .collect()
}

/// The busiest part of a song with song list `list` and tracks `tracks`:
/// the step whose scene has the most clips, an audio clip (it streams) or
/// a clip on a plugin's track counting twice; the first of a tie. With no
/// song list, the first 16 bars.
pub fn busiest(list: &[ListRow], tracks: &[TrackLoad]) -> Busiest {
    let weight = |scene: usize| -> usize {
        tracks
            .iter()
            .filter(|t| t.clips.get(scene).copied().unwrap_or(false))
            .map(|t| if t.heavy { 2 } else { 1 })
            .sum()
    };
    let mut best: Option<(usize, usize, &ListRow)> = None;
    for (k, row) in list
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.lead && r.bars > 0)
    {
        let w = row.scene.map_or(0, weight);
        if best.is_none_or(|(b, _, _)| w > b) {
            best = Some((w, k, row));
        }
    }
    match best {
        Some((_, k, row)) => Busiest {
            name: row.name.clone(),
            step: Some(k),
            bar: row.starts.max(1),
            bars: row.bars.min(PART_BARS),
        },
        None => Busiest {
            name: String::new(),
            step: None,
            bar: 1,
            bars: PART_BARS,
        },
    }
}

/// What the screen does for Soundcheck.
#[derive(Clone, Debug, PartialEq)]
pub enum Do {
    /// The hint line says this.
    Tell(String),
    /// Nothing leaves the outputs (Muted), or the sound comes back.
    Mute(bool),
    /// Open this song of the set cold (the browser's `open_set_song`).
    Open(Box<SetSong>),
    /// Play the song in Perform from this step of its song list (None:
    /// from bar 1).
    Play(Option<usize>),
    /// The song ends so (the audio loop's `SongEnd`).
    End(End),
    /// The transport stops.
    Stop,
    /// Full sound again, with no ending.
    Reset,
    /// Keep song `k`'s line in the set.
    Keep(usize, Kept),
    /// The song that was open before opens again.
    Reopen,
}

/// Where Soundcheck stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// A song is opening (the first, or the next one in a change).
    Opening,
    /// A song's busiest part plays.
    Playing,
    /// The song ends by its Ends, before the next one opens.
    Ending,
    /// It's over: all done, or stopped.
    Done,
}

/// One frame's numbers while a part plays.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    /// Bars played of the part so far.
    pub bars_in: f64,
    /// The CPU now (the sound's share of the period), in %.
    pub cpu: Option<f64>,
    /// A launch is waiting for its audio from the stick.
    pub waiting: bool,
    /// Clips that played without their audio, and seconds of takes that
    /// played as silence, since the start (the misses' counters).
    pub misses: u64,
    /// The sound card ran out of sound this many times since the start.
    pub xruns: u64,
    /// Memory the song takes now, in kB, when read this frame.
    pub memory_kb: Option<u64>,
}

/// What one song's soundcheck has measured so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measure {
    pub open_ms: Option<u32>,
    pub peak_kb: u64,
    pub cpu_now: Option<f64>,
    pub cpu_worst: Option<f64>,
    /// The longest a launch waited for its audio, ms.
    pub wait_ms: u32,
    pub misses: u32,
    pub xruns: u32,
    pub change_ms: Option<u32>,
    pub part: Busiest,
    /// Bars of the part played so far.
    pub bars_in: f64,
    /// The song didn't open: why.
    pub failed: Option<String>,
    /// Its part has played (or was skipped).
    pub played: bool,
    pub skipped: bool,
}

impl Measure {
    fn kept(&self, at: u64) -> Kept {
        Kept {
            at,
            open_ms: self.open_ms.unwrap_or(0),
            peak_kb: self.peak_kb,
            cpu_x10: self
                .cpu_worst
                .map_or(0, |c| (c * 10.0).round().clamp(0.0, 9999.0) as u16),
            disk_wait_ms: self.wait_ms,
            misses: self.misses,
            xruns: self.xruns,
            change_ms: self.change_ms,
            part: self.part.name.clone(),
        }
    }
}

/// Soundcheck running.
#[derive(Clone, Debug)]
pub struct Soundcheck {
    pub loc: Loc,
    pub set: Set,
    /// The set's songs it checks, in order.
    pub songs: Vec<usize>,
    /// The song after the last one, opened only to time the change into
    /// it ("Soundcheck this song").
    then: Option<usize>,
    /// Which of `songs` is being checked (`songs.len()` for `then`).
    pos: usize,
    stage: Stage,
    pub muted: bool,
    /// When it started: UTC seconds (for the lines kept) and the screen's ms.
    at: u64,
    started_ms: u64,
    /// When the open under way started, and when Go started the change.
    open_from: u64,
    go_from: Option<u64>,
    /// The part started (the frame Play was sent) and the bar it starts on.
    measures: Vec<Measure>,
    /// The stream counters when the part started.
    misses_from: Option<u64>,
    xruns_from: Option<u64>,
    /// A launch has waited since this ms.
    waiting_from: Option<u64>,
    /// Kept to the set: each of `songs`.
    kept: Vec<bool>,
    stopped: bool,
    /// When it ended, for the summary.
    ended_ms: Option<u64>,
}

impl Soundcheck {
    /// Soundcheck for the set at `loc`: every song, or only song `only`
    /// (with its change into the next), `muted` or out loud, starting at
    /// `now_ms` (the screen's clock) and `at` (UTC seconds). What to do
    /// first.
    pub fn new(
        loc: Loc,
        set: Set,
        only: Option<usize>,
        muted: bool,
        now_ms: u64,
        at: u64,
    ) -> (Soundcheck, Vec<Do>) {
        let (songs, then) = match only {
            Some(k) if k < set.songs.len() => (vec![k], (k + 1 < set.songs.len()).then_some(k + 1)),
            Some(_) => (Vec::new(), None),
            None => ((0..set.songs.len()).collect(), None),
        };
        let n = songs.len();
        let mut sc = Soundcheck {
            loc,
            set,
            songs,
            then,
            pos: 0,
            stage: Stage::Opening,
            muted,
            at,
            started_ms: now_ms,
            open_from: now_ms,
            go_from: None,
            measures: vec![Measure::default(); n],
            misses_from: None,
            xruns_from: None,
            waiting_from: None,
            kept: vec![false; n],
            stopped: false,
            ended_ms: None,
        };
        let mut fx = Vec::new();
        if muted {
            fx.push(Do::Mute(true));
        }
        match sc.song_at(0).cloned() {
            Some(song) => fx.push(Do::Open(Box::new(song))),
            None => fx.extend(sc.finish(now_ms)),
        }
        (sc, fx)
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The set's song being checked now (or opened for its change), from 0.
    pub fn song_now(&self) -> Option<usize> {
        self.songs
            .get(self.pos)
            .copied()
            .or(self.then.filter(|_| self.pos == self.songs.len()))
    }

    /// Which of the songs it checks is being checked, from 0.
    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn measures(&self) -> &[Measure] {
        &self.measures
    }

    /// It's over.
    pub fn done(&self) -> bool {
        self.stage == Stage::Done
    }

    pub fn stopped(&self) -> bool {
        self.stopped
    }

    fn song_at(&self, pos: usize) -> Option<&SetSong> {
        let k = if pos < self.songs.len() {
            self.songs[pos]
        } else if pos == self.songs.len() {
            self.then?
        } else {
            return None;
        };
        self.set.songs.get(k)
    }

    /// The song opening is in, at `now_ms`, its busiest part `part`: its
    /// open is timed, the change into it too, and its part plays (or, for
    /// the song opened only to time the change into it, it's over).
    pub fn opened(&mut self, now_ms: u64, part: Busiest) -> Vec<Do> {
        if self.stage != Stage::Opening {
            return Vec::new();
        }
        let mut fx = Vec::new();
        let open_ms = ms(now_ms.saturating_sub(self.open_from));
        // The change into this song, kept with the song before it.
        if let Some(go) = self.go_from.take() {
            if let Some(m) = self
                .pos
                .checked_sub(1)
                .and_then(|p| self.measures.get_mut(p))
            {
                m.change_ms = Some(ms(now_ms.saturating_sub(go)));
            }
            fx.extend(self.keep(self.pos.wrapping_sub(1)));
        }
        if self.pos >= self.songs.len() {
            // The song after "Soundcheck this song": only its change.
            fx.push(Do::Stop);
            fx.extend(self.finish(now_ms));
            return fx;
        }
        let m = &mut self.measures[self.pos];
        m.open_ms = Some(open_ms);
        m.part = part.clone();
        self.stage = Stage::Playing;
        (self.misses_from, self.xruns_from, self.waiting_from) = (None, None, None);
        fx.push(Do::Play(part.step));
        fx
    }

    /// The song opening didn't open: why. It's passed over (nothing of it
    /// is kept), and the next song opens.
    pub fn open_failed(&mut self, now_ms: u64, why: &str) -> Vec<Do> {
        if self.stage != Stage::Opening {
            return Vec::new();
        }
        let name = self
            .song_at(self.pos)
            .map_or(String::new(), |s| s.name.clone());
        let mut fx = vec![Do::Tell(format!(
            "Soundcheck: {name} didn\u{2019}t open ({why}), so it goes on to the next song"
        ))];
        // The song before keeps its line, with no change timed.
        if self.go_from.take().is_some() {
            fx.extend(self.keep(self.pos.wrapping_sub(1)));
        }
        if let Some(m) = self.measures.get_mut(self.pos) {
            m.failed = Some(why.to_string());
        }
        fx.extend(self.next_song(now_ms));
        fx
    }

    /// One frame while a part plays: its numbers. When the part has played
    /// its bars, the song ends (into the next one) or Soundcheck is done.
    pub fn tick(&mut self, now_ms: u64, s: Sample) -> Vec<Do> {
        if self.stage != Stage::Playing {
            return Vec::new();
        }
        let misses_from = *self.misses_from.get_or_insert(s.misses);
        let xruns_from = *self.xruns_from.get_or_insert(s.xruns);
        let m = &mut self.measures[self.pos];
        m.bars_in = s.bars_in.max(0.0);
        m.cpu_now = s.cpu;
        if let Some(c) = s.cpu {
            m.cpu_worst = Some(m.cpu_worst.map_or(c, |w| w.max(c)));
        }
        // The counters only rise while a song is open: the most seen.
        let delta =
            |now: u64, from: u64| u32::try_from(now.saturating_sub(from)).unwrap_or(u32::MAX);
        m.misses = m.misses.max(delta(s.misses, misses_from));
        m.xruns = m.xruns.max(delta(s.xruns, xruns_from));
        if let Some(kb) = s.memory_kb {
            m.peak_kb = m.peak_kb.max(kb);
        }
        // A launch waiting for its audio: how long, up to the frame it
        // stops waiting.
        if let Some(from) = self.waiting_from {
            m.wait_ms = m.wait_ms.max(ms(now_ms.saturating_sub(from)));
        }
        if s.waiting {
            self.waiting_from.get_or_insert(now_ms);
        } else {
            self.waiting_from = None;
        }
        if m.bars_in + 1e-9 >= m.part.bars as f64 {
            m.played = true;
            return self.part_over(now_ms);
        }
        Vec::new()
    }

    /// The part is over (played or skipped): the song ends into the next
    /// one, or, with none, Soundcheck is done.
    fn part_over(&mut self, now_ms: u64) -> Vec<Do> {
        let has_next = self.pos + 1 < self.songs.len() || self.then.is_some();
        if has_next {
            self.stage = Stage::Ending;
            self.go_from = Some(now_ms);
            let end = self.song_at(self.pos).map_or(End::Cut, end_of);
            return vec![Do::End(end)];
        }
        let mut fx = self.keep(self.pos);
        fx.push(Do::Stop);
        fx.extend(self.finish(now_ms));
        fx
    }

    /// The song ending is silent: the next one opens.
    pub fn ended(&mut self, now_ms: u64) -> Vec<Do> {
        if self.stage != Stage::Ending {
            return Vec::new();
        }
        self.pos += 1;
        self.stage = Stage::Opening;
        self.open_from = now_ms;
        match self.song_at(self.pos).cloned() {
            Some(song) => vec![Do::Open(Box::new(song))],
            None => self.finish(now_ms),
        }
    }

    /// The next song opens with no change timed (after one that didn't
    /// open), or it's over.
    fn next_song(&mut self, now_ms: u64) -> Vec<Do> {
        self.pos += 1;
        self.open_from = now_ms;
        self.go_from = None;
        if self.pos >= self.songs.len() {
            // Nothing more to measure (the song after it is only for a
            // change, which a song that didn't open hasn't got).
            return self.finish(now_ms);
        }
        self.stage = Stage::Opening;
        match self.song_at(self.pos).cloned() {
            Some(song) => vec![Do::Open(Box::new(song))],
            None => self.finish(now_ms),
        }
    }

    /// Skip this song: its part stops where it is (what it measured is
    /// kept, as Stop keeps it) and the change into the next song starts.
    pub fn skip(&mut self, now_ms: u64) -> Vec<Do> {
        match self.stage {
            Stage::Playing => {
                self.measures[self.pos].skipped = true;
                self.part_over(now_ms)
            }
            _ => vec![Do::Tell("Skip this song: once its part plays".into())],
        }
    }

    /// Stop (Stop soundcheck or STOP): everything measured so far is kept,
    /// the sound comes back and the song open before opens again.
    pub fn stop(&mut self, now_ms: u64) -> Vec<Do> {
        if self.stage == Stage::Done {
            return Vec::new();
        }
        self.stopped = true;
        let mut fx = Vec::new();
        // The song whose change was under way keeps its line without it;
        // a song whose part was playing keeps what it measured.
        let keep_now = match self.stage {
            Stage::Ending => Some(self.pos),
            Stage::Opening if self.go_from.is_some() => self.pos.checked_sub(1),
            Stage::Playing => Some(self.pos),
            _ => None,
        };
        if let Some(p) = keep_now {
            fx.extend(self.keep(p));
        }
        fx.push(Do::Stop);
        fx.extend(self.finish(now_ms));
        fx
    }

    /// Song `pos` (of `songs`)'s line goes into the set, once.
    fn keep(&mut self, pos: usize) -> Vec<Do> {
        let (Some(&k), Some(m)) = (self.songs.get(pos), self.measures.get(pos)) else {
            return Vec::new();
        };
        if self.kept[pos] || m.open_ms.is_none() || m.failed.is_some() {
            return Vec::new();
        }
        self.kept[pos] = true;
        let line = m.kept(self.at);
        // The set as Soundcheck has it keeps it too, for a browser that
        // hasn't the set read.
        if let Some(song) = self.set.songs.get_mut(k) {
            song.soundcheck = Some(line.clone());
        }
        vec![Do::Keep(k, line)]
    }

    fn finish(&mut self, now_ms: u64) -> Vec<Do> {
        self.stage = Stage::Done;
        self.ended_ms = Some(now_ms);
        let mut fx = Vec::new();
        if self.muted {
            fx.push(Do::Mute(false));
        }
        fx.push(Do::Reset);
        fx.push(Do::Reopen);
        fx
    }

    /// Songs measured, and kept.
    pub fn kept_count(&self) -> usize {
        self.kept.iter().filter(|k| **k).count()
    }

    /// About how long it has left, in seconds, from `now_ms`.
    pub fn left_s(&self, now_ms: u64) -> u64 {
        if self.stage == Stage::Done {
            return 0;
        }
        let mut ms_left = 0u64;
        for pos in self.pos..self.songs.len() {
            let m = &self.measures[pos];
            let song = self.song_at(pos);
            let part_ms = part_ms(song, &m.part);
            let open = match (pos == self.pos, self.stage) {
                (true, Stage::Opening) => {
                    OPEN_GUESS_MS.saturating_sub(now_ms.saturating_sub(self.open_from))
                }
                (true, _) => 0,
                _ => OPEN_GUESS_MS,
            };
            let played = if pos == self.pos && self.stage == Stage::Playing {
                (m.bars_in / m.part.bars.max(1) as f64).clamp(0.0, 1.0)
            } else if pos == self.pos && self.stage == Stage::Ending {
                1.0
            } else {
                0.0
            };
            ms_left += open + (part_ms as f64 * (1.0 - played)) as u64;
            if pos + 1 < self.songs.len() || self.then.is_some() {
                ms_left += ENDING_GUESS_MS;
            }
        }
        if self.then.is_some() {
            ms_left += OPEN_GUESS_MS;
        }
        ms_left.div_ceil(1000)
    }

    /// What the check view draws now, at `now_ms`: the song it's on, its
    /// three steps, the meter while its part plays and its row's line
    /// (layout spec 4p, Soundcheck running, as drawn).
    pub fn screen(&self, now_ms: u64) -> mpfree_browser_ui::SoundcheckNow {
        use mpfree_browser_ui::{CheckMark, CheckRow};
        let row = |mark, word: &str, text: String| CheckRow {
            mark,
            word: word.into(),
            text,
        };
        // The song after "Soundcheck this song" shows the song checked,
        // its change under way.
        let shown = self.pos.min(self.songs.len().saturating_sub(1));
        let changing = self.pos > shown;
        let k = self.songs.get(shown).copied().unwrap_or(0);
        let name = self
            .set
            .songs
            .get(k)
            .map_or(String::new(), |s| s.name.clone());
        let m = self.measures.get(shown).cloned().unwrap_or_default();
        let next = self
            .songs
            .get(shown + 1)
            .copied()
            .or(self.then)
            .and_then(|j| self.set.songs.get(j).map(|s| (j, s.name.clone())));
        let stage = if changing { Stage::Opening } else { self.stage };
        let playing = stage == Stage::Playing && !changing;
        let size = |kb: u64| mpfree_library::preflight::size_word(kb * 1024);
        let mut steps = Vec::new();
        // 1. Opened cold.
        let opened = m.open_ms.map(|o| {
            let mut t = format!("in {}", seconds(o));
            if m.peak_kb > 0 {
                t.push_str(&format!(" \u{b7} {} at its peak", size(m.peak_kb)));
            }
            t
        });
        steps.push(match (&m.failed, &opened) {
            (Some(why), _) => row(
                CheckMark::NotReady,
                "Opened cold",
                format!("it didn\u{2019}t open: {why}"),
            ),
            (None, Some(t)) => row(CheckMark::Fine, "Opened cold", t.clone()),
            (None, None) => row(
                CheckMark::Now,
                "Opening cold",
                format!(
                    "{} so far",
                    seconds(ms(now_ms.saturating_sub(self.open_from)))
                ),
            ),
        });
        // 2. Its busiest part.
        let bars = m.part.bars.max(1);
        let span = format!("bars {} to {}", m.part.bar, m.part.bar + bars - 1);
        let part = if m.part.name.is_empty() {
            span
        } else {
            format!("{}, {span}", m.part.name)
        };
        steps.push(if m.open_ms.is_none() {
            row(
                CheckMark::Later,
                "Its busiest part",
                "once it\u{2019}s open.".into(),
            )
        } else if playing {
            row(
                CheckMark::Now,
                "Its busiest part",
                format!(
                    "{part}. Bar {}.",
                    m.part.bar + (m.bars_in.floor() as i64).clamp(0, bars - 1)
                ),
            )
        } else {
            let mut found = Vec::new();
            if let Some(c) = m.cpu_worst {
                found.push(format!("CPU {} % at worst", c.round()));
            }
            found.push(match m.wait_ms {
                0 => "no disk wait".to_string(),
                w => format!("longest disk wait {w} ms"),
            });
            found.push(match (m.misses, m.xruns) {
                (0, 0) => "no misses".to_string(),
                (mi, 0) => count(mi, "miss", "misses"),
                (0, x) => count(x, "xrun", "xruns"),
                (mi, x) => format!(
                    "{}, {}",
                    count(mi, "miss", "misses"),
                    count(x, "xrun", "xruns")
                ),
            });
            if m.skipped {
                found.push("skipped".into());
            }
            let mark = if m.misses > 0 || m.xruns > 0 {
                CheckMark::Limits
            } else {
                CheckMark::Fine
            };
            row(
                mark,
                "Its busiest part",
                format!("{part}: {}.", found.join(", ")),
            )
        });
        // 3. The change into the next song (the last song has none).
        if let Some((j, next)) = next {
            let word = format!("The change into song {}", j + 1);
            steps.push(match m.change_ms {
                Some(c) => row(
                    CheckMark::Fine,
                    &word,
                    format!("{next}, in after {}.", seconds(c)),
                ),
                None if changing || stage == Stage::Ending => row(
                    CheckMark::Now,
                    &word,
                    format!("{next}: {name}\u{2019}s ending, then a cold open, timed. Under way."),
                ),
                None => row(
                    CheckMark::Later,
                    &word,
                    format!("{next}: {name}\u{2019}s ending, then a cold open, timed."),
                ),
            });
        }
        let none_or = |n: u32, unit: &str| match n {
            0 => "none".to_string(),
            n => format!("{n}{unit}"),
        };
        let row_line = match &opened {
            None => "opening cold".to_string(),
            Some(_) => {
                let mut l = format!("opened in {}", seconds(m.open_ms.unwrap_or(0)));
                if m.peak_kb > 0 {
                    l.push_str(&format!(" \u{b7} {}", size(m.peak_kb)));
                }
                if playing {
                    l.push_str(&format!(
                        " \u{b7} now playing {}",
                        if m.part.name.is_empty() {
                            "its first bars"
                        } else {
                            m.part.name.as_str()
                        }
                    ));
                } else if stage == Stage::Ending || changing {
                    l.push_str(" \u{b7} ending");
                }
                l
            }
        };
        mpfree_browser_ui::SoundcheckNow {
            loc: self.loc.clone(),
            songs: self.songs.clone(),
            song: k,
            pos: shown,
            left_s: self.left_s(now_ms),
            steps,
            meter: playing
                .then(|| {
                    m.cpu_now
                        .map(|c| (c as f32, m.cpu_worst.unwrap_or(c) as f32))
                })
                .flatten(),
            meter_facts: if playing {
                vec![
                    ("Longest disk wait".into(), none_or(m.wait_ms, " ms")),
                    ("Stream misses".into(), none_or(m.misses, "")),
                    ("xruns".into(), none_or(m.xruns, "")),
                ]
            } else {
                Vec::new()
            },
            row_line,
            out_loud: !self.muted,
        }
    }

    /// How it went, for the check view once it's done (layout spec 4p, as
    /// drawn): how many songs and how long, the busiest, misses and xruns,
    /// and the longest pause between songs, each with its mark.
    pub fn done_rows(&self) -> Vec<mpfree_browser_ui::CheckRow> {
        use mpfree_browser_ui::{CheckMark, CheckRow};
        let row = |mark, text: String| CheckRow {
            mark,
            word: String::new(),
            text,
        };
        let name = |k: usize| {
            self.set
                .songs
                .get(k)
                .map_or(String::new(), |s| s.name.clone())
        };
        let took = self
            .ended_ms
            .unwrap_or(self.started_ms)
            .saturating_sub(self.started_ms)
            / 1000;
        let n = self.kept_count();
        let songs = if n == 1 {
            "1 song".to_string()
        } else {
            format!("{n} songs")
        };
        let mut rows = Vec::new();
        let failed: Vec<String> = self
            .songs
            .iter()
            .zip(&self.measures)
            .filter(|(_, m)| m.failed.is_some())
            .map(|(&k, _)| name(k))
            .collect();
        rows.push(if self.stopped {
            row(
                CheckMark::Note,
                format!(
                    "Stopped after {songs} in {}: what it measured is kept.",
                    duration_words(took)
                ),
            )
        } else if failed.is_empty() {
            row(
                CheckMark::Fine,
                format!("{songs} in {}. All opened.", duration_words(took)),
            )
        } else {
            row(
                CheckMark::NotReady,
                format!(
                    "{songs} in {}. {} didn\u{2019}t open.",
                    duration_words(took),
                    failed.join(", ")
                ),
            )
        });
        let busiest = self
            .songs
            .iter()
            .zip(&self.measures)
            .filter(|(_, m)| m.cpu_worst.is_some())
            .max_by(|a, b| {
                a.1.cpu_worst
                    .partial_cmp(&b.1.cpu_worst)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some((&k, m)) = busiest {
            let mut t = format!(
                "The busiest: {}, CPU {} %",
                name(k),
                m.cpu_worst.unwrap_or(0.0).round()
            );
            if !m.part.name.is_empty() {
                t.push_str(&format!(" in {}", m.part.name));
            }
            t.push('.');
            rows.push(row(CheckMark::Fine, t));
        }
        let glitches: Vec<String> = self
            .songs
            .iter()
            .zip(&self.measures)
            .filter(|(_, m)| m.misses > 0 || m.xruns > 0)
            .map(|(&k, m)| {
                let what = match (m.misses, m.xruns) {
                    (mi, 0) => count(mi, "stream miss", "stream misses"),
                    (0, x) => count(x, "xrun", "xruns"),
                    (mi, x) => format!(
                        "{} and {}",
                        count(mi, "stream miss", "stream misses"),
                        count(x, "xrun", "xruns")
                    ),
                };
                format!("{what} in {}", name(k))
            })
            .collect();
        rows.push(if glitches.is_empty() {
            row(CheckMark::Fine, "No stream misses, no xruns.".into())
        } else {
            row(
                CheckMark::Limits,
                format!("{}: it may crackle there.", glitches.join(", ")),
            )
        });
        let longest = self
            .songs
            .iter()
            .enumerate()
            .filter_map(|(pos, _)| self.measures[pos].change_ms.map(|c| (pos, c)))
            .max_by_key(|(_, c)| *c);
        if let Some((pos, c)) = longest {
            let into = self
                .songs
                .get(pos + 1)
                .copied()
                .or(self.then)
                .map(name)
                .unwrap_or_default();
            rows.push(row(
                CheckMark::Fine,
                format!("Longest pause between songs: {}, into {into}.", seconds(c)),
            ));
        }
        rows
    }

    /// How it went, once it's done: "9 songs in 6 min 10 s · the busiest:
    /// Slow signal, CPU 82 % in Outro · no misses".
    pub fn summary(&self) -> String {
        let took = self
            .ended_ms
            .unwrap_or(self.started_ms)
            .saturating_sub(self.started_ms)
            / 1000;
        let n = self.kept_count();
        let mut words = vec![format!(
            "{} in {}",
            if n == 1 {
                "1 song".to_string()
            } else {
                format!("{n} songs")
            },
            duration_words(took)
        )];
        let busiest = self
            .songs
            .iter()
            .zip(&self.measures)
            .filter(|(_, m)| m.cpu_worst.is_some())
            .max_by(|a, b| {
                a.1.cpu_worst
                    .partial_cmp(&b.1.cpu_worst)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some((&k, m)) = busiest {
            let name = self.set.songs.get(k).map_or("", |s| s.name.as_str());
            let mut b = format!(
                "the busiest: {name}, CPU {} %",
                m.cpu_worst.unwrap_or(0.0).round()
            );
            if !m.part.name.is_empty() {
                b.push_str(&format!(" in {}", m.part.name));
            }
            words.push(b);
        }
        let misses: u32 = self.measures.iter().map(|m| m.misses).sum();
        let xruns: u32 = self.measures.iter().map(|m| m.xruns).sum();
        words.push(match (misses, xruns) {
            (0, 0) => "no misses".to_string(),
            (m, 0) => count(m, "miss", "misses"),
            (0, x) => count(x, "xrun", "xruns"),
            (m, x) => format!(
                "{}, {}",
                count(m, "miss", "misses"),
                count(x, "xrun", "xruns")
            ),
        });
        if self.stopped {
            words.push("stopped".into());
        }
        words.join(" \u{b7} ")
    }
}

/// About how long a song's part plays, in ms, at its tempo as its check
/// read it (else 120 BPM), four beats to a bar.
fn part_ms(song: Option<&SetSong>, part: &Busiest) -> u64 {
    let tempo = song
        .and_then(|s| s.check.as_ref())
        .map(|(_, c)| f64::from(c.tempo_x10) / 10.0)
        .filter(|t| *t > 0.0)
        .unwrap_or(TEMPO_GUESS);
    let bars = if part.bars > 0 { part.bars } else { PART_BARS };
    (bars as f64 * 4.0 * 60_000.0 / tempo) as u64
}

/// "6 min 10 s", "45 s".
pub fn duration_words(s: u64) -> String {
    match (s / 60, s % 60) {
        (0, s) => format!("{s} s"),
        (m, 0) => format!("{m} min"),
        (m, s) => format!("{m} min {s} s"),
    }
}

/// "2.5 s".
fn seconds(ms: u32) -> String {
    format!("{:.1} s", f64::from(ms) / 1000.0)
}

fn count(n: u32, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn ms(d: u64) -> u32 {
    u32::try_from(d).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_library::{Ends, Starts};

    fn song(name: &str, ends: Ends) -> SetSong {
        SetSong {
            name: name.into(),
            ends,
            starts: Starts::Stopped,
            ..SetSong::default()
        }
    }

    fn set3() -> Set {
        Set {
            songs: vec![
                song("Lanterns", Ends::Finish),
                song("Copper sky", Ends::Cut),
                song("Undertow", Ends::Fade(4)),
            ],
            ..Set::default()
        }
    }

    fn loc() -> Loc {
        Loc::top(mpfree_library::Place::Sets)
            .join("Friday.set")
            .unwrap()
    }

    fn part(name: &str, bars: i64) -> Busiest {
        Busiest {
            name: name.into(),
            step: Some(2),
            bar: 9,
            bars,
        }
    }

    fn at(bars_in: f64, cpu: f64) -> Sample {
        Sample {
            bars_in,
            cpu: Some(cpu),
            ..Sample::default()
        }
    }

    fn opens(fx: &[Do]) -> Vec<String> {
        fx.iter()
            .filter_map(|d| match d {
                Do::Open(s) => Some(s.name.clone()),
                _ => None,
            })
            .collect()
    }

    fn kept(fx: &[Do]) -> Vec<(usize, Kept)> {
        fx.iter()
            .filter_map(|d| match d {
                Do::Keep(k, l) => Some((*k, l.clone())),
                _ => None,
            })
            .collect()
    }

    /// The whole set: each song opens cold, its busiest part plays and is
    /// measured, it ends by its own Ends into the next, and each line is
    /// kept once its change is timed; the last song has no change.
    #[test]
    fn a_set_is_soundchecked_song_by_song() {
        let (mut sc, fx) = Soundcheck::new(loc(), set3(), None, false, 1_000, 1_795_460_000);
        assert_eq!(opens(&fx), ["Lanterns"]);
        assert!(!fx.contains(&Do::Mute(true)), "out loud");
        // Lanterns opens in 2.5 s; its part plays from bar 9.
        let fx = sc.opened(3_500, part("Chorus", 8));
        assert_eq!(fx, [Do::Play(Some(2))]);
        assert_eq!(sc.stage(), Stage::Playing);
        assert!(sc
            .tick(
                3_600,
                Sample {
                    memory_kb: Some(400_000),
                    ..at(0.0, 41.0)
                }
            )
            .is_empty());
        assert!(sc.tick(4_000, at(4.0, 63.5)).is_empty());
        let fx = sc.tick(
            9_000,
            Sample {
                xruns: 2,
                ..at(8.0, 55.0)
            },
        );
        // The part is over: Lanterns lets itself finish.
        assert_eq!(fx, [Do::End(End::Finish)]);
        assert!(sc.ended(11_000) == [Do::Open(Box::new(song("Copper sky", Ends::Cut)))]);
        // Copper sky is in: Lanterns's change (Go to in) is timed and kept.
        let fx = sc.opened(14_000, part("Verse", 16));
        let k = kept(&fx);
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].0, 0);
        let l = &k[0].1;
        assert_eq!((l.open_ms, l.peak_kb, l.cpu_x10), (2_500, 400_000, 635));
        assert_eq!((l.xruns, l.misses, l.change_ms), (2, 0, Some(5_000)));
        assert_eq!(l.part, "Chorus");
        assert_eq!(l.at, 1_795_460_000);
        assert!(fx.contains(&Do::Play(Some(2))));
        assert_eq!(sc.measures()[1].open_ms, Some(3_000));
        // Copper sky's part, with a launch waiting 200 ms (to the first
        // frame it no longer waits) and a miss.
        sc.tick(14_100, at(0.0, 30.0));
        sc.tick(
            15_000,
            Sample {
                waiting: true,
                ..at(2.0, 30.0)
            },
        );
        sc.tick(
            15_120,
            Sample {
                waiting: true,
                misses: 1,
                ..at(2.1, 30.0)
            },
        );
        sc.tick(15_200, at(2.2, 30.0));
        assert_eq!(sc.tick(20_000, at(16.0, 30.0)), [Do::End(End::Cut)]);
        sc.ended(20_050);
        let fx = sc.opened(22_050, part("", 16));
        let l = &kept(&fx)[0].1;
        assert_eq!(
            (l.disk_wait_ms, l.misses, l.change_ms),
            (200, 1, Some(2_050))
        );
        // Undertow, the last: no change; Soundcheck is done, and the song
        // open before opens again.
        let fx = sc.tick(30_000, at(16.0, 20.0));
        let k = kept(&fx);
        assert_eq!(k[0].0, 2);
        assert_eq!(k[0].1.change_ms, None);
        assert!(fx.ends_with(&[Do::Stop, Do::Reset, Do::Reopen]), "{fx:?}");
        assert!(sc.done());
        assert_eq!(sc.kept_count(), 3);
        assert_eq!(
            sc.summary(),
            "3 songs in 29 s \u{b7} the busiest: Lanterns, CPU 64 % in Chorus \u{b7} 1 miss, 2 xruns"
        );
        // The check view's lines, as drawn.
        let rows: Vec<(mpfree_browser_ui::CheckMark, String)> = sc
            .done_rows()
            .into_iter()
            .map(|r| (r.mark, r.text))
            .collect();
        use mpfree_browser_ui::CheckMark::{Fine, Limits};
        assert_eq!(
            rows,
            [
                (Fine, "3 songs in 29 s. All opened.".to_string()),
                (
                    Fine,
                    "The busiest: Lanterns, CPU 64 % in Chorus.".to_string()
                ),
                (
                    Limits,
                    "2 xruns in Lanterns, 1 stream miss in Copper sky: it may crackle there."
                        .to_string()
                ),
                (
                    Fine,
                    "Longest pause between songs: 5.0 s, into Copper sky.".to_string()
                ),
            ]
        );
    }

    /// Muted from the start to the end; Stop keeps what was measured, with
    /// no change for the song whose change was under way.
    #[test]
    fn muted_and_stopped_keeps_what_was_measured() {
        let (mut sc, fx) = Soundcheck::new(loc(), set3(), None, true, 0, 7);
        assert_eq!(fx[0], Do::Mute(true));
        sc.opened(2_000, part("Chorus", 4));
        sc.tick(2_100, at(1.0, 50.0));
        assert_eq!(sc.tick(5_000, at(4.0, 50.0)), [Do::End(End::Finish)]);
        let fx = sc.stop(6_000);
        let k = kept(&fx);
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].1.change_ms, None);
        assert!(
            fx.ends_with(&[Do::Stop, Do::Mute(false), Do::Reset, Do::Reopen]),
            "{fx:?}"
        );
        assert!(sc.stopped() && sc.done());
        assert!(sc.stop(7_000).is_empty(), "once");
        assert!(
            sc.summary().ends_with("no misses \u{b7} stopped"),
            "{}",
            sc.summary()
        );
    }

    /// Soundcheck this song: its part, then the change into the song after
    /// it, whose part isn't played.
    #[test]
    fn one_song_times_its_change_into_the_next() {
        let (mut sc, fx) = Soundcheck::new(loc(), set3(), Some(1), false, 0, 7);
        assert_eq!(opens(&fx), ["Copper sky"]);
        sc.opened(1_000, part("Verse", 2));
        assert_eq!(sc.tick(3_000, at(2.0, 10.0)), [Do::End(End::Cut)]);
        assert_eq!(opens(&sc.ended(3_010)), ["Undertow"]);
        assert_eq!(sc.song_now(), Some(2));
        let fx = sc.opened(4_010, part("Intro", 8));
        let k = kept(&fx);
        assert_eq!((k[0].0, k[0].1.change_ms), (1, Some(1_010)));
        assert!(
            !fx.contains(&Do::Play(Some(2))),
            "the next song's part isn't played"
        );
        assert!(sc.done());
        // The set's last song has nothing after it.
        let (mut sc, _) = Soundcheck::new(loc(), set3(), Some(2), false, 0, 7);
        sc.opened(1_000, part("Outro", 1));
        let fx = sc.tick(2_000, at(1.0, 10.0));
        assert_eq!(kept(&fx)[0].1.change_ms, None);
        assert!(sc.done());
    }

    /// A song that doesn't open is passed over (the song before keeps its
    /// line, with no change); Skip ends a part early.
    #[test]
    fn a_song_that_doesnt_open_is_passed_over_and_skip_moves_on() {
        let (mut sc, _) = Soundcheck::new(loc(), set3(), None, false, 0, 7);
        sc.opened(1_000, part("A", 16));
        let fx = sc.skip(2_000);
        assert_eq!(fx, [Do::End(End::Finish)]);
        assert!(sc.measures()[0].skipped);
        sc.ended(2_500);
        let fx = sc.open_failed(3_000, "a file it needs is missing");
        assert!(matches!(&fx[0], Do::Tell(t) if t.contains("Copper sky didn\u{2019}t open")));
        assert_eq!(kept(&fx)[0].1.change_ms, None);
        assert_eq!(opens(&fx), ["Undertow"]);
        sc.opened(5_000, part("B", 1));
        sc.tick(6_000, at(1.0, 5.0));
        assert!(sc.done());
        assert_eq!(
            sc.kept_count(),
            2,
            "the song that didn't open keeps nothing"
        );
        assert!(matches!(&sc.skip(7_000)[0], Do::Tell(_)));
    }

    /// The busiest part: the step whose scene has the most clips (audio
    /// and plugin tracks count twice), the first of a tie, at most 16
    /// bars; with no song list, the first 16 bars.
    #[test]
    fn the_busiest_part_is_the_step_with_the_most_playing() {
        let track = |clips: &[bool], heavy: bool| TrackLoad {
            clips: clips.to_vec(),
            heavy,
        };
        // A kit, keys, an audio track and a plugin's track.
        let tracks = vec![
            track(&[true, true, true], false),
            track(&[false, true, true], false),
            track(&[false, false, true], true),
            track(&[false, true, false], true),
        ];
        let row = |name: &str, scene: usize, starts: i64, bars: i64| ListRow {
            name: name.into(),
            scene: Some(scene),
            starts,
            bars,
            ..ListRow::default()
        };
        let list = vec![
            row("Intro", 0, 1, 4),
            row("Verse", 1, 5, 8),
            row("Chorus", 2, 13, 32),
            row("Verse 2", 1, 45, 8),
        ];
        // Verse: kit, keys, the plugin's track (2) = 4; Chorus: kit, keys,
        // the audio clip (2) = 4: Verse comes first.
        assert_eq!(
            busiest(&list, &tracks),
            Busiest {
                name: "Verse".into(),
                step: Some(1),
                bar: 5,
                bars: 8
            }
        );
        let mut more = tracks.clone();
        more[0].clips.push(false);
        more.push(track(&[false, false, true], false));
        assert_eq!(
            busiest(&list, &more),
            Busiest {
                name: "Chorus".into(),
                step: Some(2),
                bar: 13,
                bars: 16
            }
        );
        assert_eq!(
            busiest(&[], &tracks),
            Busiest {
                name: String::new(),
                step: None,
                bar: 1,
                bars: 16
            }
        );
    }

    /// The check view's steps follow it: opening, the part with its bar,
    /// then what each found, and the change under way.
    #[test]
    fn the_check_view_shows_its_three_steps() {
        use mpfree_browser_ui::CheckMark;
        let (mut sc, _) = Soundcheck::new(loc(), set3(), None, true, 0, 7);
        let n = sc.screen(1_200);
        assert_eq!((n.song, n.pos, n.out_loud), (0, 0, false));
        let marks: Vec<CheckMark> = n.steps.iter().map(|r| r.mark).collect();
        assert_eq!(marks, [CheckMark::Now, CheckMark::Later, CheckMark::Later]);
        assert_eq!(n.steps[0].text, "1.2 s so far");
        assert_eq!(n.steps[0].word, "Opening cold");
        assert_eq!(n.row_line, "opening cold");
        assert_eq!(n.steps[2].word, "The change into song 2");
        assert_eq!(
            n.steps[2].text,
            "Copper sky: Lanterns\u{2019}s ending, then a cold open, timed."
        );
        sc.opened(2_500, part("Chorus", 8));
        sc.tick(
            2_600,
            Sample {
                memory_kb: Some(527_344),
                ..at(0.0, 41.0)
            },
        );
        sc.tick(3_000, at(2.5, 63.0));
        let n = sc.screen(3_000);
        assert_eq!(n.steps[0].text, "in 2.5 s \u{b7} 540 MB at its peak");
        assert_eq!(
            n.row_line,
            "opened in 2.5 s \u{b7} 540 MB \u{b7} now playing Chorus"
        );
        assert_eq!(n.steps[1].mark, CheckMark::Now);
        assert_eq!(n.steps[1].text, "Chorus, bars 9 to 16. Bar 11.");
        assert_eq!(
            n.meter_facts[0],
            ("Longest disk wait".to_string(), "none".to_string())
        );
        assert_eq!(n.meter, Some((63.0, 63.0)));
        sc.tick(
            6_000,
            Sample {
                xruns: 1,
                ..at(8.0, 50.0)
            },
        );
        let n = sc.screen(6_000);
        assert_eq!(n.steps[1].mark, CheckMark::Limits);
        assert_eq!(
            n.steps[1].text,
            "Chorus, bars 9 to 16: CPU 63 % at worst, no disk wait, 1 xrun."
        );
        assert_eq!(n.steps[2].mark, CheckMark::Now);
        assert_eq!(n.meter, None);
        // Copper sky opening: its own steps.
        sc.ended(7_000);
        let n = sc.screen(7_500);
        assert_eq!((n.song, n.pos), (1, 1));
        assert_eq!(n.steps[0].mark, CheckMark::Now);
        assert_eq!(n.steps[2].word, "The change into song 3");
        // "Soundcheck this song", the last of the set: no change.
        let (sc, _) = Soundcheck::new(loc(), set3(), Some(2), false, 0, 7);
        let n = sc.screen(0);
        assert_eq!(n.steps.len(), 2);
        assert!(n.out_loud);
    }

    /// The time left and the words for it.
    #[test]
    fn about_how_long_it_takes() {
        // 9 songs at 120 BPM: 16 bars is 32 s, an open 5 s, 8 endings 5 s.
        let set = Set {
            songs: (0..9)
                .map(|i| song(&format!("S{i}"), Ends::Finish))
                .collect(),
            ..Set::default()
        };
        let s = 9 * 37 + 8 * 5;
        assert_eq!(duration_words(370), "6 min 10 s");
        assert_eq!(duration_words(45), "45 s");
        assert_eq!(duration_words(120), "2 min");
        let (sc, _) = Soundcheck::new(loc(), set.clone(), None, false, 0, 7);
        assert_eq!(sc.left_s(0), s);
        assert_eq!(sc.left_s(2_000), s - 2);
    }
}
