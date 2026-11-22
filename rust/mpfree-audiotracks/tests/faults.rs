//! Fault tests for streaming (the disk streaming plan's suite W, phase
//! P0.5): a fake disk that fails on purpose, and every result checked
//! against the sound that went in, not only the counters.
//!
//! - A short read is a counted error and the chunk is asked for again;
//!   it never plays as silence counted as success.
//! - A read error is counted and the chunk asked for again.
//! - A failed write is tried again; writes that keep failing leave the
//!   take At risk with its audio still in memory, and a failed finish
//!   does too.
//! - A take goes Recording, Saving, Saved.
//! - A reply for a take that was forgotten changes nothing.
//!
//! One test is ignored: it shows a finding the old engine keeps until the
//! shared stream layer replaces it (docs/dev/design/disk-streaming.md).

use std::collections::{HashMap, VecDeque};

use mpfree_audiotracks::{AudioTracks, Command as A, Input, Io, SaveState, CHUNK, WRITE_TRIES};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050; // at 240 BPM
const START: u64 = 1_000;
const P: u64 = 256;
const BAR_TICKS: i64 = 3_840;

fn sig(g: u64) -> f32 {
    ((g * 7919) % 20011) as f32 / 20011.0 - 0.5
}

/// A write (with its samples, copied when asked) or a finish.
enum Job {
    Write(u16, u32, Vec<f32>),
    Finish(u16),
}

/// A disk that fails on purpose. Writes and finishes are done in order,
/// one round a period (an answer's new requests wait for the next);
/// reads are answered `delay` periods later.
#[derive(Default)]
struct Disk {
    chunks: HashMap<(u16, u32), Vec<f32>>,
    pending: VecDeque<(u64, u16, u32)>,
    delay: u64,
    /// Writes and finishes not yet done (a stalled disk holds them).
    stalled: VecDeque<Job>,
    stall_writes: bool,
    /// The next this many reads come back short (half the chunk) / fail.
    short_reads: u32,
    failed_reads: u32,
    /// The next this many writes fail.
    failed_writes: u32,
    /// Finishing a take's file fails.
    fail_finish: bool,
    finished: Vec<u16>,
    /// Writes tried again.
    rewrites: u32,
}

struct Rig {
    t: Transport,
    a: AudioTracks,
    b: Block,
    frame: u64,
    period: u64,
    out: Vec<f32>,
    live: bool,
    disk: Disk,
}

