//! What a crash leaves for the next start, and the safe start it brings.
//!
//! The app has no Quit: the power switch ends it, and the report's folder
//! (`/run/mpfree-app`) is in RAM, so a power-off leaves nothing behind.
//! While the app or the pad player runs, [`RUNNING`] in that folder holds
//! its pid ([`start`]). When the run ends well (the pad player's STOP, or a
//! start that stops with an error it reports: [`Running`] is dropped) the
//! file says `ended`; a stop from outside (SIGTERM as `systemctl stop`
//! sends it, SIGINT, SIGHUP) removes it (`sys::remove_on_stop`). So a
//! `running` file that still holds a pid at the start means the last run in
//! this boot crashed: a Rust panic (whose line [`note_panics`] adds to the
//! file), a crash outside Rust, or the kernel's out-of-memory killer.
//!
//! Then the start is a **safe start**: the new, empty project stays even
//! when Prefs > At start says Last project, so a project that crashes the
//! app as it opens cannot do it again at every start. The musician is told
//! why (the hint line and the Events log; the report says it too), and
//! BROWSE still opens the last project. On the test boot's stick a crash
//! restarts the app (`Restart=on-failure` in its unit), so this is what
//! the musician sees a few seconds after a crash.
//!
//! A panic ends the program in a release build (`panic = "abort"`), with
//! nothing in the report and nothing on the screen. So [`note_panics`]
//! writes the panic's line, `PANIC: thread 'name' at file:line:column:
//! message`, to the report and to `running`, synced, before Rust prints its
//! own message on stderr (which reaches the system log). Report lines still
//! queued for the `mpfree-report` thread at that moment are lost.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::panic::{self, Location};
use std::path::{Path, PathBuf};
use std::thread;

use crate::sys;

/// The file in the report's folder that says a run is going.
pub const RUNNING: &str = "running";
/// How a panic's line starts, in the report and in [`RUNNING`].
pub const PANIC: &str = "PANIC: ";
/// What [`RUNNING`] says once its run ended well.
const ENDED: &str = "ended\n";

/// How the last run in this boot ended, as this start finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LastRun {
    /// It ended well, or there was none in this boot.
    Fine,
    /// It crashed: its panic's line, if it was a panic.
    Crashed(Option<String>),
}

/// This run's [`RUNNING`] file. When this is dropped (the run ended, or
/// stopped with an error it reported) the file says [`ENDED`], but not
/// while a panic unwinds (a test build's): the panic's line stays for the
/// next start.
#[derive(Debug)]
pub struct Running {
    path: PathBuf,
}

impl Drop for Running {
    fn drop(&mut self) {
        if thread::panicking() {
            return;
        }
        sys::forget_on_stop();
        let _ = fs::write(&self.path, ENDED);
    }
}

