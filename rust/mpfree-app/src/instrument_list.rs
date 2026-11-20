//! The project's list of instruments: step 2 of the instrument redesign
//! ([instruments.md](../../../docs/dev/design/instruments.md)).
//!
//! Instruments make the sound; tracks record and play back to them, and
//! the pads play them. To point at an instrument, a track (and later a
//! knob's owner, a script or a MIDI port) needs a name for it that stays
//! put when tracks are added, moved or taken away: an [`InstrumentId`]. It
//! is not a track number, a mixer channel or a place in a list. A project
//! gives each new instrument the next number, keeps that counter in its
//! file, and never gives a number out again once its instrument is gone,
//! so something that named an old instrument can't reach a new one by
//! mistake.
//!
//! Step 2a: the list holds one instrument, the shared synth (the pads'
//! synth, which tracks on the Synth kit play), as instrument 1 in every
//! project. Its sound stays in the synth part: the list says what an
//! instrument is, not how it sounds now or whether it is playing (the
//! shared synth's running engine is `synth::Synth`, in the audio loop).
//! The tracks' own instruments join the list in step 2b, and tracks get
//! lasting ids in step 2c.
//!
//! A list is made and read on the screen thread (`project::opening`),
//! moved into the audio loop whole as a project opens, and the loop's old
//! one goes back to be freed there: the loop never makes or frees one.

use std::fmt;

/// The instrument list part's first line.
pub const FORMAT: &str = "mpfree-instrument-list 1";

/// An instrument's lasting number in its project, from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstrumentId(u32);

impl InstrumentId {
    /// The shared synth: instrument 1 in every project.
    pub const SHARED_SYNTH: InstrumentId = InstrumentId(1);

    /// Instrument `n` (None for 0: numbers start at 1).
    pub fn from_number(n: u32) -> Option<InstrumentId> {
        (n >= 1).then_some(InstrumentId(n))
    }
}

impl fmt::Display for InstrumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What an instrument of the list is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The pads' synth, whichever engine the Synth tab plays. Its sound is
    /// the project's synth part, and it sounds on the Synth strip.
    SharedSynth,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::SharedSynth => "shared-synth",
        }
    }

    fn from_word(w: &str) -> Option<Kind> {
        [Kind::SharedSynth].into_iter().find(|k| k.word() == w)
    }
}

/// One instrument of the list: its id and what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Listed {
    id: InstrumentId,
    kind: Kind,
}

/// The project's instruments, each with its lasting id, and the number
/// the next one will get.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstrumentList {
    listed: Vec<Listed>,
    /// The id the next instrument gets: past every id the project ever
    /// gave, so none is given twice.
    next: u32,
}

impl Default for InstrumentList {
    fn default() -> InstrumentList {
        InstrumentList::new()
    }
}

impl InstrumentList {
    /// A new project's list, and an old project's (saved before
    /// instruments had ids): the shared synth as instrument 1.
    pub fn new() -> InstrumentList {
        InstrumentList {
            listed: vec![Listed {
                id: InstrumentId::SHARED_SYNTH,
                kind: Kind::SharedSynth,
            }],
            next: 2,
        }
    }

    /// What instrument `id` is, or None if the list hasn't got it.
    pub fn kind(&self, id: InstrumentId) -> Option<Kind> {
        self.listed.iter().find(|l| l.id == id).map(|l| l.kind)
    }

    /// The list as its part's text:
    ///
    /// ```text
    /// mpfree-instrument-list 1
    /// next 2
    /// instrument 1 shared-synth
    /// ```
    pub fn text(&self) -> String {
        use std::fmt::Write as _;
        let mut s = format!("{FORMAT}\nnext {}\n", self.next);
        for l in &self.listed {
            let _ = writeln!(s, "instrument {} {}", l.id, l.kind.word());
        }
        s
    }

