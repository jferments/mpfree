//! Make a phrase of your own (the Yours shelf, step 3 of the composing
//! plan's phrases): notes from a clip or a MIDI file become a phrase the
//! arp's phrase player plays from any note, as MPFree's own do.
//!
//! What the notes say decides how the phrase is written, so nothing needs
//! asking:
//!
//! - **The key** is the listener's reading ([`listen::key`]): its
//!   likeliest, or that one's relative major or minor when the phrase
//!   starts on its tonic. Its tonic is the home note the phrase is written from, and
//!   its mode the scale its steps count in: major, Mixolydian and Lydian
//!   write in major; minor, Dorian and Phrygian in minor. No reading
//!   writes for either, from the first note.
//! - **The home note** is that tonic just above the low end of the part
//!   (at or under its lowest quarter, a bass line's lower half, and four
//!   half steps), so a bass line
//!   on C2 that drops to C1 is written from C2, a riff around C4 from C4
//!   and a line climbing from C4 from C4.
//! - **The kind**: chords where most of its starts are two notes or more
//!   together, bass where it sits under E3, an arp line where it runs as a
//!   pulse (two notes a beat or more one at a time, from its first start
//!   to its last end, three in four of them the same distance apart),
//!   else a riff.
//! - **The grid** is the coarsest of 1/16, 1/8T, 1/16T and 1/32 (and
//!   their 1/32T) that nearly every start sits on; the starts and lengths
//!   are kept to the tick, as played.
//! - **The length** is the clip's, in whole bars from its first, at most
//!   eight ([`MAX_BARS`]).

use mpfree_arp::phrase::{Kind, Written, MAX_BARS};
use mpfree_arp::{Phrase, Rate};
use mpfree_context::key::{Key, Mode};
use mpfree_context::listen::{self, Played};
use mpfree_seq::{Clip, Holds};
use mpfree_transport::time::{Meter, Swing, Tick};

use crate::timing::{heard, Playback};

/// A phrase made from notes, with what was read to write it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Made {
    pub phrase: Phrase,
    /// The key the notes were heard in (None: no clear key).
    pub key: Option<Key>,
    /// The note the phrase is written from.
    pub home: u8,
}

/// Why no phrase was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MakeRefused {
    /// There are no notes.
    NoNotes,
    /// The meter isn't counted in quarter notes (6/8, 7/8): a phrase's
    /// bars are beats of a quarter.
    Meter,
}

impl MakeRefused {
    pub const fn words(self) -> &'static str {
        match self {
            MakeRefused::NoNotes => "No notes to make a phrase of",
            MakeRefused::Meter => {
                "A phrase counts its bars in quarter notes: this one is in another meter"
            }
        }
    }
}

/// Starts this close count as together (a chord) or on a grid line.
const TOGETHER: Tick = 30;

/// The grids a phrase may be written on, coarsest first.
const GRIDS: [Rate; 6] = [
    Rate::Sixteenth,
    Rate::EighthTriplet,
    Rate::SixteenthTriplet,
    Rate::ThirtySecond,
    Rate::ThirtySecondTriplet,
    Rate::Eighth,
];

/// A phrase from a clip's notes (`at` from the clip's start) in `meter`,
/// `len` ticks long (0: as long as its notes reach). See the module docs.
pub fn phrase_of(notes: &[Played], len: Tick, meter: Meter) -> Result<Made, MakeRefused> {
    phrase_of_as(notes, len, meter, Asked::default())
}

/// What the musician changed of what the notes say (the Make a phrase
/// sheet's Kind and Home rows): None keeps what was read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Asked {
    pub kind: Option<Kind>,
    /// The note a held pad plays the phrase from. The phrase keeps the
    /// scale it was heard in, so a home off the key's tonic counts its
    /// steps from there.
    pub home: Option<u8>,
}

