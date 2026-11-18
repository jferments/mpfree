//! The synths' quality ratchet: every number of `mpfree_fxlab::synth`'s
//! report (every engine and factory preset against the synth library's
//! targets) may stay or get better, never worse, against
//! `quality/synth.tsv`. Targets not met yet are printed, not failed. A
//! change that improves a number records it with
//!
//! ```text
//! MPFREE_QUALITY_BLESS=1 cargo test --release --locked -p mpfree-fxlab --test synth
//! ```
//!
//! and the file's diff shows the change in review; a number made worse on
//! purpose is blessed the same way and explained in the pull request
//! (`docs/dev/design/synth-quality.md`). It renders every preset many
//! times, so it runs in release only (its own CI step) and is ignored in
//! the debug test run.

use mpfree_fxlab::{quality, synth};

const RATE: u32 = 44_100;

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: cargo test --release --locked -p mpfree-fxlab --test synth"
)]
fn no_synth_quality_number_gets_worse() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/quality/synth.tsv");
    let rows = synth::report(RATE);
    print!("{}", synth::to_text(&rows));
    print!("{}", synth::summary(&rows));
    if std::env::var_os("MPFREE_QUALITY_BLESS").is_some() {
        std::fs::write(path, synth::ratchet_text(&rows)).expect("synth.tsv is writable");
        println!("recorded {path}");
        return;
    }
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}; record it with MPFREE_QUALITY_BLESS=1"));
    let recorded = quality::ratchet_read(&text).expect("synth.tsv reads");
    let problems = synth::ratchet_check(&rows, &recorded);
    assert!(
        problems.is_empty(),
        "{} synth quality numbers fail the ratchet (an improvement is \
         recorded with MPFREE_QUALITY_BLESS=1):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
