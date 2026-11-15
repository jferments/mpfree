//! Two small CLAP plugins of our own, for testing the host without anyone
//! else's binary: a 16-voice test synth and a stereo gain effect. They are
//! built into this crate (reached through [`ENTRY`], as if dlopened) and
//! into `mpfree-clap-testplug`, a real `.clap` shared library.
//!
//! They keep the CLAP rules a host relies on: no allocation in `process`,
//! events on their exact frame, parameters readable from the main thread
//! while the audio thread runs (values are atomics), state as plain bytes.
//!
//! The synth has presets: three built in ([`BUILT_IN_PRESETS`]), and files
//! ending `.mptp` (its state bytes) in the folder named by the environment
//! variable [`PRESETS_ENV`], or else in [`HOME_PRESETS`] in its home
//! folder when that is there (as a real plugin keeps its user presets). A
//! preset discovery provider lists them and the synth's preset-load
//! extension loads them.

use crate::sys::*;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// The entry point both builds export.
pub const ENTRY: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    get_factory: Some(entry_get_factory),
};

/// `init` calls minus `deinit` calls, for tests of a host's bookkeeping.
pub static LIVE_INITS: AtomicU32 = AtomicU32::new(0);

pub const SYNTH_ID: &str = "org.mpfree.test-synth";
pub const GAIN_ID: &str = "org.mpfree.test-gain";

/// The folder of the synth's preset files, when set.
pub const PRESETS_ENV: &str = "MPFREE_TESTPLUG_PRESETS";
/// Without [`PRESETS_ENV`], this folder in `$HOME`, when it is there.
pub const HOME_PRESETS: &str = "mpfree-testplug-presets";
/// The synth's built-in presets: load key, name, category, and the gain,
/// decay and wave they set.
pub const BUILT_IN_PRESETS: [(&str, &str, &str, f64, f64, f64); 3] = [
    ("soft", "Soft Sine", "Keys", -12.0, 800.0, 0.0),
    ("bright", "Bright Saw", "Leads", -6.0, 300.0, 1.0),
    ("short", "Short Square", "Plucks", -9.0, 60.0, 2.0),
];

/// Test synth parameter ids (deliberately not 0, 1, 2).
pub mod synth_params {
    pub const GAIN: u32 = 10;
    pub const DECAY: u32 = 20;
    pub const WAVE: u32 = 30;
    /// Read-only: voices sounding, sent back as output events.
    pub const VOICES: u32 = 40;
    /// Read-only: the transport's tempo, sent back as output events.
    pub const TEMPO: u32 = 50;
    /// Hidden: never shown to a user.
    pub const SECRET: u32 = 60;
}

/// Test gain parameter ids.
pub mod gain_params {
    pub const GAIN: u32 = 1;
    /// 1 makes `process` return `CLAP_PROCESS_ERROR`.
    pub const FAIL: u32 = 2;
    /// Milliseconds `process` sleeps (a plugin that misses its deadline;
    /// for the sandbox's tests only).
    pub const STALL: u32 = 3;
    /// 1 makes `process` abort the process (a crashing plugin; for the
    /// sandbox's tests only, never in the host's own process).
    pub const CRASH: u32 = 4;
}

unsafe extern "C" fn entry_init(_path: *const c_char) -> bool {
    LIVE_INITS.fetch_add(1, Ordering::SeqCst);
    true
}

unsafe extern "C" fn entry_deinit() {
    LIVE_INITS.fetch_sub(1, Ordering::SeqCst);
}

struct SyncPtr<T>(T);
// SAFETY: the wrapped values point only at 'static, never-written data.
unsafe impl<T> Sync for SyncPtr<T> {}

static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(factory_count),
    get_plugin_descriptor: Some(factory_descriptor),
    create_plugin: Some(factory_create),
};

static SYNTH_FEATURES: SyncPtr<[*const c_char; 4]> = SyncPtr([
    c"instrument".as_ptr(),
    c"synthesizer".as_ptr(),
    c"stereo".as_ptr(),
    ptr::null(),
]);
static GAIN_FEATURES: SyncPtr<[*const c_char; 3]> =
    SyncPtr([c"audio-effect".as_ptr(), c"stereo".as_ptr(), ptr::null()]);

static SYNTH_DESC: SyncPtr<clap_plugin_descriptor> = SyncPtr(clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: c"org.mpfree.test-synth".as_ptr(),
    name: c"MPFree Test Synth".as_ptr(),
    vendor: c"MPFree".as_ptr(),
    url: c"".as_ptr(),
    manual_url: c"".as_ptr(),
    support_url: c"".as_ptr(),
    version: c"1.0.0".as_ptr(),
    description: c"A 16-voice test synth for the CLAP host's tests".as_ptr(),
    features: &SYNTH_FEATURES.0 as *const [*const c_char; 4] as *const *const c_char,
});

