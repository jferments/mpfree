//! A plugin's own presets: finding them (CLAP preset discovery) and the
//! description a host keeps of each one, for loading it later
//! ([`crate::ClapControl::load_preset`], CLAP preset-load).
//!
//! [`crate::Library::presets`] asks each preset provider in a plugin file
//! where its presets are, walks those folders (sorted, so the list comes
//! out the same each time) and asks the provider about each file, all on
//! the calling thread. A provider may take seconds (Surge XT reads its
//! whole data folder first), and a bad one could crash, so MPFree runs it
//! in a process of its own (the plugin sandbox's `--presets` mode).
//!
//! Limits keep a huge or looping folder from running away: at most
//! [`MAX_PRESETS`] presets, [`MAX_FILES`] files looked at, [`MAX_DEPTH`]
//! folders deep, and symbolic links to folders are not followed.

use crate::sys::*;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;

/// At most this many presets from one plugin file.
pub const MAX_PRESETS: usize = 4096;
/// At most this many files looked at in one plugin file's folders.
pub const MAX_FILES: usize = 16384;
/// Folders below a location deeper than this are not walked.
pub const MAX_DEPTH: usize = 8;
/// Errors kept (the rest are counted).
const MAX_ERRORS: usize = 16;
/// Providers asked, per plugin file.
const MAX_PROVIDERS: u32 = 64;

/// Where a preset lives.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    /// A file on disk (the plugin's own format).
    File(PathBuf),
    /// Inside the plugin itself.
    Plugin,
}

/// One preset, as the plugin's provider described it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    pub name: String,
    pub place: Place,
    /// Which preset inside the file (or inside the plugin), when the place
    /// holds more than one.
    pub load_key: Option<String>,
    /// The CLAP plugin ids it is for; empty means any plugin in the file.
    pub plugins: Vec<String>,
    /// The name of the location it was found in ("Surge XT Factory
    /// Presets").
    pub collection: String,
    /// The provider's category, or else the folders between the location
    /// and the file ("Leads", "Pads / Warm"); empty at the top.
    pub category: String,
    /// The first creator the provider named.
    pub creator: String,
    /// `CLAP_PRESET_DISCOVERY_IS_*` bits: the preset's, or else its
    /// location's.
    pub flags: u32,
}

impl Preset {
    /// Whether this preset is for plugin `id`.
    pub fn is_for(&self, id: &str) -> bool {
        self.plugins.is_empty() || self.plugins.iter().any(|p| p == id)
    }

    /// Whether it came with the plugin (rather than from the user).
    pub fn is_factory(&self) -> bool {
        self.flags & CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT != 0
    }

    /// Whether the user made it.
    pub fn is_user(&self) -> bool {
        self.flags & CLAP_PRESET_DISCOVERY_IS_USER_CONTENT != 0
    }
}

/// What a search found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Found {
    pub presets: Vec<Preset>,
    /// What went wrong (the first few): a file the provider couldn't read,
    /// a provider that wouldn't start.
    pub errors: Vec<String>,
    /// Errors beyond those kept.
    pub more_errors: usize,
    /// True when a limit stopped the search, so the list is not complete.
    pub cut_short: bool,
}

impl Found {
    fn error(&mut self, e: String) {
        if self.errors.len() < MAX_ERRORS {
            self.errors.push(e);
        } else {
            self.more_errors += 1;
        }
    }
}

// ------------------------------------------------------------------ indexer

/// A filetype a provider declared: its extension, lowercase, or None for
/// every file.
type Filetype = Option<String>;

struct Location {
    flags: u32,
    name: String,
    kind: u32,
    location: Option<String>,
}

#[derive(Default)]
struct Index {
    filetypes: Vec<Filetype>,
    locations: Vec<Location>,
}

unsafe fn text(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = CStr::from_ptr(p).to_string_lossy().into_owned();
    (!s.is_empty()).then_some(s)
}

unsafe fn index<'a>(i: *const clap_preset_discovery_indexer) -> Option<&'a mut Index> {
    if i.is_null() || (*i).indexer_data.is_null() {
        None
    } else {
        Some(&mut *((*i).indexer_data as *mut Index))
    }
}

unsafe extern "C" fn ix_filetype(
    i: *const clap_preset_discovery_indexer,
    t: *const clap_preset_discovery_filetype,
) -> bool {
    let Some(ix) = index(i) else { return false };
    if t.is_null() || ix.filetypes.len() >= 64 {
        return false;
    }
    let ext = text((*t).file_extension).map(|e| e.trim_start_matches('.').to_lowercase());
    ix.filetypes.push(ext.filter(|e| !e.is_empty()));
    true
}

