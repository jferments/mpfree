//! The parts of the VST 2.4 binary interface the MPC's plugin host uses.
//!
//! Written from the public description of the interface in the inMusic MPC
//! Plugin Builder SDK (`framework/mpc_vst2_abi.h`, MIT), which is itself an
//! independent description; no Steinberg header is used. Field order and
//! sizes are the ABI: every struct here is `repr(C)`.

use std::ffi::c_void;

pub type VstInt32 = i32;
pub type VstIntPtr = isize;

/// The host's callback into the plugin's world (`audioMaster`).
pub type HostCallback = unsafe extern "C" fn(
    effect: *mut AEffect,
    opcode: VstInt32,
    index: VstInt32,
    value: VstIntPtr,
    ptr: *mut c_void,
    opt: f32,
) -> VstIntPtr;

pub type DispatcherProc = unsafe extern "C" fn(
    effect: *mut AEffect,
    opcode: VstInt32,
    index: VstInt32,
    value: VstIntPtr,
    ptr: *mut c_void,
    opt: f32,
) -> VstIntPtr;
pub type ProcessProc = unsafe extern "C" fn(
    effect: *mut AEffect,
    inputs: *mut *mut f32,
    outputs: *mut *mut f32,
    frames: VstInt32,
);
pub type ProcessDoubleProc = unsafe extern "C" fn(
    effect: *mut AEffect,
    inputs: *mut *mut f64,
    outputs: *mut *mut f64,
    frames: VstInt32,
);
pub type SetParameterProc = unsafe extern "C" fn(effect: *mut AEffect, index: VstInt32, value: f32);
pub type GetParameterProc = unsafe extern "C" fn(effect: *mut AEffect, index: VstInt32) -> f32;

/// `'VstP'`, checked first by the host.
pub const EFFECT_MAGIC: VstInt32 = 0x5673_7450;

/// The one struct host and plugin share.
#[repr(C)]
pub struct AEffect {
    pub magic: VstInt32,
    pub dispatcher: Option<DispatcherProc>,
    pub process: Option<ProcessProc>,
    pub set_parameter: Option<SetParameterProc>,
    pub get_parameter: Option<GetParameterProc>,
    pub num_programs: VstInt32,
    pub num_params: VstInt32,
    pub num_inputs: VstInt32,
    pub num_outputs: VstInt32,
    pub flags: VstInt32,
    pub resvd1: VstIntPtr,
    pub resvd2: VstIntPtr,
    pub initial_delay: VstInt32,
    pub real_qualities: VstInt32,
    pub off_qualities: VstInt32,
    pub io_ratio: f32,
    pub object: *mut c_void,
    pub user: *mut c_void,
    pub unique_id: VstInt32,
    pub version: VstInt32,
    pub process_replacing: Option<ProcessProc>,
    pub process_double_replacing: Option<ProcessDoubleProc>,
    pub future: [u8; 56],
}

pub const FLAG_CAN_REPLACING: VstInt32 = 1 << 4;
pub const FLAG_PROGRAM_CHUNKS: VstInt32 = 1 << 5;
pub const FLAG_IS_SYNTH: VstInt32 = 1 << 8;

// Host -> plugin opcodes.
pub const EFF_OPEN: VstInt32 = 0;
pub const EFF_CLOSE: VstInt32 = 1;
pub const EFF_SET_PROGRAM: VstInt32 = 2;
pub const EFF_GET_PROGRAM: VstInt32 = 3;
pub const EFF_GET_PROGRAM_NAME: VstInt32 = 5;
pub const EFF_GET_PARAM_LABEL: VstInt32 = 6;
pub const EFF_GET_PARAM_DISPLAY: VstInt32 = 7;
pub const EFF_GET_PARAM_NAME: VstInt32 = 8;
pub const EFF_SET_SAMPLE_RATE: VstInt32 = 10;
pub const EFF_SET_BLOCK_SIZE: VstInt32 = 11;
pub const EFF_MAINS_CHANGED: VstInt32 = 12;
pub const EFF_GET_CHUNK: VstInt32 = 23;
pub const EFF_SET_CHUNK: VstInt32 = 24;
pub const EFF_PROCESS_EVENTS: VstInt32 = 25;
pub const EFF_CAN_BE_AUTOMATED: VstInt32 = 26;
pub const EFF_GET_PROGRAM_NAME_INDEXED: VstInt32 = 29;
pub const EFF_GET_PLUG_CATEGORY: VstInt32 = 35;
pub const EFF_GET_EFFECT_NAME: VstInt32 = 45;
pub const EFF_GET_VENDOR_STRING: VstInt32 = 47;
pub const EFF_GET_PRODUCT_STRING: VstInt32 = 48;
pub const EFF_GET_VENDOR_VERSION: VstInt32 = 49;
pub const EFF_CAN_DO: VstInt32 = 51;
pub const EFF_GET_TAIL_SIZE: VstInt32 = 52;
pub const EFF_GET_VST_VERSION: VstInt32 = 58;
pub const EFF_START_PROCESS: VstInt32 = 71;
pub const EFF_STOP_PROCESS: VstInt32 = 72;

