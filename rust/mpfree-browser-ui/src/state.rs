//! The browser's logic: where it is, what it lists, what is selected, what
//! a tap, the dial or a button does. No drawing here: [`Browser::screen`]
//! gives the widget what to show, and the widget's touches come back as
//! [`Input`]s. Every file operation goes to the library's worker thread
//! (or runs at once, for previews and tests), so nothing here blocks.

use mpfree_library::audio::duration;
use mpfree_library::{
    valid_name, AudioFormat, AudioInfo, Done, Entry, Job, Kind, Library, Loc, Meta, Outcome, Place,
    Snapshot, TrashItem, Worker,
};
use std::path::PathBuf;

/// The rows of the places column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Place(Place),
    Recent,
    Favourites,
    /// The kits made from the owner's own backup (kits.bin): read only.
    Factory,
}

/// The places column, top to bottom. A gap is drawn before Recent and
/// before Trash.
pub const SOURCES: [Source; 9] = [
    Source::Place(Place::Projects),
    Source::Place(Place::Samples),
    Source::Place(Place::Kits),
    Source::Place(Place::Patterns),
    Source::Place(Place::Recordings),
    Source::Recent,
    Source::Favourites,
    Source::Factory,
    Source::Place(Place::Trash),
];

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Place(p) => p.label(),
            Source::Recent => "Recent",
            Source::Favourites => "Favourites",
            Source::Factory => "Factory",
        }
    }
}

/// What the list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Audio,
    Projects,
    Kits,
}

pub const FILTERS: [Filter; 4] = [Filter::All, Filter::Audio, Filter::Projects, Filter::Kits];

impl Filter {
    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Audio => "Audio",
            Filter::Projects => "Projects",
            Filter::Kits => "Kits",
        }
    }

    fn shows(self, k: Kind) -> bool {
        match self {
            Filter::All => true,
            Filter::Audio => matches!(k, Kind::Folder | Kind::Audio(_)),
            Filter::Projects => matches!(k, Kind::Folder | Kind::Project),
            Filter::Kits => matches!(k, Kind::Folder | Kind::Kit),
        }
    }
}

/// One row of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub kind: Kind,
    /// Right-hand text: a length, a size, where it came from.
    pub detail: String,
    pub favourite: bool,
    /// Where it is (None for a factory kit).
    pub loc: Option<Loc>,
    /// For a row in the Trash.
    pub trash_id: Option<String>,
    /// For a factory kit: its index in kits.bin.
    pub factory: Option<usize>,
}

/// The buttons under the selected item, and the list's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Load,
    LoadToPad,
    Open,
    Audition,
    Favourite,
    Rename,
    Copy,
    Cut,
    Paste,
    NewFolder,
    Delete,
    Restore,
    EmptyTrash,
}

impl Button {
    pub fn label(self) -> &'static str {
        match self {
            Button::Load => "Load",
            Button::LoadToPad => "Load to pad",
            Button::Open => "Open",
            Button::Audition => "Audition",
            Button::Favourite => "Favourite",
            Button::Rename => "Rename",
            Button::Copy => "Copy",
            Button::Cut => "Cut",
            Button::Paste => "Paste",
            Button::NewFolder => "New folder",
            Button::Delete => "Delete",
            Button::Restore => "Put back",
            Button::EmptyTrash => "Empty Trash",
        }
    }
}

/// A key of the on-screen keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Space,
    Backspace,
    Shift,
    Cancel,
    Done,
}

/// What the widget's touches meant.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Source(usize),
    /// A tap on a row: selects it, or opens it if it was selected.
    Row(usize),
    /// The list dragged by this many rows (positive: further down).
    Scroll(i32),
    Back,
    Filter(Filter),
    Button(Button),
    Key(Key),
    ConfirmYes,
    ConfirmNo,
}

/// What the app must do (the browser does everything else itself).
#[derive(Clone, Debug, PartialEq)]
pub enum BrowseAction {
    /// Give this sound to the sampler (it decodes the file).
    LoadSample {
        loc: Loc,
        path: PathBuf,
    },
    /// The same, then the next pad hit takes it (the sampler's
    /// hit-to-assign).
    LoadToPad {
        loc: Loc,
        path: PathBuf,
    },
    /// A kit file from the library.
    LoadKit {
        loc: Loc,
        path: PathBuf,
    },
    /// A kit from kits.bin, by index; `to_pad` for Load to pad.
    LoadFactoryKit {
        index: usize,
        to_pad: bool,
    },
    /// Play this file once through the preview path, at the preview level.
    Audition {
        path: PathBuf,
    },
    StopAudition,
    /// A project was read: give each engine its part.
    ProjectOpened {
        loc: Loc,
        snap: Snapshot,
    },
    /// A project was saved where it says.
    ProjectSaved {
        loc: Loc,
    },
    /// Save as was given a name: call [`Browser::save`] with the engines'
    /// parts now.
    SaveRequested,
    /// Recent and Favourites changed: save this text as the "browser"
    /// owner's file in the store ([`Browser::lists_text`]).
    SaveLists(String),
}

