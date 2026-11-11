//! MPFree's CLAP test plugins (`mpfree_clap::testplug`) as a real `.clap`
//! shared library, so the host's tests load them with `dlopen` exactly as
//! they would load anyone's plugin. Built as `libmpfree_clap_testplug.so`.

/// The CLAP entry point every plugin file exports.
#[allow(non_upper_case_globals)]
#[no_mangle]
pub static clap_entry: mpfree_clap::sys::clap_plugin_entry = mpfree_clap::testplug::ENTRY;
