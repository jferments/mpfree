//! What the player plays: notes on their frames, loops, polymeter, speed,
//! swing, quantise, launch origins, note-offs, jumps, stop and panic.

use mpfree_play::{Event, Launch, Out, PlayView, Player, Sink, ALL};
use mpfree_seq::{Clip, Note, Project, Quantise, Speed, Track};
use mpfree_transport::time::{Segment, Swing, TempoMap};
use mpfree_transport::{Block, Command, Transport};

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, out: Out) {
        self.0.push(out);
    }
}

fn note(tick: i64, len: i64, n: u8) -> Note {
    Note::new(tick, len, n, 100)
}

fn clip(length: i64, notes: &[Note]) -> Clip {
    let mut c = Clip::new("c", length);
    c.notes = notes.to_vec();
    c
}

/// One track per clip, each launched from `origin` 0.
fn view(clips: Vec<Clip>) -> PlayView {
    let mut p = Project::new();
    let mut launches = vec![];
    for (i, c) in clips.into_iter().enumerate() {
        let mut t = Track::new("t", i as u16);
        t.slots[0] = Some(c);
        p.tracks.push(t);
        launches.push(Launch {
            track: i,
            slot: 0,
            origin: 0,
            start: 0,
        });
    }
    PlayView::compile(&p, &launches).unwrap()
}

/// Run `frames` frames in periods of `period`, with the given commands.
fn run(
    view: &PlayView,
    t: &mut Transport,
    cmds: &[(u64, Command)],
    frames: u64,
    period: u32,
) -> Vec<Out> {
    for &(at, c) in cmds {
        assert!(t.command(at, c));
    }
    let mut player = Player::new();
    let mut block = Block::new();
    let mut log = Log::default();
    while t.frame() < frames {
        t.process(period.min((frames - t.frame()) as u32), &mut block);
        player.process(view, &block, &mut log);
    }
    assert_eq!(player.stats().dropped + player.stats().late_offs, 0);
    log.0
}

fn t120() -> Transport {
    Transport::new(44_100, 120.0).unwrap()
}

/// (frame, track, kind, note) with kind 1 = on, 0 = off, 2 = all off.
fn short(log: &[Out]) -> Vec<(u64, u16, u8, u8)> {
    log.iter()
        .map(|o| match o.event {
            Event::NoteOn { note, .. } => (o.frame, o.track, 1, note),
            Event::NoteOff { note } => (o.frame, o.track, 0, note),
            Event::AllOff => (o.frame, o.track, 2, 0),
            Event::Lock { .. } => (o.frame, o.track, 3, 0),
            Event::Unlock { .. } => (o.frame, o.track, 4, 0),
            Event::Lane { .. } => (o.frame, o.track, 5, 0),
            Event::Pressure { note, .. } => (o.frame, o.track, 6, note),
        })
        .collect()
}

fn ons(log: &[Out]) -> Vec<u64> {
    log.iter()
        .filter(|o| matches!(o.event, Event::NoteOn { .. }))
        .map(|o| o.frame)
        .collect()
}

#[test]
fn quarter_notes_land_on_the_beats_and_loop() {
    let v = view(vec![clip(
        3840,
        &[
            note(0, 240, 0),
            note(960, 240, 1),
            note(1920, 240, 2),
            note(2880, 240, 3),
        ],
    )]);
    let log = run(&v, &mut t120(), &[(0, Command::Play)], 100_000, 128);
    assert_eq!(ons(&log), vec![0, 22_050, 44_100, 66_150, 88_200]);
    // 240 ticks at 120 BPM is 5512.5 frames, rounded up.
    assert_eq!(
        short(&log)[..4],
        [
            (0, 0, 1, 0),
            (5_513, 0, 0, 0),
            (22_050, 0, 1, 1),
            (27_563, 0, 0, 1)
        ]
    );
    assert!(log
        .iter()
        .all(|o| o.offset < 128 && o.frame % 128 == u64::from(o.offset)));
}

#[test]
fn clips_of_different_lengths_drift_apart() {
    let v = view(vec![
        clip(3840, &[note(0, 10, 0)]),
        clip(2880, &[note(0, 10, 1)]),
    ]);
    let log = run(&v, &mut t120(), &[(0, Command::Play)], 88_200 * 3, 256);
    let on = |track| {
        log.iter()
            .filter(|o| o.track == track && matches!(o.event, Event::NoteOn { .. }))
            .map(|o| o.frame)
            .collect::<Vec<_>>()
    };
    assert_eq!(on(0), vec![0, 88_200, 176_400]);
    assert_eq!(on(1), vec![0, 66_150, 132_300, 198_450]);
}

