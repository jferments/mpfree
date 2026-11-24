//! Where the sound's time goes, per track: System status's "The tracks
//! using the most" on Sound's numbers and in a saved report (Jay's go,
//! 24 Nov 2026 00:18Z, "Yes, as drawn"; the proposal and mockup are in
//! mpfree-claude `notes/status-monitor/per-track.md`).
//!
//! One period in [`EVERY`] the audio loop counts, for each sequencer track
//! and each kit no track plays:
//!
//! - **its kit's voices:** the pads' measured time that period, shared out
//!   by what each kit's voices cost in grains (`Engine::count_kits`). No
//!   voice is timed: profiling.md's rule is never to time per voice.
//! - **its own instrument** (a synth copy or a plugin) and **its pads'
//!   synth copies**, each timed by the track host (two clock reads each).
//! - **the inserts** on its strip and on its kit's pad channels, each
//!   chain timed by the mixer (two clock reads each).
//! - **the Synth tab's synth**, its measured time, for the first track
//!   playing it.
//!
//! A kit's time goes to the track that plays it (its home,
//! `TrackKits::homes`), or stays the kit's when no track plays it ("Kit 2 ·
//! pads only"). Every 250 ms, as CPU by engine publishes, each count
//! becomes a share of the period (the same measure as the engines' shares)
//! and goes into [`TrackShared`]; the screen thread reads it there, sorts
//! it and words it. Nothing here allocates on the audio thread.

use std::sync::atomic::{AtomicU32, Ordering};

use mpfree_prof::budget::MPC;

use crate::mixing::Spender;

/// Tracks counted (every sequencer track).
pub const TRACKS: usize = mpfree_seq::MAX_TRACKS;
/// Kits counted (every kit the sampler can play at once).
pub const KITS: usize = mpfree_sampler::engine::MAX_KITS;
/// The audio loop counts one period in this many, so the clock reads cost
/// a quarter of what they would.
pub const EVERY: u64 = 4;
/// How many tracks Sound's numbers and the report list.
pub const SHOWN: usize = 5;

/// What the audio loop publishes every 250 ms: each track's and each
/// track-less kit's share of the period in the last window, in hundredths
/// of a percent (relaxed atomics: a look may mix two windows, which only
/// moves a number by a window's change).
#[derive(Debug)]
pub struct TrackShared {
    tracks: Box<[AtomicU32]>,
    kits: Box<[AtomicU32]>,
    /// Set once the loop has published.
    ready: AtomicU32,
}

impl Default for TrackShared {
    fn default() -> TrackShared {
        TrackShared {
            tracks: (0..TRACKS).map(|_| AtomicU32::new(0)).collect(),
            kits: (0..KITS).map(|_| AtomicU32::new(0)).collect(),
            ready: AtomicU32::new(0),
        }
    }
}

/// Whose a listed share is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    /// Sequencer track `t` (0 is track 1).
    Track(usize),
    /// Kit `k` (0 is kit 1), which no track plays.
    Kit(usize),
}

impl TrackShared {
    /// The [`SHOWN`] largest shares (%), largest first, leaving out those
    /// under 0.05 %. None until the loop has published.
    pub fn top(&self) -> Option<Vec<(Who, f64)>> {
        if self.ready.load(Ordering::Relaxed) == 0 {
            return None;
        }
        let pct = |a: &AtomicU32| f64::from(a.load(Ordering::Relaxed)) / 100.0;
        let mut all: Vec<(Who, f64)> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(t, a)| (Who::Track(t), pct(a)))
            .chain(
                self.kits
                    .iter()
                    .enumerate()
                    .map(|(k, a)| (Who::Kit(k), pct(a))),
            )
            .filter(|(_, p)| *p >= 0.05)
            .collect();
        all.sort_by(|a, b| b.1.total_cmp(&a.1));
        all.truncate(SHOWN);
        Some(all)
    }

    /// Puts shares in (%), as the audio loop does (and the screens'
    /// samples).
    pub fn set(&self, who: Who, pct: f64) {
        let v = (pct.max(0.0) * 100.0).round().min(f64::from(u32::MAX)) as u32;
        let slot = match who {
            Who::Track(t) => self.tracks.get(t),
            Who::Kit(k) => self.kits.get(k),
        };
        if let Some(a) = slot {
            a.store(v, Ordering::Relaxed);
        }
        self.ready.store(1, Ordering::Relaxed);
    }
}

