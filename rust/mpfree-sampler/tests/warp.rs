//! Warped regions: at the project tempo by repitching, and by a stretched
//! copy made off the audio thread.

use mpfree_analysis::yin;
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_sampler::warp::{CopyKey, Warps};

const RATE: u32 = 44_100;

/// One second of a 220 Hz sine, stereo.
fn sine(seconds: f64, hz: f64) -> Source {
    let n = (seconds * f64::from(RATE)) as usize;
    let mut v = Vec::with_capacity(n * 2);
    for i in 0..n {
        let x = (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(RATE)).sin()) as f32;
        v.extend([x, x]);
    }
    Source::new("sine", RATE, 2, v).unwrap()
}

fn warped_pad(s: &Source, bpm: f32, method: WarpMethod) -> Pad {
    let mut r = Region::whole(s);
    r.warp = Some(Warp { bpm, method });
    Pad::new("loop", Layer::new(r))
}

fn project(s: &Source, pad: Pad) -> Project {
    let mut p = Project::new();
    p.add_source(s.clone());
    p.set_pad(0, Some(pad), None).unwrap();
    p
}

/// Hits pad 0 and renders until it ends: the left channel.
fn play(e: &mut Engine) -> Vec<f32> {
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let mut out = Vec::new();
    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
    while out.len() < 10 * RATE as usize {
        e.render(&mut l, &mut r);
        out.extend_from_slice(&l);
        if e.active_voices() == 0 {
            break;
        }
    }
    let end = out
        .iter()
        .rposition(|v| v.abs() > 1e-6)
        .map_or(0, |i| i + 1);
    out.truncate(end);
    out
}

fn pitch(x: &[f32]) -> f32 {
    let m = x.len() / 2;
    yin(&x[m - 4096..m + 4096], RATE, 4096, 60.0, 2000.0, 0.15)
        .expect("pitched")
        .0
}

fn engine_at(p: &Project, warps: &mut Warps) -> Engine {
    let mut e = Engine::new(RATE, 8);
    e.set_tempo(warps.tempo());
    e.set_view(EngineView::build_warped(p, RATE, warps));
    e
}

