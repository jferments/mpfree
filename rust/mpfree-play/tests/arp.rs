//! A track through the arpeggiator: the player holds the clip's notes on
//! an mpfree-arp engine that follows the transport, and the arp's notes land
//! on the transport's grid, the same for any period size.

use mpfree_arp::{Engine, EventKind, Mode, Order, Rate, Settings};
use mpfree_play::{ArpHost, Event, Launch, Out, PlayView, Player, Sink, ALL};
use mpfree_seq::{Clip, Note, Project, Track};
use mpfree_transport::time::Swing;
use mpfree_transport::transport::MAX_PERIOD;
use mpfree_transport::{Block, Command, Transport};

const ARP: u16 = 1;

/// Notes for the arp (and a panic's `AllOff`, which is for every
/// destination) go to it; the rest are logged as played directly.
struct Route<'a> {
    arp: &'a mut ArpHost,
    direct: &'a mut Notes,
}
impl Sink for Route<'_> {
    fn send(&mut self, o: Out) {
        if o.dest == ARP || o.dest == ALL {
            self.arp.input(o.offset, o.event);
        }
        match o.event {
            Event::NoteOn { note, .. } if o.dest != ARP => self.direct.push((o.frame, note, 1)),
            _ => {}
        }
    }
}

fn arp_settings() -> Settings {
    Settings { mode: Mode::Arp, order: Order::Up, rate: Rate::Sixteenth, gate: 50, ..Settings::default() }
}

/// A one-bar chord (9, 10, 11) on the arp track, and a kick on beat 1 of a
/// direct track.
fn view() -> PlayView {
    let mut p = Project::new();
    let mut arp = Track::new("arp", ARP);
    let mut c = Clip::new("chord", 2 * 3840);
    c.notes = [9u8, 11, 10].iter().map(|&n| Note { tick: 0, len: 3840, note: n, velocity: 100 }).collect();
    arp.slots[0] = Some(c);
    let mut drums = Track::new("drums", 0);
    let mut k = Clip::new("kick", 3840);
    k.notes = vec![Note { tick: 0, len: 100, note: 0, velocity: 120 }];
    drums.slots[0] = Some(k);
    p.tracks = vec![arp, drums];
    PlayView::compile(&p, &[Launch { track: 0, slot: 0, origin: 0 }, Launch { track: 1, slot: 0, origin: 0 }]).unwrap()
}

/// (absolute frame, note, 1 = on / 0 = off).
type Notes = Vec<(u64, u8, u8)>;

/// The arp's notes and the direct notes.
fn run(t: &mut Transport, frames: u64, mut next: impl FnMut() -> u32) -> (Notes, Notes) {
    let view = view();
    let mut player = Player::new();
    let mut host = ArpHost::new(Engine::new(t.rate(), t.bpm()));
    host.engine.set_settings(arp_settings());
    let mut block = Block::new();
    let mut arp = vec![];
    let mut direct = vec![];
    while t.frame() < frames {
        let n = next().min((frames - t.frame()) as u32);
        t.process(n, &mut block);
        player.process(&view, &block, &mut Route { arp: &mut host, direct: &mut direct });
        let start = block.start_frame;
        host.process(&block, &mut |offset, kind| {
            assert!(offset < n);
            arp.push(match kind {
                EventKind::On { note, .. } => (start + u64::from(offset), note, 1),
                EventKind::Off { note } => (start + u64::from(offset), note, 0),
            });
        });
    }
    assert_eq!(host.dropped(), 0);
    (arp, direct)
}

#[test]
fn a_held_chord_arpeggiates_on_the_sixteenths() {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let (arp, direct) = run(&mut t, 88_200, || 128);
    let ons: Vec<(u64, u8)> = arp.iter().filter(|e| e.2 == 1).map(|e| (e.0, e.1)).collect();
    // A 1/16 at 120 BPM is 5512.5 frames; tick k * 240 falls on round(k * 5512.5).
    let at = |k: u64| (k as f64 * 5512.5).round() as u64;
    assert_eq!(&ons[..5], &[(at(0), 9), (at(1), 10), (at(2), 11), (at(3), 9), (at(4), 10)]);
    assert_eq!(ons.len(), 16, "one bar of 1/16s, then the chord ends");
    // Gate 50 %: each note ends half a step later.
    assert!(arp.iter().any(|e| e.2 == 0 && e.1 == 9 && (e.0 as i64 - at(0) as i64 - 2756).abs() <= 1), "{arp:?}");
    assert_eq!(direct, vec![(0, 0, 1)]);
}

