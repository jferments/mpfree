//! Finding MPC plugins on disk and reading what their folders say.
//!
//! An MPC plugin is a folder (`<Manufacturer> - VST - <Name>/`) holding a
//! shared library and its skins. Three kinds of description are in use,
//! and a folder is read from the first one it has:
//!
//! 1. **`plugin.json`**, inMusic's SDK format for MPC OS 3.10.1: `name`,
//!    `manufacturer`, `kind` or `isInstrument`, and the library in
//!    `file32` (32-bit ARM), `file64` (64-bit ARM) or `file` (either).
//! 2. **`plugin-meta.xml`**, the one-line `<PLUGIN .../>` entry that older
//!    MPC OS versions keep in their plugin list, as the Open MPC catalog's
//!    "portable" folders carry it: `file="%payload-path%/<folder>/x.so"`.
//! 3. **`mpc-plugin.json`**, the catalog's own manifest: the library in
//!    `so`.
//!
//! With none of them, a folder holding exactly one `.so` is taken as that
//! plugin. [`Folder::open`] also takes a `.so` file itself, but [`find`]
//! and [`is_mpc_plugin`] only count folders: a loose `.so` could be a CLAP
//! plugin.
//!
//! The library's path must stay inside the folder: an absolute path or
//! one that climbs out with `..` is refused, as the MPC refuses it.
//!
//! [`find`] looks through a plugins folder the way people unpack
//! plugins: plugin folders straight in it, a `VST2 Plugins/` or `Synths/`
//! folder copied from an MPC drive, and a catalog release folder (its
//! plugin folder sits in `portable/`).

use std::path::{Component, Path, PathBuf};

use mpfree_node::Kind;

use crate::json;

/// Where a folder's description came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Described {
    PluginJson,
    PluginMetaXml,
    CatalogJson,
    /// No description: the one `.so` in the folder, or a bare `.so`.
    Library,
}

/// One plugin folder as its description reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    /// The plugin folder (for a bare `.so`, the folder it is in).
    pub dir: PathBuf,
    /// The library to load, inside `dir`.
    pub library: PathBuf,
    /// Name and manufacturer as the description gives them (empty if it
    /// doesn't); the loaded library's own strings win.
    pub name: String,
    pub manufacturer: String,
    /// What the description says it is, if it says. An instrument here
    /// makes the plugin one even if its library doesn't flag itself a
    /// synth; otherwise the library's flags decide.
    pub kind: Option<Kind>,
    pub described: Described,
}

/// The library key `plugin.json` uses for this CPU.
#[cfg(all(target_arch = "arm", target_pointer_width = "32"))]
const NATIVE_KEY: Option<&str> = Some("file32");
#[cfg(target_arch = "aarch64")]
const NATIVE_KEY: Option<&str> = Some("file64");
#[cfg(not(any(target_arch = "arm", target_arch = "aarch64")))]
const NATIVE_KEY: Option<&str> = None;

impl Folder {
    /// Reads the plugin at `path`: a plugin folder or a `.so` file.
    pub fn open(path: &Path) -> Result<Folder, String> {
        if path.is_file() {
            if !is_library(path) {
                return Err(format!("{} is not a .so file", path.display()));
            }
            return Ok(Folder {
                dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
                library: path.to_path_buf(),
                name: String::new(),
                manufacturer: String::new(),
                kind: None,
                described: Described::Library,
            });
        }
        if !path.is_dir() {
            return Err(format!("{} isn't there", path.display()));
        }
        let read = |name: &str| std::fs::read_to_string(path.join(name)).ok();
        if let Some(t) = read("plugin.json") {
            return from_plugin_json(path, &t);
        }
        if let Some(t) = read("plugin-meta.xml") {
            return from_meta_xml(path, &t);
        }
        if let Some(t) = read("mpc-plugin.json") {
            return from_catalog_json(path, &t);
        }
        let libs = libraries_in(path);
        match libs.as_slice() {
            [one] => Ok(Folder {
                dir: path.to_path_buf(),
                library: one.clone(),
                name: String::new(),
                manufacturer: String::new(),
                kind: None,
                described: Described::Library,
            }),
            [] => Err(format!("{} holds no plugin", path.display())),
            _ => Err(format!(
                "{} holds {} .so files and no plugin.json saying which to load",
                path.display(),
                libs.len()
            )),
        }
    }
}

