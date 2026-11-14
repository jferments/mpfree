//! The Sample tab's two buttons beside its bar, on every editor tab (Jay,
//! 12 Nov 2026: "the sample screen would have a smaller send to pad, chop
//! to pads options"):
//!
//! - **Send to pad:** the next pad hit gets a copy of the pad picked, its
//!   sound and every edit (one undo step). A second press calls it off.
//! - **Chop to pads:** the Chop tab, where the sound is cut and its slices
//!   go onto pads; on the Chop tab it is the tab's own Chop to pads.
//!
//! Each tab's page puts [`SEND`] at the right of its bar row
//! ([`SEND_W`] wide); the app applies the presses.

use mpfree_ui::Ui;

use crate::widgets::ButtonRow;

/// The buttons' widget.
pub const SEND: &str = "smp-send";
/// Its width beside the bar.
pub const SEND_W: i32 = 330;
/// The buttons, in order.
pub const BUTTONS: [&str; 2] = ["Send to pad", "Chop to pads"];
/// Send to pad's label while it waits for the pad hit.
pub const SENDING: &str = "Hit a pad";
/// [`BUTTONS`]' places.
pub const SEND_TO_PAD: usize = 0;
pub const CHOP_TO_PADS: usize = 1;

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
            ["Hit a pad", "Chop to pads"],
            "kept, not added again"
        );
        assert_eq!(b.enabled, [true, true]);
        show(&mut ui, false, false);
        let b = ui.get::<ButtonRow>(SEND).unwrap();
        assert_eq!(b.labels, BUTTONS);
        assert_eq!(b.enabled, [false, false], "faint for a pad without a sound");
    }
}
