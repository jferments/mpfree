//! Step S3 end to end: analyse a source, chop it each way, put the slices on
//! pads and play them back through the engine.

use mpfree_analysis::SilenceSettings;
use mpfree_sampler::chop::{demo_beat, SourceAnalysis};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
const C: f32 = std::f32::consts::FRAC_1_SQRT_2; // centre pan gain

fn beat(bpm: f64, bars: usize, tail: usize) -> (Vec<f32>, Vec<usize>) { demo_beat(bpm, bars, tail, RATE) }

/// Schedules a note-on for every pad at its frame, renders `frames`, returns the left channel.
fn play(p: &Project, hits: &[(u8, u32)], frames: usize) -> Vec<f32> {
    let mut e = Engine::new(RATE, 32);
    e.set_view(EngineView::build(p, RATE));
    for &(pad, at) in hits { assert!(e.schedule(at, Event::NoteOn { pad, velocity: 127 })); }
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) { e.render(a, b); }
    l
}

#[test]
fn transient_chop_to_pads_plays_the_loop_back_sample_for_sample() {
    let (audio, hits) = beat(100.0, 2, 0);
    let s = Source::new("break", RATE, 1, audio.clone()).unwrap();
    let a = SourceAnalysis::analyse(&s);
    let c = a.transient("break", &s, 0.6).unwrap();
    // One slice per hit (the first starts at 0).
    assert_eq!(c.slices(), hits.len(), "{:?}", c.markers());
    for (m, h) in c.markers().iter().zip(&hits[1..]) { assert!((*h as i64 - *m as i64).abs() <= 64, "marker {m} for hit {h}"); }

    let mut p = Project::new();
    p.add_source(s.clone());
    let chop = p.add_chop_set(c.clone()).unwrap();
    p.slices_to_pads(chop, 0, c.slices(), 0).unwrap();
    // Hitting the pads in order at the slice starts rebuilds the loop exactly.
    let events: Vec<(u8, u32)> = (0..c.slices()).map(|i| (i as u8, c.bounds(i).unwrap().0 as u32)).collect();
    let out = play(&p, &events, audio.len());
    let worst = out.iter().zip(&audio).map(|(o, x)| (o - C * x).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1e-6, "largest difference {worst}");
}

#[test]
fn sensitivity_changes_the_slice_count_without_reanalysing() {
    let (audio, hits) = beat(120.0, 2, 0);
    let s = Source::new("b", RATE, 1, audio).unwrap();
    let a = SourceAnalysis::analyse(&s);
    let few = a.transient("few", &s, 0.0).unwrap().slices();
    let all = a.transient("all", &s, 0.6).unwrap().slices();
    assert!(few < all && all == hits.len(), "{few} {all}");
}

#[test]
fn beat_chop_uses_the_exact_loop_tempo() {
    // Two bars at 96 BPM cut to length: the loop tempo is exact.
    let (audio, _) = beat(96.0, 2, 0);
    let s = Source::new("loop", RATE, 1, audio).unwrap();
    let a = SourceAnalysis::analyse(&s);
    let est = a.tempo().unwrap();
    assert!((est.bpm - 96.0).abs() < 0.5, "estimate {}", est.bpm);
    let exact = a.loop_tempo().unwrap();
    assert!((exact - 96.0).abs() < 0.01, "loop tempo {exact}");
    let c = a.beats("beats", &s, None, 1.0).unwrap();
    assert_eq!(c.slices(), 8);
    let beat = 60.0 / 96.0 * f64::from(RATE);
    for (k, m) in c.markers().iter().enumerate() {
        assert!((*m as f64 - beat * (k + 1) as f64).abs() <= 1.0, "marker {k} at {m}");
    }
    // Half-beat slices at a typed tempo.
    assert_eq!(a.beats("eighths", &s, Some(96.0), 0.5).unwrap().slices(), 16);
    assert!(a.beats("bad", &s, Some(0.0), 1.0).is_err());
}

#[test]
fn silence_chop_and_trim() {
    let mut x = vec![0.0f32; RATE as usize * 2];
    for (at, len) in [(5_000usize, 6_000usize), (30_000, 9_000), (60_000, 3_000)] {
        for i in 0..len { x[at + i] = 0.5 * (i as f32 * 0.2).sin() * (1.0 - i as f32 / len as f32); }
    }
    let s = Source::new("chain", RATE, 1, x).unwrap();
    let a = SourceAnalysis::analyse(&s);
    let c = a.silence("chain", &s, SilenceSettings::default()).unwrap();
    // Three sounds: markers before each (the first slice is the lead-in).
    assert_eq!(c.markers().len(), 3, "{:?}", c.markers());
    for (m, at) in c.markers().iter().zip([5_000u64, 30_000, 60_000]) { assert!(*m <= at && at - m < 100); }
    let (t0, t1) = a.trim(SilenceSettings::default()).unwrap();
    assert!(t0 <= 5_000 && t0 > 4_900 && t1 > 62_000 && t1 <= 63_000, "{t0} {t1}");
}

