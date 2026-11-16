//! The host side of the VST 2.4 binary interface, as MPC OS uses it.
//!
//! [`Library::open`] loads a plugin's library; [`Library::create`] makes an
//! instance and gives an [`MpcNode`] (the audio side, a
//! [`mpfree_node::Node`]) and an [`MpcControl`] (state, value text,
//! programs: the control side). Both hold the instance; it is closed when
//! the last of them goes.
//!
//! The calls follow the order the inMusic SDK's `docs/host-contract.md`
//! gives for the MPC's host, so a plugin sees what it sees on an MPC:
//! sample rate and block size, `open`, program 0, names; on prepare,
//! sample rate and block size again, resume and start; MIDI before every
//! instrument block (even an empty list); `processReplacing` in place
//! (the same buffers in and out) for an effect; suspend and `close` at the
//! end. The structs come from `mpfree_mpcplug::abi`, MPFree's clean-room
//! description of the interface; no Steinberg header is used.
//!
//! What this host answers when a plugin asks (`audioMaster` calls): the
//! version (2400), sample rate, block size, the transport (`getTime`, every
//! block, 4/4 assumed: MPFree has no time signature yet), its own folder
//! (`getDirectory`), and yes to the capabilities the MPC says yes to. MIDI a
//! plugin sends is counted and dropped, as the MPC drops it. It answers
//! nothing private to Akai's app: a plugin that needs that isn't supported.

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use mpfree_mpcplug::abi::*;
use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed, Unit};

use crate::folder::Folder;

/// MIDI events passed to a plugin per block at most; more are dropped and
/// counted.
pub const MAX_EVENTS: usize = 512;
/// Audio channels a plugin may declare on either side.
pub const MAX_CHANNELS: usize = 32;
/// Parameters read from a plugin at most.
pub const MAX_PARAMS: usize = 4096;
/// The smallest block size announced to a plugin (the MPC announces at
/// least 128).
pub const MIN_ANNOUNCED_BLOCK: usize = 128;
/// Bytes given to a plugin for each string it writes. The interface says 8
/// to 64; plugins write more, so the host gives plenty.
const STR_BYTES: usize = 256;
/// The vendor and product this host reports.
pub const HOST_NAME: &str = "MPFree";

type MainProc = unsafe extern "C" fn(HostCallback) -> *mut AEffect;

/// A loaded plugin library. The library is never unloaded: plugins that
/// keep threads or static state are safe, and the plugin process ends
/// soon after anyway.
pub struct Library {
    pub folder: Folder,
    main: MainProc,
}

// SAFETY: the library's entry point may be called from any thread.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

