//! Auto-sampling against a fake synth that answers late (with jitter, like
//! DIN MIDI), rings after each note-off, and makes each round robin a
//! little different.

use mpfree_analysis::yin;
use mpfree_sampler::autosample::*;
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

fn hz(note: u8) -> f32 {
    440.0 * ((f32::from(note) - 69.0) / 12.0).exp2()
}

struct Voice {
    note: u8,
    amp: f32,
    phase: f32,
    env: f32,
    attack: f32,
    held: bool,
}

/// A stereo sine synth. Each cue lands `latency` plus up to `jitter`
/// frames late; a released note falls 60 dB in `release` seconds. With
/// `decay` it falls that fast while held too (a pluck).
struct Fake {
    latency: u64,
    jitter: u64,
    release: f32,
    decay: Option<f32>,
    level: f32,
    seed: u32,
    now: u64,
    hits: u32,
    due: Vec<(u64, Cue)>,
    voices: Vec<Voice>,
}

impl Fake {
    fn new(latency: u64, jitter: u64) -> Fake {
        Fake {
            latency,
            jitter,
            release: 0.2,
            decay: None,
            level: 0.25,
            seed: 0x1234_5678,
            now: 0,
            hits: 0,
            due: Vec::new(),
            voices: Vec::new(),
        }
    }

    fn random(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }

    fn falloff(seconds: f32) -> f32 {
        10f32.powf(-3.0 / (seconds * RATE as f32))
    }

    /// Interleaved, `ch` channels.
    fn render(&mut self, frames: usize, cues: &[Timed], ch: usize) -> Vec<f32> {
        for t in cues {
            let late = self.latency
                + if self.jitter > 0 {
                    u64::from(self.random()) % (self.jitter + 1)
                } else {
                    0
                };
            self.due.push((self.now + t.at as u64 + late, t.cue));
        }
        let mut out = Vec::with_capacity(frames * ch);
        for _ in 0..frames {
            let now = self.now;
            let mut k = 0;
            while k < self.due.len() {
                if self.due[k].0 > now {
                    k += 1;
                    continue;
                }
                match self.due.remove(k).1 {
                    Cue::NoteOn { note, velocity } => {
                        self.hits += 1;
                        let v = f32::from(velocity) / 127.0;
                        // Round robins differ a little in level and phase.
                        let wobble = 1.0 + 0.03 * (self.hits % 3) as f32;
                        self.voices.push(Voice {
                            note,
                            amp: self.level * v * v * wobble,
                            phase: 0.1 * (self.hits % 5) as f32,
                            env: 0.0,
                            attack: 1.0 / (0.005 * RATE as f32),
                            held: true,
                        });
                    }
                    Cue::NoteOff { note } => {
                        for v in self.voices.iter_mut().filter(|v| v.note == note) {
                            v.held = false;
                        }
                    }
                    Cue::AllOff => {
                        for v in &mut self.voices {
                            v.held = false;
                        }
                    }
                }
            }
            let (rel, dec) = (
                Fake::falloff(self.release),
                self.decay.map(Fake::falloff).unwrap_or(1.0),
            );
            let mut x = 0.0;
            for v in &mut self.voices {
                if v.held && v.attack > 0.0 {
                    v.env = (v.env + v.attack).min(1.0);
                    if v.env >= 1.0 {
                        v.attack = 0.0;
                    }
                } else if v.held {
                    v.env *= dec;
                } else {
                    v.env *= rel;
                }
                x += v.amp * v.env * (std::f32::consts::TAU * v.phase).sin();
                v.phase = (v.phase + hz(v.note) / RATE as f32).fract();
            }
            self.voices.retain(|v| v.held || v.env > 1e-7);
            out.push(x);
            if ch == 2 {
                out.push(0.9 * x);
            }
            self.now += 1;
        }
        out
    }
}

fn run(session: &mut Session, synth: &mut Fake, block: usize) {
    let ch = usize::from(session.plan().channels);
    let mut blocks = 0;
    while matches!(session.state(), State::Testing | State::Recording) {
        let cues = session.cues(block).to_vec();
        let audio = synth.render(block, &cues, ch);
        session.feed(&audio);
        blocks += 1;
        assert!(blocks < 1_000_000, "the session never ended");
    }
}

