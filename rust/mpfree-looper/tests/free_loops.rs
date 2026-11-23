//! Free loops ([`Clock::Free`]) against a real transport: a free loop
//! recorded while the song plays is a song loop, sample for sample, until
//! the song stops, then plays on; the song's locates, tempo changes and own
//! loop don't reach it; song loops sound the same beside one; STOP with the
//! song stopped stops the free loops at their tops and PLAY starts them
//! again; a free loop recorded in your own time sets the free clock, not
//! the song; free loops recorded over each other line up on its bars; STOP
//! keeps what a free pass played; Undo brings the last free loop back with
//! its clock; Play waits for the song's bar line; a note starts a free loop
//! on the free clock's nearest bar line; saved free loops open stopped; a
//! project's free clock opens as soon as the old free loops are empty; and
//! Capture into a free loop takes the free clock's bars (as it runs, or as
//! it last ran), or with none, the song's, setting the free clock from
//! them. The free clock itself counts as the song's spans do.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s).

use mpfree_looper::save::{from_text, to_text, ClockFile, Saved};
use mpfree_looper::{
    Clock, Command as L, FreeClock, Looper, NoteStart, Phase, Shift, SmartEnd, Snap, Starts,
    Status, FADE,
};
use mpfree_transport::{Block, Command as T, Meter, State, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050;
const BAR_TICKS: i64 = 3_840;
/// The transport starts playing here, so the song's bar lines are at
/// START + k * BAR.
const START: u64 = 1_000;

struct Rig {
    t: Transport,
    l: Looper,
    b: Block,
    period: usize,
    frame: u64,
    input: Vec<f32>,
    mix: Vec<f32>,
    /// Every frame's output so far, interleaved stereo.
    out: Vec<f32>,
    /// The first frame of the first period each loop was seen recording.
    recording_from: [Option<u64>; 8],
    /// Notes played (pads), given to the looper period by period.
    notes: Vec<NoteStart>,
}

impl Rig {
    fn new(period: usize) -> Rig {
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            l: Looper::new(RATE, 20),
            b: Block::new(),
            period,
            frame: 0,
            input: vec![0.0; 2 * period],
            mix: vec![0.0; 2 * period],
            out: Vec::new(),
            recording_from: [None; 8],
            notes: Vec::new(),
        }
    }

    fn at(&mut self, frame: u64, c: T) {
        assert!(self.t.command(frame, c));
    }

    fn cmd(&mut self, c: L) {
        assert!(self.l.command(c));
    }

    fn select(&mut self, n: usize) {
        assert!(self.l.select_loop(n));
    }

    fn status(&self, n: usize) -> Status {
        self.l.loop_status(n).unwrap()
    }

    /// Run up to absolute frame `to`, feeding `input(frame)`.
    fn run(&mut self, to: u64, input: &dyn Fn(u64) -> [f32; 2]) {
        while self.frame < to {
            let n = (self.period as u64).min(to - self.frame) as usize;
            for i in 0..n {
                let x = input(self.frame + i as u64);
                self.input[2 * i] = x[0];
                self.input[2 * i + 1] = x[1];
            }
            self.mix[..2 * n].fill(0.0);
            let end = self.frame + n as u64;
            for x in self
                .notes
                .iter()
                .filter(|x| x.frame >= self.frame && x.frame < end)
            {
                self.l.note(*x);
            }
            assert!(self.t.process(n as u32, &mut self.b));
            self.l
                .process(&self.b, &self.input[..2 * n], &mut self.mix[..2 * n]);
            self.out.extend_from_slice(&self.mix[..2 * n]);
            for (k, at) in self.recording_from.iter_mut().enumerate() {
                if at.is_none() && self.l.loop_status(k).unwrap().phase == Phase::Recording {
                    *at = Some(self.frame);
                }
            }
            self.frame += n as u64;
        }
    }

    fn out_at(&self, f: u64) -> [f32; 2] {
        [self.out[2 * f as usize], self.out[2 * f as usize + 1]]
    }

    /// Whether every frame in `a..b` is silent.
    fn silent(&self, a: u64, b: u64) -> bool {
        self.out[2 * a as usize..2 * b as usize]
            .iter()
            .all(|&v| v == 0.0)
    }
}

/// Deterministic noise, different on each channel.
fn noise(f: u64) -> [f32; 2] {
    let h = |s: u64| {
        let mut x = f.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(s);
        x ^= x >> 29;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 32;
        (x % 10_000) as f32 / 10_000.0 - 0.5
    };
    [h(1), h(2)]
}

fn quiet(_: u64) -> [f32; 2] {
    [0.0, 0.0]
}

/// The first frame where two outputs differ, if any.
fn first_difference(a: &[f32], b: &[f32]) -> Option<(usize, f32, f32)> {
    assert_eq!(a.len(), b.len(), "the same frames");
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .map(|i| (i / 2, a[i], b[i]))
}

fn close(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
}

/// Loop 0 records `bars` bars while the song plays: Record half a bar
/// after PLAY, so the pass starts on the song's second bar line (START +
/// BAR) and ends `bars` bars later.
fn record_with_the_song(r: &mut Rig, clock: Clock, bars: u32) {
    r.cmd(L::SetClock(clock));
    r.cmd(L::SetBars(Some(bars)));
    r.at(START, T::Play);
    r.run(START + BAR / 2, &noise);
    r.cmd(L::Record);
    r.run(START + (1 + u64::from(bars)) * BAR + 1, &noise);
    let s = r.status(0);
    assert_eq!(s.phase, Phase::Playing);
    assert_eq!(s.clock, clock);
    assert_eq!(s.frames, bars as usize * BAR as usize);
    assert_eq!(s.ticks, i64::from(bars) * BAR_TICKS);
    assert_eq!(s.origin, BAR_TICKS);
}

/// Loop `n` records free in your own time with the song stopped: Record,
/// `frames` frames, Record again; it closes on the next period's first
/// frame. Returns the frame its first pass started on and the frame it
/// closed on.
fn record_in_own_time(
    r: &mut Rig,
    n: usize,
    frames: u64,
    input: &dyn Fn(u64) -> [f32; 2],
) -> (u64, u64) {
    r.select(n);
    r.cmd(L::SetClock(Clock::Free));
    r.cmd(L::SetBars(None));
    // (Commands take effect on the next period.)
    let f = r.frame + r.period as u64;
    r.run(f, input);
    let s = r.status(n);
    assert_eq!(s.clock, Clock::Free);
    // With no free clock yet it records in your own time.
    assert_eq!(s.free, r.l.free_clock().is_none());
    r.cmd(L::Record);
    let from = r.frame;
    r.run(from + frames, input);
    assert_eq!(r.status(n).phase, Phase::Recording);
    r.cmd(L::Record);
    let mut closed = None;
    while r.status(n).phase == Phase::Recording {
        let f = r.frame + r.period as u64;
        r.run(f, input);
        closed = Some(r.frame);
    }
    (from, closed.expect("it closed"))
}

