//! Tempo from the first loop, against a real transport. With the song
//! stopped and every other loop empty, a loop's first pass records free
//! (no bars yet) from Record, or from your note with When I play. Record
//! again closes it on the next period's first frame, and its length sets
//! the tempo: the bars Length set, or on Free length the bars the reading
//! picks (`read_bars`), from the start of the bar the song stopped in.
//! The host starts the song there, on the frame the loop comes round, at
//! that tempo.
//!
//! 22.05 kHz in 4/4. The song is at 100 BPM before; a bar at 300 BPM is
//! 17 640 frames (0.8 s), at 160 BPM 33 075 (1.5 s), at 80 BPM 66 150
//! (3 s).

use mpfree_looper::{
    read_bars, After, Command as L, Looper, Phase, Reading, Starts, Sure, TempoHint, TempoKind,
    FADE, LOOP_MAX_BPM,
};
use mpfree_transport::time::PPQ;
use mpfree_transport::{Block, Command as T, Meter, State, Transport};

const RATE: u32 = 22_050;
const BAR_TICKS: i64 = 4 * PPQ as i64;

struct Rig {
    t: Transport,
    l: Looper,
    b: Block,
    period: usize,
    frame: u64,
    /// The input sounds from this frame on (silent before).
    from: u64,
    input: Vec<f32>,
    mix: Vec<f32>,
    out: Vec<f32>,
    /// What the looper asked for: loop, tempo and frame.
    tempo: Option<(usize, f64, u64)>,
    /// How the loop's length read.
    reading: Option<Reading>,
    /// Starts the looper asked for (When I play, stopped): none here.
    starts: usize,
}

impl Rig {
    /// Stopped at 100 BPM, tempo from the loop on, loop 1 on Free length.
    fn new(period: usize) -> Rig {
        let mut r = Rig {
            t: Transport::new(RATE, 100.0).unwrap(),
            l: Looper::new(RATE, 30),
            b: Block::new(),
            period,
            frame: 0,
            from: 0,
            input: vec![0.0; 2 * period],
            mix: vec![0.0; 2 * period],
            out: Vec::new(),
            tempo: None,
            reading: None,
            starts: 0,
        };
        r.cmd(L::TempoFromLoop(true));
        r.cmd(L::SetBars(None));
        r
    }

    fn cmd(&mut self, c: L) {
        assert!(self.l.command(c));
    }

    fn input_at(&self, f: u64) -> [f32; 2] {
        if f >= self.from {
            sound(f)
        } else {
            [0.0; 2]
        }
    }

    /// Runs to frame `to`, a period at a time, doing what the host does
    /// when the looper hands it a tempo: on that frame, back to the start
    /// of the bar the song stopped in, the tempo, then PLAY.
    fn run(&mut self, to: u64) {
        while self.frame < to {
            let n = (self.period as u64).min(to - self.frame) as usize;
            for i in 0..n {
                let x = self.input_at(self.frame + i as u64);
                self.input[2 * i] = x[0];
                self.input[2 * i + 1] = x[1];
            }
            self.mix[..2 * n].fill(0.0);
            assert!(self.t.process(n as u32, &mut self.b));
            self.l
                .process(&self.b, &self.input[..2 * n], &mut self.mix[..2 * n]);
            self.out.extend_from_slice(&self.mix[..2 * n]);
            self.frame += n as u64;
            if let Some((k, reading, at)) = self.l.take_tempo() {
                let bpm = reading.bpm;
                self.reading = Some(reading);
                assert_eq!(at, self.frame, "the next period's first frame");
                assert_eq!(self.t.state(), State::Stopped);
                let pos = self.t.position();
                let start = pos - pos.rem_euclid(self.t.meter().bar_ticks());
                if start != pos {
                    assert!(self.t.command(at, T::Locate(start)));
                }
                assert!(self.t.command(at, T::SetTempo(bpm)));
                assert!(self.t.command(at, T::Play));
                self.tempo = Some((k, bpm, at));
            }
            if self.l.take_start().is_some() {
                self.starts += 1;
            }
        }
    }

