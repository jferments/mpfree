//! Phase 0A's gate: the contracts the model must meet before the engine
//! grows. Re-fitting (contract 2), timing order (contract 3) and
//! repeatable results (contract 2's stable dice), each tested end to end
//! through the public API.

use mpfree_compose::dice::{DiceName, Job};
use mpfree_compose::realize::{decisions_of, realize, Fit, PlainNote};
use mpfree_compose::refit::{action, misfits, Action, Change, Target};
use mpfree_compose::timing::{heard_tick, written, Playback, Timing};
use mpfree_compose::{
    clip_notes, Event, InstrumentKind, Motion, NoteDecision, Part, PartId, Role, Source,
};
use mpfree_context::chord::{Chord, ChordAnswer, ChordTimeline, Heard, Quality, Tone};
use mpfree_context::groove::{GrooveTemplate, Hit, Lane, STEP};
use mpfree_context::harmony::{Harmony, Resolver, RowHarmony};
use mpfree_context::key::{Key, Mode};
use mpfree_context::PartRole;
use mpfree_transport::time::{Swing, Tick, PPQ};

const BEAT: Tick = PPQ as Tick;
const BAR: Tick = 4 * BEAT;
const BPM: f64 = 92.0;

fn row(chords: &[Chord]) -> Resolver {
    let line = ChordTimeline::of(
        &chords.iter().map(|&c| (c, BAR)).collect::<Vec<_>>(),
        Heard::Heard,
    );
    let length = line.len();
    Resolver::grid(RowHarmony {
        line,
        length,
        suggested: false,
    })
}

fn cm7_fm7_ebmaj7_gm7() -> [Chord; 4] {
    [
        Chord::new(0, Quality::Minor7),
        Chord::new(5, Quality::Minor7),
        Chord::new(3, Quality::Major7),
        Chord::new(7, Quality::Minor7),
    ]
}

fn fit(h: &dyn Harmony) -> Fit<'_> {
    Fit {
        harmony: h,
        key: Some(Key::new(0, Mode::Minor)),
        played_pcs: 0,
        origin: 0,
    }
}

/// A walking-ish bass: root on 1, fifth on 2, a passing note on 3, an
/// approach on 4 into the next bar's root.
fn bass_part(bars: Tick) -> Part {
    let mut decisions = Vec::new();
    for b in 0..bars {
        let t = b * BAR;
        let d = |at, role, m| NoteDecision::new(at, BEAT, 100, role, m);
        decisions.push(d(
            t,
            Role::Chord(Tone::Root),
            if b == 0 {
                Motion::NEAREST
            } else {
                Motion::down(3)
            },
        ));
        decisions.push(d(t + BEAT, Role::Chord(Tone::Fifth), Motion::up(7)));
        decisions.push(d(t + 2 * BEAT, Role::Passing, Motion::down(2)));
        decisions.push(d(t + 3 * BEAT, Role::Approach, Motion::up(1)));
    }
    Part {
        id: PartId(2),
        role: PartRole::Bass,
        kind: InstrumentKind::Bass,
        register: 36,
        decisions,
    }
}

fn anchor_dirs(part: &Part, ev: &[Event]) -> Vec<i32> {
    let anchors: Vec<i32> = part
        .decisions
        .iter()
        .zip(ev)
        .filter(|(d, _)| d.role.anchors())
        .map(|(_, e)| i32::from(e.pitch))
        .collect();
    anchors.windows(2).map(|w| (w[1] - w[0]).signum()).collect()
}

// ---------------------------------------------------------------- re-fitting

