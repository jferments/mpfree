//! Starts: When I play, against a real transport. An armed loop waits for
//! the first note: by ear (the line input over PLAY_LEVEL) or told
//! (`Played`, a pad hit). With the song playing it starts on the bar line
//! nearest the note: a note in a bar's first half starts it on that bar's
//! line, what came since copied in from what was heard; a note in the
//! second half is a pickup into the next line, played at the loop's end
//! each time round. With the song stopped the looper asks the host to
//! start the song, and bar 1 starts on the note. Counting in, a note
//! leads into bar 1. The period size changes nothing.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s).

use mpfree_looper::{Command as L, Looper, Phase, Starts, FADE};
use mpfree_transport::{Block, Command as T, State, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050;
const BAR_TICKS: i64 = 3_840;

struct Rig {
    t: Transport,
    l: Looper,
    b: Block,
    period: usize,
    frame: u64,
    /// The input sounds from this frame on (silent before)...
    from: u64,
    /// ...and, if set, falls silent again here.
    until: Option<u64>,
    input: Vec<f32>,
    mix: Vec<f32>,
    out: Vec<f32>,
    /// Where the song was started on the looper's asking.
    started: Option<u64>,
}

impl Rig {
    fn new(period: usize) -> Rig {
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            l: Looper::new(RATE, 20),
            b: Block::new(),
            period,
            frame: 0,
            from: u64::MAX,
            until: None,
            input: vec![0.0; 2 * period],
            mix: vec![0.0; 2 * period],
            out: Vec::new(),
            started: None,
        }
    }

    /// Armed to start when you play: `by_ear` (the input) or not (pads).
    fn armed(period: usize, by_ear: bool) -> Rig {
        let mut r = Rig::new(period);
        r.cmd(L::SetStarts(Starts::Play));
        r.cmd(L::StartsByEar(by_ear));
        r.cmd(L::SetBars(Some(1)));
        r.cmd(L::Record);
        r
    }

    fn cmd(&mut self, c: L) {
        assert!(self.l.command(c));
    }

    fn input_at(&self, f: u64) -> [f32; 2] {
        if f >= self.from && self.until.is_none_or(|u| f < u) {
            sound(f)
        } else {
            [0.0; 2]
        }
    }

    /// Runs to frame `to`, a period at a time, starting the song when the
    /// looper asks (as the host does: on the next period's first frame).
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
            if self.l.take_start().is_some() {
                assert_eq!(self.t.state(), State::Stopped);
                assert!(self.t.command(self.frame, T::Play));
                self.started = Some(self.frame);
            }
        }
    }

    /// Runs until loop 0 plays (at most `most` frames more).
    fn until_playing(&mut self, most: u64) {
        let to = self.frame + most;
        while self.l.status().phase != Phase::Playing {
            assert!(self.frame < to, "still {:?}", self.l.status());
            let f = self.frame + 64;
            self.run(f);
        }
    }

    /// Loop 0's layer, as it plays (from its first bar line).
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

/// What the looper hears once you play: loud (over PLAY_LEVEL) and a
/// different value on every frame, so where a frame came from shows.
fn sound(f: u64) -> [f32; 2] {
    let x = 0.1 + 0.4 * ((f % 100_003) as f32 / 100_003.0);
    [x, -x]
}

