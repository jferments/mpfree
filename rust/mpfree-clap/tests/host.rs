//! The CLAP host with MPFree's own test plugins: once linked in, and once
//! loaded from the real `.clap` file with `dlopen`.

use mpfree_clap::testplug::{self, gain_params, synth_params, GAIN_ID, SYNTH_ID};
use mpfree_clap::{request, ClapNode, Error, Library};
use mpfree_node::{Ctx, Curve, Expr, Kind, Node, NodeEvent, Timed};
use std::path::PathBuf;

const RATE: u32 = 44_100;

fn lib() -> Library {
    Library::from_entry(&testplug::ENTRY, "linked").expect("test entry")
}

fn synth(max_block: usize) -> ClapNode {
    let mut n = lib().create(SYNTH_ID).expect("test synth");
    n.prepare(RATE, max_block);
    n
}

/// The test plugins' shared library, which Cargo builds with this test:
/// next to it (target/debug/deps, or target/debug when it is a workspace
/// member) or, from Cargo 1.100, in its own build folder.
fn plugin_file() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let name = format!(
        "{}mpfree_clap_testplug{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let here = exe.parent().expect("test folder");
    let mut dirs = vec![here.to_path_buf(), here.join("..")];
    // Cargo 1.100 builds each unit in build/PACKAGE/HASH/out, so the
    // library is in its own package's folders there; take the newest.
    if let Some(build) = here.ancestors().nth(3).filter(|b| b.ends_with("build")) {
        if let Ok(rd) = std::fs::read_dir(build.join("mpfree-clap-testplug")) {
            dirs.extend(rd.flatten().map(|e| e.path().join("out")));
        }
    }
    dirs.iter()
        .map(|d| d.join(&name))
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
        .unwrap_or_else(|| panic!("{name} not built next to {}", exe.display()))
}

fn index(n: &ClapNode, clap: u32) -> u16 {
    n.param_index(clap).expect("param")
}

/// Renders `blocks` blocks of `block` frames with `events` (absolute frames).
fn render(
    n: &mut ClapNode,
    events: &[(u64, NodeEvent)],
    frames: usize,
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0f32; frames], vec![0f32; frames]);
    let mut at = 0usize;
    let mut buf = Vec::new();
    while at < frames {
        let len = block.min(frames - at);
        buf.clear();
        for &(f, e) in events {
            if f >= at as u64 && f < (at + len) as u64 {
                buf.push(Timed::new((f - at as u64) as u32, e));
            }
        }
        let c = Ctx {
            bpm: 97.0,
            ..Ctx::new(RATE, at as u64)
        };
        n.process(&c, &buf, &mut l[at..at + len], &mut r[at..at + len]);
        at += len;
    }
    (l, r)
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

#[test]
fn a_library_lists_its_plugins_and_refuses_unknown_ones() {
    let l = lib();
    let p = l.plugins();
    assert_eq!(p.len(), 2);
    assert_eq!(
        (p[0].id.as_str(), p[0].name.as_str(), p[0].vendor.as_str()),
        (SYNTH_ID, "MPFree Test Synth", "MPFree")
    );
    assert_eq!(p[0].features, vec!["instrument", "synthesizer", "stereo"]);
    assert_eq!(p[1].features, vec!["audio-effect", "stereo"]);
    assert_eq!(
        l.create("org.nobody.nothing").err(),
        Some(Error::NoPlugin("org.nobody.nothing".into()))
    );
}

