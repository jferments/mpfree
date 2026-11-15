//! The plugin sandbox (roadmap P2): each plugin runs in its own process,
//! `mpfree-plughost`, so a plugin that crashes, hangs or blocks can't take
//! the audio thread with it. Two formats: CLAP (`.clap` files, through
//! `mpfree-clap`) and MPC plugins (the VST 2.4 binary interface MPC OS
//! loads, as in the Open MPC catalog, through `mpfree-mpchost`).
//!
//! - **[`spawn`]** starts the process for one plugin and returns a
//!   [`SandboxNode`] (an [`mpfree_node::Node`], so it goes in a track host
//!   slot or an insert chain like any other) and a [`Sandbox`] (the
//!   control-thread handle).
//! - **Each block**, the node writes the block's context, events and input
//!   into memory it shares with the process, wakes it, and waits for the
//!   answer until a **deadline** (by default half the block's length). On
//!   time, the plugin's output is used, sample for sample what the plugin
//!   gives in-process. Late, the block is a **miss**: an instrument is silent
//!   and an effect passes its input through, and the node won't post again
//!   until the process has caught up. Waiting is a futex wait with a
//!   timeout: no allocation, no lock.
//! - **A crash** shows as misses until [`Sandbox::poll`] sees the process
//!   gone; from then the node is silent without waiting.
//!   [`Sandbox::restart`] starts a new process on the same shared memory,
//!   prepares it and loads the state the host saved, and the node carries
//!   on.
//! - **Main-thread work** (state, a value's text, loading a preset) goes
//!   over a socket from the control thread. The process answers the
//!   plugin's own main-thread requests itself.
//! - **A plugin's presets** are listed by a process of their own
//!   ([`presets`], `--presets`), since finding them runs the plugin's code
//!   and can take a while.
//!
//! The node's id and parameters are the same as `mpfree-clap`'s in-process
//! node (`clap:` and the plugin id), so a project doesn't care which runs.
//! An MPC plugin's node id is `mpc:` and its id ([`node_id`]).

pub mod child;
pub mod rack;
pub mod shm;
pub mod wire;

/// A plugin's presets, as [`presets`] lists them and
/// [`Sandbox::load_preset`] loads them.
pub use mpfree_clap::{Place, Preset};

use mpfree_clap::{intern_node_info, intern_params};
use mpfree_node::{Ctx, Expr, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed};
use shm::{Mapping, ShmEvent};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wire::{Hello, ToChild, ToHost};

/// How long the node waits for the plugin process each block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Deadline {
    /// This share of the block's own length (0.5: half of it).
    Fraction(f32),
    /// A fixed time (tests, offline renders).
    Fixed(Duration),
}

/// Parameter changes queued between blocks.
pub const MAX_PENDING: usize = 256;
/// How long the host waits for the process to start or answer the control
/// channel.
pub const CONTROL_TIMEOUT: Duration = Duration::from_secs(10);
/// How long [`presets`] waits for a plugin file's presets to be listed
/// (Surge XT reads its whole data folder and every patch's header).
pub const PRESETS_TIMEOUT: Duration = Duration::from_secs(120);

/// Why the sandbox couldn't start or answer.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The process couldn't load or make the plugin.
    Plugin(String),
    /// The process sent something unexpected.
    Protocol(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "plugin process: {e}"),
            Error::Plugin(e) => write!(f, "plugin process: {e}"),
            Error::Protocol(e) => write!(f, "plugin process: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

/// The sandbox's counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SandboxStats {
    /// Blocks the plugin process answered on time.
    pub blocks: u64,
    /// Blocks it answered late (or not at all).
    pub misses: u64,
    /// Blocks skipped because the process was still on a late block.
    pub busy: u64,
    /// Blocks skipped while the process was gone or not prepared.
    pub down: u64,
    /// Times the process was found gone.
    pub crashes: u64,
    /// Times it was started again.
    pub restarts: u64,
    /// Blocks the plugin itself failed (its own error).
    pub plugin_errors: u64,
}

struct Ctl {
    sock: UnixStream,
    child: Child,
}

/// What the node and the control handle share.
struct Shared {
    map: Mapping,
    program: Program,
    plugin: PathBuf,
    id: String,
    ctl: Mutex<Option<Ctl>>,
    /// The process is gone (or being replaced): the node doesn't wait.
    down: AtomicBool,
    /// The process has been prepared for the node's rate and block size.
    ready: AtomicBool,
    values: Box<[AtomicU32]>,
    /// The last prepare, for a restart: rate << 32 | max_block.
    prepared: AtomicU64,
    crashes: AtomicU64,
    restarts: AtomicU64,
    /// The plugin's state as [`Sandbox::refresh_state`] last read it: what
    /// a project save and a restart use, read without asking the process.
    snapshot: Mutex<Option<Vec<u8>>>,
}

