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

// Keep pitch through a tempo change nobody planned (5b.4): it plays on in
// pitch from the audio it was latched to, drifting from the bars, and
// rejoins on the first bar line after the copy for the tempo now is held.

fn keep(start: i64, end: i64) -> Region {
    Region {
        keep_pitch: true,
        ..stretch_region(start, end, 0)
    }
}

/// `bars` bars at 120 of the 441 Hz tone.
fn long_tone(bars: usize) -> TakeFile {
    let frames = (4.0 * bars as f64 * beat_frames(120.0)) as usize;
    take(frames, (0..frames).map(tone).collect())
}

/// The frame of song tick `t` (in beats) when the tempo went from 90 to
/// `to` at beat `c` (frame `START + c` beats at 90).
fn frame_at(beats: f64, c: f64, to: f64) -> f64 {
    START as f64 + c * beat_frames(90.0) + (beats - c) * beat_frames(to)
}

#[test]
fn on_keep_pitch_a_tempo_turned_mid_region_plays_on_in_pitch_then_rejoins_on_the_next_bar() {
    let tk = clicks_take();
    let audio = tk.audio.clone();
    let region = keep(0, 2 * BAR);
    let mut r = Rig::new(90.0, tk, vec![region]);
    let k90 = r.a.song_copy_key(&region, &r.map()).unwrap();
    let at100 = TempoMap::constant(100.0).unwrap();
    let k100 = r.a.song_copy_key(&region, &at100).unwrap();
    assert!(r.a.put_copy(copy_for(k90, &audio)).is_ok());
    assert!(r.a.put_copy(copy_for(k100, &audio)).is_ok());
    assert!(r.t.command(START, T::PlayStart));
    // The tempo turned to 100 a beat and a quarter in.
    let c = START + (1.25 * beat_frames(90.0)) as u64;
    r.run(c);
    assert!(r.t.command(c, T::TempoNow(100.0)));
    let end = frame_at(8.0, 1.25, 100.0);
    r.run(end as u64 + 4_000);
    // Beat b's click is half a beat after it. Up to the bar line its copy
    // for 90 plays on at speed 1 from where it was, so its clicks keep
    // 90's spacing; from the bar line (beat 4) the copy for 100 plays where
    // the bars put it. Each copy's hits land a few frames early (the
    // stretch's own: see the click test above).
    let bar = frame_at(4.0, 1.25, 100.0);
    let mut want: Vec<f64> = (0..4)
        .map(|b| START as f64 + (b as f64 + 0.5) * beat_frames(90.0))
        .collect();
    want.extend((4..8).map(|b| frame_at(b as f64 + 0.5, 1.25, 100.0)));
    let heard = r.clicks(START, end as u64, 0.1);
    assert_eq!(heard.len(), 8, "{heard:?}\nwant {want:?}");
    let tol = 32.0 * (120.0 / 90.0 - 1.0) + 2.0;
    for (i, (h, w)) in heard.iter().zip(&want).enumerate() {
        assert!((h - w).abs() <= tol, "click {i} at {h}, want {w}");
    }
    // Beat 3's click drifted: at 90's spacing, not 100's.
    assert!((heard[3] - frame_at(3.5, 1.25, 100.0)).abs() > 1_000.0);
    let st = r.a.stats();
    assert_eq!(st.for_now, 0, "never Repitch: {st:?}");
    let drift = bar - c as f64;
    assert!(
        (st.drifted as f64 - drift).abs() <= 2.0 * FADE as f64 + 256.0,
        "drifted to the bar line: {st:?}, {drift}"
    );
    // It holds the copy for 100 from there: the one for 90 may go.
    assert!(r.a.drop_copy(&k90));
}

