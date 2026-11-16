//! The MIDI monitor's lines in files: the logs the Monitor's Save log
//! button keeps in the data folder, and a file in RAM for the workstation
//! console (`tools/mpc-one/midi_console.py`, which follows it over SSH,
//! read only).
//!
//! The `mpfree-midilog` thread looks at the I/O view's model four times a
//! second, for both.
//!
//! **Save log.** When the Monitor asks (`Asks::save_log`), the thread
//! names the log so that it replaces no file
//! (`mpfree_midi_ui::log::file_name`, against the store's list of the
//! folder), hands it to the store, which writes it as it writes every
//! file (storage-spec.md: `files/midi-logs/<name>`, under `--data` only),
//! waits for the store's writer, and puts what happened in
//! `Live::log_saved` for the Monitor to say. Without a store that saves,
//! nothing is written and the Monitor says so.
//!
//! **The console's file.** While the developer tools are on (Prefs › Show
//! developer tools), the thread appends the monitor's new lines to
//! `midi.log` in the app's folder in RAM (`/run/mpfree-app`), one line each
//! with tab-separated fields (`mpfree_midi_ui::log::console_line`). At 1 MiB
//! the file becomes `midi.log.1` (replacing the one before) and a new one
//! starts, so the two never hold more than 2 MiB. Before the file is set
//! aside it gets a last line, [`ROTATED`], so a reader knows to open the
//! new one. A file left by an earlier start is set aside the same way.
//! Nothing is written there while the tools are off, and the folder is
//! never made: without it (a workstation) the console's file is off for
//! the run and saving goes on. It is gone when the MPC is switched off.
//!
//! The sound loop never waits for this thread: the loop only `try_lock`s
//! the model, and the thread holds the lock only while it words the new
//! lines or takes an ask.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use mpfree_midi_ui::log::{file_name, OWNER};
use mpfree_midi_ui::{LogToSave, Shared};
use mpfree_store::Store;

/// The file's name in the app's folder.
pub const FILE: &str = "midi.log";
/// The one set aside.
pub const OLD: &str = "midi.log.1";
/// A file's size before it is set aside.
pub const MAX: u64 = 1 << 20;
/// The last line of a file set aside.
pub const ROTATED: &str = "@@ROTATED";
/// How often the thread looks.
const EVERY: Duration = Duration::from_millis(250);

/// Saves the log the Monitor asked for, if it asked, and gives the
/// Monitor the answer. Returns the answer.
pub fn save_asked(model: &Shared, store: &Store) -> Option<String> {
    let ask = model
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .asks
        .save_log
        .take()?;
    let said = save(store, ask);
    model
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .live
        .log_saved = Some(said.clone());
    Some(said)
}

/// `ask`'s log to a new file in the store's `files/midi-logs/`; what
/// happened, in words.
fn save(store: &Store, ask: LogToSave) -> String {
    if !store.saves() {
        return "Save log: no data folder to keep it in this run".into();
    }
    let taken = store.list(OWNER);
    // The list stops at MAX_LIST names: past it a name could be taken
    // without showing, so nothing is saved.
    if taken.len() >= mpfree_store::MAX_LIST {
        return format!(
            "Save log: files/{OWNER}/ holds {} logs; move some off the stick first",
            taken.len()
        );
    }
    let Some(name) = file_name(&taken, &ask.stamp) else {
        return "Save log: 99 logs this minute already".into();
    };
    if let Err(e) = store.write(OWNER, &name, ask.text.into_bytes()) {
        return format!("Save log: not saved ({e})");
    }
    store.flush();
    if store.list(OWNER).contains(&name) {
        format!("Saved {} lines to files/{OWNER}/{name}", ask.lines)
    } else {
        "Save log: not saved (the run's log says why)".into()
    }
}

/// The console's file and how far it has got.
pub struct RingLog {
    dir: PathBuf,
    max: u64,
    file: Option<File>,
    size: u64,
    /// The next line's number; `None` while the tools are off (they start
    /// with the lines that come after they are turned on).
    next: Option<u64>,
}

impl RingLog {
    pub fn new(dir: &Path, max: u64) -> RingLog {
        RingLog {
            dir: dir.to_path_buf(),
            max,
            file: None,
            size: 0,
            next: None,
        }
    }

