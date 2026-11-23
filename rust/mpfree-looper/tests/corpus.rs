//! Smart loop end against a corpus of performances, ahead of the device
//! sessions (the plan's step 4): free first loops played on the pads and
//! closed by a tap a little early or late, in many grooves, tempos, meters
//! and amounts of human timing. Each is decided as Auto decides it when
//! a free first loop closes (`Looper`'s `close_free`): the reach from the
//! reading of the tap's length, at the song's tempo before ([`SONG`]),
//! the bar count from that reading with the beat played in it, and
//! [`ends::decide`] on the notes played, Auto hearing to the end of the
//! reach. Each decision is sorted:
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
//! **The wrong moves.** Before the rules in [`ends`], over 160 seeds
//! (31.9 million taps) it made 4,011, nearly all one kind: at 174 BPM a
//! tap almost a 16th early reaches back to a kick played an 8th before
//! the downbeat (half time's pickup on the "and" of 4), which won on its
//! likeness to the first hit, though the beat didn't fit it. (The reading
//! hears half time at 174 as 87 BPM, so the reach there is 100 ms, more
//! than a 1/16 note at 174.) Now a soft note just ahead of the first kick
//! is part of the opening hit ([`ends::FLAM_MS`]), and a hit far from the
//! tap ([`ends::FAR`]) needs the beat to back it, at least
//! [`ends::BEAT_BACKS`] and at least as well as it backs your tap. That
//! leaves 13, for 0.17 % fewer of the taps off by 15 ms or more moved to
//! the downbeat (99.28 to 99.11 %): all at 174 BPM with loose timing, on
//! taps 75 or 80 ms early, to a hit the beat backs 0.11 or less, 12 of
//! them 0.6 to 0.7 of the reach from the tap. The test plays five seeds,
//! the first and four picked for the far rule's last part, and allows
//! their [`KNOWN_WRONG`] moves of that kind, no more and no other.
//!
//! **By ear.** The same takes are also played on the hit track's kit
//! ([`mpfree_looper::hits::kit`]) and heard as the line input is, then
//! decided as Auto decides by ear: the hit track hears to the end of the
//! reach, and the finder ([`mpfree_looper::find::find`]) gets the job the
//! looper builds. No wrong move there either, but far fewer moves: on
//! seed 20261123 it moves 46 % of the taps it could (the pads about
//! 99 %), because a hit's tone is read over its first 5 ms, so the
//! downbeat's kick alone and the opening kick with a hat on it often
//! aren't alike enough to be the opening hit played again. The test
//! plays a seventh of one seed's takes ([`EAR_EVERY`]) and asserts no
//! wrong move and a floor on the moves ([`EAR_MOVES_AT_LEAST`]).

use mpfree_looper::ends::{self, decide, reach, Ask, NoteLog, Verdict};
use mpfree_looper::find::{find, FindJob};
use mpfree_looper::hits::kit::{Kit, Sound};
use mpfree_looper::hits::{HitLog, HitTrack};
use mpfree_looper::{read_bars, read_bars_with, InnerBeat, NoteStart, TempoHint, TempoKind};
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

/// The wrong moves the corpus is known to make, at most (see the top).
const KNOWN_WRONG: usize = 2;

/// Where those come from: taps at least this early, to a hit the beat
/// backs less than [`ends::BEAT_BACKS`].
const KNOWN_EARLY_MS: i32 = 75;

/// A wrong move: what was played and the tap, how early the tap was,
/// and how well the beat backs the loop that ends where it went.
struct Wrong {
    what: String,
    early_ms: i32,
    rhythm: f64,
}

/// The song's tempo before the first loop: a new project's, left from
/// before (it counts lightly in the reading).
const SONG: TempoHint = TempoHint {
    bpm: 120.0,
    kind: TempoKind::Earlier,
};

