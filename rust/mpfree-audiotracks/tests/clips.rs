//! Audio clips played from the arranger's per-track answer (phase 2,
//! step 2b): launches on their exact frames for any period size, the
//! three joins (straight on, equal gain at a loop seam, equal power
//! elsewhere), a fifth voice's declick, rings to the last frame, a clip
//! that goes away while it fades, edits from the next pass, song mode,
//! mutes and jumps.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s)
//! and 128 ticks exactly 735 frames. The takes copy in before the
//! transport starts, at START.

use mpfree_audiotracks::save::TakeFile;
use mpfree_audiotracks::{
    Answer, AudioTracks, Change, ClipPlay, ClipRegion, Command as A, Plays, RecordingId, FADE,
};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050;
const BT: i64 = 3_840;
const START: u64 = 10_000;

/// The frame of song tick `t` at 240 BPM, playing from START.
fn fr(t: i64) -> u64 {
    START + (t as u64) * 735 / 128
}

/// A different value on every frame (as the tracks tests), one series a
/// take.
fn sig(k: u16, g: u64) -> f32 {
    let g = g + u64::from(k) * 1_000_003;
    ((g * 7919) % 20011) as f32 / 20011.0 - 0.5
}

fn rec(k: u16) -> RecordingId {
    RecordingId(100 + u128::from(k))
}

/// A take of `frames` frames on track 0, `f(j)` its frame j (left,
/// right).
fn take(k: u16, frames: usize, f: impl Fn(u64) -> (f32, f32)) -> TakeFile {
    let mut audio = Vec::with_capacity(2 * frames);
    for j in 0..frames as u64 {
        let (l, r) = f(j);
        audio.push(l);
        audio.push(r);
    }
    TakeFile {
        id: k,
        rec: rec(k),
        track: 0,
        group: k,
        pass: 0,
        bpm: 240.0,
        anchor: 0.0,
        frames,
        channels: 2,
        file: String::new(),
        path: None,
        audio,
    }
}

/// One region of take `k`'s frames from `src`, on clip ticks `s..e`.
fn region(k: u16, s: i64, e: i64, src: i64) -> ClipRegion {
    ClipRegion {
        start: s,
        end: e,
        rec: rec(k),
        src,
        gain: 1.0,
        fade_in: 0,
        fade_out: 0,
        into: 0,
        left: 0,
    }
}

fn clip(regions: &[ClipRegion], id: u32, origin: i64, len: i64) -> ClipPlay<'_> {
    ClipPlay {
        regions,
        origin: origin as f64,
        len,
        last_frame: regions
            .last()
            .map_or(0, |r| r.src + (r.end - r.start) * 735 / 128),
        gain: 1.0,
        clip: id,
        stamp: 1,
    }
}

/// From frame `at`, track 0 plays `plays` (muted, or ringing what it
/// played).
#[derive(Clone, Copy)]
struct Ev<'a> {
    at: u64,
    plays: Plays<'a>,
    muted: bool,
    ring: bool,
}

fn ev(at: u64, plays: Plays) -> Ev {
    Ev {
        at,
        plays,
        muted: false,
        ring: false,
    }
}

struct Rig {
    t: Transport,
    a: AudioTracks,
    b: Block,
    period: usize,
    frame: u64,
    out: Vec<f32>,
}

impl Rig {
    fn new(period: usize, bpm: f64, takes: Vec<TakeFile>) -> Rig {
        let mut r = Rig {
            t: Transport::new(RATE, bpm).unwrap(),
            a: AudioTracks::new(RATE, 20),
            b: Block::new(),
            period,
            frame: 0,
            out: Vec::new(),
        };
        for tk in takes {
            assert!(r.a.load_take(tk).is_ok());
        }
        r.run(START, &[]);
        assert_eq!(r.a.loading(), 0, "the takes copy in before START");
        assert!(r.t.command(START, T::PlayStart));
        r
    }

