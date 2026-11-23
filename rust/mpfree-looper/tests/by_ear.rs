//! Smart loop end by ear (the smart loop end plan, step 3): a free first
//! loop recorded from the line input or a mix, its hits heard by the hit
//! track, its start and end found by the finder on the audio itself.
//!
//! The finder answers between periods here (the app runs it on a thread
//! of its own and hands the answer to the next period), or late, or for a
//! take that has gone. 22.05 kHz in 4/4; the song is at 100 BPM before.
//! The synthetic beat is 120 BPM (a beat is 11 025 frames, two bars
//! 88 200), from the hit track's own kit.

use mpfree_looper::find::{find, FindJob};
use mpfree_looper::hits::kit::Kit;
use mpfree_looper::{
    ends, Command as L, Looper, Phase, Shift, SmartEnd, SmartOutcome, Verdict, FADE,
    FIND_WAIT_AUTO_MS,
};
use mpfree_transport::time::PPQ;
use mpfree_transport::{Block, Command as T, State, Transport};

const RATE: u32 = 22_050;
const BAR_TICKS: i64 = 4 * PPQ as i64;
const TWO_BARS: u64 = 88_200;
/// The reach at 120 BPM: 100 ms.
const REACH: u64 = 2_205;

/// How the finder answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answers {
    /// Before the next period.
    AtOnce,
    /// Never: the looper waits until it may wait no longer.
    Never,
    /// It keeps the job until the test answers ([`Rig::answer_now`]).
    Holds,
}

struct Rig {
    t: Transport,
    l: Looper,
    b: Block,
    period: usize,
    frame: u64,
    /// The line input: what arrives on each frame.
    audio: Vec<f32>,
    input: Vec<f32>,
    mix: Vec<f32>,
    out: Vec<f32>,
    tempo: Option<(f64, u64)>,
    into: i64,
    job: FindJob,
    answers: Answers,
    held: bool,
}

impl Rig {
    /// Stopped at 100 BPM, tempo from the loop on, Free length, smart loop
    /// end `mode` by ear, the input `latency` frames behind.
    fn new(period: usize, mode: SmartEnd, latency: u32) -> Rig {
        let mut r = Rig {
            t: Transport::new(RATE, 100.0).unwrap(),
            l: Looper::new(RATE, 30),
            b: Block::new(),
            period,
            frame: 0,
            audio: vec![0.0; 2 * 8 * RATE as usize],
            input: vec![0.0; 2 * period],
            mix: vec![0.0; 2 * period],
            out: Vec::new(),
            tempo: None,
            into: 0,
            job: FindJob::new(ends::reach(RATE, f64::NAN)),
            answers: Answers::AtOnce,
            held: false,
        };
        for c in [
            L::TempoFromLoop(true),
            L::SetBars(None),
            L::SetSmartEnd(mode),
            L::SmartByEar(true),
            L::SetLatency(latency),
        ] {
            assert!(r.l.command(c));
        }
        r
    }

    /// Two bars of the kit's beat at 120 BPM, and the next downbeat,
    /// arriving from frame `from`.
    fn play_beat(&mut self, from: u64) {
        Kit::new(RATE).beat(&mut self.audio, from as usize, 120.0, 2, &|_| 0.0);
    }

    fn run(&mut self, to: u64) {
        while self.frame < to {
            let n = (self.period as u64).min(to - self.frame) as usize;
            let f = self.frame as usize;
            self.input[..2 * n].copy_from_slice(&self.audio[2 * f..2 * (f + n)]);
            self.mix[..2 * n].fill(0.0);
            assert!(self.t.process(n as u32, &mut self.b));
            self.l
                .process(&self.b, &self.input[..2 * n], &mut self.mix[..2 * n]);
            self.out.extend_from_slice(&self.mix[..2 * n]);
            self.frame += n as u64;
            if !self.held && self.l.find_job(&mut self.job) {
                match self.answers {
                    Answers::AtOnce => {
                        find(&mut self.job);
                        self.l.found(&self.job);
                    }
                    Answers::Never => {}
                    Answers::Holds => self.held = true,
                }
            }
            if let Some((_, reading, at, into)) = self.l.take_tempo() {
                assert_eq!(at, self.frame);
                assert_eq!(self.t.state(), State::Stopped);
                let pos = self.t.position();
                let start = pos - pos.rem_euclid(self.t.meter().bar_ticks()) + into;
                if start != pos {
                    assert!(self.t.command(at, T::Locate(start)));
                }
                assert!(self.t.command(at, T::SetTempo(reading.bpm)));
                assert!(self.t.command(at, T::Play));
                self.tempo = Some((reading.bpm, at));
                self.into = into;
            }
        }
    }

    /// The held job answered now.
    fn answer_now(&mut self) {
        assert!(self.held);
        find(&mut self.job);
        self.l.found(&self.job);
        self.held = false;
    }

    fn until_playing(&mut self, most: u64) {
        let to = self.frame + most;
        while !matches!(self.l.status().phase, Phase::Playing | Phase::Overdubbing) {
            assert!(self.frame < to, "still {:?}", self.l.status());
            let f = self.frame + self.period as u64;
            self.run(f);
        }
    }

    /// Record at frame 10 000, the beat arriving `early` frames later,
    /// and Record again in the period that ends `late` frames after its
    /// last downbeat arrived (the tap: the frame it closes on), whatever
    /// the period. The frame the pass started on, and the tap.
    fn late_tap(&mut self, early: u64, late: u64) -> (u64, u64) {
        self.run(10_000);
        assert!(self.l.command(L::Record));
        let rec_from = self.frame;
        self.play_beat(rec_from + early);
        let tap = rec_from + early + TWO_BARS + late;
        self.run(tap - self.period as u64);
        assert!(self.l.command(L::Record));
        self.run(tap);
        (rec_from, tap)
    }
}

