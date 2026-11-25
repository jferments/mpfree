//! Loop clips through the track player (phase 4, step 4b): where the
//! looper is told to play, and one crossfade at the track, on the
//! arranger's frames for any period size.
//!
//! As in the clips tests: 22.05 kHz at 240 BPM in 4/4, a bar exactly
//! 22 050 frames and 128 ticks exactly 735 frames, playing from START.
//! Each loop's buffer holds one value a side (loop `id` plays `id / 4` on
//! the left and minus that on the right), so the output shows its gain.

use mpfree_audiotracks::save::TakeFile;
use mpfree_audiotracks::{
    Answer, AudioTracks, Change, ClipPlay, ClipRegion, LoopPlay, LoopPlayer, Plays, RecordingId,
    FADE,
};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050;
const BT: i64 = 3_840;
const START: u64 = 10_000;

fn fr(t: i64) -> u64 {
    START + (t as u64) * 735 / 128
}

fn fade(k: usize) -> f32 {
    ((k as f64 / FADE as f64) * std::f64::consts::FRAC_PI_2).sin() as f32
}

fn level(id: u128) -> f32 {
    id as f32 / 4.0
}

fn lp(id: u128, origin: i64) -> Plays<'static> {
    Plays::Loop(LoopPlay {
        id,
        origin: origin as f64,
        len: BT,
        clip: id as u32,
        stamp: 1,
    })
}

/// A take of ones on track 0, `frames` long.
fn ones(frames: usize) -> TakeFile {
    TakeFile {
        id: 0,
        rec: RecordingId(100),
        track: 0,
        group: 0,
        pass: 0,
        bpm: 240.0,
        anchor: 0.0,
        frames,
        channels: 2,
        file: String::new(),
        path: None,
        audio: vec![1.0; 2 * frames],
    }
}

const REGION: [ClipRegion; 1] = [ClipRegion {
    start: 0,
    end: BT,
    rec: RecordingId(100),
    src: 0,
    phase: 0.0,
    gain: 1.0,
    fade_in: 0,
    fade_out: 0,
    into: 0,
    left: 0,
}];

fn take_clip(origin: i64) -> Plays<'static> {
    Plays::Clip(ClipPlay {
        regions: &REGION,
        origin: origin as f64,
        len: BT,
        last_frame: BT * 735 / 128,
        gain: 1.0,
        clip: 50,
        stamp: 1,
    })
}

#[derive(Clone, Copy)]
struct Ev {
    at: u64,
    plays: Plays<'static>,
    muted: bool,
}

fn ev(at: u64, plays: Plays<'static>) -> Ev {
    Ev {
        at,
        plays,
        muted: false,
    }
}

/// A loop run as the looper is told it, on absolute frames.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Told {
    id: u128,
    origin: f64,
    from: u64,
    to: u64,
}

struct Rig {
    t: Transport,
    a: AudioTracks,
    loops: LoopPlayer,
    b: Block,
    period: usize,
    frame: u64,
    out: Vec<f32>,
    told: Vec<Told>,
}

