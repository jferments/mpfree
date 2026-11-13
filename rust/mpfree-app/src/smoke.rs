//! The control audit's smoke tests (Jay, 13 Nov 2026: "look at every
//! button and make sure that it actually does something when touched").
//!
//! Every widget lists the controls it draws ([`mpfree_ui::Widget::targets`]).
//! The sweep here opens every sample screen ([`gui::sample_screens`]),
//! presses each control on it the way a finger would (a tap, a drag or a
//! long press, through [`Player::touch`] as the touchscreen does) and checks
//! that something happened: an action for the app or the audio thread, or a
//! change on the screen. A control that does neither is dead, and the test
//! names it. The catalogue (`docs/dev/guides/controls.md`) explains every
//! kind of control by its widget and id, and the test checks that it does.
//!
//! The workflows in `smoke/workflows.rs` go further: they drive the screen
//! and the app together (with the mixer, the time engine and the chain
//! editor behind them, as `player_session` wires them) through what a
//! musician does, and check the result.

use crate::gui::{self, FromUi, Player};
use mpfree_ui::{Fonts, Gesture, PageTarget, SoftCanvas, Touch};

mod catalogue;
mod session;
pub mod walk;
mod workflows;

pub use session::Session;

/// The finger the sweep uses (any number the screen has not seen).
const FINGER: u64 = 900;

/// What pressing a control did.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// It asked the app or the audio thread for something.
    Acted(Vec<String>),
    /// It changed the screen without asking for anything (a selection,
    /// a sub-page, a picker opening).
    Shown,
    /// Nothing at all: a dead control.
    Dead,
    /// Nothing, from a control drawn as unavailable (greyed out).
    Off,
    /// Not pressed: an earlier press took the page away for good, or the
    /// control went (a picker closed).
    Skipped,
}

/// Press `t` as a finger would and gather what the screen asked for.
pub fn press(p: &mut Player, t: &mpfree_ui::Target) -> Vec<FromUi> {
    let (x, y) = t.centre();
    let mut got: Vec<FromUi> = Vec::new();
    got.extend(p.touch(FINGER, Touch::Down, x, y));
    match t.gesture {
        Gesture::Tap => {}
        Gesture::Drag => {
            // Where the control says, in small steps (up and to the right
            // unless it says otherwise): sliders, knob cards, XY pads and
            // markers all move.
            let (dx, dy) = t.towards;
            for k in 1..=6 {
                got.extend(p.touch(FINGER, Touch::Move, x + dx * k / 6, y + dy * k / 6));
            }
        }
        Gesture::Hold => {
            for _ in 0..8 {
                p.tick(0.125);
                got.extend(p.take_actions());
            }
        }
    }
    let (ux, uy) = match t.gesture {
        Gesture::Drag => (x + t.towards.0, y + t.towards.1),
        _ => (x, y),
    };
    got.extend(p.touch(FINGER, Touch::Up, ux, uy));
    got.extend(p.take_actions());
    got
}

/// The screen as drawn now (only what changed is drawn again).
fn snapshot(p: &mut Player, c: &mut SoftCanvas) -> Vec<u8> {
    p.ui.paint(c);
    c.to_rgb()
}

/// One control pressed on one screen.
#[derive(Clone, Debug)]
pub struct Row {
    pub screen: String,
    pub page: String,
    pub widget: String,
    pub target: mpfree_ui::Target,
    pub outcome: Outcome,
}

impl Row {
    /// The catalogue's key for this kind of control: `widget/id`.
    pub fn key(&self) -> String {
        format!("{}/{}", self.widget, self.target.id)
    }
}

