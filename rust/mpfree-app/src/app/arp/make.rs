//! Make a phrase (composing tools, step 3 of the phrases): a phrase of
//! your own from a clip's notes or a MIDI file's, on a sheet that opens in
//! the clip card's place on Arrange (the card's Phrase row) and in the
//! Selected panel's place in the browser (a MIDI file's Make a phrase).
//! The design is in `docs/dev/design/arpeggiator.md`, "Make a phrase".
//!
//! - The notes are read as they play ([`mpfree_compose::clip_played`]) and
//!   made into a phrase at once ([`mpfree_compose::phrase_of_as`]): its
//!   key, kind, home note, grid and length all come from the notes.
//! - **Kind** and **Home** are what was read; either can be changed before
//!   Keep, and the phrase is made again (a kind changed re-reads the home
//!   note for that kind unless Home was changed too).
//! - While it is open the phrase is the armed track's engine's
//!   ([`App::send_phrase`]) and the pads play through Phrase, as on the Arp
//!   panel's Phrase tab, so a held pad plays it from that note (on a synth:
//!   a kit's pads play as pressed). The project keeps the track's pick,
//!   never the sheet's phrase.
//! - **Keep** puts it on Yours under the clip's or the file's name (the
//!   first number free from 2 when the name is taken), the track picks it,
//!   and with a store it is saved with the owner's other phrases
//!   ([`Effect::SavePhrases`]).
//! - **Back** (or ×) closes it and the track's pick plays again. So does
//!   anything that takes the page away, or another track.

use crate::app::{App, Effect, View};
use mpfree_arp::phrase::Kind;
use mpfree_arp::{Mode, Phrase};
use mpfree_arp_ui::make_sheet::{SheetAction, SheetView};
use mpfree_arp_ui::maker::MakerRow;
use mpfree_compose::{phrase_of_as, Asked, Made, MakeRefused};
use mpfree_context::listen::Played;
use mpfree_transport::time::{Meter, Tick};

/// The Home row's lowest and highest notes: C1 to C7.
const HOME_RANGE: (u8, u8) = (24, 96);

/// The sheet's rows.
const ROWS: [&str; 2] = ["Kind", "Home"];

/// Notes to make a phrase of, and where they came from.
#[derive(Clone, Debug, PartialEq)]
pub struct MakeFrom {
    /// "Clip 1 · Track 1", "Bassline.mid".
    pub place: String,
    /// The name Keep starts from: the clip's, the file's.
    pub name: String,
    /// From the start of the clip or the file, on its beats.
    pub notes: Vec<Played>,
    /// How long it is, in ticks (0: as far as its notes reach).
    pub len: Tick,
    pub meter: Meter,
}

impl MakeFrom {
    /// Clip `slot` of `track` in `project`, as it plays, in `meter`; or
    /// why it has nothing to make a phrase of.
    pub fn clip(
        project: &mpfree_seq::Project,
        meter: Meter,
        track: usize,
        slot: usize,
    ) -> Result<MakeFrom, String> {
        let c = project
            .clip(track, slot)
            .ok_or("Make a phrase: that slot is empty")?;
        let (notes, len) = mpfree_compose::clip_played(c);
        if notes.is_empty() {
            return Err(match c.holds {
                mpfree_seq::Holds::Hits => {
                    "Make a phrase: a clip of pad hits has no notes of its own (a pad's note is its kit's)"
                }
                _ => "Make a phrase: play or draw some notes first",
            }
            .to_string());
        }
        let clip = match c.name.trim() {
            "" => format!("Clip {}", slot + 1),
            n => n.to_string(),
        };
        let track_name = match project.tracks.get(track).map(|t| t.name.trim()) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => format!("Track {}", track + 1),
        };
        Ok(MakeFrom {
            place: format!("{clip} · {track_name}"),
            name: clip,
            notes,
            len,
            meter,
        })
    }
}

