//! The Export panel: Export in the Projects place opens it over the
//! browser (layout spec section 3; Jay's pick, 14 Nov 01:39Z). It picks
//! what to make (the mixdown, the stems, the MIDI file), the range and
//! the files' format (Prefs' for each new export; Jay's pick, 14 Nov
//! 07:51Z: both in Prefs and on the panel),
//! says how the export will play (Fast, quiet or Listen, and why), shows
//! the time done while it runs with Cancel, and when it is over the
//! outcome with Show in BROWSE.
//!
//! The panel only asks: the app makes the plan, runs the pass and says how
//! far it has got ([`crate::Browser::export_running`],
//! [`crate::Browser::export_over`]).

use mpfree_library::Loc;

/// The bars an export of the clips playing (no arrangement) lasts by
/// default, and the most it can be set to.
pub const DEFAULT_BARS: u32 = 8;
pub const MAX_BARS: u32 = 999;

/// What the export plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportRange {
    /// The arrangement from its start to its end.
    Arrangement,
    /// The clips playing now, for this many bars from the bar the
    /// playhead is in.
    Bars(u32),
}

/// How the WAV files hold each sample (the app's `mpfree_fx::Bits`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportBits {
    I24,
    /// 16-bit, dithered.
    I16,
    F32,
}

/// The rates the panel offers.
pub const EXPORT_RATES: [u32; 2] = [44_100, 48_000];

/// The WAV files' format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportFormat {
    pub bits: ExportBits,
    pub rate: u32,
}

impl ExportFormat {
    /// "24-bit, 44.1 kHz"
    pub fn words(self) -> String {
        let bits = match self.bits {
            ExportBits::I24 => "24-bit",
            ExportBits::I16 => "16-bit",
            ExportBits::F32 => "32-bit float",
        };
        format!("{bits}, {} kHz", f64::from(self.rate) / 1000.0)
    }
}

/// What the app tells the panel as it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportSetup {
    /// The arrangement's length in words ("3:50, 96 bars"), or None when
    /// the song has no arrangement.
    pub arrangement: Option<String>,
    /// The bar the playhead is in (from 1): where Bars starts.
    pub from_bar: u32,
    /// How it will play, in words: "Fast, quiet: faster than real time,
    /// the speakers quiet", or "Listen, because ...".
    pub mode: String,
    /// The tracks and returns that may get a stem.
    pub stems: usize,
    /// Where the files go, as shown ("Recordings / My song 2026-11-14
    /// 01.32").
    pub folder: String,
    /// The files' format to start from: Prefs > Export file and rate.
    pub format: ExportFormat,
}

/// Where the panel is.
#[derive(Clone, Debug, PartialEq)]
pub enum ExportPhase {
    /// Picking what to make.
    Ready,
    /// Running: the time done ("1:12 of 3:50") and the part done (0 to 1)
    /// when the length is known.
    Running { words: String, part: Option<f32> },
    /// Over: the outcome's line, and the folder for Show in BROWSE.
    Done { text: String, folder: Option<Loc> },
    /// Stopped (cancelled or failed): why.
    Stopped(String),
}

/// The panel.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportPanel {
    pub setup: ExportSetup,
    pub mixdown: bool,
    pub stems: bool,
    pub midi: bool,
    pub range: ExportRange,
    /// The bar count Bars uses, kept while the arrangement is picked.
    pub bars: u32,
    /// This export's format (Prefs' until changed here).
    pub format: ExportFormat,
    pub phase: ExportPhase,
}

/// What the panel's touches (and the hardware) mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportInput {
    Mixdown,
    Stems,
    Midi,
    Arrangement,
    Bars,
    /// One bar fewer or more.
    Fewer,
    More,
    Start,
    Cancel,
    Close,
    /// The finished export's folder, in the browser.
    Show,
    /// The files' format: 24-bit, 16-bit, 32-bit float, 44.1 or 48 kHz.
    Bits24,
    Bits16,
    Float32,
    Rate44,
    Rate48,
}

