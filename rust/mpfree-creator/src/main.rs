//! The MPFree USB creator: a small window that makes an MPFree boot stick
//! for the MPC One (Gen 1) from the owner's own Akai update file, on Linux,
//! macOS and Windows. No terminal, and no Akai files shipped.
//!
//! The window itself never writes a disk. For each step that does (making
//! the stick, switching it to remove MPFree) or reads one (copying the
//! backup off), it runs itself as the command line (`--cli`,
//! [`mpfree_stick::cli`]) with the rights to do so: pkexec on Linux, the
//! administrator password on macOS, an administrator prompt on Windows.
//! The command line writes its progress to a status file the window reads.

#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui::{self, RichText};
use mpfree_stick::disks::{self, DiskInfo};
use mpfree_stick::update::{Supported, Update};
use mpfree_stick::{ssh, Msg, CREATOR};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const MANUAL: &str =
    "https://github.com/jferments/mpfree/blob/main/docs/user/src/getting-started/installing.md";
const MIN_STICK: u64 = mpfree_stick::stick::MIN_STICK;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--cli") {
        std::process::exit(mpfree_stick::cli::main(args[1..].to_vec()));
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MPFree USB creator")
            .with_inner_size([720.0, 860.0])
            .with_min_inner_size([540.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MPFree USB creator",
        options,
        Box::new(|cc| {
            style(&cc.egui_ctx);
            Ok(Box::new(Creator::new()))
        }),
    )
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// What the window knows about the update file without unpacking it.
struct UpdateCheck {
    path: PathBuf,
    build: String,
    supported: Option<&'static Supported>,
    problem: Option<String>,
}

fn check_update(path: &Path) -> UpdateCheck {
    let mut c = UpdateCheck {
        path: path.to_path_buf(),
        build: String::new(),
        supported: None,
        problem: None,
    };
    match Update::open(path) {
        Ok(u) => {
            c.build = u.build.clone();
            c.supported = u.supported();
            if !u.for_mpc_one() {
                c.problem = Some("This update is not for the MPC One.".into());
            } else if let Err(e) = u.rootfs() {
                c.problem = Some(format!("This update can't be used: {e}"));
            }
        }
        Err(e) => c.problem = Some(format!("This is not an Akai update file: {e}")),
    }
    c
}

/// Of the update files found (newest first), the newest supported one, else
/// the newest at all.
fn best_update(found: &[PathBuf]) -> Option<UpdateCheck> {
    let checks: Vec<UpdateCheck> = found.iter().map(|p| check_update(p)).collect();
    let pick = checks
        .iter()
        .position(|c| c.supported.is_some() && c.problem.is_none())
        .or(if checks.is_empty() { None } else { Some(0) })?;
    checks.into_iter().nth(pick)
}

/// Akai update files in the usual places (newest first): a quick look at
/// the first bytes of each likely file.
fn find_updates() -> Vec<PathBuf> {
    let h = home();
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let look = |dir: &Path, depth: u32, found: &mut Vec<(std::time::SystemTime, PathBuf)>| {
        let mut stack = vec![(dir.to_path_buf(), depth)];
        while let Some((d, depth)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let Ok(md) = e.metadata() else { continue };
                let p = e.path();
                if md.is_dir() {
                    let hidden = e.file_name().to_string_lossy().starts_with('.');
                    if depth > 0 && !hidden {
                        stack.push((p, depth - 1));
                    }
                } else if md.is_file() && (100 << 20..4 << 30).contains(&md.len()) {
                    let mut head = [0u8; 4];
                    let ok = std::fs::File::open(&p)
                        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
                        .is_ok();
                    if ok && &head == b"AZ01" {
                        found.push((md.modified().unwrap_or(std::time::UNIX_EPOCH), p));
                    }
                }
            }
        }
    };
    for (d, depth) in [("Downloads", 3), ("Desktop", 2), ("Documents", 2), ("", 0)] {
        look(&h.join(d), depth, &mut found);
    }
    found.sort();
    found.dedup_by(|a, b| a.1 == b.1);
    found.into_iter().rev().map(|(_, p)| p).collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Make,
    Done,
    Backup,
    Remove,
}

#[derive(Clone, Copy, PartialEq)]
enum JobKind {
    Make,
    Backup,
    RemoveMode,
    InstallMode,
}

/// A command running with the rights to use the stick.
struct Job {
    kind: JobKind,
    child: Child,
    status: PathBuf,
    read: usize,
    progress: f32,
    line: String,
    log: Vec<String>,
    /// What the command printed (where it can be read: Linux), for the
    /// reason when the status file says nothing.
    printed: Arc<Mutex<Vec<String>>>,
}

