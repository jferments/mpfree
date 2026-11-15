//! Freeze and print: a take from a capture point is the strip's signal
//! frame for frame (lined up for latency, the same at any block size), a
//! steady take waits a loop, tails and failures, and a frozen strip sounds
//! like the live one while its instrument and inserts stand still.

use mpfree_fx::ids::delay;
use mpfree_fx::mpfree_node::{Ctx, Kind, Node, NodeInfo, ParamInfo, Timed, MAX_BLOCK};
use mpfree_fx::*;

const RATE: u32 = 48_000;
/// Frames in a beat at 120 BPM.
const BEAT: u64 = 24_000;

/// A pure delay that reports its latency.
struct Lag {
    n: usize,
    buf: [Vec<f32>; 2],
    w: usize,
}

static LAG: NodeInfo = NodeInfo {
    id: "test.lag",
    name: "Lag",
    kind: Kind::Effect,
};

impl Node for Lag {
    fn info(&self) -> &'static NodeInfo {
        &LAG
    }
    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }
    fn set_param(&mut self, _id: u16, _value: f32) {}
    fn param(&self, _id: u16) -> f32 {
        f32::NAN
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _ctx: &Ctx, _events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let size = self.n + 1;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.buf[0][self.w] = *l;
            self.buf[1][self.w] = *r;
            let rd = (self.w + size - self.n) % size;
            *l = self.buf[0][rd];
            *r = self.buf[1][rd];
            self.w = (self.w + 1) % size;
        }
    }
    fn latency(&self) -> u32 {
        self.n as u32
    }
}

/// The instrument: a decaying 440 Hz burst on every beat, a quieter one
/// on the off-beat, the right side a little different. It depends only on
/// the song frame, so every loop of whole beats repeats it.
fn sig(song: u64) -> (f32, f32) {
    let t = (song % BEAT) as f32 / RATE as f32;
    let h = ((song + BEAT / 2) % BEAT) as f32 / RATE as f32;
    let a = (std::f32::consts::TAU * 440.0 * t).sin() * (-t / 0.04).exp() * 0.5;
    let b = (std::f32::consts::TAU * 660.0 * h).sin() * (-h / 0.02).exp() * 0.2;
    (a + b, a - b)
}

/// The song: a transport at a tempo, rolling or not.
struct Song {
    frame: u64,
    /// Frames rolled since bar 1.
    song: u64,
    beats: f64,
    bpm: f64,
    playing: bool,
}

impl Song {
    fn new() -> Song {
        Song {
            frame: 1_000,
            song: 0,
            beats: 0.0,
            bpm: 120.0,
            playing: true,
        }
    }
    fn ctx(&self) -> Ctx<'static> {
        Ctx {
            rate: RATE,
            frame: self.frame,
            bpm: self.bpm,
            beats: self.beats,
            playing: self.playing,
            sidechain: None,
        }
    }
}

/// A mixer with track 0 and capture point 0 on its Before fader.
fn rig(chain: Vec<Box<dyn Node>>) -> Mixer {
    let cfg = MixerConfig {
        inputs: 0,
        pads: 0,
        tracks: 2,
        slots: 4,
        max_latency_ms: 50.0,
    };
    let mut m = Mixer::new(cfg, RATE, MAX_BLOCK);
    for n in chain {
        m.chain_mut(StripId::Track(0)).unwrap().push(n);
    }
    assert!(m.set_capture(0, Some((StripId::Track(0), CapturePoint::BeforeFader))));
    m
}

struct Out {
    left: Vec<f32>,
    right: Vec<f32>,
}

