//! The plugin sandbox's process: one CLAP plugin, served to MPFree over
//! shared memory and a socket. Started by `mpfree_plughost::spawn`.

fn main() {
    std::process::exit(mpfree_plughost::child::child_main(
        std::env::args_os().skip(1),
    ));
}