/// Makes every copy the project is missing, as a worker would.
fn make_copies(p: &Project, warps: &mut Warps) -> usize {
    let missing = warps.missing(p);
    for k in &missing {
        let mut job = Warps::job(p, k).unwrap();
        while !job.run(4096).finished {}
        warps.insert(*k, job.into_output().unwrap(), p);
    }
    missing.len()
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn repitch_plays_faster_and_higher_at_a_faster_tempo() {
    let s = sine(1.0, 220.0);
    let p = project(&s, warped_pad(&s, 100.0, WarpMethod::Repitch));
    let mut warps = Warps::new(120.0);
    assert!(warps.missing(&p).is_empty(), "repitch needs no copy");
    let out = play(&mut engine_at(&p, &mut warps));
    // 1.2 times as fast: 5/6 of a second, a fifth-ish higher.
    assert!(
        near(out.len() as f64, 44_100.0 / 1.2, 64.0),
        "{}",
        out.len()
    );
    assert!((pitch(&out) - 264.0).abs() < 1.5, "{}", pitch(&out));
    // Without warping (the plain view) it plays as recorded.
    let mut e = Engine::new(RATE, 8);
    e.set_tempo(120.0);
    e.set_view(EngineView::build(&p, RATE));
    assert!(near(play(&mut e).len() as f64, 44_100.0, 64.0));
}

#[test]
fn stretch_repitches_until_its_copy_is_made_then_keeps_the_pitch() {
    let s = sine(1.0, 220.0);
    let p = project(&s, warped_pad(&s, 100.0, WarpMethod::Stretch));
    let mut warps = Warps::new(120.0);
    assert_eq!(warps.missing(&p).len(), 1);
    // No copy yet: on the beat by repitching.
    let before = play(&mut engine_at(&p, &mut warps));
    assert!(near(before.len() as f64, 44_100.0 / 1.2, 64.0));
    assert!((pitch(&before) - 264.0).abs() < 1.5);
    // The copy: the same length, the pitch it was recorded at.
    assert_eq!(make_copies(&p, &mut warps), 1);
    assert!(warps.missing(&p).is_empty());
    let after = play(&mut engine_at(&p, &mut warps));
    assert!(
        near(after.len() as f64, 44_100.0 / 1.2, 64.0),
        "{}",
        after.len()
    );
    assert!((pitch(&after) - 220.0).abs() < 1.0, "{}", pitch(&after));
}

#[test]
fn a_slice_tap_on_a_stretched_pad_plays_that_slice_of_its_copy() {
    // A second of 220 Hz, then a second of 440 Hz, warped from 100 to 120
    // BPM: the copy is 1/1.2 as long, so each second is 36750 frames of it.
    let mut v = sine(1.0, 220.0).data().to_vec();
    v.extend_from_slice(&sine(1.0, 440.0).data()[..]);
    let s = Source::new("two", RATE, 2, v).unwrap();
    let p = project(&s, warped_pad(&s, 100.0, WarpMethod::Stretch));
    let mut warps = Warps::new(120.0);
    assert_eq!(make_copies(&p, &mut warps), 1);
    for (slice, hz) in [((0, 44_100), 220.0), ((44_100, 88_200), 440.0)] {
        let mut e = engine_at(&p, &mut warps);
        // The slice in source frames, as the Chop tab gives it.
        e.apply(Event::Audition {
            pad: 0,
            layer: 0,
            start: slice.0,
            end: slice.1,
            velocity: 127,
        });
        let mut out = Vec::new();
        let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
        while e.active_voices() > 0 && out.len() < 4 * RATE as usize {
            e.render(&mut l, &mut r);
            out.extend_from_slice(&l);
        }
        let end = out
            .iter()
            .rposition(|v| v.abs() > 1e-6)
            .map_or(0, |i| i + 1);
        out.truncate(end);
        assert!(
            near(out.len() as f64, 36_750.0, 300.0),
            "{slice:?}: {}",
            out.len()
        );
        assert!((pitch(&out) - hz).abs() < 2.0, "{slice:?}: {}", pitch(&out));
    }
}

#[test]
fn a_stretched_copy_carries_the_tune_and_keeps_the_length() {
    let s = sine(1.0, 220.0);
    let mut pad = warped_pad(&s, 120.0, WarpMethod::Stretch);
    pad.layers[0].region.tune_cents = 700;
    let p = project(&s, pad);
    let mut warps = Warps::new(90.0);
    make_copies(&p, &mut warps);
    let out = play(&mut engine_at(&p, &mut warps));
    // 120 to 90 BPM: 4/3 as long, and a fifth up.
    assert!(
        near(out.len() as f64, 44_100.0 * 4.0 / 3.0, 64.0),
        "{}",
        out.len()
    );
    let want = 220.0 * 2f32.powf(7.0 / 12.0);
    assert!(
        (pitch(&out) - want).abs() < 1.5,
        "{} not {want}",
        pitch(&out)
    );
}

#[test]
fn a_playing_loop_follows_a_tempo_change_at_once() {
    let s = sine(1.0, 220.0);
    let mut pad = warped_pad(&s, 120.0, WarpMethod::Repitch);
    let r = &mut pad.layers[0].region;
    r.mode = PlayMode::Loop;
    r.sustain_loop = Some(Loop {
        start: 0,
        end: s.frames(),
        crossfade: 0,
        mode: LoopMode::Forward,
    });
    let p = project(&s, pad);
    let mut warps = Warps::new(120.0);
    let mut e = engine_at(&p, &mut warps);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let render = |e: &mut Engine| {
        let (mut l, mut r) = (vec![0.0; 16_384], vec![0.0; 16_384]);
        for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
            e.render(a, b);
        }
        l
    };
    assert!((pitch(&render(&mut e)) - 220.0).abs() < 1.0);
    e.set_tempo(180.0);
    assert_eq!(e.tempo(), 180.0);
    assert!((pitch(&render(&mut e)) - 330.0).abs() < 1.5);
}

#[test]
fn copies_stay_within_the_budget_but_never_the_wanted_ones() {
    let s = sine(1.0, 220.0);
    let p = project(&s, warped_pad(&s, 100.0, WarpMethod::Stretch));
    let mut warps = Warps::new(100.0);
    // One copy at 100 BPM is the source's size: 44100 frames, stereo.
    let one = 44_100 * 2 * 4;
    warps.set_budget(one + one / 2);
    make_copies(&p, &mut warps);
    let at_100 = warps.wanted(&p)[0];
    assert_eq!(warps.bytes(), one);
    // At 110 BPM the 100 BPM copy is no longer wanted and goes to make room.
    warps.set_tempo(110.0);
    make_copies(&p, &mut warps);
    assert!(warps.peek(&at_100).is_none());
    assert!(warps.missing(&p).is_empty());
    // A budget too small for the one wanted copy still keeps it.
    warps.set_budget(1);
    let k = warps.wanted(&p)[0];
    warps.insert(k, vec![0.0; 10], &p);
    assert!(warps.peek(&k).is_some());
}

#[test]
fn a_copy_key_says_how_long_and_how_high() {
    let s = sine(1.0, 220.0);
    let mut r = Region::whole(&s);
    r.start = 4410;
    r.end = 26_460;
    let w = Warp::new(90.0);
    let k = CopyKey::of(&r, &w, 300, 120.0);
    assert_eq!(
        (k.start, k.end, k.frames, k.cents),
        (4410, 26_460, 16_538, 300)
    );
    assert!((k.scale() - 0.75).abs() < 1e-4);
    // A region in granular mode or warped to repitch needs no copy.
    let mut pad = Pad::new(
        "x",
        Layer::new(Region {
            warp: Some(w),
            ..r.clone()
        }),
    );
    let mut p = Project::new();
    p.add_source(s.clone());
    pad.layers[0].region.mode = PlayMode::Granular;
    p.set_pad(0, Some(pad.clone()), None).unwrap();
    assert!(Warps::new(120.0).wanted(&p).is_empty());
}