impl Library {
    /// Reads the plugin at `path` (a plugin folder or a `.so` file; see
    /// [`Folder`]) and loads its library.
    pub fn open(path: &Path) -> Result<Library, String> {
        let folder = Folder::open(path)?;
        let c = CString::new(folder.library.as_os_str().as_encoded_bytes())
            .map_err(|_| "the library's path has a NUL in it".to_string())?;
        // SAFETY: a valid C string; the handle is kept for the process's
        // life (never closed).
        let handle = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err(format!("{}: {}", folder.library.display(), dl_error()));
        }
        // SAFETY: a valid handle and symbol name.
        let sym = unsafe { libc::dlsym(handle, c"VSTPluginMain".as_ptr()) };
        let sym = if sym.is_null() {
            // The old name some VST 2 plugins export instead.
            // SAFETY: as above.
            unsafe { libc::dlsym(handle, c"main".as_ptr()) }
        } else {
            sym
        };
        if sym.is_null() {
            return Err(format!(
                "{} doesn't export VSTPluginMain: not an MPC plugin",
                folder.library.display()
            ));
        }
        // SAFETY: VSTPluginMain has this signature by the interface.
        let main = unsafe { std::mem::transmute::<*mut c_void, MainProc>(sym) };
        Ok(Library { folder, main })
    }

    /// Makes an instance: `VSTPluginMain`, the load sequence, names. The
    /// node isn't prepared yet.
    pub fn create(&self) -> Result<(MpcNode, MpcControl), String> {
        let host = Box::new(HostState::new(&self.folder.dir));
        let hp: *const HostState = &*host;
        LOADING.with(|l| l.set(hp));
        // SAFETY: the entry point by the interface, with our callback.
        let effect = unsafe { (self.main)(host_callback) };
        LOADING.with(|l| l.set(std::ptr::null()));
        if effect.is_null() {
            return Err("the plugin refused to start (VSTPluginMain gave nothing)".into());
        }
        // SAFETY: the plugin gave us its AEffect; we own `user`.
        unsafe {
            if (*effect).magic != EFFECT_MAGIC {
                return Err("not a VST 2 plugin (wrong magic number)".into());
            }
            (*effect).user = hp as *mut c_void;
        }
        let inst = Arc::new(Instance {
            effect,
            host,
            closed: AtomicBool::new(false),
        });
        // SAFETY: a live effect; the load sequence of the host contract.
        unsafe { inst.load() };
        let e = inst.fields();
        let (num_in, num_out) = (e.num_inputs.max(0) as usize, e.num_outputs.max(0) as usize);
        if num_out == 0 {
            return Err("the plugin has no audio outputs".into());
        }
        if num_in > MAX_CHANNELS || num_out > MAX_CHANNELS {
            return Err(format!(
                "the plugin wants {num_in} inputs and {num_out} outputs (at most {MAX_CHANNELS})"
            ));
        }
        let synth = e.flags & FLAG_IS_SYNTH != 0
            || inst.dispatch(EFF_GET_PLUG_CATEGORY, 0, 0, std::ptr::null_mut(), 0.0)
                == PLUG_CATEG_SYNTH;
        let kind = if synth || self.folder.kind == Some(Kind::Instrument) {
            Kind::Instrument
        } else {
            Kind::Effect
        };
        if kind == Kind::Effect && num_in > 2 {
            return Err(format!(
                "the effect wants {num_in} inputs; a track gives it 2"
            ));
        }
        let name = non_empty(inst.string(EFF_GET_EFFECT_NAME, 0))
            .or_else(|| non_empty(self.folder.name.clone()))
            .or_else(|| non_empty(inst.string(EFF_GET_PRODUCT_STRING, 0)))
            .or_else(|| {
                self.folder
                    .library
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "Plugin".into());
        let vendor = non_empty(inst.string(EFF_GET_VENDOR_STRING, 0))
            .or_else(|| non_empty(self.folder.manufacturer.clone()))
            .unwrap_or_default();
        let plugin_id = plugin_id(e.unique_id, &name);
        let nparams = (e.num_params.max(0) as usize).min(MAX_PARAMS);
        let specs: Vec<(String, f32)> = (0..nparams)
            .map(|i| {
                let n = non_empty(inst.string(EFF_GET_PARAM_NAME, i as i32))
                    .unwrap_or_else(|| format!("Param {}", i + 1));
                (n, inst.get_param(i).clamp(0.0, 1.0))
            })
            .collect();
        let info = intern_info(&format!("mpc:{plugin_id}"), &name, kind);
        let params = intern_params(&plugin_id, &specs);
        let num_programs = e.num_programs.max(0) as usize;
        let chunks = e.flags & FLAG_PROGRAM_CHUNKS != 0;
        let node = MpcNode {
            inst: Arc::clone(&inst),
            info,
            params,
            kind,
            num_in,
            num_out,
            scratch: Vec::new(),
            ins: Vec::with_capacity(num_in.max(1)),
            outs: Vec::with_capacity(num_out),
            events: Box::new(EventList {
                num_events: 0,
                reserved: 0,
                events: [std::ptr::null_mut(); MAX_EVENTS],
            }),
            midi: (0..MAX_EVENTS).map(|_| blank_midi()).collect(),
            max_block: 0,
            active: false,
            was_playing: false,
            all_off: false,
            stats: MpcStats::default(),
        };
        let control = MpcControl {
            inst,
            plugin_id,
            vendor,
            num_programs,
            chunks,
            nparams,
        };
        Ok((node, control))
    }
}

fn non_empty(s: String) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn dl_error() -> String {
    // SAFETY: dlerror gives a C string or null.
    let e = unsafe { libc::dlerror() };
    if e.is_null() {
        "could not load".into()
    } else {
        // SAFETY: non-null C string from dlerror.
        unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned()
    }
}

/// A plugin's id in MPFree: its four-character unique id (as letters when
/// they are letters and digits, else eight hex digits), a dot, and its
/// name in lowercase letters, digits and dashes. Projects keep it.
pub fn plugin_id(uid: i32, name: &str) -> String {
    let b = (uid as u32).to_be_bytes();
    let code = if b.iter().all(u8::is_ascii_alphanumeric) {
        b.iter().map(|&c| c as char).collect()
    } else {
        format!("{:08x}", uid as u32)
    };
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("plugin");
    }
    format!("{code}.{slug}")
}