/// Keeps the last lines a command prints, read on threads so that a full
/// pipe never stops it.
fn collect_output(child: &mut Child) -> Arc<Mutex<Vec<String>>> {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let readers: Vec<Box<dyn std::io::Read + Send>> = [
        child
            .stdout
            .take()
            .map(|o| Box::new(o) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|e| Box::new(e) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    .collect();
    for r in readers {
        let lines = Arc::clone(&lines);
        std::thread::spawn(move || {
            use std::io::BufRead;
            for l in std::io::BufReader::new(r).lines().map_while(Result::ok) {
                let mut v = lines.lock().unwrap_or_else(|p| p.into_inner());
                v.push(l);
                if v.len() > 200 {
                    v.remove(0);
                }
            }
        });
    }
    lines
}

/// Why a command that left no `done` or `error` line ended: from what it
/// printed, else from its exit code (pkexec: 126 when the password prompt
/// was cancelled or failed, 127 when it is not allowed).
fn why_it_ended(code: Option<i32>, printed: &[String]) -> String {
    if let Some(l) = printed.iter().rev().find(|l| l.starts_with("error ")) {
        return l[6..].to_string();
    }
    match code {
        Some(126) => "The administrator password was not accepted, or the prompt was closed. Try again.".into(),
        Some(127) => "This computer did not allow the creator to use the stick (pkexec refused). Check that you are an administrator on this computer.".into(),
        _ => {
            let last = printed
                .iter()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(|l| format!(": {l}"))
                .unwrap_or_default();
            format!(
                "It stopped without saying why (exit code {}){last}.",
                code.map_or("none".into(), |c| c.to_string())
            )
        }
    }
}

impl Job {
    /// New lines from the status file; Some(result) once the command ended.
    fn poll(&mut self) -> Option<Result<String, String>> {
        if let Ok(text) = std::fs::read_to_string(&self.status) {
            if text.len() > self.read {
                let new = &text[self.read..];
                let complete = new.rfind('\n').map_or(0, |i| i + 1);
                for l in new[..complete].lines() {
                    self.log.push(l.to_string());
                    if let Some(Msg::Progress(f, t)) = Msg::parse(l) {
                        self.progress = f;
                        self.line = t;
                    }
                }
                self.read += complete;
            }
        }
        let ended = self.child.try_wait();
        match ended {
            Ok(None) => None,
            Ok(Some(_)) | Err(_) => {
                let code = ended.ok().flatten().and_then(|s| s.code());
                // Let the readers take the last lines.
                std::thread::sleep(Duration::from_millis(100));
                let _ = std::fs::read_to_string(&self.status).map(|t| {
                    for l in t[self.read.min(t.len())..].lines() {
                        self.log.push(l.to_string());
                    }
                });
                let _ = std::fs::remove_file(&self.status);
                let last = self
                    .log
                    .iter()
                    .rev()
                    .find(|l| l.starts_with("done ") || l.starts_with("error "));
                Some(match last {
                    Some(l) if l.starts_with("done ") => Ok(l[5..].to_string()),
                    Some(l) => Err(l[6..].to_string()),
                    None => {
                        let printed = self.printed.lock().unwrap_or_else(|p| p.into_inner());
                        Err(why_it_ended(code, &printed))
                    }
                })
            }
        }
    }
}

/// Runs this program as the command line with the rights to use disks.
fn run_privileged(args: &[String]) -> std::io::Result<Child> {
    let exe = std::env::current_exe()?;
    let mut full = vec!["--cli".to_string()];
    full.extend(args.iter().cloned());
    privileged(&exe, &full)
}

#[cfg(target_os = "linux")]
fn privileged(exe: &Path, args: &[String]) -> std::io::Result<Child> {
    let root = std::fs::read_to_string("/proc/self/status")
        .map(|s| {
            s.lines()
                .any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(1) == Some("0"))
        })
        .unwrap_or(false);
    if root {
        Command::new(exe)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
    } else {
        Command::new("pkexec")
            .arg(exe)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!("pkexec: {e} (start the creator with sudo instead)"),
                )
            })
    }
}

#[cfg(target_os = "macos")]
fn privileged(exe: &Path, args: &[String]) -> std::io::Result<Child> {
    let sh = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let mut cmd = sh(&exe.display().to_string());
    for a in args {
        cmd.push(' ');
        cmd.push_str(&sh(a));
    }
    cmd.push_str(" > /dev/null 2>&1");
    let script = format!(
        "do shell script \"{}\" with prompt \"The MPFree USB creator needs your password to use the USB stick.\" with administrator privileges",
        cmd.replace('\\', "\\\\").replace('"', "\\\"")
    );
    Command::new("osascript").args(["-e", &script]).spawn()
}

#[cfg(windows)]
fn privileged(exe: &Path, args: &[String]) -> std::io::Result<Child> {
    use std::os::windows::process::CommandExt;
    // Start-Process joins its arguments with spaces: each is quoted for the
    // program's own command line, then for PowerShell.
    let ps = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let list: Vec<String> = args
        .iter()
        .map(|a| ps(&format!("\"{}\"", a.trim_end_matches('\\'))))
        .collect();
    let script = format!(
        "Start-Process -FilePath {} -ArgumentList @({}) -Verb RunAs -WindowStyle Hidden -Wait",
        ps(&exe.display().to_string()),
        list.join(",")
    );
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000) // no console window
        .spawn()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn privileged(_exe: &Path, _args: &[String]) -> std::io::Result<Child> {
    Err(std::io::Error::other("this system is not supported"))
}

#[derive(Default)]
struct Prefs {
    ssh: bool,
    ssh_key: String,
    any_update: bool,
    fresh_data: bool,
}

struct Creator {
    page: Page,
    update_text: String,
    update: Option<UpdateCheck>,
    others: Vec<PathBuf>,
    disks: Vec<DiskInfo>,
    disks_error: Option<String>,
    disks_rx: Option<mpsc::Receiver<std::io::Result<Vec<DiskInfo>>>>,
    disks_at: Instant,
    chosen: Option<String>,
    confirm: bool,
    understood: bool,
    job: Option<Job>,
    outcome: Option<(JobKind, Result<String, String>)>,
    last_log: Vec<String>,
    prefs_open: bool,
    prefs: Prefs,
    key_note: Option<String>,
    backup_to: String,
    programs: Option<PathBuf>,
    jobs_started: u32,
}