#[test]
fn a_free_loop_recorded_while_the_song_plays_is_a_song_loop_until_the_song_stops() {
    let stop = START + 8 * BAR + BAR / 3;
    let play = |clock: Clock, stops: bool| {
        let mut r = Rig::new(128);
        record_with_the_song(&mut r, clock, 2);
        if stops {
            r.at(stop, T::Stop);
        }
        r.run(START + 12 * BAR, &noise);
        r
    };
    let song = play(Clock::Song, true);
    let free = play(Clock::Free, true);
    // The song never stops under this one.
    let on = play(Clock::Song, false);
    let s = 2 * stop as usize;
    assert!(
        free.out[2 * (START + 3 * BAR) as usize..s]
            .iter()
            .any(|&v| v != 0.0),
        "the loop plays"
    );
    assert_eq!(
        first_difference(&free.out[..s], &song.out[..s]),
        None,
        "sample for sample a song loop until STOP"
    );
    assert_eq!(
        first_difference(&free.out[s..], &on.out[s..]),
        None,
        "then it plays on as if the song had never stopped"
    );
    assert!(song.silent(stop + FADE as u64 + 1, START + 12 * BAR));
    assert_eq!(free.t.state(), State::Stopped);
    let c = free.l.free_clock().expect("a free clock");
    assert!(c.running);
    assert_eq!((c.bpm, c.top), (240.0, BAR_TICKS));
    assert!(song.l.free_clock().is_none(), "song loops have none");
}

#[test]
fn song_loops_sound_the_same_with_a_free_loop_beside_them() {
    let play = |with_free: bool| {
        let mut r = Rig::new(100);
        r.cmd(L::SetBars(Some(1)));
        r.cmd(L::Record);
        r.at(START, T::Play);
        r.run(START + 2 * BAR + BAR / 2, &noise);
        assert_eq!(r.status(0).phase, Phase::Playing);
        if with_free {
            // A free loop beside it, muted so that only the song loop is
            // heard: it records two bars from the song's next bar line.
            r.select(1);
            r.cmd(L::SetClock(Clock::Free));
            r.cmd(L::Mute(true));
            r.cmd(L::SetBars(Some(2)));
            r.cmd(L::Record);
            r.select(0);
        }
        r.run(START + 7 * BAR, &noise);
        // The song moves: a locate, a tempo change, its own loop, a stop.
        r.at(START + 7 * BAR + 100, T::Locate(BAR_TICKS / 2));
        r.run(START + 9 * BAR, &noise);
        r.at(START + 9 * BAR, T::TempoNow(200.0));
        r.run(START + 10 * BAR, &noise);
        r.at(START + 10 * BAR + 7, T::SetLoop(Some((0, 2 * BAR_TICKS))));
        r.run(START + 14 * BAR, &noise);
        r.at(START + 14 * BAR + 77, T::Stop);
        r.run(START + 15 * BAR, &noise);
        r.at(START + 15 * BAR + 500, T::Play);
        r.run(START + 17 * BAR, &noise);
        r
    };
    let a = play(false);
    let b = play(true);
    assert_eq!(first_difference(&a.out, &b.out), None);
    let s = b.status(1);
    assert_eq!(
        (s.phase, s.clock, s.muted),
        (Phase::Playing, Clock::Free, true)
    );
    assert_eq!(s.frames, 2 * BAR as usize);
}

#[test]
fn a_free_loop_keeps_its_tempo_and_place_through_tempo_changes_and_locates() {
    let play = |moves: bool| {
        let mut r = Rig::new(128);
        record_with_the_song(&mut r, Clock::Free, 2);
        if moves {
            r.at(START + 4 * BAR + 300, T::Locate(17 * BAR_TICKS + 100));
            r.run(START + 5 * BAR, &noise);
            r.at(START + 5 * BAR + 1000, T::TempoNow(173.0));
            r.run(START + 6 * BAR, &noise);
            r.at(START + 6 * BAR, T::SetLoop(Some((0, BAR_TICKS))));
            r.at(START + 6 * BAR, T::Locate(0));
            r.run(START + 8 * BAR, &noise);
            r.at(START + 8 * BAR + 5, T::SetTempo(91.0));
        }
        r.run(START + 11 * BAR, &noise);
        r
    };
    let still = play(false);
    let moved = play(true);
    // Only the free loop sounds, and none of the song's moves reach it.
    assert_eq!(first_difference(&still.out, &moved.out), None);
    let c = moved.l.free_clock().unwrap();
    assert_eq!((c.bpm, c.running), (240.0, true));
    assert_eq!(moved.status(0).frames, 2 * BAR as usize);
}

#[test]
fn a_second_stop_stops_the_free_loops_and_play_starts_them_from_their_tops() {
    let mut r = Rig::new(128);
    record_with_the_song(&mut r, Clock::Free, 1);
    // What the loop holds: the bar from START + BAR.
    let looped = |f: u64| noise(START + BAR + (f - START - BAR) % BAR);
    let stop = START + 3 * BAR + BAR / 2;
    r.at(stop, T::Stop);
    r.run(stop + BAR, &noise);
    // The song stops; the free loop plays on (away from its loop point).
    let seam = START + 4 * BAR;
    for f in (stop..stop + BAR).filter(|f| f.abs_diff(seam) > FADE as u64) {
        assert!(close(r.out_at(f), looped(f)), "frame {f}");
    }
    // STOP again, with the song stopped: the free loops stop, at their tops.
    let panic = stop + BAR + 333;
    r.at(panic, T::Stop);
    r.run(panic + 2 * BAR, &noise);
    assert!(r.silent(panic + FADE as u64 + 1, panic + 2 * BAR));
    let c = r.l.free_clock().unwrap();
    assert!(!c.running);
    assert_eq!((c.anchor_tick, c.top), (BAR_TICKS, BAR_TICKS));
    assert_eq!(r.status(0).position, 0.0, "at its top");
    assert_eq!(r.status(0).phase, Phase::Playing);
    // PLAY: the song, and the free loop from its top on the same frame.
    let play = panic + 2 * BAR + 50;
    r.at(play, T::Play);
    r.run(play + 2 * BAR, &noise);
    assert!(r.l.free_clock().unwrap().running);
    assert_eq!(r.t.state(), State::Playing);
    for j in FADE as u64..BAR - FADE as u64 {
        assert!(
            close(r.out_at(play + j), noise(START + BAR + j)),
            "frame {j} of the loop"
        );
    }
}

#[test]
fn a_free_loop_in_your_own_time_sets_the_free_clock_and_not_the_song() {
    let mut r = Rig::new(128);
    r.run(1_000, &quiet);
    // 1.6 s: one bar at 150 BPM.
    let (from, closed) = record_in_own_time(&mut r, 0, 35_280, &noise);
    assert!(
        r.l.take_tempo().is_none(),
        "the song's tempo isn't asked for"
    );
    assert_eq!(r.t.state(), State::Stopped);
    let s = r.status(0);
    let len = (closed - from) as usize;
    assert_eq!(
        (s.phase, s.clock, s.frames, s.origin),
        (Phase::Playing, Clock::Free, len, 0)
    );
    assert!(!s.free);
    let c = r.l.free_clock().expect("the free clock");
    assert!(c.running);
    assert_eq!((c.anchor_frame, c.anchor_tick, c.top), (closed, 0, 0));
    // Its tempo is the loop's bars over its length, exactly.
    let bars = s.ticks / BAR_TICKS;
    assert!(bars >= 1);
    let want = 60.0 * f64::from(RATE) * 4.0 * bars as f64 / len as f64;
    assert!((c.bpm - want).abs() < 1e-9, "{} vs {want}", c.bpm);
    // It plays at once, with the song stopped, from its top.
    r.run(closed + len as u64, &quiet);
    for j in FADE as u64..len as u64 - FADE as u64 {
        assert!(close(r.out_at(closed + j), noise(from + j)), "frame {j}");
    }
    assert_eq!(r.t.state(), State::Stopped);
}

