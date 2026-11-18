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
//! **On the synth** (the pads play it: a melodic track), the pads are the
//! steps of the whole line, chords included ([`LINE`]), and a new step
//! plays the **pen**: the pad or pads picked last. Hold STEP SEQ and the
//! pads are the synth's keys: tap a note, or hold a chord, then let go.
//! STEP SEQ goes on with the pad played last as the pen.
//!
//! The app keeps the mode, the sound, the pen, the page and Follow. The audio loop
//! turns the pads' presses into step edits and lights them
//! (`step_keys.rs`), and the screen shows the "Pads: steps" chip and the
//! lane on the pads from [`super::Status::steps`].

use super::{bank_target, App, Effect, MuteKeys, View, LIGHT_OFF, LIGHT_ON};
use crate::time::{StepAction, LINE, MAX_PEN};
use mpfree_time_ui::editor::EditAction;
use mpfree_time_ui::TimeAction;

/// Pages the pads reach: 8 of 16 steps, the 128 steps a pattern can have.
pub const PAGES: usize = 8;

/// Steps on a page of pads.
pub const PAGE: usize = 16;

/// What the pads are while STEP SEQ is on or held, for the screen and the
/// audio loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepPads {
    /// The sound (a pad of the kit, 0 to 127) whose steps the pads are, or
    /// on the synth [`LINE`], the whole line.
    pub sound: u8,
    /// On the synth: the pads a new step plays (a note, or a chord).
    pub pen: Pen,
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

    /// The pads are the steps of the whole line (the synth plays them).
    pub fn line(&self) -> bool {
        self.sound == LINE
    }
}

/// STEP SEQ's pen on the synth: the pads (synth pads, 0 to 15, each a
/// note or a chord) a new step plays, in the order they were played.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pen {
    pads: [u8; MAX_PEN],
    n: u8,
}

impl Pen {
    /// A pen of one pad.
    pub fn of(pad: u8) -> Pen {
        let mut p = Pen::default();
        p.add(pad);
        p
    }

