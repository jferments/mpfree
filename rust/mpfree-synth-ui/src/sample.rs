//! Sample screens for the workstation preview and the tests: a real Poly
//! behind each page, and an example effect to show the generic page any
//! node (a hosted plugin, an FX unit) gets from its parameters alone.

use crate::view::{SynthPanel, SynthState};
use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed, Unit};
use mpfree_synth::fm_params::{self as fp, op};
use mpfree_synth::params as pp;
use mpfree_synth::params::Preset;
use mpfree_synth::wavetable::params as wtp;
use mpfree_synth::Wavetable;
use mpfree_synth::{
    Analog, Bass, Choir, Drawbar, Drums, Fm, Keys, Pluck, Poly, Squelch, Strings, Wind,
};
use mpfree_ui::qlink::QLinkColumn;
use mpfree_ui::widgets::{Chip, Header, Label};
use mpfree_ui::{Align, Color, Fonts, Theme, Tone, Ui};

pub const WIDTH: i32 = 1280;
pub const HEIGHT: i32 = 800;
/// The panel's page, under the pad player's own bar (its Synth tab lit).
pub const PAGE: &str = "Synth";

pub struct Sample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub panel: SynthPanel,
}

/// A Ui with the app's own widgets (`header`, `hint`) as the panel expects.
pub fn app_ui(fonts: Fonts, hint: &str) -> Ui {
    let mut ui = Ui::new(Theme::dark(), fonts, WIDTH, HEIGHT);
    let mut header = Header::app("Synth");
    header.chips = vec![
        Chip::new("92.0 BPM", Tone::Dim),
        Chip::new("DSP 18 %", Tone::Good),
    ];
    ui.add("header", header);
    ui.add("hint", Label::new(hint, Tone::Dim, Align::Center));
    ui.add(crate::view::QLINK_COLUMN, QLinkColumn::new());
    ui
}

/// Fills the Q-Link column with what the panel's Q-Links turn, as the app
/// does.
pub fn fill_qlinks(ui: &mut Ui, panel: &SynthPanel) {
    if let Some(q) = ui.get_mut::<QLinkColumn>(crate::view::QLINK_COLUMN) {
        q.title = "Q-Links · Synth".to_string();
        q.cards = panel.qlink_cards();
    }
}

pub fn poly_presets() -> Vec<String> {
    Poly::presets().iter().map(|p| p.name.to_string()).collect()
}

/// A Poly with preset `preset` loaded and `edits` applied after it.
pub fn poly(preset: usize, edits: &[(u16, f32)]) -> Poly {
    let mut poly = Poly::new(48_000);
    poly.load_preset(&Poly::presets()[preset]);
    for &(id, v) in edits {
        poly.set_param(id, v);
    }
    poly
}

const POLY_HINT: &str =
    "Drag a control to turn it · tap one, then the wheel · SHIFT + wheel: next page · pads play the Bass track";

struct Scene {
    name: &'static str,
    about: &'static str,
    preset: usize,
    edits: &'static [(u16, f32)],
    page: &'static str,
    focus: Option<u16>,
}

fn poly_scene(s: &Scene, fonts: Fonts) -> Result<Sample, String> {
    let node = poly(s.preset, s.edits);
    let mut st = SynthState::from_node(
        &node,
        "Bass",
        Color::rgb(0x3DDC97),
        poly_presets(),
        Some(s.preset),
    );
    st.edited = !s.edits.is_empty();
    let mut ui = app_ui(fonts, POLY_HINT);
    let mut panel = SynthPanel::add(&mut ui, PAGE, st)?;
    let page = panel
        .pages()
        .iter()
        .position(|p| p.name == s.page)
        .ok_or_else(|| format!("no page {}", s.page))?;
    panel.show_page(&mut ui, page);
    if let Some(id) = s.focus {
        let k = panel.state.index(id).ok_or("no such parameter")?;
        let ev = mpfree_ui::UiEvent {
            widget: crate::view::TILES.to_string(),
            kind: mpfree_ui::EventKind::Select(k),
        };
        panel.event(&mut ui, &ev);
    }
    fill_qlinks(&mut ui, &panel);
    ui.show(PAGE)?;
    Ok(Sample {
        name: s.name,
        about: s.about,
        ui,
        panel,
    })
}

