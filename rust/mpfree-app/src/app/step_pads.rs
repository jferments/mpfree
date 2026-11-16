//! STEP SEQ: the pads as steps (sequencer decision 2, Jay 16 Nov 2026,
//! card "Pads become steps, as designed"; `docs/dev/design/step-pads.md`).
//!
//! **The lit mode button says what the pads are.** With STEP SEQ lit, the
//! pads are the steps of one sound of the clip Beats shows (the armed
//! track's), 16 to a page, on every page, until STEP SEQ is pressed again.
//! There is one pad mode at a time: STEP SEQ, TRACK MUTE and 16 LEVEL each
//! turn the others off.
//!
//! - **The sound** is the pad played last when STEP SEQ goes on. Held,
//!   STEP SEQ gives the pads back to the kit: the pad tapped sounds, and
//!   on letting go its steps are on the pads. A quick press with no pad
//!   tapped turns the mode on or off.
//! - **The page:** PAD BANK A to D are pages 1 to 4, SHIFT or a second
//!   press 5 to 8, as TRACK MUTE picks banks of tracks. SHIFT + STEP SEQ
//!   turns Follow on, so the page turns with the playhead; picking a page
//!   by hand turns it off.
//! - **ERASE + PAD BANK** clears the sound's steps on that page.
//! - **COPY** held: a step pad picks a step and each one after takes a
//!   copy (`step_keys.rs`); a PAD BANK picks a page and the next one takes
//!   a copy of it, the sound's steps and their locks.
//!
//! The app keeps the mode, the sound, the page and Follow. The audio loop
//! turns the pads' presses into step edits and lights them
//! (`step_keys.rs`), and the screen shows the "Pads: steps" chip and the
//! lane on the pads from [`super::Status::steps`].

use super::{bank_target, App, Effect, MuteKeys, View, LIGHT_OFF, LIGHT_ON};
use crate::play_page::PlayView;
use crate::time::StepAction;

/// Pages the pads reach: 8 of 16 steps, the 128 steps a pattern can have.
pub const PAGES: usize = 8;

/// Steps on a page of pads.
pub const PAGE: usize = 16;

/// What the pads are while STEP SEQ is on or held, for the screen and the
/// audio loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepPads {
    /// The sound (a pad of the kit, 0 to 15) whose steps the pads are.
    pub sound: u8,
    /// The page of 16 steps on the pads (0: steps 1 to 16).
    pub page: usize,
    /// Follow: the page turns with the playhead.
    pub follow: bool,
    /// STEP SEQ is held: the pads play the kit, and the pad tapped is the
    /// next sound.
    pub picking: bool,
    /// COPY is held: a step pad picks the step to copy, then takes the
    /// copy, and so does a PAD BANK for a page.
    pub copying: bool,
    /// COPY + a PAD BANK picked this page to copy; the next PAD BANK
    /// takes it.
    pub copy_page: Option<usize>,
}

impl StepPads {
    /// The first step on the pads, in the clip.
    pub fn first(&self) -> usize {
        self.page * PAGE
    }
}

/// STEP SEQ's state in the app.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct StepSeq {
    on: bool,
    sound: u8,
    page: usize,
    follow: bool,
    /// STEP SEQ is down.
    held: bool,
    /// A pad was tapped while STEP SEQ was down.
    picked: bool,
    /// The pad played last: the sound when STEP SEQ goes on.
    last_pad: u8,
    /// COPY is down.
    copy_held: bool,
    /// COPY + PAD BANK: the page picked to copy.
    copy_page: Option<usize>,
}

impl App {
    /// The pads as steps: while STEP SEQ is on, or held to pick a sound.
    /// None while they play, or are mute keys.
    pub fn step_pads(&self) -> Option<StepPads> {
        let s = &self.step_seq;
        (s.on || s.held).then_some(StepPads {
            // Held with a pad tapped, or on: the sound; held to turn it
            // on: the pad played last, until one is tapped.
            sound: if s.on || s.picked {
                s.sound
            } else {
                s.last_pad
            },
            page: s.page,
            follow: s.follow,
            picking: s.held,
            copying: s.copy_held,
            copy_page: s.copy_page,
        })
    }

