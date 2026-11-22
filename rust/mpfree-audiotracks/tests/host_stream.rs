//! Playing takes from their files in the host (the audio tracks rethink,
//! phase 3b step 2): with streaming on, a recorded take's file is opened
//! once the writer has finished it and the take moves to it, its memory
//! freed, the song sounding exactly as it does from memory; a project's
//! takes open as their files with nothing copied in; a waveform or a fit
//! of a take on its file reads the file and gives what it gives from
//! memory; a project take whose file is missing is left out and said,
//! with its regions; a project's tracks wait for its takes' files;
//! clearing closes the files, and a project opens again after; streaming
//! can't start once recording to files has.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mpfree_audiotracks::files::{FileEvent, TakeState, WriterConfig};
use mpfree_audiotracks::host::AudioHost;
use mpfree_audiotracks::save::Saved;
use mpfree_audiotracks::streaming::StreamConfig;
use mpfree_audiotracks::Command as A;
use mpfree_stream::player::Config;
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050; // at 240 BPM
const P: u64 = 256;
/// Where every rig starts recording, the same frame in each.
const START: u64 = 8 * BAR;

fn sig(g: u64, c: usize) -> f32 {
    let v = ((g * 7919 + c as u64 * 104_729) % 20_011) as f32 / 20_011.0 - 0.5;
    if c == 0 {
        v
    } else {
        -v * 0.75
    }
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpfree-hs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn quick() -> WriterConfig {
    WriterConfig {
        interval_s: 0.05,
        max_interval_s: 0.2,
        commit_s: 0.01,
        ..WriterConfig::default()
    }
}

fn small() -> StreamConfig {
    StreamConfig {
        cache: Config {
            slots: 256,
            ..Config::default()
        },
        ..StreamConfig::default()
    }
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
    /// A host recording to files in `dir`, with takes played from their
    /// files if `stream`.
    fn new(dir: Option<&Path>, stream: bool) -> Rig {
        let mut h = AudioHost::new(RATE, 30);
        if stream {
            assert!(h.stream(small()));
        }
        if let Some(dir) = dir {
            assert!(h.record_to(dir.to_path_buf(), quick()).is_ok());
        }
        let mut r = Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            h,
            b: Block::new(),
            frame: 0,
            out: Vec::new(),
            live: true,
        };
        if stream {
            r.run_until(|r| r.h.tracks.player().is_some());
        }
        r
    }

    fn period(&mut self) {
        let n = P as usize;
        let (mut inp, mut mix) = ([0.0f32; 2 * P as usize], [0.0f32; 2 * P as usize]);
        for j in 0..n {
            let g = self.frame + j as u64;
            if self.live {
                inp[2 * j] = sig(g, 0);
                inp[2 * j + 1] = sig(g, 1);
            }
        }
        self.t.process(n as u32, &mut self.b);
        let map = *self.t.tempo_map();
        self.h.period(&self.b, &map, &inp, &mut mix);
        self.out.extend_from_slice(&mix);
        self.frame += P;
    }

    fn run_to(&mut self, frame: u64) {
        while self.frame < frame {
            self.period();
        }
    }

    /// Periods until `done`, giving the threads time.
    #[track_caller]
    fn run_until(&mut self, mut done: impl FnMut(&mut Rig) -> bool) {
        let t0 = Instant::now();
        while !done(self) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out");
            self.period();
            std::thread::sleep(Duration::from_micros(300));
        }
    }

    fn events(&mut self) -> Vec<FileEvent> {
        std::iter::from_fn(|| self.h.take_event()).collect()
    }

    /// Track 0 (mono) and track 1 (stereo) record two bars from
    /// [`START`], then disarm; until their files are finished.
    fn record(&mut self, files: bool) {
        self.h.command(A::Arm(0, true));
        self.h.command(A::Arm(1, true));
        self.h
            .command(A::SetInput(1, mpfree_audiotracks::Input::Stereo));
        if files {
            self.run_until(|r| r.h.armed_file(0) == Some(true) && r.h.armed_file(1) == Some(true));
        }
        assert!(self.frame < START - P);
        self.t.command(START, T::Record);
        self.t.command(START + 2 * BAR, T::Stop);
        self.run_to(START + 2 * BAR + 4 * P);
        self.h.command(A::Arm(0, false));
        self.h.command(A::Arm(1, false));
        self.live = false;
        if files {
            self.run_until(|r| {
                r.h.tracks
                    .takes()
                    .all(|k| r.h.file(k.id).is_some_and(|f| f.state == TakeState::Saved))
            });
        }
        assert_eq!(self.h.tracks.takes().count(), 2);
    }

    /// Three bars from bar 1, from the next period. The periods run far
    /// faster than the song would play, so first, stopped at bar 1, every
    /// take on its file is read into the cache (as it would be well
    /// before its frames played, in time).
    fn play_song(&mut self) -> Vec<f32> {
        self.t.command(self.frame, T::Locate(0));
        self.run_until(|r| {
            let t = &r.h.tracks;
            t.takes().all(|k| {
                !t.on_file(k.id)
                    || t.player()
                        .is_some_and(|p| p.covers(k.id, 0, k.frames as u64))
            })
        });
        let from = self.out.len();
        let p = self.frame;
        self.t.command(p, T::PlayStart);
        self.run_to(p + 3 * BAR);
        self.t.command(self.frame, T::StopAndRewind);
        self.run_to(self.frame + 4 * P);
        self.out[from..].to_vec()
    }

    /// A save, as the app takes it.
    fn save(&mut self) -> Saved {
        assert!(self.h.save());
        let mut saved = None;
        self.run_until(|r| {
            saved = r.h.take_saved();
            saved.is_some()
        });
        saved.unwrap()
    }

    fn all_on_file(&self) -> bool {
        let t = &self.h.tracks;
        t.takes().count() > 0 && t.takes().all(|k| t.on_file(k.id))
    }
}