#[test]
fn speed_scales_the_clip() {
    let mut fast = clip(960, &[note(0, 10, 0), note(480, 10, 1)]);
    fast.speed = Speed::new(2, 1).unwrap();
    let mut slow = clip(960, &[note(0, 10, 2), note(480, 10, 3)]);
    slow.speed = Speed::new(1, 2).unwrap();
    let v = view(vec![fast, slow]);
    let log = run(&v, &mut t120(), &[(0, Command::Play)], 44_101, 1000);
    let on = |track| {
        log.iter()
            .filter(|o| o.track == track && matches!(o.event, Event::NoteOn { .. }))
            .map(|o| o.frame)
            .collect::<Vec<_>>()
    };
    // Twice as fast: a note every 240 ticks. Half speed: every 960 ticks.
    assert_eq!(
        on(0),
        vec![0, 5_513, 11_025, 16_538, 22_050, 27_563, 33_075, 38_588, 44_100]
    );
    assert_eq!(on(1), vec![0, 22_050, 44_100]);
}

#[test]
fn swing_delays_the_off_beats() {
    let notes: Vec<Note> = (0..4).map(|i| note(i * 240, 10, 0)).collect();
    let v = view(vec![clip(960, &notes)]);
    let mut t = t120();
    t.command(0, Command::SetSwing(Swing::new(240, 66).unwrap()));
    let log = run(&v, &mut t, &[(0, Command::Play)], 22_050, 512);
    // Off-beat 1/16s move from 240 to 316.8, rounded to tick 317.
    let tick = |t: f64| (t * 22.968_75f64).round() as u64;
    assert_eq!(ons(&log), vec![0, tick(317.0), tick(480.0), tick(797.0)]);

    // A clip's own swing wins over the transport's.
    let mut c = clip(960, &notes);
    c.swing = Some(Swing::new(240, 50).unwrap());
    let log = run(
        &view(vec![c]),
        &mut t120(),
        &[
            (0, Command::SetSwing(Swing::new(240, 75).unwrap())),
            (0, Command::Play),
        ],
        22_050,
        512,
    );
    assert_eq!(ons(&log), vec![0, tick(240.0), tick(480.0), tick(720.0)]);
}

#[test]
fn quantise_moves_notes_on_playback_only() {
    let mut c = clip(960, &[note(250, 10, 0), note(490, 10, 1)]);
    c.quantise = Quantise::new(240, 50);
    let v = view(vec![c.clone()]);
    let log = run(&v, &mut t120(), &[(0, Command::Play)], 22_000, 128);
    let tick = |t: f64| (t * 22.968_75f64).round() as u64;
    assert_eq!(ons(&log), vec![tick(245.0), tick(485.0)]);
    assert_eq!(c.notes[0].tick, 250, "the clip keeps its recorded ticks");
}

#[test]
fn a_clip_starts_at_its_origin() {
    let mut p = Project::new();
    let mut t = Track::new("t", 0);
    t.slots[5] = Some(clip(960, &[note(0, 10, 0)]));
    p.tracks.push(t);
    let v = PlayView::compile(
        &p,
        &[Launch {
            track: 0,
            slot: 5,
            origin: 1920,
            start: 1920,
        }],
    )
    .unwrap();
    let log = run(&v, &mut t120(), &[(0, Command::Play)], 88_201, 128);
    assert_eq!(ons(&log), vec![44_100, 66_150, 88_200]);
}

#[test]
fn a_loop_jump_ends_sounding_notes_and_restarts_clips_in_place() {
    // A 3-beat clip in a 2-beat loop: after each jump the clip is back at 0.
    let v = view(vec![clip(2880, &[note(0, 100, 0), note(1800, 480, 1)])]);
    let log = run(
        &v,
        &mut t120(),
        &[(0, Command::SetLoop(Some((0, 1920)))), (0, Command::Play)],
        44_100 * 2 + 1,
        300,
    );
    let at = |t: f64| (t * 22.968_75f64).round() as u64;
    assert_eq!(
        short(&log),
        vec![
            (0, 0, 1, 0),
            (at(100.0), 0, 0, 0),
            (at(1800.0), 0, 1, 1),
            // The note would end at 2280; the loop jump at 1920 ends it.
            (44_100, 0, 0, 1),
            (44_100, 0, 1, 0),
            (44_100 + at(100.0), 0, 0, 0),
            (44_100 + at(1800.0), 0, 1, 1),
            (88_200, 0, 0, 1),
            (88_200, 0, 1, 0),
        ]
    );
}

#[test]
fn a_locate_lands_mid_clip() {
    let v = view(vec![clip(3840, &[note(0, 10, 0), note(1920, 10, 1)])]);
    let log = run(
        &v,
        &mut t120(),
        &[(0, Command::Play), (1000, Command::Locate(1000))],
        60_000,
        128,
    );
    // After the jump to tick 1000, the next note is tick 1920, 920 ticks later.
    assert_eq!(
        ons(&log),
        vec![0, 1000 + (920.0 * 22.968_75f64).round() as u64]
    );
}

