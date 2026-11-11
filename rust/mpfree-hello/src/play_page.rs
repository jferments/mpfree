//! The Play page (layout spec, "A new Play page"; Jay 09:34Z to 10:16Z): the
//! home for finger drumming, live looping and making beats. The big pads
//! are gone, because the MPC has them; the screen shows what they can't.
//!
//! The frame is the same in every view:
//! - the time engine's transport bar, with the view switch (All, Beats,
//!   Loop, Drum) on its right; one tap changes the view;
//! - the view under it;
//! - beside them, from under the top bar to the hint line, the thin master
//!   bar and the four Q-Link cards, next to the MPC's Q-Link knobs.
//!
//! The views, from what is built today:
//! - **All** (home): the loop strip, the kits and the pads.
//! - **Beats**: the pattern, the pads and the kits. The step grid comes
//!   with the sequencer's grid (`mpfree_seq::steps`).
//! - **Loop**: the loop big, its buttons, what it records and the pads.
//! - **Drum**: the timing of your hits, the pads and the kits.
//!
//! The pads panel has a heading line ([`PadsBar`]): what the pads play and,
//! while the synth has them, the Chromatic | In key switch (Jay, 12:22Z).
//!
//! The arp's panel goes into the same frame ([`framed`]): its column takes
//! the view's left and the pads stay on the right. NOTE REPEAT's slim strip
//! takes the top of the view's first column ([`PlayView::quick_layout`]),
//! so the pads don't move when it comes and goes.

use mpfree_arp::scale::pads::PadLayout;
use mpfree_arp_ui::view as arp_view;
use mpfree_time_ui::view::TRANSPORT;
use mpfree_ui::qlink::{COLUMN_WIDTH, MASTER_WIDTH};
use mpfree_ui::text::{Align, FontId};
use mpfree_ui::{EventKind, PaintCx, Rect, Touch, Widget};

/// Widget names on the page. `kits`, `pads`, `timing`, `lbuttons`, `lmeter`
/// and the loop strip are the app's own and the Looper tab's.
pub const SWITCH: &str = "play-views";
pub const MASTER: &str = "master";
pub const QLINKS: &str = "qlinks";
pub const BEATS: &str = "play-beats";
pub const PADS_HEAD: &str = "pads-head";
/// The same heading on the Synth tab, over its presets: always the synth.
pub const SYNTH_PADS_HEAD: &str = "synth-pads-head";

/// What the Beats view says until the step grid is in.
pub const BEATS_LINES: [&str; 3] = [
    "The step grid comes here: the pattern playing, a lane",
    "per pad, pages up to 64 steps (the sequencer's grid).",
    "Until then, clips record and play on the Clips tab.",
];

/// The views, in the switch's order.
pub const VIEWS: [&str; 4] = ["All", "Beats", "Loop", "Drum"];
/// The switch's width at the right of the transport bar.
const SWITCH_W: i32 = 330;
/// The pads panel: its heading line over the pads.
const PADS: &str = "col(pads-head:30, pads:*)";
const PADS_W: i32 = 440;
/// The loop strip's height in the All view.
const STRIP_H: i32 = 172;
/// The Loop view's bottom row: what the looper records, and the pads.
const LOOP_ROW_H: i32 = 300;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayView {
    #[default]
    All,
    Beats,
    Loop,
    Drum,
}

impl PlayView {
    pub const ALL: [PlayView; 4] = [
        PlayView::All,
        PlayView::Beats,
        PlayView::Loop,
        PlayView::Drum,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> PlayView {
        PlayView::ALL[i.min(3)]
    }

    pub fn name(self) -> &'static str {
        VIEWS[self.index()]
    }

    /// The view's page in the `Ui`. All, the home view, is "Play", as
    /// before.
    pub fn page(self) -> &'static str {
        match self {
            PlayView::All => "Play",
            PlayView::Beats => "Play · Beats",
            PlayView::Loop => "Play · Loop",
            PlayView::Drum => "Play · Drum",
        }
    }

    /// The view under the transport bar. With `band`, NOTE REPEAT's strip
    /// takes the top of the first column, so nothing else moves.
    fn body(self, band: bool) -> String {
        let first = |slot: &str| {
            if band {
                format!("col({}:{}, {slot}:*):*", arp_view::MINI, arp_view::MINI_H)
            } else {
                format!("{slot}:*")
            }
        };
        let strip = crate::looper_page::LOOP_STRIP;
        match self {
            PlayView::All => format!(
                "col({strip}:{STRIP_H}, row({}, {PADS}:{PADS_W}):*):*",
                first("kits")
            ),
            PlayView::Beats => format!(
                "row({}, col({PADS}:{PADS_W}, kits:*):{PADS_W}):*",
                first(BEATS)
            ),
            PlayView::Loop => format!(
                "col({strip}:*, lbuttons:64, row({}, {PADS}:{PADS_W}):{LOOP_ROW_H}):*",
                first("lmeter")
            ),
            PlayView::Drum => format!(
                "row({}, col({PADS}:{PADS_W}, kits:*):{PADS_W}):*",
                first("timing")
            ),
        }
    }

    pub fn layout(self) -> String {
        frame(&self.body(false))
    }

    /// The view while NOTE REPEAT is held or pads are latched.
    pub fn quick_layout(self) -> String {
        frame(&self.body(true))
    }
}