fn is_library(p: &Path) -> bool {
    p.extension().is_some_and(|x| x == "so")
}

fn libraries_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_library(p))
        .collect();
    v.sort();
    v
}

/// `rel` inside `dir`, refusing absolute paths and `..`.
fn inside(dir: &Path, rel: &str) -> Result<PathBuf, String> {
    let r = Path::new(rel);
    if rel.is_empty()
        || r.components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(format!(
            "the library path {rel:?} must be inside the plugin folder"
        ));
    }
    let p = dir.join(r);
    if !p.is_file() {
        return Err(format!("{} isn't there", p.display()));
    }
    Ok(p)
}

fn kind_word(s: &str) -> Option<Kind> {
    match s.to_ascii_lowercase().as_str() {
        "instrument" => Some(Kind::Instrument),
        "effect" => Some(Kind::Effect),
        _ => None,
    }
}

fn from_plugin_json(dir: &Path, text: &str) -> Result<Folder, String> {
    let o = json::object(text).ok_or("plugin.json isn't a JSON object")?;
    let s = |k: &str| o.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let rel = match NATIVE_KEY.and_then(&s) {
        Some(r) => r,
        None if o.contains_key("file32") || o.contains_key("file64") => {
            return Err(match NATIVE_KEY {
                Some(k) => format!("plugin.json has no {k} library for this CPU"),
                None => "plugin.json has only MPC (ARM) libraries".to_string(),
            })
        }
        None => s("file").ok_or("plugin.json names no library")?,
    };
    let mut kind = s("kind").as_deref().and_then(kind_word);
    if let Some(b) = o.get("isInstrument").and_then(|v| v.as_bool()) {
        kind = Some(if b { Kind::Instrument } else { Kind::Effect });
    }
    Ok(Folder {
        dir: dir.to_path_buf(),
        library: inside(dir, &rel)?,
        name: s("name").unwrap_or_default(),
        manufacturer: s("manufacturer").unwrap_or_default(),
        kind,
        described: Described::PluginJson,
    })
}

/// The value of attribute `name` in an XML start tag, with the five
/// standard entities and numeric references undone.
pub fn xml_attr(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    loop {
        let at = rest.find(name)?;
        let before = rest[..at].chars().next_back();
        let after = &rest[at + name.len()..];
        rest = after;
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let after = after.trim_start();
        let Some(after) = after.strip_prefix('=') else {
            continue;
        };
        let after = after.trim_start();
        let q = after.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let body = &after[1..];
        let end = body.find(q)?;
        return Some(unescape_xml(&body[..end]));
    }
}

fn unescape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest.find(';') else { break };
        let ent = &rest[1..semi];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn from_meta_xml(dir: &Path, text: &str) -> Result<Folder, String> {
    let start = text
        .find("<PLUGIN")
        .ok_or("plugin-meta.xml has no PLUGIN entry")?;
    let tag = &text[start..];
    let tag = &tag[..tag.find('>').ok_or("plugin-meta.xml is cut short")?];
    let file = xml_attr(tag, "file").ok_or("plugin-meta.xml names no file")?;
    // "%payload-path%/<folder>/lib.so": the part after the folder's name.
    let rel = match file.strip_prefix("%payload-path%/") {
        Some(r) => r.split_once('/').map(|(_, lib)| lib.to_string()),
        None => Path::new(&file)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned()),
    }
    .ok_or("plugin-meta.xml's file isn't a library in the folder")?;
    let kind = xml_attr(tag, "isInstrument").map(|v| {
        if v.trim() == "1" {
            Kind::Instrument
        } else {
            Kind::Effect
        }
    });
    Ok(Folder {
        dir: dir.to_path_buf(),
        library: inside(dir, &rel)?,
        name: xml_attr(tag, "name").unwrap_or_default(),
        manufacturer: xml_attr(tag, "manufacturer").unwrap_or_default(),
        kind,
        described: Described::PluginMetaXml,
    })
}