#[test]
fn the_arp_follows_transport_swing() {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::SetSwing(Swing::new(240, 66).unwrap()));
    t.command(0, Command::Play);
    let (arp, _) = run(&mut t, 30_000, || 333);
    let ons: Vec<u64> = arp.iter().filter(|e| e.2 == 1).map(|e| e.0).collect();
    let at = |tick: f64| (tick * 22.968_75).round() as u64;
    assert_eq!(&ons[..4], &[0, at(240.0 * 66.0 / 50.0), at(480.0), at(480.0 + 240.0 * 66.0 / 50.0)]);
}

fn busy(t: &mut Transport) {
    for (at, c) in [
        (0, Command::SetLoop(Some((0, 7680)))),
        (0, Command::SetSwing(Swing::new(240, 58).unwrap())),
        (500, Command::Play),
        (100_000, Command::SetTempo(97.0)),
        (150_000, Command::Locate(2_000)),
        (220_000, Command::Stop),
        (230_000, Command::Play),
        (300_000, Command::Stop),
        (310_000, Command::Stop),
        (320_000, Command::PlayStart),
    ] {
        assert!(t.command(at, c));
    }
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
fn the_arp_track_is_the_same_for_every_period_size() {
    let go = |next: &mut dyn FnMut() -> u32| {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        busy(&mut t);
        run(&mut t, 400_000, next)
    };
    let reference = go(&mut || 128);
    assert!(reference.0.iter().filter(|e| e.2 == 1).count() > 20, "{}", reference.0.len());
    for n in [1u32, 7, 64, 333, 1024, 4096, MAX_PERIOD] {
        assert!(go(&mut || n) == reference, "period size {n} differs");
    }
    for seed in [11u64, 0xa5a5] {
        let mut r = random(seed, 3000);
        assert!(go(&mut r) == reference, "random sizes (seed {seed}) differ");
    }
}

#[test]
fn live_pad_notes_mix_with_clip_notes_in_any_order() {
    // The player's notes, then live pad notes for the same period, come in
    // two batches; the arp plays them in offset order either way.
    let fed = [
        (300, Event::NoteOn { note: 60, velocity: 100 }),
        (900, Event::NoteOff { note: 64 }),
        (40, Event::NoteOn { note: 64, velocity: 90 }),
        (300, Event::NoteOn { note: 67, velocity: 80 }),
    ];
    let sorted = [fed[2], fed[0], fed[3], fed[1]];
    let play = |notes: &[(u32, Event)]| {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        t.command(0, Command::Play);
        let mut host = ArpHost::new(Engine::new(44_100, 120.0));
        host.engine.set_settings(Settings { rate: Rate::ThirtySecond, ..arp_settings() });
        let mut block = Block::new();
        let mut out = vec![];
        for period in 0..40 {
            t.process(1024, &mut block);
            if period == 0 {
                for &(offset, event) in notes {
                    host.input(offset, event);
                }
            }
            let start = block.start_frame;
            host.process(&block, &mut |offset, kind| out.push((start + u64::from(offset), kind)));
        }
        out
    };
    let out = play(&fed);
    assert!(out.len() > 10, "{out:?}");
    assert_eq!(out, play(&sorted));
}

#[test]
fn a_note_held_by_a_pad_and_a_clip_waits_for_both() {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let mut host = ArpHost::new(Engine::new(44_100, 120.0));
    host.engine.set_settings(arp_settings());
    let mut block = Block::new();
    let mut period = |host: &mut ArpHost, notes: &[(u32, Event)]| {
        t.process(512, &mut block);
        for &(offset, event) in notes {
            host.input(offset, event);
        }
        host.process(&block, &mut |_, _| {});
    };
    // The pad presses 60, the clip plays 60 and ends it: the pad still holds it.
    period(&mut host, &[(0, Event::NoteOn { note: 60, velocity: 100 }), (100, Event::NoteOn { note: 60, velocity: 80 })]);
    period(&mut host, &[(200, Event::NoteOff { note: 60 })]);
    assert!(host.engine.active().contains(60));
    // The pad lets go: released. A stray release changes nothing.
    period(&mut host, &[(0, Event::NoteOff { note: 60 }), (10, Event::NoteOff { note: 60 })]);
    assert!(!host.engine.active().contains(60));
    period(&mut host, &[(0, Event::NoteOn { note: 60, velocity: 100 })]);
    assert!(host.engine.active().contains(60), "one press presses again");
    // A panic forgets every press.
    period(&mut host, &[(0, Event::NoteOn { note: 62, velocity: 100 }), (5, Event::AllOff)]);
    period(&mut host, &[(0, Event::NoteOn { note: 62, velocity: 100 })]);
    assert!(host.engine.active().contains(62) && !host.engine.active().contains(60));
}
