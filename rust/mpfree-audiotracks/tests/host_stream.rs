//! The host streaming from real take files: a recording far longer than
//! the memory lands in its file whole and plays back from it; a save names
//! the files and opens in a fresh host, which plays the same; a missing
//! file is reported, not hidden; a disk that stalls past the read-ahead is
//! heard as counted misses while the audio loop goes on; a fit and a
//! waveform work from the files.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use mpfree_audiotracks::host::AudioHost;
use mpfree_audiotracks::save::{from_text, from_wav, to_text};
use mpfree_audiotracks::{Command as A, Input};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050; // at 240 BPM
const BAR_TICKS: i64 = 3_840;
const START: u64 = 1_000;
const P: u64 = 256;

fn sig(g: u64) -> f32 {
    ((g * 7919) % 20011) as f32 / 20011.0 - 0.5
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpfree-at-{name}-{}", std::process::id()));
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
    /// Wall-clock time given to the disk thread each period.
    pace: Duration,
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
            pace: Duration::from_micros(200),
        }
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
        if !self.pace.is_zero() {
            std::thread::sleep(self.pace);
        }
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

    /// Record track 0 (mono, left) from START for `bars` bars, and wait
    /// until every chunk is on the disk.
    fn record(&mut self, bars: u64) {
        self.h.command(A::Arm(0, true));
        self.h.command(A::SetInput(0, Input::Left));
        self.t.command(START, T::Record);
        self.t.command(START + bars * BAR, T::Stop);
        self.run_to(START + bars * BAR + 500);
        self.h.command(A::Arm(0, false));
        self.live = false;
        self.run_until(|r| r.h.tracks.unwritten() == 0 && r.h.save());
        // (save() above only to wait for the files to settle.)
        self.run_until(|r| r.h.take_saved().is_some());
    }

    fn play_from_start(&mut self, frames: u64) -> Vec<f32> {
        self.t.command(self.frame, T::Locate(0));
        self.run_to(self.frame + 40 * P);
        let p = self.frame + 1;
        self.t.command(p, T::PlayStart);
        self.run_to(p + frames);
        self.out[2 * p as usize..2 * (p + frames) as usize].to_vec()
    }
}

