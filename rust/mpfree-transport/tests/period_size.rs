//! The transport must describe the same session the same way whatever the
//! audio period size: every tick on the same absolute frame, every marker
//! on the same frame in the same order, the same spans once joined across
//! periods. A busy scripted session (count-in, tempo segments inside a
//! loop, live tempo changes, punch in and out, locates, stop, panic,
//! commands on the exact frame of a loop jump and a tempo change) is cut
//! into periods of 1 to 8192 frames, and into random sizes, and compared.
//! So is a session through two tempo ramps, with a locate into the middle
//! of one.

use mpfree_transport::time::{Segment, Swing, TempoMap};
use mpfree_transport::transport::MAX_PERIOD;
use mpfree_transport::{Block, Command, Mark, Span, Tick, Transport};

const RATE: u32 = 44_100;
const LENGTH: u64 = 700_000;

/// A span without its period-relative fields, so spans can be joined
/// across periods.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Run {
    frame: u64,
    len: u64,
    span: Span,
}

#[derive(Debug, Default, PartialEq)]
struct Log {
    ticks: Vec<(Tick, u64)>,
    marks: Vec<(u64, Mark)>,
    runs: Vec<Run>,
}

fn script(extra: &[(u64, Command)]) -> Vec<(u64, Command)> {
    let mut s = vec![
        (0, Command::SetCountIn(1)),
        (0, Command::SetLoop(Some((3840, 7680)))),
        (0, Command::SetSwing(Swing::new(240, 58).unwrap())),
        // Count in from frame 1000, record from tick 0 at frame 89 200,
        // 120 BPM to tick 1920, then the loop 3840 to 7680 at 90 BPM with a
        // segment to 140 BPM at tick 5760 in the middle of it.
        (1_000, Command::Record),
        (400_000, Command::SetTempo(133.0)),
        (420_000, Command::Record),
        (430_000, Command::ToggleOverdub),
        (450_000, Command::Locate(1_000)),
        (480_000, Command::SetLoop(None)),
        (500_000, Command::SetTempo(77.7)),
        (500_001, Command::Locate(-100)),
        (540_000, Command::Stop),
        (550_000, Command::Stop),
        (560_000, Command::Play),
        (580_000, Command::PlayStart),
        // The shortest loop: one 1/16.
        (600_000, Command::SetLoop(Some((0, 240)))),
        (620_000, Command::Record),
        (640_000, Command::StopAndRewind),
        (650_000, Command::SetCountIn(0)),
        (650_000, Command::Record),
        (690_000, Command::Stop),
    ];
    s.extend_from_slice(extra);
    s
}

fn run(script: &[(u64, Command)], next_size: impl FnMut() -> u32) -> Log {
    let map = TempoMap::new(&[
        Segment {
            start: 0,
            bpm: 120.0,
        },
        Segment {
            start: 1920,
            bpm: 90.0,
        },
        Segment {
            start: 5760,
            bpm: 140.0,
        },
    ])
    .unwrap();
    run_with(map, script, next_size)
}

fn run_with(map: TempoMap, script: &[(u64, Command)], mut next_size: impl FnMut() -> u32) -> Log {
    let mut t = Transport::new(RATE, 120.0).unwrap();
    t.set_tempo_map(map).unwrap();
    for &(at, cmd) in script {
        assert!(t.command(at, cmd));
    }
    let mut b = Block::new();
    let mut log = Log::default();
    while t.frame() < LENGTH {
        let n = next_size().min((LENGTH - t.frame()) as u32);
        assert!(t.process(n, &mut b));
        check_block(&b);
        for s in b.spans() {
            for k in s.ticks() {
                let f = s.frame_of(k as f64);
                assert!(
                    f >= s.frame && f < s.frame + u64::from(s.len),
                    "tick {k} outside its span"
                );
                assert_eq!(s.offset_of(k as f64), Some(s.offset + (f - s.frame) as u32));
                log.ticks.push((k, f));
            }
            let run = Run {
                frame: s.frame,
                len: u64::from(s.len),
                span: Span {
                    offset: 0,
                    len: 0,
                    frame: 0,
                    ..*s
                },
            };
            match log.runs.last_mut() {
                Some(last) if last.span == run.span && last.frame + last.len == run.frame => {
                    last.len += run.len
                }
                _ => log.runs.push(run),
            }
        }
        log.marks
            .extend(b.markers().iter().map(|m| (m.frame, m.mark)));
    }
    let stats = t.stats();
    assert_eq!(
        (
            stats.late,
            stats.queue_full,
            stats.rejected,
            stats.deferred,
            stats.block_full
        ),
        (0, 0, 0, 0, 0)
    );
    log
}

