//! Warp pins: hits pinned to beats inside a stretched region.

use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_sampler::warp::{CopyKey, CopyMap, Process, Warps};

const RATE: u32 = 44_100;
/// Frames per beat at 120 BPM.
const BEAT: u64 = 22_050;

/// Stereo silence `frames` long with a short 1 kHz burst at each of `at`.
fn clicks(frames: usize, at: &[usize]) -> Source {
    let mut v = vec![0.0f32; frames * 2];
    for &a in at {
        for i in 0..400.min(frames - a) {
            let t = i as f64 / f64::from(RATE);
            let x = (0.8 * (-t * 300.0).exp() * (std::f64::consts::TAU * 1000.0 * t).sin()) as f32;
            v[(a + i) * 2] = x;
            v[(a + i) * 2 + 1] = x;
        }
    }
    Source::new("clicks", RATE, 2, v).unwrap()
}

/// Where each burst starts: the first loud frame after a quiet stretch.
fn onsets(x: &[f32]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut quiet = 2000usize;
    for (i, v) in x.iter().enumerate() {
        if v.abs() > 0.1 {
            if quiet >= 2000 {
                out.push(i);
            }
            quiet = 0;
        } else {
            quiet += 1;
        }
    }
    out
}

fn warp(bpm: f32, pins: &[(u64, f64)]) -> Warp {
    Warp {
        pins: pins
            .iter()
            .map(|&(frame, beat)| Pin { frame, beat })
            .collect(),
        ..Warp::new(bpm)
    }
}

fn project(s: &Source, w: Warp) -> Project {
    let mut r = Region::whole(s);
    r.warp = Some(w);
    let mut p = Project::new();
    p.add_source(s.clone());
    p.set_pad(0, Some(Pad::new("loop", Layer::new(r))), None)
        .unwrap();
    p
}

fn make_copies(p: &Project, warps: &mut Warps) -> usize {
    let missing = warps.missing(p);
    for k in &missing {
        let mut job = Warps::job(p, k).unwrap();
        while !job.run(4096).finished {}
        warps.insert(*k, job.into_output().unwrap(), p);
    }
    missing.len()
}

/// Hits pad 0 and renders until it ends: the left channel.
fn play(p: &Project, warps: &mut Warps) -> Vec<f32> {
    let mut e = Engine::new(RATE, 8);
    e.set_tempo(warps.tempo());
    e.set_view(EngineView::build_warped(p, RATE, warps));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let mut out = Vec::new();
    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
    while e.active_voices() > 0 && out.len() < 20 * RATE as usize {
        e.render(&mut l, &mut r);
        out.extend_from_slice(&l);
    }
    out
}

fn assert_near(got: &[usize], want: &[u64], tol: u64) {
    assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(
            (*g as i64 - *w as i64).unsigned_abs() <= tol,
            "{got:?} vs {want:?}"
        );
    }
}

/// Four beats at 120 BPM played loosely: the second hit 3000 frames late,
/// the third 2000 early, the fourth on time.
fn loose() -> (Source, [usize; 4]) {
    let at = [
        0,
        BEAT as usize + 3000,
        2 * BEAT as usize - 2000,
        3 * BEAT as usize,
    ];
    (clicks(4 * BEAT as usize, &at), at)
}

/// Each of the loose hits after the first pinned to its beat.
fn pinned(at: &[usize; 4]) -> Warp {
    warp(
        120.0,
        &[
            (at[1] as u64, 1.0),
            (at[2] as u64, 2.0),
            (at[3] as u64, 3.0),
        ],
    )
}

#[test]
fn without_pins_a_region_plays_evenly() {
    let w = Warp::new(120.0);
    assert_eq!(w.beats(4 * BEAT, RATE), 4.0);
    assert_eq!(
        w.beat_at(1000, 1000 + 4 * BEAT, RATE, 1000.0 + BEAT as f64),
        1.0
    );
    assert_eq!(
        w.frame_at(1000, 1000 + 4 * BEAT, RATE, 2.5),
        1000.0 + 2.5 * BEAT as f64
    );
    // Past the end, on at the region's tempo.
    assert_eq!(w.beat_at(0, 4 * BEAT, RATE, 5.0 * BEAT as f64), 5.0);
    assert!(w.pins_ok(0, 4 * BEAT, RATE));
}