/// [`phrase_of`], with the kind or the home note as asked.
pub fn phrase_of_as(
    notes: &[Played],
    len: Tick,
    meter: Meter,
    asked: Asked,
) -> Result<Made, MakeRefused> {
    if meter.unit != 4 {
        return Err(MakeRefused::Meter);
    }
    if notes.is_empty() {
        return Err(MakeRefused::NoNotes);
    }
    let bar = meter.bar_ticks().max(1);
    let first = notes.iter().map(|n| n.at).min().unwrap_or(0);
    let from = first / bar * bar;
    let until = from + Tick::from(MAX_BARS) * bar;
    let mut kept: Vec<Played> = notes
        .iter()
        .filter(|n| n.at >= from && n.at < until)
        .map(|n| Played {
            at: n.at - from,
            ..*n
        })
        .collect();
    kept.sort_by_key(|n| (n.at, n.pitch));
    let key = key_of(&kept, meter);
    let kind = asked.kind.unwrap_or_else(|| kind_of(&kept, meter));
    let home = asked.home.unwrap_or_else(|| home_of(&kept, kind, key));
    let written = match key.map(|k| k.mode) {
        Some(Mode::Major | Mode::Mixolydian | Mode::Lydian) => Written::Major,
        Some(Mode::Minor | Mode::Dorian | Mode::Phrygian) => Written::Minor,
        None => Written::Either,
    };
    let unit = grid_of(&kept);
    let tuples: Vec<(u32, u32, u8, u8)> = kept
        .iter()
        .map(|n| (n.at as u32, n.len.max(1) as u32, n.pitch, n.velocity))
        .collect();
    let beats = meter.beats.clamp(1, 12);
    let mut phrase = Phrase::from_notes(&tuples, home, written, unit, beats);
    phrase.kind = kind;
    let reach = kept.iter().map(|n| n.at + 1).max().unwrap_or(1);
    let span = if len > from {
        (len - from).max(reach)
    } else {
        reach
    };
    phrase.bars = ((span + bar - 1) / bar).clamp(1, Tick::from(MAX_BARS)) as u8;
    Ok(Made { phrase, key, home })
}

/// A clip's notes as one pass of it plays them, on the song's beats (after
/// its quantise and speed, before swing), and that pass's length in song
/// ticks: what [`phrase_of`] reads from a clip. A clip of pad hits has no
/// notes to it (a pad's pitch is the kit's), nor has a loop's cell.
pub fn clip_played(c: &Clip) -> (Vec<Played>, Tick) {
    let len = c.speed.to_song(c.length as f64).round() as Tick;
    if c.holds != Holds::Notes || c.looped.is_some() {
        return (Vec::new(), len);
    }
    let pb = Playback {
        quantise: c.quantise,
        speed: c.speed,
        swing: Swing::default(),
        length: c.length,
        start: 0,
    };
    let mut notes: Vec<Played> = c
        .notes
        .iter()
        .filter(|n| n.on)
        .map(|n| {
            Played::new(
                heard(n.tick, &pb).round() as Tick,
                c.speed.to_song(n.len as f64).round() as Tick,
                n.note,
                n.velocity,
            )
        })
        .collect();
    notes.sort_by_key(|n| (n.at, n.pitch));
    (notes, len)
}

/// Chords where most starts are two notes or more together; bass under
/// E3; an arp line at two notes a beat or more; else a riff.
fn kind_of(notes: &[Played], meter: Meter) -> Kind {
    let starts = groups(notes);
    let stacked = starts.iter().filter(|g| g.len() >= 2).count();
    if stacked * 2 > starts.len() {
        return Kind::Chords;
    }
    let (sum, weight) = notes.iter().fold((0.0, 0.0), |(s, w), n| {
        let l = n.len.max(1) as f64;
        (s + f64::from(n.pitch) * l, w + l)
    });
    if sum / weight.max(1.0) < 52.0 {
        return Kind::Bass;
    }
    // Over the time from its first start to its last end.
    let first = notes.first().map_or(0, |n| n.at);
    let end = notes.iter().map(|n| n.at + n.len.max(1)).max().unwrap_or(0);
    let beats = ((end - first) as f64 / meter.beat_ticks().max(1) as f64).max(1.0);
    // A line is a pulse: most of its starts the same distance apart.
    let gaps: Vec<Tick> = starts.windows(2).map(|w| w[1][0].at - w[0][0].at).collect();
    let pulse = gaps
        .iter()
        .map(|g| gaps.iter().filter(|h| (*h - g).abs() <= TOGETHER).count())
        .max()
        .unwrap_or(0);
    if starts.len() as f64 / beats >= 2.0 && pulse * 4 >= gaps.len() * 3 {
        Kind::Line
    } else {
        Kind::Riff
    }
}

/// The notes grouped by start: those within [`TOGETHER`] of the first of
/// a group join it. `notes` are in order of start.
fn groups(notes: &[Played]) -> Vec<Vec<Played>> {
    let mut out: Vec<Vec<Played>> = Vec::new();
    for n in notes {
        match out.last_mut() {
            Some(g) if n.at - g[0].at <= TOGETHER => g.push(*n),
            _ => out.push(vec![*n]),
        }
    }
    out
}