    /// Writes the monitor's lines that came since the last call, while the
    /// developer tools are on. Returns how many.
    pub fn poll(&mut self, model: &Shared) -> Result<usize, String> {
        let lines = {
            let m = model.lock().unwrap_or_else(|e| e.into_inner());
            if !m.live.debug {
                self.next = None;
                return Ok(0);
            }
            let from = self.next.unwrap_or(m.monitor.total());
            let (lines, next) = mpfree_midi_ui::log::console_lines(&m, from);
            self.next = Some(next);
            lines
        };
        if lines.is_empty() {
            return Ok(0);
        }
        let mut text = lines.join("\n");
        text.push('\n');
        self.write(text.as_bytes())?;
        Ok(lines.len())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self.file.is_none() {
            // A file from an earlier start is set aside first.
            if self.dir.join(FILE).exists() {
                let mut old = OpenOptions::new()
                    .append(true)
                    .open(self.dir.join(FILE))
                    .map_err(|e| self.say(e))?;
                writeln!(old, "{ROTATED}").map_err(|e| self.say(e))?;
                fs::rename(self.dir.join(FILE), self.dir.join(OLD)).map_err(|e| self.say(e))?;
            }
            self.open()?;
        }
        let f = self.file.as_mut().expect("opened above");
        f.write_all(bytes)
            .map_err(|e| format!("{}: {e}", self.dir.join(FILE).display()))?;
        self.size += bytes.len() as u64;
        if self.size >= self.max {
            self.rotate()?;
        }
        Ok(())
    }

    fn say(&self, e: std::io::Error) -> String {
        format!("{}: {e}", self.dir.join(FILE).display())
    }

    /// A new, empty file, in the folder only if it is there.
    fn open(&mut self) -> Result<(), String> {
        let f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o644)
            .open(self.dir.join(FILE))
            .map_err(|e| self.say(e))?;
        self.file = Some(f);
        self.size = 0;
        Ok(())
    }

    /// The file is full: its last line says so, it becomes [`OLD`], and a
    /// new one starts.
    fn rotate(&mut self) -> Result<(), String> {
        if let Some(mut f) = self.file.take() {
            writeln!(f, "{ROTATED}").map_err(|e| self.say(e))?;
        }
        fs::rename(self.dir.join(FILE), self.dir.join(OLD)).map_err(|e| self.say(e))?;
        self.open()
    }
}

