//! Takes played from their files (the audio tracks rethink, phase 3b step
//! 1): a recorded take moves from the pool to its finished file while the
//! song plays, and not one sample of the sound changes; a saved take
//! opened as its file plays exactly as its copy in memory does, through
//! comps, crossfades, fades and gains; a move waits, stopped or playing,
//! until the frames about to play are in the cache, so it never plays
//! silence; frames the stick hasn't brought play silence and are counted,
//! with the take and the tick; a file that doesn't match its take keeps
//! the take in memory and is closed; a file take forgotten or cleared is
//! closed; and generated edits (comps, splits, erases, moves, gains,
//! fades, undo and redo) play the same from the files as from memory.
//!
//! The stick here is the stream layer's coordinator reading real files,
//! answering each read a set number of periods after it was asked (a
//! slow stick, or one that never answers).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use mpfree_audiotracks::save::{wav, TakeFile};
use mpfree_audiotracks::{AudioTracks, Command as A, Input, Io, Monitor};
use mpfree_stream::coord::{serve, Coordinator, Device, HoldTier, Plan, ReadReply, ReadReq};
use mpfree_stream::player::{Config, Player};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050; // at 240 BPM
const BAR_TICKS: i64 = 3_840;
const START: u64 = 1_000;
const P: usize = 256;
/// Never, in periods.
const NEVER: u64 = u64::MAX / 2;

fn sig(g: u64) -> f32 {
    ((g * 7919) % 20011) as f32 / 20011.0 - 0.5
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpfree-at-stream-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A stick that answers each read `delay` of the rig's periods after it
/// was asked.
struct Slow {
    clock: Arc<AtomicU64>,
    delay: u64,
    q: VecDeque<(u64, ReadReply)>,
}

impl Device for Slow {
    fn submit(&mut self, r: ReadReq) {
        let due = self.clock.load(Ordering::Relaxed) + self.delay;
        self.q.push_back((due, serve(r)));
    }
    fn poll(&mut self) -> Option<ReadReply> {
        let now = self.clock.load(Ordering::Relaxed);
        if self.q.front().is_some_and(|(d, _)| *d <= now) {
            return self.q.pop_front().map(|(_, r)| r);
        }
        None
    }
}

struct Rig {
    t: Transport,
    a: AudioTracks,
    c: Option<Coordinator>,
    clock: Arc<AtomicU64>,
    b: Block,
    frame: u64,
    out: Vec<f32>,
    live: bool,
}

impl Rig {
    /// Tracks with 20 s of memory; with a player and its coordinator, on
    /// a stick answering `delay` periods late, if given.
    fn new(delay: Option<u64>) -> Rig {
        let mut a = AudioTracks::new(RATE, 20);
        let clock = Arc::new(AtomicU64::new(0));
        let c = delay.map(|delay| {
            let cfg = Config {
                slots: 128,
                ..Config::default()
            };
            let plan = Plan {
                rate: RATE,
                ..Plan::default()
            };
            let (p, rx, tx) = Player::new(cfg);
            assert!(a.stream(Some(p)).is_none());
            let dev = Slow {
                clock: clock.clone(),
                delay,
                q: VecDeque::new(),
            };
            Coordinator::new(cfg, plan, rx, tx, Box::new(dev))
        });
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            a,
            c,
            clock,
            b: Block::new(),
            frame: 0,
            out: Vec::new(),
            live: true,
        }
    }

    fn period(&mut self) {
        let mut inp = [0.0f32; 2 * P];
        let mut mix = [0.0f32; 2 * P];
        for j in 0..P {
            let g = self.frame + j as u64;
            let s = if self.live { sig(g) } else { 0.0 };
            inp[2 * j] = s;
            inp[2 * j + 1] = -s;
        }
        self.t.process(P as u32, &mut self.b);
        let map = *self.t.tempo_map();
        self.a.process(&self.b, &map, &inp, &mut mix);
        self.out.extend_from_slice(&mix);
        self.frame += P as u64;
        if let Some(c) = &mut self.c {
            c.step();
        }
        self.clock.fetch_add(1, Ordering::Relaxed);
    }

    fn run_to(&mut self, frame: u64) {
        while self.frame < frame {
            self.period();
        }
    }

    fn missed(&self) -> u64 {
        self.a.stats().missed
    }

    /// Write each take to its own file in `dir` and open it as the take's
    /// stream source: (take, its lasting id, the source's generation).
    fn open_files(&mut self, dir: &Path) -> Vec<(u16, mpfree_audiotracks::RecordingId, u32)> {
        let takes: Vec<_> = self.a.takes().collect();
        let mut got = Vec::new();
        for k in takes {
            let path = dir.join(format!("take_{}.wav", k.id));
            let mut audio = vec![0.0f32; k.frames * k.channels];
            assert_eq!(self.a.read_take(k.id, 0, &mut audio), Some(k.frames));
            std::fs::write(&path, wav(&audio, k.channels, RATE)).unwrap();
            let tag = self.c.as_mut().unwrap().open(k.id, &path).unwrap();
            got.push((k.id, k.rec, tag.gen));
        }
        got
    }
}

