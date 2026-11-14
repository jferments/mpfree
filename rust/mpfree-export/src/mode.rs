//! Fast or Listen: how an export plays the song, and when it has to
//! listen whatever Prefs says.
//!
//! Fast is the default (Jay, 14 Nov 2026 01:32Z): the song renders faster
//! than real time with the speakers quiet, as most DAWs export. Listen
//! plays it once in real time, as you hear it. An export listens anyway
//! when the song uses something that only exists in real time: outside
//! gear played over MIDI or CV, sound coming in at the line input, gear
//! following MPFree's clock, a part made on another computer, or plugins
//! (until the plugin host can wait for a late block instead of dropping
//! it).

/// How an export plays the song (Prefs > Export).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Faster than real time, the speakers quiet.
    #[default]
    Fast,
    /// In real time, heard as it records.
    Listen,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Fast, Mode::Listen];

    /// The Prefs row's words.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Fast => "Fast, quiet",
            Mode::Listen => "Listen",
        }
    }

    /// One word, as the setting is saved.
    pub fn key(self) -> &'static str {
        match self {
            Mode::Fast => "fast",
            Mode::Listen => "listen",
        }
    }

    pub fn from_key(key: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.key() == key.trim())
    }

    /// What the Prefs row's panel says about it.
    pub fn about(self) -> &'static str {
        match self {
            Mode::Fast => {
                "Export renders the song faster than real time, with the speakers quiet. \
                 A song that plays outside gear exports in real time anyway."
            }
            Mode::Listen => {
                "Export plays the song once in real time and you hear it as it records. \
                 You can ride the faders while it does."
            }
        }
    }
}

/// What the song uses that only exists in real time, as the app finds it
/// when Export starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rig {
    /// Tracks that play an instrument on MIDI out, by name.
    pub external_midi: Vec<String>,
    /// The INPUTS on the back are on: whatever plays into them is in the
    /// mix.
    pub line_input: bool,
    /// CV or gate jacks in use.
    pub cv: bool,
    /// MIDI clock or start and stop sent out, to gear that follows it.
    pub clock_out: bool,
    /// A part of the sound made on another computer (offload).
    pub offload: bool,
    /// Plugins in the song, by name.
    pub plugins: Vec<String>,
}

/// Why an export plays as it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// Prefs > Export says so.
    Chosen,
    /// The track (its name) plays outside gear over MIDI.
    ExternalMidi(String),
    LineInput,
    Cv,
    ClockOut,
    Offload,
    /// The first plugin's name.
    Plugins(String),
}

impl Why {
    /// A line for the Export panel.
    pub fn words(&self, mode: Mode) -> String {
        match self {
            Why::Chosen => match mode {
                Mode::Fast => "Fast: faster than real time, the speakers quiet".into(),
                Mode::Listen => "Listening: in real time, as Prefs > Export says".into(),
            },
            Why::ExternalMidi(t) => format!(
                "Listening: {t} plays outside gear over MIDI, which only plays in real time"
            ),
            Why::LineInput => {
                "Listening: the line input is on, and what comes in there only plays in real time"
                    .into()
            }
            Why::Cv => {
                "Listening: the CV and gate jacks play outside gear, which only plays in real time"
                    .into()
            }
            Why::ClockOut => {
                "Listening: MIDI clock goes out to gear that follows it in real time".into()
            }
            Why::Offload => {
                "Listening: part of the sound is made on another computer, in real time".into()
            }
            Why::Plugins(p) => {
                format!("Listening: plugins ({p}) can't render faster than real time yet")
            }
        }
    }
}

/// How an export plays: the preference, unless the song uses something
/// that only exists in real time. The first reason found is given.
pub fn choose(pref: Mode, rig: &Rig) -> (Mode, Why) {
    if pref == Mode::Listen {
        return (Mode::Listen, Why::Chosen);
    }
    let why = if let Some(t) = rig.external_midi.first() {
        Why::ExternalMidi(t.clone())
    } else if rig.line_input {
        Why::LineInput
    } else if rig.cv {
        Why::Cv
    } else if rig.clock_out {
        Why::ClockOut
    } else if rig.offload {
        Why::Offload
    } else if let Some(p) = rig.plugins.first() {
        Why::Plugins(p.clone())
    } else {
        return (Mode::Fast, Why::Chosen);
    };
    (Mode::Listen, why)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_unless_the_song_needs_real_time() {
        assert_eq!(Mode::default(), Mode::Fast);
        let none = Rig::default();
        assert_eq!(choose(Mode::Fast, &none), (Mode::Fast, Why::Chosen));
        assert_eq!(choose(Mode::Listen, &none), (Mode::Listen, Why::Chosen));
        let each = [
            Rig {
                external_midi: vec!["Bass".into()],
                ..Rig::default()
            },
            Rig {
                line_input: true,
                ..Rig::default()
            },
            Rig {
                cv: true,
                ..Rig::default()
            },
            Rig {
                clock_out: true,
                ..Rig::default()
            },
            Rig {
                offload: true,
                ..Rig::default()
            },
            Rig {
                plugins: vec!["Surge XT".into()],
                ..Rig::default()
            },
        ];
        for rig in &each {
            let (mode, why) = choose(Mode::Fast, rig);
            assert_eq!(mode, Mode::Listen, "{rig:?}");
            assert_ne!(why, Why::Chosen);
            assert!(why.words(mode).starts_with("Listening: "));
            // Listen chosen is Listen, whatever the song has.
            assert_eq!(choose(Mode::Listen, rig), (Mode::Listen, Why::Chosen));
        }
        let (_, why) = choose(Mode::Fast, &each[0]);
        assert_eq!(
            why.words(Mode::Listen),
            "Listening: Bass plays outside gear over MIDI, which only plays in real time"
        );
    }

    #[test]
    fn outside_gear_is_named_before_the_rest() {
        let rig = Rig {
            external_midi: vec!["Keys".into(), "Bass".into()],
            line_input: true,
            plugins: vec!["Vital".into()],
            ..Rig::default()
        };
        assert_eq!(
            choose(Mode::Fast, &rig),
            (Mode::Listen, Why::ExternalMidi("Keys".into()))
        );
    }

    #[test]
    fn the_setting_reads_back() {
        for m in Mode::ALL {
            assert_eq!(Mode::from_key(m.key()), Some(m));
            assert!(!m.name().is_empty() && !m.about().is_empty());
        }
        assert_eq!(Mode::from_key(" listen\n"), Some(Mode::Listen));
        assert_eq!(Mode::from_key("loud"), None);
    }
}
