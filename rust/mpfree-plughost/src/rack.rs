//! The plugin rack: the plugins found in a folder and the sandboxed
//! processes running, watched and restarted.
//!
//! - **Finding them.** [`Rack::scan`] lists each `.clap` file in a folder
//!   by starting the plugin process with `--list` ([`crate::list`]), so even
//!   a file that crashes while loading can't take the host with it.
//! - **Making one.** [`Rack::make`] starts a plugin's process
//!   ([`crate::spawn_with`]) and keeps its control handle; the node goes
//!   wherever a node goes (a mixer insert, a track host slot).
//! - **Watching them.** Call [`Rack::watch`] about every [`WATCH_EVERY`]
//!   from a control thread. A process found gone is restarted from the
//!   plugin's last state snapshot (taken every [`SNAPSHOT_EVERY`]), at
//!   most [`MAX_RESTARTS`] times a [`RESTART_WINDOW`]; after that it stays
//!   down (an instrument is silent, an effect passes its input through)
//!   until it is made again. A handle whose node was dropped is let go,
//!   which ends its process.
//! - **Never hidden** (MPFree's fallback rule): each start, crash, restart
//!   and give-up is a line ([`Rack::take_lines`]), and the totals are in
//!   [`Rack::summary`].

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mpfree_node::{Kind, Node};

use crate::{Deadline, Health, Program, Sandbox};

/// Rack listed at most; more files are left out and said.
pub const MAX_PLUGINS: usize = 128;
/// How often the host should call [`Rack::watch`].
pub const WATCH_EVERY: Duration = Duration::from_millis(250);
/// How often each plugin's state is read for a restart and a save.
pub const SNAPSHOT_EVERY: Duration = Duration::from_secs(5);
/// Restarts allowed per plugin in [`RESTART_WINDOW`] before it stays down.
pub const MAX_RESTARTS: usize = 3;
pub const RESTART_WINDOW: Duration = Duration::from_secs(60);
/// Lines kept for the screen and the report between takes.
const MAX_LINES: usize = 64;

/// One plugin a file holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// `clap:` and the plugin's id: the node's id, what the chain editor
    /// asks the builder for and what a project names.
    pub node_id: &'static str,
    /// The plugin's own (CLAP) id.
    pub plugin_id: String,
    pub name: &'static str,
    /// "Plugin · Vendor", the picker's line about it.
    pub about: &'static str,
    pub kind: Kind,
    pub file: PathBuf,
}

/// What [`Rack::summary`] counts, for Dev > Performance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Rack found in the folder.
    pub found: usize,
    /// Plugin processes running now, and ones down (crashed and given up).
    pub running: usize,
    pub down: usize,
    pub crashes: u64,
    pub restarts: u64,
    /// Restarts that failed, and plugins that wouldn't start when added.
    pub failed: u64,
}

impl Summary {
    /// The Dev page's line, or None with no plugins found.
    pub fn line(&self) -> Option<String> {
        (self.found > 0).then(|| {
            format!(
                "{} found · {} running · {} down · {} crashes · {} restarts · {} failed",
                self.found, self.running, self.down, self.crashes, self.restarts, self.failed
            )
        })
    }
}

struct Racked {
    name: &'static str,
    sandbox: Sandbox,
    snapshot_at: Instant,
    restarts: VecDeque<Instant>,
    /// Gave up after too many restarts: left down, not polled again.
    given_up: bool,
}

/// The plugins found and the processes running. Share it (an `Arc`)
/// between the threads: [`Rack::make`], [`Rack::watch`] and
/// [`Rack::refresh_states`] wait on processes and belong on a control
/// thread; [`Rack::summary`], [`Rack::take_lines`] and the lists never wait.
pub struct Rack {
    program: Option<Program>,
    deadline: Deadline,
    entries: Vec<Entry>,
    rack: Mutex<Vec<Racked>>,
    lines: Mutex<VecDeque<String>>,
    /// Lines wait in `lines`: [`Rack::take_lines`] touches no lock without.
    said: AtomicBool,
    crashes: AtomicU64,
    restarts: AtomicU64,
    failed: AtomicU64,
}

impl Default for Rack {
    fn default() -> Self {
        Rack::none()
    }
}

