//! The FX quality ratchet: every number of the quality report for delay,
//! EQ and reverb (`mpfree_fxlab::quality`) may stay or get better, never
//! worse, against `quality/ratchet.tsv`. Targets not met yet are printed,
//! not failed. A change that improves a number records it with
//!
//! ```text
//! MPFREE_QUALITY_BLESS=1 cargo test --release --locked -p mpfree-fxlab --test quality
//! ```
//!
//! and the file's diff shows the change in review; a number made worse on
//! purpose is blessed the same way and explained in the pull request.
//! Release only: the renders take minutes in a debug build
//! (`docs/dev/design/fx-quality.md`).

use mpfree_fxlab::quality::{self, Class};

const RATE: u32 = 44_100;

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: cargo test --release --locked -p mpfree-fxlab --test quality"
)]
fn no_quality_number_gets_worse() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/quality/ratchet.tsv");
    let reports: Vec<_> = Class::ALL
        .iter()
        .map(|c| quality::report(*c, RATE))
        .collect();
    for r in &reports {
        print!("{}", quality::to_text(r));
    }
    if std::env::var_os("MPFREE_QUALITY_BLESS").is_some() {
        std::fs::write(path, quality::ratchet_text(&reports)).expect("ratchet.tsv is writable");
        println!("recorded {path}");
        return;
    }
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}; record it with MPFREE_QUALITY_BLESS=1"));
    let recorded = quality::ratchet_read(&text).expect("ratchet.tsv reads");
    let problems = quality::ratchet_check(&reports, &recorded);
    assert!(
        problems.is_empty(),
        "{} quality numbers fail the ratchet (an improvement is recorded with \
         MPFREE_QUALITY_BLESS=1):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