static GAIN_DESC: SyncPtr<clap_plugin_descriptor> = SyncPtr(clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: c"org.mpfree.test-gain".as_ptr(),
    name: c"MPFree Test Gain".as_ptr(),
    vendor: c"MPFree".as_ptr(),
    url: c"".as_ptr(),
    manual_url: c"".as_ptr(),
    support_url: c"".as_ptr(),
    version: c"1.0.0".as_ptr(),
    description: c"A stereo gain for the CLAP host's tests".as_ptr(),
    features: &GAIN_FEATURES.0 as *const [*const c_char; 3] as *const *const c_char,
});

unsafe extern "C" fn entry_get_factory(id: *const c_char) -> *const c_void {
    if id.is_null() {
        return ptr::null();
    }
    let id = CStr::from_ptr(id).to_bytes_with_nul();
    if id == CLAP_PLUGIN_FACTORY_ID {
        &FACTORY as *const clap_plugin_factory as *const c_void
    } else if id == CLAP_PRESET_DISCOVERY_FACTORY_ID {
        &PRESET_FACTORY as *const clap_preset_discovery_factory as *const c_void
    } else {
        ptr::null()
    }
}

unsafe extern "C" fn factory_count(_f: *const clap_plugin_factory) -> u32 {
    2
}

unsafe extern "C" fn factory_descriptor(
    _f: *const clap_plugin_factory,
    index: u32,
) -> *const clap_plugin_descriptor {
    match index {
        0 => &SYNTH_DESC.0,
        1 => &GAIN_DESC.0,
        _ => ptr::null(),
    }
}

unsafe extern "C" fn factory_create(
    _f: *const clap_plugin_factory,
    host: *const clap_host,
    id: *const c_char,
) -> *const clap_plugin {
    if id.is_null() || host.is_null() {
        return ptr::null();
    }
    let which = match CStr::from_ptr(id).to_str() {
        Ok(SYNTH_ID) => Which::Synth,
        Ok(GAIN_ID) => Which::Gain,
        _ => return ptr::null(),
    };
    let desc: *const clap_plugin_descriptor = match which {
        Which::Synth => &SYNTH_DESC.0,
        Which::Gain => &GAIN_DESC.0,
    };
    let p = Box::new(Plug {
        clap: clap_plugin {
            desc,
            plugin_data: ptr::null_mut(),
            init: Some(p_init),
            destroy: Some(p_destroy),
            activate: Some(p_activate),
            deactivate: Some(p_deactivate),
            start_processing: Some(p_start),
            stop_processing: Some(p_stop),
            reset: Some(p_reset),
            process: Some(p_process),
            get_extension: Some(p_get_extension),
            on_main_thread: Some(p_on_main_thread),
        },
        host,
        which,
        values: match which {
            Which::Synth => SYNTH_PARAMS
                .iter()
                .map(|p| AtomicU64::new(p.3.to_bits()))
                .collect(),
            Which::Gain => GAIN_PARAMS
                .iter()
                .map(|p| AtomicU64::new(p.3.to_bits()))
                .collect(),
        },
        rate: 44_100.0,
        voices: [Voice::default(); VOICES],
        age: 0,
        bend: 0.0,
        sent_voices: -1.0,
        sent_tempo: -1.0,
        main_calls: AtomicU32::new(0),
    });
    let raw = Box::into_raw(p);
    (*raw).clap.plugin_data = raw as *mut c_void;
    &(*raw).clap
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Synth,
    Gain,
}

const VOICES: usize = 16;

#[derive(Clone, Copy, Default)]
struct Voice {
    on: bool,
    held: bool,
    key: i16,
    level: f64,
    env: f64,
    phase: f64,
    tune: f64,
    pressure: f64,
    age: u64,
}

/// (id, name, module, default, min, max, flags)
type ParamDef = (u32, &'static CStr, &'static CStr, f64, f64, f64, u32);

const AUTO: u32 = CLAP_PARAM_IS_AUTOMATABLE;
static SYNTH_PARAMS: [ParamDef; 6] = [
    (synth_params::GAIN, c"Gain", c"Amp", -12.0, -60.0, 6.0, AUTO),
    (
        synth_params::DECAY,
        c"Decay",
        c"Amp",
        300.0,
        10.0,
        2000.0,
        AUTO,
    ),
    (
        synth_params::WAVE,
        c"Wave",
        c"Osc",
        0.0,
        0.0,
        2.0,
        AUTO | CLAP_PARAM_IS_STEPPED | CLAP_PARAM_IS_ENUM,
    ),
    (
        synth_params::VOICES,
        c"Voices",
        c"Meters",
        0.0,
        0.0,
        16.0,
        CLAP_PARAM_IS_READONLY | CLAP_PARAM_IS_STEPPED,
    ),
    (
        synth_params::TEMPO,
        c"Tempo",
        c"Meters",
        0.0,
        0.0,
        999.0,
        CLAP_PARAM_IS_READONLY,
    ),
    (
        synth_params::SECRET,
        c"Secret",
        c"",
        0.0,
        0.0,
        1.0,
        CLAP_PARAM_IS_HIDDEN,
    ),
];
static GAIN_PARAMS: [ParamDef; 4] = [
    (gain_params::GAIN, c"Gain", c"", 0.0, -60.0, 24.0, AUTO),
    (
        gain_params::FAIL,
        c"Fail",
        c"Test",
        0.0,
        0.0,
        1.0,
        CLAP_PARAM_IS_STEPPED,
    ),
    (
        gain_params::STALL,
        c"Stall",
        c"Test",
        0.0,
        0.0,
        1000.0,
        CLAP_PARAM_IS_STEPPED,
    ),
    (
        gain_params::CRASH,
        c"Crash",
        c"Test",
        0.0,
        0.0,
        1.0,
        CLAP_PARAM_IS_STEPPED,
    ),
];

