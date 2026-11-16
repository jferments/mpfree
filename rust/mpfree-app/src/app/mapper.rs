//! The control mapper (Debug mode, "Map controls"): every control of the MPC
//! One in a list, each found, skipped or still to find. The owner picks one
//! (a tap, or the wheel and +), presses it, and the number it sends is
//! written in the list and in the log (`map: REC = button 73`). Controls
//! MPFree uses (REC, OVERDUB, PAD BANK A/E…) work as soon as they are found.
//!
//! Pure logic, no I/O. Only what the controller sends decides a number: a
//! channel 1 note is a button, a channel 1 CC (other than the wheel and the
//! Q-Links, which are measured) is a knob or a dial.
//!
//! The names come from the MPC One's panel as MPFree expects it (an
//! assumption until each is found): a control that is not there is skipped,
//! and a button that is not listed goes in one of the Other rows.

use mpfree_controller::mpc_one::{button, DATA_WHEEL_CC, QLINK_TOUCH_NOTES, QLINK_TURN_CC};

/// What a control sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    /// A channel 1 note (pressed = velocity 127, released = 0).
    Button(u8),
    /// A channel 1 controller.
    Cc(u8),
}

impl Code {
    pub fn describe(self) -> String {
        match self {
            Code::Button(n) => format!("button {n}"),
            Code::Cc(n) => format!("CC {n}"),
        }
    }
}

/// What MPFree does with a control once its number is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    /// Nothing yet: the number is only written down.
    None,
    /// A measured constant in the code (SHIFT, FULL LEVEL, STOP, PLAY).
    Fixed(u8),
    Menu,
    NoteRepeat,
    TapTempo,
    Minus,
    Plus,
    Rec,
    Overdub,
    Undo,
    PlayStart,
    /// PAD BANK A/E (0) to D/H (3).
    Bank(u8),
    /// TRACK MIX (SHIFT: PAD MIX): the mixer.
    TrackMix,
    /// ERASE: nothing yet (live erase, and the FX lock mode with a step
    /// held, come later).
    Erase,
    /// MAIN: the Play page, and on it the next view.
    Main,
    /// SHIFT + 16 LEVEL: the synth pads' Chromatic | In key switch.
    SixteenLevel,
    /// NEXT SEQ: the Arrange tab.
    NextSeq,
    /// STEP SEQ: the Play page's Beats view.
    StepSeq,
    /// SAMPLER: the Sample tab (SHIFT: the Looper tab).
    Sampler,
    /// SAMPLE EDIT: the Sample tab's Wave editor.
    SampleEdit,
    /// PROG EDIT: the sound the pads play (the Synth tab, or Sample > Pads).
    ProgEdit,
    /// Q-LINK: the Q-Link column's next bank, Macros 1-4, Macros 5-8, then
    /// the page's own knobs again (on Play and the chain editor).
    QLink,
    /// TRACK MUTE: the pads mute tracks 1-8 over any page (SHIFT: the
    /// playing kit's pads), until it is pressed again.
    TrackMute,
    /// The data dial's push: in a menu, opens the row for the wheel to
    /// change, and a second push sets it (Jay, 12 Nov 08:11Z).
    DataPush,
    /// BROWSE: the browser over the page (SHIFT: save the project).
    Browse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Control {
    pub name: &'static str,
    pub job: Job,
}

const fn c(name: &'static str, job: Job) -> Control {
    Control { name, job }
}

