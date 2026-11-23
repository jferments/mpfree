//! `listen --phrases`: every phrase of an arp library as it plays, one
//! file each, for the listen before MPFree's own phrases ship
//! (docs/dev/design/arpeggiator.md, "Phrases").
//!
//! The arp's own engine plays each phrase (`mpfree_arp::Engine` in Phrase
//! mode, In key, the gate as written), with the transport playing, from a
//! note held as a hand holds it: the home note for the first half of the
//! file, then two other notes of the key, so the phrase is heard moving
//! with what is held. A phrase of one or two bars plays for four bars, a
//! longer one twice through (once through from five bars).
//!
//! Around it, so that each is heard as it would be used, play a drummer
//! (the tool's house, boom bap or ride beat, or its waltz in 3/4) and one
//! part for the harmony: a bass on the held note under a riff, chords or
//! a line, and a quiet pad of the held note's chord over a bass phrase.
//! The mix is the tool's ([`super::mix`]): each part set by how loud it
//! sounds under the phrase, a plate and an echo, every file at the same
//! loudness.
//!
//! A phrase is written with no tempo and no sound, so [`HEARD`] gives each
//! of MPFree's the tempo, the beat and the sound it was written with in
//! mind. A phrase that is not listed there (one from your own library
//! file) is heard as its kind usually is.

use super::{
    bass_sound, drummer, json, kit, kit_name, mix, preset, sits, slug, wav, Part,
    Phrase as Session, BLOCK, FILE_LUFS, RATE, TAIL,
};
use mpfree_arp::scale::{MAJOR, MINOR};
use mpfree_arp::{
    Clock, Engine, EventKind, Events, Feel, Library, Mode, Phrase, PhraseKind, Rate, Scale,
    Settings, Written,
};
use mpfree_compose::{Style, GROOVE, JAZZISH, MINIMAL};
use mpfree_context::PartRole;
use mpfree_node::{Node, NodeEvent};
use mpfree_synth::{Analog, Bass, Drawbar, Keys, Pluck, Poly, Squelch, Strings, Wind};
use mpfree_transport::time::{Meter, Swing, Tick, PPQ};
use std::fmt::Write as _;
use std::path::Path;

/// The drummer under a phrase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Beat {
    /// Four on the floor.
    House,
    BoomBap,
    /// Boom bap at half speed, its backbeat on three: under a phrase
    /// whose tempo counts twice as fast as its beat feels (trap, dub, a
    /// montuno in cut time).
    Half,
    /// A jazz ride.
    Ride,
}

impl Beat {
    /// The tool's style whose drummer, kit and bass play this beat.
    fn style(self) -> Style {
        match self {
            Beat::House => MINIMAL,
            Beat::BoomBap | Beat::Half => GROOVE,
            Beat::Ride => JAZZISH,
        }
    }

    fn words(self) -> &'static str {
        match self {
            Beat::House => "four on the floor",
            Beat::BoomBap => "boom bap",
            Beat::Half => "boom bap at half speed",
            Beat::Ride => "ride",
        }
    }
}

/// How one phrase is heard.
#[derive(Clone, Copy, Debug)]
struct Heard {
    name: &'static str,
    bpm: u16,
    beat: Beat,
    /// The instrument and its preset, as `bass: Round Mono`.
    sound: &'static str,
    /// A phrase written for either scale plays in major (else minor).
    major: bool,
    /// Octaves the held notes sit from where the kind's usually do.
    octave: i8,
}

const fn h(name: &'static str, bpm: u16, beat: Beat, sound: &'static str) -> Heard {
    Heard {
        name,
        bpm,
        beat,
        sound,
        major: false,
        octave: 0,
    }
}

impl Heard {
    const fn major(mut self) -> Heard {
        self.major = true;
        self
    }

    const fn low(mut self) -> Heard {
        self.octave = -1;
        self
    }
}

use Beat::{BoomBap, Half, House, Ride};

