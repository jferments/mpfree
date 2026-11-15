//! An [`mpfree_node::Node`] as an MPC plugin.
//!
//! MPC OS 3.10.1 loads third-party instruments and effects: Linux shared
//! libraries that speak the VST 2.4 binary interface and export one symbol,
//! `VSTPluginMain`. This crate turns any MPFree node (an effect from
//! `mpfree-fx`, an instrument) into one. A plugin crate describes itself
//! with a [`Spec`] and calls [`export_mpc_plugin!`]; it is built as a
//! `cdylib`. See `examples/mpfree_delay.rs`.
//!
//! What the MPC host does is written down in the inMusic SDK's
//! `docs/host-contract.md`; the rules this wrapper follows from it:
//!
//! - **Threads.** `process` runs on the audio thread. Parameter writes and
//!   chunk loads can come from another thread while it runs, so every
//!   parameter lives in an atomic and is handed to the node at the start
//!   of the next block. The node itself is touched only by the audio
//!   thread, except in `effMainsChanged` (resume), which the host sends
//!   before processing starts.
//! - **In place.** The host passes the same buffers as input and output.
//!   The wrapper copies input to output only when they differ, then lets
//!   the node work in place.
//! - **Any block size.** The announced block size is a maximum; a longer
//!   call is cut into pieces no longer than it.
//! - **Parameter 0 is nudged** to 0 or 1 and back on every prepare. A
//!   parameter write only stores a value, so the nudge costs nothing.
//! - **State.** The chunk stores every parameter by its stable node id, as
//!   text, so a later version with more parameters loads old projects.
//! - **Strings.** At most [`abi::PARAM_STR_BYTES`] bytes (NUL included) go
//!   into a host string buffer, as in the SDK's own framework.
//!
//! The plugin runs inside Akai's app with no sandbox, so this code never
//! panics across the boundary on purpose: bad indexes and null pointers
//! are answered with 0.

pub mod abi;
pub mod skin;

pub use skin::{host_names, host_params, ui_json};

use std::cell::UnsafeCell;
use std::ffi::{c_void, CStr};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use abi::*;
use mpfree_node::{Ctx, Curve, Node, NodeEvent, ParamInfo, Timed, Unit};

/// What a plugin tells the MPC about itself. `name` and `vendor` must equal
/// the strings in the plugin folder's `plugin.json` and its folder name
/// (`"<vendor> - VST - <name>"`), and `uid` its `uid`.
pub struct Spec {
    pub name: &'static str,
    pub vendor: &'static str,
    /// Four-character code, first character most significant.
    pub uid: [u8; 4],
    /// Plugin version, such as 1000 for 1.0.0.0.
    pub version: i32,
    pub instrument: bool,
    /// Makes the node, at its defaults.
    pub make: fn() -> Box<dyn Node>,
}

impl Spec {
    pub fn uid(&self) -> i32 {
        i32::from_be_bytes(self.uid)
    }
}

/// The most events kept for one block; more are dropped.
const MAX_EVENTS: usize = 512;
/// The block size assumed until the host announces one.
const DEFAULT_BLOCK: usize = 512;

/// One plugin instance. `effect` is first so a `*mut AEffect` from the host
/// is also a `*mut Instance`; `effect.object` points here as well.
#[repr(C)]
struct Instance {
    effect: AEffect,
    spec: &'static Spec,
    host: Option<HostCallback>,
    /// The node's writable parameters, in host index order.
    params: Vec<&'static ParamInfo>,
    /// The names the host shows, unique within the plugin ([`host_names`]).
    names: Vec<String>,
    /// Each parameter's value (not the 0-1 position), as f32 bits.
    values: Vec<AtomicU32>,
    dirty: Vec<AtomicBool>,
    any_dirty: AtomicBool,
    rate: AtomicU32,
    max_block: AtomicU32,
    audio: UnsafeCell<Audio>,
    chunk: UnsafeCell<Vec<u8>>,
}

/// What only the audio thread touches (and resume, before processing).
struct Audio {
    node: Box<dyn Node>,
    events: Vec<Timed>,
    piece: Vec<Timed>,
    frame: u64,
    prepared: bool,
}