fn from_catalog_json(dir: &Path, text: &str) -> Result<Folder, String> {
    let o = json::object(text).ok_or("mpc-plugin.json isn't a JSON object")?;
    let s = |k: &str| o.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let rel = s("so").ok_or("mpc-plugin.json names no library")?;
    Ok(Folder {
        dir: dir.to_path_buf(),
        library: inside(dir, &rel)?,
        name: s("name").unwrap_or_default(),
        manufacturer: s("manufacturer").unwrap_or_default(),
        kind: s("kind").as_deref().and_then(kind_word),
        described: Described::CatalogJson,
    })
}

/// Whether `dir` looks like a plugin folder (it has a description, or a
/// `.so` straight inside it).
pub fn is_plugin_folder(dir: &Path) -> bool {
    dir.is_dir()
        && (["plugin.json", "plugin-meta.xml", "mpc-plugin.json"]
            .iter()
            .any(|f| dir.join(f).is_file())
            || !libraries_in(dir).is_empty())
}

/// Whether `path` is an MPC plugin rather than a CLAP file: a plugin
/// folder. (A loose `.so` could be either format, so MPC plugins are
/// always found as folders, as the MPC and the catalog ship them.)
pub fn is_mpc_plugin(path: &Path) -> bool {
    is_plugin_folder(path)
}

