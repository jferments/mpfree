//! Where MPFree keeps settings and user files between starts.
//!
//! Everything lives under one folder that the caller names. On the MPC it is
//! the test stick's MPFREE-DATA partition, which the test boot's
//! `start-app.sh` mounts at `/run/mpfree-data` and passes with `--data`;
//! nothing else is written (docs/project/storage-spec.md). Inside it:
//!
//! ```text
//! FORMAT                  "mpfree-data 1"
//! settings/<owner>.conf   key = value lines, one file per owner (app, touch, arp ...)
//! files/<owner>/<name>    whole files (arp/patterns.txt, touch/calibration.txt ...)
//! ```
//!
//! - Owner and file names are `[a-z0-9._-]` only ([`valid_name`]): no paths,
//!   no `..`. A file is at most [`MAX_FILE`] bytes.
//! - The folder must hold FORMAT, or be new (empty, or only `lost+found`),
//!   or the store saves nothing: a wrong `--data` never scatters files.
//! - Reads are plain and synchronous: do them at start.
//! - Saves go to one writer thread ("mpfree-store"), never the audio
//!   thread. Each file is written whole to `NAME.part`, synced, renamed over
//!   the old one, and its folder synced, so a save that is cut off leaves
//!   the old file. Saves of one file queued together are written once (the
//!   latest). The writer reports every save, or why it failed, as one line.
//! - [`Store::none`] reads nothing and saves nothing, for tests and previews.

#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

/// The FORMAT file's first line.
pub const FORMAT: &str = "mpfree-data 1";
/// The largest file the store reads or saves.
pub const MAX_FILE: usize = 1 << 20;
/// The longest owner, file name or settings key.
pub const MAX_NAME: usize = 64;

/// An owner or file name the store accepts: 1 to [`MAX_NAME`] of
/// `[a-z0-9._-]`, not starting with `.` and not ending in `.part`.
pub fn valid_name(s: &str) -> bool {
    valid_key(s) && !s.starts_with('.') && !s.ends_with(".part")
}

/// A settings key: 1 to [`MAX_NAME`] of `[a-z0-9._-]`.
pub fn valid_key(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_NAME && s.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

/// Ordered `key = value` settings: one owner's `.conf` file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    entries: Vec<(String, String)>,
}