impl Shared {
    fn shm(&self) -> *mut shm::Shm {
        self.map.ptr
    }

    /// Asks the process something on the control channel.
    fn ask(&self, m: &ToChild) -> Result<ToHost, Error> {
        let mut g = self.ctl.lock().unwrap_or_else(|e| e.into_inner());
        let ctl = g.as_mut().ok_or(Error::Protocol("no process"))?;
        wire::send_to_child(&mut ctl.sock, m)?;
        Ok(wire::read_to_host(&mut ctl.sock)?)
    }

    fn stop_child(&self) {
        let mut g = self.ctl.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut ctl) = g.take() {
            let _ = wire::send_to_child(&mut ctl.sock, &ToChild::Quit);
            let until = Instant::now() + Duration::from_secs(1);
            loop {
                match ctl.child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) if Instant::now() < until => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Ok(None) => {
                        let _ = ctl.child.kill();
                        let _ = ctl.child.wait();
                        break;
                    }
                }
            }
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.stop_child();
    }
}

/// Starts a process on the shared block and reads its hello.
fn start(program: &Program, plugin: &Path, id: &str, map: &Mapping) -> Result<(Ctl, Hello), Error> {
    let (ours, theirs) = UnixStream::pair()?;
    ours.set_read_timeout(Some(CONTROL_TIMEOUT))?;
    // Copies at 10 and up, so the dup2 calls in the child can't clobber
    // each other.
    // SAFETY: fcntl on fds we own.
    let (shm_fd, ctl_fd) = unsafe {
        (
            libc::fcntl(map.fd, libc::F_DUPFD_CLOEXEC, 10),
            libc::fcntl(theirs.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10),
        )
    };
    if shm_fd < 0 || ctl_fd < 0 {
        return Err(Error::Io(io::Error::last_os_error()));
    }
    let mut cmd = program.command();
    cmd.arg("--plugin")
        .arg(plugin)
        .arg("--id")
        .arg(id)
        .arg("--shm-fd")
        .arg("3")
        .arg("--ctl-fd")
        .arg("4");
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            if libc::dup2(shm_fd, 3) < 0 || libc::dup2(ctl_fd, 4) < 0 {
                return Err(io::Error::last_os_error());
            }
            // Gone with us if we die.
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            // A crashing plugin leaves no core file on the stick.
            let none = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            libc::setrlimit(libc::RLIMIT_CORE, &none);
            Ok(())
        });
    }
    let spawned = cmd.spawn();
    // SAFETY: closing our copies.
    unsafe {
        libc::close(shm_fd);
        libc::close(ctl_fd);
    }
    drop(theirs);
    let child = spawned?;
    let mut ctl = Ctl { sock: ours, child };
    match wire::read_to_host(&mut ctl.sock) {
        Ok(ToHost::Hello(h)) => Ok((ctl, h)),
        Ok(ToHost::Failed(e)) => {
            let _ = ctl.child.wait();
            Err(Error::Plugin(e))
        }
        Ok(_) => {
            let _ = ctl.child.kill();
            let _ = ctl.child.wait();
            Err(Error::Protocol("expected hello"))
        }
        Err(e) => {
            let _ = ctl.child.kill();
            let _ = ctl.child.wait();
            Err(Error::Io(e))
        }
    }
}

/// How to start the plugin process: a program and the arguments that go
/// before the process's own. The `mpfree-plughost` binary takes none; a
/// program that serves as the plugin process behind a subcommand (the app
/// does, `mpfree plughost ...`) takes that word.
///
/// [`Program::with_home`] gives every process it starts a home folder of
/// its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub exe: PathBuf,
    pub lead: Vec<std::ffi::OsString>,
    /// The processes' home folder, if not the one they'd inherit.
    pub home: Option<PathBuf>,
}

impl Program {
    /// The `mpfree-plughost` binary at `exe`.
    pub fn new(exe: &Path) -> Program {
        Program {
            exe: exe.to_path_buf(),
            lead: Vec::new(),
            home: None,
        }
    }

    /// `exe` with `lead` before the process's arguments.
    pub fn with_lead(exe: &Path, lead: &[&str]) -> Program {
        Program {
            exe: exe.to_path_buf(),
            lead: lead.iter().map(Into::into).collect(),
            home: None,
        }
    }