#[test]
fn a_chord_change_refits_the_bass_keeping_its_rhythm_and_shape() {
    let part = bass_part(4);
    let before_row = row(&cm7_fm7_ebmaj7_gm7());
    let before = realize(&part, &fit(&before_row));

    // Bar 3's chord becomes Abmaj7. The bass is in a preview: re-fitted.
    let mut chords = cm7_fm7_ebmaj7_gm7();
    chords[2] = Chord::new(8, Quality::Major7);
    let after_row = row(&chords);
    assert_eq!(action(Change::Chord, Target::Preview, false), Action::Refit);
    let after = realize(&part, &fit(&after_row));

    // Rhythm, lengths, loudness and timing are untouched.
    for (a, b) in before.iter().zip(&after) {
        assert_eq!(
            (a.at, a.len, a.velocity, a.timing),
            (b.at, b.len, b.velocity, b.timing)
        );
    }
    // Bar 3's chord tones are Abmaj7's now.
    for (d, e) in part.decisions.iter().zip(&after) {
        if let (Role::Chord(t), true) = (d.role, (2 * BAR..3 * BAR).contains(&d.at)) {
            assert_eq!(
                i32::from(e.pitch).rem_euclid(12),
                i32::from(chords[2].tone(t)),
                "{t:?}"
            );
        }
    }
    // Bars 1 and 2 are unchanged up to the approach into bar 3.
    for (a, b) in before.iter().zip(&after).take(7) {
        assert_eq!(a.pitch, b.pitch);
    }
    assert_ne!(
        before[7].pitch, after[7].pitch,
        "the approach leads into the new chord"
    );
    // The line's shape (which way each chord tone moves) is kept.
    assert_eq!(anchor_dirs(&part, &before), anchor_dirs(&part, &after));
}

#[test]
fn a_kept_clip_is_marked_and_offered_a_fit_never_rewritten() {
    let part = bass_part(4);
    let old_row = row(&cm7_fm7_ebmaj7_gm7());
    let kept: Vec<PlainNote> = realize(&part, &fit(&old_row))
        .iter()
        .map(|e| PlainNote {
            at: e.at,
            len: e.len,
            pitch: e.pitch,
            velocity: e.velocity,
        })
        .collect();

    let mut chords = cm7_fm7_ebmaj7_gm7();
    chords[2] = Chord::new(8, Quality::Major7);
    let new_row = row(&chords);
    let bad = misfits(&kept, &old_row, &new_row, 0);
    assert!(!bad.is_empty());
    assert!(bad
        .iter()
        .all(|&i| (2 * BAR..3 * BAR).contains(&kept[i].at)));
    assert_eq!(
        action(Change::Chord, Target::Kept, bad.is_empty()),
        Action::OfferFit
    );
    // Unchanged chords: nothing to offer.
    assert!(misfits(&kept, &old_row, &old_row, 0).is_empty());

    // "Fit to the new chords": decisions read back from the kept notes,
    // realised under the new chords. Rhythm stays; bar 3 moves.
    let (register, ds) = decisions_of(&kept, &fit(&old_row), Source::You);
    let read = Part {
        id: PartId(9),
        role: PartRole::Bass,
        kind: InstrumentKind::Bass,
        register,
        decisions: ds,
    };
    let same = realize(&read, &fit(&old_row));
    assert!(
        same.iter().zip(&kept).all(|(e, k)| e.pitch == k.pitch),
        "read back exactly"
    );
    let fitted = realize(&read, &fit(&new_row));
    assert!(fitted
        .iter()
        .zip(&kept)
        .all(|(e, k)| e.at == k.at && e.len == k.len));
    assert!(misfits(
        &fitted
            .iter()
            .map(|e| PlainNote {
                at: e.at,
                len: e.len,
                pitch: e.pitch,
                velocity: e.velocity
            })
            .collect::<Vec<_>>(),
        &new_row,
        &new_row,
        0
    )
    .is_empty());
    for &i in &bad {
        assert!(chords[2].has(i32::from(fitted[i].pitch)));
    }
    assert!(fitted.iter().all(|e| e.source == Source::You));
}

#[test]
fn an_instrument_change_refits_range_and_lengths_only() {
    let mut part = bass_part(2);
    let r = row(&cm7_fm7_ebmaj7_gm7());
    part.kind = InstrumentKind::Chordal;
    part.register = 60;
    part.decisions.iter_mut().for_each(|d| d.len = 2 * BEAT);
    let keys = realize(&part, &fit(&r));
    part.kind = InstrumentKind::Bass;
    part.register = 36;
    let bass = realize(&part, &fit(&r));
    let profile = InstrumentKind::Bass.profile();
    for (k, b) in keys.iter().zip(&bass) {
        assert_eq!(k.at, b.at);
        assert_eq!(k.pitch % 12, b.pitch % 12, "same notes, another octave");
        assert!(profile.in_range(i32::from(b.pitch)));
    }
    // Keys may overlap; a bass never does.
    assert!(keys.windows(2).any(|w| w[0].at + w[0].len > w[1].at));
    assert!(bass.windows(2).all(|w| w[0].at + w[0].len <= w[1].at));
}

