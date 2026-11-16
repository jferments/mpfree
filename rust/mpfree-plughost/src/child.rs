//! The plugin process: loads one plugin and serves the host. A `.clap`
//! file is loaded with `mpfree-clap`; an MPC plugin (a plugin folder, the
//! Open MPC catalog's format) with `mpfree-mpchost`.
//!
//! Two threads. The **main thread** (the plugin's CLAP main thread) answers
//! the control channel: prepare, state, a value's text, a preset, quit;
//! between
//! messages it answers the plugin's own requests (`ClapControl::idle`). The
//! **audio thread** waits on the shared block's `req` word, runs one block
//! of the plugin and posts `done`.
//!
//! The node sits behind a mutex that only prepare and the audio thread take,
//! so a prepare never runs in the middle of a block. State and text go
//! through `ClapControl`, which CLAP allows beside a running block.

use crate::shm::{self, Mapping};
use crate::wire::{self, Hello, ToChild, ToHost};
use mpfree_clap::{ClapControl, ClapNode, Library, ParamSpec, Place, Preset};
use mpfree_mpchost::{MpcControl, MpcNode};
use mpfree_node::{Ctx, Expr, Kind, Node, NodeEvent, Timed, MAX_BLOCK};
use std::ffi::OsString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the main thread looks at the plugin's requests when the host
/// is quiet.
const IDLE: Duration = Duration::from_millis(100);
/// How often the audio thread looks at the quit flag when no block comes.
const AUDIO_IDLE: Duration = Duration::from_millis(200);

/// The loaded plugin's audio side, in either format.
pub enum Plug {
    Clap(Box<ClapNode>),
    Mpc(MpcNode),
}

impl Plug {
    fn node(&self) -> &dyn Node {
        match self {
            Plug::Clap(n) => n.as_ref(),
            Plug::Mpc(n) => n,
        }
    }

    fn node_mut(&mut self) -> &mut dyn Node {
        match self {
            Plug::Clap(n) => n.as_mut(),
            Plug::Mpc(n) => n,
        }
    }

    fn kind(&self) -> Kind {
        self.node().info().kind
    }

    fn is_active(&self) -> bool {
        match self {
            Plug::Clap(n) => n.is_active(),
            Plug::Mpc(n) => n.is_active(),
        }
    }

    /// Blocks the plugin itself failed.
    fn errors(&self) -> u64 {
        match self {
            Plug::Clap(n) => n.stats().errors,
            Plug::Mpc(_) => 0,
        }
    }
}

/// The loaded plugin's control side, in either format.
pub enum Control {
    Clap(ClapControl),
    Mpc(MpcControl),
}

impl Control {
    fn save_state(&self) -> Option<Vec<u8>> {
        match self {
            Control::Clap(c) => c.save_state(),
            Control::Mpc(c) => c.save_state(),
        }
    }

    fn load_state(&self, b: &[u8]) -> bool {
        match self {
            Control::Clap(c) => c.load_state(b),
            Control::Mpc(c) => c.load_state(b),
        }
    }

    fn param_text(&self, index: u16, value: f32) -> Option<String> {
        match self {
            Control::Clap(c) => c.param_text(index, value),
            Control::Mpc(c) => c.param_text(index, value),
        }
    }

    fn values(&self) -> Option<(Vec<f32>, Vec<String>)> {
        match self {
            Control::Clap(_) => None,
            Control::Mpc(c) => Some(c.values()),
        }
    }

    fn load_preset(&self, p: &Preset) -> Result<(), String> {
        match self {
            Control::Clap(c) => c.load_preset(p),
            Control::Mpc(c) => {
                let n = p
                    .load_key
                    .as_deref()
                    .and_then(program_of_key)
                    .ok_or_else(|| "that isn't one of the plugin's programs".to_string())?;
                c.load_program(n)
            }
        }
    }

    fn idle(&self) {
        if let Control::Clap(c) = self {
            c.idle();
        }
    }
}

