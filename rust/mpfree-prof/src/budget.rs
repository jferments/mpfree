//! The MPC One's audio period and the budget each module is measured
//! against.
//!
//! The audio thread runs once per period on one Cortex-A17 core. Everything
//! it does for that period (every engine, the mix, the master) must finish
//! before the period ends, every time, or the output underruns. The budget
//! is the **worst** block, not the mean: one slow block in ten thousand is
//! an audible click.
//!
//! Which block a verdict judges (Jay's choice, 2026-11-11, after round 7):
//! an engine is judged on its **p99.9** block, so one stray period (an
//! interrupt, a page fault, another thread) does not mark it OVER; the whole
//! period (`period`) is judged on its **worst** block, because that is what
//! underruns. [`judge`] applies the rule; hosts and the bench use it rather
//! than picking a statistic themselves.
//!
//! Jay agreed the table on 2026-11-11. The rule behind
//! it: the whole audio thread stays under 60 % of a period in its worst
//! block, which leaves 40 % for the kernel, interrupts, ALSA and a late
//! wake-up under PREEMPT_RT. The split follows what round 6 measured on the
//! device (synth worst 577 µs, sampler worst 699 µs, both with the stock
//! build that has NEON off).

/// A block size and rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Period {
    pub frames: u32,
    pub rate: u32,
}

/// The MPC One's ALSA period as MPFree runs it (mpfree-hello: 128 frames at
/// 44.1 kHz, three periods in the buffer).
pub const MPC: Period = Period { frames: 128, rate: 44_100 };

impl Period {
    /// The period's length in nanoseconds (2,902,494 for [`MPC`]).
    pub fn ns(&self) -> u64 { (u64::from(self.frames) * 1_000_000_000) / u64::from(self.rate.max(1)) }
    pub fn us(&self) -> f64 { f64::from(self.frames) * 1e6 / f64::from(self.rate.max(1)) }
    /// `ns` as a share of the period, in per cent.
    pub fn pct(&self, ns: f64) -> f64 { 100.0 * ns / (self.us() * 1000.0) }
}

/// The share of the audio thread's worst block the whole thread may use.
pub const AUDIO_THREAD_WORST_PCT: f64 = 60.0;

/// One module's share of the period.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budget {
    /// The probe name hosts use for this module.
    pub module: &'static str,
    /// The worst block may take this share of the period, in per cent.
    pub worst_pct: f64,
    /// What the budget covers and how it was chosen.
    pub covers: &'static str,
}

/// The per-module budgets (worst block, per cent of [`MPC`]). They add up to
/// [`AUDIO_THREAD_WORST_PCT`].
pub static MPC_BUDGETS: &[Budget] = &[
    Budget { module: "sampler", worst_pct: 25.0,
             covers: "Every pad voice, granular pads and the master grain effect. Round 6: worst 24.1 %." },
    Budget { module: "synth", worst_pct: 20.0,
             covers: "Every synth track's instrument. Round 6 (one Poly, up to 18 voices): worst 19.9 %." },
    Budget { module: "fx", worst_pct: 10.0,
             covers: "Every insert, send and master effect, including the safety limiter." },
    Budget { module: "looper", worst_pct: 3.0, covers: "The audio looper and the line input." },
    Budget { module: "time", worst_pct: 1.0, covers: "Transport, sequencer, clips, arp." },
    Budget { module: "mix", worst_pct: 1.0, covers: "Summing, levels, meters, the output copy." },
];

/// The budget for a module, if it has one.
pub fn for_module(module: &str) -> Option<&'static Budget> { MPC_BUDGETS.iter().find(|b| b.module == module) }

impl Budget {
    /// The worst block allowed, in ns, for a period.
    pub fn worst_ns(&self, p: Period) -> u64 { (p.ns() as f64 * self.worst_pct / 100.0) as u64 }
}

/// The probe name of the whole audio callback.
pub const PERIOD: &str = "period";