    /// COPY is held: a step pad pressed now picks a step to copy, or takes
    /// the copy.
    pub fn copy_held(&self) -> bool {
        self.step_seq.copy_held
    }

    /// COPY pressed or let go. Its job is STEP SEQ's for now (the layout
    /// spec's section 6): while the pads are steps, the screen says what
    /// it does.
    pub(super) fn copy_button(&mut self, pressed: bool) -> Vec<Effect> {
        self.step_seq.copy_held = pressed;
        self.step_seq.copy_page = None;
        if !self.step_seq.on {
            return Vec::new();
        }
        let mut fx = vec![self.status()];
        if pressed {
            fx.push(Effect::Note(
                "COPY held: tap a step, then the steps it goes to (a PAD BANK, then another: a page)"
                    .to_string(),
            ));
        }
        fx
    }

    /// A pad press is a step edit now: STEP SEQ is on and not held.
    pub fn pads_are_steps(&self) -> bool {
        self.step_seq.on && !self.step_seq.held
    }

    /// A pad played (on the controller or the screen): the next sound for
    /// STEP SEQ, and while STEP SEQ is held, the sound picked.
    pub(super) fn step_pad_played(&mut self, slot: usize) {
        let Ok(pad) = u8::try_from(slot) else { return };
        if pad >= 16 {
            return;
        }
        self.step_seq.last_pad = pad;
        if self.step_seq.held {
            self.step_seq.picked = true;
            self.step_seq.sound = pad;
        }
    }

    /// STEP SEQ pressed: SHIFT + STEP SEQ is Follow; alone it waits for
    /// the release, which turns the pads into steps or back (or, with a
    /// pad tapped while held, picks the sound).
    pub(super) fn step_seq_down(&mut self, shift: bool) -> Vec<Effect> {
        if shift {
            self.step_seq.follow = !self.step_seq.follow;
            return vec![
                self.status(),
                Effect::Note(format!(
                    "SHIFT + STEP SEQ: Follow {}",
                    if self.step_seq.follow {
                        "on (the pads turn the page with the playhead)"
                    } else {
                        "off"
                    }
                )),
                Effect::BankLights,
            ];
        }
        self.step_seq.held = true;
        self.step_seq.picked = false;
        vec![
            self.status(),
            // While held, PAD BANK shows the kit's bank again.
            Effect::BankLights,
        ]
    }

    /// STEP SEQ let go.
    pub(super) fn step_seq_up(&mut self) -> Vec<Effect> {
        if !self.step_seq.held {
            return Vec::new();
        }
        self.step_seq.held = false;
        if self.step_seq.picked {
            // Already on, only the sound changes and the page stays.
            let open = !self.step_seq.on;
            return self.steps_on(self.step_seq.sound, open);
        }
        if self.step_seq.on {
            self.steps_off()
        } else {
            self.steps_on(self.step_seq.last_pad, true)
        }
    }

    /// The pads become `sound`'s steps: TRACK MUTE and 16 Levels go off
    /// (one pad mode at a time), and with `open` Play shows Beats, as
    /// STEP SEQ always did.
    fn steps_on(&mut self, sound: u8, open: bool) -> Vec<Effect> {
        self.step_seq.on = true;
        self.step_seq.sound = sound.min(15);
        let mut fx = Vec::new();
        if self.mute_keys != MuteKeys::Off {
            self.mute_keys = MuteKeys::Off;
            if let Some(n) = self.map.track_mute {
                fx.push(Effect::Light(n, LIGHT_OFF));
            }
        }
        if self.levels_on {
            // The screen turns them off and says so (levels_shown).
            fx.push(Effect::SixteenLevels);
        }
        if let Some(n) = self.map.step_seq {
            fx.push(Effect::Light(n, LIGHT_ON));
        }
        fx.push(self.status());
        fx.push(Effect::Note(format!(
            "STEP SEQ: the pads are pad {}'s steps, page {}",
            self.step_seq.sound + 1,
            self.step_seq.page + 1
        )));
        fx.push(Effect::BankLights);
        if open {
            fx.extend(self.go(View::Play));
            fx.push(Effect::PlayView(Some(PlayView::Beats)));
        }
        fx
    }

