//! Regions and clips following the song's tempo (the audio tracks
//! rethink's phase 5a): one mapping from song ticks to a recording's
//! frames, kept through a tempo change, a ramp, a locate and a launch
//! with an offset; Repitch moving the pitch as tape does; a comp of takes
//! made at two tempos on its beats; a take recorded across a tempo change
//! playing Off.
//!
//! The takes are made at 120 BPM (a beat is 11 025 frames at 22.05 kHz)
//! or 100 (13 230), each with a click on every beat: a narrow Gaussian
//! pulse whose peak can be found between frames, so a click out of place
//! by a frame shows, and one heard twice or missing too. Each beat's click
//! has its own level ([`level`], eight levels round), so a click heard on
//! its frame is known to be its own beat's.

use mpfree_audiotracks::save::{TakeFile, TrackFile};
use mpfree_audiotracks::{
    Answer, AudioTracks, Change, ClipPlay, ClipRegion, Command as A, FollowTempo, Input, Monitor,
    Plays, RecordingId, Region, FADE,
};
use mpfree_transport::time::{Segment, TempoMap, PPQ};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BEAT: i64 = PPQ as i64;
const BAR: i64 = 4 * BEAT;
const START: u64 = 10_000;

/// Recording frames a beat at `bpm`.
fn beat_frames(bpm: f64) -> f64 {
    60.0 * f64::from(RATE) / bpm
}

/// A click: a Gaussian pulse 2 frames wide, `level` high at frame `at`.
fn pulse(j: f64, at: f64, level: f32) -> f32 {
    let d = (j - at) / 2.0;
    level * (-0.5 * d * d).exp() as f32
}

/// The level of the click on beat `b` of a take.
fn level(b: i64) -> f32 {
    0.3 + 0.05 * b.rem_euclid(8) as f32
}

/// A take of `beats` beats made at `bpm`, mono, a click on every beat
/// (the first at frame 0, each at its [`level`]), on `track`.
fn clicks(id: u16, track: usize, bpm: f64, beats: usize) -> TakeFile {
    let per = beat_frames(bpm);
    let frames = (beats as f64 * per).ceil() as usize + 64;
    let audio = (0..frames)
        .map(|j| {
            let b = (j as f64 / per).round();
            pulse(j as f64, b * per, level(b as i64))
        })
        .collect();
    take(id, track, bpm, frames, audio)
}

fn take(id: u16, track: usize, bpm: f64, frames: usize, audio: Vec<f32>) -> TakeFile {
    TakeFile {
        id,
        rec: RecordingId(500 + u128::from(id)),
        track,
        group: id,
        pass: 0,
        bpm,
        varied: false,
        anchor: 0.0,
        frames,
        channels: 1,
        file: String::new(),
        path: None,
        audio,
    }
}

/// A region of take `take` on song ticks `start..end`, its tick `start`
/// playing frame `src`, following the tempo as `follow` says from `bpm`.
fn region(take: u16, start: i64, end: i64, src: i64, follow: FollowTempo, bpm: f64) -> Region {
    Region {
        follow,
        ..Region::of(start, end, take, (src, 0.0), bpm)
    }
}

struct Rig {
    t: Transport,
    a: AudioTracks,
    b: Block,
    frame: u64,
    out: Vec<f32>,
    /// The song tick of every frame played while rolling (NaN stopped).
    ticks: Vec<f64>,
    /// What track 0 plays from a frame on (the arranger's answer).
    plays: Vec<(u64, Plays<'static>)>,
}

impl Rig {
    fn new(bpm: f64, takes: Vec<TakeFile>, regions: Vec<Region>) -> Rig {
        let mut r = Rig {
            t: Transport::new(RATE, bpm).unwrap(),
            a: AudioTracks::new(RATE, 30),
            b: Block::new(),
            frame: 0,
            out: Vec::new(),
            ticks: Vec::new(),
            plays: Vec::new(),
        };
        for tk in takes {
            assert!(r.a.load_take(tk).is_ok());
        }
        let track = TrackFile {
            index: 0,
            id: 0,
            input: Input::Left,
            monitor: Monitor::Auto,
            from: String::new(),
            regions,
        };
        assert!(r.a.load_track(&track));
        r.run(START);
        assert_eq!(r.a.loading(), 0, "the takes copy in before START");
        r
    }

