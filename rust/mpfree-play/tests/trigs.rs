//! Trig conditions and ratchets: A:B and 1ST count passes from the clip's
//! start by song position (across locates, loops and relaunches); FILL
//! switches on an exact frame; PRE and NEI follow the last conditional
//! note; a ratchet plays whole or not at all, spread over its note; a note
//! left out leaves out its own locks; and all of it the same at every
//! period size.

use mpfree_play::{anchored, Cue, Event, Launch, Out, PlayView, Player, Sink};
use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
use mpfree_seq::{Clip, Cond, Lock, Note, Project, Quantise, Speed, Track};
use mpfree_transport::time::Swing;
use mpfree_transport::{Block, Command, Transport};

const STEP: i64 = 240;
const BAR: i64 = 3840;
/// Frames in one bar at 120 BPM: 2 s.
const BAR_FRAMES: u64 = 88_200;
const CUT: RelAddr = RelAddr::new(OnTrack::Inst, 7);

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, out: Out) {
        self.0.push(out);
    }
}

/// What the host does: the project, what plays, and what happens when.
#[derive(Clone, Default)]
struct Session {
    project: Project,
    launches: Vec<Launch>,
    /// Transport commands, on absolute frames.
    commands: Vec<(u64, Command)>,
    /// FILL on or off, on absolute frames.
    fills: Vec<(u64, bool)>,
    /// Cues made once the transport has passed a frame.
    cues: Vec<(u64, Cue)>,
}

impl Session {
    fn new(clips: Vec<Clip>) -> Session {
        let mut project = Project::new();
        let mut launches = vec![];
        for (i, c) in clips.into_iter().enumerate() {
            let mut t = Track::new("t", i as u16);
            t.slots[0] = Some(c);
            project.tracks.push(t);
            launches.push(Launch {
                track: i,
                slot: 0,
                origin: 0,
                start: 0,
            });
        }
        Session {
            project,
            launches,
            ..Session::default()
        }
    }
}

/// Play `frames` frames in periods of `period`, as the pad player does:
/// cues and started clips go through the launches and a new compile.
fn run(s: &Session, period: u32, frames: u64) -> (Vec<Out>, Player) {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    for &(at, c) in &s.commands {
        assert!(t.command(at, c));
    }
    t.command(0, Command::Play);
    let compile = |l: &[Launch], c: &[Cue]| {
        PlayView::compile_cued(&s.project, l, c).expect("the session compiles")
    };
    let (mut launches, mut cues) = (s.launches.clone(), vec![]);
    let mut view = compile(&launches, &cues);
    let mut later = s.cues.clone();
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    while t.frame() < frames {
        let now = t.frame();
        let n = period.min((frames - now) as u32);
        let before = later.len();
        later.retain(|&(at, c)| {
            let due = at <= now;
            if due {
                cues.push(c);
            }
            !due
        });
        if later.len() != before {
            view = compile(&launches, &cues);
        }
        for &(at, on) in &s.fills {
            if (now..now + u64::from(n)).contains(&at) {
                player.set_fill(on, (at - now) as u32);
            }
        }
        t.process(n, &mut block);
        player.process(&view, &block, &mut log);
        if !player.started().is_empty() {
            for st in player.started() {
                st.apply(&mut launches, &mut cues);
            }
            view = compile(&launches, &cues);
        }
    }
    assert_eq!(player.stats().dropped, 0);
    (log.0, player)
}

fn note(tick: i64, n: u8, cond: Cond) -> Note {
    let mut x = Note::new(tick, STEP / 2, n, 100);
    x.cond = cond;
    x
}

fn clip(length: i64, notes: &[Note]) -> Clip {
    let mut c = Clip::new("c", length);
    c.notes = notes.to_vec();
    c
}

/// Note-ons: (frame, track, note).
fn ons(log: &[Out]) -> Vec<(u64, u16, u8)> {
    log.iter()
        .filter_map(|o| match o.event {
            Event::NoteOn { note, .. } => Some((o.frame, o.track, note)),
            _ => None,
        })
        .collect()
}