/// An MPC plugin's built-in program as a preset's load key.
pub fn program_key(n: usize) -> String {
    format!("program:{n}")
}

/// The program number in a load key [`program_key`] made.
pub fn program_of_key(key: &str) -> Option<usize> {
    key.strip_prefix("program:")?.parse().ok()
}

/// Loads plugin `id` from `file`: an MPC plugin (a folder, see
/// [`mpfree_mpchost::is_mpc_plugin`]) or a `.clap` file. An MPC plugin
/// runs with its folder as the current folder, as on the MPC, and
/// `MPC_PLUGIN_FOLDER` set to it (the inMusic SDK's plugins look there).
pub fn open_plugin(file: &std::path::Path, id: &str) -> Result<(Plug, Control), String> {
    if mpfree_mpchost::is_mpc_plugin(file) {
        let lib = mpfree_mpchost::Library::open(file)?;
        let _ = std::env::set_current_dir(&lib.folder.dir);
        std::env::set_var("MPC_PLUGIN_FOLDER", &lib.folder.dir);
        let (node, control) = lib.create()?;
        if control.plugin_id() != id {
            return Err(format!(
                "{} holds {}, not {id}",
                file.display(),
                control.plugin_id()
            ));
        }
        return Ok((Plug::Mpc(node), Control::Mpc(control)));
    }
    let node = Library::open(file)
        .and_then(|lib| lib.create(id))
        .map_err(|e| e.to_string())?;
    let control = node.control();
    Ok((Plug::Clap(Box::new(node)), Control::Clap(control)))
}

/// What the process was started with.
#[derive(Debug, PartialEq)]
pub struct Args {
    pub plugin: PathBuf,
    pub id: String,
    pub shm_fd: i32,
    pub ctl_fd: i32,
}

/// Reads `--plugin FILE --id PLUGIN-ID --shm-fd N --ctl-fd N`, in any
/// order.
pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Option<Args> {
    let (mut plugin, mut id, mut shm_fd, mut ctl_fd) = (None, None, None, None);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let v = it.next()?;
        match a.to_str()? {
            "--plugin" => plugin = Some(PathBuf::from(v)),
            "--id" => id = Some(v.into_string().ok()?),
            "--shm-fd" => shm_fd = v.to_str()?.parse().ok(),
            "--ctl-fd" => ctl_fd = v.to_str()?.parse().ok(),
            _ => return None,
        }
    }
    Some(Args {
        plugin: plugin?,
        id: id?,
        shm_fd: shm_fd?,
        ctl_fd: ctl_fd?,
    })
}

/// The process's whole life; returns its exit status.
pub fn child_main(args: impl IntoIterator<Item = OsString>) -> i32 {
    let args: Vec<OsString> = args.into_iter().collect();
    if args.len() == 2 && args[0] == "--list" {
        return list_main(&PathBuf::from(&args[1]));
    }
    if args.len() == 2 && args[0] == "--presets" {
        return presets_main(&PathBuf::from(&args[1]));
    }
    let Some(a) = parse_args(args) else {
        eprintln!("usage: mpfree-plughost --plugin FILE --id PLUGIN-ID --shm-fd N --ctl-fd N");
        eprintln!("(started by MPFree's plugin sandbox, not by hand)");
        return 64;
    };
    // SAFETY: the host handed us this end of a socket pair at this number.
    let mut sock = unsafe { UnixStream::from_raw_fd(a.ctl_fd) };
    let fail = |sock: &mut UnixStream, msg: String| {
        let _ = wire::send_to_host(sock, &ToHost::Failed(msg));
        2
    };
    let map = match Mapping::attach(a.shm_fd) {
        Ok(m) => Arc::new(m),
        Err(e) => return fail(&mut sock, format!("shared memory: {e}")),
    };
    let (node, control) = match open_plugin(&a.plugin, &a.id) {
        Ok(p) => p,
        Err(e) => return fail(&mut sock, e),
    };
    let hello = Hello {
        id: a.id.clone(),
        name: node.node().info().name.to_string(),
        kind: node.kind(),
        params: node
            .node()
            .params()
            .iter()
            .map(ParamSpec::from_info)
            .collect(),
    };
    if wire::send_to_host(&mut sock, &ToHost::Hello(hello)).is_err() {
        return 2;
    }
    let nparams = node.node().params().len().min(shm::MAX_PARAMS);
    let node = Arc::new(Mutex::new(node));
    let quit = Arc::new(AtomicBool::new(false));
    let audio = {
        let (map, node, quit) = (Arc::clone(&map), Arc::clone(&node), Arc::clone(&quit));
        std::thread::Builder::new()
            .name("plughost-audio".into())
            .spawn(move || audio_loop(&map, &node, nparams, &quit))
    };
    let Ok(audio) = audio else { return 2 };
    let served = serve(&mut sock, &node, &control);
    quit.store(true, Ordering::Release);
    shm::futex_wake(map.req());
    let _ = audio.join();
    // The plugin is destroyed here, on its main thread.
    drop(control);
    drop(node);
    i32::from(served.is_err())
}

