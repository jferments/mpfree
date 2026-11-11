//! A CLAP plugin host (roadmap P1): a CLAP instrument or effect as an
//! [`mpfree_node::Node`], so the track host, a mixer insert chain or an
//! offline render can hold a plugin exactly as it holds Poly or a filter.
//!
//! - **Libraries.** [`Library::open`] loads a `.clap` file (`dlopen`), checks
//!   its CLAP version and lists its plugins ([`Library::plugins`]).
//!   [`Library::from_entry`] does the same for an entry point linked into
//!   the program (our test plugins).
//! - **Nodes.** [`Library::create`] makes a [`ClapNode`]. A plugin with a
//!   note input is an instrument (it adds into the buffers); one without is
//!   an effect (it transforms them). [`Node::prepare`] activates it.
//! - **Parameters.** Each visible CLAP parameter becomes a [`ParamInfo`]
//!   whose id is its index; [`ClapNode::clap_param_id`] gives the CLAP id to
//!   store in a project. Stepped parameters are whole numbers, enums with a
//!   0-based range become named choices, the CLAP module path's first part
//!   is the group. Hidden parameters are left out.
//! - **Events.** Note on/off, pressure (a note expression), bend (MIDI pitch
//!   bend when the plugin takes MIDI, else a tuning expression on the held
//!   notes, 2 semitones at full bend), `AllOff` (a note-off for every held
//!   note) and parameter changes go to the plugin on their exact frame.
//!   Per-note expression maps to CLAP's own: a note's slide is its tuning
//!   expression (with the bend added when the bend goes that way), its tone
//!   is the brightness expression, and its release velocity goes in its
//!   note-off.
//!   The transport (tempo, beats, playing) goes with every block. Parameter
//!   values the plugin sends back update [`Node::param`].
//! - **State.** [`ClapControl`], taken before the node goes to the audio
//!   thread, saves and loads the plugin's state and answers its main-thread
//!   requests ([`ClapControl::idle`]).
//!
//! # Threads and real time
//!
//! CLAP names two threads: the main thread (here, the control thread that
//! made the library) and the audio thread (whoever calls `process`; it may
//! change, but never runs twice at once). [`ClapNode::process`] never
//! allocates or locks: events go into a list made in `prepare`, buffers are
//! made in `prepare`, parameter values are atomics. Whether the plugin
//! itself keeps the rules is up to it; the sandbox process (P2) is what
//! will protect the audio thread from one that doesn't.
//!
//! `prepare` and dropping the node stop processing from the control thread
//! while nothing else can touch the node; the host tells the plugin (through
//! `clap.thread-check`) that the calling thread is the audio thread for
//! that moment, which CLAP allows.
//!
//! A plugin's id, name and parameter list become `'static` the first time a
//! plugin with that id and list is made (they are kept for the life of the
//! program, once per distinct plugin), since nodes describe themselves with
//! `'static` data.

pub mod sys;
#[doc(hidden)]
pub mod testplug;

use mpfree_node::{
    Ctx, Curve, Expr, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed, Unit, MAX_BLOCK,
};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::fmt;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use sys::*;

/// Events one block can carry to the plugin (notes, pressure, parameters
/// and the note-offs of an `AllOff`); more are counted and dropped.
pub const MAX_EVENTS: usize = 512;
/// Parameter changes queued between blocks; more for the same parameter
/// replace the queued one, more for others are counted and dropped.
pub const MAX_PENDING: usize = 256;
/// Bend range used when the plugin only takes CLAP notes (semitones).
pub const BEND_SEMITONES: f64 = 2.0;
/// The most choices a stepped enum gets named; larger ones stay numbers.
const MAX_CHOICES: usize = 64;

/// Why a library or plugin could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `dlopen` failed (the text is `dlerror`'s).
    Open(String),
    /// The file has no `clap_entry` symbol.
    NoEntry,
    /// A CLAP version this host can't talk to.
    Version(u32, u32, u32),
    /// The entry's `init` returned false.
    Init,
    /// No plugin factory.
    NoFactory,
    /// No plugin with that id in this library.
    NoPlugin(String),
    /// `create_plugin` or the plugin's `init` failed.
    Create(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Open(e) => write!(f, "can't open the plugin file: {e}"),
            Error::NoEntry => write!(f, "not a CLAP plugin file (no clap_entry)"),
            Error::Version(a, b, c) => write!(f, "CLAP version {a}.{b}.{c} isn't supported"),
            Error::Init => write!(f, "the plugin file refused to start (init)"),
            Error::NoFactory => write!(f, "the plugin file has no plugin factory"),
            Error::NoPlugin(id) => write!(f, "no plugin {id} in this file"),
            Error::Create(id) => write!(f, "plugin {id} couldn't be made"),
        }
    }
}

impl std::error::Error for Error {}

// ------------------------------------------------------------------ libraries

/// A loaded entry point. Deinitialised (and the file closed) when the last
/// library handle and plugin made from it are gone.
struct Lib {
    /// The `dlopen` handle, or null for an entry linked into the program.
    handle: *mut c_void,
    entry: *const clap_plugin_entry,
    factory: *const clap_plugin_factory,
}

// SAFETY: a CLAP entry and factory are callable from the main thread, and
// `Lib` only calls them there (create, list, deinit on the last drop).
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

impl Drop for Lib {
    fn drop(&mut self) {
        // SAFETY: `init` succeeded for this entry; deinit balances it, and
        // nothing made from it is alive (every plugin holds an Arc<Lib>).
        unsafe {
            if let Some(deinit) = (*self.entry).deinit {
                deinit();
            }
            if !self.handle.is_null() {
                libc::dlclose(self.handle);
            }
        }
    }
}

/// What a library says about one of its plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDesc {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub description: String,
    /// CLAP feature tags, such as `"instrument"` or `"audio-effect"`.
    pub features: Vec<String>,
}

/// A CLAP plugin file, open. Open each file once and share the handle.
#[derive(Clone)]
pub struct Library {
    lib: Arc<Lib>,
}

impl Library {
    /// Loads a `.clap` file and starts it. Control thread only.
    pub fn open(path: &Path) -> Result<Library, Error> {
        let cpath = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| Error::Open("path has a NUL byte".into()))?;
        // SAFETY: dlopen/dlsym/dlerror with valid C strings; the handle is
        // closed in Lib's drop, or here on failure.
        unsafe {
            let handle = libc::dlopen(cpath.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if handle.is_null() {
                let e = libc::dlerror();
                let msg = if e.is_null() {
                    "unknown error".to_string()
                } else {
                    CStr::from_ptr(e).to_string_lossy().into_owned()
                };
                return Err(Error::Open(msg));
            }
            let sym = libc::dlsym(handle, c"clap_entry".as_ptr());
            if sym.is_null() {
                libc::dlclose(handle);
                return Err(Error::NoEntry);
            }
            Self::start(handle, sym as *const clap_plugin_entry, &cpath)
        }
    }

    /// Starts an entry point linked into this program. `path` is what the
    /// entry's `init` is told.
    pub fn from_entry(entry: &'static clap_plugin_entry, path: &str) -> Result<Library, Error> {
        let cpath = CString::new(path).map_err(|_| Error::Open("path has a NUL byte".into()))?;
        // SAFETY: the entry is 'static and valid.
        unsafe { Self::start(ptr::null_mut(), entry, &cpath) }
    }

