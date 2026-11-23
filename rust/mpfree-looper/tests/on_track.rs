//! A loop on an audio track (audio tracks phase 4, step 4b): the track
//! player says where it sounds (`Looper::play`), and the looper renders
//! it there only, at its place counted from each run's origin, at full
//! gain from the first frame, into its own buffer, for any period size.
//! A new origin going straight on crossfades the place; after silence it
//! starts where it should be with no crossfade; its own launch, mute and
//! solo are refused; off the track it launches itself again.
//!
//! 22.05 kHz at 240 BPM in 4/4: a bar is exactly 22 050 frames (1 s) and
//! 3 840 ticks. The loop is a one-bar ramp, so its sound says its place.

use mpfree_looper::{Command as L, Looper, Phase, PlayRun, FADE};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050;
const BAR_TICKS: f64 = 3_840.0;
/// The transport starts playing here, so bar lines are at START + k * BAR.
const START: u64 = 1_000;

/// The ramp the loop records: each bar from 0 up to nearly 1 (minus that
/// on the right).
fn ramp(f: u64) -> [f32; 2] {
    let v = ((f + BAR - START % BAR) % BAR) as f32 / BAR as f32;
    [v, -v]
}

/// Where a loop sounds, on absolute frames.
#[derive(Clone, Copy, Debug)]
struct Told {
    from: u64,
    to: u64,
    origin: f64,
}

struct Rig {
    t: Transport,
    l: Looper,
    b: Block,
    frame: u64,
    period: usize,
    /// Loop 0's own buffer, every frame so far (left channel).
    own: Vec<f32>,
}

impl Rig {
    /// Loop 0 recorded as a one-bar ramp from bar 2, playing.
    fn new(period: usize) -> Rig {
        let mut r = Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            l: Looper::new(RATE, 20),
            b: Block::new(),
            frame: 0,
            period,
            own: Vec::new(),
        };
        assert!(r.t.command(START, T::Play));
        r.run(START + BAR / 2, None, &[]);
        assert!(r.l.command(L::SetBars(Some(1))));
        assert!(r.l.command(L::Record));
        r.run(START + 3 * BAR, None, &[]);
        assert_eq!(r.l.loop_status(0).unwrap().phase, Phase::Playing);
        r
    }

    /// The loop's own origin, in ticks.
    fn origin(&self) -> f64 {
        self.l.loop_status(0).unwrap().origin as f64
    }

    /// Periods to `to`: with `on`, loop 0 is on a track from the first
    /// and told `told`, cut to each period.
    fn run(&mut self, to: u64, on: Option<()>, told: &[Told]) {
        let mut input = vec![0.0f32; 2 * self.period];
        while self.frame < to {
            let n = (self.period as u64).min(to - self.frame) as usize;
            let p0 = self.frame;
            for i in 0..n {
                let x = ramp(p0 + i as u64);
                input[2 * i] = x[0];
                input[2 * i + 1] = x[1];
            }
            if on.is_some() {
                let runs: Vec<PlayRun> = told
                    .iter()
                    .filter(|t| t.from < p0 + n as u64 && t.to > p0)
                    .map(|t| PlayRun {
                        from: (t.from.max(p0) - p0) as u32,
                        to: (t.to.min(p0 + n as u64) - p0) as u32,
                        origin: t.origin,
                    })
                    .collect();
                assert!(self.l.play(0, &runs));
            }
            assert!(self.t.process(n as u32, &mut self.b));
            self.l.process_each(&self.b, &input[..2 * n]);
            let o = self.l.outs().loop_out(0);
            self.own.extend(o.chunks(2).map(|c| c[0]));
            self.frame += n as u64;
        }
    }

    fn at(&self, f: u64) -> f32 {
        self.own[f as usize]
    }
}

/// The ramp's value at frame `f` played from `origin` (ticks), and
/// whether `f` is clear of the ramp's wrap (where interpolation and the
/// seam blend its ends).
fn place(f: u64, origin: f64) -> (f32, bool) {
    let tick = (f - START) as f64 * BAR_TICKS / BAR as f64;
    let p = (tick - origin).rem_euclid(BAR_TICKS) / BAR_TICKS;
    (p as f32, p > 0.05 && p < 0.95)
}

const PERIODS: [usize; 5] = [1, 7, 128, 1000, 4096];

#[test]
fn it_sounds_only_where_told_from_the_runs_origin_at_full_gain() {
    let mut first: Option<Vec<f32>> = None;
    for period in PERIODS {
        let mut r = Rig::new(period);
        let on = r.frame;
        let o = r.origin() + BAR_TICKS / 2.0;
        let (a, b) = (on + 5_003, on + 5_003 + BAR + 7_000);
        r.run(
            on + 3 * BAR,
            Some(()),
            &[Told {
                from: a,
                to: b,
                origin: o,
            }],
        );
        for f in on..r.frame {
            let v = r.at(f);
            if f < a || f >= b {
                assert_eq!(v, 0.0, "period {period} frame {f}: told nothing");
                continue;
            }
            let (want, clear) = place(f, o);
            if clear {
                assert!(
                    (v - want).abs() < 1e-3,
                    "period {period} frame {f}: {v} not {want}"
                );
            }
        }
        // Full gain on its first frame: the track fades it, not the looper.
        let (want, _) = place(a, o);
        assert!((r.at(a) - want).abs() < 1e-3, "period {period}");
        // The same for any period size.
        let got = r.own[on as usize..].to_vec();
        match &first {
            None => first = Some(got),
            Some(f) => {
                assert_eq!(f.len(), got.len());
                for (i, (x, y)) in f.iter().zip(&got).enumerate() {
                    assert!((x - y).abs() < 1e-6, "period {period} frame {i}: {x} {y}");
                }
            }
        }
    }
}

