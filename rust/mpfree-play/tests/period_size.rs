//! The player must give the same events on the same frames whatever the
//! audio period size. A busy session (count-in, a loop with a tempo segment
//! inside it, live tempo changes, punch in and out, locates, stop and
//! panic) plays five tracks: polymeter, 3/4 speed, double speed with its
//! own swing, quantised notes, long overlapping notes that play by
//! chance (40, 70 and 100 %) and a muted track. Parameter locks ride
//! along: on notes, between them (trigless), on a pad, after swing and
//! quantise, and on the muted track (which sends none). So do FX moves:
//! ramps, holds and ends on the slow, fast and muted clips, one of them
//! under a parameter lock.
//! It is cut into periods of 1 to 8192 frames and random sizes; every
//! event must match. Every note must also end before it starts again.

use mpfree_play::{Event, Launch, Out, PlayView, Player, Sink};
use mpfree_seq::mpfree_moves::{Kind, Map, Target};
use mpfree_seq::mpfree_param::{OnTrack, Owner, ParamAddr, RelAddr};
use mpfree_seq::{Clip, Lock, Note, Project, Quantise, Speed, Track};
use mpfree_transport::time::{Segment, Swing, TempoMap};
use mpfree_transport::transport::MAX_PERIOD;
use mpfree_transport::{Block, Command, Transport};

const LENGTH: u64 = 700_000;

#[derive(Default)]
struct Log(Vec<(u64, u16, u16, Event)>);
impl Sink for Log {
    fn send(&mut self, o: Out) {
        self.0.push((o.frame, o.track, o.dest, o.event));
    }
}

fn notes(step: i64, count: i64, len: i64, pitch: impl Fn(i64) -> u8) -> Vec<Note> {
    (0..count)
        .map(|i| Note {
            tick: i * step,
            len,
            note: pitch(i),
            velocity: 1 + (i * 37 % 127) as u8,
            chance: 100,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
            touch: mpfree_seq::Touch::NONE,
        })
        .collect()
}

fn session() -> PlayView {
    let mut p = Project::new();
    let mut add = |name: &str, mut c: Clip, origin: i64, mute: bool| {
        let mut t = Track::new(name, p.tracks.len() as u16 % 3);
        t.mute = mute;
        c.name = name.to_string();
        t.slots[(p.tracks.len() * 3) % 16] = Some(c);
        let slot = (p.tracks.len() * 3) % 16;
        p.tracks.push(t);
        Launch {
            track: p.tracks.len() - 1,
            slot,
            origin,
            start: origin,
        }
    };
    let mut launches = vec![];
    // Drums: 1/16s for a bar, three pads.
    let mut c = Clip::new("", 3840);
    c.notes = notes(240, 16, 120, |i| [0u8, 2, 1, 2][i as usize % 4]);
    c.locks = vec![
        Lock::new(240, Some(2), RelAddr::new(OnTrack::Pad(2), 0), 5.0),
        Lock::new(600, None, RelAddr::new(OnTrack::Inst, 3), 0.5),
        Lock::new(1440, Some(1), RelAddr::new(OnTrack::Strip, 0), 0.8),
    ];
    launches.push(add("drums", c, 0, false));
    // A 7/16 clip at 3/4 speed against it.
    let mut c = Clip::new("", 1680);
    c.speed = Speed::new(3, 4).unwrap();
    c.notes = notes(240, 7, 300, |i| 40 + i as u8);
    moves(&mut c, Target::Track(RelAddr::new(OnTrack::Inst, 5)));
    launches.push(add("poly", c, 960, false));
    // Double speed with its own 1/8 swing.
    let mut c = Clip::new("", 1920);
    c.speed = Speed::new(2, 1).unwrap();
    c.swing = Some(Swing::new(480, 61).unwrap());
    c.notes = notes(160, 12, 80, |i| 60 + (i % 5) as u8);
    c.locks = vec![
        Lock::new(320, None, RelAddr::new(OnTrack::Inst, 12), 900.0),
        Lock::new(480, None, RelAddr::new(OnTrack::Inst, 12), 450.0),
    ];
    // A lane on the parameter the locks hold, and a fixed one.
    moves(&mut c, Target::Track(RelAddr::new(OnTrack::Inst, 12)));
    moves(&mut c, Target::Fixed(ParamAddr::new(Owner::Macro(1), 0)));
    launches.push(add("fast", c, -3840, false));
    // Loose notes, quantised at 70 %, long and overlapping on one pitch,
    // each with a chance of 40, 70 or 100 %.
    let mut c = Clip::new("", 2 * 3840);
    c.quantise = Quantise::new(240, 70);
    c.notes = (0..20)
        .map(|i| Note {
            tick: i * 383 + 17,
            len: 900,
            note: 36,
            velocity: 90,
            chance: 40 + (i % 3) as u8 * 30,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
            touch: mpfree_seq::Touch::NONE,
        })
        .collect();
    c.locks = vec![Lock::new(
        3 * 383 + 17,
        None,
        RelAddr::new(OnTrack::Inst, 1),
        2.0,
    )];
    launches.push(add("loose", c, 0, false));
    // A muted track.
    let mut c = Clip::new("", 960);
    c.notes = notes(120, 8, 60, |_| 9);
    c.locks = vec![Lock::new(0, None, RelAddr::new(OnTrack::Inst, 1), 2.0)];
    moves(&mut c, Target::Track(RelAddr::new(OnTrack::Inst, 1)));
    launches.push(add("muted", c, 0, true));
    PlayView::compile(&p, &launches).unwrap()
}

