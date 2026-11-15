//! The part of the CLAP 1.2 C ABI that MPFree uses, written out by hand
//! from the MIT-licensed headers (`clap/include/clap`, free-audio/clap
//! 1.2.2). Names follow the C ones. Function pointers are `Option`s because
//! a plugin may leave any of them null.
#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_void};

pub type clap_id = u32;
pub const CLAP_INVALID_ID: clap_id = u32::MAX;
pub const CLAP_NAME_SIZE: usize = 256;
pub const CLAP_PATH_SIZE: usize = 1024;
pub const CLAP_BEATTIME_FACTOR: f64 = (1u64 << 31) as f64;
pub const CLAP_SECTIME_FACTOR: f64 = (1u64 << 31) as f64;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct clap_version {
    pub major: u32,
    pub minor: u32,
    pub revision: u32,
}

pub const CLAP_VERSION: clap_version = clap_version {
    major: 1,
    minor: 2,
    revision: 2,
};

/// The rule from `version.h`: every 1.x plugin talks to every 1.x host.
pub fn clap_version_is_compatible(v: clap_version) -> bool {
    v.major >= 1
}

// ------------------------------------------------------------------ entry

#[repr(C)]
pub struct clap_plugin_entry {
    pub clap_version: clap_version,
    pub init: Option<unsafe extern "C" fn(plugin_path: *const c_char) -> bool>,
    pub deinit: Option<unsafe extern "C" fn()>,
    pub get_factory: Option<unsafe extern "C" fn(factory_id: *const c_char) -> *const c_void>,
}

pub const CLAP_PLUGIN_FACTORY_ID: &[u8] = b"clap.plugin-factory\0";

#[repr(C)]
pub struct clap_plugin_factory {
    pub get_plugin_count: Option<unsafe extern "C" fn(factory: *const clap_plugin_factory) -> u32>,
    pub get_plugin_descriptor: Option<
        unsafe extern "C" fn(
            factory: *const clap_plugin_factory,
            index: u32,
        ) -> *const clap_plugin_descriptor,
    >,
    pub create_plugin: Option<
        unsafe extern "C" fn(
            factory: *const clap_plugin_factory,
            host: *const clap_host,
            plugin_id: *const c_char,
        ) -> *const clap_plugin,
    >,
}

// ------------------------------------------------------------------ plugin

#[repr(C)]
pub struct clap_plugin_descriptor {
    pub clap_version: clap_version,
    pub id: *const c_char,
    pub name: *const c_char,
    pub vendor: *const c_char,
    pub url: *const c_char,
    pub manual_url: *const c_char,
    pub support_url: *const c_char,
    pub version: *const c_char,
    pub description: *const c_char,
    /// Null-terminated array of feature strings.
    pub features: *const *const c_char,
}

#[repr(C)]
pub struct clap_plugin {
    pub desc: *const clap_plugin_descriptor,
    pub plugin_data: *mut c_void,
    pub init: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> bool>,
    pub destroy: Option<unsafe extern "C" fn(plugin: *const clap_plugin)>,
    pub activate: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            sample_rate: f64,
            min_frames: u32,
            max_frames: u32,
        ) -> bool,
    >,
    pub deactivate: Option<unsafe extern "C" fn(plugin: *const clap_plugin)>,
    pub start_processing: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> bool>,
    pub stop_processing: Option<unsafe extern "C" fn(plugin: *const clap_plugin)>,
    pub reset: Option<unsafe extern "C" fn(plugin: *const clap_plugin)>,
    pub process: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            process: *const clap_process,
        ) -> clap_process_status,
    >,
    pub get_extension: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, id: *const c_char) -> *const c_void,
    >,
    pub on_main_thread: Option<unsafe extern "C" fn(plugin: *const clap_plugin)>,
}

// ------------------------------------------------------------------ host