/// Plays a four-note chord on `node` for 300 ms with three of its pads
/// pressed (hard, light, none, medium), so the Pressure page has live bars.
pub fn press_chord(node: &mut dyn Node) {
    let notes = [(60u8, 110u8), (64, 40), (67, 0), (72, 85)];
    let mut ev: Vec<Timed> = notes
        .iter()
        .map(|&(n, _)| {
            Timed::new(
                0,
                NodeEvent::NoteOn {
                    note: n,
                    velocity: 110,
                },
            )
        })
        .collect();
    let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
    for k in 0..56u64 {
        if k >= 8 {
            ev.extend(
                notes
                    .iter()
                    .map(|&(note, value)| Timed::new(0, NodeEvent::Pressure { note, value })),
            );
        }
        node.process(&Ctx::new(48_000, k * 256), &ev, &mut l, &mut r);
        ev.clear();
    }
}

/// Poly · Brass on its Pressure page, with a chord held and pressed.
fn poly_pressure_scene(fonts: Fonts) -> Result<Sample, String> {
    let i = preset("Brass");
    let mut node = poly(i, &[]);
    press_chord(&mut node);
    let mut out = [0.0; 16];
    let n = node.pressures(&mut out);
    let mut st =
        SynthState::from_node(&node, "Keys", Color::rgb(0x3DDC97), poly_presets(), Some(i));
    st.pressures = out[..n].to_vec();
    let mut ui = app_ui(
        fonts,
        "Press a pad harder to swell it · Q-Links: threshold, full at, depth 1, depth 2",
    );
    let mut panel = SynthPanel::add(&mut ui, PAGE, st)?;
    let k = panel
        .pages()
        .iter()
        .position(|p| p.name == "Pressure")
        .ok_or("no Pressure page")?;
    panel.show_page(&mut ui, k);
    fill_qlinks(&mut ui, &panel);
    ui.show(PAGE)?;
    Ok(Sample { name: "synth-pressure", about: "Poly · Brass, Pressure page: a 9 dB swell and +1.5 oct, a chord held with three pads pressed", ui, panel })
}

/// Index of a Poly preset by name.
pub fn preset(name: &str) -> usize {
    Poly::presets()
        .iter()
        .position(|p| p.name == name)
        .unwrap_or(0)
}

/// The Routes page's sample: breath to level (route 2) and velocity to
/// cutoff (route 3).
const ROUTE_EDITS: &[(u16, f32)] = &[
    (pp::route_id(1, pp::route::SOURCE), 5.0),
    (pp::route_id(1, pp::route::TARGET), 3.0),
    (pp::route_id(1, pp::route::DEPTH), 60.0),
    (pp::route_id(2, pp::route::SOURCE), 1.0),
    (pp::route_id(2, pp::route::TARGET), 1.0),
    (pp::route_id(2, pp::route::DEPTH), 40.0),
];

/// Poly's Table scene: osc 1 on Vowels, the filter envelope scanning it.
const TABLE_EDITS: &[(u16, f32)] = &[
    (pp::OSC1_WAVE, pp::WAVE_TABLE as f32),
    (pp::OSC1_TABLE, 4.0),
    (pp::OSC1_POS, 33.0),
    (pp::FENV_POS1, 50.0),
];

