//! Smart loop end against a corpus of performances, ahead of the device
//! sessions (the plan's step 4): free first loops played on the pads and
//! closed by a tap a little early or late, in many grooves, tempos, meters
//! and amounts of human timing. Each is decided as the looper decides it
//! ([`ends::decide`], from the notes played, Auto hearing to the end of
//! the reach), and each decision is sorted:
//!
//! - **right**: it moved the end to the downbeat you played (onto one of
//!   its notes, or within [`RIGHT_MS`] of its kick);
//! - **kept**: your tap stood (no clear winner, or nothing near it);
//! - **wrong**: it moved the end somewhere else. This is the one that
//!   hurts.
//!
//! The rates print with `--nocapture`, a line for each groove, timing,
//! ending and fill, so a change to the score's weights, the threshold,
//! the margin or the reach shows what it does to them. Every performance
//! comes from a seeded generator, so the corpus is the same on every run
//! and every machine.
//!
//! **The known wrong moves** ([`KNOWN_WRONG`]): at fast tempos (here 174
//! BPM) the reach is a 1/16 note, so a tap almost a 16th early reaches
//! back nearly an 8th, and a kick played an 8th before the downbeat (half
//! time's pickup on the "and" of 4) can win on its likeness to your first
//! hit, though the beat doesn't fit it. Every one lands at the far edge
//! of the reach from the tap ([`EDGE`]), and the test fails on any other
//! kind. Tuning for them waits for the device sessions' real taps.

use mpfree_looper::ends::{decide, reach, Ask, NoteLog, Verdict};
use mpfree_looper::{read_bars, NoteStart};
use mpfree_transport::time::{Tick, PPQ};

const RATE: u32 = 48_000;
/// Where the pass's frames start (any frame will do).
const AT: u64 = 100_000;

fn frames(ms: f64) -> i64 {
    (ms * f64::from(RATE) / 1000.0).round() as i64
}

/// A small seeded generator (an LCG): the same corpus everywhere.
struct Seq(u64);

impl Seq {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }

    /// Roughly normal, mean 0, deviation `sd` (the sum of four uniforms).
    fn normal(&mut self, sd: f64) -> f64 {
        let s: f64 = (0..4).map(|_| self.next() - 0.5).sum();
        s * sd * (12.0f64 / 4.0).sqrt()
    }
}

/// A meter: its bar and the beat inside it, in ticks, and how many steps
/// a groove's bar has.
#[derive(Clone, Copy)]
struct Meter {
    name: &'static str,
    bar: Tick,
    beat: Tick,
}

const FOUR: Meter = Meter {
    name: "4/4",
    bar: 4 * PPQ as Tick,
    beat: PPQ as Tick,
};
const THREE: Meter = Meter {
    name: "3/4",
    bar: 3 * PPQ as Tick,
    beat: PPQ as Tick,
};
const SIX_EIGHT: Meter = Meter {
    name: "6/8",
    bar: 3 * PPQ as Tick,
    beat: PPQ as Tick / 2,
};

/// One note of a groove's bar: its step, pad, how hard, and how far
/// behind the grid it sits (a laid-back snare), in ms.
type Hit = (u32, u8, u8, f64);

struct Groove {
    name: &'static str,
    meter: Meter,
    /// Steps in a bar.
    steps: u32,
    hits: &'static [Hit],
}

const KICK: u8 = 36;
const SNARE: u8 = 38;
const HAT: u8 = 42;
const OPEN: u8 = 46;
const CLAP: u8 = 39;