/// Plays `frames` frames in blocks from `next`, feeding track 0 while the
/// mixer wants it, and running `print` (if any) after the mixer. `each`
/// runs before every block.
fn run(
    m: &mut Mixer,
    song: &mut Song,
    frames: usize,
    next: &mut dyn FnMut() -> usize,
    mut print: Option<&mut Print>,
    each: &mut dyn FnMut(&mut Mixer, &mut Song, Option<&mut Print>),
) -> Out {
    let mut out = Out {
        left: Vec::with_capacity(frames),
        right: Vec::with_capacity(frames),
    };
    let (mut l, mut r) = (vec![0.0; MAX_BLOCK], vec![0.0; MAX_BLOCK]);
    let mut done = 0;
    while done < frames {
        each(m, song, print.as_deref_mut());
        let n = next().min(frames - done).min(MAX_BLOCK);
        if m.needs_input(StripId::Track(0)) && song.playing {
            let (il, ir) = m.strip_input(StripId::Track(0)).unwrap();
            for i in 0..n {
                let (a, b) = sig(song.song + i as u64);
                il[i] += a;
                ir[i] += b;
            }
        }
        let ctx = song.ctx();
        l[..n].fill(0.0);
        r[..n].fill(0.0);
        m.process(&ctx, &mut l[..n], &mut r[..n]);
        if let Some(p) = print.as_deref_mut() {
            p.process(&ctx, m.capture(0), m.capture_latency(0).unwrap_or(0));
        }
        out.left.extend_from_slice(&l[..n]);
        out.right.extend_from_slice(&r[..n]);
        song.frame += n as u64;
        if song.playing {
            song.song += n as u64;
            song.beats += n as f64 * song.bpm / 60.0 / RATE as f64;
        }
        done += n;
    }
    out
}

fn fixed(n: usize) -> impl FnMut() -> usize {
    move || n
}

/// Block sizes from 1 to MAX_BLOCK, the same every run.
fn ragged() -> impl FnMut() -> usize {
    let mut s = 0x9e37_79b9_u64;
    move || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        1 + ((s >> 33) as usize % MAX_BLOCK)
    }
}

fn nothing() -> impl FnMut(&mut Mixer, &mut Song, Option<&mut Print>) {
    |_, _, _| {}
}

/// Records the job with `chain` on track 0, from bar 1, in blocks from
/// `next`.
fn take(chain: Vec<Box<dyn Node>>, job: PrintJob, next: &mut dyn FnMut() -> usize) -> Printed {
    let mut m = rig(chain);
    let mut song = Song::new();
    let mut p = Print::new(RATE, 20.0);
    p.arm(job).unwrap();
    let span = ((job.start + 2.0 * job.beats) * BEAT as f64) as usize;
    run(&mut m, &mut song, span, next, Some(&mut p), &mut nothing());
    assert_eq!(p.state(), PrintState::Done);
    p.take().unwrap()
}

fn job(start: f64, beats: f64) -> PrintJob {
    PrintJob {
        start,
        beats,
        steady: false,
        tail: Tail::Cut,
    }
}

#[test]
fn a_take_is_the_strip_frame_for_frame() {
    let p = take(Vec::new(), job(2.0, 4.0), &mut fixed(128));
    assert_eq!(p.frames(), 4 * BEAT as usize);
    assert_eq!(p.loop_frames, p.frames());
    assert_eq!((p.start, p.beats, p.bpm, p.rate), (2.0, 4.0, 120.0, RATE));
    for i in 0..p.frames() {
        let (a, b) = sig(2 * BEAT + i as u64);
        assert_eq!((p.left[i], p.right[i]), (a, b), "frame {i}");
    }
}

#[test]
fn a_take_lines_up_for_the_capture_points_latency() {
    let lag = || -> Vec<Box<dyn Node>> {
        vec![Box::new(Lag {
            n: 37,
            buf: [vec![0.0; 38], vec![0.0; 38]],
            w: 0,
        })]
    };
    let mut m = rig(lag());
    run(
        &mut m,
        &mut Song::new(),
        64,
        &mut fixed(64),
        None,
        &mut nothing(),
    );
    assert_eq!(m.capture_latency(0), Some(37));
    let p = take(lag(), job(1.0, 2.0), &mut fixed(64));
    for i in 0..p.frames() {
        let (a, b) = sig(BEAT + i as u64);
        assert_eq!((p.left[i], p.right[i]), (a, b), "frame {i}");
    }
    // The same take at any block size.
    let q = take(lag(), job(1.0, 2.0), &mut ragged());
    assert_eq!(p, q);
}