#[test]
fn a_free_loop_recorded_over_another_lines_up_with_its_bars() {
    let mut r = Rig::new(128);
    r.run(1_000, &quiet);
    let (_, closed) = record_in_own_time(&mut r, 0, 35_280, &noise);
    let one = r.status(0);
    let c = r.l.free_clock().unwrap();
    let bar = c.meter.bar_ticks();
    assert_eq!(one.ticks % bar, 0);
    // Loop 2, free, two bars, armed a little way in: it starts on the free
    // clock's next bar line a beat or more on.
    r.run(closed + 5_000, &quiet);
    r.select(1);
    r.cmd(L::SetClock(Clock::Free));
    r.cmd(L::SetBars(Some(2)));
    r.cmd(L::Record);
    let pressed = r.frame;
    r.run(pressed + 1, &noise);
    let s = r.status(1);
    assert_eq!(s.phase, Phase::Armed);
    let line = s.starts_on.expect("its bar line");
    assert_eq!(line % bar, 0);
    let span = c.span(0, 0, pressed);
    let line_frame = span.frame_of(line as f64);
    assert!(line_frame > pressed);
    assert_eq!(
        c.next_bar(pressed + 1),
        Some((line, line_frame)),
        "the next bar line"
    );
    r.run(
        line_frame + 2 * one.frames as u64 + 2 * r.period as u64,
        &noise,
    );
    let two = r.status(1);
    assert_eq!(two.phase, Phase::Playing);
    assert_eq!((two.origin, two.ticks), (line, 2 * bar));
    assert!(
        two.frames.abs_diff(2 * one.frames) <= 1,
        "{} vs {}",
        two.frames,
        one.frames
    );
    // Its first frame is the input on the line's frame.
    assert_eq!(r.recording_from[1].map(|f| f <= line_frame), Some(true));
    let mut first = [[0.0f32; 2]; 4];
    r.l.loop_layer(1, 0).unwrap().read(0, &mut first);
    assert_eq!(first[0], noise(line_frame));
    // Loop 1 is at its top there.
    assert_eq!((line - c.top) % one.ticks, 0);
    assert_eq!(r.t.state(), State::Stopped, "the song never started");
}

#[test]
fn stop_while_a_free_pass_records_keeps_what_you_played() {
    let mut r = Rig::new(128);
    r.run(1_000, &quiet);
    let (_, closed) = record_in_own_time(&mut r, 0, 35_280, &noise);
    let one = r.status(0);
    let c = r.l.free_clock().unwrap();
    r.select(1);
    r.cmd(L::SetClock(Clock::Free));
    r.cmd(L::SetBars(Some(4)));
    // (Not on the free clock's bar 1, where loop 0 closed: it would
    // start there and then.)
    r.run(closed + 1_000, &noise);
    r.cmd(L::Record);
    r.run(r.frame + 10, &noise);
    assert_eq!(r.status(1).phase, Phase::Armed);
    let line = r.status(1).starts_on.unwrap();
    let start = c.span(0, 0, closed).frame_of(line as f64);
    // A bar and a half in, STOP (the song is stopped already).
    let stop = start + one.frames as u64 * 3 / 2;
    r.run(stop - 100, &noise);
    assert_eq!(r.status(1).phase, Phase::Recording);
    r.at(stop, T::Stop);
    r.run(stop + 2 * one.frames as u64, &noise);
    let s = r.status(1);
    // Kept to the next bar line, silent after STOP.
    assert_eq!(s.phase, Phase::Playing);
    assert_eq!(s.ticks, 2 * c.meter.bar_ticks());
    assert!(s.frames.abs_diff(2 * one.frames) <= 1);
    assert_eq!(s.cut, Some((stop - start) as usize));
    // The free loops stopped, the free clock back on its bar 1, and each
    // loop waits in its place there: loop 0 at its start, loop 1 (which
    // started a bar later) half way through, as they played together.
    let c = r.l.free_clock().unwrap();
    assert!(!c.running);
    assert!(r.silent(stop + FADE as u64 + 1, r.frame));
    assert_eq!((r.status(0).origin, c.top), (0, 0));
    assert_eq!(
        line,
        c.meter.bar_ticks(),
        "loop 1 started on the free clock's bar 2"
    );
    assert_eq!((r.status(0).position, r.status(1).position), (0.0, 0.5));
}

#[test]
fn undo_brings_the_last_free_loop_back_with_its_clock_until_another_is_set() {
    let mut r = Rig::new(128);
    r.run(1_000, &quiet);
    let (_, closed) = record_in_own_time(&mut r, 0, 35_280, &noise);
    let was = r.l.free_clock().unwrap();
    r.run(closed + BAR, &quiet);
    r.cmd(L::Clear);
    r.run(r.frame + 4 * FADE as u64, &quiet);
    assert_eq!(r.status(0).phase, Phase::Empty);
    assert!(r.status(0).kept);
    assert!(
        r.l.free_clock().is_none(),
        "it went with the last free loop"
    );
    r.cmd(L::Undo);
    r.run(r.frame + 4 * FADE as u64, &quiet);
    assert_eq!(r.status(0).phase, Phase::Playing);
    assert_eq!(r.l.free_clock(), Some(was), "and came back with it");
    // Cleared again, and a new free loop sets a new clock: Undo can't
    // bring the old one back now.
    r.cmd(L::Clear);
    r.run(r.frame + 4 * FADE as u64, &quiet);
    assert!(r.l.free_clock().is_none());
    let refused = r.l.stats().refused;
    record_in_own_time(&mut r, 1, 30_000, &noise);
    assert_ne!(r.l.free_clock(), Some(was));
    r.select(0);
    r.cmd(L::Undo);
    r.run(r.frame + 4 * FADE as u64, &quiet);
    assert_eq!(r.status(0).phase, Phase::Empty);
    assert_eq!(r.l.stats().refused, refused + 1);
}

#[test]
fn play_for_stopped_free_loops_waits_for_the_songs_next_bar_line() {
    let mut r = Rig::new(128);
    r.run(1_000, &quiet);
    let (_, closed) = record_in_own_time(&mut r, 0, 35_280, &noise);
    r.run(closed + 1000, &quiet);
    // The song plays (the free loops play on), and Stop for the free
    // loops stops them, not the song.
    let song = r.frame + 1000;
    r.at(song, T::Play);
    r.run(song + BAR / 2, &quiet);
    assert!(r.l.free_clock().unwrap().running);
    r.cmd(L::FreeStop);
    r.run(r.frame + 1, &quiet);
    assert!(!r.l.free_clock().unwrap().running);
    assert_eq!(r.t.state(), State::Playing, "the song plays on");
    // Play for the free loops just before a bar line waits for the next
    // but one (a beat's warning).
    r.run(song + BAR - 2_000, &quiet);
    r.cmd(L::FreePlay);
    r.run(song + 2 * BAR - 100, &quiet);
    assert!(
        !r.l.free_clock().unwrap().running,
        "not on the bar line just after"
    );
    r.run(song + 2 * BAR + 100, &quiet);
    let c = r.l.free_clock().unwrap();
    assert!(c.running);
    assert_eq!((c.anchor_frame, c.anchor_tick), (song + 2 * BAR, c.top));
    // With the song stopped, Stop and Play act at once.
    r.at(r.frame, T::Stop);
    r.run(r.frame + 1, &quiet);
    assert!(
        r.l.free_clock().unwrap().running,
        "STOP stops the song first"
    );
    r.cmd(L::FreeStop);
    r.run(r.frame + 1, &quiet);
    assert!(!r.l.free_clock().unwrap().running);
    r.cmd(L::FreePlay);
    let at = r.frame;
    r.run(at + 1, &quiet);
    let c = r.l.free_clock().unwrap();
    assert_eq!((c.running, c.anchor_frame), (true, at));
}