#[repr(C)]
struct Plug {
    clap: clap_plugin,
    host: *const clap_host,
    which: Which,
    /// f64 bits, in the order of the parameter table.
    values: Vec<AtomicU64>,
    rate: f64,
    voices: [Voice; VOICES],
    age: u64,
    bend: f64,
    sent_voices: f64,
    sent_tempo: f64,
    /// `on_main_thread` calls (the host's `request_callback` answered).
    main_calls: AtomicU32,
}

impl Plug {
    fn table(&self) -> &'static [ParamDef] {
        match self.which {
            Which::Synth => &SYNTH_PARAMS,
            Which::Gain => &GAIN_PARAMS,
        }
    }
    fn index(&self, id: u32) -> Option<usize> {
        self.table().iter().position(|p| p.0 == id)
    }
    fn get(&self, id: u32) -> f64 {
        self.index(id).map_or(0.0, |i| {
            f64::from_bits(self.values[i].load(Ordering::Relaxed))
        })
    }
    fn set(&self, id: u32, v: f64) {
        if let Some(i) = self.index(id) {
            let d = &self.table()[i];
            let mut v = v.clamp(d.4, d.5);
            if d.6 & CLAP_PARAM_IS_STEPPED != 0 {
                v = v.round();
            }
            self.values[i].store(v.to_bits(), Ordering::Relaxed);
        }
    }
}

unsafe fn plug<'a>(p: *const clap_plugin) -> &'a mut Plug {
    &mut *((*p).plugin_data as *mut Plug)
}

unsafe extern "C" fn p_init(p: *const clap_plugin) -> bool {
    let me = plug(p);
    // Say hello through the host's log, if it has one.
    if let Some(get) = (*me.host).get_extension {
        let log = get(me.host, CLAP_EXT_LOG.as_ptr() as *const c_char) as *const clap_host_log;
        if !log.is_null() {
            if let Some(f) = (*log).log {
                f(me.host, CLAP_LOG_INFO, c"test plugin ready".as_ptr());
            }
        }
    }
    true
}

unsafe extern "C" fn p_destroy(p: *const clap_plugin) {
    drop(Box::from_raw((*p).plugin_data as *mut Plug));
}

unsafe extern "C" fn p_activate(p: *const clap_plugin, rate: f64, _min: u32, _max: u32) -> bool {
    let me = plug(p);
    me.rate = rate;
    me.voices = [Voice::default(); VOICES];
    // Ask for a main-thread callback, as plugins do for deferred work.
    if let Some(f) = (*me.host).request_callback {
        f(me.host);
    }
    true
}

