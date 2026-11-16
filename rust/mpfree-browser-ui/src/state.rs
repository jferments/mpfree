//! The browser's logic: where it is, what it lists, what is selected, what
//! a tap, the dial or a button does. No drawing here: [`Browser::screen`]
//! gives the widget what to show, and the widget's touches come back as
//! [`Input`]s. Every file operation goes to the library's worker thread
//! (or runs at once, for previews and tests), so nothing here blocks.

use mpfree_library::audio::duration;
use mpfree_library::{
    valid_name, AudioFormat, AudioInfo, Done, Entry, Job, Kind, Library, Loc, Meta, NewSample,
    Outcome, Place, Snapshot, TrashItem, Version, Worker,
};
use std::path::PathBuf;

use crate::export::{ExportChoice, ExportInput, ExportPanel, ExportPhase, ExportSetup};

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
pub const SOURCES: [Source; 10] = [
    Source::Place(Place::Projects),
    Source::Place(Place::Samples),
    Source::Place(Place::Kits),
    Source::Place(Place::Instruments),
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

/// A focused picker: the browser narrowed to one kind of thing, opened
/// from where that thing is used (Jay, 2026-11-11 23:16Z: BROWSE stays the
/// full file manager, and pickers open from the Synth page, the Sample page
/// or a pad, and Open project). Only Load (or Open), Audition, Favourite
/// and Cancel are offered; nothing is renamed, moved or deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// One engine's presets (its folder under Instruments, `"FM"`).
    /// Moving the selection tries each preset on the track
    /// ([`BrowseAction::TryPreset`]); Cancel puts the old sound back.
    Preset { engine: String },
    /// Sounds for the pad the picker was opened for: Load hands the file
    /// over with [`BrowseAction::LoadSample`].
    Sample,
    /// Projects. Opening one replaces the work in progress, so it asks
    /// first when that has changed since it was last saved.
    Project,
    /// The Pads tab's browser beside the pad bank (the owner's pick, 12 Nov
    /// 2026): sounds, as [`Pick::Sample`], but it stays open. Load hands
    /// the file over for the pad picked ([`BrowseAction::LoadSample`]),
    /// Load to pad for the next pad hit ([`BrowseAction::LoadToPad`]).
    Pads,
}