/// Ramps, holds and ends across the clip, with a gesture that runs over
/// its end.
fn moves(c: &mut Clip, target: Target) {
    let len = c.length as u32;
    let mut pts: Vec<(f64, f32, Kind)> = (0..len / 97)
        .map(|k| {
            let kind = [Kind::Ramp, Kind::Ramp, Kind::Step, Kind::Ramp, Kind::End][k as usize % 5];
            (f64::from(k * 97 + 3), ((k * 13) % 11) as f32 / 10.0, kind)
        })
        .collect();
    pts.retain(|p| p.0 < f64::from(len) - 40.0);
    pts.push((f64::from(len) - 20.0, 0.5, Kind::Ramp));
    pts.push((f64::from(len) + 30.0, 0.9, Kind::Ramp));
    pts.push((f64::from(len) + 60.0, 0.0, Kind::End));
    c.moves
        .put(target, Map::linear(-1.0, 1.0), &pts, len)
        .unwrap();
}

fn script(t: &mut Transport) {
    let map = TempoMap::new(&[
        Segment {
            start: 0,
            bpm: 120.0,
        },
        Segment {
            start: 1920,
            bpm: 90.0,
        },
        Segment {
            start: 5760,
            bpm: 140.0,
        },
    ])
    .unwrap();
    t.set_tempo_map(map).unwrap();
    let cmds = [
        (0, Command::SetCountIn(1)),
        (0, Command::SetLoop(Some((3840, 7680)))),
        (0, Command::SetSwing(Swing::new(240, 58).unwrap())),
        (1_000, Command::Record),
        (300_000, Command::SetTempo(133.0)),
        (330_000, Command::Record),
        (360_000, Command::Locate(1_000)),
        (420_000, Command::SetLoop(None)),
        (450_000, Command::SetTempo(77.7)),
        (450_001, Command::Locate(-100)),
        (520_000, Command::Stop),
        (530_000, Command::Stop),
        (540_000, Command::Play),
        (580_000, Command::PlayStart),
        (600_000, Command::SetLoop(Some((0, 240)))),
        (640_000, Command::StopAndRewind),
        (650_000, Command::SetCountIn(0)),
        (650_000, Command::Record),
        (690_000, Command::Stop),
    ];
    for (at, c) in cmds {
        assert!(t.command(at, c));
    }
}