/// Makes a new instance and returns its `AEffect`. A plugin's
/// `VSTPluginMain` calls this (see [`export_mpc_plugin!`]).
///
/// # Safety
/// `host` must be the host's callback, valid for the instance's life.
pub unsafe fn plugin_main(spec: &'static Spec, host: Option<HostCallback>) -> *mut AEffect {
    let node = (spec.make)();
    let params = host_params(node.as_ref());
    let values = params
        .iter()
        .map(|p| AtomicU32::new(p.default.to_bits()))
        .collect();
    let dirty = params.iter().map(|_| AtomicBool::new(true)).collect();
    let latency = node.latency() as i32;
    let n = params.len() as i32;
    let inst = Box::new(Instance {
        effect: AEffect {
            magic: EFFECT_MAGIC,
            dispatcher: Some(dispatcher),
            process: None,
            set_parameter: Some(set_parameter),
            get_parameter: Some(get_parameter),
            num_programs: 1,
            num_params: n,
            num_inputs: if spec.instrument { 0 } else { 2 },
            num_outputs: 2,
            flags: FLAG_CAN_REPLACING
                | FLAG_PROGRAM_CHUNKS
                | if spec.instrument { FLAG_IS_SYNTH } else { 0 },
            resvd1: 0,
            resvd2: 0,
            initial_delay: latency,
            real_qualities: 0,
            off_qualities: 0,
            io_ratio: 1.0,
            object: std::ptr::null_mut(),
            user: std::ptr::null_mut(),
            unique_id: spec.uid(),
            version: spec.version,
            process_replacing: Some(process_replacing),
            process_double_replacing: None,
            future: [0; 56],
        },
        spec,
        host,
        names: host_names(&params),
        params,
        values,
        dirty,
        any_dirty: AtomicBool::new(true),
        rate: AtomicU32::new(44_100),
        max_block: AtomicU32::new(DEFAULT_BLOCK as u32),
        audio: UnsafeCell::new(Audio {
            node,
            events: Vec::with_capacity(MAX_EVENTS),
            piece: Vec::with_capacity(MAX_EVENTS),
            frame: 0,
            prepared: false,
        }),
        chunk: UnsafeCell::new(Vec::new()),
    });
    let raw = Box::into_raw(inst);
    (*raw).effect.object = raw as *mut c_void;
    raw as *mut AEffect
}

/// Exports `VSTPluginMain` for a [`Spec`]. Use once, in a `cdylib`.
#[macro_export]
macro_rules! export_mpc_plugin {
    ($spec:expr) => {
        #[no_mangle]
        pub unsafe extern "C" fn VSTPluginMain(
            host: Option<$crate::abi::HostCallback>,
        ) -> *mut $crate::abi::AEffect {
            static SPEC: $crate::Spec = $spec;
            $crate::plugin_main(&SPEC, host)
        }
    };
}

unsafe fn instance<'a>(effect: *mut AEffect) -> Option<&'a Instance> {
    if effect.is_null() || (*effect).object.is_null() {
        return None;
    }
    Some(&*((*effect).object as *const Instance))
}

/// Copies `s` into a host buffer of `cap` bytes, NUL-terminated.
unsafe fn put_str(ptr: *mut c_void, cap: usize, s: &str) {
    if ptr.is_null() || cap == 0 {
        return;
    }
    let bytes = s.as_bytes();
    let mut n = bytes.len().min(cap - 1);
    while n > 0 && !s.is_char_boundary(n) {
        n -= 1;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, n);
    *(ptr as *mut u8).add(n) = 0;
}

impl Instance {
    fn value(&self, i: usize) -> f32 {
        f32::from_bits(self.values[i].load(Ordering::Relaxed))
    }

    fn store(&self, i: usize, v: f32) {
        let v = self.params[i].clamp(v);
        self.values[i].store(v.to_bits(), Ordering::Relaxed);
        self.dirty[i].store(true, Ordering::Release);
        self.any_dirty.store(true, Ordering::Release);
    }

    /// The chunk: a version line, the node id, then `id=value` lines.
    fn save(&self) -> Vec<u8> {
        let mut s = String::new();
        let _ = writeln!(s, "mpfree-mpcplug 1");
        // SAFETY: info() is read-only and static.
        let id = unsafe { (*self.audio.get()).node.info().id };
        let _ = writeln!(s, "{id}");
        for (i, p) in self.params.iter().enumerate() {
            let _ = writeln!(s, "{}={}", p.id, self.value(i));
        }
        s.into_bytes()
    }