    /// Runs a period at a time until loop 1 plays or overdubs (at most
    /// `most` more).
    fn until_playing(&mut self, most: u64) {
        let to = self.frame + most;
        while !matches!(self.l.status().phase, Phase::Playing | Phase::Overdubbing) {
            assert!(self.frame < to, "still {:?}", self.l.status());
            let f = self.frame + self.period as u64;
            self.run(f);
        }
    }

    fn layer(&self) -> Vec<[f32; 2]> {
        let v = self.l.loop_layer(0, 0).expect("a layer");
        let mut out = vec![[0.0; 2]; v.len() / 2];
        v.read(0, &mut out);
        out
    }

    fn out_at(&self, f: u64) -> [f32; 2] {
        [self.out[2 * f as usize], self.out[2 * f as usize + 1]]
    }
}

/// Loud and a different value on every frame, so where a frame came from
/// shows.
fn sound(f: u64) -> [f32; 2] {
    let x = 0.1 + 0.4 * ((f % 100_003) as f32 / 100_003.0);
    [x, -x]
}

fn close(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5
}

/// The old rule: the fewest bars (1, 2, 4 ...) at 80 BPM or more.
fn old_window(len: usize, bar: i64) -> (u32, f64) {
    let quarters = bar as f64 / PPQ as f64;
    let tempo = |bars: u32| 60.0 * f64::from(RATE) * quarters * f64::from(bars) / len as f64;
    let mut bars = 1;
    while tempo(bars) < 80.0 {
        bars *= 2;
    }
    (bars, tempo(bars))
}

#[test]
fn with_nothing_else_to_go_on_the_reading_picks_what_80_to_160_did() {
    let read = |len: usize, bar: i64| read_bars(len, RATE, bar, None, None);
    // 4/4: a bar at 120 BPM is 2 s.
    let r = read(44_100, BAR_TICKS).unwrap();
    assert_eq!(
        (r.bars, r.bpm, r.sure, r.other),
        (1, 120.0, Sure::Clear, None)
    );
    // The shortest is a bar at 300 BPM (it was a bar at 160).
    assert_eq!(
        read(17_640, BAR_TICKS).map(|r| (r.bars, r.bpm)),
        Some((1, 300.0))
    );
    assert_eq!(read(17_639, BAR_TICKS), None);
    assert_eq!(read(33_075, BAR_TICKS).map(|r| r.bpm), Some(160.0));
    // One bar at 80 exactly: as before, but it says it's a close call.
    let r = read(66_150, BAR_TICKS).unwrap();
    assert_eq!((r.bars, r.bpm, r.sure), (1, 80.0, Sure::Ambiguous));
    assert_eq!(r.other, Some((2, 160.0)));
    // A frame longer: 2 bars just under 160, the same close call.
    let r = read(66_151, BAR_TICKS).unwrap();
    assert_eq!((r.bars, r.sure), (2, Sure::Ambiguous));
    assert!(r.bpm < 160.0 && r.bpm > 159.99, "{}", r.bpm);
    assert_eq!(r.other.map(|o| o.0), Some(1));
    // 7 s: 4 bars at 137.14, clear.
    let r = read(154_350, BAR_TICKS).unwrap();
    assert_eq!((r.bars, r.sure), (4, Sure::Clear));
    assert!((r.bpm - 960.0 / 7.0).abs() < 1e-9, "{}", r.bpm);
    // 6/8 (3 quarter notes a bar): 1.5 s is a bar at 120.
    let r = read(33_075, 3 * PPQ as i64).unwrap();
    assert_eq!((r.bars, r.bpm), (1, 120.0));
    // Nothing recorded, or no bar: none.
    assert_eq!(read(0, BAR_TICKS), None);
    assert_eq!(read(44_100, 0), None);
    // Every length the old rule took reads as it did, with bars making
    // the length exactly; clear well inside the old window, a close call
    // near its edges.
    for bar in [BAR_TICKS, 3 * PPQ as i64, 7 * PPQ as i64 / 2] {
        let shortest = (bar as f64 / PPQ as f64 * 60.0 * f64::from(RATE) / 160.0).ceil() as usize;
        for len in (shortest..2_000_000).step_by(9_973) {
            let r = read(len, bar).unwrap();
            assert_eq!(
                (r.bars, r.bpm),
                old_window(len, bar),
                "{len} frames, bar {bar}"
            );
            assert!(r.bars.is_power_of_two());
            let quarters = bar as f64 / PPQ as f64;
            let frames = f64::from(r.bars) * quarters * 60.0 * f64::from(RATE) / r.bpm;
            assert!((frames - len as f64).abs() < 1e-6, "{len}: {frames}");
            if r.bpm > 88.0 && r.bpm < 146.0 {
                assert_eq!(r.sure, Sure::Clear, "{len}: {}", r.bpm);
            }
            if r.sure == Sure::Clear {
                // (Or the only reading: double would be over 300.)
                assert!(
                    (87.0..147.0).contains(&r.bpm) || 2.0 * r.bpm > LOOP_MAX_BPM,
                    "{len}: {}",
                    r.bpm
                );
            } else {
                let (b, t) = r.other.unwrap();
                assert!(b == r.bars * 2 || b * 2 == r.bars, "an octave away");
                assert!((t - r.bpm * f64::from(b) / f64::from(r.bars)).abs() < 1e-9);
            }
        }
    }
}