    fn map(&mut self, map: TempoMap) {
        self.t.set_tempo_map(map).unwrap();
    }

    fn run(&mut self, frame: u64) {
        let period = 256;
        let inp = vec![0.0f32; 2 * period];
        let mut mix = vec![0.0f32; 2 * period];
        while self.frame < frame {
            let n = (period as u64).min(frame - self.frame) as usize;
            let p0 = self.frame;
            assert!(self.t.process(n as u32, &mut self.b));
            for j in 0..n as u64 {
                let off = j as u32;
                let tick = self
                    .b
                    .spans()
                    .iter()
                    .find(|s| off >= s.offset && off < s.offset + s.len)
                    .filter(|s| {
                        matches!(
                            s.state,
                            mpfree_transport::State::Playing | mpfree_transport::State::Recording
                        )
                    })
                    .map_or(f64::NAN, |s| s.tick_at(p0 + j));
                self.ticks.push(tick);
            }
            let before = self
                .plays
                .iter()
                .rev()
                .find(|(at, _)| *at < p0)
                .map(|p| p.1);
            let b = &self.b;
            let changes: Vec<Change> = self
                .plays
                .iter()
                .filter(|(at, _)| (p0..p0 + n as u64).contains(at))
                .map(|&(at, plays)| Change {
                    frame: (at - p0) as u32,
                    tick: b
                        .spans()
                        .iter()
                        .find(|s| (at - p0) as u32 >= s.offset)
                        .map_or(0.0, |s| s.tick_at(at).round()),
                    plays,
                    muted: false,
                    ring: false,
                })
                .collect();
            let answer = [Answer {
                start: before.unwrap_or(Plays::Song),
                muted: false,
                changes: &changes,
                superseded: 0,
            }];
            mix[..2 * n].fill(0.0);
            let map = *self.t.tempo_map();
            self.a
                .process_with(&self.b, &map, &inp[..2 * n], &mut mix[..2 * n], &answer);
            self.out.extend_from_slice(&mix[..2 * n]);
            self.frame += n as u64;
        }
    }

    fn l(&self, g: u64) -> f32 {
        self.out[2 * g as usize]
    }

    /// The frames (with their fractions) where the song plays each of
    /// `beats` (song ticks), in order: every frame where the ticks pass
    /// one, so a beat played twice (a locate back) is there twice.
    fn beat_frames(&self, beats: &[i64]) -> Vec<f64> {
        let mut out = Vec::new();
        for g in 1..self.ticks.len() {
            let (a, b) = (self.ticks[g - 1], self.ticks[g]);
            if a.is_nan() || b.is_nan() || b <= a {
                continue;
            }
            for &t in beats {
                let t = t as f64;
                if a < t && t <= b {
                    out.push((g - 1) as f64 + (t - a) / (b - a));
                }
            }
        }
        out
    }

    /// The clicks heard, from frame `from` on: each peak above `floor`
    /// (and the louder of any two within 200 frames), placed between
    /// frames by the parabola through it and its neighbours.
    fn clicks(&self, from: u64, floor: f32) -> Vec<f64> {
        self.peaks(from, floor).into_iter().map(|p| p.0).collect()
    }

