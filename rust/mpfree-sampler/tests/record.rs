//! Step S4, workstation half: the rolling buffers and takes, fed period by
//! period the way the audio thread will feed them on the MPC.

use mpfree_sampler::chop::{demo_beat, SourceAnalysis};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_sampler::record::{
    bar_lines, input, rolling, Arm, End, Feed, Input, Looped, Pick, Progress, RecordError,
    Recording, RingOut, Start, Stop, Tail, Take, TakeState, BAR_LINES,
};

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
    // every copy either fails or holds exactly consecutive frames. It
    // keeps going until the reader has made a good copy while it ran (or
    // 16M frames, where f32 still holds every frame number exactly), so a
    // busy machine that starts the reader late can't let the writer
    // finish before any copy was tried.
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let (mut w, r) = rolling(2, 512);
    let copied = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&copied);
    let writer = std::thread::spawn(move || {
        let mut p = 0u64;
        while p < 20_000 || (!seen.load(Ordering::Relaxed) && p < 250_000) {
            w.push(&frame_audio(p * 64, 64));
            p += 1;
            if p.is_multiple_of(256) {
                std::thread::yield_now();
            }
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
                copied.store(true, Ordering::Relaxed);
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

// ------------------------------------------------- the line input (CAP)

/// The audio thread's side, period by period: bar lines every `bar`
/// frames from frame `first_bar` (the transport rolling), then the period
/// of [`frame_audio`].
fn feed_periods(f: &mut Feed, periods: u64, bar: u64, first_bar: u64) {
    for _ in 0..periods {
        let base = f.audio.written();
        let end = base + PERIOD as u64;
        // Bar lines that fall in this period, in order.
        let mut b = if base <= first_bar {
            first_bar
        } else {
            first_bar + (base - first_bar).div_ceil(bar) * bar
        };
        while b < end {
            f.bars.mark(b);
            b += bar;
        }
        f.bars.set_bar(Some(bar as f64));
        f.audio.push(&frame_audio(base, PERIOD));
    }
}

#[test]
fn bar_lines_look_back_and_forward_from_any_frame() {
    let (mut w, r) = bar_lines();
    assert_eq!((r.count(), r.back(0), r.bar_frames()), (0, None, None));
    for k in 0..10u64 {
        w.mark(1_000 + k * 500);
    }
    w.set_bar(Some(500.0));
    assert_eq!(
        (r.back(0), r.back(9), r.back(10)),
        (Some(5_500), Some(1_000), None)
    );
    assert_eq!(r.bar_frames(), Some(500.0));
    // The first, and the third, bar line after frame 2 000 (which is one).
    assert_eq!(r.nth_after(2_000, 1), Some(2_500));
    assert_eq!(r.nth_after(2_000, 3), Some(3_500));
    assert_eq!(r.nth_after(5_500, 1), None, "not passed yet");
    assert_eq!(r.nth_after(0, 1), Some(1_000));
    // Only the last BAR_LINES are kept: further back is not known.
    for k in 10..(BAR_LINES as u64 + 20) {
        w.mark(1_000 + k * 500);
    }
    assert_eq!(r.back(BAR_LINES - 1), Some(1_000 + 20 * 500));
    assert_eq!(r.back(BAR_LINES), None);
    assert_eq!(
        r.nth_after(0, 1),
        None,
        "the first line after frame 0 is gone"
    );
    w.set_bar(None);
    assert_eq!(r.bar_frames(), None, "stopped");
}

#[test]
fn the_last_bars_are_cut_exactly_on_the_bar_lines() {
    let (mut f, i) = input(10, RATE);
    let bar = 22_050; // 4/4 at 480 BPM, short for the test
    assert_eq!(i.last_bars(1), Err(RecordError::NoBarLines));
    feed_periods(&mut f, 1_000, bar, 3_000);
    // Lines at 3 000, 25 050, ... the latest before frame 128 000.
    let latest = i.bars.back(0).unwrap();
    assert_eq!((latest - 3_000) % bar, 0);
    let two = i.last_bars(2).unwrap();
    assert_eq!(two.len() as u64, 2 * bar * 2);
    assert_eq!(two[0], (latest - 2 * bar) as f32);
    assert_eq!(two[two.len() - 2], (latest - 1) as f32);
    assert_eq!(i.last_bars(0), Err(RecordError::BadSpan));
    // More bars than have passed.
    assert_eq!(i.last_bars(40), Err(RecordError::NoBarLines));
    // Skip back by frames, and what is held.
    assert_eq!(i.held(), 128_000);
    let last = i.last_frames(100).unwrap();
    assert_eq!((last[0], last[198]), (127_900.0, 127_999.0));
}

#[test]
fn a_recording_on_the_next_bar_ends_exactly_on_a_bar_line() {
    let (mut f, i) = input(10, RATE);
    let bar = 10_000;
    feed_periods(&mut f, 50, bar, 0);
    // Asked for at frame 6 400: it starts on the line at 10 000 and lasts
    // two bars, to the line at 30 000.
    let mut rec = Recording::new(&i, Start::NextBar, End::Bars(2), RATE as u64 * 60);
    assert_eq!(rec.armed_at(), 6_400);
    assert_eq!(rec.poll(&i), Ok(Progress::Waiting));
    let mut seen_recording = false;
    for _ in 0..40 {
        feed_periods(&mut f, 10, bar, 0);
        if matches!(rec.poll(&i).unwrap(), Progress::Recording { .. }) {
            seen_recording = true;
        }
    }
    assert!(seen_recording);
    assert_eq!(rec.started_at(), Some(10_000));
    assert_eq!(rec.progress(), Progress::Done { frames: 20_000 });
    let s = rec.finish("Take 1", Pick::Stereo).unwrap().unwrap();
    assert_eq!((s.channels(), s.frames(), s.rate()), (2, 20_000, RATE));
    assert_eq!((s.data()[0], s.data()[39_998]), (10_000.0, 29_999.0));
}

#[test]
fn a_recording_without_bar_lines_lasts_its_bars_at_the_tempo_then() {
    let (mut f, i) = input(10, RATE);
    let bar = 4_000;
    feed_periods(&mut f, 10, bar, 0);
    let mut rec = Recording::new(&i, Start::Now, End::Bars(3), RATE as u64 * 60);
    feed_periods(&mut f, 200, bar, 0);
    assert_eq!(rec.poll(&i), Ok(Progress::Done { frames: 12_000 }));
    assert_eq!(rec.started_at(), Some(1_280));
}

#[test]
fn a_press_ends_it_and_the_channels_picked_are_kept() {
    let (mut f, i) = input(10, RATE);
    feed_periods(&mut f, 10, 1_000_000, 0);
    let mut rec = Recording::new(&i, Start::Now, End::Press, RATE as u64 * 60);
    feed_periods(&mut f, 10, 1_000_000, 0);
    assert_eq!(rec.poll(&i), Ok(Progress::Recording { frames: 1_280 }));
    feed_periods(&mut f, 5, 1_000_000, 0);
    rec.stop(&i);
    assert_eq!(rec.poll(&i), Ok(Progress::Done { frames: 1_920 }));
    let left = rec.clone().finish("L", Pick::Left).unwrap().unwrap();
    assert_eq!((left.channels(), left.frames()), (1, 1_920));
    assert_eq!((left.data()[0], left.data()[1]), (1_280.0, 1_281.0));
    let right = rec.clone().finish("R", Pick::Right).unwrap().unwrap();
    assert_eq!(right.data()[0], -1_280.0);
    let mono = rec.finish("M", Pick::Mono).unwrap().unwrap();
    assert_eq!((mono.channels(), mono.data()[0]), (1, 0.0));
}

#[test]
fn stopped_while_waiting_it_records_nothing() {
    let (mut f, i) = input(10, RATE);
    feed_periods(&mut f, 10, 1_000_000, 0);
    // The transport is not rolling: no bar line comes.
    let mut rec = Recording::new(&i, Start::NextBar, End::Bars(1), RATE as u64 * 60);
    feed_periods(&mut f, 10, 1_000_000, 0);
    assert_eq!(rec.poll(&i), Ok(Progress::Waiting));
    rec.stop(&i);
    assert_eq!(rec.poll(&i), Ok(Progress::Nothing));
    assert!(rec.finish("x", Pick::Stereo).is_none());
    // A threshold never crossed, the same.
    let mut rec = Recording::new(
        &i,
        Start::Threshold {
            level: 1e9,
            pre_roll: 100,
        },
        End::Press,
        RATE as u64 * 60,
    );
    feed_periods(&mut f, 10, 1_000_000, 0);
    assert_eq!(rec.poll(&i), Ok(Progress::Waiting));
    rec.stop(&i);
    assert_eq!(rec.poll(&i), Ok(Progress::Nothing));
}

#[test]
fn a_threshold_recording_keeps_its_pre_roll_and_lasts_its_length() {
    let (mut f, i) = input(10, RATE);
    feed_periods(&mut f, 10, 1_000_000, 0);
    // frame_audio's left channel is the frame number: it crosses 3 000 at 3 001.
    let mut rec = Recording::new(
        &i,
        Start::Threshold {
            level: 3_000.0,
            pre_roll: 500,
        },
        End::Frames(1_000),
        RATE as u64 * 60,
    );
    feed_periods(&mut f, 40, 1_000_000, 0);
    assert_eq!(rec.poll(&i), Ok(Progress::Done { frames: 1_000 }));
    assert_eq!(rec.started_at(), Some(2_501));
}

#[test]
fn a_recording_put_on_a_pad_is_one_undo_step() {
    let mut p = Project::new();
    let s = Source::new("Take 1", RATE, 1, vec![0.5; 1_000]).unwrap();
    let id = p.put_source(4, s.clone()).unwrap();
    let pad = p.pad(4).unwrap();
    assert_eq!((pad.name.as_str(), pad.layers.len()), ("Take 1", 1));
    assert_eq!(pad.layers[0].region.source, id);
    assert_eq!(pad.layers[0].region.frames(), 1_000);
    // A pad already there keeps its settings with the new sound.
    p.edit_pad(4, None, |pd| pd.level_db = -6.0).unwrap();
    let s2 = Source::new("Take 2", RATE, 2, vec![0.25; 600]).unwrap();
    p.put_source(4, s2).unwrap();
    let pad = p.pad(4).unwrap();
    assert_eq!((pad.name.as_str(), pad.level_db), ("Take 2", -6.0));
    assert_eq!(p.sources().len(), 2);
    assert!(p.undo());
    assert_eq!(p.pad(4).unwrap().name, "Take 1");
    assert_eq!(p.sources().len(), 1, "the second take's sound went with it");
    assert!(p.undo() && p.undo());
    assert!(p.pad(4).is_none());
    assert!(p.sources().is_empty());
    assert_eq!(p.put_source(PADS, s), Err(ModelError::BadPad));
}

#[test]
fn a_restart_hides_what_came_from_the_old_source() {
    let (mut f, i) = input(10, RATE);
    let bar = 22_050;
    feed_periods(&mut f, 400, bar, 0);
    assert_eq!(i.held(), 51_200);
    // The input now comes from somewhere else (resampling the master),
    // and a reader can see that it has.
    assert_eq!(i.audio.restarts(), 0);
    f.restart();
    assert_eq!(i.audio.restarts(), 1);
    assert_eq!((i.held(), i.audio.oldest()), (0, 51_200));
    assert_eq!(i.last_frames(1_000).unwrap(), Vec::<f32>::new());
    assert_eq!(i.last_bars(1), Err(RecordError::Overwritten));
    feed_periods(&mut f, 200, bar, 0);
    // Only what came after: Skip back stops at the restart.
    let all = i.last_frames(60_000).unwrap();
    assert_eq!((all.len(), all[0]), (2 * 25_600, 51_200.0));
    // The latest bar began before the restart: not kept.
    assert_eq!(i.last_bars(1), Err(RecordError::Overwritten));
    // A bar wholly after the restart is.
    feed_periods(&mut f, 200, bar, 0);
    let one = i.last_bars(1).unwrap();
    assert_eq!((one.len() as u64, one[0]), (2 * bar, 66_150.0));
    // A take from before the restart is refused rather than mixed.
    let mut old = Vec::new();
    assert_eq!(
        i.audio.copy(51_000, 52_000, &mut old),
        Err(RecordError::Overwritten)
    );
    // A threshold take armed now looks only at the new source.
    let rec = Recording::new(
        &i,
        Start::Threshold {
            level: 0.5,
            pre_roll: 100_000,
        },
        End::Frames(10),
        1_000,
    );
    let mut rec = rec;
    feed_periods(&mut f, 1, bar, 0);
    assert!(matches!(rec.poll(&i), Ok(Progress::Done { frames: 10 })));
    assert_eq!(rec.data()[0], 51_200.0, "the pre-roll stops at the restart");
}

#[test]
fn bar_lines_marked_ahead_of_the_audio_wait_for_it() {
    // A point in the mix heard 300 frames late: its bar lines are marked
    // 300 frames into the period after the one the transport crossed them.
    let (mut f, i) = input(10, RATE);
    let bar = 22_050u64;
    let late = 300;
    for _ in 0..600u64 {
        let base = f.audio.written();
        let mut b = (base + PERIOD as u64).div_ceil(bar) * bar;
        b = b.saturating_sub(bar);
        while b < base + PERIOD as u64 {
            if b >= base {
                f.bars.mark(b + late);
            }
            b += bar;
        }
        f.bars.set_bar(Some(bar as f64));
        f.audio.push(&frame_audio(base, PERIOD));
    }
    // Now is 76 800; lines at 300, 22 350, 44 400, 66 450 (all in).
    let one = i.last_bars(1).unwrap();
    assert_eq!(one[0], 44_400.0);
    // A line marked past now (the audio for it isn't in yet) is not used.
    f.bars.mark(i.now() + 200);
    assert_eq!(i.bars_back(1), Some((44_400, 66_450)));
    let again = i.last_bars(1).unwrap();
    assert_eq!(again, one);
    // A take on the next bar starts on that line once its audio comes.
    let mut rec = Recording::new(&i, Start::NextBar, End::Frames(64), 1_000);
    let line = i.now() + 200;
    feed_periods_plain(&mut f, 1);
    assert!(
        matches!(rec.poll(&i), Ok(Progress::Waiting)),
        "its audio isn't in yet"
    );
    feed_periods_plain(&mut f, 1);
    assert!(matches!(
        rec.poll(&i),
        Ok(Progress::Recording { frames: 56 })
    ));
    feed_periods_plain(&mut f, 1);
    assert!(matches!(rec.poll(&i), Ok(Progress::Done { frames: 64 })));
    assert_eq!(rec.data()[0], line as f32);
}

/// Periods of [`frame_audio`] with no bar lines.
fn feed_periods_plain(f: &mut Feed, periods: u64) {
    for _ in 0..periods {
        let base = f.audio.written();
        f.audio.push(&frame_audio(base, PERIOD));
    }
}

// ------------------------------------------------------------ tails

/// Periods of `sound(frame)` on both channels, with bar lines every `bar`
/// frames from frame 0 (the transport rolling).
fn feed_sound(f: &mut Feed, periods: u64, bar: u64, sound: &dyn Fn(u64) -> f32) {
    for _ in 0..periods {
        let base = f.audio.written();
        let end = base + PERIOD as u64;
        let mut b = base.div_ceil(bar) * bar;
        while b < end {
            f.bars.mark(b);
            b += bar;
        }
        f.bars.set_bar(Some(bar as f64));
        let audio: Vec<f32> = (base..end)
            .flat_map(|i| {
                let v = sound(i);
                [v, v]
            })
            .collect();
        f.audio.push(&audio);
    }
}

/// Feeds `sound` ten periods at a time, polling `rec` after each, until
/// it is done (or gives up after `most` polls).
fn run(f: &mut Feed, i: &Input, rec: &mut Recording, bar: u64, sound: &dyn Fn(u64) -> f32) {
    for _ in 0..400 {
        feed_sound(f, 10, bar, sound);
        if matches!(
            rec.poll(i).unwrap(),
            Progress::Done { .. } | Progress::Nothing
        ) {
            return;
        }
    }
    panic!("never done: {:?}", rec.progress());
}

const RING: RingOut = RingOut {
    level: 0.001,
    quiet: 2_000,
    max: 44_100,
};

const LOOP: Tail = Tail::Loop {
    level: 0.001,
    quiet: 2_000,
};

/// Silent before frame 10 000, 1.0 from there to 30 000 (two bars of
/// 10 000), then 0.25 ringing on to `ring_to`.
fn two_bars_ringing(ring_to: u64) -> impl Fn(u64) -> f32 {
    move |f| match f {
        10_000..30_000 => 1.0,
        30_000.. if f < ring_to => 0.25,
        _ => 0.0,
    }
}

#[test]
fn a_bar_take_that_rings_out_keeps_its_tail_until_it_is_silent() {
    let (mut f, i) = input(10, RATE);
    let bar = 10_000;
    let sound = two_bars_ringing(33_000);
    feed_sound(&mut f, 50, bar, &sound);
    let mut rec = Recording::with_tail(
        &i,
        Start::NextBar,
        End::Bars(2),
        Tail::Ring(RING),
        RATE as u64 * 60,
    );
    run(&mut f, &i, &mut rec, bar, &sound);
    assert_eq!(rec.started_at(), Some(10_000));
    assert_eq!(rec.cut_frames(), Some(20_000), "its two bars");
    assert_eq!(
        rec.progress(),
        Progress::Done { frames: 23_000 },
        "and the ringing, to its last sound"
    );
    let s = rec.finish("Rung", Pick::Mono).unwrap().unwrap();
    assert_eq!(s.frames(), 23_000);
    assert_eq!((s.data()[19_999], s.data()[20_000]), (1.0, 0.25));
    assert_eq!(s.data()[22_999], 0.25);
}

#[test]
fn a_ring_out_ends_at_its_longest_and_follows_a_length_too() {
    let (mut f, i) = input(10, RATE);
    let bar = 10_000;
    let loud = |_: u64| 0.5;
    feed_sound(&mut f, 50, bar, &loud);
    let ring = RingOut { max: 5_000, ..RING };
    let mut rec = Recording::with_tail(
        &i,
        Start::NextBar,
        End::Bars(1),
        Tail::Ring(ring),
        RATE as u64 * 60,
    );
    run(&mut f, &i, &mut rec, bar, &loud);
    assert_eq!(rec.progress(), Progress::Done { frames: 15_000 });
    // Not on a bar line: a length in frames rings out the same way.
    let mut rec = Recording::with_tail(
        &i,
        Start::Now,
        End::Frames(4_000),
        Tail::Ring(ring),
        RATE as u64 * 60,
    );
    run(&mut f, &i, &mut rec, bar, &loud);
    assert_eq!(rec.cut_frames(), Some(4_000));
    assert_eq!(rec.progress(), Progress::Done { frames: 9_000 });
    // The longest take still holds, ringing included.
    let mut rec = Recording::with_tail(&i, Start::Now, End::Frames(4_000), Tail::Ring(ring), 6_000);
    run(&mut f, &i, &mut rec, bar, &loud);
    assert_eq!(rec.progress(), Progress::Done { frames: 6_000 });
}

#[test]
fn a_loop_folds_its_ringing_onto_its_start_and_keeps_its_length() {
    let (mut f, i) = input(10, RATE);
    let bar = 10_000;
    let sound = two_bars_ringing(33_000);
    feed_sound(&mut f, 50, bar, &sound);
    let mut rec = Recording::with_tail(&i, Start::NextBar, End::Bars(2), LOOP, RATE as u64 * 60);
    run(&mut f, &i, &mut rec, bar, &sound);
    assert_eq!(
        rec.looped(),
        Some(Looped::Folded { frames: 3_000 }),
        "it started from silence and went silent"
    );
    let s = rec.finish("Loop", Pick::Stereo).unwrap().unwrap();
    assert_eq!(s.frames(), 20_000, "exactly its two bars");
    let left: Vec<f32> = s.data().iter().step_by(2).copied().collect();
    assert!(
        left[..3_000].iter().all(|x| *x == 1.25),
        "the ringing, folded"
    );
    assert!(left[3_000..].iter().all(|x| *x == 1.0));
}

#[test]
fn a_loop_whose_music_went_on_keeps_its_second_time_round() {
    // Started from silence, and the bar played again after it: the
    // second time round has the first's ringing at its start.
    let (mut f, i) = input(10, RATE);
    let bar = 10_000;
    let sound = |fr: u64| match fr {
        0..10_000 => 0.0,
        _ if fr % 10_000 < 2_000 && fr >= 20_000 => 1.25,
        _ => 1.0,
    };
    feed_sound(&mut f, 50, bar, &sound);
    let mut rec = Recording::with_tail(&i, Start::NextBar, End::Bars(1), LOOP, RATE as u64 * 60);
    run(&mut f, &i, &mut rec, bar, &sound);
    assert_eq!(rec.looped(), Some(Looped::SecondTime));
    assert_eq!(
        rec.progress(),
        Progress::Done { frames: 20_000 },
        "it waited one more bar, no longer"
    );
    let s = rec.finish("Second", Pick::Left).unwrap().unwrap();
    assert_eq!(s.frames(), 10_000);
    assert!(s.data()[..2_000].iter().all(|x| *x == 1.25));
    assert!(s.data()[2_000..].iter().all(|x| *x == 1.0));
}

#[test]
fn a_loop_of_music_already_going_round_is_cut_not_folded() {
    let (mut f, i) = input(10, RATE);
    let bar = 10_000;
    // The same two bars played before: their ringing is in its start.
    let sound = |fr: u64| match fr {
        0..10_000 => 0.25,
        10_000..30_000 => 1.0,
        30_000..33_000 => 0.25,
        _ => 0.0,
    };
    feed_sound(&mut f, 50, bar, &sound);
    let mut rec = Recording::with_tail(&i, Start::NextBar, End::Bars(2), LOOP, RATE as u64 * 60);
    // It is done on the bar line: nothing to wait for after it.
    run(&mut f, &i, &mut rec, bar, &sound);
    assert_eq!(rec.looped(), Some(Looped::GoingRound));
    assert_eq!(rec.progress(), Progress::Done { frames: 20_000 });
    let s = rec.finish("Going round", Pick::Left).unwrap().unwrap();
    assert_eq!(s.frames(), 20_000);
    assert!(s.data().iter().all(|x| *x == 1.0), "nothing added twice");
}

#[test]
fn a_press_ends_a_take_at_once_ringing_or_not() {
    let (mut f, i) = input(10, RATE);
    let bar = 10_000;
    let loud = |_: u64| 0.5;
    feed_sound(&mut f, 50, bar, &loud);
    // Before its length, cut or not.
    for tail in [Tail::Cut, Tail::Ring(RING), LOOP] {
        let mut rec = Recording::with_tail(&i, Start::Now, End::Bars(4), tail, RATE as u64 * 60);
        feed_sound(&mut f, 10, bar, &loud);
        assert!(matches!(rec.poll(&i), Ok(Progress::Recording { .. })));
        rec.stop(&i);
        assert_eq!(rec.poll(&i), Ok(Progress::Done { frames: 1_280 }));
        assert_eq!(
            rec.finish("x", Pick::Left).unwrap().unwrap().frames(),
            1_280
        );
    }
    // Pressed while it rings: it ends there.
    let mut rec = Recording::with_tail(
        &i,
        Start::Now,
        End::Frames(1_000),
        Tail::Ring(RING),
        RATE as u64 * 60,
    );
    feed_sound(&mut f, 20, bar, &loud);
    assert!(matches!(rec.poll(&i), Ok(Progress::Recording { .. })));
    rec.stop(&i);
    assert_eq!(rec.poll(&i), Ok(Progress::Done { frames: 2_560 }));
}

#[test]
fn a_poll_after_the_last_bar_line_still_keeps_the_bars() {
    // The screen thread was held up past both bar lines: the take still
    // starts on the first and ends on the second.
    let (mut f, i) = input(10, RATE);
    let bar = 10_000;
    feed_periods(&mut f, 50, bar, 0);
    let mut rec = Recording::new(&i, Start::NextBar, End::Bars(1), RATE as u64 * 60);
    feed_periods(&mut f, 150, bar, 0);
    assert_eq!(rec.poll(&i), Ok(Progress::Done { frames: 10_000 }));
    assert_eq!(rec.started_at(), Some(10_000));
    let s = rec.finish("Late", Pick::Left).unwrap().unwrap();
    assert_eq!((s.data()[0], s.data()[9_999]), (10_000.0, 19_999.0));
    // Taking a single take by hand the same way.
    let (mut w, r) = rolling(2, 100_000);
    let mut t = Take::new(&r, Arm::At(500), Stop::Manual, 100_000);
    w.push(&frame_audio(0, 2_000));
    t.stop_at(1_500);
    assert_eq!(
        t.poll(&r),
        Ok(TakeState::Done {
            start: 500,
            end: 1_500
        })
    );
}
