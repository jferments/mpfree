//! The "CPU by engine" report: probe summaries turned into rows a Dev tools
//! page or a log line can show, each against its module's budget.
//!
//! Probe names are `module` or `module/detail`, such as `synth`,
//! `synth/Poly (track 2)`, `fx/Cloud reverb (master)` or `period` (the
//! whole audio callback). The module picks the budget.

use crate::budget::{self, Period, Verdict};
use crate::Summary;

/// One engine's row.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineRow {
    pub name: String,
    pub module: String,
    /// Shares of the period, in per cent.
    pub mean_pct: f64,
    pub p99_pct: f64,
    pub max_pct: f64,
    /// Share of real time over the window (the CPU it really used).
    pub cpu_pct: f64,
    /// The module's budget for its worst block, if it has one.
    pub budget_pct: Option<f64>,
    /// The worst block against the budget (only for whole-module rows:
    /// one slot alone is not the module).
    pub verdict: Option<Verdict>,
    pub over: u64,
    pub allocs: u64,
}

/// The module of a probe name: the part before the first `/`.
pub fn module_of(name: &str) -> &str { name.split('/').next().unwrap_or(name) }

/// Rows for every summary with at least one block, in the order given.
pub fn engine_rows(items: &[(&str, Summary)], p: Period) -> Vec<EngineRow> {
    let period_ns = p.ns() as f64;
    let pct = |ns: f64| 100.0 * ns / period_ns;
    items.iter().filter(|(_, s)| s.blocks > 0).map(|(name, s)| {
        let module = module_of(name);
        let whole = !name.contains('/');
        let budget_pct = if module == "period" { Some(budget::AUDIO_THREAD_WORST_PCT) } else { budget::for_module(module).map(|b| b.worst_pct) };
        EngineRow {
            name: name.to_string(),
            module: module.to_string(),
            mean_pct: pct(s.mean_ns),
            p99_pct: pct(s.p99_ns as f64),
            max_pct: pct(s.max_ns as f64),
            cpu_pct: s.realtime_pct(p.rate),
            budget_pct,
            verdict: if whole { budget_pct.map(|b| Verdict::of(pct(s.max_ns as f64), b)) } else { None },
            over: s.over,
            allocs: s.allocs,
        }
    }).collect()
}

/// The report as one log line: `cpu by engine: synth 2.1/6.0/19.9 % fits, ...`
/// (mean / p99 / worst, shares of the period).
pub fn line(rows: &[EngineRow]) -> String {
    let parts: Vec<String> = rows.iter().map(|r| {
        let mut s = format!("{} {:.1}/{:.1}/{:.1} %", r.name, r.mean_pct, r.p99_pct, r.max_pct);
        if let Some(v) = r.verdict { s.push(' '); s.push_str(v.word()); }
        if r.allocs > 0 { s.push_str(&format!(" ALLOC {}", r.allocs)); }
        s
    }).collect();
    format!("cpu by engine (mean/p99/worst of a period): {}", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::MPC;
    use crate::Probe;

    fn summary(ns: &[u64]) -> Summary {
        let mut p = Probe::new(MPC.ns());
        for &n in ns { p.record(n, 128, 0); }
        p.lifetime()
    }

    #[test]
    fn rows_carry_budgets_and_verdicts() {
        let items = [
            ("period", summary(&[300_000, 900_000])),
            ("synth", summary(&[60_000, 700_000])),
            ("synth/Poly (track 1)", summary(&[60_000, 700_000])),
            ("fx", summary(&[10_000, 20_000])),
            ("looper", Summary::default()),
            ("other", summary(&[1_000])),
        ];
        let rows = engine_rows(&items, MPC);
        assert_eq!(rows.len(), 5, "no blocks, no row");
        assert_eq!((rows[0].budget_pct, rows[0].verdict), (Some(60.0), Some(Verdict::Fits)));
        assert_eq!(rows[1].verdict, Some(Verdict::Over), "700 µs is 24 % against synth's 20 %");
        assert_eq!((rows[2].module.as_str(), rows[2].verdict, rows[2].budget_pct), ("synth", None, Some(20.0)));
        assert_eq!(rows[3].verdict, Some(Verdict::Fits));
        assert_eq!((rows[4].budget_pct, rows[4].verdict), (None, None));
        assert!((rows[1].max_pct - 24.12).abs() < 0.01);
        let l = line(&rows);
        assert!(l.starts_with("cpu by engine (mean/p99/worst of a period): period ") && l.contains("synth 13.1/24.1/24.1 % OVER") && l.contains("% OVER"), "{l}");
    }
}