unsafe extern "C" fn p_deactivate(_p: *const clap_plugin) {}
unsafe extern "C" fn p_start(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn p_stop(_p: *const clap_plugin) {}

unsafe extern "C" fn p_reset(p: *const clap_plugin) {
    let me = plug(p);
    me.voices = [Voice::default(); VOICES];
    me.bend = 0.0;
}

unsafe extern "C" fn p_on_main_thread(p: *const clap_plugin) {
    plug(p).main_calls.fetch_add(1, Ordering::Relaxed);
}

unsafe extern "C" fn p_get_extension(p: *const clap_plugin, id: *const c_char) -> *const c_void {
    if id.is_null() {
        return ptr::null();
    }
    let id = CStr::from_ptr(id).to_bytes_with_nul();
    if id == CLAP_EXT_PRESET_LOAD && plug(p).which == Which::Synth {
        &PRESET_LOAD as *const clap_plugin_preset_load as *const c_void
    } else if id == CLAP_EXT_PARAMS {
        &PARAMS as *const clap_plugin_params as *const c_void
    } else if id == CLAP_EXT_AUDIO_PORTS {
        &AUDIO_PORTS as *const clap_plugin_audio_ports as *const c_void
    } else if id == CLAP_EXT_NOTE_PORTS {
        &NOTE_PORTS as *const clap_plugin_note_ports as *const c_void
    } else if id == CLAP_EXT_STATE {
        &STATE as *const clap_plugin_state as *const c_void
    } else if id == CLAP_EXT_TAIL {
        &TAIL as *const clap_plugin_tail as *const c_void
    } else if id == CLAP_EXT_LATENCY {
        &LATENCY as *const clap_plugin_latency as *const c_void
    } else {
        ptr::null()
    }
}

// ------------------------------------------------------------------ process

fn db(v: f64) -> f64 {
    10f64.powf(v / 20.0)
}

unsafe fn events(
    proc_: &clap_process,
) -> (
    u32,
    Option<unsafe extern "C" fn(*const clap_input_events, u32) -> *const clap_event_header>,
) {
    let ev = proc_.in_events;
    if ev.is_null() {
        return (0, None);
    }
    let n = (*ev).size.map_or(0, |f| f(ev));
    (n, (*ev).get)
}

unsafe extern "C" fn p_process(
    p: *const clap_plugin,
    proc_: *const clap_process,
) -> clap_process_status {
    let me = plug(p);
    let pr = &*proc_;
    match me.which {
        Which::Synth => synth_process(me, pr),
        Which::Gain => gain_process(me, pr),
    }
}

unsafe fn apply_param(me: &Plug, h: *const clap_event_header) {
    let e = &*(h as *const clap_event_param_value);
    me.set(e.param_id, e.value);
}

unsafe fn synth_event(me: &mut Plug, h: *const clap_event_header) {
    if (*h).space_id != CLAP_CORE_EVENT_SPACE_ID {
        return;
    }
    match (*h).type_ {
        CLAP_EVENT_NOTE_ON => {
            let e = &*(h as *const clap_event_note);
            note_on(me, e.key, e.velocity);
        }
        CLAP_EVENT_NOTE_OFF => {
            let e = &*(h as *const clap_event_note);
            note_off(me, e.key);
        }
        CLAP_EVENT_NOTE_EXPRESSION => {
            let e = &*(h as *const clap_event_note_expression);
            for v in me
                .voices
                .iter_mut()
                .filter(|v| v.on && (e.key < 0 || v.key == e.key))
            {
                match e.expression_id {
                    CLAP_NOTE_EXPRESSION_TUNING => v.tune = e.value,
                    CLAP_NOTE_EXPRESSION_PRESSURE => v.pressure = e.value,
                    _ => {}
                }
            }
        }
        CLAP_EVENT_PARAM_VALUE => apply_param(me, h),
        CLAP_EVENT_MIDI => {
            let e = &*(h as *const clap_event_midi);
            let [s, a, b] = e.data;
            match s & 0xF0 {
                0x90 if b > 0 => note_on(me, a as i16, b as f64 / 127.0),
                0x80 | 0x90 => note_off(me, a as i16),
                0xE0 => me.bend = ((((b as i32) << 7) | a as i32) - 8192) as f64 / 8192.0 * 2.0,
                _ => {}
            }
        }
        _ => {}
    }
}

fn note_on(me: &mut Plug, key: i16, velocity: f64) {
    if !(0..128).contains(&key) {
        return;
    }
    me.age += 1;
    let slot = me.voices.iter().position(|v| !v.on).unwrap_or_else(|| {
        // Steal the oldest.
        let mut best = 0;
        for (i, v) in me.voices.iter().enumerate() {
            if v.age < me.voices[best].age {
                best = i;
            }
        }
        best
    });
    me.voices[slot] = Voice {
        on: true,
        held: true,
        key,
        level: velocity,
        env: 1.0,
        phase: 0.0,
        tune: 0.0,
        pressure: 0.0,
        age: me.age,
    };
}

fn note_off(me: &mut Plug, key: i16) {
    for v in me
        .voices
        .iter_mut()
        .filter(|v| v.on && (key < 0 || v.key == key))
    {
        v.held = false;
    }
}

unsafe fn synth_process(me: &mut Plug, pr: &clap_process) -> clap_process_status {
    if pr.audio_outputs_count < 1 || pr.audio_outputs.is_null() {
        return CLAP_PROCESS_ERROR;
    }
    let out = &*pr.audio_outputs;
    if out.channel_count < 2 || out.data32.is_null() {
        return CLAP_PROCESS_ERROR;
    }
    let l = *out.data32;
    let r = *out.data32.add(1);
    let n = pr.frames_count;
    let (count, get) = events(pr);
    let mut next = 0u32;
    let rate = me.rate;
    for i in 0..n {
        while next < count {
            let h = get.map_or(ptr::null(), |g| g(pr.in_events, next));
            if h.is_null() || (*h).time > i {
                break;
            }
            synth_event(me, h);
            next += 1;
        }
        let gain = db(me.get(synth_params::GAIN));
        let decay_s = me.get(synth_params::DECAY) / 1000.0;
        // -60 dB over the decay time; four times as fast once released.
        let held_mul = 10f64.powf(-3.0 / (decay_s * rate));
        let free_mul = held_mul.powi(4);
        let wave = me.get(synth_params::WAVE) as u32;
        let (mut a, mut b) = (0.0f64, 0.0f64);
        for v in me.voices.iter_mut().filter(|v| v.on) {
            let hz = 440.0 * 2f64.powf((v.key as f64 - 69.0 + v.tune + me.bend) / 12.0);
            let s = match wave {
                0 => (v.phase * std::f64::consts::TAU).sin(),
                1 => 2.0 * v.phase - 1.0,
                _ => {
                    if v.phase < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
            };
            let y = s * v.env * v.level * gain * (1.0 + v.pressure);
            // A little spread by key, so the channels differ.
            let pan = (v.key as f64 - 64.0) / 128.0;
            a += y * (0.5 - pan);
            b += y * (0.5 + pan);
            v.phase = (v.phase + hz / rate).fract();
            v.env *= if v.held { held_mul } else { free_mul };
            if v.env < 1e-6 {
                v.on = false;
            }
        }
        *l.add(i as usize) = a as f32;
        *r.add(i as usize) = b as f32;
    }
    // Late events (past the block) still apply.
    while next < count {
        let h = get.map_or(ptr::null(), |g| g(pr.in_events, next));
        if !h.is_null() {
            synth_event(me, h);
        }
        next += 1;
    }
    // Meters back to the host.
    let sounding = me.voices.iter().filter(|v| v.on).count() as f64;
    if sounding != me.sent_voices {
        me.sent_voices = sounding;
        me.set(synth_params::VOICES, sounding);
        push_value(pr, synth_params::VOICES, sounding);
    }
    if !pr.transport.is_null() && (*pr.transport).flags & CLAP_TRANSPORT_HAS_TEMPO != 0 {
        let t = (*pr.transport).tempo;
        if t != me.sent_tempo {
            me.sent_tempo = t;
            me.set(synth_params::TEMPO, t);
            push_value(pr, synth_params::TEMPO, t);
        }
    }
    if me.voices.iter().any(|v| v.on) {
        CLAP_PROCESS_CONTINUE
    } else {
        CLAP_PROCESS_SLEEP
    }
}

unsafe fn push_value(pr: &clap_process, id: u32, value: f64) {
    let out = pr.out_events;
    if out.is_null() {
        return;
    }
    let e = clap_event_param_value {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_param_value>() as u32,
            time: 0,
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_: CLAP_EVENT_PARAM_VALUE,
            flags: 0,
        },
        param_id: id,
        cookie: ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    };
    if let Some(f) = (*out).try_push {
        f(out, &e.header);
    }
}

unsafe fn gain_process(me: &mut Plug, pr: &clap_process) -> clap_process_status {
    if pr.audio_inputs_count < 1
        || pr.audio_outputs_count < 1
        || pr.audio_inputs.is_null()
        || pr.audio_outputs.is_null()
    {
        return CLAP_PROCESS_ERROR;
    }
    let (inp, out) = (&*pr.audio_inputs, &*pr.audio_outputs);
    if inp.channel_count < 2 || out.channel_count < 2 {
        return CLAP_PROCESS_ERROR;
    }
    let (il, ir) = (*inp.data32, *inp.data32.add(1));
    let (ol, or) = (*out.data32, *out.data32.add(1));
    let (count, get) = events(pr);
    let mut next = 0u32;
    for i in 0..pr.frames_count {
        while next < count {
            let h = get.map_or(ptr::null(), |g| g(pr.in_events, next));
            if h.is_null() || (*h).time > i {
                break;
            }
            if (*h).type_ == CLAP_EVENT_PARAM_VALUE {
                apply_param(me, h);
            }
            next += 1;
        }
        let g = db(me.get(gain_params::GAIN)) as f32;
        *ol.add(i as usize) = *il.add(i as usize) * g;
        *or.add(i as usize) = *ir.add(i as usize) * g;
    }
    let stall = me.get(gain_params::STALL);
    if stall > 0.0 {
        std::thread::sleep(std::time::Duration::from_millis(stall as u64));
    }
    if me.get(gain_params::CRASH) >= 0.5 {
        std::process::abort();
    }
    if me.get(gain_params::FAIL) >= 0.5 {
        CLAP_PROCESS_ERROR
    } else {
        CLAP_PROCESS_CONTINUE
    }
}

// ------------------------------------------------------------------ extensions

static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(params_count),
    get_info: Some(params_info),
    get_value: Some(params_value),
    value_to_text: Some(params_to_text),
    text_to_value: Some(params_from_text),
    flush: Some(params_flush),
};