/// What waits for a yes.
#[derive(Clone, Debug, PartialEq)]
pub enum Confirm {
    Delete(Loc),
    EmptyTrash,
}

/// What the keyboard is typing.
#[derive(Clone, Debug, PartialEq)]
pub enum Purpose {
    Rename(Loc),
    NewFolder(Loc),
    /// A name for the project being saved.
    SaveAs,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    None,
    Confirm {
        what: Confirm,
        text: String,
    },
    Keyboard {
        purpose: Purpose,
        title: String,
        text: String,
        shift: bool,
    },
}

/// What the right-hand panel shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detail {
    pub title: String,
    /// "WAV audio", "Project", "Folder" ...
    pub kind: String,
    /// Label and value lines.
    pub lines: Vec<(String, String)>,
    /// A quick waveform: (min, max) per column.
    pub overview: Option<Vec<(f32, f32)>>,
    pub buttons: Vec<Button>,
}

/// Everything the widget draws.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub source: usize,
    /// `"Samples / Drums"`.
    pub path: String,
    pub can_go_back: bool,
    pub rows: Vec<Row>,
    pub selected: Option<usize>,
    /// The first row shown.
    pub top: usize,
    pub filter: Filter,
    pub detail: Detail,
    /// The list's own buttons (New folder, Paste, Empty Trash).
    pub list_buttons: Vec<Button>,
    pub audition: bool,
    /// A line under the list: what just happened, or an error.
    pub status: Option<(String, bool)>,
    pub busy: bool,
    pub modal: Modal,
    /// `"Copied: Kick.wav"` while something is on the clipboard.
    pub clipboard: Option<String>,
}

impl Screen {
    /// Nothing to show (before the browser has listed anything).
    pub fn empty() -> Screen {
        Screen {
            source: 1,
            path: String::new(),
            can_go_back: false,
            rows: Vec::new(),
            selected: None,
            top: 0,
            filter: Filter::All,
            detail: Detail::default(),
            list_buttons: Vec::new(),
            audition: true,
            status: None,
            busy: false,
            modal: Modal::None,
            clipboard: None,
        }
    }
}

/// A quick waveform: (min, max) per column.
pub type Overview = Vec<(f32, f32)>;

/// How many rows Recent keeps.
pub const RECENT: usize = 20;
/// Earlier saves kept per project.
pub const KEEP_VERSIONS: usize = mpfree_library::project::KEEP_VERSIONS;
/// Overview columns asked of the worker.
pub const OVERVIEW_COLUMNS: usize = 360;

enum Exec {
    /// Runs each job at once (previews, tests).
    Inline(Library),
    Thread(Worker, Library),
}

enum Clip {
    Copy(Loc),
    Cut(Loc),
}

/// The browser.
pub struct Browser {
    exec: Exec,
    source: usize,
    folder: Loc,
    entries: Vec<Entry>,
    trash: Vec<TrashItem>,
    factory: Vec<String>,
    rows: Vec<Row>,
    selected: Option<usize>,
    top: usize,
    visible: usize,
    filter: Filter,
    audition: bool,
    detail_info: Option<(Loc, AudioInfo)>,
    detail_overview: Option<(Loc, Option<Overview>)>,
    detail_meta: Option<(Loc, Meta)>,
    status: Option<(String, bool)>,
    pending: usize,
    modal: Modal,
    clip: Option<Clip>,
    recent: Vec<Loc>,
    favourites: Vec<Loc>,
    /// Pick this name when the list next arrives.
    select_name: Option<String>,
    /// The project being worked on (where SHIFT + BROWSE saves).
    project: Option<Loc>,
    /// When it was first saved.
    created: u64,
    actions: Vec<BrowseAction>,
}

impl Browser {
    /// A browser whose jobs run on the library's worker thread.
    pub fn new(lib: Library) -> Browser {
        let w = Worker::start(lib.clone());
        Browser::with(Exec::Thread(w, lib))
    }

    /// A browser whose jobs run at once on the caller's thread (previews,
    /// tests).
    pub fn inline(lib: Library) -> Browser {
        Browser::with(Exec::Inline(lib))
    }

    fn with(exec: Exec) -> Browser {
        let mut b = Browser {
            exec,
            source: 1,
            folder: Loc::top(Place::Samples),
            entries: Vec::new(),
            trash: Vec::new(),
            factory: Vec::new(),
            rows: Vec::new(),
            selected: None,
            top: 0,
            visible: 11,
            filter: Filter::All,
            audition: true,
            detail_info: None,
            detail_overview: None,
            detail_meta: None,
            status: None,
            pending: 0,
            modal: Modal::None,
            clip: None,
            recent: Vec::new(),
            favourites: Vec::new(),
            select_name: None,
            project: None,
            created: 0,
            actions: Vec::new(),
        };
        b.refresh();
        b
    }

    pub fn library(&self) -> &Library {
        match &self.exec {
            Exec::Inline(l) | Exec::Thread(_, l) => l,
        }
    }