/// Every sample screen.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let scenes = [
        Scene {
            name: "synth-filter",
            about: "Poly · Grit Bass, Filter page, cutoff selected (the wheel turns it)",
            preset: preset("Grit Bass"),
            edits: &[],
            page: "Filter",
            focus: Some(pp::CUTOFF),
        },
        Scene {
            name: "synth-osc",
            about: "Poly · Supersaw, Osc page: two saws, unison 4",
            preset: preset("Supersaw"),
            edits: &[],
            page: "Osc",
            focus: None,
        },
        Scene {
            name: "synth-env",
            about: "Poly · Dust Pad, Env page: slow attack and long release",
            preset: preset("Dust Pad"),
            edits: &[],
            page: "Env",
            focus: Some(pp::AMP_R),
        },
        Scene {
            name: "synth-lfo",
            about: "Poly · Lead, LFO page, edited: a 1/8 synced triangle on the cutoff",
            preset: preset("Lead"),
            edits: &[
                (pp::LFO_SHAPE, 1.0),
                (pp::LFO_SYNC, 6.0),
                (pp::LFO_CUTOFF, 1.5),
            ],
            page: "LFO",
            focus: Some(pp::LFO_SYNC),
        },
        Scene {
            name: "synth-lfo-2",
            about: "Poly · Super Pad, LFO 2 page, edited: each note's own vibrato, \
                    fading in over a second",
            preset: preset("Super Pad"),
            edits: &[
                (pp::LFO2_RATE, 5.5),
                (pp::LFO2_FADE, 1_000.0),
                (pp::LFO2_PITCH, 0.3),
                (pp::LFO2_CUTOFF, 0.25),
            ],
            page: "LFO 2",
            focus: Some(pp::LFO2_FADE),
        },
        Scene {
            name: "synth-table",
            about: "Poly · Init, Waves page, edited: osc 1 on the Vowels table a \
                    third through, the filter envelope scanning half of it",
            preset: preset("Init"),
            edits: TABLE_EDITS,
            page: "Waves",
            focus: Some(pp::OSC1_POS),
        },
        Scene {
            name: "synth-voice",
            about: "Poly · Acid, Voice page: mono with glide",
            preset: preset("Acid"),
            edits: &[],
            page: "Voice",
            focus: None,
        },
        Scene {
            name: "synth-routes",
            about: "Poly · Lead, Routes page, edited: breath swells the level and \
                    velocity opens the filter, beside the wheel's vibrato",
            preset: preset("Lead"),
            edits: ROUTE_EDITS,
            page: "Routes",
            focus: Some(pp::route_id(1, pp::route::DEPTH)),
        },
    ];
    let mut out = Vec::new();
    for s in &scenes {
        out.push(poly_scene(s, fonts())?);
    }
    out.push(poly_pressure_scene(fonts())?);
    // Glass Keys edited for the keyboard page: the tine (operator 2) fades
    // above C3 and the body (operator 1) lifts a little below it, as DX7
    // electric pianos do.
    let kbd: &[(u16, f32)] = &[
        (fp::op_id(2, op::R_DEPTH), 45.0),
        (fp::op_id(2, op::R_CURVE), 1.0),
        (fp::op_id(1, op::L_DEPTH), 30.0),
        (fp::op_id(1, op::L_CURVE), 3.0),
    ];
    for (name, about, preset, page, edits) in [
        ("synth-fm-algo", "FM · Brass Section, Algo page: algorithm 22 drawn, feedback 7 on operator 6, the six output levels",
         "Brass Section", "Algo", &[][..]),
        ("synth-fm-mod", "FM · Brass Section, Mod page: a small pitch scoop and a delayed vibrato", "Brass Section", "Mod", &[]),
        ("synth-fm-op", "FM · Glass Keys, Op 2 page: the ratio-14 tine, ringed in the algorithm, beside its envelope",
         "Glass Keys", "Op 2", &[]),
        ("synth-fm-kbd", "FM · Glass Keys, Kbd 1-2 page, edited: the tine fades above C3, the body lifts below it",
         "Glass Keys", "Kbd 1-2", kbd),
        ("synth-fm-pressure", "FM · Brass Section, Pressure page: a 9 dB swell and +12 brightness, a chord held and pressed",
         "Brass Section", "Pressure", &[]),
    ] {
        out.push(fm_scene(name, about, preset, page, edits, fonts())?);
    }
    for (name, about, preset, page, edits) in [
        ("synth-wt-osc", "Wavetable · Morph Pad, Osc A page: the Vowels table as a stack of 16 frames, frame 3 bright",
         "Morph Pad", "Osc A", &[][..]),
        ("synth-wt-warp", "Wavetable · Growl Bass, Osc A page, edited: the Resonance table half way, bent 60 %",
         "Growl Bass", "Osc A", &[(wtp::POSITION, 50.0), (wtp::WARP, 60.0)][..]),
        ("synth-wt-sync", "Wavetable · Sync Sweep, Osc B page, edited: Basic's saw with sync warp at 40 %",
         "Sync Sweep", "Osc B", &[(wtp::osc_id(1, wtp::OSC_LEVEL), 80.0), (wtp::osc_id(1, wtp::WARP_MODE), 2.0), (wtp::osc_id(1, wtp::WARP), 40.0)][..]),
        ("synth-wt-mod", "Wavetable · Vowel Lead, Mod page: the mod envelope scans 70 % of the table", "Vowel Lead", "Mod", &[][..]),
        ("synth-wt-pressure", "Wavetable · Morph Pad, Pressure page: pressure scans 40 % of the table and swells it 6 dB, a chord held and pressed",
         "Morph Pad", "Pressure", &[][..]),
    ] {
        out.push(wt_scene(name, about, preset, page, edits, fonts())?);
    }
    out.push(drum_scene(
        "synth-drums-kick",
        "Drums · Init Kit, Pad 1: the kick (Q-Links: tune, decay, tone, snap)",
        "Init Kit",
        0,
        &[],
        fonts(),
    )?);
    out.push(drum_scene(
        "synth-drums-boom",
        "Drums · Boom Kit, Pad 14 edited: a long, driven 808-style boom",
        "Boom Kit",
        13,
        &[(
            mpfree_synth::drum_params::pad_id(13, mpfree_synth::drum_params::field::DRIVE),
            60.0,
        )],
        fonts(),
    )?);
    out.push(drum_scene(
        "synth-drums-hat",
        "Drums · Init Kit, Pad 5: the open hat in choke group 1 with the closed hat",
        "Init Kit",
        4,
        &[],
        fonts(),
    )?);
    out.push(drum_scene(
        "synth-drums-kit",
        "Drums · Electro Kit, Kit page: the kit as the pads lie, pad 1's hit, every pad's level",
        "Electro Kit",
        16,
        &[],
        fonts(),
    )?);
    out.push(pluck_scene(
        "synth-pluck-string",
        "Pluck · Steel String, String page: Karplus-Strong decay, brightness, pick and position",
        "Steel String",
        "String",
        fonts(),
    )?);
    out.push(pluck_scene(
        "synth-pluck-mallet",
        "Pluck · Bell, Mallet page: the modal bank's hardness, strike, stretch and high damp",
        "Bell",
        "Mallet",
        fonts(),
    )?);
    for (name, about, engine, preset, page) in [
        ("synth-keys-strike", "Keys · Bright Tines, Strike page: the hammer at velocity 40 and 127, and C2, C4 and C6 ringing out",
         "keys", "Bright Tines", "Strike"),
        ("synth-keys-tone", "Keys · Reed Bite, Tone page: the reed's pickup curve with the stretch a hit swings across",
         "keys", "Reed Bite", "Tone"),
        ("synth-keys-clav", "Keys · Funk Clav, Clav page: what the bridge pickup hears through the Treble switch, and how long each partial rings",
         "keys", "Funk Clav", "Clav"),
        ("synth-drawbar", "Drawbar · Jazz Perc, Drawbars page: the registration as bars, the wave it adds up to, third percussion",
         "drawbar", "Jazz Perc", "Drawbars"),
        ("synth-drawbar-rotary", "Drawbar · Gospel, Rotary page: the horn and the drum spinning up to Fast",
         "drawbar", "Gospel", "Rotary"),
        ("synth-drawbar-combo", "Drawbar · Combo Square, Drawbars page: the combo model, its flute and square reed, and the wave they add up to",
         "drawbar", "Combo Square", "Drawbars"),
        ("synth-squelch", "Squelch · Acid Line, Synth page: the saw, the filter at rest and at the envelope's peak",
         "squelch", "Acid Line", "Synth"),
        ("synth-squelch-play", "Squelch · Acid Line, Play page: the cutoff's sweep for a plain note and an accented one",
         "squelch", "Acid Line", "Play"),
        ("synth-analog-filter", "Analog · Poly Brass, Filter page: the four-pole filter and where its envelope takes it",
         "analog", "Poly Brass", "Filter"),
        ("synth-analog-voice", "Analog · Worn Cards, Voice page: each voice card's own tuning at its Age",
         "analog", "Worn Cards", "Voice"),
        ("synth-analog-mod", "Analog · Poly Brass, Mod page: the LFO, poly-mod, and what pressure and the mod wheel move",
         "analog", "Poly Brass", "Mod"),
        ("synth-bass", "Bass · Rubber Bass, Source page: the saw with its sub, locked over the sub's cycle",
         "bass", "Rubber Bass", "Source"),
        ("synth-bass-808", "Bass · Long 808, Shape page: the pitch drop landing on the note, and the amp",
         "bass", "Long 808", "Shape"),
        ("synth-strings", "Strings · Classic Strings, Registers page: the violin and viola registers up, the tone and the octaves",
         "strings", "Classic Strings", "Registers"),
        ("synth-wind", "Wind · Tenor Sax, Instrument page: the air through the reed, the breath on its falling side, and the tone",
         "wind", "Tenor Sax", "Instrument"),
        ("synth-wind-flute", "Wind · Concert Flute, Breath page: where the breath comes from, and its shape over a note",
         "wind", "Concert Flute", "Breath"),
        ("synth-choir", "Choir · Vowel Pad, Choir page: the vowel's five formants, the singers and the vowel moving",
         "choir", "Vowel Pad", "Choir"),
        ("synth-choir-shape", "Choir · Solo Voice, Shape page: the swell, and the singer's vibrato coming in late",
         "choir", "Solo Voice", "Shape"),
    ] {
        out.push(inst_scene(name, about, engine, preset, page, fonts())?);
    }
    out.push(generic_scene(fonts())?);
    Ok(out)
}

