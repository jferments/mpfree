//! Map to keys: a kit's pads made into one keygroup instrument, each pad at
//! its sound's own note, as the MPC's keygroup auto-map does with a folder
//! of samples named by their notes.
//!
//! A pad's note comes from, in order: the root of a keygroup it already is
//! (someone set it); a note in its sound's name (`Piano C3`, `Rhodes_F#2`,
//! `Strings-Bb4-ff`); the pitch heard in the sound. When the name and the
//! pitch agree on the note but not the octave (libraries number octaves
//! from C3 or from C4 for middle C), the octave heard wins, so a library
//! that numbers from C3 lands where it sounds. [`Project::map_to_keys`]
//! then spreads the zones halfway to each neighbour.
//!
//! [`Project::map_to_keys`]: crate::model::Project::map_to_keys

/// Where a pad's note came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootFrom {
    /// It was a keygroup already: its root stays.
    Keys,
    /// A note in the sound's name, the pitch heard telling nothing else.
    Name,
    /// A note in the sound's name, in the octave the pitch is heard in.
    NameAndPitch,
    /// The pitch heard in the sound.
    Pitch,
}

impl RootFrom {
    pub fn label(self) -> &'static str {
        match self {
            RootFrom::Keys => "its keygroup",
            RootFrom::Name => "its name",
            RootFrom::NameAndPitch => "its name and pitch",
            RootFrom::Pitch => "its pitch",
        }
    }
}

/// The MIDI note a sound's name gives (middle C, 60, is C4), if a word of
/// it is a note: a letter A to G, then `#`, `s` (sharp) or `b` (flat), then
/// an octave from -1 to 9, with no letter or digit right before or after
/// (`Piano C3`, `Rhodes_F#2`, `Pad-Eb4-soft`, `bass a1.wav`). The last such
/// word counts, as names end with the note more often than they start
/// with one.
pub fn note_in_name(name: &str) -> Option<u8> {
    let b = name.as_bytes();
    let mut found = None;
    for i in 0..b.len() {
        if i > 0 && b[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        if let Some((note, len)) = note_at(&b[i..]) {
            let after = b.get(i + len);
            if !after.is_some_and(|c| c.is_ascii_alphanumeric()) {
                found = Some(note);
            }
        }
    }
    found
}

/// A note word at the start of `b`: the note and how many bytes it takes.
fn note_at(b: &[u8]) -> Option<(u8, usize)> {
    let class: i32 = match b.first()?.to_ascii_uppercase() {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let mut i = 1;
    let mut shift = 0;
    match b.get(i) {
        Some(b'#') | Some(b's') | Some(b'S')
            if b.get(i + 1)
                .is_some_and(|c| c.is_ascii_digit() || *c == b'-') =>
        {
            shift = 1;
            i += 1;
        }
        Some(b'b')
            if b.get(i + 1)
                .is_some_and(|c| c.is_ascii_digit() || *c == b'-') =>
        {
            shift = -1;
            i += 1;
        }
        _ => {}
    }
    let minus = b.get(i) == Some(&b'-');
    if minus {
        i += 1;
    }
    let digit = *b.get(i)?;
    if !digit.is_ascii_digit() {
        return None;
    }
    i += 1;
    let octave = if minus {
        if digit != b'1' {
            return None;
        }
        -1
    } else {
        i32::from(digit - b'0')
    };
    let note = (octave + 1) * 12 + class + shift;
    (0..=127).contains(&note).then_some((note as u8, i))
}

/// The note pad's sound is played at: `keys_root` if the pad is a keygroup
/// already, else a note in `name`, else the pitch `heard`, as the module
/// says. None if nothing gives one.
pub fn sound_root(keys_root: Option<u8>, name: &str, heard: Option<u8>) -> Option<(u8, RootFrom)> {
    if let Some(r) = keys_root {
        return Some((r, RootFrom::Keys));
    }
    match (note_in_name(name), heard) {
        (Some(n), Some(h)) if n % 12 == h % 12 => Some((h, RootFrom::NameAndPitch)),
        (Some(n), _) => Some((n, RootFrom::Name)),
        (None, Some(h)) => Some((h, RootFrom::Pitch)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_read_from_names() {
        for (name, note) in [
            ("Piano C4", Some(60)),
            ("Piano C3.wav", Some(48)),
            ("Rhodes_F#2", Some(42)),
            ("Pad-Eb4-soft", Some(63)),
            ("Strings Bb4 ff", Some(70)),
            ("bass a1", Some(33)),
            ("Sub Cs1", Some(25)),
            ("low C-1", Some(0)),
            ("top G9", Some(127)),
            ("Piano C3 then E3", Some(52)),
            ("Kick 58", None),
            ("Break 92", None),
            ("BD", None),
            ("ABC3", None),
            ("C34", None),
            ("Master take 1", None),
            ("G#9", None),
            ("B-2", None),
        ] {
            assert_eq!(note_in_name(name), note, "{name}");
        }
    }

    #[test]
    fn the_note_comes_from_the_keygroup_then_the_name_then_the_pitch() {
        assert_eq!(
            sound_root(Some(50), "Piano C4", Some(60)),
            Some((50, RootFrom::Keys))
        );
        assert_eq!(
            sound_root(None, "Piano C4", None),
            Some((60, RootFrom::Name))
        );
        // Numbered from C3 for middle C: the name says C3, it sounds at 60.
        assert_eq!(
            sound_root(None, "Piano C3", Some(60)),
            Some((60, RootFrom::NameAndPitch))
        );
        // The name and the pitch disagree on the note: the name wins.
        assert_eq!(
            sound_root(None, "Piano C4", Some(62)),
            Some((60, RootFrom::Name))
        );
        assert_eq!(
            sound_root(None, "Lead", Some(57)),
            Some((57, RootFrom::Pitch))
        );
        assert_eq!(sound_root(None, "Kick 58", None), None);
    }
}
