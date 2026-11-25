//! Continue on Steps (composing tools, step 4) in the app: the Steps
//! head's Continue, the asks it and its panel make of the time engine, the
//! Q-Links while it is open, and the bar strip and panel filled from what
//! the time engine says ([`TimeState::continuing`]). The widgets are
//! [`crate::steps_continue`]'s; the engine is the note editor's Continue,
//! carrying a beat on ([`mpfree_compose::beat`]).

use super::*;
use crate::steps_continue::{self as sc, BarStrip, ContinuePanel, PanelLook, StripBar};
use mpfree_time_ui::continuing::{ContinueAction, ContinueAsk, Scope};

/// The hint while Continue is open on Steps.
pub(super) const HINT: &str = "Continue: Q1 similarity, Q2 variation, Q3 activity (SHIFT: finer)   ·   ‹ › or a bar: page   ·   Done: in, one UNDO   ·   Cancel: the clip as it was";

/// Similarity, Variation and Activity: a step a detent, SHIFT's, least,
/// most (the note editor's Continue's).
const KNOB: (i32, i32, i32, i32) = (5, 1, 0, 100);

impl Player {
    /// Continue is open on the Steps showing (its page in place of
    /// Beats').
    pub(super) fn steps_continuing(&self) -> bool {
        self.steps_cont.is_some()
            && self.play.view == PlayView::Beats
            && matches!(self.view, View::Play | View::Sequencer)
    }

    /// The Steps head shows Continue: a kit track's clip on the
    /// Sequencer's Steps (a synth's notes carry on in Notes).
    pub(super) fn steps_continue_offered(&self) -> bool {
        self.play.sequencer
            && self.play.view == PlayView::Beats
            && !self.play.pitched()
            && self.play.steps.clip.is_some()
    }

    /// The head's Continue: the panel opens and asks at once.
    pub(super) fn steps_continue_open(&mut self) -> Option<FromUi> {
        if !self.steps_continue_offered() {
            self.tell(
                "continue: on Steps, a kit's beat carries on; a synth's notes carry on in Notes",
            );
            return None;
        }
        let set = self.steps_cont.unwrap_or_default();
        self.steps_cont = Some(set);
        self.steps_cont_live = false;
        self.play.page = 0;
        self.show_view(self.view);
        let ask = self.steps_continue_ask();
        self.refresh_play();
        self.refresh_steps_continue();
        self.refresh_qlinks();
        self.refresh_hint();
        ask
    }

    /// An ask with the panel's settings: a new result for the clip the
    /// Steps show, carried on as a beat.
    fn steps_continue_ask(&mut self) -> Option<FromUi> {
        let set = self.steps_cont?;
        let slot = self.play.steps.clip.as_ref()?.slot;
        let track = self.play.steps.track;
        if let Some(parts) = self.compose_parts_if_new() {
            self.outbox.push(parts);
        }
        let lanes = self
            .compose_parts
            .as_ref()
            .map(|p| p.lanes.clone())
            .unwrap_or_default();
        Some(FromUi::Time(TimeAction::Continue(ContinueAction::Ask(
            Box::new(ContinueAsk {
                track,
                slot,
                settings: set.settings(set.q, &self.time.styles),
                kind: mpfree_compose::InstrumentKind::Percussive,
                lanes,
                scope: Scope::All,
                locked: Vec::new(),
            }),
        ))))
    }

    /// Closed: the panel goes, the step editor and timing strip come back.
    fn steps_continue_close(&mut self) {
        self.steps_cont = None;
        self.steps_cont_live = false;
        self.play.page = 0;
        self.show_view(self.view);
        self.refresh_play();
        self.refresh_qlinks();
    }

    /// A touch on the panel or the bar strip.
    pub(super) fn steps_continue_event(&mut self, w: &str, e: EventKind) -> Option<FromUi> {
        let mut set = self.steps_cont?;
        let out = match (w, e) {
            (sc::PANEL, EventKind::Step { index, delta }) if index < sc::ROWS => {
                set.row = index;
                let changed = set.step(sc::SET_ROWS[index], delta, &self.time.styles);
                self.steps_cont = Some(set);
                if !changed {
                    self.refresh_steps_continue();
                    return None;
                }
                let said = set.said(sc::SET_ROWS[index], &self.time.styles);
                self.tell(&format!("continue: {said}"));
                self.steps_continue_ask()
            }
            (sc::PANEL, EventKind::Select(row)) if row < sc::ROWS => {
                set.row = row;
                self.steps_cont = Some(set);
                None
            }
            (sc::PANEL, EventKind::Tab(sc::TRY_AGAIN)) => self.steps_continue_ask(),
            (sc::PANEL, EventKind::Tab(sc::BACK_A_TRY)) => {
                Some(FromUi::Time(TimeAction::Continue(ContinueAction::Step {
                    back: true,
                })))
            }
            (sc::PANEL, EventKind::Tab(sc::CANCEL)) => {
                self.steps_continue_close();
                Some(FromUi::Time(TimeAction::Continue(ContinueAction::Back)))
            }
            (sc::PANEL, EventKind::Tab(sc::DONE)) => {
                self.steps_continue_close();
                Some(FromUi::Time(TimeAction::Continue(ContinueAction::Keep)))
            }
            (sc::BARS, EventKind::Tab(bar)) => {
                self.play.page = self.steps_bar_first(bar) / mpfree_seq::steps::PAGE;
                self.refresh_play();
                None
            }
            _ => None,
        };
        self.refresh_steps_continue();
        out
    }