/// Spans cover the period once, in order; markers are in order inside it.
fn check_block(b: &Block) {
    let mut at = 0;
    for s in b.spans() {
        assert_eq!(s.offset, at);
        assert_eq!(s.frame, b.start_frame + u64::from(s.offset));
        assert!(s.len > 0);
        at += s.len;
    }
    assert_eq!(at, b.frames);
    let mut last = 0;
    for m in b.markers() {
        assert!(m.offset < b.frames && m.offset >= last);
        assert_eq!(m.frame, b.start_frame + u64::from(m.offset));
        last = m.offset;
    }
}

fn fixed(n: u32) -> impl FnMut() -> u32 {
    move || n
}

/// A small deterministic generator, so a failure always repeats.
fn random(seed: u64, max: u32) -> impl FnMut() -> u32 {
    let mut x = seed;
    move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % u64::from(max)) as u32 + 1
    }
}

fn frames_of(log: &Log, pick: impl Fn(&Mark) -> bool) -> Vec<u64> {
    log.marks
        .iter()
        .filter(|m| pick(&m.1))
        .map(|m| m.0)
        .collect()
}

#[test]
fn the_same_session_for_every_period_size() {
    let reference = run(&script(&[]), fixed(128));

    // The session does what the script says.
    let loops = frames_of(&reference, |m| matches!(m, Mark::Looped { .. }));
    let tempos = frames_of(&reference, |m| matches!(m, Mark::Tempo(_)));
    assert!(loops.len() >= 4, "loops: {loops:?}");
    assert_eq!(
        &tempos[..4],
        &[133_300, 250_900, 288_700, 347_500],
        "tempo changes: {tempos:?}"
    );
    assert!(loops.len() >= 6, "loops: {loops:?}");
    assert_eq!(reference.marks[3], (1_000, Mark::CountIn { record_at: 0 }));
    assert_eq!(reference.marks[4], (89_200, Mark::Started { tick: 0 }));
    assert!(reference.marks.iter().any(|m| m.1 == Mark::Panic));
    assert!(reference.marks.iter().any(|m| m.1 == Mark::Tempo(133.0)));
    assert!(reference
        .marks
        .iter()
        .any(|m| matches!(m.1, Mark::Located { to: -100, .. })));

    // Commands on the very frame of a tempo segment and of loop jumps. The
    // locate moves every later jump, so the jump frames come from a run
    // that already has it; the overdub toggle does not move anything.
    let segment = tempos[1];
    let locate = [(segment, Command::Locate(4_000))];
    let loops = frames_of(&run(&script(&locate), fixed(128)), |m| {
        matches!(m, Mark::Looped { .. })
    });
    let extra = [
        locate[0],
        (loops[0], Command::ToggleOverdub),
        (loops[2], Command::SetTempo(101.0)),
    ];
    let script = script(&extra);
    let reference = run(&script, fixed(128));
    let has = |f: u64, m: Mark| reference.marks.contains(&(f, m));
    assert!(
        has(
            segment,
            Mark::Located {
                from: 5760,
                to: 4_000
            }
        ),
        "locate on the segment frame"
    );
    assert!(
        has(
            loops[0],
            Mark::Looped {
                end: 7680,
                start: 3840
            }
        ) && has(loops[0], Mark::Overdub(true))
    );
    assert!(
        has(loops[2], Mark::Looped { end: 240, start: 0 }) && has(loops[2], Mark::Tempo(101.0))
    );

    for n in [1, 2, 7, 64, 127, 333, 1024, 4096, MAX_PERIOD] {
        assert!(
            run(&script, fixed(n)) == reference,
            "period size {n} differs"
        );
    }
    for seed in [1u64, 0x9e37_79b9, 12_345_678] {
        assert!(
            run(&script, random(seed, 2048)) == reference,
            "random sizes (seed {seed}) differ"
        );
    }
}

#[test]
fn each_tick_plays_once_between_jumps() {
    let log = run(&script(&[]), fixed(256));
    let jumps: Vec<u64> = frames_of(&log, |m| {
        matches!(
            m,
            Mark::Looped { .. } | Mark::Located { .. } | Mark::Started { .. }
        )
    });
    for w in log.ticks.windows(2) {
        let ((t1, f1), (t2, f2)) = (w[0], w[1]);
        assert!(f2 > f1, "frames go forward: {t1}@{f1} then {t2}@{f2}");
        if t2 != t1 + 1 {
            assert!(
                jumps.iter().any(|&j| j > f1 && j <= f2),
                "tick {t1}@{f1} then {t2}@{f2} without a jump"
            );
        }
    }
}

