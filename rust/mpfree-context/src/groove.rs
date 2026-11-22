//! The groove of played drums, kept as residual timing (contract 3 of
//! the design).
//!
//! Swing is a playback setting in mpfree (the clip's, the track's or the
//! transport's), applied by whoever schedules notes. So a groove template
//! never holds swing: each cell holds what is left of the drummer's timing
//! once the swing playback will apply has been taken out, measured from
//! where playback would put the swung step. A note written at the step
//! plus that residual, then swung by playback, lands where the drummer
//! played. Swing is never applied twice.
//!
//! Offsets are shares of a step, so they follow the tempo.

use mpfree_transport::time::{Swing, Tick, PPQ};

/// One 1/16 in ticks: the groove's grid.
pub const STEP: Tick = (PPQ / 4) as Tick;
/// The most steps a bar can have (a bar of 8/4 in 1/16s).
pub const MAX_BAR_STEPS: usize = 32;

/// The inverse of [`Swing::apply`]: the straight tick that playback's
/// swing moves to `heard`. Swing is a piecewise straight line that only
/// ever moves ticks within their pair of grid steps, so this is exact.
pub fn unswing(swing: Swing, heard: f64) -> f64 {
    if swing.amount == 50 {
        return heard;
    }
    let g = f64::from(swing.grid);
    let pair = 2.0 * g;
    let base = (heard / pair).floor() * pair;
    let y = heard - base;
    let off = g * f64::from(swing.amount) / 50.0;
    let x = if y < off {
        y * g / off
    } else {
        g + (y - off) * g / (pair - off)
    };
    base + x
}

/// Which drum a hit is, for reading and placing a groove.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lane {
    Kick,
    Snare,
    Hats,
    Other,
}

impl Lane {
    pub const ALL: [Lane; 4] = [Lane::Kick, Lane::Snare, Lane::Hats, Lane::Other];

    const fn index(self) -> usize {
        match self {
            Lane::Kick => 0,
            Lane::Snare => 1,
            Lane::Hats => 2,
            Lane::Other => 3,
        }
    }
}

/// A drum hit as heard: its song tick after playback's swing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub lane: Lane,
    pub heard: f64,
    pub velocity: u8,
}

/// One lane at one step of the bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    /// Residual timing, as a share of a step (negative is early).
    pub offset: f32,
    /// Loudness as a share of the lane's average.
    pub level: f32,
    /// Hits it was read from (0: nothing heard there).
    pub hits: u16,
}

impl Cell {
    const EMPTY: Cell = Cell {
        offset: 0.0,
        level: 1.0,
        hits: 0,
    };
}

/// A groove: per lane and per step of the bar, the residual timing and
/// loudness, against the swing playback applies.
#[derive(Clone, Debug, PartialEq)]
pub struct GrooveTemplate {
    cells: [[Cell; MAX_BAR_STEPS]; 4],
    /// Steps in a bar (16 in 4/4).
    pub bar_steps: usize,
    /// The playback swing the offsets are residual against.
    pub swing: Swing,
    /// Read from drums someone played (then a style's slow drift stays
    /// off: the drummer's feel wins).
    pub played: bool,
}

impl GrooveTemplate {
    /// No groove: every step on its swung line.
    pub fn straight(bar_steps: usize, swing: Swing) -> GrooveTemplate {
        GrooveTemplate {
            cells: [[Cell::EMPTY; MAX_BAR_STEPS]; 4],
            bar_steps: bar_steps.clamp(1, MAX_BAR_STEPS),
            swing,
            played: false,
        }
    }

    /// Read played drums against the swing playback applies to them.
    /// Each hit is placed on its nearest step (in straight time) and the
    /// cell keeps the mean residual and loudness of every hit there, over
    /// every bar played.
    pub fn read(hits: &[Hit], bar_steps: usize, swing: Swing) -> GrooveTemplate {
        let mut g = GrooveTemplate::straight(bar_steps, swing);
        g.played = !hits.is_empty();
        let n = g.bar_steps as i64;
        let mut sum_off = [[0.0f64; MAX_BAR_STEPS]; 4];
        let mut sum_vel = [[0.0f64; MAX_BAR_STEPS]; 4];
        let mut lane_vel = [0.0f64; 4];
        let mut lane_n = [0u32; 4];
        for h in hits {
            let straight = unswing(swing, h.heard);
            let step = (straight / STEP as f64).round();
            let residual = (straight - step * STEP as f64) / STEP as f64;
            let at = (step as i64).rem_euclid(n) as usize;
            let l = h.lane.index();
            sum_off[l][at] += residual;
            sum_vel[l][at] += f64::from(h.velocity);
            g.cells[l][at].hits += 1;
            lane_vel[l] += f64::from(h.velocity);
            lane_n[l] += 1;
        }
        for l in 0..4 {
            let mean = if lane_n[l] > 0 {
                lane_vel[l] / f64::from(lane_n[l])
            } else {
                1.0
            };
            for s in 0..g.bar_steps {
                let c = &mut g.cells[l][s];
                if c.hits > 0 {
                    let k = f64::from(c.hits);
                    c.offset = (sum_off[l][s] / k) as f32;
                    c.level = (sum_vel[l][s] / k / mean.max(1.0)) as f32;
                }
            }
        }
        g
    }

