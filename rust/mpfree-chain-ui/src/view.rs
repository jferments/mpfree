//! The chain editor's widget, in the effect page frame (layout spec
//! section 4b; Jay, 14 Nov 2026: "Build it as drawn"):
//!
//! - **The chain** (220 px): ‹ back and the strip button (or the Sample
//!   tab's scopes) over the strip's effects in order, each with its
//!   number, name, a line about its settings, a dot per macro that moves
//!   it, and its ON pill (bypass with spillover). A tap selects an effect
//!   and a drag moves it along the chain; under the list, Add effect opens
//!   the effect picker, and Up, Down and Remove act on the selected one.
//! - **The effect** (the rest: 884 px on the MPC): a header with the
//!   effect's name and its Mode chooser (a button per mode, or ‹ › round a
//!   chip that opens the list of them), or its page tabs when it has more
//!   than one page; then the page, its controls in labelled sections in
//!   their job's colour with the effect page's drawings, drawn by the Synth
//!   tab's section widget (`mpfree_synth_ui::section_view`) from the
//!   effect's plan ([`crate::sections`]), each knob with a ring for how far
//!   modulation moves it and a badge in its macro's colour; and under the
//!   page the Modulate row, for the control last touched: its macro (tap a
//!   number; tap it again to take it off), Learn (the next Q-Link turned
//!   takes it), + LFO and + Follower, and a link that opens its mod matrix
//!   rows over the page (drag a row's bar to set its depth, x to remove
//!   it). While the picker is open it takes the effect column.
//!
//! The widget keeps a copy of what it shows and moves it under the finger
//! at once; what the touches mean goes into a queue of [`ChainAction`]s
//! for the app (see [`crate::take_chain_actions`]).

use std::f32::consts::PI;

use mpfree_fx::mpfree_node::Curve;
use mpfree_mod::{Source, MACROS};
use mpfree_param::{Owner, ParamAddr, Strip};
use mpfree_synth_ui::draw::Mask;
use mpfree_synth_ui::section_view::{Control, Kind, SectionBox, SectionView};
use mpfree_synth_ui::sections::Job;
use mpfree_synth_ui::widgets::Card;
use mpfree_ui::{Align, EventKind, FontId, Gesture, PaintCx, Rect, Target, Touch, Widget};

use crate::eq_page::EqPage;
use crate::notes::{self, NoteAction};
use crate::picker::{Hit, PickerLayout, PickerView, HOLD};
use crate::state::{macro_color, ChainState, ParamView, StripList, XyMark, STRIP_GROUPS};

/// The section widget's `Tab` number for a heading's action (pages are
/// numbered from 0, far below it).
const ACTION_TAB: usize = 1 << 16;
/// Its `Tab` for the Vocoder's "Voice: Mic ›" in its Carrier heading: the
/// sidechain list opens.
const SIDECHAIN_TAB: usize = ACTION_TAB + 1;

/// What a touch on the chain editor meant, for the app to apply.
#[derive(Clone, Debug, PartialEq)]
pub enum ChainAction {
    /// An insert was tapped: show its page.
    Select(usize),
    /// Bypass an insert (`on` false) or switch it back on.
    SetOn {
        slot: usize,
        on: bool,
    },
    /// Put what insert `slot` holds on a pad: the Grains effect's memory
    /// (the "Capture ›" in its Hold heading; `Node::copy_held`), where a
    /// recording would land, playing as grains with its settings.
    Capture(usize),
    /// Move an insert (`Mixer` `Edit::Move`).
    Move {
        from: usize,
        to: usize,
    },
    /// Take an insert out (`Edit::Remove`).
    Remove(usize),
    /// Add effect `id` (one of [`ChainState::picks`]) at the end of the
    /// chain: `mpfree_fx_ui::make(id)`, then `Edit::Insert`.
    Add(&'static str),
    /// Open insert `slot`'s full effect page (`mpfree_fx_ui::FxPanel`): a
    /// tap on the effect's name or its drawings.
    OpenPage(usize),
    /// Show the selected insert's parameter group `g`.
    Group(usize),
    /// Switch insert `slot` to mode `mode` of its family (an index into
    /// [`crate::state::ModeView::modes`]): within one engine its fixed
    /// values change, across engines the app swaps the engine in place
    /// (`mpfree_fx::families`).
    SetMode {
        slot: usize,
        mode: usize,
    },
    /// Load one of insert `slot`'s presets from the header's list (an
    /// entry of [`ChainState::presets`]): every value it sets, as `Set`
    /// would, on engine `id` (refused if the slot holds another by then:
    /// a mode switch across engines on its way, a chain edit).
    Preset {
        slot: usize,
        id: &'static str,
        values: Vec<(u16, f32)>,
    },
    /// A knob was touched: the modulation column works on it.
    Focus(ParamAddr),
    /// Set a parameter: send it to the mod engine as `ModEvent::hand`
    /// gives (a `Set` unless it has a move lane or its track is
    /// recording, then a `Touch`).
    Set {
        addr: ParamAddr,
        value: f32,
    },
    /// The finger left a knob: send `ModEvent::Untouch` for it (nothing
    /// happens if the turn was a `Set`).
    Release(ParamAddr),
    /// Put a parameter on macro `m`, or take it off (`None`).
    Macro {
        target: ParamAddr,
        macro_: Option<usize>,
    },
    /// Arm learn: the next Q-Link turned takes this parameter.
    Learn(ParamAddr),
    /// XY → or XY ↑ on the Modulate row: the control goes on that
    /// direction of the layout the XY pad plays (a ready-made one becomes
    /// a layout of yours first), over its whole range, Even, Set; on the
    /// direction it is on already, it comes off (layout spec 4b's XY
    /// chips). The app says what happened, or why not.
    Xy {
        addr: ParamAddr,
        up: bool,
    },
    /// Add a matrix row from `source` to the parameter (depth 25 %).
    AddRow {
        target: ParamAddr,
        source: Source,
    },
    /// Set row `row`'s depth (-1 to 1).
    Depth {
        row: usize,
        depth: f32,
    },
    RemoveRow(usize),
    /// Back to the screen the editor took the place of (`ChainState::back`).
    Back,
    /// Show chain `i` of `ChainState::scopes` (the pad's or the track's).
    Scope(usize),
    /// The strip button opened the strip list on group `g` (of
    /// [`crate::STRIP_GROUPS`]), a group was picked, or the list closed
    /// (None). The app keeps the list's state, so it must hear each.
    Strips(Option<usize>),
    /// A strip in the list: its chain, in the editor's place (its Back
    /// stays where it was).
    OpenStrip(Strip),
    /// The sidechain list (layout spec 4b, "Sidechain"): the selected
    /// effect's Sidechain row opened it on group `g` (of
    /// [`crate::STRIP_GROUPS`]), a group was picked, or it shut (None).
    /// The app keeps it, as the strip list, and fills its [`StripList`]
    /// with the `sidechain` words.
    SidechainList(Option<usize>),
    /// What the selected effect listens to. `control` is its sidechain
    /// control (`mpfree_fx::sidechain::listens`), on the insert it names.
    /// A strip: the mixer feeds the effect that strip (`Mixer::set_key`)
    /// and the control turns on. None: the control turns off (its own
    /// sound, or Tempo) and the strip stays fed, so a tap on its button
    /// goes back to it.
    SetSidechain {
        control: ParamAddr,
        strip: Option<Strip>,
    },
    /// "+ Add effect" opened the effect picker (`true`), or its Cancel
    /// closed it (`false`). The app keeps the picker's state, so it must
    /// hear both, or the next state it sends undoes the tap.
    Picker(bool),
    /// Add effect `id` at the end of the chain with its factory preset
    /// `preset` (`mpfree_fx::picker::presets(id)[preset]`) applied.
    AddPreset {
        id: &'static str,
        preset: usize,
    },
    /// The picker's star: make `id` a favourite (`on`) or not. The app
    /// keeps them ([`ChainState::shelf`], `files/fx/favourites.txt`).
    Favourite {
        id: &'static str,
        on: bool,
    },
    /// The Notes stage above a note track's sound: its rows, its picker,
    /// its Plays on row and its pages ([`crate::notes`]).
    Notes(NoteAction),
}

/// The chain column's width (the effect column takes the rest: 884 px on
/// the MPC, beside the app's 128 px Q-Link column).
pub const LIST_W: i32 = 220;
/// Space between the columns, and between the chain column's cards.
const GAP: i32 = 12;
/// The chain column's head card (‹ and the strip button), its list's
/// caption ("EFFECTS · 3 of 8"), rows, Add effect and buttons.
const HEAD_H: i32 = 64;
const CAPTION_H: i32 = 28;
const ROW_H: i32 = 54;
const ROW_GAP: i32 = 4;
const ADD_H: i32 = 48;
const BUTTON_H: i32 = 44;
/// The ON pill's end of an effect's row: a tap there bypasses it.
const POWER_W: i32 = 52;
/// The effect's header, the Modulate row under the page, and the space
/// between them and the page.
const PAGE_HEAD_H: i32 = 52;
const MOD_H: i32 = 54;
const PAGE_GAP: i32 = 10;
/// The header's buttons (modes, page tabs), and their arrows when they
/// don't all fit.
const CHIP_H: i32 = 36;
const ARROW_W: i32 = 44;
/// The Mode chooser's chip between its arrows (Drive's ten), and the rows
/// of the list it opens.
const MODE_CHIP_W: i32 = 180;
const MODE_ROW_H: i32 = 40;
/// The presets button's padding before its label.
const PRESET_PAD: i32 = 12;
/// The Modulate row's buttons.
const MACRO_W: i32 = 34;
/// The Modulate row's XY → and XY ↑ (between the name and the macros),
/// and the gap between them.
const XY_CHIP_W: i32 = 60;
const XY_CHIP_GAP: i32 = 8;
const LEARN_W: i32 = 108;
const LFO_W: i32 = 76;
const FOLLOWER_W: i32 = 100;
/// The mod matrix rows over the page: the panel's width, a row's height,
/// and its source's name left of its depth bar.
const ROWS_W: i32 = 440;
const DEPTH_ROW_H: i32 = 44;
const SOURCE_W: i32 = 98;

/// What the editor has open over its page.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Open {
    /// The touched control's mod matrix rows (the Modulate row's link).
    pub rows: bool,
    /// The list of modes (the Mode chooser's chip, when the modes don't
    /// fit the header as buttons).
    pub modes: bool,
    /// The effect's presets (its name in the header).
    pub presets: bool,
    /// The first row the Notes group and the Sound group show, when their
    /// rows don't all fit (a note track's list, [`crate::notes`]).
    pub notes_from: usize,
    pub sound_from: usize,
}

/// Where everything goes, from the widget's rectangle.
pub struct Layout {
    /// The chain column's head card: ‹, the strip button or the scopes (or
    /// the chain's title).
    pub head: Rect,
    pub back: Option<Rect>,
    /// The strip button (the strip's name and which it is): a tap opens
    /// the strip list. None with scopes, or for a chain no strip names.
    pub strip: Option<Rect>,
    pub scopes: Vec<Rect>,
    /// The list's card, its caption, a row per effect, Add effect (when
    /// there is room) and Up, Down, Remove.
    pub list: Rect,
    pub caption: Rect,
    /// A row per effect of the sound's chain; a row its group can't show
    /// (a note track's list, scrolled) is an empty rect.
    pub rows: Vec<Rect>,
    pub add: Option<Rect>,
    pub buttons: [Rect; 3],
    /// A note track's Notes group above the sound's (then the caption is
    /// the Notes group's), and the Sound group's arrows and how many of
    /// its rows show at once; and the Notes page while a row of the Notes
    /// group (or its picker) takes the effect column.
    pub notes: Option<notes::ListLayout>,
    pub sound_arrows: Option<(Rect, Rect)>,
    pub sound_fit: usize,
    pub notes_page: Option<notes::PageLayout>,
    /// The effect column: the header, the page and the Modulate row (the
    /// picker takes all of it).
    pub effect: Rect,
    /// The header's card and the effect's name in it.
    pub header: Rect,
    pub title: Rect,
    /// The button that opens the effect's presets: its preset's name (or
    /// the effect's) with a chevron, at the header's left, when it has
    /// presets; and the list, while it is open: its panel and a row per
    /// entry of [`ChainState::presets`].
    pub preset_chip: Option<Rect>,
    pub preset_list: Option<Rect>,
    pub preset_rows: Vec<(Rect, usize)>,
    /// The Mode chooser: its group, each place to tap and the mode it
    /// picks (a button per mode, or the two arrows when they don't fit),
    /// and the chip between the arrows (a tap opens the list of modes).
    pub mode_group: Option<Rect>,
    pub modes: Vec<(Rect, usize)>,
    pub mode_label: Option<Rect>,
    /// The list of modes, while it is open: its panel and a row per mode.
    pub mode_list: Option<Rect>,
    pub mode_rows: Vec<(Rect, usize)>,
    /// The page tabs (two or more pages): their group, a tab per page in
    /// the order of `ChainState::groups` (a tab out of view while the row
    /// scrolls is empty: nothing to draw or tap), and the arrows to the
    /// page before and after when they don't all fit.
    pub tab_group: Option<Rect>,
    pub tabs: Vec<Rect>,
    pub tab_arrows: Option<(Rect, Rect)>,
    /// The sections.
    pub page: Rect,
    /// The picker's Cancel, its parts ([`crate::picker`]), and the effect
    /// tiles on show (the current section's page; none when the picker is
    /// shut, shows presets, or the chain is full).
    pub cancel: Option<Rect>,
    pub picker: Option<PickerLayout>,
    pub picks: Vec<Rect>,
    /// The Modulate row: the control's name over the link to its rows,
    /// macros 1 to 8, Learn, + LFO, + Follower.
    pub modulate: Rect,
    pub mod_name: Rect,
    pub rows_link: Rect,
    /// XY → and XY ↑.
    pub xy_chips: [Rect; 2],
    pub macros: [Rect; MACROS],
    pub learn: Rect,
    pub add_lfo: Rect,
    pub add_follower: Rect,
    /// The mod matrix rows over the page, while open: the panel, and each
    /// row's depth bar (its source's name at its left) and its x.
    pub rows_panel: Option<Rect>,
    pub depth_rows: Vec<(Rect, Rect)>,
}