// ------------------------------------------------------------ host state

/// What the host keeps for one instance, reached from the plugin's calls
/// through `AEffect::user`.
struct HostState {
    /// Written by the audio thread before each block; read by the plugin
    /// during it (`getTime`).
    time: UnsafeCell<VstTimeInfo>,
    rate: AtomicU32,
    block: AtomicU32,
    dir: CString,
    midi_out: AtomicU64,
    display_changed: AtomicBool,
}

// SAFETY: `time` is written only by the audio thread between blocks and
// read by the plugin on that thread; the rest is atomics.
unsafe impl Sync for HostState {}

impl HostState {
    fn new(dir: &Path) -> HostState {
        let time = VstTimeInfo {
            sample_rate: 44_100.0,
            tempo: 120.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            flags: PPQ_POS_VALID | TEMPO_VALID | BARS_VALID | TIME_SIG_VALID,
            ..VstTimeInfo::default()
        };
        HostState {
            time: UnsafeCell::new(time),
            rate: AtomicU32::new(44_100),
            block: AtomicU32::new(MIN_ANNOUNCED_BLOCK as u32),
            dir: CString::new(dir.as_os_str().as_encoded_bytes()).unwrap_or_default(),
            midi_out: AtomicU64::new(0),
            display_changed: AtomicBool::new(false),
        }
    }
}

thread_local! {
    /// The instance being made: a plugin may call the host from inside
    /// VSTPluginMain, before `user` is set.
    static LOADING: std::cell::Cell<*const HostState> = const { std::cell::Cell::new(std::ptr::null()) };
}

/// Fills a plugin's string buffer (the interface's 64-byte limit for
/// vendor and product).
unsafe fn put_str(ptr: *mut c_void, s: &str) {
    if ptr.is_null() {
        return;
    }
    let n = s.len().min(MAX_VENDOR_STR_LEN);
    std::ptr::copy_nonoverlapping(s.as_ptr(), ptr as *mut u8, n);
    *(ptr as *mut u8).add(n) = 0;
}

unsafe extern "C" fn host_callback(
    effect: *mut AEffect,
    opcode: VstInt32,
    _index: VstInt32,
    _value: VstIntPtr,
    ptr: *mut c_void,
    _opt: f32,
) -> VstIntPtr {
    let host: *const HostState = if !effect.is_null() && !(*effect).user.is_null() {
        (*effect).user as *const HostState
    } else {
        LOADING.with(|l| l.get())
    };
    let host = host.as_ref();
    match opcode {
        AUDIO_MASTER_VERSION => VST_VERSION,
        AUDIO_MASTER_CURRENT_ID => effect.as_ref().map_or(0, |e| e.unique_id as VstIntPtr),
        AUDIO_MASTER_GET_TIME => host.map_or(0, |h| h.time.get() as VstIntPtr),
        AUDIO_MASTER_PROCESS_EVENTS => {
            if let Some(h) = host {
                h.midi_out.fetch_add(1, Ordering::Relaxed);
            }
            1
        }
        AUDIO_MASTER_SIZE_WINDOW => 1,
        AUDIO_MASTER_GET_SAMPLE_RATE => {
            host.map_or(44_100, |h| h.rate.load(Ordering::Relaxed) as VstIntPtr)
        }
        AUDIO_MASTER_GET_BLOCK_SIZE => host.map_or(MIN_ANNOUNCED_BLOCK as VstIntPtr, |h| {
            h.block.load(Ordering::Relaxed) as VstIntPtr
        }),
        AUDIO_MASTER_GET_VENDOR_STRING | AUDIO_MASTER_GET_PRODUCT_STRING => {
            put_str(ptr, HOST_NAME);
            1
        }
        AUDIO_MASTER_GET_VENDOR_VERSION => 1,
        AUDIO_MASTER_CAN_DO => {
            if ptr.is_null() {
                return 0;
            }
            let ask = CStr::from_ptr(ptr as *const libc::c_char).to_bytes();
            match ask {
                b"sendVstEvents"
                | b"sendVstMidiEvent"
                | b"receiveVstEvents"
                | b"receiveVstMidiEvent"
                | b"sendVstTimeInfo"
                | b"sizeWindow"
                | b"shellCategory" => 1,
                _ => 0,
            }
        }
        // English.
        AUDIO_MASTER_GET_LANGUAGE => 1,
        AUDIO_MASTER_GET_DIRECTORY => host.map_or(0, |h| h.dir.as_ptr() as VstIntPtr),
        AUDIO_MASTER_UPDATE_DISPLAY => {
            if let Some(h) = host {
                h.display_changed.store(true, Ordering::Relaxed);
            }
            1
        }
        AUDIO_MASTER_BEGIN_EDIT | AUDIO_MASTER_END_EDIT => 1,
        // Automate, idle, IO changed, process level and the rest: nothing
        // to do, "don't know".
        AUDIO_MASTER_AUTOMATE
        | AUDIO_MASTER_IDLE
        | AUDIO_MASTER_IO_CHANGED
        | AUDIO_MASTER_GET_CURRENT_PROCESS_LEVEL => 0,
        _ => 0,
    }
}

