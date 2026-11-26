//! The audio tracks rethink's 5c.5 ratchet: every number of
//! `mpfree_fxlab::stretching`'s report (each sound of the stretch corpus
//! at each speed: the onsets found and their timing, the attacks, the
//! pre-echo, the stereo, the pitch and its wobble, the phasiness and the
//! length) may stay or get better, never worse, against
//! `quality/stretch.tsv`. A change that improves a number records it with
//!
//! ```text
//! MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test stretching
//! ```
//!
//! and the file's diff shows the change in review; a number made worse on
//! purpose is blessed the same way and explained in the pull request
//! (`docs/dev/design/tempo-follow.md`, "The stretch corpus").

use mpfree_fxlab::{quality, sampler, stretching};

const RATE: u32 = 44_100;

#[test]
fn no_stretch_number_gets_worse() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/quality/stretch.tsv");
    let rows = stretching::report(RATE);
    print!("{}", sampler::to_text(&rows));
    if std::env::var_os("MPFREE_QUALITY_BLESS").is_some() {
        std::fs::write(path, stretching::ratchet_text(&rows)).expect("stretch.tsv is writable");
        println!("recorded {path}");
        return;
    }
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}; record it with MPFREE_QUALITY_BLESS=1"));
    let recorded = quality::ratchet_read(&text).expect("stretch.tsv reads");
    let problems = sampler::ratchet_check(&rows, &recorded);
    assert!(
        problems.is_empty(),
        "{} stretch numbers fail the ratchet (an improvement is \
         recorded with MPFREE_QUALITY_BLESS=1):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
