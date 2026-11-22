//! The DSP quality plan's gates Y4 and Y7 (`docs/dev/design/dsp-quality.md`,
//! `mpfree_fxlab::robust`):
//!
//! - **Y4:** every synth engine, the drums and every registered effect,
//!   each on its own, keeps every output sample finite at every setting
//!   the layers try.
//! - **Y7:** the subjects listed in [`BIT_EXACT`] give the same output, bit
//!   for bit, in blocks of 1 and 37 frames as in blocks of 128. The rest
//!   are printed with their difference.
//!
//! Both render a lot, so they run in release only (their own CI step) and
//! are ignored in the debug test run:
//!
//! ```text
//! cargo test --release --locked -p mpfree-fxlab --test robust -- --nocapture
//! ```

use mpfree_fxlab::robust::{self, Subject};

const RATE: u32 = 44_100;

/// The subjects that are bit-exact for any block size (measured at
/// e5518efc: every engine, the drums and every effect but three). A change
/// that makes one of them depend on the block size fails here; one that
/// makes another subject bit-exact adds it. Not listed: the chorus (-2.4 dB
/// at block 1) and the phaser (-16 dB), whose LFO takes a new rate and
/// re-syncs only at the start of a block, and the rotary, whose difference
/// is rounding (below -400 dB).
const BIT_EXACT: &[&str] = &[
    "synth/Analog",
    "synth/Squelch",
    "synth/FM",
    "synth/Wavetable",
    "synth/Keys",
    "synth/Drawbar",
    "synth/Pluck",
    "synth/Strings",
    "synth/Bass",
    "synth/Wind",
    "synth/Choir",
    "synth/Drums",
    "mpfree.fx.compressor",
    "mpfree.fx.ducker",
    "mpfree.fx.transient",
    "mpfree.fx.limiter",
    "mpfree.fx.gate",
    "mpfree.fx.deesser",
    "mpfree.fx.multiband",
    "mpfree.fx.channel_strip",
    "mpfree.fx.eq",
    "mpfree.fx.filter",
    "mpfree.fx.dj_filter",
    "mpfree.fx.comb",
    "mpfree.fx.autowah",
    "mpfree.fx.drive",
    "mpfree.fx.character",
    "mpfree.fx.crush",
    "mpfree.fx.lofi",
    "mpfree.fx.amp",
    "mpfree.fx.fuzz",
    "mpfree.fx.exciter",
    "mpfree.fx.ensemble",
    "mpfree.fx.flanger",
    "mpfree.fx.tremolo",
    "mpfree.fx.autopan",
    "mpfree.fx.ringmod",
    "mpfree.fx.freqshift",
    "mpfree.fx.delay",
    "mpfree.fx.tape_delay",
    "mpfree.fx.multitap",
    "mpfree.fx.cloud",
    "mpfree.fx.room",
    "mpfree.fx.plate",
    "mpfree.fx.spring",
    "mpfree.fx.granulator",
    "mpfree.grains",
    "mpfree.fx.pitch",
    "mpfree.fx.pitch_correct",
    "mpfree.fx.harmonizer",
    "mpfree.fx.talkbox",
    "mpfree.fx.vocoder",
    "mpfree.fx.perform",
    "mpfree.fx.xy",
    "mpfree.fx.tuner",
    "mpfree.fx.width",
    "mpfree.fx.sample_delay",
    "mpfree.fx.master_safety",
];

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: cargo test --release --locked -p mpfree-fxlab --test robust"
)]
fn y4_every_engine_and_effect_stays_finite_at_every_setting() {
    let results = robust::over_all(|s| robust::y4(s, RATE));
    let mut failed = Vec::new();
    let mut renders = 0;
    for r in &results {
        renders += r.renders;
        println!(
            "Y4 {:<34} {:>5} renders, peak {:>7.1} dBFS{}",
            r.subject,
            r.renders,
            r.peak_dbfs,
            if r.failures.is_empty() {
                String::new()
            } else {
                format!(", NOT FINITE in {}", r.failures.len())
            }
        );
        for f in &r.failures {
            failed.push(format!("{}: {f}", r.subject));
        }
    }
    println!("Y4: {} subjects, {renders} renders", results.len());
    assert!(
        failed.is_empty(),
        "{} settings gave a NaN or an infinity:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: cargo test --release --locked -p mpfree-fxlab --test robust"
)]
fn y7_the_bit_exact_subjects_stay_bit_exact_for_any_block_size() {
    let results = robust::over_all(|s| robust::y7(s, RATE));
    let mut broke = Vec::new();
    let mut now_exact = Vec::new();
    for r in &results {
        let show = |d: f64| {
            if d == f64::NEG_INFINITY {
                "bit-exact".to_string()
            } else {
                format!("{d:.1} dB")
            }
        };
        println!(
            "Y7 {:<34} block 1: {:<10} block 37: {:<10} (peak {:.1} dBFS)",
            r.subject,
            show(r.diff_db[0].1),
            show(r.diff_db[1].1),
            r.peak_dbfs
        );
        assert!(
            r.peak_dbfs > -60.0,
            "{}: the Y7 render is silent",
            r.subject
        );
        let listed = BIT_EXACT.contains(&r.subject.as_str());
        if listed && !r.bit_exact() {
            broke.push(r.subject.clone());
        }
        if !listed && r.bit_exact() {
            now_exact.push(r.subject.clone());
        }
    }
    if !now_exact.is_empty() {
        println!(
            "Y7: also bit-exact, not yet listed in BIT_EXACT: {}",
            now_exact.join(", ")
        );
    }
    assert!(
        broke.is_empty(),
        "no longer bit-exact for any block size: {}",
        broke.join(", ")
    );
    // Every listed name is a subject.
    for name in BIT_EXACT {
        assert!(
            Subject::all().iter().any(|s| s.name() == *name),
            "{name} is not a subject"
        );
    }
}
