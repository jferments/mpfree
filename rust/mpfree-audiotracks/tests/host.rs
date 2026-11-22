//! The audio tracks in their host: a save made across periods while the
//! worker makes room, opened in a fresh host, plays back the same; a save
//! asked for while recording waits for the recording to end; a fit to a
//! new tempo is read, stretched on the worker, loaded and swapped in; a
//! cancelled save or waveform never comes; cleared tracks take a part
//! back under its own take ids (a project opening); a sound brought in
//! plays from its bar, a region a slice, as one undo step.

use std::time::{Duration, Instant};

use mpfree_audiotracks::host::{AudioHost, ImportRefused};
use mpfree_audiotracks::Command as A;
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050; // at 240 BPM
const BAR_TICKS: i64 = 3_840;
const START: u64 = 1_000;
const P: u64 = 256;

fn tone(g: u64) -> f32 {
    (0.5 * (std::f64::consts::TAU * 441.0 * g as f64 / f64::from(RATE)).sin()) as f32
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
    fn new() -> Rig {
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            h: AudioHost::new(RATE, 20),
            b: Block::new(),
            frame: 0,
            out: Vec::new(),
            live: true,
        }
    }

    fn period(&mut self, n: u64) {
        let n = n as usize;
        let (mut inp, mut mix) = ([0.0f32; 2 * P as usize], [0.0f32; 2 * P as usize]);
        for j in 0..n {
            let s = if self.live {
                tone(self.frame + j as u64)
            } else {
                0.0
            };
            inp[2 * j] = s;
            inp[2 * j + 1] = s;
        }
        self.t.process(n as u32, &mut self.b);
        let map = *self.t.tempo_map();
        self.h
            .period(&self.b, &map, &inp[..2 * n], &mut mix[..2 * n]);
        self.out.extend_from_slice(&mix[..2 * n]);
        self.frame += n as u64;
    }

    fn run_to(&mut self, frame: u64) {
        while self.frame < frame {
            self.period(P.min(frame - self.frame));
        }
    }

    /// Run periods until `done`, giving the worker time between them.
    #[track_caller]
    fn run_until(&mut self, mut done: impl FnMut(&mut Rig) -> bool) {
        let t0 = Instant::now();
        while !done(self) {
            assert!(
                t0.elapsed() < Duration::from_secs(60),
                "timed out at frame {}",
                self.frame
            );
            self.period(P);
            std::thread::sleep(Duration::from_micros(200));
        }
    }

    /// Record one bar on track 0 from START.
    fn record_bar(&mut self) {
        self.h.command(A::Arm(0, true));
        self.t.command(START, T::Record);
        self.t.command(START + BAR, T::Stop);
        self.run_to(START + BAR + 500);
        self.h.command(A::Arm(0, false));
        self.live = false;
        self.run_to(self.frame + 10);
    }

    /// Play from the song start for `frames`; the output played.
    fn play(&mut self, frames: u64) -> Vec<f32> {
        let p = self.frame + 10;
        self.t.command(p, T::Locate(0));
        self.t.command(p, T::PlayStart);
        self.run_to(p + frames);
        let s = self.out[2 * p as usize..].to_vec();
        self.t.command(self.frame, T::Stop);
        self.run_to(self.frame + 10);
        s
    }
}

fn rms(out: &[f32], from: u64, to: u64) -> f64 {
    let s: f64 = (from..to)
        .map(|g| f64::from(out[2 * g as usize]).powi(2))
        .sum();
    (s / (to - from) as f64).sqrt()
}

#[test]
fn a_save_made_across_periods_opens_in_a_fresh_host_and_plays_the_same() {
    let mut r = Rig::new();
    r.record_bar();
    assert!(r.h.save());
    assert!(!r.h.save(), "one save at a time");
    let mut saved = None;
    r.run_until(|r| {
        saved = r.h.take_saved();
        saved.is_some()
    });
    assert!(r.h.saving().is_none());
    let saved = saved.unwrap();
    assert_eq!(saved.takes.len(), 1);
    let k = &saved.takes[0];
    let mut direct = vec![0.0; k.frames * k.channels];
    assert_eq!(r.h.tracks.read_take(k.id, 0, &mut direct), Some(k.frames));
    assert_eq!(k.audio, direct);
    let before = r.play(BAR + 1000);
    assert!(rms(&before, 1000, BAR - 1000) > 0.2);

    let mut s = Rig::new();
    s.live = false;
    assert!(s.h.load(saved).is_ok());
    s.run_until(|s| s.h.tracks.loading() == 0);
    let after = s.play(BAR + 1000);
    assert_eq!(before, after);
}