/// Press every control on `p`'s page, one at a time, and say what each did.
/// After each, the page is put back if the control left it: by showing the
/// view again, by pressing the controls like it (the tab back), or, when
/// neither works, with a fresh copy of the screen from `fresh`.
pub fn sweep(screen: &str, p: &mut Player, fresh: &mut dyn FnMut() -> Player) -> Vec<Row> {
    let (w, h) = gui::SCREEN;
    let mut c = SoftCanvas::new(w, h);
    let page = p.page().to_string();
    let view = p.view();
    // Painted first: some widgets (the top bar's chips, the Play page's
    // transport) measure their controls when they draw.
    snapshot(p, &mut c);
    let targets: Vec<PageTarget> = p.ui.targets();
    let mut rows = Vec::new();
    let mut last: Option<String> = None;
    for pt in targets {
        if let Some(widget) = last.take() {
            if !way_back(p, &page, view, &widget) {
                if debug() {
                    eprintln!(
                        "{screen}: a fresh copy after {widget} left {page} at {}",
                        p.page()
                    );
                }
                *p = fresh();
            }
        }
        // The control as it stands now (an earlier press may have moved it,
        // or taken it away: then on the screen as it first was).
        let find = |p: &mut Player, c: &mut SoftCanvas| {
            snapshot(p, c);
            p.ui.targets().into_iter().find(|o| {
                o.widget == pt.widget
                    && o.target.id == pt.target.id
                    && o.target.index == pt.target.index
            })
        };
        let mut now = find(p, &mut c);
        if now.is_none() {
            *p = fresh();
            now = find(p, &mut c);
        }
        let Some(now) = now else {
            if debug() {
                eprintln!(
                    "{screen}: {}/{} #{} is gone",
                    pt.widget, pt.target.id, pt.target.index
                );
            }
            rows.push(Row {
                screen: screen.to_string(),
                page: page.clone(),
                widget: pt.widget,
                target: pt.target,
                outcome: Outcome::Skipped,
            });
            continue;
        };
        // The control as last pressed: faint or not there.
        let mut pressed = now.target.clone();
        let (mut acts, mut changed, mut left) = press_and_see(p, &now.target, &page, &mut c);
        if acts.is_empty() && !changed && !left && now.target.gesture == Gesture::Drag {
            // A slider or knob at the end of its travel moves only the
            // other way.
            let mut back = now.target.clone();
            back.towards = (-back.towards.0, -back.towards.1);
            (acts, changed, left) = press_and_see(p, &back, &page, &mut c);
        }
        if acts.is_empty() && !changed && !left && now.target.enabled {
            // Dead after the presses before it: once more on the screen as
            // it first was, as a finger arriving fresh would find it (an
            // earlier press may have left a selection or a zoom that this
            // press only repeats).
            *p = fresh();
            snapshot(p, &mut c);
            let first = p.ui.targets().into_iter().find(|o| {
                o.widget == now.widget
                    && o.target.id == now.target.id
                    && o.target.index == now.target.index
            });
            if let Some(first) = first {
                (acts, changed, left) = press_and_see(p, &first.target, &page, &mut c);
                pressed = first.target;
            }
        }
        if left {
            last = Some(now.widget.clone());
        }
        let outcome = if !acts.is_empty() {
            Outcome::Acted(acts.iter().map(short).collect())
        } else if changed || left {
            Outcome::Shown
        } else if !pressed.enabled {
            Outcome::Off
        } else {
            Outcome::Dead
        };
        rows.push(Row {
            screen: screen.to_string(),
            page: page.clone(),
            widget: now.widget,
            target: now.target,
            outcome,
        });
    }
    rows
}

/// Press `t` and see what came of it: the actions, whether the screen
/// changed or told something, and whether the page (or its controls) went.
fn press_and_see(
    p: &mut Player,
    t: &mpfree_ui::Target,
    page: &str,
    c: &mut SoftCanvas,
) -> (Vec<FromUi>, bool, bool) {
    let before = snapshot(p, c);
    let keys = target_keys(p);
    let tells = p.tells();
    let acts = press(p, t);
    let changed = snapshot(p, c) != before || p.tells() != tells;
    let left = p.page() != page || target_keys(p) != keys;
    (acts, changed, left)
}

fn debug() -> bool {
    std::env::var("MPFREE_AUDIT_DEBUG").is_ok()
}

/// The controls on the page now, by widget, id and index.
fn target_keys(p: &Player) -> Vec<(String, &'static str, usize)> {
    p.ui.targets()
        .into_iter()
        .map(|o| (o.widget, o.target.id, o.target.index))
        .collect()
}

