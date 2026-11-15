//! First start on a stick the MPFree USB creator made: the MPC's own drum
//! kits and samples, with no effort from the musician.
//!
//! 1. **The backup.** The pad player is not root and never touches the
//!    eMMC. It asks the stick's root service for the backup by making
//!    `/run/mpfree-app/setup-backup.request` (in RAM); systemd's
//!    `mpfree-setup-backup.path` starts `mpfree-setup-backup.service`
//!    (`/etc/mpfree-test/setup-backup.sh`), which copies the eMMC through a
//!    short read-only window into partition 4's `backup/` and reports in
//!    `/run/mpfree-setup/status` (`key=value` lines), read here for the
//!    progress bar.
//! 2. **Kits and samples.** From the backup on the stick, never from the
//!    eMMC: the content partition inside the backup's copy of the user
//!    area, read with the read-only ext4 reader (`mpfree-factory`). The kits
//!    go to `library/Kits/Akai factory kits.mpfkits`, loaded at every start
//!    before MPFree's own; the samples to `library/Samples/Akai factory/`,
//!    in Akai's folders, where BROWSE finds them. Both are written only
//!    through `mpfree-library`.
//! 3. **Done** is kept in the store (`settings/setup.conf`), so it runs
//!    once. A failed attempt is tried again at the next start, at most
//!    [`MAX_TRIES`] times; the pad player works either way.
//!
//! It runs only where the stick has the backup service (the creator's
//! sticks), only with `--data`, and never in `play`.

use mpfree_factory::import::{Content, DEFAULT_KITS};
use mpfree_library::{clean_name, Library, Loc, Place};
use mpfree_store::Settings;
use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// The store's owner name: `settings/setup.conf`.
pub const OWNER: &str = "setup";
/// The Akai kits, in the library's Kits place.
pub const KITS_FILE: &str = "Akai factory kits.mpfkits";
/// Akai's samples, in the library's Samples place.
pub const SAMPLES_FOLDER: &str = "Akai factory";
/// The stick's backup service; without it there is no first-start setup.
pub const SERVICE: &str = "/etc/mpfree-test/setup-backup.sh";
/// What the service reports.
pub const STATUS: &str = "/run/mpfree-setup/status";
/// The request's name, in the program's own folder in /run.
pub const REQUEST: &str = "setup-backup.request";
/// Starts that try the setup before it gives up until the stick is made again.
pub const MAX_TRIES: i64 = 3;
/// The largest sample copied (the library's own limit is higher).
const MAX_SAMPLE: u64 = 512 << 20;

/// Where things are; tests point them at a temporary folder.
pub struct Paths {
    pub data: PathBuf,
    pub service: PathBuf,
    pub status: PathBuf,
    pub request: PathBuf,
}

impl Paths {
    pub fn on_the_stick(data: &Path, report_dir: &Path) -> Paths {
        Paths {
            data: data.to_path_buf(),
            service: PathBuf::from(SERVICE),
            status: PathBuf::from(STATUS),
            request: report_dir.join(REQUEST),
        }
    }
}

/// How long to wait: for the service to answer at all, and for the whole
/// backup (the service's own limit is an hour).
pub struct Timing {
    pub poll: Duration,
    pub answer: Duration,
    pub backup: Duration,
}

impl Timing {
    pub fn on_the_mpc() -> Timing {
        Timing {
            poll: Duration::from_millis(500),
            answer: Duration::from_secs(60),
            backup: Duration::from_secs(70 * 60),
        }
    }
}

/// Whether the setup still has to run: the stick has the service, and the
/// store doesn't say it is done or given up.
pub fn needed(paths: &Paths, saved: &Settings) -> bool {
    paths.service.is_file()
        && saved.get_str("state") != Some("done")
        && saved.get_i64("tries").unwrap_or(0) < MAX_TRIES
}

/// The service's report.
#[derive(Debug, PartialEq)]
pub struct Status {
    pub state: String,
    pub step: String,
    pub done: u64,
    pub total: u64,
    pub message: String,
    pub folder: Option<PathBuf>,
}