/// MPFree's phrases as they were written to be heard, in the library's
/// order. Run with the built-in library, the tool says which rows and
/// phrases no longer match.
const HEARD: &[Heard] = &[
    // Bass
    h("Offbeat pump", 124, House, "bass: Round Mono"),
    h("Octave bounce", 120, House, "bass: Synth Pop"),
    h("Rolling sixteenths", 138, House, "bass: Pluck Bass"),
    h("Gallop bass", 148, BoomBap, "bass: Power Bass"),
    h("Sub drop", 140, Half, "bass: Long 808"),
    h("Funk pocket", 104, BoomBap, "bass: Funk Bass"),
    h("Dub steps", 140, Half, "bass: Rubber Bass"),
    h("Night drive", 108, House, "bass: Fat Saw"),
    h("Acid line", 128, House, "squelch: Acid Line"),
    h("Trap glide", 140, Half, "bass: 808 Slide"),
    h("Afro pulse", 112, House, "bass: Soft Triangle"),
    h("Garage skip", 132, BoomBap, "bass: Square Sub"),
    h("Shuffle walk", 126, Ride, "pluck: Upright Bass"),
    h("Waltz bass", 152, Ride, "pluck: Upright Bass").major(),
    h("Jungle sub", 168, BoomBap, "bass: 808"),
    h("Up and back", 126, BoomBap, "pluck: Muted Bass").major(),
    // Riff
    h("Minor hook", 100, BoomBap, "poly: Pluck"),
    h("Stutter lead", 128, House, "poly: Lead"),
    h("Pentatonic climb", 96, BoomBap, "pluck: Steel String"),
    h("Blues lick", 92, Ride, "wind: Tenor Sax"),
    h("Chug and stab", 140, BoomBap, "poly: Grit Bass").low(),
    h("Anthem", 128, House, "poly: Supersaw"),
    h("Call and answer", 100, BoomBap, "keys: Funk Clav").major(),
    h("Synth pop hook", 118, House, "analog: Square Keys"),
    h("Dorian strut", 104, BoomBap, "keys: Reed"),
    h("Pedal point", 132, House, "pluck: Harpsichord"),
    h("Lazy swing", 96, Ride, "pluck: Vibes"),
    h("Sixteenth burst", 120, House, "analog: Sync Lead"),
    h("Hip-hop keys", 88, BoomBap, "keys: Dusty Keys"),
    h("Harmonic turn", 110, BoomBap, "pluck: Nylon"),
    h("Shuffle riff", 112, Ride, "keys: Dirty Reed"),
    h("Waltz tune", 144, Ride, "pluck: Music Box"),
    // Chords
    h("Three three two", 122, House, "keys: Hard Tines").major(),
    h("Offbeat stabs", 124, House, "poly: Super Stab"),
    h("Skank", 80, BoomBap, "drawbar: Jazz Perc"),
    h("Wide pad", 110, House, "analog: Warm Pad"),
    h("Funk sixteenths", 104, BoomBap, "keys: Clav"),
    h("Power chug", 150, BoomBap, "poly: Super Stack").low(),
    h("Gallop chords", 140, BoomBap, "analog: Poly Brass").low(),
    h("Pop eighths", 116, BoomBap, "keys: Stage Tines").major(),
    h("Sus lift", 108, House, "keys: Soft Tines").major(),
    h("Soul sevenths", 86, BoomBap, "keys: Mellow Tines"),
    h("Twelve eight", 76, Ride, "keys: Ballad").major(),
    h("Waltz chords", 150, Ride, "keys: Soft Reed").major(),
    h("Oom pah", 116, House, "drawbar: Reedy").major(),
    h("Dub echo", 122, House, "analog: Hollow Pulse"),
    h("Stab and hold", 120, House, "strings: Disco Strings"),
    h("Montuno", 184, Half, "pluck: Marimba"),
    // Arp lines
    h("Up the chord", 126, House, "poly: Harmonic Pluck"),
    h("Rolling triad", 124, House, "poly: Pluck"),
    h("Octave pulse", 128, House, "poly: FM Keys"),
    h("Dotted climb", 118, House, "pluck: Glass"),
    h("Minor cascade", 132, House, "pluck: Harp"),
    h("Open fifths", 120, House, "poly: Digital Bell"),
    h("Seventh sweep", 110, BoomBap, "keys: Bell Keys"),
    h("Add nine shimmer", 124, House, "pluck: Kalimba").major(),
    h("Triplet waterfall", 104, Ride, "pluck: Celesta").major(),
    h("Bounce line", 126, House, "poly: Acid"),
    h("Six eight ostinato", 96, Ride, "keys: Pan Tines"),
    h("Sus shimmer", 122, House, "pluck: Bell"),
    h("Gate pattern", 138, House, "poly: Super Lead"),
    h("Wide leap", 112, BoomBap, "pluck: Pizzicato").major(),
    h("Dark pulse", 130, House, "analog: Unison Bass").low(),
    h("Swung thirds", 108, Ride, "wind: Jazz Clarinet").major(),
];