    unsafe fn start(
        handle: *mut c_void,
        entry: *const clap_plugin_entry,
        path: &CStr,
    ) -> Result<Library, Error> {
        let close = || {
            if !handle.is_null() {
                libc::dlclose(handle);
            }
        };
        let v = (*entry).clap_version;
        if !clap_version_is_compatible(v) {
            close();
            return Err(Error::Version(v.major, v.minor, v.revision));
        }
        let ok = (*entry).init.is_some_and(|init| init(path.as_ptr()));
        if !ok {
            close();
            return Err(Error::Init);
        }
        let factory = (*entry).get_factory.map_or(ptr::null(), |g| {
            g(CLAP_PLUGIN_FACTORY_ID.as_ptr() as *const c_char)
        }) as *const clap_plugin_factory;
        let lib = Arc::new(Lib {
            handle,
            entry,
            factory,
        });
        if factory.is_null() {
            return Err(Error::NoFactory);
        }
        main_thread_mark();
        Ok(Library { lib })
    }

    /// The plugins in this file.
    pub fn plugins(&self) -> Vec<PluginDesc> {
        let f = self.lib.factory;
        let mut out = Vec::new();
        // SAFETY: the factory is valid while `lib` lives; descriptors are
        // owned by the plugin file.
        unsafe {
            let n = (*f).get_plugin_count.map_or(0, |c| c(f));
            for i in 0..n {
                let d = (*f).get_plugin_descriptor.map_or(ptr::null(), |g| g(f, i));
                if !d.is_null() {
                    out.push(desc_of(&*d));
                }
            }
        }
        out
    }

    /// Makes plugin `id` (not yet active: [`Node::prepare`] activates it).
    /// Control thread only.
    pub fn create(&self, id: &str) -> Result<ClapNode, Error> {
        let cid = CString::new(id).map_err(|_| Error::NoPlugin(id.to_string()))?;
        let desc = self
            .plugins()
            .into_iter()
            .find(|d| d.id == id)
            .ok_or_else(|| Error::NoPlugin(id.to_string()))?;
        let host = HostBox::new();
        let f = self.lib.factory;
        // SAFETY: valid factory and host; the host box outlives the plugin
        // (Instance drops the plugin first).
        unsafe {
            let plugin = (*f)
                .create_plugin
                .map_or(ptr::null(), |c| c(f, &host.clap, cid.as_ptr()));
            if plugin.is_null() {
                return Err(Error::Create(id.to_string()));
            }
            let mut inst = Instance {
                plugin,
                host,
                _lib: Arc::clone(&self.lib),
                params: ptr::null(),
                audio_ports: ptr::null(),
                note_ports: ptr::null(),
                state: ptr::null(),
                latency: ptr::null(),
                tail: ptr::null(),
                values: Box::new([]),
            };
            if !(*plugin).init.is_some_and(|i| i(plugin)) {
                return Err(Error::Create(id.to_string()));
            }
            inst.params = inst.ext(CLAP_EXT_PARAMS);
            inst.audio_ports = inst.ext(CLAP_EXT_AUDIO_PORTS);
            inst.note_ports = inst.ext(CLAP_EXT_NOTE_PORTS);
            inst.state = inst.ext(CLAP_EXT_STATE);
            inst.latency = inst.ext(CLAP_EXT_LATENCY);
            inst.tail = inst.ext(CLAP_EXT_TAIL);
            let (notes_in, midi) = inst.note_input();
            let kind = if notes_in {
                Kind::Instrument
            } else {
                Kind::Effect
            };
            let (params, map) = inst.scan_params(&desc.id);
            inst.values = map.iter().map(|_| AtomicU32::new(0)).collect();
            let info = intern_info(&desc.id, &desc.name, kind);
            let mut by_clap: Vec<(clap_id, u16)> = map
                .iter()
                .enumerate()
                .map(|(i, m)| (m.id, i as u16))
                .collect();
            by_clap.sort_unstable();
            let inst = Arc::new(inst);
            inst.refresh_values(&map);
            Ok(ClapNode {
                inst,
                info,
                params,
                map,
                by_clap,
                kind,
                midi,
                rate: 0,
                max_block: 0,
                active: false,
                processing: false,
                ports: Ports::default(),
                events: Vec::with_capacity(MAX_EVENTS),
                pending: Vec::with_capacity(MAX_PENDING),
                held: [false; 128],
                notes: NoteState::new(),
                latency: 0,
                stats: ClapStats::default(),
            })
        }
    }
}

unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

unsafe fn desc_of(d: &clap_plugin_descriptor) -> PluginDesc {
    let mut features = Vec::new();
    if !d.features.is_null() {
        let mut i = 0;
        loop {
            let f = *d.features.add(i);
            if f.is_null() || i > 256 {
                break;
            }
            features.push(cstr(f));
            i += 1;
        }
    }
    PluginDesc {
        id: cstr(d.id),
        name: cstr(d.name),
        vendor: cstr(d.vendor),
        version: cstr(d.version),
        description: cstr(d.description),
        features,
    }
}

// ------------------------------------------------------------------ threads

static NEXT_THREAD: AtomicU64 = AtomicU64::new(1);
static MAIN_THREAD: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static THREAD: u64 = NEXT_THREAD.fetch_add(1, Ordering::Relaxed);
}

fn this_thread() -> u64 {
    THREAD.with(|t| *t)
}

/// The first thread to open a library is CLAP's main thread.
fn main_thread_mark() {
    let _ = MAIN_THREAD.compare_exchange(0, this_thread(), Ordering::SeqCst, Ordering::SeqCst);
}

// ------------------------------------------------------------------ the host side

/// Requests a plugin makes of the host, as bits.
pub mod request {
    pub const RESTART: u32 = 1 << 0;
    pub const PROCESS: u32 = 1 << 1;
    pub const CALLBACK: u32 = 1 << 2;
    pub const PARAMS_RESCAN: u32 = 1 << 3;
    pub const PARAMS_FLUSH: u32 = 1 << 4;
    pub const LATENCY: u32 = 1 << 5;
    pub const TAIL: u32 = 1 << 6;
    pub const STATE_DIRTY: u32 = 1 << 7;
    pub const PORTS_RESCAN: u32 = 1 << 8;
}

/// The `clap_host` a plugin talks to, with what it asked for.
#[repr(C)]
struct HostBox {
    clap: clap_host,
    requests: AtomicU32,
    logs: AtomicU32,
    /// Messages at warning level or worse.
    warnings: AtomicU32,
    /// The thread acting as the audio thread now (0: none).
    audio_thread: AtomicU64,
    /// Host calls CLAP marks main-thread only that came from the audio
    /// thread (a misbehaving plugin).
    misplaced: AtomicU32,
}

impl HostBox {
    fn new() -> Box<HostBox> {
        let mut b = Box::new(HostBox {
            clap: clap_host {
                clap_version: CLAP_VERSION,
                host_data: ptr::null_mut(),
                name: c"MPFree".as_ptr(),
                vendor: c"MPFree".as_ptr(),
                url: c"https://github.com/jferments/mpfree".as_ptr(),
                version: c"0.1.0".as_ptr(),
                get_extension: Some(host_get_extension),
                request_restart: Some(host_request_restart),
                request_process: Some(host_request_process),
                request_callback: Some(host_request_callback),
            },
            requests: AtomicU32::new(0),
            logs: AtomicU32::new(0),
            warnings: AtomicU32::new(0),
            audio_thread: AtomicU64::new(0),
            misplaced: AtomicU32::new(0),
        });
        let me: *mut HostBox = &mut *b;
        b.clap.host_data = me as *mut c_void;
        b
    }
}