/// Where everything goes. `pv`: the picker's own state; `open`: what is
/// open over the page.
pub fn layout(r: Rect, st: &ChainState, pv: &PickerView, open: Open) -> Layout {
    let (chain, rest) = r.take_left(LIST_W);
    let effect = rest.take_left(GAP).1;
    // ---- The chain column: the head card, then the list's card.
    let (head, list) = chain.take_top(HEAD_H);
    let list = list.take_top(GAP).1;
    let hin = head.inset_xy(10, (HEAD_H - 44) / 2);
    let back = st
        .back
        .is_some()
        .then(|| Rect::new(hin.x, hin.y, 44, hin.h));
    let sx = back.map_or(hin.x, |b| b.right() + 8);
    let strip = (st.scopes.is_empty() && !st.strip_name.is_empty())
        .then(|| Rect::new(sx, hin.y, hin.right() - sx, hin.h));
    let n = st.scopes.len() as i32;
    let scopes = (0..n)
        .map(|i| {
            let w = (hin.right() - sx) / n.max(1);
            Rect::new(sx + i * w, hin.y, w - if i + 1 < n { 4 } else { 0 }, hin.h)
        })
        .collect();
    let inner = list.inset_xy(10, 12);
    let (caption, body) = inner.take_top(CAPTION_H);
    let (foot, body) = body.take_bottom(BUTTON_H);
    let mut y = body.y;
    let mut rows = Vec::new();
    let (mut notes_l, mut sound_arrows, mut sound_fit) = (None, None, st.slots.len());
    let add = match &st.notes {
        // A note track: the Notes group, Plays on, then the sound's rows,
        // compact, scrolled when they don't all fit with their Add.
        Some(ns) => {
            let nl = notes::list_layout(caption, body, ns, open.notes_from);
            let area = nl.sound;
            let n = st.slots.len();
            let per = notes::ROW_H + notes::ROW_GAP;
            let want_add = n < st.room;
            let add_h = if want_add {
                notes::ADD_H + notes::ROW_GAP
            } else {
                0
            };
            let fit = (((area.h - add_h + notes::ROW_GAP) / per).max(1) as usize).min(n.max(1));
            let from = open.sound_from.min(n.saturating_sub(fit));
            let mut y = area.y;
            for i in 0..n {
                if (from..from + fit).contains(&i) {
                    rows.push(Rect::new(area.x, y, area.w, notes::ROW_H));
                    y += per;
                } else {
                    rows.push(Rect::default());
                }
            }
            if n > fit {
                sound_arrows = Some(notes::caption_arrows(nl.sound_caption));
            }
            sound_fit = fit;
            let add = (want_add && y + notes::ADD_H <= area.bottom())
                .then(|| Rect::new(area.x, y, area.w, notes::ADD_H));
            notes_l = Some(nl);
            add
        }
        None => {
            for _ in &st.slots {
                rows.push(Rect::new(body.x, y, body.w, ROW_H));
                y += ROW_H + ROW_GAP;
            }
            (st.slots.len() < st.room && y + 2 + ADD_H <= body.bottom())
                .then(|| Rect::new(body.x, y + 2, body.w, ADD_H))
        }
    };
    let bw = (foot.w - 2 * 6) / 3;
    let buttons = [0, 1, 2].map(|i| Rect::new(foot.x + i * (bw + 6), foot.y, bw, BUTTON_H));
    // ---- The effect column: the header, the page, the Modulate row.
    let (header, rest) = effect.take_top(PAGE_HEAD_H);
    let (modulate, rest) = rest.take_bottom(MOD_H);
    let page = rest.inset_xy(0, PAGE_GAP);
    // The MIDI effect picker takes the column under the header, as the
    // effect picker takes all of it.
    let notes_page = st
        .notes
        .as_ref()
        .filter(|ns| !st.picking && (ns.picking || ns.page_shown()))
        .map(|ns| notes::page_layout(notes_area(page, modulate, ns), ns));
    let shown = st.selected.is_some() && !st.picking;
    let row = Rect::new(
        header.x + 8,
        header.y + (header.h - CHIP_H - 8) / 2,
        header.w - 16,
        CHIP_H + 8,
    );
    // The Mode chooser at the header's right, the tabs left of it, the
    // name in what is left.
    let mut right = row.right();
    let (mut mode_group, mut modes, mut mode_label) = (None, Vec::new(), None);
    if let (Some(mv), true) = (&st.mode, shown) {
        let (g, m, label) = mode_row(row, mv);
        right = g.x - 12;
        (mode_group, modes, mode_label) = (Some(g), m, label);
    }
    let (mut tab_group, mut tabs, mut tab_arrows) = (None, Vec::new(), None);
    if shown && st.groups.len() > 1 {
        let area = Rect::new(row.x + 200, row.y, (right - row.x - 200).max(0), row.h);
        let (g, t, arrows) = tab_row(area, st);
        right = g.x - 12;
        (tab_group, tabs, tab_arrows) = (Some(g), t, arrows);
    }
    let title = Rect::new(row.x + 8, header.y, (right - row.x - 8).max(0), header.h);
    let (mut preset_chip, mut preset_list, mut preset_rows) = (None, None, Vec::new());
    if shown && !st.presets.is_empty() {
        let label = header_label(st);
        // Room for the label at the title's size without measuring it: 13
        // px a letter covers capitals ("EQ", "Amp"); the chevron sits
        // after the measured text, so a wider chip only adds touch room.
        let w = (PRESET_PAD + 13 * label.chars().count() as i32 + 26).min(title.w + 8);
        let chip = Rect::new(row.x, row.y, w, row.h);
        preset_chip = Some(chip);
        if open.presets {
            let n = st.presets.len() as i32;
            let fit = ((page.h - 16 + 4) / (MODE_ROW_H + 4)).max(1);
            let h = 16 + n.min(fit) * (MODE_ROW_H + 4) - 4;
            let panel = Rect::new(chip.x, page.y, chip.w.max(280), h);
            for k in 0..n.min(fit) as usize {
                let y = panel.y + 8 + k as i32 * (MODE_ROW_H + 4);
                preset_rows.push((Rect::new(panel.x + 8, y, panel.w - 16, MODE_ROW_H), k));
            }
            preset_list = Some(panel);
        }
    }
    let (mut mode_list, mut mode_rows) = (None, Vec::new());
    if let (true, Some(mv), Some(label)) = (open.modes, &st.mode, mode_label) {
        let n = mv.modes.len() as i32;
        let w = (label.w + 2 * ARROW_W + 16).max(240);
        let fit = ((page.h - 16 + 4) / (MODE_ROW_H + 4)).max(1);
        let h = 16 + n.min(fit) * (MODE_ROW_H + 4) - 4;
        let panel = Rect::new(label.right() + ARROW_W + 4 - w, page.y, w, h);
        for (k, _) in mv.modes.iter().enumerate().take(fit as usize) {
            let y = panel.y + 8 + k as i32 * (MODE_ROW_H + 4);
            mode_rows.push((Rect::new(panel.x + 8, y, panel.w - 16, MODE_ROW_H), k));
        }
        mode_list = Some(panel);
    }
    let (mut cancel, mut picker_l, mut pick_rects) = (None, None, Vec::new());
    if st.picking {
        // The picker takes the effect column.
        let (phead, body) = effect.inset(14).take_top(52);
        cancel = Some(Rect::new(phead.right() - 104, phead.y + 4, 104, 36));
        if st.slots.len() < st.room {
            let pl = crate::picker::layout(body.inset_xy(0, 4), st, pv);
            if pv.presets.is_none() {
                pick_rects = pl.tiles.clone();
            }
            picker_l = Some(pl);
        }
    }
    // ---- The Modulate row: buttons from the right, macros left of them,
    // the control's name in what is left.
    let line = modulate.inset_xy(10, (MOD_H - 44) / 2);
    let add_follower = Rect::new(line.right() - FOLLOWER_W, line.y, FOLLOWER_W, line.h);
    let add_lfo = Rect::new(add_follower.x - 6 - LFO_W, line.y, LFO_W, line.h);
    let learn = Rect::new(add_lfo.x - 6 - LEARN_W, line.y, LEARN_W, line.h);
    let m0 = learn.x - 12 - (MACROS as i32 * (MACRO_W + 3) - 3);
    let macros =
        std::array::from_fn(|m| Rect::new(m0 + m as i32 * (MACRO_W + 3), line.y, MACRO_W, line.h));
    let x0 = m0 - 12 - 2 * XY_CHIP_W - XY_CHIP_GAP;
    let xy_chips = std::array::from_fn(|k| {
        Rect::new(
            x0 + k as i32 * (XY_CHIP_W + XY_CHIP_GAP),
            line.y,
            XY_CHIP_W,
            line.h,
        )
    });
    let mod_name = Rect::new(line.x + 4, line.y, (x0 - 12 - line.x - 4).max(0), line.h);
    let rows_link = Rect::new(mod_name.x, mod_name.y + 22, mod_name.w, mod_name.h - 22);
    let (mut rows_panel, mut depth_rows) = (None, Vec::new());
    // A MIDI effect's page has the rows of its setting touched too.
    let notes_shown = notes_page.is_some() && st.notes.as_ref().is_some_and(|n| !n.picking);
    if open.rows && (shown || notes_shown) && st.focus.is_some() {
        let n = st.rows.len().max(1) as i32;
        let h = (44 + n * (DEPTH_ROW_H + 6) + 8).min(page.h);
        let panel = Rect::new(page.x, page.bottom() - h, ROWS_W.min(page.w), h);
        for k in 0..st.rows.len() as i32 {
            let row = Rect::new(
                panel.x + 14,
                panel.y + 44 + k * (DEPTH_ROW_H + 6),
                panel.w - 28,
                DEPTH_ROW_H,
            );
            if row.bottom() > panel.bottom() {
                break;
            }
            let (x, bar) = row.take_right(40);
            depth_rows.push((bar, x));
        }
        rows_panel = Some(panel);
    }
    Layout {
        head,
        back,
        strip,
        scopes,
        list,
        caption,
        rows,
        add,
        buttons,
        notes: notes_l,
        sound_arrows,
        sound_fit,
        notes_page,
        effect,
        header,
        title,
        preset_chip,
        preset_list,
        preset_rows,
        mode_group,
        modes,
        mode_label,
        mode_list,
        mode_rows,
        tab_group,
        tabs,
        tab_arrows,
        page,
        cancel,
        picker: picker_l,
        picks: pick_rects,
        modulate,
        mod_name,
        rows_link,
        xy_chips,
        macros,
        learn,
        add_lfo,
        add_follower,
        rows_panel,
        depth_rows,
    }
}

/// What the header's name says: the preset the effect's settings are
/// (not "Defaults"), else the effect's name (a family's: the mode is on
/// the Mode chooser).
fn header_label(st: &ChainState) -> &str {
    match st.preset.filter(|&k| k > 0).and_then(|k| st.presets.get(k)) {
        Some(p) => p.name,
        None => effect_name(st),
    }
}

/// The selected effect's name: a family's ("Reverb"), else the insert's.
fn effect_name(st: &ChainState) -> &str {
    match (&st.mode, st.selected.and_then(|i| st.slots.get(i))) {
        (Some(m), _) => m.family,
        (None, Some(s)) => &s.name,
        (None, None) => "",
    }
}

/// A header button's width for `name`.
fn chip_w(name: &str) -> i32 {
    28 + 9 * name.chars().count() as i32
}

/// The page tabs at the right of `area`: their group (4 px round them),
/// a tab each left to right, and when they don't all fit, an arrow each
/// end that steps to the page before and after, the tabs shown being a
/// run that holds the page shown (the rest are empty rects).
fn tab_row(area: Rect, st: &ChainState) -> (Rect, Vec<Rect>, Option<(Rect, Rect)>) {
    let widths: Vec<i32> = st.groups.iter().map(|g| chip_w(g)).collect();
    let need = |a: usize, b: usize| -> i32 {
        widths[a..b].iter().sum::<i32>() + 4 * (b - a).saturating_sub(1) as i32
    };
    let lay = |x0: i32, end: i32, first: usize| -> Vec<Rect> {
        let mut x = x0;
        (0..widths.len())
            .map(|i| {
                if i < first {
                    return Rect::new(0, 0, 0, 0);
                }
                if x + widths[i] > end {
                    // This one and every one after it are out of view.
                    x = end + 1;
                    return Rect::new(0, 0, 0, 0);
                }
                let r = Rect::new(x, area.y + 4, widths[i], CHIP_H);
                x += widths[i] + 4;
                r
            })
            .collect()
    };
    let all = need(0, widths.len());
    if all + 8 <= area.w {
        let g = Rect::new(area.right() - all - 8, area.y, all + 8, area.h);
        return (g, lay(g.x + 4, g.right() - 4, 0), None);
    }
    let g = area;
    let prev = Rect::new(g.x + 4, g.y + 4, ARROW_W, CHIP_H);
    let next = Rect::new(g.right() - 4 - ARROW_W, g.y + 4, ARROW_W, CHIP_H);
    let (x0, end) = (prev.right() + 4, next.x - 4);
    // The first tab shown: as far left as keeps the page shown in view.
    let sel = st.group.min(widths.len() - 1);
    let mut first = 0;
    while first < sel && need(first, sel + 1) > end - x0 {
        first += 1;
    }
    (g, lay(x0, end, first), Some((prev, next)))
}

/// The Mode chooser at the right of `row`: its group, a button per mode
/// when they fit beside the name, else an arrow each side of a chip with
/// the mode's name (a tap opens the list), the arrows stepping to the mode
/// before and after (none past the ends).
fn mode_row(row: Rect, mv: &crate::state::ModeView) -> (Rect, Vec<(Rect, usize)>, Option<Rect>) {
    let widths: Vec<i32> = mv.modes.iter().map(|m| chip_w(m)).collect();
    let need: i32 = widths.iter().sum::<i32>() + 4 * (widths.len() as i32 - 1).max(0);
    if need + 8 <= row.w - 220 {
        let g = Rect::new(row.right() - need - 8, row.y, need + 8, row.h);
        let mut x = g.x + 4;
        let chips = widths
            .iter()
            .enumerate()
            .map(|(i, &w)| {
                let r = Rect::new(x, g.y + 4, w, CHIP_H);
                x += w + 4;
                (r, i)
            })
            .collect();
        return (g, chips, None);
    }
    let w = 2 * ARROW_W + MODE_CHIP_W + 16;
    let g = Rect::new(row.right() - w, row.y, w, row.h);
    let prev = Rect::new(g.x + 4, g.y + 4, ARROW_W, CHIP_H);
    let label = Rect::new(prev.right() + 4, g.y + 4, MODE_CHIP_W, CHIP_H);
    let next = Rect::new(label.right() + 4, g.y + 4, ARROW_W, CHIP_H);
    let mut out = Vec::new();
    match mv.current {
        Some(c) => {
            if c > 0 {
                out.push((prev, c - 1));
            }
            if c + 1 < mv.modes.len() {
                out.push((next, c + 1));
            }
        }
        // In no mode: either arrow goes to the first.
        None => out.extend([(prev, 0), (next, 0)]),
    }
    (g, out, Some(label))
}

/// A macro button's touch area: its whole pitch, the gaps between the
/// buttons included (layout spec section 1, packed buttons).
fn macro_hit(r: Rect) -> Rect {
    Rect::new(r.x - 1, r.y, r.w + 3, r.h)
}

/// The slot a dragged effect's row lands in with the finger at height
/// `y`: the row under it, or the first or last past the ends.
fn slot_at(l: &Layout, y: i32) -> usize {
    let (Some(first), Some(last)) = (shown_row(l, false), shown_row(l, true)) else {
        return 0;
    };
    let r = l.rows[first];
    (first + ((y - r.y) / (r.h + ROW_GAP)).max(0) as usize).min(last)
}

/// The Notes group's selected row, how many rows it has and whether the
/// row is an effect (not the arp), while its page shows.
fn notes_selected(st: &ChainState) -> Option<(usize, usize, bool)> {
    let ns = st.notes.as_ref().filter(|ns| !ns.picking && !st.picking)?;
    let row = ns.selected?;
    let items = ns.items();
    let item = items.get(row)?;
    Some((row, items.len(), matches!(item, mpfree_notefx::Item::Fx(_))))
}

/// Where the Notes page goes: the page, or for its picker the page and
/// the Modulate row's place.
fn notes_area(page: Rect, modulate: Rect, ns: &notes::NotesState) -> Rect {
    if ns.picking {
        Rect::new(page.x, page.y, page.w, modulate.bottom() - page.y)
    } else {
        page
    }
}

/// The first (or `last`) row the list shows.
fn shown_row(l: &Layout, last: bool) -> Option<usize> {
    let mut shown = (0..l.rows.len()).filter(|&i| !l.rows[i].is_empty());
    if last {
        shown.next_back()
    } else {
        shown.next()
    }
}

enum Drag {
    Depth {
        k: usize,
    },
    /// An effect's row held: once the finger has moved [`SLOT_SLOP`] it
    /// lifts and follows (`y` now), and lands at slot `to` on the lift.
    Slot {
        from: usize,
        y0: i32,
        y: i32,
        to: usize,
        moved: bool,
    },
    /// A finger on the page: the section widget has it. `turned` is the
    /// control it set while down (Release goes to the app when it lifts).
    Page {
        turned: Option<ParamAddr>,
    },
    /// A finger on the EQ's page: it has it ([`crate::eq_page`]).
    Eq,
}

/// Pixels a finger moves on an effect's row before the row lifts to be
/// dragged (less is a tap).
const SLOT_SLOP: i32 = 12;

/// The strip list over the editor: "‹ Back" at the top left, where the
/// editor's own ‹ is (Jay's device round, 20 Nov: the list's Back was at
/// the top right, where nothing else keeps it), the groups down the left
/// under it, a page of strip tiles (four a row), and arrows when a group
/// has more strips than a page. As the sidechain list it has Off (or
/// Tempo) above the groups, only the groups that can feed the effect (a
/// group it leaves out is an empty rect), the line under them that says
/// why, and what the effect hears now at the head's right.
pub struct ListLayout {
    pub panel: Rect,
    /// The column's label ("LISTEN TO" or "STRIPS"), under Back.
    pub label: Rect,
    pub groups: [Rect; 6],
    pub title: Rect,
    pub back: Rect,
    pub prev: Rect,
    pub next: Rect,
    pub tiles: Vec<Rect>,
    pub off: Option<Rect>,
    pub note: Rect,
    pub now: Option<Rect>,
}

/// What the sidechain list shows that the strip list doesn't: the groups
/// it shows, and whether it has an Off button (Tempo on the Ducker; the
/// Vocoder's carriers are its own buttons).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScParts {
    pub shown: [bool; 6],
    pub off: bool,
}

/// Tiles a row in the strip list.
const LIST_COLS: i32 = 4;
const TILE_H: i32 = 72;
/// The sidechain list's Off button, and what it hears now beside Back.
const OFF_H: i32 = 60;
const NOW_W: i32 = 200;
/// The list's "‹ Back": as tall as the editor's ‹ and in its place.
const LIST_BACK_W: i32 = 112;

/// Where the strip list goes in `r`.
pub fn list_layout(r: Rect) -> ListLayout {
    list_layout_for(r, None)
}

/// Where the strip list goes in `r`, or the sidechain list with its parts
/// `sc`.
pub fn list_layout_for(r: Rect, sc: Option<ScParts>) -> ListLayout {
    let panel = r.inset_xy(0, 4);
    let inner = panel.inset(14);
    let (left, right) = inner.take_left(200);
    let right = right.take_left(14).1;
    // Back where the editor's ‹ is: the chain head card's inset.
    let back = Rect::new(r.x + 10, r.y + (HEAD_H - 44) / 2, LIST_BACK_W, 44);
    let label = Rect::new(left.x, back.bottom() + 8, left.w, 28);
    let col = label.bottom() + 4;
    let off = sc
        .filter(|p| p.off)
        .map(|_| Rect::new(left.x, col, left.w, OFF_H));
    // Below Off and a rule under it, the groups the list shows, packed.
    let top = off.map_or(col, |o| o.bottom() + 17);
    let mut at = 0;
    let groups = std::array::from_fn(|i| {
        if sc.is_some_and(|p| !p.shown[i]) {
            return Rect::default();
        }
        let g = Rect::new(left.x, top + at * 56, left.w, 48);
        at += 1;
        g
    });
    let note_y = top + at * 56 + 4;
    let note = Rect::new(left.x, note_y, left.w, (left.bottom() - note_y).max(0));
    let (head, grid) = right.take_top(36);
    let now = sc.map(|_| Rect::new(head.right() - NOW_W, head.y, NOW_W, 36));
    let arrows_x = now.map_or(head.right() + 12, |n| n.x);
    let next = Rect::new(arrows_x - 12 - 44, head.y, 44, 36);
    let prev = Rect::new(next.x - 6 - 44, head.y, 44, 36);
    let title = Rect::new(head.x, head.y, prev.x - 8 - head.x, 36);
    let grid = grid.take_top(10).1;
    let rows = ((grid.h - 30 + 10) / (TILE_H + 10)).max(1);
    let w = (grid.w - (LIST_COLS - 1) * 10) / LIST_COLS;
    let tiles = (0..rows * LIST_COLS)
        .map(|i| {
            Rect::new(
                grid.x + (i % LIST_COLS) * (w + 10),
                grid.y + (i / LIST_COLS) * (TILE_H + 10),
                w,
                TILE_H,
            )
        })
        .collect();
    ListLayout {
        panel,
        label,
        groups,
        title,
        back,
        prev,
        next,
        tiles,
        off,
        note: if sc.is_some() { note } else { Rect::default() },
        now,
    }
}

/// A strip's colour, as the mixer's strips have it.
fn strip_color(t: &mpfree_ui::Theme, s: Strip) -> mpfree_ui::Color {
    let palette = [
        t.accent,
        t.good,
        t.warn,
        t.bad,
        t.accent.mix(t.bad, 0.5),
        t.good.mix(t.accent, 0.5),
        t.warn.mix(t.bad, 0.4),
        t.accent.mix(t.text, 0.5),
    ];
    match s {
        Strip::Track(i) | Strip::Pad(i) => palette[i as usize % palette.len()],
        Strip::Input(_) => t.bad.mix(t.warn, 0.5),
        Strip::Bus(_) => t.text_dim,
        Strip::Return(_) => t.accent.mix(t.text_dim, 0.6),
        Strip::Master => t.text,
    }
}

