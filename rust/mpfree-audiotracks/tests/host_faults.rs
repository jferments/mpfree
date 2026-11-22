//! Fault tests through the host and its real disk thread (the disk
//! streaming plan's suite W, phase P0.5): the disk's fault knobs make
//! writes, finishes and reads fail on purpose, and a take's file is cut
//! short under it. Each result is checked against the sound that went in.
//!
//! Two tests are ignored: they show findings the old engine keeps until
//! the shared stream layer replaces it (docs/dev/design/disk-streaming.md).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use mpfree_audiotracks::host::{AudioHost, DiskStats};
use mpfree_audiotracks::save::{from_wav, wav_layout};
use mpfree_audiotracks::{Command as A, Input, SaveState, CHUNK, WRITE_TRIES};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050; // at 240 BPM
const START: u64 = 1_000;
const P: u64 = 256;

fn sig(g: u64) -> f32 {
    ((g * 7919) % 20011) as f32 / 20011.0 - 0.5
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpfree-hf-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

struct Rig {
    t: Transport,
    h: AudioHost,
    b: Block,
    frame: u64,
    out: Vec<f32>,
    live: bool,
}

impl Rig {
    fn new(seconds: u32, dir: &Path) -> Rig {
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            h: AudioHost::streaming(RATE, seconds, dir.to_path_buf()).unwrap(),
            b: Block::new(),
            frame: 0,
            out: Vec::new(),
            live: true,
        }
    }

    fn disk(&self) -> &DiskStats {
        self.h.disk_stats().unwrap()
    }

    fn period(&mut self) {
        let n = P as usize;
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
        self.h.period(&self.b, &map, &inp, &mut mix);
        self.out.extend_from_slice(&mix);
        self.frame += P;
        std::thread::sleep(Duration::from_micros(200));
    }

    fn run_to(&mut self, frame: u64) {
        while self.frame < frame {
            self.period();
        }
    }

    #[track_caller]
    fn run_until(&mut self, mut done: impl FnMut(&mut Rig) -> bool) {
        let t0 = Instant::now();
        while !done(self) {
            assert!(t0.elapsed() < Duration::from_secs(60), "timed out");
            self.period();
            std::thread::sleep(Duration::from_micros(300));
        }
    }

    /// Record track 0 (mono, left) from START for `bars` bars.
    fn record(&mut self, bars: u64) {
        self.h.command(A::Arm(0, true));
        self.h.command(A::SetInput(0, Input::Left));
        self.t.command(START, T::Record);
        self.t.command(START + bars * BAR, T::Stop);
        self.run_to(START + bars * BAR + 500);
        self.h.command(A::Arm(0, false));
        self.live = false;
    }

    #[track_caller]
    fn wait_for(&mut self, state: SaveState) {
        self.run_until(|r| r.h.tracks.save_state(0) == Some(state));
    }

    /// Go to the start, let the read-ahead settle, play `frames`.
    fn play_from_start(&mut self, frames: u64) -> Vec<f32> {
        self.t.command(self.frame, T::Locate(0));
        self.run_to(self.frame + 40 * P);
        let p = self.frame + 1;
        self.t.command(p, T::PlayStart);
        self.run_to(p + frames);
        self.out[2 * p as usize..2 * (p + frames) as usize].to_vec()
    }
}

/// Every frame played (past the first crossfade) is the one recorded.
#[track_caller]
fn assert_plays_back(out: &[f32], frames: u64) {
    for k in 64..frames {
        assert_eq!(out[2 * k as usize], sig(START + k), "played frame {k}");
    }
}

/// The take's file holds every frame recorded, and its header says so.
#[track_caller]
fn assert_file_whole(r: &Rig) {
    let t = r.h.tracks.take(0).unwrap();
    let (audio, ch, rate) = from_wav(&std::fs::read(r.h.take_file(0).unwrap()).unwrap()).unwrap();
    assert_eq!((ch, rate, audio.len()), (1, RATE, t.frames));
    for (k, &v) in audio.iter().enumerate() {
        assert_eq!(v, sig(START + k as u64), "file frame {k}");
    }
}