/// The home note: the key's tonic just above the low end of the part, at
/// or under its lowest quarter (a bass line's lower half; by how long each
/// note sounds; chords: of their lowest notes) and four half steps more, so a bass line on C2
/// that drops to C1 is written from C2, a riff around C4 from C4 and a
/// line that climbs from C4 to C6 from C4. With no reading, the first note
/// (the lowest of a chord).
fn home_of(notes: &[Played], kind: Kind, key: Option<Key>) -> u8 {
    let first = notes.first().map_or(60, |n| n.pitch);
    let Some(key) = key else {
        return first;
    };
    let mut line: Vec<Played> = if kind == Kind::Chords {
        groups(notes)
            .into_iter()
            .filter_map(|g| g.into_iter().min_by_key(|n| n.pitch))
            .collect()
    } else {
        notes.to_vec()
    };
    line.sort_by_key(|n| n.pitch);
    let total: f64 = line.iter().map(|n| n.len.max(1) as f64).sum();
    let mut run = 0.0;
    // A bass line's root is its middle; a part above, its low end.
    let share = if kind == Kind::Bass { 2.0 } else { 4.0 };
    let low = line
        .iter()
        .find(|n| {
            run += n.len.max(1) as f64;
            run >= total / share
        })
        .map_or(first, |n| n.pitch);
    let top = i32::from(low) + 4;
    let home = top - (top - i32::from(key.tonic % 12)).rem_euclid(12);
    home.clamp(0, 127) as u8
}

/// The key the phrase is written in: the listener's likeliest, or its
/// relative major or minor (the same notes) when the phrase starts on
/// that one's tonic (its first note, the lowest of a chord), which a short
/// phrase is often read the wrong way round on. None for no reading.
fn key_of(notes: &[Played], meter: Meter) -> Option<Key> {
    let reading = listen::key(notes, meter);
    let best = reading.best()?;
    let start = notes
        .iter()
        .take_while(|n| n.at - notes[0].at <= TOGETHER)
        .map(|n| n.pitch % 12)
        .min()?;
    let relative = reading
        .candidates
        .iter()
        .flatten()
        .map(|c| c.key)
        .find(|k| k.tonic % 12 == start && k.pcs() == best.pcs());
    Some(relative.unwrap_or(best))
}

/// The coarsest grid nearly every start (all but one in ten) sits on.
fn grid_of(notes: &[Played]) -> Rate {
    let fits = |r: Rate| {
        let g = Tick::from(r.ticks());
        let on = notes
            .iter()
            .filter(|n| {
                let off = n.at % g;
                off <= TOGETHER || g - off <= TOGETHER
            })
            .count();
        on * 10 >= notes.len() * 9
    };
    // 1/8 first when every start is on one; else the list's order.
    if fits(Rate::Eighth) {
        return Rate::Eighth;
    }
    GRIDS
        .into_iter()
        .find(|r| fits(*r))
        .unwrap_or(Rate::Sixteenth)
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: Tick = 960;
    const FOUR: Meter = Meter { beats: 4, unit: 4 };

    fn n(at: Tick, len: Tick, pitch: u8) -> Played {
        Played::new(at, len, pitch, 100)
    }

    /// A C minor bass line from C2 is a bass phrase written in minor from
    /// C2, on 1/8s, one bar; held on G it plays from G.
    #[test]
    fn a_bass_line_is_written_from_the_tonic_under_it() {
        let line = [
            n(0, Q / 2, 36),
            n(Q, Q / 2, 36),
            n(3 * Q / 2, Q / 2, 43),
            n(2 * Q, Q / 2, 39),
            n(3 * Q, Q / 2, 36),
            n(7 * Q / 2, Q / 2, 46),
        ];
        let made = phrase_of(&line, 4 * Q, FOUR).unwrap();
        let p = made.phrase;
        assert_eq!(
            made.key.map(|k| (k.tonic % 12, k.mode)),
            Some((0, Mode::Minor))
        );
        assert_eq!(made.home, 36);
        assert_eq!(
            (p.kind, p.written, p.unit, p.bars),
            (Kind::Bass, Written::Minor, Rate::Eighth, 1)
        );
        assert_eq!(p.len(), line.len());
        let semis: Vec<i8> = p.notes().iter().map(|n| n.semis).collect();
        assert_eq!(semis, [0, 0, 7, 3, 0, 10]);
        // The sheet's Kind and Home as asked: a riff from C3 is the same
        // notes an octave under its home.
        let asked = Asked {
            kind: Some(Kind::Riff),
            home: Some(48),
        };
        let made = phrase_of_as(&line, 4 * Q, FOUR, asked).unwrap();
        assert_eq!((made.home, made.phrase.kind), (48, Kind::Riff));
        let semis: Vec<i8> = made.phrase.notes().iter().map(|n| n.semis).collect();
        assert_eq!(semis, [-12, -12, -5, -9, -12, -2]);
    }

    /// A clip is read as it plays: at half speed its notes come twice as
    /// late and last twice as long, its pass is twice its own length, and
    /// a clip of pad hits gives no notes.
    #[test]
    fn a_clip_is_read_as_it_plays() {
        use mpfree_seq::{Clip, Holds, Note, Speed};
        let mut c = Clip::new("Clip 1", 4 * Q);
        c.holds = Holds::Notes;
        c.notes = vec![Note::new(Q, Q / 2, 40, 90), Note::new(0, Q / 2, 36, 100)];
        c.speed = Speed::new(1, 2).unwrap();
        let (notes, len) = clip_played(&c);
        assert_eq!(len, 8 * Q);
        assert_eq!(
            notes,
            [Played::new(0, Q, 36, 100), Played::new(2 * Q, Q, 40, 90)]
        );
        c.holds = Holds::Hits;
        assert!(clip_played(&c).0.is_empty());
    }

    /// Stabs of three notes together are chords; a run of 1/16s is an
    /// arp line; a slower tune up high is a riff.
    #[test]
    fn what_the_notes_do_names_the_kind() {
        let stabs: Vec<Played> = (0..4)
            .flat_map(|b| [60, 63, 67].map(|p| n(b * Q, Q / 4, p)))
            .collect();
        assert_eq!(
            phrase_of(&stabs, 4 * Q, FOUR).unwrap().phrase.kind,
            Kind::Chords
        );
        let run: Vec<Played> = (0..16)
            .map(|i| n(i * Q / 4, Q / 8, [60, 63, 67, 72][i as usize % 4]))
            .collect();
        let made = phrase_of(&run, 4 * Q, FOUR).unwrap();
        assert_eq!(
            (made.phrase.kind, made.phrase.unit),
            (Kind::Line, Rate::Sixteenth)
        );
        let tune = [
            n(0, Q, 72),
            n(Q, Q / 2, 70),
            n(3 * Q / 2, Q / 2, 67),
            n(2 * Q, 2 * Q, 72),
        ];
        assert_eq!(
            phrase_of(&tune, 4 * Q, FOUR).unwrap().phrase.kind,
            Kind::Riff
        );
    }

    /// The phrase is the clip's length in whole bars from its first note's
    /// bar, at most eight; triplets are written on their grid.
    #[test]
    fn the_length_and_the_grid_follow_the_clip() {
        let late = [
            n(4 * Q, Q / 3, 60),
            n(4 * Q + Q / 3, Q / 3, 62),
            n(4 * Q + 2 * Q / 3, Q / 3, 64),
        ];
        let made = phrase_of(&late, 12 * Q, FOUR).unwrap();
        assert_eq!(made.phrase.bars, 2, "from bar 2 to the clip's end");
        assert_eq!(made.phrase.notes()[0].start, 0);
        assert_eq!(made.phrase.unit, Rate::EighthTriplet);
        let long: Vec<Played> = (0..40).map(|b| n(b * 4 * Q, Q, 60)).collect();
        assert_eq!(
            phrase_of(&long, 160 * Q, FOUR).unwrap().phrase.bars,
            MAX_BARS
        );
    }

    /// No notes, or a meter not in quarters, make nothing.
    #[test]
    fn no_notes_or_another_meter_is_refused() {
        assert_eq!(
            phrase_of(&[], 4 * Q, FOUR).unwrap_err(),
            MakeRefused::NoNotes
        );
        let six = Meter { beats: 6, unit: 8 };
        assert_eq!(
            phrase_of(&[n(0, Q, 60)], 3 * Q, six).unwrap_err(),
            MakeRefused::Meter
        );
    }
}