const GROOVES: &[Groove] = &[
    Groove {
        name: "rock",
        meter: FOUR,
        steps: 8,
        hits: &[
            (0, KICK, 120, 0.0),
            (0, HAT, 70, 0.0),
            (1, HAT, 55, 0.0),
            (2, SNARE, 110, 0.0),
            (2, HAT, 70, 0.0),
            (3, HAT, 55, 0.0),
            (4, KICK, 115, 0.0),
            (4, HAT, 70, 0.0),
            (5, HAT, 55, 0.0),
            (6, SNARE, 110, 0.0),
            (6, HAT, 70, 0.0),
            (7, HAT, 55, 0.0),
        ],
    },
    Groove {
        name: "four on the floor",
        meter: FOUR,
        steps: 8,
        hits: &[
            (0, KICK, 120, 0.0),
            (1, OPEN, 70, 0.0),
            (2, KICK, 118, 0.0),
            (2, CLAP, 100, 0.0),
            (3, OPEN, 70, 0.0),
            (4, KICK, 118, 0.0),
            (5, OPEN, 70, 0.0),
            (6, KICK, 118, 0.0),
            (6, CLAP, 100, 0.0),
            (7, OPEN, 70, 0.0),
        ],
    },
    Groove {
        name: "laid-back hip hop",
        meter: FOUR,
        steps: 16,
        hits: &[
            (0, KICK, 120, 0.0),
            (0, HAT, 60, 0.0),
            (2, HAT, 50, 0.0),
            (4, SNARE, 112, 18.0),
            (4, HAT, 60, 0.0),
            (6, HAT, 50, 0.0),
            (7, KICK, 95, 0.0),
            (8, HAT, 60, 0.0),
            (10, KICK, 110, 0.0),
            (10, HAT, 50, 0.0),
            (12, SNARE, 112, 18.0),
            (12, HAT, 60, 0.0),
            (14, HAT, 50, 0.0),
        ],
    },
    Groove {
        name: "breakbeat",
        meter: FOUR,
        steps: 16,
        hits: &[
            (0, KICK, 120, 0.0),
            (0, HAT, 65, 0.0),
            (2, KICK, 100, 0.0),
            (2, HAT, 55, 0.0),
            (4, SNARE, 115, 0.0),
            (4, HAT, 65, 0.0),
            (6, HAT, 55, 0.0),
            (7, SNARE, 45, 0.0),
            (8, HAT, 65, 0.0),
            (9, SNARE, 45, 0.0),
            (10, KICK, 105, 0.0),
            (10, HAT, 55, 0.0),
            (11, KICK, 90, 0.0),
            (12, SNARE, 115, 0.0),
            (12, HAT, 65, 0.0),
            (14, HAT, 55, 0.0),
            (15, SNARE, 45, 0.0),
        ],
    },
    Groove {
        name: "half time",
        meter: FOUR,
        steps: 8,
        hits: &[
            (0, KICK, 120, 0.0),
            (0, HAT, 65, 0.0),
            (1, HAT, 55, 0.0),
            (2, HAT, 65, 0.0),
            (3, HAT, 55, 0.0),
            (4, SNARE, 118, 0.0),
            (4, HAT, 65, 0.0),
            (5, HAT, 55, 0.0),
            (6, HAT, 65, 0.0),
            (7, KICK, 90, 0.0),
            (7, HAT, 55, 0.0),
        ],
    },
    Groove {
        name: "sparse",
        meter: FOUR,
        steps: 4,
        hits: &[(0, KICK, 120, 0.0), (2, SNARE, 110, 0.0)],
    },
    Groove {
        name: "waltz",
        meter: THREE,
        steps: 3,
        hits: &[
            (0, KICK, 120, 0.0),
            (1, SNARE, 85, 0.0),
            (2, SNARE, 85, 0.0),
        ],
    },
    Groove {
        name: "6/8",
        meter: SIX_EIGHT,
        steps: 6,
        hits: &[
            (0, KICK, 120, 0.0),
            (0, HAT, 65, 0.0),
            (1, HAT, 50, 0.0),
            (2, HAT, 50, 0.0),
            (3, SNARE, 110, 0.0),
            (3, HAT, 65, 0.0),
            (4, HAT, 50, 0.0),
            (5, HAT, 50, 0.0),
        ],
    },
];