#[repr(C)]
pub struct clap_host {
    pub clap_version: clap_version,
    pub host_data: *mut c_void,
    pub name: *const c_char,
    pub vendor: *const c_char,
    pub url: *const c_char,
    pub version: *const c_char,
    pub get_extension: Option<
        unsafe extern "C" fn(host: *const clap_host, extension_id: *const c_char) -> *const c_void,
    >,
    pub request_restart: Option<unsafe extern "C" fn(host: *const clap_host)>,
    pub request_process: Option<unsafe extern "C" fn(host: *const clap_host)>,
    pub request_callback: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

// ------------------------------------------------------------------ process

pub type clap_process_status = i32;
pub const CLAP_PROCESS_ERROR: clap_process_status = 0;
pub const CLAP_PROCESS_CONTINUE: clap_process_status = 1;
pub const CLAP_PROCESS_CONTINUE_IF_NOT_QUIET: clap_process_status = 2;
pub const CLAP_PROCESS_TAIL: clap_process_status = 3;
pub const CLAP_PROCESS_SLEEP: clap_process_status = 4;

#[repr(C)]
pub struct clap_audio_buffer {
    pub data32: *mut *mut f32,
    pub data64: *mut *mut f64,
    pub channel_count: u32,
    pub latency: u32,
    pub constant_mask: u64,
}

#[repr(C)]
pub struct clap_process {
    pub steady_time: i64,
    pub frames_count: u32,
    pub transport: *const clap_event_transport,
    pub audio_inputs: *const clap_audio_buffer,
    pub audio_outputs: *mut clap_audio_buffer,
    pub audio_inputs_count: u32,
    pub audio_outputs_count: u32,
    pub in_events: *const clap_input_events,
    pub out_events: *const clap_output_events,
}

// ------------------------------------------------------------------ events

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct clap_event_header {
    pub size: u32,
    pub time: u32,
    pub space_id: u16,
    pub type_: u16,
    pub flags: u32,
}

pub const CLAP_CORE_EVENT_SPACE_ID: u16 = 0;
pub const CLAP_EVENT_IS_LIVE: u32 = 1 << 0;

pub const CLAP_EVENT_NOTE_ON: u16 = 0;
pub const CLAP_EVENT_NOTE_OFF: u16 = 1;
pub const CLAP_EVENT_NOTE_CHOKE: u16 = 2;
pub const CLAP_EVENT_NOTE_END: u16 = 3;
pub const CLAP_EVENT_NOTE_EXPRESSION: u16 = 4;
pub const CLAP_EVENT_PARAM_VALUE: u16 = 5;
pub const CLAP_EVENT_PARAM_MOD: u16 = 6;
pub const CLAP_EVENT_PARAM_GESTURE_BEGIN: u16 = 7;
pub const CLAP_EVENT_PARAM_GESTURE_END: u16 = 8;
pub const CLAP_EVENT_TRANSPORT: u16 = 9;
pub const CLAP_EVENT_MIDI: u16 = 10;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct clap_event_note {
    pub header: clap_event_header,
    pub note_id: i32,
    pub port_index: i16,
    pub channel: i16,
    pub key: i16,
    pub velocity: f64,
}

pub const CLAP_NOTE_EXPRESSION_VOLUME: i32 = 0;
pub const CLAP_NOTE_EXPRESSION_PAN: i32 = 1;
pub const CLAP_NOTE_EXPRESSION_TUNING: i32 = 2;
pub const CLAP_NOTE_EXPRESSION_VIBRATO: i32 = 3;
pub const CLAP_NOTE_EXPRESSION_EXPRESSION: i32 = 4;
pub const CLAP_NOTE_EXPRESSION_BRIGHTNESS: i32 = 5;
pub const CLAP_NOTE_EXPRESSION_PRESSURE: i32 = 6;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct clap_event_note_expression {
    pub header: clap_event_header,
    pub expression_id: i32,
    pub note_id: i32,
    pub port_index: i16,
    pub channel: i16,
    pub key: i16,
    pub value: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct clap_event_param_value {
    pub header: clap_event_header,
    pub param_id: clap_id,
    pub cookie: *mut c_void,
    pub note_id: i32,
    pub port_index: i16,
    pub channel: i16,
    pub key: i16,
    pub value: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct clap_event_param_gesture {
    pub header: clap_event_header,
    pub param_id: clap_id,
}

pub const CLAP_TRANSPORT_HAS_TEMPO: u32 = 1 << 0;
pub const CLAP_TRANSPORT_HAS_BEATS_TIMELINE: u32 = 1 << 1;
pub const CLAP_TRANSPORT_HAS_SECONDS_TIMELINE: u32 = 1 << 2;
pub const CLAP_TRANSPORT_HAS_TIME_SIGNATURE: u32 = 1 << 3;
pub const CLAP_TRANSPORT_IS_PLAYING: u32 = 1 << 4;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct clap_event_transport {
    pub header: clap_event_header,
    pub flags: u32,
    pub song_pos_beats: i64,
    pub song_pos_seconds: i64,
    pub tempo: f64,
    pub tempo_inc: f64,
    pub loop_start_beats: i64,
    pub loop_end_beats: i64,
    pub loop_start_seconds: i64,
    pub loop_end_seconds: i64,
    pub bar_start: i64,
    pub bar_number: i32,
    pub tsig_num: u16,
    pub tsig_denom: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct clap_event_midi {
    pub header: clap_event_header,
    pub port_index: u16,
    pub data: [u8; 3],
}

#[repr(C)]
pub struct clap_input_events {
    pub ctx: *mut c_void,
    pub size: Option<unsafe extern "C" fn(list: *const clap_input_events) -> u32>,
    pub get: Option<
        unsafe extern "C" fn(
            list: *const clap_input_events,
            index: u32,
        ) -> *const clap_event_header,
    >,
}

#[repr(C)]
pub struct clap_output_events {
    pub ctx: *mut c_void,
    pub try_push: Option<
        unsafe extern "C" fn(
            list: *const clap_output_events,
            event: *const clap_event_header,
        ) -> bool,
    >,
}

// ------------------------------------------------------------------ streams

#[repr(C)]
pub struct clap_istream {
    pub ctx: *mut c_void,
    pub read: Option<
        unsafe extern "C" fn(stream: *const clap_istream, buffer: *mut c_void, size: u64) -> i64,
    >,
}

#[repr(C)]
pub struct clap_ostream {
    pub ctx: *mut c_void,
    pub write: Option<
        unsafe extern "C" fn(stream: *const clap_ostream, buffer: *const c_void, size: u64) -> i64,
    >,
}

// ------------------------------------------------------------------ extensions

pub const CLAP_EXT_PARAMS: &[u8] = b"clap.params\0";
pub const CLAP_EXT_AUDIO_PORTS: &[u8] = b"clap.audio-ports\0";
pub const CLAP_EXT_NOTE_PORTS: &[u8] = b"clap.note-ports\0";
pub const CLAP_EXT_STATE: &[u8] = b"clap.state\0";
pub const CLAP_EXT_LATENCY: &[u8] = b"clap.latency\0";
pub const CLAP_EXT_TAIL: &[u8] = b"clap.tail\0";
pub const CLAP_EXT_LOG: &[u8] = b"clap.log\0";
pub const CLAP_EXT_THREAD_CHECK: &[u8] = b"clap.thread-check\0";

pub const CLAP_PARAM_IS_STEPPED: u32 = 1 << 0;
pub const CLAP_PARAM_IS_PERIODIC: u32 = 1 << 1;
pub const CLAP_PARAM_IS_HIDDEN: u32 = 1 << 2;
pub const CLAP_PARAM_IS_READONLY: u32 = 1 << 3;
pub const CLAP_PARAM_IS_BYPASS: u32 = 1 << 4;
pub const CLAP_PARAM_IS_AUTOMATABLE: u32 = 1 << 5;
pub const CLAP_PARAM_IS_MODULATABLE: u32 = 1 << 10;
pub const CLAP_PARAM_IS_ENUM: u32 = 1 << 16;

#[repr(C)]
pub struct clap_param_info {
    pub id: clap_id,
    pub flags: u32,
    pub cookie: *mut c_void,
    pub name: [c_char; CLAP_NAME_SIZE],
    pub module: [c_char; CLAP_PATH_SIZE],
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
}

#[repr(C)]
pub struct clap_plugin_params {
    pub count: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> u32>,
    pub get_info: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            index: u32,
            info: *mut clap_param_info,
        ) -> bool,
    >,
    pub get_value: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, id: clap_id, out: *mut f64) -> bool,
    >,
    pub value_to_text: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            id: clap_id,
            value: f64,
            out: *mut c_char,
            capacity: u32,
        ) -> bool,
    >,
    pub text_to_value: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            id: clap_id,
            text: *const c_char,
            out: *mut f64,
        ) -> bool,
    >,
    pub flush: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            input: *const clap_input_events,
            output: *const clap_output_events,
        ),
    >,
}