#[test]
fn a_note_starts_a_free_loop_on_the_free_clocks_nearest_bar_line() {
    let mut r = Rig::new(128);
    r.run(1_000, &quiet);
    let (_, closed) = record_in_own_time(&mut r, 0, 35_280, &noise);
    let c = r.l.free_clock().unwrap();
    let one = r.status(0).frames as u64;
    r.select(1);
    r.cmd(L::SetClock(Clock::Free));
    r.cmd(L::SetStarts(Starts::Play));
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    r.run(closed + 3 * one / 2, &quiet);
    assert!(r.status(1).listening);
    // A note a little before the free clock's next bar line: a pickup.
    let line = closed + 2 * one;
    let note = line - 900;
    r.run(note, &quiet);
    r.cmd(L::Played(note));
    r.run(line + one + 4 * r.period as u64, &noise);
    let s = r.status(1);
    assert_eq!(s.phase, Phase::Playing);
    assert_eq!(
        s.origin,
        2 * c.meter.bar_ticks(),
        "on the clock's third bar line"
    );
    assert!(s.lead.abs_diff(900) <= 1, "the note leads in: {}", s.lead);
}

#[test]
fn free_loops_open_stopped_at_their_tops() {
    let mut r = Rig::new(128);
    r.run(1_000, &quiet);
    let (from, closed) = record_in_own_time(&mut r, 0, 35_280, &noise);
    r.run(closed + 2 * BAR, &quiet);
    // Saved as a project keeps it: the text, and the layer's audio.
    let mut f = r.l.loop_file(0).expect("savable");
    let mut audio = vec![0.0; 2 * f.frames];
    assert!(r.l.read_layer(0, 0, 0, &mut audio).is_some());
    f.layers[0].file = "a.wav".into();
    let saved = Saved {
        rate: RATE,
        selected: 0,
        loops: vec![(0, f)],
        free_clock: r.l.free_clock().map(|c| ClockFile::of(&c)),
    };
    let text = to_text(&saved);
    assert!(text.contains(" clock=free"), "{text}");
    assert!(text.contains("\nfree-clock bpm="), "{text}");
    let mut back = from_text(&text).unwrap();
    assert_eq!(back, saved);
    let (_, mut f) = back.loops.remove(0);
    f.layers[0].audio = audio;
    // Into a new looper: not without its clock; with it, stopped.
    let mut o = Rig::new(128);
    o.run(1_000, &quiet);
    let f = o.l.load(0, f).expect_err("no free clock");
    let c = back.free_clock.unwrap();
    assert!(o.l.open_free_clock(c.bpm, c.meter, c.top));
    assert!(o.l.load(0, f).is_ok());
    assert!(
        !o.l.open_free_clock(c.bpm, c.meter, c.top),
        "not while a free loop loads onto it"
    );
    o.run(1_000 + 2 * BAR, &quiet);
    let s = o.status(0);
    assert_eq!(
        (s.phase, s.clock, s.loading),
        (Phase::Playing, Clock::Free, None)
    );
    assert!(o.silent(1_000, o.frame), "stopped at its top");
    assert_eq!(s.position, 0.0);
    // Play: from its top.
    o.cmd(L::FreePlay);
    let at = o.frame;
    o.run(at + 2 * BAR, &quiet);
    let len = s.frames as u64;
    for j in FADE as u64..len - FADE as u64 {
        assert!(close(o.out_at(at + j), noise(from + j)), "frame {j}");
    }
}

/// Opening a project as the app does: the old loops are forgotten, and
/// the project's free clock opens as soon as they are empty, though the
/// old clock would go only with the next period. Until then a free loop
/// holding audio keeps its clock; afterwards the old loop's Undo is gone
/// with its clock.
#[test]
fn a_project_opens_its_free_clock_as_soon_as_the_old_free_loops_are_empty() {
    let mut r = Rig::new(128);
    r.run(1_000, &quiet);
    let (_, closed) = record_in_own_time(&mut r, 0, 35_280, &noise);
    r.run(closed + BAR, &quiet);
    let old = r.l.free_clock().expect("its clock");
    let meter = Meter { beats: 3, unit: 4 };
    assert!(
        !r.l.open_free_clock(90.0, meter, 0),
        "not while a free loop holds audio"
    );
    // Clear, so that Undo could bring it back on its clock.
    r.cmd(L::Clear);
    while r.status(0).phase != Phase::Empty {
        let f = r.frame + r.period as u64;
        r.run(f, &noise);
    }
    // It faded out in this period: its clock goes with the next one.
    assert_eq!(r.l.free_clock(), Some(old), "the old clock lingers");
    assert!(r.l.open_free_clock(90.0, meter, 0));
    let new = r.l.free_clock().expect("the project's clock");
    assert_eq!((new.bpm, new.meter, new.running), (90.0, meter, false));
    let f = r.frame + r.period as u64;
    r.run(f, &quiet);
    // With no free loop on it, the opened clock goes too (a project's
    // loops load before the next period), and the old loop's Undo is
    // refused: its clock is gone.
    assert_eq!(r.l.free_clock(), None);
    let refused = r.l.stats().refused;
    r.cmd(L::Undo);
    let f = r.frame + 2 * r.period as u64;
    r.run(f, &quiet);
    assert_eq!(r.status(0).phase, Phase::Empty);
    assert_eq!(r.l.stats().refused, refused + 1);
}

/// Runs until loop `n` has copied a capture in and plays (at most
/// `most` frames more).
fn until_captured(r: &mut Rig, n: usize, most: u64) {
    let to = r.frame + most;
    loop {
        let s = r.status(n);
        if s.capturing.is_none() && s.phase == Phase::Playing {
            return;
        }
        assert!(r.frame < to, "still {s:?}");
        let f = r.frame + r.period as u64;
        r.run(f, &noise);
    }
}

/// Loop `n`'s first layer, as it holds it.
fn layer(r: &Rig, n: usize) -> Vec<[f32; 2]> {
    let v = r.l.loop_layer(n, 0).expect("a layer");
    let mut out = vec![[0.0; 2]; v.len() / 2];
    v.read(0, &mut out);
    out
}

/// Loop 0 free, recorded with the song (2 bars from START + BAR), muted;
/// then the song stops, and the free clock (the song's, copied) runs on
/// alone: its bar lines stay on the song's, START + k * BAR on tick k
/// bars, from loop 0's first one on.
fn free_loops_alone(r: &mut Rig) {
    record_with_the_song(r, Clock::Free, 2);
    r.cmd(L::Mute(true));
    r.at(START + 4 * BAR + BAR / 3, T::Stop);
    r.run(START + 5 * BAR, &noise);
    assert_eq!(r.t.state(), State::Stopped);
    assert!(r.l.free_clock().unwrap().running);
}