/// The song from memory: the same takes, recorded at the same frames,
/// never on files.
fn from_memory() -> (Rig, Vec<f32>) {
    let mut r = Rig::new(None, false);
    r.record(false);
    let song = r.play_song();
    assert!(song.iter().filter(|v| v.abs() > 1e-6).count() > BAR as usize);
    (r, song)
}

#[test]
fn a_recorded_take_moves_to_its_file_and_frees_its_memory() {
    let dir = scratch("move");
    let (_, want) = from_memory();
    let mut r = Rig::new(Some(&dir), true);
    r.record(true);
    r.run_until(|r| r.all_on_file());
    assert_eq!(
        r.h.tracks.memory().used,
        0,
        "the pool is free for recording"
    );
    let got = r.play_song();
    assert!(got == want, "played from its files, the song is the same");
    assert_eq!(r.h.tracks.stats().missed, 0);
    // A save still copies each file whole.
    let saved = r.save();
    for k in &saved.takes {
        let path = k.path.as_ref().expect("copied whole from its file");
        assert!(path.starts_with(&dir));
        assert!(k.audio.is_empty());
    }
    assert!(r.events().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_project_opens_its_takes_as_files_with_nothing_copied_in() {
    let dir = scratch("open");
    let (_, want) = from_memory();
    let mut first = Rig::new(Some(&dir), false);
    first.record(true);
    let saved = first.save();
    assert!(saved.takes.iter().all(|k| k.path.is_some()));

    let mut r = Rig::new(None, true);
    r.live = false;
    assert!(r.h.load(saved.clone()).is_ok());
    assert_eq!(r.h.opening(), 2);
    assert_eq!(r.h.tracks.takes().count(), 0, "not in until open");
    r.run_until(|r| r.h.opening() == 0);
    assert!(r.all_on_file());
    assert_eq!(r.h.tracks.memory().used, 0);
    let ids = |s: &Saved| s.takes.iter().map(|k| (k.id, k.rec)).collect::<Vec<_>>();
    let have: Vec<_> = r.h.tracks.takes().map(|k| (k.id, k.rec)).collect();
    assert_eq!(have, ids(&saved));
    let got = r.play_song();
    assert!(got == want);
    assert_eq!(r.h.tracks.stats().missed, 0);
    // Saved again: each take copied from the file it opened.
    let again = r.save();
    for (a, b) in again.takes.iter().zip(&saved.takes) {
        assert_eq!((a.id, a.rec, &a.file), (b.id, b.rec, &b.file));
        assert_eq!(a.path, b.path);
    }

    // Cleared: the files are let go of, and the project opens again.
    assert!(r.h.clear());
    r.run_to(r.frame + 4 * P);
    assert_eq!(r.h.tracks.takes().count(), 0);
    assert!(r.h.load(saved).is_ok());
    r.run_until(|r| r.h.opening() == 0);
    assert!(r.all_on_file());
    assert!(r.play_song() == want);
    assert_eq!(r.h.tracks.stats().missed, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_waveform_and_a_fit_of_a_take_on_its_file_are_those_from_memory() {
    let dir = scratch("wave");
    let (mut mem, _) = from_memory();
    let mut r = Rig::new(Some(&dir), true);
    r.record(true);
    r.run_until(|r| r.all_on_file());
    for id in [0u16, 1] {
        assert!(mem.h.wave(id, 300));
        assert!(r.h.wave(id, 300));
        let mut want = None;
        mem.run_until(|m| {
            want = m.h.take_wave();
            want.is_some()
        });
        let mut got = None;
        r.run_until(|r| {
            got = r.h.take_wave();
            got.is_some()
        });
        let (want, got) = (want.unwrap(), got.unwrap());
        assert!(!want.columns.is_empty());
        assert_eq!(got, want, "take {id}");
    }
    // Fit take 1 (stereo) from 240 to 180 BPM: the same stretched take.
    assert!(mem.h.fit(1, 180.0));
    assert!(r.h.fit(1, 180.0));
    mem.run_until(|m| m.h.fits == 1);
    r.run_until(|r| r.h.fits == 1);
    let fitted = |r: &Rig| {
        let k = r.h.tracks.takes().find(|k| k.id > 1).unwrap();
        let mut audio = vec![0.0; k.frames * k.channels];
        assert_eq!(r.h.tracks.read_take(k.id, 0, &mut audio), Some(k.frames));
        (k.frames, audio)
    };
    let (want, got) = (fitted(&mem), fitted(&r));
    assert!(want.0 > 2 * BAR as usize);
    assert!(got == want, "the fit read from its file");
    assert_eq!(r.h.fits_failed, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_project_take_whose_file_is_missing_is_left_out_and_said() {
    let dir = scratch("gone");
    let mut first = Rig::new(Some(&dir), false);
    first.record(true);
    let mut saved = first.save();
    saved.takes[1].path = Some(dir.join("not-there.wav"));
    let mut r = Rig::new(None, true);
    assert!(r.h.load(saved).is_ok());
    r.run_until(|r| r.h.opening() == 0);
    let ev = r.events();
    assert!(
        matches!(&ev[..], [FileEvent::Unopened { track: 1, kept: false, why }] if why.contains("No such file")),
        "{ev:?}"
    );
    assert!(ev[0]
        .line("Keys")
        .starts_with("audio tracks: a take of Keys was left out"));
    let have: Vec<_> = r.h.tracks.takes().map(|k| k.track).collect();
    assert_eq!(have, [0]);
    assert!(r.all_on_file());
    // The tracks went in once the takes had: without the regions of the
    // take left out, so none plays a take that isn't there.
    assert_eq!(r.h.tracks.regions(0).len(), 1);
    assert!(r.h.tracks.regions(1).is_empty());
    let again = r.save();
    assert!(again.tracks.iter().all(|t| t
        .regions
        .iter()
        .all(|g| again.takes.iter().any(|k| k.id == g.take))));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_project_s_tracks_wait_for_its_takes_files() {
    let dir = scratch("wait");
    let mut first = Rig::new(Some(&dir), false);
    first.record(true);
    let saved = first.save();
    let mut r = Rig::new(None, true);
    r.live = false;
    assert!(r.h.load(saved.clone()).is_ok());
    // Until the files are open, no region plays a take not yet there.
    assert!((0..2).all(|t| r.h.tracks.regions(t).is_empty()));
    assert_eq!(
        r.h.load(saved.clone()),
        Err("audio tracks: another project is still opening".into())
    );
    r.run_until(|r| r.h.opening() == 0);
    assert!((0..2).all(|t| r.h.tracks.regions(t).len() == 1));
    // Emptied while its takes open: nothing of it comes in after.
    assert!(r.h.clear());
    r.run_to(r.frame + 4 * P);
    assert!(r.h.load(saved.clone()).is_ok());
    assert!(r.h.clear());
    r.run_to(r.frame + 40 * P);
    assert_eq!(r.h.tracks.takes().count(), 0);
    assert!((0..2).all(|t| r.h.tracks.regions(t).is_empty()));
    assert!(r.events().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn streaming_starts_only_before_recording_to_files() {
    let dir = scratch("order");
    let mut h = AudioHost::new(RATE, 10);
    assert!(h.record_to(dir.clone(), quick()).is_ok());
    assert!(!h.stream(small()), "the writer couldn't wait for its reads");
    assert!(!h.streams());
    let mut h = AudioHost::new(RATE, 10);
    assert!(h.stream(small()));
    assert!(!h.stream(small()), "once");
    assert!(h.streams());
    let _ = std::fs::remove_dir_all(&dir);
}