#[test]
fn a_steady_take_waits_for_a_whole_loop() {
    // Armed as the transport starts: the loop has not played yet, so the
    // take starts a loop later.
    let mut m = rig(Vec::new());
    let mut song = Song::new();
    let mut p = Print::new(RATE, 10.0);
    p.arm(PrintJob::freeze(0.0, 2.0)).unwrap();
    run(
        &mut m,
        &mut song,
        256,
        &mut fixed(128),
        Some(&mut p),
        &mut nothing(),
    );
    assert_eq!(p.start_beat(), Some(2.0));
    assert_eq!(p.state(), PrintState::Waiting);
    run(
        &mut m,
        &mut song,
        5 * BEAT as usize,
        &mut fixed(128),
        Some(&mut p),
        &mut nothing(),
    );
    assert_eq!(p.state(), PrintState::Done);
    assert_eq!(p.take().unwrap().start, 2.0);

    // Armed after the transport has rolled for longer than a loop: the
    // next time round is fine. A start already gone moves on by loops.
    let mut p = Print::new(RATE, 10.0);
    p.arm(PrintJob::freeze(0.0, 2.0)).unwrap();
    run(
        &mut m,
        &mut song,
        128,
        &mut fixed(128),
        Some(&mut p),
        &mut nothing(),
    );
    // The song is at beat 6 and a bit: the next loop start is beat 8.
    assert_eq!(p.start_beat(), Some(8.0));
}

#[test]
fn a_tail_runs_until_the_strip_is_silent() {
    // One burst on beat 0 (the signal of beat 0 only), the transport
    // stopping at the end of the beats: the take goes on through the
    // stop until the burst has died away.
    let mut m = rig(Vec::new());
    let mut song = Song::new();
    let mut p = Print::new(RATE, 10.0);
    p.arm(PrintJob {
        start: 0.0,
        beats: 0.5,
        steady: false,
        tail: Tail::UntilSilent { max_ms: 3000.0 },
    })
    .unwrap();
    let mut stop = |_: &mut Mixer, s: &mut Song, _: Option<&mut Print>| {
        if s.song >= BEAT / 2 {
            s.playing = false;
        }
    };
    // Only the first half beat sounds (the burst), so the tail is the
    // burst's last whisper and then quiet.
    run(
        &mut m,
        &mut song,
        3 * BEAT as usize,
        &mut fixed(100),
        Some(&mut p),
        &mut stop,
    );
    assert_eq!(p.state(), PrintState::Done);
    let t = p.take().unwrap();
    assert_eq!(t.loop_frames, (BEAT / 2) as usize);
    let quiet = (print::QUIET_MS * 0.001 * RATE as f32) as usize;
    // The input stops with the transport, so the tail is just the quiet.
    assert!(
        t.tail_frames() >= quiet && t.tail_frames() <= quiet + 100,
        "{}",
        t.tail_frames()
    );

    // Cut stops at the beats.
    let t = take(Vec::new(), job(0.0, 0.5), &mut fixed(100));
    assert_eq!(t.tail_frames(), 0);
}

#[test]
fn a_take_fails_on_a_stop_a_tempo_change_a_jump_or_no_room() {
    let fail = |each: &mut dyn FnMut(&mut Mixer, &mut Song, Option<&mut Print>), cap: f32| {
        let mut m = rig(Vec::new());
        let mut song = Song::new();
        let mut p = Print::new(RATE, cap);
        p.arm(job(1.0, 2.0)).unwrap();
        run(
            &mut m,
            &mut song,
            4 * BEAT as usize,
            &mut fixed(128),
            Some(&mut p),
            each,
        );
        assert!(p.take().is_none());
        p.state()
    };
    let at = 2 * BEAT;
    let stop = &mut |_: &mut Mixer, s: &mut Song, _: Option<&mut Print>| {
        if s.song >= at {
            s.playing = false;
        }
    };
    assert_eq!(fail(stop, 10.0), PrintState::Failed(PrintError::Stopped));
    let tempo = &mut |_: &mut Mixer, s: &mut Song, _: Option<&mut Print>| {
        if s.song >= at {
            s.bpm = 121.0;
        }
    };
    assert_eq!(
        fail(tempo, 10.0),
        PrintState::Failed(PrintError::TempoChanged)
    );
    let mut jumped = false;
    let jump = &mut |_: &mut Mixer, s: &mut Song, _: Option<&mut Print>| {
        if s.song >= at && !jumped {
            s.song = 0;
            s.beats = 0.0;
            jumped = true;
        }
    };
    assert_eq!(fail(jump, 10.0), PrintState::Failed(PrintError::Jumped));
    assert_eq!(
        fail(&mut |_, _, _| {}, 0.5),
        PrintState::Failed(PrintError::TooLong)
    );
    // Armed twice, or with no beats: refused.
    let mut p = Print::new(RATE, 1.0);
    assert_eq!(p.arm(job(0.0, 0.0)), Err(PrintError::Refused));
    p.arm(job(0.0, 1.0)).unwrap();
    assert_eq!(p.arm(job(0.0, 1.0)), Err(PrintError::Refused));
    p.cancel();
    assert_eq!(p.state(), PrintState::Idle);
    assert!(p.arm(job(0.0, 1.0)).is_ok());
}