#[test]
fn its_own_launch_mute_and_solo_are_refused_and_off_the_track_it_launches_itself() {
    let mut r = Rig::new(128);
    let on = r.frame;
    let o = r.origin();
    let told = [Told {
        from: on,
        to: on + 4 * BAR,
        origin: o,
    }];
    r.run(on + 1_000, Some(()), &told);
    assert!(r.l.on_track(0));
    let refused = r.l.stats().refused;
    assert!(r.l.launch(0, false, None));
    assert!(r.l.command(L::Mute(true)));
    assert!(r.l.command(L::Solo(true)));
    r.run(on + BAR, Some(()), &told);
    assert_eq!(r.l.stats().refused, refused + 3);
    let s = r.l.loop_status(0).unwrap();
    assert!(s.on && !s.muted);
    for f in on + 2_000..on + BAR {
        let (want, clear) = place(f, o);
        if clear {
            assert!((r.at(f) - want).abs() < 1e-3, "frame {f}");
        }
    }
    // Off the track: it plays itself again, in place.
    r.l.leave_track(0);
    assert!(!r.l.on_track(0));
    let off = r.frame;
    r.run(off + BAR, None, &[]);
    for f in off..off + BAR {
        let (want, clear) = place(f, o);
        if clear {
            assert!((r.at(f) - want).abs() < 1e-3, "frame {f}");
        }
    }
    // And its launch works again.
    assert!(r.l.launch(0, false, None));
    let stop = r.frame;
    r.run(stop + BAR, None, &[]);
    assert!(r.own[(stop + FADE as u64 + 1) as usize..]
        .iter()
        .all(|&v| v == 0.0));
}

#[test]
fn a_new_origin_straight_on_crossfades_the_place() {
    for period in PERIODS {
        let mut r = Rig::new(period);
        let on = r.frame;
        let (o1, o2) = (r.origin(), r.origin() + BAR_TICKS / 4.0);
        let (a, c, d) = (on + 3_001, on + 3_001 + BAR / 2, on + 2 * BAR);
        r.run(
            d,
            Some(()),
            &[
                Told {
                    from: a,
                    to: c,
                    origin: o1,
                },
                Told {
                    from: c,
                    to: d,
                    origin: o2,
                },
            ],
        );
        // Up to the change, the old place; from a fade after it, the new.
        let (old, _) = place(c - 1, o1);
        assert!((r.at(c - 1) - old).abs() < 1e-3, "period {period}");
        for f in c + FADE as u64 + 1..d {
            let (want, clear) = place(f, o2);
            if clear {
                assert!((r.at(f) - want).abs() < 1e-3, "period {period} frame {f}");
            }
        }
        // In between, a crossfade: each frame between the two places.
        for f in c..c + FADE as u64 {
            let (x, _) = place(f, o1);
            let (y, _) = place(f, o2);
            let v = r.at(f);
            assert!(
                v >= x.min(y) - 1e-3 && v <= x.max(y) + 1e-3,
                "period {period} frame {f}: {v} not between {x} and {y}"
            );
        }
        assert!(
            (r.at(c) - place(c, o1).0).abs() < 0.01,
            "period {period}: starts at the old place"
        );
    }
}

#[test]
fn after_silence_it_starts_at_its_place_with_no_crossfade() {
    for period in PERIODS {
        let mut r = Rig::new(period);
        let on = r.frame;
        let o = r.origin() + 1_000.0;
        let (a, b) = (on + 2_000, on + 9_000);
        let (c, d) = (on + 14_096, on + BAR + 6_000);
        r.run(
            d + 500,
            Some(()),
            &[
                Told {
                    from: a,
                    to: b,
                    origin: o,
                },
                Told {
                    from: c,
                    to: d,
                    origin: o,
                },
            ],
        );
        for f in (b..c).chain(d..d + 500) {
            assert_eq!(r.at(f), 0.0, "period {period} frame {f}");
        }
        for f in c..d {
            let (want, clear) = place(f, o);
            if clear {
                assert!(
                    (r.at(f) - want).abs() < 1e-3,
                    "period {period} frame {f}: {}",
                    r.at(f)
                );
            }
        }
    }
}

#[test]
fn runs_out_of_order_are_refused_and_counted() {
    let mut r = Rig::new(128);
    let refused = r.l.stats().refused;
    let bad = [
        PlayRun {
            from: 10,
            to: 20,
            origin: 0.0,
        },
        PlayRun {
            from: 0,
            to: 5,
            origin: 0.0,
        },
    ];
    assert!(!r.l.play(0, &bad));
    assert!(!r.l.play(9, &[]));
    assert_eq!(r.l.stats().refused, refused + 2);
}