unsafe extern "C" fn ix_location(
    i: *const clap_preset_discovery_indexer,
    l: *const clap_preset_discovery_location,
) -> bool {
    let Some(ix) = index(i) else { return false };
    if l.is_null() || ix.locations.len() >= 64 {
        return false;
    }
    let l = &*l;
    let location = text(l.location);
    let valid = match l.kind {
        CLAP_PRESET_DISCOVERY_LOCATION_FILE => location.is_some(),
        CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN => true,
        _ => false,
    };
    if !valid {
        return false;
    }
    ix.locations.push(Location {
        flags: l.flags,
        name: text(l.name).unwrap_or_default(),
        kind: l.kind,
        location,
    });
    true
}

unsafe extern "C" fn ix_soundpack(
    _i: *const clap_preset_discovery_indexer,
    s: *const clap_preset_discovery_soundpack,
) -> bool {
    // Sound packs aren't shown yet; accepted so a provider carries on.
    !s.is_null()
}

unsafe extern "C" fn ix_extension(
    _i: *const clap_preset_discovery_indexer,
    _id: *const c_char,
) -> *const c_void {
    ptr::null()
}

// ------------------------------------------------------------------ receiver

/// What one `get_metadata` call adds to.
struct Receive<'a> {
    found: &'a mut Found,
    /// What each new preset starts as: its place, collection, folder
    /// category and flags.
    base: &'a Preset,
    /// The preset being described (an index into `found.presets`).
    current: Option<usize>,
}

unsafe fn receive<'a>(
    r: *const clap_preset_discovery_metadata_receiver,
) -> Option<&'a mut Receive<'a>> {
    if r.is_null() || (*r).receiver_data.is_null() {
        None
    } else {
        Some(&mut *((*r).receiver_data as *mut Receive))
    }
}

unsafe fn current<'a>(r: *const clap_preset_discovery_metadata_receiver) -> Option<&'a mut Preset> {
    let rc = receive(r)?;
    let k = rc.current?;
    rc.found.presets.get_mut(k)
}

unsafe extern "C" fn rc_error(
    r: *const clap_preset_discovery_metadata_receiver,
    os_error: i32,
    msg: *const c_char,
) {
    let Some(rc) = receive(r) else { return };
    let mut e = text(msg).unwrap_or_else(|| "the plugin couldn't read a preset".into());
    if os_error != 0 {
        e.push_str(&format!(" (error {os_error})"));
    }
    rc.found.error(e);
}

unsafe extern "C" fn rc_begin(
    r: *const clap_preset_discovery_metadata_receiver,
    name: *const c_char,
    load_key: *const c_char,
) -> bool {
    let Some(rc) = receive(r) else { return false };
    if rc.found.presets.len() >= MAX_PRESETS {
        rc.found.cut_short = true;
        rc.current = None;
        return false;
    }
    let mut p = rc.base.clone();
    p.name = text(name)
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| match &p.place {
            Place::File(f) => f
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Place::Plugin => String::new(),
        });
    if p.name.is_empty() {
        p.name = format!("Preset {}", rc.found.presets.len() + 1);
    }
    p.load_key = text(load_key);
    rc.found.presets.push(p);
    rc.current = Some(rc.found.presets.len() - 1);
    true
}

unsafe extern "C" fn rc_plugin_id(
    r: *const clap_preset_discovery_metadata_receiver,
    id: *const clap_universal_plugin_id,
) {
    if id.is_null() {
        return;
    }
    let abi = text((*id).abi);
    let clap = abi
        .as_deref()
        .is_none_or(|a| a.eq_ignore_ascii_case("clap"));
    if let (true, Some(id), Some(p)) = (clap, text((*id).id), current(r)) {
        if p.plugins.len() < 16 && !p.plugins.contains(&id) {
            p.plugins.push(id);
        }
    }
}

unsafe extern "C" fn rc_soundpack(
    _r: *const clap_preset_discovery_metadata_receiver,
    _id: *const c_char,
) {
}

unsafe extern "C" fn rc_flags(r: *const clap_preset_discovery_metadata_receiver, flags: u32) {
    if let Some(p) = current(r) {
        p.flags = flags;
    }
}

unsafe extern "C" fn rc_creator(
    r: *const clap_preset_discovery_metadata_receiver,
    c: *const c_char,
) {
    if let (Some(p), Some(c)) = (current(r), text(c)) {
        if p.creator.is_empty() {
            p.creator = c;
        }
    }
}

unsafe extern "C" fn rc_description(
    _r: *const clap_preset_discovery_metadata_receiver,
    _d: *const c_char,
) {
}