/// The MPC plugins in `dir`, sorted: plugin folders in it, and those one
/// level inside a `VST2 Plugins/` or `Synths/` folder or a
/// catalog release folder's `portable/`. Nothing deeper is searched.
pub fn find(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries = |d: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(d)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        v.sort();
        v
    };
    for p in entries(dir) {
        if !p.is_dir() {
            continue;
        }
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
        let portable = p.join("portable");
        if matches!(name.as_deref(), Some("VST2 Plugins" | "Synths")) {
            out.extend(entries(&p).into_iter().filter(|q| is_plugin_folder(q)));
        } else if portable.is_dir() {
            out.extend(
                entries(&portable)
                    .into_iter()
                    .filter(|q| is_plugin_folder(q)),
            );
        } else if is_plugin_folder(&p) {
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mpchost-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn plugin_json_picks_this_cpus_library_or_file() {
        let d = tmp("pj");
        put(&d.join("VST/x.so"), "");
        put(
            &d.join("plugin.json"),
            r#"{"name": "Wobble", "manufacturer": "Acme", "kind": "Effect", "file": "VST/x.so"}"#,
        );
        let f = Folder::open(&d).unwrap();
        assert_eq!(f.library, d.join("VST/x.so"));
        assert_eq!(
            (f.name.as_str(), f.manufacturer.as_str()),
            ("Wobble", "Acme")
        );
        assert_eq!(f.kind, Some(Kind::Effect));
        assert_eq!(f.described, Described::PluginJson);
        put(
            &d.join("plugin.json"),
            r#"{"name": "W", "isInstrument": true, "kind": "effect", "file32": "VST/x.so", "file64": "VST/x.so"}"#,
        );
        match NATIVE_KEY {
            Some(_) => assert_eq!(Folder::open(&d).unwrap().kind, Some(Kind::Instrument)),
            None => assert!(Folder::open(&d)
                .unwrap_err()
                .contains("only MPC (ARM) libraries")),
        }
        // No kind: the library's own flags decide.
        put(
            &d.join("plugin.json"),
            r#"{"name": "W", "file": "VST/x.so"}"#,
        );
        assert_eq!(Folder::open(&d).unwrap().kind, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_library_must_be_inside_the_folder() {
        let d = tmp("inside");
        put(&d.join("x.so"), "");
        put(&d.join("sub/plugin.json"), r#"{"file": "../x.so"}"#);
        assert!(Folder::open(&d.join("sub"))
            .unwrap_err()
            .contains("inside the plugin folder"));
        let abs = d.join("x.so");
        put(
            &d.join("sub/plugin.json"),
            &format!(r#"{{"file": "{}"}}"#, abs.display()),
        );
        assert!(Folder::open(&d.join("sub")).is_err());
        put(&d.join("sub/plugin.json"), r#"{"file": "missing.so"}"#);
        assert!(Folder::open(&d.join("sub"))
            .unwrap_err()
            .contains("isn't there"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn plugin_meta_xml_as_the_catalog_ships_it() {
        let d = tmp("meta").join("Airwindows - VST - Density2");
        put(&d.join("density2.so"), "");
        put(
            &d.join("plugin-meta.xml"),
            r#"<PLUGIN name="Density2" descriptiveName="Density2" format="VST" category="Effect" manufacturer="Air &amp; Windows" version="1.0" file="%payload-path%/Airwindows - VST - Density2/density2.so" uid="4177586e" isInstrument="0" numInputs="2" numOutputs="2" isShell="0"/>"#,
        );
        let f = Folder::open(&d).unwrap();
        assert_eq!(f.library, d.join("density2.so"));
        assert_eq!(f.name, "Density2");
        assert_eq!(f.manufacturer, "Air & Windows");
        assert_eq!(f.kind, Some(Kind::Effect));
        assert_eq!(f.described, Described::PluginMetaXml);
        // descriptiveName doesn't count as name.
        assert_eq!(
            xml_attr(r#"<P descriptiveName="a" name="b""#, "name").as_deref(),
            Some("b")
        );
        assert_eq!(
            xml_attr("<P n='&#x41;&#66;&bogus;'", "n").as_deref(),
            Some("AB&bogus;")
        );
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    #[test]
    fn catalog_json_and_a_lone_library() {
        let d = tmp("cat");
        put(&d.join("a/synth.so"), "");
        put(
            &d.join("a/mpc-plugin.json"),
            r#"{"schema": 1, "name": "[SYN] Mono Voice", "kind": "instrument", "manufacturer": "timncox", "so": "synth.so", "extras": []}"#,
        );
        let f = Folder::open(&d.join("a")).unwrap();
        assert_eq!(f.kind, Some(Kind::Instrument));
        assert_eq!(f.described, Described::CatalogJson);
        put(&d.join("b/only.so"), "");
        let f = Folder::open(&d.join("b")).unwrap();
        assert_eq!(f.library, d.join("b/only.so"));
        assert_eq!(f.described, Described::Library);
        put(&d.join("b/two.so"), "");
        assert!(Folder::open(&d.join("b"))
            .unwrap_err()
            .contains("2 .so files"));
        let f = Folder::open(&d.join("b/two.so")).unwrap();
        assert_eq!(f.dir, d.join("b"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn find_looks_where_people_unpack_plugins() {
        let d = tmp("find");
        put(&d.join("Acme - VST - A/plugin.json"), "{}");
        put(&d.join("VST2 Plugins/Acme - VST - B/plugin.json"), "{}");
        put(
            &d.join("Synths/Acme - VST - C/plugin-meta.xml"),
            "<PLUGIN/>",
        );
        put(
            &d.join("D-1.0.0/portable/Acme - VST - D/mpc-plugin.json"),
            "{}",
        );
        put(&d.join("D-1.0.0/mpc-plugin.json"), "{}");
        put(&d.join("e.so"), "");
        put(&d.join("synth.clap"), "");
        put(&d.join("notes/readme.txt"), "");
        put(&d.join("deep/deeper/Acme - VST - F/plugin.json"), "{}");
        let found: Vec<String> = find(&d)
            .iter()
            .map(|p| p.strip_prefix(&d).unwrap().display().to_string())
            .collect();
        assert_eq!(
            found,
            [
                "Acme - VST - A",
                "D-1.0.0/portable/Acme - VST - D",
                "Synths/Acme - VST - C",
                "VST2 Plugins/Acme - VST - B",
            ]
        );
        assert!(!is_mpc_plugin(&d.join("e.so")), "a loose .so may be CLAP");
        assert!(is_mpc_plugin(&d.join("Acme - VST - A")));
        assert!(!is_mpc_plugin(&d.join("synth.clap")));
        assert!(!is_mpc_plugin(&d.join("notes")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
