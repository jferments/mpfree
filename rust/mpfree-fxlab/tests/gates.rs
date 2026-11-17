//! The quick quality gate: measurements every pull request runs, each a
//! line from the effects quality plan that MPFree's effects already meet,
//! so a change that makes one worse fails here. Gaps the plan still has to
//! close are reported by `fxlab cards`, not asserted here.

use mpfree_fxlab::card::{self, Options};
use mpfree_fxlab::dut::{Dut, NodeDut};
use mpfree_fxlab::signals::{db_to_amp, pink};
use mpfree_fxlab::wav::Stereo;
use mpfree_fxlab::{harmonics, loudness, response};

const RATE: u32 = 44_100;

fn dut(id: &str) -> NodeDut {
    NodeDut::from_registry(id, RATE).unwrap_or_else(|| panic!("{id} is in the registry"))
}

#[test]
fn transparent_effects_are_flat_and_clean_at_their_defaults() {
    for id in [
        "mpfree.fx.eq",
        "mpfree.fx.dj_filter",
        "mpfree.fx.sample_delay",
        "mpfree.fx.tuner",
    ] {
        let Some(mut d) = NodeDut::from_registry(id, RATE) else {
            continue;
        };
        let r = response::measure(&mut d, -12.0, 4096);
        assert!(r.worst_error(30.0, 20_000.0, |_| 0.0) < 0.01, "{id}: {r:?}");
        let h = harmonics::measure(&mut d, 1000.0, -6.0);
        assert!(h.thd_db < -140.0, "{id}: {h:?}");
    }
}

#[test]
fn drive_hq_keeps_aliases_90_db_down_at_5_khz_full_scale() {
    let mut d = dut("mpfree.fx.drive");
    d.set(mpfree_fx::ids::drive::QUALITY, 1.0);
    let h = harmonics::measure(&mut d, 5000.0, 0.0);
    assert!(h.alias_worst_db < -90.0, "{h:?}");
}

#[test]
fn drive_eco_keeps_aliases_50_db_down_at_5_khz_full_scale() {
    let mut d = dut("mpfree.fx.drive");
    let h = harmonics::measure(&mut d, 5000.0, 0.0);
    assert!(h.alias_worst_db < -50.0, "{h:?}");
}

/// Pink at -14 dBFS RMS, `secs` long.
fn pink_14(secs: usize, seed: u64) -> Stereo {
    Stereo::mono(pink(secs * RATE as usize, db_to_amp(-14.0), seed), RATE)
}

/// The sample peak and the 4x true peak (dB) of `d`'s output on `stim`,
/// after its first half second.
fn peaks(d: &mut NodeDut, stim: &Stereo) -> (f64, f64) {
    let out = d.render(stim, d.latency());
    let body = out.slice(d.latency() + RATE as usize / 2, stim.len());
    (loudness::sample_peak(&body), loudness::true_peak(&body))
}

#[test]
fn the_limiter_holds_its_ceiling_in_samples_and_true_peaks() {
    // FX quality round 4, P1 and P2: pushed 12 dB, the samples stay at
    // the ceiling, and with True peak on so do the peaks between them
    // (within 0.1 dB, as a 4x meter reads them).
    use mpfree_fx::ids::limiter::*;
    for tp in [0.0, 1.0] {
        let mut d = dut("mpfree.fx.limiter");
        d.set(GAIN, 12.0);
        d.set(TRUE_PEAK, tp);
        let (sp, tpk) = peaks(&mut d, &pink_14(3, 5));
        assert!(sp <= -0.3 + 1e-4, "True peak {tp}: sample peak {sp:.4}");
        if tp > 0.5 {
            assert!(tpk <= -0.3 + 0.1, "true peak {tpk:.3} dBTP, ceiling -0.3");
        }
    }
}

#[test]
fn master_safety_never_passes_its_ceiling_in_samples() {
    let mut d = dut("mpfree.fx.master_safety");
    let stim = Stereo::mono(pink(2 * RATE as usize, db_to_amp(-3.0), 6), RATE);
    let out = d.render(&stim, d.latency());
    assert!(loudness::sample_peak(&out) <= -0.3 + 1e-4);
}

#[test]
fn master_safety_keeps_true_peaks_at_its_ceiling_when_it_limits_or_clips() {
    // FX quality round 4, P2 and P6 (decision 11.1: always caught).
    use mpfree_fx::ids::master::*;
    for mode in [0.0, 1.0] {
        let mut d = dut("mpfree.fx.master_safety");
        d.set(MODE, mode);
        d.set(TRIM, 12.0);
        let (sp, tpk) = peaks(&mut d, &pink_14(3, 8));
        assert!(sp <= -0.3 + 1e-4, "mode {mode}: sample peak {sp:.4}");
        assert!(tpk <= -0.3 + 0.1, "mode {mode}: true peak {tpk:.3} dBTP");
    }
}

