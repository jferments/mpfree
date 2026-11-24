//! Make a style (composing step 6, board D): the sheet that makes a style
//! from a clip, and the song's styles made from clips.
//!
//! - **The sheet** opens from the clip card's Style row on a clip with
//!   notes (pad hits too: their feel and accents are in when they are
//!   played). What it shows is read from the clip once, as it opens
//!   ([`mpfree_compose::read_clip`]); Name and Plan change before Keep.
//! - **Keep** puts the style in the song's styles as one edit UNDO takes
//!   back ([`mpfree_play::Takes::set_styles`]): a style of the same name
//!   is replaced, a 17th refused. The sheet closes.
//! - **The song's styles** are kept as lines of text in the project
//!   (`Project::styles`); read once each time they change
//!   ([`TimeControl::sync_styles`]) into `styles`, which the screens'
//!   Style rows run on into after the built-in styles (Continue, Continue
//!   on Steps, Add a part, and the arp's New idea through the app).

use std::sync::Arc;

use mpfree_compose::realize::PlainNote;
use mpfree_compose::{clip_styles, read_clip, ClipStyle, STYLES};
use mpfree_seq::MAX_STYLES;
use mpfree_time_ui::style_sheet::{StyleAction, StyleSheet, MAX_STYLE_NAME, NAME_ROW, PLAN_ROW};

use super::TimeControl;

impl TimeControl {
    /// After each action: when the project's style lines changed (Keep,
    /// UNDO, REDO), read them again, for the screens and the note editor.
    pub(super) fn sync_styles(&mut self) {
        if self.style_lines == self.project.styles {
            return;
        }
        self.style_lines = self.project.styles.clone();
        self.styles = Arc::from(clip_styles(&self.style_lines));
        if let Some(ed) = self.editor.as_mut() {
            ed.styles = Arc::clone(&self.styles);
        }
        self.changed = true;
    }

    /// The clip card's Style row: the sheet opens on clip `slot` of
    /// `track`, read from its notes.
    pub(super) fn make_style(&mut self, track: usize, slot: usize) -> Result<String, String> {
        let t = self
            .project
            .tracks
            .get(track)
            .ok_or("Make a style: no such track")?;
        let clip = self
            .project
            .clip(track, slot)
            .ok_or("Make a style: no such clip")?;
        if clip.looped.is_some() {
            return Err("A loop's cell has no notes to make a style of".into());
        }
        let notes: Vec<PlainNote> = clip
            .notes
            .iter()
            .map(|n| PlainNote {
                at: n.tick,
                len: n.len,
                pitch: n.note,
                velocity: n.velocity,
            })
            .collect();
        let meter = self.info.meter;
        let swing = clip.swing.or(t.swing).unwrap_or(self.info.swing);
        let read = read_clip(&notes, clip.length, meter, self.info.bpm, swing)
            .ok_or("Make a style: play or draw some notes first")?;
        let bar = meter.bar_ticks().max(1);
        let bars = ((clip.length.max(1) + bar - 1) / bar).max(1);
        let made = ClipStyle::new(&clip.name, read, meter);
        let from = format!(
            "From {} · {} · {bars} bar{}",
            clip.name,
            t.name,
            if bars == 1 { "" } else { "s" }
        );
        let line = format!(
            "style: read from {}: change Name and Plan, then Keep",
            clip.name
        );
        self.style_sheet = Some(Box::new(StyleSheet {
            track,
            slot,
            made,
            from,
            hits: clip.notes.iter().map(|n| (n.tick, n.velocity)).collect(),
            length: clip.length,
            bar,
            row: NAME_ROW,
        }));
        self.changed = true;
        Ok(line)
    }

    /// A touch on the sheet, its naming keyboard's Enter, or the wheel and
    /// - / + while it shows.
    pub(super) fn style_action(&mut self, a: StyleAction) -> Result<String, String> {
        let sheet = self
            .style_sheet
            .as_deref_mut()
            .ok_or("Make a style isn't open")?;
        match a {
            StyleAction::Pick(row) => {
                sheet.row = row.min(PLAN_ROW);
                self.changed = true;
                Ok(String::new())
            }
            StyleAction::Step { row: NAME_ROW, .. } => {
                Err("Name: tap it to type the style's name".to_string())
            }
            StyleAction::Step { delta, .. } => {
                sheet.row = PLAN_ROW;
                self.changed = true;
                if !sheet.step_plan(delta) {
                    let (end, name) = if delta < 0 {
                        ("first", STYLES[0].name)
                    } else {
                        ("last", STYLES[STYLES.len() - 1].name)
                    };
                    return Err(format!("Plan: {name} is the {end}"));
                }
                Ok(format!(
                    "style: phrase plan and chords from {}",
                    sheet.made.plan
                ))
            }
            StyleAction::Name(name) => {
                let name: String = name.trim().chars().take(MAX_STYLE_NAME).collect();
                if name.is_empty() {
                    return Err("A style needs a name".to_string());
                }
                if STYLES.iter().any(|s| s.name.eq_ignore_ascii_case(&name)) {
                    return Err(format!(
                        "{name} is a built-in style's name: give yours another"
                    ));
                }
                sheet.made.name = name;
                sheet.row = NAME_ROW;
                self.changed = true;
                Ok(format!("style: named {}", sheet.made.name))
            }
            StyleAction::Back => {
                self.style_sheet = None;
                self.changed = true;
                Ok("style: nothing kept".to_string())
            }
            StyleAction::Keep => self.keep_style(),
        }
    }

    /// Keep: the style joins the song's styles (one of the same name
    /// replaced), as an edit UNDO takes back, and the sheet closes.
    fn keep_style(&mut self) -> Result<String, String> {
        let Some(sheet) = self.style_sheet.as_deref() else {
            return Err("Make a style isn't open".into());
        };
        let made = sheet.made.clone();
        let mut styles = clip_styles(&self.project.styles);
        let replaced = match styles.iter().position(|s| s.name == made.name) {
            Some(k) => {
                styles[k] = made.clone();
                true
            }
            None if styles.len() >= MAX_STYLES => {
                return Err(format!(
                    "The song holds {MAX_STYLES} styles made from clips: give this one the name of one to replace it"
                ));
            }
            None => {
                styles.push(made.clone());
                false
            }
        };
        let lines = styles.iter().map(ClipStyle::to_text).collect();
        self.takes
            .set_styles(&mut self.project, lines)
            .map_err(|e| format!("Make a style: {e:?}"))?;
        self.style_sheet = None;
        self.changed = true;
        self.sync_styles();
        Ok(format!(
            "style: {} {}: in every Style row (UNDO takes it back)",
            made.name,
            if replaced { "replaced" } else { "kept" }
        ))
    }
}
