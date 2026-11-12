//! Cues: a launch or a stop made while the music plays waits for the next
//! bar line the playhead plays. The old clip plays up to it, its held notes
//! end on it, and the new clip starts there from its top, then keeps its
//! place against the song (across loop jumps too). Notes recorded after
//! the switch go into the new clip.
//!
//! Every session runs at several period sizes, with the control thread
//! applying each started cue at once, a few periods later, much later or
//! never. All of them must give the same events.

use mpfree_play::{
    anchored, Cue, Erased, Event, Launch, Out, PlayView, Player, Recorder, Sink, Started, Taken,
};
use mpfree_seq::{Clip, Note, Project, Speed, Track};
use mpfree_transport::{Block, Command, Transport};

const BAR: i64 = 3840;
/// 120 BPM at 48 kHz: 25 frames a tick.
const RATE: u32 = 48_000;
const TICK: u64 = 25;

fn at(tick: i64) -> u64 {
    tick as u64 * TICK
}

#[derive(Default)]
struct Log(Vec<(u64, u16, Event)>);
impl Sink for Log {
    fn send(&mut self, o: Out) {
        self.0.push((o.frame, o.track, o.event));
    }
}

#[derive(Clone)]
struct Session {
    project: Project,
    launches: Vec<Launch>,
    armed: Option<usize>,
    commands: Vec<(u64, Command)>,
    /// The control thread queues each cue in the first period that starts
    /// at or after its frame.
    cues: Vec<(u64, Cue)>,
    live: Vec<(u64, Event)>,
    frames: u64,
}

#[derive(Debug, PartialEq)]
struct Run {
    played: Vec<(u64, u16, Event)>,
    started: Vec<Started>,
    taken: Vec<Taken>,
    erased: Vec<Erased>,
    launches: Vec<Launch>,
    cues: Vec<Cue>,
}

/// Run `s` in periods of `period` frames. The control thread applies a
/// started cue `apply` periods after the one it started in (None: never).
fn run(s: &Session, period: u32, apply: Option<usize>) -> Run {
    let mut t = Transport::new(RATE, 120.0).unwrap();
    for &(f, c) in &s.commands {
        assert!(t.command(f, c));
    }
    let compile = |l: &[Launch], c: &[Cue]| {
        let mut v = PlayView::compile_cued(&s.project, l, c).unwrap();
        v.set_armed(s.armed);
        v
    };
    let (mut launches, mut cues) = (s.launches.clone(), Vec::new());
    let mut view = compile(&launches, &cues);
    let (mut player, mut rec, mut block, mut log) =
        (Player::new(), Recorder::new(), Block::new(), Log::default());
    let (mut started, mut taken, mut erased) = (vec![], vec![], vec![]);
    let mut waiting: Vec<(usize, Started)> = vec![];
    let (mut next_cue, mut next_live) = (0, 0);
    while t.frame() < s.frames {
        let f = t.frame();
        let n = period.min((s.frames - f) as u32);
        let mut changed = false;
        for w in &mut waiting {
            if w.0 == 0 {
                w.1.apply(&mut launches, &mut cues);
                changed = true;
            }
            w.0 = w.0.wrapping_sub(1);
        }
        waiting.retain(|w| w.0 != usize::MAX);
        while next_cue < s.cues.len() && s.cues[next_cue].0 <= f {
            let c = s.cues[next_cue].1;
            cues.retain(|x| x.track != c.track);
            cues.push(c);
            next_cue += 1;
            changed = true;
        }
        if changed {
            view = compile(&launches, &cues);
        }
        while next_live < s.live.len() && s.live[next_live].0 < f + u64::from(n) {
            rec.input((s.live[next_live].0 - f) as u32, s.live[next_live].1);
            next_live += 1;
        }
        assert!(t.process(n, &mut block));
        player.process(&view, &block, &mut log);
        rec.process(&view, &block);
        for st in player.started() {
            started.push(*st);
            if let Some(d) = apply {
                waiting.push((d, *st));
            }
        }
        taken.extend_from_slice(rec.taken());
        erased.extend_from_slice(player.erased());
    }
    assert_eq!(
        player.stats().dropped + player.stats().late_offs + rec.dropped(),
        0
    );
    Run {
        played: log.0,
        started,
        taken,
        erased,
        launches,
        cues,
    }
}

