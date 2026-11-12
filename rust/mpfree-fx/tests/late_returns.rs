//! Returns a block late ([`Mixer::set_pipelined_returns`]) measured against
//! the same mixer on time: how much of the sound changes, case by case.
//!
//! A return goes late only when an insert in it can take the block back
//! (a Cloud reverb, all wet, with at least a block of pre-delay); every
//! other return stays on time. The cases cover both kinds and the moves
//! between them (a knob turned past the limit and back, bypass, a change
//! of block length, the lane turned on and off mid-song). Each prints its
//! measures (`cargo test -p mpfree-fx --test late_returns -- --nocapture`):
//! the largest sample difference and the difference's energy against the
//! returns' own, in dB, and whether the return ran late.

use std::f32::consts::TAU;
use std::time::Duration;

use mpfree_fx::ids::{self, mixer as mx};
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_fx::*;

const RATE: u32 = 44_100;
/// The MPC's period (mpfree-app).
const BLOCK: usize = 128;
const RET: StripId = StripId::Return(0);
const TRACK: StripId = StripId::Track(0);

/// What a case changes while it plays, at a frame.
#[derive(Clone, Copy)]
enum Move {
    /// A parameter of the return's insert `slot`, sample-timed.
    Param(usize, u16, f32),
    /// Bypass (false) or back on, insert `slot`.
    On(usize, bool),
    /// Returns a block late on (inline lane) or off.
    Pipe(bool),
}

struct Case {
    name: &'static str,
    /// The return's inserts, built fresh for each run.
    build: fn(&mut Chain),
    /// Whether the return should run late (at some point).
    late: bool,
    moves: &'static [(usize, Move)],
    /// Block lengths, cycled (the app's is 128 alone).
    blocks: &'static [usize],
}

fn cloud(preset: &str, quality: f32, pre: Option<f32>, mix: f32) -> Box<Cloud> {
    let mut c = Cloud::new();
    let p = CLOUD_PRESETS
        .iter()
        .find(|p| p.name == preset)
        .unwrap_or_else(|| panic!("no preset {preset}"));
    for &(id, v) in p.values {
        c.set_param(id, v);
    }
    if let Some(v) = pre {
        c.set_param(ids::cloud::PRE_DELAY, v);
    }
    c.set_param(ids::cloud::MIX, mix);
    c.set_param(ids::cloud::QUALITY, quality);
    // Settled from the first frame, as a loaded project is.
    c.reset();
    Box::new(c)
}

/// Drums-like bursts, a plucked tone and silences: transients and tails.
fn source(n: usize) -> (Vec<f32>, Vec<f32>) {
    let mut seed = 7u32;
    let mut noise = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 9) as f32 / (1u32 << 23) as f32 - 1.0
    };
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];
    let hit = RATE as usize / 4;
    for (i, (a, b)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
        let t = (i % hit) as f32 / RATE as f32;
        let beat = i / hit;
        let burst = if beat % 4 == 3 {
            0.0
        } else {
            noise() * (-t * 30.0).exp() * 0.5
        };
        let tone = (TAU * 330.0 * i as f32 / RATE as f32).sin() * (-t * 6.0).exp() * 0.3;
        let x = if (i / (RATE as usize)) % 3 == 2 {
            0.0
        } else {
            burst + tone
        };
        *a = x;
        *b = 0.8 * x + 0.1 * burst;
    }
    (l, r)
}

struct Run {
    out: (Vec<f32>, Vec<f32>),
    wet: (Vec<f32>, Vec<f32>),
    stats: MixerStats,
}