unsafe fn host_box<'a>(h: *const clap_host) -> Option<&'a HostBox> {
    if h.is_null() || (*h).host_data.is_null() {
        None
    } else {
        Some(&*((*h).host_data as *const HostBox))
    }
}

unsafe fn request(h: *const clap_host, bit: u32) {
    if let Some(b) = host_box(h) {
        b.requests.fetch_or(bit, Ordering::AcqRel);
    }
}

unsafe fn main_only(h: *const clap_host) {
    if let Some(b) = host_box(h) {
        if b.audio_thread.load(Ordering::Relaxed) == this_thread()
            && MAIN_THREAD.load(Ordering::Relaxed) != this_thread()
        {
            b.misplaced.fetch_add(1, Ordering::Relaxed);
        }
    }
}

unsafe extern "C" fn host_request_restart(h: *const clap_host) {
    request(h, request::RESTART);
}
unsafe extern "C" fn host_request_process(h: *const clap_host) {
    request(h, request::PROCESS);
}
unsafe extern "C" fn host_request_callback(h: *const clap_host) {
    request(h, request::CALLBACK);
}

static HOST_LOG: clap_host_log = clap_host_log {
    log: Some(host_log),
};
static HOST_THREAD_CHECK: clap_host_thread_check = clap_host_thread_check {
    is_main_thread: Some(host_is_main),
    is_audio_thread: Some(host_is_audio),
};
static HOST_PARAMS: clap_host_params = clap_host_params {
    rescan: Some(host_params_rescan),
    clear: Some(host_params_clear),
    request_flush: Some(host_params_flush),
};
static HOST_STATE: clap_host_state = clap_host_state {
    mark_dirty: Some(host_state_dirty),
};
static HOST_LATENCY: clap_host_latency = clap_host_latency {
    changed: Some(host_latency_changed),
};
static HOST_TAIL: clap_host_tail = clap_host_tail {
    changed: Some(host_tail_changed),
};
static HOST_AUDIO_PORTS: clap_host_audio_ports = clap_host_audio_ports {
    is_rescan_flag_supported: Some(host_ports_flag),
    rescan: Some(host_ports_rescan),
};
static HOST_NOTE_PORTS: clap_host_note_ports = clap_host_note_ports {
    supported_dialects: Some(host_note_dialects),
    rescan: Some(host_ports_rescan),
};

unsafe extern "C" fn host_get_extension(_h: *const clap_host, id: *const c_char) -> *const c_void {
    if id.is_null() {
        return ptr::null();
    }
    let id = CStr::from_ptr(id).to_bytes_with_nul();
    let p: *const c_void = if id == CLAP_EXT_LOG {
        &HOST_LOG as *const _ as *const c_void
    } else if id == CLAP_EXT_THREAD_CHECK {
        &HOST_THREAD_CHECK as *const _ as *const c_void
    } else if id == CLAP_EXT_PARAMS {
        &HOST_PARAMS as *const _ as *const c_void
    } else if id == CLAP_EXT_STATE {
        &HOST_STATE as *const _ as *const c_void
    } else if id == CLAP_EXT_LATENCY {
        &HOST_LATENCY as *const _ as *const c_void
    } else if id == CLAP_EXT_TAIL {
        &HOST_TAIL as *const _ as *const c_void
    } else if id == CLAP_EXT_AUDIO_PORTS {
        &HOST_AUDIO_PORTS as *const _ as *const c_void
    } else if id == CLAP_EXT_NOTE_PORTS {
        &HOST_NOTE_PORTS as *const _ as *const c_void
    } else {
        ptr::null()
    };
    p
}

unsafe extern "C" fn host_log(h: *const clap_host, severity: i32, _msg: *const c_char) {
    // Counted, never printed: printing could block the audio thread.
    if let Some(b) = host_box(h) {
        b.logs.fetch_add(1, Ordering::Relaxed);
        if severity >= CLAP_LOG_WARNING {
            b.warnings.fetch_add(1, Ordering::Relaxed);
        }
    }
}

unsafe extern "C" fn host_is_main(_h: *const clap_host) -> bool {
    MAIN_THREAD.load(Ordering::Relaxed) == this_thread()
}

unsafe extern "C" fn host_is_audio(h: *const clap_host) -> bool {
    host_box(h).is_some_and(|b| b.audio_thread.load(Ordering::Relaxed) == this_thread())
}

unsafe extern "C" fn host_params_rescan(h: *const clap_host, _flags: u32) {
    main_only(h);
    request(h, request::PARAMS_RESCAN);
}
unsafe extern "C" fn host_params_clear(h: *const clap_host, _id: clap_id, _flags: u32) {
    main_only(h);
}
unsafe extern "C" fn host_params_flush(h: *const clap_host) {
    request(h, request::PARAMS_FLUSH);
}
unsafe extern "C" fn host_state_dirty(h: *const clap_host) {
    request(h, request::STATE_DIRTY);
}
unsafe extern "C" fn host_latency_changed(h: *const clap_host) {
    request(h, request::LATENCY);
}
unsafe extern "C" fn host_tail_changed(h: *const clap_host) {
    request(h, request::TAIL);
}
unsafe extern "C" fn host_ports_flag(_h: *const clap_host, _flag: u32) -> bool {
    false
}
unsafe extern "C" fn host_ports_rescan(h: *const clap_host, _flags: u32) {
    main_only(h);
    request(h, request::PORTS_RESCAN);
}
unsafe extern "C" fn host_note_dialects(_h: *const clap_host) -> u32 {
    CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI
}

// ------------------------------------------------------------------ instances

/// One plugin instance, shared by its node (audio side) and its control
/// handle (main side), as CLAP's thread rules allow.
struct Instance {
    plugin: *const clap_plugin,
    host: Box<HostBox>,
    /// Keeps the plugin file loaded while the plugin lives.
    _lib: Arc<Lib>,
    params: *const clap_plugin_params,
    audio_ports: *const clap_plugin_audio_ports,
    note_ports: *const clap_plugin_note_ports,
    state: *const clap_plugin_state,
    latency: *const clap_plugin_latency,
    tail: *const clap_plugin_tail,
    /// Each visible parameter's value now, as f32 bits.
    values: Box<[AtomicU32]>,
}

