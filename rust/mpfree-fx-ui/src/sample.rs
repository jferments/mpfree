//! Sample screens for the workstation preview and the tests: a real effect
//! behind each page, set the way a player might have it.

use crate::lock::Locks;
use crate::perform::{self, PerformPanel};
use crate::state::{make, FxState};
use crate::view::{self, FxPanel};
use mpfree_fx::ids::{
    character, chorus, cloud, comb, compressor as comp, crush, delay, dj_filter as dj, drive,
    ducker, eq, filter, flanger, harmonizer as harm, master, perform as pf, phaser, pitch,
    pitch_correct as tune, transient, tuner,
};
use mpfree_fx::{perform_state, presets};
use mpfree_ui::widgets::{Chip, Header, Label};
use mpfree_ui::{Align, EventKind, Fonts, Theme, Tone, Ui, UiEvent};

pub const WIDTH: i32 = 1280;
pub const HEIGHT: i32 = 800;
/// The panel's page in the samples.
pub const PAGE: &str = "Effect";
pub const BPM: f64 = 120.0;
pub const RATE: u32 = 48_000;

pub struct Sample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
}

/// A Ui with the app's own widgets (`header`, `hint`) as the panels
/// expect. No tab is lit: an effect's page opens over the page it came
/// from, as the mixer does.
pub fn app_ui(fonts: Fonts, hint: &str) -> Ui {
    let mut ui = Ui::new(Theme::dark(), fonts, WIDTH, HEIGHT);
    let mut header = Header::app("");
    header.chips = vec![
        Chip::new("120.0 BPM", Tone::Dim),
        Chip::new("DSP 21 %", Tone::Good),
    ];
    ui.add("header", header);
    ui.add("hint", Label::new(hint, Tone::Dim, Align::Center));
    ui
}

const HINT: &str =
    "Drag a tile to turn it · tap a tile, then the wheel · Q-Links turn the marked tiles · Back: the chain";

struct Scene {
    name: &'static str,
    about: &'static str,
    id: &'static str,
    place: &'static str,
    values: &'static [(u16, f32)],
    preset: Option<usize>,
    page: &'static str,
    focus: Option<u16>,
    /// The host's level meters for the slot (`FxState::levels`).
    levels: Option<(f32, f32)>,
    meters: &'static [(u16, f32)],
}