unsafe extern "C" fn rc_timestamps(
    _r: *const clap_preset_discovery_metadata_receiver,
    _created: u64,
    _modified: u64,
) {
}

unsafe extern "C" fn rc_feature(
    _r: *const clap_preset_discovery_metadata_receiver,
    _f: *const c_char,
) {
}

unsafe extern "C" fn rc_extra(
    r: *const clap_preset_discovery_metadata_receiver,
    key: *const c_char,
    value: *const c_char,
) {
    if let (Some(p), Some(k), Some(v)) = (current(r), text(key), text(value)) {
        if k == "category" {
            p.category = v.trim().to_string();
        }
    }
}

// ------------------------------------------------------------------ search

unsafe fn factory(entry: *const clap_plugin_entry) -> *const clap_preset_discovery_factory {
    let Some(get) = (*entry).get_factory else {
        return ptr::null();
    };
    for id in [
        CLAP_PRESET_DISCOVERY_FACTORY_ID,
        CLAP_PRESET_DISCOVERY_FACTORY_ID_COMPAT,
    ] {
        let f = get(id.as_ptr() as *const c_char);
        if !f.is_null() {
            return f as *const clap_preset_discovery_factory;
        }
    }
    ptr::null()
}

/// Every preset the file's providers know of.
///
/// # Safety
/// `entry` is a started (`init`) plugin entry; call on its main thread.
pub(crate) unsafe fn discover(entry: *const clap_plugin_entry) -> Found {
    let mut found = Found::default();
    let f = factory(entry);
    if f.is_null() {
        return found;
    }
    let n = (*f).count.map_or(0, |c| c(f)).min(MAX_PROVIDERS);
    for i in 0..n {
        if found.cut_short {
            break;
        }
        let d = (*f).get_descriptor.map_or(ptr::null(), |g| g(f, i));
        if d.is_null() || (*d).id.is_null() {
            continue;
        }
        let pid: CString = CStr::from_ptr((*d).id).to_owned();
        let name = text((*d).name).unwrap_or_else(|| pid.to_string_lossy().into_owned());
        // The indexer outlives the provider: it's dropped after `destroy`.
        let idx = Box::into_raw(Box::<Index>::default());
        let indexer = clap_preset_discovery_indexer {
            clap_version: CLAP_VERSION,
            name: c"MPFree".as_ptr(),
            vendor: c"MPFree".as_ptr(),
            url: c"https://github.com/jferments/mpfree".as_ptr(),
            version: c"0.1.0".as_ptr(),
            indexer_data: idx as *mut c_void,
            declare_filetype: Some(ix_filetype),
            declare_location: Some(ix_location),
            declare_soundpack: Some(ix_soundpack),
            get_extension: Some(ix_extension),
        };
        let prov = (*f)
            .create
            .map_or(ptr::null(), |c| c(f, &indexer, pid.as_ptr()));
        if prov.is_null() {
            found.error(format!("{name}: the preset provider wasn't made"));
            drop(Box::from_raw(idx));
            continue;
        }
        if (*prov).init.is_some_and(|init| init(prov)) {
            let (types, locs) = {
                let ix = &mut *idx;
                (
                    std::mem::take(&mut ix.filetypes),
                    std::mem::take(&mut ix.locations),
                )
            };
            crawl(prov, &types, &locs, &mut found);
        } else {
            found.error(format!("{name}: the preset provider didn't start"));
        }
        if let Some(destroy) = (*prov).destroy {
            destroy(prov);
        }
        drop(Box::from_raw(idx));
    }
    found
}

/// Whether a file's name matches one of the declared types.
fn wanted(types: &[Filetype], file: &Path) -> bool {
    if types.is_empty() || types.iter().any(Option::is_none) {
        return true;
    }
    let Some(ext) = file.extension() else {
        return false;
    };
    let ext = ext.to_string_lossy().to_lowercase();
    types.iter().flatten().any(|t| *t == ext)
}

