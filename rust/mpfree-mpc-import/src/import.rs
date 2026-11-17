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
use mpfree_sampler::model::{Project, Source};
use mpfree_sampler::{save, sound_file, wav};

use crate::drum::{drum_kit, mpc_pad_name, Placement};
use crate::kit::{keygroup_kit, note_text, Options};
use crate::xml::MAX_BYTES;
use crate::xpm::{program_type, read_drum, read_keygroup, XpmLayer};

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

/// Sample files beside a program, read once each.
struct Sounds {
    files: HashMap<String, PathBuf>,
    cache: HashMap<PathBuf, Source>,
    notes: Vec<String>,
    bytes_in: u64,
}

impl Sounds {
    fn beside(xpm: &Path) -> Result<Sounds, String> {
        let dir = xpm
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        Ok(Sounds {
            files: files_in(dir)?,
            cache: HashMap::new(),
            notes: Vec::new(),
            bytes_in: 0,
        })
    }

    fn get(&mut self, l: &XpmLayer) -> Result<Source, String> {
        let path = find(l, &self.files).ok_or("its file is not beside the program")?;
        if let Some(s) = self.cache.get(&path) {
            return Ok(s.clone());
        }
        let b = read_capped(&path, MAX_SAMPLE_FILE)?;
        self.bytes_in += b.len() as u64;
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
            self.notes.push(format!("{label}: {n}"));
        }
        self.cache.insert(path, s.clone());
        Ok(s)
    }
}

/// The program file, read and capped.
fn program_bytes(xpm: &Path) -> Result<Vec<u8>, String> {
    read_capped(xpm, MAX_BYTES as u64).map_err(|e| format!("{}: {e}", xpm.display()))
}

/// The project's name: `name`, else the program's, else the file's.
fn project_name(xpm: &Path, name: Option<&str>, prog_name: &str, fallback: &str) -> String {
    let stem = xpm
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    clean_name(
        name.unwrap_or(if prog_name.is_empty() {
            &stem
        } else {
            prog_name
        }),
        fallback,
    )
}

/// The size line for the kit's sounds.
fn kit_line(what: &str, project: &Project, bytes_in: u64) -> String {
    let sources = project.sources();
    let ram: usize = sources.iter().map(|s| s.data().len() * 4).sum();
    format!(
        "kit: {what}, {} sounds ({:.1} MiB of files, {:.1} MiB in the sampler's memory)",
        sources.len(),
        bytes_in as f64 / 1048576.0,
        ram as f64 / 1048576.0
    )
}