/// Two ramps: 90 to 160 BPM over four bars (64 steps), then 160 to 70 over
/// 1000 ticks (four steps, the last one 280 ticks), after two bars at 120.
fn ramp_map() -> TempoMap {
    let s = |start, bpm| Segment { start, bpm };
    TempoMap::new(&[
        s(0, 120.0),
        s(3840, 90.0),
        s(19_200, 160.0),
        s(20_200, 70.0),
    ])
    .unwrap()
    .ramp(1)
    .unwrap()
    .ramp(2)
    .unwrap()
}

/// The frame each tick first plays on, up to `before`.
fn first_frames(log: &Log, before: u64) -> std::collections::BTreeMap<Tick, u64> {
    let mut at = std::collections::BTreeMap::new();
    for &(k, f) in &log.ticks {
        if f < before {
            at.entry(k).or_insert(f);
        }
    }
    at
}

#[test]
fn a_tempo_ramp_plays_the_maps_staircase_for_every_period_size() {
    let map = ramp_map();
    let locate_at = 600_000;
    let script = [(1_000, Command::Play), (locate_at, Command::Locate(10_001))];
    let reference = run_with(map, &script, fixed(128));

    // One tempo change per step, rising through the first ramp and falling
    // through the second.
    let tempos: Vec<f64> = reference
        .marks
        .iter()
        .filter(|m| m.0 < locate_at)
        .filter_map(|m| match m.1 {
            Mark::Tempo(b) => Some(b),
            _ => None,
        })
        .collect();
    assert_eq!(tempos.len(), 64 + 4 + 1, "{tempos:?}");
    assert!(tempos[0] > 90.0 && tempos[0] < 91.0, "{tempos:?}");
    assert!(tempos[..64].windows(2).all(|w| w[1] > w[0]));
    assert!(tempos[63] > 159.0 && tempos[63] < 160.0, "{tempos:?}");
    assert!(tempos[63..].windows(2).all(|w| w[1] < w[0]));
    assert_eq!(tempos[68], 70.0);

    // Every tick sits where the map says, counted from the first ramp's
    // start: each 1/16 line on exactly that frame, ticks between lines
    // within the rounding of a frame.
    let frames = first_frames(&reference, locate_at);
    let f0 = frames[&3840];
    assert_eq!(f0, 1_000 + 88_200, "one bar at 120 BPM");
    for (&k, &f) in frames.range(3840..=21_000) {
        let want = f0 as f64 + map.frames_between(3840.0, k as f64, RATE);
        let line =
            (k <= 19_200 && (k - 3840) % 240 == 0) || [19_440, 19_680, 19_920, 20_200].contains(&k);
        if line {
            assert_eq!(f as f64, want, "line {k}");
        } else {
            assert!((f as f64 - want).abs() <= 1.0, "tick {k}: {f} vs {want}");
        }
    }

    // A locate into the middle of a step: the next line falls on the whole
    // frame nearest the map's, and from there every line keeps the map's
    // spacing exactly.
    let after: Vec<(Tick, u64)> = reference
        .ticks
        .iter()
        .copied()
        .filter(|&(_, f)| f >= locate_at)
        .collect();
    assert_eq!(after[0], (10_001, locate_at));
    let lines: Vec<(Tick, u64)> = after
        .iter()
        .copied()
        .filter(|&(k, _)| (k - 3840) % 240 == 0)
        .collect();
    assert!(lines.len() > 10);
    let first = locate_at as f64 + map.frames_between(10_001.0, lines[0].0 as f64, RATE);
    assert_eq!(lines[0].1 as f64, first.round());
    for w in lines.windows(2) {
        let gap = map.frames_between(w[0].0 as f64, w[1].0 as f64, RATE);
        assert_eq!(
            (w[1].1 - w[0].1) as f64,
            gap,
            "lines {} to {}",
            w[0].0,
            w[1].0
        );
    }

    for n in [1, 2, 7, 64, 127, 333, 1024, 4096, MAX_PERIOD] {
        assert!(
            run_with(map, &script, fixed(n)) == reference,
            "period size {n} differs"
        );
    }
    for seed in [1u64, 0x9e37_79b9, 12_345_678] {
        assert!(
            run_with(map, &script, random(seed, 2048)) == reference,
            "random sizes (seed {seed}) differ"
        );
    }
}