fn close(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

/// Playing from bar 1, loop 0 armed to start when you play by ear, the
/// input sounding from `note`: runs until the loop plays.
fn played_by_ear(period: usize, note: u64) -> Rig {
    let mut r = Rig::armed(period, true);
    r.from = note;
    assert!(r.t.command(0, T::Play));
    r.run(note.saturating_sub(BAR / 2));
    let s = r.l.status();
    assert_eq!(
        (s.phase, s.starts, s.listening),
        (Phase::Armed, Starts::Play, true)
    );
    r.until_playing(3 * BAR);
    r
}

#[test]
fn a_note_early_in_a_bar_starts_the_loop_on_that_bar_line() {
    for period in [64, 1_000, 4_096] {
        // Bar 3, 3 000 frames in: the loop starts on bar 3's line, and the
        // frames since then come in from what was heard.
        let note = 2 * BAR + 3_000;
        let r = played_by_ear(period, note);
        let s = r.l.status();
        assert_eq!(
            (s.frames, s.ticks, s.origin, s.lead),
            (BAR as usize, BAR_TICKS, 2 * BAR_TICKS, 0),
            "period {period}"
        );
        let a = r.layer();
        for (j, x) in a[..a.len() - FADE].iter().enumerate() {
            let want = r.input_at(2 * BAR + j as u64);
            assert_eq!(*x, want, "period {period}, frame {j}");
        }
        assert_eq!(a[3_000], sound(note), "the note, in its place");
        assert_eq!(a[2_999], [0.0; 2], "silence before it");
        assert_eq!(r.l.stats().aborted, 0);
    }
}

/// The pickup's checks: the loop starts on the line after `note` (song
/// frame `line`, tick `origin`), `lead` frames later, and the pickup
/// plays at the loop's end.
fn check_pickup(r: &Rig, note: u64, (line, origin): (u64, i64), period: usize) {
    let lead = (line - note) as usize;
    let s = r.l.status();
    assert_eq!(
        (s.frames, s.ticks, s.origin, s.lead),
        (BAR as usize, BAR_TICKS, origin, lead),
        "period {period}"
    );
    let a = r.layer();
    let len = a.len();
    // From the line on, as heard, up to the loop point's fade...
    for (x, got) in a[..len - lead - FADE].iter().enumerate() {
        assert_eq!(*got, sound(line + x as u64), "period {period}, frame {x}");
    }
    // ...then the pickup, leading into the first bar again.
    for (y, got) in a[len - lead..].iter().enumerate() {
        assert_eq!(*got, sound(note + y as u64), "period {period}, pickup {y}");
    }
}

#[test]
fn a_note_late_in_a_bar_is_a_pickup_played_at_the_loops_end() {
    for period in [64, 1_000, 4_096] {
        // Bar 3, 15 000 frames in (past half way): the loop starts on bar
        // 4's line, and the 7 050 frames before it lead into it.
        let note = 2 * BAR + 15_000;
        let mut r = played_by_ear(period, note);
        check_pickup(&r, note, (3 * BAR, 3 * BAR_TICKS), period);
        // Round again, the song plays the pickup before each bar line:
        // song frame 6 BAR - 7 050 is the note.
        let lead = 3 * BAR - note;
        r.run(6 * BAR + 64);
        for y in 0..lead {
            let f = 6 * BAR - lead + y;
            assert!(
                close(r.out_at(f), sound(note + y)),
                "period {period}, frame {f}"
            );
        }
        assert!(close(r.out_at(6 * BAR), sound(3 * BAR)), "then bar 1");
    }
}

#[test]
fn a_pickup_just_before_the_line_heard_after_it_still_leads_in() {
    // 100 frames before bar 4: a long period hears the note and passes
    // the line in the same period, so the pass starts after the line,
    // from what was heard. The loop is the same at any period size.
    let note = 3 * BAR - 100;
    for period in [64, 4_096] {
        let r = played_by_ear(period, note);
        check_pickup(&r, note, (3 * BAR, 3 * BAR_TICKS), period);
    }
}

#[test]
fn with_the_song_stopped_the_first_note_starts_it_on_bar_one() {
    for period in [64, 1_000] {
        for latency in [0u32, 300] {
            let mut r = Rig::armed(period, true);
            r.cmd(L::SetLatency(latency));
            // The note comes in `latency` late: it was played at 5 000.
            let note = 5_000;
            r.from = note + u64::from(latency);
            r.run(4_000);
            assert!(r.l.status().listening);
            r.until_playing(3 * BAR);
            let started = r.started.expect("the song started");
            assert!(
                started > note && started <= note + u64::from(latency) + 3 * period as u64,
                "period {period}: started on {started}"
            );
            let s = r.l.status();
            assert_eq!(
                (s.frames, s.origin, s.lead),
                (BAR as usize, 0, 0),
                "period {period}"
            );
            // Bar 1 starts on the note: the loop's first frame is it.
            let a = r.layer();
            for (j, x) in a[..a.len() - FADE].iter().enumerate() {
                let want = sound(r.from + j as u64);
                assert_eq!(*x, want, "period {period}, latency {latency}, frame {j}");
            }
        }
    }
}

#[test]
fn pads_start_it_on_the_note_not_on_the_sound() {
    for period in [64, 1_000] {
        // The pads' mix has the song in it: loud from the start, and that
        // starts nothing.
        let mut r = Rig::armed(period, false);
        r.from = 0;
        assert!(r.t.command(0, T::Play));
        let note = 2 * BAR + 3_000;
        r.run(note);
        let s = r.l.status();
        assert_eq!(
            (s.phase, s.listening),
            (Phase::Armed, true),
            "period {period}"
        );
        // A pad hit on `note`, told before the period that plays it.
        r.cmd(L::Played(note));
        r.until_playing(2 * BAR);
        let s = r.l.status();
        assert_eq!(
            (s.frames, s.origin, s.lead),
            (BAR as usize, 2 * BAR_TICKS, 0),
            "period {period}"
        );
        let a = r.layer();
        for (j, x) in a[..a.len() - FADE].iter().enumerate() {
            assert_eq!(*x, sound(2 * BAR + j as u64), "period {period}, frame {j}");
        }
    }
}

#[test]
fn a_pad_with_the_song_stopped_starts_it_on_that_frame() {
    // The host starts the song on the pad's frame itself (it sees the pad
    // before the period): bar 1 is the note, and nothing is asked.
    let mut r = Rig::armed(64, false);
    r.from = 0;
    r.run(5_000);
    r.cmd(L::Played(5_000));
    assert!(r.t.command(5_000, T::Play));
    r.until_playing(2 * BAR);
    assert_eq!(r.started, None);
    let s = r.l.status();
    assert_eq!((s.frames, s.origin, s.lead), (BAR as usize, 0, 0));
    let a = r.layer();
    for (j, x) in a[..a.len() - FADE].iter().enumerate() {
        assert_eq!(*x, sound(5_000 + j as u64), "frame {j}");
    }
}

#[test]
fn a_note_counting_in_leads_into_bar_one() {
    for period in [64, 1_000] {
        let mut r = Rig::armed(period, true);
        assert!(r.t.command(0, T::SetCountIn(1)));
        assert!(r.t.command(0, T::Record));
        // A bar of count-in: bar 1 starts on frame BAR. The note comes
        // 4 000 frames before it.
        let note = BAR - 4_000;
        r.from = note;
        r.until_playing(3 * BAR);
        check_pickup(&r, note, (BAR, 0), period);
    }
}

#[test]
fn a_note_too_long_before_bar_one_is_forgotten() {
    // Counting in, a short note 15 000 frames before bar 1 (more than
    // half a bar): no pickup. The loop listens again from bar 1.
    let mut r = Rig::armed(64, true);
    assert!(r.t.command(0, T::SetCountIn(1)));
    assert!(r.t.command(0, T::Record));
    r.from = BAR - 15_000;
    r.until = Some(BAR - 13_000);
    r.run(BAR + 2_000);
    let s = r.l.status();
    assert_eq!((s.phase, s.listening), (Phase::Armed, true));
    // The next note starts it, on the bar line nearest.
    r.from = BAR + 3_000;
    r.until = None;
    r.until_playing(3 * BAR);
    let s = r.l.status();
    assert_eq!((s.origin, s.lead), (0, 0));
}

#[test]
fn a_refused_start_listens_again() {
    let mut r = Rig::armed(64, true);
    r.from = 5_000;
    r.until = Some(5_100);
    // Process by hand: the song can't start (the host refuses).
    let mut b = Block::new();
    let mut input = vec![0.0f32; 128];
    let mut mix = vec![0.0f32; 128];
    let mut asked = false;
    while r.frame < 6_000 {
        for i in 0..64 {
            let x = r.input_at(r.frame + i as u64);
            input[2 * i] = x[0];
            input[2 * i + 1] = x[1];
        }
        assert!(r.t.process(64, &mut b));
        r.l.process(&b, &input, &mut mix);
        r.frame += 64;
        if r.l.take_start().is_some() {
            asked = true;
            r.l.start_refused();
        }
    }
    assert!(asked);
    let s = r.l.status();
    assert_eq!((s.phase, s.listening), (Phase::Armed, true));
    assert_eq!(r.l.take_start(), None);
}

#[test]
fn on_the_bar_by_default() {
    // Starts: On the bar, as before: a note changes nothing, the first
    // pass starts on the next bar line.
    let mut r = Rig::new(64);
    assert_eq!(r.l.status().starts, Starts::Bar);
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    r.from = 3_000;
    r.run(10_000);
    assert!(!r.l.status().listening);
    assert_eq!(r.l.take_start(), None);
    assert!(r.t.command(r.frame, T::Play));
    r.until_playing(3 * BAR);
    let s = r.l.status();
    assert_eq!((s.origin, s.lead), (0, 0));
    assert_eq!(r.layer()[0], sound(10_000));
}

#[test]
fn disarming_or_on_the_bar_forgets_the_note() {
    let mut r = Rig::armed(64, true);
    r.from = 5_000;
    r.run(5_056);
    // Heard (not placed yet), but the loop is disarmed first.
    r.cmd(L::Record);
    r.run(6_000);
    assert_eq!(r.started, None);
    assert_eq!(r.l.status().phase, Phase::Empty);
    // Armed again, it listens afresh; On the bar, it stops listening.
    r.cmd(L::Record);
    r.cmd(L::SetStarts(Starts::Bar));
    r.run(7_000);
    let s = r.l.status();
    assert_eq!(
        (s.phase, s.listening, s.starts),
        (Phase::Armed, false, Starts::Bar)
    );
    assert_eq!(r.started, None);
}