/// The notes each bar played, from frame 0 with no jumps: (bar, note).
fn by_bar(log: &[Out]) -> Vec<(u64, u8)> {
    ons(log)
        .into_iter()
        .map(|(f, _, n)| (f / BAR_FRAMES, n))
        .collect()
}

/// The bars `n` played on.
fn bars_of(log: &[Out], n: u8) -> Vec<u64> {
    by_bar(log)
        .into_iter()
        .filter(|x| x.1 == n)
        .map(|x| x.0)
        .collect()
}

fn frames_of(log: &[Out], n: u8) -> Vec<u64> {
    ons(log)
        .into_iter()
        .filter(|x| x.2 == n)
        .map(|x| x.0)
        .collect()
}

const PASSES: [Cond; 5] = [
    Cond::Every { a: 1, b: 2 },
    Cond::Every { a: 2, b: 2 },
    Cond::Every { a: 3, b: 4 },
    Cond::First,
    Cond::NotFirst,
];

/// One bar: 1:2, 2:2, 3:4, 1ST and !1ST on notes 60 to 64.
fn passes_clip() -> Clip {
    let notes: Vec<Note> = PASSES
        .iter()
        .enumerate()
        .map(|(i, &c)| note(i as i64 * 3 * STEP, 60 + i as u8, c))
        .collect();
    clip(BAR, &notes)
}

#[test]
fn a_of_b_and_first_count_passes_from_the_clips_start() {
    let s = Session::new(vec![passes_clip()]);
    let (log, player) = run(&s, 128, 8 * BAR_FRAMES);
    assert_eq!(bars_of(&log, 60), vec![0, 2, 4, 6], "1:2");
    assert_eq!(bars_of(&log, 61), vec![1, 3, 5, 7], "2:2");
    assert_eq!(bars_of(&log, 62), vec![2, 6], "3:4");
    assert_eq!(bars_of(&log, 63), vec![0], "1ST");
    assert_eq!(bars_of(&log, 64), vec![1, 2, 3, 4, 5, 6, 7], "!1ST");
    assert_eq!(player.stats().unmet, 40 - 4 - 4 - 2 - 1 - 7);
    assert_eq!(player.stats().skipped, 0);
}

#[test]
fn passes_before_the_start_count_backwards() {
    // Launched on bar 3, playing wherever the song goes: bar 1 is pass -1.
    let mut s = Session::new(vec![passes_clip()]);
    s.launches[0].origin = anchored(2 * BAR, BAR, Speed::NORMAL);
    s.launches[0].start = 2 * BAR;
    let (log, _) = run(&s, 128, 6 * BAR_FRAMES);
    assert_eq!(bars_of(&log, 60), vec![0, 2, 4], "1:2 on passes -1, 1, 3");
    assert_eq!(bars_of(&log, 61), vec![1, 3, 5], "2:2 on passes 0, 2, 4");
    assert_eq!(bars_of(&log, 62), vec![0, 4], "3:4 on passes -1, 3");
    assert_eq!(bars_of(&log, 63), vec![2], "1ST");
    assert_eq!(bars_of(&log, 64), vec![0, 1, 3, 4, 5], "!1ST");
}

#[test]
fn a_pass_is_one_pass_at_the_clips_own_speed() {
    // A bar of clip at half speed: each pass is two bars of song.
    let mut c = passes_clip();
    c.speed = Speed::new(1, 2).unwrap();
    let s = Session::new(vec![c]);
    let (log, _) = run(&s, 128, 8 * BAR_FRAMES);
    assert_eq!(bars_of(&log, 60), vec![0, 4], "1:2 on passes 1 and 3");
    assert_eq!(bars_of(&log, 63), vec![1], "1ST, half way through pass 1");
}