#[test]
fn a_save_asked_for_while_recording_waits_for_the_recording_to_end() {
    let mut r = Rig::new();
    r.h.command(A::Arm(0, true));
    r.t.command(START, T::Record);
    r.run_to(START + BAR / 2);
    assert!(r.h.save());
    for _ in 0..20 {
        r.period(P);
        std::thread::sleep(Duration::from_micros(200));
        assert_eq!(r.h.saving(), Some(0.0));
        assert!(r.h.take_saved().is_none());
    }
    r.t.command(r.frame, T::Stop);
    r.run_to(r.frame + 500);
    r.h.command(A::Arm(0, false));
    let mut saved = None;
    r.run_until(|r| {
        saved = r.h.take_saved();
        saved.is_some()
    });
    let saved = saved.unwrap();
    assert_eq!(saved.takes.len(), 1);
    assert!(!saved.takes[0].audio.is_empty());
}

#[test]
fn a_fit_through_the_host_fills_the_bar_at_the_new_tempo() {
    let mut r = Rig::new();
    r.record_bar();
    // Nothing to fit at the tempo it was recorded at, or a take not there.
    assert!(!r.h.fit(0, 240.0));
    assert!(!r.h.fit(7, 120.0));
    r.t.command(r.frame, T::SetTempo(120.0));
    r.run_to(r.frame + 10);
    assert!(r.h.fit(0, 120.0));
    assert_eq!(r.h.fitting(), Some(0));
    assert!(!r.h.fit(0, 120.0), "one fit at a time");
    r.run_until(|r| r.h.fits + r.h.fits_failed > 0);
    assert_eq!((r.h.fits, r.h.fits_failed), (1, 0));
    assert_eq!(r.h.fitting(), None);
    // The swap is applied at the next period, as every command is.
    r.run_to(r.frame + 10);
    let reg = r.h.tracks.regions(0)[0];
    assert_ne!(reg.take, 0);
    assert_eq!(reg.end, BAR_TICKS);
    assert_eq!(r.h.tracks.take(reg.take).unwrap().bpm, 120.0);
    // The bar is two seconds now, and the take fills it.
    let out = r.play(2 * BAR);
    assert!(rms(&out, 1000, 2 * BAR - 1000) > 0.2);
    // Undo plays the old take again.
    r.h.command(A::Undo);
    r.run_to(r.frame + 10);
    assert_eq!(r.h.tracks.regions(0)[0].take, 0);
}

/// Phase 0.3: a fit still under way when the tracks are emptied (a
/// project opening) never reaches the new project's takes, though take ids
/// come back after a clear: whatever the worker still sends for it is
/// dropped as it comes and counted. Tried with the clear at every stage of
/// the fit (the worker's buffer, the reading, the stretch, the swap).
#[test]
fn a_fit_from_before_a_project_opened_never_reaches_the_new_takes() {
    let mut dropped = 0;
    for periods in [0u64, 1, 2, 4, 8, 16, 32, 64] {
        let mut r = Rig::new();
        r.record_bar();
        r.t.command(r.frame, T::SetTempo(120.0));
        r.run_to(r.frame + 10);
        assert!(r.h.fit(0, 120.0));
        for _ in 0..periods {
            r.period(P);
        }
        // The new project: emptied, then a bar of its own recorded at 120,
        // which gets take 0 again.
        assert!(r.h.clear(), "after {periods} periods");
        assert_eq!(r.h.fitting(), None);
        assert_eq!(r.h.tracks.takes().count(), 0);
        r.live = true;
        let at = r.frame + 1000;
        r.h.command(A::Arm(0, true));
        r.t.command(at, T::Locate(0));
        r.t.command(at, T::Record);
        r.t.command(at + 2 * BAR, T::Stop);
        r.run_to(at + 2 * BAR + 500);
        r.h.command(A::Arm(0, false));
        r.live = false;
        // Time for anything the worker still had to come back.
        let until = Instant::now() + Duration::from_millis(300);
        r.run_until(|_| Instant::now() > until);
        let takes: Vec<_> = r.h.tracks.takes().collect();
        assert_eq!(takes.len(), 1, "after {periods} periods: {takes:?}");
        assert_eq!(
            (takes[0].id, takes[0].bpm),
            (0, 120.0),
            "after {periods} periods"
        );
        assert_eq!(r.h.tracks.regions(0).len(), 1, "after {periods} periods");
        assert_eq!(r.h.tracks.regions(0)[0].take, 0, "after {periods} periods");
        assert_eq!(r.h.fits, 0, "after {periods} periods");
        dropped += r.h.stale;
    }
    assert!(
        dropped > 0,
        "a result from before the clear came and was dropped"
    );
}

