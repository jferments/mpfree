//! What the musician is told (the Storage kinds of System status): a
//! watch fed the layer's real states, from a writer on a model stick, the
//! coordinator's admission and the player's counters, reports each change
//! once, and the status board shows what it should.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_status::kinds::{
    CARD_MISSING, CARD_NEARLY_FULL, CARD_REMOVED, NOT_SAVED_YET, RECORDING_AT_RISK,
    RECORDING_MAY_STOP, RECORDING_STOPPED, STICK_FELL_BEHIND, STICK_MISSING, STICK_NEARLY_FULL,
    STREAMING_AT_LIMIT, TAKE_DAMAGED, TAKE_RECOVERED,
};
use mpfree_status::{Board, Kind, Op, Report, Subject, System, Window};
use mpfree_stream::coord::Admission;
use mpfree_stream::player;
use mpfree_stream::rec::{Meta, RecFormat, RecId, Record, TakeState, Why};
use mpfree_stream::recover::Found;
use mpfree_stream::status::{Drive, Watch, NEARLY_FULL_BYTES};
use mpfree_stream::testkit::{played, FakeClock, SharedStick};
use mpfree_stream::writer::{Gate, TakeNo, Writer, WriterConfig};

const STEREO_FLOAT: RecFormat = RecFormat {
    float: true,
    channels: 2,
    rate: 48_000,
};
const STEP: f64 = 0.125;
const TAKE: Subject = Subject(7);

/// A writer on a model stick and a made-up clock, with one take.
struct Bench {
    w: Writer,
    stick: SharedStick,
    clock: Arc<FakeClock>,
    no: TakeNo,
    pushed: u64,
}

impl Bench {
    fn new(cfg: WriterConfig) -> Bench {
        let stick = SharedStick::new();
        let clock = FakeClock::new();
        let mut w = Writer::new(
            Box::new(stick.clone()),
            clock.clone(),
            Arc::new(Gate::new()),
            cfg,
        );
        let no = TakeNo(1);
        w.arm(
            no,
            Path::new("/stick/media"),
            "Take 1",
            RecId([1; 16]),
            STEREO_FLOAT,
            Meta::default(),
        )
        .expect("arms");
        Bench {
            w,
            stick,
            clock,
            no,
            pushed: 0,
        }
    }

    /// Capture up to the clock's time, then the writer's passes.
    fn step(&mut self) {
        self.clock.advance(STEP);
        let want = (self.clock.now_s() * 48_000.0) as u64;
        if want > self.pushed {
            let a: Vec<f32> = (0..(want - self.pushed) as usize * 2)
                .map(|i| played(1, self.pushed + (i / 2) as u64, i % 2))
                .collect();
            self.pushed += self.w.push(self.no, &a) as u64;
        }
        for _ in 0..64 {
            if self.w.tick() > 0.0 {
                return;
            }
        }
        panic!("the writer never waits");
    }
}

/// Reports, gathered.
#[derive(Default)]
struct Got(Vec<Report>);

impl Got {
    fn take(&mut self) -> Vec<(&'static str, Op)> {
        self.0.drain(..).map(|r| (r.kind.id, r.op)).collect()
    }
}

fn began(k: &'static Kind) -> (&'static str, Op) {
    (k.id, Op::Began)
}
fn ended(k: &'static Kind) -> (&'static str, Op) {
    (k.id, Op::Ended)
}
fn once(k: &'static Kind) -> (&'static str, Op) {
    (k.id, Op::Happened { times: 1 })
}

#[test]
fn a_pulled_stick_puts_the_take_at_risk_until_it_is_back() {
    let mut b = Bench::new(WriterConfig::default());
    let mut w = Watch::new();
    let mut got = Got::default();
    let feed = |b: &Bench, w: &mut Watch, got: &mut Got| {
        let v = b.w.view(b.no).expect("there");
        w.take(TAKE, &v, &mut |r| got.0.push(r));
        v.state
    };
    for _ in 0..24 {
        b.step();
        feed(&b, &mut w, &mut got);
    }
    assert!(got.take().is_empty(), "a healthy take says nothing");

    b.stick.with(|s| s.pulled = true);
    let mut states = Vec::new();
    for _ in 0..40 {
        b.step();
        states.push(feed(&b, &mut w, &mut got));
    }
    assert!(states.contains(&TakeState::AtRisk(Why::Gone)));
    let said = got.take();
    assert_eq!(said, vec![began(&RECORDING_AT_RISK)], "once, and only that");
    assert!(
        b.w.view(b.no).unwrap().show_unsaved,
        "unsaved long enough for Not saved yet, which at risk stands for"
    );

    b.stick.with(|s| s.pulled = false);
    for _ in 0..40 {
        b.step();
        feed(&b, &mut w, &mut got);
    }
    assert_eq!(b.w.view(b.no).unwrap().state, TakeState::Recording);
    assert_eq!(got.take(), vec![ended(&RECORDING_AT_RISK)]);

    b.w.stop(b.no);
    for _ in 0..8 {
        b.step();
        feed(&b, &mut w, &mut got);
    }
    assert_eq!(b.w.view(b.no).unwrap().state, TakeState::Saved);
    w.take_gone(TAKE, &mut |r| got.0.push(r));
    assert!(got.take().is_empty(), "nothing was left going");
}

