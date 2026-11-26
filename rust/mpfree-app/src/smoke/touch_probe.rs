//! The touch audit's live probes (layout spec 1a, "How a control acts" and
//! "Tested"): every kind of control, on the first sample screen where it
//! can be pressed, touched the ways that must not commit, with what the
//! screen asks the app for compared with what the control declares.
//!
//! - **Landing.** A finger lands on the control and is cancelled. A control
//!   that declares Press must ask for something as it lands; one that
//!   declares a lift or a hold must not; a drag that follows the finger
//!   from where it lands (`Movement::Relative`) must not either, since
//!   landing moves nothing.
//! - **Cancel.** A cancelled touch never commits: a control quiet as the
//!   finger lands asks for nothing when that finger is cancelled.
//! - **Slide off.** A finger lands, slides off the control's widget and
//!   lifts there: a control quiet as it lands asks for nothing.
//!
//! Only what the screen asks the app for counts (`FromUi`), less the
//! notes that say how the screen stands (`Outline`, `Held`, `UndoReady`,
//! and the chain editor's `Focus`, which only says what the wheel turns):
//! a control that only changes the screen isn't judged here, and a press's
//! own look isn't a commit. Today's breaks are listed in
//! golden/touch-acts.txt, each with a reason; a break not listed fails, and
//! blessing writes the list, keeping each line's reason:
//!
//!     MPFREE_TOUCH_BLESS=1 cargo test -p mpfree-app --bin mpfree touch_probe
//!
//! The probes press each kind once, on one set of screens: a probe that
//! takes the page away is followed by the view shown again, and when that
//! doesn't bring it back the screen's other kinds wait for a later screen.

use crate::gui::{self, FromUi, Player};
use mpfree_ui::{Commit, Fonts, Gesture, Movement, Rect, SoftCanvas, Target, Touch};
use std::collections::{BTreeMap, BTreeSet};

/// The finger the probes use (any number the screen has not seen).
const FINGER: u64 = 901;

/// How far outside its widget a sliding finger lifts.
const OFF: i32 = 24;

/// A way a control breaks the rules of how it acts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Break {
    /// It declares a lift or a hold, and asks for something as the finger
    /// lands.
    ActsOnLanding,
    /// It declares Press, and asks for nothing as the finger lands.
    QuietOnLanding,
    /// A drag that follows the finger from where it lands asks for
    /// something as the finger lands.
    LandingMoves,
    /// Quiet as the finger lands, it asks for something when that finger
    /// is cancelled.
    CancelCommits,
    /// Quiet as the finger lands, it asks for something when the finger
    /// slides off its widget and lifts there.
    SlideOffCommits,
}

impl Break {
    const ALL: [Break; 5] = [
        Break::ActsOnLanding,
        Break::QuietOnLanding,
        Break::LandingMoves,
        Break::CancelCommits,
        Break::SlideOffCommits,
    ];

    /// Its name in golden/touch-acts.txt.
    pub fn name(self) -> &'static str {
        match self {
            Break::ActsOnLanding => "acts-on-landing",
            Break::QuietOnLanding => "quiet-on-landing",
            Break::LandingMoves => "landing-moves",
            Break::CancelCommits => "cancel-commits",
            Break::SlideOffCommits => "slide-off-commits",
        }
    }

    fn named(name: &str) -> Option<Break> {
        Break::ALL.into_iter().find(|b| b.name() == name)
    }
}

/// What the screen asked for, less the notes on how it stands.
fn commits(acts: Vec<FromUi>) -> Vec<FromUi> {
    acts.into_iter()
        .filter(|a| {
            !matches!(
                a,
                FromUi::Outline(_)
                    | FromUi::Held(_)
                    | FromUi::UndoReady(_)
                    | FromUi::Chain(mpfree_chain_ui::ChainAction::Focus(_))
            )
        })
        .collect()
}

/// A point `OFF` px outside `slot` (the control's widget), straight above
/// or below the control's middle, or beside it where neither is on the
/// screen.
fn outside(slot: Rect, x: i32, y: i32) -> (i32, i32) {
    let (sw, sh) = gui::SCREEN;
    let above = slot.y - OFF;
    let below = slot.y + slot.h + OFF;
    let left = slot.x - OFF;
    let right = slot.x + slot.w + OFF;
    if above >= 0 {
        (x, above)
    } else if below < sh {
        (x, below)
    } else if left >= 0 {
        (left, y)
    } else if right < sw {
        (right, y)
    } else {
        (x, sh - 1)
    }
}