impl Pick {
    /// The places it offers, top to bottom (indices into [`SOURCES`]).
    pub fn sources(&self) -> Vec<usize> {
        let places: &[Place] = match self {
            Pick::Preset { .. } => &[Place::Instruments],
            Pick::Sample | Pick::Pads => &[Place::Samples, Place::Recordings],
            Pick::Project => &[Place::Projects],
        };
        SOURCES
            .iter()
            .enumerate()
            .filter(|(_, s)| match s {
                Source::Place(p) => places.contains(p),
                Source::Recent | Source::Favourites => true,
                Source::Factory => false,
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The folder it opens on and never goes above.
    pub fn root(&self) -> Loc {
        match self {
            Pick::Preset { engine } => Loc::top(Place::Instruments)
                .join(engine)
                .unwrap_or_else(|_| Loc::top(Place::Instruments)),
            Pick::Sample | Pick::Pads => Loc::top(Place::Samples),
            Pick::Project => Loc::top(Place::Projects),
        }
    }

    /// The title over its places.
    pub fn title(&self) -> String {
        match self {
            Pick::Preset { engine } if !engine.is_empty() => format!("{engine} presets"),
            Pick::Preset { .. } => "Presets".into(),
            Pick::Sample => "Load a sound".into(),
            Pick::Pads => "Sounds".into(),
            Pick::Project => "Open a project".into(),
        }
    }

    /// Whether a row belongs in it.
    fn shows(&self, r: &Row) -> bool {
        let Some(loc) = &r.loc else { return false };
        match self {
            Pick::Preset { engine } => {
                matches!(r.kind, Kind::Folder | Kind::Preset)
                    && loc.place == Place::Instruments
                    && (engine.is_empty() || preset_engine(loc) == *engine)
            }
            Pick::Sample | Pick::Pads => {
                matches!(r.kind, Kind::Folder | Kind::Audio(_))
                    && matches!(loc.place, Place::Samples | Place::Recordings)
            }
            Pick::Project => r.kind == Kind::Project,
        }
    }

    fn allows(&self, b: Button) -> bool {
        match self {
            // It stays open: nothing to cancel, and both loads.
            Pick::Pads => matches!(
                b,
                Button::Load
                    | Button::LoadToPad
                    | Button::Open
                    | Button::Audition
                    | Button::Favourite
            ),
            _ => matches!(
                b,
                Button::Load | Button::Open | Button::Audition | Button::Favourite | Button::Cancel
            ),
        }
    }

    /// Whether loading something closes it (every picker but the Pads
    /// tab's).
    fn closes_on_load(&self) -> bool {
        *self != Pick::Pads
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
    /// For an earlier save of a project (`loc` is the project): its id.
    pub version: Option<String>,
    /// When it last changed, seconds since 1970 (UTC), 0 if unknown.
    pub modified: u64,
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
    /// Closes a picker without loading.
    Cancel,
    /// A project's earlier saves, to open one.
    Versions,
    /// Saves the work as the current project (the first time, as a new
    /// one named for the time): the same as SHIFT + BROWSE.
    Save,
    /// Saves the work under a name typed on the keyboard.
    SaveAs,
    /// Starts a new, empty project (asks first over unsaved work).
    NewProject,
    /// Opens the Export panel: the song to WAV files and a MIDI file.
    Export,
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
            Button::Cancel => "Cancel",
            Button::Versions => "Earlier saves",
            Button::Save => "Save",
            Button::SaveAs => "Save as",
            Button::NewProject => "New project",
            Button::Export => "Export",
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
    /// The Search chip: type a name to look for in the whole library.
    Search,
    /// The order chip: by name or newest first.
    Sort,
    /// A touch on the Export panel.
    Export(ExportInput),
}

/// The order of a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    /// Folders first, then natural name order (what the library gives).
    Name,
    /// Folders first, then the newest change first.
    Newest,
}

impl Sort {
    pub fn label(self) -> &'static str {
        match self {
            Sort::Name => "A to Z",
            Sort::Newest => "Newest",
        }
    }
}

/// Seconds since 1970 (UTC) now, for a name made from the time.
fn now_utc() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
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
    /// An instrument preset from the library, for the synth thread's
    /// `mpfree-instruments` to read. `engine` is the folder under
    /// Instruments it is in (`FM`, `Poly` ...), empty if none.
    LoadPreset {
        loc: Loc,
        path: PathBuf,
        engine: String,
    },
    /// The preset picker moved to this preset: put it on the track to be
    /// played, keeping what was there to go back to.
    TryPreset {
        loc: Loc,
        path: PathBuf,
        engine: String,
    },
    /// The preset picker was cancelled after trying presets: put back the
    /// sound the track had when it opened.
    RevertPreset,
    /// A picker closed (`loaded`: by loading or opening something, else
    /// by Cancel). The app hides the overlay.
    PickerClosed {
        loaded: bool,
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
    /// The unsaved work was autosaved, into autosave's project here.
    Autosaved {
        loc: Loc,
    },
    /// Save (or Save as, once given a name) was pressed: call
    /// [`Browser::save`] with the engines' parts now.
    SaveRequested,
    /// New project was pressed (and, over unsaved work, agreed to): the
    /// app puts every engine back to a fresh, empty project. The browser
    /// has already forgotten the project it had ([`Browser::new_project`]).
    NewProject,
    /// Recent and Favourites changed: save this text as the "browser"
    /// owner's file in the store ([`Browser::lists_text`]).
    SaveLists(String),
    /// [`Browser::open_last`]'s project couldn't be opened (why); the
    /// work in progress stays as it was.
    LastNotOpened(String),
    /// Export was pressed: call [`Browser::open_export`] with what the
    /// panel needs to know about the song.
    ExportAsked,
    /// Start was pressed on the Export panel: make the plan and start the
    /// pass; say how far it has got with [`Browser::export_running`] and
    /// how it ended with [`Browser::export_over`].
    ExportStart(ExportChoice),
    /// Cancel was pressed while the export runs.
    ExportCancel,
}

/// What waits for a yes.
#[derive(Clone, Debug, PartialEq)]
pub enum Confirm {
    Delete(Loc),
    EmptyTrash,
    /// Open this project over unsaved changes.
    Open(Loc),
    /// Open this earlier save of a project over unsaved changes.
    OpenVersion(Loc, String),
    /// Start a new, empty project over unsaved changes.
    NewProject,
}

/// What the keyboard is typing.
#[derive(Clone, Debug, PartialEq)]
pub enum Purpose {
    Rename(Loc),
    NewFolder(Loc),
    /// A name for the project being saved.
    SaveAs,
    /// Text to look for.
    Search,
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
    /// The places column's title ("Library", or a picker's).
    pub title: String,
    /// The places shown, top to bottom (indices into [`SOURCES`]).
    pub sources: Vec<usize>,
    /// The filter chips shown (none in a picker).
    pub filters: Vec<Filter>,
    /// The picker this is, if it is one.
    pub pick: Option<Pick>,
    /// The list's order.
    pub sort: Sort,
    /// The Export panel, over everything, while it is open (hidden while
    /// a picker is open).
    pub export: Option<ExportPanel>,
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
            title: "Library".into(),
            sources: (0..SOURCES.len()).collect(),
            filters: FILTERS.to_vec(),
            pick: None,
            sort: Sort::Name,
            export: None,
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
/// The most search results shown.
pub const MAX_FOUND: usize = 200;

/// What the list holds besides a place's folder.
#[derive(Clone, Debug, PartialEq)]
enum Mode {
    /// The folder (or Recent, Favourites, Factory, Trash).
    Folder,
    /// A project's earlier saves, newest first.
    Versions(Loc, Vec<Version>),
    /// A search: the text, and what it found.
    Found(String, Vec<(Loc, Entry)>),
}

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
    /// The project last saved or opened, kept with Recent and Favourites:
    /// Prefs > At start can open it again ([`Browser::open_last`]).
    last: Option<Loc>,
    /// [`Browser::open_last`] is waiting for the library.
    opening_last: bool,
    /// When it was first saved.
    created: u64,
    actions: Vec<BrowseAction>,
    /// The picker open, if any, and where the full browser was before it.
    pick: Option<Pick>,
    before_pick: Option<(usize, Loc, Filter)>,
    /// A preset was tried since the preset picker opened.
    tried: bool,
    /// The work in progress changed since it was last saved or opened.
    unsaved: bool,
    /// An open of autosave's project is under way: still unsaved once it
    /// lands ([`Browser::open_landed`]).
    recovering: bool,
    mode: Mode,
    sort: Sort,
    /// The Export panel while it is open (it stays through a picker and
    /// the overlay closing, so a running export comes back with it).
    export: Option<ExportPanel>,
    /// What the last export asked for, ticked again next time.
    export_last: Option<ExportChoice>,
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
            last: None,
            opening_last: false,
            created: 0,
            actions: Vec::new(),
            pick: None,
            before_pick: None,
            tried: false,
            unsaved: false,
            recovering: false,
            mode: Mode::Folder,
            sort: Sort::Name,
            export: None,
            export_last: None,
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
            Err(e) => {
                // A preset picker for an engine with no presets saved yet:
                // its folder isn't there, which is not a fault.
                if let Some(Pick::Preset { engine }) = &self.pick {
                    if self.at_pick_root() && self.rows.is_empty() {
                        self.status = Some((
                            format!("No {engine} presets yet: save one from the Synth page"),
                            false,
                        ));
                        return;
                    }
                }
                if std::mem::take(&mut self.opening_last) {
                    self.actions.push(BrowseAction::LastNotOpened(e.clone()));
                }
                self.status = Some((e, true))
            }
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
            Ok(Outcome::Autosaved(loc)) => {
                self.actions.push(BrowseAction::Autosaved { loc });
                if self.folder == Loc::top(Place::Projects) {
                    self.refresh();
                }
            }
            Ok(Outcome::Saved(loc)) => {
                self.status = Some((format!("Saved {}", loc.name()), false));
                self.unsaved = false;
                self.project = Some(loc.clone());
                self.last = Some(loc.clone());
                self.remember(loc.clone());
                self.actions.push(BrowseAction::ProjectSaved { loc });
                self.refresh();
            }
            Ok(Outcome::Project(loc, snap)) if snap.meta.is_autosave() => {
                // Autosave's project opens as a new, unsaved one: SHIFT +
                // BROWSE keeps it as a project of the musician's (named for
                // the time), never as autosave's again.
                self.status = Some((
                    format!(
                        "Opened the work autosaved {}. SHIFT + BROWSE saves it as a new project",
                        when(snap.meta.saved)
                    ),
                    false,
                ));
                self.project = None;
                self.created = 0;
                self.unsaved = true;
                self.recovering = true;
                self.actions.push(BrowseAction::ProjectOpened { loc, snap });
                if self.pick.is_some() && !std::mem::take(&mut self.opening_last) {
                    self.close_pick(true);
                }
            }
            Ok(Outcome::Project(loc, snap)) => {
                self.status = Some((format!("Opened {}", loc.name()), false));
                self.project = Some(loc.clone());
                self.last = Some(loc.clone());
                self.created = snap.meta.created;
                self.unsaved = false;
                self.remember(loc.clone());
                self.actions.push(BrowseAction::ProjectOpened { loc, snap });
                // Opened at start (Prefs > At start), the Pads tab's
                // picker stays where it is.
                if self.pick.is_some() && !std::mem::take(&mut self.opening_last) {
                    self.close_pick(true);
                }
            }
            Ok(Outcome::Versions(loc, list)) => {
                if matches!(&self.mode, Mode::Versions(l, _) if *l == loc) {
                    self.mode = Mode::Versions(loc, list);
                    self.rebuild();
                }
            }
            Ok(Outcome::Version(loc, _id, snap)) => {
                self.status = Some((
                    format!(
                        "Opened {} as saved {}. SHIFT + BROWSE saves it as the newest",
                        loc.name(),
                        when(snap.meta.saved)
                    ),
                    false,
                ));
                self.project = Some(loc.clone());
                self.last = Some(loc.clone());
                self.created = snap.meta.created;
                // Not the newest save: saving it keeps what it replaces.
                self.unsaved = true;
                self.remember(loc.clone());
                self.actions.push(BrowseAction::ProjectOpened { loc, snap });
            }
            Ok(Outcome::Found(query, hits)) => {
                if matches!(&self.mode, Mode::Found(q, _) if *q == query) {
                    self.status = Some((
                        match hits.len() {
                            0 => format!("Nothing called \u{201c}{query}\u{201d}"),
                            MAX_FOUND => {
                                format!("The first {MAX_FOUND} found: type more to narrow it")
                            }
                            1 => "1 found".into(),
                            n => format!("{n} found"),
                        },
                        false,
                    ));
                    self.mode = Mode::Found(query, hits);
                    self.rebuild();
                }
            }
        }
    }

