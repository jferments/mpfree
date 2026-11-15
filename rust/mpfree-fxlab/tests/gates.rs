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

#[test]
fn the_limiter_in_true_peak_mode_holds_true_peaks_near_its_ceiling() {
    let mut d = dut("mpfree.fx.limiter");
    d.set(mpfree_fx::ids::limiter::GAIN, 12.0);
    d.set(mpfree_fx::ids::limiter::TRUE_PEAK, 1.0);
    let stim = Stereo::mono(pink(3 * RATE as usize, db_to_amp(-14.0), 5), RATE);
    let out = d.render(&stim, d.latency());
    let body = out.slice(d.latency() + RATE as usize / 2, 2 * RATE as usize);
    let tp = loudness::true_peak(&body);
    assert!(
        tp < -0.3 + 0.5,
        "true peak {tp:.2} dBTP against a -0.3 dB ceiling"
    );
}

#[test]
fn master_safety_never_passes_its_ceiling_in_samples() {
    let mut d = dut("mpfree.fx.master_safety");
    let stim = Stereo::mono(pink(2 * RATE as usize, db_to_amp(-3.0), 6), RATE);
    let out = d.render(&stim, d.latency());
    assert!(loudness::sample_peak(&out) <= -0.3 + 1e-4);
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