impl Creator {
    fn new() -> Creator {
        let found = find_updates();
        let mut c = Creator {
            page: Page::Make,
            update_text: String::new(),
            update: None,
            others: Vec::new(),
            disks: Vec::new(),
            disks_error: None,
            disks_rx: None,
            disks_at: Instant::now() - Duration::from_secs(60),
            chosen: None,
            confirm: false,
            understood: false,
            job: None,
            outcome: None,
            last_log: Vec::new(),
            prefs_open: false,
            prefs: Prefs::default(),
            key_note: None,
            backup_to: home().join("MPFree backups").display().to_string(),
            programs: mpfree_stick::cli::default_programs(),
            jobs_started: 0,
        };
        if let Some((p, _)) = ssh::default_public_key(&home()) {
            c.prefs.ssh_key = p.display().to_string();
        }
        if let Some(u) = best_update(&found) {
            c.update_text = u.path.display().to_string();
            c.update = Some(u);
        }
        c.others = found;
        // Debug builds only: open on another page, for screenshots of the layout.
        #[cfg(debug_assertions)]
        if let Ok(page) = std::env::var("MPFREE_CREATOR_PREVIEW") {
            c.page = match page.as_str() {
                "done" => Page::Done,
                "backup" => Page::Backup,
                "remove" => Page::Remove,
                _ => Page::Make,
            };
        }
        c
    }

    /// The update file to use: a file, or a folder holding it (typed or
    /// dropped), where the supported MPC One update is picked.
    fn set_update(&mut self, p: PathBuf) {
        if p.is_dir() {
            match mpfree_stick::update::find_in_dir(&p, self.prefs.any_update) {
                Ok(found) => return self.set_update(found),
                Err(e) => {
                    self.update_text = p.display().to_string();
                    self.update = Some(UpdateCheck {
                        path: p,
                        build: String::new(),
                        supported: None,
                        problem: Some(format!("{e}.")),
                    });
                    return;
                }
            }
        }
        self.update_text = p.display().to_string();
        self.update = Some(check_update(&p));
    }