#[test]
fn a_write_that_fails_is_tried_again_into_the_take_s_own_file() {
    let dir = scratch("rewrite");
    let mut r = Rig::new(6, &dir);
    // The first write is chunk 0's: tried again, it must not start a new
    // file (a take's chunk 0 is what names its file).
    r.disk().fail_writes.store(1, Ordering::Relaxed);
    r.record(30);
    r.wait_for(SaveState::Saved);
    assert_eq!(r.h.tracks.stats().write_errors, 1);
    assert_eq!(r.h.tracks.unwritten(), 0);
    assert_file_whole(&r);
    let wavs = std::fs::read_dir(&dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "wav")
        })
        .count();
    assert_eq!(wavs, 1, "one take, one file");
    let out = r.play_from_start(30 * BAR);
    assert_eq!(r.h.tracks.stats().missed, 0);
    assert_plays_back(&out, 30 * BAR);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn writes_that_keep_failing_leave_the_take_at_risk_and_it_still_plays() {
    let dir = scratch("at-risk");
    // 20 s of memory, 4 s of recording: it all fits.
    let mut r = Rig::new(20, &dir);
    r.disk().fail_writes.store(u64::MAX, Ordering::Relaxed);
    r.record(4);
    r.wait_for(SaveState::AtRisk);
    let t = r.h.tracks.take(0).unwrap();
    let chunks = t.frames.div_ceil(CHUNK);
    r.run_until(|r| r.h.tracks.stats().write_errors == chunks as u32 * u32::from(WRITE_TRIES));
    assert_eq!(r.h.tracks.unwritten(), chunks, "kept in memory");
    let out = r.play_from_start(4 * BAR);
    assert_eq!(r.h.tracks.stats().missed, 0);
    assert_plays_back(&out, 4 * BAR);
    // Still at risk, and no more tries than that.
    assert_eq!(r.h.tracks.save_state(0), Some(SaveState::AtRisk));
    assert_eq!(
        r.h.tracks.stats().write_errors,
        chunks as u32 * u32::from(WRITE_TRIES)
    );
    r.disk().fail_writes.store(0, Ordering::Relaxed);
    drop(r);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_finish_that_fails_once_is_tried_again_and_the_header_is_right() {
    let dir = scratch("finish");
    let mut r = Rig::new(6, &dir);
    r.disk().fail_finishes.store(1, Ordering::Relaxed);
    r.record(12);
    r.wait_for(SaveState::Saved);
    assert_eq!(r.h.tracks.stats().write_errors, 1);
    assert_file_whole(&r);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_finish_that_keeps_failing_leaves_the_take_at_risk() {
    let dir = scratch("finish-risk");
    let mut r = Rig::new(6, &dir);
    r.disk().fail_finishes.store(u64::MAX, Ordering::Relaxed);
    r.record(3);
    r.wait_for(SaveState::AtRisk);
    assert_eq!(
        r.h.tracks.stats().write_errors,
        u32::from(WRITE_TRIES),
        "each failed finish counted, no more tries than that"
    );
    r.disk().fail_finishes.store(0, Ordering::Relaxed);
    drop(r);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_short_read_is_asked_for_again_and_every_sample_plays() {
    let dir = scratch("short");
    let mut r = Rig::new(6, &dir);
    r.record(30);
    r.wait_for(SaveState::Saved);
    r.disk().short_reads.store(3, Ordering::Relaxed);
    let out = r.play_from_start(30 * BAR);
    let s = r.h.tracks.stats();
    assert_eq!(s.short_reads, 3, "{s:?}");
    assert_eq!(s.missed, 0, "{s:?}");
    assert_plays_back(&out, 30 * BAR);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_take_file_cut_short_is_heard_as_counted_misses_never_as_silence_that_passes() {
    let dir = scratch("cut");
    let mut r = Rig::new(6, &dir);
    r.record(30);
    r.wait_for(SaveState::Saved);
    // Cut the file to its first 10 bars, under the open file.
    let file = r.h.take_file(0).unwrap();
    let keep = 10 * BAR;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_len(44 + keep * 4)
        .unwrap();
    let out = r.play_from_start(30 * BAR);
    let s = r.h.tracks.stats();
    assert!(s.short_reads > 0, "{s:?}");
    // What is gone was heard as silence, and counted as missed.
    assert!(s.missed > 0, "{s:?}");
    // What the file still has plays as recorded, up to the chunk it was
    // cut in.
    let whole = keep / CHUNK as u64 * CHUNK as u64;
    assert_plays_back(&out, whole);
    drop(r);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "finding 7: a take's file gets its header and is synced only when it finishes; fixed by the \
            checkpoint records in mpfree-stream (P1)"]
fn a_recording_in_progress_has_a_file_that_says_how_much_is_in_it() {
    let dir = scratch("in-progress");
    let mut r = Rig::new(6, &dir);
    r.h.command(A::Arm(0, true));
    r.h.command(A::SetInput(0, Input::Left));
    r.t.command(START, T::Record);
    r.run_to(START + 20 * BAR);
    // Still recording: a power cut now should leave a file that can be
    // found and read up to the last chunk written.
    r.run_until(|r| r.disk().writes.load(Ordering::Relaxed) >= 10);
    let b = std::fs::read(r.h.take_file(0).unwrap()).unwrap();
    let (_, len, _, _) = wav_layout(&b).unwrap();
    assert!(len >= 10 * CHUNK as u64 * 4, "the header says {len} bytes");
    drop(r);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "finding 6: streaming refuses a sound brought in (ImportRefused::Streaming); fixed by \
            imported audio as files in mpfree-stream (P1)"]
fn a_sound_brought_in_while_streaming_gets_a_file_of_its_own() {
    let dir = scratch("import");
    let mut r = Rig::new(6, &dir);
    let audio: Vec<f32> = (0..2 * BAR).map(sig).collect();
    assert!(r.h.import(0, 0, 1, audio, Vec::new(), 0.0).is_ok());
    drop(r);
    let _ = std::fs::remove_dir_all(&dir);
}