#[test]
fn a_locate_finds_the_pass_by_song_position() {
    let s = Session {
        commands: vec![(BAR_FRAMES / 2, Command::Locate(4 * BAR))],
        ..Session::new(vec![clip(
            BAR,
            &[
                note(0, 60, Cond::Every { a: 1, b: 4 }),
                note(0, 61, Cond::First),
            ],
        )])
    };
    let l = BAR_FRAMES / 2;
    let (log, _) = run(&s, 128, l + 4 * BAR_FRAMES + 1000);
    // Pass 1, then from bar 5 (pass 5) on the locate's frame, then pass 9.
    assert_eq!(
        ons(&log),
        vec![
            (0, 0, 60),
            (0, 0, 61),
            (l, 0, 60),
            (l + 4 * BAR_FRAMES, 0, 60)
        ]
    );
    // A locate back to the start plays 1ST again.
    let back = Session {
        commands: vec![(BAR_FRAMES + 1000, Command::Locate(0))],
        ..s.clone()
    };
    let (log, _) = run(&back, 128, 2 * BAR_FRAMES);
    assert_eq!(frames_of(&log, 61), vec![0, BAR_FRAMES + 1000]);
}

#[test]
fn a_transport_loop_plays_the_same_passes_each_time() {
    let s = Session {
        commands: vec![(0, Command::SetLoop(Some((0, 2 * BAR))))],
        ..Session::new(vec![clip(
            BAR,
            &[
                note(0, 60, Cond::Every { a: 1, b: 2 }),
                note(STEP, 61, Cond::Every { a: 2, b: 2 }),
            ],
        )])
    };
    let (log, _) = run(&s, 128, 6 * BAR_FRAMES);
    assert_eq!(bars_of(&log, 60), vec![0, 2, 4]);
    assert_eq!(bars_of(&log, 61), vec![1, 3, 5]);
}

#[test]
fn a_relaunch_counts_from_its_new_start() {
    let s = Session {
        // Cued in bar 3, so it starts again on bar 4.
        cues: vec![(
            2 * BAR_FRAMES + 1000,
            Cue {
                track: 0,
                slot: Some(0),
            },
        )],
        ..Session::new(vec![clip(
            BAR,
            &[
                note(0, 60, Cond::First),
                note(0, 61, Cond::Every { a: 1, b: 2 }),
            ],
        )])
    };
    let (log, _) = run(&s, 128, 7 * BAR_FRAMES);
    assert_eq!(bars_of(&log, 60), vec![0, 3]);
    // Passes 1 and 3, then 1 and 3 again from bar 4.
    assert_eq!(bars_of(&log, 61), vec![0, 2, 3, 5]);
}

#[test]
fn fill_switches_on_an_exact_frame() {
    let notes: Vec<Note> = (0..16)
        .flat_map(|k| {
            [
                note(k * STEP, 60, Cond::Fill),
                note(k * STEP, 61, Cond::NotFill),
            ]
        })
        .collect();
    let s = Session::new(vec![clip(BAR, &notes)]);
    let (log, player) = run(&s, 128, BAR_FRAMES);
    let steps = frames_of(&log, 61);
    assert_eq!(steps.len(), 16);
    assert!(!player.fill());
    // On exactly on step 5's frame, off just after step 11's.
    let filled = Session {
        fills: vec![(steps[5], true), (steps[11] + 1, false)],
        ..s.clone()
    };
    for period in [64, 128, 1000] {
        let (log, player) = run(&filled, period, BAR_FRAMES);
        assert_eq!(frames_of(&log, 60), steps[5..=11].to_vec(), "{period}");
        let mut plain = steps[..5].to_vec();
        plain.extend_from_slice(&steps[12..]);
        assert_eq!(frames_of(&log, 61), plain, "{period}");
        assert!(!player.fill());
    }
    let held = Session {
        fills: vec![(steps[3], true)],
        ..s.clone()
    };
    let (_, player) = run(&held, 128, BAR_FRAMES);
    assert!(player.fill());
}