/// A node with a preset loaded, the preset names and the preset's index.
type Loaded = (Box<dyn Node>, Vec<String>, usize);

/// A node with the preset of this name loaded, the preset names and its
/// index.
fn loaded<N: Node + 'static>(
    mut node: N,
    presets: &'static [Preset],
    load: fn(&mut N, &Preset),
    preset: &str,
) -> Result<Loaded, String> {
    let i = presets
        .iter()
        .position(|p| p.name == preset)
        .ok_or_else(|| format!("no preset {preset}"))?;
    load(&mut node, &presets[i]);
    let names = presets.iter().map(|p| p.name.to_string()).collect();
    Ok((Box::new(node), names, i))
}

/// Keys, Drawbar, Squelch, Analog, Bass or Choir with a factory preset,
/// on one of its pages.
fn inst_scene(
    name: &'static str,
    about: &'static str,
    engine: &str,
    preset: &str,
    page: &str,
    fonts: Fonts,
) -> Result<Sample, String> {
    const RATE: u32 = 48_000;
    let (node, names, i) = match engine {
        "keys" => loaded(Keys::new(RATE), Keys::presets(), Keys::load_preset, preset)?,
        "drawbar" => loaded(
            Drawbar::new(RATE),
            Drawbar::presets(),
            Drawbar::load_preset,
            preset,
        )?,
        "squelch" => loaded(
            Squelch::new(RATE),
            Squelch::presets(),
            Squelch::load_preset,
            preset,
        )?,
        "analog" => loaded(
            Analog::new(RATE),
            Analog::presets(),
            Analog::load_preset,
            preset,
        )?,
        "bass" => loaded(Bass::new(RATE), Bass::presets(), Bass::load_preset, preset)?,
        "wind" => loaded(Wind::new(RATE), Wind::presets(), Wind::load_preset, preset)?,
        "strings" => loaded(
            Strings::new(RATE),
            Strings::presets(),
            Strings::load_preset,
            preset,
        )?,
        "choir" => loaded(
            Choir::new(RATE),
            Choir::presets(),
            Choir::load_preset,
            preset,
        )?,
        _ => return Err(format!("no engine {engine}")),
    };
    let st = SynthState::from_node(node.as_ref(), "Keys", Color::rgb(0x3DDC97), names, Some(i));
    let mut ui = app_ui(fonts, POLY_HINT);
    let mut panel = SynthPanel::add(&mut ui, PAGE, st)?;
    let k = panel
        .pages()
        .iter()
        .position(|p| p.name == page)
        .ok_or_else(|| format!("no page {page}"))?;
    panel.show_page(&mut ui, k);
    fill_qlinks(&mut ui, &panel);
    ui.show(PAGE)?;
    Ok(Sample {
        name,
        about,
        ui,
        panel,
    })
}

