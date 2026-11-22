//! The Sets place's logic (live shows B1; layout spec 4n): which set is
//! picked, the set being edited and its picked song, the projects to add,
//! the rig the app gave, and what a touch, the dial or a button does.
//! Every read and write goes to the library's worker (`Job::Sets`,
//! `Job::Songs`, `Job::SaveSet`, `Job::CheckSet`, `Job::NewSet`); the set
//! saves itself after every change. [`crate::sets`] draws it.

use super::*;
use crate::sets::{
    NeedRow, RigThing, SetButton, SetField, SetInput, SetRow, SetsScreen, SetsView, Stepper, Tone,
};
use mpfree_library::sets::{FADE_DEFAULT, FADE_MAX, FADE_MIN, MAX_NOTES};
use mpfree_library::{Ends, Need, SetSong, SongCheck, Starts, SET_EXT};

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
        let connected = self
            .sets
            .rig
            .iter()
            .filter(|r| r.connected)
            .map(|r| r.name.clone())
            .collect();
        self.sets.checking = Some(loc.clone());
        self.send(Job::CheckSet {
            loc: loc.clone(),
            set,
            connected,
            deep: false,
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
    pub(super) fn sets_checked(&mut self, loc: Loc, checks: Vec<(Found, SongCheck)>) {
        if self.sets.checking.as_ref() == Some(&loc) {
            self.sets.checking = None;
        }
        let now = now_utc();
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

    /// The set a button on the card works on: the one edited, or the one
    /// picked.
    fn this_set(&self) -> Option<(Loc, Set)> {
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
        if checking && b != SetButton::NeedsDone && b != SetButton::Done {
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
                if set.songs.is_empty() {
                    self.status =
                        Some(("No songs to check yet: Add songs picks them".into(), false));
                    return;
                }
                let deep = b == SetButton::CheckEveryFile;
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
                });
            }
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
            SetField::Needs => return,
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
        match &self.sets.editing {
            None => self.list_screen(&mut s),
            Some(e) if e.adding.is_some() => self.add_screen(e, &mut s),
            Some(e) => self.edit_screen(e, &mut s),
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
                },
                Err(_) => SetRow {
                    number: None,
                    name: stem(&e.name).to_string(),
                    sub: "can\u{2019}t be read".into(),
                    sub_tone: Tone::Bad,
                    chip: None,
                    add: false,
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
        d.buttons = vec![
            SetButton::PlaySet,
            SetButton::EditSet,
            SetButton::CheckSet,
            SetButton::Rename,
            SetButton::Copy,
            SetButton::Delete,
            SetButton::CheckEveryFile,
        ];
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