    pub fn cell(&self, lane: Lane, step: usize) -> Cell {
        self.cells[lane.index()][step % self.bar_steps]
    }

    /// The residual offset, in ticks, for a note on `step` that follows
    /// `lane` (a bass following the kick takes the kick's).
    pub fn offset_ticks(&self, lane: Lane, step: usize) -> f64 {
        f64::from(self.cell(lane, step).offset) * STEP as f64
    }

    /// The same groove against another playback swing: each cell's heard
    /// position is kept, so moving the swing knob later doesn't change
    /// where the drummer's notes land, only how the residual is split.
    pub fn against(&self, swing: Swing) -> GrooveTemplate {
        let mut g = self.clone();
        g.swing = swing;
        for l in 0..4 {
            for s in 0..self.bar_steps {
                let c = self.cells[l][s];
                if c.hits == 0 {
                    // Nothing heard there: it follows playback's swing.
                    continue;
                }
                let line = s as f64 * STEP as f64;
                let heard = self.swing.apply(line + f64::from(c.offset) * STEP as f64);
                g.cells[l][s].offset = ((unswing(swing, heard) - line) / STEP as f64) as f32;
            }
        }
        g
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EIGHTHS: u32 = PPQ / 2;
    const SIXTEENTHS: u32 = PPQ / 4;

    #[test]
    fn unswing_undoes_swing_everywhere() {
        for &(grid, amount) in &[
            (SIXTEENTHS, 61),
            (EIGHTHS, 66),
            (SIXTEENTHS, 75),
            (EIGHTHS, 50),
        ] {
            let s = Swing::new(grid, amount).unwrap();
            for t in (-2000..8000).step_by(37) {
                let t = t as f64 + 0.25;
                assert!(
                    (unswing(s, s.apply(t)) - t).abs() < 1e-9,
                    "{grid} {amount} {t}"
                );
            }
        }
    }

    /// A beat: kick 11 ms early on 1 and 3 (at 92 BPM a step is 163 ms),
    /// snare 28 ms late on 2 and 4, hats on every 1/16 swung 61 %.
    fn played(bars: i64) -> Vec<Hit> {
        let ms_to_ticks = |ms: f64| ms * 92.0 * f64::from(PPQ) / 60_000.0;
        let hat_swing = Swing::new(SIXTEENTHS, 61).unwrap();
        let mut hits = Vec::new();
        for b in 0..bars {
            let bar = b * 16 * STEP;
            for s in [0, 8] {
                hits.push(Hit {
                    lane: Lane::Kick,
                    heard: (bar + s * STEP) as f64 - ms_to_ticks(11.0),
                    velocity: 110,
                });
            }
            for s in [4, 12] {
                hits.push(Hit {
                    lane: Lane::Snare,
                    heard: (bar + s * STEP) as f64 + ms_to_ticks(28.0),
                    velocity: 100,
                });
            }
            for s in 0..16 {
                let v = if s % 2 == 0 { 90 } else { 60 };
                hits.push(Hit {
                    lane: Lane::Hats,
                    heard: hat_swing.apply((bar + s * STEP) as f64),
                    velocity: v,
                });
            }
        }
        hits
    }

    #[test]
    fn residuals_against_straight_playback_hold_the_swing() {
        let g = GrooveTemplate::read(&played(2), 16, Swing::default());
        assert!(g.played);
        let ms = |c: f64| c * 60_000.0 / (92.0 * f64::from(PPQ));
        assert!((ms(g.offset_ticks(Lane::Kick, 0)) + 11.0).abs() < 0.01);
        assert!((ms(g.offset_ticks(Lane::Snare, 12)) - 28.0).abs() < 0.01);
        // Straight playback: the hats' swing is in their residual.
        assert!(g.cell(Lane::Hats, 1).offset > 0.2);
        assert!(g.cell(Lane::Hats, 0).offset.abs() < 1e-6);
        assert!(g.cell(Lane::Hats, 1).level < g.cell(Lane::Hats, 0).level);
        assert_eq!(g.cell(Lane::Other, 3).hits, 0);
    }

    #[test]
    fn residuals_against_swung_playback_leave_the_swing_out() {
        let swing = Swing::new(SIXTEENTHS, 61).unwrap();
        let g = GrooveTemplate::read(&played(2), 16, swing);
        // Playback swings the hats already: nothing left over.
        assert!(g.cell(Lane::Hats, 1).offset.abs() < 1e-6);
        // Moving to another swing keeps where every hit lands.
        let straight = g.against(Swing::default());
        let again = GrooveTemplate::read(&played(2), 16, Swing::default());
        for s in 0..16 {
            for lane in Lane::ALL {
                let a = straight.cell(lane, s).offset;
                let b = again.cell(lane, s).offset;
                assert!((a - b).abs() < 1e-4, "{lane:?} {s}: {a} {b}");
            }
        }
    }
}