    /// Loads a chunk from [`Instance::save`]. Unknown ids are skipped; a
    /// parameter missing from the chunk goes back to its default.
    fn load(&self, data: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(data) else {
            return false;
        };
        let mut lines = text.lines();
        if lines.next() != Some("mpfree-mpcplug 1") {
            return false;
        }
        let _node_id = lines.next();
        let mut seen = vec![false; self.params.len()];
        for line in lines {
            let Some((id, v)) = line.split_once('=') else {
                continue;
            };
            let (Ok(id), Ok(v)) = (id.trim().parse::<u16>(), v.trim().parse::<f32>()) else {
                continue;
            };
            if let Some(i) = self.params.iter().position(|p| p.id == id) {
                self.store(i, v);
                seen[i] = true;
            }
        }
        for (i, s) in seen.iter().enumerate() {
            if !s {
                self.store(i, self.params[i].default);
            }
        }
        true
    }
}

/// A value as the MPC screen shows it (about 8 characters).
pub fn value_text(p: &ParamInfo, v: f32) -> String {
    match p.curve {
        Curve::Choice(names) => names.get(v.round() as usize).copied().unwrap_or("?").into(),
        Curve::Toggle => if v >= 0.5 { "On" } else { "Off" }.into(),
        Curve::Stepped => format!("{}", v.round() as i64),
        _ => match p.unit {
            Unit::Hz if v >= 1000.0 => format!("{:.2} kHz", v / 1000.0),
            Unit::Hz => format!("{:.0} Hz", v),
            Unit::Db => format!("{:.1} dB", v),
            Unit::Percent => format!("{:.0} %", v),
            Unit::Ms if v >= 1000.0 => format!("{:.2} s", v / 1000.0),
            Unit::Ms => format!("{:.0} ms", v),
            Unit::Semitones => format!("{:+.1} st", v),
            Unit::Cents => format!("{:+.0} ct", v),
            Unit::Octaves => format!("{:+.2} oct", v),
            Unit::Beats => format!("{:.2} bt", v),
            Unit::None => format!("{:.2}", v),
        },
    }
}

/// The unit label for a parameter (the MPC asks but does not show it).
pub fn unit_label(p: &ParamInfo) -> &'static str {
    match (p.curve, p.unit) {
        (Curve::Choice(_) | Curve::Toggle, _) => "",
        (_, Unit::Hz) => "Hz",
        (_, Unit::Db) => "dB",
        (_, Unit::Percent) => "%",
        (_, Unit::Ms) => "ms",
        (_, Unit::Semitones) => "st",
        (_, Unit::Cents) => "ct",
        (_, Unit::Octaves) => "oct",
        (_, Unit::Beats) => "beats",
        (_, Unit::None) => "",
    }
}

