//! Stretch's copies in the track player (the audio tracks rethink, phase
//! 5b): a region on Stretch plays its copy, on its beats at its own pitch;
//! without one it plays on time on Repitch, and takes its copy at its next
//! start, never in the middle; back at its own tempo it plays at once; a
//! copy made before an edit is never played; clearing the copies loses no
//! recording.
//!
//! The copies are made here, at once ([`copies::make`]), from the take's
//! audio, as the host's worker makes them.

use mpfree_audiotracks::copies::{self, CopyKey, CopySpec, Ends};
use mpfree_audiotracks::save::{TakeFile, TrackFile};
use mpfree_audiotracks::{
    Answer, AudioTracks, Change, ClipPlay, ClipRegion, Command as A, FollowTempo, Input, Monitor,
    Plays, RecordingId, Region, FADE,
};
use mpfree_transport::time::{TempoMap, PPQ};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BEAT: i64 = PPQ as i64;
const BAR: i64 = 4 * BEAT;
const START: u64 = 10_000;
const REC: RecordingId = RecordingId(700);

fn beat_frames(bpm: f64) -> f64 {
    60.0 * f64::from(RATE) / bpm
}

fn tone(j: usize) -> f32 {
    (0.5 * (std::f64::consts::TAU * 441.0 * j as f64 / f64::from(RATE)).sin()) as f32
}

/// Two bars at 120 of a 441 Hz tone, exactly (a loop that ends where its
/// bars do).
fn tone_take() -> TakeFile {
    let frames = (8.0 * beat_frames(120.0)) as usize;
    take(frames, (0..frames).map(tone).collect())
}

fn take(frames: usize, audio: Vec<f32>) -> TakeFile {
    TakeFile {
        id: 0,
        rec: REC,
        track: 0,
        group: 0,
        pass: 0,
        bpm: 120.0,
        varied: false,
        anchor: 0.0,
        frames,
        channels: 1,
        file: String::new(),
        path: None,
        audio,
    }
}

fn stretch_region(start: i64, end: i64, src: i64) -> Region {
    Region {
        follow: FollowTempo::Stretch,
        ..Region::of(start, end, 0, (src, 0.0), 120.0)
    }
}

/// The copy for `key`, made at once from `audio` (the take's, mono).
fn copy_for(key: CopyKey, audio: &[f32]) -> copies::Copy {
    let from = CopySpec::frames_from(&key);
    let n = CopySpec::frames_in(&key);
    let input = (0..n as i64)
        .map(|i| {
            let g = from + i;
            if g >= 0 && (g as usize) < audio.len() {
                audio[g as usize]
            } else {
                0.0
            }
        })
        .collect();
    copies::make(CopySpec {
        key,
        channels: 1,
        audio: input,
    })
    .expect("a copy")
}

struct Rig {
    t: Transport,
    a: AudioTracks,
    b: Block,
    frame: u64,
    out: Vec<f32>,
    /// What track 0 plays from a frame on (the arranger's answer).
    plays: Vec<(u64, Plays<'static>)>,
}

impl Rig {
    fn new(bpm: f64, tk: TakeFile, regions: Vec<Region>) -> Rig {
        let mut r = Rig {
            t: Transport::new(RATE, bpm).unwrap(),
            a: AudioTracks::new(RATE, 30),
            b: Block::new(),
            frame: 0,
            out: Vec::new(),
            plays: Vec::new(),
        };
        assert!(r.a.load_take(tk).is_ok());
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
        assert_eq!(r.a.loading(), 0, "the take copies in before START");
        r
    }

    fn map(&self) -> TempoMap {
        *self.t.tempo_map()
    }

    fn run(&mut self, frame: u64) {
        let period = 256;
        let inp = vec![0.0f32; 2 * period];
        let mut mix = vec![0.0f32; 2 * period];
        while self.frame < frame {
            let n = (period as u64).min(frame - self.frame) as usize;
            let p0 = self.frame;
            assert!(self.t.process(n as u32, &mut self.b));
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
            while self.a.take_dropped_copy().is_some() {}
        }
    }

    fn l(&self, g: u64) -> f32 {
        self.out[2 * g as usize]
    }

    /// The pitch over frames `a..b`, by its rising zero crossings.
    fn hz(&self, a: u64, b: u64) -> f64 {
        let ups = (a..b)
            .filter(|&g| self.l(g - 1) < 0.0 && self.l(g) >= 0.0)
            .count();
        ups as f64 * f64::from(RATE) / (b - a) as f64
    }

    fn peak(&self, a: u64, b: u64) -> f32 {
        (a..b).map(|g| self.l(g).abs()).fold(0.0f32, f32::max)
    }