    fn refresh_disks(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.disks_rx {
            if let Ok(r) = rx.try_recv() {
                match r {
                    Ok(d) => {
                        self.disks = d;
                        self.disks_error = None;
                    }
                    Err(e) => self.disks_error = Some(e.to_string()),
                }
                self.disks_rx = None;
                if let Some(id) = &self.chosen {
                    if !self.disks.iter().any(|d| &d.id == id) {
                        self.chosen = None;
                    }
                }
            }
        }
        if self.disks_rx.is_none()
            && self.job.is_none()
            && self.disks_at.elapsed() > Duration::from_secs(3)
        {
            let (tx, rx) = mpsc::channel();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(disks::list());
                ctx.request_repaint();
            });
            self.disks_rx = Some(rx);
            self.disks_at = Instant::now();
        }
    }

    fn start(&mut self, kind: JobKind, mut args: Vec<String>) {
        self.jobs_started += 1;
        let status = std::env::temp_dir().join(format!(
            "mpfree-creator-{}-{}.status",
            std::process::id(),
            self.jobs_started
        ));
        if let Err(e) = std::fs::write(&status, b"") {
            self.outcome = Some((kind, Err(format!("{}: {e}", status.display()))));
            return;
        }
        let mut full = vec!["--status".to_string(), status.display().to_string()];
        full.append(&mut args);
        match run_privileged(&full) {
            Ok(mut child) => {
                let printed = collect_output(&mut child);
                self.outcome = None;
                self.job = Some(Job {
                    kind,
                    child,
                    status,
                    read: 0,
                    progress: 0.0,
                    line: "Waiting for permission to use the stick".into(),
                    log: Vec::new(),
                    printed,
                });
            }
            Err(e) => self.outcome = Some((kind, Err(e.to_string()))),
        }
    }

    fn make(&mut self) {
        let (Some(u), Some(id)) = (&self.update, &self.chosen) else {
            return;
        };
        let mut args = vec![
            "write".to_string(),
            u.path.display().to_string(),
            "--device".into(),
            id.clone(),
            "--yes-erase".into(),
            id.clone(),
        ];
        if let Some(p) = &self.programs {
            args.extend(["--programs".into(), p.display().to_string()]);
        }
        if self.prefs.ssh {
            args.extend(["--ssh-key".into(), self.prefs.ssh_key.clone()]);
        }
        if self.prefs.any_update {
            args.push("--any-update".into());
        }
        if self.prefs.fresh_data {
            args.push("--fresh-data".into());
        }
        self.start(JobKind::Make, args);
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("MP").size(28.0).strong().color(look::TEXT));
            ui.add_space(-ui.spacing().item_spacing.x);
            ui.label(
                RichText::new("Free")
                    .size(28.0)
                    .strong()
                    .color(look::ACCENT),
            );
            ui.label(RichText::new("USB creator").size(20.0).color(look::DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Preferences").clicked() {
                    self.prefs_open = true;
                }
                ui.hyperlink_to("Help", MANUAL);
            });
        });
        ui.label(
            RichText::new("Makes the MPFree stick for your MPC One from Akai's own update file. Tested on the MPC One (Gen 1) only.")
                .color(look::DIM),
        );
        ui.add_space(10.0);
        let idle = self.job.is_none();
        ui.horizontal(|ui| {
            let make = matches!(self.page, Page::Make | Page::Done);
            if tab(ui, idle, make, "Make a stick") {
                self.page = Page::Make;
            }
            if tab(ui, idle, self.page == Page::Backup, "Copy the backup") {
                self.page = Page::Backup;
                self.outcome = None;
            }
            if tab(
                ui,
                idle,
                self.page == Page::Remove,
                "Take MPFree off my MPC",
            ) {
                self.page = Page::Remove;
                self.outcome = None;
            }
        });
        ui.add_space(14.0);
    }

    fn disk_picker(&mut self, ui: &mut egui::Ui) {
        if let Some(e) = &self.disks_error {
            status(ui, Tone::Bad, &format!("Could not list the USB disks: {e}"));
        }
        if self.disks.is_empty() {
            ui.label(
                RichText::new(
                    "No USB stick found yet. Put one into this computer and it shows up here.",
                )
                .color(look::DIM),
            );
        }
        for d in self.disks.clone() {
            let small = d.bytes < MIN_STICK;
            let mut text = d.describe();
            if small {
                text.push_str("   (too small: 16 GB or more)");
            }
            ui.add_enabled_ui(!small, |ui| {
                ui.radio_value(&mut self.chosen, Some(d.id.clone()), text);
            });
        }
    }

    fn progress(&mut self, ui: &mut egui::Ui) {
        if let Some(job) = &self.job {
            ui.add_space(10.0);
            egui::Frame::new()
                .fill(look::RAISED)
                .corner_radius(10)
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&job.line).strong());
                    ui.add_space(4.0);
                    ui.add(
                        egui::ProgressBar::new(job.progress)
                            .show_percentage()
                            .fill(look::ACCENT)
                            .animate(true),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Keep the stick in until this is done.").color(look::DIM),
                    );
                    egui::CollapsingHeader::new(RichText::new("Details").color(look::DIM)).show(
                        ui,
                        |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(160.0)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    for l in &job.log {
                                        ui.monospace(l);
                                    }
                                });
                        },
                    );
                });
        }
    }

    fn outcome_line(&self, ui: &mut egui::Ui, kind: JobKind) {
        if let Some((k, r)) = &self.outcome {
            if *k == kind {
                ui.add_space(8.0);
                match r {
                    Ok(t) => {
                        let said = match kind {
                            JobKind::RemoveMode => "Done: this is now a removal stick. Go on with step 2.".to_string(),
                            JobKind::InstallMode => "Done: this stick starts MPFree again, with your settings and files.".to_string(),
                            _ => format!("Done: {t}."),
                        };
                        status(ui, Tone::Good, &said)
                    }
                    Err(e) => status(ui, Tone::Bad, &format!("That did not work: {e}")),
                }
            }
        }
    }

    fn make_page(&mut self, ui: &mut egui::Ui) {
        if self.programs.is_none() {
            status(
                ui,
                Tone::Bad,
                "This copy of the creator is missing MPFree's programs for the MPC (the stick-programs folder next to it). Download the creator again.",
            );
            ui.add_space(10.0);
        }
        let mut update_ok = false;
        card(ui, |ui| {
            let found = self
                .update
                .as_ref()
                .is_some_and(|u| u.problem.is_none() && u.supported.is_some());
            step_head(ui, 1, "Akai's update file", found);
            ui.label("Update your MPC One to firmware 3.10.1 with Akai's instructions first. Then pick the update file you downloaded from Akai, or drop it on this window.");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.update_text)
                        .hint_text("The update file, or the folder it is in")
                        .desired_width(ui.available_width() - 130.0),
                );
                if r.lost_focus() && !self.update_text.trim().is_empty() {
                    let p = PathBuf::from(self.update_text.trim());
                    self.set_update(p);
                }
                if ui.button("Search again").clicked() {
                    self.others = find_updates();
                    if let Some(u) = best_update(&self.others) {
                        self.update_text = u.path.display().to_string();
                        self.update = Some(u);
                    }
                }
            });
            if self.others.len() > 1 {
                let others = self.others.clone();
                egui::CollapsingHeader::new(
                    RichText::new(format!(
                        "{} update files found on this computer",
                        others.len()
                    ))
                    .color(look::DIM),
                )
                .show(ui, |ui| {
                    for p in others {
                        if ui
                            .selectable_label(
                                self.update.as_ref().is_some_and(|u| u.path == p),
                                p.display().to_string(),
                            )
                            .clicked()
                        {
                            self.set_update(p);
                        }
                    }
                });
            }
            ui.add_space(4.0);
            match &self.update {
                None => {
                    ui.label(
                        RichText::new("No Akai update file found in Downloads, Desktop or Documents. Type where it is above.")
                            .color(look::DIM),
                    );
                }
                Some(u) => match (&u.problem, u.supported) {
                    (Some(p), _) => status(ui, Tone::Bad, p),
                    (None, Some(s)) => {
                        status(ui, Tone::Good, &format!("{}, Akai build {}", s.firmware, u.build));
                        update_ok = true;
                    }
                    (None, None) if self.prefs.any_update => {
                        status(ui, Tone::Warn, &format!("Akai build {}: not a version MPFree supports. Taking it anyway (developer option).", u.build));
                        update_ok = true;
                    }
                    (None, None) => status(
                        ui,
                        Tone::Bad,
                        &format!("This update (Akai build {}) is not one MPFree supports yet. MPFree needs MPC 3.10.1 for the MPC One.", u.build),
                    ),
                },
            }
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            step_head(ui, 2, "Your USB stick", self.chosen.is_some());
            ui.label("MPFree runs from this stick and keeps your settings, files and a backup of your MPC on it. Use one of 16 GB or more; 32 GB is best.");
            ui.add_space(6.0);
            self.disk_picker(ui);
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            step_head(ui, 3, "Make the stick", false);
            ui.label("Erases the stick and puts MPFree on it. It takes a few minutes, and asks for this computer's administrator password.");
            if self.prefs.ssh {
                ui.label(RichText::new("Developer SSH is on (Preferences).").color(look::WARN));
            }
            ui.add_space(8.0);
            let ready = update_ok
                && self.chosen.is_some()
                && self.job.is_none()
                && self.programs.is_some()
                && (!self.prefs.ssh || !self.prefs.ssh_key.is_empty());
            if primary(ui, ready, "Make the stick").clicked() {
                self.confirm = true;
                self.understood = false;
            }
            self.progress(ui);
            self.outcome_line(ui, JobKind::Make);
        });
    }

    fn done_page(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                sign(ui, Tone::Good, 26.0);
                ui.label(
                    RichText::new("The stick is ready")
                        .size(22.0)
                        .strong()
                        .color(look::GOOD),
                );
            });
            ui.add_space(4.0);
            ui.label("Now, on your MPC:");
            ui.add_space(8.0);
            for (n, step) in [
                "Eject the stick from this computer and put it into the MPC's USB port.",
                "Turn the MPC on. Akai's software starts as usual.",
                "Switch on the developer panel (MENU > Preferences > Enhancements), then hold SHIFT, press MENU, choose Developer and switch developer mode on. This is needed once.",
                "When the MPC asks whether to run the script on the stick (autorun.sh), tap Yes. It adds MPFree's start-up helper and restarts the MPC.",
                "The MPC starts MPFree from the stick.",
                "Afterwards, switch developer mode off again: stick out, in Akai's software, SHIFT + MENU > Developer.",
            ]
            .iter()
            .enumerate()
            {
                numbered(ui, n + 1, step);
            }
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(
                RichText::new("The first start takes a while")
                    .size(17.0)
                    .strong(),
            );
            ui.add_space(4.0);
            ui.label("MPFree backs up your MPC's internal storage onto the stick, then copies its drum kits and samples into MPFree. It can take 20 to 30 minutes, once. The screen shows how far it is and how long is left. Keep the stick in and the MPC on.");
            ui.add_space(4.0);
            ui.label(RichText::new("After that, MPFree starts whenever the stick is in at power-on. Without the stick, Akai's software starts.").color(look::DIM));
        });
        if let Some(l) = self
            .last_log
            .iter()
            .find(|l| l.contains("developer SSH on"))
        {
            ui.add_space(12.0);
            card(ui, |ui| {
                ui.label(
                    RichText::new(l.trim_start_matches("note "))
                        .strong()
                        .color(look::WARN),
                );
                ui.label("Connect with: ssh -i (your key) -p 2222 root@(the MPC's address), and check the host key matches.");
            });
        }
        ui.add_space(12.0);
        ui.label("Then copy the backup of your MPC to this computer and keep it safe (Copy the backup, above).");
        ui.hyperlink_to(
            "The whole process, with pictures, in the user manual",
            MANUAL,
        );
        ui.add_space(10.0);
        if ui.button("Make another stick").clicked() {
            self.page = Page::Make;
            self.outcome = None;
        }
    }

    fn backup_page(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(
                RichText::new("Copy your MPC's backup to this computer")
                    .size(19.0)
                    .strong(),
            );
            ui.add_space(4.0);
            ui.label("At its first start, MPFree backs up your MPC's internal storage onto the stick. Copy it here and keep it safe: it is the way back if anything ever goes wrong.");
            ui.label(
                RichText::new("The copy is checked as it is made. Nothing on the stick changes.")
                    .color(look::DIM),
            );
            ui.add_space(10.0);
            ui.label(RichText::new("The stick").strong());
            self.disk_picker(ui);
            ui.add_space(8.0);
            ui.label(RichText::new("Copy into this folder").strong());
            ui.add(
                egui::TextEdit::singleline(&mut self.backup_to).desired_width(ui.available_width()),
            );
            ui.add_space(10.0);
            let ready =
                self.chosen.is_some() && self.job.is_none() && !self.backup_to.trim().is_empty();
            if primary(ui, ready, "Copy the backup").clicked() {
                let to = self.backup_to.trim().to_string();
                let _ = std::fs::create_dir_all(&to);
                let id = self.chosen.clone().unwrap_or_default();
                self.start(
                    JobKind::Backup,
                    vec![
                        "copy-backup".into(),
                        "--device".into(),
                        id,
                        "--to".into(),
                        to,
                    ],
                );
            }
            self.progress(ui);
            self.outcome_line(ui, JobKind::Backup);
        });
    }

    fn remove_page(&mut self, ui: &mut egui::Ui) {
        let ready = self.chosen.is_some() && self.job.is_none();
        card(ui, |ui| {
            ui.label(
                RichText::new("Take MPFree off your MPC")
                    .size(19.0)
                    .strong(),
            );
            ui.add_space(4.0);
            ui.label("MPFree changes one thing on your MPC: a small start-up helper that starts MPFree when the stick is in. Everything else stays as Akai made it. This takes the helper off, with the same stick:");
            ui.add_space(10.0);
            step_head(ui, 1, "Make this stick a removal stick", false);
            ui.label("A removal stick no longer starts MPFree. Your MPFree settings, files and backup on it are kept.");
            ui.add_space(6.0);
            self.disk_picker(ui);
            ui.add_space(8.0);
            if primary(ui, ready, "Make it a removal stick").clicked() {
                let id = self.chosen.clone().unwrap_or_default();
                self.start(
                    JobKind::RemoveMode,
                    vec!["setup-mode".into(), "--device".into(), id, "remove".into()],
                );
            }
            if self
                .job
                .as_ref()
                .is_some_and(|j| j.kind == JobKind::RemoveMode)
            {
                self.progress(ui);
            }
            self.outcome_line(ui, JobKind::RemoveMode);
            ui.add_space(14.0);
            step_head(ui, 2, "Put it in the MPC and turn the MPC on", false);
            ui.label("Akai's software starts, not MPFree.");
            ui.add_space(14.0);
            step_head(
                ui,
                3,
                "Tap Yes when the MPC offers to run the stick's script",
                false,
            );
            ui.label("With developer mode on (SHIFT + MENU > Developer), the MPC asks to run autorun.sh. Tap Yes: it takes the helper off and restarts the MPC into Akai's software.");
            ui.add_space(8.0);
            status(ui, Tone::Good, "That's all: your MPC is back as Akai made it. If the MPC asks again later, tapping Yes does nothing more.");
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(RichText::new("Changed your mind?").size(17.0).strong());
            ui.add_space(4.0);
            ui.label("Make the removal stick an MPFree stick again: it starts MPFree again, with your settings and files. If you already took the helper off the MPC, put the start-up helper back as you did the first time (developer mode on, tap Yes).");
            ui.add_space(8.0);
            if ui
                .add_enabled(ready, egui::Button::new("Make it an MPFree stick again"))
                .clicked()
            {
                let id = self.chosen.clone().unwrap_or_default();
                self.start(
                    JobKind::InstallMode,
                    vec!["setup-mode".into(), "--device".into(), id, "install".into()],
                );
            }
            if self
                .job
                .as_ref()
                .is_some_and(|j| j.kind == JobKind::InstallMode)
            {
                self.progress(ui);
            }
            self.outcome_line(ui, JobKind::InstallMode);
            if self.chosen.is_none() {
                ui.label(RichText::new("Pick the stick in step 1 first.").color(look::DIM));
            }
        });
    }

    fn prefs_window(&mut self, ctx: &egui::Context) {
        let mut open = self.prefs_open;
        egui::Window::new("Preferences")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Developer options").size(17.0).strong());
                ui.label(RichText::new("For people working on MPFree. Leave them off to just play.").color(look::DIM));
                ui.add_space(10.0);
                option(ui, &mut self.prefs.ssh, "Developer SSH", "Log in to the MPC over Ethernet while MPFree runs: as root, with your SSH key only (no password), on port 2222.");
                if self.prefs.ssh {
                    ui.indent("ssh-key", |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Your public key file:");
                            ui.text_edit_singleline(&mut self.prefs.ssh_key);
                        });
                        let check = std::fs::read_to_string(self.prefs.ssh_key.trim())
                            .map_err(|e| e.to_string())
                            .and_then(|t| ssh::check_public_keys(&t).map_err(|e| e.to_string()));
                        match check {
                            Ok(_) => status(ui, Tone::Good, "A public key"),
                            Err(e) => status(ui, Tone::Bad, &e),
                        }
                        if ui.button("Make a key for me").clicked() {
                            self.key_note = Some(match ssh::make_user_key(&home().join(".ssh"), "mpfree_ed25519") {
                                Ok((k, p)) => {
                                    self.prefs.ssh_key = p.display().to_string();
                                    format!("Made {} (keep it private) and {}.", k.display(), p.display())
                                }
                                Err(e) => e.to_string(),
                            });
                        }
                        if let Some(n) = &self.key_note {
                            ui.label(n);
                        }
                    });
                }
                ui.add_space(8.0);
                option(ui, &mut self.prefs.fresh_data, "Start the stick's data fresh", "When making a stick that already has MPFree: erase its settings, files and backups too.");
                ui.add_space(8.0);
                option(ui, &mut self.prefs.any_update, "Take any Akai update (not supported)", "Only for testing a new Akai version. Such a stick may not start.");
            });
        self.prefs_open = open;
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        if !self.confirm {
            return;
        }
        let name = self
            .disks
            .iter()
            .find(|d| Some(&d.id) == self.chosen.as_ref())
            .map(|d| d.describe())
            .unwrap_or_default();
        let mut go = false;
        let r = egui::Modal::new(egui::Id::unique("confirm")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label(RichText::new("Erase this stick?").size(20.0).strong());
            ui.add_space(6.0);
            ui.label(RichText::new(&name).strong().color(look::ACCENT));
            ui.add_space(6.0);
            ui.label("Everything on it is erased and replaced by MPFree.");
            if !self.prefs.fresh_data {
                ui.label(RichText::new("If it is already an MPFree stick, its MPFree settings, files and backups are kept.").color(look::DIM));
            }
            ui.add_space(8.0);
            ui.checkbox(&mut self.understood, "Yes, erase this stick");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    self.confirm = false;
                }
                if primary(ui, self.understood, "Erase and make the stick").clicked() {
                    go = true;
                }
            });
        });
        if r.should_close() {
            self.confirm = false;
        }
        if go {
            self.confirm = false;
            self.make();
        }
    }
}

