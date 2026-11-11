//! Sample arp screens with a real engine behind them, for the workstation
//! preview and the tests. Pad names are made up (no Akai content).
//!
//! The screens sit in the pad player as it is today: its header tabs (the
//! arp is not one of them), the arp chip in the header, the Arp panel in the
//! Play page's right-hand column, and the quick band over the pads while
//! NOTE REPEAT is held.

use crate::view::{chips, hint, quick_hint, ArpPanel, ArpState};
use crate::workspace::{self, record_pad_labels, PatternWorkspace, Row};
use mpfree_arp::{Engine, Events, LatchMode, Library, Mode, Order, Pattern, Rate, Settings};
use mpfree_ui::widgets::{Header, Label, PadGrid};
use mpfree_ui::{Align, Color, Fonts, Theme, Tone, Ui};

/// The screen as the pad player sees it (the MPC One's panel, turned).
pub const SCREEN: (i32, i32) = (1280, 800);
/// The pad player's header tabs (`mpfree-hello`'s `TABS`).
const APP_TABS: [&str; 4] = ["Play", "Debug", "Console", "Prefs"];
/// The page names the samples add the panel and the quick band under.
pub const PANEL_PAGE: &str = "Play";
pub const QUICK_PAGE: &str = "Play, NOTE REPEAT held";
const PAD_NAMES: [&str; 16] = ["Kick", "Snare", "Closed Hat", "Open Hat", "Clap", "Rim", "Tom Low", "Tom Hi", "Crash",
                               "Ride", "Shaker", "Perc 1", "Perc 2", "", "Sub Hit", ""];
const SAMPLE_RATE: u32 = 44_100;

/// One sample screen: its file name and the UI showing it.
pub struct Sample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub page: ArpPanel,
    /// The quick band is showing, not the panel.
    pub quick: bool,
    pub state: ArpState,
}

/// The built-in patterns as the page lists them.
pub fn built_in_patterns() -> Vec<(String, mpfree_arp::Pattern)> {
    Library::built_in().iter().map(|(n, p)| (n.to_string(), *p)).collect()
}

/// Runs `e` for `steps` steps of its rate, in 128-frame periods.
fn run_steps(e: &mut Engine, steps: f64) {
    let frames = steps * f64::from(e.settings().rate.ticks()) / f64::from(mpfree_arp::PPQ) * 60.0 / e.bpm()
        * f64::from(SAMPLE_RATE);
    let mut out = Events::new();
    let mut done = 0.0;
    while done < frames {
        e.process(128, &mut out);
        done += 128.0;
    }
}

/// What a sample screen plays: `settings`, built-in pattern `pattern`
/// (mutated or not), pads `held` (slots), latched or still held, run for
/// `steps` steps; and whether it shows the quick band or the panel.
struct Setup<'a> {
    settings: Settings,
    pattern: usize,
    held: &'a [u8],
    steps: f64,
    mutated: bool,
    latch: bool,
    quick: bool,
}

impl Default for Setup<'_> {
    fn default() -> Self {
        Setup { settings: Settings::default(), pattern: 0, held: &[], steps: 0.0, mutated: false, latch: true,
                quick: false }
    }
}

fn scene(name: &'static str, about: &'static str, fonts: Fonts, setup: Setup) -> Result<Sample, String> {
    let Setup { settings, pattern, held, steps, mutated, latch, quick } = setup;
    let patterns = built_in_patterns();
    let mut e = Engine::new(SAMPLE_RATE, 96.0);
    e.set_settings(settings);
    let mut p = patterns.get(pattern).map(|(_, p)| *p).unwrap_or_default();
    if mutated {
        p = p.mutated(7, 40);
    }
    e.set_pattern(p);
    e.set_latch(if latch { LatchMode::Replace } else { LatchMode::Off });
    for &slot in held {
        e.press(slot, 100);
    }
    if latch {
        for &slot in held {
            e.release(slot);
        }
    }
    run_steps(&mut e, steps);
    let state = ArpState::from_engine(&e, patterns, pattern, mutated);

    let mut ui = Ui::new(Theme::dark(), fonts, SCREEN.0, SCREEN.1);
    let mut header = Header::new(&APP_TABS);
    header.chips = chips(&state);
    ui.add("header", header);
    let mut pads = PadGrid::new(16, 4);
    let kit = Color::from_pad([0x3F, 0x22, 0x00]);
    for (i, pad) in pads.pads.iter_mut().enumerate() {
        pad.label = PAD_NAMES[i].to_string();
        pad.color = (!PAD_NAMES[i].is_empty()).then_some(kit);
    }
    // The latched pads glow; the one the last step played is lit.
    for h in &state.notes {
        if let Some(pad) = pads.pads.get_mut(usize::from(h.note)) {
            pad.level = 0.35;
        }
    }
    ui.add("pads", pads);
    let line = if quick { quick_hint(settings.mode) } else { hint(settings.mode) };
    ui.add("hint", Label::new(line, Tone::Dim, Align::Center));
    let page = ArpPanel::add(&mut ui, PANEL_PAGE, QUICK_PAGE, &state)?;
    ui.show(if quick { QUICK_PAGE } else { PANEL_PAGE })?;
    Ok(Sample { name, about, ui, page, quick, state })
}

