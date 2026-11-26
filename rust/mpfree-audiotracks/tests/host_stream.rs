//! Playing takes from their files in the host (the audio tracks rethink,
//! phase 3b step 2): with streaming on, a recorded take's file is opened
//! once the writer has finished it and the take moves to it, its memory
//! freed, the song sounding exactly as it does from memory; a project's
//! takes open as their files with nothing copied in; a waveform or a fit
//! of a take on its file reads the file and gives what it gives from
//! memory; a project take whose file is missing is left out and said,
//! with its regions; a project's tracks wait for its takes' files;
//! clearing closes the files, and a project opens again after; streaming
//! can't start once recording to files has. Step 4: every region's start
//! is held in the cache (so a jump to one never waits on the drive), and
//! follows the regions as they change; so is the loop's start, so going
//! round never waits either; a cache too small for them all holds the
//! soonest and the log says so, once. A sound brought in and a stretched
//! copy are written to files of their own and play from them, or come
//! into memory if they can't be written. Phase 2: a cue's audio is held
//! and answered for, and audio clips' region starts are held too.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mpfree_audiotracks::files::{FileEvent, TakeState, WriterConfig};
use mpfree_audiotracks::host::AudioHost;
use mpfree_audiotracks::save::{from_wav, Saved};
use mpfree_audiotracks::streaming::StreamConfig;
use mpfree_audiotracks::Command as A;
use mpfree_stream::coord::Plan;
use mpfree_stream::player::Config;
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050; // at 240 BPM
const BAR_T: i64 = 4 * 960;
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
    cache(256)
}