fn run(view: &PlayView, mut next: impl FnMut() -> u32) -> Vec<(u64, u16, u16, Event)> {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    script(&mut t);
    let mut player = Player::new();
    let mut block = Block::new();
    let mut log = Log::default();
    while t.frame() < LENGTH {
        let n = next().min((LENGTH - t.frame()) as u32);
        t.process(n, &mut block);
        let before = log.0.len();
        player.process(view, &block, &mut log);
        for e in &log.0[before..] {
            assert!(
                e.0 >= block.start_frame && e.0 < block.start_frame + u64::from(n),
                "event outside its period"
            );
        }
    }
    let s = player.stats();
    assert_eq!((s.dropped, s.late_offs), (0, 0));
    assert_eq!(player.sounding(), 0, "the final stop ends every note");
    log.0
}

fn random(seed: u64, max: u32) -> impl FnMut() -> u32 {
    let mut x = seed;
    move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % u64::from(max)) as u32 + 1
    }
}

#[test]
fn the_same_events_for_every_period_size() {
    let view = session();
    let reference = run(&view, || 128);

    // The session plays: plenty of notes on every unmuted track, none on
    // the muted one, frames in order, and every note ends before it starts
    // again.
    let on = |track| {
        reference
            .iter()
            .filter(|e| e.1 == track && matches!(e.3, Event::NoteOn { .. }))
            .count()
    };
    assert!(
        on(0) > 80 && on(1) > 40 && on(2) > 200 && on(3) > 25,
        "{} {} {} {}",
        on(0),
        on(1),
        on(2),
        on(3)
    );
    assert_eq!(on(4), 0);
    assert!(reference.windows(2).all(|w| w[0].0 <= w[1].0));
    let locks = |track| {
        reference
            .iter()
            .filter(|e| e.1 == track && matches!(e.3, Event::Lock { .. }))
            .count()
    };
    assert!(
        locks(0) > 10 && locks(2) > 10 && locks(3) > 0,
        "{} {} {}",
        locks(0),
        locks(2),
        locks(3)
    );
    assert_eq!(locks(4), 0, "the muted track locks nothing");
    let mut locked = std::collections::HashSet::new();
    let mut sounding = std::collections::HashSet::new();
    let mut lanes = 0;
    for e in &reference {
        match e.3 {
            Event::Lock { addr, .. } => {
                locked.insert((e.1, addr));
            }
            Event::Unlock { addr } => assert!(locked.remove(&(e.1, addr)), "{e:?} was not locked"),
            Event::NoteOn { note, .. } => assert!(
                sounding.insert((e.1, e.2, note)),
                "{e:?} starts while sounding"
            ),
            Event::NoteOff { note } => assert!(
                sounding.remove(&(e.1, e.2, note)),
                "{e:?} ends a silent note"
            ),
            Event::AllOff => assert!(sounding.is_empty()),
            // A move holds its parameter as a lock does (and takes it back
            // from one).
            Event::Lane { addr, .. } => {
                assert_ne!(e.1, 4, "the muted track sends no moves");
                locked.insert((e.1, addr));
                lanes += 1;
            }
            Event::Pressure { note, .. } => assert!(
                sounding.contains(&(e.1, e.2, note)),
                "{e:?} presses a silent note"
            ),
        }
    }
    assert!(reference.iter().any(|e| e.3 == Event::AllOff));
    assert!(lanes > 500, "{lanes} lane values");

    for n in [1, 2, 7, 64, 127, 333, 1024, 4096, MAX_PERIOD] {
        let got = run(&view, || n);
        if got != reference {
            let k = got
                .iter()
                .zip(&reference)
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(reference.len()));
            let lo = k.saturating_sub(4);
            panic!(
                "period size {n} differs at event {k} of {} / {}:\n got  {:?}\n want {:?}",
                got.len(),
                reference.len(),
                &got[lo..(k + 4).min(got.len())],
                &reference[lo..(k + 4).min(reference.len())]
            );
        }
    }
    for seed in [3u64, 0x5eed, 987_654_321] {
        assert!(
            run(&view, random(seed, 3000)) == reference,
            "random sizes (seed {seed}) differ"
        );
    }
}