/// See the module docs.
pub struct ChainView {
    state: ChainState,
    /// The selected effect's drawings ([`crate::state::picture`]): each
    /// section shows the ones its plan names, by title.
    cards: Vec<Card>,
    /// The page: the section widget, filled from the state and the
    /// drawings ([`ChainView::fill_page`]).
    page: SectionView,
    actions: Vec<ChainAction>,
    drag: Option<(u64, Drag)>,
    /// The picker's section, page, wheel highlight and open presets.
    picker: PickerView,
    /// The strip list's group while it is open (the view keeps it open;
    /// the app fills it), and the first tile shown.
    list_open: Option<usize>,
    list_first: usize,
    /// The list open is the sidechain list (the selected effect's
    /// Sidechain row opened it), not the strip list.
    list_sidechain: bool,
    /// What is open over the page.
    open: Open,
    /// The EQ's page, while the EQ is selected ([`ChainState::eq`]).
    eq: EqPage,
    /// The rectangle it was last drawn or touched in: a row selected out
    /// of a scrolled group's view scrolls into it.
    last: std::cell::Cell<Rect>,
}

impl Default for ChainView {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainView {
    pub fn new() -> ChainView {
        ChainView {
            state: ChainState::default(),
            cards: Vec::new(),
            page: SectionView::new(),
            actions: Vec::new(),
            drag: None,
            picker: PickerView::default(),
            list_open: None,
            list_first: 0,
            list_sidechain: false,
            open: Open::default(),
            eq: EqPage::default(),
            last: std::cell::Cell::new(Rect::default()),
        }
    }

    pub fn state(&self) -> &ChainState {
        &self.state
    }

    /// The picker's own state (its section, page, highlight, presets).
    pub fn picker(&self) -> &PickerView {
        &self.picker
    }

    /// What is open over the page (the mod matrix rows, the list of
    /// modes).
    pub fn open(&self) -> Open {
        self.open
    }

    /// The page's section widget as it stands (what it draws).
    pub fn page(&self) -> &SectionView {
        &self.page
    }

    /// The EQ's page as it stands (what it draws while the EQ is selected).
    pub fn eq_page(&self) -> &EqPage {
        &self.eq
    }

    /// The EQ's page is up: the EQ is selected and the picker shut.
    /// Turns the EQ's Listen off when the EQ page shows with it on (the
    /// app calls this when the editor goes away by a button). The actions
    /// are for the app to send; none when Listen is off.
    pub fn stop_listening(&mut self) -> Vec<ChainAction> {
        if self.eq_shown() {
            self.eq.stop_listening()
        } else {
            Vec::new()
        }
    }

    fn eq_shown(&self) -> bool {
        self.state.eq && self.state.selected.is_some() && !self.state.picking
    }

    /// Shows a new state. Of the old one it keeps only an open picker
    /// (the editor opened it; a state from the app with `picking` set
    /// opens it too). The mod matrix rows stay open on the same control,
    /// the list of modes on the same effect.
    pub fn update(&mut self, st: ChainState) {
        // The app opens the sidechain list itself when a Ducker is added
        // (it starts listening, with nothing picked).
        if self.list_open.is_none() {
            if let Some(l) = st.list.as_ref().filter(|l| l.sidechain.is_some()) {
                self.list_open = Some(l.group);
                self.list_first = 0;
                self.list_sidechain = true;
            }
        }
        // It shuts when the effect it was for has gone.
        if self.list_sidechain && st.sidechain.is_none() {
            self.list_open = None;
            self.list_sidechain = false;
        }
        let opening = st.picking && !self.state.picking;
        let picking = st.picking || self.state.picking;
        let old = self.state.list.take();
        let same_slot = |a: &ChainState, b: &ChainState| {
            let uid = |s: &ChainState| s.selected.and_then(|i| s.slots.get(i)).map(|v| v.uid);
            uid(a) == uid(b) && a.mode == b.mode
        };
        if !same_slot(&self.state, &st) {
            self.open.modes = false;
            self.open.presets = false;
        }
        if st.focus != self.state.focus || st.focus.is_none() {
            self.open.rows = false;
        }
        let row = |s: &ChainState| s.notes.as_ref().and_then(|n| n.selected);
        let reveal = (
            (row(&st) != row(&self.state)).then(|| row(&st)).flatten(),
            (st.selected != self.state.selected)
                .then_some(st.selected)
                .flatten(),
        );
        self.state = st;
        self.state.picking = picking;
        // The strip list stays as the view has it: open on its group (the
        // app's strips once they come), or shut.
        let sc = self.list_sidechain;
        let mine = |l: &StripList, g: usize| l.group == g && l.sidechain.is_some() == sc;
        self.state.list = self.list_open.map(|g| match self.state.list.take() {
            Some(l) if mine(&l, g) => l,
            _ => old.filter(|l| mine(l, g)).unwrap_or(StripList {
                group: g,
                sidechain: sc.then(Default::default),
                ..StripList::default()
            }),
        });
        if opening {
            self.open_picker();
        }
        self.fill_page();
        if self.state.eq {
            self.eq.update(&self.state.params, self.state.focus);
        }
        self.reveal(reveal.0, reveal.1);
    }

    /// Scrolls a note track's groups so the Notes row `note` and the
    /// sound's row `slot` (newly selected) show.
    fn reveal(&mut self, note: Option<usize>, slot: Option<usize>) {
        let r = self.last.get();
        if r.is_empty() || self.state.notes.is_none() {
            return;
        }
        let into = |from: &mut usize, first: usize, fit: usize, i: usize| {
            if i < first {
                *from = i;
            } else if i >= first + fit {
                *from = i + 1 - fit;
            }
        };
        let l = self.layout(r);
        if let (Some(i), Some(nl)) = (note, &l.notes) {
            into(&mut self.open.notes_from, nl.from, nl.fit, i);
        }
        if let Some(i) = slot {
            let first = shown_row(&l, false).unwrap_or(0);
            into(&mut self.open.sound_from, first, l.sound_fit, i);
        }
    }

    /// The picker opens on the section it showed last (the first time:
    /// Favourites if there are any, else the first category), on its first
    /// page, nothing highlighted. The section is kept from here, so a star
    /// doesn't move the picker to Favourites under the finger.
    fn open_picker(&mut self) {
        self.picker.reopen();
        let (list, at) = self.picker.current(&self.state);
        self.picker.section = Some(list[at]);
    }

    /// The strip list's group, while it is open.
    pub fn list_open(&self) -> Option<usize> {
        self.list_open
    }

    /// Opens the strip list (or the sidechain list) on group `g`, or
    /// shuts it (None), and tells the app.
    fn show_list(&mut self, g: Option<usize>) {
        if g != self.list_open {
            self.list_first = 0;
        }
        self.list_open = g;
        let sc = self.list_sidechain;
        self.state.list = g.map(|g| match self.state.list.take() {
            Some(mut l) => {
                if l.group != g {
                    l.strips.clear();
                }
                l.group = g;
                l
            }
            None => StripList {
                group: g,
                sidechain: sc.then(Default::default),
                ..StripList::default()
            },
        });
        if g.is_none() {
            self.list_sidechain = false;
        }
        self.push(if sc {
            ChainAction::SidechainList(g)
        } else {
            ChainAction::Strips(g)
        });
    }

    /// The selected effect's sidechain, when its page shows it.
    fn sidechain(&self) -> Option<&crate::state::SidechainView> {
        let st = &self.state;
        st.sidechain
            .as_ref()
            .filter(|sc| st.selected == Some(sc.slot) && !st.picking)
    }

    /// The selected effect's sidechain control: its index in
    /// [`ChainState::params`].
    fn sidechain_control(&self) -> Option<usize> {
        let sc = self.sidechain()?;
        self.state
            .params
            .iter()
            .position(|p| p.addr.param == sc.listens.param)
    }

    /// Opens the sidechain list for the selected effect: on the group of
    /// the track it listens to, else the strip's own (layout spec 4b).
    fn open_sidechain(&mut self) {
        let Some(sc) = self.sidechain() else {
            return;
        };
        let g = crate::state::sidechain_group(self.state.strip, sc.track.as_ref().map(|t| t.0));
        self.list_open = None;
        self.list_sidechain = true;
        self.state.list = None;
        self.show_list(Some(g));
    }

    /// What the sidechain list shows that the strip list doesn't, while it
    /// is the list open: the groups that can feed the effect (and the one
    /// shown), and whether it has Off (or Tempo).
    fn sc_parts(&self, list: &StripList) -> Option<ScParts> {
        list.sidechain.as_ref()?;
        let off = self
            .state
            .sidechain
            .as_ref()
            .is_some_and(|sc| sc.listens.off.is_some());
        Some(ScParts {
            shown: std::array::from_fn(|i| list.counts[i] > 0 || i == list.group),
            off,
        })
    }

    /// Where the list open goes in `r`.
    fn list_geom(&self, r: Rect) -> ListLayout {
        let sc = self.state.list.as_ref().and_then(|l| self.sc_parts(l));
        list_layout_for(r, sc)
    }

    /// The sidechain list's choice: `strip` (None: Off, or Tempo). The
    /// list shuts and the page shows it at once.
    fn set_sidechain(&mut self, strip: Option<(Strip, String)>) {
        let control = self
            .sidechain_control()
            .and_then(|k| self.state.params.get(k))
            .map(|p| p.addr);
        self.list_open = None;
        self.list_sidechain = false;
        self.state.list = None;
        let Some(control) = control else {
            return;
        };
        self.push(ChainAction::SetSidechain {
            control,
            strip: strip.as_ref().map(|s| s.0),
        });
        if let Some(sc) = self.state.sidechain.as_mut() {
            match strip {
                Some(t) => {
                    sc.track = Some(t);
                    sc.listening = true;
                }
                None => sc.listening = false,
            }
        }
        self.fill_page();
    }

