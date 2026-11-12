//! Streaming: the memory as a cache in front of a disk. A fake disk here
//! answers the tracks' requests after a chosen delay, as the host's disk
//! thread would: a recording far longer than the memory is kept whole and
//! plays back sample for sample; comped loop passes play the same as with
//! everything in memory; a loop over the middle of a long take plays every
//! pass; a disk slower than the read-ahead is heard as counted misses, never
//! a stall.

use std::collections::{HashMap, VecDeque};

use mpfree_audiotracks::{AudioTracks, Command as A, Input, Io, CHUNK};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050; // at 240 BPM
const BAR_TICKS: i64 = 3_840;
const START: u64 = 1_000;
const P: u64 = 256;

fn sig(g: u64) -> f32 {
    ((g * 7919) % 20011) as f32 / 20011.0 - 0.5
}

/// A disk: chunks by (take, chunk); reads answered `delay` periods later.
#[derive(Default)]
struct Disk {
    chunks: HashMap<(u16, u32), Vec<f32>>,
    finished: Vec<u16>,
    pending: VecDeque<(u64, u16, u32)>,
    delay: u64,
    reads: u64,
    writes: u64,
}

struct Rig {
    t: Transport,
    a: AudioTracks,
    b: Block,
    frame: u64,
    period: u64,
    out: Vec<f32>,
    live: bool,
    disk: Option<Disk>,
}

impl Rig {
    fn new(seconds: u32, streaming: bool) -> Rig {
        let mut a = AudioTracks::new(RATE, seconds);
        assert!(a.stream(streaming));
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            a,
            b: Block::new(),
            frame: 0,
            period: 0,
            out: Vec::new(),
            live: true,
            disk: streaming.then(Disk::default),
        }
    }

    fn period(&mut self, n: u64) {
        let n = n as usize;
        let (mut inp, mut mix) = ([0.0f32; 2 * P as usize], [0.0f32; 2 * P as usize]);
        for j in 0..n {
            let s = if self.live {
                sig(self.frame + j as u64)
            } else {
                0.0
            };
            inp[2 * j] = s;
            inp[2 * j + 1] = -s;
        }
        self.t.process(n as u32, &mut self.b);
        let map = *self.t.tempo_map();
        self.a
            .process(&self.b, &map, &inp[..2 * n], &mut mix[..2 * n]);
        self.out.extend_from_slice(&mix[..2 * n]);
        self.frame += n as u64;
        self.period += 1;
        self.serve();
    }

    /// The disk thread's part, between periods.
    fn serve(&mut self) {
        let Some(d) = self.disk.as_mut() else {
            return;
        };
        while let Some(r) = self.a.take_io() {
            match r {
                Io::Write { take, chunk } => {
                    let data = self
                        .a
                        .chunk(take, chunk)
                        .expect("a chunk to write")
                        .to_vec();
                    d.chunks.insert((take, chunk), data);
                    d.writes += 1;
                    self.a.written(take, chunk);
                }
                Io::Read { take, chunk } => {
                    d.pending.push_back((self.period + d.delay, take, chunk))
                }
                Io::Finish { take } => d.finished.push(take),
            }
        }
        while d.pending.front().is_some_and(|p| p.0 <= self.period) {
            let (_, take, chunk) = d.pending.pop_front().unwrap();
            d.reads += 1;
            match d.chunks.get(&(take, chunk)) {
                Some(data) => {
                    self.a.fill(take, chunk, data);
                }
                None => self.a.fill_failed(take, chunk),
            }
        }
    }

    fn run_to(&mut self, frame: u64) {
        while self.frame < frame {
            self.period(P.min(frame - self.frame));
        }
    }

    fn l(&self, g: u64) -> f32 {
        self.out[2 * g as usize]
    }
}

/// Record track 0 (mono, left input) from START for `bars` bars.
fn record(r: &mut Rig, bars: u64) {
    r.a.command(A::Arm(0, true));
    r.a.command(A::SetInput(0, Input::Left));
    r.t.command(START, T::Record);
    r.t.command(START + bars * BAR, T::Stop);
    r.run_to(START + bars * BAR + 500);
    r.a.command(A::Arm(0, false));
    r.live = false;
    r.run_to(r.frame + 10);
}

/// Go to tick 0, let the read-ahead settle for `settle` frames, and play
/// `frames`; the first frame played.
fn play_from_start(r: &mut Rig, settle: u64, frames: u64) -> u64 {
    r.t.command(r.frame, T::Locate(0));
    r.run_to(r.frame + settle);
    let p = r.frame + 1;
    r.t.command(p, T::PlayStart);
    r.run_to(p + frames);
    p
}