/// `clap_host_params::rescan` flags.
pub const CLAP_PARAM_RESCAN_VALUES: u32 = 1 << 0;
pub const CLAP_PARAM_RESCAN_TEXT: u32 = 1 << 1;
pub const CLAP_PARAM_RESCAN_INFO: u32 = 1 << 2;
pub const CLAP_PARAM_RESCAN_ALL: u32 = 1 << 3;

#[repr(C)]
pub struct clap_host_params {
    pub rescan: Option<unsafe extern "C" fn(host: *const clap_host, flags: u32)>,
    pub clear: Option<unsafe extern "C" fn(host: *const clap_host, id: clap_id, flags: u32)>,
    pub request_flush: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

pub const CLAP_AUDIO_PORT_IS_MAIN: u32 = 1 << 0;

#[repr(C)]
pub struct clap_audio_port_info {
    pub id: clap_id,
    pub name: [c_char; CLAP_NAME_SIZE],
    pub flags: u32,
    pub channel_count: u32,
    pub port_type: *const c_char,
    pub in_place_pair: clap_id,
}

#[repr(C)]
pub struct clap_plugin_audio_ports {
    pub count: Option<unsafe extern "C" fn(plugin: *const clap_plugin, is_input: bool) -> u32>,
    pub get: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            index: u32,
            is_input: bool,
            info: *mut clap_audio_port_info,
        ) -> bool,
    >,
}