#[test]
fn the_reading_never_leaves_the_apps_tempos() {
    let hints = [
        None,
        Some(TempoHint {
            bpm: 45.0,
            kind: TempoKind::Set,
        }),
        Some(TempoHint {
            bpm: 290.0,
            kind: TempoKind::Set,
        }),
    ];
    for hint in hints {
        for said in [None, Some(1), Some(3), Some(8)] {
            for len in (1..3_000_000).step_by(7_919) {
                let Some(r) = read_bars(len, RATE, BAR_TICKS, said, hint) else {
                    // Only too short to close.
                    let n = said.unwrap_or(1);
                    let fastest = 60.0 * f64::from(RATE) * 4.0 * f64::from(n) / len as f64;
                    assert!(fastest > LOOP_MAX_BPM, "{len} {said:?}: {fastest}");
                    continue;
                };
                assert!((30.0..=300.0).contains(&r.bpm), "{len} {said:?}: {}", r.bpm);
                if let Some(n) = said.filter(|_| !r.overruled()) {
                    assert_eq!(r.bars, n, "{len}: Length's bars kept");
                }
            }
        }
    }
}

#[test]
fn a_free_pass_sets_the_tempo_and_the_song_starts_where_it_comes_round() {
    for period in [64, 1_000, 4_096] {
        let mut r = Rig::new(period);
        r.run(10_000);
        // Record: the pass starts on the next period's first frame.
        r.cmd(L::Record);
        let rec_from = r.frame;
        r.run(rec_from + 1);
        let s = r.l.status();
        assert_eq!(
            (s.phase, s.free),
            (Phase::Recording, true),
            "period {period}"
        );
        // About 2.5 s later, Record again: it closes at the end of the
        // period that takes the press.
        r.run(rec_from + 55_000);
        r.cmd(L::Record);
        let press = r.frame;
        r.until_playing(2 * period as u64);
        let (k, bpm, at) = r.tempo.expect("a tempo");
        assert_eq!(k, 0);
        let s = r.l.status();
        let len = s.frames;
        assert_eq!(
            at,
            rec_from + len as u64,
            "comes round where the song starts"
        );
        assert!(at > press && at <= press + period as u64, "period {period}");
        assert_eq!((s.ticks, s.origin, s.free), (BAR_TICKS, 0, false));
        let read = r.reading.unwrap();
        assert_eq!(
            (read.bars, read.bpm, read.sure, read.said),
            (1, bpm, Sure::Clear, None)
        );
        assert!(
            (55_000..=55_000 + period).contains(&len),
            "period {period}: {len}"
        );
        // The loop is what was played from Record.
        let a = r.layer();
        assert_eq!(a.len(), len);
        for (j, x) in a[..len - FADE].iter().enumerate() {
            assert_eq!(*x, sound(rec_from + j as u64), "period {period}, frame {j}");
        }
        // The song plays at that tempo from bar 1, on that frame, and the
        // loop comes round with it (after its fade in), a pass later too.
        r.run(at + 2 * len as u64 + 2 * FADE as u64);
        assert_eq!(r.t.bpm(), bpm);
        assert_eq!(r.t.tempo_map().segments().len(), 1);
        for j in [FADE as u64, 5_000, len as u64 - 2 * FADE as u64] {
            assert!(
                close(r.out_at(at + j), sound(rec_from + j)),
                "period {period}, frame {j}"
            );
            assert!(
                close(r.out_at(at + len as u64 + j), sound(rec_from + j)),
                "period {period}, a pass later, frame {j}"
            );
        }
        // Its bar is its length at the new tempo, to the frame: no print
        // is wanted.
        let frames = BAR_TICKS as f64 * 60.0 * f64::from(RATE) / (bpm * f64::from(PPQ));
        assert!((frames - len as f64).abs() < 1e-6, "{frames} for {len}");
        assert_eq!(r.l.stats().aborted, 0);
        assert_eq!(r.starts, 0);
    }
}