/// MPFree's own colours (the pad player's default theme, mpfree-ui's `Theme::dark`).
mod look {
    use eframe::egui::Color32;
    pub const BG: Color32 = Color32::from_rgb(0x0B, 0x0D, 0x12);
    pub const SURFACE: Color32 = Color32::from_rgb(0x15, 0x19, 0x22);
    pub const RAISED: Color32 = Color32::from_rgb(0x22, 0x28, 0x38);
    pub const HOVER: Color32 = Color32::from_rgb(0x2C, 0x34, 0x48);
    pub const PRESSED: Color32 = Color32::from_rgb(0x36, 0x40, 0x58);
    pub const OUTLINE: Color32 = Color32::from_rgb(0x2B, 0x32, 0x45);
    pub const FIELD: Color32 = Color32::from_rgb(0x0F, 0x12, 0x19);
    pub const TEXT: Color32 = Color32::from_rgb(0xE9, 0xEC, 0xF2);
    pub const DIM: Color32 = Color32::from_rgb(0x98, 0xA1, 0xB3);
    pub const FAINT: Color32 = Color32::from_rgb(0x5F, 0x68, 0x7B);
    pub const ACCENT: Color32 = Color32::from_rgb(0x6E, 0xA8, 0xFE);
    pub const GOOD: Color32 = Color32::from_rgb(0x3D, 0xDC, 0x97);
    pub const WARN: Color32 = Color32::from_rgb(0xFF, 0xC1, 0x45);
    pub const BAD: Color32 = Color32::from_rgb(0xFF, 0x5C, 0x5C);
}