#[test]
fn a_slow_stick_says_not_saved_yet_and_takes_it_back() {
    let mut b = Bench::new(WriterConfig {
        max_interval_s: 2.0,
        ..WriterConfig::default()
    });
    let mut w = Watch::new();
    let mut got = Got::default();
    // Every sync takes 5 s of the made-up clock: saving works, slowly.
    let clock = b.clock.clone();
    b.stick.with(|s| s.sync_cost = Some((clock, 5.0)));
    for _ in 0..80 {
        b.step();
        let v = b.w.view(b.no).unwrap();
        w.take(TAKE, &v, &mut |r| got.0.push(r));
    }
    let said = got.take();
    assert_eq!(said.first(), Some(&began(&NOT_SAVED_YET)), "{said:?}");
    assert!(!said.contains(&began(&RECORDING_AT_RISK)), "not at risk");

    b.stick.with(|s| s.sync_cost = None);
    for _ in 0..80 {
        b.step();
        let v = b.w.view(b.no).unwrap();
        w.take(TAKE, &v, &mut |r| got.0.push(r));
    }
    assert!(!b.w.view(b.no).unwrap().show_unsaved);
    assert!(!w.is_going(&NOT_SAVED_YET, TAKE), "taken back");
}

#[test]
fn full_capture_memory_warns_then_stops_the_take_once() {
    let mut b = Bench::new(WriterConfig {
        limit: 512 << 10,
        ..WriterConfig::default()
    });
    let mut w = Watch::new();
    let mut got = Got::default();
    let mut seen = Vec::new();
    // The stick is out, so capture memory fills: about 1.4 s of audio.
    b.stick.with(|s| s.pulled = true);
    for _ in 0..24 {
        b.step();
        let v = b.w.view(b.no).unwrap();
        w.take(TAKE, &v, &mut |r| got.0.push(r));
        seen.extend(got.take());
    }
    assert_eq!(
        b.w.view(b.no).unwrap().state,
        TakeState::Stopped(Why::Memory)
    );
    let stops = seen
        .iter()
        .filter(|r| **r == once(&RECORDING_STOPPED))
        .count();
    assert_eq!(stops, 1, "stopped is said once: {seen:?}");
    let warned = seen.iter().position(|r| *r == began(&RECORDING_MAY_STOP));
    let stopped = seen.iter().position(|r| *r == once(&RECORDING_STOPPED));
    assert!(
        warned.is_some() && warned < stopped,
        "the warning came first: {seen:?}"
    );
    assert!(
        seen.contains(&ended(&RECORDING_MAY_STOP)),
        "and ended there"
    );
    assert!(!w.is_going(&RECORDING_MAY_STOP, TAKE));
    assert!(
        !w.is_going(&RECORDING_AT_RISK, TAKE),
        "a stopped take isn't at risk (its state says Stopped)"
    );
    w.take_gone(TAKE, &mut |r| got.0.push(r));
    seen = got.take();
    assert!(seen.iter().all(|(_, op)| *op == Op::Ended), "{seen:?}");
}

#[test]
fn streaming_drive_card_and_after_a_cut() {
    let mut w = Watch::new();
    let mut got = Got::default();
    let mut out = |r| got.0.push(r);

    // Admission: only "not even the least safe read-ahead" is said.
    let tight = Admission {
        covered: false,
        safe: true,
        ..Admission::default()
    };
    w.admission(&tight, &mut out);
    w.admission(
        &Admission {
            safe: false,
            ..tight
        },
        &mut out,
    );
    w.admission(
        &Admission {
            safe: false,
            ..tight
        },
        &mut out,
    );
    w.admission(&tight, &mut out);

    // The player: missed frames since last fed; the first feed only sets
    // where counting starts.
    let at = |missed| player::Stats {
        missed,
        ..player::Stats::default()
    };
    w.player(&at(500), &mut out);
    w.player(&at(500), &mut out);
    w.player(&at(756), &mut out);

    // Two sources lost with their card: one report.
    w.card_lost(0, &mut out);
    w.card_lost(2, &mut out);

    // The stick: room under the line, hovering, back; armed takes raise
    // the line; read-only.
    let d = |free, armed| Drive {
        card: false,
        writable: true,
        free,
        armed_bytes_per_s: armed,
    };
    w.drive(&d(NEARLY_FULL_BYTES + 1, 0), &mut out);
    w.drive(&d(NEARLY_FULL_BYTES - 1, 0), &mut out);
    w.drive(&d(NEARLY_FULL_BYTES + 1, 0), &mut out);
    w.drive(&d(NEARLY_FULL_BYTES * 2, 0), &mut out);
    w.drive(&d(NEARLY_FULL_BYTES * 2, 4_000_000), &mut out);
    w.drive(
        &Drive {
            writable: false,
            ..d(0, 0)
        },
        &mut out,
    );

    // After a cut.
    let path = PathBuf::from("/stick/media/Take 1.wav");
    let record = Record {
        id: RecId([1; 16]),
        seq: 3,
        last: false,
        frames: 48_000,
        fmt: STEREO_FLOAT,
        tail_crc: 0,
        whole_crc: 0,
        meta: Meta::default(),
    };
    for f in [
        Found::CutOff {
            stem: "a".into(),
            path: path.clone(),
            record,
            tail: None,
        },
        Found::Unverified {
            stem: "b".into(),
            path: path.clone(),
        },
        Found::Removed { stem: "c".into() },
    ] {
        w.found(&f, &mut out);
    }
    w.checked(TAKE, true, &mut out);
    w.checked(TAKE, false, &mut out);

    assert_eq!(
        got.take(),
        vec![
            began(&STREAMING_AT_LIMIT),
            ended(&STREAMING_AT_LIMIT),
            once(&STICK_FELL_BEHIND),
            once(&CARD_REMOVED),
            began(&STICK_NEARLY_FULL),
            ended(&STICK_NEARLY_FULL),
            began(&STICK_NEARLY_FULL),
            began(&STICK_MISSING),
            ended(&STICK_NEARLY_FULL),
            once(&TAKE_RECOVERED),
            began(&TAKE_DAMAGED),
        ]
    );
}