#[test]
fn a_long_pass_takes_more_bars_from_the_bar_the_song_stopped_in() {
    let mut r = Rig::new(256);
    // Played into bar 3 at 100 BPM (a bar is 52 920 frames), stopped.
    assert!(r.t.command(0, T::Play));
    r.run(2 * 52_920 + 20_000);
    assert!(r.t.command(r.frame, T::Stop));
    r.run(r.frame + 256);
    r.cmd(L::Record);
    let rec_from = r.frame;
    // About 7 s: 4 bars at about 137 BPM.
    r.run(rec_from + 154_000);
    r.cmd(L::Record);
    r.until_playing(512);
    let (_, bpm, at) = r.tempo.unwrap();
    let s = r.l.status();
    assert_eq!((s.ticks, s.origin), (4 * BAR_TICKS, 2 * BAR_TICKS));
    assert!((136.0..138.0).contains(&bpm), "{bpm}");
    r.run(at + 1_000);
    assert_eq!(r.t.bpm(), bpm);
    // The song starts at bar 3's start, where the loop does.
    assert!(r.t.position() >= 2 * BAR_TICKS);
    assert!(close(r.out_at(at + 500), sound(rec_from + 500)));
}

#[test]
fn closed_too_soon_it_waits_to_be_a_bar_at_300() {
    for period in [64, 4_096] {
        let mut r = Rig::new(period);
        r.cmd(L::Record);
        r.run(1);
        let rec_from = 0;
        // 0.4 s in: shorter than a bar at 300 BPM.
        r.run(8_820);
        r.cmd(L::Record);
        r.run(13_000);
        let s = r.l.status();
        assert_eq!((s.phase, s.ending, s.free), (Phase::Recording, true, true));
        assert!(r.tempo.is_none());
        r.until_playing(17_640);
        let (_, bpm, at) = r.tempo.unwrap();
        let len = (at - rec_from) as usize;
        assert!(
            (17_640..17_640 + period).contains(&len),
            "period {period}: {len}"
        );
        assert!(bpm <= 300.0 && bpm > 240.0, "{bpm}");
    }
}

#[test]
fn the_song_starting_under_a_free_pass_throws_it_away() {
    for start in [T::Play, T::Record] {
        let mut r = Rig::new(256);
        r.cmd(L::Record);
        r.run(20_000);
        assert_eq!(r.l.status().phase, Phase::Recording);
        if start == T::Record {
            assert!(r.t.command(r.frame, T::SetCountIn(1)));
        }
        assert!(r.t.command(r.frame, start));
        r.run(r.frame + 512);
        let s = r.l.status();
        assert_eq!((s.phase, s.free), (Phase::Empty, false), "{start:?}");
        assert_eq!(r.l.stats().aborted, 1, "counted");
        assert!(r.tempo.is_none());
    }
}