    /// The same program, with each process it starts living in `dir`:
    /// `HOME` and the XDG config, data, cache and state folders are set
    /// inside it, so whatever a plugin keeps for itself (Surge XT makes
    /// `~/.Surge XT`, Dexed writes its cartridges under
    /// `~/.local/share`) goes there and never into the inherited home. The
    /// folder isn't made here: the caller makes it, or the plugin does.
    pub fn with_home(mut self, dir: &Path) -> Program {
        self.home = Some(dir.to_path_buf());
        self
    }

    fn command(&self) -> Command {
        let mut c = Command::new(&self.exe);
        c.args(&self.lead);
        if let Some(h) = &self.home {
            c.env("HOME", h)
                .env("XDG_CONFIG_HOME", h.join(".config"))
                .env("XDG_DATA_HOME", h.join(".local").join("share"))
                .env("XDG_CACHE_HOME", h.join(".cache"))
                .env("XDG_STATE_HOME", h.join(".local").join("state"));
        }
        c
    }
}

/// One plugin in a `.clap` file, as [`list`] read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub kind: Kind,
}

/// Reads one line of `mpfree-plughost --list` ([`child::list_line`]).
pub fn parse_list_line(line: &str) -> Option<Listed> {
    let mut f = line.split('\t');
    if f.next()? != "plugin" {
        return None;
    }
    let (id, name, vendor) = (f.next()?, f.next()?, f.next()?);
    let kind = match f.next()? {
        "instrument" => Kind::Instrument,
        "effect" => Kind::Effect,
        _ => return None,
    };
    if f.next().is_some() || id.is_empty() {
        return None;
    }
    Some(Listed {
        id: id.to_string(),
        name: name.to_string(),
        vendor: vendor.to_string(),
        kind,
    })
}

/// Lists the plugins in `plugin` (a `.clap` file, or an MPC plugin) by starting the plugin
/// process with `--list`, so a plugin file that crashes
/// or hangs while loading can't take MPFree with it. Waits at most
/// [`CONTROL_TIMEOUT`], then kills the process. Control thread only.
pub fn list(program: &Program, plugin: &Path) -> Result<Vec<Listed>, Error> {
    let out = run_tool(program, "--list", plugin, "list", CONTROL_TIMEOUT)?;
    Ok(out.lines().filter_map(parse_list_line).collect())
}

/// Every preset the `.clap` file `plugin`'s preset providers know of (for
/// all its plugins: [`mpfree_clap::Preset::is_for`] picks one's), found by
/// the plugin process with `--presets`, so a provider that crashes or
/// hangs can't take MPFree with it. Waits at most [`PRESETS_TIMEOUT`].
/// Control thread only; it can take seconds.
pub fn presets(program: &Program, plugin: &Path) -> Result<mpfree_clap::Found, Error> {
    let out = run_tool(
        program,
        "--presets",
        plugin,
        "list its presets",
        PRESETS_TIMEOUT,
    )?;
    Ok(parse_presets(&out))
}

/// Reads `--presets` output ([`child::preset_line`] and the lines after).
/// Other lines (a plugin's own chatter on stdout) are skipped.
pub fn parse_presets(out: &str) -> mpfree_clap::Found {
    let mut found = mpfree_clap::Found::default();
    for line in out.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f.as_slice() {
            ["preset", ..] => {
                if let Some(p) = parse_preset_line(line) {
                    found.presets.push(p);
                }
            }
            ["error", e] => found.errors.push(unescape(e)),
            ["more-errors", n] => found.more_errors = n.parse().unwrap_or(0),
            ["cut-short"] => found.cut_short = true,
            _ => {}
        }
    }
    found
}

fn unescape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('t') => o.push('\t'),
            Some('n') => o.push('\n'),
            Some('r') => o.push('\r'),
            Some(other) => o.push(other),
            None => o.push('\\'),
        }
    }
    o
}

/// Reads one `preset` line of `--presets` ([`child::preset_line`]).
pub fn parse_preset_line(line: &str) -> Option<mpfree_clap::Preset> {
    let f: Vec<String> = line.split('\t').map(unescape).collect();
    if f.len() < 8 || f[0] != "preset" || f[1].is_empty() {
        return None;
    }
    let place = if f[2] == "plugin" {
        mpfree_clap::Place::Plugin
    } else {
        let path = f[2].strip_prefix("file:")?;
        if path.is_empty() {
            return None;
        }
        mpfree_clap::Place::File(PathBuf::from(path))
    };
    Some(mpfree_clap::Preset {
        name: f[1].clone(),
        place,
        load_key: (!f[3].is_empty()).then(|| f[3].clone()),
        flags: f[4].parse().ok()?,
        collection: f[5].clone(),
        category: f[6].clone(),
        creator: f[7].clone(),
        plugins: f[8..].iter().filter(|s| !s.is_empty()).cloned().collect(),
    })
}