/// The `mpfree-midilog` thread; it stops when this is dropped.
pub struct Writer {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Writer {
    /// Starts the thread on `model`: Save log through `store`, the
    /// console's file in `dir`. Each save's answer is also a line on the
    /// thread's error output (the run's log). The console's file stops at
    /// its first error (a folder that isn't there, as on a workstation),
    /// said there once; saving goes on.
    pub fn start(dir: &Path, model: Shared, store: Store) -> Result<Writer, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let mut ring = Some(RingLog::new(dir, MAX));
        let handle = std::thread::Builder::new()
            .name("mpfree-midilog".into())
            .spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    // In the run's log (the device checks read it there).
                    if let Some(said) = save_asked(&model, &store) {
                        eprintln!("midi log: {said}");
                    }
                    if let Some(Err(e)) = ring.as_mut().map(|r| r.poll(&model)) {
                        eprintln!("midi log: the console's file is off for this run ({e})");
                        ring = None;
                    }
                    std::thread::sleep(EVERY);
                }
            })
            .map_err(|e| format!("midi log: no thread ({e})"))?;
        Ok(Writer {
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_midi::monitor::{Dir, Feed};
    use mpfree_midi::DIN;
    use std::sync::Mutex;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mpfree-midilog-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn feed(model: &Shared, from: u8, n: u8) {
        let mut f = Feed::new();
        let mut m = model.lock().unwrap();
        for k in from..from + n {
            f.bytes(DIN, Dir::In, f64::from(k) * 0.01, &[0x90, k, 100]);
        }
        f.drain_into(&mut m.monitor, 1.0);
    }

    fn ask(model: &Shared, stamp: &str, text: &str) {
        model.lock().unwrap().asks.save_log = Some(LogToSave {
            stamp: stamp.into(),
            text: text.into(),
            lines: 2,
        });
    }

    /// Nothing while the tools are off; on, only what comes after; each
    /// line once; a full file is set aside with its last line saying so.
    #[test]
    fn the_console_log_is_written_while_the_tools_are_on_and_set_aside_when_full() {
        let d = dir("ring");
        fs::create_dir_all(&d).unwrap();
        let model: Shared = Arc::new(Mutex::new(mpfree_midi_ui::IoModel::default()));
        let mut log = RingLog::new(&d, 600);
        feed(&model, 0, 3);
        assert_eq!(log.poll(&model), Ok(0));
        assert!(!d.join(FILE).exists());
        model.lock().unwrap().live.debug = true;
        // Turned on: the lines before don't come.
        assert_eq!(log.poll(&model), Ok(0));
        feed(&model, 10, 2);
        assert_eq!(log.poll(&model), Ok(2));
        assert_eq!(log.poll(&model), Ok(0));
        let text = fs::read_to_string(d.join(FILE)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].starts_with("3\t") && lines[0].contains("Note on"),
            "{text}"
        );
        // Enough to pass 600 bytes: set aside, and a new file goes on.
        feed(&model, 20, 12);
        assert_eq!(log.poll(&model), Ok(12));
        let old = fs::read_to_string(d.join(OLD)).unwrap();
        assert_eq!(old.lines().last(), Some(ROTATED));
        feed(&model, 40, 1);
        assert_eq!(log.poll(&model), Ok(1));
        let new = fs::read_to_string(d.join(FILE)).unwrap();
        assert_eq!(new.lines().count(), 1);
        assert!(!new.contains(ROTATED));
        // A new start sets the file it finds aside first.
        let mut again = RingLog::new(&d, 600);
        model.lock().unwrap().live.debug = false;
        assert_eq!(again.poll(&model), Ok(0));
        model.lock().unwrap().live.debug = true;
        again.poll(&model).unwrap();
        feed(&model, 50, 1);
        assert_eq!(again.poll(&model), Ok(1));
        let set_aside = fs::read_to_string(d.join(OLD)).unwrap();
        assert_eq!(set_aside, format!("{new}{ROTATED}\n"));
        fs::remove_dir_all(&d).unwrap();
    }

    /// No folder (a workstation): an error, not a panic, and the folder is
    /// not made.
    #[test]
    fn without_its_folder_the_console_log_is_an_error_not_a_panic() {
        let d = dir("none");
        let model: Shared = Arc::new(Mutex::new(mpfree_midi_ui::IoModel::default()));
        model.lock().unwrap().live.debug = true;
        let mut log = RingLog::new(&d, MAX);
        log.poll(&model).unwrap();
        feed(&model, 0, 1);
        assert!(log.poll(&model).is_err());
        assert!(!d.exists());
    }

    /// Save log: a new file each time, never over one, through the store;
    /// the Monitor is told once.
    #[test]
    fn a_saved_log_is_a_new_file_in_the_stores_folder() {
        let d = dir("saves");
        fs::create_dir_all(&d).unwrap();
        let store = Store::at(&d, |_| {});
        let model: Shared = Arc::new(Mutex::new(mpfree_midi_ui::IoModel::default()));
        assert_eq!(save_asked(&model, &store), None);
        ask(&model, "2026-11-16-1958", "one\n");
        let said = save_asked(&model, &store).unwrap();
        assert_eq!(
            said,
            "Saved 2 lines to files/midi-logs/midi-log-2026-11-16-1958.txt"
        );
        ask(&model, "2026-11-16-1958", "two\n");
        save_asked(&model, &store).unwrap();
        let folder = d.join("files").join(OWNER);
        let read = |n: &str| fs::read_to_string(folder.join(n)).unwrap();
        assert_eq!(read("midi-log-2026-11-16-1958.txt"), "one\n");
        assert_eq!(read("midi-log-2026-11-16-1958-2.txt"), "two\n");
        let mut m = model.lock().unwrap();
        assert!(m.asks.save_log.is_none());
        assert_eq!(
            m.live.log_saved.take().as_deref(),
            Some("Saved 2 lines to files/midi-logs/midi-log-2026-11-16-1958-2.txt")
        );
        drop(m);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn without_a_data_folder_nothing_is_saved_and_the_monitor_hears_why() {
        let model: Shared = Arc::new(Mutex::new(mpfree_midi_ui::IoModel::default()));
        ask(&model, "2026-11-16-1958", "one\n");
        let said = save_asked(&model, &Store::none()).unwrap();
        assert!(said.contains("no data folder"), "{said}");
        assert_eq!(model.lock().unwrap().live.log_saved, Some(said));
    }
}