#[test]
fn capture_into_a_free_loop_takes_the_free_clocks_last_bars_and_plays_them_on_it() {
    let mut r = Rig::new(128);
    free_loops_alone(&mut r);
    // Bar 8, beat 2 (counting from the song's bar 1).
    r.run(START + 7 * BAR + 6_000, &noise);
    // The song's last bars ended where it stopped; the free clock's run on.
    let song = r.l.capture_take(Clock::Song, 2).expect("the song's");
    assert_eq!(song.origin, 2 * BAR_TICKS, "the song's bars 3 and 4");
    let t = r.l.capture_take(Clock::Free, 2).expect("the free clock's");
    assert_eq!(
        (t.bars, t.start, t.frames, t.ticks, t.origin),
        (
            2,
            START + 5 * BAR,
            2 * BAR as usize,
            2 * BAR_TICKS,
            5 * BAR_TICKS
        )
    );
    assert!(!r.l.capture_waits(Clock::Free));
    // Its bar lines since it started (on loop 0's first frame).
    let mut lines = [(0, 0); 16];
    let n = r.l.heard_lines(Clock::Free, &mut lines);
    let want: Vec<(i64, u64)> = (1..=7u64)
        .map(|k| (k as i64 * BAR_TICKS, START + k * BAR))
        .collect();
    assert_eq!(&lines[..n], &want[..]);
    r.select(1);
    r.cmd(L::SetClock(Clock::Free));
    r.cmd(L::Capture(2));
    until_captured(&mut r, 1, BAR);
    let s = r.status(1);
    assert_eq!(
        (s.clock, s.frames, s.ticks, s.origin),
        (Clock::Free, 2 * BAR as usize, 2 * BAR_TICKS, 5 * BAR_TICKS)
    );
    let a = layer(&r, 1);
    for (j, x) in a[..a.len() - FADE].iter().enumerate() {
        assert_eq!(*x, noise(START + 5 * BAR + j as u64), "frame {j}");
    }
    // Once faded in it plays on the free clock: frame f plays its frame
    // at f's place in the free clock's bars.
    let from = r.frame + FADE as u64;
    r.run(from + 2 * BAR, &noise);
    for f in from..from + 2 * BAR {
        let p = (f - START - 5 * BAR) % (2 * BAR);
        if p < FADE as u64 || p > 2 * BAR - FADE as u64 {
            continue;
        }
        assert!(close(r.out_at(f), a[p as usize]), "frame {f}");
    }
    assert_eq!(r.l.stats().refused, 0);
}

#[test]
fn song_bars_captured_into_a_free_loop_set_the_free_clock_from_the_songs() {
    // At 239 BPM a bar is 22 142.26 frames, so bar lines fall on rounded
    // frames: a clock worked out from them would drift from the song's.
    let stop = START + 8 * BAR + BAR / 3;
    let play = |clock: Clock, stops: bool| {
        let mut r = Rig::new(128);
        r.t = Transport::new(RATE, 239.0).unwrap();
        r.cmd(L::SetClock(clock));
        r.at(START, T::Play);
        // Bar 4, beat 2: the song's bars 2 and 3.
        r.run(START + 3 * BAR + 6_000, &noise);
        let t = r.l.capture_take(clock, 2).expect("two bars");
        assert_eq!((t.origin, t.ticks), (BAR_TICKS, 2 * BAR_TICKS));
        r.cmd(L::Capture(2));
        if stops {
            r.at(stop, T::Stop);
        }
        r.run(START + 12 * BAR, &noise);
        assert_eq!(r.status(0).phase, Phase::Playing);
        r
    };
    let song = play(Clock::Song, true);
    let free = play(Clock::Free, true);
    // The song never stops under this one.
    let on = play(Clock::Song, false);
    let s = 2 * stop as usize;
    assert!(free.out[..s].iter().any(|&v| v != 0.0), "it plays");
    assert_eq!(
        first_difference(&free.out[..s], &song.out[..s]),
        None,
        "sample for sample a song loop until STOP"
    );
    assert_eq!(
        first_difference(&free.out[s..], &on.out[s..]),
        None,
        "then it plays on as if the song had never stopped"
    );
    let c = free.l.free_clock().expect("set from the song's bars");
    assert!(c.running);
    assert_eq!(
        (c.bpm, c.top, c.meter),
        (239.0, BAR_TICKS, Meter { beats: 4, unit: 4 })
    );
    let st = free.status(0);
    assert_eq!(
        (st.clock, st.origin, st.ticks),
        (Clock::Free, BAR_TICKS, 2 * BAR_TICKS)
    );
    assert!(song.l.free_clock().is_none());
}

#[test]
fn the_songs_last_bars_captured_into_a_free_loop_once_it_stopped_wait_for_play() {
    let mut r = Rig::new(128);
    r.cmd(L::SetClock(Clock::Free));
    r.at(START, T::Play);
    r.at(START + 4 * BAR + BAR / 3, T::Stop);
    r.run(START + 5 * BAR, &noise);
    // The bars the song played last: 3 and 4.
    r.cmd(L::Capture(2));
    until_captured(&mut r, 0, BAR);
    let s = r.status(0);
    assert_eq!(
        (s.clock, s.origin, s.ticks),
        (Clock::Free, 2 * BAR_TICKS, 2 * BAR_TICKS)
    );
    // Their tempo, stopped on their first bar: nothing plays.
    let c = r.l.free_clock().expect("set from them");
    assert_eq!(
        (c.bpm, c.running, c.top, c.anchor_tick),
        (240.0, false, 2 * BAR_TICKS, 2 * BAR_TICKS)
    );
    let quiet_from = r.frame;
    r.run(quiet_from + BAR, &noise);
    assert!(r.silent(quiet_from, r.frame));
    // PLAY: the song, and the free loop from its top with it.
    let play = r.frame + 77;
    r.at(play, T::Play);
    r.run(play + 2 * BAR, &noise);
    assert!(r.l.free_clock().unwrap().running);
    let a = layer(&r, 0);
    for j in FADE as u64..2 * BAR - FADE as u64 {
        assert_eq!(a[j as usize], noise(START + 2 * BAR + j));
        assert!(close(r.out_at(play + j), a[j as usize]), "frame {j}");
    }
}

#[test]
fn in_a_bars_last_beat_a_free_capture_waits_for_the_free_clocks_bar_line() {
    let mut r = Rig::new(128);
    free_loops_alone(&mut r);
    // Bar 7, beat 4.
    r.run(START + 6 * BAR + 17_000, &noise);
    assert!(r.l.capture_waits(Clock::Free));
    assert!(!r.l.capture_waits(Clock::Song), "the song is stopped");
    r.select(1);
    r.cmd(L::SetClock(Clock::Free));
    r.cmd(L::Capture(2));
    r.run(START + 7 * BAR - 256, &noise);
    assert_eq!(r.status(1).capturing, Some(0.0), "waits for bar 8");
    until_captured(&mut r, 1, BAR);
    let s = r.status(1);
    assert_eq!(
        (s.origin, s.ticks),
        (5 * BAR_TICKS, 2 * BAR_TICKS),
        "bars 6 and 7"
    );
    assert_eq!(layer(&r, 1)[0], noise(START + 5 * BAR));
    // Waiting again, but the free loops stop first: the bars up to the
    // last line.
    r.select(2);
    r.cmd(L::SetClock(Clock::Free));
    r.run(START + 9 * BAR + 17_000, &noise);
    r.cmd(L::Capture(2));
    r.run(r.frame + 128, &noise);
    assert_eq!(r.status(2).capturing, Some(0.0), "waits for bar 11");
    r.cmd(L::FreeStop);
    until_captured(&mut r, 2, BAR);
    let s = r.status(2);
    assert_eq!(
        (s.origin, s.ticks),
        (7 * BAR_TICKS, 2 * BAR_TICKS),
        "bars 8 and 9"
    );
    assert_eq!(layer(&r, 2)[0], noise(START + 7 * BAR));
    assert!(!r.l.free_clock().unwrap().running);
    assert_eq!(r.l.stats().refused, 0);
}

