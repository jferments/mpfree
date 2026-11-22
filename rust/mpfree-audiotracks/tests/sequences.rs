//! Phase 0.4: generated sequences of what a musician does to the audio
//! tracks in their host, from a seeded generator (no test crate): record
//! on any track, edit (comp, split, erase, move, gain), undo and redo, fit
//! a take to a new tempo, save (some saves stopped part way, as a project
//! opening does), open another project or the last save (the tracks
//! emptied while a fit or a save may be under way), and forget takes.
//! After every step: every region plays a take that is there, a track's
//! regions are in order and never overlap, and nothing from before the
//! last open is in the tracks. Every save opens in a fresh host as it was
//! saved, and reopening the last save gives back the same tracks.

use std::time::{Duration, Instant};

use mpfree_audiotracks::host::AudioHost;
use mpfree_audiotracks::save::Saved;
use mpfree_audiotracks::{Command as A, Input, MAX_TRACKS};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050; // at 240 BPM
const BAR_TICKS: i64 = 3_840;
const P: u64 = 256;

/// xorshift64*: the same sequence for a seed on every machine.
struct Gen(u64);

impl Gen {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn pick<T: Copy>(&mut self, of: &[T]) -> T {
        of[self.below(of.len() as u64) as usize]
    }
}

struct Rig {
    t: Transport,
    h: AudioHost,
    b: Block,
    frame: u64,
    live: bool,
}

impl Rig {
    /// Tracks with `seconds` of stereo memory.
    fn new(seconds: u32) -> Rig {
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            h: AudioHost::new(RATE, seconds),
            b: Block::new(),
            frame: 0,
            live: false,
        }
    }

    fn period(&mut self) {
        let n = P as usize;
        let (mut inp, mut mix) = ([0.0f32; 2 * P as usize], [0.0f32; 2 * P as usize]);
        if self.live {
            for j in 0..n {
                let g = self.frame + j as u64;
                let s = ((g * 7919) % 20011) as f32 / 20011.0 - 0.5;
                inp[2 * j] = s;
                inp[2 * j + 1] = -s;
            }
        }
        self.t.process(n as u32, &mut self.b);
        let map = *self.t.tempo_map();
        self.h.period(&self.b, &map, &inp, &mut mix);
        self.frame += P;
    }

    fn run(&mut self, periods: u64) {
        for _ in 0..periods {
            self.period();
        }
    }

    #[track_caller]
    fn run_until(&mut self, what: &str, mut done: impl FnMut(&mut Rig) -> bool) {
        let t0 = Instant::now();
        while !done(self) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out: {what}");
            self.period();
            std::thread::sleep(Duration::from_micros(100));
        }
    }

    /// The tick the transport is at now.
    fn bar_frame(&self) -> u64 {
        self.frame.div_ceil(BAR) * BAR
    }
}

/// What a save holds, without the audio: comparable across hosts.
fn shape(s: &Saved) -> String {
    let mut s = s.clone();
    for k in &mut s.takes {
        k.audio = Vec::new();
        k.file = String::new();
    }
    format!("{:?} {:?}", s.tracks, s.takes)
}

#[track_caller]
fn check(r: &Rig, step: &str) {
    let tracks = &r.h.tracks;
    for t in 0..MAX_TRACKS {
        let regions = tracks.regions(t);
        for (i, g) in regions.iter().enumerate() {
            assert!(g.start < g.end, "{step}: track {t} region {i} {g:?}");
            assert!(
                tracks.take(g.take).is_some(),
                "{step}: track {t} region {i} plays take {} that isn't there",
                g.take
            );
            if i > 0 {
                assert!(
                    regions[i - 1].end <= g.start,
                    "{step}: track {t} regions {} and {i} overlap",
                    i - 1
                );
            }
        }
    }
}

/// Every take of `saved` with its audio whole, and every region of it
/// on a take it has.
#[track_caller]
fn check_saved(s: &Saved, step: &str) {
    for k in &s.takes {
        assert_eq!(
            k.audio.len(),
            k.frames * k.channels,
            "{step}: take {}",
            k.id
        );
    }
    for t in &s.tracks {
        for g in &t.regions {
            assert!(
                s.takes.iter().any(|k| k.id == g.take),
                "{step}: saved track {} plays take {} the save hasn't got",
                t.index,
                g.take
            );
        }
    }
}

/// Saves, running until it is done or, with `stop`, stopping it after
/// that many periods (a project opening over it). The save, when done.
fn save(r: &mut Rig, stop: Option<u64>, step: &str) -> Option<Saved> {
    assert!(r.h.save(), "{step}: a save is already under way");
    if let Some(n) = stop {
        r.run(n);
        if let Some(s) = r.h.take_saved() {
            return Some(s);
        }
        r.h.cancel_save();
        r.run(4);
        assert!(r.h.take_saved().is_none(), "{step}: a stopped save came");
        assert!(r.h.saving().is_none(), "{step}");
        return None;
    }
    let mut out = None;
    r.run_until(step, |r| {
        out = r.h.take_saved();
        out.is_some()
    });
    out
}

/// Opens `part` (or nothing: New) as a project does: the tracks emptied,
/// then the part put back once they are.
fn open(r: &mut Rig, part: Option<&Saved>, step: &str) {
    for t in 0..MAX_TRACKS {
        r.h.command(A::Arm(t, false));
    }
    r.h.cancel_save();
    r.run_until(step, |r| r.h.clear());
    assert_eq!(r.h.tracks.takes().count(), 0, "{step}");
    if let Some(p) = part {
        assert_eq!(r.h.load(p.clone()), Ok(()), "{step}");
        r.run_until(step, |r| r.h.tracks.loading() == 0);
        let back = r.h.tracks.save_file().expect("settled");
        let mut want = p.clone();
        want.takes.sort_by_key(|k| k.id);
        assert_eq!(shape(&back), shape(&want), "{step}: opened as saved");
    }
}