/// Runs the plugin process as a tool on one plugin file (`--list`,
/// `--presets`) and gives what it printed. Kills it after `timeout`.
fn run_tool(
    program: &Program,
    mode: &str,
    plugin: &Path,
    what: &str,
    timeout: Duration,
) -> Result<String, Error> {
    use std::io::Read;
    use std::process::Stdio;
    let mut cmd = program.command();
    cmd.arg(mode)
        .arg(plugin)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            let none = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            libc::setrlimit(libc::RLIMIT_CORE, &none);
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    // Read both pipes on their own threads so a chatty plugin can't fill
    // one and stall while we wait.
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let until = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(st) => break st,
            None if Instant::now() < until => std::thread::sleep(Duration::from_millis(5)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Plugin(format!(
                    "{} took longer than {} s to {what}",
                    plugin.display(),
                    timeout.as_secs()
                )));
            }
        }
    };
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    if !status.success() {
        use std::os::unix::process::ExitStatusExt;
        let why = match (status.code(), status.signal()) {
            (_, Some(sig)) => format!("crashed while loading (signal {sig})"),
            _ => err.lines().next().unwrap_or("would not load").to_string(),
        };
        return Err(Error::Plugin(format!("{}: {why}", plugin.display())));
    }
    Ok(out)
}

/// Whether `plugin` is an MPC plugin (a plugin folder, the Open MPC
/// catalog's format) rather than a `.clap` file.
pub fn is_mpc(plugin: &Path) -> bool {
    mpfree_mpchost::is_mpc_plugin(plugin)
}

/// The node id of plugin `id` in `plugin`: `mpc:` and the id for an MPC
/// plugin ([`mpfree_mpchost::plugin_id`]), `clap:` and the id for a CLAP
/// one.
pub fn node_id(plugin: &Path, id: &str) -> String {
    if is_mpc(plugin) {
        format!("mpc:{id}")
    } else {
        format!("clap:{id}")
    }
}

/// Starts `exe` (the `mpfree-plughost` binary) for plugin `id` in
/// `plugin`: a `.clap` file, or an MPC plugin folder (whose one
/// plugin's id is its [`mpfree_mpchost::plugin_id`]). Control thread only.
pub fn spawn(exe: &Path, plugin: &Path, id: &str) -> Result<(SandboxNode, Sandbox), Error> {
    spawn_with(&Program::new(exe), plugin, id)
}

/// [`spawn`] with any [`Program`] (the app itself, say, with `plughost`
/// in front of the arguments).
pub fn spawn_with(
    program: &Program,
    plugin: &Path,
    id: &str,
) -> Result<(SandboxNode, Sandbox), Error> {
    let map = Mapping::create()?;
    let (ctl, hello) = start(program, plugin, id, &map)?;
    let node_id = node_id(plugin, &hello.id);
    let info = intern_node_info(&node_id, &hello.name, hello.kind);
    let params = intern_params(&node_id, hello.params.clone());
    let values: Box<[AtomicU32]> = params
        .iter()
        .map(|p| AtomicU32::new(p.default.to_bits()))
        .collect();
    let shared = Arc::new(Shared {
        map,
        program: program.clone(),
        plugin: plugin.to_path_buf(),
        id: id.to_string(),
        ctl: Mutex::new(Some(ctl)),
        down: AtomicBool::new(false),
        ready: AtomicBool::new(false),
        values,
        prepared: AtomicU64::new(0),
        crashes: AtomicU64::new(0),
        restarts: AtomicU64::new(0),
        snapshot: Mutex::new(None),
    });
    let node = SandboxNode {
        shared: Arc::clone(&shared),
        info,
        params,
        kind: hello.kind,
        rate: 0,
        max_block: 0,
        pending: Vec::with_capacity(MAX_PENDING),
        latency: 0,
        tail: 0,
        deadline: Deadline::Fraction(0.5),
        stats: SandboxStats::default(),
    };
    Ok((node, Sandbox { shared }))
}

