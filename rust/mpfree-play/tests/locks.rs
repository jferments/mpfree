//! Parameter locks, the Elektron way: a lock plays on its step's frame
//! before the note-on, and holds until the track's next note without a
//! lock on that parameter; trigless locks, pad locks, chords, chance,
//! stop, locate, a switch and a mute.

use mpfree_play::{Cue, Event, Launch, Out, PlayView, Player, Sink};
use mpfree_seq::mpfree_param::{OnTrack, Owner, ParamAddr, RelAddr};
use mpfree_seq::{Clip, Lock, Note, Project, Track};
use mpfree_transport::{Block, Command, Transport};

const STEP: i64 = 240;
const BAR: i64 = 3840;
const CUT: RelAddr = RelAddr::new(OnTrack::Inst, 7);
const TUNE: RelAddr = RelAddr::new(OnTrack::Pad(2), 0);

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, out: Out) {
        self.0.push(out);
    }
}

/// What happened, in order: (frame, track, what).
#[derive(Clone, Copy, Debug, PartialEq)]
enum What {
    On(u8),
    Off(u8),
    Lock(ParamAddr, f32),
    Unlock(ParamAddr),
    AllOff,
}

fn what(log: &[Out]) -> Vec<(u64, u16, What)> {
    log.iter()
        .map(|o| {
            let w = match o.event {
                Event::NoteOn { note, .. } => What::On(note),
                Event::NoteOff { note } => What::Off(note),
                Event::Lock { addr, value } => What::Lock(addr, value),
                Event::Unlock { addr } => What::Unlock(addr),
                Event::AllOff => What::AllOff,
            };
            (o.frame, o.track, w)
        })
        .collect()
}

/// Without the note-offs, for reading the order of the rest.
fn ons_and_locks(log: &[Out]) -> Vec<(u64, u16, What)> {
    what(log)
        .into_iter()
        .filter(|e| !matches!(e.2, What::Off(_)))
        .collect()
}

fn clip(length: i64, notes: &[(i64, u8)], locks: &[Lock]) -> Clip {
    let mut c = Clip::new("c", length);
    c.notes = notes
        .iter()
        .map(|&(tick, n)| Note::new(tick, STEP / 2, n, 100))
        .collect();
    c.locks = locks.to_vec();
    c
}

fn project(tracks: usize) -> Project {
    let mut p = Project::new();
    for i in 0..tracks {
        p.tracks.push(Track::new("t", i as u16));
    }
    p
}

fn one(c: Clip) -> PlayView {
    on_track(c, 0)
}

fn on_track(c: Clip, track: usize) -> PlayView {
    let mut p = project(track + 1);
    p.tracks[track].slots[0] = Some(c);
    PlayView::compile(
        &p,
        &[Launch {
            track,
            slot: 0,
            origin: 0,
        }],
    )
    .unwrap()
}

/// Play `frames` frames in periods of 128, with the commands.
fn run(view: &PlayView, cmds: &[(u64, Command)], frames: u64) -> (Vec<Out>, Player) {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    for &(at, c) in cmds {
        assert!(t.command(at, c));
    }
    t.command(0, Command::Play);
    let mut player = Player::new();
    let mut block = Block::new();
    let mut log = Log::default();
    while t.frame() < frames {
        t.process(128.min((frames - t.frame()) as u32), &mut block);
        player.process(view, &block, &mut log);
    }
    assert_eq!(player.stats().dropped, 0);
    (log.0, player)
}

/// Frames in one bar at 120 BPM: 2 s.
const BAR_FRAMES: u64 = 88_200;

fn inst(t: u8, p: u16) -> ParamAddr {
    ParamAddr::new(Owner::Inst(t), p)
}

