//! A test MPC plugin library: one `.so` that is MPFree's Delay (an effect)
//! or Squelch (an instrument), depending on the folder it is loaded from.
//! At `VSTPluginMain` it asks the host for its folder
//! (`audioMasterGetDirectory`); a folder whose name holds `Squelch` makes
//! the instrument, any other the effect. The tests copy the one library
//! into plugin folders of both kinds, so they also check that the host
//! answers that call while the plugin is being made.
//!
//! The plugins come from `mpfree-mpcplug`, the wrapper that makes MPFree
//! engines into MPC plugins, so a test can compare the plugin's sound with
//! the same engine run in-process.

use std::ffi::{c_void, CStr};

use mpfree_mpcplug::abi::{AEffect, HostCallback, AUDIO_MASTER_GET_DIRECTORY};
use mpfree_mpcplug::{plugin_main, Spec};
use mpfree_node::Node;

/// The effect: MPFree's Delay.
pub fn make_delay() -> Box<dyn Node> {
    mpfree_fx::registry::make("mpfree.fx.delay").expect("delay is in the registry")
}

/// The instrument: MPFree's Squelch.
pub fn make_squelch() -> Box<dyn Node> {
    Box::new(mpfree_synth::Squelch::new(44_100))
}

pub static DELAY: Spec = Spec {
    name: "Test Delay",
    vendor: "MPFree Tests",
    uid: *b"MTdl",
    version: 1,
    instrument: false,
    make: make_delay,
};

pub static SQUELCH: Spec = Spec {
    name: "Test Squelch",
    vendor: "MPFree Tests",
    uid: *b"MTsq",
    version: 1,
    instrument: true,
    make: make_squelch,
};

/// The plugin for a folder path: Squelch if the path's last part holds
/// `Squelch`.
pub fn spec_for(dir: &str) -> &'static Spec {
    let last = dir.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    if last.contains("Squelch") {
        &SQUELCH
    } else {
        &DELAY
    }
}

/// # Safety
/// Called by an MPC plugin host with its callback.
#[no_mangle]
pub unsafe extern "C" fn VSTPluginMain(host: Option<HostCallback>) -> *mut AEffect {
    let mut dir = String::new();
    if let Some(h) = host {
        let p = h(
            std::ptr::null_mut(),
            AUDIO_MASTER_GET_DIRECTORY,
            0,
            0,
            std::ptr::null_mut(),
            0.0,
        ) as *const c_void;
        if !p.is_null() {
            dir = CStr::from_ptr(p as *const std::ffi::c_char)
                .to_string_lossy()
                .into_owned();
        }
    }
    plugin_main(spec_for(&dir), host)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_folder_picks_the_plugin() {
        assert_eq!(
            super::spec_for("/x/MPFree Tests - VST - Test Squelch").name,
            "Test Squelch"
        );
        assert_eq!(super::spec_for("/x/Squelch/Delay").name, "Test Delay");
        assert_eq!(super::spec_for("").name, "Test Delay");
    }
}