/// The budget share for a probe module: the whole thread's for
/// [`PERIOD`], else the module's own, if it has one.
pub fn pct_for(module: &str) -> Option<f64> {
    if module == PERIOD { Some(AUDIO_THREAD_WORST_PCT) } else { for_module(module).map(|b| b.worst_pct) }
}

/// The block a verdict judges: the worst for [`PERIOD`], the p99.9 for an
/// engine (see the module doc).
pub fn judged_ns(module: &str, s: &crate::Summary) -> u64 { if module == PERIOD { s.max_ns } else { s.p999_ns } }

/// What the judge says in its own words: "worst" or "p99.9".
pub fn judged_word(module: &str) -> &'static str { if module == PERIOD { "worst" } else { "p99.9" } }

/// A summary against its module's budget: (the judged block as a share of
/// the period, the verdict), or `None` for a module with no budget or a
/// summary with no blocks.
pub fn judge(module: &str, s: &crate::Summary, p: Period) -> Option<(f64, Verdict)> {
    let b = pct_for(module)?;
    if s.blocks == 0 { return None; }
    let pct = p.pct(judged_ns(module, s) as f64);
    Some((pct, Verdict::of(pct, b)))
}

/// How a measured block compares with a budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Under 75 % of the budget.
    Fits,
    /// 75 to 100 % of the budget: fits, with little room.
    Tight,
    Over,
}

impl Verdict {
    pub fn of(used_pct: f64, budget_pct: f64) -> Verdict {
        if used_pct > budget_pct { Verdict::Over } else if used_pct > 0.75 * budget_pct { Verdict::Tight } else { Verdict::Fits }
    }

    pub fn word(self) -> &'static str {
        match self { Verdict::Fits => "fits", Verdict::Tight => "tight", Verdict::Over => "OVER" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mpc_period() {
        assert_eq!(MPC.ns(), 2_902_494);
        assert!((MPC.us() - 2902.494).abs() < 1e-3);
        assert!((MPC.pct(577_000.0) - 19.88).abs() < 0.01, "round 6's synth worst");
    }

    #[test]
    fn the_budgets_add_up_to_the_thread_budget() {
        let sum: f64 = MPC_BUDGETS.iter().map(|b| b.worst_pct).sum();
        assert!((sum - AUDIO_THREAD_WORST_PCT).abs() < 1e-9, "{sum}");
        for (i, b) in MPC_BUDGETS.iter().enumerate() {
            assert!(MPC_BUDGETS[..i].iter().all(|o| o.module != b.module), "{} twice", b.module);
        }
        assert_eq!(for_module("synth").unwrap().worst_ns(MPC), 580_498);
        assert!(for_module("nope").is_none());
    }

    #[test]
    fn engines_are_judged_on_p999_and_the_period_on_its_worst() {
        // Round 7's looper: p99 and p99.9 fit, one stray period did not.
        let mut pr = crate::Probe::new(MPC.ns());
        for _ in 0..9999 { pr.record(30_000, 128, 0); }
        pr.record(200_000, 128, 0);
        let s = pr.lifetime();
        let (pct, v) = judge("looper", &s, MPC).unwrap();
        assert!(pct < 3.0 && v != Verdict::Over, "{pct} {v:?}");
        let (pct, v) = judge(PERIOD, &s, MPC).unwrap();
        assert!((pct - 6.89).abs() < 0.01 && v == Verdict::Fits, "{pct}");
        assert_eq!((judged_word("looper"), judged_word(PERIOD)), ("p99.9", "worst"));
        assert!(judge("nope", &s, MPC).is_none());
        assert!(judge("synth", &crate::Summary::default(), MPC).is_none());
        assert_eq!(pct_for(PERIOD), Some(60.0));
    }

    #[test]
    fn verdicts() {
        assert_eq!(Verdict::of(10.0, 20.0), Verdict::Fits);
        assert_eq!(Verdict::of(19.9, 20.0), Verdict::Tight);
        assert_eq!(Verdict::of(24.1, 20.0), Verdict::Over);
        assert_eq!(Verdict::Over.word(), "OVER");
    }
}