#[test]
fn a_long_recording_lands_in_its_file_and_plays_back_from_it() {
    let dir = scratch("long");
    let mut r = Rig::new(6, &dir);
    let cap = r.h.tracks.memory().capacity;
    r.record(40);
    let t = r.h.tracks.take(0).unwrap();
    assert!(!t.cut_short);
    assert!(t.frames > 3 * cap);
    let file = r.h.take_file(0).unwrap();
    let (audio, ch, rate) = from_wav(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!((ch, rate, audio.len()), (1, RATE, t.frames));
    for k in (0..t.frames).step_by(1013) {
        assert_eq!(audio[k], sig(START + k as u64), "file frame {k}");
    }
    assert!(Path::new(&format!("{}.peaks", file.display())).exists());
    let out = r.play_from_start(40 * BAR);
    for k in (64..40 * BAR as usize).step_by(997) {
        assert_eq!(out[2 * k], sig(START + k as u64), "played frame {k}");
    }
    assert_eq!(r.h.tracks.stats().missed, 0);
    let d = r.h.disk_stats().unwrap();
    assert!(d.reads.load(Ordering::Relaxed) > 20);
    assert_eq!(d.errors.load(Ordering::Relaxed), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_save_names_the_files_and_opens_in_a_fresh_host() {
    let dir = scratch("save");
    let mut r = Rig::new(6, &dir);
    r.record(20);
    r.h.command(A::Comp {
        take: 0,
        from: 2 * BAR_TICKS,
        to: 3 * BAR_TICKS,
    });
    r.h.command(A::SetLevel(0, 0.5));
    assert!(r.h.save());
    let mut saved = None;
    r.run_until(|r| {
        saved = r.h.take_saved();
        saved.is_some()
    });
    let saved = saved.unwrap();
    assert_eq!(saved.takes.len(), 1);
    assert!(
        saved.takes[0].audio.is_empty(),
        "the audio stays in its file"
    );
    assert!(dir.join(&saved.takes[0].file).exists());
    let text = to_text(&saved);
    let before = r.play_from_start(20 * BAR);

    let mut s = Rig::new(6, &dir);
    s.live = false;
    assert!(s.h.load(from_text(&text).unwrap()).is_ok());
    s.run_until(|s| s.h.opening() == 0);
    assert!(s.h.open_errors.is_empty(), "{:?}", s.h.open_errors);
    let after = s.play_from_start(20 * BAR);
    assert!(before == after, "the opened save plays differently");
    assert_eq!(s.h.tracks.stats().missed, 0);

    // A save whose file is gone: said so, and the rest still loads.
    let mut gone = from_text(&text).unwrap();
    gone.takes[0].file = "not-there.wav".into();
    let mut u = Rig::new(6, &dir);
    assert!(u.h.load(gone).is_ok());
    u.run_until(|u| u.h.opening() == 0);
    assert_eq!(u.h.open_errors.len(), 1);
    assert!(
        u.h.open_errors[0].contains("not-there.wav"),
        "{:?}",
        u.h.open_errors
    );
    assert!(u.h.tracks.take(0).is_none());
    assert_eq!(u.h.tracks.regions(0).len(), saved.tracks[0].regions.len());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_stalling_disk_is_heard_as_counted_misses_while_the_loop_goes_on() {
    let dir = scratch("stall");
    let mut r = Rig::new(6, &dir);
    r.record(30);
    // Every disk call now takes 5 s: far past the read-ahead.
    r.h.disk_stats()
        .unwrap()
        .delay_us
        .store(5_000_000, Ordering::Relaxed);
    r.pace = Duration::ZERO;
    let t0 = Instant::now();
    r.t.command(r.frame, T::Locate(20 * BAR_TICKS));
    let p = r.frame + 1;
    r.t.command(p, T::Play);
    r.run_to(p + 8 * BAR);
    // The periods never waited for the disk.
    assert!(t0.elapsed() < Duration::from_secs(4), "{:?}", t0.elapsed());
    let missed = r.h.tracks.stats().missed;
    assert!(missed > 0);
    // At most every frame played (to the end of the last period).
    assert!(missed <= 8 * BAR + P, "{missed} of {}", 8 * BAR);
    r.h.disk_stats()
        .unwrap()
        .delay_us
        .store(0, Ordering::Relaxed);
    drop(r);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_fit_and_a_waveform_work_from_the_files() {
    let dir = scratch("fit");
    let mut r = Rig::new(6, &dir);
    r.record(12);
    // A waveform from the disk side's table, against the file itself.
    assert!(r.h.wave(0, 50));
    let mut w = None;
    r.run_until(|r| {
        w = r.h.take_wave();
        w.is_some()
    });
    let w = w.unwrap();
    let (audio, _, _) = from_wav(&std::fs::read(r.h.take_file(0).unwrap()).unwrap()).unwrap();
    assert_eq!(w.columns.len(), audio.len().div_ceil(w.frames_per_column));
    assert!(w.columns.len() <= 50);
    for (c, &(lo, hi)) in w.columns.iter().enumerate() {
        let a = c * w.frames_per_column;
        let b = (a + w.frames_per_column).min(audio.len());
        let l = audio[a..b].iter().copied().fold(f32::MAX, f32::min);
        let h = audio[a..b].iter().copied().fold(f32::MIN, f32::max);
        assert_eq!((lo, hi), (l, h), "column {c}");
    }
    // Fit to half the tempo: read from the file, stretched, streamed into
    // a new file, swapped in.
    r.t.command(r.frame, T::SetTempo(120.0));
    r.run_to(r.frame + 10);
    assert!(r.h.fit(0, 120.0));
    assert_eq!(r.h.fitting(), Some(0));
    r.run_until(|r| r.h.fits + r.h.fits_failed > 0);
    assert_eq!((r.h.fits, r.h.fits_failed), (1, 0));
    r.run_to(r.frame + 10);
    let new = r.h.tracks.regions(0)[0].take;
    assert_ne!(new, 0);
    r.run_until(|r| r.h.tracks.unwritten() == 0 && r.h.save());
    r.run_until(|r| r.h.take_saved().is_some());
    let f = r.h.take_file(new).unwrap();
    assert_ne!(f, r.h.take_file(0).unwrap());
    let (stretched, _, _) = from_wav(&std::fs::read(&f).unwrap()).unwrap();
    assert_eq!(stretched.len(), r.h.tracks.take(new).unwrap().frames);
    assert!((stretched.len() as f64 / audio.len() as f64 - 2.0).abs() < 0.01);
    std::fs::remove_dir_all(&dir).unwrap();
}