unsafe extern "C" fn dispatcher(
    effect: *mut AEffect,
    opcode: VstInt32,
    index: VstInt32,
    value: VstIntPtr,
    ptr: *mut c_void,
    opt: f32,
) -> VstIntPtr {
    let Some(inst) = instance(effect) else {
        return 0;
    };
    let param = |i: VstInt32| -> Option<usize> {
        (i >= 0 && (i as usize) < inst.params.len()).then_some(i as usize)
    };
    match opcode {
        EFF_OPEN => 0,
        EFF_CLOSE => {
            // SAFETY: the host closes an instance once and never uses it
            // again; `object` points at the Box made in plugin_main.
            let raw = (*effect).object as *mut Instance;
            (*effect).object = std::ptr::null_mut();
            drop(Box::from_raw(raw));
            0
        }
        EFF_SET_PROGRAM => 0,
        EFF_GET_PROGRAM => 0,
        EFF_GET_PROGRAM_NAME => {
            put_str(ptr, MAX_PROG_NAME_LEN + 1, "Default");
            0
        }
        EFF_GET_PROGRAM_NAME_INDEXED => {
            if index == 0 {
                put_str(ptr, MAX_PROG_NAME_LEN + 1, "Default");
                1
            } else {
                0
            }
        }
        EFF_GET_PARAM_NAME => {
            match param(index) {
                Some(i) => put_str(ptr, PARAM_STR_BYTES, &inst.names[i]),
                None => put_str(ptr, PARAM_STR_BYTES, ""),
            }
            0
        }
        EFF_GET_PARAM_DISPLAY => {
            match param(index) {
                Some(i) => put_str(
                    ptr,
                    PARAM_STR_BYTES,
                    &value_text(inst.params[i], inst.value(i)),
                ),
                None => put_str(ptr, PARAM_STR_BYTES, ""),
            }
            0
        }
        EFF_GET_PARAM_LABEL => {
            match param(index) {
                Some(i) => put_str(ptr, PARAM_STR_BYTES, unit_label(inst.params[i])),
                None => put_str(ptr, PARAM_STR_BYTES, ""),
            }
            0
        }
        EFF_CAN_BE_AUTOMATED => param(index).is_some() as VstIntPtr,
        EFF_SET_SAMPLE_RATE => {
            if opt.is_finite() && opt >= 8000.0 {
                inst.rate.store(opt as u32, Ordering::Relaxed);
            }
            0
        }
        EFF_SET_BLOCK_SIZE => {
            if value > 0 {
                inst.max_block
                    .store(value.min(1 << 16) as u32, Ordering::Relaxed);
            }
            0
        }
        EFF_MAINS_CHANGED => {
            if value != 0 {
                // Resume: the host sends it before processing starts.
                let audio = &mut *inst.audio.get();
                let rate = inst.rate.load(Ordering::Relaxed);
                let max = inst.max_block.load(Ordering::Relaxed) as usize;
                audio.node.prepare(rate, max);
                audio.node.reset();
                audio.prepared = true;
                for d in &inst.dirty {
                    d.store(true, Ordering::Relaxed);
                }
                inst.any_dirty.store(true, Ordering::Release);
            }
            0
        }
        EFF_GET_CHUNK => {
            if ptr.is_null() {
                return 0;
            }
            let chunk = &mut *inst.chunk.get();
            *chunk = inst.save();
            *(ptr as *mut *mut c_void) = chunk.as_mut_ptr() as *mut c_void;
            chunk.len() as VstIntPtr
        }
        EFF_SET_CHUNK => {
            if ptr.is_null() || value <= 0 {
                return 0;
            }
            let data = std::slice::from_raw_parts(ptr as *const u8, value as usize);
            inst.load(data) as VstIntPtr
        }
        EFF_PROCESS_EVENTS => {
            if !ptr.is_null() {
                take_events(inst, &*(ptr as *const VstEvents));
            }
            1
        }
        EFF_GET_PLUG_CATEGORY => {
            if inst.spec.instrument {
                PLUG_CATEG_SYNTH
            } else {
                PLUG_CATEG_EFFECT
            }
        }
        EFF_GET_EFFECT_NAME => {
            put_str(ptr, MAX_EFFECT_NAME_LEN + 1, inst.spec.name);
            1
        }
        EFF_GET_VENDOR_STRING => {
            put_str(ptr, MAX_VENDOR_STR_LEN + 1, inst.spec.vendor);
            1
        }
        EFF_GET_PRODUCT_STRING => {
            put_str(ptr, MAX_VENDOR_STR_LEN + 1, inst.spec.name);
            1
        }
        EFF_GET_VENDOR_VERSION => inst.spec.version as VstIntPtr,
        EFF_CAN_DO => {
            if ptr.is_null() {
                return 0;
            }
            match CStr::from_ptr(ptr as *const _).to_bytes() {
                b"receiveVstEvents" | b"receiveVstMidiEvent" if inst.spec.instrument => 1,
                b"receiveVstEvents" | b"receiveVstMidiEvent" => -1,
                b"receiveVstTimeInfo" => 1,
                b"sendVstEvents" | b"sendVstMidiEvent" | b"bypass" | b"MPE" => -1,
                _ => 0,
            }
        }
        EFF_GET_TAIL_SIZE => {
            let tail = (*inst.audio.get()).node.tail_frames();
            if tail == 0 {
                1
            } else {
                tail as VstIntPtr
            }
        }
        EFF_GET_VST_VERSION => VST_VERSION,
        _ => 0,
    }
}

/// Turns the host's MIDI for the next block into node events, in order.
unsafe fn take_events(inst: &Instance, list: &VstEvents) {
    let audio = &mut *inst.audio.get();
    let n = list.num_events.max(0) as usize;
    let first = list.events.as_ptr();
    for k in 0..n {
        if audio.events.len() >= MAX_EVENTS {
            break;
        }
        let e = *first.add(k);
        if e.is_null() || (*e).kind != VST_MIDI_TYPE {
            continue;
        }
        let m = &*(e as *const VstMidiEvent);
        let [status, d1, d2, _] = m.midi_data;
        let ev = match status & 0xF0 {
            0x90 if d2 > 0 => NodeEvent::NoteOn {
                note: d1 & 0x7F,
                velocity: d2 & 0x7F,
            },
            0x80 | 0x90 => NodeEvent::NoteOff { note: d1 & 0x7F },
            0xE0 => {
                let raw = ((d2 as i32 & 0x7F) << 7) | (d1 as i32 & 0x7F);
                NodeEvent::Bend {
                    value: ((raw - 8192) as f32 / 8192.0).clamp(-1.0, 1.0),
                }
            }
            _ => continue,
        };
        let t = Timed::new(m.delta_frames.max(0) as u32, ev);
        // Insertion keeps events at one offset in the host's order.
        let at = audio.events.partition_point(|x| x.offset <= t.offset);
        audio.events.insert(at, t);
    }
}