/// What Start asks the app for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportChoice {
    pub mixdown: bool,
    pub stems: bool,
    pub midi: bool,
    pub range: ExportRange,
    pub format: ExportFormat,
}

impl ExportPanel {
    /// A panel ready to start: everything ticked (or as `last` had it), the
    /// arrangement when there is one, otherwise 8 bars, and Prefs' format
    /// (a format changed on the panel was for that export only).
    pub fn new(setup: ExportSetup, last: Option<ExportChoice>) -> ExportPanel {
        let (mixdown, stems, midi, bars) = match last {
            Some(c) => (
                c.mixdown,
                c.stems,
                c.midi,
                match c.range {
                    ExportRange::Bars(n) => n,
                    ExportRange::Arrangement => DEFAULT_BARS,
                },
            ),
            None => (true, true, true, DEFAULT_BARS),
        };
        let range = match (&setup.arrangement, last.map(|c| c.range)) {
            (Some(_), Some(ExportRange::Bars(n))) => ExportRange::Bars(n),
            (Some(_), _) => ExportRange::Arrangement,
            (None, _) => ExportRange::Bars(bars),
        };
        let format = setup.format;
        ExportPanel {
            setup,
            mixdown,
            stems,
            midi,
            range,
            bars,
            format,
            phase: ExportPhase::Ready,
        }
    }

    /// What Start would ask for.
    pub fn choice(&self) -> ExportChoice {
        ExportChoice {
            mixdown: self.mixdown,
            stems: self.stems,
            midi: self.midi,
            range: self.range,
            format: self.format,
        }
    }

    /// Something is ticked, so Start can go.
    pub fn can_start(&self) -> bool {
        self.phase == ExportPhase::Ready && (self.mixdown || self.stems || self.midi)
    }

    pub fn running(&self) -> bool {
        matches!(self.phase, ExportPhase::Running { .. })
    }

    /// The range in words, for the panel.
    pub fn range_words(&self) -> String {
        match self.range {
            ExportRange::Arrangement => format!(
                "The arrangement ({})",
                self.setup.arrangement.as_deref().unwrap_or("none")
            ),
            ExportRange::Bars(n) => {
                let to = self.setup.from_bar + n - 1;
                format!(
                    "{n} bar{} of the clips playing, bars {} to {to}",
                    if n == 1 { "" } else { "s" },
                    self.setup.from_bar
                )
            }
        }
    }

    /// Steps the bar count (the wheel, or - / + on the panel), and picks
    /// Bars.
    pub fn step_bars(&mut self, delta: i32) {
        let n = (i64::from(self.bars) + i64::from(delta)).clamp(1, i64::from(MAX_BARS)) as u32;
        self.bars = n;
        self.range = ExportRange::Bars(n);
    }

