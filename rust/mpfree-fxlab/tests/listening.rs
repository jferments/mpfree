//! The sampler plan's step 18 ratchet: every number of
//! `mpfree_fxlab::listening`'s report (each listening sound through each
//! grain mode: the level change, the wobble and how far the peaks rise)
//! may stay or get better, never worse, against `quality/listening.tsv`.
//! Targets not met yet are printed, not failed. A change that improves a
//! number records it with
//!
//! ```text
//! MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test listening
//! ```
//!
//! and the file's diff shows the change in review; a number made worse on
//! purpose is blessed the same way and explained in the pull request
//! (`docs/dev/design/sampler-architecture.md`, "Phase 4").

use mpfree_fxlab::{listening, quality, sampler};

const RATE: u32 = 44_100;

#[test]
fn no_listening_number_gets_worse() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/quality/listening.tsv");
    let rows = listening::report(RATE);
    print!("{}", sampler::to_text(&rows));
    if std::env::var_os("MPFREE_QUALITY_BLESS").is_some() {
        std::fs::write(path, listening::ratchet_text(&rows)).expect("listening.tsv is writable");
        println!("recorded {path}");
        return;
    }
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}; record it with MPFREE_QUALITY_BLESS=1"));
    let recorded = quality::ratchet_read(&text).expect("listening.tsv reads");
    let problems = sampler::ratchet_check(&rows, &recorded);
    assert!(
        problems.is_empty(),
        "{} listening numbers fail the ratchet (an improvement is \
         recorded with MPFREE_QUALITY_BLESS=1):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