#[test]
fn keep_pitch_drifts_in_pitch_until_its_copy_comes_then_rejoins_on_the_bar_after_with_no_click() {
    let tk = long_tone(4);
    let audio = tk.audio.clone();
    let region = keep(0, 4 * BAR);
    let mut r = Rig::new(90.0, tk, vec![region]);
    let k90 = r.a.song_copy_key(&region, &r.map()).unwrap();
    let at100 = TempoMap::constant(100.0).unwrap();
    let k100 = r.a.song_copy_key(&region, &at100).unwrap();
    assert!(r.a.put_copy(copy_for(k90, &audio)).is_ok());
    assert!(r.t.command(START, T::PlayStart));
    let c = START + (1.25 * beat_frames(90.0)) as u64;
    r.run(c);
    assert!(r.t.command(c, T::TempoNow(100.0)));
    // Past bar 2's line with no copy for 100: it plays on in pitch (441
    // Hz, where Repitch from 120 at 100 is 367.5).
    let bar2 = frame_at(4.0, 1.25, 100.0) as u64;
    r.run(bar2 + 4_000);
    let hz = r.hz(c + 1_000, bar2 + 3_000);
    assert!((hz - 441.0).abs() <= 1.0, "drifting: {hz} Hz");
    let st = r.a.stats();
    assert_eq!(st.for_now, 0, "{st:?}");
    // The host is asked for the copy for 100 at once; the copy for 90
    // still plays, so it stays.
    let mut wants = Vec::new();
    r.a.drift_wants(&mut |k| wants.push(k));
    assert_eq!(wants, vec![k100]);
    assert!(!r.a.drop_copy(&k90), "the copy it drifts on stays");
    // The copy comes: it rejoins on the next bar line, bar 3's.
    assert!(r.a.put_copy(copy_for(k100, &audio)).is_ok());
    let bar3 = frame_at(8.0, 1.25, 100.0) as u64;
    r.run(bar3 + 20_000);
    let hz = r.hz(bar2 + 4_000, bar3 - 200);
    assert!((hz - 441.0).abs() <= 1.0, "still drifting: {hz} Hz");
    let hz = r.hz(bar3 + 200, bar3 + 20_000);
    assert!((hz - 441.0).abs() <= 1.0, "its copy for 100: {hz} Hz");
    // The rejoin crossfades the two: no step bigger than the tone's own.
    let tone_step = (0.5 * std::f64::consts::TAU * 441.0 / f64::from(RATE)) as f32;
    let j = r.jump(bar3 - 300, bar3 + 300);
    assert!(j <= 1.5 * tone_step, "{j}");
    let drifted = r.a.stats().drifted;
    assert!(
        (drifted as f64 - (bar3 - c) as f64).abs() <= 2.0 * FADE as f64 + 256.0,
        "{drifted}"
    );
    // From there it holds the copy for 100.
    let mut wants = Vec::new();
    r.a.drift_wants(&mut |k| wants.push(k));
    assert!(wants.is_empty());
    assert!(r.a.drop_copy(&k90));
    assert!(!r.a.drop_copy(&k100), "the copy it plays now stays");
}

/// `bars` bars at 120 of noise (no two stretches alike, so where it
/// plays from shows).
fn noise_take(bars: usize) -> TakeFile {
    let frames = (4.0 * bars as f64 * beat_frames(120.0)) as usize;
    let mut x: u32 = 0x1234_5678;
    let audio = (0..frames)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (f64::from(x >> 8) / f64::from(1u32 << 24) - 0.5) as f32 * 0.8
        })
        .collect();
    take(frames, audio)
}

#[test]
fn keep_pitch_at_its_own_tempo_plays_on_from_its_recording_and_rejoins_it_when_the_tempo_comes_back(
) {
    let tk = noise_take(4);
    let audio = tk.audio.clone();
    let region = keep(0, 4 * BAR);
    let mut r = Rig::new(120.0, tk, vec![region]);
    assert!(r.t.command(START, T::PlayStart));
    let per = beat_frames(120.0);
    // At its own tempo it plays its recording, frame for frame. Turned to
    // 100 a beat and a quarter in, it plays on from there at speed 1.
    let c = START + (1.25 * per) as u64;
    r.run(c);
    assert!(r.t.command(c, T::TempoNow(100.0)));
    let c2 = c + 30_000;
    r.run(c2);
    for g in START + FADE as u64 + 1..c2 {
        assert_eq!(r.l(g), audio[(g - START) as usize], "frame {g}");
    }
    // Back at 120 it rejoins its recording where the bars put it, on bar
    // 2's line: 5,000 frames back (30,000 frames at 100 are 25,000 at
    // 120), give or take the tick each tempo change lands on.
    assert!(r.t.command(c2, T::TempoNow(120.0)));
    let beats_at_c2 = 1.25 + 30_000.0 / beat_frames(100.0);
    let bar2 = (c2 as f64 + (4.0 - beats_at_c2) * per) as u64;
    r.run(bar2 + 20_000);
    for g in c2..bar2 - 20 {
        assert_eq!(
            r.l(g),
            audio[(g - START) as usize],
            "still drifting: frame {g}"
        );
    }
    let from = bar2 + FADE as u64 + 20;
    let plays =
        |d: i64| (from..from + 10_000).all(|g| r.l(g) == audio[((g - START) as i64 + d) as usize]);
    let back = (-5_020..-4_980).filter(|&d| plays(d)).collect::<Vec<_>>();
    assert_eq!(back.len(), 1, "on the bars, from one place: {back:?}");
    let st = r.a.stats();
    assert_eq!((st.copied, st.for_now), (0, 0), "{st:?}");
    assert!(st.drifted > 0, "{st:?}");
}

