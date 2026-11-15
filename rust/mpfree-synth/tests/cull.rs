//! Load shedding (`Node::cull_candidate`, `Node::cull`) on every synth
//! engine: a voice given up goes with the steal's short fade, the count
//! follows, and the least audible voice goes first.

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::params::{AMP_R, VEL_AMP};
use mpfree_synth::{Analog, Drawbar, Drums, Fm, Keys, Pluck, Poly, Squelch, Wavetable};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;

fn engines() -> Vec<(&'static str, Box<dyn Node>)> {
    vec![
        ("Poly", Box::new(Poly::new(RATE))),
        ("FM", Box::new(Fm::new(RATE))),
        ("Wavetable", Box::new(Wavetable::new(RATE))),
        ("Pluck", Box::new(Pluck::new(RATE))),
        ("Keys", Box::new(Keys::new(RATE))),
        ("Drums", Box::new(Drums::new(RATE))),
        ("Analog", Box::new(Analog::new(RATE))),
    ]
}

/// The organ (its wheels turn whatever the keys do, so a key given up
/// saves little) and the mono bass offer no voice.
#[test]
fn drawbar_and_squelch_offer_no_voice() {
    let mut engines: [(&str, Box<dyn Node>); 2] = [
        ("Drawbar", Box::new(Drawbar::new(RATE))),
        ("Squelch", Box::new(Squelch::new(RATE))),
    ];
    for (name, n) in engines.iter_mut() {
        let mut t = 0;
        render(
            n.as_mut(),
            &mut t,
            960,
            &[(0, on(36, 127)), (0, on(43, 100))],
        );
        assert!(n.cull_candidate().is_none(), "{name}");
        assert!(!n.cull(), "{name}");
    }
}

fn on(note: u8, velocity: u8) -> NodeEvent {
    NodeEvent::NoteOn { note, velocity }
}

/// Renders `frames` frames in blocks from `at`, with `events` (frame,
/// event) on their frames; returns the left channel.
fn render(n: &mut dyn Node, at: &mut u64, frames: usize, events: &[(u64, NodeEvent)]) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames);
    let mut done = 0;
    while done < frames {
        let len = BLOCK.min(frames - done);
        let ev: Vec<Timed> = events
            .iter()
            .filter(|(f, _)| *f >= *at && *f < *at + len as u64)
            .map(|&(f, e)| Timed::new((f - *at) as u32, e))
            .collect();
        let (mut l, mut r) = (vec![0.0; len], vec![0.0; len]);
        n.process(&Ctx::new(RATE, *at), &ev, &mut l, &mut r);
        out.extend_from_slice(&l);
        *at += len as u64;
        done += len;
    }
    out
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, v| m.max(v.abs()))
}

#[test]
fn every_engine_gives_up_a_voice_with_a_fade_and_counts_it() {
    for ((name, mut a), (_, mut b)) in engines().into_iter().zip(engines()) {
        // Two notes (Drums: kick and snare), 20 ms in.
        let events = [(0, on(36, 127)), (0, on(38, 90))];
        let (mut ta, mut tb) = (0, 0);
        render(a.as_mut(), &mut ta, 960, &events);
        render(b.as_mut(), &mut tb, 960, &events);
        assert_eq!(b.sounding_voices(), 2, "{name}");
        let c = b
            .cull_candidate()
            .unwrap_or_else(|| panic!("{name}: no candidate"));
        assert!(c.level.is_finite() && c.level > 0.0, "{name}: {c:?}");
        assert!(b.cull(), "{name}");
        // Already fading: the other voice is the one offered now.
        assert_eq!(
            b.sounding_voices(),
            2,
            "{name}: the fading voice still counts"
        );
        let kept = render(a.as_mut(), &mut ta, 4800, &[]);
        let culled = render(b.as_mut(), &mut tb, 4800, &[]);
        assert_eq!(b.sounding_voices(), a.sounding_voices() - 1, "{name}");
        // What went: the difference, which must start near nothing and
        // grow over the fade (no step), then be the whole voice.
        let gone: Vec<f32> = kept.iter().zip(&culled).map(|(k, c)| k - c).collect();
        let whole = peak(&gone);
        assert!(whole > 1e-4, "{name}: nothing went");
        assert!(
            peak(&gone[..4]) <= 0.05 * whole,
            "{name}: a step of {} against {whole}",
            peak(&gone[..4])
        );
        // The rest of the sound is still there.
        assert!(peak(&culled) > 1e-4, "{name}");
    }
}

#[test]
fn every_engine_gives_up_all_it_has_and_then_says_so() {
    for (name, mut n) in engines() {
        let events = [(0, on(36, 127)), (0, on(38, 100)), (0, on(43, 60))];
        let mut t = 0;
        render(n.as_mut(), &mut t, 960, &events);
        let mut went = 0;
        while n.cull_candidate().is_some() {
            assert!(n.cull(), "{name}");
            went += 1;
            assert!(went <= 3, "{name}: offered a voice already going");
        }
        assert_eq!(went, 3, "{name}");
        assert!(!n.cull(), "{name}");
        render(n.as_mut(), &mut t, 960, &[]);
        assert_eq!(n.sounding_voices(), 0, "{name}");
        // What the engine itself rings with after its last voice (Keys'
        // pickup high-pass and key noise, Analog's chorus) dies within
        // 200 ms.
        render(n.as_mut(), &mut t, 9600, &[]);
        let after = render(n.as_mut(), &mut t, 480, &[]);
        assert!(peak(&after) < 1e-6, "{name}: {}", peak(&after));
    }
}

#[test]
fn the_quietest_voice_goes_first_and_a_releasing_one_counts_quieter() {
    // Poly with full velocity response and a long release.
    let mut p = Poly::new(RATE);
    p.set_param(VEL_AMP, 100.0);
    p.set_param(AMP_R, 2000.0);
    let mut t = 0;
    // A loud and a quiet held note: the quiet one goes.
    render(&mut p, &mut t, 4800, &[(0, on(48, 127)), (0, on(55, 30))]);
    let c = p.cull_candidate().unwrap();
    assert!(!c.releasing);
    assert!(c.level < 0.1, "{c:?}");
    assert!(p.cull());
    let c = p.cull_candidate().unwrap();
    assert!(c.level > 0.5, "the loud one is left: {c:?}");

    // A loud note just let go against a held one at about half its level:
    // the released one is counted 12 dB down, so it goes first.
    let mut p = Poly::new(RATE);
    p.set_param(VEL_AMP, 100.0);
    p.set_param(AMP_R, 2000.0);
    let mut t = 0;
    render(
        &mut p,
        &mut t,
        4800,
        &[
            (0, on(48, 127)),
            (0, on(55, 90)),
            (2400, NodeEvent::NoteOff { note: 48 }),
        ],
    );
    let c = p.cull_candidate().unwrap();
    assert!(c.releasing, "{c:?}");
    assert!(c.level > 0.5, "{c:?}");
}

#[test]
fn an_old_drum_hit_goes_before_a_fresh_one() {
    let mut d = Drums::new(RATE);
    let mut t = 0;
    // The kick at 0, the snare 150 ms later: 10 ms after that, the kick
    // has decayed under the fresh snare.
    render(
        &mut d,
        &mut t,
        7680,
        &[(0, on(36, 127)), (7200, on(38, 127))],
    );
    assert_eq!(d.sounding_voices(), 2);
    let first = d.cull_candidate().unwrap();
    assert!(d.cull());
    let second = d.cull_candidate().unwrap();
    assert!(first.level < second.level, "{first:?} then {second:?}");
}
