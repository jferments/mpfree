//! The Plugins place's worker (Jay, 15 Nov: "Ethernet cable, or from the
//! stick"; layout spec section 3): a thread of its own that keeps the Open
//! MPC catalog, downloads and installs plugins, lists the zips on the stick
//! and draws the pictures, so the screen never waits on the network or the
//! stick. MPFree ships none of these plugins; the musician picks them.
//!
//! - **The catalog** is fetched over HTTPS (`mpfree_mpcstore::fetch`, the
//!   system's libcurl) and kept as `files/plugins/.catalog.json`, so the
//!   list is there without a network too.
//! - **Install** downloads the release's 32-bit ARM zip into
//!   `files/plugins/.downloads/`, checks it against the catalog's SHA-256,
//!   unpacks its plugin folder (`mpfree_mpcstore::install`) and adds it to
//!   the rack ([`Plugins::add`]), so it can be used at once.
//! - **On the stick:** the zips straight in `files/plugins/`, as people
//!   download them, each matched to the catalog by its SHA-256.
//! - **Pictures:** an installed plugin's own screen, each tab drawn as the
//!   MPC draws it (`mpfree_mpcskin`, every parameter at 0), then the
//!   catalog's picture; scaled to the Selected column.

use crate::plugins::Plugins;
use mpfree_mpcskin::handle::Params;
use mpfree_mpcskin::json;
use mpfree_mpcstore::{catalog, fetch, install, sha256, zip};
use mpfree_ui::{Image, Rect, SoftCanvas};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;

/// The catalog as last fetched, in the plugins folder.
pub const CATALOG_FILE: &str = ".catalog.json";
/// Downloads in progress and the catalog's pictures, in the plugins
/// folder.
pub const DOWNLOADS: &str = ".downloads";
/// The plugins marked Favourite, one id (or folder) a line.
pub const FAVOURITES: &str = ".favourites";
/// The biggest zip or catalog fetched.
const MAX_DOWNLOAD: u64 = 512 << 20;
/// The picture's size in the Selected column: 1280 x 628 scaled to 396.
pub const PICTURE_W: i32 = 396;
pub const PICTURE_H: i32 = 194;