#[repr(C)]
pub struct clap_host_audio_ports {
    pub is_rescan_flag_supported:
        Option<unsafe extern "C" fn(host: *const clap_host, flag: u32) -> bool>,
    pub rescan: Option<unsafe extern "C" fn(host: *const clap_host, flags: u32)>,
}

pub const CLAP_NOTE_DIALECT_CLAP: u32 = 1 << 0;
pub const CLAP_NOTE_DIALECT_MIDI: u32 = 1 << 1;
pub const CLAP_NOTE_DIALECT_MIDI_MPE: u32 = 1 << 2;
pub const CLAP_NOTE_DIALECT_MIDI2: u32 = 1 << 3;

#[repr(C)]
pub struct clap_note_port_info {
    pub id: clap_id,
    pub supported_dialects: u32,
    pub preferred_dialect: u32,
    pub name: [c_char; CLAP_NAME_SIZE],
}

#[repr(C)]
pub struct clap_plugin_note_ports {
    pub count: Option<unsafe extern "C" fn(plugin: *const clap_plugin, is_input: bool) -> u32>,
    pub get: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            index: u32,
            is_input: bool,
            info: *mut clap_note_port_info,
        ) -> bool,
    >,
}

#[repr(C)]
pub struct clap_host_note_ports {
    pub supported_dialects: Option<unsafe extern "C" fn(host: *const clap_host) -> u32>,
    pub rescan: Option<unsafe extern "C" fn(host: *const clap_host, flags: u32)>,
}

#[repr(C)]
pub struct clap_plugin_state {
    pub save: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool,
    >,
    pub load: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, stream: *const clap_istream) -> bool,
    >,
}