#[test]
fn a_free_capture_waiting_for_its_bar_line_lets_go_once_the_free_loops_are_cleared() {
    let mut r = Rig::new(128);
    free_loops_alone(&mut r);
    // Bar 7, beat 4: it waits for bar 8's line.
    r.run(START + 6 * BAR + 17_000, &noise);
    r.select(1);
    r.cmd(L::SetClock(Clock::Free));
    r.cmd(L::Capture(2));
    r.run(r.frame + 128, &noise);
    assert_eq!(r.status(1).capturing, Some(0.0));
    // Loop 0, the only free loop, cleared first: the free clock goes, and
    // the bars the capture waits for with it.
    r.select(0);
    r.cmd(L::Clear);
    r.run(START + 7 * BAR + BAR / 2, &noise);
    assert!(r.l.free_clock().is_none());
    let s = r.status(1);
    assert_eq!((s.phase, s.capturing), (Phase::Empty, None));
    assert_eq!(r.l.stats().refused, 1);
}

#[test]
fn stopped_free_loops_keep_their_last_bars_for_capture_until_they_start_again() {
    let mut r = Rig::new(128);
    free_loops_alone(&mut r);
    r.run(START + 7 * BAR + 6_000, &noise);
    r.cmd(L::FreeStop);
    r.run(START + 8 * BAR, &noise);
    assert!(!r.l.free_clock().unwrap().running);
    // Stopped, the bars they played last can still be taken.
    let t = r.l.capture_take(Clock::Free, 2).expect("the last run's");
    assert_eq!((t.start, t.origin), (START + 5 * BAR, 5 * BAR_TICKS));
    assert!(!r.l.capture_waits(Clock::Free), "nothing comes");
    // Play: only bars since then count, so none yet.
    r.cmd(L::FreePlay);
    let play = r.frame;
    r.run(play + BAR / 2, &noise);
    assert_eq!(r.l.free_clock().unwrap().anchor_frame, play);
    assert_eq!(r.l.capture_take(Clock::Free, 1), None);
    r.select(1);
    r.cmd(L::SetClock(Clock::Free));
    r.cmd(L::Capture(1));
    r.run(r.frame + 2 * r.period as u64, &noise);
    assert_eq!(r.status(1).capturing, None);
    assert_eq!(r.l.stats().refused, 1, "no whole bar since Play");
    // A bar later there is one: the bar from Play, on the clock's bar 1.
    r.run(play + BAR + BAR / 2, &noise);
    let t = r.l.capture_take(Clock::Free, 4).expect("one bar");
    assert_eq!((t.bars, t.start, t.origin), (1, play, BAR_TICKS));
}

/// Loop 0 free in your own time, smart loop end on `mode`: two bars at
/// 120 BPM on the pads from Record (a note on every beat: 11 025 frames),
/// and Record again 60 ms (1 323 frames) after the downbeat that ends
/// them. The frame the pass started on, and the tap: the frame it closes
/// on without smart loop end.
fn free_late_tap(r: &mut Rig, mode: SmartEnd) -> (u64, u64) {
    r.cmd(L::SetClock(Clock::Free));
    r.cmd(L::SetBars(None));
    r.cmd(L::SetSmartEnd(mode));
    r.run(10_000, &noise);
    r.cmd(L::Record);
    let rec_from = r.frame;
    r.notes = (0..=8)
        .map(|k| NoteStart {
            frame: rec_from + k * 11_025,
            source: 0,
            key: if k % 2 == 0 { 36 } else { 38 },
            velocity: if k % 2 == 0 { 120 } else { 100 },
        })
        .collect();
    r.run(rec_from + 88_200 + 1_323, &noise);
    r.cmd(L::Record);
    // The tap is the end of the period that takes the press.
    let f = r.frame + r.period as u64;
    r.run(f, &noise);
    assert_eq!(r.status(0).phase, Phase::Playing);
    (rec_from, f)
}

/// Suggest's offer on a free loop: Snap moves its end and the free
/// clock's tempo together on its next downbeat, with nothing asked of the
/// time engine; from there it plays the two bars as recorded, sample for
/// sample, the free clock at exactly 120 BPM, its bar 1 on that downbeat.
/// The song's tempo is untouched. Use my tap puts the tap back the same
/// way.
#[test]
fn snap_on_a_free_loop_changes_the_free_clocks_tempo_not_the_songs() {
    let mut r = Rig::new(256);
    let (rec_from, tap) = free_late_tap(&mut r, SmartEnd::Suggest);
    let tap_bpm = r.l.free_clock().expect("set by the pass").bpm;
    r.run(tap + 2_205 + 256, &noise);
    let by = Shift {
        start: 0,
        end: (rec_from + 88_200) as i64 - tap as i64,
    };
    assert_eq!(r.status(0).offer, Some(by), "Suggest offers on a free loop");
    let p = r.l.snap_plan(0, Snap::Found, r.frame).unwrap();
    let s = r.status(0);
    assert_eq!(p.by, by);
    assert!((p.bpm - 120.0).abs() < 1e-9, "{}", p.bpm);
    assert_eq!((p.tick - s.origin).rem_euclid(s.ticks), 0, "a downbeat");
    assert!(p.at > r.frame && p.at <= r.frame + 256 + s.frames as u64);
    assert!(r.l.snap_at(p));
    r.run(p.at, &noise);
    assert_eq!(r.status(0).snapping, Some(p.at), "until it lands");
    assert_eq!(r.l.free_clock().unwrap().bpm, tap_bpm);
    r.run(p.at + 70_000, &noise);
    assert_eq!(r.l.take_snap().map(|d| d.landed), Some(true));
    let s = r.status(0);
    assert_eq!(
        (s.frames, s.ticks, s.origin),
        (88_200, 2 * BAR_TICKS, p.tick)
    );
    assert_eq!((s.offer, s.moved, s.snapping), (None, Some(by), None));
    let c = r.l.free_clock().unwrap();
    assert_eq!(
        (c.bpm, c.anchor_frame, c.anchor_tick, c.top, c.running),
        (p.bpm, p.at, p.tick, p.tick, true)
    );
    for j in [FADE as u64 + 10, 5_000, 60_000] {
        assert!(close(r.out_at(p.at + j), noise(rec_from + j)), "frame {j}");
    }
    assert_eq!((r.t.bpm(), r.t.state()), (240.0, State::Stopped));
    // Use my tap: back to the tap, with its tempo.
    let t = r.l.snap_plan(0, Snap::Tap, r.frame).unwrap();
    assert_eq!((t.by, t.bpm), (-by, tap_bpm));
    assert!(r.l.snap_at(t));
    r.run(t.at + 1_000, &noise);
    assert!(r.l.take_snap().unwrap().landed);
    assert_eq!(r.status(0).frames as u64, tap - rec_from);
    assert_eq!(r.l.free_clock().unwrap().bpm, tap_bpm);
    assert_eq!(r.l.stats().refused, 0);
}