/// A chain with an echo whose repeats run across the loop's end.
fn echo() -> Vec<Box<dyn Node>> {
    let mut d = Delay::new();
    d.set_param(delay::SYNC, 0.0);
    d.set_param(delay::TIME, 375.0);
    d.set_param(delay::FEEDBACK, 30.0);
    d.set_param(delay::MIX, 40.0);
    vec![Box::new(d)]
}

#[test]
fn a_frozen_strip_sounds_like_the_live_one() {
    let total = 14 * BEAT as usize;
    // Live, all the way.
    let mut m = rig(echo());
    let live = run(
        &mut m,
        &mut Song::new(),
        total,
        &mut fixed(128),
        None,
        &mut nothing(),
    );

    // Frozen: a steady take of beats 4 to 8 (armed at bar 1, so it waits
    // a loop), frozen as soon as it is done; the instrument stops once the
    // strip no longer needs it.
    let mut m = rig(echo());
    let mut p = Print::new(RATE, 10.0);
    p.arm(PrintJob::freeze(0.0, 4.0)).unwrap();
    let mut states = Vec::new();
    let mut freeze = |m: &mut Mixer, _: &mut Song, p: Option<&mut Print>| {
        if let Some(t) = p.and_then(|p| p.take()) {
            assert!(m.freeze(StripId::Track(0), t.frozen()).is_ok());
        }
        let s = m.freeze_state(StripId::Track(0));
        if states.last() != Some(&s) {
            states.push(s);
        }
    };
    let frozen = run(
        &mut m,
        &mut Song::new(),
        total,
        &mut ragged(),
        Some(&mut p),
        &mut freeze,
    );
    assert_eq!(
        states,
        [FreezeState::Off, FreezeState::FadingIn, FreezeState::On]
    );
    assert!(!m.needs_input(StripId::Track(0)));
    let worst = live
        .left
        .iter()
        .zip(&frozen.left)
        .chain(live.right.iter().zip(&frozen.right))
        .fold(0.0f32, |w, (a, b)| w.max((a - b).abs()));
    assert!(worst < 1e-3, "worst difference {worst}");
    // The echo's own memory is not what plays: the insert no longer runs.
    assert_eq!(m.frozen(StripId::Track(0)).unwrap().beats, 4.0);
}

#[test]
fn frozen_audio_follows_the_transport_and_thaws_click_free() {
    let mut m = rig(Vec::new());
    let mut song = Song::new();
    let mut p = Print::new(RATE, 10.0);
    p.arm(job(0.0, 2.0)).unwrap();
    run(
        &mut m,
        &mut song,
        2 * BEAT as usize + 256,
        &mut fixed(128),
        Some(&mut p),
        &mut nothing(),
    );
    let t = p.take().unwrap();
    assert!(m.freeze(StripId::Track(1), t.clone().frozen()).is_ok());
    // Not twice, and not the master.
    let f = t.clone().frozen();
    let f = m.freeze(StripId::Track(1), f).unwrap_err();
    assert!(m.freeze(StripId::Master, f).is_err());
    run(
        &mut m,
        &mut song,
        BEAT as usize,
        &mut fixed(128),
        None,
        &mut nothing(),
    );
    assert_eq!(m.freeze_state(StripId::Track(1)), FreezeState::On);

    // Stopped: the frozen audio fades out within a few ms (watched on its
    // own Before fader capture point; the master's DC blocker rings on).
    assert!(m.set_capture(1, Some((StripId::Track(1), CapturePoint::BeforeFader))));
    song.playing = false;
    run(
        &mut m,
        &mut song,
        2048,
        &mut fixed(128),
        None,
        &mut nothing(),
    );
    let (l, r) = m.capture(1).unwrap();
    assert!(l.iter().chain(r).all(|&v| v == 0.0));
    song.playing = true;

    // Thaw: the strip wants its instrument at once, crossfades back, and
    // gives the audio back.
    assert!(m.thaw(StripId::Track(1)));
    assert!(m.needs_input(StripId::Track(1)));
    assert!(!m.thaw(StripId::Track(0)));
    let mut states = Vec::new();
    let mut watch = |m: &mut Mixer, _: &mut Song, _: Option<&mut Print>| {
        let s = m.freeze_state(StripId::Track(1));
        if states.last() != Some(&s) {
            states.push(s);
        }
    };
    run(&mut m, &mut song, 4800, &mut fixed(128), None, &mut watch);
    assert_eq!(states, [FreezeState::FadingOut, FreezeState::Off]);
    let back = m.pop_thawed().unwrap();
    assert_eq!(back.frames(), t.loop_frames);
    assert!(m.pop_thawed().is_none());
}