/// Three notes an octave apart (C3, C4, C5), soft and hard.
fn small() -> Plan {
    Plan {
        name: "Fake".into(),
        lo: 48,
        hi: 72,
        every: 12,
        velocities: vec![64, 127],
        hold: 0.3,
        tail: 2.0,
        gap: 0.05,
        ..Plan::default()
    }
}

fn record(plan: Plan, synth: &mut Fake, block: usize) -> Recording {
    let mut s = Session::new(plan, 1).unwrap();
    run(&mut s, synth, block);
    assert_eq!(s.state(), State::Done);
    s.finish()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn first_over(x: &[f32], ch: usize, level: f32) -> usize {
    x.chunks(ch)
        .position(|f| f.iter().any(|v| v.abs() >= level))
        .unwrap()
}

#[test]
fn the_default_plan_costs_what_the_plan_doc_says() {
    let p = Plan::default();
    assert!(p.check().is_ok());
    assert_eq!(p.notes().len(), 17);
    assert_eq!(p.notes().first(), Some(&36));
    assert_eq!(p.notes().last(), Some(&84));
    let e = p.estimate();
    assert_eq!(e.takes, 51);
    assert!((e.seconds / 60.0 - 3.57).abs() < 0.01, "{}", e.seconds);
    assert!(
        (e.seconds_most / 60.0 - 5.27).abs() < 0.01,
        "{}",
        e.seconds_most
    );
    assert!((e.bytes as f64 / 1e6 - 72.0).abs() < 0.1, "{}", e.bytes);
    assert!(
        (e.bytes_most as f64 / 1e6 - 108.0).abs() < 0.1,
        "{}",
        e.bytes_most
    );
    let mono = Plan {
        channels: 1,
        ..Plan::default()
    };
    assert_eq!(mono.estimate().bytes * 2, e.bytes);
    // Low to high, each layer soft to hard.
    let t = p.takes();
    assert_eq!((t[0].note, t[0].velocity), (36, 40));
    assert_eq!((t[2].note, t[2].velocity), (36, 127));
    assert_eq!((t[3].note, t[3].velocity), (39, 40));
}

#[test]
fn zones_reach_halfway_and_the_ends_stretch_to_the_whole_keyboard() {
    let z = Plan::default().zones();
    assert_eq!(
        z[0],
        Keys {
            lo: 0,
            hi: 37,
            root: 36
        }
    );
    assert_eq!(
        z[1],
        Keys {
            lo: 38,
            hi: 40,
            root: 39
        }
    );
    assert_eq!(
        z[16],
        Keys {
            lo: 83,
            hi: 127,
            root: 84
        }
    );
    for w in z.windows(2) {
        assert_eq!(w[0].hi + 1, w[1].lo, "every key is covered once");
    }
    // Four apart: the upper note takes the key in the middle.
    let p = Plan {
        lo: 36,
        hi: 40,
        every: 4,
        ..Plan::default()
    };
    assert_eq!(
        p.zones(),
        [
            Keys {
                lo: 0,
                hi: 37,
                root: 36
            },
            Keys {
                lo: 38,
                hi: 127,
                root: 40
            }
        ]
    );
    // A highest note off the stride isn't sampled.
    let p = Plan {
        lo: 36,
        hi: 41,
        every: 3,
        ..Plan::default()
    };
    assert_eq!(p.notes(), [36, 39]);
}

#[test]
fn velocity_layers_answer_from_the_layer_below_up_to_their_own() {
    let p = Plan::default();
    assert_eq!(p.velocity_ranges(), [(1, 40), (41, 90), (91, 127)]);
    let one = Plan {
        velocities: vec![64],
        ..Plan::default()
    };
    assert_eq!(one.velocity_ranges(), [(1, 127)]);
    assert_eq!(
        Plan::spread_velocities(4, VelocityCurve::Linear),
        [32, 64, 95, 127]
    );
    let exp = Plan::spread_velocities(4, VelocityCurve::Exp);
    let log = Plan::spread_velocities(4, VelocityCurve::Log);
    assert!(exp[0] < 32 && log[0] > 32, "{exp:?} {log:?}");
    assert_eq!(exp[3], 127);
    let eight = Plan::spread_velocities(8, VelocityCurve::Exp);
    assert!(eight.windows(2).all(|w| w[0] < w[1]), "{eight:?}");
}

#[test]
fn plans_past_a_pads_layers_or_the_memory_budget_are_refused() {
    let bad = |p: Plan| p.check().unwrap_err();
    assert_eq!(
        bad(Plan {
            velocities: Plan::spread_velocities(8, VelocityCurve::Linear),
            round_robins: 4,
            ..Plan::default()
        }),
        PlanError::TooManyLayers
    );
    assert_eq!(
        bad(Plan {
            velocities: vec![90, 40],
            ..Plan::default()
        }),
        PlanError::Velocities
    );
    assert_eq!(
        bad(Plan {
            every: 0,
            ..Plan::default()
        }),
        PlanError::Notes
    );
    assert_eq!(
        bad(Plan {
            hold: f32::NAN,
            ..Plan::default()
        }),
        PlanError::Times
    );
    let big = Plan {
        lo: 0,
        hi: 127,
        every: 1,
        velocities: Plan::spread_velocities(8, VelocityCurve::Linear),
        round_robins: 2,
        hold: 10.0,
        ..Plan::default()
    };
    let e = bad(big);
    assert!(matches!(e, PlanError::TooBig { .. }));
    assert!(e.to_string().contains("MB"), "{e}");
    assert!(Session::new(
        Plan {
            every: 13,
            ..Plan::default()
        },
        1
    )
    .is_err());
}

#[test]
fn a_session_records_every_note_and_builds_keygroups_that_play_them() {
    let mut synth = Fake::new(300, 80);
    let rec = record(small(), &mut synth, 256);
    assert!(rec.complete && !rec.clipped);
    let latency = rec.latency.unwrap();
    assert!((300..=381).contains(&latency), "{latency}");
    assert_eq!(rec.takes.len(), 6);

    let inst = build(&rec).unwrap();
    assert_eq!(inst.pads.len(), 3);
    assert_eq!(inst.sources.len(), 6);
    assert_eq!(inst.from, 48);
    assert_eq!(inst.looped, 0);
    // Every take starts 2 ms before its own attack, however late it came.
    let pre = (0.002 * RATE as f32).round() as usize;
    for s in &inst.sources {
        let at = first_over(s.data(), 2, peak(s.data()) * 0.01);
        assert!(at.abs_diff(pre) <= 2, "{}: attack at {at}", s.name);
    }
    let p = &inst.pads[1];
    assert_eq!(p.name, "Fake C4");
    assert_eq!(
        p.keys,
        Some(Keys {
            lo: 54,
            hi: 65,
            root: 60
        })
    );
    assert_eq!(p.polyphony, KEYGROUP_POLYPHONY);
    assert_eq!(p.select, LayerSelect::All);
    assert_eq!(p.layers.len(), 2);
    assert_eq!((p.layers[0].vel_lo, p.layers[0].vel_hi), (1, 64));
    assert_eq!((p.layers[1].vel_lo, p.layers[1].vel_hi), (65, 127));
    assert_eq!(p.layers[0].region.mode, PlayMode::Gate);
    assert_eq!(inst.pads[0].keys.map(|k| k.lo), Some(0));
    assert_eq!(inst.pads[2].keys.map(|k| k.hi), Some(127));

    let mut project = Project::new();
    inst.put(&mut project, 0).unwrap();
    assert_eq!(project.sources().len(), 6);
    assert_eq!(project.pads_play(), PadsPlay::Keys { from: 48 });
    assert_eq!(project.keygroups_for(62), [1]);
    assert_eq!(project.keygroups_for(20), [0]);

    // D4 plays the C4 take two semitones up.
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(&project, RATE));
    e.apply(Event::Key {
        note: 62,
        velocity: 127,
    });
    let (mut l, mut r) = (vec![0.0; 12_000], vec![0.0; 12_000]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    let (f, _) = yin(&l[1000..9192], RATE, 4096, 40.0, 3000.0, 0.15).expect("pitched");
    assert!((f - hz(62)).abs() < hz(62) * 0.003, "{f}");

    // One undo takes the whole instrument back.
    assert!(project.undo());
    assert!((0..PADS).all(|i| project.pad(i).is_none()));
    assert!(project.sources().is_empty());
    assert_eq!(project.pads_play(), PadsPlay::Pads);
    assert!(project.redo());
    assert_eq!(project.keygroups_for(62), [1]);
}

#[test]
fn each_note_ends_where_it_went_silent_not_at_the_longest_tail() {
    let mut synth = Fake::new(200, 0);
    let plan = small();
    let rec = record(plan.clone(), &mut synth, 128);
    let hold = (plan.hold * RATE as f32) as usize;
    let most = hold + (plan.tail * RATE as f32) as usize;
    for t in &rec.takes {
        let n = t.audio.len() / 2;
        // 200 frames late, 0.2 s release, 0.1 s of quiet: well short of 2 s.
        assert!(n < hold + RATE as usize / 2, "{n}");
        assert!(n < most);
    }
    let inst = build(&rec).unwrap();
    for (pad, s) in inst.pads.iter().zip(inst.sources.chunks(2)) {
        for s in s {
            let n = s.frames() as usize;
            // The release is about 0.2 s to -60 dB; the take keeps most of it.
            assert!(
                n > hold + RATE as usize / 10 && n < hold + RATE as usize / 3,
                "{n}"
            );
        }
        // Lifting a key fades it like the synth's release.
        assert!(
            (100.0..300.0).contains(&pad.amp.release_ms),
            "{}",
            pad.amp.release_ms
        );
    }
}

#[test]
fn the_same_session_records_the_same_takes_whatever_the_block_size() {
    let a = record(small(), &mut Fake::new(150, 40), 64);
    let b = record(small(), &mut Fake::new(150, 40), 1000);
    assert_eq!(a.takes, b.takes);
    assert_eq!(a.latency, b.latency);
}

#[test]
fn loops_are_found_in_a_held_tone_and_left_out_of_a_decaying_one() {
    let plan = Plan {
        hold: 1.0,
        loops: Some(LoopMode::Forward),
        ..small()
    };
    let rec = record(plan.clone(), &mut Fake::new(120, 30), 256);
    let inst = build(&rec).unwrap();
    assert_eq!(inst.looped, 6);
    for (s, layer) in inst
        .sources
        .iter()
        .zip(inst.pads.iter().flat_map(|p| &p.layers))
    {
        let r = &layer.region;
        assert_eq!(r.mode, PlayMode::Loop);
        r.check(s).unwrap();
        let l = r.sustain_loop.unwrap();
        assert_eq!(l.mode, LoopMode::Forward);
        assert!(l.crossfade > 0);
        // A long loop inside the held part, its ends matching.
        assert!(l.end - l.start > u64::from(RATE) / 4, "{l:?}");
        assert!(l.end < (1.0 * RATE as f32) as u64);
        let x = s.data();
        let (a, b) = (l.start as usize * 2, l.end as usize * 2);
        let diff: f32 = (0..400).map(|k| (x[a + k] - x[b + k]).abs()).sum::<f32>() / 400.0;
        assert!(diff < 0.02 * peak(x), "{diff}");
    }
    let mut project = Project::new();
    inst.put(&mut project, 0).unwrap();
    // Held past its take, C4 keeps sounding in its loop; let go, it rings out.
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(&project, RATE));
    e.apply(Event::Key {
        note: 60,
        velocity: 127,
    });
    let level = |e: &mut Engine, seconds: f32| {
        let n = (seconds * RATE as f32) as usize;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
            e.render(a, b);
        }
        peak(&l[n - 4410..])
    };
    let held = level(&mut e, 2.5);
    assert!(held > 0.1, "{held}");
    e.apply(Event::KeyOff { note: 60 });
    assert!(level(&mut e, 1.0) < held * 0.01);

    // A pluck dies away while held: nothing loops.
    let mut pluck = Fake::new(120, 30);
    pluck.decay = Some(0.25);
    let rec = record(plan, &mut pluck, 256);
    let inst = build(&rec).unwrap();
    assert_eq!(inst.looped, 0);
    assert!(inst
        .pads
        .iter()
        .flat_map(|p| &p.layers)
        .all(|l| l.region.sustain_loop.is_none() && l.region.mode == PlayMode::Gate));
}