    /// [`Rig::clicks`] with each click's level, its peak's between frames.
    fn peaks(&self, from: u64, floor: f32) -> Vec<(f64, f32)> {
        let n = self.out.len() / 2;
        let mut out: Vec<(f64, f32)> = Vec::new();
        for g in (from as usize).max(1)..n - 1 {
            let (a, b, c) = (self.l(g as u64 - 1), self.l(g as u64), self.l(g as u64 + 1));
            if b > floor && b >= a && b > c {
                let d = a - 2.0 * b + c;
                let x = if d == 0.0 { 0.0 } else { 0.5 * (a - c) / d };
                let at = g as f64 + f64::from(x);
                let top = b - 0.25 * (a - c) * x;
                match out.last_mut() {
                    Some(last) if at - last.0 < 200.0 => {
                        if top > last.1 {
                            *last = (at, top);
                        }
                    }
                    _ => out.push((at, top)),
                }
            }
        }
        out
    }
}

/// Every click heard is on its beat's frame, within `tol` frames, and
/// none is missing or heard twice.
fn on_their_beats(heard: &[f64], want: &[f64], tol: f64, what: &str) {
    assert_eq!(
        heard.len(),
        want.len(),
        "{what}: {} clicks heard, {} beats\nheard {heard:?}\nwant {want:?}",
        heard.len(),
        want.len()
    );
    for (i, (h, w)) in heard.iter().zip(want).enumerate() {
        assert!(
            (h - w).abs() <= tol,
            "{what}: click {i} at {h}, its beat at {w}"
        );
    }
}

fn beats(from: i64, to: i64) -> Vec<i64> {
    (from / BEAT..to / BEAT).map(|b| b * BEAT).collect()
}

#[test]
fn a_tone_made_at_120_and_played_at_90_fills_its_two_bars_at_330_75_hz() {
    // Two bars at 120 of a 441 Hz tone.
    let frames = (8.0 * beat_frames(120.0)) as usize;
    let audio = (0..frames)
        .map(|j| (0.5 * (std::f64::consts::TAU * 441.0 * j as f64 / f64::from(RATE)).sin()) as f32)
        .collect();
    let mut r = Rig::new(
        90.0,
        vec![take(0, 0, 120.0, frames, audio)],
        vec![region(0, 0, 2 * BAR, 0, FollowTempo::Repitch, 120.0)],
    );
    assert!(r.t.command(START, T::PlayStart));
    let len = (8.0 * beat_frames(90.0)) as u64;
    r.run(START + len + 4_000);
    // Its pitch: zero crossings over the middle 4 seconds.
    let (a, b) = (START + 10_000, START + 10_000 + 4 * u64::from(RATE));
    let ups = (a..b)
        .filter(|&g| r.l(g - 1) < 0.0 && r.l(g) >= 0.0)
        .count();
    let hz = ups as f64 / 4.0;
    assert!((hz - 330.75).abs() <= 0.25, "{hz} Hz");
    // It fills its two bars: full level to its end, then its fade out
    // over FADE frames and silence.
    let peak = |from: u64, to: u64| (from..to).map(|g| r.l(g).abs()).fold(0.0f32, f32::max);
    assert!(peak(START + len - 400, START + len) > 0.49);
    assert_eq!(
        peak(START + len + FADE as u64 + 1, START + len + 4_000),
        0.0
    );
}

#[test]
fn every_click_lands_on_its_beat_across_a_tempo_change_a_ramp_and_a_locate() {
    let made = 120.0;
    let take = || clicks(0, 0, made, 24);
    let whole = || vec![region(0, 0, 6 * BAR, 0, FollowTempo::Repitch, made)];
    // A tempo change halfway: 90 to bar 3, then 150.
    let mut r = Rig::new(90.0, vec![take()], whole());
    r.map(
        TempoMap::new(&[
            Segment {
                start: 0,
                bpm: 90.0,
            },
            Segment {
                start: 3 * BAR,
                bpm: 150.0,
            },
        ])
        .unwrap(),
    );
    assert!(r.t.command(START, T::PlayStart));
    r.run(START + 8 * u64::from(RATE));
    // The first click, at the region's start, is under its crossfade in
    // and before the song rolls: the beats from beat 1.
    let want = r.beat_frames(&beats(0, 6 * BAR));
    on_their_beats(&r.clicks(START + 2, 0.15), &want, 0.5, "a tempo change");

    // A ramp from 100 to 140 over bars 2 to 4, played as 1/16 steps.
    let mut r = Rig::new(100.0, vec![take()], whole());
    r.map(
        TempoMap::new(&[
            Segment {
                start: 0,
                bpm: 100.0,
            },
            Segment {
                start: 2 * BAR,
                bpm: 100.0,
            },
            Segment {
                start: 4 * BAR,
                bpm: 140.0,
            },
        ])
        .unwrap()
        .ramp(1)
        .unwrap(),
    );
    assert!(r.t.command(START, T::PlayStart));
    r.run(START + 8 * u64::from(RATE));
    let want = r.beat_frames(&beats(0, 6 * BAR));
    on_their_beats(&r.clicks(START + 2, 0.15), &want, 0.5, "a ramp");

    // At 90, a locate back into bar 3 from bar 4: bar 3's beats again,
    // each once more, on their frames after the jump.
    let mut r = Rig::new(90.0, vec![take()], whole());
    assert!(r.t.command(START, T::PlayStart));
    let jump = START + (4.0 * 4.0 * beat_frames(90.0)) as u64 + 3_000;
    r.run(jump);
    assert!(r.t.command(jump, T::Locate(2 * BAR + BEAT / 2)));
    r.run(jump + 4 * u64::from(RATE));
    let want = r.beat_frames(&beats(0, 6 * BAR));
    let heard = r.peaks(START + 2, 0.15);
    let at: Vec<f64> = heard.iter().map(|p| p.0).collect();
    on_their_beats(&at, &want, 0.5, "a locate");
    // Each the moment of the recording its beat is: beats 1 to 16, then
    // from the locate, beats 9 on again.
    let jumped = heard.iter().position(|p| p.0 > jump as f64).unwrap();
    for (i, (_, top)) in heard.iter().enumerate() {
        let b = if i < jumped {
            1 + i as i64
        } else {
            9 + (i - jumped) as i64
        };
        let want = level(b);
        assert!(
            (top - want).abs() < 0.02,
            "click {i}: level {top}, beat {b}'s {want}"
        );
    }
}

#[test]
fn a_clip_launched_with_an_offset_plays_its_beats_on_the_song_s_beats() {
    // A 2-bar clip of a take made at 120, its passes counted from bar 0,
    // launched at bar 3 at 90: it starts a bar into its pass.
    let made = 120.0;
    let tk = clicks(0, 0, made, 8);
    let regions: &'static [ClipRegion] = Box::leak(Box::new([ClipRegion {
        start: 0,
        end: 2 * BAR,
        rec: RecordingId(500),
        src: 0,
        phase: 0.0,
        gain: 1.0,
        fade_in: 0,
        fade_out: 0,
        into: 0,
        left: 0,
        follow: FollowTempo::Repitch,
        keep_pitch: false,
        bpm: made,
    }]));
    let mut r = Rig::new(90.0, vec![tk], vec![]);
    assert!(r.t.command(START, T::PlayStart));
    let launch = START + (3.0 * 4.0 * beat_frames(90.0)).round() as u64;
    let clip = ClipPlay {
        regions,
        origin: 0.0,
        len: 2 * BAR,
        last_frame: (8.0 * beat_frames(made)) as i64,
        gain: 1.0,
        clip: 1,
        stamp: 1,
    };
    r.plays.push((launch, Plays::Clip(clip)));
    r.run(launch + 4 * u64::from(RATE));
    // Bar 3 is the second bar of its pass: its beats from bar 3 on, the
    // first under the launch's crossfade.
    let want: Vec<f64> = r
        .beat_frames(&beats(3 * BAR, 6 * BAR))
        .into_iter()
        .filter(|&f| f < (launch + 4 * u64::from(RATE)) as f64 - 200.0)
        .collect();
    on_their_beats(
        &r.clicks(launch + 2, 0.15),
        &want[1..],
        0.5,
        "a launch with an offset",
    );
    // And each is its own beat's: song beat 13 (bar 3's second) is the
    // clip's beat 5, its pass's second bar's second.
    for (i, (_, top)) in r.peaks(launch + 2, 0.15).iter().enumerate() {
        let song_beat = 13 + i as i64;
        let want = level(song_beat % 8);
        assert!(
            (top - want).abs() < 0.02,
            "click {i}: level {top}, beat {song_beat}'s {want}"
        );
    }
}