// --------------------------------------------------------------- instance

struct Instance {
    effect: *mut AEffect,
    host: Box<HostState>,
    closed: AtomicBool,
}

// SAFETY: the host contract lets the host call the plugin from the audio
// thread and the UI thread at once; MpcNode keeps processing to one
// thread.
unsafe impl Send for Instance {}
unsafe impl Sync for Instance {}

impl Instance {
    fn fields(&self) -> &AEffect {
        // SAFETY: valid until close, which only Drop does.
        unsafe { &*self.effect }
    }

    fn dispatch(
        &self,
        op: VstInt32,
        index: VstInt32,
        value: VstIntPtr,
        ptr: *mut c_void,
        opt: f32,
    ) -> VstIntPtr {
        match self.fields().dispatcher {
            // SAFETY: the plugin's own dispatcher on its own effect.
            Some(d) => unsafe { d(self.effect, op, index, value, ptr, opt) },
            None => 0,
        }
    }

    fn string(&self, op: VstInt32, index: i32) -> String {
        let mut buf = [0u8; STR_BYTES];
        self.dispatch(op, index, 0, buf.as_mut_ptr() as *mut c_void, 0.0);
        buf[STR_BYTES - 1] = 0;
        CStr::from_bytes_until_nul(&buf)
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn get_param(&self, i: usize) -> f32 {
        match self.fields().get_parameter {
            // SAFETY: index below num_params.
            Some(g) => unsafe { g(self.effect, i as i32) },
            None => 0.0,
        }
    }

    fn set_param(&self, i: usize, v: f32) {
        if let Some(s) = self.fields().set_parameter {
            // SAFETY: index below num_params.
            unsafe { s(self.effect, i as i32, v) }
        }
    }

    fn set_format(&self, rate: u32, block: usize) {
        self.host.rate.store(rate, Ordering::Relaxed);
        self.host.block.store(block as u32, Ordering::Relaxed);
        self.dispatch(EFF_SET_SAMPLE_RATE, 0, 0, std::ptr::null_mut(), rate as f32);
        self.dispatch(
            EFF_SET_BLOCK_SIZE,
            0,
            block as VstIntPtr,
            std::ptr::null_mut(),
            0.0,
        );
    }

    /// Steps 2 to 5 of the host contract's load sequence (one `open`).
    unsafe fn load(&self) {
        self.set_format(44_100, MIN_ANNOUNCED_BLOCK);
        self.dispatch(EFF_OPEN, 0, 0, std::ptr::null_mut(), 0.0);
        let n = (self.fields().num_params.max(0) as usize).min(MAX_PARAMS);
        for i in 0..n {
            self.dispatch(EFF_CAN_BE_AUTOMATED, i as i32, 0, std::ptr::null_mut(), 0.0);
        }
        if self.fields().num_programs > 0 {
            self.dispatch(EFF_SET_PROGRAM, 0, 0, std::ptr::null_mut(), 0.0);
        }
    }

    fn suspend(&self) {
        self.dispatch(EFF_STOP_PROCESS, 0, 0, std::ptr::null_mut(), 0.0);
        self.dispatch(EFF_MAINS_CHANGED, 0, 0, std::ptr::null_mut(), 0.0);
    }

    fn resume(&self) {
        self.dispatch(EFF_MAINS_CHANGED, 0, 1, std::ptr::null_mut(), 0.0);
        self.dispatch(EFF_START_PROCESS, 0, 0, std::ptr::null_mut(), 0.0);
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.suspend();
            self.dispatch(EFF_CLOSE, 0, 0, std::ptr::null_mut(), 0.0);
        }
    }
}