const WT_HINT: &str = "Drag a control to turn it · tap one, then the wheel · SHIFT + wheel: next page · pads play the Lead track";

/// The Wavetable synth with a factory preset, on one of its pages.
fn wt_scene(
    name: &'static str,
    about: &'static str,
    preset: &str,
    page: &str,
    edits: &[(u16, f32)],
    fonts: Fonts,
) -> Result<Sample, String> {
    let presets = Wavetable::presets();
    let i = presets
        .iter()
        .position(|p| p.name == preset)
        .ok_or_else(|| format!("no Wavetable preset {preset}"))?;
    let mut node = Wavetable::new(48_000);
    node.load_preset(&presets[i]);
    for &(id, v) in edits {
        node.set_param(id, v);
    }
    let names = presets.iter().map(|p| p.name.to_string()).collect();
    let mut st = SynthState::from_node(&node, "Lead", Color::rgb(0xD46CFF), names, Some(i));
    st.edited = !edits.is_empty();
    if page == "Pressure" {
        press_chord(&mut node);
        let mut out = [0.0; 16];
        let n = node.pressures(&mut out);
        st.pressures = out[..n].to_vec();
    }
    let mut ui = app_ui(fonts, WT_HINT);
    let mut panel = SynthPanel::add(&mut ui, PAGE, st)?;
    let k = panel
        .pages()
        .iter()
        .position(|p| p.name == page)
        .ok_or_else(|| format!("no page {page}"))?;
    panel.show_page(&mut ui, k);
    fill_qlinks(&mut ui, &panel);
    ui.show(PAGE)?;
    Ok(Sample {
        name,
        about,
        ui,
        panel,
    })
}