#[test]
fn stop_ends_notes_and_stop_again_panics() {
    let v = view(vec![clip(3840, &[note(0, 3000, 7)])]);
    let log = run(
        &v,
        &mut t120(),
        &[
            (0, Command::Play),
            (10_000, Command::Stop),
            (20_000, Command::Stop),
        ],
        30_000,
        128,
    );
    assert_eq!(
        short(&log),
        vec![(0, 0, 1, 7), (10_000, 0, 0, 7), (20_000, ALL, 2, 0)]
    );
    assert_eq!(log[2].dest, ALL);
}

#[test]
fn a_note_that_starts_again_ends_the_old_one_first() {
    let v = view(vec![clip(960, &[note(0, 900, 4), note(480, 900, 4)])]);
    let log = run(&v, &mut t120(), &[(0, Command::Play)], 11_026, 64);
    assert_eq!(
        short(&log),
        vec![(0, 0, 1, 4), (11_025, 0, 0, 4), (11_025, 0, 1, 4)]
    );
}

#[test]
fn muted_tracks_and_count_ins_are_silent() {
    let mut p = Project::new();
    let mut t = Track::new("t", 0);
    t.slots[0] = Some(clip(960, &[note(0, 10, 0)]));
    t.mute = true;
    p.tracks.push(t);
    let v = PlayView::compile(
        &p,
        &[Launch {
            track: 0,
            slot: 0,
            origin: 0,
            start: 0,
        }],
    )
    .unwrap();
    assert!(run(&v, &mut t120(), &[(0, Command::Play)], 50_000, 128).is_empty());

    // One bar of count-in from frame 0: the first note is on the record tick.
    let v = view(vec![clip(960, &[note(0, 10, 0)])]);
    let log = run(
        &v,
        &mut t120(),
        &[(0, Command::SetCountIn(1)), (0, Command::Record)],
        88_201,
        128,
    );
    assert_eq!(ons(&log), vec![88_200]);
}

#[test]
fn notes_follow_tempo_changes() {
    let v = view(vec![clip(960, &[note(0, 10, 0)])]);
    let mut t = t120();
    t.set_tempo_map(
        TempoMap::new(&[
            Segment {
                start: 0,
                bpm: 120.0,
            },
            Segment {
                start: 1920,
                bpm: 60.0,
            },
        ])
        .unwrap(),
    )
    .unwrap();
    let log = run(
        &v,
        &mut t,
        &[(0, Command::Play)],
        22_050 * 2 + 44_100 * 2 + 1,
        1000,
    );
    assert_eq!(ons(&log), vec![0, 22_050, 44_100, 88_200, 132_300]);
}

#[test]
fn chance_leaves_notes_out_differently_on_every_pass() {
    // Sixteen 1/16s at 50 %, over a transport loop of one bar: every pass
    // of the loop rolls again.
    let mut c = clip(
        3840,
        &(0..16).map(|i| note(i * 240, 100, 0)).collect::<Vec<_>>(),
    );
    c.notes.iter_mut().for_each(|n| n.chance = 50);
    let v = view(vec![c]);
    let cmds = [(0, Command::SetLoop(Some((0, 3840)))), (0, Command::Play)];
    let bar = 88_200u64;
    let log = run(&v, &mut t120(), &cmds, 16 * bar, 128);
    let on = ons(&log);
    assert!(
        (96..=160).contains(&on.len()),
        "about half of 256: {}",
        on.len()
    );
    let passes: Vec<Vec<u64>> = (0..16)
        .map(|p| {
            on.iter()
                .filter(|&&f| f / bar == p)
                .map(|f| f % bar)
                .collect()
        })
        .collect();
    assert!(passes.windows(2).any(|w| w[0] != w[1]), "the passes differ");
    for period in [1, 4096] {
        assert_eq!(
            ons(&run(&v, &mut t120(), &cmds, 16 * bar, period)),
            on,
            "period {period}"
        );
    }
    // At 100 % every note plays.
    let mut sure = clip(
        3840,
        &(0..16).map(|i| note(i * 240, 100, 0)).collect::<Vec<_>>(),
    );
    sure.notes[3].chance = 100;
    assert_eq!(
        ons(&run(&view(vec![sure]), &mut t120(), &cmds, 2 * bar, 128)).len(),
        32
    );
}

#[test]
fn all_128_tracks_play_each_on_its_own_track() {
    let n = mpfree_seq::MAX_TRACKS;
    let clips = (0..n)
        .map(|k| clip(3840, &[note(0, 120, (k % 100) as u8 + 20)]))
        .collect();
    let v = view(clips);
    let log = run(&v, &mut t120(), &[(0, Command::Play)], 4_000, 256);
    let ons: Vec<(u16, u8)> = short(&log)
        .into_iter()
        .filter(|s| s.2 == 1)
        .map(|s| (s.1, s.3))
        .collect();
    assert_eq!(ons.len(), n, "{ons:?}");
    for (k, &(track, note)) in ons.iter().enumerate() {
        assert_eq!((track as usize, note), (k, (k % 100) as u8 + 20));
    }
}
