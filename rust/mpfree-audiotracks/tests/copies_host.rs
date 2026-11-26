//! Stretch's copies made through the host (the audio tracks rethink,
//! phase 5b): the copy a song region on Stretch wants is read, made on
//! the scheduler and played; a copy for a region edited while it was made
//! is dropped, never played, and the new one made; a clip the app lists
//! gets its copy; with no room in the copies' share a region plays
//! Repitch, and so does one the scheduler's memory check refuses; the
//! copies share the one budget for derived audio by priority; the
//! tracks emptied (a project opening) drop what was under way; Clear
//! copies lets them go and they are made again; a region on Keep pitch
//! drifts through a tempo change until the host makes the copy for the
//! new tempo, then rejoins the bars.
//!
//! The scheduler is a manual one the test steps between periods, so what
//! it does is the test's to see.

use std::time::{Duration, Instant};

use mpfree_audiotracks::copying::ClipWant;
use mpfree_audiotracks::host::AudioHost;
use mpfree_audiotracks::save::{TakeFile, TrackFile};
use mpfree_audiotracks::{
    ClipRegion, Command as A, FollowTempo, Input, Monitor, RecordingId, Region, MAX_CLIP_REGIONS,
};
use mpfree_stretch::sched::{Config, Priority, Sched};
use mpfree_transport::time::PPQ;
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: i64 = 4 * PPQ as i64;
const P: u64 = 256;
const REC: RecordingId = RecordingId(900);

fn tone(j: usize) -> f32 {
    (0.5 * (std::f64::consts::TAU * 441.0 * j as f64 / f64::from(RATE)).sin()) as f32
}

/// Two bars at 120 of a 441 Hz tone, as take 0.
fn tone_take() -> TakeFile {
    let frames = (8.0 * 60.0 * f64::from(RATE) / 120.0) as usize;
    TakeFile {
        id: 0,
        rec: REC,
        track: 0,
        group: 0,
        pass: 0,
        bpm: 120.0,
        varied: false,
        anchor: 0.0,
        frames,
        channels: 1,
        file: String::new(),
        path: None,
        audio: (0..frames).map(tone).collect(),
    }
}

fn stretch_region() -> Region {
    Region {
        follow: FollowTempo::Stretch,
        ..Region::of(0, 2 * BAR, 0, (0, 0.0), 120.0)
    }
}

struct Rig {
    t: Transport,
    h: AudioHost,
    s: Sched,
    b: Block,
    frame: u64,
    out: Vec<f32>,
}

impl Rig {
    fn new(bpm: f64, regions: Vec<Region>) -> Rig {
        Rig::on(Config::default(), bpm, regions)
    }

    fn on(config: Config, bpm: f64, regions: Vec<Region>) -> Rig {
        let s = Sched::manual(config);
        let mut r = Rig {
            t: Transport::new(RATE, bpm).unwrap(),
            h: AudioHost::new(RATE, 20),
            s: s.clone(),
            b: Block::new(),
            frame: 0,
            out: Vec::new(),
        };
        r.h.copies_on(s);
        assert!(r.h.tracks.load_take(tone_take()).is_ok());
        let track = TrackFile {
            index: 0,
            id: 0,
            input: Input::Left,
            monitor: Monitor::Auto,
            from: String::new(),
            regions,
        };
        assert!(r.h.tracks.load_track(&track));
        r
    }

    fn period(&mut self) {
        let n = P as usize;
        let inp = [0.0f32; 2 * P as usize];
        let mut mix = [0.0f32; 2 * P as usize];
        self.t.process(n as u32, &mut self.b);
        let map = *self.t.tempo_map();
        self.h
            .period(&self.b, &map, &inp[..2 * n], &mut mix[..2 * n]);
        self.out.extend_from_slice(&mix[..2 * n]);
        self.frame += P;
    }