const SCENES: &[Scene] = &[
    Scene {
        name: "01-eq-bands",
        about: "EQ, Bands page: the response with a box per band in its Q-Link's colour, band 2 selected",
        id: "mpfree.fx.eq",
        place: "Drums · insert 1",
        values: &[
            (eq::LOW_CUT, 32.0),
            (eq::LOW_SLOPE, 2.0),
            (eq::gain(0), 3.5),
            (eq::freq(0), 70.0),
            (eq::gain(1), -4.5),
            (eq::freq(1), 380.0),
            (eq::q(1), 1.6),
            (eq::gain(2), 2.5),
            (eq::freq(2), 3200.0),
            (eq::gain(3), 3.0),
            (eq::freq(3), 11_000.0),
        ],
        preset: None,
        page: "Bands",
        focus: Some(eq::gain(1)),
        levels: None,
        meters: &[],
    },
    Scene {
        name: "02-eq-cuts",
        about: "EQ, Cuts page: the low and high cuts and the output",
        id: "mpfree.fx.eq",
        place: "Bass · insert 2",
        values: &[
            (eq::LOW_CUT, 45.0),
            (eq::LOW_SLOPE, 2.0),
            (eq::HIGH_CUT, 7000.0),
            (eq::HIGH_SLOPE, 1.0),
            (eq::gain(1), 2.0),
            (eq::freq(1), 120.0),
            (eq::OUTPUT, -1.5),
        ],
        preset: None,
        page: "Cuts",
        focus: None,
        levels: None,
        meters: &[],
    },
    Scene {
        name: "03-filter",
        about: "Multi filter: a resonant 24 dB low-pass, the dot at the cutoff",
        id: "mpfree.fx.filter",
        place: "Synth · insert 1",
        values: &[
            (filter::SLOPE, 1.0),
            (filter::CUTOFF, 1400.0),
            (filter::RESONANCE, 55.0),
            (filter::DRIVE, 6.0),
        ],
        preset: None,
        page: "Multi filter",
        focus: Some(filter::CUTOFF),
        levels: None,
        meters: &[],
    },
    Scene {
        name: "04-dj-filter",
        about: "DJ filter turned left: the low-pass falling, with resonance",
        id: "mpfree.fx.dj_filter",
        place: "Master · insert 1",
        values: &[(dj::POSITION, -55.0), (dj::RESONANCE, 45.0)],
        preset: None,
        page: "DJ filter",
        focus: None,
        levels: None,
        meters: &[],
    },
    Scene {
        name: "05-drive",
        about: "Drive, Tube with bias: the curve bends more one way, and a sine through it",
        id: "mpfree.fx.drive",
        place: "Bass · insert 1",
        values: &[(drive::MODE, 2.0), (drive::DRIVE, 15.0), (drive::BIAS, 45.0)],
        preset: None,
        page: "Drive",
        focus: Some(drive::DRIVE),
        levels: None,
        meters: &[(drive::LEVEL_CHANGE, 0.4)],
    },
    Scene {
        name: "06-comb",
        about: "Comb at A2 with negative feedback: odd harmonics only, and how long it rings",
        id: "mpfree.fx.comb",
        place: "Loop · insert 1",
        values: &[
            (comb::PITCH, 110.0),
            (comb::FEEDBACK, -90.0),
            (comb::DAMPING, 7000.0),
        ],
        preset: None,
        page: "Comb",
        focus: Some(comb::PITCH),
        levels: None,
        meters: &[],
    },
    Scene {
        name: "07-crush",
        about: "Crush at 4 bits and 4 kHz: the staircase a sine comes out as",
        id: "mpfree.fx.crush",
        place: "Drums · insert 3",
        values: &[(crush::BITS, 4.0), (crush::RATE, 4000.0)],
        preset: None,
        page: "Crush",
        focus: None,
        levels: None,
        meters: &[],
    },
    Scene {
        name: "08-delay",
        about: "Delay at 1/8D with ping-pong: each repeat at its time and level, left and right",
        id: "mpfree.fx.delay",
        place: "Return B",
        values: &[
            (delay::SYNC, 7.0),
            (delay::FEEDBACK, 62.0),
            (delay::PING_PONG, 1.0),
            (delay::MIX, 100.0),
        ],
        preset: None,
        page: "Delay",
        focus: Some(delay::FEEDBACK),
        levels: None,
        meters: &[],
    },
    Scene {
        name: "09-chorus",
        about: "Chorus with three voices: each voice's delay over two cycles",
        id: "mpfree.fx.chorus",
        place: "Synth · insert 2",
        values: &[(chorus::VOICES, 3.0), (chorus::DEPTH, 60.0)],
        preset: None,
        page: "Chorus",
        focus: None,
        levels: None,
        meters: &[],
    },
    Scene {
        name: "10-flanger",
        about: "Flanger: the sweep (left solid, right dashed) and the comb it makes",
        id: "mpfree.fx.flanger",
        place: "Drums · insert 2",
        values: &[(flanger::FEEDBACK, 70.0), (flanger::DEPTH, 70.0)],
        preset: None,
        page: "Flanger",
        focus: None,
        levels: None,
        meters: &[],
    },
    Scene {
        name: "11-phaser",
        about: "Phaser with 8 stages: the sweep and the notches at its middle (dashed: at its top)",
        id: "mpfree.fx.phaser",
        place: "Synth · insert 3",
        values: &[(phaser::STAGES, 3.0), (phaser::FEEDBACK, 55.0)],
        preset: None,
        page: "Phaser",
        focus: None,
        levels: None,
        meters: &[],
    },
    Scene {
        name: "12-lofi",
        about: "Lo-fi with the Dusty vinyl preset: the wear and the tone, presets on the right",
        id: "mpfree.fx.lofi",
        place: "Loop · insert 2",
        values: &[],
        preset: Some(1),
        page: "Lo-fi",
        focus: None,
        levels: None,
        meters: &[],
    },
    Scene {
        name: "13-compressor",
        about: "Compressor: the curve with the threshold, a burst in and out, the reduction now",
        id: "mpfree.fx.compressor",
        place: "Drums · insert 4",
        values: &[
            (comp::THRESHOLD, -24.0),
            (comp::RATIO, 4.0),
            (comp::ATTACK, 8.0),
            (comp::RELEASE, 120.0),
            (comp::MAKEUP, 6.0),
        ],
        preset: None,
        page: "Compressor",
        focus: Some(comp::THRESHOLD),
        levels: None,
        meters: &[(comp::REDUCTION, -4.5)],
    },
    Scene {
        name: "14-reverb",
        about: "Cloud reverb, Big hall on Return A: the decay (pre-delay, early taps, tail) and tone",
        id: "mpfree.fx.cloud",
        place: "Return A",
        values: &[],
        preset: Some(2),
        page: "Reverb",
        focus: Some(cloud::DECAY),
        levels: None,
        meters: &[],
    },
    Scene {
        name: "15-reverb-tone",
        about: "Cloud reverb, Tone page: the cuts, and the damping's loss each pass (dashed)",
        id: "mpfree.fx.cloud",
        place: "Return A",
        values: &[(cloud::DAMPING, 4500.0), (cloud::LOW_CUT, 160.0)],
        preset: Some(2),
        page: "Tone",
        focus: None,
        levels: None,
        meters: &[],
    },
    Scene {
        name: "16-master",
        about: "Master safety in Soft clip: the curve and ceiling, and what it caught",
        id: "mpfree.fx.master_safety",
        place: "Master",
        values: &[(master::MODE, 1.0), (master::CEILING, -0.3)],
        preset: None,
        page: "Master safety",
        focus: None,
        levels: None,
        meters: &[
            (master::REDUCTION, -2.1),
            (master::OVERS, 37.0),
            (master::CLIPPED, 0.0),
        ],
    },
];

