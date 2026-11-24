//! The Sets place's logic (live shows B1; layout spec 4p): which set is
//! picked, the set being edited and its picked song, the projects to add,
//! the rig the app gave, and what a touch, the dial or a button does.
//! Every read and write goes to the library's worker (`Job::Sets`,
//! `Job::Songs`, `Job::SaveSet`, `Job::CheckSet`, `Job::NewSet`); the set
//! saves itself after every change. [`crate::sets`] draws it.

use super::*;
use crate::sets::{
    CheckMark, CheckRow, NeedRow, RigThing, SetButton, SetField, SetInput, SetRow, SetsScreen,
    SetsView, SoundcheckNow, Stepper, Tone,
};
use mpfree_library::preflight::{AppCheck, AppChecks, Machine, Mark, SongPreflight};
use mpfree_library::sets::{FADE_DEFAULT, FADE_MAX, FADE_MIN, MAX_NOTES};
use mpfree_library::{Ends, Need, SetSong, SongCheck, Starts, SET_EXT};
use std::sync::Arc;

/// The longest name a set takes (layout spec 1).
pub const SET_NAME_MAX: usize = 32;

/// The Sets place's state, kept by the browser.
#[derive(Debug, Default)]
pub(super) struct SetsPlace {
    /// Every set, as last read: its entry and its songs, or why it
    /// couldn't be read.
    sets: Vec<(Entry, Result<Set, String>)>,
    read: bool,
    selected: Option<usize>,
    top: usize,
    editing: Option<Editing>,
    /// The projects a set can hold (folder, lasting id), once read.
    songs: Option<Vec<(String, String)>>,
    rig: Vec<RigThing>,
    /// The set being checked.
    checking: Option<Loc>,
    /// New set is waiting for the library: it opens once made.
    new_set: bool,
    /// The rows the list has room for: sets or projects, and songs.
    rows: (usize, usize),
    /// The set the Show view plays (live shows B1), kept in step with the
    /// place's: a check or a song's changes on stage land on it even when
    /// the place hasn't listed the sets.
    playing: Option<(Loc, Set)>,
    /// The check view (live shows B2), while it shows.
    viewing: Option<CheckView>,
    /// What the app knows of this MPC, for the check.
    machine: Machine,
    /// The app's half of the check.
    app: Option<AppCheck>,
    /// A fixed time for checks that come back (the sample screens, so they
    /// are the same every run); None: the clock.
    clock: Option<u64>,
    /// Soundcheck's question (live shows B2b), in the check view's Selected
    /// column until it starts or is cancelled.
    asking: Option<SoundAsk>,
    /// Soundcheck running, as the app last told it.
    running: Option<SoundcheckNow>,
    /// How the last Soundcheck went (the set, its lines and the words
    /// after them), until Done.
    sound_done: Option<(Loc, Vec<CheckRow>, String)>,
    /// Sound as last picked (Muted), for the next question.
    muted: bool,
}

/// Soundcheck's question: the set, every song or one, and the Sound.
#[derive(Debug, Clone)]
struct SoundAsk {
    loc: Loc,
    only: Option<usize>,
    muted: bool,
}

/// The check view: the set it checks, each song's last preflight, and the
/// song picked.
#[derive(Debug)]
struct CheckView {
    loc: Loc,
    /// Each song's check, once it comes back.
    songs: Option<Vec<SongPreflight>>,
    picked: Option<usize>,
    top: usize,
    /// The last check read every file in full.
    deep: bool,
    /// When it came back, in seconds since 1970.
    at: u64,
}

/// The set being edited.
#[derive(Debug)]
struct Editing {
    loc: Loc,
    set: Set,
    picked: Option<usize>,
    top: usize,
    /// Adding songs: the project picked and the first one shown.
    adding: Option<(Option<usize>, usize)>,
    /// The value the dial turns, if any.
    focus: Option<SetField>,
    /// The Needs list shows in the Selected column.
    needs: bool,
    /// Remove from set was tapped once: a second tap removes.
    armed: bool,
}