    /// Periods, the scheduler stepped between them (`step`), until `done`.
    #[track_caller]
    fn run_until(&mut self, step: bool, mut done: impl FnMut(&mut Rig) -> bool) {
        let t0 = Instant::now();
        while !done(self) {
            assert!(
                t0.elapsed() < Duration::from_secs(60),
                "timed out at frame {}",
                self.frame
            );
            self.period();
            if step {
                self.s.step();
            }
            std::thread::sleep(Duration::from_micros(200));
        }
    }

    fn copies(&self) -> usize {
        self.h.copy_memory().1
    }

    fn l(&self, g: u64) -> f32 {
        self.out[2 * g as usize]
    }

    fn hz(&self, a: u64, b: u64) -> f64 {
        let ups = (a..b)
            .filter(|&g| self.l(g - 1) < 0.0 && self.l(g) >= 0.0)
            .count();
        ups as f64 * f64::from(RATE) / (b - a) as f64
    }
}

#[test]
fn a_song_regions_copy_is_read_made_on_the_scheduler_and_played() {
    let mut r = Rig::new(90.0, vec![stretch_region()]);
    // Nothing is asked for until the tempo has held for half a second.
    for _ in 0..RATE as u64 / 2 / P - 2 {
        r.period();
    }
    assert!(r.s.is_empty() && r.h.copy_memory().2 == 0);
    r.run_until(true, |r| r.copies() == 1);
    let c = r.h.copy_counts();
    assert_eq!(
        (c.made, c.stale, c.no_room, c.failed),
        (1, 0, 0, 0),
        "{c:?}"
    );
    assert!(r.s.counts().steps > 2, "made a step at a time");
    // It plays its copy: its own pitch at 90.
    let at = r.frame;
    assert!(r.t.command(at, T::PlayStart));
    r.run_until(false, |r| r.frame >= at + 3 * u64::from(RATE));
    let hz = r.hz(at + 2_000, at + 3 * u64::from(RATE) - 500);
    assert!((hz - 441.0).abs() <= 0.5, "{hz} Hz");
    let st = r.h.tracks.stats();
    assert!(st.copied > 0 && st.for_now == 0, "{st:?}");
}

#[test]
fn a_copy_for_a_region_edited_while_it_was_made_is_dropped_and_the_new_one_made() {
    let mut r = Rig::new(90.0, vec![stretch_region()]);
    // Its source read and handed to the scheduler, not stepped yet.
    r.run_until(false, |r| !r.s.is_empty());
    // Its tempo set by hand meanwhile: another copy is wanted.
    r.h.command(A::SetTempo {
        track: 0,
        at: 0,
        bpm: 110.0,
    });
    r.run_until(true, |r| r.h.copy_counts().made == 1);
    let c = r.h.copy_counts();
    // The first was cancelled on the scheduler, or came and was dropped.
    assert!(
        r.s.counts().cancelled + c.stale >= 1,
        "{c:?} {:?}",
        r.s.counts()
    );
    assert_eq!(r.copies(), 1);
    let key =
        r.h.tracks
            .song_copy_key(&r.h.tracks.regions(0)[0], r.t.tempo_map())
            .unwrap();
    assert!(
        r.h.tracks.has_copy(&key),
        "the copy held is the edited region's"
    );
}

#[test]
fn a_clip_the_app_lists_gets_its_copy_at_the_tempo_at_the_playhead() {
    let mut r = Rig::new(90.0, vec![]);
    let region = ClipRegion {
        start: 0,
        end: 2 * BAR,
        rec: REC,
        src: 0,
        phase: 0.0,
        gain: 1.0,
        fade_in: 0,
        fade_out: 0,
        into: 0,
        left: 0,
        follow: FollowTempo::Stretch,
        keep_pitch: false,
        bpm: 0.0,
    };
    let mut regions = [region; MAX_CLIP_REGIONS];
    regions[0] = region;
    r.h.clip_copies(vec![ClipWant {
        regions,
        n: 1,
        len: 2 * BAR,
        last_frame: tone_take().frames as i64,
        priority: Priority::Prepared,
    }]);
    r.run_until(true, |r| r.copies() == 1);
    let v: Vec<_> = r.h.tracks.copies().collect();
    assert_eq!(v[0].key.ends, mpfree_audiotracks::copies::Ends::Wrapped);
    assert_eq!(f64::from_bits(v[0].key.tempo), 90.0);
}

