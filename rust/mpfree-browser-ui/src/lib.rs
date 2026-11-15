//! The browser page for the MPC One: where projects, samples, kits,
//! patterns and recordings are found, heard, loaded, saved and tidied
//! (controls plan decision 13, approved by Jay 2026-11-11).
//!
//! **A proposal to the Hello MPC thread,** which owns the app's layout and
//! button wiring, and agreed the placement on 2026-11-11: BROWSE opens the
//! browser over the current page, under the top bar, as Prefs and the
//! mixer do (it is not a tab); BROWSE again closes it; SHIFT + BROWSE
//! saves the project from anywhere. This crate is what opens.
//!
//! - [`state`]: the logic ([`Browser`]): places, folders, the list, the
//!   selection, the buttons, the keyboard and the questions, Recent and
//!   Favourites, saving and opening projects. Every file operation runs
//!   on `mpfree-library`'s worker thread.
//! - [`view`]: the widget ([`BrowserView`]): places on the left, the list
//!   in the middle, the selected item on the right with a quick waveform
//!   and its buttons; the on-screen keyboard and the yes/no card over
//!   them.
//! - [`export`]: the Export panel ([`ExportPanel`]) that Export in the
//!   Projects place opens over the browser: what to make, the range, how
//!   it will play, then the time done and the outcome. The app makes and
//!   runs the export (`mpfree-export`).
//! - [`sample`]: sample screens over a real library in a scratch folder,
//!   for the workstation preview (`mpfree-browser-preview`) and the tests.
//!
//! For the app, once a frame: [`step`] passes the widget's touches to the
//! browser, takes what the worker finished, and shows the result; it
//! returns [`BrowseAction`]s to carry out (load a sound into the sampler,
//! audition a file, give an opened project's parts to the engines, save
//! Recent and Favourites in the store). Hardware while the browser is
//! open, all on [`Browser`]: the data dial [`Browser::dial`], SHIFT + dial
//! [`Browser::dial_places`], the dial's push [`Browser::push`], `-`
//! [`Browser::back`], `+` [`Browser::enter`], PLAY [`Browser::play`],
//! ERASE [`Browser::erase`]; SHIFT + BROWSE anywhere [`Browser::save`].
//! The Q-Links: [`qlinks`] and [`qlink_turn`].

#![forbid(unsafe_code)]

pub mod export;
pub mod sample;
pub mod state;
pub mod view;

pub use export::{
    ExportBits, ExportChoice, ExportFormat, ExportInput, ExportPanel, ExportPhase, ExportRange,
    ExportSetup, StemsBy, EXPORT_RATES,
};
pub use state::{BrowseAction, Browser, Button, Filter, Input, Pick, Screen, Sort, Source};
pub use view::BrowserView;

use mpfree_ui::Ui;

/// The widget's name.
pub const BROWSER: &str = "browser";
/// The page's name in the [`Ui`].
pub const BROWSER_PAGE: &str = "browser";
/// The page: the app's header, the browser, the hint line.
pub const BROWSER_LAYOUT: &str = "col(header:60, browser:*, hint:26)";
/// The hint line while the browser is open.
pub const BROWSER_HINT: &str = "Dial: move  ·  push: open or load  ·  - / +: up / into a folder  ·  PLAY: hear it  ·  ERASE: to Trash  ·  BROWSE closes  ·  SHIFT + BROWSE: save project";

/// The hint line while the Export panel shows.
pub const EXPORT_HINT: &str = "Dial: bars  ·  push: Start (when done, Close)  ·  -: close  ·  BROWSE hides it (an export keeps running)";

/// The hint line while a picker is open ([`Browser::open_picker`]).
pub const PICKER_HINT: &str = "Dial: move  ·  push: load  ·  - / +: up / into a folder  ·  PLAY: hear it  ·  BROWSE or Cancel: close without loading";