/// A set's name without `.set`.
fn stem(name: &str) -> &str {
    name.strip_suffix(SET_EXT).unwrap_or(name)
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

/// A check's chip and its colour.
fn chip(check: Option<&SongCheck>) -> (String, Tone) {
    match check {
        None => ("NOT CHECKED".into(), Tone::Dim),
        Some(c) => {
            let tone = match c.word() {
                "READY" => Tone::Good,
                "LIMITS" => Tone::Warn,
                "NOT READY" => Tone::Bad,
                _ => Tone::Dim,
            };
            (c.word().into(), tone)
        }
    }
}

/// A chip's word and its colour.
fn chip_of(word: &str) -> (String, Tone) {
    let tone = match word {
        "READY" => Tone::Good,
        "LIMITS" => Tone::Warn,
        "NOT READY" => Tone::Bad,
        _ => Tone::Dim,
    };
    (word.into(), tone)
}

/// How a set's songs did at their last check: (ready, with limits, not
/// ready, not checked, the latest check's time).
fn tally(set: &Set) -> (usize, usize, usize, usize, u64) {
    let (mut ready, mut limits, mut not, mut unchecked, mut at) = (0, 0, 0, 0, 0);
    for s in &set.songs {
        match &s.check {
            None => unchecked += 1,
            Some((t, c)) => {
                at = at.max(*t);
                match c.word() {
                    "NOT READY" => not += 1,
                    "LIMITS" => limits += 1,
                    "NOT CHECKED" => unchecked += 1,
                    _ => ready += 1,
                }
            }
        }
    }
    (ready, limits, not, unchecked, at)
}

/// A set's chip in the Sets place: its worst song.
fn set_chip(set: &Set) -> Option<(String, Tone)> {
    if set.songs.is_empty() {
        return None;
    }
    let (_, limits, not, unchecked, _) = tally(set);
    Some(if not > 0 {
        ("NOT READY".into(), Tone::Bad)
    } else if unchecked > 0 {
        ("NOT CHECKED".into(), Tone::Dim)
    } else if limits > 0 {
        ("LIMITS".into(), Tone::Warn)
    } else {
        ("READY".into(), Tone::Good)
    })
}

/// "10 ready, 1 with limits, 1 not ready".
fn tally_words(set: &Set) -> String {
    let (ready, limits, not, unchecked, _) = tally(set);
    let mut parts = Vec::new();
    if ready > 0 {
        parts.push(format!("{ready} ready"));
    }
    if limits > 0 {
        parts.push(format!("{limits} with limits"));
    }
    if not > 0 {
        parts.push(format!("{not} not ready"));
    }
    if unchecked > 0 {
        parts.push(format!("{unchecked} not checked"));
    }
    parts.join(", ")
}

/// About how long Soundcheck takes ("about 6 minutes"): each song's open
/// (5 s until measured), its busiest part (16 bars at its tempo, else 120
/// BPM) and each change's ending (5 s), as the app's soundcheck.rs counts.
fn soundcheck_about(set: &Set, only: Option<usize>) -> String {
    let part_ms = |s: Option<&SetSong>| -> u64 {
        let tempo = s
            .and_then(|s| s.check.as_ref())
            .map(|(_, c)| f64::from(c.tempo_x10) / 10.0)
            .filter(|t| *t > 0.0)
            .unwrap_or(120.0);
        (16.0 * 4.0 * 60_000.0 / tempo) as u64
    };
    let n = set.songs.len();
    let ms: u64 = match only {
        Some(k) => 5_000 + part_ms(set.songs.get(k)) + if k + 1 < n { 10_000 } else { 0 },
        None => {
            (0..n)
                .map(|k| 5_000 + part_ms(set.songs.get(k)))
                .sum::<u64>()
                + 5_000 * n.saturating_sub(1) as u64
        }
    };
    match (ms.div_ceil(1000) + 30) / 60 {
        0 => "under a minute".into(),
        1 => "about 1 minute".into(),
        m => format!("about {m} minutes"),
    }
}

fn song_sub(s: &SetSong) -> String {
    format!("{} \u{b7} {}", s.starts.label(), s.ends.label())
}

/// The song's Needs in a few words.
fn needs_words(s: &SetSong) -> String {
    match s.needs.as_slice() {
        [] => "\u{2014}".into(),
        [one] => one.name.clone(),
        [one, rest @ ..] => format!("{} and {} more", one.name, rest.len()),
    }
}

/// The next value of a list, `d` steps on, going round.
fn turn<T: Copy + PartialEq>(all: &[T], now: T, d: i32) -> T {
    let i = all.iter().position(|x| *x == now).unwrap_or(0) as i32;
    all[(i + d).rem_euclid(all.len() as i32) as usize]
}

/// Keeps `sel` in view among `n` rows, `vis` at a time, from `top`.
fn in_view(sel: Option<usize>, top: usize, vis: usize, n: usize) -> usize {
    let vis = vis.max(1);
    let mut top = top.min(n.saturating_sub(vis));
    if let Some(s) = sel {
        if s < top {
            top = s;
        } else if s >= top + vis {
            top = s + 1 - vis;
        }
    }
    top
}

/// What the browser's own keys do in the place.
pub(super) enum SetKey {
    Dial(i32),
    Push,
    Back,
    Enter,
    Erase,
}

impl Browser {
    /// The Sets place is the one shown (and no picker is open).
    pub fn sets_shown(&self) -> bool {
        self.pick.is_none()
            && self.current() == Source::Place(Place::Sets)
            && self.mode == Mode::Folder
    }

    /// A set is being edited (the hint line says what – / + do there).
    pub fn set_editing(&self) -> bool {
        self.sets_shown() && self.sets.editing.is_some()
    }

    /// Songs are being added to the set being edited.
    pub fn set_adding(&self) -> bool {
        self.sets_shown()
            && self
                .sets
                .editing
                .as_ref()
                .is_some_and(|e| e.adding.is_some())
    }

    /// The rig's keyboards and learned controls, and which are connected
    /// now, from the app: what a song's Needs offers and the check asks
    /// for.
    pub fn set_rig(&mut self, rig: Vec<RigThing>) {
        self.sets.rig = rig;
    }

    /// What the app knows of this MPC now (memory free for songs, the
    /// stick's room), for the next check.
    pub fn set_machine(&mut self, machine: Machine) {
        self.sets.machine = machine;
    }

    /// The app's half of the set check: the lines only it can see in a
    /// song (plugins, outputs), run on the library's worker.
    pub fn set_app_checks(&mut self, app: Arc<dyn AppChecks>) {
        self.sets.app = Some(AppCheck(app));
    }

    /// Checks that come back are stamped `secs` (seconds since 1970)
    /// instead of the clock's time: for sample screens that must draw the
    /// same every run.
    pub fn fix_set_clock(&mut self, secs: u64) {
        self.sets.clock = Some(secs);
    }

    /// Soundcheck running, as the app has it now (None: it isn't), for
    /// the check view to draw (live shows B2b).
    pub fn set_soundcheck(&mut self, now: Option<SoundcheckNow>) {
        if let Some(n) = &now {
            // The check view follows the song it's on.
            let vis = self.song_rows();
            if let Some(v) = self.sets.viewing.as_mut().filter(|v| v.loc == n.loc) {
                let len = v.songs.as_ref().map_or(0, Vec::len);
                if v.picked != Some(n.song) && n.song < len {
                    v.picked = Some(n.song);
                    v.top = in_view(v.picked, v.top, vis, len);
                }
            }
            self.sets.asking = None;
            self.sets.sound_done = None;
        }
        self.sets.running = now;
    }

    /// Soundcheck is running.
    pub fn soundcheck_running(&self) -> bool {
        self.sets.running.is_some()
    }

    /// Soundcheck measured song `k` of the set at `loc`: its line goes into
    /// the set, which is saved. `theirs` is the set as Soundcheck has it,
    /// for when the browser hasn't read it (BROWSE never showed Sets).
    pub fn soundchecked(
        &mut self,
        loc: &Loc,
        k: usize,
        line: mpfree_library::Soundcheck,
        theirs: &Set,
    ) {
        let mut set = self.set_at(loc).unwrap_or_else(|| theirs.clone());
        let Some(song) = set.songs.get_mut(k) else {
            return;
        };
        song.soundcheck = Some(line);
        self.sets_keep(loc, set);
    }

    /// Soundcheck on the set at `loc` is over: `rows` say how it went and
    /// `words` what happens now, until Done, and the set is checked again
    /// with what it measured.
    pub fn soundcheck_over(&mut self, loc: &Loc, rows: Vec<CheckRow>, words: String) {
        self.sets.running = None;
        self.sets.sound_done = Some((loc.clone(), rows, words));
        if let Some(set) = self.set_at(loc) {
            if self.sets.viewing.as_ref().is_some_and(|v| v.loc == *loc) {
                let deep = self.sets.viewing.as_ref().is_some_and(|v| v.deep);
                self.send_check(loc.clone(), set, deep);
            } else {
                self.check_set_now(loc);
            }
        }
    }

    /// The question before Soundcheck: on the set at `loc`, every song or
    /// only `only`. It opens the check view on the set.
    fn ask_soundcheck(&mut self, loc: Loc, set: Set, only: Option<usize>) {
        if self.sets.playing.is_some() {
            self.status = Some(("Close the set or stop it first".into(), false));
            return;
        }
        if self.sets.running.is_some() {
            self.status = Some(("Soundcheck is running: Stop soundcheck first".into(), false));
            return;
        }
        if set.songs.is_empty() {
            self.status = Some((
                "No songs to soundcheck yet: Add songs picks them".into(),
                false,
            ));
            return;
        }
        if self.sets.viewing.as_ref().is_none_or(|v| v.loc != loc) {
            self.check_view(loc.clone(), set, false);
        }
        self.sets.sound_done = None;
        let muted = self.sets.muted;
        self.sets.asking = Some(SoundAsk { loc, only, muted });
    }

    /// The song open now has changes that Soundcheck saves first, or it
    /// can't be saved (it was never saved): the line that says so, and
    /// whether Start can go.
    fn soundcheck_open_song(&self) -> (Option<(String, Tone)>, bool) {
        if !self.unsaved {
            return (None, true);
        }
        match &self.project {
            Some(p) => (
                Some((
                    format!(
                        "{}, open now, has changes: they\u{2019}re saved first. It opens again at the end.",
                        p.name()
                    ),
                    Tone::Warn,
                )),
                true,
            ),
            None => (
                Some((
                    "The song open now was never saved: save it first, or it would be lost".into(),
                    Tone::Bad,
                )),
                false,
            ),
        }
    }

    /// The check view shows (the hint line says what the keys do there).
    pub fn set_check_shown(&self) -> bool {
        self.sets_shown() && self.sets.viewing.is_some()
    }

    /// The rows the Sets place's list has room for (sets or projects, and
    /// a set's songs).
    pub fn set_set_rows(&mut self, list: usize, songs: usize) {
        if list > 0 && songs > 0 {
            self.sets.rows = (list, songs);
        }
    }

    /// Until the widget says, the rows of the MPC's screen.
    fn list_rows(&self) -> usize {
        match self.sets.rows.0 {
            0 => 10,
            n => n,
        }
    }

    fn song_rows(&self) -> usize {
        match self.sets.rows.1 {
            0 => 9,
            n => n,
        }
    }

    /// The set the Show view plays now, or none (live shows B1). Its
    /// changes come back as [`BrowseAction::SetChanged`].
    pub fn set_playing(&mut self, playing: Option<(Loc, Set)>) {
        self.sets.playing = playing;
    }

    /// The set at `loc` as the place has it now: the editor's, the list's,
    /// or the one playing.
    fn set_at(&self, loc: &Loc) -> Option<Set> {
        match &self.sets.editing {
            Some(e) if e.loc == *loc => return Some(e.set.clone()),
            _ => {}
        }
        if let Some((_, Ok(s))) = self.sets.sets.iter().find(|(e, _)| e.name == loc.name()) {
            return Some(s.clone());
        }
        self.sets
            .playing
            .as_ref()
            .filter(|(l, _)| l == loc)
            .map(|(_, s)| s.clone())
    }

    /// Check the set at `loc` now (the Show view's MENU, and Play set),
    /// as Check set does. It comes back as [`BrowseAction::SetChanged`].
    pub fn check_set_now(&mut self, loc: &Loc) {
        let Some(set) = self.set_at(loc) else {
            return;
        };
        if set.songs.is_empty() || self.sets.checking.is_some() {
            return;
        }
        self.send_check(loc.clone(), set, false);
    }

    /// The check view on the set at `loc`, checked now (the Show view's
    /// MENU: Check the set): the Sets place, showing it.
    pub fn open_check(&mut self, loc: &Loc) {
        let Some(at) = SOURCES
            .iter()
            .position(|s| *s == Source::Place(Place::Sets))
        else {
            return;
        };
        self.go_source(at);
        let Some(set) = self.set_at(loc) else {
            return;
        };
        self.select_name = Some(loc.name().to_string());
        self.check_view(loc.clone(), set, false);
    }

    /// Check `set` (at `loc`) on the library's worker, reading every file
    /// in full when `deep`.
    fn send_check(&mut self, loc: Loc, set: Set, deep: bool) {
        let connected = self
            .sets
            .rig
            .iter()
            .filter(|r| r.connected)
            .map(|r| r.name.clone())
            .collect();
        self.sets.checking = Some(loc.clone());
        self.send(Job::CheckSet {
            loc,
            set,
            connected,
            deep,
            machine: self.sets.machine,
            app: self.sets.app.clone(),
        });
    }

    /// Open the check view on `set` and check it.
    fn check_view(&mut self, loc: Loc, set: Set, deep: bool) {
        if set.songs.is_empty() {
            self.status = Some(("No songs to check yet: Add songs picks them".into(), false));
            return;
        }
        let keep =
            self.sets.viewing.take().filter(|v| {
                v.loc == loc && v.songs.as_ref().map(Vec::len) == Some(set.songs.len())
            });
        self.status = Some((
            format!(
                "{} {}\u{2026}",
                if deep {
                    "Reading every file of"
                } else {
                    "Checking"
                },
                stem(loc.name())
            ),
            false,
        ));
        let mut view = keep.unwrap_or(CheckView {
            loc: loc.clone(),
            songs: None,
            picked: None,
            top: 0,
            deep,
            at: 0,
        });
        view.deep = deep;
        self.sets.viewing = Some(view);
        // A quick check of this set already on its way lands here too.
        if deep || self.sets.checking.as_ref() != Some(&loc) {
            self.send_check(loc, set, deep);
        }
    }

    /// The Set editor on the set at `loc` (the Show view's MENU, live
    /// shows B1): the Sets place, its editor open on that set.
    pub fn edit_set(&mut self, loc: &Loc) {
        let Some(at) = SOURCES
            .iter()
            .position(|s| *s == Source::Place(Place::Sets))
        else {
            return;
        };
        self.go_source(at);
        let Some(set) = self.set_at(loc) else {
            return;
        };
        self.select_name = Some(loc.name().to_string());
        let picked = (!set.songs.is_empty()).then_some(0);
        self.sets.editing = Some(Editing {
            loc: loc.clone(),
            set,
            picked,
            top: 0,
            adding: None,
            focus: None,
            needs: false,
            armed: false,
        });
    }

    /// Song `song` of the set at `loc` changed on stage: its recovery copy
    /// is `copy` (a project in Projects). The set remembers it, so the Set
    /// editor offers Keep the changes and Throw them away after the show.
    pub fn mark_on_stage(&mut self, loc: &Loc, song: usize, copy: &str) {
        let Some(mut set) = self.set_at(loc) else {
            return;
        };
        let Some(s) = set.songs.get_mut(song) else {
            return;
        };
        if s.on_stage.len() >= mpfree_library::sets::MAX_ON_STAGE {
            s.on_stage.remove(0);
        }
        s.on_stage.push(copy.to_string());
        self.sets_keep(loc, set);
    }

    /// Reads every set again (and the projects, while adding).
    pub(super) fn sets_refresh(&mut self) {
        self.send_in(Lane::List, Job::Sets);
        if self
            .sets
            .editing
            .as_ref()
            .is_some_and(|e| e.adding.is_some())
        {
            self.send(Job::Songs);
        }
    }

    /// The place was left or shown again: back to the list of sets.
    pub(super) fn sets_reset(&mut self) {
        self.sets.editing = None;
        self.sets.viewing = None;
        self.sets.read = false;
    }

    // ------------------------------------------------------------ answers

    pub(super) fn sets_listed(&mut self, list: Vec<(Entry, Result<Set, String>)>) {
        let keep = self.select_name.take().or_else(|| {
            self.sets
                .selected
                .and_then(|i| self.sets.sets.get(i))
                .map(|(e, _)| e.name.clone())
        });
        self.sets.sets = list;
        self.sets.read = true;
        let n = self.sets.sets.len();
        self.sets.selected = keep
            .and_then(|k| self.sets.sets.iter().position(|(e, _)| e.name == k))
            .or_else(|| self.sets.selected.map(|s| s.min(n.saturating_sub(1))))
            .or((n > 0).then_some(0))
            .filter(|&s| s < n);
        self.sets.top = in_view(self.sets.selected, self.sets.top, self.list_rows(), n);
        // The set being edited keeps what the editor has (it saved it).
    }

    pub(super) fn sets_songs(&mut self, songs: Vec<(String, String)>) {
        let n = songs.len();
        self.sets.songs = Some(songs);
        let rows = self.list_rows();
        if let Some((sel, top)) = self.sets.editing.as_mut().and_then(|e| e.adding.as_mut()) {
            *sel = sel.or((n > 0).then_some(0)).filter(|&s| s < n);
            *top = in_view(*sel, *top, rows, n);
        }
    }

    /// The check of a set came back: each song keeps it (and a song that
    /// was renamed or moved takes its new name), and the set is saved.
    pub(super) fn sets_checked(&mut self, loc: Loc, checks: Vec<SongPreflight>) {
        if self.sets.checking.as_ref() == Some(&loc) {
            self.sets.checking = None;
        }
        let now = self.sets.clock.unwrap_or_else(now_utc);
        if let Some(v) = self.sets.viewing.as_mut().filter(|v| v.loc == loc) {
            // The first song not ready, else the first with limits, is
            // picked; a song picked before stays.
            let worst = |m: Mark| {
                checks
                    .iter()
                    .position(|p| p.lines.iter().any(|l| l.mark == m))
            };
            if v.picked.is_none_or(|k| k >= checks.len()) {
                v.picked = worst(Mark::NotReady)
                    .or_else(|| worst(Mark::Limits))
                    .or((!checks.is_empty()).then_some(0));
            }
            v.at = now;
            v.songs = Some(checks.clone());
        }
        let checks: Vec<(Found, SongCheck)> =
            checks.into_iter().map(|p| (p.found, p.check)).collect();
        let Some(mut set) = self.set_at(&loc) else {
            return;
        };
        if set.songs.len() != checks.len() {
            // The set changed while it was checked: the check is stale.
            self.status = Some((
                "The set changed while it was checked: Check set again".into(),
                false,
            ));
            return;
        }
        for (song, (found, check)) in set.songs.iter_mut().zip(checks) {
            if let Found::Moved(l) = &found {
                song.name = l.name().to_string();
            }
            song.check = Some((now, check));
        }
        let words = tally_words(&set);
        self.status = Some((format!("Checked {}: {words}", stem(loc.name())), false));
        self.sets_keep(&loc, set);
    }

    /// A recovery copy is in its song (Keep the changes): no set lists it
    /// any more.
    pub(super) fn sets_kept(&mut self, copy: Loc, song: Loc) {
        self.status = Some((
            format!(
                "Kept the changes from the show in {}: its Earlier saves have the version from before",
                song.name()
            ),
            false,
        ));
        let mut changed = Vec::new();
        let editing = self
            .sets
            .editing
            .as_ref()
            .map(|e| (e.loc.clone(), e.set.clone()));
        let playing = self.sets.playing.clone();
        let listed = self.sets.sets.iter().filter_map(|(e, s)| {
            let s = s.as_ref().ok()?;
            Some((Loc::top(Place::Sets).join(&e.name).ok()?, s.clone()))
        });
        for (loc, mut set) in editing
            .into_iter()
            .chain(playing)
            .chain(listed.collect::<Vec<_>>())
        {
            if changed.iter().any(|(l, _)| *l == loc) {
                continue;
            }
            let mut any = false;
            for s in &mut set.songs {
                let before = s.on_stage.len();
                s.on_stage.retain(|c| c != copy.name());
                any |= s.on_stage.len() != before;
            }
            if any {
                changed.push((loc, set));
            }
        }
        for (loc, set) in changed {
            self.sets_keep(&loc, set);
        }
        self.forget(&copy);
    }

    /// A set made by New set: it opens in the editor.
    pub(super) fn sets_made(&mut self, loc: Loc) {
        self.sets.new_set = false;
        self.select_name = Some(loc.name().to_string());
        self.status = Some((
            format!(
                "Made {}: Add songs picks them from Projects",
                stem(loc.name())
            ),
            false,
        ));
        self.sets.editing = Some(Editing {
            loc,
            set: Set::default(),
            picked: None,
            top: 0,
            adding: None,
            focus: None,
            needs: false,
            armed: false,
        });
        self.sets_refresh();
    }

    /// New set is waiting for the library.
    pub(super) fn sets_waits_new(&self) -> bool {
        self.sets.new_set
    }

    /// A job failed: nothing waits on it any more.
    pub(super) fn sets_failed(&mut self) {
        self.sets.new_set = false;
        self.sets.checking = None;
    }

    // ------------------------------------------------------------ saving

    /// `set` is now the set at `loc`: in the editor, in the list, in the
    /// Show view, and saved.
    fn sets_keep(&mut self, loc: &Loc, set: Set) {
        if let Some(e) = self.sets.editing.as_mut().filter(|e| e.loc == *loc) {
            e.set = set.clone();
        }
        if let Some((_, s)) = self.sets.playing.as_mut().filter(|(l, _)| l == loc) {
            *s = set.clone();
            self.actions.push(BrowseAction::SetChanged {
                loc: loc.clone(),
                set: set.clone(),
            });
        }
        if let Some((_, s)) = self
            .sets
            .sets
            .iter_mut()
            .find(|(e, _)| e.name == loc.name())
        {
            *s = Ok(set.clone());
        }
        self.send(Job::SaveSet {
            loc: loc.clone(),
            set,
        });
    }

    /// The set being edited changed: keep it.
    fn sets_save(&mut self) {
        if let Some(e) = &self.sets.editing {
            let (loc, set) = (e.loc.clone(), e.set.clone());
            self.sets_keep(&loc, set);
        }
    }

    // ------------------------------------------------------------ input

    /// The browser's keys, while the place shows. True if it took them.
    pub(super) fn send_sets(&mut self, k: SetKey) -> bool {
        if !self.sets_shown() || !matches!(self.modal, Modal::None) || self.export_shown().is_some()
        {
            return false;
        }
        if self.sets.viewing.is_some() {
            // The check view: the dial picks a song (its checks show), -
            // goes back to the set.
            match k {
                SetKey::Dial(d) => self.sets_pick(d),
                SetKey::Back => self.sets_input(SetInput::Back),
                SetKey::Push | SetKey::Enter | SetKey::Erase => {}
            }
            return true;
        }
        let editing = self.sets.editing.as_ref();
        let adding = editing.is_some_and(|e| e.adding.is_some());
        let focus = editing.and_then(|e| e.focus);
        let needs = editing.is_some_and(|e| e.needs);
        match (k, editing.is_some(), adding) {
            // The list of sets.
            (SetKey::Dial(d), false, _) => self.sets_pick(d),
            (SetKey::Push | SetKey::Enter, false, _) => self.sets_button(SetButton::EditSet),
            (SetKey::Back, false, _) => {}
            (SetKey::Erase, false, _) => self.sets_button(SetButton::Delete),
            // Adding songs.
            (SetKey::Dial(d), true, true) => self.sets_pick(d),
            (SetKey::Push | SetKey::Enter, true, true) => {
                if let Some(i) = self.sets.editing.as_ref().and_then(|e| e.adding?.0) {
                    self.sets_add(i);
                }
            }
            (SetKey::Back, true, true) => self.sets_button(SetButton::Done),
            (SetKey::Erase, true, true) => {}
            // A set's songs.
            (SetKey::Dial(d), true, false) => match focus {
                Some(f) if !needs => self.sets_step(f, d),
                _ => self.sets_pick(d),
            },
            (SetKey::Push, true, false) => {
                if !needs {
                    self.sets_focus_next();
                }
            }
            (SetKey::Back, true, false) => self.sets_button(SetButton::MoveUp),
            (SetKey::Enter, true, false) => self.sets_button(SetButton::MoveDown),
            (SetKey::Erase, true, false) => self.sets_button(SetButton::Remove),
        }
        true
    }

    pub(super) fn sets_input(&mut self, i: SetInput) {
        if !self.sets_shown() {
            return;
        }
        if !matches!(
            i,
            SetInput::Button(SetButton::Remove | SetButton::ConfirmRemove)
        ) {
            if let Some(e) = self.sets.editing.as_mut() {
                e.armed = false;
            }
        }
        match i {
            SetInput::Row(r) => self.sets_row(r),
            SetInput::Scroll(d) => self.sets_scroll(d),
            SetInput::Button(b) => self.sets_button(b),
            SetInput::Step(f, d) => {
                if let Some(e) = self.sets.editing.as_mut() {
                    if f != SetField::Needs {
                        e.focus = Some(f);
                    }
                }
                self.sets_step(f, d);
            }
            SetInput::Notes => self.sets_notes(),
            SetInput::Need(k) => self.sets_need(k),
            SetInput::Add(i) => {
                if let Some((sel, _)) = self.sets.editing.as_mut().and_then(|e| e.adding.as_mut()) {
                    *sel = Some(i);
                }
                if self.sets.checking.is_none() {
                    self.sets_add(i);
                }
            }
            SetInput::Back if self.sets.asking.is_some() => self.sets.asking = None,
            SetInput::Back if self.sets.viewing.is_some() => {
                self.sets.viewing = None;
                if self.sets.editing.is_none() {
                    self.sets_refresh();
                }
            }
            SetInput::Back => {
                let adding = self
                    .sets
                    .editing
                    .as_ref()
                    .is_some_and(|e| e.adding.is_some());
                if adding {
                    self.sets_button(SetButton::Done);
                } else {
                    self.sets.editing = None;
                    self.sets_refresh();
                }
            }
        }
    }

    fn sets_row(&mut self, r: usize) {
        if self.sets.running.is_some() && self.sets.viewing.is_some() {
            // The song being soundchecked stays shown until it's over.
            self.status = Some(("Soundcheck is running: Stop soundcheck first".into(), false));
            return;
        }
        if let Some(v) = self.sets.viewing.as_mut() {
            if v.songs.as_ref().is_some_and(|s| r < s.len()) {
                v.picked = Some(r);
            }
            return;
        }
        let checking = self.sets.checking.is_some();
        match self.sets.editing.as_mut() {
            None => {
                if r >= self.sets.sets.len() {
                    return;
                }
                if self.sets.selected == Some(r) {
                    // A second tap is Edit set.
                    self.sets_button(SetButton::EditSet);
                } else {
                    self.sets.selected = Some(r);
                }
            }
            Some(e) => match e.adding.as_mut() {
                Some((sel, _)) => {
                    let n = self.sets.songs.as_ref().map_or(0, Vec::len);
                    if r >= n {
                        return;
                    }
                    if *sel == Some(r) && !checking {
                        self.sets_add(r);
                    } else {
                        *sel = Some(r);
                    }
                }
                None => {
                    if r < e.set.songs.len() {
                        e.picked = Some(r);
                        e.focus = None;
                        e.needs = false;
                    }
                }
            },
        }
    }

    fn sets_scroll(&mut self, d: i32) {
        let (list, songs) = (self.list_rows(), self.song_rows());
        let n_songs = self.sets.songs.as_ref().map_or(0, Vec::len);
        let n_sets = self.sets.sets.len();
        if let Some(v) = self.sets.viewing.as_mut() {
            let n = v.songs.as_ref().map_or(0, Vec::len);
            let max = n.saturating_sub(songs) as i32;
            v.top = (v.top as i32 + d).clamp(0, max.max(0)) as usize;
            return;
        }
        let (top, n, vis) = match self.sets.editing.as_mut() {
            None => (&mut self.sets.top, n_sets, list),
            Some(e) => match e.adding.as_mut() {
                Some((_, top)) => (top, n_songs, list),
                None => {
                    let n = e.set.songs.len();
                    (&mut e.top, n, songs)
                }
            },
        };
        let max = n.saturating_sub(vis) as i32;
        *top = (*top as i32 + d).clamp(0, max.max(0)) as usize;
    }

    /// The dial: the next or the one before, in the list showing.
    fn sets_pick(&mut self, d: i32) {
        let (list, songs) = (self.list_rows(), self.song_rows());
        let n_songs = self.sets.songs.as_ref().map_or(0, Vec::len);
        let n_sets = self.sets.sets.len();
        let step = |sel: Option<usize>, n: usize| -> Option<usize> {
            (n > 0).then(|| sel.map_or(0, |s| s as i32 + d).clamp(0, n as i32 - 1) as usize)
        };
        if let Some(v) = self.sets.viewing.as_mut() {
            let n = v.songs.as_ref().map_or(0, Vec::len);
            v.picked = step(v.picked, n);
            v.top = in_view(v.picked, v.top, songs, n);
            return;
        }
        match self.sets.editing.as_mut() {
            None => {
                self.sets.selected = step(self.sets.selected, n_sets);
                self.sets.top = in_view(self.sets.selected, self.sets.top, list, n_sets);
            }
            Some(e) => match e.adding.as_mut() {
                Some((sel, top)) => {
                    *sel = step(*sel, n_songs);
                    *top = in_view(*sel, *top, list, n_songs);
                }
                None => {
                    let n = e.set.songs.len();
                    e.picked = step(e.picked, n);
                    e.top = in_view(e.picked, e.top, songs, n);
                    e.needs = false;
                    e.armed = false;
                }
            },
        }
    }

    /// The set picked in the list, readable, with its place.
    fn picked_set(&self) -> Option<(Loc, Result<Set, String>)> {
        let (e, s) = self.sets.sets.get(self.sets.selected?)?;
        let loc = Loc::top(Place::Sets).join(&e.name).ok()?;
        Some((loc, s.clone()))
    }

    /// The set a button on the card works on: the one checked, the one
    /// edited, or the one picked.
    fn this_set(&self) -> Option<(Loc, Set)> {
        if let Some(v) = &self.sets.viewing {
            return self.set_at(&v.loc).map(|s| (v.loc.clone(), s));
        }
        if let Some(e) = &self.sets.editing {
            return Some((e.loc.clone(), e.set.clone()));
        }
        match self.picked_set()? {
            (loc, Ok(set)) => Some((loc, set)),
            (_, Err(_)) => None,
        }
    }

    fn sets_button(&mut self, b: SetButton) {
        let checking = self.sets.checking.is_some();
        let always = matches!(
            b,
            SetButton::NeedsDone
                | SetButton::Done
                | SetButton::StopSoundcheck
                | SetButton::SkipSong
                | SetButton::CancelSoundcheck
                | SetButton::SoundcheckDone
        );
        if checking && !always {
            self.status = Some(("Checking the set: one moment".into(), false));
            return;
        }
        match b {
            SetButton::NewSet => {
                self.modal = Modal::Keyboard {
                    purpose: Purpose::NewSet,
                    title: "Name the new set".into(),
                    text: String::new(),
                    shift: true,
                };
            }
            SetButton::PlaySet => {
                if self.sets.running.is_some() {
                    self.status =
                        Some(("Soundcheck is running: Stop soundcheck first".into(), false));
                    return;
                }
                let Some((loc, set)) = self.this_set() else {
                    return self.set_unreadable();
                };
                if set.songs.is_empty() {
                    self.status = Some((
                        format!("{} has no songs yet: Edit set adds them", stem(loc.name())),
                        false,
                    ));
                    return;
                }
                if self.unsaved {
                    self.modal = Modal::Confirm {
                        text: format!(
                            "Unsaved changes. Play \u{201c}{}\u{201d} anyway?",
                            stem(loc.name())
                        ),
                        what: Confirm::PlaySet(loc, Box::new(set)),
                    };
                    return;
                }
                self.actions.push(BrowseAction::PlaySet { loc, set });
            }
            SetButton::EditSet => {
                let Some((loc, set)) = self.this_set() else {
                    return self.set_unreadable();
                };
                let picked = (!set.songs.is_empty()).then_some(0);
                self.sets.editing = Some(Editing {
                    loc,
                    set,
                    picked,
                    top: 0,
                    adding: None,
                    focus: None,
                    needs: false,
                    armed: false,
                });
            }
            SetButton::CheckSet | SetButton::CheckEveryFile => {
                let Some((loc, set)) = self.this_set() else {
                    return self.set_unreadable();
                };
                self.check_view(loc, set, b == SetButton::CheckEveryFile);
            }
            // Soundcheck (live shows B2b): it asks first.
            SetButton::Soundcheck => {
                let Some((loc, set)) = self.this_set() else {
                    return self.set_unreadable();
                };
                self.ask_soundcheck(loc, set, None);
            }
            SetButton::SoundcheckSong => {
                let Some((loc, set)) = self.this_set() else {
                    return self.set_unreadable();
                };
                let Some(k) = self.sets.viewing.as_ref().and_then(|v| v.picked) else {
                    return;
                };
                self.ask_soundcheck(loc, set, Some(k));
            }
            SetButton::StartSoundcheck => {
                let Some(ask) = self.sets.asking.clone() else {
                    return;
                };
                let Some(set) = self.set_at(&ask.loc) else {
                    return self.set_unreadable();
                };
                if !self.soundcheck_open_song().1 {
                    self.status = Some(("Save the song open now first".into(), false));
                    return;
                }
                self.sets.asking = None;
                self.status = Some((
                    format!("Soundcheck: {}\u{2026}", stem(ask.loc.name())),
                    false,
                ));
                self.actions.push(BrowseAction::Soundcheck {
                    loc: ask.loc,
                    set,
                    only: ask.only,
                    muted: ask.muted,
                });
            }
            SetButton::CancelSoundcheck => self.sets.asking = None,
            SetButton::StopSoundcheck => {
                if self.sets.running.is_some() {
                    self.actions.push(BrowseAction::StopSoundcheck);
                }
            }
            SetButton::SkipSong => {
                if self.sets.running.is_some() {
                    self.actions.push(BrowseAction::SkipSoundcheckSong);
                }
            }
            SetButton::SoundcheckDone => self.sets.sound_done = None,
            SetButton::Rename => {
                let Some((loc, _)) = self.picked_set() else {
                    return;
                };
                self.modal = Modal::Keyboard {
                    title: format!("Rename {}", stem(loc.name())),
                    text: stem(loc.name()).to_string(),
                    purpose: Purpose::RenameSet(loc),
                    shift: false,
                };
            }
            SetButton::Copy => {
                if let Some((loc, _)) = self.picked_set() {
                    self.send(Job::Copy {
                        from: loc,
                        to: Loc::top(Place::Sets),
                    });
                }
            }
            SetButton::Delete => {
                if let Some((loc, _)) = self.picked_set() {
                    self.modal = Modal::Confirm {
                        text: format!(
                            "Move the set {} to the Trash? Its songs stay in Projects.",
                            stem(loc.name())
                        ),
                        what: Confirm::Delete(loc),
                    };
                }
            }
            SetButton::AddSongs => {
                if let Some(e) = self.sets.editing.as_mut() {
                    e.adding = Some((None, 0));
                    e.needs = false;
                    e.focus = None;
                    self.send(Job::Songs);
                }
            }
            SetButton::Done => {
                if let Some(e) = self.sets.editing.as_mut() {
                    e.adding = None;
                    let rows = match self.sets.rows.1 {
                        0 => 9,
                        n => n,
                    };
                    e.top = in_view(e.picked, e.top, rows, e.set.songs.len());
                }
            }
            SetButton::MoveUp | SetButton::MoveDown => {
                let up = b == SetButton::MoveUp;
                let rows = self.song_rows();
                let Some(e) = self.sets.editing.as_mut().filter(|e| e.adding.is_none()) else {
                    return;
                };
                let Some(k) = e.picked else { return };
                let to = if up { k.checked_sub(1) } else { Some(k + 1) };
                let Some(to) = to.filter(|&t| t < e.set.songs.len()) else {
                    return;
                };
                e.set.songs.swap(k, to);
                e.picked = Some(to);
                e.top = in_view(e.picked, e.top, rows, e.set.songs.len());
                self.sets_save();
            }
            SetButton::KeepChanges | SetButton::ThrowChanges => {
                let keep = b == SetButton::KeepChanges;
                let Some(e) = self.sets.editing.as_mut() else {
                    return;
                };
                let Some(song) = e.picked.and_then(|k| e.set.songs.get_mut(k)) else {
                    return;
                };
                let mut copies = std::mem::take(&mut song.on_stage);
                let name = song.name.clone();
                let Some(newest) = copies.pop() else {
                    return;
                };
                let to_trash: Vec<String> = if keep {
                    copies
                } else {
                    copies.into_iter().chain([newest.clone()]).collect()
                };
                for c in &to_trash {
                    if let Ok(l) = Loc::top(Place::Projects).join(c) {
                        self.send(Job::Delete(l));
                    }
                }
                if keep {
                    match (
                        Loc::top(Place::Projects).join(&newest),
                        Loc::top(Place::Projects).join(&name),
                    ) {
                        (Ok(copy), Ok(song)) => {
                            // The copy stays listed until it is in.
                            if let Some(s) = self
                                .sets
                                .editing
                                .as_mut()
                                .and_then(|e| e.picked.and_then(|k| e.set.songs.get_mut(k)))
                            {
                                s.on_stage = vec![newest.clone()];
                            }
                            self.status =
                                Some((format!("Keeping the changes in {name}\u{2026}"), false));
                            self.send(Job::KeepOnStage { copy, song });
                        }
                        _ => return,
                    }
                } else {
                    self.status = Some((
                        format!(
                            "Threw away {name}\u{2019}s changes from the show: they\u{2019}re in the Trash"
                        ),
                        false,
                    ));
                }
                self.sets_save();
            }
            SetButton::OpenInMake => {
                let Some(name) = self.picked_song().map(|s| s.name.clone()) else {
                    return;
                };
                let Ok(loc) = Loc::top(Place::Projects).join(&name) else {
                    return;
                };
                if self.unsaved {
                    self.modal = Modal::Confirm {
                        text: format!("Unsaved changes. Open \u{201c}{name}\u{201d} anyway?"),
                        what: Confirm::Open(loc),
                    };
                } else {
                    self.send(Job::OpenProject(loc));
                }
            }
            SetButton::Remove => {
                if let Some(e) = self.sets.editing.as_mut() {
                    if e.picked.is_some() {
                        e.armed = true;
                    }
                }
            }
            SetButton::ConfirmRemove => {
                let rows = self.song_rows();
                let Some(e) = self.sets.editing.as_mut() else {
                    return;
                };
                let Some(k) = e.picked.filter(|&k| k < e.set.songs.len()) else {
                    return;
                };
                let gone = e.set.songs.remove(k);
                e.armed = false;
                let n = e.set.songs.len();
                e.picked = (n > 0).then(|| k.min(n - 1));
                e.top = in_view(e.picked, e.top, rows, n);
                self.status = Some((
                    format!("Took {} out of the set (it stays in Projects)", gone.name),
                    false,
                ));
                self.sets_save();
            }
            SetButton::NeedsDone => {
                if let Some(e) = self.sets.editing.as_mut() {
                    e.needs = false;
                }
            }
        }
    }

    fn set_unreadable(&mut self) {
        if let Some((loc, Err(why))) = self.picked_set() {
            self.status = Some((
                format!("{} can\u{2019}t be read: {why}", stem(loc.name())),
                true,
            ));
        }
    }

    fn picked_song(&self) -> Option<&SetSong> {
        if let Some(v) = &self.sets.viewing {
            let k = v.picked?;
            return match &self.sets.editing {
                Some(e) if e.loc == v.loc => e.set.songs.get(k),
                _ => self
                    .sets
                    .sets
                    .iter()
                    .find(|(e, _)| e.name == v.loc.name())
                    .and_then(|(_, s)| s.as_ref().ok())
                    .or(self
                        .sets
                        .playing
                        .as_ref()
                        .filter(|(l, _)| *l == v.loc)
                        .map(|(_, s)| s))
                    .and_then(|s| s.songs.get(k)),
            };
        }
        let e = self.sets.editing.as_ref()?;
        e.set.songs.get(e.picked?)
    }

    fn picked_song_mut(&mut self) -> Option<&mut SetSong> {
        let e = self.sets.editing.as_mut()?;
        let k = e.picked?;
        e.set.songs.get_mut(k)
    }

    /// Puts project `i` of the list after the song picked (or at the end).
    fn sets_add(&mut self, i: usize) {
        let Some((name, id)) = self.sets.songs.as_ref().and_then(|s| s.get(i)).cloned() else {
            return;
        };
        let rows = self.song_rows();
        let Some(e) = self.sets.editing.as_mut() else {
            return;
        };
        if e.set.songs.len() >= mpfree_library::sets::MAX_SONGS {
            self.status = Some((
                format!(
                    "A set holds {} songs at most",
                    mpfree_library::sets::MAX_SONGS
                ),
                true,
            ));
            return;
        }
        let at = e.picked.map_or(e.set.songs.len(), |k| k + 1);
        e.set.songs.insert(at, SetSong::new(&id, &name));
        e.picked = Some(at);
        e.top = in_view(e.picked, e.top, rows, e.set.songs.len());
        let n = e.set.songs.len();
        self.status = Some((format!("Added {name} as song {} of {n}", at + 1), false));
        self.sets_save();
    }

    /// Push in a set's songs: the next value the dial turns (Starts, Ends,
    /// Fade when it applies), then none.
    fn sets_focus_next(&mut self) {
        let fade = matches!(self.picked_song().map(|s| s.ends), Some(Ends::Fade(_)));
        let Some(e) = self.sets.editing.as_mut() else {
            return;
        };
        if e.picked.is_none() {
            return;
        }
        e.focus = match e.focus {
            None => Some(SetField::Starts),
            Some(SetField::Starts) => Some(SetField::Ends),
            Some(SetField::Ends) if fade => Some(SetField::Fade),
            _ => None,
        };
    }

    fn sets_step(&mut self, f: SetField, d: i32) {
        if f == SetField::Sound {
            if let Some(a) = self.sets.asking.as_mut() {
                a.muted = !a.muted;
                self.sets.muted = a.muted;
            }
            return;
        }
        if f == SetField::Needs {
            if let Some(e) = self.sets.editing.as_mut().filter(|e| e.picked.is_some()) {
                e.needs = true;
                e.focus = None;
            }
            return;
        }
        let Some(song) = self.picked_song_mut() else {
            return;
        };
        match f {
            SetField::Starts => song.starts = turn(&Starts::ALL, song.starts, d),
            SetField::Ends => {
                let all = [Ends::Finish, Ends::Fade(FADE_DEFAULT), Ends::Cut];
                let now = match song.ends {
                    Ends::Fade(_) => Ends::Fade(FADE_DEFAULT),
                    e => e,
                };
                song.ends = turn(&all, now, d);
            }
            SetField::Fade => match song.ends {
                Ends::Fade(s) => {
                    song.ends =
                        Ends::Fade((s as i32 + d).clamp(FADE_MIN as i32, FADE_MAX as i32) as u8)
                }
                _ => return,
            },
            SetField::Needs | SetField::Sound => return,
        }
        self.sets_save();
    }

    fn sets_notes(&mut self) {
        let Some(song) = self.picked_song() else {
            return;
        };
        self.modal = Modal::Keyboard {
            purpose: Purpose::SetNotes,
            title: format!("Notes for {}", song.name),
            text: song.notes.replace('\n', " "),
            shift: song.notes.is_empty(),
        };
    }

    /// The notes typed: the picked song keeps them.
    pub(super) fn sets_notes_typed(&mut self, text: String) {
        if let Some(song) = self.picked_song_mut() {
            song.set_notes(text.trim());
            self.sets_save();
        }
    }

    /// The rig's things and the song's other needs, in the Needs list's
    /// order: (name, the song's need of it, connected).
    fn need_list(&self, song: &SetSong) -> Vec<(String, Option<bool>, Option<bool>)> {
        let mut out: Vec<(String, Option<bool>, Option<bool>)> = self
            .sets
            .rig
            .iter()
            .map(|r| {
                let need = song
                    .needs
                    .iter()
                    .find(|n| n.name == r.name)
                    .map(|n| n.needed);
                (r.name.clone(), need, Some(r.connected))
            })
            .collect();
        for n in &song.needs {
            if !self.sets.rig.iter().any(|r| r.name == n.name) {
                out.push((n.name.clone(), Some(n.needed), None));
            }
        }
        out
    }

    /// A tap on the Needs list: not needed, Needed, Nice to have, and round.
    fn sets_need(&mut self, k: usize) {
        let Some(song) = self.picked_song() else {
            return;
        };
        let Some((name, need, _)) = self.need_list(song).into_iter().nth(k) else {
            return;
        };
        let Some(song) = self.picked_song_mut() else {
            return;
        };
        song.needs.retain(|n| n.name != name);
        match need {
            None => song.needs.push(Need { name, needed: true }),
            Some(true) => song.needs.push(Need {
                name,
                needed: false,
            }),
            Some(false) => {}
        }
        self.sets_save();
    }

    /// The keyboard's Done for a set's name.
    pub(super) fn sets_named(&mut self, purpose: Purpose, name: String) {
        match purpose {
            Purpose::NewSet => {
                self.sets.new_set = true;
                self.send(Job::NewSet(name));
            }
            Purpose::RenameSet(loc) => {
                let new = format!("{name}{SET_EXT}");
                if loc.name() != new {
                    self.select_name = Some(new.clone());
                    self.send(Job::Rename { loc, name: new });
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ screen

    pub(super) fn sets_screen(&self) -> SetsScreen {
        let (status, status_tone) = match &self.status {
            Some((s, bad)) => (s.clone(), if *bad { Tone::Bad } else { Tone::Dim }),
            None => (String::new(), Tone::Dim),
        };
        let checking = self.sets.checking.is_some();
        let mut s = SetsScreen {
            status,
            status_tone,
            ..SetsScreen::default()
        };
        match (&self.sets.viewing, &self.sets.editing) {
            (Some(v), _) => self.check_screen(v, &mut s),
            (None, None) => self.list_screen(&mut s),
            (None, Some(e)) if e.adding.is_some() => self.add_screen(e, &mut s),
            (None, Some(e)) => self.edit_screen(e, &mut s),
        }
        if checking {
            s.detail.faint = s
                .detail
                .buttons
                .iter()
                .copied()
                .filter(|b| !matches!(b, SetButton::NeedsDone | SetButton::Done))
                .collect();
        }
        s
    }

    fn list_screen(&self, s: &mut SetsScreen) {
        s.view = SetsView::List;
        s.title = "Sets".into();
        s.head_buttons = vec![SetButton::NewSet];
        s.empty = if self.sets.read {
            "No sets yet: New set makes one".into()
        } else {
            "Reading\u{2026}".into()
        };
        s.rows = self
            .sets
            .sets
            .iter()
            .map(|(e, set)| match set {
                Ok(set) => SetRow {
                    number: None,
                    name: stem(&e.name).to_string(),
                    sub: if set.songs.is_empty() {
                        "no songs yet".into()
                    } else {
                        plural(set.songs.len(), "song")
                    },
                    sub_tone: Tone::Dim,
                    chip: set_chip(set),
                    add: false,
                    ..SetRow::default()
                },
                Err(_) => SetRow {
                    number: None,
                    name: stem(&e.name).to_string(),
                    sub: "can\u{2019}t be read".into(),
                    sub_tone: Tone::Bad,
                    chip: None,
                    add: false,
                    ..SetRow::default()
                },
            })
            .collect();
        s.selected = self.sets.selected;
        s.top = self.sets.top;
        let d = &mut s.detail;
        d.eyebrow = "Selected".into();
        let Some((loc, set)) = self.picked_set() else {
            d.line = "A set is a list of songs to play in order".into();
            return;
        };
        d.title = stem(loc.name()).to_string();
        d.line = "Set".into();
        let set = match set {
            Ok(set) => set,
            Err(why) => {
                d.reason = Some((format!("It can\u{2019}t be read: {why}"), Tone::Bad));
                d.buttons = vec![SetButton::Rename, SetButton::Copy, SetButton::Delete];
                return;
            }
        };
        d.facts
            .push(("Songs".into(), set.songs.len().to_string(), Tone::Dim));
        let (_, _, _, _, at) = tally(&set);
        if set.songs.is_empty() {
            d.facts
                .push(("Checked".into(), "nothing to check yet".into(), Tone::Dim));
        } else if at == 0 {
            d.facts
                .push(("Checked".into(), "not yet".into(), Tone::Dim));
        } else {
            d.facts.push(("Checked".into(), when(at), Tone::Dim));
            d.facts.push((String::new(), tally_words(&set), Tone::Dim));
        }
        // Soundcheck (live shows B2b): when, and how many songs.
        if !set.songs.is_empty() {
            let checked: Vec<u64> = set
                .songs
                .iter()
                .filter_map(|s| s.soundcheck.as_ref().map(|c| c.at))
                .collect();
            let value = match checked.iter().max() {
                None => "not yet".to_string(),
                Some(&at) => {
                    let n = set.songs.len();
                    let some = if checked.len() == n {
                        format!("all {n} songs")
                    } else {
                        format!("{} of {n} songs", checked.len())
                    };
                    format!("{} \u{b7} {some}", when(at))
                }
            };
            d.facts.push(("Soundcheck".into(), value, Tone::Dim));
        }
        let worst = |pick: fn(&SongCheck) -> Option<&String>| -> Vec<String> {
            set.songs
                .iter()
                .enumerate()
                .filter_map(|(i, song)| {
                    let (_, c) = song.check.as_ref()?;
                    pick(c).map(|why| format!("{} {}: {why}.", i + 1, song.name))
                })
                .collect()
        };
        let not = worst(|c| c.not_ready.first());
        let limits = worst(|c| {
            if c.not_ready.is_empty() {
                c.limits.first()
            } else {
                None
            }
        });
        if !not.is_empty() {
            d.lists.push(("Not ready".into(), Tone::Bad, not));
        }
        if !limits.is_empty() {
            d.lists.push(("With limits".into(), Tone::Warn, limits));
        }
        // Check every file is on the check view's title line (live shows
        // B2); Soundcheck after Check set (B2b).
        d.buttons = vec![
            SetButton::PlaySet,
            SetButton::EditSet,
            SetButton::CheckSet,
            SetButton::Soundcheck,
            SetButton::Rename,
            SetButton::Copy,
            SetButton::Delete,
        ];
        if self.sets.running.is_some() {
            d.faint = vec![SetButton::PlaySet, SetButton::Soundcheck];
        } else if self.sets.playing.is_some() {
            d.faint = vec![SetButton::Soundcheck];
        }
    }

    /// The check view (live shows B2; layout spec 4p): each song with its
    /// chip and numbers, the picked song's every check beside it.
    fn check_screen(&self, v: &CheckView, s: &mut SetsScreen) {
        s.view = SetsView::Check;
        s.title = format!("Check \u{b7} {}", stem(v.loc.name()));
        s.back = true;
        s.head_buttons = vec![SetButton::CheckEveryFile];
        s.empty = "Checking\u{2026}".into();
        let set = self.set_at(&v.loc);
        let d = &mut s.detail;
        let (Some(songs), Some(set)) = (&v.songs, set) else {
            s.sub = Some((None, "Checking every song\u{2026}".into()));
            d.eyebrow = "Selected".into();
            d.line = "Checking every song\u{2026}".into();
            return;
        };
        let (mut ready, mut limits, mut not) = (0, 0, 0);
        for p in songs {
            match p.chip() {
                "NOT READY" => not += 1,
                "LIMITS" => limits += 1,
                _ => ready += 1,
            }
        }
        let mut counts = Vec::new();
        if ready > 0 {
            counts.push(format!("{ready} ready"));
        }
        if limits > 0 {
            counts.push(format!("{limits} with limits"));
        }
        if not > 0 {
            counts.push(format!("{not} not ready"));
        }
        // As the approved drawing has it: the soundcheck only once there is
        // one (each song's line says "not soundchecked yet" until then).
        let now = self.sets.clock.unwrap_or_else(now_utc);
        let mut words = vec![counts.join(", ")];
        words.push(format!(
            "{} {}",
            if v.deep {
                "every file read"
            } else {
                "quick check"
            },
            when_short(v.at, now)
        ));
        let soundchecked = set
            .songs
            .iter()
            .filter_map(|s| s.soundcheck.as_ref().map(|c| c.at))
            .max();
        if let Some(at) = soundchecked {
            words.push(format!("soundchecked {}", when_short(at, now)));
        }
        let set_chip = if not > 0 {
            chip_of("NOT READY")
        } else if limits > 0 {
            chip_of("LIMITS")
        } else {
            chip_of("READY")
        };
        s.sub = Some((Some(set_chip), words.join(" \u{b7} ")));
        s.rows = songs
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let song = set.songs.get(i);
                let name = song.map_or(String::new(), |s| s.name.clone());
                let numbers = match (song.and_then(|s| s.soundcheck.as_ref()), p.estimate) {
                    (Some(sc), _) => sc.line(),
                    (None, Some(e)) => format!("{} \u{b7} not soundchecked yet", e.line()),
                    (None, None) => String::new(),
                };
                let first_bad = p
                    .lines
                    .iter()
                    .find(|l| l.mark == Mark::NotReady)
                    .map(|l| l.words.clone());
                let (sub, sub_tone) = match first_bad {
                    Some(w) if numbers.is_empty() => (w, Tone::Bad),
                    Some(w) => (format!("{w} \u{b7} {numbers}"), Tone::Bad),
                    None => (numbers, Tone::Dim),
                };
                SetRow {
                    number: Some(i + 1),
                    name,
                    sub,
                    sub_tone,
                    chip: Some(chip_of(p.chip())),
                    add: false,
                    ..SetRow::default()
                }
            })
            .collect();
        s.selected = v.picked;
        s.top = v.top;
        // Soundcheck (live shows B2b): running on this set, its question,
        // or how it went.
        if let Some(n) = self.sets.running.as_ref().filter(|n| n.loc == v.loc) {
            return self.soundcheck_running_screen(n, &set, s);
        }
        if let Some(a) = self.sets.asking.as_ref().filter(|a| a.loc == v.loc) {
            return self.soundcheck_ask_screen(a, &set, &mut s.detail);
        }
        if let Some((_, rows, words)) = self
            .sets
            .sound_done
            .as_ref()
            .filter(|(l, _, _)| *l == v.loc)
        {
            d.eyebrow = "Soundcheck".into();
            d.title = stem(v.loc.name()).to_string();
            d.checks = rows.clone();
            d.after = Some((words.clone(), Tone::Dim));
            d.buttons = vec![SetButton::SoundcheckDone];
            return;
        }
        let Some(k) = v.picked.filter(|&k| k < songs.len()) else {
            d.eyebrow = "Selected".into();
            d.line = "Pick a song to see every check".into();
            return;
        };
        let p = &songs[k];
        let song = set.songs.get(k);
        d.eyebrow = format!("Song {} of {}", k + 1, songs.len());
        d.title = song.map_or(String::new(), |s| s.name.clone());
        d.chip = Some(chip_of(p.chip()));
        let at = when_short(v.at, self.sets.clock.unwrap_or_else(now_utc));
        d.line = match p.check.tempo() {
            Some(tempo) => format!("{tempo} \u{b7} checked {at}"),
            None => format!("checked {at}"),
        };
        d.checks = p
            .lines
            .iter()
            .map(|l| CheckRow {
                mark: match l.mark {
                    Mark::Fine => CheckMark::Fine,
                    Mark::Note => CheckMark::Note,
                    Mark::Limits => CheckMark::Limits,
                    Mark::NotReady => CheckMark::NotReady,
                },
                word: l.area.word().to_string(),
                text: l.words.clone(),
            })
            .collect();
        d.buttons = vec![SetButton::SoundcheckSong, SetButton::OpenInMake];
        if self.sets.playing.is_some() || self.sets.running.is_some() {
            d.faint = vec![SetButton::SoundcheckSong];
        }
    }

    /// The check view while Soundcheck plays its set: the title button
    /// stops it, its line says how far it is, the song it's on is outlined
    /// with a SOUNDCHECK chip and those to come wait, faint; the Selected
    /// column shows its three steps for that song.
    fn soundcheck_running_screen(&self, n: &SoundcheckNow, set: &Set, s: &mut SetsScreen) {
        s.head_buttons = vec![SetButton::StopSoundcheck];
        let left = n.left_s.div_ceil(60);
        s.sub = Some((
            Some(("SOUNDCHECK".into(), Tone::Accent)),
            format!(
                "song {} of {} \u{b7} {} \u{b7} {}",
                (n.pos + 1).min(n.songs.len().max(1)),
                n.songs.len(),
                if left <= 1 {
                    "about a minute left".to_string()
                } else {
                    format!("about {left} min left")
                },
                if n.out_loud {
                    "playing out loud"
                } else {
                    "muted"
                }
            ),
        ));
        let to_come: Vec<usize> = n.songs.iter().skip(n.pos + 1).copied().collect();
        for (i, row) in s.rows.iter_mut().enumerate() {
            if i == n.song {
                row.lit = true;
                row.chip = Some(("SOUNDCHECK".into(), Tone::Accent));
                if !n.row_line.is_empty() {
                    row.sub = n.row_line.clone();
                    row.sub_tone = Tone::Dim;
                }
            } else if to_come.contains(&i) {
                row.faint = true;
                row.sub = "waiting".into();
                row.sub_tone = Tone::Dim;
            }
        }
        s.selected = None;
        let d = &mut s.detail;
        let song = set.songs.get(n.song);
        d.eyebrow = format!(
            "Soundcheck \u{b7} song {} of {}",
            n.song + 1,
            set.songs.len()
        );
        d.title = song.map_or(String::new(), |s| s.name.clone());
        d.line = song.map_or(String::new(), |s| {
            let ends = format!("ends: {}", s.ends.label());
            match s.check.as_ref().and_then(|(_, c)| c.tempo()) {
                Some(tempo) => format!("{tempo} \u{b7} {ends}"),
                None => ends,
            }
        });
        d.checks = n.steps.clone();
        d.meter = n.meter;
        d.meter_facts = n.meter_facts.clone();
        d.meter_at = 2;
        if n.out_loud {
            d.after = Some((
                "Playing out loud on Main out. Turn the PA down, or stop and start again Muted."
                    .into(),
                Tone::Warn,
            ));
        }
        d.buttons = vec![SetButton::StopSoundcheck, SetButton::SkipSong];
    }

    /// Soundcheck's question in the Selected column: what it does and about
    /// how long, Sound (Out loud or Muted), a line when the song open now
    /// has changes, then Start soundcheck and Cancel.
    fn soundcheck_ask_screen(&self, a: &SoundAsk, set: &Set, d: &mut crate::sets::SetDetail) {
        let about = soundcheck_about(set, a.only);
        d.eyebrow = "Soundcheck".into();
        match a.only.and_then(|k| set.songs.get(k).map(|s| (k, s))) {
            Some((k, song)) => {
                d.title = song.name.clone();
                d.line = format!("Song {} of {}", k + 1, set.songs.len());
                d.para = Some(match set.songs.get(k + 1) {
                    Some(next) => format!(
                        "Opens {} cold and plays its busiest part, then its change into {}: {about}.",
                        song.name, next.name
                    ),
                    None => format!(
                        "Opens {} cold and plays its busiest part: {about}.",
                        song.name
                    ),
                });
            }
            None => {
                let n = set.songs.len();
                d.title = stem(a.loc.name()).to_string();
                d.line = plural(n, "song");
                d.para = Some(if n == 1 {
                    format!("Opens its song cold and plays its busiest part: {about}.")
                } else {
                    format!(
                        "Opens each of the {n} songs in turn and plays its busiest part, then its change into the next: {about}."
                    )
                });
            }
        }
        d.steppers = vec![Stepper {
            field: SetField::Sound,
            value: if a.muted { "Muted" } else { "Out loud" }.into(),
            faint: false,
            focused: false,
        }];
        let (line, can) = self.soundcheck_open_song();
        d.after = Some(line.unwrap_or_else(|| {
            if a.muted {
                (
                    "Muted measures the same; nothing leaves the outputs".into(),
                    Tone::Dim,
                )
            } else {
                (
                    "It plays on Main out: Muted measures the same, silently".into(),
                    Tone::Dim,
                )
            }
        }));
        d.buttons = vec![SetButton::StartSoundcheck, SetButton::CancelSoundcheck];
        if !can {
            d.faint = vec![SetButton::StartSoundcheck];
        }
    }

    fn edit_screen(&self, e: &Editing, s: &mut SetsScreen) {
        s.view = SetsView::Edit;
        s.title = stem(e.loc.name()).to_string();
        s.back = true;
        s.head_buttons = vec![SetButton::AddSongs, SetButton::CheckSet];
        s.empty = "No songs yet: Add songs picks them from Projects".into();
        s.rows = e
            .set
            .songs
            .iter()
            .enumerate()
            .map(|(i, song)| {
                let changed = !song.on_stage.is_empty();
                SetRow {
                    number: Some(i + 1),
                    name: song.name.clone(),
                    sub: if changed {
                        "changed on stage".into()
                    } else {
                        song_sub(song)
                    },
                    sub_tone: if changed { Tone::Accent2 } else { Tone::Dim },
                    chip: Some(chip(song.check.as_ref().map(|(_, c)| c))),
                    add: false,
                    ..SetRow::default()
                }
            })
            .collect();
        s.selected = e.picked;
        s.top = e.top;
        let n = e.set.songs.len();
        let d = &mut s.detail;
        let Some((k, song)) = e.picked.and_then(|k| Some((k, e.set.songs.get(k)?))) else {
            d.eyebrow = "Selected".into();
            d.line = "Add songs puts projects from Projects in the set".into();
            return;
        };
        d.title = song.name.clone();
        let checked = match &song.check {
            None => "not checked yet".to_string(),
            Some((at, c)) => {
                let word = match c.word() {
                    "READY" => "Ready",
                    "LIMITS" => "Ready with limits",
                    "NOT READY" => "Not ready",
                    _ => "Files not checked",
                };
                match c.tempo() {
                    Some(t) => format!("{t} \u{b7} {word}, checked {}", when(*at)),
                    None => format!("{word}, checked {}", when(*at)),
                }
            }
        };
        d.line = format!("Projects \u{b7} {checked}");
        if let Some((_, c)) = &song.check {
            d.reason = c
                .not_ready
                .first()
                .map(|w| (w.clone(), Tone::Bad))
                .or_else(|| c.limits.first().map(|w| (w.clone(), Tone::Warn)));
        }
        if e.needs {
            d.eyebrow = format!("Needs \u{b7} song {} of {n}", k + 1);
            d.line = "Tap one: Needed, Nice to have, or not needed".into();
            d.reason = None;
            d.needs = Some(
                self.need_list(song)
                    .into_iter()
                    .map(|(name, need, connected)| NeedRow {
                        name,
                        state: match need {
                            Some(true) => "Needed".into(),
                            Some(false) => "Nice to have".into(),
                            None => "\u{2014}".into(),
                        },
                        tone: match need {
                            Some(true) => Tone::Good,
                            Some(false) => Tone::Warn,
                            None => Tone::Dim,
                        },
                        connected: match connected {
                            Some(true) => "connected".into(),
                            Some(false) => "not connected".into(),
                            None => "not in your rig".into(),
                        },
                        connected_tone: match (connected, need) {
                            (Some(true), _) => Tone::Dim,
                            (_, Some(true)) => Tone::Bad,
                            _ => Tone::Dim,
                        },
                    })
                    .collect(),
            );
            if d.needs.as_ref().is_some_and(Vec::is_empty) {
                d.line =
                    "Your rig has no keyboards or learned controls yet: they come from I/O".into();
            }
            d.buttons = vec![SetButton::NeedsDone];
            return;
        }
        d.eyebrow = format!("Song {} of {n}", k + 1);
        let fade = match song.ends {
            Ends::Fade(sec) => Some(sec),
            _ => None,
        };
        d.steppers = vec![
            Stepper {
                field: SetField::Starts,
                value: song.starts.label().into(),
                faint: false,
                focused: e.focus == Some(SetField::Starts),
            },
            Stepper {
                field: SetField::Ends,
                value: match song.ends {
                    Ends::Fade(_) => "Fade".into(),
                    other => other.label(),
                },
                faint: false,
                focused: e.focus == Some(SetField::Ends),
            },
            Stepper {
                field: SetField::Fade,
                value: fade.map_or("\u{2014}".into(), |s| format!("{s} s")),
                faint: fade.is_none(),
                focused: e.focus == Some(SetField::Fade) && fade.is_some(),
            },
            Stepper {
                field: SetField::Needs,
                value: needs_words(song),
                faint: false,
                focused: false,
            },
        ];
        let count = song.notes.chars().count();
        d.notes = Some((
            song.notes.clone(),
            if count == 0 {
                format!(
                    "Tap to type: the key, cues, who starts \u{b7} up to {MAX_NOTES} characters"
                )
            } else {
                format!("Tap to type \u{b7} {count} of {MAX_NOTES} characters")
            },
        ));
        d.buttons = vec![
            SetButton::MoveUp,
            SetButton::MoveDown,
            SetButton::OpenInMake,
            if e.armed {
                SetButton::ConfirmRemove
            } else {
                SetButton::Remove
            },
        ];
        // Changed on stage (live shows B1): its changes wait in a recovery
        // copy until they are kept or thrown away.
        if let Some(newest) = song.on_stage.last() {
            let mut lines = vec![format!(
                "Its changes from the show are in Projects as \u{201c}{newest}\u{201d}."
            )];
            if song.on_stage.len() > 1 {
                lines.push(format!(
                    "It changed {} times: Keep the changes keeps the last, and the others go to the Trash.",
                    song.on_stage.len()
                ));
            }
            lines.push(
                "Keep the changes puts them in the song; its Earlier saves keep the version from before."
                    .into(),
            );
            d.lists
                .push(("Changed on stage".into(), Tone::Accent2, lines));
            d.buttons
                .splice(0..0, [SetButton::KeepChanges, SetButton::ThrowChanges]);
        }
        if k == 0 {
            d.faint.push(SetButton::MoveUp);
        }
        if k + 1 == n {
            d.faint.push(SetButton::MoveDown);
        }
    }

    fn add_screen(&self, e: &Editing, s: &mut SetsScreen) {
        let (sel, top) = e.adding.unwrap_or((None, 0));
        s.view = SetsView::Add;
        s.title = format!("Add songs to {}", stem(e.loc.name()));
        s.back = true;
        s.head_buttons = vec![SetButton::Done];
        s.empty = match &self.sets.songs {
            None => "Reading Projects\u{2026}".into(),
            Some(_) => "No projects in Projects yet: save one, and it can join a set".into(),
        };
        s.rows = self
            .sets
            .songs
            .iter()
            .flatten()
            .map(|(name, id)| {
                let times = e
                    .set
                    .songs
                    .iter()
                    .filter(|song| {
                        if id.is_empty() || song.id.is_empty() {
                            song.name == *name
                        } else {
                            song.id == *id
                        }
                    })
                    .count();
                SetRow {
                    number: None,
                    name: name.clone(),
                    sub: match times {
                        0 => String::new(),
                        1 => "in the set".into(),
                        n => format!("in the set {n} times"),
                    },
                    sub_tone: Tone::Dim,
                    chip: None,
                    add: true,
                    ..SetRow::default()
                }
            })
            .collect();
        s.selected = sel;
        s.top = top;
        let d = &mut s.detail;
        d.eyebrow = "Adding songs".into();
        d.title = stem(e.loc.name()).to_string();
        let n = e.set.songs.len();
        d.line = match e.picked {
            Some(k) if k + 1 < n => format!("+ puts a song after song {}", k + 1),
            _ => "+ puts a song at the end".into(),
        };
        if n > 0 {
            d.lists.push((
                plural(n, "song"),
                Tone::Dim,
                e.set
                    .songs
                    .iter()
                    .enumerate()
                    .map(|(i, song)| format!("{} {}", i + 1, song.name))
                    .collect(),
            ));
        }
    }
}
