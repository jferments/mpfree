//! The recording writer's schedule (docs/dev/design/recording-files.md,
//! the second recording step) on a made-up clock and a model stick:
//! rounds on the interval, waiting for playback but never past the
//! longest wait, spread or together, the interval lengthening when rounds
//! fall behind and coming back, the capture reserve, "Unsaved" seconds,
//! capture memory running out; then the recorder on its own thread with
//! real files.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_stream::format;
use mpfree_stream::fsio::{Fs, StdFs};
use mpfree_stream::rec::{
    checkpoint_path, recording_path, stem, Meta, RecFormat, RecId, TakeState, Why,
};
use mpfree_stream::testkit::{folder, played, FakeClock, SharedStick};
use mpfree_stream::writer::{Gate, RealClock, Recorder, Status, TakeNo, Writer, WriterConfig};

const STEREO_FLOAT: RecFormat = RecFormat {
    float: true,
    channels: 2,
    rate: 48_000,
};
const MONO_24: RecFormat = RecFormat {
    float: false,
    channels: 1,
    rate: 44_100,
};
/// For the long runs: little audio.
const MONO_8K: RecFormat = RecFormat {
    float: false,
    channels: 1,
    rate: 8_000,
};
/// A step of the made-up clock that adds up exactly.
const STEP: f64 = 0.125;

fn media() -> PathBuf {
    PathBuf::from("/stick/media")
}

fn id(n: u8) -> RecId {
    RecId([n; 16])
}

/// Frames `from..from+n` of take `m`'s reference audio, interleaved.
fn audio(fmt: RecFormat, m: u32, from: u64, n: u64) -> Vec<f32> {
    let ch = usize::from(fmt.channels);
    (0..n as usize * ch)
        .map(|i| played(m, from + (i / ch) as u64, i % ch))
        .collect()
}

/// A writer on a model stick and a made-up clock.
struct Bench {
    w: Writer,
    stick: SharedStick,
    clock: Arc<FakeClock>,
    gate: Arc<Gate>,
    /// Per take: its number, format, marker and frames pushed.
    takes: Vec<(TakeNo, RecFormat, u32, u64)>,
}

impl Bench {
    fn new(cfg: WriterConfig) -> Bench {
        let stick = SharedStick::new();
        let clock = FakeClock::new();
        let gate = Arc::new(Gate::new());
        let w = Writer::new(
            Box::new(stick.clone()),
            clock.clone(),
            Arc::clone(&gate),
            cfg,
        );
        Bench {
            w,
            stick,
            clock,
            gate,
            takes: Vec::new(),
        }
    }

    fn arm(&mut self, fmt: RecFormat, m: u32) -> TakeNo {
        let no = TakeNo(m);
        self.w
            .arm(
                no,
                &media(),
                &format!("Take {m}"),
                id(m as u8),
                fmt,
                Meta::default(),
            )
            .expect("arms");
        self.takes.push((no, fmt, m, 0));
        no
    }

    /// Capture up to the clock's time, for every take still recording.
    fn capture(&mut self) {
        let now = self.clock.now_s();
        for (no, fmt, m, pushed) in &mut self.takes {
            let want = (now * f64::from(fmt.rate)) as u64;
            if want > *pushed {
                let took = self.w.push(*no, &audio(*fmt, *m, *pushed, want - *pushed));
                *pushed += took as u64;
            }
        }
    }

    /// Passes until the writer wants to wait.
    fn passes(&mut self) {
        for _ in 0..64 {
            if self.w.tick() > 0.0 {
                return;
            }
        }
        panic!("the writer never waits");
    }

    /// Run `seconds` in steps of `step`, capturing and passing; `each`
    /// sees the bench after every step.
    fn run(&mut self, seconds: f64, step: f64, mut each: impl FnMut(&mut Bench)) {
        let end = self.clock.now_s() + seconds - 1e-9;
        while self.clock.now_s() < end {
            self.clock.advance(step);
            self.capture();
            self.passes();
            each(self);
        }
    }