fn run(case: &Case, piped: bool, threaded: bool, n: usize) -> Run {
    let mut m = Mixer::new(MixerConfig::default(), RATE, 512);
    m.set_now(TRACK, mx::OUTPUT, mx::output::MASTER);
    m.set_now(TRACK, mx::VOLUME, -10.0);
    m.set_now(TRACK, mx::send(0), -4.0);
    (case.build)(m.chain_mut(RET).unwrap());
    assert!(m.set_capture(0, Some((RET, CapturePoint::AfterFader))));
    let lane = mpfree_lanes::Config {
        threaded,
        ..mpfree_lanes::Config::default()
    };
    let pipe = |m: &mut Mixer, on: bool| {
        if !piped {
            return;
        }
        let cfg = on.then(|| lane.clone());
        m.set_pipelined_returns(cfg, Duration::from_secs(2));
    };
    pipe(&mut m, true);
    let src = source(n);
    let mut out = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut wet = (vec![0.0f32; n], vec![0.0f32; n]);
    let (mut at, mut b, mut next) = (0, 0, 0);
    while at < n {
        let len = case.blocks[b % case.blocks.len()].min(n - at);
        b += 1;
        while next < case.moves.len() && case.moves[next].0 < at + len {
            let (f, mv) = case.moves[next];
            let off = f.saturating_sub(at) as u32;
            match mv {
                Move::Param(slot, id, value) => {
                    let ev = Timed::new(off, NodeEvent::Param { id, value });
                    assert!(m.queue_insert(RET, slot, ev));
                }
                Move::On(slot, on) => m.set_on(RET, slot, on),
                Move::Pipe(on) => pipe(&mut m, on),
            }
            next += 1;
        }
        let (l, r) = m.strip_input(TRACK).unwrap();
        l[..len].copy_from_slice(&src.0[at..at + len]);
        r[..len].copy_from_slice(&src.1[at..at + len]);
        let ctx = Ctx::new(RATE, at as u64);
        m.process(&ctx, &mut out.0[at..at + len], &mut out.1[at..at + len]);
        let (cl, cr) = m.capture(0).unwrap();
        wet.0[at..at + len].copy_from_slice(&cl[..len]);
        wet.1[at..at + len].copy_from_slice(&cr[..len]);
        at += len;
    }
    let stats = m.stats();
    pipe(&mut m, false);
    let after = m.stats();
    assert!(after.pipe_stops >= stats.pipe_stops);
    assert_eq!(
        m.chain(RET).unwrap().lead(),
        0,
        "{}: back on time",
        case.name
    );
    Run { out, wet, stats }
}

fn energy(x: &(Vec<f32>, Vec<f32>)) -> f64 {
    x.0.iter()
        .chain(&x.1)
        .map(|&v| f64::from(v) * f64::from(v))
        .sum()
}
fn db(x: f64) -> f64 {
    10.0 * x.max(1e-30).log10()
}

/// The measures of one case: the largest sample difference (dBFS) and the
/// difference's energy against the returns' (dB), or None when the two
/// are the same bit for bit.
struct Loss {
    max_db: f64,
    vs_wet_db: f64,
    exact: bool,
}

fn measure(case: &Case, threaded: bool) -> (Loss, Run) {
    // About 3 s, whole blocks of the app's period: a shorter last block
    // would let a reverb with a short pre-delay start there.
    let n = 1033 * BLOCK;
    let a = run(case, false, false, n);
    let b = run(case, true, threaded, n);
    let diff: (Vec<f32>, Vec<f32>) = (
        b.out.0.iter().zip(&a.out.0).map(|(x, y)| x - y).collect(),
        b.out.1.iter().zip(&a.out.1).map(|(x, y)| x - y).collect(),
    );
    let max = diff
        .0
        .iter()
        .chain(&diff.1)
        .fold(0.0f32, |m, v| m.max(v.abs()));
    let exact = a
        .out
        .0
        .iter()
        .chain(&a.out.1)
        .zip(b.out.0.iter().chain(&b.out.1))
        .all(|(x, y)| x.to_bits() == y.to_bits());
    assert!(energy(&a.wet) > 1e-3, "{}: the return sounds", case.name);
    let loss = Loss {
        max_db: 20.0 * f64::from(max).max(1e-15).log10(),
        vs_wet_db: db(energy(&diff)) - db(energy(&a.wet)),
        exact,
    };
    let s = &b.stats;
    println!(
        "{:<44} {:>5} {:>9} {:>11} late blocks {:>4}, starts {}, stops {}, lost {}",
        case.name,
        if s.pipe_starts > 0 { "late" } else { "on" },
        if exact {
            "exact".to_string()
        } else {
            format!("{:.1} dB", loss.max_db)
        },
        if exact {
            "-".to_string()
        } else {
            format!("{:.1} dB", loss.vs_wet_db)
        },
        s.pipe_posts,
        s.pipe_starts,
        s.pipe_stops,
        s.pipe_lost_input,
    );
    assert_eq!(s.pipe_late, 0, "{}", case.name);
    assert_eq!(s.pipe_lost_input, 0, "{}", case.name);
    assert_eq!(
        s.pipe_starts > 0,
        case.late,
        "{}: late {} expected {}",
        case.name,
        s.pipe_starts > 0,
        case.late
    );
    if case.late {
        assert!(s.pipe_posts > 0, "{}", case.name);
    }
    (loss, b)
}