fn run_seed(seed: u64, steps: usize, seconds: u32) -> (usize, usize, u64) {
    let mut g = Gen(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut r = Rig::new(seconds);
    let mut last: Option<Saved> = None;
    let (mut saves, mut opens) = (0, 0);
    for i in 0..steps {
        let op = g.below(12);
        let step = format!("seed {seed} step {i} op {op}");
        match op {
            // Record 1 or 2 bars on a track, mono or stereo.
            0..=2 => {
                let track = g.below(MAX_TRACKS as u64) as usize;
                let bars = 1 + g.below(2);
                let input = g.pick(&[Input::Left, Input::Right, Input::Stereo]);
                r.h.command(A::SetInput(track, input));
                r.h.command(A::Arm(track, true));
                let at = r.bar_frame() + BAR;
                r.t.command(at, T::Locate(g.below(6) as i64 * BAR_TICKS));
                r.t.command(at, T::Record);
                r.t.command(at + bars * BAR, T::Stop);
                r.live = true;
                while r.frame < at + bars * BAR + 600 {
                    r.period();
                }
                r.live = false;
                r.h.command(A::Arm(track, false));
                r.run(4);
            }
            // An edit on a track's song, wherever it lands.
            3 | 4 => {
                let track = g.below(MAX_TRACKS as u64) as usize;
                let at = g.below(8 * BAR_TICKS as u64) as i64;
                let to = at + 1 + g.below(2 * BAR_TICKS as u64) as i64;
                let take = r.h.tracks.takes().map(|k| k.id).max().unwrap_or(0);
                let c = match g.below(5) {
                    0 => A::Comp { take, from: at, to },
                    1 => A::Split { track, at },
                    2 => A::Erase {
                        track,
                        from: at,
                        to,
                    },
                    3 => A::Move { track, at, to },
                    _ => A::SetGain {
                        track,
                        at,
                        gain: 0.5,
                    },
                };
                r.h.command(c);
                r.run(2);
            }
            5 => {
                r.h.command(if g.below(2) == 0 { A::Undo } else { A::Redo });
                r.run(2);
            }
            // Fit a take to another tempo; it may still be under way when
            // a save or an open comes.
            6 => {
                let takes: Vec<u16> = r.h.tracks.takes().map(|k| k.id).collect();
                if !takes.is_empty() {
                    let take = g.pick(&takes);
                    let bpm = g.pick(&[120.0, 180.0, 300.0]);
                    r.h.fit(take, bpm);
                    r.run(g.below(40));
                }
            }
            // Save: whole, or stopped part way.
            7 | 8 => {
                let stop = (g.below(3) == 0).then(|| g.below(6));
                if let Some(s) = save(&mut r, stop, &step) {
                    check_saved(&s, &step);
                    // It opens in a fresh host as it was saved.
                    let mut fresh = Rig::new(30);
                    open(&mut fresh, Some(&s), &step);
                    last = Some(s);
                    saves += 1;
                }
            }
            // Open: New, or the last save, maybe with a fit under way.
            9 | 10 => {
                let part = if g.below(3) == 0 { None } else { last.as_ref() };
                open(&mut r, part, &step);
                opens += 1;
            }
            // Forget the history and a take no region plays.
            _ => {
                r.h.command(A::ClearHistory);
                r.run(2);
                let free: Vec<u16> =
                    r.h.tracks
                        .takes()
                        .map(|k| k.id)
                        .filter(|&id| !r.h.tracks.in_use(id))
                        .collect();
                if !free.is_empty() {
                    let id = g.pick(&free);
                    r.h.command(A::Forget(id));
                    r.run(2);
                    assert!(r.h.tracks.take(id).is_none(), "{step}: take {id} forgotten");
                }
            }
        }
        check(&r, &step);
    }
    // Whatever was left under way finishes; a save now, reopened in place,
    // gives back the same tracks.
    let step = format!("seed {seed} end");
    r.run_until(&step, |r| r.h.fitting().is_none());
    r.run(4);
    let s = save(&mut r, None, &step).expect("whole");
    check_saved(&s, &step);
    let before = shape(&r.h.tracks.save_file().expect("settled"));
    open(&mut r, Some(&s), &step);
    assert_eq!(
        shape(&r.h.tracks.save_file().expect("settled")),
        before,
        "{step}"
    );
    check(&r, &step);
    eprintln!(
        "seed {seed}: {saves} saves, {opens} opens, {} fits ({} failed), {} stale, {} takes now, stats {:?}",
        r.h.fits,
        r.h.fits_failed,
        r.h.stale,
        r.h.tracks.takes().count(),
        r.h.tracks.stats()
    );
    (saves, opens, r.h.stale)
}

#[test]
fn generated_sequences_keep_every_take_a_region_plays_and_open_as_saved() {
    let (mut saves, mut opens, mut stale) = (0, 0, 0);
    for seed in 1..=20 {
        let (s, o, d) = run_seed(seed, 40, 30);
        saves += s;
        opens += o;
        stale += d;
    }
    // The sequences reach what they are for: saves, opens, and an open
    // while a fit was under way.
    assert!(saves >= 20 && opens >= 20, "{saves} saves, {opens} opens");
    assert!(stale > 0, "no fit was under way at an open");
}

/// The same with memory for a few seconds only: recordings that don't fit
/// and fits with no room are refused or cut short, and nothing else
/// changes.
#[test]
fn generated_sequences_with_the_memory_full_lose_nothing_else() {
    for seed in 101..=106 {
        run_seed(seed, 40, 3);
    }
}