#[test]
fn snap_moves_a_marker_to_a_zero_crossing() {
    let x: Vec<f32> = (0..4_000).map(|i| (std::f32::consts::TAU * (i as f32 + 0.25) / 200.0).sin()).collect();
    let s = Source::new("sine", RATE, 1, x).unwrap();
    let a = SourceAnalysis::analyse(&s);
    assert_eq!(a.snap(1_090), 1_100);
    assert_eq!(a.snap(1_000), 1_000);
}

#[test]
fn chopping_and_assigning_undo_in_one_step_each() {
    let (audio, _) = beat(110.0, 1, 0);
    let s = Source::new("b", RATE, 1, audio).unwrap();
    let mut p = Project::new();
    p.add_source(s.clone());
    p.set_pad(2, Some(Pad::new("keep", Layer::new(Region::whole(&s)))), None).unwrap();
    let c = ChopSet::grid("g", &s, 8).unwrap();
    let chop = p.add_chop_set(c).unwrap();
    p.slices_to_pads(chop, 0, 8, 0).unwrap();
    assert_eq!(p.pad(2).unwrap().name, "g 3");
    assert!(p.undo(), "one undo takes all eight pads back");
    assert_eq!(p.pad(2).unwrap().name, "keep");
    assert!(p.pad(0).is_none() && p.pad(7).is_none());
    assert_eq!(p.chop_sets.len(), 1);
    assert!(p.undo(), "the next undo removes the chop set");
    assert!(p.chop_sets.is_empty());
    assert!(p.redo() && p.redo());
    assert_eq!(p.chop_sets.len(), 1);
    assert_eq!(p.pad(7).unwrap().name, "g 8");
    // A bad request changes nothing.
    let before = p.revision();
    assert!(p.slices_to_pads(0, 4, 8, 0).is_err());
    assert!(p.slices_to_pads(0, 0, 8, 125).is_err());
    assert_eq!(p.revision(), before);
}

#[test]
fn markers_from_any_list_are_cleaned() {
    let s = Source::new("s", RATE, 1, vec![0.1; 1_000]).unwrap();
    let c = ChopSet::from_markers("m", &s, ChopMode::Manual, [500, 0, 200, 500, 1_000, 5_000, 800]).unwrap();
    assert_eq!(c.markers(), &[200, 500, 800]);
    assert!(ChopSet::from_markers("m", &s, ChopMode::Manual, 1..200).is_err(), "too many slices");
    let other = Source::new("o", RATE, 1, vec![0.2; 1_000]).unwrap();
    let a = SourceAnalysis::analyse(&other);
    assert!(a.transient("x", &s, 0.5).is_err(), "analysis of another source");
    assert!(Project::new().add_chop_set(c).is_err(), "source not in the project");
}

#[test]
fn the_chop_tool_runs_its_demo_and_passes_the_rebuild_check() {
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("mpfree-chop-demo");
    let _ = std::fs::remove_dir_all(&out);
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_mpfree-chop"))
        .arg("--demo").arg(&out).args(["--mode", "beat", "--beats", "0.5"])
        .output().unwrap();
    let text = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "{text}\n{}", String::from_utf8_lossy(&run.stderr));
    assert!(text.contains("loop_tempo=96.000") && text.contains("slices=16") && text.contains("rebuild=PASS"), "{text}");
    for f in ["chop-report.txt", "demo-input.wav", "rebuild.wav", "one-by-one.wav", "reversed.wav", "slices/slice-16.wav"] {
        assert!(out.join(f).is_file(), "{f} missing");
    }
    // Every slice file decodes and has the slice's length.
    let s = mpfree_sampler::PreparedSample::read_wav(&out.join("slices/slice-01.wav")).unwrap();
    assert_eq!(s.frames(), (60.0 / 96.0 / 2.0 * f64::from(RATE)).round() as usize);
    let bad = std::process::Command::new(env!("CARGO_BIN_EXE_mpfree-chop")).arg("--demo").output().unwrap();
    assert!(!bad.status.success(), "usage error without an output folder");
}