pub fn parse_status(text: &str) -> Option<Status> {
    let mut s = Status {
        state: String::new(),
        step: String::new(),
        done: 0,
        total: 0,
        message: String::new(),
        folder: None,
    };
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k {
            "state" => s.state = v.to_string(),
            "step" => s.step = v.to_string(),
            "done" => s.done = v.parse().unwrap_or(0),
            "total" => s.total = v.parse().unwrap_or(0),
            "message" => s.message = v.to_string(),
            "folder" if !v.is_empty() => s.folder = Some(PathBuf::from(v)),
            _ => {}
        }
    }
    (!s.state.is_empty()).then_some(s)
}

/// One picture of the wizard: a heading, a line under it and the bar (0 to 1).
pub struct Show {
    pub message: String,
    pub detail: String,
    pub value: f32,
}

fn show(f: &mut dyn FnMut(Show), message: &str, detail: String, value: f32) {
    f(Show {
        message: message.to_string(),
        detail: format!("First start   ·   {detail}"),
        value,
    });
}

/// What the setup did: the settings to keep (state, tries, the backup's
/// folder, how many kits and samples) and the kits file, if one was made.
pub struct Outcome {
    pub saved: Settings,
    pub kits: Option<PathBuf>,
    pub failed: Option<String>,
}

const BACKING_UP: &str = "Backing up your MPC";
const EXTRACTING: &str = "Extracting Akai drum kits and samples";

fn gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1e9)
}

/// Runs the whole setup. `saved` is what the store holds now.
pub fn run(
    paths: &Paths,
    saved: &Settings,
    timing: &Timing,
    show_f: &mut dyn FnMut(Show),
    say: &mut dyn FnMut(String),
) -> Outcome {
    let mut out = saved.clone();
    let tries = saved.get_i64("tries").unwrap_or(0) + 1;
    out.set_i64("tries", tries);
    say(format!("first start: setup, try {tries} of {MAX_TRIES}"));
    let fail = |mut out: Settings, why: String, say: &mut dyn FnMut(String)| {
        say(format!("first start: {why}"));
        out.set_str("state", "failed");
        out.set_str("why", &why);
        Outcome {
            saved: out,
            kits: None,
            failed: Some(why),
        }
    };
    let folder = match backup(paths, timing, show_f, say) {
        Ok(f) => f,
        Err(why) => return fail(out, why, say),
    };
    out.set_str("backup", &folder.display().to_string());
    match extract(paths, &folder, show_f, say) {
        Ok((kits, made, samples)) => {
            out.set_str("state", "done");
            out.remove("why");
            out.set_i64("akai-kits", made as i64);
            out.set_i64("akai-samples", samples as i64);
            show(show_f, "Setup done", "Starting the pad player".into(), 1.0);
            say(format!(
                "first start: done: {made} Akai kits, {samples} samples; the backup is in {}",
                folder.display()
            ));
            Outcome {
                saved: out,
                kits,
                failed: None,
            }
        }
        Err(why) => fail(out, why, say),
    }
}