/// Auto by ear: the loop closes on the downbeat heard, two bars at
/// exactly 120 from the first kick (a smart start: Record came 80 ms
/// before it), and the song starts as far in as the loop came round.
#[test]
fn auto_by_ear_closes_on_the_downbeat_heard_and_starts_on_the_first_hit() {
    let early = 1_764; // 80 ms
    let mut r = Rig::new(256, SmartEnd::Auto, 0);
    let (rec_from, tap) = r.late_tap(early, 1_323);
    assert!(r.l.status().closing);
    r.until_playing(REACH + 2_048);
    let (bpm, at) = r.tempo.expect("a tempo");
    let s = r.l.status();
    assert_eq!((s.frames as u64, s.ticks), (TWO_BARS, 2 * BAR_TICKS));
    assert_eq!(bpm, 120.0);
    let hit = rec_from + early;
    let by = (hit + TWO_BARS) as i64 - tap as i64;
    let note = r.l.take_smart().expect("a word on the tap");
    assert_eq!(
        (note.outcome, note.start),
        (SmartOutcome::Moved(by), early as i64)
    );
    assert_eq!(
        s.moved,
        Some(Shift {
            start: early as i64,
            end: by
        })
    );
    let st = r.l.stats();
    assert_eq!(
        (st.finds_asked, st.finds_answered, st.finds_late),
        (1, 1, 0)
    );
    assert!(st.hits_heard >= 17, "{st:?}");
    // It plays from the kick, the song as far in as it came round.
    let into = (at - (hit + TWO_BARS)) as f64 * (2 * BAR_TICKS) as f64 / TWO_BARS as f64;
    assert_eq!(r.into, into.round() as i64);
    r.run(at + 20_000);
    let from = hit + (r.into as f64 * TWO_BARS as f64 / (2 * BAR_TICKS) as f64).round() as u64;
    // (Within the loop point's seam the two kicks, near enough the same,
    // are crossfaded.)
    for j in [FADE as u64 + 10, 5_000, 15_000] {
        let want = r.audio[2 * (from + j) as usize];
        let got = r.out[2 * (at + j) as usize];
        assert!((got - want).abs() < 2e-3, "frame {j}: {got} for {want}");
    }
}

/// Suggest by ear: the loop closes on the press and the tap, then offers
/// the first kick and the downbeat heard as one snap, once the finder
/// has answered.
#[test]
fn suggest_by_ear_offers_the_first_hit_and_the_downbeat_heard() {
    let early = 1_764;
    let mut r = Rig::new(256, SmartEnd::Suggest, 0);
    let (rec_from, tap) = r.late_tap(early, 1_323);
    r.until_playing(512);
    assert_eq!(r.l.status().frames as u64, tap - rec_from, "on the tap");
    r.run(tap + REACH + 1_024);
    let by = Shift {
        start: early as i64,
        end: (rec_from + early + TWO_BARS) as i64 - tap as i64,
    };
    assert_eq!(r.l.status().offer, Some(by));
    let note = r.l.take_smart().unwrap();
    assert_eq!(note.outcome, SmartOutcome::Offered(by.end));
}

/// The ends found are the same at any period, and an input heard later
/// (its latency) records the same loop a latency earlier.
#[test]
fn the_same_take_finds_the_same_ends_at_any_period_and_latency() {
    let early = 1_764;
    let mut want = None;
    for (period, latency) in [(64, 0), (256, 0), (1024, 0), (4096, 0), (256, 300)] {
        let mut r = Rig::new(period, SmartEnd::Auto, latency);
        let (rec_from, _) = r.late_tap(early, 1_323);
        r.until_playing(REACH + 300 + 2 * 4096);
        let s = r.l.status();
        let start = rec_from as i64 + s.moved.unwrap().start + i64::from(latency);
        let got = (s.frames, start);
        assert_eq!(*want.get_or_insert(got), got, "{period}, {latency}");
        assert_eq!(s.frames as u64, TWO_BARS);
    }
}

/// The finder never answers: Auto waits a little past its reach, then
/// keeps the tap (counted, and the word says why).
#[test]
fn with_no_answer_in_time_auto_keeps_the_tap() {
    let mut r = Rig::new(256, SmartEnd::Auto, 0);
    r.answers = Answers::Never;
    let (rec_from, tap) = r.late_tap(0, 1_323);
    r.until_playing(REACH + 4_096);
    let (_, at) = r.tempo.unwrap();
    let wait = (FIND_WAIT_AUTO_MS * f64::from(RATE) / 1000.0).round() as u64;
    assert!(at >= tap + REACH + wait, "{at}");
    assert_eq!(r.l.status().frames as u64, tap - rec_from);
    let note = r.l.take_smart().unwrap();
    assert_eq!(note.outcome, SmartOutcome::Kept(Verdict::Late.why()));
    assert_eq!(r.l.stats().finds_late, 1);
}

/// An answer for a take that has gone is dropped: the loop was cleared
/// while the finder held the job.
#[test]
fn an_answer_for_a_cleared_take_is_dropped() {
    let mut r = Rig::new(256, SmartEnd::Suggest, 0);
    r.answers = Answers::Holds;
    let (_, tap) = r.late_tap(0, 1_323);
    r.until_playing(512);
    r.run(tap + REACH + 512);
    assert!(r.held, "the finder has the job");
    assert!(r.l.command(L::Clear));
    r.run(r.frame + 512);
    r.answer_now();
    r.run(r.frame + 512);
    assert_eq!(r.l.stats().finds_stale, 1);
    assert_eq!(r.l.status().offer, None);
    assert_eq!(r.l.take_smart(), None);
}
