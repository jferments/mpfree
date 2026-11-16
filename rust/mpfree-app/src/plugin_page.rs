//! The Plugins place in BROWSE (Jay, 15 Nov 23:36Z, "build it as drawn";
//! layout spec section 3): what its list and selected column show, from the
//! worker's listing ([`crate::plugin_store`]), and what each touch, dial
//! turn and button does. The browser draws it (`mpfree_browser_ui::plugins`).
//!
//! - **Catalog:** the Open MPC catalog's plugins that run on this MPC, each
//!   tagged Installed, Update or Zip on the stick when it is.
//! - **Installed:** the MPC plugin folders in `files/plugins/`, however
//!   they got there.
//! - **On the stick:** the release zips copied straight into
//!   `files/plugins/`, matched to the catalog by their SHA-256.
//! - Favourites come first in every order; Most used is the catalog's
//!   download count.

use crate::plugin_store::{Ask, InstalledPlugin, Listing, News, Worker, ZipFile};
use crate::plugins::Plugins;
use mpfree_browser_ui::{
    KindFilter, PluginButton, PluginDetail, PluginInput, PluginRow, PluginsScreen, PluginsView,
    Tone,
};
use mpfree_mpcstore::catalog::{Catalog, Fit, Plugin};
use mpfree_ui::Image;
use std::path::PathBuf;
use std::sync::Arc;

/// Pictures kept drawn, so stepping back to one is at once.
const PICTURES_KEPT: usize = 6;

/// The list's orders, stepped by the order chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    MostUsed,
    Newest,
    AToZ,
}

impl SortBy {
    pub fn label(self) -> &'static str {
        match self {
            SortBy::MostUsed => "Most used",
            SortBy::Newest => "Newest",
            SortBy::AToZ => "A to Z",
        }
    }

    fn next(self) -> SortBy {
        match self {
            SortBy::MostUsed => SortBy::Newest,
            SortBy::Newest => SortBy::AToZ,
            SortBy::AToZ => SortBy::MostUsed,
        }
    }
}

/// What a row stands for: an index into the catalog or the listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Catalog(usize),
    Installed(usize),
    Zip(usize),
}

/// The ask running, as the worker names it.
#[derive(Clone, Debug)]
struct Busy {
    key: String,
    /// "Downloading Surge", "Installing Surge".
    what: String,
    done: u64,
    total: u64,
}

/// The selected item's picture.
#[derive(Clone, Debug, Default)]
struct Pic {
    key: String,
    index: usize,
    count: usize,
    image: Option<Arc<Image>>,
    why: String,
    waiting: bool,
}

pub struct PluginPage {
    dir: Option<PathBuf>,
    worker: Option<Worker>,
    listing: Listing,
    listed: bool,
    /// The catalog was asked for once this run (Update list asks again).
    fetched: bool,
    fetching: bool,
    view: PluginsView,
    filter: KindFilter,
    sort: SortBy,
    search: Option<String>,
    items: Vec<Item>,
    selected: Option<usize>,
    /// The selected item's identity, which outlives a new listing.
    sel_id: Option<String>,
    top: usize,
    rows_room: usize,
    favourites: Vec<String>,
    /// Remove or Delete zip asked once, for this item.
    confirm: Option<(PluginButton, String)>,
    busy: Option<Busy>,
    said: Option<(String, Tone)>,
    picture: Pic,
    /// The last few pictures drawn, by (key, index), with their count.
    cache: Vec<((String, usize), Arc<Image>, usize)>,
    changed: bool,
    /// The sample screen's page: no worker, no network.
    offline: bool,
    /// Whether the catalog came over the network this run (None: not
    /// tried yet).
    reached: Option<bool>,
    /// The catalog fetch running was asked for with Update list.
    by_hand: bool,
}

impl Default for PluginPage {
    fn default() -> Self {
        PluginPage::new()
    }
}

/// "15 Nov 22:00 UTC" from the catalog's ISO 8601 time.
fn when(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let num = |r: std::ops::Range<usize>| iso.get(r).and_then(|t| t.parse::<usize>().ok());
    match (num(5..7), num(8..10), iso.get(11..16)) {
        (Some(m @ 1..=12), Some(d), Some(hm)) => format!("{d} {} {hm} UTC", MONTHS[m - 1]),
        (Some(m @ 1..=12), Some(d), None) => format!("{d} {}", MONTHS[m - 1]),
        _ => iso.to_string(),
    }
}

fn human(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// "GitHub" for github.com and its file hosts, else the address's host.
fn host_name(url: &str) -> String {
    let host = url
        .strip_prefix("https://")
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("");
    if host == "github.com" || host.ends_with(".githubusercontent.com") {
        "GitHub".into()
    } else {
        host.to_string()
    }
}

fn kind_label(instrument: bool) -> &'static str {
    if instrument {
        "Instrument"
    } else {
        "Effect"
    }
}