/// Every sample screen.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let base = Settings::default();
    Ok(vec![
        scene("quick-hat-roll", "Quick band: NOTE REPEAT held with the closed hat, 1/16", fonts(),
              Setup { settings: Settings { mode: Mode::Repeat, gate: 40, ..base }, held: &[2], steps: 5.5, latch: false, quick: true, ..Setup::default() })?,
        scene("quick-arp-latched", "Quick band: Arp Up 1/16 over pads 1 3 5, latched", fonts(),
              Setup { settings: Settings { mode: Mode::Arp, order: Order::Up, octaves: 2, ..base }, held: &[0, 2, 4], steps: 3.5, quick: true, ..Setup::default() })?,
        scene("arp-up-2-oct", "Panel: Arp, Up over 2 octaves, pads 1 3 5 latched", fonts(),
              Setup { settings: Settings { mode: Mode::Arp, order: Order::Up, octaves: 2, swing: 58, ..base }, held: &[0, 2, 4], steps: 3.5, ..Setup::default() })?,
        scene("arp-converge", "Panel: Arp, Converge, 1/16T, pads 1 2 3 4 latched", fonts(),
              Setup { settings: Settings { mode: Mode::Arp, order: Order::Converge, rate: Rate::SixteenthTriplet, gate: 80, ..base }, held: &[0, 1, 2, 3], steps: 6.5, ..Setup::default() })?,
        scene("arp-random", "Panel: Arp, Random, pads 1 5 latched", fonts(),
              Setup { settings: Settings { mode: Mode::Arp, order: Order::Random, octaves: 3, ..base }, held: &[0, 4], steps: 2.5, ..Setup::default() })?,
        scene("repeat", "Panel: Note Repeat at 1/32, pad 3 latched", fonts(),
              Setup { settings: Settings { mode: Mode::Repeat, rate: Rate::ThirtySecond, gate: 30, ..base }, held: &[2], steps: 5.5, ..Setup::default() })?,
        scene("pattern", "Panel: Pattern \"Climb\" over pads 1 2 5", fonts(),
              Setup { settings: Settings { mode: Mode::Pattern, ..base }, pattern: 3, held: &[0, 1, 4], steps: 4.5, ..Setup::default() })?,
        scene("pattern-mutated", "Panel: Pattern \"Rolls\", mutated (Original is lit)", fonts(),
              Setup { settings: Settings { mode: Mode::Pattern, ..base }, pattern: 6, held: &[0, 2, 4], steps: 2.5, mutated: true, ..Setup::default() })?,
        scene("direct", "Panel: Direct, nothing generated, so no step strip", fonts(),
              Setup { latch: false, ..Setup::default() })?,
    ])
}

/// One sample screen of the pattern workspace.
pub struct WorkspaceSample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub ws: PatternWorkspace,
}

fn workspace_scene(name: &'static str, about: &'static str, fonts: Fonts, recording: bool)
    -> Result<WorkspaceSample, String> {
    let climb = built_in_patterns().into_iter().find(|(n, _)| n == "Climb").map(|(_, p)| p).unwrap_or_default();
    let mut ui = Ui::new(Theme::dark(), fonts, SCREEN.0, SCREEN.1);
    let mut e = Engine::new(SAMPLE_RATE, 96.0);
    e.set_settings(Settings { mode: Mode::Pattern, ..Settings::default() });
    let mut header = Header::new(&APP_TABS);
    let state = ArpState::from_engine(&e, built_in_patterns(), 3, true);
    header.chips = chips(&state);
    ui.add("header", header);
    let mut pads = PadGrid::new(16, 4);
    let notes = Color::from_pad([0x00, 0x22, 0x3F]);
    let marks = Color::from_pad([0x3F, 0x22, 0x00]);
    for (i, (pad, label)) in pads.pads.iter_mut().zip(record_pad_labels()).enumerate() {
        pad.label = label.to_string();
        pad.color = (!label.is_empty()).then_some(if i < 9 { notes } else { marks });
    }
    ui.add("pads", pads);
    ui.add("hint", Label::new(workspace::hint(recording), Tone::Dim, Align::Center));
    let mut ws = PatternWorkspace::add(&mut ui, "Pattern", "Pattern, recording", "Climb", climb)?;
    if recording {
        // Part way through recording on the pads, in Step time.
        let taken = Pattern::parse("1 3 2! . 1+ _").map_err(|e| e.to_string())?;
        let mut steps = [mpfree_arp::Step::Rest; 8];
        steps[..taken.len()].copy_from_slice(taken.steps());
        ws.recorded(Pattern::from_steps(&steps).ok_or("pattern")?);
        ws.recording = true;
        ws.editor.select(Row::Step, 6);
        ws.update(&mut ui, None);
    } else {
        // An accent, a ratchet and a chance added; the ratchet is selected.
        ws.editor.tap(0);
        ws.editor.turn(Row::Ratchet, 3, 2);
        ws.editor.turn(Row::Chance, 5, -4);
        ws.editor.select(Row::Ratchet, 3);
        ws.update(&mut ui, Some(4));
    }
    ui.show(ws.page())?;
    Ok(WorkspaceSample { name, about, ui, ws })
}

/// The pattern workspace's sample screens.
pub fn workspace_samples(fonts: impl Fn() -> Fonts) -> Result<Vec<WorkspaceSample>, String> {
    Ok(vec![
        workspace_scene("workspace-edit", "Pattern workspace: \"Climb\" with an accent, a ratchet and a chance added",
                        fonts(), false)?,
        workspace_scene("workspace-record", "Pattern workspace: recording on the pads, 6 of 8 steps in", fonts(), true)?,
    ])
}