#[test]
fn round_robins_take_turns_on_the_pad() {
    let plan = Plan {
        velocities: vec![127],
        round_robins: 2,
        ..small()
    };
    let rec = record(plan, &mut Fake::new(100, 20), 256);
    let inst = build(&rec).unwrap();
    let p = &inst.pads[0];
    assert_eq!(p.select, LayerSelect::Cycle);
    assert_eq!(p.layers.len(), 2);
    assert!(p.layers.iter().all(|l| (l.vel_lo, l.vel_hi) == (1, 127)));
    assert_ne!(p.layers[0].region.source, p.layers[1].region.source);
    assert!(
        inst.sources[1].name.ends_with("rr2"),
        "{}",
        inst.sources[1].name
    );
}

#[test]
fn normalizing_puts_the_loudest_take_at_minus_12_db_and_keeps_the_layers_apart() {
    let rec = record(small(), &mut Fake::new(100, 0), 256);
    let inst = build(&rec).unwrap();
    let loudest = inst
        .sources
        .iter()
        .map(|s| peak(s.data()))
        .fold(0.0, f32::max);
    let after = 20.0 * loudest.log10() + inst.gain_db;
    assert!((after - NORMALIZE_TO_DB).abs() < 0.01, "{after}");
    let p = &inst.pads[0];
    assert!(p.layers.iter().all(|l| l.region.gain_db == inst.gain_db));
    // The soft layer is quieter as recorded; a hit at its own velocity
    // plays it as recorded.
    let soft = peak(inst.sources[0].data());
    let hard = peak(inst.sources[1].data());
    assert!(soft < hard * 0.5);
    let v: f32 = 64.0 / 127.0;
    assert!((p.layers[0].level_db + 40.0 * v.log10()).abs() < 1e-4);
    assert_eq!(p.layers[1].level_db, 0.0);
    let off = Plan {
        normalize: false,
        ..small()
    };
    let rec = record(off, &mut Fake::new(100, 0), 256);
    assert_eq!(build(&rec).unwrap().gain_db, 0.0);
}