// SAFETY: CLAP plugins are made to be called from the main thread and the
// audio thread at once, each function on its named thread; the node and
// the control handle keep to that. Everything else here is atomics or
// immutable after `create`.
unsafe impl Send for Instance {}
unsafe impl Sync for Instance {}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the node deactivated the plugin before letting go of it
        // (and CLAP asks for destroy after a failed init too); destroy is
        // the last call. The host box and the library outlive this call:
        // fields drop after this body.
        unsafe {
            if let Some(destroy) = (*self.plugin).destroy {
                destroy(self.plugin);
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PMap {
    id: clap_id,
    cookie: *mut c_void,
}

impl Instance {
    unsafe fn ext<T>(&self, id: &[u8]) -> *const T {
        (*self.plugin).get_extension.map_or(ptr::null(), |g| {
            g(self.plugin, id.as_ptr() as *const c_char)
        }) as *const T
    }

    /// Whether the plugin takes notes, and whether its main note port takes
    /// MIDI.
    unsafe fn note_input(&self) -> (bool, bool) {
        let np = self.note_ports;
        if np.is_null() {
            return (false, false);
        }
        let n = (*np).count.map_or(0, |c| c(self.plugin, true));
        if n == 0 {
            return (false, false);
        }
        let mut info: clap_note_port_info = std::mem::zeroed();
        let ok = (*np)
            .get
            .is_some_and(|g| g(self.plugin, 0, true, &mut info));
        (
            true,
            ok && info.supported_dialects & CLAP_NOTE_DIALECT_MIDI != 0,
        )
    }

    /// The visible parameters, as `'static` descriptions and a map to CLAP
    /// ids.
    unsafe fn scan_params(&self, plugin_id: &str) -> (&'static [ParamInfo], Vec<PMap>) {
        let pp = self.params;
        if pp.is_null() {
            return (&[], Vec::new());
        }
        let n = (*pp).count.map_or(0, |c| c(self.plugin));
        let mut owned = Vec::new();
        let mut map = Vec::new();
        for i in 0..n {
            if map.len() >= usize::from(u16::MAX) {
                break;
            }
            let mut info: clap_param_info = std::mem::zeroed();
            if !(*pp).get_info.is_some_and(|g| g(self.plugin, i, &mut info))
                || info.flags & CLAP_PARAM_IS_HIDDEN != 0
            {
                continue;
            }
            let name = fixed_str(&info.name);
            let module = fixed_str(&info.module);
            let group = module
                .split('/')
                .find(|s| !s.is_empty())
                .unwrap_or("Plugin")
                .to_string();
            let (min, max) = (info.min_value as f32, info.max_value as f32);
            let (min, max) = if min.is_finite() && max.is_finite() && min <= max {
                (min, max)
            } else {
                (0.0, 1.0)
            };
            let default = (info.default_value as f32).clamp(min, max);
            let default = if default.is_finite() { default } else { min };
            let stepped = info.flags & CLAP_PARAM_IS_STEPPED != 0;
            let mut choices = None;
            if stepped
                && info.flags & CLAP_PARAM_IS_ENUM != 0
                && min == 0.0
                && max >= 1.0
                && (max as usize) < MAX_CHOICES
            {
                let names: Option<Vec<String>> = (0..=max as usize)
                    .map(|k| self.text(info.id, k as f64))
                    .collect();
                choices = names;
            }
            let curve = match (stepped, choices.is_some()) {
                (true, true) => CurveSpec::Choice(choices.unwrap_or_default()),
                (true, false)
                    if min == 0.0 && max == 1.0 && info.flags & CLAP_PARAM_IS_BYPASS != 0 =>
                {
                    CurveSpec::Toggle
                }
                (true, false) => CurveSpec::Stepped,
                _ => CurveSpec::Linear,
            };
            let default = if stepped {
                default.round().clamp(min, max)
            } else {
                default
            };
            owned.push(ParamSpec {
                name,
                group,
                min,
                max,
                default,
                curve,
                readonly: info.flags & CLAP_PARAM_IS_READONLY != 0,
            });
            map.push(PMap {
                id: info.id,
                cookie: info.cookie,
            });
        }
        (intern_params(plugin_id, owned), map)
    }

    /// A value as the plugin writes it (main thread).
    unsafe fn text(&self, id: clap_id, value: f64) -> Option<String> {
        let pp = self.params;
        if pp.is_null() {
            return None;
        }
        let mut buf = [0 as c_char; 128];
        if (*pp)
            .value_to_text
            .is_some_and(|f| f(self.plugin, id, value, buf.as_mut_ptr(), buf.len() as u32))
        {
            buf[buf.len() - 1] = 0;
            Some(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
        } else {
            None
        }
    }

    /// Reads every visible parameter's value from the plugin (main thread).
    fn refresh_values(&self, map: &[PMap]) {
        let pp = self.params;
        if pp.is_null() {
            return;
        }
        for (i, m) in map.iter().enumerate() {
            let mut v = 0f64;
            // SAFETY: main-thread call on a live plugin.
            let ok = unsafe {
                (*pp)
                    .get_value
                    .is_some_and(|g| g(self.plugin, m.id, &mut v))
            };
            if ok {
                self.values[i].store((v as f32).to_bits(), Ordering::Relaxed);
            }
        }
    }

    fn mark_audio(&self) {
        self.host
            .audio_thread
            .store(this_thread(), Ordering::Relaxed);
    }
}

fn fixed_str(b: &[c_char]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    let bytes: Vec<u8> = b[..n].iter().map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

// ------------------------------------------------------------------ 'static descriptions

/// A parameter curve with owned names (see [`ParamSpec`]).
#[derive(Clone, PartialEq, Debug)]
pub enum CurveSpec {
    Linear,
    Stepped,
    Toggle,
    Choice(Vec<String>),
}

/// A parameter description with owned strings: what a plugin says about a
/// parameter, before [`intern_params`] makes it a `'static` [`ParamInfo`].
/// The sandbox sends these between processes.
#[derive(Clone, PartialEq, Debug)]
pub struct ParamSpec {
    pub name: String,
    pub group: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub curve: CurveSpec,
    pub readonly: bool,
}

impl ParamSpec {
    /// The owned form of a description.
    pub fn from_info(p: &ParamInfo) -> ParamSpec {
        ParamSpec {
            name: p.name.to_string(),
            group: p.group.to_string(),
            min: p.min,
            max: p.max,
            default: p.default,
            curve: match p.curve {
                Curve::Stepped => CurveSpec::Stepped,
                Curve::Toggle => CurveSpec::Toggle,
                Curve::Choice(names) => {
                    CurveSpec::Choice(names.iter().map(|n| n.to_string()).collect())
                }
                Curve::Linear | Curve::Log => CurveSpec::Linear,
            },
            readonly: p.readonly,
        }
    }
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

type ParamKey = (String, Vec<OwnedParamKey>);
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct OwnedParamKey(String, String, u32, u32, u32, String, bool);

fn key_of(p: &ParamSpec) -> OwnedParamKey {
    let curve = match &p.curve {
        CurveSpec::Linear => "l".to_string(),
        CurveSpec::Stepped => "s".to_string(),
        CurveSpec::Toggle => "t".to_string(),
        CurveSpec::Choice(n) => format!("c{}", n.join("\u{1}")),
    };
    OwnedParamKey(
        p.name.clone(),
        p.group.clone(),
        p.min.to_bits(),
        p.max.to_bits(),
        p.default.to_bits(),
        curve,
        p.readonly,
    )
}

/// A plugin's range as [`ParamInfo::clamp`] needs it: finite ends (0 and 1
/// stand in for one that is not), the lower first, and the default inside.
/// `f32::clamp` panics on a NaN or reversed range, and a plugin (or a
/// sandboxed plugin process) may report anything.
fn safe_range(p: &ParamSpec) -> (f32, f32, f32) {
    let finite = |x: f32, or: f32| if x.is_finite() { x } else { or };
    let (a, b) = (finite(p.min, 0.0), finite(p.max, 1.0));
    let (min, max) = if a <= b { (a, b) } else { (b, a) };
    (min, max, finite(p.default, min).clamp(min, max))
}

/// The `'static` descriptions for a plugin's parameter list; their ids are
/// their positions. Each distinct (plugin, list) is leaked once, for the
/// life of the program.
pub fn intern_params(plugin_id: &str, list: Vec<ParamSpec>) -> &'static [ParamInfo] {
    static TABLE: OnceLock<Mutex<HashMap<ParamKey, &'static [ParamInfo]>>> = OnceLock::new();
    let key = (
        plugin_id.to_string(),
        list.iter().map(key_of).collect::<Vec<_>>(),
    );
    let table = TABLE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut t = table.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = t.get(&key) {
        return p;
    }
    let infos: Vec<ParamInfo> = list
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (min, max, default) = safe_range(p);
            ParamInfo {
                id: i as u16,
                name: leak(&p.name),
                group: leak(&p.group),
                min,
                max,
                default,
                unit: Unit::None,
                curve: match &p.curve {
                    CurveSpec::Linear => Curve::Linear,
                    CurveSpec::Stepped => Curve::Stepped,
                    CurveSpec::Toggle => Curve::Toggle,
                    CurveSpec::Choice(names) => {
                        let names: Vec<&'static str> = names.iter().map(|n| leak(n)).collect();
                        Curve::Choice(Box::leak(names.into_boxed_slice()))
                    }
                },
                readonly: p.readonly,
            }
        })
        .collect();
    let infos: &'static [ParamInfo] = Box::leak(infos.into_boxed_slice());
    t.insert(key, infos);
    infos
}

type InfoTable = HashMap<(String, String, bool), &'static NodeInfo>;

/// The `'static` [`NodeInfo`] for plugin `id` (shown as `name`): its node
/// id is `clap:` and the plugin id. Leaked once per distinct plugin.
pub fn intern_info(id: &str, name: &str, kind: Kind) -> &'static NodeInfo {
    static TABLE: OnceLock<Mutex<InfoTable>> = OnceLock::new();
    let key = (id.to_string(), name.to_string(), kind == Kind::Instrument);
    let table = TABLE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut t = table.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = t.get(&key) {
        return i;
    }
    let info: &'static NodeInfo = Box::leak(Box::new(NodeInfo {
        id: leak(&format!("clap:{id}")),
        name: leak(name),
        kind,
    }));
    t.insert(key, info);
    info
}