/// How the playing ends at the downbeat: on it (the downbeat's hit, then
/// nothing), playing on through the reach as Auto listens, or stopping
/// before it (nothing played there: the tap must stand).
#[derive(Clone, Copy, PartialEq)]
enum After {
    Stops,
    PlaysOn,
    Silent,
}

impl After {
    fn words(self) -> &'static str {
        match self {
            After::Stops => "stops",
            After::PlaysOn => "plays on",
            After::Silent => "stops before the downbeat",
        }
    }
}

/// One performance and the tap that closes it.
struct Take {
    notes: Vec<NoteStart>,
    /// The pass's first frame (the press).
    start: u64,
    /// The downbeat you played: the frame of the kick that comes round
    /// (or where it would have, when you stopped before it).
    downbeat: u64,
    /// The downbeat's notes as you played them (the kick and what you
    /// played with it); none when you stopped before it.
    on_it: Vec<u64>,
}

/// `bars` bars of `g` at `bpm` from AT (plus `early` ms of REC before
/// the first hit), each note off the grid by up to `sd` ms of human
/// timing, then the downbeat's hit and, if it plays on, the next bar's
/// first half. With `fill`, the last beat before the downbeat is a fill:
/// four sixteenths on the snare, getting louder.
#[allow(clippy::too_many_arguments)]
fn play(
    g: &Groove,
    bpm: f64,
    bars: u32,
    sd: f64,
    early: f64,
    after: After,
    fill: bool,
    s: &mut Seq,
) -> Take {
    let beat_ms = 60_000.0 / bpm * g.meter.beat as f64 / f64::from(PPQ);
    let bar_ms = beat_ms * (g.meter.bar / g.meter.beat) as f64;
    let step_ms = bar_ms / f64::from(g.steps);
    let quarter_ms = 60_000.0 / bpm;
    let end_ms = f64::from(bars) * bar_ms;
    let fill_from = end_ms - quarter_ms;
    let first = AT + frames(early) as u64;
    let mut notes = Vec::new();
    let mut downbeat = None;
    let mut on_it = Vec::new();
    let mut push = |ms: f64, key: u8, velocity: f64, s: &mut Seq| {
        let frame = (first as i64 + frames(ms + s.normal(sd))).max(AT as i64) as u64;
        let v = (velocity * (1.0 + s.normal(0.08))).clamp(1.0, 127.0) as u8;
        notes.push(NoteStart {
            frame,
            source: 0,
            key,
            velocity: v,
        });
        frame
    };
    let last_step = match after {
        After::Silent => bars * g.steps - 1,
        After::Stops => bars * g.steps,
        After::PlaysOn => bars * g.steps + g.steps / 2,
    };
    for k in 0..=last_step {
        let (bar, step) = (k / g.steps, k % g.steps);
        for &(at, key, velocity, behind) in g.hits.iter().filter(|h| h.0 == step) {
            let grid = f64::from(bar) * bar_ms + f64::from(at) * step_ms;
            if k == bars * g.steps && after == After::Stops && key != KICK {
                continue;
            }
            if fill && grid >= fill_from && grid < end_ms {
                continue;
            }
            let frame = push(grid + behind, key, f64::from(velocity), s);
            if k == bars * g.steps {
                on_it.push(frame);
                if key == KICK {
                    downbeat = Some(frame);
                }
            }
        }
    }
    if fill {
        for (i, v) in [60.0, 75.0, 90.0, 105.0].into_iter().enumerate() {
            push(fill_from + i as f64 * quarter_ms / 4.0, SNARE, v, s);
        }
    }
    notes.sort_by_key(|n| n.frame);
    Take {
        notes,
        start: AT,
        // Stopping before it, the downbeat is where it would have been.
        downbeat: downbeat.unwrap_or(first + frames(end_ms) as u64),
        on_it,
    }
}