#[test]
fn a_session_that_hears_nothing_stops_after_the_level_test() {
    let mut synth = Fake::new(100, 0);
    synth.level = 0.0;
    let mut s = Session::new(small(), 1).unwrap();
    run(&mut s, &mut synth, 256);
    assert_eq!(s.state(), State::NoSound);
    assert_eq!(s.progress().done, 0);
    let rec = s.finish();
    assert!(!rec.complete && rec.takes.is_empty());
    assert_eq!(build(&rec).unwrap_err(), BuildError::Empty);
}

#[test]
fn a_cancelled_session_keeps_the_notes_it_finished() {
    let mut synth = Fake::new(100, 0);
    let mut s = Session::new(small(), 1).unwrap();
    assert_eq!(s.progress().total, 6);
    while s.progress().done < 3 {
        let cues = s.cues(256).to_vec();
        let audio = synth.render(256, &cues, 2);
        s.feed(&audio);
    }
    assert_eq!(s.progress().note, Some(60));
    assert_eq!(s.cancel(), Cue::AllOff);
    assert_eq!(s.state(), State::Cancelled);
    assert!(s.cues(256).is_empty(), "nothing more is sent");
    let rec = s.finish();
    assert!(!rec.complete);
    assert_eq!(rec.takes.len(), 3);
    // C3 has both its layers; C4 only one, so it is left out.
    let inst = build(&rec).unwrap();
    assert_eq!(inst.pads.len(), 1);
    assert_eq!(
        inst.pads[0].keys,
        Some(Keys {
            lo: 0,
            hi: 127,
            root: 48
        })
    );
}

#[test]
fn clipping_is_flagged_and_nothing_stops_for_it() {
    let mut synth = Fake::new(100, 0);
    synth.level = 2.0;
    let rec = record(small(), &mut synth, 256);
    assert!(rec.clipped && rec.complete);
    let inst = build(&rec).unwrap();
    assert!(inst.gain_db < -12.0);
}

#[test]
fn an_instrument_that_doesnt_fit_changes_nothing() {
    let rec = record(small(), &mut Fake::new(100, 0), 256);
    let inst = build(&rec).unwrap();
    let mut project = Project::new();
    assert_eq!(
        inst.clone().put(&mut project, PADS - 2),
        Err(ModelError::BadPad)
    );
    assert!(project.sources().is_empty() && !project.can_undo());
    inst.put(&mut project, PADS - 3).unwrap();
    assert_eq!(project.keygroups_for(60), [PADS - 2]);
}

#[test]
fn mono_plans_record_one_channel() {
    let plan = Plan {
        channels: 1,
        ..small()
    };
    let rec = record(plan, &mut Fake::new(100, 0), 256);
    let inst = build(&rec).unwrap();
    assert!(inst.sources.iter().all(|s| s.channels() == 1));
    assert!(inst.bytes() > 0);
}