    /// One of the panel's touches, while it is ready (Start, Cancel, Close
    /// and Show are the browser's: [`crate::Browser`] handles them).
    pub fn pick(&mut self, i: ExportInput) {
        if self.phase != ExportPhase::Ready {
            return;
        }
        match i {
            ExportInput::Mixdown => self.mixdown = !self.mixdown,
            ExportInput::Stems => self.stems = !self.stems,
            ExportInput::Midi => self.midi = !self.midi,
            ExportInput::Arrangement if self.setup.arrangement.is_some() => {
                self.range = ExportRange::Arrangement
            }
            ExportInput::Bars => self.range = ExportRange::Bars(self.bars),
            ExportInput::Fewer => self.step_bars(-1),
            ExportInput::More => self.step_bars(1),
            ExportInput::Bits24 => self.format.bits = ExportBits::I24,
            ExportInput::Bits16 => self.format.bits = ExportBits::I16,
            ExportInput::Float32 => self.format.bits = ExportBits::F32,
            ExportInput::Rate44 => self.format.rate = EXPORT_RATES[0],
            ExportInput::Rate48 => self.format.rate = EXPORT_RATES[1],
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(arr: bool) -> ExportSetup {
        ExportSetup {
            arrangement: arr.then(|| "3:50, 96 bars".to_string()),
            from_bar: 5,
            mode: "Fast, quiet".into(),
            stems: 4,
            folder: "Recordings / Song 2026-11-14 01.32".into(),
            format: ExportFormat {
                bits: ExportBits::I24,
                rate: 44_100,
            },
        }
    }

    #[test]
    fn the_range_follows_the_song() {
        let p = ExportPanel::new(setup(true), None);
        assert_eq!(p.range, ExportRange::Arrangement);
        assert!(p.mixdown && p.stems && p.midi && p.can_start());
        assert_eq!(p.range_words(), "The arrangement (3:50, 96 bars)");
        let mut p = ExportPanel::new(setup(false), None);
        assert_eq!(p.range, ExportRange::Bars(8));
        assert_eq!(p.range_words(), "8 bars of the clips playing, bars 5 to 12");
        p.step_bars(-20);
        assert_eq!(p.range, ExportRange::Bars(1));
        assert_eq!(p.range_words(), "1 bar of the clips playing, bars 5 to 5");
        p.step_bars(5000);
        assert_eq!(p.range, ExportRange::Bars(MAX_BARS));
        // With an arrangement, Bars keeps its count, and - / + pick it.
        let mut p = ExportPanel::new(setup(true), None);
        p.pick(ExportInput::More);
        assert_eq!(p.range, ExportRange::Bars(9));
        p.pick(ExportInput::Arrangement);
        assert_eq!(p.range, ExportRange::Arrangement);
        p.pick(ExportInput::Bars);
        assert_eq!(p.range, ExportRange::Bars(9));
        p.pick(ExportInput::Stems);
        assert!(!p.stems);
        // No arrangement: it can't be picked.
        let mut p = ExportPanel::new(setup(false), None);
        p.pick(ExportInput::Arrangement);
        assert_eq!(p.range, ExportRange::Bars(8));
    }

    #[test]
    fn the_format_starts_as_prefs_has_it_and_changes_for_one_export() {
        let mut p = ExportPanel::new(setup(true), None);
        assert_eq!(p.format.words(), "24-bit, 44.1 kHz");
        p.pick(ExportInput::Bits16);
        p.pick(ExportInput::Rate48);
        assert_eq!(p.choice().format.words(), "16-bit, 48 kHz");
        p.pick(ExportInput::Float32);
        assert_eq!(p.choice().format.words(), "32-bit float, 48 kHz");
        // The next export starts from Prefs again.
        let last = p.choice();
        let p = ExportPanel::new(setup(true), Some(last));
        assert_eq!(p.format, setup(true).format);
        assert_eq!(p.choice().format.bits, ExportBits::I24);
    }

    #[test]
    fn the_last_choice_is_kept() {
        let last = ExportChoice {
            mixdown: true,
            stems: false,
            midi: false,
            range: ExportRange::Bars(16),
            format: setup(false).format,
        };
        let p = ExportPanel::new(setup(false), Some(last));
        assert_eq!(p.choice(), last);
        let p = ExportPanel::new(setup(true), Some(last));
        assert_eq!(p.range, ExportRange::Bars(16));
        let last = ExportChoice {
            range: ExportRange::Arrangement,
            ..last
        };
        let p = ExportPanel::new(setup(false), Some(last));
        assert_eq!(p.range, ExportRange::Bars(8), "no arrangement now");
        let mut p = ExportPanel::new(setup(true), None);
        p.pick(ExportInput::Mixdown);
        p.pick(ExportInput::Stems);
        p.pick(ExportInput::Midi);
        assert!(!p.can_start());
        p.phase = ExportPhase::Running {
            words: "0:01".into(),
            part: None,
        };
        p.pick(ExportInput::Midi);
        assert!(!p.midi, "nothing changes while it runs");
    }
}