const APP: &[usize] = &[BLOCK];

/// Reverbs that can take a block back: late, and the same sound bit for
/// bit, at every quality.
#[test]
fn reverbs_with_a_pre_delay_run_late_with_no_change_at_all() {
    let cases = [
        Case {
            name: "Small room (4 ms), Full",
            build: |c| {
                c.push(cloud("Small room", 0.0, None, 100.0));
            },
            late: true,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Small room (4 ms), Eco",
            build: |c| {
                c.push(cloud("Small room", 2.0, None, 100.0));
            },
            late: true,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Big hall (25 ms), Light",
            build: |c| {
                c.push(cloud("Big hall", 1.0, None, 100.0));
            },
            late: true,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Big hall (25 ms), Eco",
            build: |c| {
                c.push(cloud("Big hall", 2.0, None, 100.0));
            },
            late: true,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Endless cloud (40 ms), Eco",
            build: |c| {
                c.push(cloud("Endless cloud", 2.0, None, 100.0));
            },
            late: true,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Dark plate, pre-delay 3 ms, Eco",
            build: |c| {
                c.push(cloud("Dark plate", 2.0, Some(3.0), 100.0));
            },
            late: true,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "EQ then Big hall, Eco",
            build: |c| {
                c.push(Box::new(Eq::new()));
                c.set_param(0, ids::eq::gain(1), 5.0);
                c.push(cloud("Big hall", 2.0, None, 100.0));
            },
            late: true,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Big hall then compressor, Eco",
            build: |c| {
                c.push(cloud("Big hall", 2.0, None, 100.0));
                c.push(Box::new(Compressor::new()));
                c.set_param(1, ids::compressor::THRESHOLD, -30.0);
            },
            late: true,
            moves: &[],
            blocks: APP,
        },
    ];
    for case in &cases {
        let (loss, _) = measure(case, false);
        assert!(loss.exact, "{}: {:.1} dB", case.name, loss.max_db);
    }
    // On a worker thread, as on the MPC's second core.
    let (loss, _) = measure(&cases[3], true);
    assert!(loss.exact, "on a worker: {:.1} dB", loss.max_db);
}