/// The MPC One's buttons, transport first. Names are from the panel as
/// MPFree expects it.
pub const CONTROLS: [Control; 42] = [
    c("REC", Job::Rec),
    c("OVERDUB", Job::Overdub),
    c("STOP", Job::Fixed(button::STOP)),
    c("PLAY", Job::Fixed(button::PLAY)),
    c("PLAY START", Job::PlayStart),
    c("UNDO", Job::Undo),
    c("TAP TEMPO", Job::TapTempo),
    c("PAD BANK A/E", Job::Bank(0)),
    c("PAD BANK B/F", Job::Bank(1)),
    c("PAD BANK C/G", Job::Bank(2)),
    c("PAD BANK D/H", Job::Bank(3)),
    c("FULL LEVEL", Job::Fixed(button::FULL_LEVEL)),
    c("16 LEVEL", Job::SixteenLevel),
    c("NOTE REPEAT", Job::NoteRepeat),
    c("ERASE", Job::Erase),
    c("COPY", Job::None),
    c("TC", Job::None),
    c("SHIFT", Job::Fixed(button::SHIFT)),
    c("MENU", Job::Menu),
    c("MAIN", Job::Main),
    c("BROWSE", Job::Browse),
    // Opens the mixer. Named after the MPC 3.6 guide's MPC One top panel
    // (p. 348, item 17, as the FX thread cites it); not yet seen by the pad
    // player, so it works once found with the mapper.
    c("TRACK MIX / PAD MIX", Job::TrackMix),
    c("-", Job::Minus),
    c("+", Job::Plus),
    c("Q-LINK", Job::QLink),
    // The mode buttons the hardware control map gives a page (Jay, 20:40Z).
    c("TRACK MUTE", Job::TrackMute),
    c("NEXT SEQ", Job::NextSeq),
    c("STEP SEQ", Job::StepSeq),
    c("SAMPLER", Job::Sampler),
    c("SAMPLE EDIT", Job::SampleEdit),
    c("PROG EDIT", Job::ProgEdit),
    // The data dial's push (Enter), a button like the others; button 111 on
    // Jay's MPC One (Map controls, live test of 12 Nov), built in since
    // (`button::DATA_PUSH`).
    c("DATA DIAL PUSH", Job::DataPush),
    c("Other 1", Job::None),
    c("Other 2", Job::None),
    c("Other 3", Job::None),
    c("Other 4", Job::None),
    c("Other 5", Job::None),
    c("Other 6", Job::None),
    c("Other 7", Job::None),
    c("Other 8", Job::None),
    c("Other 9", Job::None),
    c("Other 10", Job::None),
];

/// Where a control stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Left,
    Found(Code),
    Skipped,
}

/// The controls the decoder already knows, named when pressed.
pub fn known(code: Code) -> Option<String> {
    match code {
        Code::Cc(DATA_WHEEL_CC) => Some("the data wheel".to_string()),
        Code::Cc(cc) => QLINK_TURN_CC
            .iter()
            .position(|&x| x == cc)
            .map(|k| format!("Q-Link {} turned", k + 1)),
        Code::Button(n) => QLINK_TOUCH_NOTES
            .iter()
            .position(|&x| x == n)
            .map(|k| format!("Q-Link {} touched", k + 1)),
    }
}

/// One row of the list as the screen shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct MapRow {
    pub name: String,
    pub value: String,
    pub mark: Mark,
}

/// What the Map controls page shows.
#[derive(Clone, Debug, PartialEq)]
pub struct MapView {
    pub rows: Vec<MapRow>,
    pub selected: usize,
    pub listening: bool,
    pub heading: String,
    pub lines: Vec<String>,
    /// The last control pressed on the page, found or not.
    pub last: String,
    pub found: usize,
    pub skipped: usize,
    pub left: usize,
}

/// A change the app makes when a control with a job is found or cleared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Learned {
    pub job: Job,
    pub code: Option<u8>,
}

pub struct Mapper {
    pub marks: Vec<Mark>,
    pub selected: usize,
    pub listening: bool,
    note: String,
    last: String,
    /// Something was found, skipped or cleared since the last summary.
    pub changed: bool,
}

impl Default for Mapper {
    fn default() -> Mapper {
        Mapper {
            marks: vec![Mark::Left; CONTROLS.len()],
            selected: 0,
            listening: false,
            note: String::new(),
            last: "nothing yet".to_string(),
            changed: false,
        }
    }
}

