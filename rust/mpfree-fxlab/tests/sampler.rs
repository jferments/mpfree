//! The sampler's quality ratchet: every number of `mpfree_fxlab::sampler`'s
//! report (pitched reads, what must fold away, loop seams, voice stops,
//! the filter against its analog prototype) may stay or get better, never
//! worse, against `quality/sampler.tsv`. Targets not met yet are printed,
//! not failed. A change that improves a number records it with
//!
//! ```text
//! MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test sampler
//! ```
//!
//! and the file's diff shows the change in review; a number made worse on
//! purpose is blessed the same way and explained in the pull request
//! (`docs/dev/design/fx-quality.md`, "The sampler"). It takes a few
//! seconds in a debug build, so it runs with the rest of the workspace's
//! tests.

use mpfree_fxlab::{quality, sampler};

const RATE: u32 = 44_100;

#[test]
fn no_sampler_quality_number_gets_worse() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/quality/sampler.tsv");
    let rows = sampler::report(RATE);
    print!("{}", sampler::to_text(&rows));
    if std::env::var_os("MPFREE_QUALITY_BLESS").is_some() {
        std::fs::write(path, sampler::ratchet_text(&rows)).expect("sampler.tsv is writable");
        println!("recorded {path}");
        return;
    }
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}; record it with MPFREE_QUALITY_BLESS=1"));
    let recorded = quality::ratchet_read(&text).expect("sampler.tsv reads");
    let problems = sampler::ratchet_check(&rows, &recorded);
    assert!(
        problems.is_empty(),
        "{} sampler quality numbers fail the ratchet (an improvement is \
         recorded with MPFREE_QUALITY_BLESS=1):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