const FM_HINT: &str = "Drag a control to turn it · tap one, then the wheel · SHIFT + wheel: next page · pads play the Keys track";

/// FM with a factory preset, on one of its pages.
fn fm_scene(
    name: &'static str,
    about: &'static str,
    preset: &str,
    page: &str,
    edits: &[(u16, f32)],
    fonts: Fonts,
) -> Result<Sample, String> {
    let presets = Fm::presets();
    let i = presets
        .iter()
        .position(|p| p.name == preset)
        .ok_or_else(|| format!("no FM preset {preset}"))?;
    let mut node = Fm::new(48_000);
    node.load_preset(&presets[i]);
    for &(id, v) in edits {
        node.set_param(id, v);
    }
    let names = presets.iter().map(|p| p.name.clone()).collect();
    let mut st = SynthState::from_node(&node, "Keys", Color::rgb(0x6EA8FE), names, Some(i));
    st.edited = !edits.is_empty();
    if page == "Pressure" {
        press_chord(&mut node);
        let mut out = [0.0; 16];
        let n = node.pressures(&mut out);
        st.pressures = out[..n].to_vec();
    }
    let mut ui = app_ui(fonts, FM_HINT);
    let mut panel = SynthPanel::add(&mut ui, PAGE, st)?;
    let k = panel
        .pages()
        .iter()
        .position(|p| p.name == page)
        .ok_or_else(|| format!("no page {page}"))?;
    panel.show_page(&mut ui, k);
    fill_qlinks(&mut ui, &panel);
    ui.show(PAGE)?;
    Ok(Sample {
        name,
        about,
        ui,
        panel,
    })
}