impl Mapper {
    /// Marks controls the app already knows (measured, or found by the
    /// guide) as found, leaving skipped and found rows alone.
    pub fn know(&mut self, i: usize, code: Option<u8>) {
        if let (Some(n), Mark::Left) = (code, self.marks[i]) {
            self.marks[i] = Mark::Found(Code::Button(n));
        }
    }

    fn owner(&self, code: Code) -> Option<usize> {
        self.marks.iter().position(|&m| m == Mark::Found(code))
    }

    /// The next control still to find after `from`, or `from` if none.
    fn next_left(&self, from: usize) -> usize {
        let n = CONTROLS.len();
        (1..=n)
            .map(|k| (from + k) % n)
            .find(|&i| self.marks[i] == Mark::Left)
            .unwrap_or(from)
    }

    pub fn select(&mut self, i: usize, listen: bool) {
        self.selected = i.min(CONTROLS.len() - 1);
        self.listening = listen;
        self.note.clear();
    }

    pub fn move_by(&mut self, delta: i32) {
        let last = CONTROLS.len() as i32 - 1;
        self.selected = (self.selected as i32 + delta).clamp(0, last) as usize;
        self.note.clear();
    }

    pub fn stop_listening(&mut self) {
        if self.listening {
            self.listening = false;
            self.note = "Stopped listening.".to_string();
        }
    }

    /// Skip the selected control (not on this MPC, or not sure).
    pub fn skip(&mut self) -> String {
        let i = self.selected;
        self.marks[i] = Mark::Skipped;
        self.listening = false;
        self.changed = true;
        self.note = format!("Skipped {}.", CONTROLS[i].name);
        self.selected = self.next_left(i);
        format!("map: {} skipped", CONTROLS[i].name)
    }

    /// Forget the selected control's number (measured ones stay).
    pub fn clear(&mut self) -> Option<(String, Learned)> {
        let i = self.selected;
        let ctl = CONTROLS[i];
        if let Job::Fixed(n) = ctl.job {
            self.note = format!("{} is measured (button {n}) and stays.", ctl.name);
            return None;
        }
        self.marks[i] = Mark::Left;
        self.changed = true;
        self.note = format!("{} is to find again.", ctl.name);
        Some((
            format!("map: {} cleared", ctl.name),
            Learned {
                job: ctl.job,
                code: None,
            },
        ))
    }

    /// A control pressed or turned while the page is open. Listening, a new
    /// number is the selected control's; otherwise it is only shown.
    /// Returns the log line and the job to apply, when one was found.
    pub fn input(&mut self, code: Code) -> Option<(String, Option<Learned>)> {
        let i = self.selected;
        let ctl = CONTROLS[i];
        let named = self
            .owner(code)
            .map(|o| CONTROLS[o].name.to_string())
            .or_else(|| known(code));
        self.last = match &named {
            Some(name) => format!("{} = {name}", code.describe()),
            None => format!("{} = not in the list yet", code.describe()),
        };
        if !self.listening {
            return None;
        }
        if self.marks[i] == Mark::Found(code) {
            self.listening = false;
            self.note = format!("{} is {} (again).", ctl.name, code.describe());
            return None;
        }
        if let Some(name) = named {
            if code == Code::Button(button::STOP) {
                self.listening = false;
                self.note = "Stopped listening (STOP).".to_string();
            } else {
                self.note = format!("That was {name} ({}).", code.describe());
            }
            return None;
        }
        if let Job::Fixed(n) = ctl.job {
            // A measured button answering with another number: say so, keep the constant.
            self.listening = false;
            self.note = format!(
                "{} is measured as button {n}, but {} came. Nothing changed.",
                ctl.name,
                code.describe()
            );
            return Some((
                format!(
                    "map: {} sent {}, measured as button {n} (kept)",
                    ctl.name,
                    code.describe()
                ),
                None,
            ));
        }
        self.marks[i] = Mark::Found(code);
        self.listening = false;
        self.changed = true;
        self.note = format!("{} is {}.", ctl.name, code.describe());
        self.last = format!("{} = {}", code.describe(), ctl.name);
        self.selected = self.next_left(i);
        let learned = match code {
            Code::Button(n) if ctl.job != Job::None => Some(Learned {
                job: ctl.job,
                code: Some(n),
            }),
            _ => None,
        };
        Some((format!("map: {} = {}", ctl.name, code.describe()), learned))
    }

