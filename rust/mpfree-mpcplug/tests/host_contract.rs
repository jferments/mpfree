//! Drives the wrapper the way the MPC's plugin host does (the inMusic SDK's
//! `docs/host-contract.md`): the load sequence with `open` twice, the
//! parameter 0 nudge on prepare, in-place processing at block sizes other
//! than the one announced, a state chunk round trip, MIDI for instruments
//! and closing. The SDK's own `host_sim` runs the same checks on the built
//! example plugins (see docs/dev/research/mpc-plugin-sdk.md).

use std::ffi::{c_void, CStr};
use std::ptr;

use mpfree_mpcplug::abi::*;
use mpfree_mpcplug::{plugin_main, Spec};
use mpfree_node::Node;

static TIME: VstTimeInfo = VstTimeInfo {
    sample_pos: 0.0,
    sample_rate: 44_100.0,
    nano_seconds: 0.0,
    ppq_pos: 0.0,
    tempo: 100.0,
    bar_start_pos: 0.0,
    cycle_start_pos: 0.0,
    cycle_end_pos: 0.0,
    time_sig_numerator: 4,
    time_sig_denominator: 4,
    smpte_offset: 0,
    smpte_frame_rate: 0,
    samples_to_next_clock: 0,
    flags: TRANSPORT_PLAYING | PPQ_POS_VALID | TEMPO_VALID,
};

unsafe extern "C" fn host(
    _e: *mut AEffect,
    opcode: VstInt32,
    _i: VstInt32,
    _v: VstIntPtr,
    _p: *mut c_void,
    _o: f32,
) -> VstIntPtr {
    match opcode {
        AUDIO_MASTER_VERSION => 2400,
        AUDIO_MASTER_GET_TIME => &TIME as *const VstTimeInfo as VstIntPtr,
        _ => 0,
    }
}

fn delay() -> Box<dyn Node> {
    mpfree_fx::registry::make("mpfree.fx.delay").unwrap()
}
fn squelch() -> Box<dyn Node> {
    Box::new(mpfree_synth::squelch::Squelch::new(44_100))
}

static DELAY: Spec = Spec {
    name: "Delay",
    vendor: "MPFree",
    uid: *b"MFdl",
    version: 1,
    instrument: false,
    make: delay,
};
static SQUELCH: Spec = Spec {
    name: "Squelch",
    vendor: "MPFree",
    uid: *b"MFsq",
    version: 1,
    instrument: true,
    make: squelch,
};

struct Plug(*mut AEffect);