/// Asks for the backup and follows it; the backup's folder on partition 4.
fn backup(
    paths: &Paths,
    timing: &Timing,
    show_f: &mut dyn FnMut(Show),
    say: &mut dyn FnMut(String),
) -> Result<PathBuf, String> {
    show(show_f, BACKING_UP, "Asking for the backup".into(), 0.0);
    // The request: an empty file in the program's own folder in /run (RAM).
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&paths.request)
    {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
        Err(e) => {
            return Err(format!(
                "could not ask for the backup ({}: {e})",
                paths.request.display()
            ))
        }
    }
    say(format!(
        "first start: asked for the backup ({})",
        paths.request.display()
    ));
    let start = Instant::now();
    let mut last = String::new();
    loop {
        let text = std::fs::read_to_string(&paths.status).unwrap_or_default();
        let status = parse_status(&text);
        match &status {
            None if start.elapsed() >= timing.answer => {
                return Err("the backup service did not answer".into());
            }
            None => {}
            Some(s) => {
                if s.state == "done" {
                    let folder = s
                        .folder
                        .clone()
                        .ok_or("the backup service named no folder")?;
                    // Only a folder in partition 4's backup/ is read.
                    if folder.parent() != Some(&paths.data.join("backup")) {
                        return Err(format!(
                            "the backup is not on partition 4 ({})",
                            folder.display()
                        ));
                    }
                    say(format!("first start: backup done: {}", folder.display()));
                    return Ok(folder);
                }
                if s.state == "failed" {
                    return Err(format!("the backup did not work: {}", s.message));
                }
                let (detail, value) = match s.step.as_str() {
                    "copying" if s.total > 0 => (
                        format!(
                            "{} of {} GB   ·   keep the stick in and the MPC on",
                            gb(s.done),
                            gb(s.total)
                        ),
                        0.05 + 0.6 * s.done as f32 / s.total as f32,
                    ),
                    "closing" => ("Switching the internal storage off again".to_string(), 0.66),
                    "verifying" => ("Checking the copy on the stick".to_string(), 0.68),
                    _ => (s.message.clone(), 0.02),
                };
                show(show_f, BACKING_UP, detail, value);
                if s.step != last {
                    say(format!("first start: backup: {} ({})", s.step, s.message));
                    last = s.step.clone();
                }
            }
        }
        if start.elapsed() >= timing.backup {
            return Err("the backup took too long".into());
        }
        thread::sleep(timing.poll);
    }
}

/// The library folder `name` in `parent`, made if missing.
fn folder_in(lib: &Library, parent: &Loc, name: &str) -> std::io::Result<Loc> {
    let loc = parent.join(name)?;
    if lib.list(&loc).is_ok() {
        return Ok(loc);
    }
    lib.new_folder(parent, name)
}