/// Read as on a free loop: the same frames read as other bars, the free
/// clock's tempo changing with it on the downbeat, so it plays on sample
/// for sample; STOP and PLAY then start it from that downbeat, its bar 1.
#[test]
fn read_as_on_a_free_loop_keeps_every_frame_and_sets_the_free_clocks_tempo() {
    let mut r = Rig::new(256);
    let (rec_from, tap) = free_late_tap(&mut r, SmartEnd::Off);
    r.run(tap + 5_000, &noise);
    let len = tap - rec_from;
    let bpm = r.l.free_clock().unwrap().bpm;
    let s = r.status(0);
    assert_eq!((s.frames as u64, s.ticks), (len, 2 * BAR_TICKS));
    let p = r.l.snap_plan(0, Snap::Bars(4), r.frame).unwrap();
    assert_eq!((p.by, p.ticks), (Shift::NONE, 4 * BAR_TICKS));
    assert!((p.bpm - 2.0 * bpm).abs() < 1e-9, "{} {bpm}", p.bpm);
    assert!(r.l.snap_at(p));
    r.run(p.at + 60_000, &noise);
    assert_eq!(r.l.take_snap().map(|d| d.landed), Some(true));
    let s = r.status(0);
    assert_eq!(
        (s.frames as u64, s.ticks, s.origin),
        (len, 4 * BAR_TICKS, p.tick)
    );
    assert_eq!(r.l.free_clock().unwrap().bpm, p.bpm);
    for j in [0, 1, FADE as u64, 5_000, 59_999] {
        assert!(close(r.out_at(p.at + j), noise(rec_from + j)), "frame {j}");
    }
    // Stopped and started again: from its downbeat, the clock's bar 1.
    r.cmd(L::FreeStop);
    r.run(r.frame + BAR, &noise);
    r.cmd(L::FreePlay);
    let at = r.frame;
    r.run(at + BAR, &noise);
    for j in FADE as u64..BAR {
        assert!(close(r.out_at(at + j), noise(rec_from + j)), "frame {j}");
    }
    assert_eq!(r.l.stats().refused, 0);
}

/// A free loop can't snap while another free loop has sound (its tempo
/// would change under it) or while the free loops are stopped; one
/// worked out before they stopped doesn't land, and the free clock keeps
/// its tempo.
#[test]
fn a_free_loop_snaps_only_alone_and_while_the_free_loops_play() {
    let mut r = Rig::new(256);
    let (_, tap) = free_late_tap(&mut r, SmartEnd::Off);
    r.run(tap + 5_000, &noise);
    let bpm = r.l.free_clock().unwrap().bpm;
    // Planned, then the free loops stop before its downbeat.
    let p = r.l.snap_plan(0, Snap::Bars(4), r.frame).unwrap();
    assert!(r.l.snap_at(p));
    r.cmd(L::FreeStop);
    r.run(p.at + 1_000, &noise);
    assert_eq!(r.l.take_snap().map(|d| d.landed), Some(false));
    assert_eq!(r.l.stats().refused, 1);
    let c = r.l.free_clock().unwrap();
    assert_eq!((c.bpm, c.running), (bpm, false));
    assert_eq!(r.status(0).ticks, 2 * BAR_TICKS, "nothing moved");
    assert_eq!(
        r.l.snap_plan(0, Snap::Bars(4), r.frame),
        Err("the free loops aren't playing")
    );
    // Playing again, with another free loop recorded over it.
    r.cmd(L::FreePlay);
    r.run(r.frame + 256, &noise);
    assert!(r.l.snap_plan(0, Snap::Bars(4), r.frame).is_ok());
    r.select(1);
    r.cmd(L::SetClock(Clock::Free));
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    r.run(r.frame + 4 * r.status(0).frames as u64, &noise);
    assert_eq!(r.status(1).phase, Phase::Playing);
    assert_eq!(
        r.l.snap_plan(0, Snap::Bars(4), r.frame),
        Err("another free loop has sound")
    );
}

/// A free clock copied from one of the song's spans gives the song's ticks
/// bit for bit, the same spans and the same bar lines, at every tempo,
/// meter and place; at rest, it stays on its bar 1.
#[test]
fn the_free_clock_counts_as_the_songs_spans_do() {
    let mut checked = 0;
    let mut lines = 0;
    for (bpm, meter) in [
        (240.0, Meter { beats: 4, unit: 4 }),
        (97.3, Meter { beats: 7, unit: 8 }),
        (183.0, Meter { beats: 3, unit: 4 }),
        (61.1, Meter { beats: 5, unit: 16 }),
    ] {
        let mut t = Transport::new(48_000, bpm).unwrap();
        t.set_meter(meter).unwrap();
        let mut b = Block::new();
        assert!(t.command(0, T::Play));
        assert!(t.command(70_001, T::Locate(123_457)));
        assert!(t.command(150_000, T::TempoNow(bpm * 1.37)));
        for k in 0..400u64 {
            let n = [1u32, 64, 333, 1024, 4096][(k % 5) as usize];
            assert!(t.process(n, &mut b));
            for span in b.spans().iter().filter(|s| s.state == State::Playing) {
                let c = FreeClock {
                    anchor_frame: span.anchor_frame,
                    anchor_tick: span.anchor_tick,
                    bpm: span.bpm,
                    rate: span.rate,
                    meter: span.meter,
                    running: true,
                    top: 0,
                };
                let mut fs = c.span(span.offset, span.len, span.frame);
                fs.swing = span.swing;
                assert_eq!(fs, *span);
                for f in span.frame..span.frame + u64::from(span.len) {
                    assert_eq!(c.tick_at(f).to_bits(), span.tick_at(f).to_bits(), "{f}");
                    checked += 1;
                }
                let bar = span.meter.bar_ticks();
                if let Some((tick, o)) = span.lines(bar).next() {
                    let at = span.frame + u64::from(o - span.offset);
                    assert_eq!(
                        c.next_bar(span.frame),
                        Some((tick, at)),
                        "the span's first line"
                    );
                    assert_eq!(c.next_bar(at), Some((tick, at)));
                    lines += 1;
                }
            }
        }
    }
    assert!(
        checked > 1_000_000 && lines > 20,
        "{checked} frames, {lines} lines"
    );
    // At rest it stays on its bar 1 and has no lines.
    let c = FreeClock::stopped(120.0, 48_000, Meter { beats: 4, unit: 4 }, 7_680);
    assert_eq!(c.tick_at(123_456), 7_680.0);
    assert_eq!(c.span(0, 64, 123_456).state, State::Stopped);
    assert_eq!(c.next_bar(123_456), None);
}