/// Newer by its dotted numbers (1.10 is newer than 1.9).
fn newer(a: &str, b: &str) -> bool {
    let parts = |s: &str| -> Vec<u64> {
        s.trim_start_matches(['v', 'V'])
            .split(['.', '-'])
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    !a.is_empty() && !b.is_empty() && parts(a) > parts(b)
}

impl PluginPage {
    pub fn new() -> PluginPage {
        PluginPage {
            dir: None,
            worker: None,
            listing: Listing::default(),
            listed: false,
            fetched: false,
            fetching: false,
            view: PluginsView::Catalog,
            filter: KindFilter::All,
            sort: SortBy::MostUsed,
            search: None,
            items: Vec::new(),
            selected: None,
            sel_id: None,
            top: 0,
            rows_room: 8,
            favourites: Vec::new(),
            confirm: None,
            busy: None,
            said: None,
            picture: Pic::default(),
            cache: Vec::new(),
            changed: false,
            offline: false,
            reached: None,
            by_hand: false,
        }
    }

    /// The manual's picture of the place: [`SAMPLE_CATALOG`] with two
    /// plugins installed and one zip on the stick, `picture` as the
    /// selected plugin's, and no worker (nothing is fetched or written).
    pub fn sample(picture: Option<Arc<Image>>) -> PluginPage {
        let mut p = PluginPage::new();
        p.offline = true;
        p.fetched = true;
        let installed = |id: &str, name: &str, maker: &str, instrument, version: &str| {
            let folder = format!("{maker} - VST - {name}");
            InstalledPlugin {
                path: PathBuf::from("files/plugins").join(&folder),
                folder,
                name: name.into(),
                maker: maker.into(),
                instrument,
                version: version.into(),
                id: id.into(),
                by_mpfree: true,
            }
        };
        let listing = Listing {
            catalog: mpfree_mpcstore::catalog::parse(SAMPLE_CATALOG)
                .ok()
                .map(Arc::new),
            installed: vec![
                installed(
                    "example.drift-pad",
                    "Drift Pad",
                    "Example Audio",
                    true,
                    "2.0.1",
                ),
                installed("shed.mono-bass", "Mono Bass", "Synth Shed", true, "1.2.0"),
            ],
            zips: vec![ZipFile {
                path: PathBuf::from("files/plugins/Plate-Reverb-armv7.zip"),
                file: "Plate-Reverb-armv7.zip".into(),
                bytes: 2_400_000,
                id: "shed.plate".into(),
                version: "0.9.2".into(),
                name: "Plate Reverb".into(),
                maker: "Synth Shed".into(),
                instrument: Some(false),
                folders: vec!["Synth Shed - VST - Plate Reverb".into()],
                ..ZipFile::default()
            }],
            can_download: true,
            no_folder: false,
            favourites: vec!["example.drift-pad".into()],
        };
        p.take(News::Listing(Box::new(listing)));
        // Mono Bass: installed, with an update out; its own screen first,
        // then the catalog's picture.
        p.select(2);
        if let Some(img) = picture {
            p.cache
                .push(((p.picture.key.clone(), 0), Arc::clone(&img), 2));
            p.picture.image = Some(img);
        }
        p.picture.count = 2;
        p.picture.waiting = false;
        p
    }

    /// A page that never fetches the catalog by itself (tests: no network).
    #[cfg(test)]
    pub fn without_fetching() -> PluginPage {
        PluginPage {
            fetched: true,
            ..PluginPage::new()
        }
    }

    /// The plugins folder (`files/plugins/` in the data folder).
    pub fn set_dir(&mut self, dir: Option<PathBuf>) {
        self.dir = dir;
    }

    fn catalog(&self) -> Option<&Catalog> {
        self.listing.catalog.as_deref()
    }

    fn cat(&self, id: &str) -> Option<&Plugin> {
        if id.is_empty() {
            return None;
        }
        self.catalog().and_then(|c| c.get(id))
    }

    fn installed_id(&self, id: &str) -> Option<&InstalledPlugin> {
        (!id.is_empty())
            .then(|| self.listing.installed.iter().find(|i| i.id == id))
            .flatten()
    }

    fn zip_id(&self, id: &str) -> Option<&ZipFile> {
        (!id.is_empty())
            .then(|| {
                self.listing
                    .zips
                    .iter()
                    .find(|z| z.id == id && z.problem.is_empty())
            })
            .flatten()
    }

    /// The catalog's id, else the folder or the zip's name: what the
    /// favourites and the pictures know an item by.
    fn identity(&self, it: Item) -> String {
        match it {
            Item::Catalog(i) => self
                .catalog()
                .map_or(String::new(), |c| c.plugins[i].id.clone()),
            Item::Installed(i) => {
                let p = &self.listing.installed[i];
                if p.id.is_empty() {
                    format!("folder:{}", p.folder)
                } else {
                    p.id.clone()
                }
            }
            Item::Zip(i) => {
                let z = &self.listing.zips[i];
                if z.id.is_empty() {
                    format!("zip:{}", z.file)
                } else {
                    z.id.clone()
                }
            }
        }
    }

    fn name_of(&self, it: Item) -> (String, String, Option<bool>) {
        match it {
            Item::Catalog(i) => {
                let p = &self.catalog().expect("catalog rows").plugins[i];
                (
                    p.name.clone(),
                    p.author.clone(),
                    Some(p.kind == "instrument"),
                )
            }
            Item::Installed(i) => {
                let p = &self.listing.installed[i];
                (p.name.clone(), p.maker.clone(), Some(p.instrument))
            }
            Item::Zip(i) => {
                let z = &self.listing.zips[i];
                (z.name.clone(), z.maker.clone(), z.instrument)
            }
        }
    }

    /// The release the catalog would install for an id, if newer than
    /// `have`.
    fn update_for(&self, id: &str, have: &str) -> Option<String> {
        let r = self.cat(id)?.release.as_ref()?;
        newer(&r.version, have).then(|| r.version.clone())
    }

    fn selected_item(&self) -> Option<Item> {
        self.selected.and_then(|k| self.items.get(k).copied())
    }

    // -------------------------------------------------------------- list

    fn rebuild(&mut self) {
        // By its identity: the indexes may be the last listing's.
        let keep = self.sel_id.take();
        let mut items: Vec<Item> = match self.view {
            PluginsView::Catalog => self.catalog().map_or(Vec::new(), |c| {
                (0..c.plugins.len())
                    .filter(|&i| c.plugins[i].fit == Fit::Yes)
                    .map(Item::Catalog)
                    .collect()
            }),
            PluginsView::Installed => (0..self.listing.installed.len())
                .map(Item::Installed)
                .collect(),
            PluginsView::OnStick => (0..self.listing.zips.len()).map(Item::Zip).collect(),
        };
        let look = self.search.as_deref().map(str::to_lowercase);
        items.retain(|&it| {
            let (name, maker, instrument) = self.name_of(it);
            let kind_ok = match self.filter {
                KindFilter::All => true,
                KindFilter::Instruments => instrument == Some(true),
                KindFilter::Effects => instrument == Some(false),
            };
            let look_ok = look.as_deref().is_none_or(|q| {
                let tags = self
                    .cat(&self.identity(it))
                    .map(|p| p.tags.join(" "))
                    .unwrap_or_default();
                format!("{name} {maker} {tags}").to_lowercase().contains(q)
            });
            kind_ok && look_ok
        });
        let key = |it: Item| {
            let id = self.identity(it);
            let fav = !self.favourites.contains(&id);
            let p = self.cat(&id);
            let name = self.name_of(it).0.to_lowercase();
            (
                fav,
                p.map_or(0, |p| p.downloads),
                p.map_or(String::new(), |p| p.updated.clone()),
                name,
            )
        };
        let mut keyed: Vec<_> = items.into_iter().map(|it| (key(it), it)).collect();
        match self.sort {
            SortBy::MostUsed => keyed
                .sort_by(|(a, _), (b, _)| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.3.cmp(&b.3))),
            SortBy::Newest => keyed
                .sort_by(|(a, _), (b, _)| a.0.cmp(&b.0).then(b.2.cmp(&a.2)).then(a.3.cmp(&b.3))),
            SortBy::AToZ => keyed.sort_by(|(a, _), (b, _)| a.0.cmp(&b.0).then(a.3.cmp(&b.3))),
        }
        self.items = keyed.into_iter().map(|(_, it)| it).collect();
        self.selected = keep
            .and_then(|k| self.items.iter().position(|&it| self.identity(it) == k))
            .or((!self.items.is_empty()).then_some(0));
        self.sel_id = self.selected_item().map(|it| self.identity(it));
        self.keep_in_view();
        self.ask_picture(false);
    }

    fn keep_in_view(&mut self) {
        let room = self.rows_room.max(1);
        let max_top = self.items.len().saturating_sub(room);
        if let Some(k) = self.selected {
            if k < self.top {
                self.top = k;
            } else if k >= self.top + room {
                self.top = k + 1 - room;
            }
        }
        self.top = self.top.min(max_top);
    }

    fn back_to_top(&mut self) {
        self.top = 0;
        self.select(0);
    }

    fn select(&mut self, k: usize) {
        if self.items.is_empty() {
            return;
        }
        let k = k.min(self.items.len() - 1);
        if self.selected != Some(k) {
            self.selected = Some(k);
            self.sel_id = Some(self.identity(self.items[k]));
            self.confirm = None;
            self.keep_in_view();
            self.ask_picture(false);
        }
    }

    // ----------------------------------------------------------- picture

    /// The selected item's skin folder (when installed) and the catalog's
    /// picture.
    fn picture_source(&self, it: Item) -> (Option<PathBuf>, Option<String>) {
        let id = self.identity(it);
        let folder = match it {
            Item::Installed(i) => Some(self.listing.installed[i].path.clone()),
            _ => self.installed_id(&id).map(|p| p.path.clone()),
        };
        let url = self
            .cat(&id)
            .map(|p| p.screenshot.clone())
            .filter(|u| u.starts_with("https://"));
        (folder, url)
    }

    fn ask_picture(&mut self, next: bool) {
        let Some(it) = self.selected_item() else {
            self.picture = Pic::default();
            return;
        };
        let (folder, url) = self.picture_source(it);
        let key = format!("{}|{}", self.identity(it), folder.is_some());
        let index = if next && self.picture.key == key && self.picture.count > 0 {
            if self.picture.count == 1 && self.picture.image.is_some() {
                return;
            }
            (self.picture.index + 1) % self.picture.count
        } else if self.picture.key == key {
            if self.picture.image.is_some() || self.picture.waiting {
                return;
            }
            self.picture.index
        } else {
            0
        };
        if folder.is_none() && url.is_none() {
            self.picture = Pic {
                key,
                why: "No picture".into(),
                ..Pic::default()
            };
            return;
        }
        let count = if self.picture.key == key {
            self.picture.count
        } else {
            0
        };
        if let Some((_, img, n)) = self
            .cache
            .iter()
            .find(|(k, _, _)| k.0 == key && k.1 == index)
        {
            self.picture = Pic {
                key,
                index,
                count: *n,
                image: Some(Arc::clone(img)),
                why: String::new(),
                waiting: false,
            };
            return;
        }
        self.picture = Pic {
            key: key.clone(),
            index,
            count,
            image: None,
            why: String::new(),
            waiting: true,
        };
        if let Some(w) = &self.worker {
            w.ask(Ask::Picture {
                key,
                index,
                folder,
                url,
            });
        }
    }

    // ------------------------------------------------------------ worker

    fn start(&mut self, rack: &Arc<Plugins>) {
        if self.worker.is_none() && !self.offline {
            self.worker = Some(Worker::start(self.dir.clone(), Arc::clone(rack)));
        }
    }

    fn ask(&self, a: Ask) {
        if let Some(w) = &self.worker {
            w.ask(a);
        }
    }

    fn fetch_catalog(&mut self) {
        self.fetched = true;
        self.fetching = true;
        self.ask(Ask::FetchCatalog);
    }

    /// The worker's news, once a frame. True when the screen changed.
    pub fn poll(&mut self) -> bool {
        let Some(w) = &self.worker else {
            return false;
        };
        let news = w.news();
        if news.is_empty() {
            return false;
        }
        for n in news {
            self.take(n);
        }
        true
    }

    fn take(&mut self, n: News) {
        match n {
            News::Listing(l) => {
                let first = !self.listed;
                self.listing = *l;
                self.listed = true;
                self.favourites = self.listing.favourites.clone();
                self.rebuild();
                // The catalog once a run, when it can come: the list is
                // the kept one until then.
                if first && !self.fetched && self.listing.can_download && !self.listing.no_folder {
                    self.fetch_catalog();
                }
            }
            News::Progress { key, done, total } => {
                if let Some(b) = self.busy.as_mut().filter(|b| b.key == key) {
                    b.done = done;
                    b.total = total;
                }
            }
            News::Done { key, said, changed } => {
                if key == "catalog" {
                    self.fetching = false;
                    self.reached = Some(said.is_ok());
                    // Fetched on opening: the status line says how it went
                    // (Ethernet or no network). Said in words only when
                    // asked for (Update list) or when there is no list.
                    let by_hand = std::mem::take(&mut self.by_hand);
                    match &said {
                        Ok(_) if !by_hand => {
                            self.said = None;
                            return;
                        }
                        Err(_) if !by_hand && self.catalog().is_some() => return,
                        _ => {}
                    }
                } else if self.busy.as_ref().is_some_and(|b| b.key == key) {
                    self.busy = None;
                }
                self.said = Some(match said {
                    Ok(s) => (s, Tone::Good),
                    Err(e) => (e, Tone::Bad),
                });
                if changed {
                    self.changed = true;
                    // The installed plugin's own screen is a picture now.
                    self.picture = Pic::default();
                    self.cache.clear();
                }
            }
            News::Picture {
                key,
                index,
                count,
                image,
                why,
            } => {
                if let Some(img) = &image {
                    self.cache.retain(|(k, _, _)| !(k.0 == key && k.1 == index));
                    if self.cache.len() >= PICTURES_KEPT {
                        self.cache.remove(0);
                    }
                    self.cache
                        .push(((key.clone(), index), Arc::clone(img), count));
                }
                if key == self.picture.key && index == self.picture.index {
                    self.picture.count = count;
                    self.picture.image = image;
                    self.picture.why = why;
                    self.picture.waiting = false;
                }
            }
        }
    }

    /// The plugins changed since last asked (installed or updated): the
    /// app offers them on the Synth tab and in the chain editor.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    // ------------------------------------------------------------- input

    /// A touch, dial turn or button on the place. `rows` is the room the
    /// list has.
    pub fn input(&mut self, i: PluginInput, rows: usize, rack: &Arc<Plugins>) {
        self.rows_room = rows.max(1);
        // What was last said stays until the musician moves on.
        if matches!(
            i,
            PluginInput::View(_)
                | PluginInput::Filter(_)
                | PluginInput::Sort
                | PluginInput::SearchText(_)
                | PluginInput::Row(_)
                | PluginInput::Dial(_)
        ) && !self.by_hand
        {
            self.said = None;
        }
        match i {
            PluginInput::Shown => {
                self.start(rack);
                self.ask(Ask::List);
                if !self.listed {
                    self.rebuild();
                }
            }
            // The view or chip already on: back to the list's top.
            PluginInput::View(v) if v == self.view => self.back_to_top(),
            PluginInput::Filter(f) if f == self.filter => self.back_to_top(),
            PluginInput::View(v) => {
                self.view = v;
                self.top = 0;
                self.confirm = None;
                self.rebuild();
            }
            PluginInput::Filter(f) => {
                self.filter = f;
                self.rebuild();
            }
            PluginInput::Sort => {
                self.sort = self.sort.next();
                self.top = 0;
                self.rebuild();
            }
            // The browser opens its keyboard and gives SearchText.
            PluginInput::Search => {}
            PluginInput::SearchText(t) => {
                let t = t.trim().to_string();
                self.search = (!t.is_empty()).then_some(t);
                self.top = 0;
                self.rebuild();
            }
            // The selected row again: as push (the browser's rows too).
            PluginInput::Row(k) if self.selected == Some(k) => self.ask_picture(true),
            PluginInput::Row(k) => self.select(k),
            PluginInput::Scroll(d) => {
                let max_top = self.items.len().saturating_sub(self.rows_room) as i64;
                self.top = (self.top as i64 + i64::from(d)).clamp(0, max_top.max(0)) as usize;
            }
            PluginInput::Dial(d) => {
                let k = self
                    .selected
                    .map_or(0, |k| (k as i64 + i64::from(d)).max(0) as usize);
                self.select(k);
            }
            // Push: the next picture (the hint line says so).
            PluginInput::Push => self.ask_picture(true),
            // +: Install, Update or Install from zip, or a Remove asked.
            PluginInput::Enter => {
                let d = self.detail();
                let first = d.buttons.iter().copied().find(|b| {
                    matches!(
                        b,
                        PluginButton::Install
                            | PluginButton::InstallZip
                            | PluginButton::Update
                            | PluginButton::ConfirmRemove
                            | PluginButton::ConfirmDeleteZip
                    )
                });
                if let Some(b) = first {
                    self.button(b);
                }
            }
            PluginInput::Back => {
                if self.confirm.take().is_none() && self.search.take().is_some() {
                    self.rebuild();
                }
            }
            PluginInput::Picture => self.ask_picture(true),
            PluginInput::Button(b) => self.button(b),
        }
    }

    fn button(&mut self, b: PluginButton) {
        let it = self.selected_item();
        let id = it.map(|it| self.identity(it)).unwrap_or_default();
        if !matches!(
            b,
            PluginButton::ConfirmRemove | PluginButton::ConfirmDeleteZip
        ) {
            self.confirm = None;
        }
        match b {
            PluginButton::Refresh => {
                self.said = Some(("Updating the list…".into(), Tone::Dim));
                self.by_hand = true;
                self.fetch_catalog();
            }
            PluginButton::Install | PluginButton::Update => {
                if self.busy.is_some() {
                    return;
                }
                let Some(p) = self.cat(&id) else {
                    return;
                };
                let what = format!("Downloading {}", p.name);
                let total = p.release.as_ref().map_or(0, |r| r.size);
                self.busy = Some(Busy {
                    key: id.clone(),
                    what,
                    done: 0,
                    total,
                });
                self.said = None;
                self.ask(Ask::Install { id });
            }
            PluginButton::InstallZip => {
                if self.busy.is_some() {
                    return;
                }
                let z = match it {
                    Some(Item::Zip(i)) => Some(&self.listing.zips[i]),
                    _ => self.zip_id(&id),
                };
                let Some(z) = z.cloned() else {
                    return;
                };
                self.busy = Some(Busy {
                    key: z.path.display().to_string(),
                    what: format!("Installing {}", z.name),
                    done: 0,
                    total: 0,
                });
                self.said = None;
                self.ask(Ask::InstallZip { path: z.path });
            }
            PluginButton::Remove => self.confirm = Some((PluginButton::ConfirmRemove, id)),
            PluginButton::DeleteZip => self.confirm = Some((PluginButton::ConfirmDeleteZip, id)),
            PluginButton::ConfirmRemove => {
                if self.confirm.take().is_none_or(|(_, k)| k != id) {
                    return;
                }
                let folder = match it {
                    Some(Item::Installed(i)) => Some(self.listing.installed[i].folder.clone()),
                    _ => self.installed_id(&id).map(|p| p.folder.clone()),
                };
                if let Some(folder) = folder {
                    self.picture = Pic::default();
                    self.ask(Ask::Remove { folder });
                }
            }
            PluginButton::ConfirmDeleteZip => {
                if self.confirm.take().is_none_or(|(_, k)| k != id) {
                    return;
                }
                if let Some(Item::Zip(i)) = it {
                    let path = self.listing.zips[i].path.clone();
                    self.ask(Ask::DeleteZip { path });
                }
            }
            PluginButton::Cancel => {
                if let Some(w) = &self.worker {
                    w.cancel();
                }
            }
            PluginButton::Pictures => self.ask_picture(true),
            PluginButton::Favourite => {
                if id.is_empty() {
                    return;
                }
                if let Some(k) = self.favourites.iter().position(|f| *f == id) {
                    self.favourites.remove(k);
                } else {
                    self.favourites.push(id);
                }
                self.ask(Ask::Favourites(self.favourites.clone()));
                self.rebuild();
            }
        }
    }

    // ------------------------------------------------------------ screen

    fn row(&self, it: Item) -> PluginRow {
        let id = self.identity(it);
        let (name, maker, instrument) = self.name_of(it);
        let (tag, tone) = if let Some(b) = self.busy.as_ref().filter(|b| b.key == id) {
            match (b.done * 100).checked_div(b.total) {
                Some(pct) => (format!("{pct}%"), Tone::Accent),
                None => ("Downloading".into(), Tone::Accent),
            }
        } else {
            match it {
                Item::Catalog(_) => match self.installed_id(&id) {
                    Some(p) => match self.update_for(&id, &p.version) {
                        Some(v) => (format!("Update {v}"), Tone::Warn),
                        None => ("Installed".into(), Tone::Good),
                    },
                    None if self.zip_id(&id).is_some() => ("Zip on the stick".into(), Tone::Plain),
                    None => (kind_label(instrument == Some(true)).into(), Tone::Dim),
                },
                Item::Installed(i) => {
                    let p = &self.listing.installed[i];
                    match self.update_for(&id, &p.version) {
                        Some(v) => (format!("Update {v}"), Tone::Warn),
                        None if !p.version.is_empty() => (p.version.clone(), Tone::Dim),
                        None => (kind_label(p.instrument).into(), Tone::Dim),
                    }
                }
                Item::Zip(i) => {
                    let z = &self.listing.zips[i];
                    if !z.problem.is_empty() {
                        ("Can't install".into(), Tone::Bad)
                    } else {
                        match self.installed_id(&id) {
                            Some(p) if newer(&z.version, &p.version) => {
                                (format!("Update {}", z.version), Tone::Warn)
                            }
                            Some(_) => ("Installed".into(), Tone::Good),
                            None => ("Not installed".into(), Tone::Plain),
                        }
                    }
                }
            }
        };
        PluginRow {
            name,
            maker,
            instrument: instrument == Some(true),
            tag,
            tone,
            favourite: self.favourites.contains(&id),
        }
    }

    fn detail(&self) -> PluginDetail {
        let Some(it) = self.selected_item() else {
            return PluginDetail {
                picture_empty: String::new(),
                ..PluginDetail::default()
            };
        };
        let id = self.identity(it);
        let (name, maker, instrument) = self.name_of(it);
        let cat = self.cat(&id);
        let installed = match it {
            Item::Installed(i) => Some(&self.listing.installed[i]),
            _ => self.installed_id(&id),
        };
        let zip = match it {
            Item::Zip(i) => Some(&self.listing.zips[i]),
            _ => self.zip_id(&id),
        };
        let mut d = PluginDetail {
            title: name.clone(),
            maker,
            favourite: self.favourites.contains(&id),
            ..PluginDetail::default()
        };
        let line = |l: &str, v: String, t: Tone| (l.to_string(), v, t);
        if let Some(ins) = instrument {
            let style = cat.map_or("", |p| p.style.as_str());
            let kind = if style.is_empty() {
                kind_label(ins).to_string()
            } else {
                format!("{} · {style}", kind_label(ins))
            };
            d.lines.push(line("Kind", kind, Tone::Plain));
        }
        // The version this item is about.
        let release = cat.and_then(|p| p.release.as_ref());
        match it {
            Item::Zip(_) => {
                let z = zip.expect("a zip row");
                d.lines.push(line("File", z.file.clone(), Tone::Plain));
                let v = if z.version.is_empty() {
                    human(z.bytes)
                } else {
                    format!("{} · {}", z.version, human(z.bytes))
                };
                d.lines.push(line("Version", v, Tone::Plain));
                d.lines.push(if cat.is_some() {
                    line("Catalog", "The catalog's own zip".into(), Tone::Good)
                } else {
                    line(
                        "Catalog",
                        "Not the catalog's: check where it came from".into(),
                        Tone::Warn,
                    )
                });
            }
            _ => {
                if let Some(r) = release {
                    let v = format!("{} · {}", r.version, human(r.size));
                    d.lines.push(line("Version", v, Tone::Plain));
                } else if let Some(p) = installed.filter(|p| !p.version.is_empty()) {
                    d.lines
                        .push(line("Version", p.version.clone(), Tone::Plain));
                }
            }
        }
        if let Some(p) = cat {
            if !p.license.is_empty() {
                d.lines
                    .push(line("Licence", p.license.clone(), Tone::Plain));
            }
            if let Some(r) = release {
                let pct = r
                    .cpu_pct
                    .map(|c| format!(" (the catalog measured {c:.0} %)"))
                    .unwrap_or_default();
                let cpu = match r.cpu.as_str() {
                    "PASS" => Some(("Light", Tone::Good)),
                    "WARN" => Some(("Heavy", Tone::Warn)),
                    "FAIL" => Some(("Too heavy", Tone::Bad)),
                    _ => None,
                };
                if let Some((word, t)) = cpu {
                    d.lines.push(line("CPU", format!("{word}{pct}"), t));
                }
                if !r.requires.is_empty() {
                    d.lines.push(line("Needs", r.requires.clone(), Tone::Warn));
                }
                if !r.depends.is_empty() {
                    let names: Vec<String> = r
                        .depends
                        .iter()
                        .map(|dep| self.cat(dep).map_or(dep.clone(), |q| q.name.clone()))
                        .collect();
                    d.lines
                        .push(line("Also needs", names.join(", "), Tone::Warn));
                }
            }
            d.summary = p.summary.clone();
        }
        if let Some(p) = installed {
            let from = if p.by_mpfree { "" } else { " (copied)" };
            d.lines
                .push(line("Here", format!("Installed{from}"), Tone::Good));
        } else if !matches!(it, Item::Zip(_)) && zip.is_some() {
            d.lines
                .push(line("Here", "Zip on the stick".into(), Tone::Plain));
        }
        // Picture.
        d.picture = self.picture.image.clone();
        if self.picture.count > 1 {
            d.picture_label = format!("{} / {}", self.picture.index + 1, self.picture.count);
        }
        d.picture_empty = if self.picture.waiting {
            "Drawing…".into()
        } else if self.picture.why.is_empty() {
            "No picture".into()
        } else {
            self.picture.why.clone()
        };
        // What it does next, and the buttons.
        let mine = self
            .busy
            .as_ref()
            .filter(|b| b.key == id || zip.is_some_and(|z| b.key == z.path.display().to_string()));
        if let Some(b) = mine {
            d.progress = Some(if b.total > 0 {
                (b.done as f32 / b.total as f32).min(1.0)
            } else {
                0.0
            });
            d.note = if b.total > 0 {
                format!("{}: {} of {}", b.what, human(b.done), human(b.total))
            } else {
                format!("{}…", b.what)
            };
            d.note_tone = Tone::Accent;
            d.buttons = vec![PluginButton::Cancel];
            return d;
        }
        let confirm = |plain: PluginButton, ask: PluginButton| match &self.confirm {
            Some((c, k)) if *c == ask && *k == id => ask,
            _ => plain,
        };
        let can_fetch = self.listing.can_download && !self.listing.no_folder;
        let mut buttons = Vec::new();
        match it {
            Item::Zip(_) => {
                let z = zip.expect("a zip row");
                if !z.problem.is_empty() {
                    d.note = format!("Can't install: {}", z.problem);
                    d.note_tone = Tone::Bad;
                } else {
                    buttons.push(PluginButton::InstallZip);
                    d.note = format!("Unpacks {} into files/plugins/", z.folders.join(", "));
                }
            }
            _ => {
                let update = installed.and_then(|p| self.update_for(&id, &p.version));
                if let Some(v) = &update {
                    if let Some(z) = zip.filter(|z| z.version == *v) {
                        buttons.push(PluginButton::InstallZip);
                        d.note = format!("{v} is on the stick ({})", z.file);
                    } else if can_fetch {
                        buttons.push(PluginButton::Update);
                        d.note =
                            format!("{v} is out: Update downloads it and keeps your own files");
                    } else {
                        d.note = format!("{v} is out: copy its zip into files/plugins/ to update");
                    }
                    d.note_tone = Tone::Accent;
                } else if let Some(p) = installed {
                    d.note = format!("In files/plugins/{}", p.folder);
                } else if let Some(z) = zip {
                    buttons.push(PluginButton::InstallZip);
                    d.note = format!("Its zip is on the stick ({})", z.file);
                } else if let (Some(p), Some(r)) = (cat, release) {
                    if can_fetch {
                        buttons.push(PluginButton::Install);
                        d.note = format!(
                            "Downloads from {}, checks it against the catalog, then installs.",
                            host_name(&r.url)
                        );
                    } else if self.listing.no_folder {
                        d.note = "No data folder here: nowhere to install".into();
                        d.note_tone = Tone::Warn;
                    } else {
                        d.note = format!(
                            "No downloads here: copy {}'s zip into files/plugins/",
                            p.name
                        );
                        d.note_tone = Tone::Warn;
                    }
                }
            }
        }
        // Pictures steps through them: not there for a lone one.
        if self.picture.count != 1 {
            buttons.push(PluginButton::Pictures);
        }
        if !id.starts_with("zip:") {
            buttons.push(PluginButton::Favourite);
        }
        match it {
            Item::Zip(_) => buttons.push(confirm(
                PluginButton::DeleteZip,
                PluginButton::ConfirmDeleteZip,
            )),
            _ if installed.is_some() => {
                buttons.push(confirm(PluginButton::Remove, PluginButton::ConfirmRemove))
            }
            _ => {}
        }
        d.buttons = buttons;
        d
    }

    /// The line under the list.
    fn status(&self) -> (String, Tone) {
        if let Some(b) = &self.busy {
            let pct = (b.done * 100)
                .checked_div(b.total)
                .map_or(String::new(), |p| format!(" {p}%"));
            return (format!("{}…{pct}", b.what), Tone::Accent);
        }
        if let Some((s, t)) = &self.said {
            return (s.clone(), *t);
        }
        if !self.listed {
            return ("Reading the plugins folder…".into(), Tone::Dim);
        }
        if self.listing.no_folder {
            return (
                "No data folder: plugins can't be installed here".into(),
                Tone::Warn,
            );
        }
        let n = self.items.len();
        let shown = match self.view {
            PluginsView::Catalog => {
                let Some(c) = self.catalog() else {
                    return if self.fetching {
                        ("Fetching the catalog…".into(), Tone::Dim)
                    } else {
                        ("No catalog yet".into(), Tone::Dim)
                    };
                };
                let all = c.plugins.len();
                let runs = c.runnable().count();
                let mut s = format!(
                    "Open MPC catalog, {}: {runs} of {all} run on this MPC",
                    when(&c.generated)
                );
                if n != runs {
                    s = format!("{n} shown · {s}");
                }
                match self.reached {
                    Some(true) => s.push_str("  ·  Ethernet"),
                    Some(false) => s.push_str("  ·  no network"),
                    None => {}
                }
                s
            }
            PluginsView::Installed => format!("{n} installed"),
            PluginsView::OnStick => format!("{n} zips in files/plugins/"),
        };
        (shown, Tone::Dim)
    }

    fn empty(&self) -> String {
        if !self.listed {
            return "Reading…".into();
        }
        if self.search.is_some() || self.filter != KindFilter::All {
            return "Nothing matches.".into();
        }
        match self.view {
            PluginsView::Catalog if self.catalog().is_none() => {
                if self.fetching {
                    "Fetching the Open MPC catalog…".into()
                } else if self.listing.can_download {
                    "No catalog yet. Plug in an Ethernet cable and tap Update list, or copy plugin zips into files/plugins/ on the stick.".into()
                } else {
                    "No catalog here. Copy plugin zips from open-mpc.github.io into files/plugins/ on the stick: they show under On the stick.".into()
                }
            }
            PluginsView::Catalog => "The catalog lists none that run on this MPC.".into(),
            PluginsView::Installed => {
                "No MPC plugins installed yet. Install one from Catalog or On the stick.".into()
            }
            PluginsView::OnStick => {
                "No zips in files/plugins/. Download a plugin's armv7 zip from open-mpc.github.io and copy it there.".into()
            }
        }
    }

    pub fn screen(&self) -> PluginsScreen {
        let (status, status_tone) = self.status();
        PluginsScreen {
            view: self.view,
            rows: self.items.iter().map(|&it| self.row(it)).collect(),
            selected: self.selected,
            top: self.top,
            filter: self.filter,
            sort: self.sort.label().into(),
            search: self.search.clone(),
            status,
            status_tone,
            empty: self.empty(),
            list_button: (self.view == PluginsView::Catalog
                && self.listing.can_download
                && !self.listing.no_folder)
                .then_some(PluginButton::Refresh),
            detail: self.detail(),
        }
    }
}