// Plugin -> host opcodes.
pub const AUDIO_MASTER_AUTOMATE: VstInt32 = 0;
pub const AUDIO_MASTER_VERSION: VstInt32 = 1;
pub const AUDIO_MASTER_CURRENT_ID: VstInt32 = 2;
pub const AUDIO_MASTER_IDLE: VstInt32 = 3;
pub const AUDIO_MASTER_GET_TIME: VstInt32 = 7;
pub const AUDIO_MASTER_PROCESS_EVENTS: VstInt32 = 8;
pub const AUDIO_MASTER_IO_CHANGED: VstInt32 = 13;
pub const AUDIO_MASTER_SIZE_WINDOW: VstInt32 = 15;
pub const AUDIO_MASTER_GET_SAMPLE_RATE: VstInt32 = 16;
pub const AUDIO_MASTER_GET_BLOCK_SIZE: VstInt32 = 17;
pub const AUDIO_MASTER_GET_CURRENT_PROCESS_LEVEL: VstInt32 = 23;
pub const AUDIO_MASTER_GET_VENDOR_STRING: VstInt32 = 32;
pub const AUDIO_MASTER_GET_PRODUCT_STRING: VstInt32 = 33;
pub const AUDIO_MASTER_GET_VENDOR_VERSION: VstInt32 = 34;
pub const AUDIO_MASTER_CAN_DO: VstInt32 = 37;
pub const AUDIO_MASTER_GET_LANGUAGE: VstInt32 = 38;
pub const AUDIO_MASTER_GET_DIRECTORY: VstInt32 = 41;
pub const AUDIO_MASTER_UPDATE_DISPLAY: VstInt32 = 42;
pub const AUDIO_MASTER_BEGIN_EDIT: VstInt32 = 43;
pub const AUDIO_MASTER_END_EDIT: VstInt32 = 44;

pub const VST_VERSION: VstIntPtr = 2400;
pub const MAX_PROG_NAME_LEN: usize = 24;
pub const MAX_VENDOR_STR_LEN: usize = 64;
pub const MAX_EFFECT_NAME_LEN: usize = 32;
/// The most this crate writes into a parameter string buffer, NUL included
/// (the SDK's own framework uses the same bound).
pub const PARAM_STR_BYTES: usize = 32;

pub const PLUG_CATEG_EFFECT: VstIntPtr = 1;
pub const PLUG_CATEG_SYNTH: VstIntPtr = 2;

pub const VST_MIDI_TYPE: VstInt32 = 1;

#[repr(C)]
pub struct VstEvent {
    pub kind: VstInt32,
    pub byte_size: VstInt32,
    pub delta_frames: VstInt32,
    pub flags: VstInt32,
    pub data: [u8; 16],
}

#[repr(C)]
pub struct VstMidiEvent {
    pub kind: VstInt32,
    pub byte_size: VstInt32,
    pub delta_frames: VstInt32,
    pub flags: VstInt32,
    pub note_length: VstInt32,
    pub note_offset: VstInt32,
    pub midi_data: [u8; 4],
    pub detune: i8,
    pub note_off_velocity: u8,
    pub reserved1: u8,
    pub reserved2: u8,
}

/// `events` holds `num_events` pointers in practice.
#[repr(C)]
pub struct VstEvents {
    pub num_events: VstInt32,
    pub reserved: VstIntPtr,
    pub events: [*mut VstEvent; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VstTimeInfo {
    pub sample_pos: f64,
    pub sample_rate: f64,
    pub nano_seconds: f64,
    pub ppq_pos: f64,
    pub tempo: f64,
    pub bar_start_pos: f64,
    pub cycle_start_pos: f64,
    pub cycle_end_pos: f64,
    pub time_sig_numerator: VstInt32,
    pub time_sig_denominator: VstInt32,
    pub smpte_offset: VstInt32,
    pub smpte_frame_rate: VstInt32,
    pub samples_to_next_clock: VstInt32,
    pub flags: VstInt32,
}

pub const TRANSPORT_CHANGED: VstInt32 = 1;
pub const TRANSPORT_PLAYING: VstInt32 = 1 << 1;
pub const PPQ_POS_VALID: VstInt32 = 1 << 9;
pub const TEMPO_VALID: VstInt32 = 1 << 10;
pub const BARS_VALID: VstInt32 = 1 << 11;
pub const TIME_SIG_VALID: VstInt32 = 1 << 13;