#[test]
fn frozen_audio_held_quiet_fades_out_and_comes_back() {
    let mut m = rig(Vec::new());
    let mut song = Song::new();
    let mut p = Print::new(RATE, 10.0);
    p.arm(job(0.0, 2.0)).unwrap();
    run(
        &mut m,
        &mut song,
        2 * BEAT as usize + 256,
        &mut fixed(128),
        Some(&mut p),
        &mut nothing(),
    );
    let t = p.take().unwrap();
    assert!(
        !m.set_frozen_quiet(StripId::Track(1), true),
        "not frozen yet"
    );
    assert!(m.freeze(StripId::Track(1), t.frozen()).is_ok());
    assert!(m.set_capture(1, Some((StripId::Track(1), CapturePoint::BeforeFader))));
    let loud = |m: &Mixer| {
        let (l, r) = m.capture(1).unwrap();
        l.iter().chain(r).fold(0f32, |a, &v| a.max(v.abs()))
    };
    let mut peak = 0f32;
    let mut watch = |m: &mut Mixer, _: &mut Song, _: Option<&mut Print>| peak = peak.max(loud(m));
    run(
        &mut m,
        &mut song,
        BEAT as usize,
        &mut fixed(128),
        None,
        &mut watch,
    );
    assert!(peak > 0.1, "it plays: {peak}");
    // Held quiet (its track muted): silent within a few ms while the song
    // rolls on, and still frozen.
    assert!(m.set_frozen_quiet(StripId::Track(1), true));
    run(
        &mut m,
        &mut song,
        2048,
        &mut fixed(128),
        None,
        &mut nothing(),
    );
    assert_eq!(loud(&m), 0.0);
    assert_eq!(m.freeze_state(StripId::Track(1)), FreezeState::On);
    // Let go: it plays again, in step with the song.
    assert!(m.set_frozen_quiet(StripId::Track(1), false));
    let mut peak = 0f32;
    let mut watch = |m: &mut Mixer, _: &mut Song, _: Option<&mut Print>| peak = peak.max(loud(m));
    run(
        &mut m,
        &mut song,
        BEAT as usize,
        &mut fixed(128),
        None,
        &mut watch,
    );
    assert!(peak > 0.1, "it plays again: {peak}");
    // A thaw lets go of the hold for the next freeze.
    assert!(m.set_frozen_quiet(StripId::Track(1), true));
    assert!(m.thaw(StripId::Track(1)));
    run(
        &mut m,
        &mut song,
        4800,
        &mut fixed(128),
        None,
        &mut nothing(),
    );
    let back = m.pop_thawed().unwrap();
    assert!(!m.set_frozen_quiet(StripId::Track(1), false), "thawed");
    assert!(m.freeze(StripId::Track(1), back).is_ok());
    let mut peak = 0f32;
    let mut watch = |m: &mut Mixer, _: &mut Song, _: Option<&mut Print>| peak = peak.max(loud(m));
    run(
        &mut m,
        &mut song,
        BEAT as usize,
        &mut fixed(128),
        None,
        &mut watch,
    );
    assert!(peak > 0.1, "frozen again, not held: {peak}");
}