unsafe extern "C" fn params_count(p: *const clap_plugin) -> u32 {
    plug(p).table().len() as u32
}

unsafe extern "C" fn params_info(
    p: *const clap_plugin,
    index: u32,
    info: *mut clap_param_info,
) -> bool {
    let Some(d) = plug(p).table().get(index as usize) else {
        return false;
    };
    let info = &mut *info;
    info.id = d.0;
    info.flags = d.6;
    info.cookie = ptr::null_mut();
    copy_str(&mut info.name, d.1);
    copy_str(&mut info.module, d.2);
    info.default_value = d.3;
    info.min_value = d.4;
    info.max_value = d.5;
    true
}

fn copy_str(dst: &mut [c_char], s: &CStr) {
    let b = s.to_bytes();
    let n = b.len().min(dst.len() - 1);
    for (d, &c) in dst.iter_mut().zip(&b[..n]) {
        *d = c as c_char;
    }
    dst[n] = 0;
}

unsafe extern "C" fn params_value(p: *const clap_plugin, id: clap_id, out: *mut f64) -> bool {
    let me = plug(p);
    if me.index(id).is_none() {
        return false;
    }
    *out = me.get(id);
    true
}

const WAVES: [&CStr; 3] = [c"Sine", c"Saw", c"Square"];