#[test]
fn a_lock_plays_before_its_note_and_ends_at_the_next_note_without_one() {
    let view = one(clip(
        BAR / 4,
        &[(0, 60), (STEP, 60), (2 * STEP, 60), (3 * STEP, 60)],
        &[
            Lock::new(STEP, None, CUT, 1.0),
            Lock::new(2 * STEP, None, CUT, 2.0),
        ],
    ));
    let (log, player) = run(&view, &[], BAR_FRAMES / 4 + 100);
    let e = ons_and_locks(&log);
    let kinds: Vec<What> = e.iter().map(|x| x.2).collect();
    let cut = inst(0, 7);
    assert_eq!(
        kinds,
        vec![
            What::On(60),
            What::Lock(cut, 1.0),
            What::On(60),
            What::Lock(cut, 2.0),
            What::On(60),
            What::Unlock(cut),
            What::On(60),
            // The next pass: step 0 has nothing to end.
            What::On(60),
        ]
    );
    // Each lock and unlock on its note's frame.
    assert_eq!(e[1].0, e[2].0);
    assert_eq!(e[3].0, e[4].0);
    assert_eq!(e[5].0, e[6].0);
    // The note before ends before the lock comes.
    let lock_at = log
        .iter()
        .position(|o| matches!(o.event, Event::Lock { .. }))
        .unwrap();
    assert!(log[..lock_at]
        .iter()
        .any(|o| matches!(o.event, Event::NoteOff { .. })));
    assert_eq!((player.stats().locks, player.locked()), (2, 0));
}

#[test]
fn a_trigless_lock_holds_until_the_next_note() {
    let view = one(clip(
        BAR / 4,
        &[(0, 60), (2 * STEP, 60)],
        &[Lock::new(STEP, None, CUT, 0.5)],
    ));
    let (log, _) = run(&view, &[], BAR_FRAMES / 4 - 100);
    let e = ons_and_locks(&log);
    let cut = inst(0, 7);
    assert_eq!(
        e.iter().map(|x| x.2).collect::<Vec<_>>(),
        vec![
            What::On(60),
            What::Lock(cut, 0.5),
            What::Unlock(cut),
            What::On(60)
        ]
    );
    assert!(
        e[0].0 < e[1].0 && e[1].0 < e[2].0,
        "on its own step's frame"
    );
    assert_eq!(e[2].0, e[3].0);
}

#[test]
fn a_pad_lock_ends_only_at_that_pads_next_note() {
    // Pads 0 and 2 take turns; pad 2's tune is locked on step 1, and the
    // instrument's cutoff on step 0.
    let view = one(clip(
        BAR / 4,
        &[(0, 0), (STEP, 2), (2 * STEP, 0), (3 * STEP, 2)],
        &[
            Lock::new(0, Some(0), CUT, 3.0),
            Lock::new(STEP, Some(2), TUNE, -5.0),
        ],
    ));
    let (log, _) = run(&view, &[], BAR_FRAMES / 4 - 100);
    let (cut, tune) = (inst(0, 7), ParamAddr::new(Owner::Pad(0, 2), 0));
    assert_eq!(
        ons_and_locks(&log).iter().map(|x| x.2).collect::<Vec<_>>(),
        vec![
            What::Lock(cut, 3.0),
            What::On(0),
            // Pad 2's note ends the instrument's lock, not its own pad's.
            What::Lock(tune, -5.0),
            What::Unlock(cut),
            What::On(2),
            // Pad 0 does not end pad 2's tune.
            What::On(0),
            What::Unlock(tune),
            What::On(2),
        ]
    );
}

#[test]
fn a_chord_keeps_its_lock_for_every_note() {
    let view = one(clip(
        BAR / 4,
        &[(0, 60), (0, 64), (0, 67), (STEP, 60)],
        &[Lock::new(0, None, CUT, 1.5)],
    ));
    let (log, _) = run(&view, &[], BAR_FRAMES / 4 - 100);
    let cut = inst(0, 7);
    assert_eq!(
        ons_and_locks(&log).iter().map(|x| x.2).collect::<Vec<_>>(),
        vec![
            What::Lock(cut, 1.5),
            What::On(60),
            What::On(64),
            What::On(67),
            What::Unlock(cut),
            What::On(60),
        ]
    );
}

#[test]
fn a_note_its_chance_leaves_out_ends_nothing() {
    let mut c = clip(
        BAR / 4,
        &[(0, 60), (STEP, 60), (2 * STEP, 60)],
        &[Lock::new(0, None, CUT, 1.0)],
    );
    c.notes[1].chance = 1;
    let view = one(c);
    let (log, player) = run(&view, &[], BAR_FRAMES / 4 - 100);
    assert_eq!(player.stats().skipped, 1, "the 1 % note rolled out");
    let cut = inst(0, 7);
    assert_eq!(
        ons_and_locks(&log).iter().map(|x| x.2).collect::<Vec<_>>(),
        vec![
            What::Lock(cut, 1.0),
            What::On(60),
            What::Unlock(cut),
            What::On(60)
        ]
    );
}

