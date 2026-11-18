//! A clip's play direction (sequencer S6): its 1/16 steps in reverse,
//! ping-pong or random order, each step's notes and locks moving with it
//! and keeping their place around its line, the same at every period
//! size; and a note recorded on such a clip lands on the step heard.

use mpfree_play::{Event, Launch, Out, PlayView, Player, Recorder, Sink};
use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
use mpfree_seq::{Clip, Direction, Lock, Note, Project, Track};
use mpfree_transport::{Block, Command, Transport};

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, out: Out) {
        self.0.push(out);
    }
}

/// Frames a tick at 120 BPM and 44.1 kHz.
const FRAMES_PER_TICK: f64 = 44_100.0 * 0.5 / 960.0;

/// Four 1/16 steps, a note on each line (60 to 63), one a little early
/// for step 2 (70 at 470) and one a little early for the next pass's
/// step 0 (71 at 950), and a lock on step 3's line.
fn clip(direction: Direction) -> Clip {
    let mut c = Clip::new("c", 960);
    c.notes = vec![
        Note::new(0, 60, 60, 100),
        Note::new(240, 60, 61, 100),
        Note::new(470, 60, 70, 100),
        Note::new(480, 60, 62, 100),
        Note::new(720, 60, 63, 100),
        Note::new(950, 5, 71, 100),
    ];
    c.locks = vec![Lock::new(720, None, RelAddr::new(OnTrack::Inst, 2), 0.5)];
    c.direction = direction;
    c
}

fn view(c: Clip) -> PlayView {
    let mut p = Project::new();
    let mut t = Track::new("t", 0);
    t.slots[0] = Some(c);
    p.tracks.push(t);
    let launch = Launch {
        track: 0,
        slot: 0,
        origin: 0,
        start: 0,
    };
    PlayView::compile(&p, &[launch]).unwrap()
}

/// Play `ticks` ticks in periods of `period` frames: each note-on and
/// lock as (tick, note), a lock as note 255.
fn play(view: &PlayView, ticks: i64, period: u32) -> Vec<(i64, u8)> {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    assert!(t.command(0, Command::Play));
    let frames = (ticks as f64 * FRAMES_PER_TICK) as u64;
    let mut player = Player::new();
    let mut block = Block::new();
    let mut log = Log::default();
    while t.frame() < frames {
        t.process(period.min((frames - t.frame()) as u32), &mut block);
        player.process(view, &block, &mut log);
    }
    assert_eq!(player.stats().dropped + player.stats().late_offs, 0);
    log.0
        .iter()
        .filter_map(|o| {
            let tick = (o.frame as f64 / FRAMES_PER_TICK).round() as i64;
            match o.event {
                Event::NoteOn { note, .. } => Some((tick, note)),
                Event::Lock { .. } => Some((tick, 255)),
                _ => None,
            }
        })
        .collect()
}

/// The same at every period size, from one frame up.
fn played(c: Clip, ticks: i64) -> Vec<(i64, u8)> {
    let v = view(c);
    let one = play(&v, ticks, 1);
    for period in [64, 128, 1000, 4096] {
        assert_eq!(play(&v, ticks, period), one, "period {period}");
    }
    one
}

#[test]
fn reverse_plays_the_last_step_first_each_note_around_its_line() {
    let got = played(clip(Direction::Reverse), 1920);
    // Play step 0 plays clip step 3 (its lock on the line, then its
    // note), step 1 plays step 2 (70 ten ticks early), step 2 plays
    // step 1, and step 3 plays step 0, with 71 ten ticks before it.
    let pass = [
        (0, 255),
        (0, 63),
        (230, 70),
        (240, 62),
        (480, 61),
        (710, 71),
        (720, 60),
    ];
    let mut want: Vec<(i64, u8)> = pass.to_vec();
    want.extend(pass.iter().map(|&(t, n)| (t + 960, n)));
    assert_eq!(got, want);
    // Forward plays it as written.
    let got = played(clip(Direction::Forward), 960);
    assert_eq!(
        got,
        [
            (0, 60),
            (240, 61),
            (470, 70),
            (480, 62),
            (720, 255),
            (720, 63),
            (950, 71)
        ]
    );
}