unsafe extern "C" fn params_to_text(
    p: *const clap_plugin,
    id: clap_id,
    value: f64,
    out: *mut c_char,
    cap: u32,
) -> bool {
    let me = plug(p);
    if cap == 0 || me.index(id).is_none() {
        return false;
    }
    let dst = std::slice::from_raw_parts_mut(out, cap as usize);
    if me.which == Which::Synth && id == synth_params::WAVE {
        let k = (value.round() as usize).min(WAVES.len() - 1);
        copy_str(dst, WAVES[k]);
        return true;
    }
    // Plain numbers, without allocating: the integer part only.
    let mut tmp = [0u8; 24];
    let mut v = value.round() as i64;
    let neg = v < 0;
    v = v.abs();
    let mut i = tmp.len();
    loop {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    if neg {
        i -= 1;
        tmp[i] = b'-';
    }
    let s = &tmp[i..];
    let n = s.len().min(dst.len() - 1);
    for (d, &c) in dst.iter_mut().zip(&s[..n]) {
        *d = c as c_char;
    }
    dst[n] = 0;
    true
}

unsafe extern "C" fn params_from_text(
    _p: *const clap_plugin,
    _id: clap_id,
    _text: *const c_char,
    _out: *mut f64,
) -> bool {
    false
}

unsafe extern "C" fn params_flush(
    p: *const clap_plugin,
    input: *const clap_input_events,
    _out: *const clap_output_events,
) {
    let me = plug(p);
    if input.is_null() {
        return;
    }
    let n = (*input).size.map_or(0, |f| f(input));
    for i in 0..n {
        let h = (*input).get.map_or(ptr::null(), |g| g(input, i));
        if !h.is_null() && (*h).type_ == CLAP_EVENT_PARAM_VALUE {
            apply_param(me, h);
        }
    }
}

static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(ap_count),
    get: Some(ap_get),
};

unsafe extern "C" fn ap_count(p: *const clap_plugin, is_input: bool) -> u32 {
    match (plug(p).which, is_input) {
        (Which::Synth, true) => 0,
        _ => 1,
    }
}

unsafe extern "C" fn ap_get(
    p: *const clap_plugin,
    index: u32,
    is_input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    if index >= ap_count(p, is_input) {
        return false;
    }
    let info = &mut *info;
    info.id = if is_input { 0 } else { 1 };
    copy_str(&mut info.name, if is_input { c"In" } else { c"Out" });
    info.flags = CLAP_AUDIO_PORT_IS_MAIN;
    info.channel_count = 2;
    info.port_type = c"stereo".as_ptr();
    info.in_place_pair = CLAP_INVALID_ID;
    true
}

static NOTE_PORTS: clap_plugin_note_ports = clap_plugin_note_ports {
    count: Some(np_count),
    get: Some(np_get),
};

unsafe extern "C" fn np_count(p: *const clap_plugin, is_input: bool) -> u32 {
    u32::from(plug(p).which == Which::Synth && is_input)
}

unsafe extern "C" fn np_get(
    p: *const clap_plugin,
    index: u32,
    is_input: bool,
    info: *mut clap_note_port_info,
) -> bool {
    if index >= np_count(p, is_input) {
        return false;
    }
    let info = &mut *info;
    info.id = 0;
    info.supported_dialects = CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI;
    info.preferred_dialect = CLAP_NOTE_DIALECT_CLAP;
    copy_str(&mut info.name, c"Notes");
    true
}

static STATE: clap_plugin_state = clap_plugin_state {
    save: Some(state_save),
    load: Some(state_load),
};

/// State: a magic, then each parameter's f64 bits, little-endian.
const MAGIC: &[u8; 4] = b"MPTP";

unsafe extern "C" fn state_save(p: *const clap_plugin, s: *const clap_ostream) -> bool {
    let me = plug(p);
    let Some(write) = (*s).write else {
        return false;
    };
    let put = |b: &[u8]| -> bool {
        let mut off = 0;
        while off < b.len() {
            let n = write(
                s,
                b[off..].as_ptr() as *const c_void,
                (b.len() - off) as u64,
            );
            if n <= 0 {
                return false;
            }
            off += n as usize;
        }
        true
    };
    if !put(MAGIC) {
        return false;
    }
    for v in &me.values {
        if !put(&v.load(Ordering::Relaxed).to_le_bytes()) {
            return false;
        }
    }
    true
}

unsafe extern "C" fn state_load(p: *const clap_plugin, s: *const clap_istream) -> bool {
    let me = plug(p);
    let Some(read) = (*s).read else { return false };
    let get = |b: &mut [u8]| -> bool {
        let mut off = 0;
        while off < b.len() {
            let n = read(
                s,
                b[off..].as_mut_ptr() as *mut c_void,
                (b.len() - off) as u64,
            );
            if n <= 0 {
                return false;
            }
            off += n as usize;
        }
        true
    };
    let mut magic = [0u8; 4];
    if !get(&mut magic) || &magic != MAGIC {
        return false;
    }
    for i in 0..me.values.len() {
        let mut b = [0u8; 8];
        if !get(&mut b) {
            return false;
        }
        let id = me.table()[i].0;
        me.set(id, f64::from_bits(u64::from_le_bytes(b)));
    }
    true
}

