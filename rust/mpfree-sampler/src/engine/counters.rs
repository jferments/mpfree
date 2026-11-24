//! What the engine counts, for Dev > Debug, the status line and the tests.

use super::*;

/// Counters for the debug page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub hits: u64,
    pub steals: u64,
    /// Events the full queue refused (never a let-go).
    pub dropped_events: u64,
    /// Of those, hits: a pad, key, take or audition that didn't play.
    pub refused_hits: u64,
    /// Pressure and live values the full queue merged into a newer value
    /// for the same pad and setting, to make room.
    pub merged_values: u64,
    /// Let-gos the full queue had no room for, kept aside to land on their
    /// frame (or with the same place's next let-go, if one comes first).
    pub late_let_gos: u64,
    pub peak_voices: usize,
    /// Grains not made for the grain budget: it was full, or the cloud had
    /// its share of a crowded budget ([`crate::grain_share`]).
    pub skipped_grains: u64,
    /// Split: parts of blocks the lane's voices were handed over for, and
    /// the ones it missed its deadline on (those voices are unheard for
    /// that part, and events wait until they are back).
    pub lane_parts: u64,
    pub lane_late: u64,
    /// Voices given up under load ([`Engine::cull`]).
    pub culls: u64,
    /// Frames the clouds wanted more grains than the budget holds, so each
    /// got a share and thinned to fit it ([`crate::grain_share`]).
    pub crowded_frames: u64,
    /// Grains ended early to keep room for new hits, each faded out over a
    /// few milliseconds, never cut ([`crate::grain_share`]).
    pub faded_grains: u64,
    /// Times a cloud wanting grains got no share of a crowded budget: it
    /// makes no new ones and finishes those it has
    /// ([`crate::grain_share`]).
    pub held_back_clouds: u64,
    /// Layer grain settings a live value or a lock couldn't hold: the
    /// kit's table of [`LAYER_VALUES`] was full ([`Event::LayerLive`]).
    pub refused_layer_values: u64,
}

impl Engine {
    /// The counters. While the lane renders its share of the voices (or
    /// was late for it), that share's skipped grains are counted as they
    /// were when it left, so they never go back.
    pub fn stats(&self) -> Stats {
        let b = match self.pool.b.as_ref() {
            Some(b) => b.budget.skipped,
            None => self.pool.b_skipped,
        };
        Stats {
            skipped_grains: self.pool.a.budget.skipped + b,
            ..self.stats
        }
    }

    pub fn active_voices(&self) -> usize {
        self.pool.iter().filter(|(v, _)| v.active).count()
    }

    /// Grain Shape turns that waited for a window table to free, every
    /// grain voice's and the master effect's ([`mpfree_grain::Stats::window_waits`]).
    /// The lane's share counts while it is here.
    pub fn window_waits(&self) -> u64 {
        let voices: u64 = self.pool.iter().map(|(_, g)| g.stats().window_waits).sum();
        voices + self.master.as_ref().map_or(0, |fx| fx.stats().window_waits)
    }
}
