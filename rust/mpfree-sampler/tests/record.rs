//! Step S4, workstation half: the rolling buffers and takes, fed period by
//! period the way the audio thread will feed them on the MPC.

use mpfree_sampler::chop::{demo_beat, SourceAnalysis};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_sampler::record::{rolling, Arm, RecordError, Stop, Take, TakeState};

const RATE: u32 = 44_100;
const PERIOD: usize = 128;

/// A recognisable stereo signal: left = frame number, right = minus it.
fn frame_audio(from: u64, frames: usize) -> Vec<f32> {
    (0..frames)
        .flat_map(|i| {
            let v = (from + i as u64) as f32;
            [v, -v]
        })
        .collect()
}

#[test]
fn skip_back_and_exact_spans() {
    let (mut w, r) = rolling(2, 1_000);
    for p in 0..20u64 {
        w.push(&frame_audio(p * PERIOD as u64, PERIOD));
    }
    assert_eq!(r.written(), 2_560);
    assert_eq!(r.oldest(), 1_560);
    // The last 300 frames.
    let last = r.skip_back(300).unwrap();
    assert_eq!(last.len(), 600);
    assert_eq!((last[0], last[1], last[598]), (2_260.0, -2_260.0, 2_559.0));
    // More than the buffer holds gives what it holds.
    assert_eq!(r.skip_back(5_000).unwrap().len(), 2_000);
    // An exact span, such as "the last bar" from the transport.
    let mut out = Vec::new();
    r.copy(2_000, 2_010, &mut out).unwrap();
    assert_eq!(
        out.iter().step_by(2).copied().collect::<Vec<_>>(),
        (2_000..2_010).map(|f| f as f32).collect::<Vec<_>>()
    );
    // Too old or not yet there.
    assert_eq!(
        r.copy(1_000, 1_100, &mut out),
        Err(RecordError::Overwritten)
    );
    assert_eq!(
        r.copy(2_500, 2_600, &mut out),
        Err(RecordError::NotYetWritten)
    );
    assert_eq!(r.copy(10, 5, &mut out), Err(RecordError::BadSpan));
    assert_eq!(out.len(), 20, "failed copies leave the output alone");
}

#[test]
fn mono_buffers_average_the_engine_output() {
    let (mut w, r) = rolling(1, 64);
    w.push_planar(&[1.0, 0.5], &[0.0, 0.5]);
    assert_eq!(r.skip_back(2).unwrap(), vec![0.5, 0.5]);
}

#[test]
fn a_reader_racing_the_writer_never_gets_torn_audio() {
    // The writer laps a small buffer as fast as it can on another thread;
    // every copy either fails or holds exactly consecutive frames.
    let (mut w, r) = rolling(2, 512);
    let writer = std::thread::spawn(move || {
        for p in 0..20_000u64 {
            w.push(&frame_audio(p * 64, 64));
        }
    });
    let (mut ok, mut lost) = (0, 0);
    let mut out = Vec::new();
    while !writer.is_finished() {
        let end = r.written();
        let start = end.saturating_sub(400);
        out.clear();
        match r.copy(start, end, &mut out) {
            Ok(()) => {
                ok += 1;
                for (i, f) in out.chunks_exact(2).enumerate() {
                    assert_eq!(f[0], (start + i as u64) as f32, "torn copy");
                    assert_eq!(f[1], -f[0]);
                }
            }
            Err(RecordError::Overwritten) => lost += 1,
            Err(e) => panic!("{e:?}"),
        }
    }
    writer.join().unwrap();
    assert!(ok > 0, "some copies succeed ({lost} were overwritten)");
}

/// Runs a take against a signal fed in periods, polling every few periods.
fn record(signal: &[f32], channels: u8, arm: Arm, stop: Stop, stop_button: Option<u64>) -> Take {
    let ch = usize::from(channels);
    let (mut w, r) = rolling(channels, RATE as usize * 2);
    let mut t = Take::new(&r, arm, stop, RATE as u64 * 60);
    for (k, period) in signal.chunks(PERIOD * ch).enumerate() {
        w.push(period);
        if let Some(f) = stop_button {
            if w.written() >= f {
                t.stop_at(f);
            }
        }
        if k % 5 == 4 {
            t.poll(&r).unwrap();
        }
    }
    t.poll(&r).unwrap();
    t
}

#[test]
fn manual_take_is_exact() {
    let signal = frame_audio(0, 20_000);
    let t = record(&signal, 2, Arm::At(1_234), Stop::Manual, Some(9_000));
    assert_eq!(
        t.state(),
        TakeState::Done {
            start: 1_234,
            end: 9_000
        }
    );
    assert_eq!(t.data().len(), (9_000 - 1_234) * 2);
    assert_eq!(
        (t.data()[0], t.data()[t.data().len() - 2]),
        (1_234.0, 8_999.0)
    );
}

#[test]
fn in_time_take_is_exactly_the_bars() {
    // Two bars at 120 BPM starting on the next bar line, at frame 88 200.
    let bar = 4 * RATE as u64 / 2;
    let signal = frame_audio(0, 6 * bar as usize);
    let t = record(&signal, 2, Arm::At(bar), Stop::Length(2 * bar), None);
    assert_eq!(
        t.state(),
        TakeState::Done {
            start: bar,
            end: 3 * bar
        }
    );
    assert_eq!(t.data().len() as u64, 2 * bar * 2);
    assert_eq!(t.data()[0], bar as f32);
}