#[derive(Default, Clone, Copy)]
struct Tally {
    right: u32,
    kept: u32,
    wrong: u32,
    /// Taps it could have moved (the downbeat within the reach) and that
    /// were at least 15 ms off it: where a move helps.
    could: u32,
    could_right: u32,
}

impl Tally {
    fn add(&mut self, o: &Tally) {
        self.right += o.right;
        self.kept += o.kept;
        self.wrong += o.wrong;
        self.could += o.could;
        self.could_right += o.could_right;
    }

    fn line(&self) -> String {
        let n = self.right + self.kept + self.wrong;
        let pct = |x: u32, of: u32| 100.0 * f64::from(x) / f64::from(of.max(1));
        format!(
            "{n:5} taps: {:5.1} % right, {:5.1} % kept, {} wrong; moved {:5.1} % of the {} taps 15 ms or more off within the reach",
            pct(self.right, n),
            pct(self.kept, n),
            self.wrong,
            pct(self.could_right, self.could),
            self.could
        )
    }
}

/// A move is right when it lands this near the downbeat you played: on
/// one of its notes (with loose timing the hat on it can come well before
/// or after the kick, and either is the downbeat), or this near the kick.
const RIGHT_MS: f64 = 30.0;

/// The wrong moves the corpus is known to make (see the top), at most.
const KNOWN_WRONG: usize = 12;

/// Where those land: at least this much of the reach from the tap.
const EDGE: f64 = 0.75;

/// A wrong move: what was played, the tap, and how far from the tap it
/// went, as a part of the reach.
struct Wrong {
    what: String,
    edge: f64,
}

/// Decide every tap from `-80` to `+80` ms around the downbeat of one
/// take, Auto hearing to the end of the reach.
fn taps(take: &Take, g: &Groove, bpm: f64, wrongs: &mut Vec<Wrong>, what: &str) -> Tally {
    let mut log = NoteLog::new();
    let reach_frames = reach(RATE, bpm);
    let right = frames(RIGHT_MS) as u64;
    let bar = g.meter.bar;
    let bars_for = |len: usize| read_bars(len, RATE, bar, None, None).map(|r| r.bars);
    let mut t = Tally::default();
    for off_ms in (-80..=80).step_by(5) {
        let tap = (take.downbeat as i64 + frames(f64::from(off_ms))) as u64;
        let heard_to = tap + reach_frames;
        log.clear();
        for n in take.notes.iter().filter(|n| n.frame <= heard_to) {
            log.push(*n);
        }
        let ask = Ask {
            rate: RATE,
            start: take.start,
            tap,
            reach: reach_frames,
            heard_to,
            bar,
            beat: g.meter.beat,
        };
        let d = decide(&log, &ask, &bars_for);
        let played = !take.on_it.is_empty();
        let on_it = |end: u64| {
            end.abs_diff(take.downbeat) <= right || take.on_it.iter().any(|&f| f.abs_diff(end) <= 1)
        };
        let off = tap.abs_diff(take.downbeat);
        let helps = played && off <= reach_frames && off >= frames(15.0) as u64;
        if helps {
            t.could += 1;
        }
        match d.verdict {
            Verdict::Moved if played && on_it(d.end) => {
                t.right += 1;
                if helps {
                    t.could_right += 1;
                }
            }
            Verdict::Moved => {
                t.wrong += 1;
                let best = d.best.expect("moved to a candidate");
                wrongs.push(Wrong {
                    what: format!(
                        "{what}, tap {off_ms:+} ms: moved {:+.1} ms from the downbeat (onset {:.2}, rhythm {:.2}, context {:.2}, score {:.2}; next {:.2})",
                        (d.end as f64 - take.downbeat as f64) * 1000.0 / f64::from(RATE),
                        best.onset,
                        best.rhythm,
                        best.context,
                        best.score,
                        d.second.map_or(0.0, |c| c.score),
                    ),
                    edge: d.end.abs_diff(tap) as f64 / reach_frames as f64,
                });
            }
            _ => t.kept += 1,
        }
    }
    t
}