/// How a phrase with no row is heard: as its kind usually is.
fn usual(phrase: &Phrase) -> Heard {
    let swings = phrase.feel() != Feel::Straight;
    let (bpm, sound) = match phrase.kind {
        PhraseKind::Bass => (112, "bass: Round Mono"),
        PhraseKind::Riff => (108, "keys: Stage Tines"),
        PhraseKind::Chords => (108, "keys: Mellow Tines"),
        PhraseKind::Line => (120, "poly: Pluck"),
    };
    let beat = match phrase.kind {
        _ if swings || phrase.beats == 3 => Ride,
        PhraseKind::Line => House,
        _ => BoomBap,
    };
    h("", bpm, beat, sound)
}

/// A sound by its instrument and preset, as `bass: Round Mono`.
fn sound(spec: &str) -> Result<Box<dyn Node>, String> {
    let (instrument, name) = spec
        .split_once(':')
        .ok_or_else(|| format!("{spec}: a sound is an instrument, a colon and a preset"))?;
    let name = name.trim();
    // Every instrument here loads a preset the same way.
    macro_rules! load {
        ($synth:ty) => {{
            let p = <$synth>::presets()
                .iter()
                .find(|p| p.name == name)
                .ok_or_else(|| format!("{spec}: no such preset"))?;
            let mut s = <$synth>::new(RATE);
            s.load_preset(p);
            Box::new(s) as Box<dyn Node>
        }};
    }
    Ok(match instrument.trim() {
        "bass" => load!(Bass),
        "squelch" => load!(Squelch),
        "analog" => load!(Analog),
        "poly" => load!(Poly),
        "keys" => load!(Keys),
        "pluck" => load!(Pluck),
        "strings" => load!(Strings),
        "wind" => load!(Wind),
        "drawbar" => load!(Drawbar),
        other => return Err(format!("{spec}: no instrument {other}")),
    })
}

/// A phrase's setting: everything the file needs beside its notes.
struct Setting {
    heard: Heard,
    /// The tool's own brief for the drummer, the kit and the bass.
    session: Session,
    scale: Scale,
    major: bool,
    /// The note held in each bar of the file.
    held: Vec<u8>,
    /// The transport's swing, which the drummer follows, and the phrase
    /// too unless it is written in triplets (the engine leaves those
    /// straight).
    swing: Swing,
}

/// The notes held: the home note, then two others of the key. In A minor
/// the home note is A, then F and G (the chords a minor tune most often
/// moves to); in C major C, then F and G.
fn held_notes(kind: PhraseKind, major: bool, octave: i8) -> [u8; 3] {
    let notes: [i16; 3] = match (kind, major) {
        // A bass climbs to them, so its lowest notes can still be heard.
        (PhraseKind::Bass, false) => [33, 41, 43],
        (PhraseKind::Bass, true) => [36, 41, 43],
        // Chords stay around middle C, where a close chord is clear.
        (PhraseKind::Chords, false) => [57, 53, 55],
        (PhraseKind::Chords, true) => [48, 53, 55],
        (_, false) => [57, 53, 55],
        (_, true) => [60, 65, 67],
    };
    notes.map(|n| (n + 12 * i16::from(octave)).clamp(0, 127) as u8)
}

/// Bars in a phrase's file: four for one or two bars, twice through for
/// three or four, once through from five.
fn file_bars(once: usize) -> usize {
    match once {
        0..=2 => 4,
        3 | 4 => 2 * once,
        _ => once,
    }
}