#[test]
fn a_comp_of_takes_made_at_120_and_100_plays_on_its_beats_at_110() {
    // Take 0 made at 120 and take 1 at 100, each from song tick 0: two
    // bars of the first, then bars 2 and 3 of the second (from its beat 8).
    let src1 = (8.0 * beat_frames(100.0)).round() as i64;
    let mut r = Rig::new(
        110.0,
        vec![clicks(0, 0, 120.0, 16), clicks(1, 0, 100.0, 16)],
        vec![
            region(0, 0, 2 * BAR, 0, FollowTempo::Repitch, 120.0),
            region(1, 2 * BAR, 4 * BAR, src1, FollowTempo::Repitch, 100.0),
        ],
    );
    assert!(r.t.command(START, T::PlayStart));
    r.run(START + (17.0 * beat_frames(110.0)) as u64);
    // Up to the click at its end, which plays in its crossfade out.
    let want = r.beat_frames(&beats(0, 4 * BAR + BEAT));
    on_their_beats(
        &r.clicks(START + 2, 0.15),
        &want,
        0.5,
        "a comp of two tempos",
    );
}

#[test]
fn at_its_own_tempo_a_region_following_the_tempo_plays_as_off_frame_for_frame() {
    let sig = |j: u64| ((j * 7919) % 20011) as f32 / 20011.0 - 0.5;
    let frames = 4 * 44_100;
    let audio: Vec<f32> = (0..frames as u64).map(sig).collect();
    let play = |follow: FollowTempo| {
        let mut r = Rig::new(
            120.0,
            vec![take(0, 0, 120.0, frames, audio.clone())],
            vec![region(0, BAR, 3 * BAR, 1_000, follow, 120.0)],
        );
        assert!(r.t.command(START, T::PlayStart));
        r.run(START + 4 * 22_050 * 2);
        r.out
    };
    let off = play(FollowTempo::Off);
    assert!(off.iter().any(|&x| x != 0.0));
    assert_eq!(play(FollowTempo::Repitch), off);
    assert_eq!(
        play(FollowTempo::Stretch),
        off,
        "Stretch plays as Repitch until 5b"
    );
}

