//! The Sample tab's three buttons beside its bar, on every editor tab (Jay,
//! 12 Nov 2026: "the sample screen would have a smaller send to pad, chop
//! to pads options"; Stems since the stems plan's step 3):
//!
//! - **Send to pad:** the next pad hit gets a copy of the pad picked, its
//!   sound and every edit (one undo step). A second press calls it off.
//! - **Chop to pads:** the Wave page's Slice tool, where the sound is cut
//!   and its slices go onto pads; with the Slice tool it is the tool's own
//!   Chop to pads.
//! - **Stems:** the Stems page ([`crate::stems_page`]), which splits the
//!   sound into vocals, drums, bass and other; lit while it is open, when
//!   a press takes the page back to Wave.
//!
//! With a take kept on the Wave page (one-page step 2b), Send to pad puts
//! the take itself on the pad hit (it leaves the page, one undo step);
//! Chop to pads waits until it is on a pad ([`show_take`]).
//!
//! Each tab's page puts [`SEND`] at the right of its bar row
//! ([`SEND_W`] wide); the app applies the presses.

use mpfree_ui::Ui;

use crate::widgets::ButtonRow;

/// The buttons' widget.
pub const SEND: &str = "smp-send";
/// Its width beside the bar (the pages' layouts say it as a number).
pub const SEND_W: i32 = 460;
/// The buttons, in order.
pub const BUTTONS: [&str; 3] = ["Send to pad", "Chop to pads", "Stems"];
/// Send to pad's label while it waits for the pad hit.
pub const SENDING: &str = "Hit a pad";
/// [`BUTTONS`]' places.
pub const SEND_TO_PAD: usize = 0;
pub const CHOP_TO_PADS: usize = 1;
pub const STEMS: usize = 2;

/// Adds the buttons to `ui` unless a tab added them already.
pub fn add(ui: &mut Ui) {
    if ui.get::<ButtonRow>(SEND).is_none() {
        ui.add(SEND, ButtonRow::new(&BUTTONS));
    }
}

/// Shows the buttons for the pad picked: faint while it has no sound (on
/// the Rec tab, say), Send to pad saying so while it waits for a pad hit.
pub fn show(ui: &mut Ui, has_sound: bool, sending: bool) {
    if let Some(b) = ui.get_mut::<ButtonRow>(SEND) {
        b.labels[SEND_TO_PAD] = if sending {
            SENDING
        } else {
            BUTTONS[SEND_TO_PAD]
        }
        .to_string();
        b.enabled = vec![has_sound; BUTTONS.len()];
        b.why = vec!["The pad picked has no sound".to_string(); BUTTONS.len()];
    }
}

/// Shows the buttons for a take kept on the page: Send to pad puts it on
/// the pad hit; Chop to pads puts its slices on pads, as a pad's (the one
/// Sample page's step 3b).
pub fn show_take(ui: &mut Ui, sending: bool) {
    if let Some(b) = ui.get_mut::<ButtonRow>(SEND) {
        b.labels[SEND_TO_PAD] = if sending {
            SENDING
        } else {
            BUTTONS[SEND_TO_PAD]
        }
        .to_string();
        b.enabled = vec![true; BUTTONS.len()];
        b.why = vec![String::new(); BUTTONS.len()];
    }
}

/// Lights Stems while the Stems page is open (a press then takes the page
/// back to Wave).
pub fn light_stems(ui: &mut Ui, open: bool) {
    let primary = open.then_some(STEMS);
    // Only a change is drawn again.
    if ui
        .get::<ButtonRow>(SEND)
        .is_some_and(|b| b.primary != primary)
    {
        if let Some(b) = ui.get_mut::<ButtonRow>(SEND) {
            b.primary = primary;
        }
    }
}

/// Stems' label: "Stems", or how far a split has got while the page
/// doesn't show ("Stems 42 %").
pub fn label_stems(ui: &mut Ui, label: &str) {
    if ui
        .get::<ButtonRow>(SEND)
        .is_some_and(|b| b.labels.get(STEMS).is_some_and(|l| l != label))
    {
        if let Some(b) = ui.get_mut::<ButtonRow>(SEND) {
            b.labels[STEMS] = label.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, Theme};

    #[test]
    fn the_buttons_are_added_once_and_say_when_a_send_waits() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        add(&mut ui);
        show(&mut ui, true, true);
        add(&mut ui);
        let b = ui.get::<ButtonRow>(SEND).unwrap();
        assert_eq!(
            b.labels,
            ["Hit a pad", "Chop to pads", "Stems"],
            "kept, not added again"
        );
        assert_eq!(b.enabled, [true, true, true]);
        show(&mut ui, false, false);
        let b = ui.get::<ButtonRow>(SEND).unwrap();
        assert_eq!(b.labels, BUTTONS);
        assert_eq!(
            b.enabled,
            [false, false, false],
            "faint for a pad without a sound"
        );
        light_stems(&mut ui, true);
        assert_eq!(ui.get::<ButtonRow>(SEND).unwrap().primary, Some(STEMS));
        light_stems(&mut ui, false);
        assert_eq!(ui.get::<ButtonRow>(SEND).unwrap().primary, None);
        label_stems(&mut ui, "Stems 42 %");
        assert_eq!(
            ui.get::<ButtonRow>(SEND).unwrap().labels[STEMS],
            "Stems 42 %"
        );
        show(&mut ui, true, false);
        assert_eq!(
            ui.get::<ButtonRow>(SEND).unwrap().labels[STEMS],
            "Stems 42 %",
            "kept while the pad changes"
        );
    }
}