/// The Play frame around `body`: the transport bar and the view switch
/// over it, the master bar and the Q-Link cards beside.
pub fn frame(body: &str) -> String {
    format!(
        "col(header:60, row(col(row({TRANSPORT}:*, {SWITCH}:{SWITCH_W}):44, {body}):*, \
             {MASTER}:{MASTER_WIDTH}, {QLINKS}:{COLUMN_WIDTH}):*, hint:26)"
    )
}

/// A layout written for the old Play page, `col(header:60, BODY, hint:26)`,
/// in the frame: the arp's panel and band. When BODY is the pads beside a
/// column (`row(pads:*, col(...):W):*`), the column takes the view's left
/// and the pads panel the right, where the views have it. A layout already
/// in the frame comes back as it is.
pub fn framed(layout: &str) -> String {
    if layout.contains(SWITCH) {
        return layout.to_string();
    }
    let Some(body) = layout
        .strip_prefix("col(header:60, ")
        .and_then(|r| r.strip_suffix(", hint:26)"))
    else {
        return layout.to_string();
    };
    let side = body
        .strip_prefix("row(pads:*, ")
        .and_then(|r| r.strip_suffix("):*"))
        .and_then(|r| r.rsplit_once(':'));
    match side {
        Some((column, width)) if width.parse::<i32>().is_ok() => {
            frame(&format!("row({column}:*, {PADS}:{PADS_W}):*"))
        }
        _ => frame(body),
    }
}

/// The Synth tab's layout with the pads heading, and its switch, over the
/// presets (Jay, 12:22Z: the switch on Play and on the Synth tab).
pub fn synth_with_pads(layout: &str) -> String {
    let presets = mpfree_synth_ui::view::PRESETS;
    layout.replacen(
        &format!("{presets}:300"),
        &format!("col({SYNTH_PADS_HEAD}:30, {presets}:*):300"),
        1,
    )
}

// ---------------------------------------------------------------- pads bar

/// While the synth has the pads: the bank playing's layout, whether a
/// scale is picked (In key needs one), and the bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SynthPads {
    pub layout: PadLayout,
    pub scale: bool,
    pub bank: usize,
}

/// The pads panel's heading line: what the pads play, and while the synth
/// has them, the Chromatic | In key switch. A tap on a half gives `Tab(0)`
/// (Chromatic) or `Tab(1)` (In key).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadsBar {
    /// "PADS · BANK A · BOOM BAP", "SYNTH · BANK B · C3 TO D#4".
    pub title: String,
    pub synth: Option<SynthPads>,
}

/// The switch's halves, in tap order.
pub const LAYOUT_SWITCH: [&str; 2] = ["Chromatic", "In key"];
const PILL_W: i32 = 184;

impl PadsBar {
    /// The switch's two halves in the bar's rectangle `r`.
    pub fn halves(r: Rect) -> [Rect; 2] {
        let pill = Rect::new(
            r.right() - PILL_W,
            r.y + 1,
            PILL_W.min(r.w),
            (r.h - 2).max(0),
        );
        let w = pill.w / 2;
        [
            Rect::new(pill.x, pill.y, w, pill.h),
            Rect::new(pill.x + w, pill.y, pill.w - w, pill.h),
        ]
    }
}

impl Widget for PadsBar {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let line = r.inset_xy(4, 0);
        let Some(s) = self.synth else {
            cx.text(
                FontId::Bold,
                t.size_small,
                line,
                Align::Left,
                t.text_dim,
                &self.title.to_uppercase(),
            );
            return;
        };
        let halves = PadsBar::halves(r);
        let title = Rect::new(line.x, line.y, (halves[0].x - 8 - line.x).max(0), line.h);
        cx.text(
            FontId::Bold,
            t.size_small,
            title,
            Align::Left,
            t.text_dim,
            &self.title.to_uppercase(),
        );
        let pill = Rect::new(
            halves[0].x,
            halves[0].y,
            halves[0].w + halves[1].w,
            halves[0].h,
        );
        cx.canvas.fill_round_rect(pill, pill.h / 2, t.raised);
        let on = match s.layout {
            PadLayout::Chromatic => Some(0),
            PadLayout::InKey => Some(1),
            PadLayout::Chords => None,
        };
        for (i, (half, name)) in halves.iter().zip(LAYOUT_SWITCH).enumerate() {
            let lit = on == Some(i);
            if lit {
                cx.canvas
                    .fill_round_rect(half.inset(2), (half.h - 4) / 2, t.accent);
            }
            let ink = if lit {
                t.bg
            } else if i == 1 && !s.scale {
                t.text_faint
            } else {
                t.text_dim
            };
            cx.text(
                if lit { FontId::Bold } else { FontId::Sans },
                t.size_small,
                *half,
                Align::Center,
                ink,
                name,
            );
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down || self.synth.is_none() {
            return None;
        }
        PadsBar::halves(r)
            .iter()
            .position(|h| h.contains(x, y))
            .map(EventKind::Tab)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::layout;

    fn slots(spec: &str) -> Vec<(String, Rect)> {
        layout::solve(
            &layout::parse(spec).unwrap(),
            Rect::new(12, 12, 1256, 776),
            12,
        )
    }

    fn rect(spec: &str, name: &str) -> Rect {
        slots(spec)
            .into_iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} in {spec}"))
            .1
    }