/// The run at every period size and every way of applying started cues,
/// which must all agree (but for the launches and cues left at the end).
fn same_everywhere(s: &Session) -> Run {
    let first = run(s, 128, Some(0));
    for period in [1, 64, 1000, 4096] {
        for apply in [Some(0), Some(3), Some(40), None] {
            let r = run(s, period, apply);
            let what = format!("period {period}, applied {apply:?}");
            assert_eq!(r.played, first.played, "{what}");
            assert_eq!(r.started, first.started, "{what}");
            assert_eq!(r.taken, first.taken, "{what}");
            assert_eq!(r.erased, first.erased, "{what}");
            if apply == Some(0) {
                assert_eq!(
                    (&r.launches, &r.cues),
                    (&first.launches, &first.cues),
                    "{what}"
                );
            }
        }
    }
    first
}

fn note(tick: i64, len: i64, n: u8) -> Note {
    Note::new(tick, len, n, 100)
}

/// Track 0: slot 0 holds A, a bar of beats (note 1), playing from 0; slot 1
/// holds B, two bars with note 20 on its top and note 21 on the off-beat of
/// its second bar.
fn session(frames: u64) -> Session {
    let mut p = Project::new();
    let mut t = Track::new("drums", 0);
    let mut a = Clip::new("A", BAR);
    a.notes = (0..4).map(|i| note(i * 960, 100, 1)).collect();
    let mut b = Clip::new("B", 2 * BAR);
    b.notes = vec![note(0, 100, 20), note(BAR + 480, 100, 21)];
    t.slots[0] = Some(a);
    t.slots[1] = Some(b);
    p.tracks.push(t);
    Session {
        project: p,
        launches: vec![Launch {
            track: 0,
            slot: 0,
            origin: 0,
            start: 0,
        }],
        armed: None,
        commands: vec![(0, Command::Play)],
        cues: vec![],
        live: vec![],
        frames,
    }
}