    /// The largest step from one frame to the next over `a..b`.
    fn jump(&self, a: u64, b: u64) -> f32 {
        (a.max(1)..b)
            .map(|g| (self.l(g) - self.l(g - 1)).abs())
            .fold(0.0f32, f32::max)
    }
}

#[test]
fn a_two_bar_loop_made_at_120_fills_two_bars_at_90_at_its_own_pitch() {
    let tk = tone_take();
    let audio = tk.audio.clone();
    let region = stretch_region(0, 2 * BAR, 0);
    let mut r = Rig::new(90.0, tk, vec![region]);
    let key =
        r.a.song_copy_key(&region, &r.map())
            .expect("a key on Stretch at 90");
    assert_eq!(key.ends, Ends::Edges);
    let len = (8.0 * beat_frames(90.0)) as u64;
    assert_eq!(key.out, len as i64);
    assert!(r.a.put_copy(copy_for(key, &audio)).is_ok());
    assert!(r.t.command(START, T::PlayStart));
    r.run(START + len + 4_000);
    // Its own pitch, 441 Hz, over the middle 4 seconds (Repitch would be
    // 330.75).
    let hz = r.hz(START + 10_000, START + 10_000 + 4 * u64::from(RATE));
    assert!((hz - 441.0).abs() <= 0.5, "{hz} Hz");
    // It fills its two bars at 90: full level to its end, then its fade
    // out over FADE frames and silence.
    assert!(r.peak(START + len - 400, START + len) > 0.45);
    assert_eq!(
        r.peak(START + len + FADE as u64 + 1, START + len + 4_000),
        0.0
    );
    let st = r.a.stats();
    assert!(st.copied >= len - 256, "{st:?}");
    assert_eq!(st.for_now, 0);
}

#[test]
fn back_at_its_own_tempo_it_plays_its_recording_at_once_frame_for_frame() {
    let tk = tone_take();
    let audio = tk.audio.clone();
    let region = stretch_region(0, 2 * BAR, 0);
    let mut r = Rig::new(120.0, tk, vec![region]);
    // At its own tempo it needs no copy.
    assert!(r.a.song_copy_key(&region, &r.map()).is_none());
    assert!(r.t.command(START, T::PlayStart));
    r.run(START + 30_000);
    for g in START + FADE as u64 + 1..START + 30_000 {
        assert_eq!(r.l(g), audio[(g - START) as usize], "frame {g}");
    }
    let st = r.a.stats();
    assert_eq!((st.copied, st.for_now), (0, 0));
}

#[test]
fn a_copy_made_before_an_edit_or_for_another_tempo_is_never_played() {
    let tk = tone_take();
    let audio = tk.audio.clone();
    let region = stretch_region(0, 2 * BAR, 0);
    // A copy for 100 BPM, and the song at 90.
    let mut r = Rig::new(90.0, tk, vec![region]);
    let at100 = TempoMap::constant(100.0).unwrap();
    let k100 = r.a.song_copy_key(&region, &at100).unwrap();
    assert!(r.a.put_copy(copy_for(k100, &audio)).is_ok());
    // And the copy for 90, made before the region's tempo was set by hand
    // to 115.
    let k90 = r.a.song_copy_key(&region, &r.map()).unwrap();
    assert!(r.a.put_copy(copy_for(k90, &audio)).is_ok());
    r.a.command(A::SetTempo {
        track: 0,
        at: 0,
        bpm: 115.0,
    });
    assert!(r.t.command(START, T::PlayStart));
    r.run(START + 30_000);
    let st = r.a.stats();
    assert_eq!(st.copied, 0, "{st:?}");
    assert!(st.for_now > 20_000, "on time on Repitch meanwhile: {st:?}");
    // Repitch from 115: 441 × 90 / 115.
    let hz = r.hz(START + 2_000, START + 28_000);
    assert!((hz - 441.0 * 90.0 / 115.0).abs() <= 1.0, "{hz} Hz");
}

#[test]
fn clearing_the_copies_loses_no_recording_and_keeps_the_one_playing() {
    let tk = tone_take();
    let audio = tk.audio.clone();
    let region = stretch_region(0, 2 * BAR, 0);
    let mut r = Rig::new(90.0, tk, vec![region]);
    let key = r.a.song_copy_key(&region, &r.map()).unwrap();
    let other =
        r.a.song_copy_key(&region, &TempoMap::constant(100.0).unwrap())
            .unwrap();
    assert!(r.a.put_copy(copy_for(key, &audio)).is_ok());
    assert!(r.a.put_copy(copy_for(other, &audio)).is_ok());
    assert_eq!(r.a.copy_bytes().1, 2);
    assert!(r.t.command(START, T::PlayStart));
    r.run(START + 10_000);
    // The copy playing stays; the other goes.
    assert_eq!(r.a.clear_copies(), 1);
    assert!(r.a.has_copy(&key) && !r.a.has_copy(&other));
    assert!(!r.a.drop_copy(&key), "a copy playing is never let go");
    r.run(START + 20_000);
    let hz = r.hz(START + 11_000, START + 20_000);
    assert!((hz - 441.0).abs() <= 1.0, "{hz} Hz");
    // Stopped, nothing plays it: it goes too, and the recording is as it
    // was.
    assert!(r.t.command(START + 20_000, T::Stop));
    r.run(START + 21_000);
    assert_eq!(r.a.clear_copies(), 1);
    assert_eq!(r.a.copy_bytes(), (0, 0));
    let mut back = vec![0.0f32; audio.len()];
    assert_eq!(r.a.read_take(0, 0, &mut back), Some(audio.len()));
    assert_eq!(back, audio);
}

/// A looping clip: one region filling its pass, the whole take.
fn loop_clip(regions: &'static [ClipRegion]) -> Plays<'static> {
    Plays::Clip(ClipPlay {
        regions,
        origin: 0.0,
        len: 2 * BAR,
        last_frame: (8.0 * beat_frames(120.0)) as i64,
        gain: 1.0,
        clip: 1,
        stamp: 1,
    })
}

static LOOP: [ClipRegion; 1] = [ClipRegion {
    start: 0,
    end: 2 * BAR,
    rec: REC,
    src: 0,
    phase: 0.0,
    gain: 1.0,
    fade_in: 0,
    fade_out: 0,
    into: 0,
    left: 0,
    follow: FollowTempo::Stretch,
    keep_pitch: false,
    bpm: 0.0,
}];

#[test]
fn without_its_copy_a_clip_plays_on_time_and_takes_its_copy_at_its_next_pass() {
    let tk = tone_take();
    let audio = tk.audio.clone();
    let mut r = Rig::new(90.0, tk, vec![]);
    let clip = loop_clip(&LOOP);
    r.plays.push((START, clip));
    assert!(r.t.command(START, T::PlayStart));
    let pass = (8.0 * beat_frames(90.0)) as u64;
    // Halfway through the first pass its copy comes.
    r.run(START + pass / 2);
    let Plays::Clip(c) = clip else { unreachable!() };
    let mut keys = Vec::new();
    r.a.clip_copy_keys(&c, 90.0, &mut |i, k| keys.push((i, k)));
    assert_eq!(keys.len(), 1);
    let key = keys[0].1;
    assert_eq!(
        key.ends,
        Ends::Wrapped,
        "a loop that ends where its bars do"
    );
    assert_eq!(key.out, pass as i64);
    assert!(r.a.put_copy(copy_for(key, &audio)).is_ok());
    r.run(START + 3 * pass);
    // The first pass plays on time on Repitch to its end, never its copy
    // in the middle...
    let first = r.hz(START + 2_000, START + pass - 500);
    assert!((first - 441.0 * 90.0 / 120.0).abs() <= 1.0, "{first} Hz");
    // ...and the next passes their copy, at their own pitch.
    for p in 1..3 {
        let hz = r.hz(START + p * pass + 500, START + (p + 1) * pass - 500);
        assert!((hz - 441.0).abs() <= 1.0, "pass {p}: {hz} Hz");
    }
    // The switch fades from one into the other: no step bigger than the
    // tone's own, either side of it.
    let tone_step = (0.5 * std::f64::consts::TAU * 441.0 / f64::from(RATE)) as f32;
    let at = START + pass;
    assert!(
        r.jump(at - 200, at + 200) <= 1.5 * tone_step,
        "{}",
        r.jump(at - 200, at + 200)
    );
    // The wrapped copy goes round pass to pass with no step either.
    let at = START + 2 * pass;
    assert!(
        r.jump(at - 200, at + 200) <= 1.5 * tone_step,
        "{}",
        r.jump(at - 200, at + 200)
    );
    let st = r.a.stats();
    assert!(st.copied > 0 && st.for_now > 0, "{st:?}");
}

#[test]
fn a_tempo_turned_mid_pass_plays_repitch_to_the_pass_end_then_the_new_copy() {
    let tk = tone_take();
    let audio = tk.audio.clone();
    let mut r = Rig::new(90.0, tk, vec![]);
    let clip = loop_clip(&LOOP);
    let Plays::Clip(c) = clip else { unreachable!() };
    let key_at = |a: &AudioTracks, bpm: f64| {
        let mut k = None;
        a.clip_copy_keys(&c, bpm, &mut |_, key| k = Some(key));
        k.unwrap()
    };
    // Its copies for 90 and for 100 are both here.
    let (k90, k100) = (key_at(&r.a, 90.0), key_at(&r.a, 100.0));
    assert!(r.a.put_copy(copy_for(k90, &audio)).is_ok());
    assert!(r.a.put_copy(copy_for(k100, &audio)).is_ok());
    r.plays.push((START, clip));
    assert!(r.t.command(START, T::PlayStart));
    let pass90 = (8.0 * beat_frames(90.0)) as u64;
    r.run(START + pass90 / 2);
    // The tempo turned to 100 halfway through the first pass.
    let half = r.frame;
    assert!(r.t.command(half, T::TempoNow(100.0)));
    r.run(half + 2 * pass90);
    let before = r.hz(START + 1_000, half - 200);
    assert!(
        (before - 441.0).abs() <= 1.0,
        "its copy for 90: {before} Hz"
    );
    // The rest of the pass: Repitch from 120 at 100, not the copy for 100
    // in the middle.
    let pass100 = 8.0 * beat_frames(100.0);
    let left = (pass90 / 2) as f64 * 90.0 / 100.0;
    let end = half + left as u64;
    let rest = r.hz(half + 1_500, end - 300);
    assert!(
        (rest - 441.0 * 100.0 / 120.0).abs() <= 1.5,
        "for now: {rest} Hz"
    );
    let next = r.hz(end + 500, end + pass100 as u64 - 500);
    assert!((next - 441.0).abs() <= 1.0, "its copy for 100: {next} Hz");
}

/// Two bars at 120 with a click halfway through every beat (so none sits
/// on the loop's seam), each beat's at its own level.
fn clicks_take() -> TakeFile {
    let per = beat_frames(120.0);
    let frames = (8.0 * per) as usize;
    let audio = (0..frames)
        .map(|j| {
            (0..8)
                .map(|b| {
                    let d = (j as f64 - (b as f64 + 0.5) * per) / 2.0;
                    (0.3 + 0.05 * b as f64) * (-0.5 * d * d).exp()
                })
                .sum::<f64>() as f32
        })
        .collect();
    take(frames, audio)
}

impl Rig {
    /// The clicks heard over `a..b`: each peak above `floor` (the louder
    /// of any two within 200 frames), placed between frames.
    fn clicks(&self, a: u64, b: u64, floor: f32) -> Vec<f64> {
        let mut out: Vec<(f64, f32)> = Vec::new();
        for g in a.max(1)..b {
            let (x, y, z) = (self.l(g - 1), self.l(g), self.l(g + 1));
            if y > floor && y >= x && y > z {
                let d = x - 2.0 * y + z;
                let f = if d == 0.0 { 0.0 } else { 0.5 * (x - z) / d };
                let at = g as f64 + f64::from(f);
                match out.last_mut() {
                    Some(last) if at - last.0 < 200.0 => {
                        if y > last.1 {
                            *last = (at, y);
                        }
                    }
                    _ => out.push((at, y)),
                }
            }
        }
        out.into_iter().map(|p| p.0).collect()
    }
}

#[test]
fn every_click_lands_on_its_beat_across_the_switch_from_repitch_to_its_copy() {
    let tk = clicks_take();
    let audio = tk.audio.clone();
    let mut r = Rig::new(90.0, tk, vec![]);
    let clip = loop_clip(&LOOP);
    r.plays.push((START, clip));
    assert!(r.t.command(START, T::PlayStart));
    let per = beat_frames(90.0);
    let pass = (8.0 * per) as u64;
    r.run(START + pass / 3);
    let Plays::Clip(c) = clip else { unreachable!() };
    let mut key = None;
    r.a.clip_copy_keys(&c, 90.0, &mut |_, k| key = Some(k));
    assert!(r.a.put_copy(copy_for(key.unwrap(), &audio)).is_ok());
    r.run(START + 3 * pass + 1_000);
    // Beat b's click is half a beat after it, at 90.
    let want = |p: u64| -> Vec<f64> {
        (0..8)
            .map(|b| (START + p * pass) as f64 + (b as f64 + 0.5) * per)
            .collect()
    };
    let check = |p: u64, tol: f64, what: &str| {
        let heard = r.clicks(START + p * pass, START + (p + 1) * pass, 0.1);
        let w = want(p);
        assert_eq!(heard.len(), 8, "{what}: {heard:?}\nwant {w:?}");
        for (i, (h, w)) in heard.iter().zip(&w).enumerate() {
            assert!(
                (h - w).abs() <= tol,
                "{what}: click {i} at {h}, its beat's at {w}"
            );
        }
    };
    // Repitch: on the frame.
    check(0, 0.5, "the first pass, on Repitch");
    // The copy: each hit where the stretch puts it. Its hit finder finds
    // a click's onset about 30 frames (at 22.05 kHz) before its peak, and
    // the attack plays at its own speed from there, so a hit lands about
    // 30 × (stretch − 1) frames early: 10 here (0.45 ms), the same for
    // every click, none missing or heard twice, the seam included.
    let tol = 32.0 * (120.0 / 90.0 - 1.0) + 2.0;
    check(1, tol, "the next pass, from its copy");
    check(2, tol, "the one after");
}