/// Looks at how the last run ended, then marks this one as going: writes
/// [`RUNNING`] in `dir` with this pid and has a stop from outside remove it.
/// Without the file (an error) the run goes on, and a crash would not be
/// noticed at the next start.
pub fn start(dir: &Path) -> (LastRun, Result<Running, String>) {
    let path = dir.join(RUNNING);
    let last = match fs::read_to_string(&path) {
        Ok(text) if text == ENDED => LastRun::Fine,
        Ok(text) => LastRun::Crashed(panic_in(&text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => LastRun::Fine,
        Err(_) => LastRun::Crashed(None),
    };
    let running = mark(dir, &path)
        .map(|_| Running { path })
        .map_err(|e| e.to_string());
    (last, running)
}

/// Writes this run's [`RUNNING`] file (its pid) and has a stop remove it.
fn mark(dir: &Path, path: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(path, format!("pid {}\n", std::process::id()))?;
    sys::remove_on_stop(path)
}

/// The first panic line in a [`RUNNING`] file, without its `PANIC: `.
fn panic_in(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix(PANIC))
        .map(|l| l.trim().to_string())
}

/// The report's line for a safe start.
pub fn report_line(last: &LastRun) -> Option<String> {
    let LastRun::Crashed(why) = last else {
        return None;
    };
    Some(format!(
        "safe start: the last run crashed ({}); a new project, even if Prefs > At start says Last project",
        why.as_deref()
            .unwrap_or("no panic line: a crash outside Rust, or out of memory; the system log may say")
    ))
}

/// The screen's words for a safe start (the hint line, briefly, and the
/// Events log): `open_last` is Prefs > At start, `last` the project it
/// would have opened.
pub fn screen_line(open_last: bool, last: Option<&str>) -> String {
    match (open_last, last) {
        (true, Some(name)) => {
            format!("The last run crashed, so {name} was not opened. BROWSE opens it.")
        }
        _ => "The last run crashed; this is a new project.".to_string(),
    }
}

/// A panic's line: `PANIC: thread 'name' at file:line:column: message`.
pub fn panic_line(
    thread: Option<&str>,
    at: Option<&Location<'_>>,
    message: Option<&str>,
) -> String {
    let who = match thread {
        Some(name) => format!("thread '{name}'"),
        None => "an unnamed thread".to_string(),
    };
    let place = match at {
        Some(l) => format!(" at {}:{}:{}", l.file(), l.line(), l.column()),
        None => String::new(),
    };
    let what = message.unwrap_or("(no message)");
    // One line, whatever the message holds.
    let what = what.replace(['\n', '\r'], " ");
    format!("{PANIC}{who}{place}: {what}")
}

/// From now on a panic, before anything else, appends its line to `report`
/// (made if missing) and to `running` (only if it is there: a run is
/// going), each synced, then lets Rust's own message follow on stderr.
pub fn note_panics(report: PathBuf, running: PathBuf) {
    let rust_says = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let line = panic_line(
            thread::current().name(),
            info.location(),
            info.payload_as_str(),
        );
        let _ = report.parent().map(fs::create_dir_all);
        let to_report = OpenOptions::new().create(true).append(true).open(&report);
        let to_running = OpenOptions::new().append(true).open(&running);
        for file in [to_report, to_running].into_iter().flatten() {
            let mut f = file;
            let _ = writeln!(f, "{line}");
            let _ = f.sync_data();
        }
        rust_says(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mpfree-crash-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_panic_line_names_the_thread_the_place_and_the_message() {
        let here = Location::caller();
        let line = panic_line(
            Some("mpfree-audio"),
            Some(here),
            Some("index out of bounds"),
        );
        assert_eq!(
            line,
            format!(
                "PANIC: thread 'mpfree-audio' at {}:{}:{}: index out of bounds",
                here.file(),
                here.line(),
                here.column()
            )
        );
        assert_eq!(
            panic_line(None, None, Some("two\nlines")),
            "PANIC: an unnamed thread: two lines"
        );
        assert_eq!(
            panic_line(None, None, None),
            "PANIC: an unnamed thread: (no message)"
        );
    }

    #[test]
    fn a_run_that_ends_well_leaves_a_normal_start() {
        let d = scratch("fine");
        let (last, running) = start(&d);
        assert_eq!(last, LastRun::Fine, "no file: no run in this boot");
        assert_eq!(
            fs::read_to_string(d.join(RUNNING)).unwrap(),
            format!("pid {}\n", std::process::id())
        );
        drop(running.unwrap());
        assert_eq!(fs::read_to_string(d.join(RUNNING)).unwrap(), ENDED);
        let (last, running) = start(&d);
        assert_eq!(last, LastRun::Fine);
        assert_eq!(report_line(&last), None);
        drop(running.unwrap());
        // A stop from outside removes the file (here, with its folder):
        // also a normal start.
        fs::remove_dir_all(&d).unwrap();
        let (last, running) = start(&d);
        assert_eq!(last, LastRun::Fine);
        drop(running.unwrap());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_run_that_never_ended_makes_a_safe_start_with_its_panic() {
        let d = scratch("crashed");
        fs::create_dir_all(&d).unwrap();
        // Killed outright (out of memory): only the pid.
        fs::write(d.join(RUNNING), "pid 4242\n").unwrap();
        let (last, running) = start(&d);
        assert_eq!(last, LastRun::Crashed(None));
        assert!(report_line(&last).unwrap().contains("no panic line"));
        drop(running.unwrap());
        // A panic added its line.
        fs::write(
            d.join(RUNNING),
            "pid 4242\nPANIC: thread 'mpfree-audio' at src/a.rs:1:2: boom\n",
        )
        .unwrap();
        let (last, running) = start(&d);
        assert_eq!(
            last,
            LastRun::Crashed(Some("thread 'mpfree-audio' at src/a.rs:1:2: boom".into()))
        );
        assert_eq!(
            report_line(&last).unwrap(),
            "safe start: the last run crashed (thread 'mpfree-audio' at src/a.rs:1:2: boom); \
             a new project, even if Prefs > At start says Last project"
        );
        // This run's file replaced the old one.
        assert_eq!(
            fs::read_to_string(d.join(RUNNING)).unwrap(),
            format!("pid {}\n", std::process::id())
        );
        drop(running.unwrap());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_screen_says_what_was_not_opened() {
        assert_eq!(
            screen_line(true, Some("Night Drive")),
            "The last run crashed, so Night Drive was not opened. BROWSE opens it."
        );
        assert_eq!(
            screen_line(false, Some("Night Drive")),
            "The last run crashed; this is a new project."
        );
        assert_eq!(
            screen_line(true, None),
            "The last run crashed; this is a new project."
        );
    }

    /// A panic says where and why in the report and in `running`, so the
    /// next start is a safe start that can say why. (The hook is the
    /// process's, so a test thread panicking meanwhile adds its line too.)
    #[test]
    fn a_panic_is_written_to_the_report_and_makes_the_next_start_safe() {
        let d = scratch("panic");
        let (_, running) = start(&d);
        let running = running.unwrap();
        note_panics(d.join("report.txt"), d.join(RUNNING));
        let t = thread::Builder::new()
            .name("mpfree-boom".into())
            .spawn(|| panic!("boom"))
            .unwrap();
        assert!(t.join().is_err());
        // Back to Rust's own hook.
        drop(panic::take_hook());
        let report = fs::read_to_string(d.join("report.txt")).unwrap();
        let line = report
            .lines()
            .find(|l| l.starts_with("PANIC: thread 'mpfree-boom'"))
            .unwrap();
        assert!(
            line.starts_with("PANIC: thread 'mpfree-boom' at mpfree-app/src/crash.rs:")
                && line.ends_with(": boom"),
            "{line}"
        );
        let mark = fs::read_to_string(d.join(RUNNING)).unwrap();
        assert!(mark.lines().any(|l| l == line), "{mark}");
        // A release build aborts at the panic: this run's file is never
        // marked ended.
        std::mem::forget(running);
        let (last, running) = start(&d);
        assert!(
            matches!(&last, LastRun::Crashed(Some(why)) if why.starts_with("thread 'mpfree-boom'")),
            "{last:?}"
        );
        drop(running.unwrap());
        let _ = fs::remove_dir_all(&d);
    }
}