#[test]
fn threshold_take_keeps_its_pre_roll_and_stops_on_silence() {
    // Quiet hiss, a sound at 30 000 for 10 000 frames, then hiss again.
    let mut signal: Vec<f32> = (0..80_000)
        .map(|i| if (i * 7919) % 13 == 0 { 0.001 } else { -0.0005 })
        .collect();
    for i in 0..10_000 {
        signal[30_000 + i] = 0.6 * (i as f32 * 0.05).sin() * (1.0 - i as f32 / 10_000.0) + 0.0001;
    }
    let pre = 441;
    let t = record(
        &signal,
        1,
        Arm::Threshold {
            level: 0.02,
            pre_roll: pre,
            armed_at: 5_000,
        },
        Stop::Silence {
            level: 0.02,
            frames: RATE as u64 / 2,
        },
        None,
    );
    let TakeState::Done { start, end } = t.state() else {
        panic!("{:?}", t.state())
    };
    let onset = (30_000..).find(|&f| signal[f].abs() > 0.02).unwrap() as u64;
    assert_eq!(
        start,
        onset - pre,
        "starts the pre-roll before the first loud frame"
    );
    let last = (0..40_000).rev().find(|&f| signal[f].abs() > 0.02).unwrap() as u64;
    assert_eq!(end, last + 1, "ends after the last loud frame");
    assert_eq!(t.data(), &signal[start as usize..end as usize]);
}

#[test]
fn a_threshold_take_waits_and_a_late_poll_loses_the_take_cleanly() {
    let (mut w, r) = rolling(1, 1_000);
    let mut t = Take::new(
        &r,
        Arm::Threshold {
            level: 0.5,
            pre_roll: 10,
            armed_at: 0,
        },
        Stop::Manual,
        100_000,
    );
    w.push(&[0.0; 800]);
    assert_eq!(t.poll(&r).unwrap(), TakeState::Waiting);
    w.push(&[0.9; 100]);
    assert!(matches!(
        t.poll(&r).unwrap(),
        TakeState::Recording { start: 790, .. }
    ));
    // The worker stalls for longer than the buffer: the take reports it.
    w.push(&[0.9; 3_000]);
    assert_eq!(t.poll(&r), Err(RecordError::Overwritten));
}

#[test]
fn the_length_limit_stops_a_forgotten_take() {
    let (mut w, r) = rolling(1, 10_000);
    let mut t = Take::new(&r, Arm::At(0), Stop::Manual, 2_000);
    for _ in 0..40 {
        w.push(&[0.1; 128]);
        t.poll(&r).unwrap();
    }
    assert_eq!(
        t.state(),
        TakeState::Done {
            start: 0,
            end: 2_000
        }
    );
    assert_eq!(t.data().len(), 2_000);
}

#[test]
fn record_chop_and_play_back_in_one_session() {
    // A drum loop arrives at the input after a second of near-silence, in
    // 128-frame periods. A threshold take catches it with a short pre-roll,
    // it is chopped by transients and played back from the pads.
    let (beat, hits) = demo_beat(100.0, 2, 0, RATE);
    let lead = RATE as usize;
    let mut input = vec![0.0001f32; lead];
    input.extend_from_slice(&beat);
    input.extend(std::iter::repeat_n(0.0001f32, RATE as usize));
    let t = record(
        &input,
        1,
        Arm::Threshold {
            level: 0.01,
            pre_roll: 64,
            armed_at: 0,
        },
        Stop::Length(beat.len() as u64),
        None,
    );
    let TakeState::Done { start, .. } = t.state() else {
        panic!()
    };
    let onset = (0..).find(|&f| input[f].abs() > 0.01).unwrap();
    assert_eq!(start as usize, onset - 64);
    let source = t.into_source("take 1", RATE).unwrap().unwrap();

    let a = SourceAnalysis::analyse(&source);
    let chop = a.transient("take 1", &source, 0.6).unwrap();
    // One slice per hit; the 64-frame pre-roll stays with the first one.
    assert_eq!(chop.slices(), hits.len());
    let mut p = Project::new();
    p.add_source(source.clone());
    let c = p.add_chop_set(chop.clone()).unwrap();
    p.slices_to_pads(c, 0, chop.slices(), 0).unwrap();
    let mut e = Engine::new(RATE, 32);
    e.set_view(EngineView::build(&p, RATE));
    for i in 0..chop.slices() {
        assert!(e.schedule(
            chop.bounds(i).unwrap().0 as u32,
            Event::NoteOn {
                pad: i as u8,
                velocity: 127
            }
        ));
    }
    let frames = source.frames() as usize;
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    let worst = l
        .iter()
        .zip(source.data().iter())
        .map(|(o, x)| (o - std::f32::consts::FRAC_1_SQRT_2 * x).abs())
        .fold(0.0f32, f32::max);
    assert!(worst < 1e-6, "{worst}");
}