    /// One line with every control found and skipped, for the log.
    pub fn summary(&self) -> String {
        let found: Vec<String> = CONTROLS
            .iter()
            .zip(&self.marks)
            .filter_map(|(c, m)| match m {
                Mark::Found(code) => Some(format!("{} {}", c.name, code.describe())),
                _ => None,
            })
            .collect();
        let skipped: Vec<&str> = CONTROLS
            .iter()
            .zip(&self.marks)
            .filter(|(_, m)| **m == Mark::Skipped)
            .map(|(c, _)| c.name)
            .collect();
        let left = self.marks.iter().filter(|m| **m == Mark::Left).count();
        format!(
            "map summary: {} found ({}); skipped: {}; {left} left",
            found.len(),
            found.join(", "),
            if skipped.is_empty() {
                "none".to_string()
            } else {
                skipped.join(", ")
            }
        )
    }

    pub fn view(&self) -> MapView {
        let rows: Vec<MapRow> = CONTROLS
            .iter()
            .zip(&self.marks)
            .enumerate()
            .map(|(i, (c, &m))| {
                let value = match m {
                    _ if self.listening && i == self.selected => "press it now".to_string(),
                    Mark::Found(code) => code.describe(),
                    Mark::Skipped => "skipped".to_string(),
                    Mark::Left => "-".to_string(),
                };
                MapRow {
                    name: c.name.to_string(),
                    value,
                    mark: m,
                }
            })
            .collect();
        let count = |k: fn(&Mark) -> bool| self.marks.iter().filter(|m| k(m)).count();
        let ctl = CONTROLS[self.selected];
        let (heading, mut lines) = if self.listening {
            (
                format!("Press {}", ctl.name),
                vec![
                    "Press or turn it once: its number goes in the list and the log.".to_string(),
                    "Buttons do nothing else while MPFree listens.".to_string(),
                    "STOP or Stop listening: stop. The wheel picks another control.".to_string(),
                ],
            )
        } else {
            let state = match self.marks[self.selected] {
                Mark::Found(code) => format!("{} is {}.", ctl.name, code.describe()),
                Mark::Skipped => format!("{} was skipped.", ctl.name),
                Mark::Left => format!("{} is still to find.", ctl.name),
            };
            (
                ctl.name.to_string(),
                vec![
                    state,
                    "Tap a control, or pick it with the wheel and press +, then press it."
                        .to_string(),
                    "-: skip it (not on this MPC). Unlisted buttons: use an Other row.".to_string(),
                ],
            )
        };
        if !self.note.is_empty() {
            lines.insert(0, self.note.clone());
        }
        MapView {
            rows,
            selected: self.selected,
            listening: self.listening,
            heading,
            lines,
            last: self.last.clone(),
            found: count(|m| matches!(m, Mark::Found(_))),
            skipped: count(|m| *m == Mark::Skipped),
            left: count(|m| *m == Mark::Left),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(name: &str) -> usize {
        CONTROLS.iter().position(|c| c.name == name).unwrap()
    }

    #[test]
    fn names_are_unique_and_measured_buttons_are_listed() {
        for (i, a) in CONTROLS.iter().enumerate() {
            assert!(
                CONTROLS.iter().skip(i + 1).all(|b| b.name != a.name),
                "{}",
                a.name
            );
        }
        for n in [
            button::STOP,
            button::PLAY,
            button::FULL_LEVEL,
            button::SHIFT,
        ] {
            assert!(CONTROLS.iter().any(|c| c.job == Job::Fixed(n)));
        }
        assert_eq!(
            CONTROLS
                .iter()
                .filter(|c| matches!(c.job, Job::Bank(_)))
                .count(),
            4
        );
    }

    #[test]
    fn listening_takes_a_new_number_and_moves_to_the_next_control_left() {
        let mut m = Mapper::default();
        m.know(index("STOP"), Some(button::STOP));
        m.select(index("REC"), true);
        assert_eq!(m.view().rows[0].value, "press it now");
        let (line, learned) = m.input(Code::Button(73)).unwrap();
        assert_eq!(line, "map: REC = button 73");
        assert_eq!(
            learned,
            Some(Learned {
                job: Job::Rec,
                code: Some(73)
            })
        );
        assert!(!m.listening);
        assert_eq!(m.selected, index("OVERDUB"));
        let v = m.view();
        assert_eq!(v.rows[0].value, "button 73");
        assert_eq!(v.found, 2);
        assert!(v.lines[0].contains("REC is button 73"));
        assert_eq!(v.last, "button 73 = REC");
    }

    #[test]
    fn known_numbers_are_named_not_taken_and_stop_stops_listening() {
        let mut m = Mapper::default();
        m.know(index("STOP"), Some(button::STOP));
        m.know(index("MENU"), Some(button::MENU));
        m.select(index("ERASE"), true);
        assert_eq!(m.input(Code::Button(button::MENU)), None);
        assert!(m.listening);
        assert!(m.view().lines[0].contains("That was MENU (button 123)"));
        assert_eq!(m.input(Code::Cc(17)), None, "Q-Link 2 is measured");
        assert!(m.view().lines[0].contains("Q-Link 2 turned"));
        assert_eq!(m.input(Code::Button(button::STOP)), None);
        assert!(!m.listening);
        assert_eq!(m.marks[index("ERASE")], Mark::Left);
        // Not listening: a press is only shown.
        assert_eq!(m.input(Code::Button(52)), None);
        assert_eq!(m.view().last, "button 52 = not in the list yet");
    }

    #[test]
    fn a_knob_is_a_cc_and_others_take_unlisted_buttons() {
        let mut m = Mapper::default();
        m.select(index("Other 1"), true);
        let (line, learned) = m.input(Code::Cc(30)).unwrap();
        assert_eq!((line.as_str(), learned), ("map: Other 1 = CC 30", None));
        m.select(index("Other 2"), true);
        assert_eq!(
            m.input(Code::Button(52)).unwrap().0,
            "map: Other 2 = button 52"
        );
        assert!(m.summary().contains("Other 1 CC 30, Other 2 button 52"));
    }

    #[test]
    fn skip_clear_and_measured_buttons() {
        let mut m = Mapper::default();
        m.select(index("TC"), false);
        assert_eq!(m.skip(), "map: TC skipped");
        assert_eq!(m.marks[index("TC")], Mark::Skipped);
        assert_ne!(m.selected, index("TC"));
        m.select(index("PAD BANK A/E"), true);
        let (_, learned) = m.input(Code::Button(35)).unwrap();
        assert_eq!(
            learned,
            Some(Learned {
                job: Job::Bank(0),
                code: Some(35)
            })
        );
        m.select(index("PAD BANK A/E"), false);
        assert_eq!(
            m.clear(),
            Some((
                "map: PAD BANK A/E cleared".to_string(),
                Learned {
                    job: Job::Bank(0),
                    code: None
                }
            ))
        );
        m.know(index("STOP"), Some(button::STOP));
        m.select(index("STOP"), false);
        assert_eq!(m.clear(), None);
        // A measured button sending something else keeps its constant.
        m.select(index("PLAY"), true);
        let (line, learned) = m.input(Code::Button(99)).unwrap();
        assert!(
            line.contains("measured as button 82 (kept)") && learned.is_none(),
            "{line}"
        );
        let s = m.summary();
        assert!(
            s.starts_with("map summary: 1 found") && s.contains("skipped: TC"),
            "{s}"
        );
    }
}