    pub fn pads(&self) -> &[u8] {
        &self.pads[..usize::from(self.n)]
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// `pad` joins the pen (a chord), if it isn't in it and there is room.
    pub fn add(&mut self, pad: u8) {
        if !self.pads().contains(&pad) && usize::from(self.n) < MAX_PEN {
            self.pads[usize::from(self.n)] = pad;
            self.n += 1;
        }
    }

    /// The pen as a step action's keys, each at `velocity`.
    pub fn keys(&self, velocity: u8) -> ([(u8, u8); MAX_PEN], u8) {
        let mut keys = [(0, 0); MAX_PEN];
        for (k, &p) in self.pads().iter().enumerate() {
            keys[k] = (p, velocity);
        }
        (keys, self.n)
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
    /// On the synth: the pen, and the pads held while STEP SEQ is (a pad
    /// played with none of them held starts a new pen).
    pen: Pen,
    pen_down: u16,
}

impl App {
    /// The pads as steps: while STEP SEQ is on, or held to pick a sound.
    /// None while they play, or are mute keys.
    pub fn step_pads(&self) -> Option<StepPads> {
        let s = &self.step_seq;
        (s.on || s.held).then_some(StepPads {
            // Held with a pad tapped, or on: the sound; held to turn it
            // on: the pad played last, until one is tapped. On the synth,
            // the whole line.
            sound: if self.pad_layout.is_some() {
                LINE
            } else if s.on || s.picked {
                s.sound
            } else {
                s.last_pad
            },
            pen: if s.pen.is_empty() {
                Pen::of(s.last_pad)
            } else {
                s.pen
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

    /// COPY pressed or let go. While the pads are steps it is STEP SEQ's
    /// (the layout spec's section 6), and the screen says what it does;
    /// on Arrange, a tap on a clip copies it and one on an empty slot
    /// pastes (sequencer S3): the status carries it either way.
    pub(super) fn copy_button(&mut self, pressed: bool) -> Vec<Effect> {
        self.step_seq.copy_held = pressed;
        self.step_seq.copy_page = None;
        if !self.step_seq.on {
            return vec![self.status()];
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
    /// STEP SEQ, and while STEP SEQ is held, the sound picked; on the
    /// synth, the pen (pads held together are a chord).
    pub(super) fn step_pad_played(&mut self, slot: usize) {
        if slot >= 16 {
            return;
        }
        // A kit's pad on the page (pads 1 to 128); on the synth, the slot.
        let Ok(sound) = u8::try_from(self.kit_pad(slot)) else {
            return;
        };
        let pad = slot as u8;
        self.step_seq.last_pad = sound;
        if self.step_seq.held {
            self.step_seq.picked = true;
            self.step_seq.sound = sound;
            if self.pad_layout.is_some() {
                let s = &mut self.step_seq;
                if s.pen_down == 0 {
                    s.pen = Pen::default();
                }
                s.pen.add(pad);
                s.pen_down |= 1 << pad;
            }
        }
    }

    /// A pad let go: while STEP SEQ is held, the next pad played starts a
    /// new pen once none is down.
    pub(super) fn step_pad_let_go(&mut self, slot: usize) {
        if slot < 16 {
            self.step_seq.pen_down &= !(1u16 << slot);
        }
    }

    /// The pad a keyboard's key `note` puts on a step of the line: the
    /// pad whose note (a chord pad's lowest) it is. None with a kit, or a
    /// note no pad starts on.
    pub fn pad_of_key(&self, note: u8) -> Option<u8> {
        let layout = self.pad_layout.as_ref()?;
        let pad = layout
            .iter()
            .take(16)
            .position(|p| p.notes().first() == Some(&note))?;
        u8::try_from(pad).ok()
    }

    /// The pen's notes by name, for the lines and the chip: "C3", or a
    /// chord's "C3 Eb3 G3".
    pub fn pen_name(&self, pen: &Pen) -> String {
        let Some(layout) = &self.pad_layout else {
            return String::new();
        };
        let mut notes: Vec<u8> = pen
            .pads()
            .iter()
            .filter_map(|&p| layout.get(usize::from(p)))
            .flat_map(|n| n.notes().iter().copied())
            .collect();
        notes.sort_unstable();
        notes.dedup();
        let names: Vec<String> = notes.iter().map(|&n| crate::synth::note_name(n)).collect();
        match names.len() {
            0 => "no note".to_string(),
            1..=4 => names.join(" "),
            k => format!("{} +{}", names[..3].join(" "), k - 3),
        }
    }

    /// The row the pads' steps are: the sound, or on the synth the line.
    fn step_row(&self) -> u8 {
        self.step_pads().map_or(self.step_seq.sound, |p| p.sound)
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
        self.step_seq.pen_down = 0;
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
    /// (one pad mode at a time), and with `open` the Sequencer shows its
    /// Steps (Beats' pages, which were Play's until the redesign).
    fn steps_on(&mut self, sound: u8, open: bool) -> Vec<Effect> {
        self.step_seq.on = true;
        self.step_seq.sound = sound.min(127);
        let mut fx = Vec::new();
        // The loop keys give the pads up too.
        fx.extend(self.loop_keys_off());
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
        let line = match self.step_pads() {
            Some(p) if p.line() => format!(
                "STEP SEQ: the pads are the line's steps, page {}; new steps play {} (hold STEP SEQ and play the pads to change it)",
                self.step_seq.page + 1,
                self.pen_name(&p.pen)
            ),
            _ => format!(
                "STEP SEQ: the pads are pad {}'s steps, page {}",
                super::kit_pad_name(usize::from(self.step_seq.sound)),
                self.step_seq.page + 1
            ),
        };
        fx.push(Effect::Note(line));
        fx.push(Effect::BankLights);
        if open {
            // The Sequencer's Steps: the note editor closes (‹ Steps).
            fx.extend(self.go(View::Sequencer));
            if self.notes_open {
                fx.push(Effect::Time(TimeAction::Edit(EditAction::Button(0))));
            }
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
        let row = self.step_row();
        // ERASE or COPY + PAD BANK: the page that button lights now
        // (SHIFT: the other four).
        let group = (s.page / 4 * 4 + if shift { 4 } else { 0 }) % PAGES;
        let lit = group + (button as usize & 3);
        if self.erase_held {
            return vec![Effect::Steps(StepAction::ClearSteps {
                pad: row,
                from: lit * PAGE,
                to: (lit + 1) * PAGE,
            })];
        }
        if s.copy_held {
            return match s.copy_page {
                Some(from) if from != lit => vec![Effect::Steps(StepAction::CopySteps {
                    pad: row,
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
