//! Cross-check with mpfree-core's ClickEngine, the metronome that already
//! ran on the MPC: at a constant tempo, the transport's beats land on the
//! very frames the ClickEngine clicks on (both place beat k at
//! round(k * 60 * rate / bpm), from absolute values).

use mpfree_core::{ClickConfig, ClickEngine};
use mpfree_transport::{Block, Command, Transport};

fn click_frames(rate: u32, bpm: f64, frames: u64) -> Vec<u64> {
    let mut e = ClickEngine::new();
    assert!(e.configure(ClickConfig {
        sample_rate: rate,
        bpm,
        beats_per_bar: 4
    }));
    let (mut l, mut r) = ([0.0f32; 1], [0.0f32; 1]);
    let mut out = vec![];
    for f in 0..frames {
        let before = e.beats_emitted();
        e.render(&mut l, &mut r).unwrap();
        if e.beats_emitted() > before {
            out.push(f);
        }
    }
    out
}

fn transport_beats(rate: u32, bpm: f64, frames: u64) -> Vec<u64> {
    let mut t = Transport::new(rate, bpm).unwrap();
    t.command(0, Command::Play);
    let mut b = Block::new();
    let mut out = vec![];
    while t.frame() < frames {
        t.process(((frames - t.frame()) as u32).min(256), &mut b);
        for s in b.spans() {
            out.extend(
                s.lines(s.meter.beat_ticks())
                    .map(|(_, o)| b.start_frame + u64::from(o)),
            );
        }
    }
    out
}

#[test]
fn beats_match_the_click_engine() {
    for &(rate, bpm) in &[
        (44_100, 120.0),
        (44_100, 93.0),
        (44_100, 127.3),
        (48_000, 140.0),
        (44_100, 400.0),
        (22_050, 20.5),
        (96_000, 77.77),
    ] {
        let frames = u64::from(rate) * 12;
        let click = click_frames(rate, bpm, frames);
        assert!(click.len() >= 4);
        assert_eq!(
            transport_beats(rate, bpm, frames),
            click,
            "{rate} Hz, {bpm} BPM"
        );
    }
}