/// A made-up catalog for the manual's picture (no real plugin).
pub const SAMPLE_CATALOG: &str = r#"{"schema":1,"generated":"2026-11-15T06:00:00Z","plugins":[
  {"id":"example.tape-echo","name":"Tape Echo","author":"Example Audio","kind":"effect","license":"MIT",
   "summary":"A warm tape delay with wow, flutter and a spring tank in the feedback.","tags":["delay","tape"],
   "downloads":5200,"updated":"2026-11-02","latest":"1.4.0","versions":[{"version":"1.4.0","channel":"stable","date":"2026-11-02",
   "cpu":{"verdict":"PASS","p99_pct":9.0},"assets":{"armv7":{"url":"https://github.com/example/tape-echo/releases/download/1.4.0/tape-echo-armv7.zip","sha256":"1111111111111111111111111111111111111111111111111111111111111111","size":3250585}}}]},
  {"id":"example.drift-pad","name":"Drift Pad","author":"Example Audio","kind":"instrument","license":"GPL-3.0",
   "summary":"Slow evolving pads from two detuned wavetables.","downloads":4100,"updated":"2026-10-20","latest":"2.0.1",
   "versions":[{"version":"2.0.1","channel":"stable","date":"2026-10-20","cpu":{"verdict":"PASS","p99_pct":18.0},
   "assets":{"armv7":{"url":"https://github.com/example/drift-pad/releases/download/2.0.1/drift-pad-armv7.zip","sha256":"2222222222222222222222222222222222222222222222222222222222222222","size":5452595}}}]},
  {"id":"shed.mono-bass","name":"Mono Bass","author":"Synth Shed","kind":"instrument","license":"MIT",
   "summary":"A one-oscillator bass with a ladder filter and glide.","downloads":3500,"updated":"2026-11-10","latest":"1.3.0",
   "versions":[{"version":"1.3.0","channel":"stable","date":"2026-11-10","cpu":{"verdict":"PASS","p99_pct":7.0},
   "assets":{"armv7":{"url":"https://github.com/shed/mono-bass/releases/download/1.3.0/mono-bass-armv7.zip","sha256":"3333333333333333333333333333333333333333333333333333333333333333","size":1887436}}}]},
  {"id":"shed.plate","name":"Plate Reverb","author":"Synth Shed","kind":"effect","license":"BSD-3-Clause",
   "summary":"A bright plate with pre-delay and damping.","downloads":2900,"updated":"2026-09-30","latest":"0.9.2",
   "versions":[{"version":"0.9.2","channel":"stable","date":"2026-09-30",
   "assets":{"armv7":{"url":"https://github.com/shed/plate/releases/download/0.9.2/Plate-Reverb-armv7.zip","sha256":"4444444444444444444444444444444444444444444444444444444444444444","size":2400000}}}]},
  {"id":"lab6.fm-bells","name":"FM Bells","author":"Lab Six","kind":"instrument","license":"MIT",
   "summary":"Four-operator FM tuned for bells and glass.","downloads":2400,"updated":"2026-08-14","latest":"1.0.0",
   "versions":[{"version":"1.0.0","channel":"stable","date":"2026-08-14",
   "assets":{"armv7":{"url":"https://github.com/lab6/fm-bells/releases/download/1.0.0/fm-bells-armv7.zip","sha256":"5555555555555555555555555555555555555555555555555555555555555555","size":1200000}}}]},
  {"id":"lab6.crusher","name":"Bit Crusher","author":"Lab Six","kind":"effect","license":"MIT",
   "summary":"Bits, rate and a little drive.","downloads":1800,"updated":"2026-07-01","latest":"1.1.0",
   "versions":[{"version":"1.1.0","channel":"stable","date":"2026-07-01",
   "assets":{"armv7":{"url":"https://github.com/lab6/crusher/releases/download/1.1.0/crusher-armv7.zip","sha256":"6666666666666666666666666666666666666666666666666666666666666666","size":800000}}}]},
  {"id":"lab6.ensemble","name":"Ensemble","author":"Lab Six","kind":"effect","license":"MIT",
   "summary":"A three-voice string ensemble chorus.","downloads":1500,"updated":"2026-06-11","latest":"1.0.2",
   "versions":[{"version":"1.0.2","channel":"stable","date":"2026-06-11",
   "assets":{"armv7":{"url":"https://github.com/lab6/ensemble/releases/download/1.0.2/ensemble-armv7.zip","sha256":"7777777777777777777777777777777777777777777777777777777777777777","size":900000}}}]},
  {"id":"example.drum-synth","name":"Drum Synth","author":"Example Audio","kind":"instrument","license":"MIT",
   "summary":"Kicks, snares and hats from noise and sines.","downloads":1200,"updated":"2026-10-05","latest":"0.4.0",
   "versions":[{"version":"0.4.0","channel":"stable","date":"2026-10-05",
   "assets":{"armv7":{"url":"https://github.com/example/drum-synth/releases/download/0.4.0/drum-synth-armv7.zip","sha256":"8888888888888888888888888888888888888888888888888888888888888888","size":1500000}}}]},
  {"id":"shed.grain-cloud","name":"Grain Cloud","author":"Synth Shed","kind":"instrument","license":"GPL-3.0",
   "summary":"Granular clouds from the sounds you load.","downloads":900,"updated":"2026-11-12","latest":"0.2.0",
   "versions":[{"version":"0.2.0","channel":"stable","date":"2026-11-12","cpu":{"verdict":"WARN","p99_pct":41.0},
   "assets":{"armv7":{"url":"https://github.com/shed/grain-cloud/releases/download/0.2.0/grain-cloud-armv7.zip","sha256":"9999999999999999999999999999999999999999999999999999999999999999","size":2100000}}}]},
  {"id":"example.addin","name":"Some Add-in","author":"Example Audio","kind":"addin","downloads":9999,"versions":[]}
]}"#;

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_mpcstore::catalog;

    const CATALOG: &str = r#"{"schema":1,"generated":"2026-11-10T00:00:00Z","plugins":[
      {"id":"maker.synth","name":"Big Synth","author":"Maker","kind":"instrument","license":"MIT",
       "summary":"A synth.","screenshot":"https://raw.githubusercontent.com/x/y.png","tags":["poly"],
       "downloads":50,"updated":"2026-10-01","latest":"1.2.0",
       "versions":[{"version":"1.2.0","channel":"stable","date":"2026-10-01",
         "cpu":{"verdict":"PASS","p99_pct":12.0},
         "manifest":{"folder":"portable/Maker - VST - Big Synth"},
         "assets":{"armv7":{"url":"https://github.com/x/y/releases/download/1.2.0/a.zip","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":2097152}}}]},
      {"id":"maker.echo","name":"Echo","author":"Maker","kind":"effect","license":"GPL-3.0",
       "summary":"An echo.","downloads":90,"updated":"2026-11-01","latest":"0.3.0",
       "versions":[{"version":"0.3.0","channel":"stable","date":"2026-11-01",
         "manifest":{"folder":"portable/Maker - VST - Echo"},
         "assets":{"armv7":{"url":"https://github.com/x/e/releases/download/0.3.0/e.zip","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","size":1048576}}}]},
      {"id":"maker.addin","name":"Add-in","author":"Maker","kind":"addin","downloads":999,
       "versions":[]}
    ]}"#;

    fn page_with(
        installed: Vec<InstalledPlugin>,
        zips: Vec<ZipFile>,
        can_download: bool,
    ) -> PluginPage {
        let mut p = PluginPage::new();
        let cat = catalog::parse(CATALOG).expect("the test catalog reads");
        p.take(News::Listing(Box::new(Listing {
            catalog: Some(Arc::new(cat)),
            installed,
            zips,
            can_download,
            no_folder: false,
            favourites: Vec::new(),
        })));
        p
    }

    fn rows(p: &PluginPage) -> Vec<(String, String)> {
        p.screen()
            .rows
            .into_iter()
            .map(|r| (r.name, r.tag))
            .collect()
    }

    #[test]
    fn the_catalog_lists_what_runs_here_most_used_first() {
        let p = page_with(Vec::new(), Vec::new(), true);
        assert_eq!(
            rows(&p),
            vec![
                ("Echo".into(), "Effect".into()),
                ("Big Synth".into(), "Instrument".into())
            ],
            "the add-in is left out"
        );
        let s = p.screen();
        assert_eq!(s.selected, Some(0));
        assert_eq!(s.list_button, Some(PluginButton::Refresh));
        assert_eq!(s.detail.buttons[0], PluginButton::Install);
        assert_eq!(
            s.detail.note,
            "Downloads from GitHub, checks it against the catalog, then installs."
        );
        assert!(s
            .detail
            .lines
            .contains(&("Version".into(), "0.3.0 · 1.0 MB".into(), Tone::Plain)));
        assert_eq!(
            s.status,
            "Open MPC catalog, 10 Nov 00:00 UTC: 2 of 3 run on this MPC"
        );
    }

    #[test]
    fn orders_filters_and_search() {
        let mut p = page_with(Vec::new(), Vec::new(), true);
        let rack = Arc::new(Plugins::none());
        p.input(PluginInput::Sort, 8, &rack);
        assert_eq!(p.screen().sort, "Newest");
        assert_eq!(rows(&p)[0].0, "Echo");
        p.input(PluginInput::Sort, 8, &rack);
        assert_eq!(p.screen().sort, "A to Z");
        assert_eq!(rows(&p)[0].0, "Big Synth");
        p.input(PluginInput::Filter(KindFilter::Effects), 8, &rack);
        assert_eq!(rows(&p).len(), 1);
        p.input(PluginInput::Filter(KindFilter::All), 8, &rack);
        p.input(PluginInput::SearchText("poly".into()), 8, &rack);
        assert_eq!(
            rows(&p),
            vec![("Big Synth".into(), "Instrument".into())],
            "tags count"
        );
        p.input(PluginInput::Back, 8, &rack);
        assert_eq!(rows(&p).len(), 2, "Back clears the search");
    }

    #[test]
    fn installed_and_older_says_update_and_remove_asks_twice() {
        let ins = InstalledPlugin {
            folder: "Maker - VST - Big Synth".into(),
            path: PathBuf::from("/nowhere/Maker - VST - Big Synth"),
            name: "Big Synth".into(),
            maker: "Maker".into(),
            instrument: true,
            version: "1.1.0".into(),
            id: "maker.synth".into(),
            by_mpfree: true,
        };
        let mut p = page_with(vec![ins], Vec::new(), true);
        let rack = Arc::new(Plugins::none());
        assert_eq!(rows(&p)[1], ("Big Synth".into(), "Update 1.2.0".into()));
        p.input(PluginInput::View(PluginsView::Installed), 8, &rack);
        let d = p.screen().detail;
        assert_eq!(
            d.buttons,
            vec![
                PluginButton::Update,
                PluginButton::Pictures,
                PluginButton::Favourite,
                PluginButton::Remove
            ]
        );
        p.input(PluginInput::Button(PluginButton::Remove), 8, &rack);
        assert!(p
            .screen()
            .detail
            .buttons
            .contains(&PluginButton::ConfirmRemove));
        p.input(PluginInput::Dial(1), 8, &rack);
        assert!(
            p.screen()
                .detail
                .buttons
                .contains(&PluginButton::ConfirmRemove),
            "one row: the dial stays put and the question stays"
        );
        p.input(PluginInput::Back, 8, &rack);
        assert!(
            p.screen().detail.buttons.contains(&PluginButton::Remove),
            "Back takes it back"
        );
    }

    #[test]
    fn a_zip_on_the_stick_installs_from_the_zip_and_a_strange_one_is_flagged() {
        let known = ZipFile {
            path: PathBuf::from("/nowhere/echo.zip"),
            file: "echo.zip".into(),
            bytes: 1 << 20,
            id: "maker.echo".into(),
            version: "0.3.0".into(),
            name: "Echo".into(),
            maker: "Maker".into(),
            instrument: Some(false),
            folders: vec!["Maker - VST - Echo".into()],
            ..ZipFile::default()
        };
        let strange = ZipFile {
            path: PathBuf::from("/nowhere/thing.zip"),
            file: "thing.zip".into(),
            name: "thing.zip".into(),
            problem: "no MPC plugin folder in it".into(),
            ..ZipFile::default()
        };
        let mut p = page_with(Vec::new(), vec![known, strange], false);
        let rack = Arc::new(Plugins::none());
        assert_eq!(rows(&p)[0], ("Echo".into(), "Zip on the stick".into()));
        assert_eq!(p.screen().detail.buttons[0], PluginButton::InstallZip);
        assert_eq!(p.screen().list_button, None, "no downloads here");
        p.input(PluginInput::View(PluginsView::OnStick), 8, &rack);
        let tags: Vec<String> = rows(&p).into_iter().map(|r| r.1).collect();
        assert!(tags.contains(&"Can't install".to_string()), "{tags:?}");
        p.input(PluginInput::Row(1), 8, &rack);
        let d = p.screen().detail;
        assert!(d.note.starts_with("Can't install"), "{}", d.note);
        assert!(!d.buttons.contains(&PluginButton::InstallZip));
        assert!(d.buttons.contains(&PluginButton::DeleteZip));
    }

    #[test]
    fn favourites_come_first_and_the_dial_keeps_the_selection_in_view() {
        let mut p = page_with(Vec::new(), Vec::new(), true);
        let rack = Arc::new(Plugins::none());
        p.input(PluginInput::Dial(1), 1, &rack);
        assert_eq!(p.screen().selected, Some(1));
        assert_eq!(p.screen().top, 1, "one row of room: the list follows");
        p.input(PluginInput::Button(PluginButton::Favourite), 1, &rack);
        let s = p.screen();
        assert_eq!(s.rows[0].name, "Big Synth");
        assert!(s.rows[0].favourite);
        assert_eq!(s.selected, Some(0), "the selection follows its plugin");
    }

    #[test]
    fn the_sample_shows_every_kind_of_row() {
        let p = PluginPage::sample(None);
        let s = p.screen();
        let tags: Vec<&str> = s.rows.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(s.rows[0].name, "Drift Pad", "the favourite first");
        for want in [
            "Installed",
            "Update 1.3.0",
            "Zip on the stick",
            "Effect",
            "Instrument",
        ] {
            assert!(tags.contains(&want), "{want} in {tags:?}");
        }
        assert_eq!(s.rows.len(), 9, "the add-in left out");
        assert_eq!(s.selected, Some(2));
        assert_eq!(s.detail.title, "Mono Bass");
        assert_eq!(s.detail.buttons[0], PluginButton::Update);
        assert_eq!(s.detail.picture_label, "1 / 2");
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(newer("1.10.0", "1.9.3"));
        assert!(newer("v2", "1.9"));
        assert!(!newer("1.2.0", "1.2.0"));
        assert!(!newer("", "1.0"));
    }
}