#[cfg(test)]
mod measure {
    use super::*;
    use mpfree_arp::Library;

    /// MPFree's 64 phrases played as notes from C (bass from C2, the
    /// others from C4) and made into phrases again: how many come back as
    /// their kind, from the right home, with the same steps. `cargo test
    /// -p mpfree-compose --lib print_the_64_made_again -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore]
    fn print_the_64_made_again() {
        let lib = Library::built_in();
        let (mut kind, mut home, mut same, mut all) = (0, 0, 0, 0);
        for (name, p) in lib.phrases() {
            let from: u8 = if p.kind == Kind::Bass { 36 } else { 60 };
            let notes: Vec<Played> = p
                .notes()
                .iter()
                .map(|n| {
                    let pitch = (i16::from(from) + i16::from(n.semis)).clamp(0, 127) as u8;
                    Played::new(Tick::from(n.start), Tick::from(n.len), pitch, n.velocity)
                })
                .collect();
            let meter = Meter {
                beats: p.beats,
                unit: 4,
            };
            let made = phrase_of(&notes, Tick::from(p.ticks()), meter).unwrap();
            all += 1;
            kind += usize::from(made.phrase.kind == p.kind);
            home += usize::from(made.home == from);
            let semis = |q: &Phrase| q.notes().iter().map(|n| n.semis).collect::<Vec<_>>();
            same += usize::from(made.home == from && semis(&made.phrase) == semis(p));
            if made.phrase.kind != p.kind || made.home != from {
                println!(
                    "{name}: {:?} from {} (was {:?} from {from})",
                    made.phrase.kind, made.home, p.kind
                );
            }
        }
        println!("{all} phrases: kind {kind}, home {home}, same notes {same}");
    }
}
