//! Trigger groups (the trigger groups design, approved 22 Nov), proved
//! against a small model of their own: seeded random clips of notes on two
//! tracks, with several groups and lone notes on one tick (their notes
//! mixed in the clip's list, so a lone conditional note sits between a
//! group's notes), every condition and a spread of chances, with quantise
//! (which lines notes from different ticks up without joining them) and
//! swing, launched and in a song, with FILL on for a few bars, at three
//! period sizes.
//!
//! The model walks each pass's notes in the order the player meets them
//! (by where they play, the left track first, then the clip's order) and
//! checks that every note of a trigger gets one decision, made at its
//! first note; that a trigger reads the last result the units before it
//! left (PRE on its own track, NEI on the track to the left) and leaves
//! one result behind; and that an off trigger plays nothing and decides
//! nothing. A chance below 100 is the one thing it can't work out, so it
//! takes that throw from what played, once per trigger. Every period size
//! plays the same.

use mpfree_play::{Event, Launch, Out, PlayView, Player, Sink};
use mpfree_seq::{Clip, Cond, Holds, Note, Placement, Project, Quantise, Track};
use mpfree_transport::time::{Swing, Tick};
use mpfree_transport::{Block, Command, Transport};
use std::collections::HashMap;

const BAR: Tick = 3840;
/// Frames in one bar at 120 BPM: 2 s.
const BAR_FRAMES: u64 = 88_200;
const BARS: u64 = 16;
/// FILL is on for these passes (from just before the first to just
/// before the one after the last).
const FILL: std::ops::Range<u64> = 3..6;
const TRACKS: usize = 2;

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, out: Out) {
        self.0.push(out);
    }
}

/// A small seeded generator (xorshift), so a failure names its seed.
struct Dice(u64);
impl Dice {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<T: Copy>(&mut self, of: &[T]) -> T {
        of[self.below(of.len())]
    }
}