const DRUM_HINT: &str = "Drag a control to turn it · tap one, then the wheel · SHIFT + wheel: pad page · pads play the kit";

/// The drum synth with a factory kit, on pad `pad`'s page (16: the Kit page).
fn drum_scene(
    name: &'static str,
    about: &'static str,
    kit: &str,
    pad: usize,
    edits: &[(u16, f32)],
    fonts: Fonts,
) -> Result<Sample, String> {
    let kits = Drums::presets();
    let i = kits
        .iter()
        .position(|k| k.name == kit)
        .ok_or_else(|| format!("no kit {kit}"))?;
    let mut node = Drums::new(48_000);
    node.load_preset(&kits[i]);
    for &(id, v) in edits {
        node.set_param(id, v);
    }
    let names = kits.iter().map(|k| k.name.to_string()).collect();
    let mut st = SynthState::from_node(&node, "Drums", Color::rgb(0xF5A524), names, Some(i));
    st.edited = !edits.is_empty();
    let mut ui = app_ui(fonts, DRUM_HINT);
    let mut panel = SynthPanel::add(&mut ui, PAGE, st)?;
    // Past the last pad: the Kit page, after the pads' pages.
    let page = if pad < 16 {
        crate::pages::drum_page(pad)
    } else {
        16
    };
    panel.show_page(&mut ui, page);
    fill_qlinks(&mut ui, &panel);
    ui.show(PAGE)?;
    Ok(Sample {
        name,
        about,
        ui,
        panel,
    })
}

/// The pluck synth with a factory preset, on one of its pages.
fn pluck_scene(
    name: &'static str,
    about: &'static str,
    preset: &str,
    page: &str,
    fonts: Fonts,
) -> Result<Sample, String> {
    let presets = Pluck::presets();
    let i = presets
        .iter()
        .position(|p| p.name == preset)
        .ok_or_else(|| format!("no pluck preset {preset}"))?;
    let mut node = Pluck::new(48_000);
    node.load_preset(&presets[i]);
    let names = presets.iter().map(|p| p.name.to_string()).collect();
    let st = SynthState::from_node(&node, "Keys", Color::rgb(0x3DDC97), names, Some(i));
    let mut ui = app_ui(fonts, POLY_HINT);
    let mut panel = SynthPanel::add(&mut ui, PAGE, st)?;
    let k = panel
        .pages()
        .iter()
        .position(|p| p.name == page)
        .ok_or_else(|| format!("no page {page}"))?;
    panel.show_page(&mut ui, k);
    fill_qlinks(&mut ui, &panel);
    ui.show(PAGE)?;
    Ok(Sample {
        name,
        about,
        ui,
        panel,
    })
}

// ---------------------------------------------------------------- example effect

static ECHO_INFO: NodeInfo = NodeInfo {
    id: "example.echo",
    name: "Tape Echo (example)",
    kind: Kind::Effect,
};

#[allow(clippy::too_many_arguments)]
const fn ep(
    id: u16,
    name: &'static str,
    group: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: Unit,
    curve: Curve,
) -> ParamInfo {
    ParamInfo {
        id,
        name,
        group,
        min,
        max,
        default,
        unit,
        curve,
        readonly: false,
    }
}