impl Rig {
    fn new(seconds: u32) -> Rig {
        let mut a = AudioTracks::new(RATE, seconds);
        assert!(a.stream(true));
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            a,
            b: Block::new(),
            frame: 0,
            period: 0,
            out: Vec::new(),
            live: true,
            disk: Disk::default(),
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
        let d = &mut self.disk;
        while let Some(r) = self.a.take_io() {
            match r {
                Io::Write { take, chunk } | Io::Rewrite { take, chunk } => {
                    if matches!(r, Io::Rewrite { .. }) {
                        d.rewrites += 1;
                    }
                    let data = self.a.chunk(take, chunk).expect("a chunk").to_vec();
                    d.stalled.push_back(Job::Write(take, chunk, data));
                }
                Io::Read { take, chunk } => {
                    d.pending.push_back((self.period + d.delay, take, chunk))
                }
                Io::Finish { take } => d.stalled.push_back(Job::Finish(take)),
            }
        }
        while !d.stall_writes {
            match d.stalled.pop_front() {
                Some(Job::Write(take, chunk, _)) if d.failed_writes > 0 => {
                    d.failed_writes -= 1;
                    self.a.write_failed(take, chunk);
                }
                Some(Job::Write(take, chunk, data)) => {
                    d.chunks.insert((take, chunk), data);
                    self.a.written(take, chunk);
                }
                Some(Job::Finish(take)) => {
                    d.finished.push(take);
                    self.a.finished(take, !d.fail_finish);
                }
                None => break,
            }
        }
        while d.pending.front().is_some_and(|p| p.0 <= self.period) {
            let (_, take, chunk) = d.pending.pop_front().unwrap();
            match d.chunks.get(&(take, chunk)) {
                Some(_) if d.failed_reads > 0 => {
                    d.failed_reads -= 1;
                    self.a.fill_failed(take, chunk);
                }
                Some(data) if d.short_reads > 0 => {
                    d.short_reads -= 1;
                    self.a.fill(take, chunk, &data[..data.len() / 2]);
                }
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

/// Go to tick 0, let the read-ahead settle, play `frames`; the first frame.
fn play_from_start(r: &mut Rig, frames: u64) -> u64 {
    r.t.command(r.frame, T::Locate(0));
    r.run_to(r.frame + 20 * P);
    let p = r.frame + 1;
    r.t.command(p, T::PlayStart);
    r.run_to(p + frames);
    p
}

/// Every frame played matches what went in (past the first FADE frames).
fn assert_plays_back(r: &Rig, p: u64, bars: u64) {
    for k in 64..bars * BAR {
        let (got, want) = (r.l(p + k), sig(START + k));
        assert_eq!(got, want, "frame {k}");
    }
}

#[test]
fn a_short_read_is_a_counted_error_and_the_chunk_is_asked_for_again() {
    // 6 s of memory, 30 s of recording: playback reads from the disk.
    let mut r = Rig::new(6);
    record(&mut r, 30);
    r.disk.short_reads = 3;
    let p = play_from_start(&mut r, 30 * BAR);
    let s = r.a.stats();
    assert_eq!(s.short_reads, 3, "{s:?}");
    // Each was asked for again in time: every sample plays.
    assert_eq!(s.missed, 0, "{s:?}");
    assert_plays_back(&r, p, 30);
}

#[test]
fn a_read_error_is_counted_and_the_chunk_is_asked_for_again() {
    let mut r = Rig::new(6);
    record(&mut r, 30);
    r.disk.failed_reads = 3;
    let p = play_from_start(&mut r, 30 * BAR);
    let s = r.a.stats();
    assert_eq!(s.read_errors, 3, "{s:?}");
    assert_eq!(s.missed, 0, "{s:?}");
    assert_plays_back(&r, p, 30);
}

#[test]
fn a_take_goes_recording_then_saving_then_saved() {
    let mut r = Rig::new(6);
    r.disk.stall_writes = true;
    r.a.command(A::Arm(0, true));
    r.a.command(A::SetInput(0, Input::Left));
    r.t.command(START, T::Record);
    r.t.command(START + 2 * BAR, T::Stop);
    r.run_to(START + BAR);
    assert_eq!(r.a.save_state(0), Some(SaveState::Recording));
    r.run_to(START + 2 * BAR + 500);
    r.a.command(A::Arm(0, false));
    r.live = false;
    r.run_to(r.frame + 10);
    // Stopped, but the disk holds its writes: not saved yet.
    assert_eq!(r.a.save_state(0), Some(SaveState::Saving));
    r.disk.stall_writes = false;
    r.run_to(r.frame + 10);
    assert_eq!(r.disk.finished, vec![0]);
    assert_eq!(r.a.save_state(0), Some(SaveState::Saved));
    // Not streaming: no disk, so no save state.
    let mut m = AudioTracks::new(RATE, 4);
    assert_eq!(m.save_state(0), None);
    assert!(m.stream(false));
}

#[test]
fn a_failed_write_is_tried_again_and_the_take_is_saved() {
    let mut r = Rig::new(6);
    r.disk.failed_writes = 2;
    record(&mut r, 30);
    let s = r.a.stats();
    assert_eq!(s.write_errors, 2, "{s:?}");
    assert_eq!(r.disk.rewrites, 2);
    assert_eq!(r.a.unwritten(), 0);
    assert_eq!(r.a.save_state(0), Some(SaveState::Saved));
    // Played back from the disk, every sample is there.
    let p = play_from_start(&mut r, 30 * BAR);
    assert_eq!(r.a.stats().missed, 0);
    assert_plays_back(&r, p, 30);
}

#[test]
fn writes_that_keep_failing_leave_the_take_at_risk_with_its_audio_kept() {
    // 20 s of memory, 4 s of recording: it all fits.
    let mut r = Rig::new(20);
    r.disk.failed_writes = 1_000;
    record(&mut r, 4);
    assert_eq!(r.a.save_state(0), Some(SaveState::AtRisk));
    let t = r.a.take(0).unwrap();
    let chunks = t.frames.div_ceil(CHUNK);
    assert_eq!(r.a.unwritten(), chunks, "the unsaved audio stays in memory");
    assert!(!t.cut_short);
    // A few tries for each chunk, not a storm of them.
    let s = r.a.stats();
    assert_eq!(
        s.write_errors,
        chunks as u32 * u32::from(WRITE_TRIES),
        "{s:?}"
    );
    // It plays back, every sample, from memory.
    let p = play_from_start(&mut r, 4 * BAR);
    assert_eq!(r.a.stats().missed, 0);
    assert_plays_back(&r, p, 4);
}

#[test]
fn a_failed_finish_is_tried_again_then_leaves_the_take_at_risk() {
    let mut r = Rig::new(6);
    r.disk.fail_finish = true;
    record(&mut r, 3);
    r.run_to(r.frame + 10 * P);
    assert_eq!(r.disk.finished, vec![0; WRITE_TRIES as usize]);
    assert_eq!(r.a.save_state(0), Some(SaveState::AtRisk));
    assert_eq!(r.a.stats().write_errors, u32::from(WRITE_TRIES));
}

#[test]
fn a_finish_that_fails_once_is_tried_again_and_the_take_is_saved() {
    let mut r = Rig::new(6);
    r.disk.fail_finish = true;
    r.a.command(A::Arm(0, true));
    r.a.command(A::SetInput(0, Input::Left));
    r.t.command(START, T::Record);
    r.t.command(START + 3 * BAR, T::Stop);
    // The first finish fails; the disk is well again for the next.
    while r.disk.finished.is_empty() {
        r.period(P);
    }
    r.disk.fail_finish = false;
    r.run_to(r.frame + 10 * P);
    assert_eq!(r.disk.finished, vec![0, 0]);
    assert_eq!(r.a.save_state(0), Some(SaveState::Saved));
    assert_eq!(r.a.stats().write_errors, 1);
}

#[test]
fn a_reply_for_a_forgotten_take_changes_nothing() {
    let mut r = Rig::new(6);
    record(&mut r, 30);
    // Reads come back 1 s late; forget the take while they are out.
    r.disk.delay = BAR / P;
    r.t.command(r.frame, T::Locate(0));
    r.run_to(r.frame + 4 * P);
    assert!(!r.disk.pending.is_empty(), "reads are out");
    // Nothing plays it, and no undo step could bring it back.
    r.a.command(A::Erase {
        track: 0,
        from: 0,
        to: 40 * BAR_TICKS,
    });
    r.a.command(A::ClearHistory);
    r.a.command(A::Forget(0));
    r.run_to(r.frame + 2 * BAR);
    assert_eq!(r.a.stats().refused, 0);
    assert!(r.a.take(0).is_none());
    let s = r.a.stats();
    assert_eq!((s.read_errors, s.short_reads), (0, 0), "{s:?}");
    assert_eq!(r.a.memory().used, 0, "nothing is held for it");
}

#[test]
#[ignore = "finding 5: replies are matched by take and chunk, and take numbers come back after a project \
            change; fixed by generations in mpfree-stream (P1)"]
fn a_reply_from_before_a_project_change_never_lands_in_the_new_project() {
    let mut r = Rig::new(6);
    record(&mut r, 30);
    // Reads go out for take 0 and are held.
    r.disk.delay = 1_000_000;
    r.t.command(r.frame, T::Locate(0));
    r.run_to(r.frame + 4 * P);
    let (_, take, chunk) = *r.disk.pending.front().expect("a read is out");
    let old = r.disk.chunks[&(take, chunk)].clone();
    // The project changes, and a new take 0 is recorded from other input.
    assert!(r.a.clear());
    r.live = true;
    let at = r.frame + 10;
    r.a.command(A::Arm(0, true));
    r.a.command(A::SetInput(0, Input::Left));
    r.t.command(at, T::Record);
    r.t.command(at + 30 * BAR, T::Stop);
    r.run_to(at + 30 * BAR + 500);
    r.a.command(A::Arm(0, false));
    r.live = false;
    assert!(r.a.take(take).is_some());
    assert_ne!(r.disk.chunks[&(take, chunk)], old);
    // The new take's chunk is asked for, and the old reply arrives first.
    r.t.command(r.frame, T::Locate(0));
    r.run_to(r.frame + 4 * P);
    assert!(
        !r.a.fill(take, chunk, &old),
        "the old project's audio was taken"
    );
}

#[test]
fn every_chunk_written_is_whole() {
    // A write is asked for once a chunk is full, and the last one at stop.
    let mut r = Rig::new(6);
    record(&mut r, 30);
    let t = r.a.take(0).unwrap();
    let chunks = t.frames.div_ceil(CHUNK);
    for c in 0..chunks as u32 {
        let want = (t.frames - c as usize * CHUNK).min(CHUNK);
        assert_eq!(r.disk.chunks[&(0, c)].len(), want, "chunk {c}");
    }
}