static KEEP_LOOP: [ClipRegion; 1] = [ClipRegion {
    keep_pitch: true,
    ..LOOP[0]
}];

#[test]
fn a_clip_on_keep_pitch_drifts_through_a_tempo_change_and_rejoins_within_its_pass() {
    let tk = tone_take();
    let audio = tk.audio.clone();
    let mut r = Rig::new(90.0, tk, vec![]);
    let clip = loop_clip(&KEEP_LOOP);
    let Plays::Clip(cp) = clip else {
        unreachable!()
    };
    let key_at = |a: &AudioTracks, bpm: f64| {
        let mut k = None;
        a.clip_copy_keys(&cp, bpm, &mut |_, key| k = Some(key));
        k.unwrap()
    };
    let (k90, k100) = (key_at(&r.a, 90.0), key_at(&r.a, 100.0));
    assert!(r.a.put_copy(copy_for(k90, &audio)).is_ok());
    assert!(r.a.put_copy(copy_for(k100, &audio)).is_ok());
    r.plays.push((START, clip));
    assert!(r.t.command(START, T::PlayStart));
    let c = START + (1.25 * beat_frames(90.0)) as u64;
    r.run(c);
    assert!(r.t.command(c, T::TempoNow(100.0)));
    let pass_end = frame_at(8.0, 1.25, 100.0) as u64;
    let pass100 = (8.0 * beat_frames(100.0)) as u64;
    r.run(pass_end + pass100);
    // In pitch all the way: its copy for 90 drifting to the bar line, its
    // copy for 100 from there, and the next pass from that copy.
    let bar = frame_at(4.0, 1.25, 100.0) as u64;
    for (a, b, what) in [
        (START + 1_000, c - 200, "its copy for 90"),
        (c + 500, bar - 200, "drifting"),
        (bar + 200, pass_end - 200, "its copy for 100"),
        (pass_end + 200, pass_end + pass100 - 500, "the next pass"),
    ] {
        let hz = r.hz(a, b);
        assert!((hz - 441.0).abs() <= 1.0, "{what}: {hz} Hz");
    }
    let tone_step = (0.5 * std::f64::consts::TAU * 441.0 / f64::from(RATE)) as f32;
    for at in [bar, pass_end] {
        let j = r.jump(at - 300, at + 300);
        assert!(j <= 1.5 * tone_step, "at {at}: {j}");
    }
    let st = r.a.stats();
    assert_eq!(st.for_now, 0, "{st:?}");
    assert!(st.drifted > 0 && st.copied > 0, "{st:?}");
}

#[test]
fn a_clips_voice_says_when_it_plays_repitch_for_want_of_its_copy_to_the_end_of_its_pass() {
    // Phase 5c.2: the cell's "repitch now".
    let tk = tone_take();
    let audio = tk.audio.clone();
    let mut r = Rig::new(90.0, tk, vec![]);
    let clip = loop_clip(&LOOP);
    let Plays::Clip(cp) = clip else {
        unreachable!()
    };
    assert_eq!(
        r.a.on_stretch(0, 1),
        copies::OnStretch::default(),
        "not playing"
    );
    r.plays.push((START, clip));
    assert!(r.t.command(START, T::PlayStart));
    r.run(START + beat_frames(90.0) as u64);
    let now = r.a.on_stretch(0, 1);
    assert!(now.repitch && !now.drift, "{now:?}");
    assert_eq!(
        r.a.on_stretch(0, 2),
        copies::OnStretch::default(),
        "another clip"
    );
    // Its copy comes mid-pass: Repitch to the end of it, its copy after.
    let mut k90 = None;
    r.a.clip_copy_keys(&cp, 90.0, &mut |_, k| k90 = Some(k));
    assert!(r.a.put_copy(copy_for(k90.unwrap(), &audio)).is_ok());
    let pass = 8.0 * beat_frames(90.0);
    r.run(START + (0.9 * pass) as u64);
    assert!(r.a.on_stretch(0, 1).repitch, "to the end of its pass");
    r.run(START + (1.1 * pass) as u64);
    assert_eq!(
        r.a.on_stretch(0, 1),
        copies::OnStretch::default(),
        "its copy"
    );
}