/// A plugin in its own process, as a node. See the crate docs.
pub struct SandboxNode {
    shared: Arc<Shared>,
    info: &'static NodeInfo,
    params: &'static [ParamInfo],
    kind: Kind,
    rate: u32,
    max_block: usize,
    pending: Vec<(u16, f32)>,
    latency: u32,
    tail: u32,
    deadline: Deadline,
    stats: SandboxStats,
}

impl SandboxNode {
    /// The plugin's state as the control thread last read it
    /// ([`Sandbox::refresh_state`]), for a project file. Never asks the
    /// process and never waits: None if no state was read yet, or the
    /// control thread is storing a new one this moment. It copies the
    /// bytes (an allocation), so not on the audio thread's period.
    pub fn saved_state(&self) -> Option<Vec<u8>> {
        self.shared.snapshot.try_lock().ok()?.clone()
    }

    /// The `.clap` file the plugin came from.
    pub fn plugin_file(&self) -> &Path {
        &self.shared.plugin
    }

    pub fn set_deadline(&mut self, d: Deadline) {
        self.deadline = d;
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The node's counters (crashes and restarts come from the control
    /// handle's side).
    pub fn stats(&self) -> SandboxStats {
        SandboxStats {
            crashes: self.shared.crashes.load(Ordering::Relaxed),
            restarts: self.shared.restarts.load(Ordering::Relaxed),
            ..self.stats
        }
    }

    /// One round trip: `left`/`right` are the piece of the block that
    /// starts `at` frames in (events' offsets are from the whole block's
    /// start). False if the process didn't answer (the rest of the block
    /// is left as it is).
    fn block(
        &mut self,
        ctx: &Ctx,
        events: &[Timed],
        at: usize,
        left: &mut [f32],
        right: &mut [f32],
    ) -> bool {
        let n = left.len();
        if self.shared.down.load(Ordering::Acquire) || !self.shared.ready.load(Ordering::Acquire) {
            self.stats.down += 1;
            return false;
        }
        let p = self.shared.shm();
        // SAFETY: the shared block is valid while `shared` lives; the
        // req/done protocol gives this side the request fields now (the
        // child has finished: done == req) and the answer fields after it
        // posts done == k.
        unsafe {
            let req = shm::load(&(*p).req);
            if shm::load(&(*p).done) != req {
                self.stats.busy += 1;
                return false;
            }
            (*p).frames = n as u32;
            (*p).rate = ctx.rate;
            (*p).frame = ctx.frame;
            (*p).bpm = ctx.bpm;
            (*p).beats = ctx.beats;
            (*p).playing = u32::from(ctx.playing);
            let mut k = 0usize;
            for &(id, v) in &self.pending {
                if k == shm::MAX_EVENTS {
                    break;
                }
                (*p).events[k] = if id == u16::MAX {
                    ShmEvent {
                        offset: 0,
                        kind: shm::EV_RESET,
                        ..ShmEvent::default()
                    }
                } else {
                    ShmEvent {
                        offset: 0,
                        kind: shm::EV_PARAM,
                        a: u32::from(id),
                        b: 0,
                        value: v,
                        _pad: 0,
                    }
                };
                k += 1;
            }
            self.pending.clear();
            for t in events {
                if k == shm::MAX_EVENTS {
                    break;
                }
                let local = Timed {
                    offset: (t.offset as usize).saturating_sub(at) as u32,
                    ev: t.ev,
                };
                (*p).events[k] = flatten(&local, n);
                k += 1;
            }
            (*p).nevents = k as u32;
            if self.kind == Kind::Effect {
                (&mut (*p).in_l)[..n].copy_from_slice(left);
                (&mut (*p).in_r)[..n].copy_from_slice(right);
            }
            let seq = req.wrapping_add(1);
            shm::post(&(*p).req, seq);
            let until = Instant::now() + self.wait_for(n);
            loop {
                let d = shm::load(&(*p).done);
                if d == seq {
                    break;
                }
                let now = Instant::now();
                if now >= until {
                    // An instrument adds nothing; an effect leaves its
                    // input as it is.
                    self.stats.misses += 1;
                    return false;
                }
                shm::futex_wait(&(*p).done, d, Some(until - now));
            }
            let (ol, or) = (&(&(*p).out_l)[..n], &(&(*p).out_r)[..n]);
            if self.kind == Kind::Effect {
                left.copy_from_slice(ol);
                right.copy_from_slice(or);
            } else {
                for (d, s) in left.iter_mut().zip(ol) {
                    *d += *s;
                }
                for (d, s) in right.iter_mut().zip(or) {
                    *d += *s;
                }
            }
            let nv = ((*p).nvalues as usize)
                .min(self.shared.values.len())
                .min(shm::MAX_PARAMS);
            for i in 0..nv {
                self.shared.values[i].store((*p).values[i].to_bits(), Ordering::Relaxed);
            }
            self.stats.plugin_errors = u64::from((*p).errors);
            self.stats.blocks += 1;
        }
        true
    }

    fn wait_for(&self, frames: usize) -> Duration {
        match self.deadline {
            Deadline::Fixed(d) => d,
            Deadline::Fraction(f) => {
                let secs = frames as f64 / f64::from(self.rate.max(1)) * f64::from(f.max(0.0));
                Duration::from_secs_f64(secs)
            }
        }
    }
}

fn flatten(t: &Timed, n: usize) -> ShmEvent {
    let offset = t.offset.min(n.saturating_sub(1) as u32);
    let e = |kind, a: u32, b: u32, value: f32| ShmEvent {
        offset,
        kind,
        a,
        b,
        value,
        _pad: 0,
    };
    match t.ev {
        NodeEvent::NoteOn { note, velocity } => {
            e(shm::EV_NOTE_ON, u32::from(note), u32::from(velocity), 0.0)
        }
        NodeEvent::NoteOff { note } => e(shm::EV_NOTE_OFF, u32::from(note), 0, 0.0),
        NodeEvent::Pressure { note, value } => {
            e(shm::EV_PRESSURE, u32::from(note), u32::from(value), 0.0)
        }
        NodeEvent::Bend { value } => e(shm::EV_BEND, 0, 0, value),
        NodeEvent::Expression { note, kind, value } => {
            let k = match kind {
                Expr::Slide => 0,
                Expr::Tone => 1,
                Expr::Release => 2,
            };
            e(shm::EV_EXPRESSION, u32::from(note), k, value)
        }
        NodeEvent::AllOff => e(shm::EV_ALL_OFF, 0, 0, 0.0),
        NodeEvent::Param { id, value } => e(shm::EV_PARAM, u32::from(id), 0, value),
    }
}

impl Node for SandboxNode {
    fn info(&self) -> &'static NodeInfo {
        self.info
    }