fn setting(name: &str, phrase: &Phrase) -> Setting {
    let heard = HEARD
        .iter()
        .copied()
        .find(|r| r.name == name)
        .unwrap_or_else(|| usual(phrase));
    let major = match phrase.written {
        Written::Major => true,
        Written::Minor => false,
        Written::Either => heard.major,
    };
    let scale = if major {
        Scale::new(0, MAJOR)
    } else {
        Scale::new(9, MINOR)
    };
    let bars = file_bars(usize::from(phrase.bars));
    let [home, second, third] = held_notes(phrase.kind, major, heard.octave);
    let half = bars.div_ceil(2);
    let moved = half + (bars - half).div_ceil(2);
    let held = (0..bars)
        .map(|bar| match bar {
            b if b < half => home,
            b if b < moved => second,
            _ => third,
        })
        .collect();
    // A phrase written to swing is heard with the transport swinging, on
    // the grid it is written on: 1/8s a little lazier than 1/16s. Under a
    // phrase in triplets the transport shuffles (1/8s at a triplet's
    // 67 %), so the drummer plays its threes.
    let swing = match (phrase.feel(), phrase.unit) {
        (Feel::Swung, Rate::Eighth) => Swing::new(PPQ / 2, 64),
        (Feel::Swung, _) => Swing::new(PPQ / 4, 60),
        (Feel::Triplet, _) => Swing::new(PPQ / 2, 67),
        (Feel::Straight, _) => None,
    }
    .unwrap_or_default();
    let meter = Meter::new(phrase.beats, 4).unwrap_or_default();
    let session = Session {
        slug: slug(name),
        name: name.to_string(),
        notes: Vec::new(),
        bars,
        meter,
        bpm: f64::from(heard.bpm),
        swing,
        style: heard.beat.style(),
        role: if phrase.kind == PhraseKind::Bass {
            PartRole::Bass
        } else {
            PartRole::Melody
        },
    };
    Setting {
        heard,
        session,
        scale,
        major,
        held,
        swing,
    }
}

impl Setting {
    fn bar_ticks(&self) -> Tick {
        self.session.meter.bar_ticks()
    }

    /// The frame of a tick with no swing: where the bar lines are.
    fn frame(&self, tick: Tick) -> u64 {
        self.swung(tick, Swing::default())
    }

    fn swung(&self, tick: Tick, swing: Swing) -> u64 {
        let heard = swing.apply(tick as f64);
        (heard / f64::from(PPQ) * 60.0 / self.session.bpm * f64::from(RATE))
            .round()
            .max(0.0) as u64
    }

    fn end(&self) -> u64 {
        self.frame(self.held.len() as Tick * self.bar_ticks())
    }
}

/// What the engine played: note-ons and note-offs at their frames.
struct Played {
    /// (frame, the note and its velocity for a note-on, None for a note-off)
    events: Vec<(u64, u8, Option<u8>)>,
    notes: usize,
}

/// The phrase from the arp's engine, as a track of the app would play it:
/// the transport playing, the note held from the first bar line, changed
/// on bar lines by a hand that presses the new note and lets the old one
/// go, and let go on the last bar line.
fn play(phrase: &Phrase, s: &Setting) -> Result<Played, String> {
    let mut engine = Engine::new(RATE, s.session.bpm);
    engine.set_settings(
        Settings {
            mode: Mode::Phrase,
            scale: Some(s.scale),
            ..Settings::default()
        }
        .clamped(),
    );
    engine.set_phrase(*phrase);
    let end = s.end();
    let mut events = Events::new();
    let mut played = Played {
        events: Vec::new(),
        notes: 0,
    };
    let mut holding: Option<u8> = None;
    let mut bar = 0usize;
    let block = BLOCK as u64;
    let mut frame = 0u64;
    // A period after the last bar line, for the note-offs of letting go.
    while frame < end + block {
        // The clock first, then the hand, as the app does it.
        engine.sync(&Clock {
            anchor_tick: 0,
            frames_since_anchor: frame,
            bpm: s.session.bpm,
            playing: true,
            jumped: false,
            // As the app hands the transport's swing to a track's arp.
            swing: (s.swing.amount > 50).then_some((s.swing.grid, s.swing.amount)),
            bar_ticks: s.bar_ticks() as u32,
        });
        // Each bar's note goes down in the period its bar line is in, so
        // the line's own notes already play from it.
        while bar <= s.held.len() && s.frame(bar as Tick * s.bar_ticks()) < frame + block {
            let next = s.held.get(bar).copied();
            if next != holding {
                if let Some(n) = next {
                    engine.press(n, 100);
                }
                if let Some(n) = holding {
                    engine.release(n);
                }
                holding = next;
            }
            bar += 1;
        }
        engine.process(BLOCK as u32, &mut events);
        for e in events.as_slice() {
            let at = frame + u64::from(e.offset);
            match e.kind {
                EventKind::On { note, velocity } => {
                    played.events.push((at, note, Some(velocity)));
                    played.notes += 1;
                }
                EventKind::Off { note } => played.events.push((at, note, None)),
            }
        }
        frame += block;
    }
    if events.dropped + engine.overflow > 0 {
        return Err(format!(
            "the engine dropped {} events",
            events.dropped + engine.overflow
        ));
    }
    Ok(played)
}