unsafe fn crawl(
    prov: *const clap_preset_discovery_provider,
    types: &[Filetype],
    locs: &[Location],
    found: &mut Found,
) {
    let mut files = 0usize;
    for loc in locs {
        if found.cut_short {
            return;
        }
        let base = Preset {
            name: String::new(),
            place: Place::Plugin,
            load_key: None,
            plugins: Vec::new(),
            collection: loc.name.clone(),
            category: String::new(),
            creator: String::new(),
            flags: loc.flags,
        };
        match (loc.kind, &loc.location) {
            (CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN, _) => {
                read(
                    prov,
                    CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN,
                    None,
                    &base,
                    found,
                );
            }
            (CLAP_PRESET_DISCOVERY_LOCATION_FILE, Some(path)) => {
                let path = PathBuf::from(path);
                if path.is_dir() {
                    walk(prov, types, &path, &path, 0, &base, &mut files, found);
                } else if path.is_file() {
                    files += 1;
                    let base = Preset {
                        place: Place::File(path.clone()),
                        ..base
                    };
                    read(
                        prov,
                        CLAP_PRESET_DISCOVERY_LOCATION_FILE,
                        Some(&path),
                        &base,
                        found,
                    );
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn walk(
    prov: *const clap_preset_discovery_provider,
    types: &[Filetype],
    top: &Path,
    dir: &Path,
    depth: usize,
    base: &Preset,
    files: &mut usize,
    found: &mut Found,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        found.error(format!("{}: the folder couldn't be read", dir.display()));
        return;
    };
    let mut entries: Vec<std::fs::DirEntry> = rd.flatten().collect();
    entries.sort_by_key(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        (n.to_lowercase(), n)
    });
    for e in entries {
        if found.cut_short {
            return;
        }
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if depth < MAX_DEPTH {
                walk(prov, types, top, &path, depth + 1, base, files, found);
            }
            continue;
        }
        // A file, or a link to one (links to folders are not followed).
        if !(ft.is_file() || (ft.is_symlink() && path.is_file())) || !wanted(types, &path) {
            continue;
        }
        *files += 1;
        if *files > MAX_FILES {
            found.cut_short = true;
            return;
        }
        let category = path
            .parent()
            .and_then(|p| p.strip_prefix(top).ok())
            .map(|rel| {
                rel.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .unwrap_or_default();
        let base = Preset {
            place: Place::File(path.clone()),
            category,
            ..base.clone()
        };
        read(
            prov,
            CLAP_PRESET_DISCOVERY_LOCATION_FILE,
            Some(&path),
            &base,
            found,
        );
    }
}

/// Asks the provider about one place.
unsafe fn read(
    prov: *const clap_preset_discovery_provider,
    kind: u32,
    path: Option<&Path>,
    base: &Preset,
    found: &mut Found,
) {
    let Some(get) = (*prov).get_metadata else {
        return;
    };
    let cpath = match path {
        Some(p) => match CString::new(p.as_os_str().as_encoded_bytes()) {
            Ok(c) => Some(c),
            Err(_) => return,
        },
        None => None,
    };
    let before = found.errors.len() + found.more_errors;
    let mut rc = Receive {
        found,
        base,
        current: None,
    };
    let receiver = clap_preset_discovery_metadata_receiver {
        receiver_data: &mut rc as *mut Receive as *mut c_void,
        on_error: Some(rc_error),
        begin_preset: Some(rc_begin),
        add_plugin_id: Some(rc_plugin_id),
        set_soundpack_id: Some(rc_soundpack),
        set_flags: Some(rc_flags),
        add_creator: Some(rc_creator),
        set_description: Some(rc_description),
        set_timestamps: Some(rc_timestamps),
        add_feature: Some(rc_feature),
        add_extra_info: Some(rc_extra),
    };
    let ok = get(
        prov,
        kind,
        cpath.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
        &receiver,
    );
    let found = rc.found;
    let said = found.errors.len() + found.more_errors > before;
    if !ok && !said && !found.cut_short {
        let what = path.map_or("its built-in presets".into(), |p| p.display().to_string());
        found.error(format!("the plugin couldn't read {what}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_match_extensions_without_case() {
        let t = vec![Some("fxp".to_string())];
        assert!(wanted(&t, Path::new("/a/Lead.fxp")));
        assert!(wanted(&t, Path::new("/a/Lead.FXP")));
        assert!(!wanted(&t, Path::new("/a/Lead.wav")));
        assert!(!wanted(&t, Path::new("/a/fxp")));
        assert!(
            wanted(&[], Path::new("/a/anything")),
            "no types: every file"
        );
        assert!(wanted(&[Some("x".into()), None], Path::new("/a/b.y")));
    }

    #[test]
    fn a_preset_is_for_its_plugins() {
        let mut p = Preset {
            name: "A".into(),
            place: Place::Plugin,
            load_key: None,
            plugins: Vec::new(),
            collection: String::new(),
            category: String::new(),
            creator: String::new(),
            flags: CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT,
        };
        assert!(p.is_for("x") && p.is_factory() && !p.is_user());
        p.plugins.push("y".into());
        assert!(p.is_for("y") && !p.is_for("x"));
    }
}
