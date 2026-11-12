//! CPU by engine: the audio loop times each stage of every period with
//! mpfree-prof probes and publishes the numbers for the Dev tab's "CPU by
//! engine" page, with no lock and no allocation on the audio thread.
//!
//! Stages, in the order the page lists them:
//!
//! - `period`: the whole period's work, from the top of the loop to the
//!   write to the sound card (MIDI in, the app, every engine, the mix), not
//!   the write's own wait.
//! - `sampler`: the pads (`render_dry`), with the master grain effect.
//! - `synth`: Poly or FM.
//! - `looper`: the audio looper and the line input.
//! - `time`: the transport, the clips and the arp (`time.advance`,
//!   `app.process`).
//! - `mix`: the mixer (every sound summed, the Level as its master fader,
//!   Master safety), the metronome, and the mixer panel's state while it
//!   shows (`mixing.rs`).
//!
//! The FX are not in the pad player yet, so the page says so instead of
//! showing 0.
//!
//! Every 250 ms the audio loop publishes each stage's last window and its
//! whole-session numbers into [`EngineShared`] (`SharedSummary`: a sequence
//! lock over atomics). The screen thread reads them only while the page
//! shows. The counting allocator is not installed in the pad player, so no
//! allocation figures are shown.

use std::sync::Arc;

use mpfree_prof::budget::{self, Verdict, MPC};
use mpfree_prof::{Profiler, SharedSummary, Span, Summary};
use mpfree_ui::Tone;

/// The stages, in the order of the page.
pub const STAGES: [&str; 6] = ["period", "sampler", "synth", "looper", "time", "mix"];
pub const PERIOD: usize = 0;
pub const SAMPLER: usize = 1;
pub const SYNTH: usize = 2;
pub const LOOPER: usize = 3;
pub const TIME: usize = 4;
pub const MIX: usize = 5;

/// What the page calls each stage.
const LABELS: [&str; 6] = [
    "Whole period",
    "Sampler (pads, grains)",
    "Synth (Poly or FM)",
    "Looper and line input",
    "Transport, clips, arp",
    "Mixer, metronome",
];

/// The numbers the audio loop publishes: each stage's last window (about
/// 250 ms) and the whole session.
#[derive(Debug, Default)]
pub struct EngineShared {
    pub window: [SharedSummary; 6],
    pub life: [SharedSummary; 6],
}

/// The audio loop's probes. Made before the stream starts (it allocates);
/// after that nothing allocates.
pub struct EngineProbes {
    prof: Profiler,
    shared: Arc<EngineShared>,
}

impl EngineProbes {
    pub fn new() -> EngineProbes {
        EngineProbes {
            prof: Profiler::new(&STAGES, MPC.ns()),
            shared: Arc::new(EngineShared::default()),
        }
    }

    /// Probes publishing into `shared` (for the screens' samples).
    pub fn with_shared(shared: Arc<EngineShared>) -> EngineProbes {
        EngineProbes {
            prof: Profiler::new(&STAGES, MPC.ns()),
            shared,
        }
    }

    /// The handle the screen thread reads (sent to it once, at start).
    pub fn shared(&self) -> Arc<EngineShared> {
        Arc::clone(&self.shared)
    }

    /// Ends stage `i`'s span for this period.
    #[inline]
    pub fn stop(&mut self, i: usize, span: Span) {
        self.prof.stop(i, span, MPC.frames);
    }

    /// Records a time the loop already measured for stage `i` (the
    /// sampler's and the looper's, which it times for its status lines).
    #[inline]
    pub fn add(&mut self, i: usize, d: std::time::Duration) {
        if let Some(p) = self.prof.probe_mut(i) {
            p.record(d.as_nanos().min(u64::MAX as u128) as u64, MPC.frames, 0);
        }
    }

    /// Publishes every stage's window (and restarts it) and lifetime.
    pub fn publish(&mut self) {
        for i in 0..STAGES.len() {
            if let Some(p) = self.prof.probe_mut(i) {
                let w = p.take_window();
                self.shared.window[i].publish(&w);
                self.shared.life[i].publish(&p.lifetime());
            }
        }
    }

    /// The whole session, for the log at the end.
    pub fn lifetime(&self) -> Vec<(&'static str, Summary)> {
        self.prof.lifetime()
    }
}