/// Returns that cannot take a block back stay on time, so they too are
/// unchanged: no pre-delay, a dry mix, effects with no pre-delay, a
/// bypassed reverb.
#[test]
fn returns_that_cannot_take_the_block_back_stay_on_time() {
    let cases = [
        Case {
            name: "Dark plate (no pre-delay)",
            build: |c| {
                c.push(cloud("Dark plate", 2.0, None, 100.0));
            },
            late: false,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Echo chamber (no pre-delay)",
            build: |c| {
                c.push(cloud("Echo chamber", 2.0, None, 100.0));
            },
            late: false,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Big hall, pre-delay 2 ms (under a block)",
            build: |c| {
                c.push(cloud("Big hall", 2.0, Some(2.0), 100.0));
            },
            late: false,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Big hall at 60 % mix (dry passes)",
            build: |c| {
                c.push(cloud("Big hall", 2.0, None, 60.0));
            },
            late: false,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Delay",
            build: |c| {
                c.push(Box::new(Delay::new()));
                c.set_param(0, ids::delay::MIX, 100.0);
            },
            late: false,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Parallel compressor",
            build: |c| {
                c.push(Box::new(Compressor::new()));
                c.set_param(0, ids::compressor::THRESHOLD, -30.0);
                c.set_param(0, ids::compressor::RATIO, 8.0);
            },
            late: false,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Parallel drive",
            build: |c| {
                c.push(Box::new(Drive::new()));
                c.set_param(0, ids::drive::DRIVE, 18.0);
            },
            late: false,
            moves: &[],
            blocks: APP,
        },
        Case {
            name: "Big hall, bypassed (dry passes)",
            build: |c| {
                c.push(cloud("Big hall", 2.0, None, 100.0));
                c.set_on_now(0, false);
            },
            late: false,
            moves: &[],
            blocks: APP,
        },
    ];
    for case in &cases {
        let (loss, _) = measure(case, false);
        assert!(loss.exact, "{}: {:.1} dB", case.name, loss.max_db);
    }
}

/// Moving between late and on time while it plays: each move is seamless
/// when the reverb stays able to take the block back as it happens, and a
/// pre-delay or mix turned past the limit costs at most one block, far
/// under the reverb.
#[test]
fn starting_and_stopping_while_playing_is_seamless() {
    use ids::cloud::{MIX, PRE_DELAY};
    const S: usize = RATE as usize;
    let hall = |c: &mut Chain| {
        c.push(cloud("Big hall", 2.0, None, 100.0));
    };
    let exact = [
        Case {
            name: "Lane turned on and off mid-song",
            build: hall,
            late: true,
            moves: &[
                (0, Move::Pipe(false)),
                (22_050, Move::Pipe(true)),
                (66_150, Move::Pipe(false)),
                (88_200, Move::Pipe(true)),
            ],
            blocks: APP,
        },
        Case {
            name: "Block length changes (128, 64, 256, 96)",
            build: hall,
            late: true,
            moves: &[],
            blocks: &[128, 128, 128, 64, 64, 256, 128, 96, 96, 128],
        },
        Case {
            name: "Odd block lengths at Eco stay on time",
            build: hall,
            late: false,
            moves: &[],
            blocks: &[127],
        },
        Case {
            name: "Bypassed and back",
            build: hall,
            late: true,
            moves: &[(S / 2, Move::On(0, false)), (2 * S, Move::On(0, true))],
            blocks: APP,
        },
    ];
    for case in &exact {
        let (loss, _) = measure(case, false);
        assert!(loss.exact, "{}: {:.1} dB", case.name, loss.max_db);
    }
    // A knob turned past what the reverb can take back: the block in which
    // it moves still runs late with the new value, then it goes back on
    // time; and late again once it is back.
    let knobs = [
        Case {
            name: "Pre-delay 25 -> 1 -> 25 ms",
            build: hall,
            late: true,
            moves: &[
                (S, Move::Param(0, PRE_DELAY, 1.0)),
                (2 * S, Move::Param(0, PRE_DELAY, 25.0)),
            ],
            blocks: APP,
        },
        Case {
            name: "Mix 100 -> 70 -> 100 %",
            build: hall,
            late: true,
            moves: &[
                (S, Move::Param(0, MIX, 70.0)),
                (2 * S, Move::Param(0, MIX, 100.0)),
            ],
            blocks: APP,
        },
    ];
    for case in &knobs {
        let (loss, run) = measure(case, false);
        assert!(
            run.stats.pipe_stops >= 1 && run.stats.pipe_starts >= 2,
            "{}",
            case.name
        );
        assert!(
            loss.exact || loss.vs_wet_db < -50.0,
            "{}: the difference is {:.1} dB under the reverb",
            case.name,
            -loss.vs_wet_db
        );
    }
}