impl Rack {
    /// No plugins.
    pub fn none() -> Rack {
        Rack {
            program: None,
            deadline: Deadline::Fraction(0.5),
            entries: Vec::new(),
            rack: Mutex::new(Vec::new()),
            lines: Mutex::new(VecDeque::new()),
            said: AtomicBool::new(false),
            crashes: AtomicU64::new(0),
            restarts: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        }
    }

    /// Lists the `.clap` files in `dir` with `program` (the plugin
    /// process); `shown` is how the lines name the folder. It starts a
    /// process per file and waits for each: before the sound starts.
    /// Returns the plugins and lines for the report (what was found, and
    /// why a file was left out).
    pub fn scan(program: Program, dir: &Path, shown: &str) -> (Rack, Vec<String>) {
        let mut notes = Vec::new();
        let mut p = Rack::none();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return (p, notes);
        };
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|f| f.extension().is_some_and(|x| x == "clap"))
            .collect();
        files.sort();
        if files.is_empty() {
            return (p, notes);
        }
        for file in &files {
            let short = file
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            match crate::list(&program, file) {
                Err(e) => notes.push(format!("plugins: {short} left out: {e}")),
                Ok(list) if list.is_empty() => {
                    notes.push(format!("plugins: {short} left out: it holds no plugins"))
                }
                Ok(list) => {
                    for l in list {
                        if p.entries.len() == MAX_PLUGINS {
                            notes
                                .push(format!("plugins: only the first {MAX_PLUGINS} are offered"));
                            break;
                        }
                        let node_id = format!("clap:{}", l.id);
                        if p.entries.iter().any(|e| e.node_id == node_id) {
                            notes.push(format!(
                                "plugins: {} in {short} left out: another file has the same id",
                                l.id
                            ));
                            continue;
                        }
                        let about = if l.vendor.is_empty() {
                            "Plugin".to_string()
                        } else {
                            format!("Plugin · {}", l.vendor)
                        };
                        let name = if l.name.is_empty() {
                            l.id.clone()
                        } else {
                            l.name.clone()
                        };
                        // Kept for the program's life, like the nodes'
                        // own descriptions (mpfree_clap::intern_info).
                        p.entries.push(Entry {
                            node_id: Box::leak(node_id.into_boxed_str()),
                            plugin_id: l.id,
                            name: Box::leak(name.into_boxed_str()),
                            about: Box::leak(about.into_boxed_str()),
                            kind: l.kind,
                            file: file.clone(),
                        });
                    }
                }
            }
        }
        let (fx, inst) = (p.effects().count(), p.entries.len() - p.effects().count());
        notes.push(format!(
            "plugins: {fx} effect(s) and {inst} instrument(s) in {shown}; each runs in its own process"
        ));
        p.program = Some(program);
        (p, notes)
    }

    /// The deadline each node made from now on waits for its process
    /// (half the block by default; a fixed time for tests and offline
    /// renders).
    pub fn set_deadline(&mut self, d: Deadline) {
        self.deadline = d;
    }

    /// Every plugin found.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The effects, in the order found.
    pub fn effects(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.kind == Kind::Effect)
    }

    pub fn find(&self, node_id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.node_id == node_id)
    }

    /// True for a plugin's node id (`clap:`...).
    pub fn is_plugin(id: &str) -> bool {
        id.starts_with("clap:")
    }

    /// Starts plugin `node_id`'s process and gives its node, keeping its
    /// handle in the rack. Control thread only: it waits for the process.
    pub fn make(&self, node_id: &str) -> Result<Box<dyn Node>, String> {
        let (Some(program), Some(e)) = (self.program.as_ref(), self.find(node_id)) else {
            return Err(format!("no plugin {node_id}"));
        };
        match crate::spawn_with(program, &e.file, &e.plugin_id) {
            Ok((mut node, sandbox)) => {
                node.set_deadline(self.deadline);
                // The first snapshot: a restart before the first timed one
                // still gets the plugin's starting state.
                sandbox.refresh_state();
                self.rack
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(Racked {
                        name: e.name,
                        sandbox,
                        snapshot_at: Instant::now(),
                        restarts: VecDeque::new(),
                        given_up: false,
                    });
                self.say(format!("plugins: {} started in its own process", e.name));
                Ok(Box::new(node))
            }
            Err(err) => {
                self.failed.fetch_add(1, Ordering::Relaxed);
                let line = format!("plugins: {} would not start: {err}", e.name);
                self.say(line.clone());
                Err(line)
            }
        }
    }

    /// Looks at every plugin process (a control thread, every
    /// [`WATCH_EVERY`]): lets go of the ones whose node is gone, restarts
    /// the ones that crashed, and refreshes the state snapshots that are
    /// due. Takes the rack's lock for as long as that takes; only the
    /// control thread and [`Rack::summary`] take it.
    pub fn watch(&self, now: Instant) {
        let mut rack = self.rack.lock().unwrap_or_else(|e| e.into_inner());
        rack.retain(|r| !r.sandbox.orphaned());
        for r in rack.iter_mut().filter(|r| !r.given_up) {
            match r.sandbox.poll() {
                Health::Running => {
                    if now.duration_since(r.snapshot_at) >= SNAPSHOT_EVERY {
                        r.sandbox.refresh_state();
                        r.snapshot_at = now;
                    }
                }
                Health::Gone(code) => {
                    self.crashes.fetch_add(1, Ordering::Relaxed);
                    let how = if code < 0 {
                        format!("signal {}", -code)
                    } else {
                        format!("exit {code}")
                    };
                    while r
                        .restarts
                        .front()
                        .is_some_and(|t| now.duration_since(*t) > RESTART_WINDOW)
                    {
                        r.restarts.pop_front();
                    }
                    if r.restarts.len() >= MAX_RESTARTS {
                        r.given_up = true;
                        self.say(format!(
                            "plugins: {} crashed ({how}) {} times in {} s: left off (its effect passes the sound through); remove and add it again to retry",
                            r.name,
                            MAX_RESTARTS + 1,
                            RESTART_WINDOW.as_secs()
                        ));
                        continue;
                    }
                    r.restarts.push_back(now);
                    match r.sandbox.restart_from_snapshot() {
                        Ok(()) => {
                            self.restarts.fetch_add(1, Ordering::Relaxed);
                            r.snapshot_at = now;
                            self.say(format!(
                                "plugins: {} crashed ({how}); restarted from its last state",
                                r.name
                            ));
                        }
                        Err(e) => {
                            self.failed.fetch_add(1, Ordering::Relaxed);
                            self.say(format!(
                                "plugins: {} crashed ({how}) and did not restart: {e}",
                                r.name
                            ));
                        }
                    }
                }
            }
        }
    }

    /// Reads every running plugin's state now (before a project save).
    /// Waits for each process: not on the audio thread.
    pub fn refresh_states(&self) {
        let now = Instant::now();
        let mut rack = self.rack.lock().unwrap_or_else(|e| e.into_inner());
        for r in rack.iter_mut().filter(|r| !r.given_up) {
            r.sandbox.refresh_state();
            r.snapshot_at = now;
        }
    }

    /// The counts, for the host's Dev page. Never waits: if the control
    /// thread holds the rack: None.
    pub fn summary(&self) -> Option<Summary> {
        let rack = self.rack.try_lock().ok()?;
        let down = rack.iter().filter(|r| r.given_up).count();
        Some(Summary {
            found: self.entries.len(),
            running: rack.len() - down,
            down,
            crashes: self.crashes.load(Ordering::Relaxed),
            restarts: self.restarts.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
        })
    }

    fn say(&self, line: String) {
        let mut l = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        if l.len() == MAX_LINES {
            l.pop_front();
        }
        l.push_back(line);
        self.said.store(true, Ordering::Release);
    }

    /// The lines said since the last take, for the report and the screen.
    /// Never waits (None when the control thread is adding one), and with
    /// nothing said it only reads a flag: no lock, no allocation.
    pub fn take_lines(&self) -> Option<Vec<String>> {
        if !self.said.load(Ordering::Acquire) {
            return None;
        }
        let mut l = self.lines.try_lock().ok()?;
        self.said.store(false, Ordering::Release);
        if l.is_empty() {
            return None;
        }
        Some(l.drain(..).collect())
    }
}