    /// The wheel on the panel: its picked row, a step a detent.
    pub(super) fn steps_continue_wheel(&mut self, delta: i32) -> Option<FromUi> {
        let row = self.steps_cont?.row;
        self.steps_continue_event(sc::PANEL, EventKind::Step { index: row, delta })
    }

    /// The wheel's push: the panel's next row (round to the first).
    pub(super) fn steps_continue_next_row(&mut self) {
        if let Some(set) = self.steps_cont.as_mut() {
            set.row = (set.row + 1) % sc::ROWS;
        }
        self.refresh_steps_continue();
    }

    /// Q-Link `k` turned while Continue is open: Similarity, Variation or
    /// Activity, and a new ask (the last result plays on until it's
    /// ready). Range turns nothing on drums.
    pub(super) fn steps_continue_knob(&mut self, k: usize, delta: i32) {
        let Some(mut set) = self.steps_cont else {
            return;
        };
        if k >= 3 {
            self.tell("continue: Range has nothing to do on drums");
            return;
        }
        let (step, fine, lo, hi) = KNOB;
        let by = if self.status.shift { fine } else { step };
        let was = set.q[k];
        set.q[k] = (was + delta * by).clamp(lo, hi);
        if set.q[k] == was {
            return;
        }
        self.steps_cont = Some(set);
        self.refresh_qlinks();
        if let Some(a) = self.steps_continue_ask() {
            self.outbox.push(a);
        }
    }

    /// The Q-Links' cards while Continue is open on Steps.
    pub(super) fn steps_continue_cards(&self) -> Option<[QLinkCard; 4]> {
        if !self.steps_continuing() {
            return None;
        }
        let q = self.steps_cont?.q;
        let words = mpfree_time_ui::editor::compose::cards(q);
        let mut cards: [QLinkCard; 4] = std::array::from_fn(|k| {
            let (name, value) = &words[k];
            QLinkCard::new(name, "CONTINUE", value, q[k].clamp(0, 100) as f32 / 100.0)
        });
        cards[2].bipolar = true;
        cards[3] = QLinkCard::new("Range", "CONTINUE", "drums: none", 0.0);
        Some(cards)
    }

    /// The first step of bar `bar` of the clip the Steps show.
    fn steps_bar_first(&self, bar: usize) -> usize {
        bar * self.steps_per_bar()
    }

    /// The steps in a bar of the meter, at the clip's step size.
    fn steps_per_bar(&self) -> usize {
        let step = self.play.steps.clip.as_ref().map_or(240, |c| c.step.max(1));
        (self.play.transport.meter.bar_ticks().max(1) / step).max(1) as usize
    }

    /// The grid while Continue is open: which bar shows and what it does,
    /// and where the new bars' steps start on the page; the head's button
    /// otherwise.
    pub(super) fn fill_steps_cont(&self, g: &mut StepGrid) {
        let open = self.steps_continuing();
        let add = self.steps_continue_offered() && !open;
        if g.add_continue != add {
            g.add_continue = add;
        }
        let cont = open.then(|| {
            let per = self.steps_per_bar();
            let first = self.play.page * mpfree_seq::steps::PAGE;
            let bar = first / per;
            let c = self.time.continuing.as_deref();
            let made = c.and_then(|c| c.made.as_deref());
            let job = made
                .and_then(|m| m.plan.get(bar))
                .map_or_else(|| "yours".to_string(), |p| p.job.words().to_string());
            let fill = made.is_some_and(|m| m.fills.contains(&bar));
            let new_from = self
                .play
                .steps
                .clip
                .as_ref()
                .and_then(|c| c.new_from)
                .map(|n| n.saturating_sub(first))
                .filter(|&n| n < mpfree_seq::steps::PAGE);
            GridCont {
                bar: format!(
                    "Bar {} · {}{}",
                    bar + 1,
                    if bar == 0 && made.is_some() {
                        "yours".to_string()
                    } else {
                        job
                    },
                    if fill { " · fill" } else { "" }
                ),
                new_from,
            }
        });
        if g.cont != cont {
            g.cont = cont;
        }
    }