#[test]
fn a_recording_much_longer_than_the_memory_is_kept_and_plays_back() {
    // 6 s of stereo memory (8 chunks), 40 s of mono recording (27 chunks).
    let mut r = Rig::new(6, true);
    let cap = r.a.memory().capacity;
    record(&mut r, 40);
    let t = r.a.take(0).unwrap();
    assert!(!t.cut_short);
    assert_eq!(r.a.stats().full, 0);
    assert!(t.frames >= 40 * BAR as usize);
    assert!(
        t.frames > 3 * cap,
        "{} frames, memory {cap} samples",
        t.frames
    );
    let d = r.disk.as_ref().unwrap();
    assert_eq!(d.finished, vec![0]);
    assert_eq!(d.writes as usize, t.frames.div_ceil(CHUNK));
    assert_eq!(r.a.unwritten(), 0);
    // Played back from the start: every sample, nothing missed.
    let p = play_from_start(&mut r, 20 * P, 40 * BAR);
    for k in (0..40 * BAR).step_by(997) {
        // Frame k of the region is input frame START + k, crossfaded in
        // over the first FADE frames.
        if k >= 64 {
            assert_eq!(r.l(p + k), sig(START + k), "frame {k}");
        }
    }
    assert_eq!(r.a.stats().missed, 0);
    assert!(r.disk.as_ref().unwrap().reads > 20);
    assert!(r.a.memory().used <= cap);
}

#[test]
fn a_slow_disk_within_the_read_ahead_misses_nothing() {
    let mut r = Rig::new(10, true);
    record(&mut r, 30);
    // Reads come back 2 s late: well inside the 4 s read-ahead (given
    // time after the jump to bar 1 to read past the first chunk).
    r.disk.as_mut().unwrap().delay = 2 * BAR / P;
    let p = play_from_start(&mut r, 3 * BAR, 30 * BAR);
    assert_eq!(r.a.stats().missed, 0);
    assert_eq!(r.l(p + 25 * BAR), sig(START + 25 * BAR));
}

#[test]
fn a_disk_slower_than_the_read_ahead_is_heard_as_counted_misses() {
    let mut r = Rig::new(10, true);
    record(&mut r, 30);
    // Reads come back 8 s late: past the read-ahead and the memory.
    r.disk.as_mut().unwrap().delay = 8 * BAR / P;
    play_from_start(&mut r, 3 * BAR, 30 * BAR);
    let missed = r.a.stats().missed;
    assert!(missed > 0);
    assert!(missed < 30 * BAR, "{missed}");
    assert_eq!(r.a.stats().read_errors, 0);
}

#[test]
fn comped_loop_passes_stream_the_same_as_in_memory() {
    // Three passes of a 4-bar loop, comped bar by bar from passes 1 and 3,
    // in a memory far too small for all three (12 bars of mono: 9 chunks,
    // the memory holds 5).
    let run = |streaming: bool| {
        let mut r = Rig::new(if streaming { 4 } else { 60 }, streaming);
        r.a.command(A::Arm(0, true));
        r.a.command(A::SetInput(0, Input::Left));
        r.t.command(0, T::SetLoop(Some((0, 4 * BAR_TICKS))));
        r.t.command(START, T::Record);
        r.t.command(START + 12 * BAR, T::Stop);
        r.run_to(START + 12 * BAR + 500);
        r.a.command(A::Arm(0, false));
        r.live = false;
        let ids: Vec<u16> = r.a.takes().map(|k| k.id).collect();
        assert_eq!(ids.len(), 3);
        for bar in 0..4 {
            let take = if bar % 2 == 0 { ids[0] } else { ids[2] };
            r.a.command(A::Comp {
                take,
                from: bar * BAR_TICKS,
                to: (bar + 1) * BAR_TICKS,
            });
        }
        r.t.command(r.frame, T::SetLoop(None));
        r.run_to(r.frame + 10);
        let p = play_from_start(&mut r, 20 * P, 4 * BAR);
        assert_eq!(r.a.stats().missed, 0, "streaming {streaming}");
        r.out[2 * p as usize..].to_vec()
    };
    let in_memory = run(false);
    let streamed = run(true);
    assert_eq!(in_memory.len(), streamed.len());
    assert!(in_memory == streamed, "streamed output differs");
    assert!(in_memory.iter().any(|&s| s != 0.0));
}

#[test]
fn a_loop_over_the_middle_of_a_long_take_plays_every_pass() {
    let mut r = Rig::new(8, true);
    record(&mut r, 30);
    // Loop bars 20 to 22 (2 s), far into the take; reads 0.5 s late.
    r.disk.as_mut().unwrap().delay = BAR / 2 / P;
    r.t.command(r.frame, T::SetLoop(Some((20 * BAR_TICKS, 22 * BAR_TICKS))));
    r.t.command(r.frame, T::Locate(20 * BAR_TICKS));
    r.run_to(r.frame + 20 * P);
    let p = r.frame + 1;
    r.t.command(p, T::Play);
    r.run_to(p + 6 * 2 * BAR);
    assert_eq!(r.a.stats().missed, 0);
    // Every pass plays the take's bars 20 and 21.
    for pass in 0..6 {
        let at = p + pass * 2 * BAR + BAR / 2;
        assert_eq!(r.l(at), sig(START + 20 * BAR + BAR / 2), "pass {pass}");
    }
}

#[test]
fn streaming_turns_on_only_with_no_takes() {
    let mut r = Rig::new(4, false);
    record(&mut r, 1);
    assert!(!r.a.stream(true));
    assert!(!r.a.streaming());
    assert_eq!(r.a.take_io(), None);
}