#[test]
fn not_free_while_off_or_another_loop_in_use() {
    // Off: armed while stopped, it waits for PLAY as it always has.
    let mut r = Rig::new(256);
    r.cmd(L::TempoFromLoop(false));
    r.cmd(L::Record);
    r.run(30_000);
    assert_eq!(r.l.status().phase, Phase::Armed);
    // Off with bars set: the same.
    let mut r = Rig::new(256);
    r.cmd(L::TempoFromLoop(false));
    r.cmd(L::SetBars(Some(2)));
    r.cmd(L::Record);
    r.run(30_000);
    assert_eq!(r.l.status().phase, Phase::Armed);
    // Another loop has sound: the song's tempo is already the loops'.
    let mut r = Rig::new(256);
    r.cmd(L::Record);
    r.run(20_000);
    r.cmd(L::Record);
    r.until_playing(40_000);
    let bpm = r.tempo.take().unwrap().1;
    assert!(r.t.command(r.frame, T::Stop));
    r.run(r.frame + 512);
    assert!(r.l.select_loop(1));
    r.cmd(L::Record);
    r.run(r.frame + 30_000);
    let s = r.l.loop_status(1).unwrap();
    assert_eq!((s.phase, s.free), (Phase::Armed, false));
    assert!(r.tempo.is_none());
    assert_eq!(r.t.bpm(), bpm);
}

#[test]
fn with_length_set_the_bars_are_at_the_tempo_you_play() {
    // 2 bars in about 3.6 s: 2 bars at about 133 BPM, not the old
    // tempo's 2 bars, and nothing read.
    let mut r = Rig::new(256);
    r.cmd(L::SetBars(Some(2)));
    r.run(256);
    let s = r.l.status();
    assert_eq!(
        (s.phase, s.free),
        (Phase::Empty, true),
        "Record would record free"
    );
    r.cmd(L::Record);
    r.run(512);
    assert_eq!(r.l.status().phase, Phase::Recording);
    r.run(80_256);
    r.cmd(L::Record);
    r.until_playing(512);
    let (_, bpm, at) = r.tempo.unwrap();
    let reading = r.reading.unwrap();
    assert_eq!(
        (reading.bars, reading.said, reading.sure),
        (2, Some(2), Sure::Clear)
    );
    let s = r.l.status();
    assert_eq!(s.ticks, 2 * BAR_TICKS);
    let want = 2.0 * 4.0 * 60.0 * f64::from(RATE) / s.frames as f64;
    assert!((bpm - want).abs() < 1e-9, "{bpm} for {} frames", s.frames);
    assert!((130.0..135.0).contains(&bpm), "{bpm}");
    r.run(at + 1_000);
    assert_eq!(r.t.bpm(), bpm);
    // 4 bars closed at 1.5 s would be 640 BPM: it waits to be 4 bars at
    // 300 (70 560 frames).
    let mut r = Rig::new(256);
    r.cmd(L::SetBars(Some(4)));
    r.cmd(L::Record);
    r.run(33_075);
    r.cmd(L::Record);
    r.run(60_000);
    assert_eq!(r.l.status().phase, Phase::Recording);
    r.until_playing(70_560);
    let (_, bpm, _) = r.tempo.unwrap();
    assert_eq!(r.reading.unwrap().bars, 4);
    assert!(bpm <= 300.0 && bpm > 295.0, "{bpm}");
}

#[test]
fn a_tempo_set_by_hand_settles_a_close_call() {
    // 3.04 s: 1 bar at 79 or 2 at 158. The song was at 80: left from
    // before, it leans the close call to 79; set by hand, it settles it.
    let len = 67_000;
    for meant in [false, true] {
        let mut r = Rig::new(256);
        assert!(r.t.command(0, T::SetTempo(80.0)));
        r.cmd(L::TempoMeant(meant));
        r.run(256);
        r.cmd(L::Record);
        r.run(256 + len);
        r.cmd(L::Record);
        r.until_playing(512);
        let reading = r.reading.unwrap();
        assert_eq!(reading.bars, 1, "set by hand: {meant}");
        let sure = if meant { Sure::Clear } else { Sure::Ambiguous };
        assert_eq!(reading.sure, sure, "set by hand: {meant}");
    }
}