#[test]
fn a_waveform_read_through_the_host_matches_the_take() {
    let mut r = Rig::new();
    r.record_bar();
    assert!(!r.h.wave(9, 100), "no such take");
    assert!(r.h.wave(0, 100));
    let mut w = None;
    r.run_until(|r| {
        w = r.h.take_wave();
        w.is_some()
    });
    let w = w.unwrap();
    let t = r.h.tracks.take(0).unwrap();
    assert_eq!((w.id, w.frames, w.channels), (0, t.frames, t.channels));
    assert_eq!(w.frames_per_column % (256 / t.channels), 0);
    assert!((80..=100).contains(&w.columns.len()), "{}", w.columns.len());
    assert_eq!(w.columns.len(), t.frames.div_ceil(w.frames_per_column));
    // Each column against the samples themselves.
    for (c, &(lo, hi)) in w.columns.iter().enumerate() {
        let a = c * w.frames_per_column;
        let b = (a + w.frames_per_column).min(t.frames);
        let (mut l, mut h) = (f32::MAX, f32::MIN);
        for k in a..b {
            for ch in 0..t.channels {
                let s = r.h.tracks.sample(0, k, ch).unwrap();
                l = l.min(s);
                h = h.max(s);
            }
        }
        assert_eq!((lo, hi), (l, h), "column {c}");
    }
    // A tone at 0.5: the middle columns reach near it.
    assert!(w.columns[w.columns.len() / 2].1 > 0.45);
    // Asking again replaces a wave not yet done; one result comes.
    assert!(r.h.wave(0, 50));
    assert!(r.h.wave(0, 20));
    let mut w = None;
    r.run_until(|r| {
        w = r.h.take_wave();
        w.is_some()
    });
    assert!(w.unwrap().columns.len() <= 20);
    for _ in 0..50 {
        r.period(P);
        std::thread::sleep(Duration::from_micros(200));
        assert!(r.h.take_wave().is_none());
    }
}

#[test]
fn a_cancelled_save_leaves_nothing_behind_and_the_next_save_works() {
    let mut r = Rig::new();
    r.record_bar();
    // Cancelled while sizing or reading, and again with one finished but
    // not taken: nothing comes from it.
    for wait in [0, 3, 400] {
        assert!(r.h.save());
        for _ in 0..wait {
            r.period(P);
            std::thread::sleep(Duration::from_micros(200));
        }
        r.h.cancel_save();
        assert!(r.h.saving().is_none());
        for _ in 0..50 {
            r.period(P);
            std::thread::sleep(Duration::from_micros(200));
            assert!(r.h.take_saved().is_none());
        }
    }
    assert!(r.h.save());
    let mut saved = None;
    r.run_until(|r| {
        saved = r.h.take_saved();
        saved.is_some()
    });
    let saved = saved.unwrap();
    assert_eq!(saved.takes.len(), 1);
    assert_eq!(saved.takes[0].audio.len(), saved.takes[0].frames);
}

#[test]
fn clear_empties_the_tracks_and_a_part_loads_over_them() {
    let mut r = Rig::new();
    r.record_bar();
    // Level, pan, mute and solo aren't saved (the app's mixer and track
    // list keep them); the input and monitor are.
    r.h.command(A::SetMonitor(1, mpfree_audiotracks::Monitor::Off));
    r.h.command(A::SetInput(1, mpfree_audiotracks::Input::Right));
    r.run_to(r.frame + 10);
    assert!(r.h.save());
    let mut saved = None;
    r.run_until(|r| {
        saved = r.h.take_saved();
        saved.is_some()
    });
    let saved = saved.unwrap();
    let before = r.play(BAR + 1000);

    // Refused while a track records.
    r.live = true;
    r.h.command(A::Arm(2, true));
    r.t.command(r.frame + 10, T::Record);
    r.run_to(r.frame + 2000);
    assert!(r.h.tracks.recording(2).is_some());
    assert!(!r.h.tracks.clear());
    r.t.command(r.frame, T::Stop);
    r.h.command(A::Arm(2, false));
    r.live = false;
    r.run_until(|r| r.h.tracks.clear());

    assert_eq!(r.h.tracks.takes().count(), 0);
    assert_eq!(r.h.tracks.memory().used, 0);
    assert_eq!(r.h.tracks.history(), (0, 0));
    for t in 0..mpfree_audiotracks::MAX_TRACKS {
        assert!(r.h.tracks.regions(t).is_empty());
        assert!(!r.h.tracks.armed(t));
        let s = r.h.tracks.settings(t).unwrap();
        assert_eq!((s.level, s.pan, s.mute, s.solo), (1.0, 0.0, false, false));
    }
    assert!(r
        .h
        .tracks
        .settings(mpfree_audiotracks::MAX_TRACKS)
        .is_none());
    let silent = r.play(BAR + 1000);
    assert!(rms(&silent, 0, BAR) < 1e-6);

    // The saved part goes back in under its own take ids.
    assert!(r.h.load(saved).is_ok());
    r.run_until(|r| r.h.tracks.loading() == 0);
    let s = r.h.tracks.settings(1).unwrap();
    assert_eq!(
        (s.monitor, s.input),
        (
            mpfree_audiotracks::Monitor::Off,
            mpfree_audiotracks::Input::Right
        )
    );
    assert_eq!(r.play(BAR + 1000), before);
}