#[test]
fn the_board_names_the_cause_not_its_echoes() {
    let mut w = Watch::new();
    let mut board = Board::new();
    let t0 = Instant::now();
    // A take at risk and slow, then the stick's partition gone.
    let mut v = mpfree_stream::writer::TakeView {
        state: TakeState::AtRisk(Why::Writes),
        captured: 48_000,
        committed: 0,
        unsaved_s: 10.0,
        show_unsaved: true,
        memory: 0,
        nearly_full: false,
        stats: Default::default(),
        hole: 0,
        carrying: false,
        asks: 0,
    };
    let mut reports = Vec::new();
    w.take(TAKE, &v, &mut |r| reports.push(r));
    w.drive(
        &Drive {
            card: false,
            writable: false,
            free: 0,
            armed_bytes_per_s: 0,
        },
        &mut |r| reports.push(r),
    );
    for r in reports.drain(..) {
        board.apply(r, t0);
    }
    let t = t0 + Duration::from_secs(5);
    board.tick(t);
    let shown: Vec<&str> = board
        .problems(System::Storage, Window::Now)
        .iter()
        .map(|p| p.kind.id)
        .collect();
    assert_eq!(shown, vec![STICK_MISSING.id], "the cause only");

    // The partition is back; the take is still at risk, then saves.
    w.drive(
        &Drive {
            card: false,
            writable: true,
            free: NEARLY_FULL_BYTES * 4,
            armed_bytes_per_s: 0,
        },
        &mut |r| reports.push(r),
    );
    v.state = TakeState::Recording;
    v.show_unsaved = false;
    w.take(TAKE, &v, &mut |r| reports.push(r));
    let ops: Vec<(&str, Op)> = reports.iter().map(|r| (r.kind.id, r.op)).collect();
    assert_eq!(
        ops,
        vec![ended(&STICK_MISSING), ended(&RECORDING_AT_RISK)],
        "Not saved yet never began: at risk stood for it"
    );
}

/// Takes recording on the SD card (S2b, phase B2b): the card's own
/// problems, never the stick's; a move from one drive to the other ends the
/// first one's, and so does the watch stopping.
#[test]
fn the_card_says_its_own_problems_and_a_move_ends_the_other_drives() {
    let mut w = Watch::new();
    let mut got = Vec::new();
    let stick = |writable, free| Drive {
        card: false,
        writable,
        free,
        armed_bytes_per_s: 0,
    };
    let card = |writable, free| Drive {
        card: true,
        ..stick(writable, free)
    };
    // The stick nearly full, then takes move to the card, which is too.
    w.drive(&stick(true, NEARLY_FULL_BYTES - 1), &mut |r| got.push(r));
    w.drive(&card(true, NEARLY_FULL_BYTES - 1), &mut |r| got.push(r));
    // The card taken out.
    w.drive(&card(false, 0), &mut |r| got.push(r));
    // The takes move back to the stick: the card's go.
    w.drive_gone(&mut |r| got.push(r));
    w.drive(&stick(true, NEARLY_FULL_BYTES * 4), &mut |r| got.push(r));
    let ops: Vec<(&str, Op)> = got.iter().map(|r| (r.kind.id, r.op)).collect();
    assert_eq!(
        ops,
        vec![
            began(&STICK_NEARLY_FULL),
            ended(&STICK_NEARLY_FULL),
            began(&CARD_NEARLY_FULL),
            began(&CARD_MISSING),
            ended(&CARD_NEARLY_FULL),
            ended(&CARD_MISSING),
        ]
    );
}