    /// The bar strip and the panel from what the time engine says; the
    /// panel closes once Continue is over (kept, gone back, or taken by
    /// another clip).
    pub(super) fn refresh_steps_continue(&mut self) {
        let Some(set) = self.steps_cont else {
            return;
        };
        let c = self.time.continuing.clone();
        let here = self
            .play
            .steps
            .clip
            .as_ref()
            .map(|k| (self.play.steps.track, k.slot));
        match c.as_deref() {
            Some(c) if Some((c.track, c.slot)) == here => self.steps_cont_live = true,
            Some(_) | None if self.steps_cont_live => {
                self.steps_continue_close();
                return;
            }
            _ => {}
        }
        // The panel.
        let (items, current, back, _) = mpfree_time_ui::view::try_list(&self.time);
        let rows = (0..sc::ROWS)
            .map(|r| {
                (
                    mpfree_time_ui::editor::compose::CONTINUE_ROWS[sc::SET_ROWS[r]].to_string(),
                    set.value(sc::SET_ROWS[r], &self.time.styles),
                )
            })
            .collect();
        let bar = self.play.transport.meter.bar_ticks().max(1);
        let status = mpfree_time_ui::view::continue_hint(c.as_deref(), bar);
        let look = PanelLook {
            rows,
            row: set.row.min(sc::ROWS - 1),
            tries: items.into_iter().map(|i| (i.title, i.detail)).collect(),
            shown: current,
            status,
            warn: c.as_deref().is_some_and(|c| c.words.is_some()),
            back: back.map(|b| format!("‹ Try {b}")),
            ready: c
                .as_deref()
                .is_some_and(|c| c.position.is_some() && c.words.is_none()),
        };
        if self
            .ui
            .get::<ContinuePanel>(sc::PANEL)
            .is_some_and(|p| p.look != look)
        {
            if let Some(p) = self.ui.get_mut::<ContinuePanel>(sc::PANEL) {
                p.look = look;
            }
        }
        // The bar strip: each bar's hits by sound, in the grid's colours.
        let strip = self.steps_strip(c.as_deref());
        if self
            .ui
            .get::<BarStrip>(sc::BARS)
            .is_some_and(|b| *b != strip)
        {
            if let Some(b) = self.ui.get_mut::<BarStrip>(sc::BARS) {
                *b = strip;
            }
        }
    }

    /// The bars of the clip the Steps show (the one heard with the result
    /// while it shows), each with its job and its hits.
    fn steps_strip(&self, c: Option<&mpfree_time_ui::continuing::Continuing>) -> BarStrip {
        let Some(clip) = self.play.steps.clip.as_ref() else {
            return BarStrip::default();
        };
        let per = self.steps_per_bar();
        let bars = clip.steps.div_ceil(per).max(1);
        let made = c.and_then(|c| c.made.as_deref());
        let new_from = clip.new_from.map(|n| n / per);
        let colors: Vec<(usize, Color)> = self
            .ui
            .get::<StepGrid>(play_page::STEPS)
            .map(|g| {
                g.lanes
                    .iter()
                    .filter(|l| l.lock.is_none() && !l.empty)
                    .map(|l| (l.pad, l.color))
                    .collect()
            })
            .unwrap_or_default();
        let sounds: Vec<(usize, Color, &Vec<Option<mpfree_seq::steps::Step>>)> = colors
            .iter()
            .filter_map(|&(pad, color)| {
                let row = clip.rows.iter().find(|(p, _)| usize::from(*p) == pad)?;
                row.1
                    .iter()
                    .any(Option::is_some)
                    .then_some((pad, color, &row.1))
            })
            .take(6)
            .collect();
        let shown = self.play.page * mpfree_seq::steps::PAGE / per;
        BarStrip {
            bars: (0..bars)
                .map(|b| {
                    let new = new_from.is_some_and(|n| b >= n);
                    let job = if !new {
                        "yours".to_string()
                    } else {
                        made.and_then(|m| m.plan.get(b))
                            .map_or_else(String::new, |p| p.job.words().to_string())
                    };
                    let mut hits = Vec::new();
                    for (row, (_, color, steps)) in sounds.iter().enumerate() {
                        for s in b * per..((b + 1) * per).min(steps.len()) {
                            if let Some(st) = &steps[s] {
                                hits.push((row, (s - b * per) * 16 / per, st.velocity, *color));
                            }
                        }
                    }
                    StripBar {
                        job,
                        fill: made.is_some_and(|m| m.fills.contains(&b)),
                        new,
                        hits,
                        rows: sounds.len(),
                    }
                })
                .collect(),
            shown,
        }
    }
}