/// Bring `p` back to `page` after a press of one of `widget`'s controls took
/// it away: show the view again, or press `widget`'s other controls (a tab
/// bar's tab back). True when the page is back.
fn way_back(p: &mut Player, page: &str, view: crate::app::View, widget: &str) -> bool {
    if p.page() == page {
        return true;
    }
    p.apply(gui::ToUi::Show(view));
    if p.page() == page {
        return true;
    }
    let mut tried: Vec<(&'static str, usize)> = Vec::new();
    for _ in 0..12 {
        let next = p.ui.targets().into_iter().find(|o| {
            o.widget == widget
                && o.target.gesture == Gesture::Tap
                && !tried.contains(&(o.target.id, o.target.index))
        });
        let Some(next) = next else { return false };
        tried.push((next.target.id, next.target.index));
        press(p, &next.target);
        if p.page() == page {
            return true;
        }
    }
    false
}

/// An action, short enough for a report line.
pub fn short(a: &FromUi) -> String {
    let s = format!("{a:?}");
    let s: String = s.chars().take(80).collect();
    s
}

/// Every sample screen, swept. `sample_screens` builds every screen in one
/// frame, which outgrows a debug build's 2 MiB test thread, so this runs on
/// a thread with the main thread's 8 MiB (as `every_sample_screen_draws`).
/// A screen a press cannot be brought back from is taken fresh from a spare
/// set of screens (built when there is none left for it).
pub fn sweep_all() -> Vec<Row> {
    on_big_stack(|| {
        let mut rows = Vec::new();
        let first: Vec<(&'static str, Player)> = gui::sample_screens(Fonts::embedded);
        let mut spares: Vec<Vec<Option<Player>>> = Vec::new();
        for (i, (name, mut p)) in first.into_iter().enumerate() {
            let mut fresh = || {
                if let Some(p) = spares.iter_mut().find_map(|set| set[i].take()) {
                    return p;
                }
                let set: Vec<Option<Player>> = gui::sample_screens(Fonts::embedded)
                    .into_iter()
                    .map(|(_, p)| Some(p))
                    .collect();
                spares.push(set);
                spares
                    .last_mut()
                    .and_then(|set| set[i].take())
                    .expect("the same screens again")
            };
            rows.extend(sweep(name, &mut p, &mut fresh));
            // The spares of this screen are not needed again.
            for set in spares.iter_mut() {
                set[i] = None;
            }
        }
        rows
    })
}

/// Run `f` on a thread with the main thread's stack.
pub fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(f)
        .expect("a thread for the smoke test")
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e))
}

/// The report: one line per control pressed, dead ones first.
pub fn report(rows: &[Row]) -> String {
    let mut out = String::from("| outcome | screen | widget/id | # | label | gesture | did |\n|---|---|---|---|---|---|---|\n");
    let mut sorted: Vec<&Row> = rows.iter().collect();
    sorted.sort_by_key(|r| !matches!(r.outcome, Outcome::Dead));
    for r in sorted {
        let (o, did) = match &r.outcome {
            Outcome::Acted(a) => ("acted", a.join("; ")),
            Outcome::Shown => ("shown", String::new()),
            Outcome::Dead => ("DEAD", String::new()),
            Outcome::Off => ("off", String::new()),
            Outcome::Skipped => ("skipped", String::new()),
        };
        out.push_str(&format!(
            "| {o} | {} | {} | {} | {} | {:?} | {} |\n",
            r.screen,
            r.key(),
            r.target.index,
            r.target.label.replace('|', "/"),
            r.target.gesture,
            did.replace('|', "/"),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sweep, once for the tests that read it (it takes a few
    /// minutes in a debug build).
    fn swept() -> &'static [Row] {
        static ROWS: std::sync::OnceLock<Vec<Row>> = std::sync::OnceLock::new();
        ROWS.get_or_init(|| {
            let rows = sweep_all();
            if let Ok(path) = std::env::var("MPFREE_AUDIT_OUT") {
                std::fs::write(&path, report(&rows)).expect("the audit report is written");
            }
            rows
        })
    }

    /// Every control on every sample screen does something when pressed.
    #[test]
    fn every_control_on_every_screen_does_something() {
        let rows = swept();
        let dead: Vec<&Row> = rows
            .iter()
            .filter(|r| r.outcome == Outcome::Dead)
            .filter(|r| !catalogue::known_dead(&r.key()))
            .collect();
        let lines: Vec<String> = dead
            .iter()
            .map(|r| {
                format!(
                    "{} on {}: {} #{} \"{}\"",
                    r.key(),
                    r.screen,
                    r.target.id,
                    r.target.index,
                    r.target.label
                )
            })
            .collect();
        println!("{} controls pressed, {} dead", rows.len(), lines.len());
        assert!(lines.is_empty(), "dead controls:\n{}", lines.join("\n"));
    }

    /// Every kind of control the sweep finds is explained in the controls
    /// catalogue (Jay, 13 Nov: "for every single button/control in the app,
    /// you should have a clear explanation of what the UI control is for,
    /// how it is supposed to be used, and an example workflow").
    #[test]
    fn every_swept_control_is_in_the_catalogue() {
        let text = std::fs::read_to_string(catalogue::PATH).expect("docs/dev/guides/controls.md");
        let listed = catalogue::keys(&text);
        let mut missing: Vec<String> = swept()
            .iter()
            .map(Row::key)
            .filter(|k| !listed.contains(k))
            .collect();
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "controls with no entry in docs/dev/guides/controls.md: {missing:?}"
        );
    }
}