    // ------------------------------------------------------------ jobs

    fn send(&mut self, job: Job) {
        match &mut self.exec {
            Exec::Inline(lib) => {
                let result = mpfree_library::worker::run(lib, job).map_err(|e| e.to_string());
                self.apply(Done { id: 0, result });
            }
            Exec::Thread(w, _) => {
                if w.send(job) != 0 {
                    self.pending += 1;
                } else {
                    self.status = Some(("The library is not running".into(), true));
                }
            }
        }
    }

    /// Takes what the worker finished; call once a frame. True if the
    /// screen changed.
    pub fn poll(&mut self) -> bool {
        let done = match &self.exec {
            Exec::Thread(w, _) => w.take(),
            Exec::Inline(_) => Vec::new(),
        };
        let any = !done.is_empty();
        for d in done {
            self.pending = self.pending.saturating_sub(1);
            self.apply(d);
        }
        any
    }

    fn apply(&mut self, d: Done) {
        match d.result {
            Err(e) => self.status = Some((e, true)),
            Ok(Outcome::Listed(loc, entries)) => {
                if loc == self.folder && self.current_source().is_place() {
                    self.entries = entries;
                    self.rebuild();
                }
            }
            Ok(Outcome::Trash(items)) => {
                if self.current() == Source::Place(Place::Trash) {
                    self.trash = items;
                    self.rebuild();
                }
            }
            Ok(Outcome::Info(loc, info)) => self.detail_info = Some((loc, info)),
            Ok(Outcome::Overview(loc, o)) => self.detail_overview = Some((loc, o)),
            Ok(Outcome::Read(loc, bytes)) => {
                if let Ok(m) = Meta::parse(&String::from_utf8_lossy(&bytes)) {
                    self.detail_meta = loc.parent().map(|p| (p, m));
                }
            }
            Ok(Outcome::At(loc)) => {
                self.status = Some((format!("Done: {}", loc.display()), false));
                if loc.parent().as_ref() == Some(&self.folder) {
                    self.select_name = Some(loc.name().to_string());
                }
                self.refresh();
            }
            Ok(Outcome::Deleted(item)) => {
                self.status = Some((format!("Moved {} to the Trash", item.name), false));
                if let Some(from) = &item.from {
                    self.forget(from);
                }
                self.refresh();
            }
            Ok(Outcome::Emptied(bytes)) => {
                self.status = Some((format!("Trash emptied: {} freed", size(bytes)), false));
                self.refresh();
            }
            Ok(Outcome::Saved(loc)) => {
                self.status = Some((format!("Saved {}", loc.name()), false));
                self.project = Some(loc.clone());
                self.remember(loc.clone());
                self.actions.push(BrowseAction::ProjectSaved { loc });
                self.refresh();
            }
            Ok(Outcome::Project(loc, snap)) => {
                self.status = Some((format!("Opened {}", loc.name()), false));
                self.project = Some(loc.clone());
                self.created = snap.meta.created;
                self.remember(loc.clone());
                self.actions.push(BrowseAction::ProjectOpened { loc, snap });
            }
        }
    }

    /// Lists the current place again.
    pub fn refresh(&mut self) {
        match self.current() {
            Source::Place(Place::Trash) => self.send(Job::Trash),
            Source::Place(_) => self.send(Job::List(self.folder.clone())),
            _ => self.rebuild(),
        }
    }

    // ------------------------------------------------------------ rows

    fn current(&self) -> Source {
        SOURCES[self.source]
    }

    fn current_source(&self) -> SourceKind {
        SourceKind(self.current())
    }

    fn rebuild(&mut self) {
        let keep = self.select_name.take().or_else(|| {
            self.selected
                .and_then(|i| self.rows.get(i))
                .map(|r| r.name.clone())
        });
        let fav = |l: &Loc| self.favourites.contains(l);
        let mut rows: Vec<Row> = match self.current() {
            Source::Place(Place::Trash) => self
                .trash
                .iter()
                .map(|t| Row {
                    name: t.name.clone(),
                    kind: t.kind,
                    detail: t
                        .from
                        .as_ref()
                        .and_then(Loc::parent)
                        .map(|p| format!("from {}", p.display()))
                        .unwrap_or_default(),
                    favourite: false,
                    loc: None,
                    trash_id: Some(t.id.clone()),
                    factory: None,
                })
                .collect(),
            Source::Place(_) => self
                .entries
                .iter()
                .filter_map(|e| {
                    let loc = self.folder.join(&e.name).ok()?;
                    Some(Row {
                        detail: entry_detail(e),
                        favourite: fav(&loc),
                        name: e.name.clone(),
                        kind: e.kind,
                        loc: Some(loc),
                        trash_id: None,
                        factory: None,
                    })
                })
                .collect(),
            Source::Recent | Source::Favourites => {
                let list = if self.current() == Source::Recent {
                    &self.recent
                } else {
                    &self.favourites
                };
                list.iter()
                    .map(|l| Row {
                        name: l.name().to_string(),
                        kind: guess_kind(l),
                        detail: l.parent().map(|p| p.display()).unwrap_or_default(),
                        favourite: fav(l),
                        loc: Some(l.clone()),
                        trash_id: None,
                        factory: None,
                    })
                    .collect()
            }
            Source::Factory => self
                .factory
                .iter()
                .enumerate()
                .map(|(i, n)| Row {
                    name: n.clone(),
                    kind: Kind::Kit,
                    detail: "read only".into(),
                    favourite: false,
                    loc: None,
                    trash_id: None,
                    factory: Some(i),
                })
                .collect(),
        };
        rows.retain(|r| self.filter.shows(r.kind));
        self.rows = rows;
        self.selected = keep
            .and_then(|n| self.rows.iter().position(|r| r.name == n))
            .or(if self.rows.is_empty() { None } else { Some(0) });
        self.top = self.top.min(self.rows.len().saturating_sub(1));
        self.keep_in_view();
        self.on_select(false);
    }