/// Saves `project` as a new project named `name` in the library in
/// `data`, with only the sampler's part (the kit named `kit_name`), or
/// says where it would go on a dry run. Never replaces a project.
fn save_kit(
    data: &Path,
    name: &str,
    kit_name: &str,
    project: &Project,
    dry_run: bool,
    lines: &mut Vec<String>,
) -> Result<PathBuf, String> {
    let loc =
        Loc::new(Place::Projects, &[name]).map_err(|e| format!("project name {name:?}: {e}"))?;
    let library = if dry_run {
        None
    } else {
        Some(Library::at(data).map_err(|e| format!("{}: {e}", data.display()))?)
    };
    let root = library
        .as_ref()
        .map_or_else(|| data.join("library"), |l| l.root().to_path_buf());
    let path = root.join(Place::Projects.folder()).join(name);
    if fs::symlink_metadata(&path).is_ok() {
        return Err(format!(
            "a project named {name:?} is already there; give another with --name"
        ));
    }
    let Some(library) = library else {
        lines.push(format!(
            "dry run: nothing written; the project would be {}",
            path.display()
        ));
        return Ok(path);
    };
    let kit_name = if kit_name.is_empty() { name } else { kit_name };
    let text = save::to_text(&[(kit_name, project)], 0, &wav::file_name);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let snap = Snapshot {
        meta: Meta {
            name: name.to_string(),
            created: now,
            saved: now,
            app: format!("mpfree-mpc-import {}", env!("CARGO_PKG_VERSION")),
            ..Meta::default()
        },
        parts: vec![(SAMPLER_PART.to_string(), text)],
    };
    let samples: Vec<NewSample> = project
        .sources()
        .iter()
        .cloned()
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
        .map_err(|e| format!("saving {}: {e}", path.display()))?;
    lines.push(format!("project: {}", path.display()));
    Ok(path)
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
    let bytes = program_bytes(xpm)?;
    let prog = read_keygroup(&bytes).map_err(|e| format!("{}: {e}", xpm.display()))?;
    let mut sounds = Sounds::beside(xpm)?;
    let name = project_name(xpm, name, &prog.name, "Imported keygroup");
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
    let kit = keygroup_kit(&prog, &mut |l| sounds.get(l), opts)?;
    lines.append(&mut sounds.notes);
    lines.push(kit_line(
        &format!("{} pads, {} layers", kit.pads, kit.layers),
        &kit.project,
        sounds.bytes_in,
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
    let project = save_kit(data, &name, &prog.name, &kit.project, dry_run, &mut lines)?;
    Ok(Report { project, lines })
}

/// Imports the drum program at `xpm` as a new project, as
/// [`import_keygroup`] does for keygroups, with its pads placed by
/// `placement` ([`crate::drum`]).
pub fn import_drum(
    xpm: &Path,
    data: &Path,
    name: Option<&str>,
    opts: &Options,
    placement: Placement,
    dry_run: bool,
) -> Result<Report, String> {
    let bytes = program_bytes(xpm)?;
    let prog = read_drum(&bytes).map_err(|e| format!("{}: {e}", xpm.display()))?;
    let mut sounds = Sounds::beside(xpm)?;
    let name = project_name(xpm, name, &prog.name, "Imported drums");
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
        format!("pads with a sample: {}", prog.pads.len()),
        format!(
            "pads placed {}",
            match placement {
                Placement::Notes =>
                    "by note, so MIDI exported from the MPC plays them (pad 1 is C2, note 36)",
                Placement::Pads =>
                    "as on the MPC (A01 on pad 1); MIDI exported from the MPC plays other pads",
            }
        ),
    ];
    let kit = drum_kit(&prog, &mut |l| sounds.get(l), opts, placement)?;
    lines.append(&mut sounds.notes);
    lines.push(kit_line(
        &format!("{} pads, {} layers", kit.placed.len(), kit.layers),
        &kit.project,
        sounds.bytes_in,
    ));
    lines.extend(kit.notes.iter().cloned());
    lines.push("pad  MPC pad  note        layers (velocities)  sound".into());
    let mut placed = kit.placed.clone();
    placed.sort_by_key(|p| p.index);
    for pl in &placed {
        let Some(p) = kit.project.pad(pl.index) else {
            continue;
        };
        let vels: Vec<String> = p
            .layers
            .iter()
            .map(|l| format!("{}-{}", l.vel_lo, l.vel_hi))
            .collect();
        let note = match pl.note {
            Some(n) => format!("{n:3} {:<4}", note_text(n)),
            None => "none".to_string(),
        };
        lines.push(format!(
            "{:3}  {:<7}  {:<8}{}  {:<19}  {}",
            pl.index + 1,
            mpc_pad_name(pl.mpc_pad),
            note,
            if pl.answers_note { "  " } else { " *" },
            vels.join(", "),
            p.name
        ));
    }
    if placed.iter().any(|p| !p.answers_note) {
        lines.push("* doesn't answer its MPC note here".into());
    }
    let project = save_kit(data, &name, &prog.name, &kit.project, dry_run, &mut lines)?;
    Ok(Report { project, lines })
}

/// A program file's type, read from the file: `Keygroup`, `Drum`, ...
pub fn program_kind(xpm: &Path) -> Result<String, String> {
    program_type(&program_bytes(xpm)?).map_err(|e| format!("{}: {e}", xpm.display()))
}