/// The window's look: MPFree's colours, larger type, more room between things.
fn style(ctx: &egui::Context) {
    use egui::{FontId, Stroke, TextStyle};
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Heading, FontId::proportional(24.0)),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Button, FontId::proportional(15.0)),
            (TextStyle::Small, FontId::proportional(12.5)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(10.0, 7.0);
        s.spacing.button_padding = egui::vec2(14.0, 6.0);
        s.spacing.interact_size.y = 28.0;
        let v = &mut s.visuals;
        v.dark_mode = true;
        v.panel_fill = look::BG;
        v.window_fill = look::SURFACE;
        v.window_stroke = Stroke::new(1.0, look::OUTLINE);
        v.window_corner_radius = 12.into();
        v.extreme_bg_color = look::FIELD;
        v.faint_bg_color = look::SURFACE;
        v.hyperlink_color = look::ACCENT;
        v.warn_fg_color = look::WARN;
        v.error_fg_color = look::BAD;
        v.selection.bg_fill = look::ACCENT.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, look::ACCENT);
        let w = &mut v.widgets;
        w.noninteractive.fg_stroke = Stroke::new(1.0, look::TEXT);
        w.noninteractive.bg_stroke = Stroke::new(1.0, look::OUTLINE);
        for (wv, fill) in [
            (&mut w.inactive, look::RAISED),
            (&mut w.hovered, look::HOVER),
            (&mut w.active, look::PRESSED),
            (&mut w.open, look::HOVER),
        ] {
            wv.bg_fill = fill;
            wv.weak_bg_fill = fill;
            wv.corner_radius = 8.into();
            wv.fg_stroke = Stroke::new(1.0, look::TEXT);
        }
        w.hovered.bg_stroke = Stroke::new(1.0, look::ACCENT.gamma_multiply(0.6));
    });
}