/// Adds the browser widget and its page. The app's `header` and `hint`
/// widgets are added if they are not there yet.
pub fn add(ui: &mut Ui, browser: &Browser) -> Result<(), String> {
    if ui.get::<mpfree_ui::widgets::Header>("header").is_none() {
        ui.add("header", mpfree_ui::widgets::Header::new(&[]));
    }
    if ui.get::<mpfree_ui::widgets::Label>("hint").is_none() {
        ui.add(
            "hint",
            mpfree_ui::widgets::Label::new(
                BROWSER_HINT,
                mpfree_ui::Tone::Dim,
                mpfree_ui::Align::Left,
            ),
        );
    }
    ui.add(BROWSER, BrowserView::new(browser.screen()));
    ui.add_page(BROWSER_PAGE, BROWSER_LAYOUT)
}

/// Once a frame while the browser is open (and whenever a job may have
/// finished): the widget's touches go to the browser, the worker's
/// results come in, the widget shows the new screen. Returns what the app
/// must do.
pub fn step(ui: &mut Ui, browser: &mut Browser) -> Vec<BrowseAction> {
    // Taking the inputs changes nothing on screen: the view is redrawn
    // only when something changed (below), not every frame.
    let (inputs, rows) = match ui.get_mut_quiet::<BrowserView>(BROWSER) {
        Some(v) => (v.take_inputs(), v.visible_rows()),
        None => (Vec::new(), 0),
    };
    if let Some(r) = ui.rect_of(BROWSER) {
        browser.set_visible(BrowserView::rows_in(r));
    } else {
        browser.set_visible(rows);
    }
    let changed = !inputs.is_empty();
    for i in inputs {
        browser.input(i);
    }
    let polled = browser.poll();
    let actions = browser.take_actions();
    if changed || polled || !actions.is_empty() || needs_show(ui, browser) {
        show(ui, browser);
    }
    actions
}

fn needs_show(ui: &Ui, browser: &Browser) -> bool {
    ui.get::<BrowserView>(BROWSER)
        .is_some_and(|v| *v.screen() != browser.screen())
}

/// Shows the browser's screen now (after a hardware press).
pub fn show(ui: &mut Ui, browser: &Browser) {
    if let Some(v) = ui.get_mut_quiet::<BrowserView>(BROWSER) {
        v.show(browser.screen());
    }
}

/// What the Q-Link cards say while the browser is open: (title, value).
pub fn qlinks(browser: &Browser, preview_level_db: f32) -> [(String, String); 4] {
    let s = browser.screen();
    let pos = match (s.selected, s.rows.len()) {
        (Some(i), n) if n > 0 => format!("{} / {n}", i + 1),
        _ => "—".into(),
    };
    [
        ("Scroll".into(), pos),
        ("Show".into(), s.filter.label().into()),
        ("Preview".into(), format!("{preview_level_db:.0} dB")),
        ("Place".into(), state::SOURCES[s.source].label().into()),
    ]
}

/// What a Q-Link turn does that the app must carry out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QLinkEffect {
    None,
    /// Change the preview level by this many dB.
    PreviewLevel(f32),
}

/// Q-Link `knob` (0 to 3) turned by `detents`: 1 scrolls the list, 2
/// changes what it shows, 3 is the preview level (the app's), 4 moves
/// through the places.
pub fn qlink_turn(browser: &mut Browser, knob: usize, detents: i32) -> QLinkEffect {
    match knob {
        0 => browser.dial(detents),
        1 if browser.picking().is_none() => {
            let f = browser.screen().filter;
            let i = state::FILTERS.iter().position(|x| *x == f).unwrap_or(0) as i32;
            let n = state::FILTERS.len() as i32;
            let next = state::FILTERS[(i + detents.signum()).rem_euclid(n) as usize];
            browser.input(Input::Filter(next));
        }
        2 => return QLinkEffect::PreviewLevel(detents as f32),
        3 => browser.dial_places(detents.signum()),
        _ => {}
    }
    QLinkEffect::None
}