/// Kits and samples from the backup: (the kits file, kits made, samples copied).
fn extract(
    paths: &Paths,
    folder: &Path,
    show_f: &mut dyn FnMut(Show),
    say: &mut dyn FnMut(String),
) -> Result<(Option<PathBuf>, usize, usize), String> {
    show(show_f, EXTRACTING, "Reading the backup".into(), 0.7);
    // The backup's copy of the eMMC's user area: a file on partition 4, read only.
    let image = folder.join("mmcblk0.img");
    let file = File::open(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let content =
        Content::from_disk(file).map_err(|e| format!("the backup's content partition: {e}"))?;
    let lib = Library::at(&paths.data).map_err(|e| format!("the library: {e}"))?;
    // Kits.
    let kits_loc = Loc::new(Place::Kits, &[KITS_FILE]).map_err(|e| e.to_string())?;
    let mut made = 0;
    let kits = match lib.file_path(&kits_loc) {
        Ok(p) => {
            say(format!("first start: {KITS_FILE} is there already; kept"));
            Some(p)
        }
        Err(_) => {
            let bytes = content
                .kits(
                    DEFAULT_KITS,
                    &mut |line| say(format!("first start: kits: {line}")),
                    &mut |done, total| {
                        show(
                            show_f,
                            EXTRACTING,
                            format!("Making drum kit {} of {total}", (done + 1).min(total)),
                            0.7 + 0.1 * done as f32 / total.max(1) as f32,
                        )
                    },
                )
                .map_err(|e| format!("the kits: {e}"))?;
            match bytes {
                Some(b) => {
                    made = b
                        .get(8..12)
                        .map_or(0, |n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize);
                    let loc = lib
                        .write_new(&Loc::top(Place::Kits), KITS_FILE, &b)
                        .map_err(|e| format!("saving the kits: {e}"))?;
                    Some(lib.file_path(&loc).map_err(|e| e.to_string())?)
                }
                None => {
                    say("first start: no drum programs in the backup; no Akai kits".into());
                    None
                }
            }
        }
    };
    // Samples, in Akai's folders.
    let samples = content.samples();
    let top = folder_in(&lib, &Loc::top(Place::Samples), SAMPLES_FOLDER)
        .map_err(|e| format!("the samples folder: {e}"))?;
    let mut copied = 0;
    let total = samples.len();
    for (n, (path, size)) in samples.iter().enumerate() {
        if n % 8 == 0 {
            show(
                show_f,
                EXTRACTING,
                format!("Copying samples: {n} of {total}"),
                0.8 + 0.2 * n as f32 / total.max(1) as f32,
            );
        }
        if *size > MAX_SAMPLE {
            say(format!("first start: {path} left out ({size} bytes)"));
            continue;
        }
        let mut parts: Vec<String> = path
            .split('/')
            .filter(|p| !p.is_empty())
            .map(|p| clean_name(p, "sample"))
            .collect();
        let Some(name) = parts.pop() else { continue };
        let mut at = top.clone();
        let mut ok = true;
        for p in &parts {
            match folder_in(&lib, &at, p) {
                Ok(l) => at = l,
                Err(e) => {
                    say(format!("first start: {path}: folder {p}: {e}"));
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        // Kept from an earlier try that was cut off.
        if at.join(&name).is_ok_and(|l| lib.file_path(&l).is_ok()) {
            copied += 1;
            continue;
        }
        match content
            .read(path, MAX_SAMPLE)
            .and_then(|b| lib.write_new(&at, &name, &b))
        {
            Ok(_) => copied += 1,
            Err(e) => say(format!("first start: {path}: {e}")),
        }
    }
    Ok((kits, made, copied))
}

/// The kits to play: the Akai kits first, then MPFree's own, at most
/// `max` in all.
pub fn combine<T>(akai: Vec<T>, own: Vec<T>, max: usize) -> Vec<T> {
    akai.into_iter().chain(own).take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("mpfree-setup-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("data")).unwrap();
        std::fs::create_dir_all(p.join("run")).unwrap();
        Tmp(p)
    }

    fn paths(t: &Tmp) -> Paths {
        std::fs::write(t.0.join("setup-backup.sh"), "# service\n").unwrap();
        Paths {
            data: t.0.join("data"),
            service: t.0.join("setup-backup.sh"),
            status: t.0.join("run/status"),
            request: t.0.join("run/setup-backup.request"),
        }
    }

    fn quick() -> Timing {
        Timing {
            poll: Duration::from_millis(5),
            answer: Duration::from_millis(100),
            backup: Duration::from_secs(5),
        }
    }

    fn run_it(p: &Paths, saved: &Settings) -> (Outcome, Vec<Show>, Vec<String>) {
        let (mut shown, mut said) = (Vec::new(), Vec::new());
        let o = run(p, saved, &quick(), &mut |s| shown.push(s), &mut |l| {
            said.push(l)
        });
        (o, shown, said)
    }

    #[test]
    fn status_lines() {
        let s =
            parse_status("state=running\nstep=copying\ndone=10\ntotal=40\nmessage=Backing up\n")
                .unwrap();
        assert_eq!(
            (s.state.as_str(), s.step.as_str(), s.done, s.total),
            ("running", "copying", 10, 40)
        );
        assert_eq!(s.folder, None);
        assert!(parse_status("").is_none());
        let s = parse_status("state=done\nfolder=/x/backup/y\n").unwrap();
        assert_eq!(s.folder, Some(PathBuf::from("/x/backup/y")));
    }

    #[test]
    fn needed_only_with_the_service_and_until_done() {
        let t = tmp("needed");
        let p = paths(&t);
        let mut s = Settings::default();
        assert!(needed(&p, &s));
        s.set_i64("tries", MAX_TRIES);
        assert!(!needed(&p, &s), "gives up after {MAX_TRIES} tries");
        let mut s = Settings::default();
        s.set_str("state", "done");
        assert!(!needed(&p, &s));
        std::fs::remove_file(&p.service).unwrap();
        assert!(
            !needed(&p, &Settings::default()),
            "only on the creator's sticks"
        );
    }

    #[test]
    fn kits_and_samples_from_the_backup() {
        let t = tmp("done");
        let p = paths(&t);
        let folder = p.data.join("backup/mpc-one-emmc-20261115T120000Z");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("mmcblk0.img"),
            mpfree_factory::import::testdisk::disk(),
        )
        .unwrap();
        std::fs::write(
            &p.status,
            format!("state=done\nstep=done\nfolder={}\n", folder.display()),
        )
        .unwrap();
        let (o, shown, said) = run_it(&p, &Settings::default());
        assert_eq!(o.failed, None, "{said:?}");
        assert!(p.request.exists(), "the backup was asked for");
        assert_eq!(o.saved.get_str("state"), Some("done"));
        assert_eq!(o.saved.get_i64("tries"), Some(1));
        let kits = o.kits.expect("a kits file");
        assert_eq!(kits, p.data.join("library/Kits").join(KITS_FILE));
        let n = o.saved.get_i64("akai-kits").unwrap();
        assert!(n >= 1);
        let bytes = std::fs::read(&kits).unwrap();
        assert_eq!(crate::kit::parse_kits(&bytes).unwrap().len() as i64, n);
        let samples = o.saved.get_i64("akai-samples").unwrap();
        assert!(samples >= 4, "{samples}");
        let top = p.data.join("library/Samples").join(SAMPLES_FOLDER);
        let mut found = 0;
        let mut stack = vec![top];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    found += 1;
                }
            }
        }
        assert_eq!(found as i64, samples);
        assert!(shown.iter().any(|s| s.message == EXTRACTING));
        assert_eq!(shown.last().unwrap().message, "Setup done");
        // Run again (as if the store had not saved): nothing is copied twice.
        let (o, _, _) = run_it(&p, &Settings::default());
        assert_eq!(o.saved.get_i64("akai-samples"), Some(samples));
        assert!(!p
            .data
            .join("library/Kits/Akai factory kits 2.mpfkits")
            .exists());
    }

    #[test]
    fn a_failed_or_missing_backup_is_tried_again_later() {
        let t = tmp("fail");
        let p = paths(&t);
        let (o, _, _) = run_it(&p, &Settings::default());
        assert_eq!(
            o.failed.as_deref(),
            Some("the backup service did not answer")
        );
        assert_eq!(o.saved.get_str("state"), Some("failed"));
        assert!(needed(&p, &o.saved), "tried again at the next start");
        std::fs::write(
            &p.status,
            "state=failed\nstep=checking\nmessage=partition 4 has 10 MiB free\n",
        )
        .unwrap();
        let (o, _, _) = run_it(&p, &o.saved);
        assert_eq!(
            o.failed.as_deref(),
            Some("the backup did not work: partition 4 has 10 MiB free")
        );
        assert_eq!(o.saved.get_i64("tries"), Some(2));
        // A folder outside partition 4's backup/ is never read.
        std::fs::write(&p.status, "state=done\nfolder=/etc\n").unwrap();
        let (o, _, _) = run_it(&p, &o.saved);
        assert_eq!(
            o.failed.as_deref(),
            Some("the backup is not on partition 4 (/etc)")
        );
        assert!(!needed(&p, &o.saved), "three tries, then it gives up");
    }

    #[test]
    fn the_kits_use_the_mpc_ones_pad_notes() {
        assert_eq!(
            mpfree_factory::kitfile::PAD_NOTES,
            mpfree_controller::mpc_one::PAD_NOTES
        );
    }

    #[test]
    fn akai_kits_come_first() {
        assert_eq!(combine(vec![1, 2], vec![3, 4, 5], 4), vec![1, 2, 3, 4]);
        assert_eq!(combine(Vec::<u8>::new(), vec![3], 16), vec![3]);
    }
}