    /// The strip list over the editor.
    fn paint_list(&self, cx: &mut PaintCx, r: Rect, list: &StripList) {
        let t = cx.theme.clone();
        let ll = self.list_geom(r);
        let sc = list.sidechain.as_ref();
        let listens = self.state.sidechain.as_ref();
        cx.canvas.fill_round_rect(ll.panel, 14, t.surface);
        cx.canvas.stroke_round_rect(ll.panel, 14, 1, t.outline);
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            ll.label,
            Align::Left,
            t.text_dim,
            if sc.is_some() { "LISTEN TO" } else { "STRIPS" },
        );
        // The sidechain list's Off (the Ducker's Tempo), lit while the
        // effect hears its own sound, with a rule under it.
        if let (Some(b), Some(l)) = (ll.off, listens) {
            let lit = !l.listening;
            cx.canvas
                .fill_round_rect(b, t.round(10), if lit { t.picked() } else { t.raised });
            let (name, about) = match l.listens.row {
                mpfree_fx::sidechain::Row::Source => ("Tempo", "ducks on the beat"),
                _ => ("Off", "listens to its own track"),
            };
            let inner = b.inset_xy(14, 8);
            let (top, bottom) = inner.take_top(inner.h / 2 + 2);
            cx.text(
                FontId::Bold,
                t.size_body - 2.0,
                top,
                Align::Left,
                if lit { t.on_picked() } else { t.text },
                name,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 3.0,
                bottom,
                Align::Left,
                if lit { t.on_picked() } else { t.text_dim },
                about,
            );
            cx.canvas
                .fill_rect(Rect::new(b.x, b.bottom() + 8, b.w, 1), t.outline);
        }
        for (i, (b, name)) in ll.groups.iter().zip(STRIP_GROUPS).enumerate() {
            if b.is_empty() {
                continue;
            }
            let on = i == list.group;
            cx.canvas.fill_round_rect(
                *b,
                t.round(10),
                if on {
                    t.raised
                } else {
                    t.surface.mix(t.raised, 0.35)
                },
            );
            if on {
                cx.canvas
                    .fill_rect(Rect::new(b.x, b.y + 8, 3, b.h - 16), t.accent);
            }
            let inner = b.inset_xy(14, 0);
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_body - 2.0,
                inner,
                Align::Left,
                if on { t.text } else { t.text_dim },
                name,
            );
            if i < 5 {
                cx.text(
                    FontId::Sans,
                    t.size_small - 2.0,
                    inner,
                    Align::Right,
                    t.text_faint,
                    &list.counts[i].to_string(),
                );
            }
        }
        let group = STRIP_GROUPS[list.group.min(STRIP_GROUPS.len() - 1)];
        let title = match sc {
            Some(sc) => sc.title.clone(),
            None => format!("{} · TAP ONE TO OPEN ITS CHAIN", group.to_uppercase()),
        };
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            ll.title,
            Align::Left,
            t.text_dim,
            &title,
        );
        if let Some(sc) = sc {
            // Why the groups left out can't feed it, under the groups.
            let mut y = ll.note.y;
            for line in wrap(cx, t.size_small - 3.0, &sc.note, ll.note.w) {
                if y + 18 > ll.note.bottom() {
                    break;
                }
                cx.text(
                    FontId::Sans,
                    t.size_small - 3.0,
                    Rect::new(ll.note.x, y, ll.note.w, 18),
                    Align::Left,
                    t.text_dim,
                    &line,
                );
                y += 18;
            }
            if let (Some(b), false) = (ll.now, sc.now.is_empty()) {
                cx.canvas
                    .fill_round_rect(b, t.round(10), t.surface.mix(t.raised, 0.5));
                let tone = sc.current.map_or(t.text_faint, |s| strip_color(&t, s));
                cx.canvas
                    .fill_round_rect(Rect::new(b.x + 10, b.y + 9, 4, b.h - 18), 2, tone);
                cx.text(
                    FontId::Sans,
                    t.size_small - 2.0,
                    Rect::new(b.x + 22, b.y, b.w - 30, b.h),
                    Align::Left,
                    t.text,
                    &sc.now,
                );
            }
        }
        cx.canvas.fill_round_rect(ll.back, t.round(10), t.raised);
        cx.text(
            FontId::Bold,
            t.size_body,
            ll.back,
            Align::Center,
            t.text,
            "‹  Back",
        );
        let n = list.strips.len();
        let page = ll.tiles.len().max(1);
        if n > page {
            for (b, s, live) in [
                (ll.prev, "<", self.list_first > 0),
                (ll.next, ">", self.list_first + page < n),
            ] {
                cx.canvas
                    .fill_round_rect(b, t.round(10), if live { t.raised } else { t.surface });
                cx.text(
                    FontId::Bold,
                    t.size_title,
                    b,
                    Align::Center,
                    if live { t.text } else { t.text_faint },
                    s,
                );
            }
        }
        let numbered = list.group <= 1;
        for (i, tile) in self.list_tiles(&ll) {
            let p = &list.strips[i];
            // In the sidechain list a strip it can't hear is greyed with
            // why, and the one it hears now is outlined.
            let refused = p.refused.is_some();
            cx.canvas.fill_round_rect(
                tile,
                t.round(10),
                if refused {
                    t.surface.mix(t.raised, 0.35)
                } else {
                    t.raised
                },
            );
            let current = match sc {
                Some(sc) => sc.current == Some(p.strip),
                None => p.strip == self.state.strip,
            };
            if current {
                cx.canvas.stroke_round_rect(tile, t.round(10), 2, t.accent);
            }
            cx.canvas.fill_round_rect(
                Rect::new(tile.x, tile.y + 12, 4, tile.h - 24),
                2,
                strip_color(&t, p.strip),
            );
            let inner = Rect::new(tile.x + 18, tile.y + 12, tile.w - 30, tile.h - 24);
            let (top, bottom) = inner.take_top(inner.h / 2 + 2);
            let name = if numbered {
                format!("{}  {}", i + 1, p.name)
            } else {
                p.name.clone()
            };
            let muted = p.sub.starts_with("muted");
            cx.text(
                FontId::Bold,
                t.size_small,
                top,
                Align::Left,
                if refused {
                    t.text_faint
                } else if muted {
                    t.text_dim
                } else {
                    t.text
                },
                &name,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 3.0,
                bottom,
                Align::Left,
                if refused { t.text_faint } else { t.text_dim },
                p.refused.unwrap_or(&p.sub),
            );
        }
        if n == 0 {
            let area = Rect::new(
                ll.title.x,
                ll.title.bottom() + 10,
                ll.now.map_or(ll.next.right(), |n| n.right()) - ll.title.x,
                TILE_H,
            );
            cx.text(
                FontId::Sans,
                t.size_body,
                area,
                Align::Left,
                t.text_dim,
                "No strips here",
            );
        }
    }

    /// The group the strip button opens the list on: the open strip's.
    fn group_of(s: Strip) -> usize {
        match s {
            Strip::Track(_) => 0,
            Strip::Pad(_) => 1,
            Strip::Input(_) => 2,
            Strip::Return(_) => 3,
            Strip::Bus(_) => 4,
            Strip::Master => 5,
        }
    }

    /// The tiles shown on the list's page: (strip index, tile).
    fn list_tiles(&self, ll: &ListLayout) -> Vec<(usize, Rect)> {
        let n = self.state.list.as_ref().map_or(0, |l| l.strips.len());
        (self.list_first..n).zip(ll.tiles.iter().copied()).collect()
    }

    /// A tap while the strip list is open: a group, a strip (its chain),
    /// the arrows, or Back (anywhere off the panel shuts it too). In the
    /// sidechain list a strip is what the effect listens to (one greyed
    /// does nothing), and Off (or Tempo) its own sound.
    fn tap_list(&mut self, r: Rect, x: i32, y: i32) {
        let ll = self.list_geom(r);
        if !ll.panel.contains(x, y) || ll.back.contains(x, y) {
            self.show_list(None);
            return;
        }
        if ll.off.is_some_and(|b| b.contains(x, y)) {
            self.set_sidechain(None);
            return;
        }
        if let Some(g) = ll.groups.iter().position(|b| b.contains(x, y)) {
            if Some(g) != self.list_open {
                self.show_list(Some(g));
            }
            return;
        }
        let n = self.state.list.as_ref().map_or(0, |l| l.strips.len());
        let page = ll.tiles.len().max(1);
        if ll.prev.contains(x, y) && n > page {
            self.list_first = self.list_first.saturating_sub(page);
            return;
        }
        if ll.next.contains(x, y) && n > page {
            if self.list_first + page < n {
                self.list_first += page;
            }
            return;
        }
        let hit = self
            .list_tiles(&ll)
            .into_iter()
            .find(|(_, t)| t.contains(x, y))
            .map(|(i, _)| i);
        let Some(p) = hit.and_then(|i| self.state.list.as_ref()?.strips.get(i).cloned()) else {
            return;
        };
        if self.list_sidechain {
            if p.refused.is_none() {
                self.set_sidechain(Some((p.strip, p.name)));
            }
            return;
        }
        self.list_open = None;
        self.state.list = None;
        self.push(ChainAction::OpenStrip(p.strip));
    }

    /// Shows the selected effect's drawings ([`crate::state::picture`]:
    /// every one its effect page draws, page `""`; empty for none). Each
    /// section shows the ones its plan names.
    pub fn set_picture(&mut self, cards: Vec<Card>) {
        self.cards = cards;
        self.fill_page();
    }

    /// Where everything goes in `r` as the widget stands.
    pub fn layout(&self, r: Rect) -> Layout {
        layout(r, &self.state, &self.picker, self.open)
    }

    /// Where parameter `k`'s control (an index into
    /// [`ChainState::params`]) is in `r`, as the page draws it.
    pub fn control_rect(&self, r: Rect, k: usize) -> Option<Rect> {
        let page = self.layout(r).page;
        self.page.control_geom(page, k).map(|(_, g)| g.cell)
    }

    /// Fills the section widget from the state: a box per section, a
    /// control per parameter (its index in [`ChainState::params`] is what
    /// its events carry), the drawings its plan names, and the touched
    /// control selected.
    fn fill_page(&mut self) {
        let st = &self.state;
        let boxes = st
            .sections
            .iter()
            .map(|s| {
                let mut b = SectionBox::new(&s.key, &s.title, s.job);
                b.note = s.note.clone();
                b.link = s.link;
                if let Some(a) = s.action {
                    b.link = Some(ACTION_TAB);
                    b.link_text = Some(format!("{a} ›"));
                }
                b.rows = s.rows;
                b.cards = s
                    .drawings
                    .iter()
                    .filter_map(|d| self.cards.iter().find(|c| &c.title == d).cloned())
                    .collect();
                b.controls = s
                    .controls
                    .iter()
                    .filter_map(|c| control(&st.params, c, s.rows))
                    .collect();
                b.switch = s
                    .switch
                    .as_ref()
                    .and_then(|c| control(&st.params, c, false));
                b
            })
            .collect();
        let mut boxes: Vec<SectionBox> = boxes;
        self.sidechain_row(&mut boxes);
        let st = &self.state;
        self.page.spec = st.spec.clone();
        self.page.sections = boxes;
        self.page.selected = st
            .focus
            .and_then(|f| st.params.iter().position(|p| p.addr == f));
    }

    /// The selected effect's sidechain on its page (layout spec 4b,
    /// "Sidechain"): its control as one row, "Off | Kick ›" (the Ducker's
    /// "Kick › | Tempo"), or on the Vocoder on Input "Voice: Mic ›" in its
    /// Carrier heading; and, set to listen with nothing picked, its
    /// section's note in red.
    fn sidechain_row(&self, boxes: &mut [SectionBox]) {
        use mpfree_fx::sidechain::Row;
        let (Some(sc), Some(k)) = (self.sidechain(), self.sidechain_control()) else {
            return;
        };
        for b in boxes.iter_mut() {
            if !b.controls.iter().any(|c| c.index == k) {
                continue;
            }
            match sc.listens.row {
                Row::Switch | Row::Source => {
                    let (names, index) = if sc.listens.row == Row::Switch {
                        (vec!["Off".to_string(), sc.button()], sc.listening as usize)
                    } else {
                        (
                            vec![sc.button(), "Tempo".to_string()],
                            !sc.listening as usize,
                        )
                    };
                    for c in b.controls.iter_mut().filter(|c| c.index == k) {
                        c.value = names[index].clone();
                        c.unit.clear();
                        c.kind = Kind::Choice {
                            names: names.clone(),
                            index,
                        };
                    }
                }
                Row::Voice if sc.listening => {
                    b.link = Some(SIDECHAIN_TAB);
                    b.link_text = Some(sc.button());
                }
                Row::Voice => {}
            }
            if sc.missing() {
                b.note = sc.listens.nothing.to_string();
                b.note_warn = true;
            }
        }
    }

    /// Whether a finger lifted at `x`, `y` on the page is on the
    /// Sidechain row's track button while a tap there opens the list: it
    /// is lit (to change the track), or nothing is picked yet. While Off
    /// is lit a tap on a track already picked goes back to it, as the
    /// row's other button does.
    fn on_sidechain_button(&self, l: &Layout, x: i32, y: i32) -> bool {
        use mpfree_fx::sidechain::Row;
        let (Some(sc), Some(k)) = (self.sidechain(), self.sidechain_control()) else {
            return false;
        };
        let part = match sc.listens.row {
            Row::Switch => 1,
            Row::Source => 0,
            Row::Voice => return false,
        };
        if !sc.listening && sc.track.is_some() {
            return false;
        }
        self.page
            .control_geom(l.page, k)
            .is_some_and(|(_, g)| !g.chip && g.parts.get(part).is_some_and(|p| p.contains(x, y)))
    }

    /// The data wheel while the picker is open: through the section's
    /// tiles (or the presets list), or with SHIFT through the sections.
    /// False when the picker is shut, so the wheel keeps its other job.
    pub fn wheel(&mut self, delta: i32, shift: bool) -> bool {
        if !self.state.picking || delta == 0 {
            return false;
        }
        let pv = &mut self.picker;
        if shift {
            let (list, at) = pv.current(&self.state);
            let to = (at as i32 + delta).clamp(0, list.len() as i32 - 1) as usize;
            pv.section = Some(list[to]);
            pv.page = 0;
            pv.presets = None;
            pv.highlight = None;
            return true;
        }
        let n = match pv.presets {
            Some(id) => 1 + mpfree_fx::picker::preset_rows(id).len(),
            None => pv.picks(&self.state).len(),
        };
        if n > 0 {
            let at = pv.highlight.map_or(-1, |h| h as i32);
            pv.highlight = Some((at + delta).clamp(0, n as i32 - 1) as usize);
        }
        true
    }

    /// The data wheel's push while the picker is open: on an effect, open
    /// its presets (or add it, when it has none); on the presets list, add
    /// the effect with the one highlighted ("Defaults" first). False when
    /// the picker is shut.
    pub fn push_wheel(&mut self) -> bool {
        if !self.state.picking {
            return false;
        }
        let Some(h) = self.picker.highlight else {
            // The first push lights the first tile.
            self.picker.highlight = Some(0);
            return true;
        };
        match self.picker.presets {
            Some(id) => self.add_preset(id, h),
            None => {
                if let Some(p) = self.picker.picks(&self.state).get(h) {
                    self.open_or_add(p.id);
                }
            }
        }
        true
    }

    /// Opens an effect's presets, or adds it when it has none.
    fn open_or_add(&mut self, id: &'static str) {
        if mpfree_fx::picker::preset_rows(id).is_empty() {
            self.state.picking = false;
            self.push(ChainAction::Add(id));
        } else {
            self.picker.presets = Some(id);
            self.picker.highlight = Some(0);
            self.picker.page = 0;
        }
    }

    /// Adds `id` with presets list row `row` (0 is "Defaults"). A row of a
    /// family's list adds the engine of its mode.
    fn add_preset(&mut self, id: &'static str, row: usize) {
        self.state.picking = false;
        self.picker.presets = None;
        let rows = mpfree_fx::picker::preset_rows(id);
        self.push(match row.checked_sub(1).and_then(|r| rows.get(r)) {
            Some(r) => ChainAction::AddPreset {
                id: r.id,
                preset: r.preset,
            },
            None => ChainAction::Add(id),
        });
    }

    /// A touch in the open picker, on the finger's way down. Sections,
    /// pages, Back and the star act at once; a tile, a preset row and a
    /// presets line wait for the finger to lift on them, so a slip adds
    /// nothing (and a tile tells a tap from a hold).
    fn tap_picker(&mut self, pl: &PickerLayout, finger: u64, x: i32, y: i32) {
        let Some(hit) = crate::picker::hit(pl, x, y) else {
            return;
        };
        let pv = &mut self.picker;
        match hit {
            Hit::Section(i) => {
                let (list, _) = pv.current(&self.state);
                pv.section = list.get(i).copied();
                pv.page = 0;
                pv.highlight = None;
                pv.presets = None;
            }
            Hit::Prev | Hit::Next => {
                let step = if hit == Hit::Next { 1 } else { -1 };
                pv.page = (pl.page as i32 + step).clamp(0, pl.pages as i32 - 1) as usize;
                pv.highlight = None;
            }
            Hit::Back => {
                pv.presets = None;
                pv.highlight = None;
                pv.page = 0;
            }
            Hit::Star(i) => {
                if let Some(p) = pv.picks(&self.state).get(i) {
                    let on = !self.state.shelf.is_favourite(p.id);
                    self.state.shelf.set_favourite(p.id, on);
                    self.push(ChainAction::Favourite { id: p.id, on });
                }
            }
            Hit::Chip(_) | Hit::Tile(_) => {
                pv.press = Some((finger, hit, std::time::Instant::now()))
            }
        }
    }

    /// A finger lifted on what it went down on: a tile adds its effect (or
    /// opens its presets if it was held, or adds an effect that has none);
    /// a preset row adds the effect with that preset; a presets line opens
    /// the presets.
    fn lift_picker(&mut self, pl: &PickerLayout, finger: u64, x: i32, y: i32) {
        let Some((f, hit, since)) = self.picker.press.take() else {
            return;
        };
        if f != finger || crate::picker::hit(pl, x, y) != Some(hit) {
            return;
        }
        let pick = |s: &ChainView, i: usize| s.picker.picks(&s.state).get(i).copied();
        match (hit, self.picker.presets) {
            (Hit::Tile(row), Some(id)) => self.add_preset(id, row),
            (Hit::Tile(i), None) => {
                let Some(p) = pick(self, i) else {
                    return;
                };
                if since.elapsed() >= HOLD {
                    self.open_or_add(p.id);
                } else {
                    self.state.picking = false;
                    self.push(ChainAction::Add(p.id));
                }
            }
            (Hit::Chip(i), None) => {
                if let Some(p) = pick(self, i) {
                    self.open_or_add(p.id);
                }
            }
            _ => {}
        }
    }

    /// A finger has stayed on an effect's tile past [`HOLD`] and the
    /// effect has presets: [`ChainView::hold_tick`] opens them now.
    pub fn hold_due(&self) -> bool {
        let Some((_, Hit::Tile(i), since)) = self.picker.press else {
            return false;
        };
        self.state.picking
            && self.picker.presets.is_none()
            && since.elapsed() >= HOLD
            && self
                .picker
                .picks(&self.state)
                .get(i)
                .is_some_and(|p| !mpfree_fx::picker::preset_rows(p.id).is_empty())
    }

    /// Call every frame while the picker is open: a finger held on a tile
    /// opens its presets while it is still down, as other holds do, and
    /// its lift then does nothing more. True when it opened them. (An
    /// effect with no presets waits for the lift, which adds it.)
    pub fn hold_tick(&mut self) -> bool {
        if !self.hold_due() {
            return false;
        }
        let Some((_, Hit::Tile(i), _)) = self.picker.press.take() else {
            return false;
        };
        let Some(p) = self.picker.picks(&self.state).get(i).copied() else {
            return false;
        };
        self.open_or_add(p.id);
        true
    }

    /// Tests: the finger on a tile has been there `secs` longer.
    #[cfg(test)]
    pub(crate) fn age_press(&mut self, secs: f32) {
        if let Some((_, _, since)) = &mut self.picker.press {
            *since -= std::time::Duration::from_secs_f32(secs);
        }
    }

    pub fn take_actions(&mut self) -> Vec<ChainAction> {
        std::mem::take(&mut self.actions)
    }

    fn push(&mut self, a: ChainAction) {
        // Leaving the EQ page stops Listen first, so the track never stays
        // on one band.
        let leaving = match &a {
            ChainAction::Select(i) => self.state.selected != Some(*i),
            ChainAction::Back
            | ChainAction::Scope(_)
            | ChainAction::OpenStrip(_)
            | ChainAction::OpenPage(_)
            | ChainAction::Picker(true)
            | ChainAction::Remove(_) => true,
            _ => false,
        };
        if leaving && self.eq_shown() {
            for stop in self.eq.stop_listening() {
                self.actions.push(stop);
            }
        }
        // Show what can be shown at once.
        match &a {
            ChainAction::Select(i) => self.state.selected = Some(*i),
            // The removed effect's page shuts at once, before the app's
            // next state: the list, nothing picked.
            ChainAction::Remove(slot) if self.state.selected == Some(*slot) => {
                self.state.selected = None;
                self.state.eq = false;
            }
            ChainAction::SetOn { slot, on } => {
                if let Some(s) = self.state.slots.get_mut(*slot) {
                    s.on = *on;
                }
            }
            ChainAction::Group(g) => self.state.group = *g,
            ChainAction::Scope(i) => self.state.scope = *i,
            ChainAction::Focus(addr) => {
                if self.state.focus != Some(*addr) {
                    self.open.rows = false;
                }
                self.state.focus = Some(*addr);
            }
            ChainAction::Set { addr, value } => {
                for p in self.state.params.iter_mut().filter(|p| p.addr == *addr) {
                    p.value = *value;
                    p.pos = p.info.to_unit(*value);
                }
            }
            ChainAction::Macro { target, macro_ } => {
                if let Some(p) = self.state.params.iter_mut().find(|p| p.addr == *target) {
                    p.macro_ = *macro_;
                }
            }
            ChainAction::Learn(_) => self.state.learning = true,
            ChainAction::Depth { row, depth } => {
                if let Some(r) = self.state.rows.iter_mut().find(|r| r.index == *row) {
                    r.depth = *depth;
                }
            }
            _ => {}
        }
        self.actions.push(a);
        self.fill_page();
    }

    /// Moves insert `from` to slot `to` (Up, Down, a drag or the wheel):
    /// asks the app, and shows it moved, picked, at once.
    fn move_slot(&mut self, from: usize, to: usize) {
        if from == to || from >= self.state.slots.len() || to >= self.state.slots.len() {
            return;
        }
        self.push(ChainAction::Move { from, to });
        let s = self.state.slots.remove(from);
        self.state.slots.insert(to, s);
        self.state.selected = Some(to);
    }

    /// A touch on the Notes group, its Plays on row, a group's arrows, or
    /// Up, Down and Remove while a Notes row is selected. True when it
    /// took the touch.
    fn tap_notes(&mut self, l: &Layout, x: i32, y: i32) -> bool {
        let (Some(ns), Some(nl)) = (self.state.notes.clone(), l.notes.as_ref()) else {
            return false;
        };
        if let Some((up, down)) = nl.arrows {
            if up.contains(x, y) {
                self.open.notes_from = nl.from.saturating_sub(1);
                return true;
            }
            if down.contains(x, y) {
                self.open.notes_from = nl.from + 1;
                return true;
            }
        }
        if let Some((up, down)) = l.sound_arrows {
            let first = shown_row(l, false).unwrap_or(0);
            if up.contains(x, y) {
                self.open.sound_from = first.saturating_sub(1);
                return true;
            }
            if down.contains(x, y) {
                self.open.sound_from = first + 1;
                return true;
            }
        }
        if let Some(&(r, row)) = nl.rows.iter().find(|(r, _)| r.contains(x, y)) {
            let item = ns.item(row);
            match item {
                Some(mpfree_notefx::Item::Fx(p)) if notes::pill_of(r).contains(x, y) => {
                    let on = !ns.fx.slots[p].is_some_and(|s| s.on);
                    self.push(ChainAction::Notes(NoteAction::SetOn { row, on }));
                }
                _ => self.select_note(row),
            }
            return true;
        }
        if nl.add.is_some_and(|r| r.contains(x, y)) {
            self.state.picking = false;
            if let Some(n) = self.state.notes.as_mut() {
                n.picking = true;
            }
            self.push(ChainAction::Notes(NoteAction::Picker(true)));
            return true;
        }
        if nl.plays.contains(x, y) {
            self.push(ChainAction::Notes(NoteAction::PlaysOn));
            return true;
        }
        let Some((row, n, fx)) = notes_selected(&self.state) else {
            return false;
        };
        let to = if l.buttons[0].contains(x, y) {
            (row > 0).then(|| row - 1)
        } else if l.buttons[1].contains(x, y) {
            (row + 1 < n).then_some(row + 1)
        } else {
            if l.buttons[2].contains(x, y) && fx {
                self.push(ChainAction::Notes(NoteAction::Remove(row)));
                return true;
            }
            return l.buttons.iter().any(|b| b.contains(x, y));
        };
        if let Some(to) = to {
            self.push(ChainAction::Notes(NoteAction::Move { from: row, to }));
            if let Some(n) = self.state.notes.as_mut() {
                n.fx.move_row(row, to);
                n.selected = Some(to);
            }
        }
        true
    }

    /// Row `row` of the Notes group shows its page (the sound's effect,
    /// and any picker, give way at once).
    fn select_note(&mut self, row: usize) {
        self.state.picking = false;
        self.state.selected = None;
        if let Some(n) = self.state.notes.as_mut() {
            n.selected = Some(row);
            n.picking = false;
        }
        self.open.presets = false;
        self.open.modes = false;
        self.open.rows = false;
        self.push(ChainAction::Notes(NoteAction::Select(row)));
    }

    /// A sound effect (or its picker) takes the page from the Notes group.
    fn leave_notes(&mut self) {
        if let Some(n) = self.state.notes.as_mut() {
            n.selected = None;
            n.picking = false;
        }
    }

    /// A tap on the Notes page: a setting, the arp's button, or the
    /// picker's.
    fn tap_notes_page(&mut self, pl: &notes::PageLayout, x: i32, y: i32) {
        let Some(ns) = self.state.notes.as_ref() else {
            return;
        };
        // The setting under the finger becomes the one the Modulate row
        // works on.
        let touched = notes::setting_at(pl, x, y).and_then(|(k, _)| {
            let (_, id, _) = ns.page_fx()?;
            Some(notes::addr(ns.track, id, k))
        });
        let a = notes::tap_page(pl, ns, x, y);
        if let Some(addr) = touched.filter(|f| self.state.focus != Some(*f)) {
            if self.state.params.iter().any(|p| p.addr == addr) {
                self.push(ChainAction::Focus(addr));
            }
        }
        let Some(a) = a else {
            return;
        };
        if matches!(a, NoteAction::Picker(false) | NoteAction::Add(_)) {
            if let Some(n) = self.state.notes.as_mut() {
                n.picking = false;
            }
        }
        self.set_note(a);
    }

    /// A MIDI effect's setting is the one touched last: the Modulate row
    /// under its page is the usual one (its macro, Learn, + LFO, its rows).
    fn notes_modulate(&self) -> bool {
        self.state.notes.is_some()
            && self
                .state
                .focused()
                .is_some_and(|p| matches!(p.addr.owner, Owner::NoteFx(..)))
    }

    /// Setting `k` of the MIDI effect whose page shows, with its value
    /// ("Inversion 1"), for the hint line while a finger holds it.
    pub fn note_setting(&self, k: usize) -> Option<String> {
        let (_, _, fx) = self.state.notes.as_ref()?.page_fx()?;
        let name = fx.settings().get(k)?.name;
        Some(format!("{name} {}", fx.value_text(k)))
    }

    /// Whether `t` is a MIDI effect's setting held under the wheel
    /// ([`ChainView::note_setting`] reads it).
    pub fn is_note_setting(t: &Target) -> bool {
        t.id == NOTE_SETTING
    }

    /// A Notes action for the app; a setting shows its new value at once
    /// (the app's state follows), so turns in a row each step on from the
    /// last.
    fn set_note(&mut self, a: NoteAction) {
        if let (NoteAction::Set { id, k, value }, Some(n)) = (a, self.state.notes.as_mut()) {
            if let Some(s) = n.fx.place_of(id).and_then(|p| n.fx.slots[p].as_mut()) {
                s.fx.set(k, value);
            }
        }
        self.push(ChainAction::Notes(a));
    }

    /// The effect whose row (not its ON pill) is under `(x, y)`, when the
    /// chain can be rearranged (not while the picker is up).
    fn slot_under(&self, l: &Layout, x: i32, y: i32) -> Option<usize> {
        if self.state.picking {
            return None;
        }
        l.rows
            .iter()
            .position(|r| r.contains(x, y) && !r.take_right(POWER_W).0.contains(x, y))
    }

    /// A finger down on the chain column, the header or the Modulate row
    /// (the page, the strip list and what is open over the page are
    /// [`ChainView::touch_inner`]'s).
    fn tap(&mut self, l: &Layout, finger: u64, x: i32, y: i32) {
        if l.back.is_some_and(|b| b.contains(x, y)) {
            self.push(ChainAction::Back);
            return;
        }
        if l.strip.is_some_and(|b| b.contains(x, y)) {
            self.show_list(Some(ChainView::group_of(self.state.strip)));
            return;
        }
        if let Some(i) = l.scopes.iter().position(|r| r.contains(x, y)) {
            self.push(ChainAction::Scope(i));
            return;
        }
        if self.state.picking {
            if l.cancel.is_some_and(|c| c.contains(x, y)) {
                self.state.picking = false;
                self.push(ChainAction::Picker(false));
                return;
            }
            if let Some(pl) = &l.picker {
                if l.effect.contains(x, y) {
                    let pl = pl.clone();
                    self.tap_picker(&pl, finger, x, y);
                    return;
                }
            }
            if l.effect.contains(x, y) {
                return;
            }
        }
        if self.tap_notes(l, x, y) {
            return;
        }
        let st = &self.state;
        for (i, r) in l.rows.iter().enumerate() {
            if r.contains(x, y) {
                if r.take_right(POWER_W).0.contains(x, y) {
                    let on = !st.slots[i].on;
                    self.push(ChainAction::SetOn { slot: i, on });
                } else {
                    self.leave_notes();
                    self.push(ChainAction::Select(i));
                }
                return;
            }
        }
        if l.add.is_some_and(|r| r.contains(x, y)) {
            self.leave_notes();
            self.state.picking = true;
            self.open_picker();
            self.push(ChainAction::Picker(true));
            return;
        }
        let st = &self.state;
        if let Some(sel) = st.selected {
            let n = st.slots.len();
            if l.buttons[0].contains(x, y) && sel > 0 {
                self.move_slot(sel, sel - 1);
                return;
            }
            if l.buttons[1].contains(x, y) && sel + 1 < n {
                self.move_slot(sel, sel + 1);
                return;
            }
            if l.buttons[2].contains(x, y) {
                self.push(ChainAction::Remove(sel));
                return;
            }
        }
        if st.picking || st.selected.is_none() {
            return;
        }
        if let Some(g) = l.tabs.iter().position(|r| r.contains(x, y)) {
            self.push(ChainAction::Group(g));
            return;
        }
        if let Some((prev, next)) = l.tab_arrows {
            let n = st.groups.len();
            if prev.contains(x, y) {
                if st.group > 0 {
                    self.push(ChainAction::Group(st.group - 1));
                }
                return;
            }
            if next.contains(x, y) {
                if st.group + 1 < n {
                    self.push(ChainAction::Group(st.group + 1));
                }
                return;
            }
        }
        if let (Some(&(_, mode)), Some(slot)) =
            (l.modes.iter().find(|(r, _)| r.contains(x, y)), st.selected)
        {
            if st.mode.as_ref().is_some_and(|m| m.current != Some(mode)) {
                self.push(ChainAction::SetMode { slot, mode });
            }
            return;
        }
        if l.mode_label.is_some_and(|r| r.contains(x, y)) {
            self.open.modes = !self.open.modes;
            return;
        }
        if l.preset_chip.is_some_and(|r| r.contains(x, y)) {
            self.open.presets = true;
            self.open.modes = false;
            return;
        }
        self.tap_modulate(l, x, y);
    }

    /// A tap on the Modulate row, for the control touched last: its macro
    /// (again: off it), Learn, the link to its rows, + LFO or + Follower.
    fn tap_modulate(&mut self, l: &Layout, x: i32, y: i32) {
        let st = &self.state;
        let Some(focus) = st.focus else { return };
        if let Some(m) = l.macros.iter().position(|r| macro_hit(*r).contains(x, y)) {
            let now = st.focused().and_then(|p| p.macro_);
            self.push(ChainAction::Macro {
                target: focus,
                macro_: if now == Some(m) { None } else { Some(m) },
            });
            return;
        }
        if l.learn.contains(x, y) {
            self.push(ChainAction::Learn(focus));
            return;
        }
        // A greyed chip goes too: the app says why the pad can't move it.
        if let Some(k) = l.xy_chips.iter().position(|r| r.contains(x, y)) {
            self.push(ChainAction::Xy {
                addr: focus,
                up: k == 1,
            });
            return;
        }
        if l.rows_link.contains(x, y) && !st.rows.is_empty() {
            self.open.rows = !self.open.rows;
            return;
        }
        // A new row opens the rows, to set its depth.
        let source = if l.add_lfo.contains(x, y) {
            Source::Lfo(0)
        } else if l.add_follower.contains(x, y) {
            Source::Follower(0)
        } else {
            return;
        };
        if focused_takes_rows(st) {
            self.push(ChainAction::AddRow {
                target: focus,
                source,
            });
            self.open.rows = true;
        }
    }

    /// What a finger on the page meant: the section widget's event as
    /// actions. A control touched becomes the one the Modulate row works
    /// on (a meter doesn't); a drag sets it (`turned`: the app hears
    /// Release when the finger lifts); a button, an arrow or a switch sets
    /// it a step at once; a section's link shows that page.
    fn page_event(&mut self, ev: EventKind, turned: &mut Option<ParamAddr>) {
        match ev {
            EventKind::Select(i) => match self.state.params.get(i) {
                Some(p) if !p.info.readonly => {
                    if self.state.focus != Some(p.addr) {
                        let addr = p.addr;
                        self.push(ChainAction::Focus(addr));
                    }
                }
                _ => self.fill_page(),
            },
            EventKind::Value(pos) => {
                let Some(p) = self.page.selected.and_then(|i| self.state.params.get(i)) else {
                    return;
                };
                let (addr, value) = (p.addr, p.info.from_unit(pos));
                *turned = Some(addr);
                if value != p.value {
                    self.push(ChainAction::Set { addr, value });
                }
            }
            EventKind::Step { index, delta } => {
                let Some(p) = self.state.params.get(index) else {
                    return;
                };
                let (addr, info, now) = (p.addr, p.info, p.value);
                let value = match info.curve {
                    Curve::Toggle if delta > 0 => info.max,
                    Curve::Toggle => info.min,
                    _ => (now.round() + delta as f32).clamp(info.min, info.max),
                };
                if self.state.focus != Some(addr) {
                    self.push(ChainAction::Focus(addr));
                }
                if value != now {
                    self.push(ChainAction::Set { addr, value });
                    self.push(ChainAction::Release(addr));
                }
            }
            EventKind::Tab(n) if n < self.state.groups.len() && n != self.state.group => {
                self.push(ChainAction::Group(n));
            }
            // A heading's action: Capture.
            EventKind::Tab(ACTION_TAB) => {
                if let Some(slot) = self.state.selected {
                    self.push(ChainAction::Capture(slot));
                }
            }
            // The Vocoder's "Voice: Mic ›": the sidechain list.
            EventKind::Tab(SIDECHAIN_TAB) => self.open_sidechain(),
            _ => {}
        }
    }
}