    fn params(&self) -> &'static [ParamInfo] {
        self.params
    }

    fn set_param(&mut self, id: u16, value: f32) {
        let Some(p) = self.params.get(usize::from(id)) else {
            return;
        };
        if p.readonly {
            return;
        }
        let v = p.clamp(value);
        self.shared.values[usize::from(id)].store(v.to_bits(), Ordering::Relaxed);
        if let Some(q) = self.pending.iter_mut().find(|q| q.0 == id) {
            q.1 = v;
        } else if self.pending.len() < self.pending.capacity() {
            self.pending.push((id, v));
        }
    }

    fn param(&self, id: u16) -> f32 {
        self.shared
            .values
            .get(usize::from(id))
            .map_or(f32::NAN, |v| f32::from_bits(v.load(Ordering::Relaxed)))
    }

    fn prepare(&mut self, rate: u32, max_block: usize) {
        let max_block = max_block.clamp(1, mpfree_node::MAX_BLOCK);
        self.rate = rate;
        self.max_block = max_block;
        self.shared.ready.store(false, Ordering::Release);
        self.shared.prepared.store(
            (u64::from(rate) << 32) | max_block as u64,
            Ordering::Relaxed,
        );
        if let Ok(ToHost::Prepared { ok, latency, tail }) = self.shared.ask(&ToChild::Prepare {
            rate,
            max_block: max_block as u32,
        }) {
            self.latency = latency;
            self.tail = tail;
            self.shared.ready.store(ok, Ordering::Release);
        }
    }

    fn reset(&mut self) {
        // The plugin process resets before the next block it gets.
        if self.pending.iter().any(|q| q.0 == u16::MAX) {
            return;
        }
        if self.pending.len() < self.pending.capacity() {
            self.pending.push((u16::MAX, 0.0));
        }
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        if n == 0 || self.max_block == 0 {
            return;
        }
        // Longer than the process was prepared for: in pieces, like the
        // in-process host.
        let beats_per_frame = ctx.bpm / 60.0 / f64::from(ctx.rate.max(1));
        let mut at = 0usize;
        let mut next = 0usize;
        while at < n {
            let len = (n - at).min(self.max_block);
            let first = next;
            while next < events.len() && (events[next].offset as usize) < at + len {
                next += 1;
            }
            let c = Ctx {
                frame: ctx.frame + at as u64,
                beats: ctx.beats + at as f64 * beats_per_frame,
                ..*ctx
            };
            if !self.block(
                &c,
                &events[first..next],
                at,
                &mut left[at..at + len],
                &mut right[at..at + len],
            ) {
                return;
            }
            at += len;
        }
    }

    fn tail_frames(&self) -> u32 {
        self.tail
    }

    fn latency(&self) -> u32 {
        self.latency
    }

    /// The last snapshot ([`SandboxNode::saved_state`]): never asks the
    /// process or waits, so a project save may call it between periods.
    /// The rack refreshes it every few seconds; its parameters are saved
    /// apart, so a knob turned since then is still kept.
    fn save_state(&self) -> Option<Vec<u8>> {
        self.saved_state()
    }

    /// Hands the plugin a project's state (a round trip to the process,
    /// off the audio thread) and keeps it as the snapshot, so a restart
    /// after a crash comes back with it.
    fn load_state(&mut self, state: &[u8]) -> Result<(), String> {
        let ok = matches!(
            self.shared.ask(&ToChild::LoadState(state.to_vec())),
            Ok(ToHost::Loaded(true))
        );
        if !ok {
            return Err("the plugin refused it".to_string());
        }
        *self
            .shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(state.to_vec());
        Ok(())
    }

    /// The `.clap` file it came from (`file=verb.clap`), so a project
    /// whose plugin is missing can say which file to put back.
    fn save_extras(&self) -> Vec<(String, String)> {
        self.shared
            .plugin
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| vec![("file".to_string(), n.to_string())])
            .unwrap_or_default()
    }
}