impl MakeFrom {
    /// A MIDI file's first part with notes (`midi_file::import`'s parts:
    /// a file track's notes on one channel), from the file's start in its
    /// first meter, named after the file.
    pub fn midi(m: &crate::midi_file::MidiImport) -> Result<MakeFrom, String> {
        let part = m
            .parts
            .iter()
            .find(|p| !p.notes.is_empty())
            .ok_or("Make a phrase: the file has no notes")?;
        let name = std::path::Path::new(&m.file)
            .file_stem()
            .map_or_else(|| m.file.clone(), |s| s.to_string_lossy().into_owned());
        Ok(MakeFrom {
            place: m.file.clone(),
            name,
            notes: part
                .notes
                .iter()
                .map(|n| Played::new(n.tick as Tick, n.len as Tick, n.note, n.velocity))
                .collect(),
            len: m.length,
            meter: Meter {
                beats: m.meter.0,
                unit: m.meter.1,
            },
        })
    }
}

/// The sheet while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    from: MakeFrom,
    /// The page it opened on: another page closes it.
    view: View,
    asked: Asked,
    /// What the notes say, asked nothing.
    read: Result<Made, MakeRefused>,
    /// What Keep would keep.
    made: Result<Made, MakeRefused>,
    /// The row the wheel is on.
    row: usize,
}

impl Sheet {
    fn new(from: MakeFrom, view: View) -> Sheet {
        let read = phrase_of_as(&from.notes, from.len, from.meter, Asked::default());
        Sheet {
            from,
            view,
            asked: Asked::default(),
            made: read,
            read,
            row: 0,
        }
    }

    fn remake(&mut self) {
        let f = &self.from;
        self.made = phrase_of_as(&f.notes, f.len, f.meter, self.asked);
    }

    /// Row `row` stepped by `delta`: whether it changed.
    fn step(&mut self, row: usize, delta: i32) -> bool {
        let Ok(made) = self.made else {
            return false;
        };
        let d = delta.signum();
        match row {
            0 => {
                let now = Kind::ALL
                    .iter()
                    .position(|&k| k == made.phrase.kind)
                    .unwrap_or(0) as i32;
                let to = now + d;
                if !(0..Kind::ALL.len() as i32).contains(&to) {
                    return false;
                }
                self.asked.kind = Some(Kind::ALL[to as usize]);
            }
            _ => {
                let to = i32::from(made.home) + d;
                if !(i32::from(HOME_RANGE.0)..=i32::from(HOME_RANGE.1)).contains(&to) {
                    return false;
                }
                self.asked.home = Some(to as u8);
            }
        }
        self.remake();
        true
    }

    fn phrase(&self) -> Option<Phrase> {
        self.made.ok().map(|m| m.phrase)
    }
}

/// "Bass, C2": what the notes said, for the line under the rows once
/// either row was changed.
fn said(m: &Made) -> String {
    format!(
        "{}, {}",
        m.phrase.kind.label(),
        crate::synth::note_name(m.home)
    )
}

impl App {
    /// The phrase a held pad plays while the sheet is open.
    pub(super) fn sheet_try(&self) -> Option<Phrase> {
        self.sheet.as_ref()?.phrase()
    }

    /// The sheet opens on `from` (the clip card's Phrase row, a MIDI
    /// file's Make a phrase), on the page shown now. New idea or More like
    /// this closes first.
    pub fn open_sheet(&mut self, from: MakeFrom) -> Vec<Effect> {
        let mut fx = self.close_maker();
        let sheet = Sheet::new(from, self.view);
        let note = match &sheet.made {
            Ok(m) => format!("make a phrase: {}", m.phrase.to_text()),
            Err(why) => format!("make a phrase: {}", why.words()),
        };
        self.sheet = Some(sheet);
        self.send_phrase();
        fx.extend(self.sheet_engine());
        fx.push(Effect::Note(note));
        fx.push(self.arp_view());
        fx
    }