#[test]
fn pre_and_nei_follow_the_last_conditional_note() {
    let left = clip(
        BAR,
        &[
            note(0, 60, Cond::Every { a: 1, b: 2 }),
            // PRE and !PRE notes read the 1:2 note's result; they do not
            // set it, so the !PRE note still follows the 1:2 note.
            note(4 * STEP, 61, Cond::Pre),
            note(8 * STEP, 62, Cond::NotPre),
            // Not conditional: it changes nothing.
            note(10 * STEP, 63, Cond::Always),
            // The first track has no neighbour.
            note(12 * STEP, 64, Cond::Nei),
            note(12 * STEP, 65, Cond::NotNei),
        ],
    );
    let right = clip(
        BAR,
        &[
            // On the same tick as the 1:2 note, which is decided first.
            note(0, 70, Cond::Nei),
            note(4 * STEP, 71, Cond::NotNei),
            note(6 * STEP, 72, Cond::Pre),
        ],
    );
    let s = Session::new(vec![left, right]);
    let (log, _) = run(&s, 128, 4 * BAR_FRAMES);
    for bar in 0..4 {
        let played: Vec<u8> = by_bar(&log)
            .into_iter()
            .filter(|x| x.0 == bar)
            .map(|x| x.1)
            .collect();
        let want = if bar % 2 == 0 {
            vec![60, 70, 61, 63, 65]
        } else {
            vec![71, 62, 63, 65]
        };
        assert_eq!(played, want, "bar {}", bar + 1);
    }
}

#[test]
fn chance_sets_the_last_result_too() {
    let mut maybe = note(0, 60, Cond::Always);
    maybe.chance = 50;
    let s = Session::new(vec![clip(
        BAR,
        &[
            maybe,
            note(STEP, 61, Cond::Pre),
            note(STEP, 62, Cond::NotPre),
        ],
    )]);
    let (log, player) = run(&s, 128, 32 * BAR_FRAMES);
    let rolled = bars_of(&log, 60);
    assert!(
        (8..=24).contains(&rolled.len()),
        "{} of 32 at 50 %",
        rolled.len()
    );
    assert_eq!(bars_of(&log, 61), rolled);
    assert_eq!(player.stats().skipped, 32 - rolled.len() as u64);
    assert_eq!(player.stats().unmet, 32);
}

#[test]
fn a_locate_resets_pre_and_a_loop_jump_keeps_it() {
    // 2:2 plays on bar 2; PRE on bar 1's step 9 follows it.
    let c = clip(
        BAR,
        &[
            note(0, 60, Cond::Every { a: 2, b: 2 }),
            note(8 * STEP, 61, Cond::Pre),
        ],
    );
    // Loop from mid bar 1 to mid bar 2: after each jump, PRE follows the
    // 2:2 note that just played.
    let looped = Session {
        commands: vec![(0, Command::SetLoop(Some((BAR / 4, BAR + BAR / 4))))],
        ..Session::new(vec![c.clone()])
    };
    let (log, _) = run(&looped, 128, 4 * BAR_FRAMES + 1000);
    assert_eq!(frames_of(&log, 60).len(), 4);
    assert_eq!(frames_of(&log, 61).len(), 3, "every pass but the first");
    // A locate to the same place forgets it.
    let at = BAR_FRAMES + BAR_FRAMES / 4;
    let located = Session {
        commands: vec![(at, Command::Locate(BAR / 4))],
        ..Session::new(vec![c])
    };
    let (log, _) = run(&located, 128, at + BAR_FRAMES / 2);
    assert_eq!(frames_of(&log, 60), vec![BAR_FRAMES]);
    assert_eq!(frames_of(&log, 61), vec![]);
}