impl Rig {
    fn new(period: usize, takes: Vec<TakeFile>) -> Rig {
        let mut r = Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            a: AudioTracks::new(RATE, 20),
            loops: LoopPlayer::new(),
            b: Block::new(),
            period,
            frame: 0,
            out: Vec::new(),
            told: Vec::new(),
        };
        for tk in takes {
            assert!(r.a.load_take(tk).is_ok());
        }
        r.run(START, &[]);
        assert_eq!(r.a.loading(), 0);
        assert!(r.t.command(START, T::PlayStart));
        r
    }

    fn stop_at(&mut self, frame: u64) {
        assert!(self.t.command(frame, T::Stop));
    }

    /// Periods to `frame`, track 0 answering `evs` as the arranger would
    /// (as the clips tests' rig), its loops planned, rendered and mixed.
    fn run(&mut self, frame: u64, evs: &[Ev]) {
        let inp = vec![0.0f32; 2 * self.period];
        let mut mix = vec![0.0f32; 2 * self.period];
        let bufs: Vec<Vec<f32>> = (0..8u128)
            .map(|id| {
                (0..self.period)
                    .flat_map(|_| [level(id), -level(id)])
                    .collect()
            })
            .collect();
        while self.frame < frame {
            let n = (self.period as u64).min(frame - self.frame) as usize;
            let p0 = self.frame;
            assert!(self.t.process(n as u32, &mut self.b));
            let b = &self.b;
            let tick = |g: u64| -> f64 {
                let off = (g - p0) as u32;
                b.spans()
                    .iter()
                    .find(|s| off >= s.offset && off < s.offset + s.len.max(1))
                    .map_or(0.0, |s| s.tick_at(g).round())
            };
            let before = evs.iter().rev().find(|e| e.at < p0);
            let changes: Vec<Change> = evs
                .iter()
                .filter(|e| (p0..p0 + n as u64).contains(&e.at))
                .map(|e| Change {
                    frame: (e.at - p0) as u32,
                    tick: tick(e.at),
                    plays: e.plays,
                    muted: e.muted,
                    ring: false,
                })
                .collect();
            assert!(changes.len() <= 4);
            let answer = [Answer {
                start: before.map_or(Plays::Nothing, |e| e.plays),
                muted: before.is_some_and(|e| e.muted),
                changes: &changes,
                superseded: 0,
            }];
            self.loops.plan(&self.b, &answer);
            for r in self.loops.runs(0) {
                let t = Told {
                    id: r.id,
                    origin: r.origin,
                    from: p0 + u64::from(r.from),
                    to: p0 + u64::from(r.to),
                };
                // A run going straight on from the last period's is one.
                match self
                    .told
                    .iter_mut()
                    .rev()
                    .find(|o| o.id == t.id && o.origin == t.origin && o.to == t.from)
                {
                    Some(o) => o.to = t.to,
                    None => self.told.push(t),
                }
            }
            mix[..2 * n].fill(0.0);
            let map = *self.t.tempo_map();
            let src = |id: u128| bufs.get(id as usize).map(|v| &v[..2 * n]);
            self.a.process_with_loops(
                &self.b,
                &map,
                &inp[..2 * n],
                &mut mix[..2 * n],
                &answer,
                &self.loops,
                &src,
            );
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

const PERIODS: [usize; 5] = [1, 7, 128, 1000, 4096];

#[test]
fn a_loop_opens_on_its_frame_and_the_looper_is_told_from_there() {
    let launch = fr(BT);
    let mut outs = Vec::new();
    for period in PERIODS {
        let mut r = Rig::new(period, vec![]);
        r.run(launch + BAR, &[ev(launch, lp(2, BT))]);
        assert_eq!(r.l(launch - 1), 0.0, "period {period}");
        assert_eq!(r.l(launch), 0.0);
        for k in 1..FADE as u64 {
            let want = fade(k as usize) * level(2);
            assert!((r.l(launch + k) - want).abs() < 1e-6, "period {period} {k}");
        }
        for k in FADE as u64..BAR {
            assert_eq!(r.l(launch + k), level(2), "period {period} {k}");
            assert_eq!(r.r(launch + k), -level(2));
        }
        assert_eq!(
            r.told,
            [Told {
                id: 2,
                origin: BT as f64,
                from: launch,
                to: launch + BAR,
            }],
            "period {period}"
        );
        outs.push(r.out);
    }
    for o in &outs[1..] {
        assert!(o == &outs[0], "the same output for any period");
    }
}

#[test]
fn a_take_clip_and_a_loop_cross_over_once_with_equal_power() {
    let (a, b) = (fr(BT), fr(2 * BT));
    for period in PERIODS {
        let mut r = Rig::new(period, vec![ones(4 * BAR as usize)]);
        // The take clip from bar 2, the loop from bar 3, the clip again
        // from bar 4.
        let evs = [
            ev(a, take_clip(BT)),
            ev(b, lp(4, 2 * BT)),
            ev(fr(3 * BT), take_clip(3 * BT)),
        ];
        r.run(fr(4 * BT), &evs);
        for k in 0..=FADE as u64 {
            let k_ = k as usize;
            let want = fade(FADE - k_) + fade(k_) * level(4);
            assert!((r.l(b + k) - want).abs() < 1e-5, "period {period} into {k}");
            let back = fade(k_) + fade(FADE - k_) * level(4);
            assert!(
                (r.l(fr(3 * BT) + k) - back).abs() < 1e-5,
                "period {period} back {k}"
            );
        }
        assert_eq!(r.l(b + 2 * FADE as u64), level(4));
        // The looper is told the loop from its frame to the end of its
        // fade out, and nothing else.
        assert_eq!(
            r.told,
            [Told {
                id: 4,
                origin: (2 * BT) as f64,
                from: b,
                to: fr(3 * BT) + FADE as u64,
            }],
            "period {period}"
        );
    }
}

#[test]
fn the_tracks_mute_ramps_a_loop_and_it_comes_back_in_place() {
    let (launch, m, u) = (fr(BT), fr(2 * BT), fr(3 * BT));
    for period in PERIODS {
        let mut r = Rig::new(period, vec![]);
        let evs = [
            ev(launch, lp(3, BT)),
            Ev {
                muted: true,
                ..ev(m, lp(3, BT))
            },
            ev(u, lp(3, BT)),
        ];
        r.run(fr(4 * BT), &evs);
        for k in 0..=FADE as u64 {
            let down = (1.0 - k as f32 / FADE as f32) * level(3);
            assert!((r.l(m + k) - down).abs() < 1e-6, "period {period} {k}");
            let up = k as f32 / FADE as f32 * level(3);
            assert!((r.l(u + k) - up).abs() < 1e-6, "period {period} {k}");
        }
        assert_eq!(r.l(m + FADE as u64 + 10), 0.0);
        // One run all through: muted, it ran on from the same origin.
        assert_eq!(
            r.told,
            [Told {
                id: 3,
                origin: BT as f64,
                from: launch,
                to: fr(4 * BT),
            }],
            "period {period}"
        );
    }
}

#[test]
fn two_loops_cross_over_in_two_voices_and_a_third_takes_the_closing_one() {
    let (a, b) = (fr(BT), fr(2 * BT));
    let c = b + 10;
    for period in PERIODS {
        let mut r = Rig::new(period, vec![]);
        r.run(fr(3 * BT), &[ev(a, lp(1, BT)), ev(b, lp(2, 2 * BT))]);
        for k in 0..=FADE as u64 {
            let k_ = k as usize;
            let want = fade(FADE - k_) * level(1) + fade(k_) * level(2);
            assert!((r.l(b + k) - want).abs() < 1e-6, "period {period} {k}");
        }
        assert_eq!(
            r.told,
            [
                Told {
                    id: 1,
                    origin: BT as f64,
                    from: a,
                    to: b + FADE as u64,
                },
                Told {
                    id: 2,
                    origin: (2 * BT) as f64,
                    from: b,
                    to: fr(3 * BT),
                },
            ],
            "period {period}"
        );
        assert_eq!(r.loops.stolen(), 0);
    }
    // A third, ten frames into the fade: it takes the voice closing.
    let mut r = Rig::new(128, vec![]);
    r.run(
        fr(3 * BT),
        &[ev(a, lp(1, BT)), ev(b, lp(2, 2 * BT)), ev(c, lp(3, 2 * BT))],
    );
    assert_eq!(r.loops.stolen(), 1);
    assert!(r.told.iter().any(|t| t.id == 3 && t.from == c));
}

#[test]
fn the_same_loop_from_another_origin_stays_one_voice() {
    let (a, b) = (fr(BT), fr(2 * BT));
    for period in PERIODS {
        let mut r = Rig::new(period, vec![]);
        r.run(fr(3 * BT), &[ev(a, lp(5, BT)), ev(b, lp(5, 0))]);
        // No fade at the switch: the looper crossfades its place.
        for k in 0..2 * FADE as u64 {
            assert_eq!(r.l(b - FADE as u64 + k), level(5), "period {period} {k}");
        }
        assert_eq!(
            r.told,
            [
                Told {
                    id: 5,
                    origin: BT as f64,
                    from: a,
                    to: b,
                },
                Told {
                    id: 5,
                    origin: 0.0,
                    from: b,
                    to: fr(3 * BT),
                },
            ],
            "period {period}"
        );
    }
}

#[test]
fn stopped_nothing_plays_and_the_next_start_is_afresh() {
    let a = fr(BT);
    let stop = fr(2 * BT);
    let mut r = Rig::new(128, vec![]);
    r.stop_at(stop);
    r.run(fr(3 * BT), &[ev(a, lp(6, BT))]);
    assert_eq!(r.told.len(), 1);
    assert_eq!(r.told[0].from, a);
    assert!(r.told[0].to <= stop + 1, "{:?}", r.told);
    assert!(r.out[2 * (stop as usize + 1)..].iter().all(|&s| s == 0.0));
}

#[test]
fn a_loop_back_inside_its_own_fade_reopens_its_voice_from_where_it_was() {
    // Loop 4, a take clip at `b`, loop 4 again 20 frames into the fade
    // (from another origin): one voice, its gain going back up from where
    // the fade had it, and the looper told one run at a time.
    let (a, b) = (fr(BT), fr(2 * BT));
    let c = b + 20;
    let takes = || vec![ones(4 * BAR as usize)];
    for period in PERIODS {
        let mut r = Rig::new(period, takes());
        r.run(
            fr(3 * BT),
            &[ev(a, lp(4, BT)), ev(b, take_clip(2 * BT)), ev(c, lp(4, 0))],
        );
        assert_eq!(r.loops.stolen(), 0, "period {period}");
        // The loop's runs never overlap: the looper plays one place.
        for (i, x) in r.told.iter().enumerate() {
            for y in &r.told[i + 1..] {
                assert!(x.to <= y.from || y.to <= x.from, "{:?}", r.told);
            }
        }
        assert_eq!(
            r.told,
            [
                Told {
                    id: 4,
                    origin: BT as f64,
                    from: a,
                    to: c,
                },
                Told {
                    id: 4,
                    origin: 0.0,
                    from: c,
                    to: fr(3 * BT),
                },
            ],
            "period {period}"
        );
        // The loop's share (the right side is minus it, the take clip's
        // ones are the same on both): continuous through `c`, and all of
        // it once the fade back is done.
        let share = |g: u64| (r.l(g) - r.r(g)) / (2.0 * level(4));
        let at_c = share(c);
        assert!(
            (at_c - fade(FADE - 20)).abs() < 1e-5,
            "period {period} {at_c}"
        );
        assert!((share(c - 1) - fade(FADE - 19)).abs() < 1e-5);
        for k in 0..FADE as u64 {
            assert!(
                share(c + k + 1) >= share(c + k) - 1e-6,
                "period {period} {k}"
            );
        }
        assert!((share(c + FADE as u64 - 20) - 1.0).abs() < 1e-6);
    }
}

/// Listen (phase 4e.2): after the plan, the track named hears what its
/// source gives for `LISTEN`, as it is, on every frame of the period; no
/// other track does, the next plan forgets it, and a track past the last
/// is none.
#[test]
fn listen_is_mixed_into_the_track_named_as_it_is_until_the_next_plan() {
    use mpfree_audiotracks::LISTEN;
    let mut t = Transport::new(RATE, 240.0).unwrap();
    let mut b = Block::new();
    assert!(t.process(64, &mut b), "stopped");
    let mut lp = LoopPlayer::new();
    let buf: Vec<f32> = (0..64)
        .flat_map(|i| [i as f32 / 64.0, -(i as f32) / 64.0])
        .collect();
    let src = |id: u128| (id == LISTEN).then_some(&buf[..]);
    let mix = |lp: &LoopPlayer, t: usize| {
        let mut d = vec![0.25f32; 128];
        lp.mix_into(t, src, &mut d);
        d
    };
    lp.plan(&b, &[]);
    lp.listen(Some(2));
    let want: Vec<f32> = buf.iter().map(|v| 0.25 + v).collect();
    assert_eq!(mix(&lp, 2), want);
    assert!(mix(&lp, 1).iter().all(|&v| v == 0.25), "not another track");
    assert!(lp.runs(2).is_empty(), "no run of a loop's");
    lp.plan(&b, &[]);
    assert!(mix(&lp, 2).iter().all(|&v| v == 0.25), "forgotten");
    lp.listen(Some(10_000));
    assert!((0..20).all(|t| mix(&lp, t).iter().all(|&v| v == 0.25)));
}