    /// Reads a list part ([`InstrumentList::text`]). Refuses a part with
    /// a line it can't read, an id given twice or not below `next`, or
    /// without the shared synth as instrument 1 (the opener then says so
    /// and gives the new project's list).
    pub fn parse(text: &str) -> Result<InstrumentList, String> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        if lines.next() != Some(FORMAT) {
            return Err(format!("not a {FORMAT} part"));
        }
        let (mut listed, mut next) = (Vec::<Listed>::new(), None);
        for line in lines {
            let w: Vec<&str> = line.split_whitespace().collect();
            match w.as_slice() {
                ["next", n] if next.is_none() => match n.parse::<u32>() {
                    Ok(n) if n >= 2 => next = Some(n),
                    _ => return Err(format!("can't read \"{line}\"")),
                },
                ["instrument", n, kind] => {
                    let id = n.parse().ok().and_then(InstrumentId::from_number);
                    let (Some(id), Some(kind)) = (id, Kind::from_word(kind)) else {
                        return Err(format!("can't read \"{line}\""));
                    };
                    if listed.iter().any(|l| l.id == id) {
                        return Err(format!("instrument {id} is listed twice"));
                    }
                    listed.push(Listed { id, kind });
                }
                _ => return Err(format!("can't read \"{line}\"")),
            }
        }
        let Some(next) = next else {
            return Err("no \"next\" line".to_string());
        };
        if let Some(l) = listed.iter().find(|l| l.id.0 >= next) {
            return Err(format!("instrument {} isn't below next ({next})", l.id));
        }
        let synths: Vec<InstrumentId> = listed
            .iter()
            .filter(|l| l.kind == Kind::SharedSynth)
            .map(|l| l.id)
            .collect();
        if synths != [InstrumentId::SHARED_SYNTH] {
            return Err("the shared synth isn't instrument 1".to_string());
        }
        Ok(InstrumentList { listed, next })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new project's list is the shared synth as instrument 1, and its
    /// text reads back to the same list.
    #[test]
    fn a_new_list_is_the_shared_synth_as_instrument_1() {
        let l = InstrumentList::new();
        assert_eq!(l.kind(InstrumentId::SHARED_SYNTH), Some(Kind::SharedSynth));
        assert_eq!(l.kind(InstrumentId(2)), None);
        assert_eq!(
            l.text(),
            "mpfree-instrument-list 1\nnext 2\ninstrument 1 shared-synth\n"
        );
        assert_eq!(InstrumentList::parse(&l.text()), Ok(l));
        assert_eq!(InstrumentId::from_number(0), None);
        assert_eq!(
            InstrumentId::from_number(1),
            Some(InstrumentId::SHARED_SYNTH)
        );
    }

    /// The counter is kept: a list whose project once gave out higher ids
    /// comes back with the same next number, so none is given twice.
    #[test]
    fn the_next_id_is_kept_through_the_text() {
        let text = "mpfree-instrument-list 1\nnext 7\ninstrument 1 shared-synth\n";
        let l = InstrumentList::parse(text).expect("reads");
        assert_eq!(l.next, 7);
        assert_eq!(l.text(), text);
    }

    /// A part that doesn't hold together is refused, with why.
    #[test]
    fn a_list_that_does_not_hold_together_is_refused() {
        let refused = |body: &str| InstrumentList::parse(&format!("{FORMAT}\n{body}")).unwrap_err();
        assert!(InstrumentList::parse("mpfree-instrument-list 2\nnext 2\n").is_err());
        assert!(refused("instrument 1 shared-synth\n").contains("next"));
        assert!(refused("next 2\nnext 3\ninstrument 1 shared-synth\n").contains("can't read"));
        assert!(refused("next 1\ninstrument 1 shared-synth\n").contains("can't read"));
        assert!(refused("next 3\n").contains("shared synth"));
        assert!(refused("next 3\ninstrument 2 shared-synth\n").contains("shared synth"));
        assert!(refused("next 2\ninstrument 0 shared-synth\n").contains("can't read"));
        assert!(refused("next 2\ninstrument 1 kazoo\n").contains("can't read"));
        assert!(
            refused("next 3\ninstrument 1 shared-synth\ninstrument 1 shared-synth\n")
                .contains("twice")
        );
        assert!(
            refused("next 2\ninstrument 1 shared-synth\ninstrument 2 shared-synth\n")
                .contains("below next")
        );
        assert!(refused("next 2\ninstrument 1 shared-synth\nhello\n").contains("can't read"));
    }
}
