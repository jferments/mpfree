//! Sample screens for the workstation preview and the tests: a real Poly
//! behind each page, and an example effect to show the generic page any
//! node (a hosted plugin, an FX unit) gets from its parameters alone.

use crate::view::{SynthPanel, SynthState};
use mpfree_node::{Ctx, Curve, Kind, Node, NodeInfo, ParamInfo, Timed, Unit};
use mpfree_synth::params as pp;
use mpfree_synth::{Fm, Poly};
use mpfree_ui::widgets::{Chip, Header, Label};
use mpfree_ui::{Align, Color, Fonts, Theme, Tone, Ui};

pub const WIDTH: i32 = 1280;
pub const HEIGHT: i32 = 800;
/// The time engine's tabs, as in its mockups. The panel's page is Synth.
pub const TABS: [&str; 6] = ["Steps", "Clips", "Looper", "Arrange", "Synth", "Mix"];
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
    let mut header = Header::new(&TABS);
    header.active = 4;
    header.chips = vec![Chip::new("92.0 BPM", Tone::Dim), Chip::new("DSP 18 %", Tone::Good)];
    ui.add("header", header);
    ui.add("hint", Label::new(hint, Tone::Dim, Align::Center));
    ui
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
    "Drag a tile to turn it · tap a tile, then the wheel · SHIFT + wheel: page · pads play the Bass track";

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
    let mut st = SynthState::from_node(&node, "Bass", Color::rgb(0x3DDC97), poly_presets(), Some(s.preset));
    st.edited = !s.edits.is_empty();
    let mut ui = app_ui(fonts, POLY_HINT);
    let mut panel = SynthPanel::add(&mut ui, PAGE, st)?;
    let page = panel.pages().iter().position(|p| p.name == s.page).ok_or_else(|| format!("no page {}", s.page))?;
    panel.show_page(&mut ui, page);
    if let Some(id) = s.focus {
        let k = panel.pages()[page].params.iter().position(|&i| panel.state.params[i].id == id).ok_or("focus not on page")?;
        let ev = mpfree_ui::UiEvent { widget: crate::view::TILES.to_string(), kind: mpfree_ui::EventKind::Select(k) };
        panel.event(&mut ui, &ev);
    }
    ui.show(PAGE)?;
    Ok(Sample { name: s.name, about: s.about, ui, panel })
}

/// Index of a Poly preset by name.
pub fn preset(name: &str) -> usize {
    Poly::presets().iter().position(|p| p.name == name).unwrap_or(0)
}

/// Every sample screen.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let scenes = [
        Scene { name: "synth-filter", about: "Poly · Grit Bass, Filter page, cutoff selected (the wheel turns it)",
                preset: preset("Grit Bass"), edits: &[], page: "Filter", focus: Some(pp::CUTOFF) },
        Scene { name: "synth-osc", about: "Poly · Supersaw, Osc page: two saws, unison 4",
                preset: preset("Supersaw"), edits: &[], page: "Osc", focus: None },
        Scene { name: "synth-env", about: "Poly · Dust Pad, Env page: slow attack and long release",
                preset: preset("Dust Pad"), edits: &[], page: "Env", focus: Some(pp::AMP_R) },
        Scene { name: "synth-lfo", about: "Poly · Lead, LFO page, edited: a 1/8 synced triangle on the cutoff",
                preset: preset("Lead"), edits: &[(pp::LFO_SHAPE, 1.0), (pp::LFO_SYNC, 6.0), (pp::LFO_CUTOFF, 1.5)],
                page: "LFO", focus: Some(pp::LFO_SYNC) },
        Scene { name: "synth-voice", about: "Poly · Acid, Voice page: mono with glide",
                preset: preset("Acid"), edits: &[], page: "Voice", focus: None },
    ];
    let mut out = Vec::new();
    for s in &scenes {
        out.push(poly_scene(s, fonts())?);
    }
    for (name, about, preset, page) in [
        ("synth-fm-algo", "FM · Brass Section, Algo page: algorithm 22, feedback 7", "Brass Section", "Algo"),
        ("synth-fm-op", "FM · Glass Keys, Op 2 page: the ratio-14 tine", "Glass Keys", "Op 2"),
        ("synth-fm-kbd", "FM · Glass Keys, Kbd 1-2 page: keyboard scaling", "Glass Keys", "Kbd 1-2"),
    ] {
        out.push(fm_scene(name, about, preset, page, fonts())?);
    }
    out.push(generic_scene(fonts())?);
    Ok(out)
}