// ------------------------------------------------------------------ the node

/// A node's counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClapStats {
    /// Blocks the plugin processed.
    pub blocks: u64,
    /// Blocks it failed (`CLAP_PROCESS_ERROR`, or `start_processing`
    /// refused); their output is silence (an instrument) or the dry input
    /// (an effect).
    pub errors: u64,
    /// Events that didn't fit in a block's list.
    pub dropped: u64,
    /// `activate` failures.
    pub activate_failed: u64,
}

/// One audio port's buffers: a channel each, and the pointer list CLAP
/// reads.
#[derive(Default)]
struct Port {
    chans: Vec<Vec<f32>>,
    ptrs: Vec<*mut f32>,
}

#[derive(Default)]
struct Ports {
    inputs: Vec<Port>,
    outputs: Vec<Port>,
    in_bufs: Vec<clap_audio_buffer>,
    out_bufs: Vec<clap_audio_buffer>,
    main_in: Option<usize>,
    main_out: Option<usize>,
}

/// One event in the list handed to the plugin.
#[repr(C)]
#[derive(Clone, Copy)]
union EvSlot {
    header: clap_event_header,
    note: clap_event_note,
    expr: clap_event_note_expression,
    param: clap_event_param_value,
    midi: clap_event_midi,
}

/// What the plugin's `in_events` points at during one call.
struct InList {
    slots: *const EvSlot,
    len: u32,
}

unsafe extern "C" fn in_size(list: *const clap_input_events) -> u32 {
    (*((*list).ctx as *const InList)).len
}

unsafe extern "C" fn in_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    let l = &*((*list).ctx as *const InList);
    if index < l.len {
        &(*l.slots.add(index as usize)).header
    } else {
        ptr::null()
    }
}

/// What the plugin's `out_events` writes to during one call.
struct OutList<'a> {
    by_clap: &'a [(clap_id, u16)],
    values: &'a [AtomicU32],
}

unsafe extern "C" fn out_push(
    list: *const clap_output_events,
    ev: *const clap_event_header,
) -> bool {
    if ev.is_null() {
        return false;
    }
    let l = &*((*list).ctx as *const OutList);
    if (*ev).space_id == CLAP_CORE_EVENT_SPACE_ID && (*ev).type_ == CLAP_EVENT_PARAM_VALUE {
        let e = &*(ev as *const clap_event_param_value);
        if let Ok(k) = l.by_clap.binary_search_by_key(&e.param_id, |&(id, _)| id) {
            let i = usize::from(l.by_clap[k].1);
            l.values[i].store((e.value as f32).to_bits(), Ordering::Relaxed);
        }
    }
    // Gestures, note ends and the rest are taken and ignored.
    true
}

/// A CLAP plugin as a node. See the crate docs.
pub struct ClapNode {
    inst: Arc<Instance>,
    info: &'static NodeInfo,
    params: &'static [ParamInfo],
    map: Vec<PMap>,
    by_clap: Vec<(clap_id, u16)>,
    kind: Kind,
    midi: bool,
    rate: u32,
    max_block: usize,
    active: bool,
    processing: bool,
    ports: Ports,
    events: Vec<EvSlot>,
    pending: Vec<(u16, f32)>,
    held: [bool; 128],
    /// The bend and each note's own slide, tone and release velocity.
    notes: NoteState,
    latency: u32,
    stats: ClapStats,
}

/// What the per-note events say, so a tuning expression carries the bend
/// and the note's slide together, and a note starts with its own values.
#[derive(Clone, Copy, Debug)]
struct NoteState {
    /// The bend in semitones, when it goes in tuning expressions (0 for a
    /// plugin that takes MIDI: its bend goes as MIDI pitch bend).
    bend: f64,
    slide: [f32; 128],
    tone: [f32; 128],
    /// Release velocity for the coming note-off; NaN for none sent.
    release: [f32; 128],
}

impl NoteState {
    fn new() -> NoteState {
        NoteState {
            bend: 0.0,
            slide: [0.0; 128],
            tone: [0.0; 128],
            release: [f32::NAN; 128],
        }
    }

    /// A note's tuning expression: the bend plus its slide.
    fn tuning(&self, key: u8) -> f64 {
        self.bend + f64::from(self.slide[usize::from(key)])
    }

    /// At a note-off: its velocity (0 when none was sent), and the note
    /// back to neutral.
    fn off(&mut self, key: u8) -> f64 {
        let k = usize::from(key);
        self.slide[k] = 0.0;
        self.tone[k] = 0.0;
        let r = std::mem::replace(&mut self.release[k], f32::NAN);
        if r.is_nan() {
            0.0
        } else {
            f64::from(r)
        }
    }
}

// SAFETY: the raw pointers are into this node's own buffers (made in
// prepare, never moved after) or the plugin, which CLAP lets the audio
// thread use; the node is used by one thread at a time (&mut self).
unsafe impl Send for ClapNode {}