static TAIL: clap_plugin_tail = clap_plugin_tail {
    get: Some(tail_get),
};

unsafe extern "C" fn tail_get(p: *const clap_plugin) -> u32 {
    let me = plug(p);
    match me.which {
        // Down to -100 dB at the held decay rate.
        Which::Synth => (me.get(synth_params::DECAY) / 1000.0 * 100.0 / 60.0 * me.rate) as u32,
        Which::Gain => 0,
    }
}

static LATENCY: clap_plugin_latency = clap_plugin_latency {
    get: Some(latency_get),
};

unsafe extern "C" fn latency_get(_p: *const clap_plugin) -> u32 {
    0
}

/// The test plugin's count of `on_main_thread` calls, for host tests. Only
/// for plugins made by this module.
///
/// # Safety
/// `p` must be a live plugin created by this module's factory.
pub unsafe fn main_thread_calls(p: *const clap_plugin) -> u32 {
    plug(p).main_calls.load(Ordering::Relaxed)
}

// ------------------------------------------------------------------ presets

static PRESET_FACTORY: clap_preset_discovery_factory = clap_preset_discovery_factory {
    count: Some(pf_count),
    get_descriptor: Some(pf_descriptor),
    create: Some(pf_create),
};

pub const PRESET_PROVIDER_ID: &str = "org.mpfree.test-presets";

static PROVIDER_DESC: SyncPtr<clap_preset_discovery_provider_descriptor> =
    SyncPtr(clap_preset_discovery_provider_descriptor {
        clap_version: CLAP_VERSION,
        id: c"org.mpfree.test-presets".as_ptr(),
        name: c"Test synth presets".as_ptr(),
        vendor: c"MPFree".as_ptr(),
    });

unsafe extern "C" fn pf_count(_f: *const clap_preset_discovery_factory) -> u32 {
    1
}

unsafe extern "C" fn pf_descriptor(
    _f: *const clap_preset_discovery_factory,
    index: u32,
) -> *const clap_preset_discovery_provider_descriptor {
    if index == 0 {
        &PROVIDER_DESC.0
    } else {
        ptr::null()
    }
}

#[repr(C)]
struct Provider {
    clap: clap_preset_discovery_provider,
    indexer: *const clap_preset_discovery_indexer,
}

unsafe extern "C" fn pf_create(
    _f: *const clap_preset_discovery_factory,
    indexer: *const clap_preset_discovery_indexer,
    id: *const c_char,
) -> *const clap_preset_discovery_provider {
    if id.is_null() || indexer.is_null() || CStr::from_ptr(id).to_str() != Ok(PRESET_PROVIDER_ID) {
        return ptr::null();
    }
    let p = Box::into_raw(Box::new(Provider {
        clap: clap_preset_discovery_provider {
            desc: &PROVIDER_DESC.0,
            provider_data: ptr::null_mut(),
            init: Some(pv_init),
            destroy: Some(pv_destroy),
            get_metadata: Some(pv_metadata),
            get_extension: Some(pv_extension),
        },
        indexer,
    }));
    (*p).clap.provider_data = p as *mut c_void;
    &(*p).clap
}

unsafe fn provider<'a>(p: *const clap_preset_discovery_provider) -> &'a Provider {
    &*((*p).provider_data as *const Provider)
}

unsafe extern "C" fn pv_init(p: *const clap_preset_discovery_provider) -> bool {
    let ix = provider(p).indexer;
    let (Some(filetype), Some(location)) = ((*ix).declare_filetype, (*ix).declare_location) else {
        return false;
    };
    let t = clap_preset_discovery_filetype {
        name: c"Test synth preset".as_ptr(),
        description: ptr::null(),
        file_extension: c"mptp".as_ptr(),
    };
    let built_in = clap_preset_discovery_location {
        flags: CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT,
        name: c"Built in".as_ptr(),
        kind: CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN,
        location: ptr::null(),
    };
    if !filetype(ix, &t) || !location(ix, &built_in) {
        return false;
    }
    let home = std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join(HOME_PRESETS))
        .filter(|d| d.is_dir())
        .map(|d| d.into_os_string());
    if let Some(dir) = std::env::var_os(PRESETS_ENV)
        .filter(|d| !d.is_empty())
        .or(home)
    {
        let Ok(dir) = std::ffi::CString::new(dir.as_encoded_bytes()) else {
            return false;
        };
        let files = clap_preset_discovery_location {
            flags: CLAP_PRESET_DISCOVERY_IS_USER_CONTENT,
            name: c"Your presets".as_ptr(),
            kind: CLAP_PRESET_DISCOVERY_LOCATION_FILE,
            location: dir.as_ptr(),
        };
        if !location(ix, &files) {
            return false;
        }
    }
    true
}

unsafe extern "C" fn pv_destroy(p: *const clap_preset_discovery_provider) {
    drop(Box::from_raw((*p).provider_data as *mut Provider));
}

unsafe extern "C" fn pv_extension(
    _p: *const clap_preset_discovery_provider,
    _id: *const c_char,
) -> *const c_void {
    ptr::null()
}