    /// The pads play again: STEP SEQ again, TRACK MUTE, 16 LEVEL, or a tap
    /// on the "Pads: steps" chip.
    pub fn steps_off(&mut self) -> Vec<Effect> {
        if !self.step_seq.on {
            return Vec::new();
        }
        self.step_seq.on = false;
        let mut fx = Vec::new();
        if let Some(n) = self.map.step_seq {
            fx.push(Effect::Light(n, LIGHT_OFF));
        }
        fx.push(self.status());
        fx.push(Effect::Note("STEP SEQ: the pads play again".to_string()));
        fx.push(Effect::BankLights);
        fx
    }

    /// PAD BANK while the pads are steps: the page (SHIFT or a second
    /// press: 5 to 8). Picking a page turns Follow off. With ERASE held,
    /// the sound's steps on that page are cleared instead.
    pub(super) fn step_page_button(&mut self, button: u8, shift: bool) -> Vec<Effect> {
        let s = self.step_seq;
        // ERASE or COPY + PAD BANK: the page that button lights now
        // (SHIFT: the other four).
        let group = (s.page / 4 * 4 + if shift { 4 } else { 0 }) % PAGES;
        let lit = group + (button as usize & 3);
        if self.erase_held {
            return vec![Effect::Steps(StepAction::ClearSteps {
                pad: s.sound,
                from: lit * PAGE,
                to: (lit + 1) * PAGE,
            })];
        }
        if s.copy_held {
            return match s.copy_page {
                Some(from) if from != lit => vec![Effect::Steps(StepAction::CopySteps {
                    pad: s.sound,
                    from: from * PAGE,
                    to: lit * PAGE,
                    count: PAGE,
                })],
                _ => {
                    self.step_seq.copy_page = Some(lit);
                    let line = format!(
                        "COPY: page {} picked; press the PAD BANK it goes to",
                        lit + 1
                    );
                    vec![self.status(), Effect::Tell(line)]
                }
            };
        }
        let page = bank_target(s.page, button, shift);
        self.step_seq.page = page;
        let mut line = format!(
            "STEP SEQ · page {} (steps {}-{})",
            page + 1,
            page * PAGE + 1,
            (page + 1) * PAGE
        );
        if s.follow {
            self.step_seq.follow = false;
            line.push_str(" · Follow off");
        }
        vec![self.status(), Effect::Note(line), Effect::BankLights]
    }

    /// The screen's ‹ and › on Beats while the pads are steps: the pads
    /// show the same page. Follow goes off, as with PAD BANK.
    pub fn set_step_page(&mut self, page: usize) -> Vec<Effect> {
        let page = page.min(PAGES - 1);
        if !self.step_seq.on || (page == self.step_seq.page && !self.step_seq.follow) {
            return Vec::new();
        }
        self.step_seq.page = page;
        self.step_seq.follow = false;
        vec![self.status(), Effect::BankLights]
    }

    /// Follow: the audio loop's page of the playhead. The pads (and Beats)
    /// turn to it.
    pub fn follow_page(&mut self, page: usize) -> Vec<Effect> {
        let page = page.min(PAGES - 1);
        if !self.step_seq.on || !self.step_seq.follow || page == self.step_seq.page {
            return Vec::new();
        }
        self.step_seq.page = page;
        vec![self.status(), Effect::BankLights]
    }
}