/// Character with its Pushed tape preset (after the lock scenes, so the
/// earlier screens keep their numbers).
const CHARACTER: Scene = Scene {
    name: "21-character",
    about: "Character with the Pushed tape preset: the tape curve, and a loud low sine held by it",
    id: "mpfree.fx.character",
    place: "Drums · insert 2",
    values: &[],
    preset: Some(3),
    page: "Character",
    focus: Some(character::AMOUNT),
    levels: None,
    meters: &[],
};

/// The effects added with the registry, after Character so the earlier
/// screens keep their numbers.
const NEW_FX: [Scene; 7] = [
    Scene {
        name: "22-ducker-tempo",
        about: "Ducker, Big pump preset: two quarter-note cycles of the pump at the song's tempo",
        id: "mpfree.fx.ducker",
        place: "Pads · insert 1",
        values: &[],
        preset: Some(1),
        page: "Ducker",
        focus: Some(ducker::RELEASE),
        levels: Some((-8.5, -19.0)),
        meters: &[(ducker::GAIN, -11.0)],
    },
    Scene {
        name: "23-ducker-key",
        about:
            "Ducker, Kick duck preset: the key over the threshold (dashed) and the duck it makes",
        id: "mpfree.fx.ducker",
        place: "Bass · insert 2",
        values: &[],
        preset: Some(0),
        page: "Ducker",
        focus: Some(ducker::DEPTH),
        levels: None,
        meters: &[],
    },
    Scene {
        name: "24-transient",
        about: "Transient shaper, Punchy kick preset: a hit's level in (dashed) and out",
        id: "mpfree.fx.transient",
        place: "Drums · insert 1",
        values: &[],
        preset: Some(0),
        page: "Transient shaper",
        focus: Some(transient::ATTACK),
        levels: Some((-14.0, -9.5)),
        meters: &[],
    },
    Scene {
        name: "25-pitch-stairs",
        about: "Pitch shifter, Rising stairs preset: each pass through the feedback a step higher",
        id: "mpfree.fx.pitch",
        place: "Keys · insert 3",
        values: &[],
        preset: Some(3),
        page: "Pitch shifter",
        focus: Some(pitch::FEEDBACK),
        levels: None,
        meters: &[],
    },
    Scene {
        name: "26-pitch-correct",
        about: "Pitch correct, A minor snap preset: the stairs onto the key's notes, \
                a voice on A3 pulled 12 cents up",
        id: "mpfree.fx.pitch_correct",
        place: "Vocal · insert 1",
        values: &[],
        preset: Some(2),
        page: "Pitch correct",
        focus: Some(tune::SPEED),
        levels: Some((-16.0, -16.5)),
        meters: &[(tune::NOTE, 57.0), (tune::CORRECTION, 12.0)],
    },
    Scene {
        name: "27-harmonizer",
        about: "Harmonizer, Close harmony preset: a 3rd above and a 3rd below, \
                major or minor as C major has them",
        id: "mpfree.fx.harmonizer",
        place: "Vocal · insert 2",
        values: &[],
        preset: Some(1),
        page: "Harmonizer",
        focus: Some(harm::VOICE1),
        levels: None,
        meters: &[(harm::NOTE, 64.0)],
    },
    Scene {
        name: "28-tuner",
        about: "Tuner on the line input: a bass's A string 7 cents flat",
        id: "mpfree.fx.tuner",
        place: "Input 1 · insert 1",
        values: &[],
        preset: None,
        page: "Tuner",
        focus: Some(tuner::REFERENCE),
        levels: Some((-18.0, -18.0)),
        meters: &[
            (tuner::NOTE, 45.0),
            (tuner::CENTS, -7.0),
            (tuner::HZ, 109.56),
        ],
    },
];

