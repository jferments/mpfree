//! A program folder made into a project in the library: the `.xpm` read,
//! its samples found beside it, the kit made ([`crate::kit`]) and saved
//! as a new project with only the sampler's part, so opening it puts the
//! kit on the pads.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mpfree_library::project::KEEP_VERSIONS;
use mpfree_library::{clean_name, Library, Loc, Meta, NewSample, Place, Snapshot};
use mpfree_sampler::model::Source;
use mpfree_sampler::{save, sound_file, wav};

use crate::kit::{keygroup_kit, note_text, Options};
use crate::xml::MAX_BYTES;
use crate::xpm::{read_keygroup, XpmLayer};

/// The largest sample file read.
pub const MAX_SAMPLE_FILE: u64 = 1 << 30;
/// The sampler's part in a project.
pub const SAMPLER_PART: &str = "sampler";
/// Sample file endings tried after a sample's name.
const ENDINGS: [&str; 4] = ["wav", "aif", "aiff", "flac"];

/// What an import did.
#[derive(Debug)]
pub struct Report {
    /// Where the project is (or would be, for a dry run).
    pub project: PathBuf,
    pub lines: Vec<String>,
}

/// The files in `dir` by lower-case name (not links, not folders).
fn files_in(dir: &Path) -> Result<HashMap<String, PathBuf>, String> {
    let mut out = HashMap::new();
    let rd = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        if t.is_file() {
            out.insert(e.file_name().to_string_lossy().to_lowercase(), e.path());
        }
    }
    Ok(out)
}

/// A layer's sample file beside the program: its `SampleFile`'s name,
/// else its sample name with a sound file ending, any case.
fn find(l: &XpmLayer, files: &HashMap<String, PathBuf>) -> Option<PathBuf> {
    let mut keys = Vec::new();
    if !l.file.is_empty() {
        let f = l.file.replace('\\', "/");
        keys.push(f.rsplit('/').next().unwrap_or("").to_lowercase());
    }
    if !l.sample.is_empty() {
        let low = l.sample.to_lowercase();
        keys.extend(ENDINGS.iter().map(|e| format!("{low}.{e}")));
        keys.push(low);
    }
    keys.iter().find_map(|k| files.get(k).cloned())
}

fn read_capped(path: &Path, cap: u64) -> Result<Vec<u8>, String> {
    let f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut b = Vec::new();
    f.take(cap + 1)
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    if b.len() as u64 > cap {
        return Err(format!("bigger than {cap} bytes"));
    }
    Ok(b)
}

/// Imports the keygroup program at `xpm` as a new project named `name`
/// (the program's name when None) in the library in `data` (the folder
/// the app is given with `--data`). A dry run reads everything and makes
/// the kit but writes nothing. An existing project is never replaced.
pub fn import_keygroup(
    xpm: &Path,
    data: &Path,
    name: Option<&str>,
    opts: &Options,
    dry_run: bool,
) -> Result<Report, String> {
    let bytes =
        read_capped(xpm, MAX_BYTES as u64).map_err(|e| format!("{}: {e}", xpm.display()))?;
    let prog = read_keygroup(&bytes).map_err(|e| format!("{}: {e}", xpm.display()))?;
    let dir = xpm
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let files = files_in(dir)?;
    let stem = xpm
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = clean_name(
        name.unwrap_or(if prog.name.is_empty() {
            &stem
        } else {
            &prog.name
        }),
        "Imported keygroup",
    );
    let loc =
        Loc::new(Place::Projects, &[&name]).map_err(|e| format!("project name {name:?}: {e}"))?;

    let mut lines = vec![
        format!(
            "program: {} ({})",
            xpm.display(),
            if prog.name.is_empty() {
                "no name"
            } else {
                &prog.name
            }
        ),
        format!("keygroups with a sample: {}", prog.keygroups.len()),
    ];
    let mut cache: HashMap<PathBuf, Source> = HashMap::new();
    let mut read_notes = Vec::new();
    let mut bytes_in = 0u64;
    let mut sound = |l: &XpmLayer| -> Result<Source, String> {
        let path = find(l, &files).ok_or("its file is not beside the program")?;
        if let Some(s) = cache.get(&path) {
            return Ok(s.clone());
        }
        let b = read_capped(&path, MAX_SAMPLE_FILE)?;
        bytes_in += b.len() as u64;
        let label = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = Path::new(&label)
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (s, note) = sound_file::read(&stem, &b).map_err(|e| format!("{label}: {e}"))?;
        if let Some(n) = note {
            read_notes.push(format!("{label}: {n}"));
        }
        cache.insert(path, s.clone());
        Ok(s)
    };
    let kit = keygroup_kit(&prog, &mut sound, opts)?;
    lines.extend(read_notes);
    let sources = kit.project.sources().to_vec();
    let ram: usize = sources.iter().map(|s| s.data().len() * 4).sum();
    lines.push(format!(
        "kit: {} pads, {} layers, {} sounds ({:.1} MiB of files, {:.1} MiB in the sampler's memory)",
        kit.pads,
        kit.layers,
        sources.len(),
        bytes_in as f64 / 1048576.0,
        ram as f64 / 1048576.0
    ));
    lines.extend(kit.notes.iter().cloned());
    lines.push("pad  keys          root  layers (velocities)".into());
    for i in 0..kit.pads {
        let Some(p) = kit.project.pad(i) else {
            continue;
        };
        let Some(k) = p.keys else { continue };
        let vels: Vec<String> = p
            .layers
            .iter()
            .map(|l| format!("{}-{}", l.vel_lo, l.vel_hi))
            .collect();
        lines.push(format!(
            "{:3}  {:>4} to {:<4}  {:>4}  {}",
            i + 1,
            note_text(k.lo),
            note_text(k.hi),
            note_text(k.root),
            vels.join(", ")
        ));
    }

    let library = if dry_run {
        None
    } else {
        Some(Library::at(data).map_err(|e| format!("{}: {e}", data.display()))?)
    };
    let root = library
        .as_ref()
        .map_or_else(|| data.join("library"), |l| l.root().to_path_buf());
    let project = root.join(Place::Projects.folder()).join(&name);
    if fs::symlink_metadata(&project).is_ok() {
        return Err(format!(
            "a project named {name:?} is already there; give another with --name"
        ));
    }
    let Some(library) = library else {
        lines.push(format!(
            "dry run: nothing written; the project would be {}",
            project.display()
        ));
        return Ok(Report { project, lines });
    };

    let names = [prog.name.as_str()];
    let kit_name = names
        .iter()
        .find(|n| !n.is_empty())
        .copied()
        .unwrap_or(&name);
    let text = save::to_text(&[(kit_name, &kit.project)], 0, &wav::file_name);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let snap = Snapshot {
        meta: Meta {
            name: name.clone(),
            created: now,
            saved: now,
            app: format!("mpfree-mpc-import {}", env!("CARGO_PKG_VERSION")),
            ..Meta::default()
        },
        parts: vec![(SAMPLER_PART.to_string(), text)],
    };
    let samples: Vec<NewSample> = sources
        .into_iter()
        .map(|s| {
            let file = wav::file_name(&s);
            NewSample::new(
                file,
                Arc::new(move |w: &mut dyn std::io::Write| wav::write(&s, w)),
            )
        })
        .collect();
    library
        .save_project_with(&loc, &snap, KEEP_VERSIONS, &samples)
        .map_err(|e| format!("saving {}: {e}", project.display()))?;
    lines.push(format!("project: {}", project.display()));
    Ok(Report { project, lines })
}