// ------------------------------------------------------------------ node

/// `VstEvents` with room for [`MAX_EVENTS`].
#[repr(C)]
struct EventList {
    num_events: VstInt32,
    reserved: VstIntPtr,
    events: [*mut VstEvent; MAX_EVENTS],
}

fn blank_midi() -> VstMidiEvent {
    VstMidiEvent {
        kind: VST_MIDI_TYPE,
        byte_size: std::mem::size_of::<VstMidiEvent>() as i32,
        delta_frames: 0,
        flags: 0,
        note_length: 0,
        note_offset: 0,
        midi_data: [0; 4],
        detune: 0,
        note_off_velocity: 0,
        reserved1: 0,
        reserved2: 0,
    }
}

/// A node's counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MpcStats {
    pub blocks: u64,
    /// MIDI events left out because a block had more than [`MAX_EVENTS`].
    pub events_dropped: u64,
    /// MIDI events the plugin sent (the host drops them, as the MPC does).
    pub midi_out: u64,
}

/// An MPC plugin instance as a node. See the module docs.
pub struct MpcNode {
    inst: Arc<Instance>,
    info: &'static NodeInfo,
    params: &'static [ParamInfo],
    kind: Kind,
    num_in: usize,
    num_out: usize,
    /// Buffers for an instrument's inputs and outputs and an effect's
    /// outputs past two, `max_block` long; made in prepare.
    scratch: Vec<Vec<f32>>,
    ins: Vec<*mut f32>,
    outs: Vec<*mut f32>,
    events: Box<EventList>,
    midi: Vec<VstMidiEvent>,
    max_block: usize,
    active: bool,
    was_playing: bool,
    /// Notes off and sound off go to an instrument with the next block.
    all_off: bool,
    stats: MpcStats,
}

// SAFETY: the raw pointers point into this node's own buffers or the
// instance, which is Send + Sync.
unsafe impl Send for MpcNode {}

impl MpcNode {
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Prepared and running.
    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn inputs(&self) -> usize {
        self.num_in
    }

    pub fn outputs(&self) -> usize {
        self.num_out
    }

    pub fn stats(&self) -> MpcStats {
        MpcStats {
            midi_out: self.inst.host.midi_out.load(Ordering::Relaxed),
            ..self.stats
        }
    }

    fn put_time(&mut self, ctx: &Ctx) {
        let playing = ctx.playing;
        let mut flags = PPQ_POS_VALID | TEMPO_VALID | BARS_VALID | TIME_SIG_VALID;
        if playing {
            flags |= TRANSPORT_PLAYING;
        }
        if playing != self.was_playing {
            flags |= TRANSPORT_CHANGED;
        }
        self.was_playing = playing;
        let beats = if ctx.beats.is_finite() {
            ctx.beats
        } else {
            0.0
        };
        let t = VstTimeInfo {
            sample_pos: ctx.frame as f64,
            sample_rate: f64::from(ctx.rate),
            nano_seconds: 0.0,
            ppq_pos: beats,
            tempo: ctx.bpm,
            bar_start_pos: (beats / 4.0).floor() * 4.0,
            cycle_start_pos: 0.0,
            cycle_end_pos: 0.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            smpte_offset: 0,
            smpte_frame_rate: 0,
            samples_to_next_clock: 0,
            flags,
        };
        // SAFETY: only this thread writes it, between blocks.
        unsafe { *self.inst.host.time.get() = t };
    }

    fn push_midi(&mut self, k: &mut usize, offset: u32, bytes: [u8; 3]) {
        if *k == MAX_EVENTS {
            self.stats.events_dropped += 1;
            return;
        }
        let m = &mut self.midi[*k];
        *m = blank_midi();
        m.delta_frames = offset as i32;
        m.midi_data = [bytes[0], bytes[1], bytes[2], 0];
        *k += 1;
    }