/// Parameter `c.param` as a control of the section widget: drawn as what
/// it is (a meter for a read-only value, buttons or a chip for a choice, a
/// pill for a switch, a fader where the plan says, a stepper for a small
/// whole number, a slider in a section of rows, else a knob), its value and
/// unit apart.
fn control(params: &[ParamView], c: &crate::sections::CtlState, rows: bool) -> Option<Control> {
    let p = params.get(c.param)?;
    let bipolar = p.info.min < 0.0 && p.info.max > 0.0;
    let pos = p.pos;
    let kind = if p.info.readonly {
        Kind::Meter
    } else {
        match p.info.curve {
            Curve::Choice(names) => Kind::Choice {
                names: names.iter().map(|s| s.to_string()).collect(),
                index: p.value.round().max(0.0) as usize,
            },
            Curve::Toggle => Kind::Toggle(p.value >= 0.5),
            _ if c.fader => Kind::Fader { pos, bipolar },
            Curve::Stepped if p.info.max - p.info.min <= 48.0 => Kind::Stepper,
            _ if rows => Kind::Slider { pos, bipolar },
            _ => Kind::Knob { pos, bipolar },
        }
    };
    let text = p.text();
    let (value, unit) = match text.rsplit_once(' ') {
        Some((v, u)) if !u.chars().any(|ch| ch.is_ascii_digit()) && !v.is_empty() => {
            (v.to_string(), u.to_string())
        }
        _ => (text, String::new()),
    };
    Some(Control {
        index: c.param,
        label: c.label.clone(),
        value,
        unit,
        kind,
        tier: c.tier,
        qlink: c.qlink,
        dim: c.dim.is_some(),
        tint: None,
    })
}

/// One effect's row in the chain column: its number, name, summary (or
/// "bypassed") with a dot per macro that moves it and a wave mark if the
/// matrix does, and its ON pill.
fn paint_slot(
    cx: &mut PaintCx,
    t: &mpfree_ui::Theme,
    row: Rect,
    i: usize,
    s: &crate::state::SlotView,
    sel: bool,
) {
    cx.canvas.fill_round_rect(
        row,
        t.round(10),
        if sel {
            t.raised
        } else {
            t.surface.mix(t.raised, 0.35)
        },
    );
    if sel {
        cx.canvas
            .fill_round_rect(Rect::new(row.x, row.y + 13, 4, row.h - 26), 2, t.accent);
    }
    let ink = t.button_ink(s.on);
    let badge = Rect::new(row.x + 10, row.y + (row.h - 24) / 2, 24, 24);
    cx.canvas.fill_round_rect(
        badge,
        t.round(6),
        if s.on {
            t.accent.mix(t.surface, 0.55)
        } else {
            t.outline
        },
    );
    cx.text(
        FontId::Bold,
        t.size_small - 1.0,
        badge,
        Align::Center,
        ink,
        &(i + 1).to_string(),
    );
    let (power, text) = row.take_right(POWER_W);
    let text = Rect::new(
        badge.right() + 8,
        row.y,
        text.right() - badge.right() - 10,
        row.h,
    );
    let (top, bottom) = text.take_top(row.h / 2 + 1);
    cx.text(
        FontId::Bold,
        t.size_body - 2.0,
        top.take_bottom(22).0,
        Align::Left,
        ink,
        &s.name,
    );
    // The summary stops short of the macro dots and the wave mark at its
    // right: the dots are 8 px in 11 px steps, the wave's ink about 5 px
    // into its 16 px box left of them.
    let mut dx = text.right();
    for m in s.macros.iter().rev() {
        dx -= 11;
        cx.canvas
            .fill_round_rect(Rect::new(dx, bottom.y + 6, 8, 8), 4, macro_color(t, *m));
    }
    if s.modulated {
        dx -= 16;
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            Rect::new(dx, bottom.y, 16, 20),
            Align::Center,
            t.accent,
            "~",
        );
    }
    let sub = if s.on {
        s.summary.clone()
    } else {
        "bypassed".to_string()
    };
    // "Nothing picked": set to listen to a sidechain with no track.
    let warn = s.on && s.warn;
    cx.text(
        if warn { FontId::Bold } else { FontId::Sans },
        t.size_small - 2.0,
        Rect::new(bottom.x, bottom.y, dx - 4 - bottom.x, 20),
        Align::Left,
        if warn { t.bad } else { t.text_dim },
        &sub,
    );
    let pill = Rect::new(power.x + 4, power.y + (power.h - 24) / 2, 40, 24);
    cx.canvas.fill_round_rect(
        pill,
        12,
        if s.on {
            t.good.mix(t.surface, 0.3)
        } else {
            t.outline
        },
    );
    cx.text(
        FontId::Bold,
        t.size_small - 3.0,
        pill,
        Align::Center,
        if s.on { t.on_accent } else { t.text_dim },
        if s.on { "ON" } else { "OFF" },
    );
}

/// The strip button: the strip's colour, its name over which strip it is,
/// and a chevron (a tap opens the strip list).
fn paint_strip_button(cx: &mut PaintCx, b: Rect, st: &ChainState) {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(b, 10, t.raised);
    cx.canvas.stroke_round_rect(b, 10, 1, t.outline);
    let dot = Rect::new(b.x + 10, b.y + b.h / 2 - 4, 8, 8);
    cx.canvas.fill_round_rect(dot, 4, strip_color(&t, st.strip));
    let inner = Rect::new(dot.right() + 8, b.y + 3, b.w - 26 - 30, b.h - 6);
    let (top, bottom) = inner.take_top(inner.h / 2 + 1);
    cx.text(
        FontId::Bold,
        t.size_small,
        top,
        Align::Left,
        t.text,
        &st.strip_name,
    );
    if st.strip_kind != st.strip_name {
        cx.text(
            FontId::Sans,
            t.size_small - 3.0,
            bottom,
            Align::Left,
            t.text_dim,
            &st.strip_kind,
        );
    }
    chevron(cx, b.right() - 22, b.y + b.h / 2 - 3, t.text_dim);
}

/// A small chevron pointing down (something opens a list), its top left
/// at (x, y): two short diagonals, 11 px wide.
fn chevron(cx: &mut PaintCx, x: i32, y: i32, c: mpfree_ui::Color) {
    for i in 0..5 {
        cx.canvas.fill_rect(Rect::new(x + i, y + i, 2, 2), c);
        cx.canvas.fill_rect(Rect::new(x + 9 - i, y + i, 2, 2), c);
    }
}

/// Switches (toggles and choices) take macros but no matrix rows.
fn focused_takes_rows(st: &ChainState) -> bool {
    st.focused().is_some_and(|p| {
        !matches!(
            p.info.curve,
            mpfree_fx::mpfree_node::Curve::Toggle | mpfree_fx::mpfree_node::Curve::Choice(_)
        )
    })
}

/// Where on a dial `d` across a setting is at knob position `p`: the
/// angle its ring uses (clockwise on screen from the bottom left).
fn ring_angle(p: f32) -> f32 {
    0.75 * PI + 1.5 * PI * p.clamp(0.0, 1.0)
}

/// The XY pad's arc's radius round a dial `d` across: outside its ring,
/// and outside the matrix's ring when it has one.
fn xy_radius(d: f32, p: &ParamView) -> f32 {
    d * 0.5 + if p.mod_range.is_some() { 13.0 } else { 10.0 }
}

/// A box round a dial at `kx`, `ky` that holds everything drawn at `r`.
fn xy_box(kx: f32, ky: f32, r: f32) -> Rect {
    let reach = r + 10.0;
    Rect::new(
        (kx - reach) as i32,
        (ky - reach) as i32,
        (2.0 * reach) as i32 + 1,
        (2.0 * reach) as i32 + 1,
    )
}