    /// The finished take's audio, checked against what was captured.
    fn check_saved(&mut self, no: TakeNo) {
        let (_, fmt, m, pushed) = *self.takes.iter().find(|t| t.0 == no).expect("ours");
        let v = self.w.view(no).expect("there");
        assert_eq!(v.state, TakeState::Saved);
        assert_eq!(v.captured, pushed);
        let path = self.w.final_path(no).expect("named");
        let bytes = self
            .stick
            .with(|s| s.bytes(&path).expect("finished file").to_vec());
        let l = format::layout(&mut io::Cursor::new(&bytes)).expect("readable WAV");
        assert_eq!(l.frames, pushed);
        let from = l.data_at as usize;
        let mut out = vec![0.0f32; pushed as usize * usize::from(fmt.channels)];
        format::decode(
            l.sample,
            l.endian,
            &bytes[from..from + pushed as usize * l.block_align()],
            &mut out,
        );
        let want = audio(fmt, m, 0, pushed);
        if fmt.float {
            assert!(out == want, "the take's audio, exactly");
        } else {
            let worst = out
                .iter()
                .zip(&want)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            assert!(
                worst < 2.0 / 8_388_608.0,
                "24-bit audio within a step: {worst}"
            );
        }
        let s = stem(&format!("Take {m}"), &id(m as u8));
        assert!(!self
            .stick
            .with(|st| st.exists(&recording_path(&media(), &s))));
        assert!(!self
            .stick
            .with(|st| st.exists(&checkpoint_path(&media(), &s))));
    }
}

#[test]
fn rounds_commit_on_the_interval_and_a_stop_finishes_the_take() {
    let mut b = Bench::new(WriterConfig::default());
    let no = b.arm(STEREO_FLOAT, 1);
    let mut worst_unsaved = 0.0f64;
    let mut shown = false;
    b.run(10.0, STEP, |b| {
        let v = b.w.view(no).unwrap();
        worst_unsaved = worst_unsaved.max(v.unsaved_s);
        shown |= v.show_unsaved;
    });
    let v = b.w.view(no).unwrap();
    assert_eq!(v.state, TakeState::Recording);
    assert_eq!(b.w.stats().rounds, 5, "at 2, 4, 6, 8 and 10 s");
    assert_eq!(v.stats.rounds, 5);
    // An interval, a step and a part-filled unit at most.
    assert!(
        worst_unsaved < 2.0 + STEP + 0.18,
        "{worst_unsaved} s unsaved"
    );
    assert!(!shown, "never past twice the interval");
    assert_eq!(b.w.stats().late, 0);
    assert_eq!(b.w.interval(), 2.0);
    b.w.stop(no);
    b.passes();
    b.check_saved(no);
    assert_eq!(b.w.reserve_bytes(), 0);
    assert!(b.w.forget(no));
    assert!(b.w.takes().is_empty());
}