/// Track 0 (mono) loop-records three passes of a bar, the first comped
/// back in over part of the bar; track 2 (stereo) punches in a bar after
/// it; a region's gain and fades set. Then silence in, stopped.
fn session(r: &mut Rig) {
    r.a.command(A::Arm(0, true));
    r.t.command(0, T::SetLoop(Some((0, BAR_TICKS))));
    r.t.command(START, T::Record);
    r.t.command(START + 3 * BAR, T::Stop);
    r.run_to(START + 3 * BAR + 200);
    r.a.command(A::Arm(0, false));
    r.a.command(A::Arm(2, true));
    r.a.command(A::SetInput(2, Input::Stereo));
    r.a.command(A::SetMonitor(2, Monitor::Off));
    let p = r.frame + 100;
    r.t.command(p, T::SetLoop(None));
    r.t.command(p, T::PlayStart);
    r.t.command(p + BAR, T::Record);
    r.t.command(p + 2 * BAR, T::Stop);
    r.run_to(p + 2 * BAR + 200);
    r.a.command(A::Arm(2, false));
    let first = r.a.takes().next().unwrap().id;
    r.a.command(A::Comp {
        take: first,
        from: 500,
        to: 2500,
    });
    r.a.command(A::SetGain {
        track: 0,
        at: 600,
        gain: 0.8,
    });
    r.a.command(A::SetFades {
        track: 2,
        at: BAR_TICKS + 100,
        fade_in: 900,
        fade_out: 1500,
    });
    r.live = false;
    r.t.command(r.frame + 10, T::StopAndRewind);
    r.run_to(r.frame + 2 * P as u64);
    assert_eq!(r.a.takes().count(), 4);
    assert!(r.a.takes().all(|k| !k.recording));
}

/// Play the song from bar 1 for three bars, from the rig's next period;
/// `at_half` runs half a bar in. Gives the frames played.
fn play_song(r: &mut Rig, mut at_half: impl FnMut(&mut Rig)) -> Vec<f32> {
    let from = r.out.len();
    let p = r.frame;
    r.t.command(p, T::PlayStart);
    r.run_to(p + BAR / 2);
    at_half(r);
    r.run_to(p + 3 * BAR);
    r.t.command(r.frame, T::StopAndRewind);
    r.run_to(r.frame + 4 * P as u64);
    r.out[from..].to_vec()
}

fn loud(out: &[f32]) -> usize {
    out.iter().filter(|v| v.abs() > 1e-6).count()
}