const FM_HINT: &str = "Drag a tile to turn it · tap a tile, then the wheel · SHIFT + wheel: page · pads play the Keys track";

/// FM with a factory preset, on one of its pages.
fn fm_scene(name: &'static str, about: &'static str, preset: &str, page: &str, fonts: Fonts) -> Result<Sample, String> {
    let presets = Fm::presets();
    let i = presets.iter().position(|p| p.name == preset).ok_or_else(|| format!("no FM preset {preset}"))?;
    let mut node = Fm::new(48_000);
    node.load_preset(&presets[i]);
    let names = presets.iter().map(|p| p.name.clone()).collect();
    let st = SynthState::from_node(&node, "Keys", Color::rgb(0x6EA8FE), names, Some(i));
    let mut ui = app_ui(fonts, FM_HINT);
    let mut panel = SynthPanel::add(&mut ui, PAGE, st)?;
    let k = panel.pages().iter().position(|p| p.name == page).ok_or_else(|| format!("no page {page}"))?;
    panel.show_page(&mut ui, k);
    ui.show(PAGE)?;
    Ok(Sample { name, about, ui, panel })
}

// ---------------------------------------------------------------- example effect

static ECHO_INFO: NodeInfo = NodeInfo { id: "example.echo", name: "Tape Echo (example)", kind: Kind::Effect };

#[allow(clippy::too_many_arguments)]
const fn ep(id: u16, name: &'static str, group: &'static str, min: f32, max: f32, default: f32, unit: Unit, curve: Curve)
    -> ParamInfo {
    ParamInfo { id, name, group, min, max, default, unit, curve, readonly: false }
}

/// The parameters of an example effect. It has no sound: it is here to show
/// the page a node gets with no page plan and no visuals.
pub static ECHO_PARAMS: &[ParamInfo] = &[
    ep(0, "Time", "Delay", 10.0, 2000.0, 375.0, Unit::Ms, Curve::Log),
    ep(1, "Sync", "Delay", 0.0, 4.0, 2.0, Unit::None, Curve::Choice(&["Off", "1/4", "1/8 D", "1/8", "1/16"])),
    ep(2, "Feedback", "Delay", 0.0, 110.0, 45.0, Unit::Percent, Curve::Linear),
    ep(3, "Mix", "Delay", 0.0, 100.0, 30.0, Unit::Percent, Curve::Linear),
    ep(4, "Low cut", "Tone", 20.0, 2000.0, 120.0, Unit::Hz, Curve::Log),
    ep(5, "High cut", "Tone", 500.0, 20000.0, 6000.0, Unit::Hz, Curve::Log),
    ep(6, "Drive", "Tone", 0.0, 24.0, 6.0, Unit::Db, Curve::Linear),
    ep(7, "Wow", "Wobble", 0.0, 100.0, 20.0, Unit::Percent, Curve::Linear),
    ep(8, "Flutter", "Wobble", 0.0, 100.0, 10.0, Unit::Percent, Curve::Linear),
    ep(9, "Age", "Wobble", 0.0, 100.0, 35.0, Unit::Percent, Curve::Linear),
    ep(10, "Width", "Output", 0.0, 100.0, 70.0, Unit::Percent, Curve::Linear),
    ep(11, "Level", "Output", -24.0, 6.0, 0.0, Unit::Db, Curve::Linear),
    ep(12, "Freeze", "Output", 0.0, 1.0, 0.0, Unit::None, Curve::Toggle),
];

/// A node with parameters and no sound, for the generic page.
pub struct Example {
    values: Vec<f32>,
}

impl Example {
    pub fn new() -> Example {
        Example { values: ECHO_PARAMS.iter().map(|p| p.default).collect() }
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
        ECHO_PARAMS.iter().position(|p| p.id == id).map(|i| self.values[i]).unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _ctx: &Ctx, _events: &[Timed], _left: &mut [f32], _right: &mut [f32]) {}
}

fn generic_scene(fonts: Fonts) -> Result<Sample, String> {
    let node = Example::new();
    let presets = vec!["Default".to_string(), "Slapback".to_string(), "Dub Wash".to_string()];
    let st = SynthState::from_node(&node, "Keys FX", Color::rgb(0x6EA8FE), presets, Some(0));
    let mut ui = app_ui(fonts, "Any node gets this page from its parameters alone · a plugin, or an effect");
    let panel = SynthPanel::add(&mut ui, PAGE, st)?;
    ui.show(PAGE)?;
    Ok(Sample {
        name: "generic-node",
        about: "An example effect with no page plan: generic pages from its parameter groups, no visuals",
        ui,
        panel,
    })
}