#[test]
fn a_cancelled_waveform_never_comes() {
    let mut r = Rig::new();
    r.record_bar();
    for wait in [0, 2, 400] {
        assert!(r.h.wave(0, 100));
        for _ in 0..wait {
            r.period(P);
            std::thread::sleep(Duration::from_micros(200));
        }
        r.h.cancel_wave();
        for _ in 0..50 {
            r.period(P);
            std::thread::sleep(Duration::from_micros(200));
            assert!(r.h.take_wave().is_none(), "after waiting {wait}");
        }
    }
    assert!(r.h.wave(0, 100));
    let mut w = None;
    r.run_until(|r| {
        w = r.h.take_wave();
        w.is_some()
    });
    assert_eq!(w.unwrap().id, 0);
}

/// The one Sample page's step 6 (Put on Audio track): a sound brought in
/// is a take of its own on the track asked; it copies in, then plays from
/// the bar asked to its end, a region a slice, as one undo step. A second
/// waits for the first, and one bigger than the memory is refused.
#[test]
fn a_sound_brought_in_plays_from_its_bar_as_one_undo_step() {
    let mut r = Rig::new();
    r.live = false;
    // Half a bar of the tone, stereo, onto track 3 at bar 2.
    let half = (BAR / 2) as usize;
    let audio: Vec<f32> = (0..half)
        .flat_map(|g| {
            let s = tone(g as u64);
            [s, s]
        })
        .collect();
    // Two slices: cut a quarter of a bar in (a cut past the end is left
    // out).
    let cuts = vec![half as u64 / 2, 10 * BAR];
    let id =
        r.h.import(2, BAR_TICKS, 2, audio.clone(), cuts, 240.0)
            .unwrap();
    assert_eq!(r.h.importing(), Some(id));
    assert_eq!(
        r.h.import(3, 0, 2, audio, Vec::new(), 0.0),
        Err(ImportRefused::Busy)
    );
    r.run_until(|r| r.h.importing().is_none());
    assert_eq!(r.h.imports, 1);
    let take = r.h.tracks.take(id).unwrap();
    assert_eq!(
        (take.track, take.channels, take.frames, take.bpm),
        (2, 2, half, 240.0)
    );
    let regs = r.h.tracks.regions(2);
    let q = BAR_TICKS / 4;
    assert_eq!(regs.len(), 2, "{regs:?}");
    assert_eq!(
        (regs[0].take, regs[0].start, regs[0].src),
        (id, BAR_TICKS, 0)
    );
    assert!((regs[0].end - (BAR_TICKS + q)).abs() <= 1, "{regs:?}");
    assert_eq!((regs[1].take, regs[1].start), (id, regs[0].end));
    assert!((regs[1].src - half as i64 / 2).abs() <= 1, "{regs:?}");
    assert!((regs[1].end - (BAR_TICKS + 2 * q)).abs() <= 1, "{regs:?}");
    // It plays in bar 2, across the cut too, and nothing before.
    let out = r.play(2 * BAR);
    assert_eq!(rms(&out, 1000, BAR - 1000), 0.0);
    assert!(rms(&out, BAR + 1000, BAR + half as u64 - 1000) > 0.2);
    let cut = BAR + half as u64 / 2;
    assert!(rms(&out, cut - 200, cut + 200) > 0.2);
    // One undo takes both slices off.
    assert!(r.h.command(A::Undo));
    r.run_to(r.frame + 10);
    assert!(r.h.tracks.regions(2).is_empty());
    // Bigger than the memory (20 s): refused, and nothing waits.
    let big = vec![0.0; 2 * RATE as usize * 30];
    assert_eq!(
        r.h.import(1, 0, 2, big, Vec::new(), 0.0),
        Err(ImportRefused::NoRoom)
    );
    assert_eq!(r.h.importing(), None);
}