/// Note-ons and note-offs as (frame, on).
fn edges(log: &[Out]) -> Vec<(u64, bool, u8)> {
    log.iter()
        .filter_map(|o| match o.event {
            Event::NoteOn { note, .. } => Some((o.frame, true, note)),
            Event::NoteOff { note } => Some((o.frame, false, note)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_ratchet_plays_its_hits_spread_over_the_note() {
    let mut r = Note::new(0, STEP, 60, 100);
    r.ratchet = 4;
    let ratchet = Session::new(vec![clip(BAR, &[r])]);
    let plain = Session::new(vec![clip(
        BAR,
        &[0, 60, 120, 180].map(|t| Note::new(t, 60, 60, 100)),
    )]);
    for period in [64, 128, 1000] {
        let (a, pa) = run(&ratchet, period, 2 * BAR_FRAMES);
        let (b, _) = run(&plain, period, 2 * BAR_FRAMES);
        assert_eq!(edges(&a), edges(&b), "{period}");
        assert_eq!(pa.stats().notes, 8);
    }
}

#[test]
fn swing_and_quantise_move_the_first_hit_and_the_rest_keep_their_spacing() {
    let swing = Swing::new(240, 66).unwrap();
    let swung = swing.apply(STEP as f64).round() as i64;
    let mut r = Note::new(STEP + 7, 3 * STEP, 60, 100);
    r.ratchet = 3;
    let mut c = clip(BAR, &[r]);
    c.swing = Some(swing);
    c.quantise = Quantise::new(240, 100);
    let ratchet = Session::new(vec![c]);
    let plain = Session::new(vec![clip(
        BAR,
        &[0, 240, 480].map(|t| Note::new(swung + t, 240, 60, 100)),
    )]);
    let (a, _) = run(&ratchet, 128, BAR_FRAMES);
    let (b, _) = run(&plain, 128, BAR_FRAMES);
    assert_eq!(edges(&a), edges(&b));
    // At double speed the note is half as long, and so are its hits.
    let mut c = clip(BAR, &[r]);
    c.speed = Speed::new(2, 1).unwrap();
    let fast = Session::new(vec![c]);
    // 247 clip ticks is 123.5 song ticks, so the first hit is on 124.
    let plain = Session::new(vec![clip(
        BAR,
        &[124, 244, 364, 2044, 2164, 2284].map(|t| Note::new(t, 120, 60, 100)),
    )]);
    let (a, _) = run(&fast, 128, BAR_FRAMES);
    let (b, _) = run(&plain, 128, BAR_FRAMES);
    assert_eq!(edges(&a), edges(&b));
}

#[test]
fn a_ratchet_plays_whole_or_not_at_all() {
    let mut r = Note::new(STEP, 2 * STEP, 60, 100);
    r.ratchet = 8;
    r.chance = 50;
    // Short periods, so the hits of one note fall in several.
    let s = Session::new(vec![clip(BAR, &[r])]);
    let (log, player) = run(&s, 64, 40 * BAR_FRAMES);
    let mut per_bar = [0; 40];
    for (bar, _) in by_bar(&log) {
        per_bar[bar as usize] += 1;
    }
    assert!(per_bar.iter().all(|&n| n == 0 || n == 8), "{per_bar:?}");
    let played = per_bar.iter().filter(|&&n| n == 8).count() as u64;
    assert!((10..=30).contains(&played), "{played} of 40 at 50 %");
    assert_eq!(player.stats().skipped, 40 - played);
}

#[test]
fn a_ratchet_stops_at_a_switch_and_a_locate() {
    // A ratchet over the whole last beat of bar 1.
    let mut r = Note::new(3 * BAR / 4, BAR / 4, 60, 100);
    r.ratchet = 4;
    let mut s = Session::new(vec![clip(BAR, &[r])]);
    s.project.tracks[0].slots[1] = Some(clip(BAR, &[]));
    // The empty clip is cued before bar 2: two hits play, two do not.
    s.cues = vec![(
        3 * BAR_FRAMES / 4 + 1000,
        Cue {
            track: 0,
            slot: Some(1),
        },
    )];
    let (log, _) = run(&s, 128, 2 * BAR_FRAMES);
    assert_eq!(
        frames_of(&log, 60).len(),
        4,
        "a switch on bar 2 cuts nothing in bar 1"
    );
    // Half a bar long, from the last beat of bar 1: hits on 2880, 3360,
    // 3840 and 4320. A switch on bar 2 (3840) drops the last two.
    let mut over = s.clone();
    over.project.tracks[0].slots[0] = Some({
        let mut r = Note::new(3 * BAR / 4, BAR / 2, 60, 100);
        r.ratchet = 4;
        clip(BAR, &[r])
    });
    over.cues = vec![];
    let (log, _) = run(&over, 128, 2 * BAR_FRAMES);
    assert_eq!(frames_of(&log, 60).len(), 6, "without the switch");
    over.cues = s.cues.clone();
    over.cues[0].0 = 1000;
    let (log, player) = run(&over, 128, 2 * BAR_FRAMES);
    assert_eq!(frames_of(&log, 60).len(), 2, "cut on the switch");
    assert_eq!(player.sounding(), 0);
    // A locate during the hits drops the rest.
    let located = Session {
        commands: vec![(BAR_FRAMES * 3 / 4 + 3000, Command::Locate(2 * BAR))],
        cues: vec![],
        ..s.clone()
    };
    let (log, player) = run(&located, 128, BAR_FRAMES + 1000);
    assert_eq!(frames_of(&log, 60).len(), 1);
    assert_eq!(player.sounding(), 0);
}

/// Locks and note-ons in order, without frames.
#[derive(Clone, Copy, Debug, PartialEq)]
enum What {
    On(u8),
    Lock(f32),
    Unlock,
}

fn locks_and_ons(log: &[Out]) -> Vec<What> {
    log.iter()
        .filter_map(|o| match o.event {
            Event::NoteOn { note, .. } => Some(What::On(note)),
            Event::Lock { value, .. } => Some(What::Lock(value)),
            Event::Unlock { .. } => Some(What::Unlock),
            _ => None,
        })
        .collect()
}

#[test]
fn a_note_left_out_leaves_out_its_own_locks() {
    use What::{Lock as L, On, Unlock as U};
    let mut c = clip(
        BAR,
        &[
            note(0, 60, Cond::Always),
            note(4 * STEP, 61, Cond::Every { a: 1, b: 2 }),
            note(12 * STEP, 62, Cond::Always),
        ],
    );
    c.locks = vec![
        Lock::new(0, None, CUT, 1.0),
        // On the 1:2 note's row: only on the passes it plays.
        Lock::new(4 * STEP, Some(61), CUT, 2.0),
        // Trigless: always.
        Lock::new(8 * STEP, None, CUT, 3.0),
    ];
    let s = Session::new(vec![c]);
    let (log, _) = run(&s, 128, 2 * BAR_FRAMES);
    assert_eq!(
        locks_and_ons(&log),
        vec![
            L(1.0),
            On(60),
            L(2.0),
            On(61),
            L(3.0),
            U,
            On(62),
            // Pass 2: the 1:2 note and its lock are left out, and it ends
            // nothing, so the first lock holds until the trigless one.
            L(1.0),
            On(60),
            L(3.0),
            U,
            On(62),
        ]
    );
}

#[test]
fn a_whole_step_lock_on_a_chord_goes_only_with_every_note() {
    use What::{Lock as L, On};
    let mut c = clip(
        BAR,
        &[
            // Step 1: one note of the chord plays on even passes.
            note(0, 60, Cond::Every { a: 1, b: 2 }),
            note(0, 64, Cond::Always),
            // Step 9: neither note plays on pass 2.
            note(8 * STEP, 60, Cond::Every { a: 1, b: 2 }),
            note(8 * STEP, 64, Cond::First),
        ],
    );
    c.locks = vec![
        Lock::new(0, None, CUT, 1.0),
        Lock::new(8 * STEP, None, CUT, 2.0),
    ];
    let s = Session::new(vec![c]);
    let (log, _) = run(&s, 128, 2 * BAR_FRAMES);
    assert_eq!(
        locks_and_ons(&log),
        vec![
            L(1.0),
            On(60),
            On(64),
            L(2.0),
            On(60),
            On(64),
            // Pass 2: the chord's lock still plays on step 1, and step 9's
            // lock goes with both its notes.
            L(1.0),
            On(64),
        ]
    );
}

/// A busy session: four tracks with every condition, chance, ratchets,
/// locks, FILL switched mid-bar, a transport loop, a locate and a cue.
fn busy() -> Session {
    let conds = [
        Cond::Always,
        Cond::Every { a: 1, b: 2 },
        Cond::Every { a: 3, b: 3 },
        Cond::Fill,
        Cond::NotFill,
        Cond::Pre,
        Cond::NotPre,
        Cond::Nei,
        Cond::NotNei,
        Cond::First,
        Cond::NotFirst,
    ];
    let mut x = 12345u64;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut clips = vec![];
    for t in 0..4 {
        let mut c = Clip::new("c", BAR * (1 + t % 2));
        for k in 0..(16 * (1 + t % 2)) {
            let mut n = Note::new(
                k * STEP + (rnd() % 30) as i64,
                60 + (rnd() % 400) as i64,
                (rnd() % 8) as u8,
                100,
            );
            n.cond = conds[(rnd() % conds.len() as u64) as usize];
            n.chance = [100, 100, 70][(rnd() % 3) as usize];
            n.ratchet = 1 + (rnd() % 4) as u8;
            if rnd() % 3 == 0 {
                c.locks.push(Lock::new(n.tick, Some(n.note), CUT, k as f32));
            }
            c.notes.push(n);
        }
        if t == 1 {
            c.swing = Swing::new(240, 62);
        }
        if t == 2 {
            c.quantise = Quantise::new(240, 60);
            c.speed = Speed::new(3, 2).unwrap();
        }
        clips.push(c);
    }
    let mut s = Session::new(clips);
    s.project.tracks[3].slots[1] = Some(s.project.tracks[3].slots[0].clone().unwrap());
    s.commands = vec![
        (0, Command::SetLoop(Some((0, 6 * BAR)))),
        (9 * BAR_FRAMES + 12_345, Command::Locate(3 * BAR + 77)),
    ];
    s.fills = vec![
        (BAR_FRAMES + 30_001, true),
        (2 * BAR_FRAMES + 5, false),
        (7 * BAR_FRAMES + 1, true),
        (7 * BAR_FRAMES + 900, false),
    ];
    s.cues = vec![(
        4 * BAR_FRAMES + 2_000,
        Cue {
            track: 3,
            slot: Some(1),
        },
    )];
    s
}

#[test]
fn the_same_at_every_period_size() {
    let s = busy();
    // The offset in the period differs; the frame must not.
    let run = |period| {
        let (log, p) = run(&s, period, 14 * BAR_FRAMES);
        let log: Vec<_> = log
            .into_iter()
            .map(|o| (o.frame, o.track, o.dest, o.event))
            .collect();
        (log, p)
    };
    let (want, p) = run(128);
    let stats = p.stats();
    assert!(want.len() > 2000, "{} events", want.len());
    assert!(
        stats.unmet > 100 && stats.skipped > 20 && stats.locks > 50,
        "{stats:?}"
    );
    for period in [1, 63, 64, 100, 256, 512, 1000, 1024] {
        let (got, q) = run(period);
        assert_eq!(got.len(), want.len(), "{period}");
        if let Some(i) = got.iter().zip(&want).position(|(a, b)| a != b) {
            panic!(
                "{period}: event {i}: {:?} against {:?}",
                &got[i..(i + 4).min(got.len())],
                &want[i..(i + 4).min(want.len())]
            );
        }
        assert_eq!(q.stats(), stats, "{period}");
    }
}