impl ClapNode {
    /// The handle for main-thread work (state, the plugin's requests).
    pub fn control(&self) -> ClapControl {
        ClapControl {
            inst: Arc::clone(&self.inst),
            map: self.map.iter().map(|m| m.id).collect(),
        }
    }

    /// The CLAP id of parameter `id` (its index), for project files.
    pub fn clap_param_id(&self, id: u16) -> Option<u32> {
        self.map.get(usize::from(id)).map(|m| m.id)
    }

    /// The parameter index of CLAP id `clap`.
    pub fn param_index(&self, clap: u32) -> Option<u16> {
        self.by_clap
            .binary_search_by_key(&clap, |&(id, _)| id)
            .ok()
            .map(|k| self.by_clap[k].1)
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn stats(&self) -> ClapStats {
        self.stats
    }

    /// Whether the plugin is active (`prepare` worked).
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Messages the plugin logged, and how many were warnings or worse.
    pub fn logs(&self) -> (u32, u32) {
        (
            self.inst.host.logs.load(Ordering::Relaxed),
            self.inst.host.warnings.load(Ordering::Relaxed),
        )
    }

    /// Main-thread-only host calls the plugin made from the audio thread.
    pub fn misplaced_calls(&self) -> u32 {
        self.inst.host.misplaced.load(Ordering::Relaxed)
    }

    /// The raw plugin, for tests of our own test plugins.
    #[doc(hidden)]
    pub fn raw_plugin(&self) -> *const clap_plugin {
        self.inst.plugin
    }

    fn stop(&mut self) {
        if self.processing {
            self.inst.mark_audio();
            // SAFETY: active and processing; nothing else runs this node.
            unsafe {
                if let Some(f) = (*self.inst.plugin).stop_processing {
                    f(self.inst.plugin);
                }
            }
            self.processing = false;
        }
    }

    fn deactivate(&mut self) {
        self.stop();
        if self.active {
            // SAFETY: main thread, active, not processing.
            unsafe {
                if let Some(f) = (*self.inst.plugin).deactivate {
                    f(self.inst.plugin);
                }
            }
            self.active = false;
        }
    }

    unsafe fn build_ports(&mut self, max_block: usize) {
        let mut ports = Ports::default();
        let ap = self.inst.audio_ports;
        for is_input in [true, false] {
            let n = if ap.is_null() {
                0
            } else {
                (*ap).count.map_or(0, |c| c(self.inst.plugin, is_input))
            };
            for i in 0..n.min(16) {
                let mut info: clap_audio_port_info = std::mem::zeroed();
                let ok = (*ap)
                    .get
                    .is_some_and(|g| g(self.inst.plugin, i, is_input, &mut info));
                let ch = if ok {
                    info.channel_count.clamp(1, 32) as usize
                } else {
                    2
                };
                let mut port = Port {
                    chans: (0..ch).map(|_| vec![0f32; max_block]).collect(),
                    ptrs: Vec::new(),
                };
                port.ptrs = port.chans.iter_mut().map(|c| c.as_mut_ptr()).collect();
                let main = ok && info.flags & CLAP_AUDIO_PORT_IS_MAIN != 0;
                if is_input {
                    if main && ports.main_in.is_none() {
                        ports.main_in = Some(ports.inputs.len());
                    }
                    ports.inputs.push(port);
                } else {
                    if main && ports.main_out.is_none() {
                        ports.main_out = Some(ports.outputs.len());
                    }
                    ports.outputs.push(port);
                }
            }
        }
        if ports.main_in.is_none() && !ports.inputs.is_empty() {
            ports.main_in = Some(0);
        }
        if ports.main_out.is_none() && !ports.outputs.is_empty() {
            ports.main_out = Some(0);
        }
        let mk = |p: &mut Port| clap_audio_buffer {
            data32: p.ptrs.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: p.ptrs.len() as u32,
            latency: 0,
            constant_mask: 0,
        };
        ports.in_bufs = ports.inputs.iter_mut().map(mk).collect();
        ports.out_bufs = ports.outputs.iter_mut().map(mk).collect();
        self.ports = ports;
    }

    fn push_ev(&mut self, slot: EvSlot) {
        if self.events.len() < self.events.capacity() {
            self.events.push(slot);
        } else {
            self.stats.dropped += 1;
        }
    }

    fn header(ty: u16, size: usize, time: u32) -> clap_event_header {
        clap_event_header {
            size: size as u32,
            time,
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_: ty,
            flags: CLAP_EVENT_IS_LIVE,
        }
    }

    fn note(&mut self, ty: u16, time: u32, key: u8, velocity: f64) {
        let note = clap_event_note {
            header: Self::header(ty, std::mem::size_of::<clap_event_note>(), time),
            note_id: -1,
            port_index: 0,
            channel: 0,
            key: i16::from(key),
            velocity,
        };
        self.push_ev(EvSlot { note });
    }

    fn expression(&mut self, time: u32, which: i32, key: i16, value: f64) {
        let expr = clap_event_note_expression {
            header: Self::header(
                CLAP_EVENT_NOTE_EXPRESSION,
                std::mem::size_of::<clap_event_note_expression>(),
                time,
            ),
            expression_id: which,
            note_id: -1,
            port_index: 0,
            channel: 0,
            key,
            value,
        };
        self.push_ev(EvSlot { expr });
    }

    fn param_ev(&mut self, time: u32, index: u16, value: f32) {
        let Some(m) = self.map.get(usize::from(index)).copied() else {
            return;
        };
        let param = clap_event_param_value {
            header: Self::header(
                CLAP_EVENT_PARAM_VALUE,
                std::mem::size_of::<clap_event_param_value>(),
                time,
            ),
            param_id: m.id,
            cookie: m.cookie,
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: f64::from(value),
        };
        self.push_ev(EvSlot { param });
    }

    /// One of our events into the list, at `time` frames into the call.
    fn translate(&mut self, time: u32, ev: &NodeEvent) {
        let notes = self.kind == Kind::Instrument;
        match *ev {
            NodeEvent::NoteOn { note, velocity } if notes && note < 128 => {
                if velocity == 0 {
                    self.held[usize::from(note)] = false;
                    let off = self.notes.off(note);
                    self.note(CLAP_EVENT_NOTE_OFF, time, note, off);
                } else {
                    self.held[usize::from(note)] = true;
                    self.note(
                        CLAP_EVENT_NOTE_ON,
                        time,
                        note,
                        f64::from(velocity.min(127)) / 127.0,
                    );
                    // A new note starts at the bend and its own values.
                    let tuning = self.notes.tuning(note);
                    if tuning != 0.0 {
                        self.expression(time, CLAP_NOTE_EXPRESSION_TUNING, i16::from(note), tuning);
                    }
                    let tone = self.notes.tone[usize::from(note)];
                    if tone != 0.0 {
                        self.expression(
                            time,
                            CLAP_NOTE_EXPRESSION_BRIGHTNESS,
                            i16::from(note),
                            f64::from(tone),
                        );
                    }
                }
            }
            NodeEvent::NoteOff { note } if notes && note < 128 => {
                self.held[usize::from(note)] = false;
                let off = self.notes.off(note);
                self.note(CLAP_EVENT_NOTE_OFF, time, note, off);
            }
            NodeEvent::Expression { note, kind, value } if notes && note < 128 => {
                let Some(v) = kind.clamp(value) else {
                    return;
                };
                let k = usize::from(note);
                match kind {
                    Expr::Slide => {
                        self.notes.slide[k] = v;
                        if self.held[k] {
                            let t = self.notes.tuning(note);
                            self.expression(time, CLAP_NOTE_EXPRESSION_TUNING, i16::from(note), t);
                        }
                    }
                    Expr::Tone => {
                        self.notes.tone[k] = v;
                        if self.held[k] {
                            self.expression(
                                time,
                                CLAP_NOTE_EXPRESSION_BRIGHTNESS,
                                i16::from(note),
                                f64::from(v),
                            );
                        }
                    }
                    Expr::Release => self.notes.release[k] = v,
                }
            }
            NodeEvent::Pressure { note, value } if notes && note < 128 => {
                self.expression(
                    time,
                    CLAP_NOTE_EXPRESSION_PRESSURE,
                    i16::from(note),
                    f64::from(value.min(127)) / 127.0,
                );
            }
            NodeEvent::Bend { value } if notes => {
                let v = f64::from(value).clamp(-1.0, 1.0);
                if self.midi {
                    let b = (v * 8191.0 + 8192.0).round().clamp(0.0, 16383.0) as u16;
                    let midi = clap_event_midi {
                        header: Self::header(
                            CLAP_EVENT_MIDI,
                            std::mem::size_of::<clap_event_midi>(),
                            time,
                        ),
                        port_index: 0,
                        data: [0xE0, (b & 0x7F) as u8, (b >> 7) as u8],
                    };
                    self.push_ev(EvSlot { midi });
                } else {
                    self.notes.bend = v * BEND_SEMITONES;
                    for k in 0..128u8 {
                        if self.held[usize::from(k)] {
                            let t = self.notes.tuning(k);
                            self.expression(time, CLAP_NOTE_EXPRESSION_TUNING, i16::from(k), t);
                        }
                    }
                }
            }
            NodeEvent::AllOff if notes => {
                for k in 0..128u8 {
                    if std::mem::take(&mut self.held[usize::from(k)]) {
                        let off = self.notes.off(k);
                        self.note(CLAP_EVENT_NOTE_OFF, time, k, off);
                    }
                }
                let bend = self.notes.bend;
                self.notes = NoteState::new();
                self.notes.bend = bend;
            }
            NodeEvent::Param { id, value } => {
                if let Some(p) = self.params.get(usize::from(id)) {
                    if !p.readonly {
                        let v = p.clamp(value);
                        self.inst.values[usize::from(id)].store(v.to_bits(), Ordering::Relaxed);
                        self.param_ev(time, id, v);
                    }
                }
            }
            _ => {}
        }
    }

    /// Runs the plugin over `len` frames starting `at` frames into the
    /// block, with this chunk's events already in `self.events`.
    unsafe fn run(
        &mut self,
        ctx: &Ctx,
        at: usize,
        len: usize,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let effect = self.kind == Kind::Effect;
        // Inputs: the track's sound into the main input (an effect), zeros
        // elsewhere.
        for (i, port) in self.ports.inputs.iter_mut().enumerate() {
            for (c, ch) in port.chans.iter_mut().enumerate() {
                let src: Option<&[f32]> = if effect && Some(i) == self.ports.main_in {
                    Some(if c % 2 == 0 {
                        &left[at..at + len]
                    } else {
                        &right[at..at + len]
                    })
                } else {
                    None
                };
                match src {
                    Some(s) => ch[..len].copy_from_slice(s),
                    None => ch[..len].fill(0.0),
                }
            }
        }
        for port in &mut self.ports.outputs {
            for ch in &mut port.chans {
                ch[..len].fill(0.0);
            }
        }
        let beats_per_frame = ctx.bpm / 60.0 / f64::from(ctx.rate.max(1));
        let beats = ctx.beats + at as f64 * beats_per_frame;
        let secs = (ctx.frame + at as u64) as f64 / f64::from(ctx.rate.max(1));
        let mut flags = CLAP_TRANSPORT_HAS_TEMPO
            | CLAP_TRANSPORT_HAS_BEATS_TIMELINE
            | CLAP_TRANSPORT_HAS_SECONDS_TIMELINE;
        if ctx.playing {
            flags |= CLAP_TRANSPORT_IS_PLAYING;
        }
        let transport = clap_event_transport {
            header: Self::header(
                CLAP_EVENT_TRANSPORT,
                std::mem::size_of::<clap_event_transport>(),
                0,
            ),
            flags,
            song_pos_beats: (beats * CLAP_BEATTIME_FACTOR) as i64,
            song_pos_seconds: (secs * CLAP_SECTIME_FACTOR) as i64,
            tempo: ctx.bpm,
            tempo_inc: 0.0,
            loop_start_beats: 0,
            loop_end_beats: 0,
            loop_start_seconds: 0,
            loop_end_seconds: 0,
            bar_start: 0,
            bar_number: 0,
            tsig_num: 4,
            tsig_denom: 4,
        };
        let in_list = InList {
            slots: self.events.as_ptr(),
            len: self.events.len() as u32,
        };
        let in_events = clap_input_events {
            ctx: &in_list as *const InList as *mut c_void,
            size: Some(in_size),
            get: Some(in_get),
        };
        let out_list = OutList {
            by_clap: &self.by_clap,
            values: &self.inst.values,
        };
        let out_events = clap_output_events {
            ctx: &out_list as *const OutList as *mut c_void,
            try_push: Some(out_push),
        };
        let process = clap_process {
            steady_time: (ctx.frame + at as u64) as i64,
            frames_count: len as u32,
            transport: &transport,
            audio_inputs: self.ports.in_bufs.as_ptr(),
            audio_outputs: self.ports.out_bufs.as_mut_ptr(),
            audio_inputs_count: self.ports.in_bufs.len() as u32,
            audio_outputs_count: self.ports.out_bufs.len() as u32,
            in_events: &in_events,
            out_events: &out_events,
        };
        let status = (*self.inst.plugin)
            .process
            .map_or(CLAP_PROCESS_ERROR, |p| p(self.inst.plugin, &process));
        self.stats.blocks += 1;
        if status == CLAP_PROCESS_ERROR {
            // Silence from an instrument; the dry input stays for an effect.
            self.stats.errors += 1;
            return;
        }
        let Some(out) = self.ports.main_out.and_then(|o| self.ports.outputs.get(o)) else {
            return;
        };
        let ol = &out.chans[0][..len];
        let or = &out.chans[if out.chans.len() > 1 { 1 } else { 0 }][..len];
        if effect {
            left[at..at + len].copy_from_slice(ol);
            right[at..at + len].copy_from_slice(or);
        } else {
            for (d, s) in left[at..at + len].iter_mut().zip(ol) {
                *d += *s;
            }
            for (d, s) in right[at..at + len].iter_mut().zip(or) {
                *d += *s;
            }
        }
    }
}

impl Node for ClapNode {
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
        self.inst.values[usize::from(id)].store(v.to_bits(), Ordering::Relaxed);
        if let Some(q) = self.pending.iter_mut().find(|q| q.0 == id) {
            q.1 = v;
        } else if self.pending.len() < self.pending.capacity() {
            self.pending.push((id, v));
        } else {
            self.stats.dropped += 1;
        }
    }