/// What came of the three touches on one control.
#[derive(Debug, Default)]
pub struct Probe {
    pub landing: Vec<FromUi>,
    pub cancel: Vec<FromUi>,
    pub slide: Vec<FromUi>,
    /// A plain tap, only for a control that declares Press and was quiet
    /// as the finger landed: a control whose tap asks for nothing only
    /// changes the screen, which the probes don't judge.
    pub tap: Vec<FromUi>,
}

/// Land on `t` and cancel; then, if that asked for nothing and it isn't a
/// drag, land again, slide off its widget (`slot`) and lift there; and if
/// it declares Press, tap it.
pub fn probe(p: &mut Player, t: &Target, slot: Rect) -> Probe {
    let (x, y) = t.centre();
    let mut landing: Vec<FromUi> = p.touch(FINGER, Touch::Down, x, y).into_iter().collect();
    landing.extend(p.take_actions());
    let mut cancel: Vec<FromUi> = p.touch(FINGER, Touch::Cancel, x, y).into_iter().collect();
    cancel.extend(p.take_actions());
    let landing = commits(landing);
    let mut slide = Vec::new();
    if landing.is_empty() && t.gesture != Gesture::Drag {
        let (ox, oy) = outside(slot, x, y);
        slide.extend(p.touch(FINGER, Touch::Down, x, y));
        slide.extend(p.touch(FINGER, Touch::Move, ox, oy));
        slide.extend(p.touch(FINGER, Touch::Up, ox, oy));
        slide.extend(p.take_actions());
    }
    let mut tap = Vec::new();
    if landing.is_empty() && t.gesture == Gesture::Tap && t.commit == Commit::Press {
        tap.extend(p.touch(FINGER, Touch::Down, x, y));
        tap.extend(p.touch(FINGER, Touch::Up, x, y));
        tap.extend(p.take_actions());
    }
    Probe {
        landing,
        cancel: commits(cancel),
        slide: commits(slide),
        tap: commits(tap),
    }
}

/// The rules `t` breaks, from what its probe found.
pub fn judge(t: &Target, found: &Probe) -> Vec<Break> {
    let mut out = Vec::new();
    let landed = !found.landing.is_empty();
    if t.gesture == Gesture::Drag {
        if landed && t.movement == Movement::Relative {
            out.push(Break::LandingMoves);
        }
        return out;
    }
    match (t.commit, landed) {
        (Commit::Press, false) if !found.tap.is_empty() => out.push(Break::QuietOnLanding),
        (Commit::LiftInside | Commit::Hold, true) => out.push(Break::ActsOnLanding),
        _ => {}
    }
    if !landed {
        if !found.cancel.is_empty() {
            out.push(Break::CancelCommits);
        }
        // A control that moves (a note, a block, a value that follows the
        // finger) moves when the finger slides: that is its drag.
        if !found.slide.is_empty() && t.movement == Movement::None {
            out.push(Break::SlideOffCommits);
        }
    }
    out
}

/// One break found: the kind of control, the screen, the label and what
/// was asked for.
#[derive(Clone, Debug)]
pub struct Found {
    pub screen: String,
    pub label: String,
    pub asked: String,
}

/// What the probes found: every break by (kind, break), and the kinds
/// probed.
pub struct Probed {
    pub breaks: BTreeMap<(String, Break), Found>,
    pub kinds: BTreeSet<String>,
    pub seen: BTreeSet<String>,
}

/// Probe every kind of control once, on the first sample screen where it
/// is enabled and still there when its turn comes.
pub fn probe_all() -> Probed {
    super::on_big_stack(|| {
        let _alone = gui::all_screens();
        let fonts = Fonts::embedded();
        let (w, h) = gui::SCREEN;
        let mut c = SoftCanvas::new(w, h);
        let mut breaks = BTreeMap::new();
        let mut kinds = BTreeSet::new();
        let mut seen = BTreeSet::new();
        for (screen, mut p) in gui::sample_screens(|| fonts.clone()) {
            let page = p.page().to_string();
            let view = p.view();
            p.ui.paint(&mut c);
            let first = p.ui.targets();
            for pt in &first {
                seen.insert(format!("{}/{}", pt.widget, pt.target.id));
            }
            for pt in first {
                let kind = format!("{}/{}", pt.widget, pt.target.id);
                if kinds.contains(&kind) || !pt.target.enabled {
                    continue;
                }
                if p.page() != page {
                    p.apply(gui::ToUi::Show(view));
                    if p.page() != page {
                        break;
                    }
                }
                // The control as it stands now: an earlier probe may have
                // moved it, or taken it away.
                p.ui.paint(&mut c);
                let Some(now) = p.ui.targets().into_iter().find(|o| {
                    o.widget == pt.widget
                        && o.target.id == pt.target.id
                        && o.target.index == pt.target.index
                        && o.target.enabled
                }) else {
                    continue;
                };
                let Some(slot) =
                    p.ui.slots()
                        .into_iter()
                        .rev()
                        .find(|(name, r)| {
                            let (x, y) = now.target.centre();
                            *name == now.widget && r.contains(x, y)
                        })
                        .map(|(_, r)| r)
                else {
                    continue;
                };
                let found = probe(&mut p, &now.target, slot);
                kinds.insert(kind.clone());
                for b in judge(&now.target, &found) {
                    let asked = found
                        .landing
                        .iter()
                        .chain(&found.cancel)
                        .chain(&found.slide)
                        .chain(&found.tap)
                        .map(super::short)
                        .collect::<Vec<_>>()
                        .join("; ");
                    breaks.entry((kind.clone(), b)).or_insert(Found {
                        screen: screen.to_string(),
                        label: now.target.label.clone(),
                        asked,
                    });
                }
            }
        }
        Probed {
            breaks,
            kinds,
            seen,
        }
    })
}