/// The effect `s` shows, with its values, as a page.
fn scene(s: &Scene, fonts: Fonts) -> Result<Sample, String> {
    let mut node = make(s.id).ok_or_else(|| format!("no effect {}", s.id))?;
    node.prepare(RATE, 512);
    if let Some(i) = s.preset {
        let p = presets(s.id)
            .get(i)
            .ok_or_else(|| format!("{}: no preset {i}", s.name))?;
        for &(id, v) in p.values {
            node.set_param(id, v);
        }
    }
    for &(id, v) in s.values {
        node.set_param(id, v);
    }
    let mut st = FxState::from_node(
        node.as_ref(),
        s.place,
        mpfree_ui::Color::rgb(0x3DDC97),
        BPM,
        RATE,
    );
    st.preset = s.preset;
    st.edited = s.preset.is_some() && !s.values.is_empty();
    st.levels = s.levels;
    // Meters as the node would report them while music plays.
    for &(id, v) in s.meters {
        if let Some(i) = st.index(id) {
            st.values[i] = v;
        }
    }
    let mut ui = app_ui(fonts, HINT);
    let mut panel = FxPanel::add(&mut ui, PAGE, st)?;
    let page = panel
        .pages()
        .iter()
        .position(|p| p.name == s.page)
        .ok_or_else(|| format!("{}: no page {}", s.name, s.page))?;
    panel.show_page(&mut ui, page);
    if let Some(id) = s.focus {
        let k = panel.pages()[page]
            .params
            .iter()
            .position(|&i| panel.state.params[i].id == id)
            .ok_or("focus not on page")?;
        panel.event(
            &mut ui,
            &UiEvent {
                widget: view::TILES.to_string(),
                kind: EventKind::Select(k),
            },
        );
    }
    ui.show(PAGE)?;
    Ok(Sample {
        name: s.name,
        about: s.about,
        ui,
    })
}