/// The drummer's hits as (frame, pad, velocity), for the whole file.
fn beat(s: &Setting) -> Vec<(u64, u8, u8)> {
    let bar = s.bar_ticks();
    let bars = s.held.len();
    let mut hits = Vec::new();
    if s.heard.beat == Beat::Half && s.session.meter.beats == 4 {
        // One bar of the drummer over every two of the phrase.
        for pair in 0..bars.div_ceil(2) {
            for (tick, pad, velocity) in drummer(&s.session, pair) {
                let at = 2 * pair as Tick * bar + 2 * tick;
                if at < bars as Tick * bar {
                    hits.push((s.swung(at, s.swing), pad, velocity));
                }
            }
        }
    } else {
        for b in 0..bars {
            for (tick, pad, velocity) in drummer(&s.session, b) {
                hits.push((s.swung(b as Tick * bar + tick, s.swing), pad, velocity));
            }
        }
    }
    hits
}

/// One phrase as heard: left and right, and what the engine played.
fn render(phrase: &Phrase, s: &Setting) -> Result<(Vec<f32>, Vec<f32>, Played), String> {
    let played = play(phrase, s)?;
    let bass = phrase.kind == PhraseKind::Bass;
    // The phrase leads: a bass dry, chords with a little room, a riff or
    // a line with room and an echo.
    let (reverb, delay) = match phrase.kind {
        PhraseKind::Bass => (0.04, 0.0),
        PhraseKind::Chords => (0.25, 0.06),
        PhraseKind::Riff | PhraseKind::Line => (0.22, 0.16),
    };
    let mut lead = Part::new(sound(s.heard.sound)?, 1.0, reverb, delay);
    for &(at, note, velocity) in &played.events {
        lead.events.push(match velocity {
            Some(velocity) => (at, 1, NodeEvent::NoteOn { note, velocity }),
            None => (at, 0, NodeEvent::NoteOff { note }),
        });
    }
    let (drums_lu, bass_lu) = sits(&s.session);
    let mut drums = Part::new(kit(&s.session), 0.5, 0.05, 0.0)
        .under(drums_lu)
        .backing();
    for (at, pad, velocity) in beat(s) {
        drums.note(at, at + 2_000, pad, velocity);
    }
    // The harmony: the held note's chord as a pad over a bass phrase, the
    // held note on a bass under anything else.
    let bar = s.bar_ticks();
    let mut harmony = if bass {
        let mut strings = Strings::new(RATE);
        strings.load_preset(preset(Strings::presets(), "Ballad Pad", |p| p.name));
        Part::new(Box::new(strings), 0.26, 0.3, 0.0)
            .under(-11.0)
            .backing()
    } else {
        Part::new(bass_sound(&s.session), 0.55, 0.0, 0.0).under(bass_lu - 2.0)
    };
    let mut b = 0;
    while b < s.held.len() {
        let note = i16::from(s.held[b]);
        if bass {
            // One chord for as long as the note is held: the third and
            // the fifth of the key over it, close, from F3 up.
            let mut to = b + 1;
            while to < s.held.len() && s.held[to] == s.held[b] {
                to += 1;
            }
            let root = 53 + (note - 53).rem_euclid(12);
            let (on, off) = (s.frame(b as Tick * bar), s.frame(to as Tick * bar));
            for degree in [0, 2, 4] {
                let pitch = s.scale.walk(root, degree).clamp(0, 127) as u8;
                harmony.note(on, off, pitch, 64);
            }
            b = to;
        } else {
            // Struck again on every bar line, from E1 up.
            let root = (28 + (note - 28).rem_euclid(12)) as u8;
            let on = s.frame(b as Tick * bar);
            let off = s.frame((b + 1) as Tick * bar - Tick::from(PPQ) / 4);
            harmony.note(on, off, root, 96);
            b += 1;
        }
    }
    let mut parts = [lead, drums, harmony];
    let frames = (s.end() as f64 + TAIL * f64::from(RATE)) as usize;
    let (left, right) = mix(&mut parts, drums_lu, s.session.bpm, frames);
    Ok((left, right, played))
}

fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        NAMES[usize::from(note % 12)],
        i16::from(note / 12) - 1
    )
}

/// What the engine played, bar by bar, for `EXPLAIN=1`: each note where
/// it starts, counted in 1/16s from its bar line as heard.
fn explain(name: &str, s: &Setting, played: &Played) {
    let sixteenth = 60.0 / s.session.bpm / 4.0 * f64::from(RATE);
    eprintln!("{name} ({} bpm, {}):", s.heard.bpm, s.heard.sound);
    for (bar, &held) in s.held.iter().enumerate() {
        let from = s.frame(bar as Tick * s.bar_ticks());
        let to = s.frame((bar + 1) as Tick * s.bar_ticks());
        let notes: Vec<String> = played
            .events
            .iter()
            .filter(|(at, _, velocity)| velocity.is_some() && (from..to).contains(at))
            .map(|&(at, note, velocity)| {
                format!(
                    "{:.2}:{}/{}",
                    (at - from) as f64 / sixteenth,
                    note_name(note),
                    velocity.unwrap_or(0)
                )
            })
            .collect();
        eprintln!(
            "  bar {} (held {}): {}",
            bar + 1,
            note_name(held),
            notes.join(" ")
        );
    }
}

/// The held notes as words: "A3 for 2 bars, F3 for 1 bar, G3 for 1 bar".
fn held_words(held: &[u8]) -> String {
    let mut runs: Vec<(u8, usize)> = Vec::new();
    for &n in held {
        match runs.last_mut() {
            Some((last, bars)) if *last == n => *bars += 1,
            _ => runs.push((n, 1)),
        }
    }
    runs.iter()
        .map(|&(n, bars)| {
            format!(
                "{} for {bars} bar{}",
                note_name(n),
                if bars == 1 { "" } else { "s" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Writes every phrase of the library (MPFree's own with no `file`) into
/// `dir`: a WAV each, phrases.json for a listening page and report.txt.
/// `only` keeps the phrases whose name or kind holds that text.
pub(super) fn run(file: Option<&Path>, only: Option<&str>, dir: &Path) -> Result<(), String> {
    let library = match file {
        None => Library::built_in(),
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("can't read {}: {e}", path.display()))?;
            Library::parse(&text).map_err(|errors| {
                let lines: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
                format!("{}:\n  {}", path.display(), lines.join("\n  "))
            })?
        }
    };
    if file.is_none() {
        // The rows are for MPFree's phrases: say when the two part ways.
        let unheard: Vec<&str> = library
            .phrases()
            .map(|(name, _)| name)
            .filter(|name| !HEARD.iter().any(|r| r.name == *name))
            .collect();
        let gone: Vec<&str> = HEARD
            .iter()
            .map(|r| r.name)
            .filter(|name| library.phrase(name).is_none())
            .collect();
        if !unheard.is_empty() || !gone.is_empty() {
            return Err(format!(
                "the rows of HEARD (examples/listen/phrases.rs) and MPFree's phrases differ: no row for [{}], no phrase for [{}]",
                unheard.join(", "),
                gone.join(", ")
            ));
        }
    }
    let wanted = |name: &str, phrase: &Phrase| {
        only.is_none_or(|text| {
            let text = text.to_lowercase();
            name.to_lowercase().contains(&text)
                || phrase.kind.label().to_lowercase().contains(&text)
        })
    };
    let picked: Vec<(usize, &str, &Phrase)> = library
        .phrases()
        .enumerate()
        .filter(|(_, (name, phrase))| wanted(name, phrase))
        .map(|(i, (name, phrase))| (i + 1, name, phrase))
        .collect();
    if picked.is_empty() {
        return Err("no phrase to play".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
    let mut report = String::new();
    let _ = writeln!(
        report,
        "mpfree phrases as they play: {}\n",
        match file {
            None => "MPFree's own".to_string(),
            Some(path) => path.display().to_string(),
        }
    );
    let mut rows = Vec::new();
    let mut peaks = 0.0f32;
    for (number, name, phrase) in picked {
        let s = setting(name, phrase);
        let (left, right, played) = render(phrase, &s).map_err(|e| format!("{name}: {e}"))?;
        if played.notes == 0 {
            return Err(format!("{name}: nothing played"));
        }
        if std::env::var_os("EXPLAIN").is_some() {
            explain(name, &s, &played);
        }
        let (bytes, peak) = wav(&left, &right);
        peaks = peaks.max(peak);
        let stem = format!("{number:02}-{}-{}", phrase.kind.word(), slug(name));
        let path = dir.join(format!("{stem}.wav"));
        std::fs::write(&path, bytes).map_err(|e| format!("can't write {}: {e}", path.display()))?;
        let seconds = left.len() as f64 / f64::from(RATE);
        let key = if s.major { "C major" } else { "A minor" };
        let swing = if s.swing.amount > 50 {
            format!(
                ", swing {} at {} %",
                if s.swing.grid == PPQ / 2 {
                    "1/8"
                } else {
                    "1/16"
                },
                s.swing.amount
            )
        } else {
            String::new()
        };
        let _ = writeln!(
            report,
            "{:<34} {:<9} {:<18} {} bpm{swing}, {}, {} ({}); {key}, held {}; {} notes, {seconds:.1} s, peak {:.1} dBFS",
            format!("{stem}.wav"),
            phrase.kind.label(),
            name,
            s.heard.bpm,
            s.heard.beat.words(),
            s.heard.sound,
            kit_name(&s.session),
            held_words(&s.held),
            played.notes,
            20.0 * peak.max(1e-9).log10()
        );
        rows.push(format!(
            "  {{\"number\": {number}, \"name\": {}, \"kind\": {}, \"written\": {}, \"feel\": {}, \"bars\": {}, \"meter\": {}, \"text\": {}, \"bpm\": {}, \"beat\": {}, \"sound\": {}, \"key\": {}, \"held\": [{}], \"file\": {}, \"notes\": {}, \"seconds\": {seconds:.1}, \"ticks\": {}, \"shape\": [{}]}}",
            json(name),
            json(phrase.kind.label()),
            json(match phrase.written {
                Written::Major => "major",
                Written::Minor => "minor",
                Written::Either => "either",
            }),
            json(phrase.feel().label()),
            phrase.bars,
            json(&format!("{}/4", phrase.beats)),
            json(&phrase.to_text()),
            s.heard.bpm,
            json(s.heard.beat.words()),
            json(s.heard.sound),
            json(key),
            s.held
                .iter()
                .map(|&n| json(&note_name(n)))
                .collect::<Vec<_>>()
                .join(", "),
            json(&stem),
            played.notes,
            phrase.ticks(),
            // The phrase as written, for a picture: each note's start and
            // length in ticks and its semitones from the home note.
            phrase
                .notes()
                .iter()
                .map(|n| format!("[{}, {}, {}]", n.start, n.len, n.semis))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let _ = writeln!(
        report,
        "\nEvery file is at the same loudness ({FILE_LUFS} LUFS, with a soft knee from -1.9 dBFS instead of clipping); the loudest peak was {:.1} dBFS.",
        20.0 * peaks.max(1e-9).log10()
    );
    let index = format!("{{\"phrases\": [\n{}\n]}}\n", rows.join(",\n"));
    std::fs::write(dir.join("phrases.json"), index).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("report.txt"), &report).map_err(|e| e.to_string())?;
    print!("{report}");
    println!("Wrote {}", dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starter(name: &str) -> (Phrase, Setting) {
        let library = Library::built_in();
        let phrase = *library.phrase(name).expect("a phrase MPFree ships");
        (phrase, setting(name, &phrase))
    }

    /// Where each note the engine played starts.
    fn ons(played: &Played) -> Vec<(u64, u8)> {
        played
            .events
            .iter()
            .filter_map(|&(at, note, velocity)| velocity.map(|_| (at, note)))
            .collect()
    }

    /// MPFree's phrases and the rows they are heard by are one list in
    /// one order, and every row names a preset its instrument has.
    #[test]
    fn every_phrase_mpfree_ships_has_a_row_and_its_sound() {
        let library = Library::built_in();
        let names: Vec<&str> = library.phrases().map(|(name, _)| name).collect();
        let rows: Vec<&str> = HEARD.iter().map(|r| r.name).collect();
        assert_eq!(names, rows);
        for r in HEARD {
            assert!(sound(r.sound).is_ok(), "{}: {}", r.name, r.sound);
            assert!((40..=240).contains(&r.bpm), "{}", r.name);
        }
        for kind in PhraseKind::ALL {
            let mut phrase = Phrase::default();
            phrase.kind = kind;
            assert!(sound(usual(&phrase).sound).is_ok(), "{kind:?}");
        }
        assert!(sound("bass: No Such Preset").is_err());
        assert!(sound("kazoo: Round Mono").is_err());
    }

    /// A one-bar phrase plays four bars: two from the home note, then a
    /// bar from each of two others, each bar's notes from the note held
    /// in it and where they are written.
    #[test]
    fn each_bar_plays_from_the_note_held_in_it() {
        let (phrase, s) = starter("Offbeat pump");
        assert_eq!(s.held, [33, 33, 41, 43], "A1, A1, F2, G2");
        let played = play(&phrase, &s).unwrap();
        let ons = ons(&played);
        // An 1/8 after every beat, four to a bar.
        assert_eq!(ons.len(), 16);
        let beat = Tick::from(PPQ);
        for (i, &(at, note)) in ons.iter().enumerate() {
            assert_eq!(note, s.held[i / 4], "note {i}");
            let written = i as Tick * beat + beat / 2;
            assert!(at.abs_diff(s.frame(written)) <= 1, "note {i} at {at}");
        }
        // Every note ends, and none past the last bar line by more than
        // the period the hand lets go in.
        let offs = played.events.iter().filter(|e| e.2.is_none()).count();
        assert_eq!(offs, ons.len());
        let last = played.events.iter().map(|e| e.0).max().unwrap();
        assert!(last <= s.end() + BLOCK as u64);
    }

    /// A longer phrase plays twice through, the second time from the two
    /// other notes; a phrase in 3/4 counts its bars in threes.
    #[test]
    fn a_longer_phrase_plays_twice_through() {
        assert_eq!([1, 2, 3, 4, 5, 8].map(file_bars), [4, 4, 6, 8, 5, 8]);
        let (phrase, s) = starter("Waltz bass");
        assert_eq!((phrase.bars, phrase.beats), (4, 3));
        assert_eq!(s.held, [36, 36, 36, 36, 41, 41, 43, 43]);
        assert_eq!(s.bar_ticks(), 3 * Tick::from(PPQ));
        assert!(s.major, "written for either scale, heard in major");
    }

    /// A phrase written to swing is heard with the transport swinging,
    /// and the drummer swings with it; a phrase in triplets plays its
    /// threes straight over a drummer who shuffles.
    #[test]
    fn the_phrase_and_the_drummer_keep_one_swing() {
        let sixteenth = Tick::from(PPQ) / 4;
        let (phrase, s) = starter("Garage skip");
        assert_eq!((s.swing.grid, s.swing.amount), (PPQ / 4, 60));
        // Its second note is on the eighth 1/16 of the bar, an off one.
        let second = ons(&play(&phrase, &s).unwrap())[1].0;
        let swung = s.swung(7 * sixteenth, s.swing);
        assert!(second.abs_diff(swung) <= 1);
        assert!(
            swung > s.frame(7 * sixteenth) + 500,
            "a fifth of a 1/16 late"
        );
        // The drummer's off 1/16s land with it (give or take its humanize).
        let late = beat(&s)
            .iter()
            .filter(|h| h.0.abs_diff(swung) < 300)
            .count();
        assert!(late > 0);

        let (phrase, s) = starter("Shuffle walk");
        assert_eq!((s.swing.grid, s.swing.amount), (PPQ / 2, 67));
        // The second note is on the third 1/8 triplet of the first beat.
        let second = ons(&play(&phrase, &s).unwrap())[1].0;
        let triplet = Tick::from(PPQ) / 3;
        assert!(second.abs_diff(s.frame(2 * triplet)) <= 1);
    }
}