#[test]
fn stop_and_locate_unlock_everything_after_the_note_offs() {
    let view = one(clip(
        BAR,
        &[(0, 60)],
        &[Lock::new(0, None, CUT, 1.0), Lock::new(0, None, TUNE, 2.0)],
    ));
    for cmd in [Command::Stop, Command::Locate(BAR / 2)] {
        let (log, player) = run(&view, &[(1000, cmd)], 3000);
        let w = what(&log);
        let n = w.len();
        assert_eq!(
            w[n - 3..].iter().map(|x| (x.0, x.2)).collect::<Vec<_>>(),
            vec![
                (1000, What::Off(60)),
                (1000, What::Unlock(inst(0, 7))),
                (1000, What::Unlock(ParamAddr::new(Owner::Pad(0, 2), 0))),
            ],
            "{cmd:?}"
        );
        assert_eq!(player.locked(), 0);
    }
}

#[test]
fn a_switch_ends_the_old_clips_locks_before_the_new_ones() {
    let mut p = project(1);
    p.tracks[0].slots[0] = Some(clip(BAR, &[(0, 60)], &[Lock::new(0, None, CUT, 1.0)]));
    p.tracks[0].slots[1] = Some(clip(BAR, &[(0, 62)], &[Lock::new(0, None, CUT, 9.0)]));
    let launch = [Launch {
        track: 0,
        slot: 0,
        origin: 0,
    }];
    let playing = PlayView::compile(&p, &launch).unwrap();
    // Cued while bar 1 plays: it starts on bar 2.
    let cued = PlayView::compile_cued(
        &p,
        &launch,
        &[Cue {
            track: 0,
            slot: Some(1),
        }],
    )
    .unwrap();
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let (mut player, mut block, mut l) = (Player::new(), Block::new(), Log::default());
    while t.frame() < BAR_FRAMES + 100 {
        t.process(128, &mut block);
        let v = if t.frame() <= 1024 { &playing } else { &cued };
        player.process(v, &block, &mut l);
    }
    let log = l.0;
    let cut = inst(0, 7);
    let e = ons_and_locks(&log);
    assert_eq!(
        e.iter().map(|x| x.2).collect::<Vec<_>>(),
        vec![
            What::Lock(cut, 1.0),
            What::On(60),
            What::Unlock(cut),
            What::Lock(cut, 9.0),
            What::On(62),
        ]
    );
    assert!(e[2].0 == e[3].0 && e[3].0 == e[4].0, "all on the bar line");
}

#[test]
fn a_muted_track_lets_its_locks_go_and_sends_none() {
    let c = clip(BAR, &[(0, 60), (STEP, 60)], &[Lock::new(0, None, CUT, 1.0)]);
    let view = one(c.clone());
    let mut p = project(1);
    p.tracks[0].slots[0] = Some(c);
    p.tracks[0].mute = true;
    let muted = PlayView::compile(
        &p,
        &[Launch {
            track: 0,
            slot: 0,
            origin: 0,
        }],
    )
    .unwrap();
    // Locked on step 0, then muted before step 1.
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    t.process(1024, &mut block);
    player.process(&view, &block, &mut log);
    assert_eq!(player.locked(), 1);
    while t.frame() < BAR_FRAMES * 2 {
        t.process(1024, &mut block);
        player.process(&muted, &block, &mut log);
    }
    let w = ons_and_locks(&log.0);
    assert_eq!(
        w.iter().map(|x| x.2).collect::<Vec<_>>(),
        vec![
            What::Lock(inst(0, 7), 1.0),
            What::On(60),
            What::Unlock(inst(0, 7))
        ]
    );
    // On the first tick of the first period that plays the muted view.
    assert!((1024..1024 + 23).contains(&w[2].0), "{}", w[2].0);
    assert_eq!(player.locked(), 0);
}

#[test]
fn a_clip_locks_the_track_it_plays_on() {
    let view = on_track(clip(BAR, &[(0, 60)], &[Lock::new(0, None, CUT, 1.0)]), 3);
    let (log, _) = run(&view, &[], 1000);
    assert_eq!(ons_and_locks(&log)[0], (0, 3, What::Lock(inst(3, 7), 1.0)));
}