    /// Turns the block's events into MIDI (parameters are set straight
    /// away) and hands the list to the plugin.
    fn send_events(&mut self, events: &[Timed]) {
        let mut k = 0usize;
        if std::mem::take(&mut self.all_off) {
            self.push_midi(&mut k, 0, [0xb0, 123, 0]);
            self.push_midi(&mut k, 0, [0xb0, 120, 0]);
        }
        for t in events {
            let b = |v: u8| v.min(127);
            let msg = match t.ev {
                NodeEvent::NoteOn { note, velocity } => [0x90, b(note), b(velocity).max(1)],
                NodeEvent::NoteOff { note } => [0x80, b(note), 0],
                NodeEvent::Pressure { note, value } => [0xa0, b(note), b(value)],
                NodeEvent::Bend { value } => {
                    let v = (8192.0 + value.clamp(-1.0, 1.0) * 8191.0).round() as u16;
                    [0xe0, (v & 0x7f) as u8, (v >> 7) as u8]
                }
                NodeEvent::AllOff => {
                    self.push_midi(&mut k, t.offset, [0xb0, 123, 0]);
                    [0xb0, 120, 0]
                }
                NodeEvent::Param { id, value } => {
                    self.set_param(id, value);
                    continue;
                }
                _ => continue,
            };
            self.push_midi(&mut k, t.offset, msg);
        }
        for i in 0..k {
            self.events.events[i] = &mut self.midi[i] as *mut VstMidiEvent as *mut VstEvent;
        }
        self.events.num_events = k as i32;
        let list = &mut *self.events as *mut EventList as *mut c_void;
        self.inst.dispatch(EFF_PROCESS_EVENTS, 0, 0, list, 0.0);
    }
}

impl Node for MpcNode {
    fn info(&self) -> &'static NodeInfo {
        self.info
    }

    fn params(&self) -> &'static [ParamInfo] {
        self.params
    }

    fn set_param(&mut self, id: u16, value: f32) {
        let i = usize::from(id);
        if i < self.params.len() && value.is_finite() {
            self.inst.set_param(i, value.clamp(0.0, 1.0));
        }
    }

    fn param(&self, id: u16) -> f32 {
        let i = usize::from(id);
        if i < self.params.len() {
            self.inst.get_param(i)
        } else {
            f32::NAN
        }
    }

    fn prepare(&mut self, rate: u32, max_block: usize) {
        if self.active {
            self.inst.suspend();
        }
        let block = max_block.max(MIN_ANNOUNCED_BLOCK);
        self.max_block = max_block;
        let n = if self.kind == Kind::Effect && self.num_in > 0 {
            self.num_out.saturating_sub(2)
        } else {
            self.num_in + self.num_out
        };
        self.scratch = (0..n).map(|_| vec![0.0; block]).collect();
        self.inst.set_format(rate, block);
        self.inst.resume();
        self.active = true;
    }

    fn reset(&mut self) {
        if self.active {
            self.inst.suspend();
            self.inst.resume();
        }
        if self.kind == Kind::Instrument {
            self.all_off = true;
        }
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len()).min(self.max_block);
        let Some(proc_) = self.inst.fields().process_replacing else {
            return;
        };
        if !self.active || n == 0 {
            return;
        }
        self.put_time(ctx);
        let effect_path = self.kind == Kind::Effect && self.num_in > 0;
        if effect_path {
            // Effects get no MIDI; parameters still apply.
            for t in events {
                if let NodeEvent::Param { id, value } = t.ev {
                    self.set_param(id, value);
                }
            }
        } else {
            self.send_events(events);
        }
        self.ins.clear();
        self.outs.clear();
        let (l, r) = (left.as_mut_ptr(), right.as_mut_ptr());
        if effect_path {
            // In place, as the MPC does it.
            self.ins.push(l);
            if self.num_in > 1 {
                self.ins.push(r);
            }
            self.outs.push(l);
            if self.num_out > 1 {
                self.outs.push(r);
            }
            for s in self.scratch.iter_mut() {
                self.outs.push(s.as_mut_ptr());
            }
        } else {
            for (i, s) in self.scratch.iter_mut().enumerate() {
                if i < self.num_in {
                    s[..n].fill(0.0);
                    self.ins.push(s.as_mut_ptr());
                } else {
                    self.outs.push(s.as_mut_ptr());
                }
            }
        }
        // SAFETY: every pointer has at least n frames; the lists are as
        // long as the plugin's declared channel counts.
        unsafe {
            proc_(
                self.inst.effect,
                self.ins.as_mut_ptr(),
                self.outs.as_mut_ptr(),
                n as i32,
            )
        };
        if effect_path {
            if self.num_out == 1 {
                right[..n].copy_from_slice(&left[..n]);
            }
        } else {
            let o = &self.scratch[self.num_in..];
            let (a, b) = (&o[0][..n], &o[if o.len() > 1 { 1 } else { 0 }][..n]);
            for (d, s) in left[..n].iter_mut().zip(a) {
                *d += *s;
            }
            for (d, s) in right[..n].iter_mut().zip(b) {
                *d += *s;
            }
        }
        self.stats.blocks += 1;
    }

    fn tail_frames(&self) -> u32 {
        match self
            .inst
            .dispatch(EFF_GET_TAIL_SIZE, 0, 0, std::ptr::null_mut(), 0.0)
        {
            // 0: the plugin doesn't say; 1: no tail.
            t if t > 1 => t.min(u32::MAX as VstIntPtr) as u32,
            _ => 0,
        }
    }

    fn latency(&self) -> u32 {
        self.inst.fields().initial_delay.max(0) as u32
    }
}