    /// Lists the current place again.
    pub fn refresh(&mut self) {
        match &self.mode {
            Mode::Versions(l, _) => return self.send(Job::Versions(l.clone())),
            Mode::Found(q, _) => return self.send(Job::Search(q.clone(), MAX_FOUND)),
            Mode::Folder => {}
        }
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
        let mut rows: Vec<Row> = match (&self.mode, self.current()) {
            (Mode::Versions(project, list), _) => list
                .iter()
                .map(|v| Row {
                    name: if v.saved > 0 {
                        format!("Saved {}", when(v.saved))
                    } else {
                        v.id.clone()
                    },
                    kind: Kind::Project,
                    detail: String::new(),
                    favourite: false,
                    loc: Some(project.clone()),
                    trash_id: None,
                    factory: None,
                    version: Some(v.id.clone()),
                    modified: v.saved,
                })
                .collect(),
            (Mode::Found(_, hits), _) => hits
                .iter()
                .map(|(l, e)| Row {
                    name: e.name.clone(),
                    kind: e.kind,
                    detail: l.parent().map(|p| p.display()).unwrap_or_default(),
                    favourite: fav(l),
                    loc: Some(l.clone()),
                    trash_id: None,
                    factory: None,
                    version: None,
                    modified: e.modified,
                })
                .collect(),
            (Mode::Folder, src) => match src {
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
                        version: None,
                        modified: 0,
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
                            version: None,
                            modified: e.modified,
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
                            version: None,
                            modified: 0,
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
                        version: None,
                        modified: 0,
                    })
                    .collect(),
            },
        };
        if self.sort == Sort::Newest && !matches!(self.mode, Mode::Versions(..)) {
            // Stable: equal times keep the name order.
            rows.sort_by(|a, b| {
                b.kind
                    .is_folder()
                    .cmp(&a.kind.is_folder())
                    .then_with(|| b.modified.cmp(&a.modified))
            });
        }
        rows.retain(|r| self.filter.shows(r.kind));
        if let Some(p) = &self.pick {
            rows.retain(|r| p.shows(r));
        }
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
        if row.version.is_some() {
            return;
        }
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
            Kind::Preset if by_hand && matches!(self.pick, Some(Pick::Preset { .. })) => {
                if let Ok(path) = self.library().file_path(&loc) {
                    self.tried = true;
                    self.actions.push(BrowseAction::TryPreset {
                        engine: preset_engine(&loc),
                        loc,
                        path,
                    });
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

    /// Recent and Favourites, and the last project, as saved by
    /// [`Browser::lists_text`].
    pub fn load_lists(&mut self, text: &str) {
        self.recent.clear();
        self.favourites.clear();
        self.last = None;
        for line in text.lines() {
            if let Some(l) = line.strip_prefix("project = ").and_then(Loc::from_line) {
                self.last = Some(l);
            } else if let Some(l) = line.strip_prefix("recent = ").and_then(Loc::from_line) {
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
    /// or `favourite = ...` line each, after the last project's
    /// (`project = Projects/Night drive`).
    pub fn lists_text(&self) -> String {
        let mut s = String::new();
        if let Some(l) = &self.last {
            s.push_str(&format!("project = {}\n", l.to_line()));
        }
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
        let last = self.last.take_if(|p| l.contains(p)).is_some();
        if last || self.recent.len() + self.favourites.len() != before {
            self.actions
                .push(BrowseAction::SaveLists(self.lists_text()));
        }
    }

    // ------------------------------------------------------------ moving around

    pub fn go_source(&mut self, i: usize) {
        if i >= SOURCES.len()
            || self
                .pick
                .as_ref()
                .is_some_and(|p| !p.sources().contains(&i))
        {
            return;
        }
        self.source = i;
        self.mode = Mode::Folder;
        if let Source::Place(p) = SOURCES[i] {
            self.folder = match &self.pick {
                Some(pk) if pk.root().place == p => pk.root(),
                _ => Loc::top(p),
            };
        }
        self.top = 0;
        self.selected = None;
        self.entries.clear();
        self.rows.clear();
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
        self.mode = Mode::Folder;
        self.folder = folder;
        self.top = 0;
        self.selected = None;
        self.entries.clear();
        // Not the last folder's rows under this one's name, even if
        // listing it fails.
        self.rows.clear();
        self.refresh();
    }

    /// Up one folder (the - button, Back).
    pub fn back(&mut self) {
        if self.export_shown().is_some() {
            return self.export_input(ExportInput::Close);
        }
        if let Mode::Versions(l, _) = &self.mode {
            self.select_name = Some(l.name().to_string());
        }
        if self.mode != Mode::Folder {
            self.mode = Mode::Folder;
            self.top = 0;
            self.status = None;
            return self.refresh();
        }
        if self.at_pick_root() {
            return;
        }
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
        if self.pick.is_none() {
            if let Some(p) = self.export.as_mut() {
                // The panel's bar count, while it is ready.
                if p.phase == ExportPhase::Ready {
                    p.step_bars(delta);
                }
                return;
            }
        }
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
        let shown = self.shown_sources();
        let at = shown.iter().position(|s| *s == self.source).unwrap_or(0) as i32;
        let i = (at + delta).rem_euclid(shown.len() as i32) as usize;
        self.go_source(shown[i]);
    }

    fn at_pick_root(&self) -> bool {
        self.pick.as_ref().is_some_and(|p| p.root() == self.folder)
    }

    fn shown_sources(&self) -> Vec<usize> {
        match &self.pick {
            Some(p) => p.sources(),
            None => (0..SOURCES.len()).collect(),
        }
    }

    // ------------------------------------------------------------ pickers

    /// Opens the browser as a focused picker (see [`Pick`]). The full
    /// browser's place and folder come back when it closes.
    pub fn open_picker(&mut self, p: Pick) {
        if self.pick.is_none() {
            self.before_pick = Some((self.source, self.folder.clone(), self.filter));
        }
        let root = p.root();
        self.pick = Some(p);
        self.tried = false;
        self.filter = Filter::All;
        self.modal = Modal::None;
        self.status = None;
        self.go_folder(root);
    }

    /// The picker open, if any.
    pub fn picking(&self) -> Option<&Pick> {
        self.pick.as_ref()
    }

    /// Cancel (or BROWSE, or the app closing it): nothing is loaded, and a
    /// tried preset is taken back.
    pub fn cancel_picker(&mut self) {
        if self.pick.is_some() {
            if self.tried {
                self.actions.push(BrowseAction::RevertPreset);
            }
            self.close_pick(false);
        }
    }

    fn close_pick(&mut self, loaded: bool) {
        self.pick = None;
        self.tried = false;
        self.modal = Modal::None;
        self.actions.push(BrowseAction::PickerClosed { loaded });
        if let Some((source, folder, filter)) = self.before_pick.take() {
            self.filter = filter;
            if matches!(SOURCES[source], Source::Place(_)) {
                self.go_folder(folder);
            } else {
                self.go_source(source);
            }
        }
    }

    /// The app says whether the work in progress has changed since it was
    /// last saved or opened (opening a project over it then asks first).
    pub fn set_unsaved(&mut self, unsaved: bool) {
        self.unsaved = unsaved;
    }

    /// The app says an open has landed (the engines hold the project):
    /// nothing is unsaved then, unless it was autosave's project, whose
    /// work is still to be saved as a project of the musician's.
    pub fn open_landed(&mut self) {
        self.unsaved = std::mem::take(&mut self.recovering);
    }

    /// The dial's push: open a folder or project, load anything else, or
    /// answer the question on screen.
    pub fn push(&mut self) {
        if let Some(p) = self.export_shown() {
            let i = match p.phase {
                ExportPhase::Ready => ExportInput::Start,
                ExportPhase::Running { .. } => return,
                ExportPhase::Done { .. } | ExportPhase::Stopped(_) => ExportInput::Close,
            };
            return self.export_input(i);
        }
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
        // The Export panel, over everything, takes only its own touches.
        match (self.export_shown().is_some(), &i) {
            (true, Input::Export(e)) => return self.export_input(*e),
            (true, _) | (false, Input::Export(_)) => return,
            (false, _) => {}
        }
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
            Input::Search => {
                self.modal = Modal::Keyboard {
                    purpose: Purpose::Search,
                    title: "Search the library for".into(),
                    text: match &self.mode {
                        Mode::Found(q, _) => q.clone(),
                        _ => String::new(),
                    },
                    shift: false,
                };
            }
            Input::Sort => {
                self.sort = match self.sort {
                    Sort::Name => Sort::Newest,
                    Sort::Newest => Sort::Name,
                };
                self.rebuild();
            }
            Input::Export(_) => {}
        }
    }

    /// A button that works on the selected row, with none selected: say so
    /// in place rather than do nothing (Jay, 13 Nov).
    fn pick_first(&mut self, b: Button) {
        self.status = Some((
            format!("Pick something in the list first, then {}", b.label()),
            false,
        ));
    }

    fn button(&mut self, b: Button) {
        if self.pick.as_ref().is_some_and(|p| !p.allows(b)) {
            return;
        }
        let row = self.selected_row().cloned();
        let loc = row.as_ref().and_then(|r| r.loc.clone());
        // An earlier save's row stands for the project's folder: only
        // opening it means anything (never Delete, Rename ... on the project).
        if let Some(id) = row.as_ref().and_then(|r| r.version.clone()) {
            if let (Button::Open | Button::Load, Some(loc)) = (b, loc) {
                if self.unsaved {
                    self.modal = Modal::Confirm {
                        text: format!(
                            "Unsaved changes. Open the save of \u{201c}{}\u{201d} from {} anyway?",
                            loc.name(),
                            row.as_ref().map_or(String::new(), |r| when(r.modified))
                        ),
                        what: Confirm::OpenVersion(loc, id),
                    };
                } else {
                    self.send(Job::OpenVersion(loc, id));
                }
            }
            return;
        }
        match b {
            Button::Cancel => self.cancel_picker(),
            Button::Save => self.actions.push(BrowseAction::SaveRequested),
            Button::Export => self.actions.push(BrowseAction::ExportAsked),
            Button::SaveAs => self.save_as(now_utc()),
            Button::NewProject => {
                if self.unsaved {
                    self.modal = Modal::Confirm {
                        text: "Unsaved changes. Start a new, empty project anyway?".into(),
                        what: Confirm::NewProject,
                    };
                } else {
                    self.start_new_project();
                }
            }
            Button::Versions => {
                let Some(r) = row else {
                    return self.pick_first(b);
                };
                let Some(loc) = loc.filter(|_| r.kind == Kind::Project) else {
                    return;
                };
                self.mode = Mode::Versions(loc.clone(), Vec::new());
                self.top = 0;
                self.selected = None;
                self.rows.clear();
                self.send(Job::Versions(loc));
            }
            Button::Load | Button::LoadToPad => {
                let to_pad = b == Button::LoadToPad;
                let Some(r) = row else {
                    return self.pick_first(b);
                };
                if let Some(index) = r.factory {
                    self.actions
                        .push(BrowseAction::LoadFactoryKit { index, to_pad });
                    self.status = Some((format!("Loading {}", r.name), false));
                    return;
                }
                let Some(loc) = loc else { return };
                match r.kind {
                    Kind::Project | Kind::Folder => self.button(Button::Open),
                    Kind::Audio(_) | Kind::Kit | Kind::Preset => {
                        let Ok(path) = self.library().file_path(&loc) else {
                            // Which button asked, so Load to pad after Load
                            // says something new.
                            self.status =
                                Some((format!("{}: {} is not there", b.label(), r.name), true));
                            return;
                        };
                        self.actions.push(match (r.kind, to_pad) {
                            (Kind::Kit, _) => BrowseAction::LoadKit {
                                loc: loc.clone(),
                                path,
                            },
                            (Kind::Preset, _) => BrowseAction::LoadPreset {
                                loc: loc.clone(),
                                path,
                                engine: preset_engine(&loc),
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
                        if self.pick.as_ref().is_some_and(Pick::closes_on_load) {
                            self.close_pick(true);
                        }
                    }
                    _ => self.status = Some((format!("{} is not a sound", r.name), true)),
                }
            }
            Button::Open => {
                let Some(r) = row else {
                    return self.pick_first(b);
                };
                let Some(loc) = loc else { return };
                match r.kind {
                    Kind::Folder => self.go_folder(loc),
                    Kind::Project if self.unsaved => {
                        self.modal = Modal::Confirm {
                            text: format!(
                                "Unsaved changes. Open \u{201c}{}\u{201d} anyway?",
                                r.name
                            ),
                            what: Confirm::Open(loc),
                        };
                    }
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
                if !self.current_source().is_writable_place() || self.mode != Mode::Folder {
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
                let Some(r) = row else {
                    return self.pick_first(b);
                };
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
                Confirm::Open(loc) => self.send(Job::OpenProject(loc)),
                Confirm::OpenVersion(loc, id) => self.send(Job::OpenVersion(loc, id)),
                Confirm::NewProject => self.start_new_project(),
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
                if *purpose == Purpose::Search {
                    if name.is_empty() {
                        return;
                    }
                    self.modal = Modal::None;
                    self.mode = Mode::Found(name.clone(), Vec::new());
                    self.top = 0;
                    self.selected = None;
                    self.rows.clear();
                    self.status = Some((format!("Searching for \u{201c}{name}\u{201d}"), false));
                    self.send(Job::Search(name, MAX_FOUND));
                    return;
                }
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
                    Purpose::Search => {}
                }
            }
        }
    }

    // ------------------------------------------------------------ projects

    /// The project being worked on, if it has been saved or opened.
    pub fn project(&self) -> Option<&Loc> {
        self.project.as_ref()
    }

    /// The project last saved or opened, in this run or an earlier one.
    pub fn last_project(&self) -> Option<&Loc> {
        self.last.as_ref()
    }

    /// Opens the last project (Prefs > At start), as the Projects list
    /// would, without closing the Pads tab's picker. False when there is
    /// none. It comes as [`BrowseAction::ProjectOpened`], or
    /// [`BrowseAction::LastNotOpened`] if the library can't open it.
    pub fn open_last(&mut self) -> bool {
        let Some(loc) = self.last.clone() else {
            return false;
        };
        self.opening_last = true;
        self.send(Job::OpenProject(loc));
        true
    }

    /// SHIFT + BROWSE: saves `snap` as the current project, or, the first
    /// time, as a new project named for the time (no typing needed;
    /// Rename changes it later). `now` is seconds since 1970 (UTC).
    pub fn save(&mut self, snap: Snapshot, now: u64) {
        self.save_with(snap, Vec::new(), now);
    }

    /// [`Browser::save`] with the sounds the parts name (the sampler's
    /// recordings and kits), written into the project first where it
    /// doesn't have them yet.
    pub fn save_with(&mut self, mut snap: Snapshot, samples: Vec<NewSample>, now: u64) {
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
            samples,
        });
    }

    /// Autosave (every 2 minutes while there is unsaved work, the app's
    /// clock): `snap` and its sounds go to autosave's own project
    /// (`mpfree_library::Library::autosave_project`), keeping no earlier
    /// saves. The current project, the unsaved mark and the status line
    /// stay as they are; a failed autosave says so on the status line.
    pub fn autosave_with(&mut self, mut snap: Snapshot, samples: Vec<NewSample>, now: u64) {
        snap.meta.name = match &self.project {
            Some(l) => format!("Autosave of {}", l.name()),
            None => "Autosave of a new project".to_string(),
        };
        if snap.meta.created == 0 {
            snap.meta.created = now;
        }
        snap.meta.saved = now;
        self.send(Job::Autosave { snap, samples });
    }

    /// At the start, before anything autosaves: autosave's project, if it
    /// holds work newer than the last project's save (the last run ended,
    /// by a crash or the power switch, with unsaved work), is set aside as
    /// a project of its own (`mpfree_library::Library::keep_autosave`), so
    /// that this run's autosave can't write over it. Its new place, or why
    /// it stayed. A rename in Projects, on the caller's thread.
    pub fn keep_newer_autosave(&mut self) -> Option<Result<Loc, String>> {
        let (loc, saved) = self.newer_autosave()?;
        let kept = self
            .library()
            .keep_autosave(&loc, saved)
            .map_err(|e| e.to_string());
        if kept.is_ok() && self.folder == Loc::top(Place::Projects) {
            self.refresh();
        }
        Some(kept)
    }

    /// Autosave's project and when it was last written, if there is one
    /// and it holds work newer than the last project's save (the work a
    /// crash or the power switch would otherwise lose). Reads two
    /// project.txt files.
    fn newer_autosave(&self) -> Option<(Loc, u64)> {
        let (loc, meta) = self.library().autosave_meta()?;
        let last = self
            .last
            .as_ref()
            .and_then(|l| self.library().project_meta(l).ok())
            .map_or(0, |m| m.saved);
        (meta.saved > last).then_some((loc, meta.saved))
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

    /// New project, agreed: forget the project and ask the app for a
    /// fresh one ([`BrowseAction::NewProject`]).
    fn start_new_project(&mut self) {
        self.new_project();
        self.unsaved = false;
        self.status = Some((
            "New project: nothing saved yet (Save names it for the time)".into(),
            false,
        ));
        self.actions.push(BrowseAction::NewProject);
    }

    /// The status line now, and whether it is an error: what a save that
    /// didn't land said, say.
    pub fn status_line(&self) -> Option<(&str, bool)> {
        self.status.as_ref().map(|(t, bad)| (t.as_str(), *bad))
    }

    /// Whether the work has changed since it was last saved or opened.
    pub fn unsaved(&self) -> bool {
        self.unsaved
    }

    /// Whether the worker has jobs not answered yet (a save, say): the app
    /// keeps stepping the browser while it doesn't show, so the answer
    /// lands.
    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    // ------------------------------------------------------------ export

    /// Opens the Export panel (after [`BrowseAction::ExportAsked`]), with
    /// what it needs to know about the song. A panel already open (an
    /// export running, say) stays as it is.
    pub fn open_export(&mut self, setup: ExportSetup) {
        if self.export.as_ref().is_some_and(ExportPanel::running) {
            return;
        }
        self.export = Some(ExportPanel::new(setup, self.export_last));
    }

    /// The panel, if it is open.
    pub fn export_panel(&self) -> Option<&ExportPanel> {
        self.export.as_ref()
    }

    /// The panel as the screen shows it (not while a picker is open).
    fn export_shown(&self) -> Option<&ExportPanel> {
        self.export.as_ref().filter(|_| self.pick.is_none())
    }

    /// Whether an export runs (Start was pressed, and it isn't over).
    pub fn exporting(&self) -> bool {
        self.export.as_ref().is_some_and(ExportPanel::running)
    }

    /// How far the export has got: "1:12 of 3:50", and the part done when
    /// the length is known.
    pub fn export_running(&mut self, words: String, part: Option<f32>) {
        if let Some(p) = self.export.as_mut().filter(|p| p.running()) {
            p.phase = ExportPhase::Running {
                words,
                part: part.map(|f| f.clamp(0.0, 1.0)),
            };
        }
    }

    /// The export is over: `folder` is where its files are when it made
    /// them (Show in BROWSE goes there), None when it stopped or failed;
    /// `text` says what happened. The status line says it too.
    pub fn export_over(&mut self, text: String, folder: Option<Loc>) {
        let ok = folder.is_some();
        self.status = Some((text.clone(), !ok));
        let phase = match folder {
            Some(f) => ExportPhase::Done {
                text,
                folder: Some(f),
            },
            None => ExportPhase::Stopped(text),
        };
        match self.export.as_mut() {
            Some(p) => p.phase = phase,
            None => {
                // Closed in the meantime: the status line has it.
            }
        }
    }

    fn export_input(&mut self, i: ExportInput) {
        let Some(p) = self.export.as_mut() else {
            return;
        };
        match i {
            ExportInput::Start => {
                if p.can_start() {
                    let c = p.choice();
                    self.export_last = Some(c);
                    p.phase = ExportPhase::Running {
                        words: "Starting".into(),
                        part: None,
                    };
                    self.actions.push(BrowseAction::ExportStart(c));
                }
            }
            ExportInput::Cancel if p.running() => {
                self.actions.push(BrowseAction::ExportCancel);
                p.phase = ExportPhase::Running {
                    words: "Stopping".into(),
                    part: None,
                };
            }
            ExportInput::Cancel | ExportInput::Close => {
                if !p.running() {
                    self.export = None;
                }
            }
            ExportInput::Show => {
                if let ExportPhase::Done {
                    folder: Some(f), ..
                } = &p.phase
                {
                    let f = f.clone();
                    self.export = None;
                    self.mode = Mode::Folder;
                    self.go_folder(f);
                }
            }
            other => p.pick(other),
        }
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
            path: match (&self.mode, src) {
                (Mode::Versions(l, _), _) => format!("{} / Earlier saves", l.name()),
                (Mode::Found(q, _), _) => format!("Search: \u{201c}{q}\u{201d}"),
                (_, Source::Place(Place::Trash)) => "Trash".into(),
                (_, Source::Place(_)) => self.folder.display(),
                (_, s) => s.label().into(),
            },
            can_go_back: self.mode != Mode::Folder
                || (src != Source::Place(Place::Trash)
                    && self.current_source().is_place()
                    && !self.folder.is_top()
                    && !self.at_pick_root()),
            rows: self.rows.clone(),
            selected: self.selected,
            top: self.top,
            filter: self.filter,
            detail: self.detail(),
            list_buttons: match src {
                // The Pads tab's stays open: nothing to cancel.
                _ if self.pick == Some(Pick::Pads) => Vec::new(),
                _ if self.pick.is_some() => vec![Button::Cancel],
                // Nothing to make or paste in a search or a project's saves.
                _ if self.mode != Mode::Folder => Vec::new(),
                Source::Place(Place::Trash) => vec![Button::EmptyTrash],
                Source::Place(p) => {
                    // Projects are made by saving, so no New folder there:
                    // New project, Save and Save as instead.
                    let mut v = if p == Place::Projects {
                        vec![
                            Button::NewProject,
                            Button::SaveAs,
                            Button::Save,
                            Button::Export,
                        ]
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
            title: self
                .pick
                .as_ref()
                .map_or_else(|| "Library".into(), Pick::title),
            sources: self.shown_sources(),
            filters: if self.pick.is_some() {
                Vec::new()
            } else {
                FILTERS.to_vec()
            },
            pick: self.pick.clone(),
            sort: self.sort,
            export: self.export.clone().filter(|_| self.pick.is_none()),
        }
    }

    fn detail(&self) -> Detail {
        let Some(r) = self.selected_row() else {
            match &self.mode {
                Mode::Versions(l, _) => {
                    return Detail {
                        title: l.name().into(),
                        kind: "No earlier saves yet: each save keeps the one before".into(),
                        ..Detail::default()
                    }
                }
                Mode::Found(q, _) => {
                    return Detail {
                        title: format!("\u{201c}{q}\u{201d}"),
                        kind: if self.pending > 0 {
                            "Searching…".into()
                        } else {
                            "Nothing found".into()
                        },
                        ..Detail::default()
                    }
                }
                Mode::Folder => {}
            }
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
        if let (Some(_), Some(l)) = (&r.version, &r.loc) {
            d.kind = "Earlier save".into();
            d.lines.push(("Project".into(), l.name().into()));
            d.lines
                .push(("Kept".into(), format!("The last {KEEP_VERSIONS} saves")));
            d.buttons = vec![Button::Open];
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
                    Button::Versions,
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
            Kind::Preset => {
                if let Some(e) = loc.map(preset_engine).filter(|e| !e.is_empty()) {
                    d.lines.push(("Engine".into(), e));
                }
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
        if !self.current_source().is_writable_place() || self.mode != Mode::Folder {
            // Recent, Favourites and search results point at things: no cut or rename there.
            d.buttons
                .retain(|b| !matches!(b, Button::Cut | Button::Rename));
        }
        if let Some(p) = &self.pick {
            d.buttons.retain(|b| p.allows(*b));
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
        Kind::Preset => "Preset".into(),
        Kind::Pattern => "Pattern".into(),
        Kind::Other => "File".into(),
    }
}

/// The engine a preset belongs to: the first folder under Instruments
/// (`Instruments/FM/Bells/Glass.preset` is FM's), or empty.
fn preset_engine(loc: &Loc) -> String {
    match (loc.place, loc.names()) {
        (Place::Instruments, [engine, _, ..]) => engine.clone(),
        _ => String::new(),
    }
}