    /// The sheet opened or closed: the pads play through Phrase while it
    /// is open, and the NOTE REPEAT light follows.
    fn sheet_engine(&mut self) -> Vec<Effect> {
        let was = self.generating();
        self.update_engine();
        if self.generating() != was {
            self.repeat_light().into_iter().collect()
        } else {
            Vec::new()
        }
    }

    /// A touch on the sheet, or the wheel or - / + while it is open.
    pub(in crate::app) fn sheet_action(&mut self, a: SheetAction) -> Vec<Effect> {
        let Some(s) = self.sheet.as_mut() else {
            return Vec::new();
        };
        match a {
            SheetAction::Step { row, delta } => {
                s.row = row.min(ROWS.len() - 1);
                self.sheet_row_step(delta)
            }
            SheetAction::Wheel(d) => {
                let to = (s.row as i32 + d).clamp(0, ROWS.len() as i32 - 1) as usize;
                if to == s.row {
                    return Vec::new();
                }
                s.row = to;
                vec![self.arp_view()]
            }
            SheetAction::Change(d) => self.sheet_row_step(d),
            SheetAction::Back => self.close_sheet(),
            SheetAction::Keep => self.keep_sheet(),
        }
    }

    /// The wheel's row stepped by `delta`: the phrase is made again, and
    /// a held pad plays the new one.
    fn sheet_row_step(&mut self, delta: i32) -> Vec<Effect> {
        let Some(s) = self.sheet.as_mut() else {
            return Vec::new();
        };
        let row = s.row;
        if !s.step(row, delta) {
            let why = match (row, delta > 0) {
                (0, false) => "Kind is at its first, Bass",
                (0, true) => "Kind is at its last, Arp lines",
                (_, false) => "Home is at its lowest, C1",
                (_, true) => "Home is at its highest, C7",
            };
            return match s.made {
                Ok(_) => vec![Effect::Note(format!("make a phrase: {why}"))],
                Err(_) => Vec::new(),
            };
        }
        self.send_phrase();
        let text = self.sheet_try().map(|p| p.to_text()).unwrap_or_default();
        vec![
            Effect::Note(format!("make a phrase: {text}")),
            self.arp_view(),
        ]
    }

    /// Back, or anything that takes the sheet away: it closes and the
    /// track's pick plays again. Nothing else changes.
    pub(in crate::app) fn close_sheet(&mut self) -> Vec<Effect> {
        if self.sheet.take().is_none() {
            return Vec::new();
        }
        self.send_phrase();
        let mut fx = self.sheet_engine();
        fx.push(Effect::Note(
            "make a phrase: back, nothing kept".to_string(),
        ));
        fx.push(self.arp_view());
        fx
    }

    /// The sheet is open on another page than the one shown: it closes.
    pub(in crate::app) fn sheet_follows_view(&mut self) -> Vec<Effect> {
        match &self.sheet {
            Some(s) if s.view != self.view => self.close_sheet(),
            _ => Vec::new(),
        }
    }

    /// The name Keep gives the sheet's phrase: the clip's or the file's,
    /// or with that taken the first number free from 2.
    fn sheet_keep_name(&self, base: &str) -> String {
        super::maker::keep_name(base, self.phrases.iter().map(|(n, _)| n.as_str()))
    }

    /// Keep: the phrase joins Yours and the track picks it; with a store,
    /// the owner's phrases are saved.
    fn keep_sheet(&mut self) -> Vec<Effect> {
        let Some(phrase) = self.sheet_try() else {
            return Vec::new();
        };
        let base = self.sheet.as_ref().map(|s| s.from.name.clone());
        let name = self.sheet_keep_name(&base.unwrap_or_default());
        self.sheet = None;
        let light = self.sheet_engine();
        self.own_phrase(&name, phrase);
        let Some(i) = self.phrases.iter().position(|(n, _)| *n == name) else {
            return Vec::new();
        };
        let mut fx = light;
        fx.extend([
            Effect::Phrases(self.phrase_list()),
            // A library line, so the report carries it home.
            Effect::Note(format!("arp-phrase-kept: {name} = {}", phrase.to_text())),
            Effect::Note(format!(
                "Make a phrase: {name} is on the Phrase tab's Yours"
            )),
        ]);
        fx.extend(self.pick_phrase(i));
        if self.pattern_file {
            fx.extend(self.phrases_text().map(Effect::SavePhrases));
        } else {
            fx.push(Effect::Note(
                "the phrase stays in memory until the power goes off (no --data folder to keep it in)"
                    .to_string(),
            ));
        }
        fx.push(self.arp_view());
        fx
    }