#[test]
fn when_i_play_it_records_free_from_your_note() {
    for (period, latency) in [(64usize, 0u32), (1_000, 300), (4_096, 0)] {
        let mut r = Rig::new(period);
        r.cmd(L::SetStarts(Starts::Play));
        r.cmd(L::StartsByEar(true));
        r.cmd(L::SetLatency(latency));
        r.from = u64::MAX;
        r.cmd(L::Record);
        r.run(8_000);
        let s = r.l.status();
        assert_eq!((s.phase, s.listening, s.free), (Phase::Armed, true, true));
        // Played on 9 000, heard `latency` later.
        let note = 9_000;
        r.from = note + u64::from(latency);
        r.run(note + 60_000);
        assert_eq!(r.l.status().phase, Phase::Recording, "period {period}");
        r.cmd(L::Record);
        r.until_playing(2 * period as u64);
        let (_, _, at) = r.tempo.unwrap();
        let s = r.l.status();
        assert_eq!(at, note + s.frames as u64, "period {period}");
        // Its first frame is the note; the song didn't start on it. (The
        // last `latency` frames come in after it closes.)
        r.run(at + u64::from(latency) + 2 * period as u64);
        let a = r.layer();
        for (j, x) in a[..a.len() - FADE].iter().enumerate() {
            let want = r.input_at(note + u64::from(latency) + j as u64);
            assert_eq!(*x, want, "period {period}, frame {j}");
        }
        assert_eq!(a[0], sound(r.from));
        assert_eq!(r.starts, 0, "no start on the note");
    }
}

#[test]
fn a_pad_starts_a_free_pass_on_its_frame() {
    let mut r = Rig::new(256);
    r.cmd(L::SetStarts(Starts::Play));
    r.cmd(L::Record);
    r.run(5_120);
    // A pad on the coming period's first frame (the host reads the pads
    // before the transport runs).
    r.cmd(L::Played(5_120));
    r.run(5_120 + 40_000);
    let s = r.l.status();
    assert_eq!((s.phase, s.free), (Phase::Recording, true));
    r.cmd(L::Record);
    r.until_playing(512);
    let (_, _, at) = r.tempo.unwrap();
    assert_eq!(at, 5_120 + r.l.status().frames as u64);
    assert_eq!(r.layer()[0], sound(5_120));
}

#[test]
fn then_overdub_overdubs_from_where_the_song_starts() {
    let mut r = Rig::new(256);
    r.cmd(L::SetAfter(After::Overdub));
    r.cmd(L::Record);
    r.run(40_000);
    r.cmd(L::Record);
    r.until_playing(512);
    let (_, _, at) = r.tempo.unwrap();
    r.run(at + 2_000);
    let s = r.l.status();
    assert_eq!(s.phase, Phase::Overdubbing);
    assert_eq!(r.t.state(), State::Playing);
}

#[test]
fn a_meter_of_three_quarters_sets_its_tempo_in_quarters() {
    let mut r = Rig::new(256);
    let meter = Meter::new(6, 8).unwrap();
    r.t.set_meter(meter).unwrap();
    r.cmd(L::Record);
    // About 3 s: 2 bars of 6/8 at 120 BPM (a bar is 1.5 s).
    r.run(66_000);
    r.cmd(L::Record);
    r.until_playing(512);
    let (_, bpm, _) = r.tempo.unwrap();
    let s = r.l.status();
    assert_eq!(s.ticks, 2 * meter.bar_ticks());
    let reading = r.reading.unwrap();
    let want = 60.0 * f64::from(RATE) * 3.0 * 2.0 / s.frames as f64;
    assert_eq!((reading.bars, bpm), (2, want));
    assert!((119.0..121.0).contains(&bpm), "{bpm}");
}