/// The whole corpus: every groove at several tempos, 1, 2 and 4 bars,
/// tight to loose timing, REC on the first hit or a moment before it,
/// the bar before the downbeat as played or with a fill, and the playing
/// stopping on the downbeat, playing on, or stopping before it; every tap
/// from 80 ms early to 80 ms late in 5 ms steps. A tap moves only to the
/// downbeat you played, but for the known few at the reach's far edge.
#[test]
fn across_the_corpus_a_tap_moves_only_to_the_downbeat_you_played() {
    const TEMPOS: &[f64] = &[72.0, 90.0, 100.0, 120.0, 128.0, 140.0, 174.0];
    const BARS: &[u32] = &[1, 2, 4];
    // Human timing (the deviation of each note, ms) and REC's lead.
    const TIMING: &[(&str, f64)] = &[("tight", 2.0), ("human", 6.0), ("loose", 12.0)];
    const EARLY: &[f64] = &[0.0, 60.0];
    const AFTER: [After; 3] = [After::Stops, After::PlaysOn, After::Silent];
    let mut s = Seq(20_261_123);
    let mut wrongs = Vec::new();
    let mut total = Tally::default();
    let mut by_timing = [Tally::default(); 3];
    let mut by_after = [Tally::default(); 3];
    let mut by_fill = [Tally::default(); 2];
    let mut lines = Vec::new();
    for g in GROOVES {
        let mut groove = Tally::default();
        for &bpm in TEMPOS {
            for &bars in BARS {
                for (ti, &(timing, sd)) in TIMING.iter().enumerate() {
                    for &early in EARLY {
                        for (ai, &after) in AFTER.iter().enumerate() {
                            for fill in [false, true] {
                                let take = play(g, bpm, bars, sd, early, after, fill, &mut s);
                                let what = format!(
                                    "{} at {bpm} BPM, {bars} bars, {timing}, REC {early} ms early, {}{}",
                                    g.name,
                                    if fill { "a fill, " } else { "" },
                                    after.words()
                                );
                                let t = taps(&take, g, bpm, &mut wrongs, &what);
                                groove.add(&t);
                                by_timing[ti].add(&t);
                                by_after[ai].add(&t);
                                by_fill[usize::from(fill)].add(&t);
                            }
                        }
                    }
                }
            }
        }
        lines.push(format!(
            "{:>26} ({}): {}",
            g.name,
            g.meter.name,
            groove.line()
        ));
        total.add(&groove);
    }
    for (i, (timing, sd)) in TIMING.iter().enumerate() {
        lines.push(format!(
            "{:>26} (±{sd} ms): {}",
            timing,
            by_timing[i].line()
        ));
    }
    for (i, after) in AFTER.iter().enumerate() {
        lines.push(format!("{:>26}: {}", after.words(), by_after[i].line()));
    }
    for (i, fill) in ["as played", "with a fill"].iter().enumerate() {
        lines.push(format!("{:>26}: {}", fill, by_fill[i].line()));
    }
    lines.push(format!("{:>26}: {}", "all", total.line()));
    eprintln!("smart loop end, the corpus (Auto, from the pads):");
    for l in &lines {
        eprintln!("  {l}");
    }
    for w in &wrongs {
        eprintln!("  wrong: {}", w.what);
    }
    let new: Vec<_> = wrongs
        .iter()
        .filter(|w| w.edge < EDGE)
        .map(|w| &w.what)
        .collect();
    assert!(
        new.is_empty(),
        "wrong moves away from the reach's edge: {new:#?}"
    );
    assert!(
        wrongs.len() <= KNOWN_WRONG,
        "{} wrong moves, {KNOWN_WRONG} known",
        wrongs.len()
    );
}