/// The parameters of an example effect. It has no sound: it is here to show
/// the page a node gets with no page plan and no visuals.
pub static ECHO_PARAMS: &[ParamInfo] = &[
    ep(
        0,
        "Time",
        "Delay",
        10.0,
        2000.0,
        375.0,
        Unit::Ms,
        Curve::Log,
    ),
    ep(
        1,
        "Sync",
        "Delay",
        0.0,
        4.0,
        2.0,
        Unit::None,
        Curve::Choice(&["Off", "1/4", "1/8 D", "1/8", "1/16"]),
    ),
    ep(
        2,
        "Feedback",
        "Delay",
        0.0,
        110.0,
        45.0,
        Unit::Percent,
        Curve::Linear,
    ),
    ep(
        3,
        "Mix",
        "Delay",
        0.0,
        100.0,
        30.0,
        Unit::Percent,
        Curve::Linear,
    ),
    ep(
        4,
        "Low cut",
        "Tone",
        20.0,
        2000.0,
        120.0,
        Unit::Hz,
        Curve::Log,
    ),
    ep(
        5,
        "High cut",
        "Tone",
        500.0,
        20000.0,
        6000.0,
        Unit::Hz,
        Curve::Log,
    ),
    ep(6, "Drive", "Tone", 0.0, 24.0, 6.0, Unit::Db, Curve::Linear),
    ep(
        7,
        "Wow",
        "Wobble",
        0.0,
        100.0,
        20.0,
        Unit::Percent,
        Curve::Linear,
    ),
    ep(
        8,
        "Flutter",
        "Wobble",
        0.0,
        100.0,
        10.0,
        Unit::Percent,
        Curve::Linear,
    ),
    ep(
        9,
        "Age",
        "Wobble",
        0.0,
        100.0,
        35.0,
        Unit::Percent,
        Curve::Linear,
    ),
    ep(
        10,
        "Width",
        "Output",
        0.0,
        100.0,
        70.0,
        Unit::Percent,
        Curve::Linear,
    ),
    ep(
        11,
        "Level",
        "Output",
        -24.0,
        6.0,
        0.0,
        Unit::Db,
        Curve::Linear,
    ),
    ep(
        12,
        "Freeze",
        "Output",
        0.0,
        1.0,
        0.0,
        Unit::None,
        Curve::Toggle,
    ),
];

/// A node with parameters and no sound, for the generic page.
pub struct Example {
    values: Vec<f32>,
}

impl Example {
    pub fn new() -> Example {
        Example {
            values: ECHO_PARAMS.iter().map(|p| p.default).collect(),
        }
    }
}

impl Default for Example {
    fn default() -> Example {
        Example::new()
    }
}

impl Node for Example {
    fn info(&self) -> &'static NodeInfo {
        &ECHO_INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        ECHO_PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        if let Some(i) = ECHO_PARAMS.iter().position(|p| p.id == id) {
            self.values[i] = ECHO_PARAMS[i].clamp(value);
        }
    }
    fn param(&self, id: u16) -> f32 {
        ECHO_PARAMS
            .iter()
            .position(|p| p.id == id)
            .map(|i| self.values[i])
            .unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _ctx: &Ctx, _events: &[Timed], _left: &mut [f32], _right: &mut [f32]) {}
}

fn generic_scene(fonts: Fonts) -> Result<Sample, String> {
    let node = Example::new();
    let presets = vec![
        "Default".to_string(),
        "Slapback".to_string(),
        "Dub Wash".to_string(),
    ];
    let st = SynthState::from_node(&node, "Keys FX", Color::rgb(0x6EA8FE), presets, Some(0));
    let mut ui = app_ui(
        fonts,
        "Any node gets this page from its parameters alone · a plugin, or an effect",
    );
    let panel = SynthPanel::add(&mut ui, PAGE, st)?;
    fill_qlinks(&mut ui, &panel);
    ui.show(PAGE)?;
    Ok(Sample {
        name: "generic-node",
        about: "An example effect with no page plan: generic pages from its parameter groups, no visuals",
        ui,
        panel,
    })
}