#[repr(C)]
pub struct clap_host_state {
    pub mark_dirty: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

#[repr(C)]
pub struct clap_plugin_latency {
    pub get: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> u32>,
}

#[repr(C)]
pub struct clap_host_latency {
    pub changed: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

#[repr(C)]
pub struct clap_plugin_tail {
    pub get: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> u32>,
}

#[repr(C)]
pub struct clap_host_tail {
    pub changed: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

pub const CLAP_LOG_DEBUG: i32 = 0;
pub const CLAP_LOG_INFO: i32 = 1;
pub const CLAP_LOG_WARNING: i32 = 2;
pub const CLAP_LOG_ERROR: i32 = 3;
pub const CLAP_LOG_FATAL: i32 = 4;
pub const CLAP_LOG_HOST_MISBEHAVING: i32 = 5;
pub const CLAP_LOG_PLUGIN_MISBEHAVING: i32 = 6;

#[repr(C)]
pub struct clap_host_log {
    pub log:
        Option<unsafe extern "C" fn(host: *const clap_host, severity: i32, msg: *const c_char)>,
}

#[repr(C)]
pub struct clap_host_thread_check {
    pub is_main_thread: Option<unsafe extern "C" fn(host: *const clap_host) -> bool>,
    pub is_audio_thread: Option<unsafe extern "C" fn(host: *const clap_host) -> bool>,
}

// ------------------------------------------------------------------ presets

/// The preset discovery factory (CLAP 1.2), from `clap_plugin_entry::get_factory`.
pub const CLAP_PRESET_DISCOVERY_FACTORY_ID: &[u8] = b"clap.preset-discovery-factory/2\0";
/// The same factory under its draft id, which older plugins answer to.
pub const CLAP_PRESET_DISCOVERY_FACTORY_ID_COMPAT: &[u8] =
    b"clap.preset-discovery-factory/draft-2\0";
pub const CLAP_EXT_PRESET_LOAD: &[u8] = b"clap.preset-load/2\0";
pub const CLAP_EXT_PRESET_LOAD_COMPAT: &[u8] = b"clap.preset-load.draft/2\0";

/// A location that is a file or a folder on disk.
pub const CLAP_PRESET_DISCOVERY_LOCATION_FILE: u32 = 0;
/// Presets inside the plugin itself; the location is null.
pub const CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN: u32 = 1;

pub const CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT: u32 = 1 << 0;
pub const CLAP_PRESET_DISCOVERY_IS_USER_CONTENT: u32 = 1 << 1;
pub const CLAP_PRESET_DISCOVERY_IS_DEMO_CONTENT: u32 = 1 << 2;
pub const CLAP_PRESET_DISCOVERY_IS_FAVORITE: u32 = 1 << 3;

#[repr(C)]
pub struct clap_universal_plugin_id {
    pub abi: *const c_char,
    pub id: *const c_char,
}

#[repr(C)]
pub struct clap_preset_discovery_metadata_receiver {
    pub receiver_data: *mut c_void,
    pub on_error: Option<
        unsafe extern "C" fn(
            receiver: *const clap_preset_discovery_metadata_receiver,
            os_error: i32,
            error_message: *const c_char,
        ),
    >,
    pub begin_preset: Option<
        unsafe extern "C" fn(
            receiver: *const clap_preset_discovery_metadata_receiver,
            name: *const c_char,
            load_key: *const c_char,
        ) -> bool,
    >,
    pub add_plugin_id: Option<
        unsafe extern "C" fn(
            receiver: *const clap_preset_discovery_metadata_receiver,
            plugin_id: *const clap_universal_plugin_id,
        ),
    >,
    pub set_soundpack_id: Option<
        unsafe extern "C" fn(
            receiver: *const clap_preset_discovery_metadata_receiver,
            soundpack_id: *const c_char,
        ),
    >,
    pub set_flags: Option<
        unsafe extern "C" fn(receiver: *const clap_preset_discovery_metadata_receiver, flags: u32),
    >,
    pub add_creator: Option<
        unsafe extern "C" fn(
            receiver: *const clap_preset_discovery_metadata_receiver,
            creator: *const c_char,
        ),
    >,
    pub set_description: Option<
        unsafe extern "C" fn(
            receiver: *const clap_preset_discovery_metadata_receiver,
            description: *const c_char,
        ),
    >,
    pub set_timestamps: Option<
        unsafe extern "C" fn(
            receiver: *const clap_preset_discovery_metadata_receiver,
            creation_time: u64,
            modification_time: u64,
        ),
    >,
    pub add_feature: Option<
        unsafe extern "C" fn(
            receiver: *const clap_preset_discovery_metadata_receiver,
            feature: *const c_char,
        ),
    >,
    pub add_extra_info: Option<
        unsafe extern "C" fn(
            receiver: *const clap_preset_discovery_metadata_receiver,
            key: *const c_char,
            value: *const c_char,
        ),
    >,
}

#[repr(C)]
pub struct clap_preset_discovery_filetype {
    pub name: *const c_char,
    pub description: *const c_char,
    /// Without the dot; null or empty matches every file.
    pub file_extension: *const c_char,
}

#[repr(C)]
pub struct clap_preset_discovery_location {
    pub flags: u32,
    pub name: *const c_char,
    pub kind: u32,
    /// A file or folder for `LOCATION_FILE`; null for `LOCATION_PLUGIN`.
    pub location: *const c_char,
}

#[repr(C)]
pub struct clap_preset_discovery_soundpack {
    pub flags: u32,
    pub id: *const c_char,
    pub name: *const c_char,
    pub description: *const c_char,
    pub homepage_url: *const c_char,
    pub vendor: *const c_char,
    pub image_path: *const c_char,
    pub release_timestamp: u64,
}

#[repr(C)]
pub struct clap_preset_discovery_provider_descriptor {
    pub clap_version: clap_version,
    pub id: *const c_char,
    pub name: *const c_char,
    pub vendor: *const c_char,
}

#[repr(C)]
pub struct clap_preset_discovery_provider {
    pub desc: *const clap_preset_discovery_provider_descriptor,
    pub provider_data: *mut c_void,
    pub init: Option<unsafe extern "C" fn(provider: *const clap_preset_discovery_provider) -> bool>,
    pub destroy: Option<unsafe extern "C" fn(provider: *const clap_preset_discovery_provider)>,
    pub get_metadata: Option<
        unsafe extern "C" fn(
            provider: *const clap_preset_discovery_provider,
            location_kind: u32,
            location: *const c_char,
            metadata_receiver: *const clap_preset_discovery_metadata_receiver,
        ) -> bool,
    >,
    pub get_extension: Option<
        unsafe extern "C" fn(
            provider: *const clap_preset_discovery_provider,
            extension_id: *const c_char,
        ) -> *const c_void,
    >,
}

#[repr(C)]
pub struct clap_preset_discovery_indexer {
    pub clap_version: clap_version,
    pub name: *const c_char,
    pub vendor: *const c_char,
    pub url: *const c_char,
    pub version: *const c_char,
    pub indexer_data: *mut c_void,
    pub declare_filetype: Option<
        unsafe extern "C" fn(
            indexer: *const clap_preset_discovery_indexer,
            filetype: *const clap_preset_discovery_filetype,
        ) -> bool,
    >,
    pub declare_location: Option<
        unsafe extern "C" fn(
            indexer: *const clap_preset_discovery_indexer,
            location: *const clap_preset_discovery_location,
        ) -> bool,
    >,
    pub declare_soundpack: Option<
        unsafe extern "C" fn(
            indexer: *const clap_preset_discovery_indexer,
            soundpack: *const clap_preset_discovery_soundpack,
        ) -> bool,
    >,
    pub get_extension: Option<
        unsafe extern "C" fn(
            indexer: *const clap_preset_discovery_indexer,
            extension_id: *const c_char,
        ) -> *const c_void,
    >,
}

#[repr(C)]
pub struct clap_preset_discovery_factory {
    pub count: Option<unsafe extern "C" fn(factory: *const clap_preset_discovery_factory) -> u32>,
    pub get_descriptor: Option<
        unsafe extern "C" fn(
            factory: *const clap_preset_discovery_factory,
            index: u32,
        ) -> *const clap_preset_discovery_provider_descriptor,
    >,
    pub create: Option<
        unsafe extern "C" fn(
            factory: *const clap_preset_discovery_factory,
            indexer: *const clap_preset_discovery_indexer,
            provider_id: *const c_char,
        ) -> *const clap_preset_discovery_provider,
    >,
}

#[repr(C)]
pub struct clap_plugin_preset_load {
    pub from_location: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            location_kind: u32,
            location: *const c_char,
            load_key: *const c_char,
        ) -> bool,
    >,
}

#[repr(C)]
pub struct clap_host_preset_load {
    pub on_error: Option<
        unsafe extern "C" fn(
            host: *const clap_host,
            location_kind: u32,
            location: *const c_char,
            load_key: *const c_char,
            os_error: i32,
            msg: *const c_char,
        ),
    >,
    pub loaded: Option<
        unsafe extern "C" fn(
            host: *const clap_host,
            location_kind: u32,
            location: *const c_char,
            load_key: *const c_char,
        ),
    >,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    /// The sizes gcc gives these structs from the real headers, on x86_64
    /// and on armv7 hard-float (the MPC One).
    #[test]
    fn layouts_match_the_c_headers() {
        let wide = size_of::<usize>() == 8;
        let w = |a: usize, b: usize| if wide { a } else { b };
        let got = [
            size_of::<clap_event_header>(),
            size_of::<clap_event_note>(),
            size_of::<clap_event_note_expression>(),
            size_of::<clap_event_midi>(),
            size_of::<clap_event_transport>(),
            size_of::<clap_event_param_value>(),
            size_of::<clap_event_param_gesture>(),
            size_of::<clap_param_info>(),
            size_of::<clap_audio_buffer>(),
            size_of::<clap_version>(),
            size_of::<clap_process>(),
            size_of::<clap_plugin>(),
            size_of::<clap_host>(),
            size_of::<clap_plugin_descriptor>(),
            size_of::<clap_audio_port_info>(),
            size_of::<clap_note_port_info>(),
            size_of::<clap_plugin_entry>(),
            size_of::<clap_universal_plugin_id>(),
            size_of::<clap_preset_discovery_metadata_receiver>(),
            size_of::<clap_preset_discovery_filetype>(),
            size_of::<clap_preset_discovery_location>(),
            size_of::<clap_preset_discovery_soundpack>(),
            size_of::<clap_preset_discovery_provider_descriptor>(),
            size_of::<clap_preset_discovery_provider>(),
            size_of::<clap_preset_discovery_indexer>(),
            size_of::<clap_preset_discovery_factory>(),
            size_of::<clap_plugin_preset_load>(),
            size_of::<clap_host_preset_load>(),
        ];
        let want = [
            16,
            40,
            40,
            24,
            104,
            w(56, 48),
            20,
            1320,
            w(32, 24),
            12,
            w(64, 40),
            w(96, 48),
            w(88, 48),
            w(88, 48),
            w(288, 276),
            268,
            w(40, 24),
            w(16, 8),
            w(88, 44),
            w(24, 12),
            w(32, 16),
            w(64, 40),
            w(40, 24),
            w(48, 24),
            w(88, 48),
            w(24, 12),
            w(8, 4),
            w(16, 8),
        ];
        assert_eq!(got, want);
        assert_eq!(std::mem::offset_of!(clap_event_note, velocity), 32);
        assert_eq!(
            std::mem::offset_of!(clap_event_param_value, value),
            w(48, 40)
        );
        assert_eq!(std::mem::offset_of!(clap_event_transport, tempo), 40);
        assert_eq!(std::mem::offset_of!(clap_param_info, min_value), 1296);
        assert_eq!(align_of::<clap_event_transport>(), 8);
        assert_eq!(
            std::mem::offset_of!(clap_preset_discovery_location, location),
            w(24, 12)
        );
        assert_eq!(
            std::mem::offset_of!(clap_preset_discovery_soundpack, release_timestamp),
            w(56, 32)
        );
        assert_eq!(
            std::mem::offset_of!(clap_preset_discovery_indexer, get_extension),
            w(80, 44)
        );
        assert_eq!(
            std::mem::offset_of!(clap_preset_discovery_metadata_receiver, add_extra_info),
            w(80, 40)
        );
    }
}