#[derive(Clone, Copy)]
enum Tone {
    Good,
    Warn,
    Bad,
}

/// A panel holding one thing to do.
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(look::SURFACE)
        .stroke(egui::Stroke::new(1.0, look::OUTLINE))
        .corner_radius(14)
        .inner_margin(egui::Margin::same(20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A tick, a cross or an exclamation mark in a circle of `size` points, drawn
/// (the window's fonts have no such signs).
fn sign(ui: &mut egui::Ui, tone: Tone, size: f32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let (c, k) = (r.center(), size / 16.0);
    let colour = match tone {
        Tone::Good => look::GOOD,
        Tone::Warn => look::WARN,
        Tone::Bad => look::BAD,
    };
    let p = ui.painter();
    p.circle_filled(c, size / 2.0, colour);
    let ink = egui::Stroke::new(1.8 * k, look::BG);
    let at = |x: f32, y: f32| c + egui::vec2(x * k, y * k);
    match tone {
        Tone::Good => {
            p.line_segment([at(-4.0, 0.0), at(-1.2, 3.0)], ink);
            p.line_segment([at(-1.2, 3.0), at(4.2, -3.2)], ink);
        }
        Tone::Bad => {
            p.line_segment([at(-3.2, -3.2), at(3.2, 3.2)], ink);
            p.line_segment([at(-3.2, 3.2), at(3.2, -3.2)], ink);
        }
        Tone::Warn => {
            p.line_segment([at(0.0, -4.0), at(0.0, 1.2)], ink);
            p.circle_filled(at(0.0, 4.0), 1.1 * k, look::BG);
        }
    }
}

/// A step's number in a circle (a tick once it is done) and its title.
fn step_head(ui: &mut egui::Ui, n: usize, title: &str, done: bool) {
    ui.horizontal(|ui| {
        if done {
            sign(ui, Tone::Good, 28.0);
        } else {
            let (r, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 14.0, look::ACCENT);
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                n.to_string(),
                egui::FontId::proportional(15.0),
                look::BG,
            );
        }
        ui.label(RichText::new(title).size(18.0).strong());
    });
    ui.add_space(2.0);
}

/// A numbered line in a list of steps.
fn numbered(ui: &mut egui::Ui, n: usize, text: &str) {
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(24.0, 22.0), egui::Sense::hover());
        ui.painter().text(
            r.left_center() + egui::vec2(2.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{n}."),
            egui::FontId::proportional(15.0),
            look::ACCENT,
        );
        ui.add(egui::Label::new(text).wrap());
    });
    ui.add_space(4.0);
}