/// The XY pad's stretch round a dial at `kx`, `ky` (layout spec 4b's XY
/// marks): a thin arc in the text colour at radius `r`, from a tick at
/// the layout's from to an arrow at its to.
fn paint_xy_arc(cx: &mut PaintCx, kx: f32, ky: f32, r: f32, m: XyMark) {
    let t = cx.theme.clone();
    let angle = ring_angle;
    let bbox = xy_box(kx, ky, r);
    let at = |a: f32, rr: f32| (kx + rr * a.cos(), ky + rr * a.sin());
    let (a0, a1) = (angle(m.from), angle(m.to));
    let mut arc = Mask::new(bbox);
    let lo = a0.min(a1);
    arc.arc(kx, ky, r, lo, a0.max(a1).max(lo + 0.02), 1.5);
    arc.segment(at(a0, r - 4.0), at(a0, r + 4.0), 1.5);
    // The arrow's head, pointing on round the ring the way it runs.
    let way = if m.to >= m.from { 1.0 } else { -1.0 };
    let tip = at(a1, r);
    let (tx, ty) = (-a1.sin() * way, a1.cos() * way);
    let (nx, ny) = (a1.cos(), a1.sin());
    let back = (tip.0 - tx * 6.0, tip.1 - ty * 6.0);
    arc.segment(tip, (back.0 + nx * 3.5, back.1 + ny * 3.5), 1.5);
    arc.segment(tip, (back.0 - nx * 3.5, back.1 - ny * 3.5), 1.5);
    arc.draw(cx.canvas, t.text);
}

/// The dot on the XY pad's arc ([`paint_xy_arc`]): the second accent
/// where the setting really is now, hollow while a recorded move plays it.
fn paint_xy_dot(cx: &mut PaintCx, kx: f32, ky: f32, r: f32, m: XyMark) {
    let t = cx.theme.clone();
    let a = ring_angle(m.now);
    let (dx, dy) = (kx + r * a.cos(), ky + r * a.sin());
    // A rim in the background's colour: it reads over a badge it passes.
    let mut rim = Mask::new(xy_box(kx, ky, r));
    rim.disc(dx, dy, 5.5);
    rim.draw(cx.canvas, t.bg);
    let mut dot = Mask::new(xy_box(kx, ky, r));
    if m.recorded {
        dot.arc(dx, dy, 3.0, 0.0, 2.0 * PI, 1.5);
    } else {
        dot.disc(dx, dy, 4.0);
    }
    dot.draw(cx.canvas, t.accent2);
}

/// What the Modulate row's XY chips say.
const XY_CHIPS: [&str; 2] = ["XY →", "XY ↑"];

/// The XY pad moves settings with a range: not a switch or a meter.
fn focused_takes_xy(st: &ChainState) -> bool {
    st.focused()
        .is_some_and(|p| !p.info.readonly && p.info.curve != Curve::Toggle)
}

fn depth_of(bar: Rect, x: i32) -> f32 {
    let t = (x - bar.x) as f32 / bar.w.max(1) as f32;
    ((t * 2.0 - 1.0).clamp(-1.0, 1.0) * 100.0).round() / 100.0
}

impl Widget for ChainView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let st = &self.state;
        self.last.set(r);
        let l = self.layout(r);
        cx.canvas.fill_rect(r, t.bg);
        self.paint_chain(cx, &l);
        if st.picking {
            paint_picker(cx, &l, st, &self.picker);
        } else if let (Some(ns), Some(pl)) = (&st.notes, &l.notes_page) {
            // A row of the Notes group (or its picker) has the column.
            notes::paint_header(cx, &t, l.header, ns);
            notes::paint_page(cx, &t, notes_area(l.page, l.modulate, ns), pl, ns);
            if self.notes_modulate() {
                self.paint_modulate(cx, &l);
            } else if !ns.picking {
                notes::paint_modulate(cx, &t, l.modulate, ns);
            }
        } else {
            self.paint_effect(cx, &l);
        }
        if let Some(list) = &st.list {
            self.paint_list(cx, r, list);
        }
    }

    /// Passes the touch on to the app (an event) whenever it made actions,
    /// as the mixer does: `Ui::touch` drops a widget's touch that answers
    /// None, and the app takes the actions only when it gets the event.
    fn targets(&self, r: Rect) -> Vec<Target> {
        let l = self.layout(r);
        let st = &self.state;
        let mut out = Vec::new();
        if let Some(list) = &st.list {
            // The strip list takes the editor while it is open.
            let ll = self.list_geom(r);
            if let Some(b) = ll.off {
                let name = match st.sidechain.as_ref().map(|s| s.listens.row) {
                    Some(mpfree_fx::sidechain::Row::Source) => "Tempo",
                    _ => "Off",
                };
                out.push(Target::tap("sidechain-off", 0, name, b));
            }
            for (i, (b, name)) in ll.groups.iter().zip(STRIP_GROUPS).enumerate() {
                if b.is_empty() {
                    continue;
                }
                out.push(Target::tap("strips-group", i, name, *b).disabled_if(i == list.group));
            }
            let (n, page) = (list.strips.len(), ll.tiles.len().max(1));
            if n > page {
                out.push(
                    Target::tap("strips-prev", 0, "<", ll.prev).disabled_if(self.list_first == 0),
                );
                out.push(
                    Target::tap("strips-next", 0, ">", ll.next)
                        .disabled_if(self.list_first + page >= n),
                );
            }
            for (i, tile) in self.list_tiles(&ll) {
                out.push(
                    Target::tap("strips-pick", i, list.strips[i].name.clone(), tile)
                        .disabled_if(list.strips[i].refused.is_some()),
                );
            }
            out.push(Target::tap("strips-back", 0, "‹ Back", ll.back));
            return out;
        }
        if let Some(b) = l.back {
            out.push(Target::tap("back", 0, "‹ Back", b));
        }
        if let Some(b) = l.strip {
            out.push(Target::tap("strip", 0, st.strip_name.clone(), b));
        }
        for (i, sr) in l.scopes.iter().enumerate() {
            let label = st.scopes.get(i).cloned().unwrap_or_default();
            out.push(Target::tap("scope", i, label, *sr));
        }
        if st.picking {
            if let Some(c) = l.cancel {
                out.push(Target::tap("cancel", 0, "Cancel", c));
            }
            if let Some(pl) = &l.picker {
                picker_targets(&self.picker, st, pl, &mut out);
            }
        }
        if let (Some(ns), Some(nl)) = (&st.notes, &l.notes) {
            let n = ns.items().len();
            if let Some((up, down)) = nl.arrows {
                out.push(Target::tap("notes-up", 0, "Notes ▲", up).disabled_if(nl.from == 0));
                out.push(
                    Target::tap("notes-down", 0, "Notes ▼", down)
                        .disabled_if(nl.from + nl.fit >= n),
                );
            }
            for &(r, row) in &nl.rows {
                let Some((name, _, _)) = ns.row_view(row) else {
                    continue;
                };
                let is_fx = matches!(ns.item(row), Some(mpfree_notefx::Item::Fx(_)));
                if is_fx {
                    let (power, rest) = r.take_right(POWER_W);
                    out.push(Target::tap("note", row, name.clone(), rest));
                    out.push(Target::tap("note-on", row, format!("{name} ON"), power));
                } else {
                    out.push(Target::tap("note", row, name, r));
                }
            }
            if let Some(a) = nl.add {
                out.push(Target::tap("notes-add", 0, "+ MIDI effect", a));
            }
            out.push(Target::tap("plays-on", 0, "Plays on", nl.plays));
            if let Some((up, down)) = l.sound_arrows {
                let first = shown_row(&l, false).unwrap_or(0);
                out.push(Target::tap("sound-up", 0, "Sound ▲", up).disabled_if(first == 0));
                out.push(
                    Target::tap("sound-down", 0, "Sound ▼", down)
                        .disabled_if(first + l.sound_fit >= st.slots.len()),
                );
            }
        }
        for (i, rr) in l.rows.iter().enumerate() {
            if rr.is_empty() {
                continue;
            }
            let name = st.slots.get(i).map_or(String::new(), |v| v.name.clone());
            let (power, rest) = rr.take_right(POWER_W);
            out.push(
                Target::tap("slot", i, name.clone(), rest)
                    .moves_if(st.selected == Some(i) && !st.picking),
            );
            out.push(Target::tap("slot-on", i, format!("{name} ON"), power));
        }
        if let Some(a) = l.add {
            out.push(Target::tap("add", 0, "+ Add effect", a));
        }
        if let Some(sel) = st.selected {
            let n = st.slots.len();
            let words = ["Move up", "Move down", "Remove"];
            for (k, b) in l.buttons.iter().enumerate() {
                let off = (k == 0 && sel == 0) || (k == 1 && sel + 1 >= n);
                out.push(
                    Target::tap(["move-up", "move-down", "remove"][k], 0, words[k], *b)
                        .disabled_if(off),
                );
            }
        }
        if let Some((row, n, fx)) = notes_selected(st) {
            let words = ["Move up", "Move down", "Remove"];
            for (k, b) in l.buttons.iter().enumerate() {
                let off = [row == 0, row + 1 >= n, !fx][k];
                out.push(
                    Target::tap(["move-up", "move-down", "remove"][k], 0, words[k], *b)
                        .disabled_if(off),
                );
            }
        }
        if let (Some(ns), Some(pl)) = (&st.notes, &l.notes_page) {
            notes::page_targets(pl, ns, &mut out);
            return out;
        }
        if st.picking || st.selected.is_none() {
            return out;
        }
        // What is open over the page answers alone (a touch anywhere else
        // only shuts it): the presets or the mode list and its chip, or
        // the matrix rows and the Modulate row under them.
        if l.preset_list.is_some() {
            out.clear();
            if let Some(chip) = l.preset_chip {
                out.push(Target::tap(
                    "presets-open",
                    0,
                    format!("{} presets", effect_name(st)),
                    chip,
                ));
            }
            for &(r, k) in &l.preset_rows {
                let name = st.presets.get(k).map_or("", |p| p.name);
                out.push(Target::tap("preset-load", k, name, r).disabled_if(st.preset == Some(k)));
            }
            return out;
        }
        if let (Some(_), Some(mv)) = (l.mode_list, &st.mode) {
            out.clear();
            if let Some(chip) = l.mode_label {
                out.push(Target::tap(
                    "mode-list",
                    0,
                    format!("{} modes", mv.family),
                    chip,
                ));
            }
            for &(r, m) in &l.mode_rows {
                let name = mv.modes.get(m).copied().unwrap_or("");
                out.push(
                    Target::tap("mode-pick", m, format!("{} mode {name}", mv.family), r)
                        .disabled_if(mv.current == Some(m)),
                );
            }
            return out;
        }
        if l.rows_panel.is_some() {
            out.clear();
            self.modulate_targets(&l, &mut out);
            return out;
        }
        // The header: the presets, the page tabs, the Mode chooser.
        if let Some(chip) = l.preset_chip {
            out.push(Target::tap(
                "presets-open",
                0,
                format!("{} presets", effect_name(st)),
                chip,
            ));
        }
        for (g, tr) in l.tabs.iter().enumerate() {
            if tr.is_empty() {
                continue;
            }
            let label = st.groups.get(g).map_or("", String::as_str);
            out.push(Target::tap("group", g, label, *tr).disabled_if(g == st.group));
        }
        if let Some((prev, next)) = l.tab_arrows {
            let n = st.groups.len();
            out.push(Target::tap("group-prev", 0, "‹ pages", prev).disabled_if(st.group == 0));
            out.push(Target::tap("group-next", 0, "pages ›", next).disabled_if(st.group + 1 >= n));
        }
        if let Some(mv) = &st.mode {
            match l.mode_label {
                // A button per mode.
                None => {
                    for &(r, m) in &l.modes {
                        let name = mv.modes.get(m).copied().unwrap_or("");
                        out.push(
                            Target::tap("mode", m, format!("{} mode {name}", mv.family), r)
                                .disabled_if(mv.current == Some(m)),
                        );
                    }
                }
                // ‹ and › round the chip.
                Some(chip) => {
                    for &(r, m) in &l.modes {
                        let name = mv.modes.get(m).copied().unwrap_or("");
                        let (id, label) = if r.x < chip.x {
                            ("mode-prev", format!("‹ {} mode {name}", mv.family))
                        } else {
                            ("mode-next", format!("{} mode {name} ›", mv.family))
                        };
                        out.push(Target::tap(id, 0, label, r));
                    }
                    out.push(Target::tap(
                        "mode-list",
                        0,
                        format!("{} modes", mv.family),
                        chip,
                    ));
                }
            }
        }
        if self.eq_shown() {
            self.eq.targets(l.page, &mut out);
        } else {
            out.extend(self.page.targets(l.page));
        }
        self.modulate_targets(&l, &mut out);
        out
    }

    fn steps(&self, t: &Target) -> bool {
        match t.id {
            "knob" | "fader" | "slider" | "drawbar" => self
                .state
                .params
                .get(t.index)
                .is_some_and(|p| !p.info.readonly),
            "slot" => t.index < self.state.slots.len() && !self.state.picking,
            "eq-band" => self.eq_shown() && self.eq.band_target(t.index).is_some(),
            _ => false,
        }
    }

    /// The data wheel on a knob, fader or slider: a hundredth of its
    /// travel per detent (a switch with few places moves one place), as a
    /// touch, a turn and a lift of a finger would. On a grabbed effect:
    /// one place along the chain per detent (turned up, later in the
    /// chain), as Up and Down do.
    fn step(&mut self, t: &Target, delta: i32, _r: Rect) -> Option<EventKind> {
        self.turn(t, delta, WHEEL_STEP)
    }

    /// Held with SHIFT: a knob, fader or slider moves a tenth as far. A
    /// MIDI effect's setting moves one step a detent either way.
    fn held_step(&mut self, t: &Target, delta: i32, fine: bool, _r: Rect) -> Option<EventKind> {
        if t.id == NOTE_SETTING {
            let a = notes::step_setting(self.state.notes.as_ref()?, t.index, delta)?;
            self.set_note(a);
            return Some(EventKind::Select(self.actions.len()));
        }
        self.turn(t, delta, if fine { WHEEL_STEP / 10.0 } else { WHEEL_STEP })
    }

    /// SHIFT + the wheel's push on a knob, fader or slider: the setting's
    /// own default, as a touch, a turn and a lift would set it.
    fn reset(&mut self, t: &Target, _r: Rect) -> Option<EventKind> {
        if !matches!(t.id, "knob" | "fader" | "slider" | "drawbar") {
            return None;
        }
        let p = self.state.params.get(t.index)?;
        if p.info.readonly {
            return None;
        }
        let (addr, value) = (p.addr, p.info.clamp(p.info.default));
        if value == p.value {
            return None;
        }
        if self.state.focus != Some(addr) {
            self.push(ChainAction::Focus(addr));
        }
        self.push(ChainAction::Set { addr, value });
        self.push(ChainAction::Release(addr));
        Some(EventKind::Select(self.actions.len()))
    }

    /// A finger resting on a MIDI effect's setting puts it under the wheel
    /// (its whole row: the chips, or a card's row); elsewhere, as for any
    /// widget, the knob, fader or slider under it.
    fn held(&self, x: i32, y: i32, r: Rect) -> Option<Target> {
        let l = self.layout(r);
        if let (Some(_), Some(pl)) = (&self.state.notes, &l.notes_page) {
            let (k, rect) = notes::setting_at(pl, x, y)?;
            return Some(Target::tap(NOTE_SETTING, k, self.note_setting(k)?, rect));
        }
        self.targets(r)
            .into_iter()
            .filter(|t| t.rect.contains(x, y) && t.enabled)
            .filter(|t| t.gesture == Gesture::Drag && !t.moves && self.steps(t))
            .min_by_key(|t| t.rect.w as i64 * t.rect.h as i64)
    }

    /// A grabbed effect the wheel moved is listed at its new place.
    fn focus_index(&self, t: &Target) -> usize {
        match (t.id, self.state.selected) {
            ("slot", Some(sel)) => sel,
            _ => t.index,
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let before = self.actions.len();
        let open = self.open;
        self.touch_inner(finger, phase, x, y, r);
        // Opening or shutting something over the page is a change to draw
        // too.
        (self.actions.len() > before || self.open != open)
            .then_some(EventKind::Select(self.actions.len()))
    }
}

/// A knob's step per detent of the data wheel, as a share of its travel.
const WHEEL_STEP: f32 = 0.01;

/// A MIDI effect's setting held under the data wheel ([`Widget::held`]):
/// its index is the setting's.
const NOTE_SETTING: &str = "notes-setting";

impl ChainView {
    /// The wheel on control `t` by `delta` detents, `each` of a knob's
    /// travel per detent ([`Widget::step`], [`Widget::held_step`]).
    fn turn(&mut self, t: &Target, delta: i32, each: f32) -> Option<EventKind> {
        if t.id == "eq-band" {
            let b = self.eq.band_target(t.index)?;
            let mut out = Vec::new();
            self.eq.wheel(b, delta, &mut out);
            for a in out {
                self.push(a);
            }
            return Some(EventKind::Select(self.actions.len()));
        }
        if t.id == "slot" {
            let n = self.state.slots.len();
            let from = self.state.selected.unwrap_or(t.index);
            let to = (from as i64 + delta as i64).clamp(0, n.max(1) as i64 - 1) as usize;
            if to == from || from >= n {
                return None;
            }
            self.move_slot(from, to);
            return Some(EventKind::Select(self.actions.len()));
        }
        let p = self.state.params.get(t.index)?;
        let (addr, pos, from) = (p.addr, p.pos, p.value);
        let value = (1..=100)
            .map(|n| p.info.from_unit(pos + (delta * n) as f32 * each))
            .find(|v| *v != from)?;
        if self.state.focus != Some(addr) {
            self.push(ChainAction::Focus(addr));
        }
        self.push(ChainAction::Set { addr, value });
        self.push(ChainAction::Release(addr));
        Some(EventKind::Select(self.actions.len()))
    }