/// The control-thread side of a sandboxed plugin.
pub struct Sandbox {
    shared: Arc<Shared>,
}

/// What [`Sandbox::poll`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Running,
    /// The process is gone (its exit status, if it exited, or the signal
    /// that killed it as a negative number).
    Gone(i32),
}

impl Sandbox {
    /// Checks the process. The first time it is found gone, the node stops
    /// waiting for it and the crash is counted.
    pub fn poll(&self) -> Health {
        let mut g = self.shared.ctl.lock().unwrap_or_else(|e| e.into_inner());
        let Some(ctl) = g.as_mut() else {
            return Health::Gone(0);
        };
        match ctl.child.try_wait() {
            Ok(None) => Health::Running,
            Ok(Some(status)) => {
                use std::os::unix::process::ExitStatusExt;
                let code = status
                    .code()
                    .unwrap_or_else(|| -status.signal().unwrap_or(0));
                if !self.shared.down.swap(true, Ordering::AcqRel) {
                    self.shared.crashes.fetch_add(1, Ordering::Relaxed);
                }
                Health::Gone(code)
            }
            Err(_) => Health::Gone(0),
        }
    }

    /// Starts a new process for the same plugin on the same shared memory,
    /// prepares it as the node was, and loads `state` (from
    /// [`Sandbox::save_state`] before the crash).
    pub fn restart(&self, state: Option<&[u8]>) -> Result<(), Error> {
        self.shared.down.store(true, Ordering::Release);
        self.shared.stop_child();
        // No process runs now: the next request starts fresh.
        let p = self.shared.shm();
        // SAFETY: nothing else writes `done` while no process runs, and the
        // node doesn't touch the block while `down` is set.
        unsafe {
            let req = shm::load(&(*p).req);
            (*p).done.store(req, Ordering::Release);
        }
        let (ctl, _hello) = start(
            &self.shared.program,
            &self.shared.plugin,
            &self.shared.id,
            &self.shared.map,
        )?;
        *self.shared.ctl.lock().unwrap_or_else(|e| e.into_inner()) = Some(ctl);
        let prepared = self.shared.prepared.load(Ordering::Relaxed);
        if prepared != 0 {
            let (rate, max_block) = ((prepared >> 32) as u32, (prepared & 0xFFFF_FFFF) as u32);
            match self.shared.ask(&ToChild::Prepare { rate, max_block })? {
                ToHost::Prepared { ok: true, .. } => {}
                _ => return Err(Error::Protocol("prepare failed after restart")),
            }
        }
        if let Some(s) = state {
            match self.shared.ask(&ToChild::LoadState(s.to_vec()))? {
                ToHost::Loaded(true) => {}
                _ => return Err(Error::Protocol("state didn't load after restart")),
            }
        }
        self.shared.restarts.fetch_add(1, Ordering::Relaxed);
        self.shared.down.store(false, Ordering::Release);
        Ok(())
    }