/// A cache of `slots` units. The plan expects a 0.5 s stall, so a region's
/// held start is a second (the stall and half a second more), whatever the
/// default stall is.
fn cache(slots: usize) -> StreamConfig {
    StreamConfig {
        cache: Config {
            slots,
            ..Config::default()
        },
        plan: Plan {
            stall_s: 0.5,
            ..StreamConfig::default().plan
        },
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
        Rig::with(dir, stream.then(small))
    }

    /// [`Rig::new`] with takes played from their files through `cfg`.
    fn with(dir: Option<&Path>, cfg: Option<StreamConfig>) -> Rig {
        Rig::sized(dir, cfg, 30)
    }

    /// [`Rig::with`] with memory for `seconds` of stereo.
    fn sized(dir: Option<&Path>, cfg: Option<StreamConfig>, seconds: u32) -> Rig {
        let mut h = AudioHost::new(RATE, seconds);
        let stream = cfg.is_some();
        if let Some(cfg) = cfg {
            assert!(h.stream(cfg));
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

    /// Periods until `done`, giving the threads time: more of it a period
    /// before [`START`], so a busy machine's threads (the writer making an
    /// armed take's file, the player starting) have seconds to answer in
    /// the lead-in, not a quarter of one.
    #[track_caller]
    fn run_until(&mut self, mut done: impl FnMut(&mut Rig) -> bool) {
        let t0 = Instant::now();
        while !done(self) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out");
            self.period();
            let pause = if self.frame < START { 4000 } else { 300 };
            std::thread::sleep(Duration::from_micros(pause));
        }
    }

    /// Track `tracks`' files made for their armed takes, before [`START`].
    #[track_caller]
    fn armed(&mut self, tracks: &[usize]) {
        let t0 = Instant::now();
        self.run_until(|r| tracks.iter().all(|&t| r.h.armed_file(t) == Some(true)));
        assert!(
            self.frame < START - P,
            "the files took {:?} to be made, past the lead-in",
            t0.elapsed()
        );
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
            self.armed(&[0, 1]);
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

    /// Track 0 (mono) records `bars` bars from [`START`], then disarms;
    /// until its take is on its file.
    fn record_long(&mut self, bars: u64) {
        self.h.command(A::Arm(0, true));
        self.armed(&[0]);
        self.t.command(START, T::Record);
        self.t.command(START + bars * BAR, T::Stop);
        self.run_to(START + bars * BAR + 4 * P);
        self.h.command(A::Arm(0, false));
        self.live = false;
        self.run_until(|r| r.all_on_file());
        assert_eq!(self.h.tracks.takes().count(), 1);
    }

    /// Track 0 cut at each of `bars`; its regions after.
    fn split(&mut self, bars: impl IntoIterator<Item = i64>) -> Vec<mpfree_audiotracks::Region> {
        for bar in bars {
            assert!(self.h.command(A::Split {
                track: 0,
                at: bar * BAR_T
            }));
        }
        self.period();
        self.h.tracks.regions(0).to_vec()
    }

    /// Whether the cache holds frames `from..from + frames` of take `k`.
    fn cached(&self, k: u16, from: i64, frames: u64) -> bool {
        self.h
            .tracks
            .player()
            .is_some_and(|p| p.covers(k, from as u64, frames))
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
    let k = mem.h.tracks.takes().find(|k| k.id > 1).unwrap();
    let mut want = vec![0.0; k.frames * k.channels];
    assert_eq!(mem.h.tracks.read_take(k.id, 0, &mut want), Some(k.frames));
    assert!(k.frames > 2 * BAR as usize);
    // The stretched copy was written to a file of its own and plays from
    // it, taking no memory: the same audio.
    let new = r.h.tracks.takes().find(|k| k.id > 1).unwrap();
    assert_eq!((new.frames, new.channels), (k.frames, k.channels));
    r.run_until(|r| r.h.tracks.on_file(new.id));
    assert_eq!(r.h.tracks.memory().used, 0);
    let saved = r.save();
    let file = saved.takes.iter().find(|t| t.id == new.id).unwrap();
    assert_eq!(file.file, format!("take_{}.wav", new.rec));
    let path = file.path.as_ref().expect("copied whole from its file");
    assert!(path.starts_with(&dir));
    let (got, ch, rate) = from_wav(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!((ch, rate), (2, RATE));
    assert!(
        got == want,
        "the fit read from its file, written to its own"
    );
    assert_eq!(r.h.fits_failed, 0);
    assert!(r.events().is_empty());
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

#[test]
fn the_starts_of_regions_on_their_files_are_held_so_a_jump_to_one_plays_at_once() {
    let dir = scratch("heads");
    let mut r = Rig::with(Some(&dir), Some(cache(64)));
    r.record_long(24);
    let regions = r.split([6, 12, 18]);
    assert_eq!(regions.len(), 4);
    r.run_until(|r| r.h.heads_wanted == 4 && r.h.heads_held == 4);
    // A second of each: the longest stall the plan expects, and half a
    // second more.
    let k = regions[0].take;
    r.run_until(|r| regions.iter().all(|g| r.cached(k, g.src, BAR)));
    // Only the starts: not the rest of a region (the playhead, stopped
    // at the end of the take, wants nothing).
    assert!(!r.cached(k, regions[2].src + 3 * BAR as i64, P));
    // Playing: a jump into the middle of a region, which the cache can't
    // have yet, plays silence for a moment; a jump to a region's start
    // plays at once.
    r.t.command(r.frame, T::Locate(0));
    r.run_to(r.frame + 2 * P);
    r.t.command(r.frame, T::PlayStart);
    r.run_to(r.frame + 10 * P);
    let missed = |r: &Rig| r.h.tracks.stats().missed;
    assert_eq!(missed(&r), 0, "bar 1's start is held too");
    r.t.command(r.frame, T::Locate(15 * BAR_T));
    r.run_to(r.frame + 2 * P);
    let m = missed(&r);
    assert!(m > 0);
    r.t.command(r.frame, T::Locate(12 * BAR_T));
    r.run_to(r.frame + 2 * P);
    assert_eq!(missed(&r), m);
    r.t.command(r.frame, T::StopAndRewind);
    r.run_to(r.frame + 2 * P);
    // Moved over the next one: the starts held follow the regions.
    assert!(r.h.command(A::Move {
        track: 0,
        at: 13 * BAR_T,
        to: 15 * BAR_T
    }));
    r.period();
    let moved = r.h.tracks.regions(0).to_vec();
    assert_ne!(moved, regions);
    r.run_until(|r| moved.iter().all(|g| r.cached(k, g.src, BAR)));
    assert!(r.events().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Step 5d: frames a take on its file played as silence because the drive
/// was late reach System status as Stick fell behind, about no track.
#[test]
fn the_drive_falling_behind_is_told_to_system_status() {
    use mpfree_audiotracks::files::Problem;
    use mpfree_status::{kinds, Report};
    let dir = scratch("behind");
    let mut r = Rig::with(Some(&dir), Some(cache(64)));
    r.record_long(24);
    r.run_until(|r| r.all_on_file());
    let before: Vec<Problem> = std::iter::from_fn(|| r.h.take_problem()).collect();
    assert!(
        !before
            .iter()
            .any(|p| p.report.kind == &kinds::STICK_FELL_BEHIND),
        "{before:?}"
    );
    // A jump into the middle of the take, played from there at once (Play,
    // not PlayStart, which plays from bar 1: whether bar 1's start was
    // held yet decided it before), which the cache can't have yet.
    r.t.command(r.frame, T::Locate(15 * BAR_T));
    r.t.command(r.frame, T::Play);
    r.run_to(r.frame + 2 * P);
    assert!(r.h.tracks.stats().missed > 0);
    // The player's counters go to System status every 64 periods.
    r.run_to(r.frame + 70 * P);
    let after: Vec<Problem> = std::iter::from_fn(|| r.h.take_problem()).collect();
    assert!(
        after.contains(&Problem {
            report: Report::happened(&kinds::STICK_FELL_BEHIND, 1),
            track: None,
        }),
        "{after:?}"
    );
    assert_eq!(r.h.problems_lost, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_cache_too_small_for_every_start_holds_the_soonest_and_says_so_once() {
    let dir = scratch("short");
    // 24 units: the first 16 starts (a second each, 22 units of the
    // mono take) fit, the next 8 don't.
    let mut r = Rig::with(Some(&dir), Some(cache(24)));
    r.record_long(24);
    r.t.command(r.frame, T::Locate(0));
    r.run_to(r.frame + 2 * P);
    let regions = r.split(1..24);
    assert_eq!(regions.len(), 24);
    let mut ev = Vec::new();
    r.run_until(|r| {
        ev.extend(r.events());
        !ev.is_empty()
    });
    assert!(
        matches!(ev[..], [FileEvent::HeadsShort { held: 16, of: 24 }]),
        "{ev:?}"
    );
    assert!(ev[0].line("").starts_with(
        "audio tracks: the cache holds the starts of 16 of the 24 regions playing from the drive"
    ));
    assert_eq!((r.h.heads_held, r.h.heads_wanted), (16, 24));
    // The soonest: from the playhead on.
    let k = regions[0].take;
    r.run_until(|r| regions[..16].iter().all(|g| r.cached(k, g.src, BAR)));
    assert!(regions[17..].iter().all(|g| !r.cached(k, g.src, P)));
    // Asked again a few seconds on, in case room came back: the same, so
    // the log hears nothing more.
    r.run_to(r.frame + 1100 * P);
    for _ in 0..100 {
        r.period();
        std::thread::sleep(Duration::from_micros(300));
    }
    assert!(r.events().is_empty());
    assert_eq!(r.h.heads_held, 16);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_loop_s_start_is_held_too_so_going_round_never_waits() {
    let dir = scratch("loop");
    let mut r = Rig::with(Some(&dir), Some(cache(64)));
    r.record_long(24);
    let regions = r.split([6, 12, 18]);
    r.run_until(|r| r.h.heads_wanted == 4 && r.h.heads_held == 4);
    let k = regions[0].take;
    // Bar 16, inside the third region: not held yet.
    let at = regions[2].src + 3 * BAR as i64;
    assert!(!r.cached(k, at, P));
    r.t.command(r.frame, T::SetLoop(Some((15 * BAR_T, 17 * BAR_T))));
    r.run_until(|r| r.h.heads_wanted == 5 && r.h.heads_held == 5);
    r.run_until(|r| r.cached(k, at, BAR));
    // Stopped at bar 17 until its frames are in, then on and round the
    // loop: never a frame late.
    r.t.command(r.frame, T::Locate(16 * BAR_T));
    r.run_until(|r| r.cached(k, at + BAR as i64, BAR));
    r.t.command(r.frame, T::Play);
    r.run_to(r.frame + BAR + 8 * P);
    assert_eq!(r.h.tracks.stats().missed, 0);
    r.t.command(r.frame, T::StopAndRewind);
    // No loop: only the regions' starts.
    r.t.command(r.frame, T::SetLoop(None));
    r.run_until(|r| r.h.heads_wanted == 4 && r.h.heads_held == 4);
    assert!(r.events().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Phase 2, step 2b: what a cue that launches audio needs (the time
/// engine's needs) is held on the stream thread, and the host answers once
/// it is all in the cache; a recording not here is short at once, and so
/// is more than the cache holds; an audio clip's region starts are held
/// with the song regions' starts, so a clip launched while stopped starts
/// from memory at PLAY.
#[test]
fn a_cue_s_audio_is_held_and_answered_once_it_is_in_the_cache() {
    use mpfree_audiotracks::host::CueAudio;
    use mpfree_audiotracks::RecordingId;
    let dir = scratch("cues");
    let mut r = Rig::with(Some(&dir), Some(cache(24)));
    r.record_long(24);
    r.run_until(|r| r.h.heads_wanted == 1 && r.h.heads_held == 1);
    let k = r.h.tracks.takes().next().unwrap();
    // Bar 13 of the take: no region starts there.
    let from = 12 * BAR as i64;
    assert!(!r.cached(k.id, from, P));
    r.h.hold_cue(7, &[(k.rec, from, from + BAR as i64)]);
    let mut said = None;
    r.run_until(|r| {
        said = r.h.cue_audio();
        said.is_some()
    });
    assert_eq!(said, Some((7, CueAudio::Held)));
    // The audio thread has it within a few periods of the answer: the
    // units go to it at its next boundary, and the test's periods run far
    // faster than real ones (300 us apart), so on a busy machine the
    // stream thread can take a few to hand them over. 8 of them are still
    // well inside a launch line.
    let f = r.frame;
    r.run_until(|r| r.cached(k.id, from, BAR));
    assert!(r.frame - f <= 8 * P, "{}", r.frame - f);
    // A recording not here (still loading, say): short, at once.
    r.h.hold_cue(8, &[(RecordingId(9999), 0, 100)]);
    assert_eq!(r.h.cue_audio(), Some((8, CueAudio::Short)));
    // Twenty seconds don't fit in the cache beside what is held.
    r.h.hold_cue(9, &[(k.rec, 0, 20 * BAR as i64)]);
    r.run_until(|r| {
        said = r.h.cue_audio();
        said.is_some()
    });
    assert_eq!(said, Some((9, CueAudio::Short)));
    // The cue landed: its hold goes, and nothing more is said.
    r.h.let_cue_go(7);
    // A clip's region start, bar 6 of the take, is held with the song's.
    r.h.clip_starts(vec![(k.rec, 5 * BAR)]);
    r.run_until(|r| r.h.heads_wanted == 2 && r.h.heads_held == 2);
    r.run_until(|r| r.cached(k.id, 5 * BAR as i64, BAR));
    for _ in 0..50 {
        r.period();
    }
    assert_eq!(r.h.cue_audio(), None);
    assert!(r.events().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two bars of stereo, as the Sample page would bring in.
fn a_sound() -> Vec<f32> {
    (0..2 * BAR).flat_map(|g| [sig(g, 0), sig(g, 1)]).collect()
}

#[test]
fn a_sound_brought_in_is_written_to_its_file_and_plays_from_it() {
    let dir = scratch("import");
    let bring = |r: &mut Rig| {
        r.live = false;
        let id = r.h.import(1, 0, 2, a_sound(), vec![BAR], 120.0).unwrap();
        r.run_until(|r| r.h.importing().is_none());
        assert_eq!(r.h.tracks.regions(1).len(), 2, "a region a slice");
        id
    };
    let mut mem = Rig::new(None, false);
    bring(&mut mem);
    let want = mem.play_song();
    assert!(want.iter().filter(|v| v.abs() > 1e-6).count() > BAR as usize);

    let mut r = Rig::new(Some(&dir), true);
    let id = bring(&mut r);
    assert!(r.h.tracks.on_file(id));
    assert_eq!(r.h.tracks.memory().used, 0, "no memory taken");
    assert_eq!(r.h.opening(), 0);
    let got = r.play_song();
    assert!(got == want, "played from its file, the same");
    assert_eq!(r.h.tracks.stats().missed, 0);
    // A save copies its file whole, named for its lasting id.
    let saved = r.save();
    let rec = r.h.tracks.take(id).unwrap().rec;
    assert_eq!(saved.takes[0].file, format!("take_{rec}.wav"));
    let path = saved.takes[0].path.as_ref().expect("copied from its file");
    assert_eq!(path, &dir.join(format!("take_{rec}.wav")));
    assert!(r.events().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_sound_whose_file_cant_be_written_comes_into_memory_and_is_said() {
    let dir = scratch("unwritten");
    let mut r = Rig::new(Some(&dir), true);
    r.live = false;
    // The folder is gone: the file can't be written. (The worker makes it
    // after starting the player, so wait for it first.)
    r.run_until(|_| dir.is_dir());
    std::fs::remove_dir_all(&dir).unwrap();
    let id = r.h.import(1, 0, 2, a_sound(), vec![], 120.0).unwrap();
    r.run_until(|r| r.h.importing().is_none());
    assert!(!r.h.tracks.on_file(id));
    assert_eq!(r.h.tracks.regions(1).len(), 1);
    assert!(r.h.tracks.memory().used >= 4 * BAR as usize, "in memory");
    let ev = r.events();
    assert!(
        matches!(&ev[..], [FileEvent::Unwritten { track: 1, why }] if why.contains("No such file")),
        "{ev:?}"
    );
    assert!(ev[0].line("Keys").starts_with(
        "audio tracks: Keys's new take plays from memory, as its file couldn't be written"
    ));
}

/// A project recorded at another rate, its takes converted as it opens
/// ([`Saved::convert`]): each take is written to a file of its own and
/// plays from there, the same as from memory, and the tracks wait for
/// the files.
#[test]
fn a_project_s_converted_takes_are_written_to_files_and_play_from_them() {
    let dir = scratch("convert");
    let (mut mem, _) = from_memory();
    // As if recorded at twice the rate, then opened here.
    let mut saved = mem.save();
    saved.convert(2 * RATE);
    saved.convert(RATE);
    assert!(saved
        .takes
        .iter()
        .all(|k| k.path.is_none() && !k.audio.is_empty()));
    let mut mem = Rig::new(None, false);
    mem.live = false;
    assert!(mem.h.load(saved.clone()).is_ok());
    // Its takes copied in (a little each period) before it plays.
    mem.run_until(|r| r.h.tracks.loading() == 0);
    let want = mem.play_song();
    assert!(want.iter().filter(|v| v.abs() > 1e-6).count() > BAR as usize);

    let mut r = Rig::new(Some(&dir), true);
    r.live = false;
    assert!(r.h.load(saved.clone()).is_ok());
    assert_eq!(r.h.opening(), 2);
    assert!(
        (0..2).all(|t| r.h.tracks.regions(t).is_empty()),
        "the tracks wait"
    );
    r.run_until(|r| r.h.opening() == 0);
    assert!(r.all_on_file());
    assert_eq!(r.h.tracks.memory().used, 0, "no memory taken");
    assert!((0..2).all(|t| r.h.tracks.regions(t).len() == 1));
    let got = r.play_song();
    assert!(got == want, "played from their files, the same");
    assert_eq!(r.h.tracks.stats().missed, 0);
    // A save copies each file whole, named for its new lasting id.
    let again = r.save();
    for (a, b) in again.takes.iter().zip(&saved.takes) {
        assert_eq!((a.id, a.frames), (b.id, b.frames));
        assert!(a.rec.is_given());
        assert_eq!(a.file, format!("take_{}.wav", a.rec));
        assert_eq!(a.path.as_ref(), Some(&dir.join(&a.file)));
        let (audio, ch, rate) = from_wav(&std::fs::read(dir.join(&a.file)).unwrap()).unwrap();
        assert_eq!((ch, rate), (b.channels, RATE));
        assert!(audio == b.audio, "the converted sound, as it was");
    }
    assert!(r.events().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Track 0 (mono) records `bars` bars from [`START`], then disarms; with
/// `files`, into its file, at about four times the speed it plays (each
/// period given time, so the writer commits as it would).
fn record_paced(r: &mut Rig, bars: u64, files: bool) {
    r.h.command(A::Arm(0, true));
    if files {
        r.armed(&[0]);
    }
    assert!(r.frame < START - P);
    r.t.command(START, T::Record);
    r.t.command(START + bars * BAR, T::Stop);
    while r.frame < START + bars * BAR + 4 * P {
        r.period();
        if files && r.frame > START {
            std::thread::sleep(Duration::from_millis(3));
        }
    }
    r.h.command(A::Arm(0, false));
    r.live = false;
}

#[test]
fn a_recording_longer_than_memory_records_on_as_its_file_takes_it() {
    let dir = scratch("long");
    // 8 s of mono into memory for 3 s of stereo (6 s of mono).
    let mut mem = Rig::sized(None, None, 30);
    record_paced(&mut mem, 8, false);
    let want = mem.save();
    let mut short = Rig::sized(Some(&dir), None, 3);
    record_paced(&mut short, 8, true);
    assert_eq!(
        short.h.tracks.stats().full,
        1,
        "without streaming: cut short"
    );
    let _ = std::fs::remove_dir_all(&dir);

    let dir = scratch("long-on");
    let mut r = Rig::sized(Some(&dir), Some(small()), 3);
    record_paced(&mut r, 8, true);
    assert_eq!(r.h.tracks.stats().full, 0, "never out of memory");
    r.run_until(|r| r.all_on_file());
    assert_eq!(r.h.tracks.memory().used, 0);
    let k = r.h.tracks.takes().next().unwrap();
    assert_eq!(k.frames, want.takes[0].frames);
    // Its file holds what memory would have.
    let saved = r.save();
    let path = saved.takes[0]
        .path
        .as_ref()
        .expect("copied whole from its file");
    let (audio, ch, _) = from_wav(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(ch, 1);
    assert!(audio == want.takes[0].audio, "the same sound");
    // Its waveform (from what the writer was given) is memory's.
    assert!(mem.h.wave(k.id, 300));
    assert!(r.h.wave(k.id, 300));
    let mut want_wave = None;
    mem.run_until(|m| {
        want_wave = m.h.take_wave();
        want_wave.is_some()
    });
    let mut got_wave = None;
    r.run_until(|r| {
        got_wave = r.h.take_wave();
        got_wave.is_some()
    });
    assert!(!want_wave.as_ref().unwrap().columns.is_empty());
    assert_eq!(got_wave, want_wave);
    // And plays as it does from memory.
    let got = r.play_song();
    assert!(got == mem.play_song());
    assert_eq!(r.h.tracks.stats().missed, 0);
    assert!(r.events().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The SD card plan's phase B2a: a project's takes on the card are read
/// only while the card is open. When it goes, System status hears it and
/// a cue waiting for their audio is short at once; when it opens again (a
/// new insertion), they open again from the same files, the regions'
/// starts are held again, and the song plays as before. A project opened
/// while the card is out leaves its takes out, and says why.
#[test]
fn takes_on_the_card_open_again_when_the_card_comes_back() {
    use mpfree_audiotracks::files::Problem;
    use mpfree_audiotracks::host::CueAudio;
    use mpfree_status::{kinds, Report};
    use mpfree_stream::storage::Lease;
    let card = scratch("card");
    let (_, want) = from_memory();
    let mut first = Rig::new(Some(&card), false);
    first.record(true);
    let saved = first.save();
    assert!(saved
        .takes
        .iter()
        .all(|k| k.path.as_ref().is_some_and(|p| p.starts_with(&card))));

    let mut r = Rig::new(None, true);
    r.live = false;
    let lease = Lease::new(&card);
    r.h.own_device(lease.clone());
    r.h.own_device(lease.clone());
    assert!(r.h.load(saved.clone()).is_ok());
    r.run_until(|r| r.h.opening() == 0);
    assert!(r.all_on_file());
    // The opening counts the takes playing from it, which hold the card.
    r.run_until(|_| lease.users() == 2);
    assert!(r.play_song() == want);
    r.run_until(|r| r.h.heads_wanted > 0 && r.h.heads_held == r.h.heads_wanted);
    let gens = |r: &Rig| -> Vec<Option<u32>> {
        r.h.tracks
            .takes()
            .map(|k| r.h.tracks.file_gen(k.id))
            .collect()
    };
    let before = gens(&r);
    assert!(before.iter().all(Option::is_some));
    let _ = std::iter::from_fn(|| r.h.take_problem()).count();

    // The card goes.
    lease.revoke();
    let mut got: Vec<Problem> = Vec::new();
    r.run_until(|r| {
        got.extend(std::iter::from_fn(|| r.h.take_problem()));
        !got.is_empty()
    });
    assert_eq!(
        got,
        [Problem {
            report: Report::happened(&kinds::CARD_REMOVED, 1),
            track: None,
        }]
    );
    let k = r.h.tracks.takes().next().unwrap();
    r.h.hold_cue(5, &[(k.rec, BAR as i64, 2 * BAR as i64)]);
    let mut said = None;
    r.run_until(|r| {
        said = r.h.cue_audio();
        said.is_some()
    });
    assert_eq!(said, Some((5, CueAudio::Short)), "its audio can't come");
    assert_eq!(gens(&r), before, "nothing opens again until it is back");
    // The takes on it let go of their files there, so it can be closed
    // cleanly (an Eject).
    r.run_until(|_| mpfree_stream::storage::open_under(&card).is_ok_and(|o| o.is_empty()));

    // It opens again: a new insertion.
    let again = Lease::new(&card);
    r.h.own_device(again.clone());
    r.run_until(|r| {
        gens(r)
            .iter()
            .zip(&before)
            .all(|(now, was)| now.is_some() && now != was)
    });
    r.run_until(|r| r.h.heads_held == r.h.heads_wanted);
    r.run_until(|_| again.users() == 2);
    assert_eq!(lease.users(), 0, "the opening before holds nothing");
    // Which takes, by their files' names: what an Eject says goes quiet.
    let playing = again.playing();
    assert_eq!(playing.len(), 2, "{playing:?}");
    assert!(playing
        .iter()
        .all(|(k, n)| r.h.tracks.take(*k).is_some() && n.ends_with(".wav")));
    assert!(lease.playing().is_empty());
    assert!(r.play_song() == want, "read again from the card, the same");
    assert_eq!(r.h.tracks.stats().missed, 0);
    let ev = r.events();
    assert!(ev.is_empty(), "{ev:?}");

    // Out again, and the project opened again meanwhile: its takes are
    // left out, and why is said.
    assert!(r.h.clear());
    r.run_until(|_| again.users() == 0);
    again.revoke();
    r.run_to(r.frame + 4 * P);
    assert!(r.h.load(saved).is_ok());
    r.run_until(|r| r.h.opening() == 0);
    assert_eq!(r.h.tracks.takes().count(), 0);
    let ev = r.events();
    assert_eq!(ev.len(), 2, "{ev:?}");
    assert!(
        ev.iter().all(|e| matches!(
            e,
            FileEvent::Unopened { kept: false, why, .. } if why.contains("the SD card isn't there")
        )),
        "{ev:?}"
    );
    let _ = std::fs::remove_dir_all(&card);
}

/// The SD card plan's phase B2c: a take recording on the card when the
/// card goes (taken out: its opening revoked) carries on on the stick, the
/// same take, recording on: once the app asks for the stick's folder, the
/// take's file there holds zeros for what was committed on the card and
/// everything after, the card's part is left where it was, the take plays
/// from its file on the stick from then on, and the log says so. When the
/// same card is back, the take gets its first part from it and is whole on
/// the stick, the card left as it was, and the next save names it anew.
#[test]
fn a_take_on_the_card_carries_on_on_the_stick_when_the_card_goes() {
    use mpfree_stream::rec::Hole;
    use mpfree_stream::storage::{CardMark, Lease};
    let card = scratch("carry-card");
    let stick = scratch("carry-stick");
    let mut mem = Rig::sized(None, None, 30);
    record_paced(&mut mem, 8, false);
    let want = mem.save();

    let mut r = Rig::sized(None, Some(small()), 30);
    let lease = Lease::new(&card);
    let mark = CardMark {
        id: *b"card-marker-id-1",
        serial: 0x5EC0_1A2B,
    };
    assert!(lease.set_mark(mark));
    r.h.own_device(lease.clone());
    assert!(r.h.record_to(card.clone(), quick()).is_ok());
    r.h.command(A::Arm(0, true));
    r.armed(&[0]);
    r.t.command(START, T::Record);
    r.t.command(START + 8 * BAR, T::Stop);
    let paced = |r: &mut Rig| {
        r.period();
        if r.frame > START {
            std::thread::sleep(Duration::from_millis(3));
        }
    };
    while r.frame < START + 2 * BAR {
        paced(&mut r);
    }
    // Some of it is on the card, its memory given back.
    let t0 = Instant::now();
    let k = loop {
        let k = r.h.tracks.takes().next().map(|k| k.id);
        if let Some(k) = k.filter(|&k| r.h.tracks.file_frames(k).is_some_and(|n| n > 0)) {
            break k;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(30),
            "nothing on the card"
        );
        paced(&mut r);
    };
    assert!(r.frame < START + 6 * BAR, "the card took too long");
    let on_card = r.h.tracks.file_gen(k).unwrap();
    // What an Eject says carries on on the stick.
    assert_eq!(r.h.recording_on(&card), [0]);
    assert!(r.h.recording_on(&stick).is_empty());

    // The card goes; the app moves recording to the stick.
    lease.revoke();
    assert!(r.h.recording_on(&card).is_empty(), "its opening is over");
    assert!(
        r.h.record_to(stick.clone(), quick()).is_ok(),
        "nothing waits"
    );
    while r.frame < START + 8 * BAR + 4 * P {
        paced(&mut r);
    }
    r.h.command(A::Arm(0, false));
    r.live = false;
    r.run_until(|r| r.h.file(k).is_some_and(|f| f.state == TakeState::Saved) && r.all_on_file());
    assert_eq!(r.h.tracks.stats().full, 0);
    let now = r.h.tracks.file_gen(k).unwrap();
    assert_ne!(now, on_card, "it plays from its file on the stick");
    // Nothing on the card is open any more, so it can be closed cleanly
    // (an Eject).
    let open = mpfree_stream::storage::open_under(&card).unwrap();
    assert!(open.is_empty(), "{open:?}");

    // Its file on the stick: zeros for the card's part, then the rest.
    let holes: Vec<PathBuf> = std::fs::read_dir(&stick)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".wav.hole"))
        .collect();
    assert_eq!(holes.len(), 1);
    let hole = Hole::decode(&std::fs::read(&holes[0]).unwrap()).expect("a hole record");
    assert!(hole.frames > 0);
    assert_eq!(hole.from.parent(), Some(card.as_path()));
    assert_eq!(
        (hole.card, hole.serial),
        (mark.id, mark.serial),
        "which card"
    );
    let saved = r.save();
    let path = saved.takes[0].path.as_ref().expect("its file holds it all");
    assert!(path.starts_with(&stick));
    let (audio, ch, _) = from_wav(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(ch, 1);
    let mut wanted = want.takes[0].audio.clone();
    assert_eq!(audio.len(), wanted.len());
    wanted[..hole.frames as usize].fill(0.0);
    assert!(
        audio == wanted,
        "zeros where the card has it, then the take"
    );
    // The card's part is left where it was, for when the card is back.
    let name = hole.from.to_string_lossy().into_owned();
    let ckpt = PathBuf::from(name.replace(".wav.recording", ".wav.checkpoint"));
    assert!(name.ends_with(".wav.recording") && ckpt.is_file());
    let bytes = std::fs::read(&hole.from).unwrap();
    let l = mpfree_stream::format::layout(&mut std::io::Cursor::new(&bytes)).unwrap();
    let n = hole.frames as usize;
    assert!(l.frames as usize >= n);
    let mut there = vec![0.0f32; n];
    let from = l.data_at as usize;
    mpfree_stream::format::decode(
        l.sample,
        l.endian,
        &bytes[from..from + n * l.block_align()],
        &mut there,
    );
    assert!(there == want.takes[0].audio[..n], "the card holds its part");
    let ev = r.events();
    assert!(
        ev.iter().any(|e| matches!(
            e,
            FileEvent::CarriedOn { track: 0, hole_s }
                if (*hole_s - hole.frames as f32 / RATE as f32).abs() < 1e-3
        )),
        "{ev:?}"
    );
    // System status: the card came out, the track's take waiting for its
    // part from it.
    let said = |r: &mut Rig| {
        let mut v = Vec::new();
        while let Some(p) = r.h.take_problem() {
            v.push((p.report.kind.id, p.report.op));
        }
        v
    };
    let mut problems = said(&mut r);
    assert!(
        problems.contains(&(mpfree_status::kinds::CARD_OUT.id, mpfree_status::Op::Began)),
        "{problems:?}"
    );
    assert_eq!(r.h.card_parts(), [0]);

    // The same card is back (read-only, as a pulled card comes back): the
    // take gets its first part from it, into its file on the stick, and
    // is whole there; nothing on the card changes.
    let card_before = std::fs::read(&hole.from).unwrap();
    // Another card in its place (its volume serial differs) is not it.
    let other = Lease::new(&card);
    let not_it = CardMark {
        serial: mark.serial ^ 1,
        ..mark
    };
    assert!(other.set_mark(not_it));
    r.h.own_device(other.clone());
    r.h.join_card(not_it, vec![card.clone()]);
    for _ in 0..200 {
        r.period();
        std::thread::sleep(Duration::from_micros(300));
    }
    let ev = r.events();
    assert!(
        !ev.iter()
            .any(|e| matches!(e, FileEvent::Joined { .. } | FileEvent::Unjoined { .. })),
        "{ev:?}"
    );
    assert!(holes[0].is_file() && other.users() == 0);
    other.revoke();
    let back = Lease::new(&card);
    assert!(back.set_mark(mark));
    r.h.own_device(back.clone());
    r.h.join_card(mark, vec![card.clone()]);
    let mut ev = Vec::new();
    r.run_until(|r| {
        ev.extend(r.events());
        ev.iter()
            .any(|e| matches!(e, FileEvent::Joined { .. } | FileEvent::Unjoined { .. }))
    });
    assert!(
        ev.iter().any(|e| matches!(
            e,
            FileEvent::Joined { track: 0, joined_s }
                if (*joined_s - hole.frames as f32 / RATE as f32).abs() < 1e-3
        )),
        "{ev:?}"
    );
    let (audio, _, _) = from_wav(&std::fs::read(path).unwrap()).unwrap();
    assert!(
        audio == want.takes[0].audio,
        "the take is whole on the stick"
    );
    assert!(!holes[0].exists(), "its hole record goes");
    // Whole: nothing waits for the card, and System status says so.
    assert!(r.h.card_parts().is_empty());
    problems.extend(said(&mut r));
    assert!(
        problems.contains(&(mpfree_status::kinds::CARD_OUT.id, mpfree_status::Op::Ended)),
        "{problems:?}"
    );
    assert_eq!(std::fs::read(&hole.from).unwrap(), card_before);
    assert!(ckpt.is_file(), "the card's files are left as they were");
    // It plays from its whole file, and the card is let go.
    r.run_until(|r| r.h.tracks.file_gen(k).is_some_and(|g| g != now) && back.users() == 0);
    // The project's copy has the zeros, so the next save copies the whole
    // take in under a name of its own.
    let again = r.save();
    let whole = &again.takes[0];
    assert_eq!(whole.path.as_deref(), Some(path.as_path()));
    assert!(whole.file.ends_with("_whole.wav"), "{}", whole.file);
    assert_ne!(whole.file, saved.takes[0].file);
    let _ = std::fs::remove_dir_all(&card);
    let _ = std::fs::remove_dir_all(&stick);
}

/// The SD card goes while a take records on it and the stick can't take
/// recording over (its folder can't be made): System status says
/// recording stopped when the card came out, until recording moves again.
#[test]
fn a_card_that_goes_with_nowhere_to_carry_on_says_recording_stopped() {
    use mpfree_status::{kinds, Op};
    use mpfree_stream::storage::Lease;
    let card = scratch("carry-nowhere-card");
    let stick = scratch("carry-nowhere-stick");
    std::fs::create_dir_all(&card).unwrap();
    std::fs::create_dir_all(&stick).unwrap();
    // A file where the stick's folder would be made.
    let blocked = stick.join("not-a-folder");
    std::fs::write(&blocked, b"x").unwrap();
    let mut r = Rig::sized(None, Some(small()), 30);
    let lease = Lease::new(&card);
    r.h.own_device(lease.clone());
    assert!(r.h.record_to(card.clone(), quick()).is_ok());
    r.h.command(A::Arm(0, true));
    r.armed(&[0]);
    r.t.command(START, T::Record);
    r.run_until(|r| r.h.recording_on(&card) == [0]);
    let said = |r: &mut Rig| {
        let mut v = Vec::new();
        while let Some(p) = r.h.take_problem() {
            v.push((p.report.kind.id, p.report.op));
        }
        v
    };
    lease.revoke();
    assert!(r.h.record_to(blocked.join("work"), quick()).is_ok());
    let mut problems = Vec::new();
    r.run_until(|r| {
        problems.extend(said(r));
        problems.contains(&(kinds::CARD_OUT_STOPPED.id, Op::Began))
    });
    assert!(
        !problems.contains(&(kinds::CARD_OUT.id, Op::Began)),
        "{problems:?}"
    );
    // Recording moves again (the card back, or room on the stick).
    r.t.command(r.frame + P, T::Stop);
    r.live = false;
    r.run_until(|r| r.h.record_to(stick.join("work"), quick()).is_ok());
    r.run_until(|r| {
        problems.extend(said(r));
        problems.contains(&(kinds::CARD_OUT_STOPPED.id, Op::Ended))
    });
    let _ = std::fs::remove_dir_all(&card);
    let _ = std::fs::remove_dir_all(&stick);
}

/// The SD card plan's phase B2a: sound never clicks as it goes missing or
/// comes back. A take on the card plays on after the card is pulled until
/// the cache runs out; at every gap in what plays (the card's, or a read
/// late on a busy machine) the sound has faded to nearly nothing on both
/// sides, where without the fade it would stop and start at full level.
#[test]
fn sound_fades_at_every_gap_instead_of_clicking() {
    use mpfree_stream::player::DECLICK;
    use mpfree_stream::storage::Lease;
    let card = scratch("declick");
    std::fs::create_dir_all(&card).unwrap();
    let mut r = Rig::with(Some(&card), Some(cache(24)));
    let lease = Lease::new(&card);
    r.h.own_device(lease.clone());
    r.record_long(24);
    // The recording's sync times are the card's, for Dev and the logs.
    r.run_until(|r| r.h.timings().syncs.is_some_and(|s| s.times.count > 0));
    let syncs = r.h.timings().syncs.unwrap();
    assert!(syncs.on_card && syncs.dir.starts_with(&card), "{syncs:?}");
    // Played from the take's start, and the card pulled two bars in: the
    // cache can't hold the whole take, so what it hasn't read goes missing.
    r.t.command(r.frame, T::Locate(8 * BAR_T));
    let from = r.out.len();
    r.t.command(r.frame, T::Play);
    r.run_to(r.frame + 2 * BAR);
    // Some of the card's reads came back, and their times are up (the
    // stream thread puts them up every LOOK), before the card goes: a read
    // that comes back after it went is never used or timed, and a release
    // build can get here before any has come back.
    r.run_until(|r| {
        r.h.timings()
            .reads
            .get(1)
            .is_some_and(|d| d.reads.count > 0)
    });
    lease.revoke();
    r.run_to(r.frame + 16 * BAR);
    assert!(r.h.tracks.stats().missed > 0, "the card's audio ran out");
    // Its reads were the card's own drive's: the stick read nothing.
    let reads = r.h.timings().reads;
    assert_eq!(reads.len(), 2, "{reads:?}");
    assert_eq!(reads[1].root.as_deref(), Some(card.as_path()));
    assert_eq!(reads[0].reads.count, 0, "{reads:?}");
    let out = &r.out[from..];
    let silent = |j: usize| out[2 * j] == 0.0 && out[2 * j + 1] == 0.0;
    let quiet = |j: usize| out[2 * j].abs() < 0.05 && out[2 * j + 1].abs() < 0.05;
    let frames = out.len() / 2;
    let (mut j, mut gaps) = (0, 0);
    while j < frames {
        if !silent(j) {
            j += 1;
            continue;
        }
        let start = j;
        while j < frames && silent(j) {
            j += 1;
        }
        if j - start < 2 * DECLICK {
            continue;
        }
        gaps += 1;
        if start >= 4 {
            assert!(
                (start - 4..start).all(quiet),
                "a cliff into the gap at frame {start}: {:?}",
                &out[2 * (start - 4)..2 * start]
            );
        }
        if j + 4 <= frames {
            assert!(
                (j..j + 4).all(quiet),
                "a cliff out of the gap at frame {j}: {:?}",
                &out[2 * j..2 * (j + 4)]
            );
        }
    }
    assert!(gaps > 0, "a gap to look at");
    let _ = std::fs::remove_dir_all(&card);
}