/// A preset file's parameter values, if it is one (the state format).
fn read_preset_file(path: &CStr) -> Option<Vec<f64>> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::path::Path::new(std::ffi::OsStr::from_bytes(path.to_bytes()));
    let bytes = std::fs::read(path).ok()?;
    let n = SYNTH_PARAMS.len();
    if bytes.len() != 4 + 8 * n || &bytes[..4] != MAGIC {
        return None;
    }
    Some(
        bytes[4..]
            .chunks_exact(8)
            .map(|c| f64::from_bits(u64::from_le_bytes(c.try_into().unwrap_or([0; 8]))))
            .collect(),
    )
}

/// The bytes of a preset file setting the synth's gain, decay and wave
/// (the rest at their defaults), for tests.
pub fn preset_file(gain: f64, decay: f64, wave: f64) -> Vec<u8> {
    let mut b = MAGIC.to_vec();
    for p in &SYNTH_PARAMS {
        let v = match p.0 {
            synth_params::GAIN => gain,
            synth_params::DECAY => decay,
            synth_params::WAVE => wave,
            _ => p.3,
        };
        b.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    b
}

unsafe extern "C" fn pv_metadata(
    _p: *const clap_preset_discovery_provider,
    kind: u32,
    location: *const c_char,
    r: *const clap_preset_discovery_metadata_receiver,
) -> bool {
    let rc = &*r;
    let (Some(begin), Some(add_id)) = (rc.begin_preset, rc.add_plugin_id) else {
        return false;
    };
    let synth = clap_universal_plugin_id {
        abi: c"clap".as_ptr(),
        id: c"org.mpfree.test-synth".as_ptr(),
    };
    match kind {
        CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN => {
            for (key, name, category, ..) in BUILT_IN_PRESETS {
                let (Ok(key), Ok(name), Ok(category)) = (
                    std::ffi::CString::new(key),
                    std::ffi::CString::new(name),
                    std::ffi::CString::new(category),
                ) else {
                    return false;
                };
                if !begin(r, name.as_ptr(), key.as_ptr()) {
                    return true;
                }
                add_id(r, &synth);
                if let Some(f) = rc.add_creator {
                    f(r, c"MPFree".as_ptr());
                }
                if let Some(f) = rc.add_extra_info {
                    f(r, c"category".as_ptr(), category.as_ptr());
                }
            }
            true
        }
        CLAP_PRESET_DISCOVERY_LOCATION_FILE if !location.is_null() => {
            if read_preset_file(CStr::from_ptr(location)).is_none() {
                if let Some(f) = rc.on_error {
                    f(r, 0, c"not a test synth preset".as_ptr());
                }
                return false;
            }
            // A file holds one preset: no name (the host uses the file's)
            // and no load key.
            if begin(r, ptr::null(), ptr::null()) {
                add_id(r, &synth);
            }
            true
        }
        _ => false,
    }
}

static PRESET_LOAD: clap_plugin_preset_load = clap_plugin_preset_load {
    from_location: Some(preset_from_location),
};

unsafe extern "C" fn preset_from_location(
    p: *const clap_plugin,
    kind: u32,
    location: *const c_char,
    load_key: *const c_char,
) -> bool {
    let me = plug(p);
    let values: Option<Vec<(u32, f64)>> = match kind {
        CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN if !load_key.is_null() => {
            let key = CStr::from_ptr(load_key).to_str().unwrap_or("");
            BUILT_IN_PRESETS.iter().find(|b| b.0 == key).map(|b| {
                vec![
                    (synth_params::GAIN, b.3),
                    (synth_params::DECAY, b.4),
                    (synth_params::WAVE, b.5),
                ]
            })
        }
        CLAP_PRESET_DISCOVERY_LOCATION_FILE if !location.is_null() => {
            read_preset_file(CStr::from_ptr(location)).map(|v| {
                SYNTH_PARAMS
                    .iter()
                    .zip(v)
                    .filter(|(d, _)| d.6 & CLAP_PARAM_IS_READONLY == 0)
                    .map(|(d, v)| (d.0, v))
                    .collect()
            })
        }
        _ => None,
    };
    let host = me.host;
    let get = (*host).get_extension;
    let ext = |id: &[u8]| get.map_or(ptr::null(), |g| g(host, id.as_ptr() as *const c_char));
    let pl = ext(CLAP_EXT_PRESET_LOAD) as *const clap_host_preset_load;
    let Some(values) = values else {
        if !pl.is_null() {
            if let Some(f) = (*pl).on_error {
                f(
                    host,
                    kind,
                    location,
                    load_key,
                    0,
                    c"no such preset".as_ptr(),
                );
            }
        }
        return false;
    };
    for (id, v) in values {
        me.set(id, v);
    }
    if !pl.is_null() {
        if let Some(f) = (*pl).loaded {
            f(host, kind, location, load_key);
        }
    }
    let params = ext(CLAP_EXT_PARAMS) as *const clap_host_params;
    if !params.is_null() {
        if let Some(f) = (*params).rescan {
            f(host, CLAP_PARAM_RESCAN_VALUES);
        }
    }
    true
}