/// What the screen asks the worker.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// List what is installed and on the stick, with the catalog kept.
    List,
    FetchCatalog,
    /// Download and install the catalog's plugin with this id.
    Install {
        id: String,
    },
    InstallZip {
        path: PathBuf,
    },
    /// Keep these as the favourites.
    Favourites(Vec<String>),
    DeleteZip {
        path: PathBuf,
    },
    Remove {
        folder: String,
    },
    /// Picture `index` of an item: its skin's tabs (an installed folder,
    /// or a zip's plugin folder once unpacked), then the catalog's picture.
    Picture {
        key: String,
        index: usize,
        folder: Option<PathBuf>,
        url: Option<String>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct InstalledPlugin {
    /// Its folder's name in `files/plugins/`.
    pub folder: String,
    pub path: PathBuf,
    pub name: String,
    pub maker: String,
    pub instrument: bool,
    /// From MPFree's record, else the folder's own description; may be
    /// empty.
    pub version: String,
    /// The catalog's id, when known (MPFree's record, or the folder name).
    pub id: String,
    /// Installed by MPFree (Remove then keeps the musician's own files).
    pub by_mpfree: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ZipFile {
    pub path: PathBuf,
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
    /// The catalog's id and version when its SHA-256 is the catalog's.
    pub id: String,
    pub version: String,
    pub name: String,
    pub maker: String,
    pub instrument: Option<bool>,
    /// The plugin folders in it.
    pub folders: Vec<String>,
    /// Why it can't be installed, if it can't.
    pub problem: String,
}

#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub catalog: Option<Arc<catalog::Catalog>>,
    pub installed: Vec<InstalledPlugin>,
    pub zips: Vec<ZipFile>,
    /// libcurl is there: downloads can work (with a network).
    pub can_download: bool,
    /// No data folder: nowhere to install.
    pub no_folder: bool,
    /// The favourites as kept.
    pub favourites: Vec<String>,
}

/// What the worker tells the screen.
#[derive(Clone, Debug)]
pub enum News {
    Listing(Box<Listing>),
    /// A download's progress (total 0 while unknown).
    Progress {
        key: String,
        done: u64,
        total: u64,
    },
    /// An ask finished: what to say, and whether the plugins changed (the
    /// app then offers them on the Synth tab and in the chain editor).
    Done {
        key: String,
        said: Result<String, String>,
        changed: bool,
    },
    Picture {
        key: String,
        index: usize,
        count: usize,
        image: Option<Arc<Image>>,
        why: String,
    },
}

/// The screen's end of the worker.
pub struct Worker {
    tx: Sender<Ask>,
    rx: Receiver<News>,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    /// Starts the worker on `dir` (`files/plugins/`, None without a data
    /// folder) with the app's rack.
    pub fn start(dir: Option<PathBuf>, rack: Arc<Plugins>) -> Worker {
        let (tx, ask_rx) = channel::<Ask>();
        let (news_tx, rx) = channel::<News>();
        let cancel = Arc::new(AtomicBool::new(false));
        let c = Arc::clone(&cancel);
        let _ = std::thread::Builder::new()
            .name("mpfree-plugin-store".into())
            .spawn(move || run(dir, rack, ask_rx, news_tx, c));
        Worker { tx, rx, cancel }
    }

    pub fn ask(&self, a: Ask) {
        if matches!(a, Ask::Install { .. }) {
            self.cancel.store(false, Ordering::Relaxed);
        }
        let _ = self.tx.send(a);
    }

    /// Stops the download running.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn news(&self) -> Vec<News> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(n) => out.push(n),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }
}

struct State {
    dir: Option<PathBuf>,
    rack: Arc<Plugins>,
    catalog: Option<Arc<catalog::Catalog>>,
    /// Each zip's SHA-256 by (path, size, modified), so a listing doesn't
    /// read every zip again.
    sums: HashMap<(PathBuf, u64, std::time::SystemTime), String>,
    tx: Sender<News>,
    cancel: Arc<AtomicBool>,
}

fn run(
    dir: Option<PathBuf>,
    rack: Arc<Plugins>,
    rx: Receiver<Ask>,
    tx: Sender<News>,
    cancel: Arc<AtomicBool>,
) {
    let mut st = State {
        catalog: dir.as_deref().and_then(read_catalog),
        dir,
        rack,
        sums: HashMap::new(),
        tx,
        cancel,
    };
    while let Ok(mut a) = rx.recv() {
        // A run of picture asks: only the newest matters.
        while matches!(a, Ask::Picture { .. }) {
            match rx.try_recv() {
                Ok(next @ Ask::Picture { .. }) => a = next,
                Ok(other) => {
                    st.handle(a);
                    a = other;
                }
                Err(_) => break,
            }
        }
        st.handle(a);
    }
}

fn read_catalog(dir: &Path) -> Option<Arc<catalog::Catalog>> {
    let text = std::fs::read_to_string(dir.join(CATALOG_FILE)).ok()?;
    catalog::parse(&text).ok().map(Arc::new)
}

impl State {
    fn send(&self, n: News) {
        let _ = self.tx.send(n);
    }

    fn done(&self, key: &str, said: Result<String, String>, changed: bool) {
        self.send(News::Done {
            key: key.to_string(),
            said,
            changed,
        });
    }

    fn handle(&mut self, a: Ask) {
        match a {
            Ask::List => {
                let l = self.listing();
                self.send(News::Listing(Box::new(l)));
            }
            Ask::FetchCatalog => {
                let r = self.fetch_catalog();
                let ok = r.is_ok();
                self.done("catalog", r, false);
                if ok {
                    let l = self.listing();
                    self.send(News::Listing(Box::new(l)));
                }
            }
            Ask::Install { id } => {
                let r = self.install_from_catalog(&id);
                let changed = r.is_ok();
                self.done(&id, r, changed);
                let l = self.listing();
                self.send(News::Listing(Box::new(l)));
            }
            Ask::InstallZip { path } => {
                let key = path.display().to_string();
                let r = self.install_zip(&path);
                let changed = r.is_ok();
                self.done(&key, r, changed);
                let l = self.listing();
                self.send(News::Listing(Box::new(l)));
            }
            Ask::Favourites(list) => {
                if let Err(e) = self.keep_favourites(&list) {
                    self.done(
                        "favourites",
                        Err(format!("Favourites not kept: {e}")),
                        false,
                    );
                }
            }
            Ask::DeleteZip { path } => {
                let key = path.display().to_string();
                let r = match self.dir.as_deref() {
                    Some(d) if path.parent() == Some(d) => std::fs::remove_file(&path)
                        .map(|_| format!("Deleted {}", file_name(&path)))
                        .map_err(|e| format!("{}: {e}", file_name(&path))),
                    _ => Err("not a zip in the plugins folder".into()),
                };
                self.done(&key, r, false);
                let l = self.listing();
                self.send(News::Listing(Box::new(l)));
            }
            Ask::Remove { folder } => {
                let r = match self.dir.as_deref() {
                    Some(d) => install::remove(d, &folder).map(|r| {
                        let mut s = format!("Removed {folder}");
                        if r.kept > 0 {
                            s.push_str(&format!(
                                " ({} of your own files left in its folder)",
                                r.kept
                            ));
                        }
                        s.push_str(". A track already playing it keeps it until MPFree restarts.");
                        s
                    }),
                    None => Err("no plugins folder".into()),
                };
                self.done(&folder, r, false);
                let l = self.listing();
                self.send(News::Listing(Box::new(l)));
            }
            Ask::Picture {
                key,
                index,
                folder,
                url,
            } => {
                let n = self.picture(&key, index, folder.as_deref(), url.as_deref());
                self.send(n);
            }
        }
    }

    fn keep_favourites(&self, list: &[String]) -> Result<(), String> {
        let dir = self.dir.as_deref().ok_or("no plugins folder")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let part = dir.join(format!("{FAVOURITES}.part"));
        let mut text = list.join("\n");
        text.push('\n');
        std::fs::write(&part, text).map_err(|e| e.to_string())?;
        std::fs::rename(&part, dir.join(FAVOURITES)).map_err(|e| e.to_string())
    }

    fn fetch_catalog(&mut self) -> Result<String, String> {
        let dir = self.dir.clone().ok_or("no plugins folder")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let part = dir.join(format!("{CATALOG_FILE}.part"));
        let mut last = Err(String::new());
        for url in catalog::URLS {
            last = fetch::fetch(url, &part, 32 << 20, &mut |_, _| true);
            if last.is_ok() {
                break;
            }
        }
        last.map_err(|e| format!("The catalog didn't download: {e}"))?;
        let text = std::fs::read_to_string(&part).map_err(|e| e.to_string())?;
        let cat = catalog::parse(&text).inspect_err(|_| {
            let _ = std::fs::remove_file(&part);
        })?;
        std::fs::rename(&part, dir.join(CATALOG_FILE)).map_err(|e| e.to_string())?;
        let n = cat.runnable().count();
        let all = cat.plugins.len();
        self.catalog = Some(Arc::new(cat));
        Ok(format!(
            "The catalog lists {all} plugins; {n} run on this MPC"
        ))
    }

    fn install_from_catalog(&mut self, id: &str) -> Result<String, String> {
        let dir = self.dir.clone().ok_or("no plugins folder")?;
        let cat = self.catalog.clone().ok_or("no catalog yet")?;
        let p = cat
            .get(id)
            .ok_or_else(|| format!("{id} is not in the catalog"))?;
        let r = p
            .release
            .as_ref()
            .ok_or_else(|| format!("{} can't be installed here: {}", p.name, p.fit.why_not()))?;
        let downloads = dir.join(DOWNLOADS);
        std::fs::create_dir_all(&downloads).map_err(|e| format!("{}: {e}", downloads.display()))?;
        let file = downloads.join(format!("{id}-{}.zip", r.version.replace('/', "-")));
        let tx = self.tx.clone();
        let cancel = Arc::clone(&self.cancel);
        let key = id.to_string();
        let mut last = std::time::Instant::now();
        fetch::fetch(&r.url, &file, MAX_DOWNLOAD, &mut |done, total| {
            if last.elapsed().as_millis() >= 200 || (total > 0 && done == total) {
                last = std::time::Instant::now();
                let _ = tx.send(News::Progress {
                    key: key.clone(),
                    done,
                    total: if total == 0 { r.size } else { total },
                });
            }
            !cancel.load(Ordering::Relaxed)
        })
        .map_err(|e| format!("{} didn't download: {e}", p.name))?;
        let bytes = std::fs::read(&file).map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&file);
        let bytes = bytes?;
        let done = install::install(
            &bytes,
            &dir,
            Some(&r.sha256),
            install::Origin {
                id: &p.id,
                version: &r.version,
            },
        )?;
        self.added(&done, &format!("{} {}", p.name, r.version))
    }

    fn install_zip(&mut self, path: &Path) -> Result<String, String> {
        let dir = self.dir.clone().ok_or("no plugins folder")?;
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", file_name(path)))?;
        let sum = sha256::hex(&bytes);
        let cat = self.catalog.clone();
        let known = cat.as_deref().and_then(|c| c.by_sha256(&sum));
        let origin = known.map_or(install::Origin::default(), |(p, r)| install::Origin {
            id: &p.id,
            version: &r.version,
        });
        let done = install::install(&bytes, &dir, None, origin)?;
        let what = known.map_or_else(
            || file_name(path),
            |(p, r)| format!("{} {}", p.name, r.version),
        );
        self.added(&done, &what)
    }

    /// Offers what was unpacked from now on (the rack lists each folder).
    fn added(&self, done: &[install::Installed], what: &str) -> Result<String, String> {
        let mut names = Vec::new();
        let mut problems = Vec::new();
        for d in done {
            match self.rack.add(&d.folder) {
                Ok((entries, _)) => names.extend(entries.iter().map(|e| e.name.to_string())),
                Err(e) => problems.push(e),
            }
        }
        if names.is_empty() {
            return Err(format!(
                "{what} was unpacked but doesn't load: {}",
                problems.join("; ")
            ));
        }
        let replaced = done.iter().any(|d| d.replaced);
        Ok(format!(
            "{} {what}: {} ready on the Synth tab or in the chain editor",
            if replaced { "Updated" } else { "Installed" },
            names.join(", ")
        ))
    }

    fn listing(&mut self) -> Listing {
        let Some(dir) = self.dir.clone() else {
            return Listing {
                catalog: self.catalog.clone(),
                can_download: fetch::available(),
                no_folder: true,
                ..Listing::default()
            };
        };
        let cat = self.catalog.clone();
        // Installed: the rack's MPC plugin folders still there.
        let mut installed: Vec<InstalledPlugin> = Vec::new();
        for e in self.rack.entries() {
            if !e.node_id.starts_with("mpc:") || !e.file.is_dir() {
                continue;
            }
            if installed.iter().any(|i| i.path == e.file) {
                continue;
            }
            let folder = file_name(&e.file);
            let rec = install::Record::read(&e.file);
            let own = own_description(&e.file);
            let id = rec
                .as_ref()
                .map(|r| r.id.clone())
                .filter(|i| !i.is_empty())
                .or_else(|| {
                    cat.as_deref()
                        .and_then(|c| c.by_folder(&folder))
                        .map(|p| p.id.clone())
                })
                .unwrap_or_default();
            let version = rec
                .as_ref()
                .map(|r| r.version.clone())
                .filter(|v| !v.is_empty())
                .unwrap_or(own.version);
            installed.push(InstalledPlugin {
                maker: e.about.strip_prefix("Plugin · ").unwrap_or("").to_string(),
                name: e.name.to_string(),
                instrument: e.kind == mpfree_node::Kind::Instrument,
                folder,
                path: e.file.clone(),
                version,
                id,
                by_mpfree: rec.is_some(),
            });
        }
        // Zips on the stick.
        let mut zips = Vec::new();
        for path in install::zips(&dir) {
            let meta = std::fs::metadata(&path).ok();
            let bytes = meta.as_ref().map_or(0, |m| m.len());
            let mut z = ZipFile {
                file: file_name(&path),
                path: path.clone(),
                bytes,
                ..ZipFile::default()
            };
            if bytes > MAX_DOWNLOAD {
                z.problem = "too big to be a plugin".into();
                zips.push(z);
                continue;
            }
            let data = match std::fs::read(&path) {
                Ok(d) => d,
                Err(e) => {
                    z.problem = e.to_string();
                    zips.push(z);
                    continue;
                }
            };
            let stamp = meta.and_then(|m| m.modified().ok());
            z.sha256 = match stamp {
                Some(t) => self
                    .sums
                    .entry((path.clone(), bytes, t))
                    .or_insert_with(|| sha256::hex(&data))
                    .clone(),
                None => sha256::hex(&data),
            };
            describe_zip(&mut z, &data);
            if z.name.is_empty() {
                z.name = z.file.clone();
            }
            if let Some((p, r)) = cat.as_deref().and_then(|c| c.by_sha256(&z.sha256)) {
                z.id = p.id.clone();
                z.version = r.version.clone();
                z.name = p.name.clone();
                z.maker = p.author.clone();
                z.instrument = Some(p.kind == "instrument");
            }
            zips.push(z);
        }
        let favourites = std::fs::read_to_string(dir.join(FAVOURITES))
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
        Listing {
            catalog: cat,
            installed,
            zips,
            can_download: fetch::available(),
            no_folder: false,
            favourites,
        }
    }

    fn picture(
        &mut self,
        key: &str,
        index: usize,
        folder: Option<&Path>,
        url: Option<&str>,
    ) -> News {
        let tabs = folder.and_then(skin_tabs).unwrap_or(0);
        let count = tabs + usize::from(url.is_some());
        let news = |image: Option<Image>, why: String| News::Picture {
            key: key.to_string(),
            index,
            count,
            image: image.map(Arc::new),
            why,
        };
        if count == 0 {
            return news(None, "No picture".into());
        }
        let i = index % count;
        if i < tabs {
            let folder = folder.expect("tabs come from a folder");
            return match draw_tab(folder, i) {
                Ok(img) => news(Some(img), String::new()),
                Err(e) => news(None, e),
            };
        }
        let url = url.expect("the last picture is the catalog's");
        let Some(dir) = self.dir.clone() else {
            return news(None, "No picture".into());
        };
        // Named for the address, so every view of a plugin shares it.
        let cache = dir.join(DOWNLOADS).join(format!(
            "picture-{}.png",
            &sha256::hex(url.as_bytes())[..16]
        ));
        if !cache.is_file() {
            let _ = std::fs::create_dir_all(dir.join(DOWNLOADS));
            if let Err(e) = fetch::fetch(url, &cache, 16 << 20, &mut |_, _| true) {
                return news(None, format!("The catalog's picture needs a network ({e})"));
            }
        }
        match std::fs::read(&cache)
            .map_err(|e| e.to_string())
            .and_then(|b| mpfree_mpcskin::png::decode(&b))
        {
            Ok(p) => news(Some(fit(p.w as i32, p.h as i32, &p.px)), String::new()),
            Err(e) => {
                let _ = std::fs::remove_file(&cache);
                news(None, format!("The catalog's picture doesn't read: {e}"))
            }
        }
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[derive(Default)]
struct Own {
    version: String,
}

/// A folder's own version: plugin.json, then mpc-plugin.json.
fn own_description(folder: &Path) -> Own {
    for f in ["plugin.json", "mpc-plugin.json"] {
        if let Ok(t) = std::fs::read_to_string(folder.join(f)) {
            if let Ok(j) = json::parse(&t) {
                let v = j.at("version").str_or_empty();
                if !v.is_empty() {
                    return Own {
                        version: v.to_string(),
                    };
                }
            }
        }
    }
    Own::default()
}

/// Name, maker, kind and version from a zip's own `mpc-plugin.json` or
/// `plugin.json`, and its plugin folders.
fn describe_zip(z: &mut ZipFile, data: &[u8]) {
    let parsed = match zip::Zip::parse(data) {
        Ok(p) => p,
        Err(e) => {
            z.problem = format!("not a zip that reads ({e})");
            return;
        }
    };
    let found = install::plugin_folders(&parsed);
    if found.is_empty() {
        z.problem = "no MPC plugin folder in it".into();
        return;
    }
    z.folders = found.iter().map(|f| f.folder.clone()).collect();
    for e in &parsed.entries {
        let leaf = e.name.rsplit('/').next().unwrap_or("");
        if leaf != "mpc-plugin.json" && leaf != "plugin.json" {
            continue;
        }
        let Ok(bytes) = parsed.read(e, 1 << 20) else {
            continue;
        };
        let Ok(j) = json::parse(&String::from_utf8_lossy(&bytes)) else {
            continue;
        };
        let get = |k: &str| j.at(k).str_or_empty().to_string();
        if z.name.is_empty() {
            z.name = get("name");
        }
        if z.maker.is_empty() {
            z.maker = get("manufacturer");
        }
        if z.version.is_empty() {
            z.version = get("version");
        }
        if z.instrument.is_none() {
            match j.at("kind").str_or_empty() {
                "instrument" => z.instrument = Some(true),
                "effect" => z.instrument = Some(false),
                _ => {}
            }
        }
    }
    if z.name.is_empty() {
        z.name = z.folders[0].clone();
    }
}

/// How many tabs a plugin folder's skin has, if it has one that reads.
fn skin_tabs(folder: &Path) -> Option<usize> {
    let n = mpfree_mpcskin::view::guess_params(folder);
    let (skin, q) =
        mpfree_mpcskin::view::load(folder, &mpfree_mpcskin::view::MPC_ONE, n).ok()??;
    let v = mpfree_mpcskin::view::View::new(skin, q);
    let t = v.tabs().len();
    (t > 0).then_some(t)
}

struct Zero(usize);

impl Params for Zero {
    fn count(&self) -> usize {
        self.0
    }
    fn value(&self, _: usize) -> f32 {
        0.0
    }
    fn name(&self, i: usize) -> String {
        format!("Parameter {}", i + 1)
    }
    fn text(&self, _: usize) -> String {
        String::new()
    }
}

/// Tab `i` of a folder's skin, drawn at 1280 x 628 with every parameter
/// at 0 and scaled to the Selected column.
pub fn draw_tab(folder: &Path, i: usize) -> Result<Image, String> {
    use mpfree_mpcskin::view::{load, View, MPC_ONE, TAB_H, TAB_W};
    let n = mpfree_mpcskin::view::guess_params(folder);
    let (skin, q) = load(folder, &MPC_ONE, n)?.ok_or("no skin")?;
    let mut v = View::new(skin, q);
    v.set_tab(i);
    let (w, h) = (TAB_W as i32, TAB_H as i32);
    let mut c = SoftCanvas::new(w, h);
    v.draw(&Zero(n), &mut c, Rect::new(0, 0, w, h));
    Ok(fit(w, h, &c.px))
}

/// A picture scaled (area average) to fit [`PICTURE_W`] x [`PICTURE_H`],
/// keeping its shape.
pub fn fit(w: i32, h: i32, px: &[u32]) -> Image {
    let s = (PICTURE_W as f32 / w as f32)
        .min(PICTURE_H as f32 / h as f32)
        .min(1.0);
    let ow = ((w as f32 * s).round() as i32).max(1);
    let oh = ((h as f32 * s).round() as i32).max(1);
    let mut out = Image::new(ow, oh);
    for oy in 0..oh {
        let y0 = (oy as i64 * h as i64 / oh as i64) as i32;
        let y1 = (((oy + 1) as i64 * h as i64 / oh as i64) as i32)
            .max(y0 + 1)
            .min(h);
        for ox in 0..ow {
            let x0 = (ox as i64 * w as i64 / ow as i64) as i32;
            let x1 = (((ox + 1) as i64 * w as i64 / ow as i64) as i32)
                .max(x0 + 1)
                .min(w);
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = px[(y * w + x) as usize];
                    r += (p >> 16) & 0xff;
                    g += (p >> 8) & 0xff;
                    b += p & 0xff;
                    n += 1;
                }
            }
            out.px[(oy * ow + ox) as usize] = ((r / n) << 16) | ((g / n) << 8) | (b / n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_scaled_to_the_column_keeping_its_shape() {
        let px = vec![0x00ff_8040u32; 1280 * 628];
        let img = fit(1280, 628, &px);
        // 1280 x 628 fills the column's height; 395 of its 396 wide.
        assert_eq!((img.w, img.h), (PICTURE_W - 1, PICTURE_H));
        assert!(img.px.iter().all(|&p| p == 0x00ff_8040));
        let tall = fit(100, 400, &vec![0; 40_000]);
        assert_eq!(tall.h, PICTURE_H);
        assert!(tall.w < PICTURE_W);
        let small = fit(10, 5, &[0; 50]);
        assert_eq!((small.w, small.h), (10, 5), "never enlarged");
    }

    fn scratch(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("mpfree-plugin-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn wait(w: &Worker, want: impl Fn(&News) -> bool) -> Vec<News> {
        let start = std::time::Instant::now();
        let mut seen = Vec::new();
        while start.elapsed().as_secs() < 30 {
            for n in w.news() {
                let hit = want(&n);
                seen.push(n);
                if hit {
                    return seen;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("no answer: {seen:?}");
    }

    #[test]
    fn lists_a_zip_on_the_stick_and_refuses_one_that_is_not_a_plugin() {
        let dir = scratch("list");
        std::fs::write(dir.join("notes.zip"), b"not a zip").unwrap();
        let w = Worker::start(Some(dir.clone()), Arc::new(Plugins::none()));
        w.ask(Ask::List);
        let seen = wait(&w, |n| matches!(n, News::Listing(_)));
        let News::Listing(l) = seen.last().unwrap() else {
            unreachable!()
        };
        assert!(!l.no_folder);
        assert_eq!(l.zips.len(), 1);
        assert!(l.zips[0].problem.contains("not a zip"), "{:?}", l.zips[0]);
        assert_eq!(l.zips[0].name, "notes.zip");
        w.ask(Ask::InstallZip {
            path: dir.join("notes.zip"),
        });
        let seen = wait(&w, |n| matches!(n, News::Done { .. }));
        let Some(News::Done { said, changed, .. }) = seen.last() else {
            unreachable!()
        };
        assert!(said.is_err() && !changed, "{said:?}");
        w.ask(Ask::DeleteZip {
            path: dir.join("notes.zip"),
        });
        wait(&w, |n| matches!(n, News::Done { said: Ok(_), .. }));
        assert!(!dir.join("notes.zip").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_data_folder_it_says_so() {
        let w = Worker::start(None, Arc::new(Plugins::none()));
        w.ask(Ask::List);
        let seen = wait(&w, |n| matches!(n, News::Listing(_)));
        let News::Listing(l) = seen.last().unwrap() else {
            unreachable!()
        };
        assert!(l.no_folder);
        w.ask(Ask::Install { id: "x".into() });
        let seen = wait(&w, |n| matches!(n, News::Done { .. }));
        assert!(matches!(seen.last(), Some(News::Done { said: Err(_), .. })));
    }
}