    /// The plugin's state, for a project file and for a restart.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        match self.shared.ask(&ToChild::SaveState) {
            Ok(ToHost::State(s)) => s,
            _ => None,
        }
    }

    pub fn load_state(&self, bytes: &[u8]) -> bool {
        matches!(
            self.shared.ask(&ToChild::LoadState(bytes.to_vec())),
            Ok(ToHost::Loaded(true))
        )
    }

    /// Loads one of the plugin's presets (as [`presets`] found it). The
    /// values follow in the next blocks; the snapshot follows at the next
    /// [`Sandbox::refresh_state`] (some plugins load in the background).
    /// The error is the plugin's own message when it gave one.
    pub fn load_preset(&self, p: &mpfree_clap::Preset) -> Result<(), String> {
        let file = match &p.place {
            mpfree_clap::Place::File(f) => Some(f.as_os_str().as_encoded_bytes().to_vec()),
            mpfree_clap::Place::Plugin => None,
        };
        let m = ToChild::LoadPreset {
            file,
            key: p.load_key.clone(),
        };
        match self.shared.ask(&m) {
            Ok(ToHost::PresetLoaded(r)) => r,
            Ok(_) => Err("the plugin process answered something else".into()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// How the plugin shows `value` of parameter `index`.
    pub fn param_text(&self, index: u16, value: f32) -> Option<String> {
        match self.shared.ask(&ToChild::ParamText { index, value }) {
            Ok(ToHost::Text(t)) => t,
            _ => None,
        }
    }

    /// Every parameter's value (0 to 1) and text as an MPC plugin holds
    /// them now, for its skin; None from a CLAP plugin or a process that
    /// didn't answer. A round trip to the process: control thread only.
    pub fn values(&self) -> Option<(Vec<f32>, Vec<String>)> {
        match self.shared.ask(&ToChild::Values) {
            Ok(ToHost::Values(v)) => v,
            _ => None,
        }
    }

    /// Reads the plugin's state from the process and keeps it as the
    /// snapshot that [`SandboxNode::saved_state`] and
    /// [`Sandbox::restart_from_snapshot`] use. A round trip to the process:
    /// control thread only. False if the process didn't answer with a
    /// state (the old snapshot stays).
    pub fn refresh_state(&self) -> bool {
        match self.save_state() {
            Some(s) => {
                *self
                    .shared
                    .snapshot
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(s);
                true
            }
            None => false,
        }
    }

    /// Sets the snapshot (a state loaded from a project, before the
    /// process has been asked).
    pub fn set_snapshot(&self, state: Option<Vec<u8>>) {
        *self
            .shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = state;
    }

    /// [`Sandbox::restart`] with the last snapshot.
    pub fn restart_from_snapshot(&self) -> Result<(), Error> {
        let state = self
            .shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        self.restart(state.as_deref())
    }

    /// The node has been dropped: only this handle is left, and the
    /// process can be ended by dropping it.
    pub fn orphaned(&self) -> bool {
        Arc::strong_count(&self.shared) == 1
    }

    /// The plugin's CLAP id (the node's id is `clap:` and this).
    pub fn plugin_id(&self) -> &str {
        &self.shared.id
    }

    /// The process's id, while it runs.
    pub fn pid(&self) -> Option<u32> {
        self.shared
            .ctl
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|c| c.child.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_home_folder_goes_to_every_process() {
        let env = |p: &Program| -> Vec<(String, Option<String>)> {
            p.command()
                .get_envs()
                .map(|(k, v)| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.map(|v| v.to_string_lossy().into_owned()),
                    )
                })
                .collect()
        };
        let plain = Program::new(Path::new("/bin/true"));
        assert!(env(&plain).is_empty(), "inherited as it is");
        let homed = plain
            .clone()
            .with_home(Path::new("/data/files/plugin-home"));
        let mut got = env(&homed);
        got.sort();
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            got,
            [
                ("HOME".into(), some("/data/files/plugin-home")),
                (
                    "XDG_CACHE_HOME".into(),
                    some("/data/files/plugin-home/.cache")
                ),
                (
                    "XDG_CONFIG_HOME".into(),
                    some("/data/files/plugin-home/.config")
                ),
                (
                    "XDG_DATA_HOME".into(),
                    some("/data/files/plugin-home/.local/share")
                ),
                (
                    "XDG_STATE_HOME".into(),
                    some("/data/files/plugin-home/.local/state")
                ),
            ]
        );
        assert_ne!(plain, homed);
    }
}