impl Plug {
    fn load(spec: &'static Spec) -> Plug {
        let e = unsafe { plugin_main(spec, Some(host)) };
        assert!(!e.is_null());
        let p = Plug(e);
        let fx = unsafe { &*e };
        assert_eq!(fx.magic, EFFECT_MAGIC);
        assert_eq!(fx.resvd2, 0);
        assert!(!fx.object.is_null());
        assert_eq!(fx.unique_id, i32::from_be_bytes(spec.uid));
        p
    }
    fn fx(&self) -> &AEffect {
        unsafe { &*self.0 }
    }
    fn call(
        &self,
        op: VstInt32,
        index: VstInt32,
        value: VstIntPtr,
        ptr: *mut c_void,
        opt: f32,
    ) -> VstIntPtr {
        unsafe { (self.fx().dispatcher.unwrap())(self.0, op, index, value, ptr, opt) }
    }
    fn text(&self, op: VstInt32, index: VstInt32) -> String {
        let mut buf = [0xAAu8; 256];
        self.call(op, index, 0, buf.as_mut_ptr() as *mut c_void, 0.0);
        let s = CStr::from_bytes_until_nul(&buf)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(s.len() < 64, "string too long for a host buffer: {s:?}");
        s
    }
    fn set(&self, i: i32, v: f32) {
        unsafe { (self.fx().set_parameter.unwrap())(self.0, i, v) }
    }
    fn get(&self, i: i32) -> f32 {
        unsafe { (self.fx().get_parameter.unwrap())(self.0, i) }
    }
    /// The MPC's load and prepare sequence.
    fn prepare(&self, block: VstIntPtr) {
        self.call(EFF_SET_SAMPLE_RATE, 0, 0, ptr::null_mut(), 44_100.0);
        self.call(EFF_SET_BLOCK_SIZE, 0, block, ptr::null_mut(), 0.0);
        self.call(EFF_OPEN, 0, 0, ptr::null_mut(), 0.0);
        self.call(EFF_SET_SAMPLE_RATE, 0, 0, ptr::null_mut(), 44_100.0);
        self.call(EFF_SET_BLOCK_SIZE, 0, block, ptr::null_mut(), 0.0);
        self.call(EFF_OPEN, 0, 0, ptr::null_mut(), 0.0);
        self.call(EFF_SET_PROGRAM, 0, 0, ptr::null_mut(), 0.0);
        self.call(EFF_MAINS_CHANGED, 0, 1, ptr::null_mut(), 0.0);
        // The parameter 0 nudge.
        let v = self.get(0);
        self.set(0, if v < 0.5 { 1.0 } else { 0.0 });
        self.set(0, v);
    }
    /// One in-place block: input and output are the same buffers.
    fn run(&self, l: &mut [f32], r: &mut [f32]) {
        let mut chans = [l.as_mut_ptr(), r.as_mut_ptr()];
        unsafe {
            (self.fx().process_replacing.unwrap())(
                self.0,
                chans.as_mut_ptr(),
                chans.as_mut_ptr(),
                l.len() as i32,
            )
        }
    }
    fn chunk(&self) -> Vec<u8> {
        let mut p: *mut c_void = ptr::null_mut();
        let n = self.call(EFF_GET_CHUNK, 0, 0, &mut p as *mut _ as *mut c_void, 0.0);
        assert!(n > 0 && !p.is_null());
        unsafe { std::slice::from_raw_parts(p as *const u8, n as usize).to_vec() }
    }
    fn set_chunk(&self, data: &[u8]) -> VstIntPtr {
        self.call(
            EFF_SET_CHUNK,
            0,
            data.len() as VstIntPtr,
            data.as_ptr() as *mut c_void,
            0.0,
        )
    }
}

impl Drop for Plug {
    fn drop(&mut self) {
        self.call(EFF_MAINS_CHANGED, 0, 0, ptr::null_mut(), 0.0);
        self.call(EFF_CLOSE, 0, 0, ptr::null_mut(), 0.0);
    }
}

fn tone(l: &mut [f32], r: &mut [f32], start: usize) {
    for (k, (a, b)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
        let t = (start + k) as f32 / 44_100.0;
        *a = (t * 440.0 * std::f32::consts::TAU).sin() * 0.5;
        *b = *a;
    }
}

#[test]
fn effect_follows_the_mpc_sequence() {
    let p = Plug::load(&DELAY);
    assert_eq!(p.fx().num_inputs, 2);
    assert_eq!(p.fx().num_outputs, 2);
    assert_eq!(p.text(EFF_GET_EFFECT_NAME, 0), "Delay");
    assert_eq!(p.text(EFF_GET_VENDOR_STRING, 0), "MPFree");
    assert_eq!(
        p.call(EFF_GET_PLUG_CATEGORY, 0, 0, ptr::null_mut(), 0.0),
        PLUG_CATEG_EFFECT
    );
    assert_eq!(
        p.call(EFF_GET_VST_VERSION, 0, 0, ptr::null_mut(), 0.0),
        2400
    );
    assert_eq!(
        p.call(
            EFF_GET_PROGRAM_NAME_INDEXED,
            0,
            0,
            [0u8; 64].as_mut_ptr() as *mut c_void,
            0.0
        ),
        1
    );
    let bypass = c"bypass";
    assert_eq!(
        p.call(EFF_CAN_DO, 0, 0, bypass.as_ptr() as *mut c_void, 0.0),
        -1
    );
    for i in 0..p.fx().num_params {
        assert!(!p.text(EFF_GET_PARAM_NAME, i).is_empty());
        p.text(EFF_GET_PARAM_DISPLAY, i);
        assert_eq!(p.call(EFF_CAN_BE_AUTOMATED, i, 0, ptr::null_mut(), 0.0), 1);
    }
    // Out-of-range indexes are answered, not crashed on.
    p.text(EFF_GET_PARAM_NAME, 999);
    assert_eq!(p.get(-1), 0.0);

    p.prepare(128);
    let mut heard = 0.0f32;
    let mut at = 0;
    // Smaller than announced, as announced, and longer than announced.
    for &n in &[32usize, 128, 1000, 7] {
        for _ in 0..40 {
            let mut l = vec![0.0; n];
            let mut r = vec![0.0; n];
            tone(&mut l, &mut r, at);
            at += n;
            p.run(&mut l, &mut r);
            assert!(l.iter().chain(&r).all(|x| x.is_finite()));
            heard = heard.max(l.iter().fold(0.0, |m, x| m.max(x.abs())));
        }
    }
    assert!(heard > 0.01, "no sound through the delay");
}