#[test]
fn pins_bend_the_line_between_frames_and_beats() {
    let (_, at) = loose();
    let w = pinned(&at);
    let end = 4 * BEAT;
    assert!(w.pins_ok(0, end, RATE));
    for (i, &a) in at.iter().enumerate() {
        assert!((w.beat_at(0, end, RATE, a as f64) - i as f64).abs() < 1e-9);
        assert!((w.frame_at(0, end, RATE, i as f64) - a as f64).abs() < 1e-6);
    }
    // Half way between two pins is half way between their beats.
    let mid = (at[1] + at[2]) as f64 / 2.0;
    assert!((w.beat_at(0, end, RATE, mid) - 1.5).abs() < 1e-9);
    // Points: the start, the pins, the end.
    let pts = w.points(0, end, RATE);
    assert_eq!(pts.len(), 5);
    assert_eq!(pts[0], (0.0, 0.0));
    assert_eq!(pts[4], (end as f64, 4.0));
    // A hit between pins moves with the line: with the third hit's pin
    // gone, the fourth is pushed along by the stretch from the second.
    let w = warp(120.0, &[(at[1] as u64, 1.0), (at[2] as u64, 2.0)]);
    let b = w.beat_at(0, end, RATE, at[3] as f64);
    assert!((b - (2.0 + 2.0 * (at[3] - at[2]) as f64 / (end as f64 - at[2] as f64))).abs() < 1e-9);
}

#[test]
fn pins_out_of_order_outside_or_too_far_are_refused() {
    let end = 4 * BEAT;
    let bad = [
        // Beats out of order.
        warp(120.0, &[(BEAT, 2.0), (2 * BEAT, 1.0)]),
        // On the start, or past the end.
        warp(120.0, &[(0, 0.5)]),
        warp(120.0, &[(end, 3.5)]),
        // Past the region's last beat.
        warp(120.0, &[(2 * BEAT, 4.5)]),
        // A beat's frames squeezed into a tenth of a beat: ten times as fast.
        warp(120.0, &[(BEAT, 0.1)]),
        warp(120.0, &[(BEAT, f64::NAN)]),
    ];
    let s = clicks(end as usize, &[0]);
    for w in bad {
        assert!(!w.pins_ok(0, end, RATE), "{:?}", w.pins);
        let mut r = Region::whole(&s);
        r.warp = Some(w);
        assert!(matches!(
            r.check(&s),
            Err(ModelError::BadRegion("warp pins"))
        ));
    }
    // Four times as fast is still fine.
    assert!(warp(120.0, &[(BEAT, 0.25)]).pins_ok(0, end, RATE));
}

#[test]
fn a_moved_region_keeps_its_pins_on_their_hits() {
    let (_, at) = loose();
    let w = warp(120.0, &[(at[1] as u64, 1.0), (at[2] as u64, 2.0)]);
    let old = (0, 4 * BEAT);
    // The start moved to the second hit: it is beat 0 now, its pin goes
    // (it would sit on the start) and the third hit is a beat on.
    let n = w.moved(old, (at[1] as u64, 4 * BEAT), RATE);
    assert_eq!(n.pins.len(), 1);
    assert_eq!(n.pins[0].frame, at[2] as u64);
    assert!((n.pins[0].beat - 1.0).abs() < 1e-9);
    // The end moved in before the third hit: its pin goes.
    let n = w.moved(old, (0, at[2] as u64 - 10), RATE);
    assert_eq!(n.pins.len(), 1);
    assert_eq!(n.pins[0].frame, at[1] as u64);
    // Unmoved, nothing changes.
    assert_eq!(w.moved(old, old, RATE), w);
}