#[test]
fn a_warp_tempo_out_of_range_is_refused() {
    let s = sine(0.1, 220.0);
    let mut p = Project::new();
    p.add_source(s.clone());
    for (bpm, ok) in [
        (19.0, false),
        (20.0, true),
        (999.0, true),
        (1000.0, false),
        (f32::NAN, false),
    ] {
        let pad = warped_pad(&s, bpm, WarpMethod::Stretch);
        assert_eq!(p.set_pad(0, Some(pad), None).is_ok(), ok, "{bpm}");
    }
}

#[test]
fn process_makes_a_new_source_in_the_layers_place_and_undo_brings_it_back() {
    use mpfree_sampler::warp::Process;
    let s = sine(1.0, 220.0);
    let mut pad = warped_pad(&s, 100.0, WarpMethod::Stretch);
    let r = &mut pad.layers[0].region;
    (r.start, r.end) = (4410, 44_100);
    r.sustain_loop = Some(Loop {
        start: 8820,
        end: 44_100,
        crossfade: 100,
        mode: LoopMode::Forward,
    });
    r.mode = PlayMode::Loop;
    r.fade_out = Fade {
        frames: 441,
        curve: FadeCurve::Linear,
    };
    let mut p = project(&s, pad);
    // 100 BPM to 120: 5/6 as long, and a fifth up while at it.
    let mut job_of = Process::to_tempo(0, 0, 100.0, 120.0);
    job_of.semitones = 7.0;
    let mut job = job_of.job(&p).unwrap();
    while !job.run(4096).finished {}
    let id = job_of.finish(&mut p, job.into_output().unwrap()).unwrap();
    let new = p.source(id).unwrap();
    assert_eq!(new.frames(), 33_075);
    assert!(
        new.name.contains("x0.83") && new.name.contains("+7st"),
        "{}",
        new.name
    );
    let r = &p.pad(0).unwrap().layers[0].region;
    assert_eq!((r.source, r.start, r.end), (id, 0, 33_075));
    let l = r.sustain_loop.unwrap();
    assert_eq!((l.start, l.end, l.crossfade), (3675, 33_075, 83));
    assert_eq!(r.fade_out.frames, 368);
    assert!((r.warp.unwrap().bpm - 120.0).abs() < 0.01);
    // At 120 BPM the processed region needs no stretching at all.
    let k = Warps::new(120.0).key_for(r, 0).unwrap();
    assert_eq!(k.frames, 33_075);
    // One undo: the old region and no new source.
    assert!(p.undo());
    assert_eq!(p.pad(0).unwrap().layers[0].region.source, s.id());
    assert!(p.source(id).is_none());
    assert!(p.redo());
    assert_eq!(p.pad(0).unwrap().layers[0].region.source, id);
}

#[test]
fn the_worker_makes_copies_in_turn_and_cancel_drops_the_rest() {
    use mpfree_sampler::warp::{Done, Worker};
    use std::time::Duration;
    let s = sine(1.0, 220.0);
    let p = project(&s, warped_pad(&s, 100.0, WarpMethod::Stretch));
    let mut warps = Warps::new(120.0);
    let w = Worker::spawn().unwrap();
    let k = warps.missing(&p)[0];
    assert!(w.send(k, Warps::spec(&p, &k).unwrap()));
    match w.recv_timeout(Duration::from_secs(20)) {
        Some(Done::Copy(key, data, _)) => {
            assert_eq!(key, k);
            assert_eq!(data.len() as u64, k.frames * 2);
            warps.insert(key, data, &p);
        }
        other => panic!("{other:?}"),
    }
    assert!(warps.missing(&p).is_empty());
    // A long job cancelled while queued or running comes back cancelled;
    // one sent after the cancel runs.
    let long = sine(30.0, 220.0);
    let q = project(&long, warped_pad(&long, 100.0, WarpMethod::Stretch));
    let a = warps.missing(&q)[0];
    assert!(w.send(a, Warps::spec(&q, &a).unwrap()));
    w.cancel();
    warps.set_tempo(90.0);
    let b = warps.missing(&p)[0];
    assert!(w.send(b, Warps::spec(&p, &b).unwrap()));
    assert!(matches!(w.recv_timeout(Duration::from_secs(20)), Some(Done::Cancelled(k)) if k == a));
    assert!(matches!(w.recv_timeout(Duration::from_secs(20)), Some(Done::Copy(k, ..)) if k == b));
    // Dropping it with a long job running returns soon.
    assert!(w.send(a, Warps::spec(&q, &a).unwrap()));
    let t = std::time::Instant::now();
    drop(w);
    assert!(t.elapsed() < Duration::from_secs(10));
}