/// Presses at random on two song loops and two free loops together, with
/// the song starting, stopping, locating and changing tempo under them,
/// the free loops' own Stop and Play, Capture and snaps (a round with a
/// snap runs until it lands), at an odd period size, beside the same
/// presses with no free loop at all (the free loops' presses left out,
/// everything else the same). The free loops are muted, so what is heard
/// is the song loops alone. Nothing panics, every layer plays its loop's
/// length, the memory adds up, a free loop playing always has the free
/// clock, and the song loops come out exactly as they do with no free
/// loop: the same output to the bit, the same state and the same layers,
/// sample for sample. The free loops take and give back memory, so the
/// song loops' overdubs get other blocks than they would without them:
/// that is how this found overdubs at another tempo playing what their
/// blocks held before (`tests/looper.rs`, an overdub at another tempo).
#[test]
fn random_presses_on_free_loops_leave_the_song_loops_as_they_would_be() {
    use mpfree_looper::Edit as E;
    let mut seed = 0x5EED_F2EE_100F_C0DEu64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let rig = || {
        let mut r = Rig::new(97);
        r.l = Looper::new(RATE, 240);
        r
    };
    let (mut a, mut b) = (rig(), rig());
    // Loops 2 and 3 are free, and muted from the start.
    for n in [2, 3] {
        a.select(n);
        a.cmd(L::SetClock(Clock::Free));
        a.cmd(L::Mute(true));
    }
    let input = |f: u64| {
        let x = noise(f);
        [0.3 * x[0], 0.2 * x[1]]
    };
    let same = |a: &Rig, b: &Rig, n: usize, round: u32, layers: bool| {
        let (x, y) = (a.status(n), b.status(n));
        let key = |s: &Status| {
            (
                s.phase,
                s.layers,
                s.redo,
                s.frames,
                s.ticks,
                s.origin,
                s.position.to_bits(),
                s.muted,
                s.version,
                (s.cut, s.starts_on, s.cue, s.kept),
                (s.speed, s.rate, s.clock, s.free),
            )
        };
        assert_eq!(key(&x), key(&y), "song loop {n} in round {round}");
        assert_eq!(x.capturing.map(f32::to_bits), y.capturing.map(f32::to_bits));
        if layers {
            for k in 0..x.layers {
                let (u, v) = (a.l.loop_layer(n, k).unwrap(), b.l.loop_layer(n, k).unwrap());
                let mut p = vec![[0.0f32; 2]; u.len() / 2];
                let mut q = vec![[0.0f32; 2]; v.len() / 2];
                u.read(0, &mut p);
                v.read(0, &mut q);
                assert!(p == q, "song loop {n} layer {k} in round {round}");
            }
        }
    };
    let (mut free_played, mut free_snaps, mut free_captures, mut song_played) = (0, 0, 0, 0);
    a.at(START, T::Play);
    b.at(START, T::Play);
    for round in 0..1_200u32 {
        // Every draw is made in both runs alike; only the free loops'
        // presses are left out of the run without them.
        let what = next() % 24;
        let free = (10..18).contains(&what);
        // Loop 3 records less, so that loop 2 is often the only free loop
        // with sound, which a free snap needs.
        let n = match next() % 4 {
            0 if free => 3,
            _ if free => 2,
            k => (k % 2) as usize,
        };
        let pick = next() % 16;
        let arg = next();
        // A snap needs sound: on an empty free loop, Capture instead.
        let pick = if free && pick > 12 && a.status(n).phase == Phase::Empty {
            12
        } else {
            pick
        };
        // A press; a snap planned gives the frame it lands on, and the
        // round runs to it.
        let press = |r: &mut Rig| -> Option<u64> {
            r.select(n);
            let c = match pick {
                0 | 1 => L::Record,
                2 | 3 => L::Overdub,
                4 => L::Undo,
                5 => L::Redo,
                6 => L::Edit(E::Half),
                7 => L::Edit(E::Double),
                8 => L::Edit(E::Reverse),
                9 => L::Edit(E::Nudge((arg % 9) as i32 - 4)),
                10 if arg % 3 == 0 => L::Forget,
                10 => L::Clear,
                11 => L::SetBars([None, Some(1), Some(2)][(arg % 3) as usize]),
                12 => L::Capture((arg % 2 + 1) as u32),
                13 if !free => L::Mute(arg % 2 == 0),
                13..=15 => {
                    let s = [Snap::Bars(1), Snap::Bars(2), Snap::Bars(4), Snap::Undo]
                        [(arg % 4) as usize];
                    let p = r.l.snap_plan(n, s, r.frame).ok()?;
                    let at = p.at;
                    r.l.snap_at(p);
                    return Some(at);
                }
                _ => return None,
            };
            r.cmd(c);
            None
        };
        // One input, one recording: a loop that starts recording stops
        // the others recording, so a free loop starts only while no song
        // loop records (or is armed to).
        let song_records = (0..2).any(|k| {
            matches!(
                a.status(k).phase,
                Phase::Armed | Phase::Recording | Phase::Overdubbing
            )
        });
        // A capture counts once it has made a loop.
        let captured = (free && pick == 12 && a.status(n).phase == Phase::Empty).then_some(n);
        let mut lands = None;
        match what {
            0..=9 => {
                lands = press(&mut a);
                assert_eq!(press(&mut b), lands);
            }
            10..=17 if song_records && pick < 4 => {}
            10..=17 => lands = press(&mut a),
            18..=21 => {
                let c = match what {
                    // STOP half as often as PLAY: a second STOP with the
                    // song stopped stops the free loops too.
                    18 => T::Play,
                    19 if arg % 2 == 0 => T::Play,
                    19 => T::Stop,
                    20 => T::Locate((arg % 24) as i64 * BAR_TICKS / 4),
                    _ => T::TempoNow(100.0 + (arg % 150) as f64),
                };
                let f = a.frame;
                let (x, y) = (a.t.command(f, c), b.t.command(f, c));
                assert_eq!(x, y);
            }
            22 if arg % 3 == 0 => {
                a.cmd(L::FreeStop);
                b.cmd(L::FreeStop);
            }
            _ => {
                a.cmd(L::FreePlay);
                b.cmd(L::FreePlay);
            }
        }
        let to = (a.frame + 300 + next() % 3_000).max(lands.map_or(0, |at| at + 1));
        a.run(to, &input);
        b.run(to, &input);
        if let Some(d) = a.l.take_snap() {
            free_snaps += usize::from(d.landed && d.plan.n >= 2);
        }
        let _ = b.l.take_snap();
        for n in 0..4 {
            let s = a.status(n);
            for k in 0..s.layers {
                let w = a.l.loop_layer(n, k).unwrap();
                assert_eq!(w.len(), 2 * s.frames, "loop {n} layer {k} in round {round}");
            }
            if n >= 2 {
                assert_eq!(s.clock, Clock::Free);
                if matches!(s.phase, Phase::Playing | Phase::Overdubbing) {
                    assert!(a.l.free_clock().is_some(), "loop {n} plays, round {round}");
                    free_played += 1;
                }
                free_captures += usize::from(captured == Some(n) && s.phase != Phase::Empty);
            } else {
                song_played += usize::from(s.phase == Phase::Playing);
                same(&a, &b, n, round, true);
            }
        }
        let m = a.l.memory();
        assert!(
            m.used <= m.capacity / 2,
            "memory {} of {}",
            m.used,
            m.capacity
        );
        assert_eq!(m.used, (0..8).map(|n| a.status(n).used).sum::<usize>());
        assert_eq!(first_difference(&a.out, &b.out), None, "round {round}");
    }
    for n in 0..2 {
        same(&a, &b, n, 1_200, true);
    }
    assert!(a.out.iter().all(|v| v.is_finite() && v.abs() < 8.0));
    assert!(
        free_played > 1_000 && song_played > 300 && free_snaps >= 4 && free_captures >= 4,
        "free loops played {free_played}, song loops {song_played}, free snaps {free_snaps}, \
         captures {free_captures}"
    );
}