/// Note-ons as (song tick, note), for a run without jumps.
fn ons(r: &Run) -> Vec<(i64, u8)> {
    r.played
        .iter()
        .filter_map(|&(f, _, e)| match e {
            Event::NoteOn { note, .. } => Some(((f / TICK) as i64, note)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_launch_waits_for_the_bar_line_and_starts_the_clip_from_its_top() {
    let mut s = session(at(5 * BAR));
    s.cues = vec![(
        at(1500),
        Cue {
            track: 0,
            slot: Some(1),
        },
    )];
    let r = same_everywhere(&s);
    assert_eq!(
        ons(&r),
        vec![
            (0, 1),
            (960, 1),
            (1920, 1),
            (2880, 1),
            (BAR, 20),
            (2 * BAR + 480, 21),
            (3 * BAR, 20),
            (4 * BAR + 480, 21)
        ]
    );
    let origin = anchored(BAR, 2 * BAR, Speed::NORMAL);
    assert_eq!(
        r.started,
        vec![Started {
            track: 0,
            slot: Some(1),
            at: BAR,
            origin
        }]
    );
    assert_eq!(
        (r.launches, r.cues),
        (
            vec![Launch {
                track: 0,
                slot: 1,
                origin,
                start: BAR,
            }],
            vec![]
        )
    );
}

#[test]
fn a_note_held_over_the_bar_line_ends_on_it() {
    let mut s = session(at(2 * BAR));
    s.project.tracks[0].slots[0].as_mut().unwrap().notes = vec![note(2000, 6000, 7)];
    s.cues = vec![(
        at(1500),
        Cue {
            track: 0,
            slot: Some(1),
        },
    )];
    let r = same_everywhere(&s);
    let around: Vec<_> = r
        .played
        .iter()
        .filter(|p| p.0 >= at(2000))
        .copied()
        .collect();
    assert_eq!(
        around,
        vec![
            (
                at(2000),
                0,
                Event::NoteOn {
                    note: 7,
                    velocity: 100
                }
            ),
            (at(BAR), 0, Event::NoteOff { note: 7 }),
            (
                at(BAR),
                0,
                Event::NoteOn {
                    note: 20,
                    velocity: 100
                }
            ),
            (at(BAR + 100), 0, Event::NoteOff { note: 20 }),
        ]
    );
}

#[test]
fn a_stop_waits_for_the_bar_line_too() {
    let mut s = session(at(3 * BAR));
    s.project.tracks[0].slots[0]
        .as_mut()
        .unwrap()
        .notes
        .push(note(3000, 2000, 9));
    s.cues = vec![(
        at(1500),
        Cue {
            track: 0,
            slot: None,
        },
    )];
    let r = same_everywhere(&s);
    assert_eq!(
        ons(&r),
        vec![(0, 1), (960, 1), (1920, 1), (2880, 1), (3000, 9)]
    );
    assert_eq!(
        r.played.last(),
        Some(&(at(BAR), 0, Event::NoteOff { note: 9 })),
        "the held note ends on the bar line"
    );
    assert_eq!(
        r.started,
        vec![Started {
            track: 0,
            slot: None,
            at: BAR,
            origin: BAR
        }]
    );
    assert_eq!((r.launches, r.cues), (vec![], vec![]));
}

#[test]
fn cues_on_two_tracks_start_on_the_same_bar_line() {
    let mut s = session(at(2 * BAR));
    let mut t = s.project.tracks[0].clone();
    t.name = "keys".into();
    t.dest = 1;
    s.project.tracks.push(t);
    s.launches.push(Launch {
        track: 1,
        slot: 1,
        origin: 0,
        start: 0,
    });
    s.cues = vec![
        (
            at(700),
            Cue {
                track: 1,
                slot: Some(0),
            },
        ),
        (
            at(2500),
            Cue {
                track: 0,
                slot: Some(1),
            },
        ),
    ];
    let r = same_everywhere(&s);
    let starts: Vec<_> = r.started.iter().map(|x| (x.track, x.slot, x.at)).collect();
    assert_eq!(starts, vec![(0, Some(1), BAR), (1, Some(0), BAR)]);
    let bar2: Vec<_> = r
        .played
        .iter()
        .filter(|p| p.0 == at(BAR) && matches!(p.2, Event::NoteOn { .. }))
        .map(|p| (p.1, p.2))
        .collect();
    assert_eq!(
        bar2,
        vec![
            (
                0,
                Event::NoteOn {
                    note: 20,
                    velocity: 100
                }
            ),
            (
                1,
                Event::NoteOn {
                    note: 1,
                    velocity: 100
                }
            )
        ]
    );
}

#[test]
fn in_a_loop_the_next_bar_line_can_be_the_loop_start() {
    // A two-bar loop. Cued in its second bar, B waits for the loop to jump
    // back: the start is the next bar line the playhead plays.
    let loop_frames = at(2 * BAR);
    let mut s = session(4 * loop_frames);
    s.commands
        .insert(0, (0, Command::SetLoop(Some((0, 2 * BAR)))));
    s.cues = vec![(
        at(5000),
        Cue {
            track: 0,
            slot: Some(1),
        },
    )];
    let r = same_everywhere(&s);
    let ons: Vec<(u64, u8)> = r
        .played
        .iter()
        .filter_map(|&(f, _, e)| match e {
            Event::NoteOn { note, .. } => Some((f, note)),
            _ => None,
        })
        .collect();
    let mut want: Vec<(u64, u8)> = (0..8).map(|i| (at(i * 960), 1)).collect();
    for pass in 1..4 {
        want.push((pass * loop_frames, 20));
        want.push((pass * loop_frames + at(BAR + 480), 21));
    }
    assert_eq!(ons, want);
    assert_eq!(r.started.iter().map(|x| x.at).collect::<Vec<_>>(), vec![0]);
}

#[test]
fn a_clip_started_in_a_loop_keeps_its_place_after_the_jump_back() {
    // Cued in the loop's first bar, B starts on bar 2. After the jump back
    // it does not wait for bar 2 again: song tick 0 is its second bar.
    let loop_frames = at(2 * BAR);
    let mut s = session(3 * loop_frames);
    s.commands
        .insert(0, (0, Command::SetLoop(Some((0, 2 * BAR)))));
    s.cues = vec![(
        at(1500),
        Cue {
            track: 0,
            slot: Some(1),
        },
    )];
    let r = same_everywhere(&s);
    let ons: Vec<(u64, u8)> = r
        .played
        .iter()
        .filter_map(|&(f, _, e)| match e {
            Event::NoteOn { note, .. } => Some((f, note)),
            _ => None,
        })
        .collect();
    let mut want: Vec<(u64, u8)> = (0..4).map(|i| (at(i * 960), 1)).collect();
    want.push((at(BAR), 20));
    for pass in 1..3 {
        want.push((pass * loop_frames + at(480), 21));
        want.push((pass * loop_frames + at(BAR), 20));
    }
    assert_eq!(ons, want);
}

#[test]
fn a_cue_made_while_stopped_waits_for_the_first_bar_line_played() {
    let mut s = session(at(3 * BAR));
    // Play from the middle of bar 1, then cue at once: it starts on bar 2.
    s.commands = vec![(0, Command::Locate(2000)), (0, Command::Play)];
    s.cues = vec![(
        0,
        Cue {
            track: 0,
            slot: Some(1),
        },
    )];
    let r = same_everywhere(&s);
    assert_eq!(&ons(&r)[..2], &[(2880 - 2000, 1), (BAR - 2000, 20)]);
    assert_eq!(r.started[0].at, BAR);
}

#[test]
fn notes_played_after_the_switch_go_into_the_new_clip() {
    let mut s = session(at(3 * BAR));
    s.armed = Some(0);
    s.commands = vec![(0, Command::ToggleOverdub), (0, Command::Record)];
    s.cues = vec![(
        at(1500),
        Cue {
            track: 0,
            slot: Some(1),
        },
    )];
    // Note 60 is held over the bar line; 62 comes just after it, in the
    // same period for the larger sizes, and 61 later.
    s.live = vec![
        (
            at(3000),
            Event::NoteOn {
                note: 60,
                velocity: 90,
            },
        ),
        (
            at(BAR + 10),
            Event::NoteOn {
                note: 62,
                velocity: 70,
            },
        ),
        (at(BAR + 50), Event::NoteOff { note: 62 }),
        (at(4500), Event::NoteOff { note: 60 }),
        (
            at(5000),
            Event::NoteOn {
                note: 61,
                velocity: 80,
            },
        ),
        (at(5200), Event::NoteOff { note: 61 }),
    ];
    let r = same_everywhere(&s);
    let mut got: Vec<_> = r
        .taken
        .iter()
        .map(|k| (k.slot, k.note.tick, k.note.len, k.note.note))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            (0, 3000, BAR - 3000, 60),
            (1, 10, 40, 62),
            (1, 5000 - BAR, 200, 61)
        ]
    );
}

#[test]
fn a_switch_on_the_armed_track_ends_the_replace_pass() {
    // A is two bars of beats. Recording over it with overdub off erases
    // its first bar; at the switch the take goes on as overdub, so B plays.
    let mut s = session(at(3 * BAR));
    s.project.tracks[0].slots[0].as_mut().unwrap().length = 2 * BAR;
    s.project.tracks[0].slots[0].as_mut().unwrap().notes =
        (0..8).map(|i| note(i * 960, 100, 1)).collect();
    s.armed = Some(0);
    s.commands = vec![(0, Command::Record)];
    s.cues = vec![(
        at(1500),
        Cue {
            track: 0,
            slot: Some(1),
        },
    )];
    let r = same_everywhere(&s);
    assert_eq!(
        r.erased
            .iter()
            .map(|e| (e.slot, e.note.tick))
            .collect::<Vec<_>>(),
        vec![(0, 0), (0, 960), (0, 1920), (0, 2880)]
    );
    assert_eq!(ons(&r), vec![(BAR, 20), (2 * BAR + 480, 21)]);
}
