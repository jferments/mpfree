//! The filter measures' ratchet (DSP quality Y1 and Y2,
//! `mpfree_fxlab::filters`): every synth and sampler filter against the
//! curve its page draws may stay or get better, never worse, against
//! `quality/filters.tsv`. Targets not met yet are printed, not failed. A
//! change that improves a number records it with
//!
//! ```text
//! MPFREE_QUALITY_BLESS=1 cargo test --release --locked -p mpfree-fxlab --test filters
//! ```
//!
//! and the file's diff shows the change in review; a number made worse on
//! purpose is blessed the same way and explained in the pull request
//! (`docs/dev/design/dsp-quality.md`). Release only: a debug build takes
//! minutes.

use mpfree_fxlab::{filters, quality, sampler};

const RATE: u32 = 44_100;

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: cargo test --release --locked -p mpfree-fxlab --test filters"
)]
fn no_filter_number_gets_worse() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/quality/filters.tsv");
    let rows = filters::report(RATE);
    print!("{}", sampler::to_text(&rows));
    if std::env::var_os("MPFREE_QUALITY_BLESS").is_some() {
        std::fs::write(path, filters::ratchet_text(&rows)).expect("filters.tsv is writable");
        println!("recorded {path}");
        return;
    }
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}; record it with MPFREE_QUALITY_BLESS=1"));
    let recorded = quality::ratchet_read(&text).expect("filters.tsv reads");
    let problems = sampler::ratchet_check(&rows, &recorded);
    assert!(
        problems.is_empty(),
        "{} filter numbers fail the ratchet (an improvement is recorded with \
         MPFREE_QUALITY_BLESS=1):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