// --------------------------------------------------------------- control

/// The control side of an instance: state, value text, programs. The host
/// contract lets these run beside a block on the audio thread.
pub struct MpcControl {
    inst: Arc<Instance>,
    plugin_id: String,
    vendor: String,
    num_programs: usize,
    chunks: bool,
    nparams: usize,
}

const STATE_MAGIC: &[u8; 4] = b"MPCS";
const STATE_CHUNK: u8 = 0;
const STATE_PARAMS: u8 = 1;
/// The most state accepted from a plugin or a project (the MPC takes up
/// to 128 MB; MPFree keeps states in project files).
pub const MAX_STATE: usize = 64 << 20;

impl MpcControl {
    /// [`plugin_id`] for this instance.
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn vendor(&self) -> &str {
        &self.vendor
    }

    /// Whether the plugin asked for its names to be read again since the
    /// last call.
    pub fn take_display_changed(&self) -> bool {
        self.inst
            .host
            .display_changed
            .swap(false, Ordering::Relaxed)
    }

    /// The state for a project: the program number and either the
    /// plugin's own chunk (if it keeps one) or every parameter's value, as
    /// the MPC saves them.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        let program =
            self.inst
                .dispatch(EFF_GET_PROGRAM, 0, 0, std::ptr::null_mut(), 0.0) as i32;
        let mut out = Vec::with_capacity(16);
        out.extend_from_slice(STATE_MAGIC);
        out.push(1);
        out.extend_from_slice(&program.to_le_bytes());
        if self.chunks {
            let mut data: *mut c_void = std::ptr::null_mut();
            let len = self.inst.dispatch(
                EFF_GET_CHUNK,
                0,
                0,
                &mut data as *mut *mut c_void as *mut c_void,
                0.0,
            );
            if len <= 0 || data.is_null() || len as usize > MAX_STATE {
                return None;
            }
            out.push(STATE_CHUNK);
            out.extend_from_slice(&(len as u32).to_le_bytes());
            // SAFETY: the plugin gave `len` bytes at `data`, valid until
            // its next call.
            out.extend_from_slice(unsafe {
                std::slice::from_raw_parts(data as *const u8, len as usize)
            });
        } else {
            out.push(STATE_PARAMS);
            out.extend_from_slice(&(self.nparams as u32).to_le_bytes());
            for i in 0..self.nparams {
                out.extend_from_slice(&self.inst.get_param(i).to_le_bytes());
            }
        }
        Some(out)
    }

    /// Loads a state [`MpcControl::save_state`] made. False if it isn't
    /// one.
    pub fn load_state(&self, b: &[u8]) -> bool {
        let Some(rest) = b.strip_prefix(STATE_MAGIC) else {
            return false;
        };
        if rest.len() < 10 || rest[0] != 1 {
            return false;
        }
        let program = i32::from_le_bytes([rest[1], rest[2], rest[3], rest[4]]);
        let mode = rest[5];
        let n = u32::from_le_bytes([rest[6], rest[7], rest[8], rest[9]]) as usize;
        let body = &rest[10..];
        if program >= 0 && (program as usize) < self.num_programs {
            self.inst.dispatch(
                EFF_SET_PROGRAM,
                0,
                program as VstIntPtr,
                std::ptr::null_mut(),
                0.0,
            );
        }
        match mode {
            STATE_CHUNK if self.chunks && body.len() == n && n <= MAX_STATE => {
                let mut copy = body.to_vec();
                self.inst.dispatch(
                    EFF_SET_CHUNK,
                    0,
                    n as VstIntPtr,
                    copy.as_mut_ptr() as *mut c_void,
                    0.0,
                );
                true
            }
            STATE_PARAMS if body.len() == n * 4 => {
                for (i, v) in body.chunks_exact(4).enumerate().take(self.nparams) {
                    let v = f32::from_le_bytes([v[0], v[1], v[2], v[3]]);
                    if v.is_finite() {
                        self.inst.set_param(i, v.clamp(0.0, 1.0));
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// The plugin's text for parameter `index` at `value`. A VST 2 plugin
    /// only shows its current value, so any other value has no text.
    pub fn param_text(&self, index: u16, value: f32) -> Option<String> {
        let i = usize::from(index);
        if i >= self.nparams || (self.inst.get_param(i) - value).abs() > 1e-6 {
            return None;
        }
        non_empty(self.inst.string(EFF_GET_PARAM_DISPLAY, i as i32))
    }

    /// The plugin's built-in programs, by number (the MPC's preset list).
    pub fn programs(&self) -> Vec<String> {
        (0..self.num_programs.min(1024))
            .map(|i| {
                let mut buf = [0u8; STR_BYTES];
                let ok = self.inst.dispatch(
                    EFF_GET_PROGRAM_NAME_INDEXED,
                    i as i32,
                    -1,
                    buf.as_mut_ptr() as *mut c_void,
                    0.0,
                );
                buf[STR_BYTES - 1] = 0;
                let name = CStr::from_bytes_until_nul(&buf)
                    .map(|c| c.to_string_lossy().trim().to_string())
                    .unwrap_or_default();
                if ok != 0 && !name.is_empty() {
                    name
                } else {
                    format!("Program {}", i + 1)
                }
            })
            .collect()
    }

    /// Selects built-in program `n`.
    pub fn load_program(&self, n: usize) -> Result<(), String> {
        if n >= self.num_programs {
            return Err(format!(
                "the plugin has {} programs, not {}",
                self.num_programs,
                n + 1
            ));
        }
        self.inst.dispatch(
            EFF_SET_PROGRAM,
            0,
            n as VstIntPtr,
            std::ptr::null_mut(),
            0.0,
        );
        Ok(())
    }
}

// --------------------------------------------------------------- interning

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn intern_info(node_id: &str, name: &str, kind: Kind) -> &'static NodeInfo {
    type Table = HashMap<(String, String, bool), &'static NodeInfo>;
    static T: OnceLock<Mutex<Table>> = OnceLock::new();
    let key = (
        node_id.to_string(),
        name.to_string(),
        kind == Kind::Instrument,
    );
    let mut t = T
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    t.entry(key).or_insert_with(|| {
        Box::leak(Box::new(NodeInfo {
            id: leak(node_id),
            name: leak(name),
            kind,
        }))
    })
}

fn intern_params(plugin_id: &str, specs: &[(String, f32)]) -> &'static [ParamInfo] {
    type Table = HashMap<(String, Vec<(String, u32)>), &'static [ParamInfo]>;
    static T: OnceLock<Mutex<Table>> = OnceLock::new();
    let key = (
        plugin_id.to_string(),
        specs
            .iter()
            .map(|(n, d)| (n.clone(), d.to_bits()))
            .collect(),
    );
    let mut t = T
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    t.entry(key).or_insert_with(|| {
        let v: Vec<ParamInfo> = specs
            .iter()
            .enumerate()
            .map(|(i, (n, d))| ParamInfo {
                id: i as u16,
                name: leak(n),
                group: "",
                min: 0.0,
                max: 1.0,
                default: *d,
                unit: Unit::None,
                curve: Curve::Linear,
                readonly: false,
            })
            .collect();
        Box::leak(v.into_boxed_slice())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_the_unique_id_and_the_name() {
        let fourcc = i32::from_be_bytes(*b"AwXn");
        assert_eq!(plugin_id(fourcc, "Density2"), "AwXn.density2");
        assert_eq!(
            plugin_id(0x0102_0304, "[SYN] Mono Voice"),
            "01020304.syn-mono-voice"
        );
        assert_eq!(plugin_id(-1, "  "), "ffffffff.plugin");
        assert_eq!(plugin_id(fourcc, "Émilie's -- Verb!"), "AwXn.milie-s-verb");
    }
}