/// What a line of the list says when it was written by blessing.
const NEW_REASON: &str =
    "before the touch audit (25 Nov 2026 baseline); until its screen is redone";

/// Where the list lives, from this crate.
#[cfg(test)]
const LIST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/golden/touch-acts.txt");

/// golden/touch-acts.txt: (kind, break) to reason.
pub fn read_list(text: &str) -> BTreeMap<(String, Break), String> {
    let mut out = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (fields, reason) = line.split_once('#').unwrap_or((line, ""));
        let f: Vec<&str> = fields.split_whitespace().collect();
        let [kind, b] = f[..] else {
            panic!(
                "golden/touch-acts.txt line {}: expected KIND BREAK # REASON",
                n + 1
            )
        };
        let b = Break::named(b).unwrap_or_else(|| {
            panic!(
                "golden/touch-acts.txt line {}: {b:?} is not one of {:?}",
                n + 1,
                Break::ALL.map(Break::name)
            )
        });
        out.insert((kind.to_string(), b), reason.trim().to_string());
    }
    out
}

pub fn write_list(list: &BTreeMap<(String, Break), String>) -> String {
    let mut text = String::from(
        "# How today's controls break the layout spec's rules of how a control acts\n\
         # (section 1a), found by the touch audit's live probes (smoke/touch_probe.rs):\n\
         #     KIND BREAK # REASON\n\
         # acts-on-landing: declares a lift or a hold, asks for something as the finger lands\n\
         # quiet-on-landing: declares Press, asks for nothing as the finger lands\n\
         # landing-moves: a drag that follows the finger asks for something as it lands\n\
         # cancel-commits: quiet as the finger lands, asks for something when it is cancelled\n\
         # slide-off-commits: quiet as the finger lands, asks for something when it slides off and lifts\n\
         # A break not listed fails. Write the list with\n\
         #     MPFREE_TOUCH_BLESS=1 cargo test -p mpfree-app --bin mpfree touch_probe\n",
    );
    for ((kind, b), reason) in list {
        text.push_str(&format!("{kind} {} # {reason}\n", b.name()));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(commit: Commit) -> Target {
        Target::tap("x", 0, "x", Rect::new(0, 0, 64, 64)).commit(commit)
    }

    fn asked() -> Vec<FromUi> {
        vec![FromUi::Tab(1)]
    }

    #[test]
    fn a_probe_is_judged_by_what_the_control_declares() {
        let quiet = Probe::default();
        let landed = Probe {
            landing: asked(),
            ..Probe::default()
        };
        assert_eq!(judge(&tap(Commit::LiftInside), &quiet), vec![]);
        assert_eq!(judge(&tap(Commit::Press), &landed), vec![]);
        assert_eq!(
            judge(&tap(Commit::LiftInside), &landed),
            vec![Break::ActsOnLanding]
        );
        assert_eq!(judge(&tap(Commit::Press), &quiet), vec![], "screen only");
        let lifted = Probe {
            tap: asked(),
            ..Probe::default()
        };
        assert_eq!(
            judge(&tap(Commit::Press), &lifted),
            vec![Break::QuietOnLanding]
        );
        let slid = Probe {
            slide: asked(),
            cancel: asked(),
            ..Probe::default()
        };
        assert_eq!(
            judge(&tap(Commit::LiftInside), &slid),
            vec![Break::CancelCommits, Break::SlideOffCommits]
        );
        let note = tap(Commit::LiftInside).movement(Movement::Structural);
        assert_eq!(judge(&note, &slid), vec![Break::CancelCommits], "its drag");
        let fader = Target::drag("f", 0, "f", Rect::new(0, 0, 64, 200));
        assert_eq!(judge(&fader, &landed), vec![Break::LandingMoves]);
        let xy = fader.clone().movement(Movement::Absolute);
        assert_eq!(judge(&xy, &landed), vec![]);
    }

    #[test]
    fn the_notes_on_how_the_screen_stands_are_not_commits() {
        let got = commits(vec![
            FromUi::Outline(true),
            FromUi::Held(true),
            FromUi::UndoReady(false),
            FromUi::Tab(2),
        ]);
        assert_eq!(got, vec![FromUi::Tab(2)]);
    }

    #[test]
    fn a_sliding_finger_lifts_outside_the_widget() {
        assert_eq!(outside(Rect::new(12, 84, 400, 300), 50, 90), (50, 60));
        assert_eq!(outside(Rect::new(0, 0, 400, 300), 50, 90), (50, 324));
        assert_eq!(outside(Rect::new(100, 0, 400, 800), 150, 90), (76, 90));
    }

    #[test]
    fn the_list_reads_back_what_it_wrote() {
        let mut list = BTreeMap::new();
        list.insert(
            ("mixer/mute".to_string(), Break::ActsOnLanding),
            "a reason".to_string(),
        );
        list.insert(
            ("clips/slot".to_string(), Break::SlideOffCommits),
            String::new(),
        );
        assert_eq!(read_list(&write_list(&list)), list);
    }

    /// Every kind of control acts as it declares: no cancelled or slid-off
    /// touch commits, no drag that follows the finger moves as it lands,
    /// and the press or the lift is the one it says, unless listed.
    #[test]
    fn every_kind_of_control_acts_as_it_declares() {
        let probed = probe_all();
        println!(
            "touch probes: {} of {} kinds of control probed, {} breaks",
            probed.kinds.len(),
            probed.seen.len(),
            probed.breaks.len()
        );
        assert!(
            probed.kinds.len() * 10 >= probed.seen.len() * 8,
            "only {} of {} kinds of control could be probed",
            probed.kinds.len(),
            probed.seen.len()
        );
        if crate::smoke::debug() {
            for ((kind, b), f) in &probed.breaks {
                eprintln!(
                    "{kind} {} on {} {:?}: {}",
                    b.name(),
                    f.screen,
                    f.label,
                    f.asked
                );
            }
        }
        let old = std::fs::read_to_string(LIST)
            .map(|t| read_list(&t))
            .unwrap_or_default();
        if std::env::var_os("MPFREE_TOUCH_BLESS").is_some() {
            let list: BTreeMap<(String, Break), String> = probed
                .breaks
                .keys()
                .map(|k| {
                    let reason = old
                        .get(k)
                        .filter(|r| !r.is_empty())
                        .cloned()
                        .unwrap_or_else(|| NEW_REASON.to_string());
                    (k.clone(), reason)
                })
                .collect();
            std::fs::write(LIST, write_list(&list)).expect("golden/touch-acts.txt is written");
            eprintln!("wrote {} lines to {LIST}", list.len());
            return;
        }
        let new: Vec<String> = probed
            .breaks
            .iter()
            .filter(|(k, _)| !old.contains_key(*k))
            .map(|((kind, b), f)| {
                format!(
                    "{kind} {}: on {}, {:?} asked for {}",
                    b.name(),
                    f.screen,
                    f.label,
                    if f.asked.is_empty() {
                        "nothing".to_string()
                    } else {
                        f.asked.clone()
                    }
                )
            })
            .collect();
        let gone = old
            .keys()
            .filter(|k| !probed.breaks.contains_key(*k))
            .count();
        if gone > 0 {
            eprintln!(
                "touch probes: {gone} lines of golden/touch-acts.txt no longer break; \
                 bless to write it (MPFREE_TOUCH_BLESS=1)"
            );
        }
        assert!(
            new.is_empty(),
            "{} controls act in a way golden/touch-acts.txt doesn't list:\n{}\n\
             A cancelled touch, or a finger that slides off and lifts, never commits; a drag \
             that follows the finger moves nothing as it lands (layout spec 1a). Where the \
             control acts as it should, declare how it commits (Target::commit, \
             mpfree_ui::touch). If the break is meant, write the list and give its line a \
             reason:\n\
             MPFREE_TOUCH_BLESS=1 cargo test -p mpfree-app --bin mpfree touch_probe",
            new.len(),
            new.join("\n")
        );
    }
}