    fn param(&self, id: u16) -> f32 {
        self.inst
            .values
            .get(usize::from(id))
            .map_or(f32::NAN, |v| f32::from_bits(v.load(Ordering::Relaxed)))
    }

    fn prepare(&mut self, rate: u32, max_block: usize) {
        let max_block = max_block.clamp(1, MAX_BLOCK);
        self.deactivate();
        self.rate = rate;
        self.max_block = max_block;
        // SAFETY: main thread, plugin inactive.
        unsafe {
            self.build_ports(max_block);
            let ok = (*self.inst.plugin)
                .activate
                .is_some_and(|a| a(self.inst.plugin, f64::from(rate), 1, max_block as u32));
            self.active = ok;
            if !ok {
                self.stats.activate_failed += 1;
            }
            self.latency = if ok && !self.inst.latency.is_null() {
                (*self.inst.latency).get.map_or(0, |g| g(self.inst.plugin))
            } else {
                0
            };
        }
        self.inst.refresh_values(&self.map);
        // Changes still queued for the plugin are what param() shows.
        for &(id, v) in &self.pending {
            self.inst.values[usize::from(id)].store(v.to_bits(), Ordering::Relaxed);
        }
        self.held = [false; 128];
        self.notes = NoteState::new();
    }

    fn reset(&mut self) {
        if self.active {
            self.inst.mark_audio();
            // SAFETY: audio-thread call on an active plugin.
            unsafe {
                if let Some(f) = (*self.inst.plugin).reset {
                    f(self.inst.plugin);
                }
            }
        }
        self.held = [false; 128];
        self.notes = NoteState::new();
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        if !self.active || n == 0 {
            return;
        }
        self.inst.mark_audio();
        if !self.processing {
            // SAFETY: audio thread, active.
            let ok = unsafe {
                (*self.inst.plugin)
                    .start_processing
                    .is_none_or(|s| s(self.inst.plugin))
            };
            if !ok {
                self.stats.errors += 1;
                return;
            }
            self.processing = true;
        }
        let mut at = 0usize;
        let mut next = 0usize;
        let mut first = true;
        while at < n {
            let len = (n - at).min(self.max_block);
            self.events.clear();
            if first {
                for k in 0..self.pending.len() {
                    let (id, v) = self.pending[k];
                    self.param_ev(0, id, v);
                }
                self.pending.clear();
                first = false;
            }
            while next < events.len() && (events[next].offset as usize) < at + len {
                let t = (events[next].offset as usize).saturating_sub(at) as u32;
                let ev = events[next].ev;
                self.translate(t, &ev);
                next += 1;
            }
            // SAFETY: buffers sized in prepare for max_block; len fits.
            unsafe { self.run(ctx, at, len, left, right) };
            at += len;
        }
        // Events past the block's end break the node rules; they are
        // dropped and counted.
        if next < events.len() {
            self.stats.dropped += (events.len() - next) as u64;
        }
    }