#[test]
fn fit_drops_the_pins_a_new_tempo_leaves_out() {
    let (_, at) = loose();
    let mut w = warp(120.0, &[(at[1] as u64, 1.0), (at[2] as u64, 2.0)]);
    // At 60 BPM the region lasts two beats: the pin on beat 2 is past it,
    // and the one on beat 1 would play the rest of it far too fast.
    w.bpm = 60.0;
    w.fit(0, 4 * BEAT, RATE);
    assert!(w.pins_ok(0, 4 * BEAT, RATE));
    assert!(w.pins.len() < 2);
}

#[test]
fn a_copy_key_tells_pin_sets_apart_only_when_stretching() {
    let (s, at) = loose();
    let r = Region::whole(&s);
    let plain = Warp::new(120.0);
    let pinned = warp(120.0, &[(at[1] as u64, 1.0)]);
    let a = CopyKey::of(&r, &plain, 0, 100.0);
    let b = CopyKey::of(&r, &pinned, 0, 100.0);
    assert_eq!(a.pins, 0);
    assert_ne!(b.pins, 0);
    assert_ne!(a, b);
    assert_eq!(a.frames, b.frames);
    // Repitching, pins change nothing.
    let rep = Warp {
        method: WarpMethod::Repitch,
        ..pinned.clone()
    };
    assert_eq!(CopyKey::of(&r, &rep, 0, 100.0).pins, 0);
    // The copy map runs through the pins, scaled to the copy's length.
    let m = CopyMap::new(&pinned, RATE, &b);
    let k = b.frames as f64 / (4 * BEAT) as f64;
    assert!((m.at(at[1] as f64) - BEAT as f64 * k).abs() < 1e-6);
    assert!((m.source(BEAT as f64 * k) - at[1] as f64).abs() < 1e-6);
    assert!((m.at((4 * BEAT) as f64) - b.frames as f64).abs() < 1e-6);
}

#[test]
fn a_stretched_loop_plays_its_pinned_hits_on_the_beat() {
    let (s, at) = loose();
    let w = pinned(&at);
    let p = project(&s, w);
    // At its own tempo and at half of it: the hits on every beat.
    for (tempo, beat) in [(120.0, BEAT), (60.0, 2 * BEAT)] {
        let mut warps = Warps::new(tempo);
        assert_eq!(make_copies(&p, &mut warps), 1);
        let out = play(&p, &mut warps);
        assert_near(&onsets(&out), &[0, beat, 2 * beat, 3 * beat], 160);
    }
    // Without the pins the same loop plays loose.
    let p = project(&s, Warp::new(120.0));
    let mut warps = Warps::new(60.0);
    make_copies(&p, &mut warps);
    let late = 2 * (BEAT + 3000);
    let early = 2 * (2 * BEAT - 2000);
    assert_near(
        &onsets(&play(&p, &mut warps)),
        &[0, late, early, 6 * BEAT],
        160,
    );
}

#[test]
fn processing_a_pinned_region_makes_its_hits_land_on_the_beat() {
    let (s, at) = loose();
    let w = pinned(&at);
    let mut p = project(&s, w);
    let proc = Process::to_tempo(0, 0, 120.0, 60.0);
    let mut job = proc.job(&p).unwrap();
    while !job.run(4096).finished {}
    proc.finish(&mut p, job.into_output().unwrap()).unwrap();
    let r = &p.pad(0).unwrap().layers[0].region;
    let w = r.warp.as_ref().unwrap();
    assert!(w.pins.is_empty(), "the pins are in the audio now");
    assert!((w.bpm - 60.0).abs() < 0.01);
    let new = p.source(r.source).unwrap();
    let left: Vec<f32> = new.data().iter().step_by(2).copied().collect();
    assert_near(&onsets(&left), &[0, 2 * BEAT, 4 * BEAT, 6 * BEAT], 160);
}