/// `--list FILE`: loads the file and prints one line per plugin in it
/// ([`list_line`]), so MPFree can see what a plugin file holds without
/// loading it into its own process. Exit 0 with the lines, 2 if the file
/// won't load (the reason on stderr).
fn list_main(file: &std::path::Path) -> i32 {
    if mpfree_mpchost::is_mpc_plugin(file) {
        return match mpc_tool(file) {
            Ok((node, control)) => {
                use std::io::Write;
                let line = list_line(
                    control.plugin_id(),
                    node.info().name,
                    control.vendor(),
                    node.kind(),
                );
                i32::from(writeln!(io::stdout().lock(), "{line}").is_err()) * 2
            }
            Err(e) => {
                eprintln!("{e}");
                2
            }
        };
    }
    match Library::open(file) {
        Ok(lib) => {
            use std::io::Write;
            let mut out = io::stdout().lock();
            for d in lib.plugins() {
                let kind = if d.features.iter().any(|f| f == "instrument") {
                    Kind::Instrument
                } else {
                    Kind::Effect
                };
                if writeln!(out, "{}", list_line(&d.id, &d.name, &d.vendor, kind)).is_err() {
                    return 2;
                }
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

/// One plugin as `--list` prints it: `plugin`, then its id, name, vendor
/// and `instrument` or `effect`, separated by tabs (tabs and line breaks
/// in the plugin's own text become spaces).
pub fn list_line(id: &str, name: &str, vendor: &str, kind: Kind) -> String {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let kind = match kind {
        Kind::Instrument => "instrument",
        Kind::Effect => "effect",
    };
    format!(
        "plugin\t{}\t{}\t{}\t{kind}",
        clean(id),
        clean(name),
        clean(vendor)
    )
}

/// `--presets FILE`: loads the file and prints one line per preset its
/// preset providers know of ([`preset_line`]), then an `error` line per
/// error kept, `more-errors N` and `cut-short` when they apply. Exit 0
/// with the lines (none for a file with no presets), 2 if the file won't
/// load (the reason on stderr).
fn presets_main(file: &std::path::Path) -> i32 {
    use std::io::Write;
    if mpfree_mpchost::is_mpc_plugin(file) {
        let (_node, control) = match mpc_tool(file) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let mut out = io::stdout().lock();
        for (i, name) in control.programs().into_iter().enumerate() {
            let p = mpc_program(&control, i, name);
            if let Some(l) = preset_line(&p) {
                if writeln!(out, "{l}").is_err() {
                    return 2;
                }
            }
        }
        return 0;
    }
    let lib = match Library::open(file) {
        Ok(lib) => lib,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let found = lib.presets();
    let mut out = io::stdout().lock();
    let mut lines: Vec<String> = Vec::new();
    let mut skipped = 0usize;
    for p in &found.presets {
        match preset_line(p) {
            Some(l) => lines.push(l),
            None => skipped += 1,
        }
    }
    for e in &found.errors {
        lines.push(format!("error\t{}", escape(e)));
    }
    if skipped > 0 {
        lines.push(format!(
            "error\t{skipped} presets have a path that isn't UTF-8 and were left out"
        ));
    }
    if found.more_errors > 0 {
        lines.push(format!("more-errors\t{}", found.more_errors));
    }
    if found.cut_short {
        lines.push("cut-short".into());
    }
    for l in lines {
        if writeln!(out, "{l}").is_err() {
            return 2;
        }
    }
    0
}

/// Escapes a field of a `--presets` line: backslash, tab and line breaks.
pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\t' => o.push_str("\\t"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o
}

/// An MPC plugin made in a tool process (`--list`, `--presets`), with its
/// folder as the current folder as when it plays.
fn mpc_tool(file: &std::path::Path) -> Result<(MpcNode, MpcControl), String> {
    let lib = mpfree_mpchost::Library::open(file)?;
    let _ = std::env::set_current_dir(&lib.folder.dir);
    std::env::set_var("MPC_PLUGIN_FOLDER", &lib.folder.dir);
    lib.create()
}

/// An MPC plugin's built-in program `n` as a preset: it comes with the
/// plugin, and loads by number ([`program_key`]).
pub fn mpc_program(control: &MpcControl, n: usize, name: String) -> Preset {
    Preset {
        name,
        place: Place::Plugin,
        load_key: Some(program_key(n)),
        plugins: vec![control.plugin_id().to_string()],
        collection: String::new(),
        category: String::new(),
        creator: control.vendor().to_string(),
        flags: mpfree_clap::sys::CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT,
    }
}

/// One preset as `--presets` prints it: `preset`, then its name, its place
/// (`plugin`, or `file:` and the path), its load key (empty for none), its
/// flags, collection, category and creator, then one field per plugin id
/// it is for; tab-separated, each field [`escape`]d. None when the path
/// isn't UTF-8.
pub fn preset_line(p: &Preset) -> Option<String> {
    let place = match &p.place {
        Place::Plugin => "plugin".to_string(),
        Place::File(f) => format!("file:{}", f.to_str()?),
    };
    let mut f = vec![
        "preset".to_string(),
        escape(&p.name),
        escape(&place),
        escape(p.load_key.as_deref().unwrap_or("")),
        p.flags.to_string(),
        escape(&p.collection),
        escape(&p.category),
        escape(&p.creator),
    ];
    f.extend(p.plugins.iter().map(|id| escape(id)));
    Some(f.join("\t"))
}

/// The main thread: answers the host until it says quit or goes away.
fn serve(sock: &mut UnixStream, node: &Mutex<Plug>, control: &Control) -> io::Result<()> {
    loop {
        let mut pfd = libc::pollfd {
            fd: sock.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        let r = unsafe { libc::poll(&mut pfd, 1, IDLE.as_millis() as libc::c_int) };
        if r < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if r == 0 {
            control.idle();
            continue;
        }
        let m = match wire::read_to_child(sock) {
            Ok(m) => m,
            // The host is gone (or closed its end): quit quietly.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let answer = match m {
            ToChild::Prepare { rate, max_block } => {
                let mut n = node.lock().unwrap_or_else(|e| e.into_inner());
                n.node_mut().prepare(rate, max_block as usize);
                ToHost::Prepared {
                    ok: n.is_active(),
                    latency: n.node().latency(),
                    tail: n.node().tail_frames(),
                }
            }
            ToChild::SaveState => ToHost::State(control.save_state()),
            ToChild::LoadState(b) => ToHost::Loaded(control.load_state(&b)),
            ToChild::ParamText { index, value } => ToHost::Text(control.param_text(index, value)),
            ToChild::Values => ToHost::Values(control.values()),
            ToChild::LoadPreset { file, key } => {
                use std::os::unix::ffi::OsStringExt;
                let place = match file {
                    Some(b) => Place::File(PathBuf::from(OsString::from_vec(b))),
                    None => Place::Plugin,
                };
                let p = Preset {
                    name: String::new(),
                    place,
                    load_key: key,
                    plugins: Vec::new(),
                    collection: String::new(),
                    category: String::new(),
                    creator: String::new(),
                    flags: 0,
                };
                ToHost::PresetLoaded(control.load_preset(&p))
            }
            ToChild::Quit => return Ok(()),
        };
        wire::send_to_host(sock, &answer)?;
        control.idle();
    }
}

/// The audio thread: one block per request, until quit.
fn audio_loop(map: &Mapping, node: &Mutex<Plug>, nparams: usize, quit: &AtomicBool) {
    let (req, done) = (map.req(), map.done());
    let mut events: Vec<Timed> = Vec::with_capacity(shm::MAX_EVENTS);
    // Start from the last answered request, not the last posted one: the
    // host may post its first request before this thread gets going. (On a
    // restart the host sets `done` to `req` before starting us, writing
    // off a request the old process never answered.)
    let mut seen = shm::load(done);
    loop {
        if quit.load(Ordering::Acquire) {
            return;
        }
        let r = shm::load(req);
        if r == seen {
            shm::futex_wait(req, r, Some(AUDIO_IDLE));
            continue;
        }
        seen = r;
        // SAFETY: the host posted request r and won't touch the block
        // until we post done == r.
        unsafe { run_block(map.ptr, node, &mut events, nparams) };
        shm::post(done, r);
    }
}

fn event_of(e: &shm::ShmEvent) -> Option<NodeEvent> {
    let byte = |v: u32| v.min(127) as u8;
    Some(match e.kind {
        shm::EV_NOTE_ON => NodeEvent::NoteOn {
            note: byte(e.a),
            velocity: byte(e.b),
        },
        shm::EV_NOTE_OFF => NodeEvent::NoteOff { note: byte(e.a) },
        shm::EV_PRESSURE => NodeEvent::Pressure {
            note: byte(e.a),
            value: byte(e.b),
        },
        shm::EV_BEND => NodeEvent::Bend { value: e.value },
        shm::EV_EXPRESSION => NodeEvent::Expression {
            note: byte(e.a),
            kind: match e.b {
                0 => Expr::Slide,
                1 => Expr::Tone,
                2 => Expr::Release,
                _ => return None,
            },
            value: e.value,
        },
        shm::EV_ALL_OFF => NodeEvent::AllOff,
        shm::EV_PARAM => NodeEvent::Param {
            id: u16::try_from(e.a).ok()?,
            value: e.value,
        },
        _ => return None,
    })
}

/// Runs one block from the shared request into the shared answer.
///
/// # Safety
/// `p` is the mapped block, and this side owns the request and answer
/// fields (between the host's `req` and our `done`).
unsafe fn run_block(p: *mut shm::Shm, node: &Mutex<Plug>, events: &mut Vec<Timed>, nparams: usize) {
    let n = ((*p).frames as usize).min(MAX_BLOCK);
    let ctx = Ctx {
        rate: (*p).rate,
        frame: (*p).frame,
        bpm: (*p).bpm,
        beats: (*p).beats,
        playing: (*p).playing != 0,
        sidechain: None,
    };
    events.clear();
    let mut reset = false;
    let count = ((*p).nevents as usize).min(shm::MAX_EVENTS);
    for e in &(&(*p).events)[..count] {
        if e.kind == shm::EV_RESET {
            reset = true;
        } else if let Some(ev) = event_of(e) {
            events.push(Timed {
                offset: e.offset.min(n.saturating_sub(1) as u32),
                ev,
            });
        }
    }
    let mut guard = node.lock().unwrap_or_else(|e| e.into_inner());
    let kind = guard.kind();
    let g = guard.node_mut();
    if reset {
        g.reset();
    }
    let out_l = &mut (&mut (*p).out_l)[..n];
    let out_r = &mut (&mut (*p).out_r)[..n];
    if kind == Kind::Effect {
        out_l.copy_from_slice(&(&(*p).in_l)[..n]);
        out_r.copy_from_slice(&(&(*p).in_r)[..n]);
    } else {
        out_l.fill(0.0);
        out_r.fill(0.0);
    }
    if n > 0 {
        g.process(&ctx, events, out_l, out_r);
    }
    for i in 0..nparams {
        (*p).values[i] = g.param(i as u16);
    }
    (*p).nvalues = nparams as u32;
    (*p).errors = guard.errors().min(u64::from(u32::MAX)) as u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn arguments_are_read_in_any_order() {
        let want = Args {
            plugin: "/x/p.clap".into(),
            id: "org.x".into(),
            shm_fd: 3,
            ctl_fd: 4,
        };
        assert_eq!(
            parse_args(os(&[
                "--plugin",
                "/x/p.clap",
                "--id",
                "org.x",
                "--shm-fd",
                "3",
                "--ctl-fd",
                "4"
            ])),
            Some(want)
        );
        let a = parse_args(os(&[
            "--ctl-fd", "9", "--shm-fd", "8", "--id", "y", "--plugin", "q",
        ]))
        .unwrap();
        assert_eq!((a.shm_fd, a.ctl_fd), (8, 9));
    }

    #[test]
    fn bad_arguments_are_refused() {
        assert_eq!(parse_args(os(&[])), None);
        assert_eq!(
            parse_args(os(&["--plugin", "p", "--id", "x", "--shm-fd", "3"])),
            None
        );
        assert_eq!(
            parse_args(os(&[
                "--plugin", "p", "--id", "x", "--shm-fd", "three", "--ctl-fd", "4"
            ])),
            None
        );
        assert_eq!(
            parse_args(os(&[
                "--plugin", "p", "--id", "x", "--shm-fd", "3", "--ctl-fd", "4", "--what", "1"
            ])),
            None
        );
        assert_eq!(parse_args(os(&["--plugin"])), None);
    }

    #[test]
    fn events_map_back() {
        let e = |kind, a, b, value| shm::ShmEvent {
            offset: 0,
            kind,
            a,
            b,
            value,
            _pad: 0,
        };
        assert_eq!(
            event_of(&e(shm::EV_NOTE_ON, 60, 100, 0.0)),
            Some(NodeEvent::NoteOn {
                note: 60,
                velocity: 100
            })
        );
        assert_eq!(
            event_of(&e(shm::EV_NOTE_OFF, 300, 0, 0.0)),
            Some(NodeEvent::NoteOff { note: 127 })
        );
        assert_eq!(
            event_of(&e(shm::EV_PRESSURE, 61, 40, 0.0)),
            Some(NodeEvent::Pressure {
                note: 61,
                value: 40
            })
        );
        assert_eq!(
            event_of(&e(shm::EV_BEND, 0, 0, -0.5)),
            Some(NodeEvent::Bend { value: -0.5 })
        );
        assert_eq!(
            event_of(&e(shm::EV_ALL_OFF, 0, 0, 0.0)),
            Some(NodeEvent::AllOff)
        );
        assert_eq!(
            event_of(&e(shm::EV_PARAM, 3, 0, 0.25)),
            Some(NodeEvent::Param { id: 3, value: 0.25 })
        );
        assert_eq!(event_of(&e(shm::EV_PARAM, 70_000, 0, 0.25)), None);
        assert_eq!(
            event_of(&e(shm::EV_EXPRESSION, 62, 1, 0.75)),
            Some(NodeEvent::Expression {
                note: 62,
                kind: Expr::Tone,
                value: 0.75
            })
        );
        assert_eq!(event_of(&e(shm::EV_EXPRESSION, 62, 9, 0.75)), None);
        assert_eq!(event_of(&e(99, 0, 0, 0.0)), None);
    }
}
