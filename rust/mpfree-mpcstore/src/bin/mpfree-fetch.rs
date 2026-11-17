//! `mpfree-fetch URL OUT MAX`: one download for MPFree's Plugins place,
//! for the pad player on the MPC, which is linked statically and so can't
//! open the system's libcurl itself (`mpfree_mpcstore::fetch`). This one is
//! linked dynamically against the MPC's glibc: build it for the MPC with
//! `cargo zigbuild --target armv7-unknown-linux-gnueabihf.2.34`, never with
//! `+crt-static`. It goes beside the pad player (`/etc/mpfree-test/` on the
//! stick). Only `https://` on the catalog's hosts, as the pad player.
//! `mpfree-fetch --tls` downloads nothing: it starts libcurl and says what
//! its OpenSSL said (`fetch::tls_through_helper`), for the Plugins place's
//! first line.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(mpfree_mpcstore::fetch::helper_main(&args));
}