#[test]
fn a_clip_drifting_on_keep_pitch_says_so_and_the_bar_it_rejoins_on_once_its_copy_is_here() {
    // Phase 5c.2: the cell's "rejoins 9.1".
    let tk = tone_take();
    let audio = tk.audio.clone();
    let mut r = Rig::new(90.0, tk, vec![]);
    let clip = loop_clip(&KEEP_LOOP);
    let Plays::Clip(cp) = clip else {
        unreachable!()
    };
    let key_at = |a: &AudioTracks, bpm: f64| {
        let mut k = None;
        a.clip_copy_keys(&cp, bpm, &mut |_, key| k = Some(key));
        k.unwrap()
    };
    let (k90, k100) = (key_at(&r.a, 90.0), key_at(&r.a, 100.0));
    assert!(r.a.put_copy(copy_for(k90, &audio)).is_ok());
    r.plays.push((START, clip));
    assert!(r.t.command(START, T::PlayStart));
    let c = START + (1.25 * beat_frames(90.0)) as u64;
    r.run(c);
    assert_eq!(
        r.a.on_stretch(0, 1),
        copies::OnStretch::default(),
        "its copy for 90"
    );
    assert!(r.t.command(c, T::TempoNow(100.0)));
    r.run(frame_at(1.5, 1.25, 100.0) as u64);
    let now = r.a.on_stretch(0, 1);
    assert!(now.drift && now.join.is_none() && !now.repitch, "{now:?}");
    // Its copy for 100 comes: it rejoins on the next bar line, bar 2.
    assert!(r.a.put_copy(copy_for(k100, &audio)).is_ok());
    r.run(frame_at(2.0, 1.25, 100.0) as u64);
    let now = r.a.on_stretch(0, 1);
    assert!(now.drift, "{now:?}");
    assert_eq!(now.join, Some(BAR as f64));
    r.run(frame_at(5.0, 1.25, 100.0) as u64);
    assert_eq!(
        r.a.on_stretch(0, 1),
        copies::OnStretch::default(),
        "rejoined"
    );
}

impl Rig {
    /// The frames over `a..b` that drop to nothing between two loud ones:
    /// a frame of silence where two regions meet.
    fn dropouts(&self, a: u64, b: u64) -> Vec<u64> {
        (a.max(1)..b)
            .filter(|&g| {
                self.l(g).abs() < 1e-4 && self.l(g - 1).abs() > 0.3 && self.l(g + 1).abs() > 0.3
            })
            .collect()
    }
}

#[test]
fn a_looping_copy_goes_round_with_no_frame_missing_after_a_tempo_change() {
    // The tempo turned on a tick whose frame falls on a half: the pass
    // that ends and the one that starts once worked out their frames
    // either side of it (a frame of silence between them).
    let tk = tone_take();
    let audio = tk.audio.clone();
    let mut r = Rig::new(90.0, tk, vec![]);
    let clip = loop_clip(&LOOP);
    let Plays::Clip(cp) = clip else {
        unreachable!()
    };
    let mut k100 = None;
    r.a.clip_copy_keys(&cp, 100.0, &mut |_, k| k100 = Some(k));
    assert!(r.a.put_copy(copy_for(k100.unwrap(), &audio)).is_ok());
    r.plays.push((START, clip));
    assert!(r.t.command(START, T::PlayStart));
    let c = START + (1.25 * beat_frames(90.0)) as u64;
    r.run(c);
    assert!(r.t.command(c, T::TempoNow(100.0)));
    let pass_end = frame_at(8.0, 1.25, 100.0) as u64;
    let pass100 = (8.0 * beat_frames(100.0)) as u64;
    r.run(pass_end + 2 * pass100 + 1_000);
    for p in 1..3 {
        let at = pass_end + p * pass100;
        assert_eq!(
            r.dropouts(at - 300, at + 300),
            Vec::<u64>::new(),
            "pass {p}"
        );
    }
}