    fn keep_in_view(&mut self) {
        let v = self.visible.max(1);
        if let Some(s) = self.selected {
            if s < self.top {
                self.top = s;
            } else if s >= self.top + v {
                self.top = s + 1 - v;
            }
        }
        self.top = self.top.min(self.rows.len().saturating_sub(v));
    }

    /// The selection changed: ask for its details, and audition it if a
    /// person moved it (not when a list arrives).
    fn on_select(&mut self, by_hand: bool) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        let Some(loc) = row.loc.clone() else {
            return;
        };
        match row.kind {
            Kind::Audio(_) if row.trash_id.is_none() => {
                let have = self.detail_info.as_ref().is_some_and(|(l, _)| *l == loc);
                if !have {
                    self.send(Job::Info(loc.clone()));
                    self.send(Job::Overview(loc.clone(), OVERVIEW_COLUMNS));
                }
                if self.audition && by_hand {
                    if let Ok(path) = self.library().file_path(&loc) {
                        self.actions.push(BrowseAction::Audition { path });
                    }
                }
            }
            Kind::Project => {
                if let Ok(f) = loc.join(mpfree_library::PROJECT_FILE) {
                    self.send(Job::Read(f, 64 * 1024));
                }
            }
            _ => {}
        }
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.selected.and_then(|i| self.rows.get(i))
    }

    // ------------------------------------------------------------ lists

    /// The factory kits' names (from kits.bin), shown under Factory.
    pub fn set_factory(&mut self, names: Vec<String>) {
        self.factory = names;
        if self.current() == Source::Factory {
            self.rebuild();
        }
    }

    /// Recent and Favourites, as saved by [`Browser::lists_text`].
    pub fn load_lists(&mut self, text: &str) {
        self.recent.clear();
        self.favourites.clear();
        for line in text.lines() {
            if let Some(l) = line.strip_prefix("recent = ").and_then(Loc::from_line) {
                if self.recent.len() < RECENT && !self.recent.contains(&l) {
                    self.recent.push(l);
                }
            } else if let Some(l) = line.strip_prefix("favourite = ").and_then(Loc::from_line) {
                if !self.favourites.contains(&l) {
                    self.favourites.push(l);
                }
            }
        }
        self.rebuild();
    }

    /// Recent and Favourites as text, one `recent = Samples/Drums/Kick.wav`
    /// or `favourite = ...` line each.
    pub fn lists_text(&self) -> String {
        let mut s = String::new();
        for l in &self.recent {
            s.push_str(&format!("recent = {}\n", l.to_line()));
        }
        for l in &self.favourites {
            s.push_str(&format!("favourite = {}\n", l.to_line()));
        }
        s
    }

    fn remember(&mut self, l: Loc) {
        self.recent.retain(|r| *r != l);
        self.recent.insert(0, l);
        self.recent.truncate(RECENT);
        self.actions
            .push(BrowseAction::SaveLists(self.lists_text()));
    }

    fn forget(&mut self, l: &Loc) {
        let before = self.recent.len() + self.favourites.len();
        self.recent.retain(|r| !l.contains(r));
        self.favourites.retain(|r| !l.contains(r));
        if self.recent.len() + self.favourites.len() != before {
            self.actions
                .push(BrowseAction::SaveLists(self.lists_text()));
        }
    }

    // ------------------------------------------------------------ moving around

    pub fn go_source(&mut self, i: usize) {
        if i >= SOURCES.len() {
            return;
        }
        self.source = i;
        if let Source::Place(p) = SOURCES[i] {
            self.folder = Loc::top(p);
        }
        self.top = 0;
        self.selected = None;
        self.entries.clear();
        self.trash.clear();
        self.modal = Modal::None;
        self.refresh();
    }

    fn go_folder(&mut self, folder: Loc) {
        if let Some(i) = SOURCES
            .iter()
            .position(|s| *s == Source::Place(folder.place))
        {
            self.source = i;
        }
        self.select_name = None;
        self.folder = folder;
        self.top = 0;
        self.selected = None;
        self.entries.clear();
        self.refresh();
    }

    /// Up one folder (the - button, Back).
    pub fn back(&mut self) {
        if let Some(parent) = self.folder.parent() {
            if self.current_source().is_place() {
                let name = self.folder.name().to_string();
                self.go_folder(parent);
                self.select_name = Some(name);
            }
        }
    }

    /// Into the selected folder (the + button).
    pub fn enter(&mut self) {
        if let Some(r) = self.selected_row().cloned() {
            if r.kind == Kind::Folder {
                if let Some(l) = r.loc {
                    self.go_folder(l);
                }
            }
        }
    }

    /// The data dial: moves the selection.
    pub fn dial(&mut self, delta: i32) {
        if !matches!(self.modal, Modal::None) || self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as i32;
        let s = self
            .selected
            .map_or(0, |s| s as i32 + delta)
            .clamp(0, n - 1) as usize;
        if Some(s) != self.selected {
            self.selected = Some(s);
            self.keep_in_view();
            self.on_select(true);
        }
    }

    /// SHIFT + the data dial: moves through the places.
    pub fn dial_places(&mut self, delta: i32) {
        let n = SOURCES.len() as i32;
        let i = (self.source as i32 + delta).rem_euclid(n) as usize;
        self.go_source(i);
    }

    /// The dial's push: open a folder or project, load anything else, or
    /// answer the question on screen.
    pub fn push(&mut self) {
        match &self.modal {
            Modal::Confirm { .. } => return self.input(Input::ConfirmYes),
            Modal::Keyboard { .. } => return self.input(Input::Key(Key::Done)),
            Modal::None => {}
        }
        let Some(r) = self.selected_row().cloned() else {
            return;
        };
        match r.kind {
            Kind::Folder | Kind::Project => self.button(Button::Open),
            _ if r.trash_id.is_some() => {}
            _ => self.button(Button::Load),
        }
    }

    /// PLAY while the browser is open: hear the selected sound.
    pub fn play(&mut self) {
        if let Some(Row {
            kind: Kind::Audio(_),
            loc: Some(loc),
            ..
        }) = self.selected_row().cloned()
        {
            if let Ok(path) = self.library().file_path(&loc) {
                self.actions.push(BrowseAction::Audition { path });
            }
        }
    }

    /// ERASE while the browser is open: Delete (asks first).
    pub fn erase(&mut self) {
        self.button(Button::Delete);
    }

    /// The list was given this many rows of room by the widget.
    pub fn set_visible(&mut self, rows: usize) {
        if rows > 0 && rows != self.visible {
            self.visible = rows;
            self.keep_in_view();
        }
    }

    // ------------------------------------------------------------ input

    pub fn input(&mut self, i: Input) {
        if !matches!(self.modal, Modal::None)
            && !matches!(i, Input::Key(_) | Input::ConfirmYes | Input::ConfirmNo)
        {
            return;
        }
        match i {
            Input::Source(s) => self.go_source(s),
            Input::Row(r) if r < self.rows.len() => {
                if self.selected == Some(r) {
                    self.push();
                } else {
                    self.selected = Some(r);
                    self.keep_in_view();
                    self.on_select(true);
                }
            }
            Input::Row(_) => {}
            Input::Scroll(d) => {
                let max = self.rows.len().saturating_sub(self.visible.max(1)) as i32;
                self.top = (self.top as i32 + d).clamp(0, max.max(0)) as usize;
            }
            Input::Back => self.back(),
            Input::Filter(f) => {
                self.filter = f;
                self.rebuild();
            }
            Input::Button(b) => self.button(b),
            Input::Key(k) => self.key(k),
            Input::ConfirmYes => self.confirm(true),
            Input::ConfirmNo => self.confirm(false),
        }
    }

    fn button(&mut self, b: Button) {
        let row = self.selected_row().cloned();
        let loc = row.as_ref().and_then(|r| r.loc.clone());
        match b {
            Button::Load | Button::LoadToPad => {
                let to_pad = b == Button::LoadToPad;
                let Some(r) = row else { return };
                if let Some(index) = r.factory {
                    self.actions
                        .push(BrowseAction::LoadFactoryKit { index, to_pad });
                    self.status = Some((format!("Loading {}", r.name), false));
                    return;
                }
                let Some(loc) = loc else { return };
                match r.kind {
                    Kind::Project | Kind::Folder => self.button(Button::Open),
                    Kind::Audio(_) | Kind::Kit => {
                        let Ok(path) = self.library().file_path(&loc) else {
                            self.status = Some((format!("{} is not there", r.name), true));
                            return;
                        };
                        self.actions.push(match (r.kind, to_pad) {
                            (Kind::Kit, _) => BrowseAction::LoadKit {
                                loc: loc.clone(),
                                path,
                            },
                            (_, true) => BrowseAction::LoadToPad {
                                loc: loc.clone(),
                                path,
                            },
                            _ => BrowseAction::LoadSample {
                                loc: loc.clone(),
                                path,
                            },
                        });
                        self.status = Some((
                            if to_pad {
                                format!("{}: hit a pad to put it there", r.name)
                            } else {
                                format!("Loaded {}", r.name)
                            },
                            false,
                        ));
                        self.remember(loc);
                    }
                    _ => self.status = Some((format!("{} is not a sound", r.name), true)),
                }
            }
            Button::Open => {
                let Some(r) = row else { return };
                let Some(loc) = loc else { return };
                match r.kind {
                    Kind::Folder => self.go_folder(loc),
                    Kind::Project => self.send(Job::OpenProject(loc)),
                    _ => self.button(Button::Load),
                }
            }
            Button::Audition => {
                self.audition = !self.audition;
                if self.audition {
                    self.play();
                } else {
                    self.actions.push(BrowseAction::StopAudition);
                }
            }
            Button::Favourite => {
                let Some(loc) = loc else { return };
                if let Some(i) = self.favourites.iter().position(|f| *f == loc) {
                    self.favourites.remove(i);
                } else {
                    self.favourites.push(loc);
                }
                self.actions
                    .push(BrowseAction::SaveLists(self.lists_text()));
                self.rebuild();
            }
            Button::Rename => {
                let Some(loc) = loc else { return };
                self.modal = Modal::Keyboard {
                    title: format!("Rename {}", loc.name()),
                    text: loc.name().to_string(),
                    purpose: Purpose::Rename(loc),
                    shift: false,
                };
            }
            Button::NewFolder => {
                if self.current_source().is_writable_place() {
                    self.modal = Modal::Keyboard {
                        title: format!("New folder in {}", self.folder.display()),
                        text: "New folder".into(),
                        purpose: Purpose::NewFolder(self.folder.clone()),
                        shift: false,
                    };
                }
            }
            Button::Copy | Button::Cut => {
                let Some(loc) = loc else { return };
                self.status = Some((
                    format!(
                        "{} {}: go to a folder and press Paste",
                        if b == Button::Copy { "Copied" } else { "Cut" },
                        loc.name()
                    ),
                    false,
                ));
                self.clip = Some(if b == Button::Copy {
                    Clip::Copy(loc)
                } else {
                    Clip::Cut(loc)
                });
            }
            Button::Paste => {
                if !self.current_source().is_writable_place() {
                    return;
                }
                let to = self.folder.clone();
                match self.clip.take() {
                    Some(Clip::Copy(from)) => {
                        self.send(Job::Copy {
                            from: from.clone(),
                            to,
                        });
                        self.clip = Some(Clip::Copy(from));
                    }
                    Some(Clip::Cut(from)) => {
                        let moved_to = to.join(from.name()).ok();
                        if self.favourites.contains(&from) || self.recent.contains(&from) {
                            if let Some(n) = moved_to {
                                for l in self.favourites.iter_mut().chain(self.recent.iter_mut()) {
                                    if *l == from {
                                        *l = n.clone();
                                    }
                                }
                                self.actions
                                    .push(BrowseAction::SaveLists(self.lists_text()));
                            }
                        }
                        self.send(Job::Move { from, to });
                    }
                    None => {}
                }
            }
            Button::Delete => {
                let Some(r) = row else { return };
                let Some(loc) = loc else { return };
                if r.factory.is_some() || r.trash_id.is_some() {
                    return;
                }
                self.modal = Modal::Confirm {
                    text: format!("Move \u{201c}{}\u{201d} to the Trash?", r.name),
                    what: Confirm::Delete(loc),
                };
            }
            Button::Restore => {
                if let Some(id) = row.and_then(|r| r.trash_id) {
                    self.send(Job::Restore(id));
                }
            }
            Button::EmptyTrash => {
                if !self.trash.is_empty() {
                    let total = self.trash.len();
                    self.modal = Modal::Confirm {
                        text: format!(
                            "Delete the {total} item{} in the Trash for good?",
                            if total == 1 { "" } else { "s" }
                        ),
                        what: Confirm::EmptyTrash,
                    };
                }
            }
        }
    }

    fn confirm(&mut self, yes: bool) {
        let m = std::mem::replace(&mut self.modal, Modal::None);
        if let (true, Modal::Confirm { what, .. }) = (yes, m) {
            match what {
                Confirm::Delete(loc) => {
                    if self.clip.as_ref().is_some_and(|c| match c {
                        Clip::Copy(l) | Clip::Cut(l) => loc.contains(l),
                    }) {
                        self.clip = None;
                    }
                    self.send(Job::Delete(loc));
                }
                Confirm::EmptyTrash => self.send(Job::EmptyTrash),
            }
        }
    }

    fn key(&mut self, k: Key) {
        let Modal::Keyboard {
            purpose,
            text,
            shift,
            ..
        } = &mut self.modal
        else {
            return;
        };
        match k {
            Key::Char(c) => {
                if text.len() < mpfree_library::name::MAX_NAME {
                    text.push(if *shift { c.to_ascii_uppercase() } else { c });
                    *shift = false;
                }
            }
            Key::Space => {
                if !text.is_empty() && text.len() < mpfree_library::name::MAX_NAME {
                    text.push(' ');
                }
            }
            Key::Backspace => {
                text.pop();
            }
            Key::Shift => *shift = !*shift,
            Key::Cancel => self.modal = Modal::None,
            Key::Done => {
                let name = text.trim().to_string();
                if !valid_name(&name) {
                    self.status = Some((
                        "That name can't be used: no / \\ : * ? \" < > |, and not starting with a dot".into(),
                        true,
                    ));
                    return;
                }
                let purpose = purpose.clone();
                self.modal = Modal::None;
                match purpose {
                    Purpose::Rename(loc) => {
                        if loc.name() != name {
                            self.send(Job::Rename { loc, name });
                        }
                    }
                    Purpose::NewFolder(parent) => self.send(Job::NewFolder { parent, name }),
                    Purpose::SaveAs => {
                        if let Ok(loc) = Loc::top(Place::Projects).join(&name) {
                            self.project = Some(loc);
                            self.actions.push(BrowseAction::SaveRequested);
                        }
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ projects

    /// The project being worked on, if it has been saved or opened.
    pub fn project(&self) -> Option<&Loc> {
        self.project.as_ref()
    }

    /// SHIFT + BROWSE: saves `snap` as the current project, or, the first
    /// time, as a new project named for the time (no typing needed;
    /// Rename changes it later). `now` is seconds since 1970 (UTC).
    pub fn save(&mut self, mut snap: Snapshot, now: u64) {
        let loc = match &self.project {
            Some(l) => l.clone(),
            None => {
                let name = self.library().new_project_name(now);
                match Loc::top(Place::Projects).join(&name) {
                    Ok(l) => l,
                    Err(e) => {
                        self.status = Some((e.to_string(), true));
                        return;
                    }
                }
            }
        };
        if snap.meta.name.is_empty() {
            snap.meta.name = loc.name().to_string();
        }
        if self.project.is_none() || self.created == 0 {
            self.created = now;
        }
        if snap.meta.created == 0 {
            snap.meta.created = self.created;
        }
        snap.meta.saved = now;
        self.status = Some((format!("Saving {}", loc.name()), false));
        self.send(Job::SaveProject {
            project: loc,
            snap,
            keep: KEEP_VERSIONS,
        });
    }

    /// Save as: asks for a name (the time-stamped one ready to keep).
    pub fn save_as(&mut self, now: u64) {
        self.modal = Modal::Keyboard {
            purpose: Purpose::SaveAs,
            title: "Save the project as".into(),
            text: self
                .project
                .as_ref()
                .map(|l| l.name().to_string())
                .unwrap_or_else(|| self.library().new_project_name(now)),
            shift: false,
        };
    }

    /// Forgets the current project (a new, empty one is being made).
    pub fn new_project(&mut self) {
        self.project = None;
        self.created = 0;
    }

    // ------------------------------------------------------------ out

    /// What the app must do since the last call.
    pub fn take_actions(&mut self) -> Vec<BrowseAction> {
        std::mem::take(&mut self.actions)
    }

    /// What the widget draws.
    pub fn screen(&self) -> Screen {
        let src = self.current();
        Screen {
            source: self.source,
            path: match src {
                Source::Place(Place::Trash) => "Trash".into(),
                Source::Place(_) => self.folder.display(),
                s => s.label().into(),
            },
            can_go_back: src != Source::Place(Place::Trash)
                && self.current_source().is_place()
                && !self.folder.is_top(),
            rows: self.rows.clone(),
            selected: self.selected,
            top: self.top,
            filter: self.filter,
            detail: self.detail(),
            list_buttons: match src {
                Source::Place(Place::Trash) => vec![Button::EmptyTrash],
                Source::Place(p) => {
                    // Projects are made by saving, so no New folder there.
                    let mut v = if p == Place::Projects {
                        Vec::new()
                    } else {
                        vec![Button::NewFolder]
                    };
                    if self.clip.is_some() {
                        v.push(Button::Paste);
                    }
                    v
                }
                _ => Vec::new(),
            },
            audition: self.audition,
            status: self.status.clone(),
            busy: self.pending > 0,
            modal: self.modal.clone(),
            clipboard: self.clip.as_ref().map(|c| match c {
                Clip::Copy(l) => format!("Copied: {}", l.name()),
                Clip::Cut(l) => format!("Cut: {}", l.name()),
            }),
        }
    }

    fn detail(&self) -> Detail {
        let Some(r) = self.selected_row() else {
            return Detail {
                title: self.current().label().into(),
                kind: match self.current() {
                    Source::Place(Place::Trash) => "The Trash is empty".into(),
                    Source::Factory => "No factory kits on this stick".into(),
                    Source::Recent => "Nothing loaded or saved yet".into(),
                    Source::Favourites => "Nothing starred yet".into(),
                    _ => "Empty folder".into(),
                },
                ..Detail::default()
            };
        };
        let mut d = Detail {
            title: r.name.clone(),
            kind: kind_label(r.kind),
            ..Detail::default()
        };
        if let Some(from) = &r.trash_id.as_ref().and(Some(&r.detail)) {
            d.lines
                .push(("Was in".into(), from.trim_start_matches("from ").into()));
            d.buttons = vec![Button::Restore];
            return d;
        }
        if r.factory.is_some() {
            d.kind = "Factory kit (read only)".into();
            d.lines
                .push(("From".into(), "kits.bin on the stick".into()));
            d.buttons = vec![Button::Load, Button::LoadToPad];
            return d;
        }
        let loc = r.loc.as_ref();
        if let Some(l) = loc.and_then(Loc::parent) {
            d.lines.push(("In".into(), l.display()));
        }
        match r.kind {
            Kind::Audio(_) => {
                if let Some((_, i)) = self.detail_info.as_ref().filter(|(l, _)| Some(l) == loc) {
                    d.lines.push(("Length".into(), duration(i.seconds())));
                    d.lines.push((
                        "Format".into(),
                        format!(
                            "{} · {}",
                            mpfree_library::audio::rate_label(i.rate),
                            if i.float {
                                format!("{}-bit float", i.bits)
                            } else {
                                format!("{}-bit", i.bits)
                            }
                        ),
                    ));
                    d.lines.push((
                        "Channels".into(),
                        match i.channels {
                            1 => "Mono".into(),
                            2 => "Stereo".into(),
                            n => format!("{n}"),
                        },
                    ));
                }
                if let Some((_, o)) = self
                    .detail_overview
                    .as_ref()
                    .filter(|(l, _)| Some(l) == loc)
                {
                    d.overview = o.clone();
                }
                d.buttons = vec![
                    Button::Load,
                    Button::LoadToPad,
                    Button::Audition,
                    Button::Favourite,
                    Button::Rename,
                    Button::Copy,
                    Button::Cut,
                    Button::Delete,
                ];
            }
            Kind::Project => {
                if let Some((_, m)) = self.detail_meta.as_ref().filter(|(l, _)| Some(l) == loc) {
                    d.lines
                        .push(("Tempo".into(), format!("{} BPM", trim_num(m.tempo))));
                    if m.saved > 0 {
                        d.lines.push(("Saved".into(), when(m.saved)));
                    }
                }
                if self.project.as_ref() == loc {
                    d.lines.push(("".into(), "Open now".into()));
                }
                d.buttons = vec![
                    Button::Open,
                    Button::Favourite,
                    Button::Rename,
                    Button::Copy,
                    Button::Delete,
                ];
            }
            Kind::Folder => {
                d.buttons = vec![
                    Button::Open,
                    Button::Favourite,
                    Button::Rename,
                    Button::Copy,
                    Button::Cut,
                    Button::Delete,
                ];
            }
            Kind::Kit => {
                d.lines.push(("Size".into(), r.detail.clone()));
                d.buttons = vec![
                    Button::Load,
                    Button::Favourite,
                    Button::Rename,
                    Button::Copy,
                    Button::Cut,
                    Button::Delete,
                ];
            }
            _ => {
                d.lines.push(("Size".into(), r.detail.clone()));
                d.buttons = vec![Button::Rename, Button::Copy, Button::Cut, Button::Delete];
            }
        }
        if !self.current_source().is_writable_place() {
            // Recent and Favourites point at things: no cut or rename there.
            d.buttons
                .retain(|b| !matches!(b, Button::Cut | Button::Rename));
        }
        d
    }
}

struct SourceKind(Source);

impl SourceKind {
    fn is_place(&self) -> bool {
        matches!(self.0, Source::Place(_))
    }
    fn is_writable_place(&self) -> bool {
        matches!(self.0, Source::Place(p) if p != Place::Trash)
    }
}

/// `"1.2 MB"`.
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    if bytes < 1000 {
        format!("{bytes} B")
    } else if b < 1e6 {
        format!("{:.0} KB", b / 1e3)
    } else if b < 1e9 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.2} GB", b / 1e9)
    }
}

fn trim_num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `"2026-11-11 21:04 UTC"`.
pub fn when(secs: u64) -> String {
    let (y, m, d, h, mi) = mpfree_library::project::civil(secs);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02} UTC")
}

fn entry_detail(e: &Entry) -> String {
    match e.kind {
        Kind::Folder => String::new(),
        Kind::Project => "Project".into(),
        _ => size(e.bytes),
    }
}

fn guess_kind(l: &Loc) -> Kind {
    if l.place == Place::Projects && l.names().len() == 1 {
        Kind::Project
    } else if l.name().contains('.') {
        Kind::of_file(l.name())
    } else {
        Kind::Folder
    }
}

pub fn kind_label(k: Kind) -> String {
    match k {
        Kind::Folder => "Folder".into(),
        Kind::Project => "Project".into(),
        Kind::Audio(f) => format!(
            "{} audio",
            match f {
                AudioFormat::Wav => "WAV",
                AudioFormat::Aiff => "AIFF",
                AudioFormat::Flac => "FLAC",
            }
        ),
        Kind::Kit => "Kit".into(),
        Kind::Pattern => "Pattern".into(),
        Kind::Other => "File".into(),
    }
}