#[test]
fn with_no_clear_chord_nothing_is_invented() {
    let none = Resolver::nothing();
    let played = (1 << 0) | (1 << 3) | (1 << 7); // C Eb G
    let f = Fit {
        harmony: &none,
        key: None,
        played_pcs: played,
        origin: 0,
    };
    let mut part = bass_part(2);
    part.kind = InstrumentKind::Chordal;
    for e in realize(&part, &f) {
        if e.role.anchors() {
            assert!(played & (1 << (e.pitch % 12)) != 0, "{}", e.pitch);
        }
    }
    assert_eq!(none.at(0).answer, ChordAnswer::NoClearChord);
}

// ------------------------------------------------------------- timing order

const SWING_61: Swing = Swing {
    grid: PPQ / 4,
    amount: 61,
};

fn ms(ticks: f64) -> f64 {
    ticks * 60_000.0 / (BPM * f64::from(PPQ))
}

/// Two bars a drummer played: kick 11 ms early on 1 and 3, snare 28 ms
/// late on 2 and 4, hats on every 1/16 swung 61 %.
fn drummer() -> Vec<Hit> {
    let t = |ms: f64| ms * BPM * f64::from(PPQ) / 60_000.0;
    let mut hits = Vec::new();
    for b in 0..2 {
        let bar = b * BAR;
        for s in [0, 8] {
            hits.push(Hit {
                lane: Lane::Kick,
                heard: (bar + s * STEP) as f64 - t(11.0),
                velocity: 110,
            });
        }
        for s in [4, 12] {
            hits.push(Hit {
                lane: Lane::Snare,
                heard: (bar + s * STEP) as f64 + t(28.0),
                velocity: 100,
            });
        }
        for s in 0..16 {
            hits.push(Hit {
                lane: Lane::Hats,
                heard: SWING_61.apply((bar + s * STEP) as f64),
                velocity: 80,
            });
        }
    }
    hits
}

/// Where the drummer's hit on `lane` at `step` was heard, in bar 1.
fn drummer_at(lane: Lane, step: Tick) -> f64 {
    drummer()
        .iter()
        .filter(|h| h.lane == lane)
        .map(|h| h.heard)
        .min_by(|a, b| {
            (a - (step * STEP) as f64)
                .abs()
                .total_cmp(&(b - (step * STEP) as f64).abs())
        })
        .unwrap()
}

#[test]
fn new_notes_land_on_the_drummer_with_swing_applied_once() {
    // The clip the new part goes into swings 1/16s at 61 %, like the hats.
    let pb = Playback {
        swing: SWING_61,
        ..Playback::plain(2 * BAR)
    };
    // Read the groove against straight playback or against the clip's
    // swing: either way the notes land where the drummer played.
    for read_against in [Swing::default(), SWING_61] {
        let g = GrooveTemplate::read(&drummer(), 16, read_against);
        for (lane, step) in [
            (Lane::Kick, 0),
            (Lane::Kick, 8),
            (Lane::Snare, 4),
            (Lane::Hats, 3),
            (Lane::Hats, 7),
        ] {
            let timing = Timing::from_groove(&g, lane, step as usize, pb.swing);
            let w = written(step * STEP, timing, BPM, pb.length);
            let heard = heard_tick(w, &pb) as f64;
            let target = drummer_at(lane, step);
            // An early note on the first step is written at the clip's end,
            // heard just before the next pass: compare around the loop.
            let len = pb.length as f64;
            let apart = (heard - target).rem_euclid(len);
            assert!(
                ms(apart.min(len - apart)) < 1.0,
                "{lane:?} step {step} read against {}: heard {heard}, drummer {target}",
                read_against.amount
            );
        }
    }
}

#[test]
fn total_offsets_would_swing_twice() {
    // What the contract prevents: offsets measured from the straight grid
    // (swing included) written into a clip that playback swings again.
    let pb = Playback {
        swing: SWING_61,
        ..Playback::plain(2 * BAR)
    };
    let g = GrooveTemplate::read(&drummer(), 16, Swing::default());
    let step = 3;
    let total = Timing {
        groove: g.cell(Lane::Hats, step).offset,
        play: 0.0,
    };
    let wrong = heard_tick(written(step as Tick * STEP, total, BPM, pb.length), &pb) as f64;
    let right_timing = Timing::from_groove(&g, Lane::Hats, step, pb.swing);
    let right = heard_tick(
        written(step as Tick * STEP, right_timing, BPM, pb.length),
        &pb,
    ) as f64;
    let target = drummer_at(Lane::Hats, step as Tick);
    assert!(ms((wrong - target).abs()) > 15.0, "double swing is audible");
    assert!(ms((right - target).abs()) < 1.0);
}