/// Decide every tap from `-80` to `+80` ms around the downbeat of one
/// take, as Auto does when a free first loop closes (`Looper`'s
/// `close_free`): the reach from the reading of the tap's length (so a
/// half time groove at 174 BPM, read at 87, has a reach of 100 ms, not a
/// 1/16 note at 174), the bar count from that reading with the beat
/// played in it, the same for every candidate, and Auto hearing to the
/// end of the reach.
fn taps(take: &Take, g: &Groove, wrongs: &mut Vec<Wrong>, what: &str) -> Tally {
    let mut log = NoteLog::new();
    let right = frames(RIGHT_MS) as u64;
    let bar = g.meter.bar;
    let beat = g.meter.beat;
    let mut t = Tally::default();
    for off_ms in (-80..=80).step_by(5) {
        let tap = (take.downbeat as i64 + frames(f64::from(off_ms))) as u64;
        let tap_len = (tap - take.start) as usize;
        let tap_reading =
            read_bars(tap_len, RATE, bar, None, Some(SONG)).expect("every take reads");
        let reach_frames = reach(RATE, tap_reading.bpm);
        let heard_to = tap + reach_frames;
        log.clear();
        for n in take.notes.iter().filter(|n| n.frame <= heard_to) {
            log.push(*n);
        }
        let inner = InnerBeat::new(&log, RATE, take.start, tap_len, bar, beat);
        let bars = read_bars_with(tap_len, RATE, bar, None, Some(SONG), &|b| inner.fit(b))
            .unwrap_or(tap_reading)
            .bars;
        let ask = Ask {
            rate: RATE,
            start: take.start,
            tap,
            reach: reach_frames,
            heard_to,
            bar,
            beat,
        };
        let d = decide(&log, &ask, &|_| Some(bars));
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
                    early_ms: -off_ms,
                    rhythm: best.rhythm,
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
/// downbeat you played.
#[test]
fn across_the_corpus_a_tap_moves_only_to_the_downbeat_you_played() {
    const TEMPOS: &[f64] = &[72.0, 90.0, 100.0, 120.0, 128.0, 140.0, 174.0];
    const BARS: &[u32] = &[1, 2, 4];
    // Human timing (the deviation of each note, ms) and REC's lead.
    const TIMING: &[(&str, f64)] = &[("tight", 2.0), ("human", 6.0), ("loose", 12.0)];
    const EARLY: &[f64] = &[0.0, 60.0];
    const AFTER: [After; 3] = [After::Stops, After::PlaysOn, After::Silent];
    // The corpus's own seed, and four that fail without the far rule's
    // last part: in three a far hit that fits the beat less well than
    // your tap would win, and in one a hit 0.7 to 0.75 of the reach away.
    const SEEDS: &[u64] = &[20_261_123, 37, 121, 231, 212];
    let mut wrongs = Vec::new();
    let mut total = Tally::default();
    let mut by_timing = [Tally::default(); 3];
    let mut by_after = [Tally::default(); 3];
    let mut by_fill = [Tally::default(); 2];
    let mut lines = Vec::new();
    let mut grooves = [Tally::default(); GROOVES.len()];
    for &seed in SEEDS {
        let mut s = Seq(seed);
        for (gi, g) in GROOVES.iter().enumerate() {
            let groove = &mut grooves[gi];
            for &bpm in TEMPOS {
                for &bars in BARS {
                    for (ti, &(timing, sd)) in TIMING.iter().enumerate() {
                        for &early in EARLY {
                            for (ai, &after) in AFTER.iter().enumerate() {
                                for fill in [false, true] {
                                    let take = play(g, bpm, bars, sd, early, after, fill, &mut s);
                                    let what = format!(
                                    "{} at {bpm} BPM, {bars} bars, {timing}, REC {early} ms early, {}{} (seed {seed})",
                                    g.name,
                                    if fill { "a fill, " } else { "" },
                                    after.words()
                                );
                                    let t = taps(&take, g, &mut wrongs, &what);
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
        }
    }
    for (g, groove) in GROOVES.iter().zip(&grooves) {
        lines.push(format!(
            "{:>26} ({}): {}",
            g.name,
            g.meter.name,
            groove.line()
        ));
        total.add(groove);
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
        .filter(|w| w.early_ms < KNOWN_EARLY_MS || w.rhythm >= ends::BEAT_BACKS)
        .map(|w| &w.what)
        .collect();
    assert!(
        new.is_empty(),
        "wrong moves not of the known kind: {new:#?}"
    );
    assert!(
        wrongs.len() <= KNOWN_WRONG,
        "{} wrong moves, {KNOWN_WRONG} known",
        wrongs.len()
    );
}

/// By ear, the take is heard from this frame on (half a second of silence
/// before the press, for the hit track to settle).
const EAR_BASE: u64 = AT - RATE as u64 / 2;

/// By ear, the hit track hears the input in periods this long.
const EAR_PERIOD: usize = 256;

/// By ear, a move is on one of the downbeat's notes when it lands this
/// near it (the finder places the end on the sound, a little after the
/// note starts).
const EAR_ON_IT_MS: f64 = 2.0;

/// What each pad sounds like on the hit track's kit, and how loud at
/// full velocity.
fn sound_of(key: u8) -> (Sound, f32) {
    match key {
        KICK => (Sound::Kick, 0.8),
        SNARE => (Sound::Snare, 0.7),
        CLAP => (Sound::Snare, 0.6),
        OPEN => (Sound::Hat, 0.35),
        _ => (Sound::Hat, 0.25),
    }
}

/// The take as the line input hears it, from [`EAR_BASE`] to `to`:
/// each note played on the kit (interleaved stereo).
fn render(take: &Take, to: u64) -> Vec<f32> {
    let frames = (to - EAR_BASE) as usize;
    let mut out = vec![0.0f32; 2 * frames];
    let mut kit = Kit::new(RATE);
    for n in take.notes.iter().filter(|n| n.frame < to) {
        let (sound, gain) = sound_of(n.key);
        let at = (n.frame - EAR_BASE) as usize;
        kit.hit(&mut out, at, sound, gain * f32::from(n.velocity) / 127.0);
    }
    out
}

/// The same taps as [`taps`], every `step_ms` from -80 to +80, decided as
/// Auto decides them by ear (recording the line input or a mix): the
/// take played on the kit, its hits heard by the hit track as far as Auto
/// hears for each tap, the reach and the bars read as for the pads (the
/// bars with the beat the hits make), and the finder's answer
/// ([`find::find`]: the hits ranked by the same rules, the start and the
/// end placed on the sound).
fn taps_by_ear(
    take: &Take,
    g: &Groove,
    step_ms: usize,
    job: &mut FindJob,
    wrongs: &mut Vec<String>,
    what: &str,
) -> Tally {
    let right = frames(RIGHT_MS) as u64;
    let near_note = frames(EAR_ON_IT_MS) as u64;
    let (bar, beat) = (g.meter.bar, g.meter.beat);
    // Each tap and its reach, in the order Auto would have heard enough.
    let mut asks: Vec<(i32, u64, u64)> = (-80..=80)
        .step_by(step_ms)
        .map(|off_ms| {
            let tap = (take.downbeat as i64 + frames(f64::from(off_ms))) as u64;
            let tap_len = (tap - take.start) as usize;
            let reading =
                read_bars(tap_len, RATE, bar, None, Some(SONG)).expect("every take reads");
            (off_ms, tap, reach(RATE, reading.bpm))
        })
        .collect();
    asks.sort_by_key(|&(_, tap, reach)| tap + reach);
    let last = asks.last().map_or(AT, |&(_, tap, reach)| tap + reach);
    let audio = render(take, last);
    let mono = |from: u64, to: u64, out: &mut [f32]| -> usize {
        out.fill(0.0);
        let mut heard = 0;
        for (i, y) in out.iter_mut().enumerate() {
            let f = from + i as u64;
            if f >= to {
                break;
            }
            if f >= EAR_BASE {
                *y = audio[2 * (f - EAR_BASE) as usize];
                heard = i + 1;
            }
        }
        heard
    };
    let mut track = HitTrack::new(RATE);
    let mut log = HitLog::new();
    let mut heard = EAR_BASE;
    let mut t = Tally::default();
    for &(off_ms, tap, reach_frames) in &asks {
        let heard_to = tap + reach_frames;
        while heard < heard_to {
            let n = ((heard_to - heard) as usize).min(EAR_PERIOD);
            let i = (heard - EAR_BASE) as usize;
            track.hear(heard, &audio[2 * i..2 * (i + n)], 0, &mut log);
            heard += n as u64;
        }
        let tap_len = (tap - take.start) as usize;
        let inner = InnerBeat::new(&log, RATE, take.start, tap_len, bar, beat);
        let bars = read_bars_with(tap_len, RATE, bar, None, Some(SONG), &|b| inner.fit(b))
            .or_else(|| read_bars(tap_len, RATE, bar, None, Some(SONG)))
            .expect("every take reads")
            .bars;
        let ask = Ask {
            rate: RATE,
            start: take.start,
            tap,
            reach: reach_frames,
            heard_to,
            bar,
            beat,
        };
        // The job as the looper makes it (`Looper::find_job`): the hits,
        // the audio round the opening hit and round the reach.
        let lead = decide(&log, &ask, &|_| Some(bars)).lead;
        job.ask = ask;
        job.bars = bars;
        job.hits.clone_from(&log);
        job.answer = None;
        job.open_from = FindJob::open_from_for(take.start + lead);
        job.open_len = mono(job.open_from, heard_to, &mut job.open);
        job.near_from = FindJob::near_from_for(tap, reach_frames);
        job.near_len = mono(job.near_from, heard_to, &mut job.near);
        find(job);
        let d = job.answer.expect("the finder answers");
        let played = !take.on_it.is_empty();
        let on_it = |end: u64| {
            end.abs_diff(take.downbeat) <= right
                || take.on_it.iter().any(|&f| f.abs_diff(end) <= near_note)
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
                wrongs.push(format!(
                    "{what}, tap {off_ms:+} ms: moved {:+.1} ms from the downbeat (onset {:.2}, rhythm {:.2}, context {:.2}, score {:.2}; next {:.2}; line-up {:.2})",
                    (d.end as f64 - take.downbeat as f64) * 1000.0 / f64::from(RATE),
                    best.onset,
                    best.rhythm,
                    best.context,
                    best.score,
                    d.second.map_or(0.0, |c| c.score),
                    job.line_up,
                ));
            }
            _ => t.kept += 1,
        }
    }
    t
}

/// By ear, the test plays every this many takes of a seed's corpus (all
/// grooves and tempos, a seventh of the rest; every take is still made,
/// so the takes are the full corpus's). `MPFREE_EAR_EVERY=1` plays all
/// 6,048.
const EAR_EVERY: usize = 7;

/// By ear, Auto moves at least this share of the taps it could (15 ms or
/// more off, the downbeat within the reach): a floor that catches the
/// finder hearing nothing, not a target. On seed 20261123 it moves 46 %.
const EAR_MOVES_AT_LEAST: f64 = 0.3;

/// The corpus by ear: the same performances played on the hit track's
/// kit and heard from the line input, decided as Auto decides by ear,
/// with no wrong move. It keeps more taps than the pads do: a kick heard
/// with a hat on it and a kick alone differ by 20 dB or more in the top
/// band, so the downbeat often isn't the opening hit played again by
/// tone, and the tap stands. Measured with `--nocapture`; set
/// `MPFREE_EAR_SEEDS` (seeds, commas between), `MPFREE_EAR_EVERY` and
/// `MPFREE_EAR_STEP_MS` (every 20 ms from -80 to +80 by default) to hear
/// more. The whole of seed 20261123 (54,432 taps) takes about a minute.
#[test]
fn by_ear_across_the_corpus_a_tap_moves_only_to_the_downbeat_you_played() {
    const TEMPOS: &[f64] = &[72.0, 90.0, 100.0, 120.0, 128.0, 140.0, 174.0];
    const BARS: &[u32] = &[1, 2, 4];
    const TIMING: &[(&str, f64)] = &[("tight", 2.0), ("human", 6.0), ("loose", 12.0)];
    const EARLY: &[f64] = &[0.0, 60.0];
    const AFTER: [After; 3] = [After::Stops, After::PlaysOn, After::Silent];
    let seeds: Vec<u64> = std::env::var("MPFREE_EAR_SEEDS")
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().parse().expect("a seed"))
                .collect()
        })
        .unwrap_or_else(|_| vec![20_261_123]);
    let step_ms: usize = std::env::var("MPFREE_EAR_STEP_MS")
        .map(|s| s.parse().expect("a step in ms"))
        .unwrap_or(20);
    let every: usize = std::env::var("MPFREE_EAR_EVERY")
        .map(|s| s.parse().expect("a whole number"))
        .unwrap_or(EAR_EVERY)
        .max(1);
    let mut k = 0usize;
    let mut job = FindJob::new(reach(RATE, f64::NAN));
    let mut wrongs = Vec::new();
    let mut total = Tally::default();
    let mut grooves = [Tally::default(); GROOVES.len()];
    let mut by_timing = [Tally::default(); 3];
    let mut by_after = [Tally::default(); 3];
    let mut by_tempo = [Tally::default(); 7];
    for &seed in &seeds {
        let mut s = Seq(seed);
        for (gi, g) in GROOVES.iter().enumerate() {
            for (bi, &bpm) in TEMPOS.iter().enumerate() {
                for &bars in BARS {
                    for (ti, &(timing, sd)) in TIMING.iter().enumerate() {
                        for &early in EARLY {
                            for (ai, &after) in AFTER.iter().enumerate() {
                                for fill in [false, true] {
                                    let take = play(g, bpm, bars, sd, early, after, fill, &mut s);
                                    k += 1;
                                    if !(k - 1).is_multiple_of(every) {
                                        continue;
                                    }
                                    let what = format!(
                                        "{} at {bpm} BPM, {bars} bars, {timing}, REC {early} ms early, {}{} (seed {seed})",
                                        g.name,
                                        if fill { "a fill, " } else { "" },
                                        after.words()
                                    );
                                    let t = taps_by_ear(
                                        &take,
                                        g,
                                        step_ms,
                                        &mut job,
                                        &mut wrongs,
                                        &what,
                                    );
                                    grooves[gi].add(&t);
                                    by_timing[ti].add(&t);
                                    by_after[ai].add(&t);
                                    by_tempo[bi].add(&t);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!("smart loop end, the corpus by ear (Auto, the line input):");
    for (g, groove) in GROOVES.iter().zip(&grooves) {
        eprintln!("  {:>26} ({}): {}", g.name, g.meter.name, groove.line());
        total.add(groove);
    }
    for (i, (timing, sd)) in TIMING.iter().enumerate() {
        eprintln!("  {:>26} (±{sd} ms): {}", timing, by_timing[i].line());
    }
    for (i, after) in AFTER.iter().enumerate() {
        eprintln!("  {:>26}: {}", after.words(), by_after[i].line());
    }
    for (i, bpm) in TEMPOS.iter().enumerate() {
        eprintln!("  {:>22} BPM: {}", bpm, by_tempo[i].line());
    }
    eprintln!("  {:>26}: {}", "all", total.line());
    for w in &wrongs {
        eprintln!("  wrong: {w}");
    }
    assert!(
        wrongs.is_empty(),
        "{} wrong moves by ear: {wrongs:#?}",
        wrongs.len()
    );
    let moved = f64::from(total.could_right) / f64::from(total.could.max(1));
    assert!(
        moved >= EAR_MOVES_AT_LEAST,
        "by ear Auto moved only {:.1} % of the taps it could",
        100.0 * moved
    );
}
