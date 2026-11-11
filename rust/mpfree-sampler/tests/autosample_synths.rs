//! Auto-sampling MPFree's own synths offline: the keygroups play at the
//! synth's pitch, every engine makes an instrument, and a job on its
//! worker thread can be cancelled.

use mpfree_analysis::yin;
use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_sampler::autosample::*;
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_synth::{Drums, Fm, Pluck, Poly, Wavetable};

const RATE: u32 = 44_100;

fn pitch(x: &[f32]) -> f32 {
    yin(&x[4000..12192], RATE, 4096, 40.0, 3000.0, 0.15)
        .expect("pitched")
        .0
}

/// The synth itself playing `note` for `frames` frames (left channel).
fn synth_note(node: &mut dyn Node, note: u8, frames: usize) -> Vec<f32> {
    node.prepare(RATE, 1024);
    node.reset();
    let mut out = Vec::new();
    let mut f = 0;
    while out.len() < frames {
        let ev = [Timed::new(
            0,
            NodeEvent::NoteOn {
                note,
                velocity: 100,
            },
        )];
        let (mut l, mut r) = (vec![0.0; 1024], vec![0.0; 1024]);
        node.process(
            &Ctx::new(RATE, f),
            if f == 0 { &ev } else { &[] },
            &mut l,
            &mut r,
        );
        out.extend_from_slice(&l);
        f += 1024;
    }
    out
}

/// The sampler playing `note` from `project`'s keygroups (left channel).
fn sampled_note(project: &Project, note: u8, frames: usize) -> Vec<f32> {
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(project, RATE));
    e.apply(Event::Key {
        note,
        velocity: 100,
    });
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    l
}

fn small(name: &str) -> Plan {
    Plan {
        name: name.into(),
        lo: 48,
        hi: 72,
        every: 6,
        velocities: vec![100],
        hold: 0.5,
        tail: 1.5,
        gap: 0.05,
        ..Plan::default()
    }
}

#[test]
fn a_sampled_poly_plays_every_note_at_the_synths_own_pitch() {
    let mut poly = Poly::new(RATE);
    let mut s = Session::new(small("Poly Init"), 1).unwrap();
    render(&mut s, &mut poly, |_| true);
    assert_eq!(s.state(), State::Done);
    let rec = s.finish();
    assert!(!rec.clipped);
    let inst = build(&rec).unwrap();
    assert_eq!(inst.pads.len(), 5);
    let mut project = Project::new();
    inst.put(&mut project, 0).unwrap();
    // Sampled notes (54, 66), and notes between them, transposed.
    for note in [54u8, 57, 62, 66, 71] {
        let want = pitch(&synth_note(&mut Poly::new(RATE), note, 16_384));
        let got = pitch(&sampled_note(&project, note, 16_384));
        assert!(
            (got - want).abs() < want * 0.005,
            "note {note}: sampled {got} Hz, the synth {want} Hz"
        );
    }
}

#[test]
fn every_engine_makes_an_instrument() {
    let engines: Vec<(&str, Box<dyn Node>)> = vec![
        ("Poly", Box::new(Poly::new(RATE))),
        ("FM", Box::new(Fm::new(RATE))),
        ("Pluck", Box::new(Pluck::new(RATE))),
        ("Wavetable", Box::new(Wavetable::new(RATE))),
        ("Drums", Box::new(Drums::new(RATE))),
    ];
    for (name, mut node) in engines {
        let plan = Plan {
            lo: 36,
            hi: 48,
            every: 12,
            hold: 0.3,
            tail: 1.0,
            ..small(name)
        };
        let mut s = Session::new(plan, 1).unwrap();
        render(&mut s, node.as_mut(), |_| true);
        assert_eq!(s.state(), State::Done, "{name}");
        let inst = build(&s.finish()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(inst.pads.len(), 2, "{name}");
        let mut project = Project::new();
        inst.put(&mut project, 0).unwrap();
        let out = sampled_note(&project, 40, 8192);
        assert!(out.iter().any(|x| x.abs() > 0.01), "{name} plays");
    }
}

#[test]
fn a_job_runs_on_its_own_thread_and_can_be_cancelled() {
    let plan = small("Poly Init");
    let job = Job::start(plan.clone(), Box::new(Poly::new(RATE))).unwrap();
    let (_, total) = job.progress();
    assert_eq!(total, 5);
    let done = job.finish();
    assert_eq!(done.state, State::Done);
    assert_eq!(done.instrument.unwrap().pads.len(), 5);
    assert!(
        done.audio_seconds > 5.0 * 0.5
            && done.audio_seconds < plan.estimate().seconds_most as f64 + 1.0
    );
    assert!(done.render_seconds > 0.0);

    // Cancelled at once: nothing recorded, nothing built.
    let job = Job::start(plan.clone(), Box::new(Poly::new(RATE))).unwrap();
    job.cancel();
    let done = job.finish();
    assert_eq!(done.state, State::Cancelled);
    assert_eq!(done.instrument.unwrap_err(), BuildError::Empty);

    // A refused plan says why before any thread starts.
    let bad = Plan {
        round_robins: 9,
        ..plan
    };
    assert_eq!(
        Job::start(bad, Box::new(Poly::new(RATE))).err(),
        Some(PlanError::RoundRobins)
    );
}

#[test]
fn each_note_starts_fresh_whatever_still_rings_from_the_note_before() {
    // A long release and a short tail: the note before is still ringing
    // when the next starts. The node is reset before every new note, so
    // each take sounds as the synth does playing that note alone.
    let patch = || {
        let mut p = Poly::new(RATE);
        p.set_param(mpfree_synth::params::AMP_R, 1_500.0);
        p
    };
    let plan = Plan {
        tail: 0.2,
        ..small("Long release")
    };
    let mut s = Session::new(plan, 1).unwrap();
    render(&mut s, &mut patch(), |_| true);
    let rec = s.finish();
    assert_eq!(rec.takes.len(), 5);
    let hz = |n: u8| 440.0 * ((f32::from(n) - 69.0) / 12.0).exp2();
    for t in rec.takes.iter().skip(1) {
        let take: Vec<f32> = t.audio.chunks(2).map(|f| f[0]).take(8192).collect();
        // The note before is a tritone down: none of this note's harmonics.
        let this = level_at(&take, hz(t.take.note));
        let before = level_at(&take, hz(t.take.note - 6));
        assert!(
            before < this * 0.01,
            "note {}: the note before at {:.1} dB",
            t.take.note,
            20.0 * (before / this).log10()
        );
    }
}

/// The level of `hz` in `x` (Goertzel, Hann window).
fn level_at(x: &[f32], hz: f32) -> f32 {
    let n = x.len();
    let w = std::f32::consts::TAU * hz / RATE as f32;
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for (i, v) in x.iter().enumerate() {
        let hann = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
        let s0 = v * hann + 2.0 * w.cos() * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).sqrt()
}