unsafe extern "C" fn set_parameter(effect: *mut AEffect, index: VstInt32, value: f32) {
    let Some(inst) = instance(effect) else { return };
    if index < 0 || index as usize >= inst.params.len() || !value.is_finite() {
        return;
    }
    let i = index as usize;
    inst.store(i, inst.params[i].from_unit(value));
}

unsafe extern "C" fn get_parameter(effect: *mut AEffect, index: VstInt32) -> f32 {
    let Some(inst) = instance(effect) else {
        return 0.0;
    };
    if index < 0 || index as usize >= inst.params.len() {
        return 0.0;
    }
    let i = index as usize;
    inst.params[i].to_unit(inst.value(i))
}

/// The host's transport for this block, if it gives one.
unsafe fn transport(inst: &Instance) -> Option<VstTimeInfo> {
    let host = inst.host?;
    let ptr = host(
        &inst.effect as *const AEffect as *mut AEffect,
        AUDIO_MASTER_GET_TIME,
        0,
        (PPQ_POS_VALID | TEMPO_VALID) as VstIntPtr,
        std::ptr::null_mut(),
        0.0,
    );
    if ptr == 0 {
        None
    } else {
        Some(*(ptr as *const VstTimeInfo))
    }
}

unsafe extern "C" fn process_replacing(
    effect: *mut AEffect,
    inputs: *mut *mut f32,
    outputs: *mut *mut f32,
    frames: VstInt32,
) {
    let Some(inst) = instance(effect) else { return };
    if frames <= 0 || outputs.is_null() {
        return;
    }
    let frames = frames as usize;
    let out_l = *outputs;
    let out_r = *outputs.add(1);
    if out_l.is_null() || out_r.is_null() {
        return;
    }
    // In place is the MPC's way; copy only when the buffers differ.
    for ch in 0..2 {
        let out = *outputs.add(ch);
        let inp = if inputs.is_null() || inst.spec.instrument {
            std::ptr::null_mut()
        } else {
            *inputs.add(ch)
        };
        if inp.is_null() {
            std::ptr::write_bytes(out, 0, frames);
        } else if inp != out {
            std::ptr::copy(inp, out, frames);
        }
    }
    let audio = &mut *inst.audio.get();
    if !audio.prepared {
        audio.events.clear();
        return;
    }
    if inst.any_dirty.swap(false, Ordering::Acquire) {
        for (i, p) in inst.params.iter().enumerate() {
            if inst.dirty[i].swap(false, Ordering::Acquire) {
                audio.node.set_param(p.id, inst.value(i));
            }
        }
    }
    let rate = inst.rate.load(Ordering::Relaxed);
    let max = (inst.max_block.load(Ordering::Relaxed) as usize).max(1);
    let time = transport(inst);
    let mut bpm = 120.0;
    let mut beats = 0.0;
    let mut playing = false;
    if let Some(t) = time {
        if t.flags & TEMPO_VALID != 0 && t.tempo > 0.0 {
            bpm = t.tempo;
        }
        if t.flags & PPQ_POS_VALID != 0 {
            beats = t.ppq_pos;
        }
        playing = t.flags & TRANSPORT_PLAYING != 0;
    }
    let left = std::slice::from_raw_parts_mut(out_l, frames);
    let right = std::slice::from_raw_parts_mut(out_r, frames);
    let mut start = 0;
    while start < frames {
        let len = (frames - start).min(max);
        audio.piece.clear();
        for t in audio.events.iter() {
            let off = t.offset as usize;
            let last_piece = start + len >= frames;
            if (off >= start && off < start + len) || (last_piece && off >= frames) {
                let o = (off.min(frames - 1) - start) as u32;
                audio.piece.push(Timed::new(o.min(len as u32 - 1), t.ev));
            }
        }
        let ctx = Ctx {
            rate,
            frame: audio.frame,
            bpm,
            beats: beats + start as f64 * bpm / 60.0 / rate as f64,
            playing,
            sidechain: None,
        };
        audio.node.process(
            &ctx,
            &audio.piece,
            &mut left[start..start + len],
            &mut right[start..start + len],
        );
        audio.frame += len as u64;
        start += len;
    }
    audio.events.clear();
}
