//! Sample Arp pages with a real engine behind them, for the workstation
//! preview and the tests. Pad names are made up (no Akai content).

use crate::view::{chips, hint, ArpPage, ArpState};
use mpfree_arp::{Engine, Events, LatchMode, Library, Mode, Order, Rate, Settings};
use mpfree_ui::widgets::{Header, Label, PadGrid};
use mpfree_ui::{Align, Color, Fonts, Theme, Tone, Ui};

/// The screen as the pad player sees it (the MPC One's panel, turned).
pub const SCREEN: (i32, i32) = (1280, 800);
const PAD_NAMES: [&str; 16] = ["Kick", "Snare", "Closed Hat", "Open Hat", "Clap", "Rim", "Tom Low", "Tom Hi", "Crash",
                               "Ride", "Shaker", "Perc 1", "Perc 2", "", "Sub Hit", ""];
const SAMPLE_RATE: u32 = 44_100;

/// One sample screen: its file name and the UI showing it.
pub struct Sample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub page: ArpPage,
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

/// What a sample page plays: `settings`, built-in pattern `pattern`
/// (mutated or not), pads `held` (slots) latched, run for `steps` steps.
struct Setup<'a> {
    settings: Settings,
    pattern: usize,
    held: &'a [u8],
    steps: f64,
    mutated: bool,
}

fn scene(name: &'static str, about: &'static str, fonts: Fonts, setup: Setup) -> Result<Sample, String> {
    let Setup { settings, pattern, held, steps, mutated } = setup;
    let patterns = built_in_patterns();
    let mut e = Engine::new(SAMPLE_RATE, 96.0);
    e.set_settings(settings);
    let mut p = patterns.get(pattern).map(|(_, p)| *p).unwrap_or_default();
    if mutated {
        p = p.mutated(7, 40);
    }
    e.set_pattern(p);
    e.set_latch(LatchMode::Replace);
    for &slot in held {
        e.press(slot, 100);
    }
    for &slot in held {
        e.release(slot);
    }
    run_steps(&mut e, steps);
    let state = ArpState::from_engine(&e, patterns, pattern, mutated);

    let mut ui = Ui::new(Theme::dark(), fonts, SCREEN.0, SCREEN.1);
    let mut header = Header::new(&["Play", "Arp", "Debug"]);
    header.active = 1;
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
    ui.add("hint", Label::new(hint(settings.mode), Tone::Dim, Align::Center));
    let page = ArpPage::add(&mut ui, "Arp", &state)?;
    ui.show("Arp")?;
    Ok(Sample { name, about, ui, page, state })
}

/// Every sample screen.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let base = Settings::default();
    Ok(vec![
        scene("arp-up-2-oct", "Arp, Up over 2 octaves, pads 1 3 5 latched", fonts(),
              Setup { settings: Settings { mode: Mode::Arp, order: Order::Up, octaves: 2, swing: 58, ..base }, pattern: 0, held: &[0, 2, 4], steps: 3.5, mutated: false })?,
        scene("arp-converge", "Arp, Converge, 1/16T, pads 1 2 3 4 latched", fonts(),
              Setup { settings: Settings { mode: Mode::Arp, order: Order::Converge, rate: Rate::SixteenthTriplet, gate: 80, ..base }, pattern: 0, held: &[0, 1, 2, 3], steps: 6.5, mutated: false })?,
        scene("arp-random", "Arp, Random, pads 1 5 latched", fonts(),
              Setup { settings: Settings { mode: Mode::Arp, order: Order::Random, octaves: 3, ..base }, pattern: 0, held: &[0, 4], steps: 2.5, mutated: false })?,
        scene("repeat", "Note Repeat at 1/32, pad 3 latched", fonts(),
              Setup { settings: Settings { mode: Mode::Repeat, rate: Rate::ThirtySecond, gate: 30, ..base }, pattern: 0, held: &[2], steps: 5.5, mutated: false })?,
        scene("pattern", "Pattern \"Climb\" over pads 1 2 5", fonts(),
              Setup { settings: Settings { mode: Mode::Pattern, ..base }, pattern: 3, held: &[0, 1, 4], steps: 4.5, mutated: false })?,
        scene("pattern-mutated", "Pattern \"Rolls\", mutated (Original is lit)", fonts(),
              Setup { settings: Settings { mode: Mode::Pattern, ..base }, pattern: 6, held: &[0, 2, 4], steps: 2.5, mutated: true })?,
        scene("direct", "Direct: nothing generated, so the strip is empty", fonts(), Setup { settings: base, pattern: 0, held: &[], steps: 0.0, mutated: false })?,
    ])
}