/// A result or a problem, in its colour, with its sign in front.
fn status(ui: &mut egui::Ui, tone: Tone, text: &str) {
    let colour = match tone {
        Tone::Good => look::GOOD,
        Tone::Warn => look::WARN,
        Tone::Bad => look::BAD,
    };
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.add_space(1.0);
            sign(ui, tone, 17.0);
        });
        ui.add(egui::Label::new(RichText::new(text).color(colour)).wrap());
    });
}

/// The one main button of a page.
fn primary(ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
    let (fill, ink) = if enabled {
        (look::ACCENT, look::BG)
    } else {
        (look::RAISED, look::FAINT)
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).strong().size(16.0).color(ink))
            .fill(fill)
            .min_size(egui::vec2(200.0, 38.0)),
    )
}

/// A tab along the top: the page it opens, lit when it is the one showing.
fn tab(ui: &mut egui::Ui, enabled: bool, selected: bool, text: &str) -> bool {
    let (fill, ink) = if selected {
        (look::RAISED, look::TEXT)
    } else {
        (egui::Color32::TRANSPARENT, look::DIM)
    };
    let b = egui::Button::new(RichText::new(text).color(ink).strong())
        .fill(fill)
        .stroke(egui::Stroke::new(
            1.0,
            if selected {
                look::OUTLINE
            } else {
                egui::Color32::TRANSPARENT
            },
        ))
        .min_size(egui::vec2(0.0, 32.0));
    ui.add_enabled(enabled, b).clicked()
}

/// A checkbox with a line under it saying what it does.
fn option(ui: &mut egui::Ui, on: &mut bool, name: &str, what: &str) {
    ui.checkbox(on, RichText::new(name).strong());
    ui.indent(name, |ui| {
        ui.label(RichText::new(what).color(look::DIM));
    });
}

impl eframe::App for Creator {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.refresh_disks(&ctx);
        if let Some(job) = &mut self.job {
            if let Some(result) = job.poll() {
                let kind = job.kind;
                if kind == JobKind::Make && result.is_ok() {
                    self.page = Page::Done;
                }
                self.outcome = Some((kind, result));
                self.last_log = std::mem::take(&mut job.log);
                self.job = None;
                self.disks_at = Instant::now() - Duration::from_secs(60);
            } else {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if let Some(p) = dropped.into_iter().next() {
            if self.job.is_none() {
                self.page = Page::Make;
                self.set_update(p);
            }
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(look::BG)
                    .inner_margin(egui::Margin::symmetric(0, 16)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // One column, at most 600 points wide, in the middle of the window
                    // (clear of the scroll bar).
                    let w = (ui.available_width() - 64.0).clamp(300.0, 600.0);
                    let side = ((ui.available_width() - w) / 2.0).max(0.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(side);
                        ui.vertical(|ui| {
                            ui.set_width(w);
                            self.header(ui);
                            match self.page {
                                Page::Make => self.make_page(ui),
                                Page::Done => self.done_page(ui),
                                Page::Backup => self.backup_page(ui),
                                Page::Remove => self.remove_page(ui),
                            }
                            ui.add_space(16.0);
                            ui.label(RichText::new(CREATOR).small().color(look::FAINT));
                        });
                    });
                });
            });
        self.prefs_window(&ctx);
        self.confirm_window(&ctx);
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::why_it_ended;

    #[test]
    fn says_why_a_step_ended_without_a_result() {
        let printed = vec![
            "note writing".to_string(),
            "error the status file /tmp/x.status: Permission denied (os error 13)".to_string(),
        ];
        assert_eq!(
            why_it_ended(Some(1), &printed),
            "the status file /tmp/x.status: Permission denied (os error 13)"
        );
        assert!(
            why_it_ended(Some(126), &[]).starts_with("The administrator password was not accepted")
        );
        assert!(why_it_ended(Some(127), &[]).contains("pkexec refused"));
        assert_eq!(
            why_it_ended(Some(3), &["Segmentation fault".to_string()]),
            "It stopped without saying why (exit code 3): Segmentation fault."
        );
        assert_eq!(
            why_it_ended(None, &[]),
            "It stopped without saying why (exit code none)."
        );
    }
}