#[test]
fn ping_pong_turns_at_the_ends_across_passes() {
    let got = played(clip(Direction::PingPong), 2880);
    // 0 1 2 3 | 2 1 0 1 | 2 3 2 1. The first pass's 71 would play before
    // the clip started, so it doesn't.
    assert_eq!(
        got,
        [
            (0, 60),
            (240, 61),
            (470, 70),
            (480, 62),
            (720, 255),
            (720, 63),
            // Pass 2: step 2 first, its early note before the pass.
            (950, 70),
            (960, 62),
            (1200, 61),
            (1430, 71),
            (1440, 60),
            (1680, 61),
            // Pass 3.
            (1910, 70),
            (1920, 62),
            (2160, 255),
            (2160, 63),
            (2390, 70),
            (2400, 62),
            (2640, 61),
            // Pass 4 starts on step 0 again: 71 just before it.
            (2870, 71),
        ]
    );
}

#[test]
fn random_plays_every_step_once_a_pass_the_same_each_time() {
    let passes = 6;
    let got = played(clip(Direction::Random), 960 * passes);
    let mut orders = vec![];
    for p in 0..passes {
        // The notes on the steps' lines, by where they played.
        let mut order: Vec<u8> = got
            .iter()
            .filter(|&&(t, n)| t >= p * 960 && t < p * 960 + 960 && (60..64).contains(&n))
            .map(|&(t, n)| {
                assert_eq!(t % 240, 0, "on a line");
                n
            })
            .collect();
        orders.push(order.clone());
        order.sort_unstable();
        assert_eq!(order, [60, 61, 62, 63], "pass {p}: every step once");
    }
    // Not the same order every pass.
    assert!(orders.windows(2).any(|w| w[0] != w[1]), "{orders:?}");
    // Each early note just before its step, wherever that played.
    for &(t, n) in &got {
        if n == 70 {
            assert!(got.contains(&(t + 10, 62)), "70 at {t}");
        }
    }
    // A recompile plays the same.
    assert_eq!(played(clip(Direction::Random), 960 * passes), got);
}

/// A clip that isn't a whole number of steps plays forward whatever its
/// direction.
#[test]
fn a_clip_of_part_steps_plays_forward() {
    let mut c = clip(Direction::Reverse);
    c.length = 950;
    c.notes.pop();
    let mut f = c.clone();
    f.direction = Direction::Forward;
    assert_eq!(played(c, 1900), played(f, 1900));
}

/// A note played on a reversed clip lands on the step heard: played just
/// after the line of play step 1 (where clip step 2 plays), it goes just
/// after clip step 2's line; a knob turned while it sounds locks it there.
#[test]
fn a_note_recorded_on_a_reversed_clip_lands_on_the_step_heard() {
    let mut c = Clip::new("c", 960);
    c.direction = Direction::Reverse;
    let mut v = view(c);
    v.set_armed(Some(0));
    let mut t = Transport::new(44_100, 120.0).unwrap();
    assert!(t.command(0, Command::Record));
    let mut rec = Recorder::new();
    let mut block = Block::new();
    let at = |tick: f64| (tick * FRAMES_PER_TICK).round() as u64;
    let live = [
        // Pressed 5 ticks after play step 1's line, released 50 later; a
        // knob turned in between.
        (
            at(245.0),
            Event::NoteOn {
                note: 64,
                velocity: 100,
            },
        ),
        (
            at(265.0),
            Event::Lock {
                addr: RelAddr::new(OnTrack::Inst, 2).on(0),
                value: 0.25,
            },
        ),
        (at(295.0), Event::NoteOff { note: 64 }),
    ];
    let mut taken = vec![];
    let mut locks = vec![];
    while t.frame() < at(1920.0) {
        let start = t.frame();
        t.process(64, &mut block);
        for &(f, e) in &live {
            if (start..t.frame()).contains(&f) {
                rec.input((f - start) as u32, e);
            }
        }
        rec.process(&v, &block);
        taken.extend(rec.taken().iter().map(|k| k.note));
        locks.extend(rec.taken_locks().iter().map(|k| k.lock));
    }
    assert_eq!(taken.len(), 1, "{taken:?}");
    // Clip step 2's line (480), 5 ticks after it.
    assert_eq!((taken[0].tick, taken[0].note), (485, 64));
    assert_eq!(locks.len(), 1, "{locks:?}");
    assert_eq!(locks[0].tick, 485, "on the note heard");
}