#[test]
fn a_pin_moves_only_as_far_as_its_neighbours_allow() {
    let (_, at) = loose();
    let end = 4 * BEAT;
    let w = pinned(&at);
    let (lo, hi) = w.pin_room(1, 0, end, RATE).unwrap();
    assert!(lo > 1.0 && hi < 3.0, "between beats 1 and 3: {lo} {hi}");
    let m = w.pin_moved(1, 2.25, 0, end, RATE);
    assert_eq!(m.pins[1].beat, 2.25);
    assert!(m.pins_ok(0, end, RATE));
    // Asked past its neighbour, it stops short of it.
    let m = w.pin_moved(1, 9.0, 0, end, RATE);
    assert!(m.pins[1].beat < 3.0 && m.pins_ok(0, end, RATE));
    assert!(w.pin_room(5, 0, end, RATE).is_none());
}

#[test]
fn a_pin_is_added_in_its_place_or_not_at_all() {
    let (_, at) = loose();
    let end = 4 * BEAT;
    let w = warp(120.0, &[(at[1] as u64, 1.0), (at[3] as u64, 3.0)]);
    let (n, i) = w.pin_added(at[2] as u64, 2.0, 0, end, RATE).unwrap();
    assert_eq!(i, 1);
    assert_eq!(n, pinned(&at));
    // Asked for a beat past the next pin: as near as there is room for.
    let (n, _) = w.pin_added(at[2] as u64, 3.5, 0, end, RATE).unwrap();
    assert!(n.pins[1].beat < 3.0 && n.pins_ok(0, end, RATE));
    // Already pinned, or outside the region.
    assert!(w.pin_added(at[1] as u64, 1.0, 0, end, RATE).is_none());
    assert!(w.pin_added(end + 5, 3.5, 0, end, RATE).is_none());
}

#[test]
fn hits_near_the_grid_are_pinned_onto_it() {
    let (_, at) = loose();
    let end = 4 * BEAT;
    let hits: Vec<u64> = at.iter().map(|&a| a as u64).collect();
    // A beat grid reaching a third of a beat: every hit after the first
    // goes on its beat (the first is on the start already).
    let w = Warp::new(120.0).pinned_to_grid(&hits, 1.0, 1.0 / 3.0, 0, end, RATE);
    assert_eq!(w, pinned(&at));
    // Sixteenths reaching half of one: the late hit is nearer the
    // sixteenth after its beat, so it goes there; the early one is still
    // nearest its beat.
    let w = Warp::new(120.0).pinned_to_grid(&hits, 0.25, 0.125, 0, end, RATE);
    let beats: Vec<f64> = w.pins.iter().map(|p| p.beat).collect();
    assert_eq!(beats, [1.25, 2.0, 3.0]);
    // Two hits near one line: the nearer one is pinned.
    let w = Warp::new(120.0).pinned_to_grid(&[BEAT - 900, BEAT + 300], 1.0, 0.25, 0, end, RATE);
    assert_eq!(
        w.pins,
        [Pin {
            frame: BEAT + 300,
            beat: 1.0
        }]
    );
}

#[test]
fn a_stretched_voice_reports_its_place_in_the_source() {
    let (s, at) = loose();
    let p = project(&s, pinned(&at));
    let mut warps = Warps::new(60.0);
    make_copies(&p, &mut warps);
    let mut e = Engine::new(RATE, 8);
    e.set_tempo(warps.tempo());
    e.set_view(EngineView::build_warped(&p, RATE, &mut warps));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    // Two beats at 60 BPM in: the copy is on beat 2, made from the third
    // hit (pinned there), not from half way through the source.
    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
    let mut done = 0;
    while done < 2 * 2 * BEAT as usize {
        e.render(&mut l, &mut r);
        done += 128;
    }
    let heads: Vec<_> = e.playheads().collect();
    assert_eq!(heads.len(), 1);
    let (pad, layer, frame) = heads[0];
    assert_eq!((pad, layer), (0, 0));
    assert!(
        (frame as i64 - at[2] as i64).abs() < 200,
        "{frame} vs {}",
        at[2]
    );
}