#[test]
fn the_limiters_latency_never_moves() {
    // P7: the mixer compensates a fixed latency, and an export fails if
    // one moves mid-pass.
    use mpfree_fx::ids::{limiter as li, master as mi};
    let lat = |id: &str, set: &[(u16, f64)]| {
        let mut d = dut(id);
        for (k, v) in set {
            d.set(*k, *v);
        }
        d.latency()
    };
    let lim = lat("mpfree.fx.limiter", &[]);
    for set in [
        &[(li::TRUE_PEAK, 1.0)][..],
        &[(li::AUTO, 0.0), (li::RELEASE, 5.0)],
        &[(li::RELEASE, 1000.0)],
    ] {
        assert_eq!(lat("mpfree.fx.limiter", set), lim, "{set:?}");
    }
    let ms = lat("mpfree.fx.master_safety", &[]);
    for set in [
        &[(mi::MODE, 1.0)][..],
        &[(mi::MODE, 2.0)],
        &[(mi::RELEASE, 10.0)],
    ] {
        assert_eq!(lat("mpfree.fx.master_safety", set), ms, "{set:?}");
    }
}

#[test]
fn the_limiters_auto_release_lets_go_slower_after_a_long_push() {
    // P4: after 3 s of 6 dB of limiting, at least twice as slow back
    // (63 %) as after 0.1 s.
    let after = |loud: f64| {
        let mut d = dut("mpfree.fx.limiter");
        mpfree_fxlab::dynamics::release_after(&mut d, -20.0, -0.3 + 6.0, loud, 1.5)
            .0
            .expect("a release")
    };
    let (short, long) = (after(0.1), after(3.0));
    assert!(long >= 2.0 * short, "{short:.1} ms, then {long:.1} ms");
}

#[test]
fn the_multiband_is_flat_at_ratio_1_at_any_mix() {
    // X2: the dry passes the crossovers' all-passes, so a Mix between
    // adds the two in phase rather than notching the crossovers.
    use mpfree_fx::ids::multiband as mb;
    for mix in [25.0, 50.0, 75.0] {
        let mut d = dut("mpfree.fx.multiband");
        for b in 0..mb::BANDS {
            d.set(mb::ratio(b), 1.0);
        }
        d.set(mb::MIX, mix);
        let r = response::measure(&mut d, -20.0, 8192);
        assert!(
            r.worst_error(20.0, 20_000.0, |_| 0.0) < 0.5,
            "Mix {mix}: {r:?}"
        );
    }
}

#[test]
fn the_multibands_auto_makeup_keeps_pink_near_the_dry() {
    // X7: with Auto makeup on, pink at -18 dBFS comes out within 1.5 LU
    // of the dry at the defaults and at Drum crush's heavy settings.
    use mpfree_fx::ids::multiband as mb;
    let x = Stereo::mono(pink(6 * RATE as usize, db_to_amp(-18.0), 11), RATE);
    let rate = RATE as usize;
    let dry = loudness::integrated(&x.slice(rate, x.len() - 2 * rate));
    let mut crush = vec![(mb::ATTACK, 1.0), (mb::RELEASE, 60.0)];
    for b in 0..mb::BANDS {
        crush.extend([(mb::threshold(b), -30.0), (mb::ratio(b), 8.0)]);
    }
    for set in [vec![], crush] {
        let mut d = dut("mpfree.fx.multiband");
        d.set(mb::AUTO_MAKEUP, 1.0);
        for &(id, v) in &set {
            d.set(id, v);
        }
        let out = d.render(&x, 0);
        let lu = loudness::integrated(&out.slice(rate, x.len() - 2 * rate)) - dry;
        assert!(lu.abs() <= 1.5, "{set:?}: {lu:+.2} LU");
    }
}

#[test]
fn every_effect_gets_a_finite_quick_card() {
    let opts = Options {
        rate: RATE,
        quick: true,
    };
    for e in mpfree_fx::EFFECTS.iter() {
        let c = card::measure(e, None, opts);
        assert!(
            c.notes.iter().all(|n| !n.contains("not finite")),
            "{}: {:?}",
            e.id,
            c.notes
        );
        let idle = c.get("idle_db").unwrap();
        assert!(
            idle < -60.0,
            "{} is still loud 1 s after its tail: {idle:.1} dB",
            e.id
        );
    }
}