/// The log line (every 30 s with the status lines, and at the end).
pub fn log_line(items: &[(&'static str, Summary)]) -> String {
    mpfree_prof::report::line(&mpfree_prof::report::engine_rows(items, MPC))
}

/// The numbers one look at the page reads: each stage's window and
/// lifetime (`None` until the audio loop has published once).
pub type Snapshot = [(Option<Summary>, Option<Summary>); 6];

pub fn read(shared: &EngineShared) -> Snapshot {
    let mut s: Snapshot = [(None, None); 6];
    for (i, slot) in s.iter_mut().enumerate() {
        let get = |x: &SharedSummary| x.read().filter(|(_, n)| *n > 0).map(|(v, _)| v);
        *slot = (get(&shared.window[i]), get(&shared.life[i]));
    }
    s
}

/// The top bar's CPU chip: the audio thread's mean share of the period
/// over the last window ("CPU 21 %"), coloured by its worst block against
/// the audio thread's budget. None until the loop has published.
pub fn cpu_chip(shared: &EngineShared) -> Option<(String, Tone)> {
    let (w, _) = read(shared)[PERIOD];
    let w = w.filter(|w| w.blocks > 0)?;
    let mean = MPC.pct(w.mean_ns);
    let t = judged(PERIOD, &w).1.map_or(Tone::Dim, tone);
    Some((format!("CPU {mean:.0} %"), t))
}

fn tone(v: Verdict) -> Tone {
    match v {
        Verdict::Fits => Tone::Good,
        Verdict::Tight => Tone::Warn,
        Verdict::Over => Tone::Bad,
    }
}

fn budget_pct(i: usize) -> f64 {
    budget::pct_for(STAGES[i]).unwrap_or(100.0)
}

/// Stage `i`'s judged block as a share of the period, and its colour, by
/// the verdict rule (Jay, 12:29Z): an engine on its p99.9 block, the whole
/// period on its worst. A stage with no budget is shown on its worst, dim.
fn judged(i: usize, s: &Summary) -> (f64, Option<Verdict>) {
    match budget::judge(STAGES[i], s, MPC) {
        Some((pct, v)) => (pct, Some(v)),
        None => (MPC.pct(s.max_ns as f64), None),
    }
}

/// The left panel: the last 250 ms of each stage, mean · p99 · worst in µs
/// and the judged block (p99.9 for an engine, the worst for the period) as
/// a share of the period, coloured against its budget.
pub fn now_rows(s: &Snapshot) -> Vec<(String, String, Tone)> {
    let mut rows = vec![(
        format!("Period: {} frames at {} Hz", MPC.frames, MPC.rate),
        format!("{:.0} µs", MPC.us()),
        Tone::Dim,
    )];
    for (i, (w, _)) in s.iter().enumerate() {
        let (value, t) = match w {
            Some(w) if w.blocks > 0 => {
                let (pct, v) = judged(i, w);
                (
                    format!(
                        "{:.0} · {:.0} · {:.0} µs ({} {pct:.1} %)",
                        w.mean_ns / 1e3,
                        w.p99_ns as f64 / 1e3,
                        w.max_ns as f64 / 1e3,
                        budget::judged_word(STAGES[i])
                    ),
                    v.map_or(Tone::Dim, tone),
                )
            }
            _ => ("waiting for the audio loop".to_string(), Tone::Dim),
        };
        rows.push((LABELS[i].to_string(), value, t));
    }
    rows.push((
        "FX".to_string(),
        "not in the pad player yet".to_string(),
        Tone::Dim,
    ));
    rows.push((
        "Columns".to_string(),
        "mean · p99 · worst, last 0.25 s".to_string(),
        Tone::Dim,
    ));
    rows
}

/// The right panel: each stage's judged block since the start against its
/// budget ("p99.9 2.1 % of 3 %"), and the real CPU each stage used.
pub fn session_rows(s: &Snapshot) -> Vec<(String, String, Tone)> {
    let mut rows = Vec::new();
    for (i, (_, l)) in s.iter().enumerate() {
        let b = budget_pct(i);
        let (value, t) = match l {
            Some(l) if l.blocks > 0 => {
                let (pct, v) = judged(i, l);
                (
                    format!(
                        "{} {pct:.1} % of {b:.0} % ({}), CPU {:.1} %",
                        budget::judged_word(STAGES[i]),
                        v.map_or("no budget", |v| v.word()),
                        l.realtime_pct(MPC.rate)
                    ),
                    v.map_or(Tone::Dim, tone),
                )
            }
            _ => (format!("budget {b:.0} %"), Tone::Dim),
        };
        rows.push((LABELS[i].to_string(), value, t));
    }
    let periods = s[PERIOD].1.map_or(0, |l| l.blocks);
    let over = s[PERIOD].1.map_or(0, |l| l.over);
    rows.push((
        "Periods timed · longer than a period".to_string(),
        format!("{periods} · {over}"),
        if over == 0 { Tone::Normal } else { Tone::Bad },
    ));
    rows.push((
        "Budgets".to_string(),
        "a proposal: docs/dev/design/profiling.md".to_string(),
        Tone::Dim,
    ));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_publish_and_the_page_reads_them() {
        let mut p = EngineProbes::new();
        let shared = p.shared();
        assert!(
            read(&shared)
                .iter()
                .all(|(w, l)| w.is_none() && l.is_none()),
            "nothing before the first publish"
        );
        let rows = now_rows(&read(&shared));
        assert_eq!(rows[1].1, "waiting for the audio loop");
        assert!(rows
            .iter()
            .any(|r| r.0 == "FX" && r.1 == "not in the pad player yet"));
        // Fixed times, so the verdicts don't depend on the machine (CI runs
        // this under qemu, where even an empty span can take milliseconds).
        for _ in 0..10 {
            for i in 0..STAGES.len() {
                p.add(i, std::time::Duration::from_micros(20));
            }
        }
        p.publish();
        let s = read(&shared);
        assert_eq!(s[SAMPLER].0.unwrap().blocks, 10);
        assert_eq!(s[MIX].1.unwrap().blocks, 10);
        let now = now_rows(&s);
        assert!(
            now[1].1.ends_with(" %)") && now[1].2 == Tone::Good,
            "{:?}",
            now[1]
        );
        let session = session_rows(&s);
        assert!(
            session[0].1.starts_with("worst 0.") && session[0].1.contains("of 60 % (fits)"),
            "{}",
            session[0].1
        );
        // The engines are judged on their p99.9 block, the period on its worst.
        assert!(
            session[SAMPLER].1.starts_with("p99.9 ") && session[SAMPLER].2 == Tone::Good,
            "{:?}",
            session[SAMPLER]
        );
        assert!(
            now[1 + SAMPLER].1.contains("(p99.9 "),
            "{:?}",
            now[1 + SAMPLER]
        );
        assert_eq!(session[6].1, "10 · 0");
        p.publish();
        assert_eq!(
            read(&shared)[SAMPLER].0.unwrap().blocks,
            0,
            "windows restart"
        );
        assert_eq!(read(&shared)[SAMPLER].1.unwrap().blocks, 10);
        assert!(log_line(&p.lifetime())
            .starts_with("cpu by engine (mean/p99/worst of a period): period "));
    }
}