/// The audio loop's counts between two publishes. Made before the sound
/// starts (it allocates); after that nothing allocates.
pub struct TrackCosts {
    /// Each track's counted ns: its instrument and inserts, its kits'
    /// share of the pads, the Synth tab's synth.
    track_ns: Box<[u64]>,
    /// Each kit's counted ns from its copies and pad channels' inserts,
    /// before it is given to its home.
    kit_ns: Box<[u64]>,
    /// Scratch for a period's counts of each kit's voices' cost, in
    /// grain-frames (`Engine::take_kit_counts`).
    units_now: Box<[u64]>,
    /// Periods counted this window.
    periods: u64,
}

impl Default for TrackCosts {
    fn default() -> TrackCosts {
        TrackCosts::new()
    }
}

impl TrackCosts {
    pub fn new() -> TrackCosts {
        TrackCosts {
            track_ns: vec![0; TRACKS].into_boxed_slice(),
            kit_ns: vec![0; KITS].into_boxed_slice(),
            units_now: vec![0; KITS].into_boxed_slice(),
            periods: 0,
        }
    }

    /// Whether period `i` is counted.
    pub fn counts(i: u64) -> bool {
        i.is_multiple_of(EVERY)
    }

    /// A counted period's numbers, after its sound is made: the kits'
    /// voices' counts (`kits`, which this zeroes), the pads' measured time
    /// that period, the Synth tab's synth's and the first track playing it,
    /// each kit's home (`homes[k]` is its track + 1, 0: none), and the
    /// mixer's counts (`spent`, called with a sink).
    pub fn add_period(
        &mut self,
        kits: impl FnOnce(&mut [u64]),
        pads_ns: u64,
        synth: Option<(usize, u64)>,
        homes: &[u8],
        spent: impl FnOnce(&mut dyn FnMut(Spender, u64)),
    ) {
        self.periods += 1;
        self.units_now.fill(0);
        kits(&mut self.units_now);
        // The pads' time by what each kit's voices cost.
        let total: u64 = self.units_now.iter().sum();
        if total > 0 {
            for (k, &u) in self.units_now.iter().enumerate() {
                if u > 0 {
                    let ns = (pads_ns as u128 * u as u128 / total as u128) as u64;
                    self.kit_ns[k] = self.kit_ns[k].saturating_add(ns);
                }
            }
        }
        if let Some((t, ns)) = synth {
            if let Some(n) = self.track_ns.get_mut(t) {
                *n = n.saturating_add(ns);
            }
        }
        let (track_ns, kit_ns) = (&mut self.track_ns, &mut self.kit_ns);
        spent(&mut |who, ns| {
            let n = match who {
                Spender::Track(t) => track_ns.get_mut(t),
                Spender::Kit(k) => kit_ns.get_mut(k),
            };
            if let Some(n) = n {
                *n = n.saturating_add(ns);
            }
        });
        // Each kit goes to its home now: a home can change between
        // publishes, and the time belongs to who played it then.
        for (k, ns) in self.kit_ns.iter_mut().enumerate() {
            let home = homes.get(k).copied().unwrap_or(0);
            if home > 0 && *ns > 0 {
                if let Some(t) = self.track_ns.get_mut(usize::from(home) - 1) {
                    *t = t.saturating_add(std::mem::take(ns));
                }
            }
        }
    }

    /// Every 250 ms: each count as a share of the period into `shared`,
    /// then starts the window again. A window with no counted period
    /// leaves `shared` as it was.
    pub fn publish(&mut self, shared: &TrackShared) {
        if self.periods == 0 {
            return;
        }
        let per = self.periods as f64;
        for (t, ns) in self.track_ns.iter_mut().enumerate() {
            shared.set(Who::Track(t), MPC.pct(std::mem::take(ns) as f64 / per));
        }
        for (k, ns) in self.kit_ns.iter_mut().enumerate() {
            shared.set(Who::Kit(k), MPC.pct(std::mem::take(ns) as f64 / per));
        }
        self.periods = 0;
    }
}