    /// A held pad plays the sheet's phrase: the pads play through Phrase
    /// (not on a kit, whose pads play as pressed).
    fn sheet_heard(&self) -> bool {
        self.engine_mode() == Mode::Phrase || self.pads_own_mode() == Mode::Phrase
    }

    /// What the sheet shows, while it is open ([`ArpView::sheet`]).
    ///
    /// [`ArpView::sheet`]: crate::app::ArpView::sheet
    pub(super) fn sheet_view(&self) -> Option<SheetView> {
        let s = self.sheet.as_ref()?;
        let hint = "Kind and Home: read from the notes, change them before Keep   ·   \
                    Keep: on the Phrase tab's Yours   ·   Back: nothing changes"
            .to_string();
        let Ok(m) = s.made else {
            let why = s.made.err().map_or("", |e| e.words());
            return Some(SheetView {
                from: format!("From {}", s.from.place),
                status: why.to_string(),
                status_warn: true,
                hint,
                ..SheetView::default()
            });
        };
        let p = m.phrase;
        let bars = match p.bars {
            1 => "1 bar".to_string(),
            n => format!("{n} bars"),
        };
        let heard = match m.key {
            Some(k) => format!("Heard in {} · on {}s", k.name(), p.unit.label()),
            None => format!("No clear key · on {}s", p.unit.label()),
        };
        let rows = vec![
            MakerRow {
                name: ROWS[0].to_string(),
                value: p.kind.label().to_string(),
                ..MakerRow::default()
            },
            MakerRow {
                name: ROWS[1].to_string(),
                value: crate::synth::note_name(m.home),
                accent: true,
                ..MakerRow::default()
            },
        ];
        let second = match s.read {
            Ok(r) if s.asked != Asked::default() && (r.phrase.kind, r.home) != (p.kind, m.home) => {
                format!("The notes said {}.", said(&r))
            }
            _ => "Both are read from the notes.".to_string(),
        };
        let name = self.sheet_keep_name(&s.from.name);
        let status = if self.sheet_heard() {
            "Hold a pad: hear it from that note"
        } else {
            "Arm a synth track to hear it"
        };
        Some(SheetView {
            from: format!("From {} · {bars}", s.from.place),
            phrase: Some(p),
            heard,
            rows,
            row: s.row,
            notes: vec!["A held pad plays it from here.".to_string(), second],
            status: status.to_string(),
            status_warn: false,
            keep_line: format!("Keep puts it on Yours as {name}."),
            name,
            can_keep: true,
            hint,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_arp::scale::pads::{layout, PadLayout, PadSetup};
    use mpfree_arp_ui::view::ArpAction;

    const Q: Tick = 960;
    const FOUR: Meter = Meter { beats: 4, unit: 4 };

    /// A C minor bass line from C2 over two bars, as a clip "Clip 1" on
    /// "Track 1".
    fn bass_line() -> MakeFrom {
        let line = [
            (0, 36),
            (Q, 36),
            (3 * Q / 2, 43),
            (2 * Q, 39),
            (3 * Q, 36),
            (4 * Q, 36),
            (5 * Q, 46),
            (6 * Q, 43),
            (7 * Q, 39),
        ];
        MakeFrom {
            place: "Clip 1 · Track 1".into(),
            name: "Clip 1".into(),
            notes: line
                .iter()
                .map(|&(at, p)| Played::new(at, Q / 2, p, 100))
                .collect(),
            len: 8 * Q,
            meter: FOUR,
        }
    }

    /// Arrange shown, the pads on a synth, the arp on its Phrase tab for
    /// the armed track.
    fn on_arrange() -> App {
        let mut app = App::new(44_100, false);
        app.start();
        app.set_pad_layout(Some(layout(&PadSetup {
            layout: PadLayout::Chromatic,
            ..PadSetup::default()
        })));
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        app.show(View::Arrange);
        assert_eq!(app.view(), View::Arrange);
        app
    }

    fn act(app: &mut App, a: SheetAction) -> Vec<Effect> {
        app.arp_action(ArpAction::Sheet(a))
    }

    fn view(app: &App) -> SheetView {
        app.sheet_view().expect("the sheet is open")
    }

    /// The sheet reads the notes (a bass line from C2, heard in C minor,
    /// on 1/8s, two bars) and a held pad plays the phrase at once; Kind
    /// and Home change it, and the line under the rows says what the
    /// notes said; Keep puts it on Yours as Clip 1 and the track picks it,
    /// a second Keep of the same clip as Clip 1 2.
    #[test]
    fn the_sheet_reads_the_notes_and_keep_joins_yours() {
        let mut app = on_arrange();
        let pick = app.eng().phrase().to_owned();
        app.pattern_file = true;
        app.open_sheet(bass_line());
        let v = view(&app);
        assert_eq!(v.from, "From Clip 1 · Track 1 · 2 bars");
        assert_eq!(v.heard, "Heard in C minor · on 1/8s");
        let rows: Vec<(&str, &str)> = v
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.value.as_str()))
            .collect();
        assert_eq!(rows, [("Kind", "Bass"), ("Home", "C2")]);
        assert_eq!(v.status, "Hold a pad: hear it from that note");
        assert_eq!(v.keep_line, "Keep puts it on Yours as Clip 1.");
        assert_eq!(v.notes[1], "Both are read from the notes.");
        let made = v.phrase.unwrap();
        assert_eq!(app.eng().phrase(), &made, "a held pad plays it");
        assert_ne!(made, pick);
        // Home up a half step with the wheel and +: C#2, said under the
        // rows.
        use mpfree_controller::mpc_one::button::PLUS;
        app.wheel(1);
        assert_eq!(view(&app).row, 1);
        app.button(PLUS, true, 0.0);
        app.button(PLUS, false, 0.0);
        let v = view(&app);
        assert_eq!(v.rows[1].value, "C#2");
        assert_eq!(v.notes[1], "The notes said Bass, C2.");
        assert_eq!(app.eng().phrase(), &v.phrase.unwrap());
        // Kind back to its first: nothing changes, and a line says so.
        act(&mut app, SheetAction::Wheel(-1));
        let fx = act(&mut app, SheetAction::Change(-1));
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Note(n) if n.contains("at its first"))));
        act(&mut app, SheetAction::Step { row: 0, delta: 1 });
        assert_eq!(view(&app).rows[0].value, "Riff");
        let kept = view(&app).phrase.unwrap();
        let fx = act(&mut app, SheetAction::Keep);
        assert!(app.sheet_view().is_none());
        let (name, p) = &app.phrases()[app.phrase_index()];
        assert_eq!((name.as_str(), *p), ("Clip 1", kept));
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::SavePhrases(t) if t.contains("Clip 1"))));
        assert_eq!(app.eng().phrase(), &kept);
        app.open_sheet(bass_line());
        assert_eq!(view(&app).keep_line, "Keep puts it on Yours as Clip 1 2.");
    }

    /// Back, × and another page close the sheet with nothing kept: the
    /// track's pick plays again; notes in 6/8 make nothing, and say why.
    #[test]
    fn back_and_leaving_change_nothing() {
        let mut app = on_arrange();
        let pick = app.eng().phrase().to_owned();
        let count = app.phrases().len();
        app.open_sheet(bass_line());
        act(&mut app, SheetAction::Back);
        assert!(app.sheet_view().is_none());
        assert_eq!(app.eng().phrase(), &pick);
        app.open_sheet(bass_line());
        app.show(View::Play);
        assert!(app.sheet_view().is_none(), "another page closes it");
        assert_eq!(app.eng().phrase(), &pick);
        assert_eq!(app.phrases().len(), count);
        app.show(View::Arrange);
        let mut six = bass_line();
        six.meter = Meter { beats: 6, unit: 8 };
        app.open_sheet(six);
        let v = view(&app);
        assert!(v.status_warn && !v.can_keep && v.phrase.is_none());
        assert!(v.status.contains("quarter notes"), "{}", v.status);
        assert!(act(&mut app, SheetAction::Keep).is_empty());
        assert_eq!(app.eng().phrase(), &pick);
    }

    /// A clip is read as it plays, named by its clip and track; pad hits
    /// and an empty slot say why there is nothing to make.
    #[test]
    fn a_clip_is_named_and_read() {
        use mpfree_seq::{Clip, Holds, Note, Project, Track};
        let mut project = Project::new();
        let mut t = Track::new("Bass", mpfree_seq::DestId::default());
        let mut c = Clip::new("", 4 * Q);
        c.holds = Holds::Notes;
        c.notes = vec![Note::new(0, Q, 36, 100)];
        t.slots[2] = Some(c);
        project.tracks.push(t);
        let from = MakeFrom::clip(&project, FOUR, 0, 2).unwrap();
        assert_eq!(
            (from.place.as_str(), from.name.as_str()),
            ("Clip 3 · Bass", "Clip 3")
        );
        assert_eq!(from.notes, [Played::new(0, Q, 36, 100)]);
        assert_eq!(from.len, 4 * Q);
        assert!(MakeFrom::clip(&project, FOUR, 0, 1)
            .unwrap_err()
            .contains("empty"));
        project.tracks[0].slots[2].as_mut().unwrap().holds = Holds::Hits;
        assert!(MakeFrom::clip(&project, FOUR, 0, 2)
            .unwrap_err()
            .contains("pad hits"));
    }

    /// A MIDI file is read from its first track with notes, named after
    /// the file, its length and meter the file's; it opens on the browser
    /// and reads as the clip it was made from. A file of no notes says so.
    #[test]
    fn a_midi_file_is_named_and_read() {
        let bytes = crate::gui::sample_bassline_mid();
        let m = crate::midi_file::import("Bassline.mid", &bytes).unwrap();
        let from = MakeFrom::midi(&m).unwrap();
        assert_eq!(
            (from.place.as_str(), from.name.as_str()),
            ("Bassline.mid", "Bassline")
        );
        let clip = crate::gui::sample_bass_clip();
        assert_eq!(from.notes, clip.notes, "as the clip it was made from");
        assert_eq!((from.len, from.meter), (8 * Q, FOUR));
        let mut app = App::new(44_100, false);
        app.start();
        app.set_pad_layout(Some(layout(&PadSetup {
            layout: PadLayout::Chromatic,
            ..PadSetup::default()
        })));
        app.show(View::Browser);
        app.open_sheet(from);
        let v = view(&app);
        assert_eq!(v.from, "From Bassline.mid · 2 bars");
        assert_eq!(v.keep_line, "Keep puts it on Yours as Bassline.");
        assert!(v.can_keep && v.phrase.is_some());
        let empty = mpfree_midi::smf::Smf {
            name: "Empty".into(),
            tempos: vec![(0, 120.0)],
            meters: vec![(0, 4, 4)],
            tracks: vec![mpfree_midi::smf::SmfTrack {
                name: "Nothing".into(),
                events: Vec::new(),
            }],
            end: 4 * 960,
        }
        .bytes();
        let none = crate::midi_file::import("Empty.mid", &empty);
        let why = none.and_then(|m| MakeFrom::midi(&m)).unwrap_err();
        assert!(why.contains("no notes"), "{why}");
    }
}