    /// The Modulate row's buttons, for the control touched, and its matrix
    /// rows while they are open.
    fn modulate_targets(&self, l: &Layout, out: &mut Vec<Target>) {
        let st = &self.state;
        if st.focus.is_some() {
            for (m, mr) in l.macros.iter().enumerate() {
                out.push(Target::tap(
                    "macro",
                    m,
                    format!("Macro {}", m + 1),
                    macro_hit(*mr),
                ));
            }
            out.push(Target::tap("learn", 0, "Learn", l.learn));
            let xy = focused_takes_xy(st);
            for (k, r) in l.xy_chips.iter().enumerate() {
                out.push(Target::tap("xy", k, XY_CHIPS[k], *r).disabled_if(!xy));
            }
            let takes = focused_takes_rows(st);
            out.push(Target::tap("add-lfo", 0, "+ LFO", l.add_lfo).disabled_if(!takes));
            out.push(
                Target::tap("add-follower", 0, "+ Follower", l.add_follower).disabled_if(!takes),
            );
            if !st.rows.is_empty() {
                out.push(Target::tap("rows", 0, "Mod rows", l.rows_link));
            }
            for (k, (bar, xr)) in l.depth_rows.iter().enumerate() {
                out.push(
                    Target::drag(
                        "depth",
                        k,
                        format!("row {} depth", k + 1),
                        bar.take_left(SOURCE_W).1,
                    )
                    .towards(30, 0),
                );
                out.push(Target::tap(
                    "remove-row",
                    k,
                    format!("row {} ×", k + 1),
                    *xr,
                ));
            }
        }
    }

    fn touch_inner(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) {
        self.last.set(r);
        let l = self.layout(r);
        if self.list_open.is_some() {
            // The strip list takes every touch while it is open.
            if phase == Touch::Down {
                self.tap_list(r, x, y);
            }
            return;
        }
        match phase {
            Touch::Down => {
                // What is open over the page takes a touch on it; one
                // anywhere else only shuts it, but for the Modulate row's
                // buttons under the rows (a second + LFO, a macro).
                if let Some(panel) = l.preset_list {
                    if panel.contains(x, y) {
                        let hit = l.preset_rows.iter().find(|(r, _)| r.contains(x, y));
                        if let (Some(&(_, k)), Some(slot)) = (hit, self.state.selected) {
                            self.open.presets = false;
                            if let Some(p) = self.state.presets.get(k) {
                                self.push(ChainAction::Preset {
                                    slot,
                                    id: p.id,
                                    values: p.values.clone(),
                                });
                            }
                        }
                    } else {
                        // The chip shuts it too (it toggles).
                        self.open.presets = false;
                    }
                    return;
                }
                if let Some(panel) = l.mode_list {
                    if panel.contains(x, y) {
                        let hit = l.mode_rows.iter().find(|(r, _)| r.contains(x, y));
                        if let (Some(&(_, mode)), Some(slot)) = (hit, self.state.selected) {
                            self.open.modes = false;
                            let current = self.state.mode.as_ref().and_then(|m| m.current);
                            if current != Some(mode) {
                                self.push(ChainAction::SetMode { slot, mode });
                            }
                        }
                    } else {
                        self.open.modes = false;
                    }
                    return;
                }
                if let Some(panel) = l.rows_panel {
                    if panel.contains(x, y) {
                        self.tap_rows(&l, finger, x, y);
                        return;
                    }
                    if !l.modulate.contains(x, y) || l.rows_link.contains(x, y) {
                        self.open.rows = false;
                        return;
                    }
                }
                // A MIDI effect's setting touched: the Modulate row works
                // on it, as on a sound's knob.
                if l.notes_page.is_some() && l.modulate.contains(x, y) && self.notes_modulate() {
                    self.tap_modulate(&l, x, y);
                    return;
                }
                if let (Some(pl), true) = (&l.notes_page, l.effect.contains(x, y)) {
                    let pl = pl.clone();
                    self.tap_notes_page(&pl, x, y);
                    return;
                }
                let on_page =
                    !self.state.picking && self.state.selected.is_some() && l.page.contains(x, y);
                if on_page && self.eq_shown() {
                    let mut out = Vec::new();
                    self.eq.down(finger, x, y, l.page, &mut out);
                    for a in out {
                        self.push(a);
                    }
                    if self.drag.is_none() {
                        self.drag = Some((finger, Drag::Eq));
                    }
                    return;
                }
                if on_page {
                    let mut turned = None;
                    if let Some(ev) = self.page.touch(finger, Touch::Down, x, y, l.page) {
                        self.page_event(ev, &mut turned);
                    }
                    self.drag = Some((finger, Drag::Page { turned }));
                    return;
                }
                self.tap(&l, finger, x, y);
                // A finger on an effect's row (not its ON pill) may drag
                // it up or down the chain.
                if let Some(from) = self.slot_under(&l, x, y) {
                    self.drag = Some((
                        finger,
                        Drag::Slot {
                            from,
                            y0: y,
                            y,
                            to: from,
                            moved: false,
                        },
                    ));
                }
            }
            Touch::Move => {
                if matches!(self.drag, Some((_, Drag::Eq))) {
                    let mut out = Vec::new();
                    self.eq.moved(finger, x, y, l.page, &mut out);
                    for a in out {
                        self.push(a);
                    }
                    return;
                }
                let Some((f, d)) = &self.drag else {
                    return;
                };
                if *f != finger {
                    return;
                }
                match *d {
                    Drag::Slot {
                        from, y0, moved, ..
                    } => {
                        let moved = moved || (y - y0).abs() >= SLOT_SLOP;
                        let to = if moved { slot_at(&l, y) } else { from };
                        self.drag = Some((
                            finger,
                            Drag::Slot {
                                from,
                                y0,
                                y,
                                to,
                                moved,
                            },
                        ));
                    }
                    Drag::Page { mut turned } => {
                        if let Some(ev) = self.page.touch(finger, Touch::Move, x, y, l.page) {
                            self.page_event(ev, &mut turned);
                        }
                        self.drag = Some((finger, Drag::Page { turned }));
                    }
                    Drag::Eq => {}
                    Drag::Depth { k } => {
                        let Some(&(bar, _)) = l.depth_rows.get(k) else {
                            return;
                        };
                        let depth = depth_of(bar.take_left(SOURCE_W).1.inset_xy(4, 0), x);
                        let Some(row) = self.state.rows.get(k) else {
                            return;
                        };
                        if row.depth != depth {
                            let row = row.index;
                            self.push(ChainAction::Depth { row, depth });
                        }
                    }
                }
            }
            Touch::Up | Touch::Cancel => {
                if let (Touch::Up, Some(pl)) = (phase, &l.picker) {
                    let pl = pl.clone();
                    self.lift_picker(&pl, finger, x, y);
                } else if self.picker.press.is_some_and(|(f, ..)| f == finger) {
                    self.picker.press = None;
                }
                if matches!(self.drag, Some((_, Drag::Eq))) {
                    let mut out = Vec::new();
                    self.eq
                        .up(finger, x, y, l.page, phase == Touch::Cancel, &mut out);
                    for a in out {
                        self.push(a);
                    }
                    if self.drag.as_ref().is_some_and(|(f, _)| *f == finger) {
                        self.drag = None;
                    }
                    return;
                }
                if !self.drag.as_ref().is_some_and(|(f, _)| *f == finger) {
                    return;
                }
                match self.drag.take() {
                    Some((_, Drag::Page { mut turned })) => {
                        let ev = self.page.touch(finger, phase, x, y, l.page);
                        if phase == Touch::Up
                            && turned.is_none()
                            && self.on_sidechain_button(&l, x, y)
                        {
                            // The track's button: the sidechain list, in
                            // place of the step the row would take.
                            self.open_sidechain();
                        } else if let Some(ev) = ev {
                            self.page_event(ev, &mut turned);
                        }
                        if let Some(addr) = turned {
                            self.push(ChainAction::Release(addr));
                        }
                    }
                    // Lifted somewhere else in the chain: it moves there (a
                    // cancelled touch leaves it).
                    Some((
                        _,
                        Drag::Slot {
                            from,
                            to,
                            moved: true,
                            ..
                        },
                    )) if phase == Touch::Up && to != from => self.move_slot(from, to),
                    _ => {}
                }
            }
        }
    }

    /// A finger down on the mod matrix rows: a row's bar starts setting
    /// its depth, its x removes it.
    fn tap_rows(&mut self, l: &Layout, finger: u64, x: i32, y: i32) {
        for (k, (bar, xr)) in l.depth_rows.iter().enumerate() {
            let Some(row) = self.state.rows.get(k).map(|r| r.index) else {
                continue;
            };
            if xr.contains(x, y) {
                self.push(ChainAction::RemoveRow(row));
                return;
            }
            let track = bar.take_left(SOURCE_W).1;
            if track.contains(x, y) {
                self.drag = Some((finger, Drag::Depth { k }));
                let depth = depth_of(track.inset_xy(4, 0), x);
                self.push(ChainAction::Depth { row, depth });
                return;
            }
        }
    }