    #[test]
    fn every_view_keeps_the_frame() {
        for v in PlayView::ALL {
            for spec in [v.layout(), v.quick_layout()] {
                assert_eq!(
                    rect(&spec, TRANSPORT),
                    Rect::new(12, 84, 998 - SWITCH_W - 12, 44),
                    "{spec}"
                );
                assert_eq!(
                    rect(&spec, MASTER),
                    Rect::new(1022, 84, MASTER_WIDTH, 666),
                    "{spec}"
                );
                assert_eq!(
                    rect(&spec, QLINKS),
                    Rect::new(1100, 84, COLUMN_WIDTH, 666),
                    "{spec}"
                );
                assert_eq!(rect(&spec, SWITCH).right(), 1010, "{spec}");
                assert_eq!(rect(&spec, "hint").y, 762, "{spec}");
            }
        }
    }

    #[test]
    fn the_band_never_moves_the_pads() {
        for v in PlayView::ALL {
            let (plain, band) = (v.layout(), v.quick_layout());
            assert_eq!(rect(&plain, "pads"), rect(&band, "pads"), "{v:?}");
            assert_eq!(rect(&band, arp_view::MINI).h, arp_view::MINI_H, "{v:?}");
            let pads = rect(&plain, "pads");
            assert!(pads.w >= 300 && pads.h >= 200, "{v:?}: {pads:?}");
        }
        assert_eq!(rect(&PlayView::All.layout(), "pads").w, PADS_W);
    }

    #[test]
    fn the_arp_panel_takes_the_left_and_the_pads_stay_right() {
        let spec = framed(arp_view::ARP_LAYOUT);
        let pads = rect(&spec, "pads");
        assert_eq!(
            (pads.x, pads.w),
            (rect(&PlayView::All.layout(), "pads").x, PADS_W)
        );
        let list = rect(&spec, arp_view::LIST);
        assert!(list.right() < pads.x && list.w >= 460, "{list:?}");
        assert_eq!(rect(&spec, QLINKS), Rect::new(1100, 84, COLUMN_WIDTH, 666));
        assert_eq!(framed(&spec), spec, "framing twice changes nothing");
        // Any other page body still gets the frame.
        let other = framed("col(header:60, log:*, hint:26)");
        assert!(rect(&other, "log").w > 900);
    }

    #[test]
    fn the_synth_tab_gets_the_pads_heading_over_its_presets() {
        use mpfree_synth_ui::view::{LAYOUT, PLAIN_LAYOUT, PRESETS};
        for l in [LAYOUT, PLAIN_LAYOUT] {
            let spec = synth_with_pads(l);
            let (head, presets) = (rect(&spec, SYNTH_PADS_HEAD), rect(&spec, PRESETS));
            assert_eq!((head.x, head.w, head.h), (presets.x, 300, 30), "{spec}");
            assert_eq!(presets.y, head.bottom() + 12);
        }
    }

    #[test]
    fn the_switch_halves_tap_chromatic_or_in_key() {
        let r = Rect::new(570, 140, 440, 30);
        let mut bar = PadsBar {
            title: "Synth · Bank A".into(),
            synth: None,
        };
        let [chromatic, in_key] = PadsBar::halves(r);
        assert_eq!(
            bar.touch(1, Touch::Down, in_key.x + 5, in_key.y + 5, r),
            None,
            "no switch for a kit"
        );
        bar.synth = Some(SynthPads {
            layout: PadLayout::Chromatic,
            scale: true,
            bank: 0,
        });
        assert_eq!(
            bar.touch(1, Touch::Down, in_key.x + 5, in_key.y + 5, r),
            Some(EventKind::Tab(1))
        );
        assert_eq!(
            bar.touch(1, Touch::Down, chromatic.x + 5, chromatic.y + 5, r),
            Some(EventKind::Tab(0))
        );
        assert_eq!(
            bar.touch(1, Touch::Down, r.x + 5, r.y + 5, r),
            None,
            "the title is not the switch"
        );
        assert_eq!(in_key.right(), r.right());
    }
}
