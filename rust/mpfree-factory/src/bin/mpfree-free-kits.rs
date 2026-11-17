//! Makes MPFree's free drum kits: fetches the files `free-kits.txt` names,
//! at its pinned commits, checks each one's SHA-256 and each library's
//! licence, and renders the kits into one kits file
//! (`mpfree_factory::freekits`). The release workflow runs it and puts the
//! file next to MPFree's programs as `free-kits`.
//!
//! ```text
//! mpfree-free-kits --cache DIR --out FILE [--credits FILE] [--manifest FILE] [--offline]
//! mpfree-free-kits --fill MANIFEST --cache DIR
//! ```
//!
//! - `--cache DIR`: where fetched files are kept, as
//!   `DIR/OWNER/REPO/COMMIT/PATH`; a file already there is used when its
//!   hash matches.
//! - `--out FILE`: the kits file. `--credits FILE`: where each kit's sounds
//!   came from, as text.
//! - `--manifest FILE`: another manifest than the one built in.
//! - `--offline`: never fetch; a missing file stops it.
//! - `--fill MANIFEST`: fetches each file whose hash is `-` and writes its
//!   hash into MANIFEST, for choosing new pads (listen to them first).
//!
//! Files are fetched with `curl` from raw.githubusercontent.com, over HTTPS
//! only.

use mpfree_factory::freekits::{self, File, Kit};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

struct Cache {
    dir: PathBuf,
    offline: bool,
}

impl Cache {
    fn path(&self, kit: &Kit, f: &File) -> PathBuf {
        let mut p = self.dir.join(&kit.repo).join(&kit.commit);
        for part in f.path.split('/') {
            p.push(part);
        }
        p
    }

    /// The file's bytes, fetched if they aren't here, checked against its
    /// hash when it has one.
    fn get(&self, kit: &Kit, f: &File) -> Result<Vec<u8>, String> {
        let path = self.path(kit, f);
        let good = |data: &[u8]| f.sha256.is_none_or(|h| freekits::sha256(data) == h);
        if let Ok(data) = std::fs::read(&path) {
            if good(&data) {
                return Ok(data);
            }
        }
        if self.offline {
            return Err(format!(
                "{}: not in the cache, or not the file the manifest names, and --offline",
                path.display()
            ));
        }
        let data = fetch(&kit.url(&f.path), &path)?;
        if !good(&data) {
            let _ = std::fs::remove_file(&path);
            return Err(format!(
                "{} at {}: its SHA-256 is {}, not the manifest's",
                f.path,
                kit.commit,
                freekits::hex(&freekits::sha256(&data))
            ));
        }
        Ok(data)
    }
}

/// Fetches `url` into `to` (through `to.part`) and returns its bytes.
fn fetch(url: &str, to: &Path) -> Result<Vec<u8>, String> {
    let dir = to.parent().ok_or("a cache path with no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = to.with_extension("part");
    eprintln!("fetching {url}");
    let status = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--retry",
            "3",
            "--connect-timeout",
            "30",
            "--max-time",
            "600",
            "--output",
        ])
        .arg(&part)
        .arg(url)
        .status()
        .map_err(|e| format!("curl: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(format!("curl couldn't fetch {url} ({status})"));
    }
    let data = std::fs::read(&part).map_err(|e| format!("{}: {e}", part.display()))?;
    std::fs::rename(&part, to).map_err(|e| format!("{}: {e}", to.display()))?;
    Ok(data)
}

/// Writes `data` to `to` through `to.part`, so a stopped run leaves no half file.
fn write(to: &Path, data: &[u8]) -> Result<(), String> {
    let part = to.with_extension("part");
    std::fs::write(&part, data).map_err(|e| format!("{}: {e}", part.display()))?;
    std::fs::rename(&part, to).map_err(|e| format!("{}: {e}", to.display()))
}

fn run(args: &[String]) -> Result<(), String> {
    let opt = |name: &str| -> Option<String> {
        let i = args.iter().position(|a| a == name)?;
        args.get(i + 1).cloned()
    };
    let cache = Cache {
        dir: PathBuf::from(opt("--cache").ok_or("--cache DIR is needed")?),
        offline: args.iter().any(|a| a == "--offline"),
    };
    let known = [
        "--cache",
        "--out",
        "--credits",
        "--manifest",
        "--fill",
        "--offline",
    ];
    let mut i = 0;
    while i < args.len() {
        if !known.contains(&args[i].as_str()) {
            return Err(format!(
                "{:?}: not an option (see the top of this file)",
                args[i]
            ));
        }
        i += if args[i] == "--offline" { 1 } else { 2 };
    }
    if let Some(manifest) = opt("--fill") {
        let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{manifest}: {e}"))?;
        let filled = freekits::fill(&text, &mut |kit, path| {
            let f = File {
                sha256: None,
                path: path.to_string(),
            };
            Ok(freekits::sha256(&cache.get(kit, &f)?))
        })?;
        write(Path::new(&manifest), filled.as_bytes())?;
        eprintln!("{manifest}: hashes written in");
        return Ok(());
    }
    let out = PathBuf::from(opt("--out").ok_or("--out FILE is needed")?);
    let text = match opt("--manifest") {
        Some(m) => std::fs::read_to_string(&m).map_err(|e| format!("{m}: {e}"))?,
        None => freekits::MANIFEST.to_string(),
    };
    let kits = freekits::parse(&text)?;
    for k in &kits {
        for f in k.files() {
            if f.sha256.is_none() {
                return Err(format!("{}: {} has no hash yet (--fill)", k.name, f.path));
            }
        }
    }
    let (file, reports) = freekits::render(&kits, &mut |k, f| cache.get(k, f))?;
    for (k, report) in kits.iter().zip(&reports) {
        eprintln!("{} ({}, {})", k.name, k.maker, k.terms);
        for r in report {
            eprintln!(
                "  pad {:>2} {:<16} {:>5.2} s  raised {:>5.1} dB{}",
                r.pad,
                r.name,
                r.seconds,
                r.gain_db,
                if r.cut { "  (cut, faded)" } else { "" }
            );
        }
    }
    write(&out, &file)?;
    eprintln!(
        "{}: {} kits, {} bytes",
        out.display(),
        kits.len(),
        file.len()
    );
    if let Some(c) = opt("--credits") {
        write(Path::new(&c), freekits::credits(&kits).as_bytes())?;
        eprintln!("{c}: the credits");
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mpfree-free-kits: {e}");
            ExitCode::FAILURE
        }
    }
}
