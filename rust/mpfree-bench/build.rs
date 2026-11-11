//! Records the RUSTFLAGS this program was built with, so every report says
//! which build (target CPU, features) the numbers belong to.
fn main() {
    let flags = std::env::var("CARGO_ENCODED_RUSTFLAGS")
        .unwrap_or_default()
        .replace('\u{1f}', " ");
    println!("cargo:rustc-env=MPFREE_BENCH_RUSTFLAGS={}", flags.trim());
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
}