#[test]
fn an_armed_take_that_never_recorded_is_disarmed_and_its_files_go() {
    let mut b = Bench::new(WriterConfig::default());
    let no = b.arm(MONO_24, 2);
    let s = stem("Take 2", &id(2));
    assert!(b.stick.with(|st| st.exists(&recording_path(&media(), &s))));
    assert!(!b.w.forget(no), "not saved: kept");
    // Its round, with nothing to commit, measures nothing.
    b.clock.advance(2.0);
    b.passes();
    assert_eq!(b.w.stats().rounds, 1);
    assert_eq!(b.w.worst_commit(), 1.0, "still the one assumed");
    assert!(b.w.disarm(no));
    assert!(!b.stick.with(|st| st.exists(&recording_path(&media(), &s))));
    assert!(!b.stick.with(|st| st.exists(&checkpoint_path(&media(), &s))));
    // A number in use is refused.
    let no = b.arm(MONO_24, 3);
    let again =
        b.w.arm(no, &media(), "Other", id(9), MONO_24, Meta::default());
    assert_eq!(again.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
}

#[test]
fn rounds_wait_for_playback_but_never_past_the_longest_wait() {
    let mut b = Bench::new(WriterConfig::default());
    let no = b.arm(STEREO_FLOAT, 1);
    // Playback needs a read within half a second: less than the stick's
    // worst commit time (1 s, assumed until measured).
    b.gate.set_cushion(0.5);
    let mut shown_at = None;
    b.run(12.0 - STEP, STEP, |b| {
        let v = b.w.view(no).unwrap();
        if v.show_unsaved && shown_at.is_none() {
            shown_at = Some(b.clock.now_s());
        }
    });
    let v = b.w.view(no).unwrap();
    assert_eq!(v.committed, 0, "the round due at 2 s waits");
    assert_eq!(b.w.stats().waited, 1, "one round, counted once");
    assert_eq!(b.w.stats().forced, 0);
    let at = shown_at.expect("\"Unsaved\" shown");
    assert_eq!(at, 4.0 + STEP, "shown once past twice the interval");
    // 10 s past its time it goes anyway, and the interval lengthens (it
    // fell behind).
    b.run(STEP, STEP, |_| {});
    let v = b.w.view(no).unwrap();
    assert!(v.committed > 0);
    assert_eq!(b.w.stats().forced, 1);
    assert_eq!(b.w.interval(), 4.0);
    assert_eq!(b.w.stats().lengthened, 1);
    // Measured now (this stick commits at once): half a second of cushion
    // is plenty, and rounds go on time.
    b.run(9.0, STEP, |_| {});
    assert_eq!(b.w.stats().waited, 1);
    assert_eq!(b.w.stats().rounds, 3);
    assert!(!b.w.view(no).unwrap().show_unsaved);
    b.w.stop(no);
    b.passes();
    b.check_saved(no);
}

/// When each take's first commit happened.
fn first_commits(cfg: WriterConfig) -> Vec<f64> {
    let mut b = Bench::new(cfg);
    let nos: Vec<TakeNo> = (1..=4).map(|m| b.arm(STEREO_FLOAT, m)).collect();
    let mut at = vec![None; 4];
    b.run(2.5, 1.0 / 128.0, |b| {
        for (k, no) in nos.iter().enumerate() {
            if at[k].is_none() && b.w.view(*no).unwrap().committed > 0 {
                at[k] = Some(b.clock.now_s());
            }
        }
    });
    for no in &nos {
        b.w.stop(*no);
    }
    b.passes();
    for no in nos {
        b.check_saved(no);
    }
    at.into_iter().map(|t| t.expect("committed")).collect()
}

#[test]
fn several_takes_rounds_are_spread_across_the_interval_or_run_together() {
    let mut spread = first_commits(WriterConfig::default());
    spread.sort_by(f64::total_cmp);
    for (k, t) in spread.iter().enumerate() {
        let want = 0.5 * (k + 1) as f64;
        assert_eq!(*t, want, "round {k}: {spread:?}");
    }
    let together = first_commits(WriterConfig {
        spread: false,
        ..WriterConfig::default()
    });
    assert!(together.iter().all(|&t| t == 2.0), "{together:?}");
}

#[test]
fn slow_rounds_lengthen_the_interval_and_quick_ones_bring_it_back() {
    let mut b = Bench::new(WriterConfig::default());
    let no = b.arm(MONO_8K, 1);
    // Each sync takes 2.5 s: a round (two syncs) takes 5.
    b.stick.with(|s| s.sync_cost = Some((b.clock.clone(), 2.5)));
    b.run(30.0, 0.5, |_| {});
    assert_eq!(b.w.interval(), 8.0, "2 to 4 to 8, then rounds fit");
    assert_eq!(b.w.stats().lengthened, 2);
    assert!((b.w.worst_commit() - 5.0).abs() < 1e-9);
    // The stick is quick again: 16 quick rounds halve it, 16 more again.
    b.stick.with(|s| s.sync_cost = None);
    // (The first quick round may be up to 8 s off.)
    b.run(8.0 + 16.0 * 8.0 + 16.0 * 4.0 + 4.0, 0.5, |_| {});
    assert_eq!(b.w.interval(), 2.0);
    assert_eq!(b.w.stats().shortened, 2);
    // Never past the longest interval.
    b.stick
        .with(|s| s.sync_cost = Some((b.clock.clone(), 20.0)));
    b.run(200.0, 0.5, |_| {});
    assert_eq!(b.w.interval(), 10.0);
    b.stick.with(|s| s.sync_cost = None);
    b.w.stop(no);
    b.passes();
    b.check_saved(no);
}

#[test]
fn the_capture_reserve_follows_the_worst_commit_time() {
    let mut b = Bench::new(WriterConfig::default());
    let no = b.arm(STEREO_FLOAT, 1);
    // 384,000 bytes a second x (1 s assumed + 2 s interval).
    assert_eq!(b.w.reserve_bytes(), 1_152_000);
    assert_eq!(b.w.reserve_units(), 18);
    // Rounds measured at 1.5 s (on time: the interval stays 2).
    b.stick
        .with(|s| s.sync_cost = Some((b.clock.clone(), 0.75)));
    b.run(6.0, STEP, |_| {});
    assert_eq!(b.w.interval(), 2.0);
    assert_eq!(b.w.reserve_bytes(), 1_344_000);
    assert_eq!(b.w.reserve_units(), 21);
    // A second take adds its own.
    b.arm(MONO_24, 2);
    assert_eq!(b.w.reserve_bytes(), (384_000 + 132_300) * 7 / 2);
    // Stopped takes need none.
    b.stick.with(|s| s.sync_cost = None);
    b.w.stop(no);
    b.w.stop(TakeNo(2));
    b.passes();
    assert_eq!(b.w.reserve_bytes(), 0);
    b.check_saved(no);
    b.check_saved(TakeNo(2));
}

#[test]
fn when_capture_memory_runs_out_recording_stops_and_the_take_is_finished() {
    let mut b = Bench::new(WriterConfig {
        limit: 512 << 10,
        ..WriterConfig::default()
    });
    let no = b.arm(STEREO_FLOAT, 1);
    // Capture memory holds 1.4 s of audio, less than an interval: it
    // fills before the first round.
    b.clock.advance(1.0);
    b.capture();
    b.passes();
    b.clock.advance(1.0);
    b.capture();
    let v = b.w.view(no).unwrap();
    assert_eq!(
        v.state,
        TakeState::Stopped(Why::Memory),
        "stopped by itself"
    );
    assert_eq!(v.captured, (512 << 10) / 8);
    assert!(v.nearly_full);
    // It is finished with what fitted.
    b.passes();
    b.check_saved(no);
    assert_eq!(b.w.stats().waited, 0);
}

#[test]
fn a_round_stops_waiting_for_playback_once_capture_memory_is_half_full() {
    let mut b = Bench::new(WriterConfig {
        limit: 2 << 20,
        ..WriterConfig::default()
    });
    let no = b.arm(STEREO_FLOAT, 1);
    b.passes();
    assert!(
        !b.gate.recording(),
        "armed with nothing captured (only monitoring): nothing waits"
    );
    b.gate.set_cushion(0.0);
    b.run(2.0, STEP, |_| {});
    assert!(b.gate.recording(), "recording: background work waits");
    let shared = b.gate.capture_fill();
    assert_eq!(
        shared,
        768_000.0 / f64::from(2 << 20),
        "shared with the coordinator"
    );
    let mut went = None;
    b.run(2.0, STEP, |b| {
        if went.is_none() && b.w.view(no).unwrap().committed > 0 {
            went = Some(b.clock.now_s());
        }
    });
    // 2 MiB at 384,000 bytes a second is half full at 2.73 s.
    assert_eq!(went, Some(2.75), "recording moved up");
    assert_eq!(b.w.stats().waited, 1);
    assert_eq!(b.w.stats().forced, 0);
    assert!(b.gate.capture_fill() < 0.5);
    b.w.stop(no);
    b.gate.set_cushion(f64::INFINITY);
    b.passes();
    b.check_saved(no);
    assert_eq!(b.gate.capture_fill(), 0.0, "nothing recording");
    assert!(!b.gate.recording(), "saved: background work goes on");
}

// ---- on its own thread ------------------------------------------------------

fn wait_for(r: &Recorder, no: TakeNo, what: &str, ok: impl Fn(&Status) -> bool) -> Status {
    let t = Instant::now();
    loop {
        if let Some(s) = r.status(no) {
            if ok(&s) {
                return s;
            }
        }
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "{what}: {:?}",
            r.status(no)
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn read_file(path: &Path, fmt: RecFormat) -> Vec<f32> {
    let bytes = std::fs::read(path).expect("finished file");
    let l = format::layout(&mut io::Cursor::new(&bytes)).expect("readable WAV");
    let mut out = vec![0.0f32; l.frames as usize * usize::from(fmt.channels)];
    let from = l.data_at as usize;
    format::decode(
        l.sample,
        l.endian,
        &bytes[from..from + l.frames as usize * l.block_align()],
        &mut out,
    );
    out
}

fn saved(s: &Status) -> bool {
    matches!(s, Status::Take(v) if v.state == TakeState::Saved)
}

#[test]
fn the_recorder_records_on_its_own_thread() {
    let dir = folder("writer-thread");
    let cfg = WriterConfig {
        interval_s: 0.05,
        commit_s: 0.01,
        ..WriterConfig::default()
    };
    let gate = Arc::new(Gate::new());
    let mut r = Recorder::start(
        Box::new(StdFs::default()),
        Arc::new(RealClock::new()),
        gate,
        cfg,
    )
    .expect("starts");
    let no = r.arm(
        &dir,
        "Take 1",
        RecId::random(),
        STEREO_FLOAT,
        Meta::default(),
    );
    wait_for(&r, no, "armed", |s| matches!(s, Status::Take(_)));
    let mut sent = 0u64;
    for _ in 0..20 {
        let a = audio(STEREO_FLOAT, 1, sent, 2_400);
        assert_eq!(r.push(no, &a), 2_400);
        sent += 2_400;
        std::thread::sleep(Duration::from_millis(5));
    }
    wait_for(
        &r,
        no,
        "a round committed",
        |s| matches!(s, Status::Take(v) if v.committed > 0),
    );
    r.stop(no);
    assert_eq!(
        r.push(no, &audio(STEREO_FLOAT, 1, sent, 10)),
        0,
        "stopped: no more"
    );
    wait_for(&r, no, "saved", saved);
    let path = r.final_path(no).expect("named");
    assert!(read_file(&path, STEREO_FLOAT) == audio(STEREO_FLOAT, 1, 0, sent));
    assert!(r.stats().rounds > 0);
    assert_eq!(r.reserve_units(), 0);
    // A take that can't be armed says why.
    let bad = r.arm(
        &dir.join("no such folder"),
        "Take 2",
        RecId::random(),
        STEREO_FLOAT,
        Meta::default(),
    );
    assert!(matches!(
        wait_for(&r, bad, "refused", |s| *s != Status::Arming),
        Status::Failed(_)
    ));
    r.forget(no);
    r.forget(bad);
    assert_eq!(r.status(no), None);
    r.close();
}

#[test]
fn the_recorder_counts_capture_memory_while_the_stick_is_busy() {
    let dir = folder("writer-memory");
    let cfg = WriterConfig {
        interval_s: 0.05,
        commit_s: 0.01,
        max_wait_s: 0.3,
        limit: 256 << 10,
        ..WriterConfig::default()
    };
    // Playback leaves no room; capture memory past half moves up.
    let gate = Arc::new(Gate::new());
    gate.set_cushion(0.0);
    let mut r = Recorder::start(
        Box::new(StdFs::default()),
        Arc::new(RealClock::new()),
        Arc::clone(&gate),
        cfg,
    )
    .expect("starts");
    let no = r.arm(
        &dir,
        "Take 1",
        RecId::random(),
        STEREO_FLOAT,
        Meta::default(),
    );
    wait_for(&r, no, "armed", |s| matches!(s, Status::Take(_)));
    let a = audio(STEREO_FLOAT, 1, 0, 48_000);
    let took = r.push(no, &a) as u64;
    assert_eq!(
        took,
        (256 << 10) / 8,
        "up to the limit, counted on this side"
    );
    assert_eq!(r.push(no, &a[..20]), 0, "full: recording stopped");
    wait_for(&r, no, "saved", saved);
    let path = r.final_path(no).expect("named");
    assert!(read_file(&path, STEREO_FLOAT) == audio(STEREO_FLOAT, 1, 0, took));
    r.close();
}

#[test]
fn the_coordinator_shares_its_cushion_through_the_gate() {
    use mpfree_stream::coord::Plan;
    use mpfree_stream::player::Config;
    use mpfree_stream::testkit::Rig;
    let mut rig = Rig::new(Config::default(), Plan::default());
    let gate = rig.c.gate();
    rig.c.step();
    assert_eq!(gate.cushion(), f64::INFINITY, "nothing plays");
}