/// A listed share's words on Sound's numbers: "Track 3 · Drums" with the
/// track's name, or "Kit 2 · pads only". `name` gives a track's name and
/// a kit's (None: just the number).
pub fn label(who: Who, name: impl Fn(Who) -> Option<String>) -> String {
    match (who, name(who)) {
        (Who::Track(t), Some(n)) if !n.is_empty() => format!("Track {} · {n}", t + 1),
        (Who::Track(t), _) => format!("Track {}", t + 1),
        (Who::Kit(k), _) => format!("Kit {} · pads only", k + 1),
    }
}

/// The same in a saved report, with no name someone typed ("track 3",
/// "kit 2, pads only").
pub fn report_label(who: Who) -> String {
    match who {
        Who::Track(t) => format!("track {}", t + 1),
        Who::Kit(k) => format!("kit {}, pads only", k + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A counted period's pads go to the kits by their voices' cost, each
    /// kit to its home track, or the kit's own while no track plays it;
    /// instruments and inserts go to their owner; the shares are of the
    /// period, as the engines' are.
    #[test]
    fn the_tracks_shares_come_from_their_kits_instruments_and_inserts() {
        let period_ns = MPC.us() * 1000.0;
        let mut c = TrackCosts::new();
        // Kit 1 on track 3, kit 2 on no track; track 5 has its own synth.
        let mut homes = [0u8; KITS];
        homes[0] = 3;
        for _ in 0..2 {
            c.add_period(
                |u| {
                    u[0] = 3;
                    u[1] = 1;
                },
                (0.40 * period_ns) as u64,
                Some((6, (0.05 * period_ns) as u64)),
                &homes,
                |sink| {
                    sink(Spender::Track(4), (0.10 * period_ns) as u64);
                    sink(Spender::Kit(0), (0.02 * period_ns) as u64);
                    sink(Spender::Kit(1), (0.01 * period_ns) as u64);
                },
            );
        }
        let shared = TrackShared::default();
        assert_eq!(shared.top(), None, "nothing before the first publish");
        c.publish(&shared);
        let top = shared.top().unwrap();
        let near = |who: Who, want: f64| {
            let got = top.iter().find(|(w, _)| *w == who).map(|(_, p)| *p);
            assert!(
                got.is_some_and(|p| (p - want).abs() < 0.02),
                "{who:?}: {got:?}, want {want}"
            );
        };
        // Track 3: three quarters of the pads (30 %) and kit 1's inserts.
        near(Who::Track(2), 32.0);
        near(Who::Kit(1), 11.0);
        near(Who::Track(4), 10.0);
        near(Who::Track(6), 5.0);
        assert_eq!(top[0].0, Who::Track(2), "largest first");
        assert_eq!(top.len(), 4, "the rest are none");
        // A window with nothing counted keeps the last shares.
        c.publish(&shared);
        assert_eq!(shared.top().unwrap().len(), 4);
    }

    /// At most five are listed, and the words name tracks by their
    /// number and name, kits as pads only, and a report by number only.
    #[test]
    fn five_are_listed_with_their_words() {
        let shared = TrackShared::default();
        for t in 0..8 {
            shared.set(Who::Track(t), 1.0 + t as f64);
        }
        let top = shared.top().unwrap();
        assert_eq!(top.len(), SHOWN);
        assert_eq!(top[0], (Who::Track(7), 8.0));
        let name = |w: Who| matches!(w, Who::Track(2)).then(|| "Drums".to_string());
        assert_eq!(label(Who::Track(2), name), "Track 3 · Drums");
        assert_eq!(label(Who::Track(4), name), "Track 5");
        assert_eq!(label(Who::Kit(1), name), "Kit 2 · pads only");
        assert_eq!(report_label(Who::Track(2)), "track 3");
        assert_eq!(report_label(Who::Kit(1)), "kit 2, pads only");
        assert!(TrackCosts::counts(0) && !TrackCosts::counts(1) && TrackCosts::counts(EVERY));
    }
}