const PERFORM_HINT: &str =
    "Hold a pad: its effect starts on the next 1/16 · drag the XY pad · Hold keeps it where you let go";

/// The Perform FX page while playing: pad 1 on, pad 9 waiting for the
/// grid, pad 7 fading out, pad 10 selected, a finger on the XY pad.
fn perform_scene(fonts: Fonts, held: bool) -> Result<Sample, String> {
    let mut node = make("mpfree.fx.perform").ok_or("no Perform FX")?;
    node.prepare(RATE, 512);
    let st = FxState::from_node(
        node.as_ref(),
        "Master · insert 1",
        mpfree_ui::Color::rgb(0x6EA8FE),
        BPM,
        RATE,
    );
    let mut ui = app_ui(fonts, PERFORM_HINT);
    let mut panel = PerformPanel::add(&mut ui, PAGE, st)?;
    let r = ui.rect_of(perform::XY).ok_or("no XY")?;
    let a = perform::XyView::area(r);
    if held {
        panel.set_meters(
            &mut ui,
            &[
                (pf::slot(0) + pf::STATE, perform_state::ON),
                (pf::slot(8) + pf::STATE, perform_state::WAITING),
                (pf::slot(6) + pf::STATE, perform_state::FADING),
            ],
        );
        // A finger down on pad 10 (selects it), then across the XY pad.
        let pads = ui.rect_of(perform::PADS).ok_or("no pads")?;
        let c = perform::PerformPads::cell(pads, 9);
        ui.show(PAGE)?;
        if let Some(ev) = ui.touch(1, mpfree_ui::Touch::Down, c.x + 20, c.y + 20) {
            panel.event(&mut ui, &ev);
        }
        let path = [
            (0.62, 0.15),
            (0.55, 0.3),
            (0.45, 0.42),
            (0.36, 0.52),
            (0.3, 0.6),
            (0.26, 0.66),
        ];
        for (k, (x, y)) in path.iter().enumerate() {
            let px = a.x + (x * a.w as f32) as i32;
            let py = a.bottom() - (y * a.h as f32) as i32;
            let phase = if k == 0 {
                mpfree_ui::Touch::Down
            } else {
                mpfree_ui::Touch::Move
            };
            if let Some(ev) = ui.touch(2, phase, px, py) {
                panel.event(&mut ui, &ev);
            }
        }
    } else {
        // Hold on, the finger gone: the pad stays where it was let go.
        ui.show(PAGE)?;
        let h = perform::XyView::hold_rect(r);
        if let Some(ev) = ui.touch(3, mpfree_ui::Touch::Down, h.x + 10, h.y + 10) {
            panel.event(&mut ui, &ev);
        }
        ui.touch(3, mpfree_ui::Touch::Up, h.x + 10, h.y + 10);
        for (k, (x, y)) in [(0.5, 0.0), (0.66, 0.2), (0.78, 0.35)].iter().enumerate() {
            let px = a.x + (x * a.w as f32) as i32;
            let py = a.bottom() - (y * a.h as f32) as i32;
            let phase = if k == 0 {
                mpfree_ui::Touch::Down
            } else {
                mpfree_ui::Touch::Move
            };
            if let Some(ev) = ui.touch(4, phase, px, py) {
                panel.event(&mut ui, &ev);
            }
        }
        let px = a.x + (0.78 * a.w as f32) as i32;
        let py = a.bottom() - (0.35 * a.h as f32) as i32;
        if let Some(ev) = ui.touch(4, mpfree_ui::Touch::Up, px, py) {
            panel.event(&mut ui, &ev);
        }
        panel.select(&mut ui, 11);
    }
    ui.show(PAGE)?;
    Ok(Sample {
        name: if held {
            "17-perform"
        } else {
            "18-perform-hold"
        },
        about: if held {
            "Perform FX: pad 1 on, pad 9 waiting for the grid, pad 7 fading, a finger on the XY pad"
        } else {
            "Perform FX with Hold on: the XY pad stays where the finger let go (high-pass and echo)"
        },
        ui,
    })
}

