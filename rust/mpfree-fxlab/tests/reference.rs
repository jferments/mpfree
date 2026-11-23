//! The DSP quality plan's Y8 (`docs/dev/design/dsp-quality.md`, step 0.5,
//! `mpfree_fxlab::reference`): each aliasing case at 44.1 kHz against a
//! render at 8 times the rate brought down to 44.1 kHz, and the 8x
//! reference against a 16x one.
//!
//! No Y8 target is set yet: each phase 1 or 2 step sets its own, and only
//! where the reference has converged. This gate keeps the measure honest:
//! every number finite, and every case listed in [`AGREES`] still has an 8x
//! reference that agrees with the 16x one. A change that makes another
//! case agree adds it.
//!
//! Release only (about 15 s on four cores), in the same CI step as Y4 and
//! Y7:
//!
//! ```text
//! cargo test --release --locked -p mpfree-fxlab --test reference -- --nocapture
//! ```

use mpfree_fxlab::reference;

/// The cases whose 8x reference agreed with the 16x one at the first
/// survey (55 of 96), less one. Not listed: the drives and sync (still
/// folding at 8x), Squelch's square, Analog's saw (C6 and C7 fizz 3.7 dB
/// apart near the floor at the survey; C5 left at step 1.2, whose
/// oscillators run flat to 0.48 of whatever rate they run at, so the 8x
/// render's fizz rose from -81.8 to -76.5 dB against 16x's -80.8, both
/// over the 44.1 kHz render's -81.3), FM (feedback noise, the square's
/// steps), and Strings and Drawbar's combo reed, whose off-harmonic energy
/// grows with the rate.
const AGREES: &[&str] = &[
    "Poly saw, C5",
    "Poly saw, C6",
    "Poly saw, C7",
    "Poly pulse, C5",
    "Poly pulse, C6",
    "Poly pulse, C7",
    "Poly triangle, C5",
    "Poly triangle, C6",
    "Poly triangle, C7",
    "Poly saw, distortion 24 dB, C5",
    "Poly saw, distortion 24 dB, C6",
    "Poly saw, distortion 24 dB, C7",
    "Squelch saw, C5",
    "Squelch saw, C6",
    "Squelch saw, C7",
    "Squelch saw, drive 100, C5",
    "Squelch saw, drive 100, C6",
    "Squelch saw, drive 100, C7",
    "Bass saw, C5",
    "Bass saw, C6",
    "Bass saw, C7",
    "Bass pulse, C5",
    "Bass pulse, C6",
    "Bass pulse, C7",
    "Bass saw, drive 100, C5",
    "Bass saw, drive 100, C6",
    "Bass saw, drive 100, C7",
    "Poly table, position 100, C5",
    "Poly table, position 100, C6",
    "Poly table, position 100, C7",
    "Wavetable Init, C5",
    "Wavetable Init, C6",
    "Wavetable Init, C7",
    "Wavetable position 100, C5",
    "Wavetable position 100, C6",
    "Wavetable position 100, C7",
    "Wind clarinet, tone 100, C5",
    "Wind clarinet, tone 100, C6",
    "Wind clarinet, tone 100, C7",
    "Wind sax, reed and tone 100, C5",
    "Wind sax, reed and tone 100, C6",
    "Wind sax, reed and tone 100, C7",
    "Wind flute, edge and tone 100, C5",
    "Wind flute, edge and tone 100, C6",
    "Wind flute, edge and tone 100, C7",
    "Choir aah, C5",
    "Choir aah, C6",
    "Choir aah, C7",
    "Choir ee, C5",
    "Choir ee, C6",
    "Choir ee, C7",
    "Drawbar combo flute, C5",
    "Drawbar combo flute, C6",
    "Drawbar combo flute, C7",
];

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: cargo test --release --locked -p mpfree-fxlab --test reference"
)]
fn y8_every_listed_reference_still_agrees_with_16x() {
    let rows = reference::y8(true);
    print!("{}", reference::to_text(&rows));
    let mut broke = Vec::new();
    let mut now = Vec::new();
    for r in &rows {
        for v in [
            r.fizz_db,
            r.ref_fizz_db,
            r.harmonic_err_db,
            r.envelope_err_db,
            r.level_db,
        ] {
            assert!(v.is_finite(), "{}: a Y8 number is not finite", r.case);
        }
        let listed = AGREES.contains(&r.case.as_str());
        let agrees = r.agrees() == Some(true);
        if listed && !agrees {
            broke.push(r.case.clone());
        }
        if !listed && agrees {
            now.push(r.case.clone());
        }
    }
    if !now.is_empty() {
        println!(
            "Y8: also agrees, not yet listed in AGREES: {}",
            now.join(", ")
        );
    }
    assert!(
        broke.is_empty(),
        "the 8x reference no longer agrees with 16x: {}",
        broke.join(", ")
    );
    for name in AGREES {
        assert!(rows.iter().any(|r| r.case == *name), "{name} is not a case");
    }
}

/// Y3 (reported only, `docs/dev/design/dsp-quality.md`): Analog's
/// poly-mod at full and each synth's fast filter envelope against their
/// 8x references. Every number finite.
#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: cargo test --release --locked -p mpfree-fxlab --test reference"
)]
fn y3_audio_rate_moves_are_measured() {
    let rows = reference::y3(true);
    print!("{}", reference::y3_text(&rows));
    for r in &rows {
        for v in [
            r.fizz_db,
            r.ref_fizz_db,
            r.harmonic_err_db,
            r.envelope_err_db,
            r.level_db,
        ] {
            assert!(v.is_finite(), "{}: a Y3 number is not finite", r.case);
        }
    }
}