#[test]
fn the_synth_is_an_instrument_with_its_parameters_mapped() {
    let n = synth(256);
    assert!(n.is_active());
    assert_eq!(n.kind(), Kind::Instrument);
    assert_eq!(n.info().id, "clap:org.mpfree.test-synth");
    assert_eq!(n.info().name, "MPFree Test Synth");
    let p = n.params();
    // The hidden parameter is left out; ids are indices.
    assert_eq!(
        p.iter().map(|p| p.name).collect::<Vec<_>>(),
        vec!["Gain", "Decay", "Wave", "Voices", "Tempo"]
    );
    assert_eq!(
        p.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    assert_eq!(
        (p[0].group, p[0].min, p[0].max, p[0].default),
        ("Amp", -60.0, 6.0, -12.0)
    );
    assert_eq!(p[2].curve, Curve::Choice(&["Sine", "Saw", "Square"]));
    assert_eq!(p[3].curve, Curve::Stepped);
    assert!(p[3].readonly && p[4].readonly && !p[0].readonly);
    mpfree_node::check_params(p).expect("valid descriptions");
    assert_eq!(n.clap_param_id(1), Some(synth_params::DECAY));
    assert_eq!(n.param_index(synth_params::WAVE), Some(2));
    assert_eq!(n.param_index(synth_params::SECRET), None);
    // Values came from the plugin.
    assert_eq!((n.param(0), n.param(1), n.param(2)), (-12.0, 300.0, 0.0));
    assert!(n.param(99).is_nan());
    // Two instances of one plugin share their descriptions.
    let m = synth(256);
    assert!(std::ptr::eq(n.params(), m.params()));
    // The plugin said hello through the host's log.
    assert_eq!(n.logs(), (1, 0));
}

#[test]
fn a_note_adds_into_the_buffers_and_ends_with_its_release() {
    let mut n = synth(256);
    let (mut l, mut r) = (vec![0.25f32; 256], vec![0.25f32; 256]);
    n.process(
        &Ctx::new(RATE, 0),
        &[Timed::new(
            10,
            NodeEvent::NoteOn {
                note: 69,
                velocity: 127,
            },
        )],
        &mut l,
        &mut r,
    );
    assert!(
        l[..10].iter().all(|&x| x == 0.25),
        "nothing before the note"
    );
    assert!(
        l[10..].iter().any(|&x| (x - 0.25).abs() > 0.01),
        "the note adds on top"
    );
    // Off, and it dies away (four times faster than held).
    let ev = [(0, NodeEvent::NoteOff { note: 69 })];
    let (l, _) = render(&mut n, &ev, RATE as usize, 256);
    assert!(
        peak(&l[RATE as usize - 256..]) < 1e-4,
        "quiet within a second of the release"
    );
    assert_eq!(n.stats().errors, 0);
}

#[test]
fn the_same_events_sound_the_same_in_any_block_size() {
    let events = [
        (
            0,
            NodeEvent::NoteOn {
                note: 60,
                velocity: 100,
            },
        ),
        (
            333,
            NodeEvent::NoteOn {
                note: 67,
                velocity: 80,
            },
        ),
        (700, NodeEvent::Param { id: 2, value: 1.0 }),
        (
            1500,
            NodeEvent::Pressure {
                note: 60,
                value: 90,
            },
        ),
        (2100, NodeEvent::Bend { value: 0.5 }),
        (2600, NodeEvent::NoteOff { note: 60 }),
        (3000, NodeEvent::AllOff),
    ];
    let want = render(&mut synth(1024), &events, 4096, 1024);
    assert!(peak(&want.0) > 0.01);
    for block in [1, 7, 64, 100, 1000] {
        let got = render(&mut synth(1024), &events, 4096, block);
        assert_eq!(got, want, "block {block}");
    }
    // Blocks longer than the plugin was activated for are cut into pieces.
    let got = render(&mut synth(64), &events, 4096, 1000);
    assert_eq!(got, want, "max block 64, blocks of 1000");
}

#[test]
fn a_notes_slide_goes_in_its_tuning_expression() {
    let on = |note| NodeEvent::NoteOn {
        note,
        velocity: 100,
    };
    let slide = |note, value| NodeEvent::Expression {
        note,
        kind: Expr::Slide,
        value,
    };
    let run = |events: &[(u64, NodeEvent)]| render(&mut synth(512), events, 24_000, 256);
    let plain = run(&[(0, on(60))]);
    // A slide sent before the note starts it there, as one sent just after.
    let before = run(&[(0, slide(60, 2.0)), (0, on(60))]);
    let after = run(&[(0, on(60)), (0, slide(60, 2.0))]);
    assert!(peak(&before.0) > 0.01);
    assert_eq!(before, after, "slide before the note");
    assert_ne!(before, plain, "the slide moved it");
    // Part way: the same until the slide, different after it.
    let part = run(&[(0, on(60)), (1000, slide(60, 2.0))]);
    assert_eq!(part.0[..1000], plain.0[..1000]);
    assert_ne!(part.0[1000..], plain.0[1000..]);
    // Another note's slide does nothing; a note-off clears the slide.
    let notes = [
        (0, on(60)),
        (500, NodeEvent::NoteOff { note: 60 }),
        (20_000, on(60)),
    ];
    let mut busy = vec![(0, slide(61, 5.0)), (0, slide(60, 3.0))];
    busy.extend(notes);
    busy.sort_by_key(|e| e.0);
    let d = run(&busy);
    let e = run(&notes);
    assert_ne!(d.0[..500], e.0[..500]);
    // (Once the first note's release has died away.)
    assert_eq!(d.0[20_000..], e.0[20_000..], "cleared at the note-off");
}

#[test]
fn a_parameter_lands_on_its_frame() {
    let mut a = synth(512);
    let mut b = synth(512);
    let gain = index(&a, synth_params::GAIN);
    let on = (
        0,
        NodeEvent::NoteOn {
            note: 69,
            velocity: 127,
        },
    );
    let (la, _) = render(&mut a, &[on], 512, 512);
    let (lb, _) = render(
        &mut b,
        &[
            on,
            (
                200,
                NodeEvent::Param {
                    id: gain,
                    value: -6.0,
                },
            ),
        ],
        512,
        512,
    );
    assert_eq!(la[..200], lb[..200]);
    // -12 dB to -6 dB is twice as loud from frame 200.
    for i in 200..512 {
        assert!((lb[i] - la[i] * 1.995_262_3).abs() < 1e-5, "frame {i}");
    }
    assert_eq!(b.param(gain), -6.0);
    // set_param between blocks lands on the next block's first frame.
    b.set_param(gain, 100.0);
    assert_eq!(b.param(gain), 6.0, "clamped to the range");
    let mut l = vec![0f32; 64];
    let mut r = vec![0f32; 64];
    b.process(&Ctx::new(RATE, 512), &[], &mut l, &mut r);
    // A change queued before a prepare survives it and still lands.
    b.set_param(gain, -30.0);
    b.prepare(RATE, 512);
    assert_eq!(b.param(gain), -30.0);
    b.process(&Ctx::new(RATE, 576), &[], &mut l, &mut r);
    assert_eq!(
        b.control().param_text(gain, b.param(gain)).as_deref(),
        Some("-30")
    );
    // Read-only parameters can't be set.
    // (The prepare reactivated the synth, so no voice sounds now.)
    b.set_param(index(&b, synth_params::VOICES), 9.0);
    assert_eq!(b.param(index(&b, synth_params::VOICES)), 0.0);
}

#[test]
fn values_the_plugin_sends_back_reach_param() {
    let mut n = synth(256);
    let voices = index(&n, synth_params::VOICES);
    let tempo = index(&n, synth_params::TEMPO);
    let ev = [
        (
            0,
            NodeEvent::NoteOn {
                note: 60,
                velocity: 100,
            },
        ),
        (
            5,
            NodeEvent::NoteOn {
                note: 64,
                velocity: 100,
            },
        ),
    ];
    render(&mut n, &ev, 256, 256);
    assert_eq!(n.param(voices), 2.0);
    assert_eq!(
        n.param(tempo),
        97.0,
        "the transport's tempo reached the plugin"
    );
}

#[test]
fn state_saves_and_loads_into_another_instance() {
    let mut a = synth(256);
    a.set_param(index(&a, synth_params::DECAY), 1234.0);
    a.set_param(index(&a, synth_params::WAVE), 2.0);
    // Parameter changes reach the plugin with the next block.
    render(&mut a, &[], 64, 64);
    let saved = a.control().save_state().expect("state");
    let b = synth(256);
    let c = b.control();
    assert!(c.load_state(&saved));
    assert_eq!((b.param(1), b.param(2)), (1234.0, 2.0));
    assert!(!c.load_state(b"junk"));
    assert_eq!(c.param_text(2, 1.0).as_deref(), Some("Saw"));
    assert_eq!(c.param_text(1, 300.0).as_deref(), Some("300"));
    // The plugin's tail follows its decay: -100 dB at 1234 ms per -60 dB.
    assert_eq!(a.tail_frames(), (1.234 * 100.0 / 60.0 * RATE as f64) as u32);
}

#[test]
fn the_gain_is_an_effect_and_a_failed_block_stays_dry() {
    let mut g = lib().create(GAIN_ID).expect("gain");
    g.prepare(RATE, 128);
    assert_eq!(g.kind(), Kind::Effect);
    assert_eq!(g.info().id, "clap:org.mpfree.test-gain");
    let gain = index(&g, gain_params::GAIN);
    g.set_param(gain, 20.0 * 2f32.log10());
    let (mut l, mut r) = (vec![0.1f32; 128], vec![-0.2f32; 128]);
    // Notes mean nothing to an effect.
    g.process(
        &Ctx::new(RATE, 0),
        &[Timed::new(
            0,
            NodeEvent::NoteOn {
                note: 60,
                velocity: 1,
            },
        )],
        &mut l,
        &mut r,
    );
    assert!(l.iter().all(|&x| (x - 0.2).abs() < 1e-6) && r.iter().all(|&x| (x + 0.4).abs() < 1e-6));
    g.set_param(index(&g, gain_params::FAIL), 1.0);
    let (mut l, mut r) = (vec![0.1f32; 128], vec![-0.2f32; 128]);
    g.process(&Ctx::new(RATE, 128), &[], &mut l, &mut r);
    assert!(
        l.iter().all(|&x| x == 0.1) && r.iter().all(|&x| x == -0.2),
        "dry on an error"
    );
    assert_eq!(g.stats().errors, 1);
}

#[test]
fn the_plugin_file_loads_with_dlopen_and_sounds_the_same() {
    let path = plugin_file();
    let file = Library::open(&path).expect("dlopen the test plugin file");
    assert_eq!(file.plugins().len(), 2);
    let mut a = file.create(SYNTH_ID).expect("synth from the file");
    a.prepare(RATE, 256);
    let ev = [
        (
            0,
            NodeEvent::NoteOn {
                note: 57,
                velocity: 110,
            },
        ),
        (900, NodeEvent::NoteOff { note: 57 }),
    ];
    let from_file = render(&mut a, &ev, 2048, 256);
    let linked = render(&mut synth(256), &ev, 2048, 256);
    assert_eq!(from_file, linked);
    assert!(Library::open(std::path::Path::new("/nonexistent/x.clap")).is_err());
    // A shared library without clap_entry is refused.
    let libc_path = std::path::Path::new("libc.so.6");
    assert_eq!(Library::open(libc_path).err(), Some(Error::NoEntry));
}

#[test]
fn a_callback_request_reaches_idle() {
    let n = synth(256);
    let c = n.control();
    // SAFETY: n is one of our test plugins and alive.
    assert_eq!(unsafe { testplug::main_thread_calls(n.raw_plugin()) }, 0);
    // The test synth asks for a callback when it is activated.
    assert_eq!(c.idle() & request::CALLBACK, request::CALLBACK);
    assert_eq!(unsafe { testplug::main_thread_calls(n.raw_plugin()) }, 1);
    assert_eq!(c.idle(), 0, "answered once");
    assert_eq!(n.misplaced_calls(), 0);
}