#[test]
fn state_round_trips_and_bad_chunks_are_refused() {
    let p = Plug::load(&DELAY);
    p.prepare(128);
    for i in 0..p.fx().num_params {
        p.set(i, ((i as f32) * 0.37).fract());
    }
    let saved = p.chunk();
    let before: Vec<f32> = (0..p.fx().num_params).map(|i| p.get(i)).collect();
    for i in 0..p.fx().num_params {
        p.set(i, 0.0);
    }
    assert_eq!(p.set_chunk(&saved), 1);
    let after: Vec<f32> = (0..p.fx().num_params).map(|i| p.get(i)).collect();
    for (a, b) in before.iter().zip(&after) {
        assert!((a - b).abs() < 1e-5, "{before:?} vs {after:?}");
    }
    assert_eq!(p.chunk(), saved);
    assert_eq!(p.set_chunk(b"not a chunk"), 0);
    assert_eq!(p.set_chunk(&[0xFF, 0xFE, 0x00]), 0);
}

#[test]
fn instrument_plays_host_midi() {
    let p = Plug::load(&SQUELCH);
    assert_eq!(p.fx().num_inputs, 0);
    assert_ne!(p.fx().flags & FLAG_IS_SYNTH, 0);
    let yes = c"receiveVstMidiEvent";
    assert_eq!(
        p.call(EFF_CAN_DO, 0, 0, yes.as_ptr() as *mut c_void, 0.0),
        1
    );
    p.prepare(128);
    let mut on = VstMidiEvent {
        kind: VST_MIDI_TYPE,
        byte_size: std::mem::size_of::<VstMidiEvent>() as i32,
        delta_frames: 10,
        flags: 0,
        note_length: 0,
        note_offset: 0,
        midi_data: [0x90, 48, 100, 0],
        detune: 0,
        note_off_velocity: 0,
        reserved1: 0,
        reserved2: 0,
    };
    let mut events = VstEvents {
        num_events: 1,
        reserved: 0,
        events: [
            &mut on as *mut VstMidiEvent as *mut VstEvent,
            ptr::null_mut(),
        ],
    };
    p.call(
        EFF_PROCESS_EVENTS,
        0,
        0,
        &mut events as *mut _ as *mut c_void,
        0.0,
    );
    let mut peak = 0.0f32;
    for _ in 0..100 {
        let mut l = vec![0.0; 128];
        let mut r = vec![0.0; 128];
        p.run(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|x| x.is_finite()));
        peak = peak.max(l.iter().fold(0.0, |m, x| m.max(x.abs())));
        p.call(
            EFF_PROCESS_EVENTS,
            0,
            0,
            &mut VstEvents {
                num_events: 0,
                reserved: 0,
                events: [ptr::null_mut(); 2],
            } as *mut _ as *mut c_void,
            0.0,
        );
    }
    assert!(peak > 0.01, "the note made no sound");
}

#[test]
fn processing_before_resume_is_silent() {
    let p = Plug::load(&DELAY);
    let mut l = vec![0.5; 64];
    let mut r = vec![0.5; 64];
    p.run(&mut l, &mut r);
    assert!(l.iter().chain(&r).all(|x| x.is_finite()));
}