#[test]
fn with_no_room_in_the_copies_share_a_region_plays_repitch() {
    let mut r = Rig::new(90.0, vec![stretch_region()]);
    r.s.budget().set_total(10_000);
    r.run_until(false, |r| r.h.copy_counts().no_room > 0);
    for _ in 0..200 {
        r.period();
        r.s.step();
    }
    assert_eq!(r.copies(), 0);
    assert!(r.s.is_empty(), "nothing was made");
    let at = r.frame;
    assert!(r.t.command(at, T::PlayStart));
    r.run_until(false, |r| r.frame >= at + u64::from(RATE));
    let hz = r.hz(at + 2_000, at + u64::from(RATE) - 500);
    assert!((hz - 441.0 * 90.0 / 120.0).abs() <= 1.0, "{hz} Hz");
}

#[test]
fn a_copy_the_schedulers_memory_check_refuses_is_counted_and_its_region_plays_repitch() {
    let config = Config {
        promise: |_| Err("no room".into()),
        ..Config::default()
    };
    let mut r = Rig::on(config, 90.0, vec![stretch_region()]);
    r.run_until(true, |r| r.h.copy_counts().refused > 0);
    // Not asked for again until what is wanted changes.
    for _ in 0..200 {
        r.period();
        r.s.step();
    }
    let c = r.h.copy_counts();
    assert_eq!((c.refused, c.failed, c.made), (1, 0, 0), "{c:?}");
    assert_eq!(r.s.counts().refused, 1);
    let at = r.frame;
    assert!(r.t.command(at, T::PlayStart));
    r.run_until(false, |r| r.frame >= at + u64::from(RATE));
    let hz = r.hz(at + 2_000, at + u64::from(RATE) - 500);
    assert!((hz - 441.0 * 90.0 / 120.0).abs() <= 1.0, "{hz} Hz");
}

#[test]
fn the_copies_share_the_one_budget_with_the_other_caches_by_priority() {
    use mpfree_stretch::budget::Cache;
    // One region at the playhead (Protect), and one bar of the same take
    // twenty bars on (Speculative; a region of the same frames would share
    // the first one's copy).
    let far = Region {
        follow: FollowTempo::Stretch,
        ..Region::of(20 * BAR, 21 * BAR, 0, (0, 0.0), 120.0)
    };
    let mut r = Rig::new(90.0, vec![stretch_region(), far]);
    let b = r.s.budget();
    b.set_total(4_000_000);
    // The sampler's Warps hold 2.5 MB: past the half a copy made just in
    // case may fill, within the whole a copy needed now may.
    b.set_held(Cache::Warps, 2_500_000);
    r.run_until(true, |r| r.copies() == 1);
    for _ in 0..300 {
        r.period();
        r.s.step();
    }
    assert_eq!(r.copies(), 1, "{:?}", r.h.copy_counts());
    assert!(
        r.h.copy_counts().no_room > 0,
        "{:?} {:?} {:?}",
        r.h.copy_counts(),
        r.s.counts(),
        r.h.copy_memory()
    );
    let v: Vec<_> = r.h.tracks.copies().collect();
    assert_eq!(
        v[0].key.out,
        r.h.tracks
            .song_copy_key(&r.h.tracks.regions(0)[0], r.t.tempo_map())
            .unwrap()
            .out
    );
    // It says what its copies use.
    assert_eq!(b.uses(Cache::Tracks), r.h.copy_memory().0 as u64);
    // The Warps let go: the far one is made too.
    b.set_held(Cache::Warps, 0);
    r.run_until(true, |r| r.copies() == 2);
    assert_eq!(b.uses(Cache::Tracks), r.h.copy_memory().0 as u64);
    assert!(b.used() <= b.total());
}