#[test]
fn the_crossfade_has_no_click() {
    // A frozen copy that is the live signal turned upside down: the
    // crossfade passes through silence smoothly, with no step bigger than
    // the signal's own.
    let mut m = rig(Vec::new());
    let mut song = Song::new();
    let mut p = Print::new(RATE, 10.0);
    p.arm(job(0.0, 1.0)).unwrap();
    run(
        &mut m,
        &mut song,
        BEAT as usize + 128,
        &mut fixed(128),
        Some(&mut p),
        &mut nothing(),
    );
    let mut f = p.take().unwrap().frozen();
    for v in f.left.iter_mut().chain(f.right.iter_mut()) {
        *v = -*v;
    }
    assert!(m.freeze(StripId::Track(0), f).is_ok());
    let out = run(
        &mut m,
        &mut song,
        2400,
        &mut fixed(64),
        None,
        &mut nothing(),
    );
    let live_step = (1..BEAT as usize)
        .map(|i| (sig(i as u64).0 - sig(i as u64 - 1).0).abs())
        .fold(0.0f32, f32::max);
    let step = out
        .left
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(step <= live_step * 1.05, "step {step} against {live_step}");
}

#[test]
fn frozen_audio_follows_the_beat_at_another_tempo() {
    let n = 96_000;
    let ramp: Vec<f32> = (0..n).map(|i| i as f32).collect();
    let f = Frozen {
        left: ramp.clone(),
        right: ramp,
        origin: 0.0,
        beats: 4.0,
        bpm: 120.0,
        rate: RATE,
    };
    let (mut l, mut r) = (vec![0.0; 8], vec![0.0; 8]);
    let mut ctx = Ctx::new(RATE, 0);
    ctx.beats = 1.0;
    ctx.bpm = 60.0;
    f.render(&ctx, &mut l, &mut r);
    assert_eq!(l[0], 24_000.0);
    assert_eq!(l[1], 24_000.5);
    // At the recorded tempo it copies, wrapping round the loop.
    ctx.bpm = 120.0;
    ctx.beats = 4.0 - 4.0 / 24_000.0;
    f.render(&ctx, &mut l, &mut r);
    assert_eq!(&l[..6], &[95_996.0, 95_997.0, 95_998.0, 95_999.0, 0.0, 1.0]);
    // Before the origin it is the loop before.
    ctx.beats = -1.0;
    f.render(&ctx, &mut l, &mut r);
    assert_eq!(l[0], 72_000.0);
}

#[test]
fn a_print_makes_a_wav_and_a_tail_folds_onto_the_loop() {
    let p = Printed {
        left: vec![1.0, 2.0, 3.0, 4.0, 0.5, 0.25],
        right: vec![-1.0, -2.0, -3.0, -4.0, -0.5, -0.25],
        rate: 44_100,
        bpm: 90.0,
        start: 8.0,
        beats: 2.0,
        loop_frames: 4,
    };
    let w = p.wav();
    assert_eq!(&w[..4], b"RIFF");
    assert_eq!(&w[8..16], b"WAVEfmt ");
    assert_eq!(u16::from_le_bytes([w[20], w[21]]), 3);
    assert_eq!(u16::from_le_bytes([w[22], w[23]]), 2);
    assert_eq!(u32::from_le_bytes([w[24], w[25], w[26], w[27]]), 44_100);
    assert_eq!(&w[36..40], b"data");
    assert_eq!(u32::from_le_bytes([w[40], w[41], w[42], w[43]]), 48);
    assert_eq!(w.len(), 44 + 48);
    assert_eq!(f32::from_le_bytes([w[44], w[45], w[46], w[47]]), 1.0);
    assert_eq!(f32::from_le_bytes([w[48], w[49], w[50], w[51]]), -1.0);
    assert_eq!(p.peak(), 4.0);
    let f = p.frozen();
    assert_eq!(f.left, [1.5, 2.25, 3.0, 4.0]);
    assert_eq!(f.right, [-1.5, -2.25, -3.0, -4.0]);
    assert_eq!((f.origin, f.beats, f.bpm, f.rate), (8.0, 2.0, 90.0, 44_100));
}