#[test]
fn a_take_recorded_across_a_tempo_change_has_no_one_tempo_and_plays_off() {
    let mut a = AudioTracks::new(RATE, 10);
    let mut t = Transport::new(RATE, 120.0).unwrap();
    let mut b = Block::new();
    assert!(a.command(A::SetInput(0, Input::Left)));
    assert!(a.command(A::Arm(0, true)));
    assert!(t.command(1_000, T::Record));
    let input: Vec<f32> = (0..512).map(|j| (j % 7) as f32 * 0.1).collect();
    let mut out = vec![0.0f32; 512];
    let mut g = 0u64;
    let change = u64::from(RATE);
    while g < 3 * u64::from(RATE) {
        if (g..g + 256).contains(&change) {
            assert!(t.command(change, T::TempoNow(100.0)));
        }
        assert!(t.process(256, &mut b));
        let map = *t.tempo_map();
        out.fill(0.0);
        a.process(&b, &map, &input, &mut out);
        g += 256;
    }
    assert!(t.command(g, T::Stop));
    assert!(t.process(256, &mut b));
    let map = *t.tempo_map();
    a.process(&b, &map, &input, &mut out);
    let take = a.take(0).unwrap();
    assert!(take.varied, "the tempo changed while it recorded");
    let r = a.regions(0)[0];
    assert_eq!(r.bpm, 0.0, "a region of it has no made-at tempo");
    assert!(a.command(A::SetFollow {
        track: 0,
        at: r.start,
        follow: FollowTempo::Repitch,
        keep_pitch: false,
    }));
    a.process(&b, &map, &input, &mut out);
    let r = a.regions(0)[0];
    assert_eq!(r.follow, FollowTempo::Repitch);
    assert_eq!(r.per_tick(RATE), None, "so it plays Off");
}