impl Settings {
    /// Reads `key = value` lines. Blank lines, `#` comments and lines with a
    /// bad key are skipped; a key given twice keeps the last value.
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                s.set_str(k.trim(), v.trim());
            }
        }
        s
    }

    /// The file's text: one `key = value` line each, in order.
    pub fn to_text(&self) -> String {
        self.entries.iter().map(|(k, v)| format!("{k} = {v}\n")).collect()
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// true / on / yes / 1, or false / off / no / 0.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get_str(key)? {
            "true" | "on" | "yes" | "1" => Some(true),
            "false" | "off" | "no" | "0" => Some(false),
            _ => None,
        }
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get_str(key)?.parse().ok()
    }

    /// A finite number.
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get_str(key)?.parse::<f64>().ok().filter(|v| v.is_finite())
    }

    /// Sets `key` (in place, or added at the end). False, and nothing
    /// changes, for a bad key or a value with a line break, a control
    /// character, or space at either end.
    pub fn set_str(&mut self, key: &str, value: &str) -> bool {
        if !valid_key(key) || value.chars().any(char::is_control) || value.trim() != value {
            return false;
        }
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some(e) => e.1 = value.to_string(),
            None => self.entries.push((key.to_string(), value.to_string())),
        }
        true
    }

    pub fn set_bool(&mut self, key: &str, value: bool) -> bool {
        self.set_str(key, if value { "true" } else { "false" })
    }

    pub fn set_i64(&mut self, key: &str, value: i64) -> bool {
        self.set_str(key, &value.to_string())
    }

    /// False for a value that is not finite.
    pub fn set_f64(&mut self, key: &str, value: f64) -> bool {
        value.is_finite() && self.set_str(key, &value.to_string())
    }

    pub fn remove(&mut self, key: &str) {
        self.entries.retain(|(k, _)| k != key);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Settings and user files under one folder, or nowhere ([`Store::none`]).
/// Clones share the folder and the writer thread.
#[derive(Clone, Default)]
pub struct Store {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    dir: PathBuf,
    /// The writer thread, or None when saving is off (the reason was reported).
    writer: Option<Sender<Job>>,
}

/// One whole file to save: its folders under the store's, its name, and
/// the name the report gives it.
struct Save {
    folders: Vec<String>,
    file: String,
    label: String,
    bytes: Vec<u8>,
}

enum Job {
    Save(Save),
    /// Answered once every job before it is done.
    Flush(Sender<()>),
}

impl Store {
    /// A store that reads nothing and saves nothing.
    pub fn none() -> Store {
        Store { inner: None }
    }

    /// The store in `dir`. `report` gets one line for every save ("saved:
    /// settings/app.conf (42 bytes)", "not saved: ...") and, now, why saving
    /// is off if it is. It runs on the writer thread.
    pub fn at(dir: &Path, report: impl Fn(String) + Send + 'static) -> Store {
        let dir = dir.to_path_buf();
        let (writer, first) = match check_folder(&dir) {
            Err(why) => {
                report(format!("storage: saving is off: {why}"));
                (None, None)
            }
            Ok(new) => {
                let (tx, rx) = mpsc::channel();
                let d = dir.clone();
                match thread::Builder::new().name("mpfree-store".into()).spawn(move || writer(&d, rx, report)) {
                    Ok(_) => (Some(tx), new.then(|| Save { folders: Vec::new(), file: "FORMAT".into(),
                                                              label: "FORMAT".into(),
                                                              bytes: format!("{FORMAT}\n").into_bytes() })),
                    // `report` went with the closure; nothing can be said on this thread.
                    Err(_) => (None, None),
                }
            }
        };
        if let (Some(tx), Some(save)) = (&writer, first) {
            let _ = tx.send(Job::Save(save));
        }
        Store { inner: Some(Arc::new(Inner { dir, writer })) }
    }

    /// True when saves can happen (a folder, FORMAT right, the writer running).
    pub fn saves(&self) -> bool {
        self.inner.as_ref().is_some_and(|i| i.writer.is_some())
    }

    /// `owner`'s settings, or empty ones if there are none (or no store).
    pub fn settings(&self, owner: &str) -> Settings {
        self.read_at(&["settings"], &format!("{owner}.conf"), owner)
            .and_then(|b| String::from_utf8(b).ok())
            .map(|t| Settings::parse(&t))
            .unwrap_or_default()
    }

    /// Queues `owner`'s settings to be saved.
    pub fn save_settings(&self, owner: &str, settings: &Settings) -> Result<(), String> {
        if !valid_name(owner) {
            return Err(format!("bad owner name {owner:?}"));
        }
        self.queue(vec!["settings".into()], format!("{owner}.conf"), settings.to_text().into_bytes())
    }

    /// `owner`'s file `name`, if it is there and at most [`MAX_FILE`] bytes.
    pub fn read(&self, owner: &str, name: &str) -> Option<Vec<u8>> {
        if !valid_name(name) {
            return None;
        }
        self.read_at(&["files", owner], name, owner)
    }

    /// Queues `owner`'s file `name` to be saved with `bytes`.
    pub fn write(&self, owner: &str, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        if !valid_name(owner) || !valid_name(name) {
            return Err(format!("bad name {owner:?}/{name:?}"));
        }
        self.queue(vec!["files".into(), owner.into()], name.into(), bytes)
    }

    /// Waits until every save queued before it is done (or failed).
    pub fn flush(&self) {
        if let Some(tx) = self.inner.as_ref().and_then(|i| i.writer.as_ref()) {
            let (done, wait) = mpsc::channel();
            if tx.send(Job::Flush(done)).is_ok() {
                let _ = wait.recv();
            }
        }
    }

    fn read_at(&self, folders: &[&str], file: &str, owner: &str) -> Option<Vec<u8>> {
        let inner = self.inner.as_ref()?;
        if !valid_name(owner) {
            return None;
        }
        let mut path = inner.dir.clone();
        for f in folders {
            path.push(f);
            if !fs::symlink_metadata(&path).ok()?.is_dir() {
                return None;
            }
        }
        path.push(file);
        if !fs::symlink_metadata(&path).ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        File::open(&path).ok()?.take(MAX_FILE as u64 + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() <= MAX_FILE).then_some(bytes)
    }

    fn queue(&self, folders: Vec<String>, file: String, bytes: Vec<u8>) -> Result<(), String> {
        let label = folders.iter().map(String::as_str).chain([file.as_str()]).collect::<Vec<_>>().join("/");
        let Some(inner) = &self.inner else { return Ok(()) };
        if bytes.len() > MAX_FILE {
            return Err(format!("{label}: {} bytes is over the {MAX_FILE}-byte limit", bytes.len()));
        }
        let tx = inner.writer.as_ref().ok_or_else(|| format!("{label}: saving is off"))?;
        tx.send(Job::Save(Save { folders, file, label: label.clone(), bytes }))
            .map_err(|_| format!("{label}: the writer has stopped"))
    }
}

/// Ok(true) for a new folder (FORMAT is written first), Ok(false) for one
/// with the right FORMAT, Err(why) for anything else.
fn check_folder(dir: &Path) -> Result<bool, String> {
    let shown = dir.display();
    let meta = fs::symlink_metadata(dir).map_err(|e| format!("{shown}: {e}"))?;
    if !meta.is_dir() {
        return Err(format!("{shown} is not a folder"));
    }
    match fs::symlink_metadata(dir.join("FORMAT")) {
        Ok(m) if m.is_file() => {
            let mut text = String::new();
            File::open(dir.join("FORMAT"))
                .and_then(|f| f.take(256).read_to_string(&mut text))
                .map_err(|e| format!("{shown}/FORMAT: {e}"))?;
            match text.lines().next().map(str::trim) {
                Some(FORMAT) => Ok(false),
                other => Err(format!("{shown}/FORMAT says {:?}, not {FORMAT:?}", other.unwrap_or(""))),
            }
        }
        Ok(_) => Err(format!("{shown}/FORMAT is not a plain file")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let others: Vec<String> = fs::read_dir(dir)
                .map_err(|e| format!("{shown}: {e}"))?
                .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                .filter(|n| n != "lost+found")
                .collect();
            if others.is_empty() {
                Ok(true)
            } else {
                Err(format!("{shown} has no FORMAT file and is not empty ({} entries), so it is not MPFree's", others.len()))
            }
        }
        Err(e) => Err(format!("{shown}/FORMAT: {e}")),
    }
}

fn writer(dir: &Path, rx: Receiver<Job>, report: impl Fn(String)) {
    let mut queue = VecDeque::new();
    while let Ok(job) = rx.recv() {
        queue.push_back(job);
        queue.extend(rx.try_iter());
        while let Some(job) = queue.pop_front() {
            match job {
                Job::Save(s) => {
                    // A later save of the same file, before any flush, replaces this one.
                    let later = queue.iter()
                        .take_while(|j| matches!(j, Job::Save(_)))
                        .any(|j| matches!(j, Job::Save(t) if t.label == s.label));
                    if later {
                        continue;
                    }
                    report(match save(dir, &s) {
                        Ok(()) => format!("saved: {} ({} bytes)", s.label, s.bytes.len()),
                        Err(e) => format!("not saved: {}: {e}", s.label),
                    });
                }
                Job::Flush(done) => {
                    let _ = done.send(());
                }
            }
        }
    }
}

/// Writes `s` whole beside its file as NAME.part, syncs it, renames it over
/// the file and syncs the folder. On failure the .part is removed and the
/// old file is as it was. Never follows a link: folders and the file must be
/// real ones, and the .part is created new.
fn save(dir: &Path, s: &Save) -> io::Result<()> {
    let mut folder = dir.to_path_buf();
    for name in &s.folders {
        let parent = folder.clone();
        folder.push(name);
        match fs::symlink_metadata(&folder) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(io::Error::other(format!("{name} is not a folder"))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                DirBuilder::new().mode(0o755).create(&folder)?;
                File::open(&parent)?.sync_all()?;
            }
            Err(e) => return Err(e),
        }
    }
    let path = folder.join(&s.file);
    match fs::symlink_metadata(&path) {
        Ok(m) if !m.is_file() => return Err(io::Error::other("there is something other than a file in its place")),
        _ => {}
    }
    let part = folder.join(format!("{}.part", s.file));
    // A .part left by a cut-off save.
    match fs::remove_file(&part) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let done = (|| {
        let mut f = OpenOptions::new().write(true).create_new(true).mode(0o644).open(&part)?;
        f.write_all(&s.bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&part, &path)?;
        File::open(&folder)?.sync_all()
    })();
    if done.is_err() {
        let _ = fs::remove_file(&part);
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    fn temp() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("mpfree-store-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn store(dir: &Path) -> (Store, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let l = lines.clone();
        (Store::at(dir, move |s| l.lock().unwrap().push(s)), lines)
    }

    #[test]
    fn names_are_plain() {
        for ok in ["app", "arp", "patterns.txt", "touch-cal_2", "a"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", ".", "..", ".hidden", "a/b", "../x", "App", "x y", "x.part", "é", &"a".repeat(65)] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn settings_read_and_write_in_order() {
        let mut s = Settings::parse("# prefs\n\ncurve = soft\n buffer=256 \nbad key = 1\nshow-dev = on\ncurve = hard\nnoequals\n");
        assert_eq!(s.iter().collect::<Vec<_>>(), vec![("curve", "hard"), ("buffer", "256"), ("show-dev", "on")]);
        assert_eq!(s.get_i64("buffer"), Some(256));
        assert_eq!(s.get_bool("show-dev"), Some(true));
        assert_eq!(s.get_bool("curve"), None);
        assert!(s.set_f64("level", 0.5) && s.set_bool("latch", false));
        assert!(!s.set_f64("level", f64::NAN));
        assert!(!s.set_str("x", "two\nlines") && !s.set_str("Bad", "1") && !s.set_str("x", " padded"));
        s.remove("buffer");
        assert_eq!(s.to_text(), "curve = hard\nshow-dev = on\nlevel = 0.5\nlatch = false\n");
        assert_eq!(Settings::parse(&s.to_text()), s);
        assert_eq!(Settings::parse("level = inf\n").get_f64("level"), None);
    }

    #[test]
    fn a_new_folder_gets_format_settings_and_files() {
        let dir = temp();
        fs::create_dir(dir.join("lost+found")).unwrap();
        let (st, lines) = store(&dir);
        assert!(st.saves());
        let mut s = Settings::default();
        s.set_str("curve", "soft");
        st.save_settings("app", &s).unwrap();
        st.write("arp", "patterns.txt", b"Skip = 1 . 3\n".to_vec()).unwrap();
        st.flush();
        assert_eq!(fs::read_to_string(dir.join("FORMAT")).unwrap(), "mpfree-data 1\n");
        assert_eq!(*lines.lock().unwrap(), vec!["saved: FORMAT (14 bytes)", "saved: settings/app.conf (13 bytes)",
                                                 "saved: files/arp/patterns.txt (13 bytes)"]);
        let (again, _) = store(&dir);
        assert_eq!(again.settings("app").get_str("curve"), Some("soft"));
        assert_eq!(again.read("arp", "patterns.txt").as_deref(), Some(&b"Skip = 1 . 3\n"[..]));
        assert_eq!(again.read("arp", "missing.txt"), None);
        assert!(again.settings("touch").is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saves_of_one_file_queued_together_write_the_latest() {
        let dir = temp();
        let (st, lines) = store(&dir);
        st.flush();
        for i in 0..50 {
            st.write("arp", "p.txt", format!("{i}").into_bytes()).unwrap();
        }
        st.flush();
        assert_eq!(st.read("arp", "p.txt").as_deref(), Some(&b"49"[..]));
        let n = lines.lock().unwrap().iter().filter(|l| l.contains("p.txt")).count();
        assert!((1..=50).contains(&n));
        assert!(!dir.join("files/arp/p.txt.part").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_that_is_not_ours_is_left_alone() {
        let dir = temp();
        fs::write(dir.join("notes.txt"), "mine").unwrap();
        let (st, lines) = store(&dir);
        assert!(!st.saves());
        assert!(st.write("arp", "p.txt", b"x".to_vec()).is_err());
        st.flush();
        assert!(lines.lock().unwrap()[0].starts_with("storage: saving is off:"));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

        let dir2 = temp();
        fs::write(dir2.join("FORMAT"), "mpfree-data 2\n").unwrap();
        let (st2, _) = store(&dir2);
        assert!(!st2.saves());
        assert!(st2.save_settings("app", &Settings::default()).is_err());

        let (st3, lines3) = store(&dir.join("missing"));
        assert!(!st3.saves() && st3.settings("app").is_empty());
        assert!(lines3.lock().unwrap()[0].contains("missing"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn bad_names_big_files_and_links_are_refused() {
        let dir = temp();
        let outside = temp();
        let (st, lines) = store(&dir);
        assert!(st.write("../x", "a", vec![]).is_err());
        assert!(st.write("arp", "a/b", vec![]).is_err());
        assert!(st.save_settings("App", &Settings::default()).is_err());
        assert!(st.write("arp", "big", vec![0; MAX_FILE + 1]).is_err());
        assert_eq!(st.read("arp", "../FORMAT"), None);
        // A link where an owner's folder should be: nothing is written through it.
        st.flush();
        fs::create_dir(dir.join("files")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("files/touch")).unwrap();
        st.write("touch", "cal.txt", b"x".to_vec()).unwrap();
        st.flush();
        assert!(lines.lock().unwrap().last().unwrap().starts_with("not saved: files/touch/cal.txt"));
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        assert_eq!(st.read("touch", "cal.txt"), None);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_cut_off_save_leaves_the_old_file() {
        let dir = temp();
        let (st, _) = store(&dir);
        st.write("arp", "p.txt", b"old".to_vec()).unwrap();
        st.flush();
        // What a save cut off before its rename leaves behind.
        fs::write(dir.join("files/arp/p.txt.part"), "half").unwrap();
        let (again, _) = store(&dir);
        assert_eq!(again.read("arp", "p.txt").as_deref(), Some(&b"old"[..]));
        again.write("arp", "p.txt", b"new".to_vec()).unwrap();
        again.flush();
        assert_eq!(again.read("arp", "p.txt").as_deref(), Some(&b"new"[..]));
        assert!(!dir.join("files/arp/p.txt.part").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn none_reads_and_saves_nothing() {
        let st = Store::none();
        assert!(!st.saves());
        assert!(st.write("arp", "p.txt", b"x".to_vec()).is_ok());
        assert!(st.settings("app").is_empty());
        assert_eq!(st.read("arp", "p.txt"), None);
        st.flush();
    }
}
