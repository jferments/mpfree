//! The pitched reads' quality ratchet: every number of
//! `mpfree_fxlab::readers`'s report (images, passband and folds of every
//! read that plays audio at another speed: the sampler's pads and grains,
//! the grain engine, the streaming cursor and the stretch's resampler)
//! may stay or get better, never worse, against `quality/readers.tsv`.
//! Targets not met yet are printed, not failed. A change that improves a
//! number records it with
//!
//! ```text
//! MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test readers
//! ```
//!
//! and the file's diff shows the change in review; a number made worse on
//! purpose is blessed the same way and explained in the pull request
//! (`docs/dev/design/pitched-reads.md`). It takes about 10 s in a debug
//! build (2 s in release), so it runs with the rest of the workspace's
//! tests, as the sampler's does.

use mpfree_fxlab::{quality, readers};

const RATE: u32 = 44_100;

#[test]
fn no_pitched_read_number_gets_worse() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/quality/readers.tsv");
    let rows = readers::report(RATE);
    print!("{}", readers::to_text(&rows));
    if std::env::var_os("MPFREE_QUALITY_BLESS").is_some() {
        std::fs::write(path, readers::ratchet_text(&rows)).expect("readers.tsv is writable");
        println!("recorded {path}");
        return;
    }
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}; record it with MPFREE_QUALITY_BLESS=1"));
    let recorded = quality::ratchet_read(&text).expect("readers.tsv reads");
    let problems = readers::ratchet_check(&rows, &recorded);
    assert!(
        problems.is_empty(),
        "{} pitched-read quality numbers fail the ratchet (an improvement is \
         recorded with MPFREE_QUALITY_BLESS=1):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