const LOCK_HINT: &str =
    "Step held: a turn locks it on the step · ERASE: the selected lock off · let go of the step to finish";

/// Lock mode: step 5 held on the filter (its cutoff and resonance locked,
/// the cutoff selected), and step 3 held on Perform FX (pads 1 and 6
/// locked on, the DJ filter locked to a high-pass).
fn lock_scenes(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let mut out = Vec::new();
    let filter_scene = SCENES
        .iter()
        .find(|s| s.name == "03-filter")
        .ok_or("no filter scene")?;
    let st = {
        let mut node = make("mpfree.fx.filter").ok_or("no filter")?;
        node.prepare(RATE, 512);
        for &(id, v) in filter_scene.values {
            node.set_param(id, v);
        }
        FxState::from_node(
            node.as_ref(),
            filter_scene.place,
            mpfree_ui::Color::rgb(0x3DDC97),
            BPM,
            RATE,
        )
    };
    let mut ui = app_ui(fonts(), LOCK_HINT);
    let mut panel = FxPanel::add(&mut ui, PAGE, st)?;
    let mut held = Locks::new("Step 5");
    held.values = vec![(filter::CUTOFF, 380.0), (filter::RESONANCE, 78.0)];
    panel.set_locks(&mut ui, Some(held));
    let k = panel.pages()[0]
        .params
        .iter()
        .position(|&i| panel.state.params[i].id == filter::CUTOFF)
        .ok_or("no cutoff tile")?;
    panel.event(
        &mut ui,
        &UiEvent {
            widget: view::TILES.to_string(),
            kind: EventKind::Select(k),
        },
    );
    ui.show(PAGE)?;
    out.push(Sample {
        name: "19-filter-lock",
        about: "Lock mode on the filter: step 5 held, its cutoff (380 Hz) and resonance locked and marked; the drawing shows what the step plays, the effect's own 1.4 kHz stays",
        ui,
    });

    let mut node = make("mpfree.fx.perform").ok_or("no Perform FX")?;
    node.prepare(RATE, 512);
    let st = FxState::from_node(
        node.as_ref(),
        "Master · insert 1",
        mpfree_ui::Color::rgb(0x6EA8FE),
        BPM,
        RATE,
    );
    let mut ui = app_ui(fonts(), LOCK_HINT);
    let mut panel = PerformPanel::add(&mut ui, PAGE, st)?;
    let mut held = Locks::new("Step 3");
    held.values = vec![
        (pf::hold(0), 1.0),
        (pf::hold(5), 1.0),
        (pf::DJ_FILTER, 45.0),
    ];
    panel.set_locks(&mut ui, Some(held));
    panel.select(&mut ui, 5);
    ui.show(PAGE)?;
    out.push(Sample {
        name: "20-perform-lock",
        about: "Lock mode on Perform FX: step 3 held, pads 1 and 6 locked on (the step plays their effects), the DJ filter locked to a high-pass",
        ui,
    });
    Ok(out)
}

/// Every sample screen. `fonts` makes a fresh set of fonts per screen.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let mut out = Vec::new();
    for s in SCENES {
        out.push(scene(s, fonts())?);
    }
    out.push(perform_scene(fonts(), true)?);
    out.push(perform_scene(fonts(), false)?);
    out.extend(lock_scenes(&fonts)?);
    out.push(scene(&CHARACTER, fonts())?);
    for s in &NEW_FX {
        out.push(scene(s, fonts())?);
    }
    Ok(out)
}