    /// Runs periods to `frame`, track 0 answering `evs` as the arranger's
    /// player would: what played before the period at its start, the
    /// events in it as changes (the last 4; the rest superseded).
    fn run(&mut self, frame: u64, evs: &[Ev]) {
        let inp = vec![0.0f32; 2 * self.period];
        let mut mix = vec![0.0f32; 2 * self.period];
        while self.frame < frame {
            let n = (self.period as u64).min(frame - self.frame) as usize;
            let p0 = self.frame;
            let before = evs.iter().rev().find(|e| e.at < p0);
            let mut changes: Vec<Change> = evs
                .iter()
                .filter(|e| (p0..p0 + n as u64).contains(&e.at))
                .map(|e| Change {
                    frame: (e.at - p0) as u32,
                    tick: 0.0,
                    plays: e.plays,
                    muted: e.muted,
                    ring: e.ring,
                })
                .collect();
            let superseded = changes.len().saturating_sub(4);
            changes.drain(..superseded);
            let answer = [Answer {
                start: before.map_or(Plays::Song, |e| e.plays),
                muted: before.is_some_and(|e| e.muted),
                changes: &changes,
                superseded: superseded as u32,
            }];
            mix[..2 * n].fill(0.0);
            assert!(self.t.process(n as u32, &mut self.b));
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
    fn r(&self, g: u64) -> f32 {
        self.out[2 * g as usize + 1]
    }
}

fn fade(k: usize) -> f32 {
    ((k as f64 / FADE as f64) * std::f64::consts::FRAC_PI_2).sin() as f32
}

fn noise(k: u16) -> impl Fn(u64) -> (f32, f32) {
    move |j| (sig(k, j), -sig(k, j))
}

#[test]
fn a_launch_lands_on_its_frame_for_any_period() {
    let regions = [region(0, 0, BT, 0)];
    let launch = fr(BT);
    let mut outs = Vec::new();
    for period in [1usize, 7, 128, 1000, 4096] {
        let mut r = Rig::new(period, 240.0, vec![take(0, 3 * BAR as usize, noise(0))]);
        let evs = [ev(launch, Plays::Clip(clip(&regions, 1, BT, BT)))];
        r.run(launch + 2 * BAR, &evs);
        // Silent before, fading in from its frame, then the take exactly.
        assert_eq!(r.l(launch - 1), 0.0, "period {period}");
        assert_eq!(r.l(launch), 0.0);
        for k in 1..FADE as u64 {
            let want = fade(k as usize) * sig(0, k);
            assert!(
                (r.l(launch + k) - want).abs() < 1e-6,
                "period {period} frame {k}"
            );
        }
        for k in FADE as u64..BAR {
            assert_eq!(r.l(launch + k), sig(0, k), "period {period} frame {k}");
            assert_eq!(r.r(launch + k), -sig(0, k));
        }
        outs.push(r.out);
    }
    for o in &outs[1..] {
        assert!(o == &outs[0], "the same output for any period");
    }
}

#[test]
fn two_launches_in_one_period_both_land() {
    let (a, b) = ([region(0, 0, BT, 0)], [region(1, 0, BT, 0)]);
    let takes = || {
        vec![
            take(0, 2 * BAR as usize, noise(0)),
            take(1, 2 * BAR as usize, noise(1)),
        ]
    };
    let run = |period: usize| {
        let mut r = Rig::new(period, 240.0, takes());
        let f = fr(BT);
        let evs = [
            ev(f, Plays::Clip(clip(&a, 1, BT, BT))),
            ev(f + 100, Plays::Clip(clip(&b, 2, BT, BT))),
        ];
        r.run(f + BAR, &evs);
        r
    };
    let one = run(1);
    let big = run(4096);
    assert!(one.out == big.out);
    let f = fr(BT);
    // The second fades in from its own frame over the first's fade out.
    for k in FADE as u64 + 100..BAR {
        assert_eq!(big.l(f + k), sig(1, k), "frame {k}");
    }
    assert!((big.l(f + 101) - (fade(1) * sig(1, 101) + fade(FADE - 1) * sig(0, 101))).abs() < 1e-6);
}

#[test]
fn a_loop_file_cut_to_length_loops_straight_on() {
    // The recording is exactly the pass: it joins straight onto its start.
    let regions = [region(0, 0, BT, 0)];
    let mut r = Rig::new(128, 240.0, vec![take(0, BAR as usize, noise(0))]);
    let f = fr(0);
    r.run(f + 3 * BAR, &[ev(f, Plays::Clip(clip(&regions, 1, 0, BT)))]);
    for k in FADE as u64..3 * BAR {
        assert_eq!(r.l(f + k), sig(0, k % BAR), "frame {k}");
    }
}

/// RMS of the left channel over `n` frames from `g`.
fn rms(r: &Rig, g: u64, n: u64) -> f64 {
    let s: f64 = (g..g + n).map(|j| f64::from(r.l(j)).powi(2)).sum();
    (s / n as f64).sqrt()
}

#[test]
fn a_loop_seam_of_audio_that_repeats_holds_its_level() {
    // 441 Hz: 50 frames a cycle, so the audio past the pass end is its
    // start again. The seam fades with equal gain: the level holds.
    let sine = |j: u64| {
        let v = (j as f64 * std::f64::consts::TAU / 50.0).sin() as f32 * 0.5;
        (v, v)
    };
    let regions = [region(0, 0, BT, 0)];
    let mut r = Rig::new(128, 240.0, vec![take(0, 2 * BAR as usize, sine)]);
    let f = fr(0);
    r.run(
        f + 2 * BAR + 1000,
        &[ev(f, Plays::Clip(clip(&regions, 1, 0, BT)))],
    );
    let steady = rms(&r, f + 1000, 50);
    let seam = f + BAR;
    for g in (seam - 100..seam + 100).step_by(10) {
        let db = 20.0 * (rms(&r, g, 50) / steady).log10();
        assert!(db.abs() < 0.1, "{db} dB at {g}");
    }
    // Sample for sample through it, near enough.
    for k in 0..200 {
        let g = seam - 100 + k;
        assert!((r.l(g) - sine(g - f).0).abs() < 1e-5, "frame {g}");
    }
}

#[test]
fn a_switch_between_unrelated_audio_keeps_its_level() {
    // Take 0 on the left only, take 1 on the right only: each channel
    // shows one voice's gain exactly. Equal power: the gains' squares sum
    // to 1 (within 1 dB) through the fade.
    let left = |_| (0.5, 0.0);
    let right = |_| (0.0, 0.5);
    let (a, b) = ([region(0, 0, BT, 0)], [region(1, 0, BT, 0)]);
    let mut r = Rig::new(
        128,
        240.0,
        vec![
            take(0, 2 * BAR as usize, left),
            take(1, 2 * BAR as usize, right),
        ],
    );
    let f = fr(0);
    let s = fr(1920);
    let evs = [
        ev(f, Plays::Clip(clip(&a, 1, 0, BT))),
        ev(s, Plays::Clip(clip(&b, 2, 1920, BT))),
    ];
    r.run(s + 1000, &evs);
    for k in 0..FADE as u64 {
        let g = s + k;
        let ga = r.l(g) / 0.5;
        let gb = r.r(g) / 0.5;
        let power = ga * ga + gb * gb;
        assert!((0.794..=1.259).contains(&power), "power {power} at {k}");
    }
}

#[test]
fn a_split_in_a_held_note_plays_as_the_whole_region_did() {
    // One region of two bars with long fades, and the same region split
    // inside its fade out (each part keeping its share): sample for sample
    // the same.
    let whole = ClipRegion {
        fade_in: 1000,
        fade_out: 20_000,
        ..region(0, 0, 2 * BT, 0)
    };
    let at = 5760; // 33 075 frames in, inside the fade out
    let split = [
        ClipRegion {
            end: at,
            left: 11_025,
            ..whole
        },
        ClipRegion {
            start: at,
            src: 33_075,
            into: 33_075,
            ..whole
        },
    ];
    let play = |regions: &[ClipRegion]| {
        let mut r = Rig::new(128, 240.0, vec![take(0, 3 * BAR as usize, noise(0))]);
        let f = fr(0);
        r.run(
            f + 2 * BAR + 500,
            &[ev(f, Plays::Clip(clip(regions, 1, 0, 4 * BT)))],
        );
        r.out
    };
    let a = play(&[whole]);
    let b = play(&split);
    assert!(a == b, "the split plays as the whole region");
    assert!(a.iter().any(|&s| s != 0.0));
}

#[test]
fn a_fifth_voice_declicks_with_no_step() {
    // Constant takes, so a voice cut short would show as a step.
    let level = |k: u16| move |_| (0.2 + 0.1 * f32::from(k), 0.0);
    let regions: Vec<[ClipRegion; 1]> = (0..5).map(|k| [region(k, 0, BT, 0)]).collect();
    let takes = (0..5)
        .map(|k| take(k, 2 * BAR as usize, level(k)))
        .collect();
    let mut r = Rig::new(16, 240.0, takes);
    let f = fr(BT);
    let evs: Vec<Ev> = (0..5)
        .map(|k| {
            ev(
                f + 8 * k as u64,
                Plays::Clip(clip(&regions[k], k as u32 + 1, BT, BT)),
            )
        })
        .collect();
    r.run(f + 2000, &evs);
    assert_eq!(r.a.stats().declicked, 1);
    let step = (f..f + 1000)
        .map(|g| (r.l(g + 1) - r.l(g)).abs())
        .fold(0.0, f32::max);
    assert!(step < 0.05, "a step of {step}");
    // Then only the last clip plays.
    assert!((r.l(f + 1500) - 0.6).abs() < 1e-6);
}

#[test]
fn a_clip_gone_while_it_fades_plays_its_fade_and_keeps_its_recording() {
    let regions = [region(0, 0, BT, 0)];
    let mut r = Rig::new(16, 240.0, vec![take(0, 2 * BAR as usize, noise(0))]);
    let f = fr(0);
    let g = fr(1920);
    // The clip names its take; at g it stops (and from then on nothing
    // names it: its regions are gone from the answers).
    r.a.command(A::Keep {
        take: 0,
        keep: true,
    });
    let evs = [
        ev(f, Plays::Clip(clip(&regions, 1, 0, BT))),
        ev(g, Plays::Nothing),
    ];
    r.run(g + 16, &evs);
    // Let go mid-fade: it stays while the voice plays it.
    r.a.command(A::Keep {
        take: 0,
        keep: false,
    });
    r.a.command(A::Forget(0));
    r.run(g + 32, &evs);
    assert!(r.a.take(0).is_some(), "still playing its fade");
    assert_eq!(r.a.stats().refused, 1, "Forget refused while it plays");
    r.run(g + 200, &evs);
    for k in 0..FADE as u64 {
        let want = fade(FADE - k as usize) * sig(0, g - f + k);
        assert!((r.l(g + k) - want).abs() < 1e-6, "frame {k}");
    }
    assert_eq!(r.l(g + FADE as u64 + 1), 0.0);
    assert!(r.a.take(0).is_none(), "forgotten once no voice plays it");
}

#[test]
fn a_once_clip_rings_to_its_last_frame_at_another_tempo() {
    // Made at twice the tempo it plays at: its half bar of ticks is half
    // its audio. The once stop at its length rings: the region plays on to
    // its last frame, and nothing starts again.
    let regions = [region(0, 0, 1920, 0)];
    let mut r = Rig::new(128, 240.0, vec![take(0, BAR as usize, noise(0))]);
    let f = START; // tick 0
    let half = BAR / 2;
    let mut once = clip(&regions, 1, 0, 1920);
    once.last_frame = BAR as i64;
    let evs = [
        ev(f, Plays::Clip(once)),
        Ev {
            ring: true,
            ..ev(f + half, Plays::Nothing)
        },
    ];
    r.run(f + BAR + 1000, &evs);
    for k in FADE as u64..BAR {
        assert_eq!(r.l(f + k), sig(0, k), "frame {k}");
    }
    assert_eq!(r.l(f + BAR + FADE as u64), 0.0);
    assert_eq!(r.l(f + BAR + 500), 0.0, "no second pass");
}

#[test]
fn a_launch_ends_a_ring_with_the_usual_fade() {
    let (a, b) = ([region(0, 0, 1920, 0)], [region(1, 0, BT, 0)]);
    let mut r = Rig::new(
        128,
        240.0,
        vec![
            take(0, BAR as usize, noise(0)),
            take(1, BAR as usize, noise(1)),
        ],
    );
    let f = START;
    let half = BAR / 2;
    let mut once = clip(&a, 1, 0, 1920);
    once.last_frame = BAR as i64;
    // 512 ticks (2 940 frames) after the ring.
    let l = f + half + 2940;
    let evs = [
        ev(f, Plays::Clip(once)),
        Ev {
            ring: true,
            ..ev(f + half, Plays::Nothing)
        },
        ev(l, Plays::Clip(clip(&b, 2, 1920 + 512, BT))),
    ];
    r.run(l + 1000, &evs);
    // Ringing up to the launch, then the crossfade.
    assert_eq!(r.l(l - 1), sig(0, l - 1 - f));
    for k in 1..FADE as u64 {
        let want = fade(FADE - k as usize) * sig(0, l + k - f) + fade(k as usize) * sig(1, k);
        assert!((r.l(l + k) - want).abs() < 1e-6, "frame {k}");
    }
    assert_eq!(r.l(l + 500), sig(1, 500));
}

#[test]
fn a_legato_clip_starts_at_its_place_in_the_pass() {
    // The player gives the pass its origin; launched half a bar in, the
    // clip plays from half way through its audio.
    let regions = [region(0, 0, BT, 0)];
    let mut r = Rig::new(128, 240.0, vec![take(0, 2 * BAR as usize, noise(0))]);
    let s = fr(BT + 1920);
    r.run(s + BAR, &[ev(s, Plays::Clip(clip(&regions, 1, BT, BT)))]);
    for k in FADE as u64..BAR / 2 {
        assert_eq!(r.l(s + k), sig(0, BAR / 2 + k), "frame {k}");
    }
    // Then its next pass from the start (the recording runs on past the
    // pass end: equal gain at the seam).
    assert_eq!(r.l(s + BAR / 2 + 200), sig(0, 200));
}

#[test]
fn a_switch_onto_the_same_audio_running_on_is_not_faded() {
    // Clip 2 holds the same take as clip 1: launched legato over it, its
    // frames carry straight on.
    let a = [region(0, 0, 2 * BT, 0)];
    let b = [region(0, 0, 2 * BT, 0)];
    let mut r = Rig::new(128, 240.0, vec![take(0, 3 * BAR as usize, noise(0))]);
    let f = fr(0);
    let s = fr(1920);
    let evs = [
        ev(f, Plays::Clip(clip(&a, 1, 0, 2 * BT))),
        ev(s, Plays::Clip(clip(&b, 2, 0, 2 * BT))),
    ];
    r.run(s + 1000, &evs);
    for g in s - 10..s + 200 {
        assert_eq!(r.l(g), sig(0, g - f), "frame {}", g - s);
    }
}

#[test]
fn song_and_clip_fade_into_each_other() {
    let a = [region(0, 0, BT, 0)];
    let mut r = Rig::new(
        128,
        240.0,
        vec![
            take(0, 2 * BAR as usize, |_| (0.5, 0.5)),
            take(1, 4 * BAR as usize, |_| (0.3, 0.3)),
        ],
    );
    // Take 1 on the song from tick 0.
    let map = *r.t.tempo_map();
    assert!(r.a.place(1, &[], &map));
    let c = fr(BT);
    let s = fr(2 * BT);
    let evs = [ev(c, Plays::Clip(clip(&a, 1, BT, BT))), ev(s, Plays::Song)];
    r.run(s + 1000, &evs);
    assert!((r.l(c - 1) - 0.3).abs() < 1e-6);
    for k in 0..FADE as u64 {
        let want = 0.3 * fade(FADE - k as usize) + 0.5 * fade(k as usize);
        assert!((r.l(c + k) - want).abs() < 1e-6, "frame {k}");
    }
    assert!((r.l(c + 500) - 0.5).abs() < 1e-6);
    for k in 0..FADE as u64 {
        let want = 0.5 * fade(FADE - k as usize) + 0.3 * fade(k as usize);
        assert!((r.l(s + k) - want).abs() < 1e-6, "frame {k}");
    }
    assert!((r.l(s + 500) - 0.3).abs() < 1e-6);
}

#[test]
fn an_edit_plays_from_the_next_pass_and_a_gain_at_once() {
    let a = [region(0, 0, 1920, 0)];
    let quiet = [ClipRegion { gain: 0.5, ..a[0] }];
    let mut r = Rig::new(128, 240.0, vec![take(0, BAR as usize, |_| (0.8, 0.8))]);
    let f = fr(0);
    let e = fr(960) + 7; // mid pass 0
    let mut edited = clip(&quiet, 1, 0, 1920);
    edited.stamp = 2;
    let evs = [
        ev(f, Plays::Clip(clip(&a, 1, 0, 1920))),
        // The same clip, a new stamp: the player's start from then on.
        ev(e, Plays::Clip(edited)),
    ];
    r.run(fr(1920) + 2000, &evs);
    // Pass 0 as it was, pass 1 the edited regions.
    assert!((r.l(fr(1500)) - 0.8).abs() < 1e-6);
    assert!((r.l(fr(1920) + 500) - 0.4).abs() < 1e-6);

    // A gain change alone glides at once.
    let mut r = Rig::new(128, 240.0, vec![take(0, BAR as usize, |_| (0.8, 0.8))]);
    let mut louder = clip(&a, 1, 0, 1920);
    louder.stamp = 2;
    louder.gain = 0.5;
    let p = f + 5000;
    let evs = [
        ev(f, Plays::Clip(clip(&a, 1, 0, 1920))),
        ev(p, Plays::Clip(louder)),
    ];
    r.run(p + 1000, &evs);
    assert!((r.l(p - 1) - 0.8).abs() < 1e-6);
    assert!((r.l(p + FADE as u64 + 10) - 0.4).abs() < 1e-6);
    let mid = r.l(p + FADE as u64 / 2);
    assert!(mid < 0.8 && mid > 0.4, "{mid}");
}

#[test]
fn a_scene_mute_ramps_on_its_frame() {
    let a = [region(0, 0, BT, 0)];
    let mut r = Rig::new(128, 240.0, vec![take(0, 2 * BAR as usize, |_| (0.5, 0.5))]);
    let f = fr(0);
    let m = fr(1920);
    let evs = [
        ev(f, Plays::Clip(clip(&a, 1, 0, BT))),
        Ev {
            muted: true,
            ..ev(m, Plays::Clip(clip(&a, 1, 0, BT)))
        },
    ];
    r.run(m + 500, &evs);
    assert!((r.l(m - 1) - 0.5).abs() < 1e-6);
    assert!(r.l(m + FADE as u64 / 2) < 0.5);
    assert_eq!(r.l(m + FADE as u64 + 1), 0.0);
}

#[test]
fn a_loop_jump_restarts_the_voices_with_no_step() {
    let a = [region(0, 0, 1920, 0)];
    let mut r = Rig::new(128, 240.0, vec![take(0, BAR as usize, |_| (0.5, 0.5))]);
    assert!(r.t.command(START, T::SetLoop(Some((0, BT)))));
    let f = fr(0);
    r.run(
        f + 2 * BAR + 1000,
        &[ev(f, Plays::Clip(clip(&a, 1, 0, 1920)))],
    );
    let jump = f + BAR;
    let step = (jump - 10..jump + 200)
        .map(|g| (r.l(g + 1) - r.l(g)).abs())
        .fold(0.0, f32::max);
    assert!(step < 0.05, "a step of {step}");
    assert!(
        (r.l(jump + 500) - 0.5).abs() < 1e-6,
        "playing again after the jump"
    );
}

#[test]
fn superseded_changes_are_counted() {
    let regions: Vec<[ClipRegion; 1]> = (0..6).map(|_| [region(0, 0, BT, 0)]).collect();
    let mut r = Rig::new(4096, 240.0, vec![take(0, BAR as usize, noise(0))]);
    let f = fr(0);
    let evs: Vec<Ev> = (0..6)
        .map(|k| {
            ev(
                f + 10 * k as u64,
                Plays::Clip(clip(&regions[k], k as u32, 0, BT)),
            )
        })
        .collect();
    r.run(f + 1000, &evs);
    assert_eq!(r.a.stats().superseded, 2);
}

#[test]
fn a_busy_session_plays_the_same_for_any_period() {
    // Launches closer than a fade (a fifth voice), an edit, mutes, song
    // mode, a ring ended by a launch, and loop jumps.
    let level = |k: u16| move |j: u64| (sig(k, j), 0.1 * f32::from(k));
    let rs: Vec<[ClipRegion; 2]> = (0..5)
        .map(|k| [region(k, 0, 960, 0), region(k, 1280, BT, 3000)])
        .collect();
    let quiet = [
        ClipRegion {
            gain: 0.5,
            ..rs[0][0]
        },
        rs[0][1],
    ];
    let mut outs = Vec::new();
    for period in [1usize, 13, 128, 4096] {
        let takes = (0..5)
            .map(|k| take(k, 3 * BAR as usize, level(k)))
            .collect();
        let mut r = Rig::new(period, 240.0, takes);
        assert!(r.t.command(START, T::SetLoop(Some((0, 4 * BT)))));
        let map = *r.t.tempo_map();
        assert!(r.a.place(4, &[], &map));
        let f = fr(0);
        let c = |k: usize, origin: i64| Plays::Clip(clip(&rs[k], k as u32 + 1, origin, BT));
        let mut edited = clip(&quiet, 1, 0, BT);
        edited.stamp = 2;
        let mut once = clip(&rs[3], 9, 2 * BT, BT);
        once.last_frame = 2 * BAR as i64;
        let mut evs = vec![ev(f, c(0, 0)), ev(f + 3000, Plays::Clip(edited))];
        for k in 1..5 {
            evs.push(ev(fr(BT) + 9 * k as u64, c(k, BT)));
        }
        evs.push(Ev {
            muted: true,
            ..ev(fr(BT) + 5000, c(4, BT))
        });
        evs.push(ev(fr(BT) + 9000, c(4, BT)));
        evs.push(ev(fr(2 * BT), Plays::Clip(once)));
        evs.push(Ev {
            ring: true,
            ..ev(fr(3 * BT), Plays::Song)
        });
        evs.push(ev(fr(3 * BT) + 7000, c(1, 3 * BT)));
        evs.push(ev(fr(4 * BT) + 300, c(2, 0)));
        r.run(fr(4 * BT) + 3 * BAR, &evs);
        assert!(r.a.stats().declicked >= 1);
        outs.push(r.out);
    }
    for (i, o) in outs.iter().enumerate().skip(1) {
        if let Some(j) = (0..o.len()).find(|&j| o[j] != outs[0][j]) {
            panic!(
                "period #{i} differs at frame {}: {} != {}",
                j / 2,
                o[j],
                outs[0][j]
            );
        }
    }
}