/// One random scenario: each track's clip of notes, every pitch once on
/// its track (so each note sent names its note), with the same quantise
/// and swing.
fn scenario(seed: u64) -> Vec<Clip> {
    let mut d = Dice(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let quantise = (d.below(2) == 0).then(|| Quantise::new(240, 100).unwrap());
    let swing = (d.below(2) == 0).then(|| Swing::new(240, 62).unwrap());
    let conds = Cond::all();
    (0..TRACKS)
        .map(|_| {
            let mut c = Clip::new("rolls", BAR);
            c.holds = Holds::Notes;
            c.quantise = quantise;
            c.swing = swing;
            let mut pitches: Vec<u8> = (0..128).collect();
            for i in (1..pitches.len()).rev() {
                pitches.swap(i, d.below(i + 1));
            }
            let mut notes = Vec::new();
            // Steps 0 to 14, so quantise never moves a note into the next
            // pass; off the grid only from step 1.
            for step in 0..15i64 {
                if d.below(3) == 0 {
                    continue;
                }
                let ticks: Vec<Tick> = if step == 0 {
                    vec![0]
                } else {
                    let offs = [-48, 0, 36];
                    let n = 1 + d.below(2);
                    let mut t: Vec<Tick> = (0..n).map(|_| step * 240 + d.pick(&offs)).collect();
                    t.dedup();
                    t
                };
                for tick in ticks {
                    let mut numbers: Vec<u8> = (1..=16).chain(129..=132).collect();
                    for _ in 0..1 + d.below(4) {
                        let group = if d.below(3) == 0 || numbers.is_empty() {
                            0
                        } else {
                            numbers.remove(d.below(numbers.len()))
                        };
                        let chance = d.pick(&[100, 100, 50, 50, 25, 75, 1]);
                        let cond = if d.below(3) == 0 {
                            Cond::Always
                        } else {
                            d.pick(&conds)
                        };
                        let on = d.below(10) != 0;
                        let members = if group == 0 { 1 } else { 1 + d.below(3) };
                        for _ in 0..members {
                            let Some(pitch) = pitches.pop() else { break };
                            notes.push(Note {
                                chance,
                                cond,
                                on,
                                group,
                                ..Note::new(tick, 120, pitch, 1 + d.below(127) as u8)
                            });
                        }
                    }
                }
            }
            // Mixed up, so other notes sit between a group's notes.
            for i in (1..notes.len()).rev() {
                notes.swap(i, d.below(i + 1));
            }
            c.notes = notes;
            assert!(c.groups_ok(), "seed {seed}");
            c
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Way {
    Launched,
    Song,
}

/// The clips on tracks of their own from tick 0, played `BARS` bars in
/// periods of `size`, FILL on for [`FILL`]: each note-on as (frame,
/// track, note).
fn play(clips: &[Clip], size: u32, way: Way) -> Vec<(u64, usize, u8)> {
    let mut p = Project::new();
    for (i, c) in clips.iter().enumerate() {
        p.tracks.push(Track::new("synth", i as u16));
        p.put_new_clip(i, 0, c.clone()).unwrap();
    }
    let view = match way {
        Way::Launched => {
            let launches: Vec<Launch> = (0..clips.len())
                .map(|track| Launch {
                    track,
                    slot: 0,
                    origin: 0,
                    start: 0,
                })
                .collect();
            PlayView::compile(&p, &launches).unwrap()
        }
        Way::Song => {
            for i in 0..clips.len() {
                let block = Placement::of(&p.tracks, i, 0, 0, (BARS as Tick + 1) * BAR);
                p.arrangement.place(block).unwrap();
            }
            let mut v = PlayView::compile(&p, &[]).unwrap();
            v.set_song(&p, true).unwrap();
            v
        }
    };
    let mut t = Transport::new(44_100, 120.0).unwrap();
    assert!(t.command(0, Command::Play));
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    let fills = [
        (FILL.start * BAR_FRAMES - 1000, true),
        (FILL.end * BAR_FRAMES - 1000, false),
    ];
    let end = BARS * BAR_FRAMES;
    while t.frame() < end {
        let now = t.frame();
        let n = size.min((end - now) as u32);
        for &(at, on) in &fills {
            if (now..now + u64::from(n)).contains(&at) {
                player.set_fill(on, (at - now) as u32);
            }
        }
        t.process(n, &mut block);
        player.process(&view, &block, &mut log);
    }
    assert_eq!(player.stats().dropped, 0);
    log.0
        .iter()
        .filter_map(|o| match o.event {
            Event::NoteOn { note, .. } => Some((o.frame, usize::from(o.track), note)),
            _ => None,
        })
        .collect()
}

/// Where a note plays in its pass, before swing (which keeps the order).
fn local(c: &Clip, n: &Note) -> f64 {
    c.quantise.map_or(n.tick as f64, |q| q.apply(n.tick as f64))
}

/// What the model found in one run: how many throws it took from what
/// played, and how many of them played.
#[derive(Default)]
struct Throws {
    taken: usize,
    played: usize,
}

/// Check what played (`ons`) against the model; see the file's notes.
fn check(clips: &[Clip], ons: &[(u64, usize, u8)], what: &str) -> Throws {
    // Which note each note-on is, in which pass.
    let mut index: HashMap<(usize, u8), usize> = HashMap::new();
    for (t, c) in clips.iter().enumerate() {
        for (i, n) in c.notes.iter().enumerate() {
            index.insert((t, n.note), i);
        }
    }
    let mut played: HashMap<(u64, usize, usize), u32> = HashMap::new();
    for &(frame, t, note) in ons {
        let i = *index
            .get(&(t, note))
            .unwrap_or_else(|| panic!("{what}: a note no clip has: {t} {note}"));
        *played.entry((frame / BAR_FRAMES, t, i)).or_default() += 1;
    }
    assert!(
        played.values().all(|&n| n == 1),
        "{what}: a note twice in a pass"
    );
    // The order the player meets a pass's notes in.
    let mut order: Vec<(usize, usize)> = (0..clips.len())
        .flat_map(|t| (0..clips[t].notes.len()).map(move |i| (t, i)))
        .collect();
    order.sort_by(|&(ta, ia), &(tb, ib)| {
        let (a, b) = (&clips[ta].notes[ia], &clips[tb].notes[ib]);
        local(&clips[ta], a)
            .total_cmp(&local(&clips[tb], b))
            .then(ta.cmp(&tb))
            .then(ia.cmp(&ib))
    });
    let mut throws = Throws::default();
    let mut last = [false; TRACKS];
    for pass in 0..BARS {
        let fill = FILL.contains(&pass);
        let number = pass as i64 + 1;
        let mut decided: HashMap<(usize, Tick, u8), bool> = HashMap::new();
        for &(t, i) in &order {
            let n = &clips[t].notes[i];
            let heard = played.contains_key(&(pass, t, i));
            let key = (t, n.tick, n.group);
            let plays = match decided.get(&key) {
                // A trigger's later note: its first note decided.
                Some(&d) if n.group != 0 => d,
                _ => {
                    let d = if !n.on {
                        false
                    } else if n.cond == Cond::Always && n.chance >= 100 {
                        true
                    } else {
                        let nei = t.checked_sub(1).map(|l| last[l]);
                        let met = n.cond.holds(number, fill, last[t], nei);
                        let d = if !met {
                            false
                        } else if n.chance >= 100 {
                            true
                        } else {
                            // The throw: the one thing taken from what played.
                            throws.taken += 1;
                            throws.played += usize::from(heard);
                            heard
                        };
                        if n.cond.sets_last(n.chance) {
                            last[t] = d;
                        }
                        d
                    };
                    if n.group != 0 {
                        decided.insert(key, d);
                    }
                    d
                }
            };
            assert_eq!(
                heard, plays,
                "{what}: pass {pass}, track {t}, note {i} ({n:?}), played {heard}, the model says {plays}"
            );
        }
    }
    throws
}

#[test]
fn every_note_of_a_trigger_gets_one_decision_and_conditions_act_once() {
    let mut throws = Throws::default();
    let mut grouped = 0;
    for seed in 1..=10u64 {
        let clips = scenario(seed);
        grouped += clips
            .iter()
            .flat_map(|c| &c.notes)
            .filter(|n| n.group != 0)
            .count();
        for way in [Way::Launched, Way::Song] {
            let sizes = [64u32, 333, 4096];
            let runs: Vec<_> = sizes.iter().map(|&s| play(&clips, s, way)).collect();
            for (k, ons) in runs.iter().enumerate() {
                assert_eq!(ons, &runs[0], "seed {seed}, {way:?}, size {}", sizes[k]);
            }
            let t = check(&clips, &runs[0], &format!("seed {seed}, {way:?}"));
            throws.taken += t.taken;
            throws.played += t.played;
        }
    }
    // The scenarios have groups to prove, and the throws really vary.
    assert!(grouped > 100, "{grouped}");
    assert!(throws.taken > 200, "{}", throws.taken);
    assert!(
        throws.played > throws.taken / 5 && throws.played < throws.taken * 4 / 5,
        "{} of {}",
        throws.played,
        throws.taken
    );
}

/// A lone note on pitch 4 and pad 4's group (number 5) on one tick roll
/// apart: each kind of trigger has keys of its own. Over many passes at
/// even odds they disagree often.
#[test]
fn a_lone_note_never_shares_a_pads_throw() {
    let mut c = Clip::new("keys", BAR);
    c.holds = Holds::Notes;
    c.notes = vec![
        Note {
            chance: 50,
            ..Note::new(0, 120, 4, 100)
        },
        Note {
            chance: 50,
            group: 5,
            ..Note::new(0, 120, 60, 100)
        },
    ];
    let ons = play(&[c], 4096, Way::Launched);
    let bars = |note: u8| -> Vec<u64> {
        ons.iter()
            .filter(|o| o.2 == note)
            .map(|o| o.0 / BAR_FRAMES)
            .collect()
    };
    let (lone, group) = (bars(4), bars(60));
    let apart = (0..BARS)
        .filter(|b| lone.contains(b) != group.contains(b))
        .count();
    assert!(apart >= 2, "{lone:?} {group:?}");
}

/// A sanity check, not the proof: a chord at 50 % over 1,000 passes of a
/// clip an eighth note long plays whole on about half of them, and never
/// in part.
#[test]
fn a_chord_at_even_odds_plays_whole_about_half_the_time() {
    const LEN: Tick = 480;
    const PASSES: u64 = 1000;
    let mut c = Clip::new("odds", LEN);
    c.holds = Holds::Notes;
    for pitch in [60, 64, 67] {
        c.notes.push(Note {
            chance: 50,
            group: 1,
            ..Note::new(0, 60, pitch, 100)
        });
    }
    let mut p = Project::new();
    p.tracks.push(Track::new("synth", 0));
    p.put_new_clip(0, 0, c).unwrap();
    let launch = Launch {
        track: 0,
        slot: 0,
        origin: 0,
        start: 0,
    };
    let view = PlayView::compile(&p, &[launch]).unwrap();
    let mut t = Transport::new(44_100, 120.0).unwrap();
    assert!(t.command(0, Command::Play));
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    // An eighth note at 120 BPM: a quarter of a second.
    let pass_frames = 44_100 / 4;
    while t.frame() < PASSES * pass_frames {
        t.process(4096, &mut block);
        player.process(&view, &block, &mut log);
    }
    let mut per_pass: HashMap<u64, usize> = HashMap::new();
    for o in &log.0 {
        if let Event::NoteOn { .. } = o.event {
            *per_pass.entry(o.frame / pass_frames).or_default() += 1;
        }
    }
    assert!(per_pass.values().all(|&n| n == 3), "{per_pass:?}");
    let whole = per_pass.len();
    assert!((420..=580).contains(&whole), "{whole} of {PASSES}");
}