#[test]
fn a_take_moves_to_its_file_while_the_song_plays_and_the_sound_never_changes() {
    let dir = scratch("move");
    let mut want = Rig::new(None);
    session(&mut want);
    let before = play_song(&mut want, |_| {});
    let again = play_song(&mut want, |_| {});
    assert!(loud(&before) > BAR as usize, "the song plays");
    assert_eq!(before, again);

    // About a quarter of a second for every read.
    let mut r = Rig::new(Some(20));
    session(&mut r);
    let used = r.a.memory().used;
    assert!(used > 0);
    let files = r.open_files(&dir);
    let got = play_song(&mut r, |r| {
        for &(take, rec, gen) in &files {
            assert!(r.a.command(A::OnFile { take, rec, gen }));
        }
    });
    assert!(got == before, "the move changed the sound");
    assert_eq!(r.missed(), 0);
    for &(take, ..) in &files {
        assert!(r.a.on_file(take), "take {take} moved");
        assert_eq!(r.a.missed(take), Some((0, 0.0)));
        // Read from its file now, not memory.
        assert_eq!(r.a.read_take(take, 0, &mut [0.0; 4]), None);
    }
    assert_eq!(r.a.memory().used, 0, "the pool is free for recording");
    assert_eq!(r.a.stats().refused, 0);

    // Played again from bar 1, all from the files.
    let got = play_song(&mut r, |_| {});
    assert!(got == again, "played from the files");
    assert_eq!(r.missed(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_move_waits_until_the_frames_at_the_playhead_are_in_the_cache() {
    let dir = scratch("wait");
    let mut want = Rig::new(None);
    session(&mut want);
    let song = play_song(&mut want, |_| {});

    // Each read takes 30 periods (0.35 s): slower than that, the song's
    // short comp regions start before their first frames come, which
    // holding every region's start in the cache answers (step 4).
    let mut r = Rig::new(Some(30));
    session(&mut r);
    let files = r.open_files(&dir);
    for &(take, rec, gen) in &files {
        r.a.command(A::OnFile { take, rec, gen });
    }
    // Stopped at bar 1, where the song's first regions are: nothing
    // moves until their frames have come.
    let p = r.frame;
    r.run_to(p + 25 * P as u64);
    let mut cued = 0;
    for &(take, ..) in &files {
        let k = r.a.take(take).unwrap();
        if r.a.regions(k.track).first().is_some_and(|g| g.take == take) {
            cued += 1;
            assert!(
                !r.a.on_file(take),
                "take {take} moved before its frames came"
            );
        }
    }
    assert_eq!(cued, 2, "a take on each track starts the song");
    r.run_until(|r| files.iter().all(|&(take, ..)| r.a.on_file(take)));
    let got = play_song(&mut r, |_| {});
    assert!(got == song);
    assert_eq!(r.missed(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

impl Rig {
    #[track_caller]
    fn run_until(&mut self, mut done: impl FnMut(&Rig) -> bool) {
        let p = self.frame;
        while !done(self) {
            assert!(self.frame < p + 60 * BAR, "timed out");
            self.period();
        }
    }
}

/// The takes of `r` as a save gives them, each with its file in `dir`.
fn saved(r: &mut Rig, dir: &Path) -> (Vec<TakeFile>, Vec<mpfree_audiotracks::save::TrackFile>) {
    let s = r.a.save_file().unwrap();
    let mut takes = s.takes;
    for k in &mut takes {
        let mut audio = vec![0.0f32; k.frames * k.channels];
        assert_eq!(r.a.read_take(k.id, 0, &mut audio), Some(k.frames));
        let path = dir.join(format!("take_{}.wav", k.id));
        std::fs::write(&path, wav(&audio, k.channels, RATE)).unwrap();
        k.path = Some(path);
        k.audio = audio;
    }
    (takes, s.tracks)
}

#[test]
fn a_saved_take_opened_as_its_file_plays_like_its_copy_in_memory() {
    let dir = scratch("open");
    let mut first = Rig::new(None);
    session(&mut first);
    let (takes, tracks) = saved(&mut first, &dir);

    // Loaded into memory, the way it always was.
    let mut mem = Rig::new(None);
    mem.live = false;
    for k in &takes {
        assert!(mem.a.load_take(k.clone()).is_ok());
    }
    for t in &tracks {
        assert!(mem.a.load_track(t));
    }
    mem.run_until(|r| r.a.loading() == 0);

    // Opened as files: nothing copied in, no memory taken.
    let mut r = Rig::new(Some(3));
    r.live = false;
    for k in &takes {
        let tag =
            r.c.as_mut()
                .unwrap()
                .open(k.id, k.path.as_ref().unwrap())
                .unwrap();
        assert!(r.a.add_file_take(k, tag.gen));
        assert!(r.a.on_file(k.id));
    }
    for t in &tracks {
        assert!(r.a.load_track(t));
    }
    assert_eq!(r.a.memory().used, 0);
    assert_eq!(r.a.loading(), 0);
    let ids = |r: &Rig| r.a.takes().map(|k| (k.id, k.rec)).collect::<Vec<_>>();
    assert_eq!(ids(&r), ids(&mem));
    // Stopped at bar 1 a moment, as after an open: the first regions'
    // frames come.
    r.run_to(r.frame + 30 * P as u64);
    let want = play_song(&mut mem, |_| {});
    let got = play_song(&mut r, |_| {});
    assert!(loud(&want) > BAR as usize);
    assert!(got == want, "a file take plays as its memory copy");
    assert_eq!(r.missed(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn frames_the_stick_hasnt_brought_play_silence_and_are_counted() {
    let dir = scratch("late");
    let mut first = Rig::new(None);
    session(&mut first);
    let (takes, tracks) = saved(&mut first, &dir);
    let mut r = Rig::new(Some(NEVER));
    r.live = false;
    for k in &takes {
        let tag =
            r.c.as_mut()
                .unwrap()
                .open(k.id, k.path.as_ref().unwrap())
                .unwrap();
        assert!(r.a.add_file_take(k, tag.gen));
    }
    for t in &tracks {
        assert!(r.a.load_track(t));
    }
    let got = play_song(&mut r, |_| {});
    assert_eq!(loud(&got), 0, "nothing came, so nothing plays");
    let missed = r.missed();
    assert!(missed > 2 * BAR, "{missed} frames missed");
    let mut sum = 0;
    for k in &takes {
        let (n, tick) = r.a.missed(k.id).unwrap();
        sum += n;
        if n > 0 {
            // Where the song was when it went silent: in the three bars.
            assert!((0.0..3.0 * BAR_TICKS as f64).contains(&tick), "{tick}");
        }
    }
    assert_eq!(sum, missed, "every miss is a take's");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_that_doesnt_match_its_take_keeps_it_in_memory_and_is_closed() {
    let dir = scratch("odd");
    let mut want = Rig::new(None);
    session(&mut want);
    let song = play_song(&mut want, |_| {});

    let mut r = Rig::new(Some(1));
    session(&mut r);
    let k = r.a.takes().next().unwrap();
    // A frame short.
    let mut audio = vec![0.0f32; k.frames - 1];
    r.a.read_take(k.id, 0, &mut audio).unwrap();
    let path = dir.join("short.wav");
    std::fs::write(&path, wav(&audio, 1, RATE)).unwrap();
    let tag = r.c.as_mut().unwrap().open(k.id, &path).unwrap();
    assert!(r.a.command(A::OnFile {
        take: k.id,
        rec: k.rec,
        gen: tag.gen
    }));
    r.run_to(r.frame + 8 * P as u64);
    assert_eq!(r.a.stats().mismatched, 1);
    assert!(!r.a.on_file(k.id));
    assert_eq!(r.a.take_io(), Some(Io::Close { take: k.id }));
    assert_eq!(r.a.take_io(), None);
    let got = play_song(&mut r, |_| {});
    assert!(got == song, "still played from memory");

    // Another take's lasting id, or no player: refused.
    let refused = r.a.stats().refused;
    let other = r.a.takes().nth(1).unwrap();
    r.a.command(A::OnFile {
        take: other.id,
        rec: k.rec,
        gen: tag.gen,
    });
    let mut bare = Rig::new(None);
    session(&mut bare);
    bare.a.command(A::OnFile {
        take: k.id,
        rec: k.rec,
        gen: 1,
    });
    r.run_to(r.frame + P as u64);
    bare.run_to(bare.frame + P as u64);
    assert_eq!(r.a.stats().refused, refused + 1);
    assert_eq!(bare.a.stats().refused, 1);
    assert!(!r.a.on_file(other.id));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_take_forgotten_or_cleared_is_closed() {
    let dir = scratch("close");
    let mut first = Rig::new(None);
    session(&mut first);
    let (takes, _) = saved(&mut first, &dir);
    let mut r = Rig::new(Some(1));
    for k in &takes {
        assert!(r.a.add_file_take(k, 1));
    }
    // No regions play them: the first can go.
    r.a.command(A::Forget(takes[0].id));
    r.run_to(r.frame + P as u64);
    assert_eq!(r.a.take_io(), Some(Io::Close { take: takes[0].id }));
    assert_eq!(r.a.take_io(), None);
    assert!(r.a.take(takes[0].id).is_none());
    assert!(r.a.clear());
    let closed: Vec<_> = std::iter::from_fn(|| r.a.take_io()).collect();
    let want: Vec<_> = takes[1..]
        .iter()
        .map(|k| Io::Close { take: k.id })
        .collect();
    assert_eq!(closed, want);
    // Without a player a take can't open as its file.
    let mut bare = Rig::new(None);
    assert!(!bare.a.add_file_take(&takes[0], 1));
    let _ = std::fs::remove_dir_all(&dir);
}

/// xorshift64*: the same sequence for a seed on every machine.
struct Gen(u64);

impl Gen {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) % n.max(1)
    }

    fn tick(&mut self) -> i64 {
        self.below(4 * BAR_TICKS as u64) as i64
    }
}

/// Half a bar played from song tick `at`, from the rig's next period.
fn play_from(r: &mut Rig, at: i64) -> Vec<f32> {
    let from = r.out.len();
    let p = r.frame;
    r.t.command(p, T::Locate(at));
    r.t.command(p, T::Play);
    r.run_to(p + BAR / 2);
    r.t.command(r.frame, T::Stop);
    r.run_to(r.frame + 2 * P as u64);
    r.out[from..].to_vec()
}

#[test]
fn generated_edits_play_the_same_from_the_files_as_from_memory() {
    for seed in [1u64, 7, 42] {
        let dir = scratch(&format!("gen{seed}"));
        let mut mem = Rig::new(None);
        session(&mut mem);
        let mut r = Rig::new(Some(2));
        session(&mut r);
        let files = r.open_files(&dir);
        // Every frame held in the cache, so this is a test of the read
        // path alone (step 4 holds the regions' starts).
        let parts: Vec<_> = r.a.takes().map(|k| (k.id, 0, k.frames as u64)).collect();
        let c = r.c.as_mut().unwrap();
        c.hold_all(&parts, None, HoldTier::Pinned).unwrap();
        for &(take, rec, gen) in &files {
            r.a.command(A::OnFile { take, rec, gen });
        }
        r.run_until(|r| {
            let p = r.a.player().unwrap();
            files.iter().all(|&(t, ..)| r.a.on_file(t))
                && parts.iter().all(|&(s, f, n)| p.covers(s, f, n))
        });
        let takes: Vec<_> = r.a.takes().map(|k| k.id).collect();
        let mut g = Gen(seed);
        let mut heard = 0;
        for step in 0..40 {
            let track = [0, 2][g.below(2) as usize];
            let (a, b) = (g.tick(), g.tick());
            let (from, to) = (a.min(b), a.max(b) + 1);
            let c = match g.below(8) {
                0 => A::Comp {
                    take: takes[g.below(takes.len() as u64) as usize],
                    from,
                    to,
                },
                1 => A::Split { track, at: a },
                2 => A::Erase { track, from, to },
                3 => A::Move {
                    track,
                    at: a,
                    to: b,
                },
                4 => A::SetGain {
                    track,
                    at: a,
                    gain: g.below(300) as f32 / 100.0,
                },
                5 => A::SetFades {
                    track,
                    at: a,
                    fade_in: g.below(4000) as u32,
                    fade_out: g.below(4000) as u32,
                },
                6 => A::Undo,
                _ => A::Redo,
            };
            mem.a.command(c);
            r.a.command(c);
            let at = g.tick() - BAR_TICKS / 2;
            let want = play_from(&mut mem, at.max(0));
            let got = play_from(&mut r, at.max(0));
            assert!(got == want, "seed {seed}, step {step}: {c:?}");
            heard += usize::from(loud(&want) > 0);
            assert_eq!(mem.a.regions(track), r.a.regions(track));
        }
        assert!(heard > 20, "seed {seed}: {heard} of 40 stretches had sound");
        assert_eq!(r.missed(), 0);
        assert_eq!(mem.a.stats().refused, r.a.stats().refused);
        assert!(takes.iter().all(|&t| r.a.on_file(t)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