    /// The chain column: the head card, then the list's card.
    fn paint_chain(&self, cx: &mut PaintCx, l: &Layout) {
        let t = cx.theme.clone();
        let st = &self.state;
        cx.canvas.fill_round_rect(l.head, t.radius, t.surface);
        if let Some(b) = l.back {
            cx.canvas.fill_round_rect(b, 10, t.raised);
            cx.text(FontId::Bold, t.size_title, b, Align::Center, t.text, "‹");
        }
        if let Some(b) = l.strip {
            paint_strip_button(cx, b, st);
        }
        for (i, (r, name)) in l.scopes.iter().zip(&st.scopes).enumerate() {
            let on = i == st.scope;
            cx.canvas.fill_round_rect(
                *r,
                t.round(10),
                if on {
                    t.raised
                } else {
                    t.surface.mix(t.raised, 0.35)
                },
            );
            if on {
                cx.canvas.fill_round_rect(
                    Rect::new(r.x + 12, r.bottom() - 3, r.w - 24, 2),
                    1,
                    t.accent,
                );
            }
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_small - 1.0,
                r.inset_xy(4, 0),
                Align::Center,
                if on { t.text } else { t.text_dim },
                name,
            );
        }
        if st.scopes.is_empty() && l.strip.is_none() {
            let x = l.back.map_or(l.head.x + 14, |b| b.right() + 10);
            let title = Rect::new(x, l.head.y, l.head.right() - 12 - x, l.head.h);
            cx.text(
                FontId::Bold,
                t.size_small,
                title,
                Align::Left,
                t.text_dim,
                &st.title.to_uppercase(),
            );
        }
        cx.canvas.fill_round_rect(l.list, t.radius, t.surface);
        let count = match st.cost_pct {
            Some(c) => format!("{} of {} · {c:.1} %", st.slots.len(), st.room),
            None => format!("{} of {}", st.slots.len(), st.room),
        };
        match (&st.notes, &l.notes) {
            // A note track: NOTES over its rows and Plays on, then SOUND.
            (Some(ns), Some(nl)) => {
                let n = ns.items().len();
                let shown = format!("{} of {}", ns.fx.count(), mpfree_notefx::MAX_FX);
                // The Sound caption shares its line with its arrows: no cost.
                let count = format!("{} of {}", st.slots.len(), st.room);
                notes::paint_caption(
                    cx,
                    &t,
                    l.caption,
                    "NOTES",
                    &shown,
                    nl.arrows,
                    nl.from > 0,
                    nl.from + nl.fit < n,
                );
                notes::paint_list(cx, &t, nl, ns);
                let first = shown_row(l, false).unwrap_or(0);
                notes::paint_caption(
                    cx,
                    &t,
                    nl.sound_caption,
                    "SOUND",
                    &count,
                    l.sound_arrows,
                    first > 0,
                    first + l.sound_fit < st.slots.len(),
                );
            }
            _ => {
                let cap = l.caption.inset_xy(6, 0).take_top(CAPTION_H - 6).0;
                cx.text(
                    FontId::Bold,
                    t.size_small - 1.0,
                    cap,
                    Align::Left,
                    t.text_dim,
                    "EFFECTS",
                );
                cx.text(
                    FontId::Sans,
                    t.size_small - 1.0,
                    cap,
                    Align::Right,
                    t.text_dim,
                    &count,
                );
            }
        }
        // A row being dragged: its own place empty but outlined, a line
        // where it will land, and the row itself under the finger.
        let lifted = match self.drag {
            Some((
                _,
                Drag::Slot {
                    from,
                    y0,
                    y,
                    to,
                    moved: true,
                },
            )) => Some((from, y - y0, to)),
            _ => None,
        };
        for (i, (s, row)) in st.slots.iter().zip(&l.rows).enumerate() {
            if row.is_empty() {
                continue;
            }
            if lifted.is_some_and(|(from, ..)| from == i) {
                cx.canvas.fill_round_rect(*row, 10, t.surface);
                cx.canvas.stroke_round_rect(*row, 10, 2, t.outline);
                continue;
            }
            paint_slot(cx, &t, *row, i, s, st.selected == Some(i));
        }
        if let (Some((from, dy, to)), Some(first), Some(last)) = (
            lifted,
            shown_row(l, false).map(|i| l.rows[i]),
            shown_row(l, true).map(|i| l.rows[i]),
        ) {
            // The row follows the finger within the list, but stays clear
            // of the landing line: below it on the way up, above it on the
            // way down, so the line is never hidden under the row.
            let (mut lo, mut hi) = (first.y, last.y);
            if to != from {
                let at = &l.rows[to];
                let y = if to < from {
                    at.y - ROW_GAP / 2 - 2
                } else {
                    at.bottom() + ROW_GAP / 2 - 2
                };
                cx.canvas
                    .fill_round_rect(Rect::new(at.x, y, at.w, 4), 2, t.accent);
                for x in [at.x - 6, at.right() - 6] {
                    cx.canvas
                        .fill_round_rect(Rect::new(x, y - 4, 12, 12), 6, t.accent);
                }
                if to < from {
                    lo = y + 14;
                } else {
                    hi = y - 10 - at.h;
                }
            }
            if let (Some(s), Some(home)) = (st.slots.get(from), l.rows.get(from)) {
                let y = (home.y + dy).clamp(lo, hi.max(lo));
                let row = Rect::new(home.x + 6, y, home.w, home.h);
                paint_slot(cx, &t, row, from, s, true);
                cx.canvas.stroke_round_rect(row, 10, 2, t.accent);
            }
        }
        if let Some(a) = l.add {
            cx.canvas.stroke_round_rect(a, 10, 1, t.outline);
            cx.text(
                FontId::Bold,
                t.size_small,
                a,
                Align::Center,
                t.text,
                "+  Add effect",
            );
        }
        let live = st.selected.is_some();
        let sel = st.selected.unwrap_or(0);
        let mut usable = [live && sel > 0, live && sel + 1 < st.slots.len(), live];
        // A row of the Notes group: Up, Down and Remove work on it (the
        // arp moves but stays).
        if let Some((row, n, fx)) = notes_selected(st) {
            usable = [row > 0, row + 1 < n, fx];
        }
        for (b, (label, ok)) in l.buttons.iter().zip([
            ("Up", usable[0]),
            ("Down", usable[1]),
            ("Remove", usable[2]),
        ]) {
            cx.canvas.fill_round_rect(
                *b,
                t.round(10),
                if ok {
                    t.raised
                } else {
                    t.surface.mix(t.raised, 0.3)
                },
            );
            cx.text(
                FontId::Bold,
                t.size_small,
                *b,
                Align::Center,
                t.button_ink(ok),
                label,
            );
        }
    }

    /// The job of the section that holds the touched control (the
    /// Modulate row's dot), else of the page's first section (the lit
    /// mode).
    fn job_of(&self, focus: bool) -> Option<Job> {
        let st = &self.state;
        let k = st
            .focus
            .and_then(|f| st.params.iter().position(|p| p.addr == f));
        match (focus, k) {
            (true, Some(k)) => st
                .sections
                .iter()
                .find(|s| s.controls.iter().chain(&s.switch).any(|c| c.param == k))
                .map(|s| s.job),
            (true, None) => None,
            (false, _) => st.sections.first().map(|s| s.job),
        }
    }

    /// The effect column: the header, the page (with its macro badges and
    /// modulation rings), the Modulate row, and what is open over the page.
    fn paint_effect(&self, cx: &mut PaintCx, l: &Layout) {
        let t = cx.theme.clone();
        let st = &self.state;
        if st.selected.and_then(|s| st.slots.get(s)).is_none() {
            cx.canvas.fill_round_rect(l.effect, t.radius, t.surface);
            cx.text(
                FontId::Sans,
                t.size_body,
                l.effect,
                Align::Center,
                t.text_dim,
                if st.slots.is_empty() {
                    "No effects yet: tap  + Add effect"
                } else {
                    "Tap an effect to open its page, or  + Add effect"
                },
            );
            return;
        }
        cx.canvas.fill_round_rect(l.header, t.radius, t.surface);
        match l.preset_chip {
            // The presets button: the preset's name (or the effect's) and
            // a chevron; after a preset's name, the effect's, dimmer.
            Some(chip) => {
                if l.preset_list.is_some() {
                    cx.canvas.fill_round_rect(chip, 10, t.raised);
                }
                let label = header_label(st);
                let room = Rect::new(
                    chip.x + PRESET_PAD,
                    l.title.y,
                    chip.w - PRESET_PAD - 26,
                    l.title.h,
                );
                let w = cx.measure(FontId::Bold, t.size_title, label).min(room.w);
                cx.text(FontId::Bold, t.size_title, room, Align::Left, t.text, label);
                chevron(cx, room.x + w + 8, chip.y + chip.h / 2 - 3, t.text_dim);
                if header_label(st) != effect_name(st) {
                    let x = room.x + w + 30;
                    let after = Rect::new(x, l.title.y, (l.title.right() - x).max(0), l.title.h);
                    cx.text(
                        FontId::Sans,
                        t.size_small,
                        after,
                        Align::Left,
                        t.text_dim,
                        effect_name(st),
                    );
                }
            }
            None => {
                cx.text(
                    FontId::Bold,
                    t.size_title,
                    l.title,
                    Align::Left,
                    t.text,
                    effect_name(st),
                );
            }
        }
        let lit = self.job_of(false).map_or(t.accent, |j| j.fill(&t));
        if let Some(mv) = &st.mode {
            paint_modes(cx, l, mv, lit);
        }
        if let Some(g) = l.tab_group {
            cx.canvas.fill_round_rect(g, 10, t.bg);
            for (k, (tr, name)) in l.tabs.iter().zip(&st.groups).enumerate() {
                if tr.is_empty() {
                    continue;
                }
                let on = k == st.group;
                if on {
                    cx.canvas.fill_round_rect(*tr, 8, t.raised);
                }
                cx.text(
                    if on { FontId::Bold } else { FontId::Sans },
                    t.size_small + 1.0,
                    *tr,
                    Align::Center,
                    if on { t.text } else { t.text_dim },
                    name,
                );
            }
            if let Some((prev, next)) = l.tab_arrows {
                let ends = [st.group == 0, st.group + 1 >= st.groups.len()];
                for ((r, glyph), end) in [(prev, "‹"), (next, "›")].into_iter().zip(ends) {
                    cx.text(
                        FontId::Bold,
                        t.size_title,
                        r,
                        Align::Center,
                        if end { t.text_faint } else { t.text },
                        glyph,
                    );
                }
            }
        }
        if self.eq_shown() {
            self.eq.paint(cx, l.page);
        } else {
            self.page.paint(cx, l.page);
            self.paint_badges(cx, l.page);
        }
        self.paint_modulate(cx, l);
        if let Some(panel) = l.rows_panel {
            self.paint_rows(cx, l, panel);
        }
        if let (Some(panel), Some(mv)) = (l.mode_list, &st.mode) {
            cx.canvas.fill_round_rect(panel, t.radius, t.surface);
            cx.canvas.stroke_round_rect(panel, t.radius, 1, t.outline);
            for &(r, m) in &l.mode_rows {
                let on = mv.current == Some(m);
                cx.canvas
                    .fill_round_rect(r, 8, if on { lit } else { t.raised });
                cx.text(
                    if on { FontId::Bold } else { FontId::Sans },
                    t.size_small + 1.0,
                    r.inset_xy(14, 0),
                    Align::Left,
                    if on { t.on_accent } else { t.text },
                    mv.modes[m],
                );
                cx.text(
                    FontId::Sans,
                    t.size_small - 1.0,
                    r.inset_xy(14, 0),
                    Align::Right,
                    if on { t.on_accent } else { t.text_dim },
                    &format!("{} of {}", m + 1, mv.modes.len()),
                );
            }
        }
        if let Some(panel) = l.preset_list {
            cx.canvas.fill_round_rect(panel, t.radius, t.surface);
            cx.canvas.stroke_round_rect(panel, t.radius, 1, t.outline);
            for &(r, k) in &l.preset_rows {
                let on = st.preset == Some(k);
                cx.canvas
                    .fill_round_rect(r, 8, if on { lit } else { t.raised });
                cx.text(
                    if on { FontId::Bold } else { FontId::Sans },
                    t.size_small + 1.0,
                    r.inset_xy(14, 0),
                    Align::Left,
                    if on { t.on_accent } else { t.text },
                    st.presets[k].name,
                );
            }
        }
    }

    /// Over the page's controls: a ring round a knob for how far the
    /// matrix moves it, a badge in its macro's colour ("M2") at the knob's
    /// top left (its Q-Link's is at its top right), the XY pad's mark (X→
    /// or Y↑ under it, the stretch the pad covers round the ring), and LK
    /// for a lock.
    fn paint_badges(&self, cx: &mut PaintCx, page: Rect) {
        let t = cx.theme.clone();
        for (k, p) in self.state.params.iter().enumerate() {
            if p.mod_range.is_none() && p.macro_.is_none() && !p.locked && p.xy.is_none() {
                continue;
            }
            let Some((_, g)) = self.page.control_geom(page, k) else {
                continue;
            };
            if g.cell.w <= 0 {
                continue;
            }
            let (bx, by) = match g.dial {
                Some((kx, ky, d)) => {
                    if let Some((lo, hi)) = p.mod_range {
                        let (a0, a1) = (0.75 * PI + 1.5 * PI * lo, 0.75 * PI + 1.5 * PI * hi);
                        let bbox = Rect::new(
                            (kx - d) as i32,
                            (ky - d) as i32,
                            (2.0 * d) as i32,
                            (2.0 * d) as i32,
                        );
                        let mut m = Mask::new(bbox);
                        m.arc(kx, ky, d * 0.5 + 5.0, a0, a1.max(a0 + 0.02), 3.0);
                        m.draw(cx.canvas, t.good);
                    }
                    if let Some(m) = p.xy {
                        paint_xy_arc(cx, kx, ky, xy_radius(d, p), m);
                    }
                    // Beside a small dial with its name at its right,
                    // the Q-Link's badge sits over it: this one goes
                    // under it.
                    let y = if g.inline {
                        ky + d * 0.5 - 14.0
                    } else {
                        ky - d * 0.5 - 2.0
                    };
                    ((kx - d * 0.5) as i32 - 14, y as i32)
                }
                None => (g.cell.x - 2, g.cell.y - 10),
            };
            let mut y = by;
            if let Some(m) = p.macro_ {
                let b = Rect::new(bx, y, 30, 18);
                cx.canvas.fill_round_rect(b, 6, macro_color(&t, m));
                cx.text(
                    FontId::Bold,
                    12.0,
                    b,
                    Align::Center,
                    t.on_accent,
                    &format!("M{}", m + 1),
                );
                y += 22;
            }
            if let Some(m) = p.xy {
                let b = Rect::new(bx, y, 30, 18);
                cx.canvas.fill_round_rect(b, 6, t.text);
                cx.text(
                    FontId::Bold,
                    12.0,
                    b,
                    Align::Center,
                    t.ink_on(t.text),
                    if m.up { "Y↑" } else { "X→" },
                );
                y += 22;
            }
            if p.locked {
                let b = Rect::new(bx, y, 30, 18);
                cx.canvas.fill_round_rect(b, 6, t.warn);
                cx.text(FontId::Bold, 12.0, b, Align::Center, t.on_accent, "LK");
            }
            // The dot last: over the badges it may pass under.
            if let (Some(m), Some((kx, ky, d))) = (p.xy, g.dial) {
                paint_xy_dot(cx, kx, ky, xy_radius(d, p), m);
            }
        }
    }

    /// The Modulate row: the touched control's name (a dot in its
    /// section's colour) over a link to its rows, macros 1 to 8, Learn,
    /// + LFO and + Follower.
    fn paint_modulate(&self, cx: &mut PaintCx, l: &Layout) {
        let t = cx.theme.clone();
        let st = &self.state;
        cx.canvas.fill_round_rect(l.modulate, t.radius, t.surface);
        let Some(p) = st.focused() else {
            cx.text(
                FontId::Sans,
                t.size_small,
                l.modulate.inset_xy(18, 0),
                Align::Left,
                t.text_dim,
                "MODULATE  ·  touch a control, then a macro number, Learn, + LFO or + Follower",
            );
            return;
        };
        let (name, link) = l.mod_name.take_top(22);
        let dot = self.job_of(true).map_or(t.accent, |j| j.fill(&t));
        cx.canvas
            .fill_round_rect(Rect::new(name.x, name.y + 7, 8, 8), 4, dot);
        cx.text(
            FontId::Bold,
            t.size_small + 1.0,
            Rect::new(name.x + 14, name.y, name.w - 14, name.h),
            Align::Left,
            t.text,
            &p.label,
        );
        let takes = focused_takes_rows(st);
        let summary = match (takes, st.rows.first()) {
            _ if st.learning => "learning: Q-Link or MIDI".to_string(),
            (false, _) => "a switch: macros only".to_string(),
            (true, None) => "no LFO or follower".to_string(),
            (true, Some(r)) => {
                let more = match st.rows.len() {
                    1 => String::new(),
                    n => format!(" +{}", n - 1),
                };
                format!("{} · {:+.0} %{more}  ›", r.source, r.depth * 100.0)
            }
        };
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            link,
            Align::Left,
            if st.learning {
                t.accent
            } else if st.rows.is_empty() {
                t.text_faint
            } else if self.open.rows {
                t.accent
            } else {
                t.text_dim
            },
            &summary,
        );
        // XY → and XY ↑: lit while the control is on that direction (the
        // text colour, as its badge), greyed on a switch.
        let movable = focused_takes_xy(st);
        let way = p.xy.map(|m| m.up);
        for (k, r) in l.xy_chips.iter().enumerate() {
            let lit = way == Some(k == 1);
            let fill = if lit {
                t.text
            } else if movable {
                t.raised
            } else {
                t.surface.mix(t.raised, 0.3)
            };
            cx.canvas.fill_round_rect(*r, t.round(10), fill);
            let ink = if lit {
                t.ink_on(t.text)
            } else {
                t.button_ink(movable)
            };
            cx.text(
                FontId::Bold,
                t.size_small,
                *r,
                Align::Center,
                ink,
                XY_CHIPS[k],
            );
        }
        for (m, mr) in l.macros.iter().enumerate() {
            let on = p.macro_ == Some(m);
            let c = macro_color(&t, m);
            cx.canvas
                .fill_round_rect(*mr, 10, if on { c } else { t.raised });
            if !on {
                cx.canvas
                    .fill_round_rect(Rect::new(mr.x + 5, mr.y + 5, 6, 6), 3, c);
            }
            cx.text(
                FontId::Bold,
                t.size_body - 1.0,
                *mr,
                Align::Center,
                if on { t.on_accent } else { t.text },
                &(m + 1).to_string(),
            );
        }
        cx.canvas
            .fill_round_rect(l.learn, 10, if st.learning { t.picked() } else { t.raised });
        cx.text(
            FontId::Bold,
            t.size_small,
            l.learn,
            Align::Center,
            if st.learning { t.on_picked() } else { t.text },
            if st.learning {
                "Turn a Q-Link"
            } else {
                "Learn"
            },
        );
        for (b, label) in [(l.add_lfo, "+ LFO"), (l.add_follower, "+ Follower")] {
            cx.canvas.fill_round_rect(
                b,
                t.round(10),
                if takes {
                    t.raised
                } else {
                    t.surface.mix(t.raised, 0.3)
                },
            );
            cx.text(
                FontId::Bold,
                t.size_small,
                b,
                Align::Center,
                t.button_ink(takes),
                label,
            );
        }
    }

    /// The touched control's mod matrix rows over the page: each row's
    /// source, its depth bar (drag it) and its x.
    fn paint_rows(&self, cx: &mut PaintCx, l: &Layout, panel: Rect) {
        let t = cx.theme.clone();
        let st = &self.state;
        cx.canvas.fill_round_rect(panel, t.radius, t.surface);
        cx.canvas.stroke_round_rect(panel, t.radius, 1, t.outline);
        let label = st
            .focused()
            .map_or(String::new(), |p| p.label.to_uppercase());
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            Rect::new(panel.x + 14, panel.y + 8, panel.w - 28, 28),
            Align::Left,
            t.text_dim,
            &format!("MOD MATRIX · {label}"),
        );
        if st.rows.is_empty() {
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(panel.x + 14, panel.y + 44, panel.w - 28, DEPTH_ROW_H),
                Align::Left,
                t.text_dim,
                "No rows yet: + LFO or + Follower adds one",
            );
        }
        for (rv, (bar, xr)) in st.rows.iter().zip(&l.depth_rows) {
            let (name, track) = bar.take_left(SOURCE_W);
            cx.text(
                FontId::Sans,
                t.size_small,
                name,
                Align::Left,
                t.text,
                &rv.source,
            );
            let track = track.inset_xy(4, 14);
            cx.canvas.fill_round_rect(track, 4, t.raised);
            let mid = track.x + track.w / 2;
            let end = mid + (rv.depth * track.w as f32 * 0.5) as i32;
            let (a, b) = if end < mid { (end, mid) } else { (mid, end) };
            cx.canvas
                .fill_round_rect(Rect::new(a, track.y, (b - a).max(2), track.h), 4, t.good);
            cx.canvas
                .fill_rect(Rect::new(mid, track.y - 3, 1, track.h + 6), t.text_dim);
            cx.text(
                FontId::Mono,
                t.size_small - 2.0,
                Rect::new(track.x, track.bottom() + 1, track.w, 14),
                Align::Center,
                t.text_dim,
                &format!("{:+.0} %", rv.depth * 100.0),
            );
            cx.text(
                FontId::Bold,
                t.size_body,
                *xr,
                Align::Center,
                t.text_faint,
                "x",
            );
        }
    }
}

/// The effect picker, over the effect column.
fn paint_picker(cx: &mut PaintCx, l: &Layout, st: &ChainState, pv: &PickerView) {
    let t = cx.theme.clone();
    let area = l.effect;
    cx.canvas.fill_round_rect(area, t.radius, t.surface);
    let (head, _) = area.inset(14).take_top(52);
    // Where the effect goes (Jay's Mix tab card, 14 Nov): the strip, and
    // the slot it lands in (the end of the chain).
    let strip = if st.strip_name.is_empty() {
        st.title.trim_end_matches(" FX").to_string()
    } else {
        st.strip_name.clone()
    }
    .to_uppercase();
    let slot = st.slots.len() + 1;
    let title = match (pv.presets, slot > st.room) {
        (_, true) => format!(
            "{strip} IS FULL · {} OF {} SLOTS USED",
            st.slots.len(),
            st.room
        ),
        (Some(_), _) => format!("PICK A PRESET OR DEFAULTS · IT GOES IN SLOT {slot} OF {strip}"),
        _ => format!("ADD TO {strip} · IT GOES IN SLOT {slot} OF {}", st.room),
    };
    cx.text(
        FontId::Bold,
        t.size_small,
        head.take_top(42).0,
        Align::Left,
        t.text_dim,
        &title,
    );
    if let Some(c) = l.cancel {
        cx.canvas.fill_round_rect(c, 9, t.raised);
        cx.text(
            FontId::Bold,
            t.size_small,
            c,
            Align::Center,
            t.text,
            "Cancel",
        );
    }
    match &l.picker {
        Some(pl) => crate::picker::paint(cx, pl, st, pv),
        None => {
            cx.text(
                FontId::Sans,
                t.size_body,
                area,
                Align::Center,
                t.text_dim,
                "This chain is full: remove an effect first",
            );
        }
    }
}

/// The Mode chooser in the header: its group, then a button per mode with
/// the current one lit in the page's colour (`lit`), or the arrows round a
/// chip with the mode's name, its place and a chevron ("Tube   3 of
/// 10"; "Drive mode" when in none).
fn paint_modes(cx: &mut PaintCx, l: &Layout, mv: &crate::state::ModeView, lit: mpfree_ui::Color) {
    let t = cx.theme.clone();
    if let Some(g) = l.mode_group {
        cx.canvas.fill_round_rect(g, 10, t.bg);
    }
    match l.mode_label {
        None => {
            for &(r, m) in &l.modes {
                let on = mv.current == Some(m);
                if on {
                    cx.canvas.fill_round_rect(r, 8, lit);
                }
                cx.text(
                    if on { FontId::Bold } else { FontId::Sans },
                    t.size_small + 1.0,
                    r,
                    Align::Center,
                    if on { t.on_accent } else { t.text_dim },
                    mv.modes[m],
                );
            }
        }
        Some(label) => {
            let prev = Rect::new(label.x - 4 - ARROW_W, label.y, ARROW_W, label.h);
            let next = Rect::new(label.right() + 4, label.y, ARROW_W, label.h);
            for (r, glyph) in [(prev, "‹"), (next, "›")] {
                let live = l.modes.iter().any(|(m, _)| *m == r);
                cx.text(
                    FontId::Bold,
                    t.size_title,
                    r,
                    Align::Center,
                    if live { t.text } else { t.text_faint },
                    glyph,
                );
            }
            cx.canvas.fill_round_rect(label, 8, lit);
            chevron(
                cx,
                label.right() - 23,
                label.y + label.h / 2 - 3,
                t.on_accent,
            );
            let inner = Rect::new(label.x + 12, label.y, label.w - 12 - 32, label.h);
            match mv.current {
                Some(c) => {
                    cx.text(
                        FontId::Bold,
                        t.size_small + 1.0,
                        inner,
                        Align::Left,
                        t.on_accent,
                        mv.modes[c],
                    );
                    cx.text(
                        FontId::Sans,
                        t.size_small - 1.0,
                        inner,
                        Align::Right,
                        t.on_accent,
                        &format!("{} of {}", c + 1, mv.modes.len()),
                    );
                }
                None => {
                    cx.text(
                        FontId::Bold,
                        t.size_small,
                        inner,
                        Align::Left,
                        t.on_accent,
                        &format!("{} mode", mv.family),
                    );
                }
            }
        }
    }
}

/// `text` in lines no wider than `w` (a word wider than `w` gets a line of
/// its own).
pub(crate) fn wrap(cx: &mut PaintCx, size: f32, text: &str, w: i32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(l) if cx.measure(FontId::Sans, size, &format!("{l} {word}")) <= w => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// The effect picker's controls, as it draws them: its sections, then the
/// section's effect tiles with their stars and presets lines (or the open
/// presets list with its way back), then the pager.
fn picker_targets(pv: &PickerView, st: &ChainState, pl: &PickerLayout, out: &mut Vec<Target>) {
    let (list, _) = pv.current(st);
    for (i, r) in pl.sections.iter().enumerate() {
        let name = list.get(i).map_or("", |s| s.name());
        out.push(Target::tap("section", i, name, *r).disabled_if(i == pl.section));
    }
    match pv.presets {
        Some(id) => {
            if let Some(b) = pl.back {
                out.push(Target::tap("presets-back", 0, "‹ Effects", b));
            }
            let presets = mpfree_fx::picker::preset_rows(id);
            for (k, r) in pl.tiles.iter().enumerate() {
                let item = pl.first + k;
                let label = match item {
                    0 => "Defaults",
                    _ => presets.get(item - 1).map_or("", |p| p.name),
                };
                out.push(Target::tap("preset", item, label, *r));
            }
        }
        None => {
            let picks = pv.picks(st);
            for (k, r) in pl.tiles.iter().enumerate() {
                let i = pl.first + k;
                let name = picks.get(i).map_or("", |p| p.name);
                out.push(Target::tap("pick", i, name, *r));
                if let Some(star) = pl.stars.get(k) {
                    out.push(Target::tap("star", i, format!("{name} favourite"), *star));
                }
                if let Some(Some(chip)) = pl.chips.get(k) {
                    out.push(Target::tap("presets", i, format!("{name} presets"), *chip));
                }
            }
        }
    }
    if let Some(r) = pl.prev {
        out.push(Target::tap("page-prev", 0, "‹", r).disabled_if(pl.page == 0));
    }
    if let Some(r) = pl.next {
        out.push(Target::tap("page-next", 0, "›", r).disabled_if(pl.page + 1 >= pl.pages));
    }
}