#[test]
fn the_preview_and_the_kept_clip_play_every_note_at_the_same_time() {
    let g = GrooveTemplate::read(&drummer(), 16, Swing::default());
    let pb = Playback {
        swing: SWING_61,
        start: 7 * BAR,
        ..Playback::plain(2 * BAR)
    };
    let r = row(&cm7_fm7_ebmaj7_gm7());
    let mut part = bass_part(2);
    for d in part.decisions.iter_mut() {
        let step = (d.at.rem_euclid(BAR) / STEP) as usize;
        d.timing = Timing::from_groove(&g, Lane::Kick, step, pb.swing);
        d.timing.play = if d.role.anchors() { 3.0 } else { -2.0 };
    }
    let events = realize(&part, &fit(&r));
    let preview: Vec<(Tick, u8)> = events
        .iter()
        .map(|e| {
            (
                heard_tick(written(e.at, e.timing, BPM, pb.length), &pb),
                e.pitch,
            )
        })
        .collect();
    let kept = clip_notes(&events, BPM, pb.length).unwrap();
    let mut played: Vec<(Tick, u8)> = kept
        .iter()
        .map(|n| (heard_tick(n.tick, &pb), n.note))
        .collect();
    let mut preview = preview;
    preview.sort();
    played.sort();
    assert_eq!(preview, played);
}

// ------------------------------------------------------- repeatable results

/// A toy bar maker for the dice contract: rhythm and roles drawn from the
/// bar's own dice.
fn make_bar(seed: u64, bar: u16, attempt: u16) -> Vec<NoteDecision> {
    let mut rhythm = DiceName {
        seed,
        part: 1,
        phrase: 0,
        job: Job::Rhythm,
        bar,
        attempt,
    }
    .dice();
    let mut pitch = DiceName {
        seed,
        part: 1,
        phrase: 0,
        job: Job::Pitch,
        bar,
        attempt,
    }
    .dice();
    let roles = [
        Role::Chord(Tone::Root),
        Role::Chord(Tone::Third),
        Role::Chord(Tone::Fifth),
        Role::Scale,
    ];
    let base = Tick::from(bar) * BAR;
    let mut out = Vec::new();
    for s in 0..16 {
        if rhythm.chance(0.5) {
            out.push(NoteDecision::new(
                base + s * STEP,
                STEP,
                80 + rhythm.below(40) as u8,
                roles[pitch.below(roles.len())],
                Motion {
                    dir: pitch.below(3) as i8 - 1,
                    size: pitch.below(5) as u8,
                },
            ));
        }
    }
    out
}

#[test]
fn trying_one_bar_again_changes_only_that_bar() {
    let first: Vec<Vec<NoteDecision>> = (0..16).map(|b| make_bar(7, b, 0)).collect();
    for attempt in 1..4 {
        let mut again = first.clone();
        again[5] = make_bar(7, 5, attempt);
        assert_ne!(again[5], first[5], "try {attempt} is new");
        for b in (0..16).filter(|&b| b != 5) {
            assert_eq!(again[b], first[b], "bar {}", b + 1);
        }
    }
    // Made in any order, the bars are the same.
    let backwards: Vec<Vec<NoteDecision>> =
        (0..16).rev().map(|b| make_bar(7, b, 0)).rev().collect();
    assert_eq!(backwards, first);
}

#[test]
fn the_same_request_makes_the_same_notes() {
    let r = row(&cm7_fm7_ebmaj7_gm7());
    let part = Part {
        id: PartId(1),
        role: PartRole::Melody,
        kind: InstrumentKind::OneNote,
        register: 67,
        decisions: (0..4).flat_map(|b| make_bar(99, b, 0)).collect(),
    };
    let a = realize(&part, &fit(&r));
    let b = realize(&part, &fit(&r));
    assert_eq!(a, b);
    assert_eq!(clip_notes(&a, BPM, 4 * BAR), clip_notes(&b, BPM, 4 * BAR));
    // Another seed is another tune.
    let other = Part {
        decisions: (0..4).flat_map(|b| make_bar(100, b, 0)).collect(),
        ..part.clone()
    };
    assert_ne!(realize(&other, &fit(&r)), a);
}