    fn tail_frames(&self) -> u32 {
        if self.inst.tail.is_null() || !self.active {
            return 0;
        }
        // SAFETY: tail.get is callable from the main or audio thread.
        unsafe { (*self.inst.tail).get.map_or(0, |g| g(self.inst.plugin)) }
    }

    fn latency(&self) -> u32 {
        self.latency
    }
}

impl Drop for ClapNode {
    fn drop(&mut self) {
        self.deactivate();
    }
}

impl fmt::Debug for ClapNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClapNode")
            .field("id", &self.info.id)
            .field("active", &self.active)
            .finish()
    }
}

// ------------------------------------------------------------------ control

/// The main-thread side of a plugin: state and the plugin's requests. Keep
/// it on the control thread; the node can be on the audio thread meanwhile.
pub struct ClapControl {
    inst: Arc<Instance>,
    map: Vec<clap_id>,
}

// SAFETY: see Instance; this handle only makes CLAP main-thread calls.
unsafe impl Send for ClapControl {}

struct WriteBuf(Vec<u8>);
struct ReadBuf<'a>(&'a [u8], usize);

unsafe extern "C" fn ostream_write(s: *const clap_ostream, buf: *const c_void, size: u64) -> i64 {
    let w = &mut *((*s).ctx as *mut WriteBuf);
    let n = size.min(1 << 30) as usize;
    w.0.extend_from_slice(std::slice::from_raw_parts(buf as *const u8, n));
    n as i64
}

unsafe extern "C" fn istream_read(s: *const clap_istream, buf: *mut c_void, size: u64) -> i64 {
    let r = &mut *((*s).ctx as *mut ReadBuf);
    let left = r.0.len() - r.1;
    let n = (size as usize).min(left);
    ptr::copy_nonoverlapping(r.0[r.1..].as_ptr(), buf as *mut u8, n);
    r.1 += n;
    n as i64
}

impl ClapControl {
    /// The plugin's state as bytes, for a project file. None if it has no
    /// state or refused.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        let st = self.inst.state;
        if st.is_null() {
            return None;
        }
        let mut w = WriteBuf(Vec::new());
        let os = clap_ostream {
            ctx: &mut w as *mut WriteBuf as *mut c_void,
            write: Some(ostream_write),
        };
        // SAFETY: main-thread call; the stream lives across it.
        let ok = unsafe { (*st).save.is_some_and(|s| s(self.inst.plugin, &os)) };
        ok.then_some(w.0)
    }

    /// Loads state saved by [`ClapControl::save_state`]; the node's values
    /// follow.
    pub fn load_state(&self, bytes: &[u8]) -> bool {
        let st = self.inst.state;
        if st.is_null() {
            return false;
        }
        let mut r = ReadBuf(bytes, 0);
        let is = clap_istream {
            ctx: &mut r as *mut ReadBuf as *mut c_void,
            read: Some(istream_read),
        };
        // SAFETY: main-thread call; the stream lives across it.
        let ok = unsafe { (*st).load.is_some_and(|l| l(self.inst.plugin, &is)) };
        if ok {
            let map: Vec<PMap> = self
                .map
                .iter()
                .map(|&id| PMap {
                    id,
                    cookie: ptr::null_mut(),
                })
                .collect();
            self.inst.refresh_values(&map);
        }
        ok
    }

    /// How the plugin shows `value` of parameter `index` ("Saw", "-6 dB").
    pub fn param_text(&self, index: u16, value: f32) -> Option<String> {
        let id = *self.map.get(usize::from(index))?;
        // SAFETY: main-thread call.
        unsafe { self.inst.text(id, f64::from(value)) }
    }

    /// Answers the plugin's requests: runs its `on_main_thread` if it asked
    /// for a callback. Returns the request bits that were pending (see
    /// [`request`]); `RESTART`, `PARAMS_RESCAN` and `PORTS_RESCAN` mean the
    /// host should make the node again from saved state.
    pub fn idle(&self) -> u32 {
        let bits = self.inst.host.requests.swap(0, Ordering::AcqRel);
        if bits & request::CALLBACK != 0 {
            // SAFETY: main-thread call on a live plugin.
            unsafe {
                if let Some(f) = (*self.inst.plugin).on_main_thread {
                    f(self.inst.plugin);
                }
            }
        }
        bits
    }
}