#[test]
fn the_tracks_emptied_drop_what_was_under_way_and_clear_copies_lets_them_go() {
    let mut r = Rig::new(90.0, vec![stretch_region()]);
    r.run_until(false, |r| !r.s.is_empty());
    // A project opening: its tracks emptied while the copy waits on the
    // scheduler.
    assert!(r.h.clear());
    // Cancelled there (the host's worker tells the scheduler), never
    // stepped.
    r.run_until(false, |r| r.s.is_empty());
    assert_eq!(r.s.counts().steps, 0);
    for _ in 0..100 {
        r.period();
        r.s.step();
    }
    assert_eq!(r.copies(), 0);
    assert_eq!(r.h.copy_counts().made, 0);
    // A new project's region on Stretch gets its own.
    assert!(r.h.tracks.load_take(tone_take()).is_ok());
    let track = TrackFile {
        index: 0,
        id: 0,
        input: Input::Left,
        monitor: Monitor::Auto,
        from: String::new(),
        regions: vec![stretch_region()],
    };
    assert!(r.h.tracks.load_track(&track));
    r.run_until(true, |r| r.copies() == 1);
    // Clear copies: it goes (nothing plays), no recording is touched, and
    // it is made again.
    assert_eq!(r.h.clear_copies(), 1);
    assert_eq!(r.copies(), 0);
    let mut back = vec![0.0f32; tone_take().frames];
    assert_eq!(r.h.tracks.read_take(0, 0, &mut back), Some(back.len()));
    assert_eq!(back, tone_take().audio);
    r.run_until(true, |r| r.copies() == 1);
    assert_eq!(r.h.copy_counts().made, 2);
}

#[test]
fn a_region_on_keep_pitch_drifts_through_a_tempo_change_until_its_copy_is_made_then_rejoins() {
    let region = Region {
        keep_pitch: true,
        ..stretch_region()
    };
    let mut r = Rig::new(90.0, vec![region]);
    r.run_until(true, |r| r.copies() == 1);
    let at = r.frame;
    assert!(r.t.command(at, T::PlayStart));
    r.run_until(true, |r| r.frame >= at + 2_048);
    // Turned to 100 a few periods in: it plays on in pitch from its copy
    // for 90 while the host makes the copy for 100 (the copy for 90 still
    // plays, so it stays).
    let c = r.frame;
    assert!(r.t.command(c, T::TempoNow(100.0)));
    r.run_until(true, |r| r.copies() == 2);
    let made = r.frame;
    assert_eq!(r.h.copy_counts().made, 2);
    // It rejoins on the next bar line, bar 2's, and holds the new copy to
    // its end.
    let beat100 = 60.0 * f64::from(RATE) / 100.0;
    let beats_at_c = (c - at) as f64 / (60.0 * f64::from(RATE) / 90.0);
    let bar2 = c + ((4.0 - beats_at_c) * beat100) as u64;
    assert!(made < bar2, "the copy came before bar 2's line, at {made}");
    let end = c + ((8.0 - beats_at_c) * beat100) as u64;
    r.run_until(true, |r| r.frame >= end + 2_048);
    let hz = r.hz(c + 500, end - 500);
    assert!((hz - 441.0).abs() <= 0.5, "in pitch all the way: {hz} Hz");
    let st = r.h.tracks.stats();
    assert_eq!(st.for_now, 0, "never Repitch: {st:?}");
    let drift = (bar2 - c) as f64;
    assert!(
        (st.drifted as f64 - drift).abs() <= 2.0 * P as f64 + 64.0,
        "drifted to bar 2's line: {st:?}, {drift}"
    );
}
