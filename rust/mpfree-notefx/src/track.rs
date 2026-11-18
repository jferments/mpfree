//! A track's Notes stage as settings, and the words a saved project keeps
//! each effect in.

use std::fmt::Write as _;
use std::str::FromStr;

use mpfree_scale::{Scale, Snap};

use crate::chain::{Slot, MAX_FX};
use crate::fx::{
    CcScale, CcSmooth, Chord, Echo, Fx, LatchMode, Randomize, Shape, Strum, StrumOrder, Time,
    Velocity, Voicing, CC_MOST,
};

/// A track's Notes stage: up to [`MAX_FX`] effects in places, top to
/// bottom, and where the track's arp sits among them: after the first
/// `arp_at` places (0: the arp first, every effect after it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackFx {
    pub slots: [Option<Slot>; MAX_FX],
    pub arp_at: u8,
    /// Each effect's id on the track, by place (0 in an empty place): a
    /// number the effect keeps when it moves, so what names one of its
    /// settings (a Q-Link, a macro, a lock) follows the effect, not the
    /// place, as an insert's slot id does on the mixer. The chain doesn't
    /// use it.
    pub ids: [u16; MAX_FX],
}

impl TrackFx {
    /// No effect in any place.
    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    /// The place of the effect with id `id`, if it is there.
    pub fn place_of(&self, id: u16) -> Option<usize> {
        (id != 0)
            .then(|| (0..MAX_FX).find(|&p| self.ids[p] == id && self.slots[p].is_some()))
            .flatten()
    }

    /// An id no effect on the track has: one past the highest in use (1
    /// on a track with none).
    pub fn new_id(&self) -> u16 {
        let mut id = (0..MAX_FX)
            .filter(|&p| self.slots[p].is_some())
            .map(|p| self.ids[p])
            .max()
            .unwrap_or(0)
            .wrapping_add(1)
            .max(1);
        while self.place_of(id).is_some() {
            id = id.wrapping_add(1).max(1);
        }
        id
    }

    /// Setting `k` of the effect with id `id`, if it is there.
    pub fn value(&self, id: u16, k: usize) -> Option<i32> {
        let p = self.place_of(id)?;
        let fx = self.slots[p]?.fx;
        (k < fx.settings().len()).then(|| fx.get(k))
    }

    /// Sets setting `k` of the effect with id `id` to `v`, kept in its
    /// range (a macro, an LFO or a lock moving it). False if the track has
    /// no such effect or setting. Never allocates.
    pub fn set_value(&mut self, id: u16, k: usize, v: i32) -> bool {
        let Some(p) = self.place_of(id) else {
            return false;
        };
        match self.slots[p].as_mut() {
            Some(slot) if k < slot.fx.settings().len() => {
                slot.fx.set(k, v);
                true
            }
            _ => false,
        }
    }

    /// The places before the arp: [`crate::Chain::process_range`] runs
    /// them on the notes going into it, and the rest on what it plays.
    pub fn arp_place(&self) -> usize {
        usize::from(self.arp_at).min(MAX_FX)
    }

    /// Whether an effect that is on acts on the controllers the track
    /// sends to MIDI out ([`TrackFx::cc_out`]).
    pub fn shapes_ccs(&self) -> bool {
        self.slots.iter().flatten().any(|s| s.on && s.fx.is_cc())
    }

    /// What MIDI out sends for controller `cc` given `value` (0 to 127):
    /// the value after each CC scale that is on and takes it, top to
    /// bottom, and how long it glides there, in ms, from the last CC
    /// smooth that is on and takes it (None: it jumps), with whether it
    /// glides when it comes with a note. Never allocates.
    pub fn cc_out(&self, cc: u8, value: u8) -> (u8, Option<CcSmooth>) {
        let mut v = value.min(127);
        let mut glide = None;
        if cc > CC_MOST {
            return (v, None);
        }
        for s in self.slots.iter().flatten().filter(|s| s.on) {
            match s.fx {
                Fx::CcScale(c) if c.takes(cc) => v = c.apply(v),
                Fx::CcSmooth(c) if c.takes(cc) => glide = Some(c),
                _ => {}
            }
        }
        (v, glide)
    }
}

/// One row of a track's Notes list, top to bottom: an effect in its
/// place, or the track's arp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Fx(usize),
    Arp,
}

#[derive(Clone, Copy)]
enum Row {
    /// An effect and its id.
    Fx(Slot, u16),
    Arp,
}

impl TrackFx {
    /// How many effects it has.
    pub fn count(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    /// The Notes list, top to bottom: the effects above the arp, the arp,
    /// the effects below it.
    pub fn items(&self) -> Vec<Item> {
        let at = self.arp_place();
        let fx = |r: std::ops::Range<usize>| {
            r.filter(|&i| self.slots[i].is_some())
                .map(Item::Fx)
                .collect::<Vec<_>>()
        };
        let mut out = fx(0..at);
        out.push(Item::Arp);
        out.extend(fx(at..MAX_FX));
        out
    }

    fn rows(&self) -> Vec<Row> {
        self.items()
            .into_iter()
            .map(|i| match i {
                Item::Fx(p) => Row::Fx(
                    self.slots[p].unwrap_or(Slot {
                        fx: Fx::Chance { percent: 100 },
                        on: false,
                    }),
                    self.ids[p],
                ),
                Item::Arp => Row::Arp,
            })
            .collect()
    }

    fn from_rows(rows: &[Row]) -> TrackFx {
        let mut t = TrackFx::default();
        let mut place = 0;
        for r in rows {
            match r {
                Row::Fx(s, id) if place < MAX_FX => {
                    t.slots[place] = Some(*s);
                    t.ids[place] = *id;
                    place += 1;
                }
                Row::Fx(..) => {}
                Row::Arp => t.arp_at = place as u8,
            }
        }
        t
    }

    /// The row of the list where `item` is.
    pub fn row_of(&self, item: Item) -> Option<usize> {
        self.items().iter().position(|i| *i == item)
    }

    /// Adds `fx` (on, with a new id) as the last effect above the arp, the
    /// places packed in order. The row it landed in, or None when every
    /// place is taken.
    pub fn add(&mut self, fx: Fx) -> Option<usize> {
        if self.count() >= MAX_FX {
            return None;
        }
        let mut rows = self.rows();
        let arp = rows.iter().position(|r| matches!(r, Row::Arp)).unwrap_or(0);
        rows.insert(arp, Row::Fx(Slot { fx, on: true }, self.new_id()));
        *self = TrackFx::from_rows(&rows);
        Some(arp)
    }

    /// Takes out the effect in row `row` (the arp stays), the places
    /// packed in order. False when the row is the arp or past the end.
    pub fn remove(&mut self, row: usize) -> bool {
        let mut rows = self.rows();
        if !matches!(rows.get(row), Some(Row::Fx(..))) {
            return false;
        }
        rows.remove(row);
        *self = TrackFx::from_rows(&rows);
        true
    }

    /// Moves row `from` to row `to` (the arp too: the effects it passes
    /// change side), the places packed in order.
    pub fn move_row(&mut self, from: usize, to: usize) -> bool {
        let mut rows = self.rows();
        if from >= rows.len() || to >= rows.len() || from == to {
            return false;
        }
        let r = rows.remove(from);
        rows.insert(to, r);
        *self = TrackFx::from_rows(&rows);
        true
    }
}

const SHAPES: [(Shape, &str); 12] = [
    (Shape::Octave, "octave"),
    (Shape::Fifth, "fifth"),
    (Shape::Major, "major"),
    (Shape::Minor, "minor"),
    (Shape::Seventh, "7th"),
    (Shape::Minor7, "m7"),
    (Shape::Major7, "maj7"),
    (Shape::Sus2, "sus2"),
    (Shape::Sus4, "sus4"),
    (Shape::Add9, "add9"),
    (Shape::Dim, "dim"),
    (Shape::Aug, "aug"),
];

fn shape_word(s: Shape) -> String {
    match s {
        Shape::InKey(n) => format!("inkey{n}"),
        Shape::Custom(m) => format!("custom{m}"),
        s => SHAPES
            .iter()
            .find(|(x, _)| *x == s)
            .map_or("major", |(_, w)| w)
            .to_string(),
    }
}

fn shape_from(w: &str) -> Result<Shape, String> {
    if let Some(n) = w.strip_prefix("inkey") {
        return Ok(Shape::InKey(num(n, 2, 5, "inkey")?));
    }
    if let Some(m) = w.strip_prefix("custom") {
        return Ok(Shape::Custom(num(m, 0, (1 << 24) - 1, "custom")?));
    }
    SHAPES
        .iter()
        .find(|(_, x)| *x == w)
        .map(|(s, _)| *s)
        .ok_or_else(|| format!("no chord shape {w}"))
}

const VOICINGS: [(Voicing, &str); 3] = [
    (Voicing::Close, "close"),
    (Voicing::Spread, "spread"),
    (Voicing::Drop2, "drop2"),
];

const ORDERS: [(StrumOrder, &str); 3] = [
    (StrumOrder::Up, "up"),
    (StrumOrder::Down, "down"),
    (StrumOrder::Played, "played"),
];

const LATCHES: [(LatchMode, &str); 2] = [
    (LatchMode::NextChord, "chord"),
    (LatchMode::Toggle, "toggle"),
];

const SNAPS: [(Snap, &str); 4] = [
    (Snap::Down, "down"),
    (Snap::Up, "up"),
    (Snap::Nearest, "nearest"),
    (Snap::Skip, "skip"),
];

fn word_of<T: PartialEq + Copy>(list: &[(T, &'static str)], v: T) -> &'static str {
    list.iter().find(|(x, _)| *x == v).map_or("-", |(_, w)| w)
}

fn from_word<T: Copy>(list: &[(T, &'static str)], w: &str, what: &str) -> Result<T, String> {
    list.iter()
        .find(|(_, x)| *x == w)
        .map(|(v, _)| *v)
        .ok_or_else(|| format!("no {what} {w}"))
}

/// "120ms" or "480t" (ticks, 960 a quarter).
fn time_word(t: Time) -> String {
    match t {
        Time::Ms(ms) => format!("{ms}ms"),
        Time::Ticks(t) => format!("{t}t"),
    }
}

fn time_from(w: &str) -> Result<Time, String> {
    if let Some(ms) = w.strip_suffix("ms") {
        Ok(Time::Ms(num(ms, 1, 10_000, "ms")?))
    } else if let Some(t) = w.strip_suffix('t') {
        Ok(Time::Ticks(num(t, 1, 4 * 3840, "ticks")?))
    } else {
        Err(format!("no time {w}"))
    }
}

/// "9:1453:1453": the root, then the scale's notes going up and coming
/// down (bit N is N semitones above the root).
fn scale_word(s: Scale) -> String {
    format!("{}:{}:{}", s.root, s.mask, s.down)
}

fn scale_from(w: &str) -> Result<Scale, String> {
    let mut p = w.split(':');
    let mut next = |hi: u16, what: &str| -> Result<u16, String> {
        num(p.next().ok_or_else(|| format!("no {what}"))?, 0, hi, what)
    };
    let root = next(11, "root")? as u8;
    let up = next(0xFFF, "scale")?;
    let down = next(0xFFF, "scale")?;
    Ok(Scale::with_down(root, up, down))
}

fn opt_word(v: Option<u8>) -> String {
    v.map_or_else(|| "-".to_string(), |v| v.to_string())
}

fn opt_from(w: &str, lo: u8, hi: u8, what: &str) -> Result<Option<u8>, String> {
    if w == "-" {
        Ok(None)
    } else {
        num(w, lo, hi, what).map(Some)
    }
}

fn num<T: FromStr + PartialOrd + Copy>(w: &str, lo: T, hi: T, what: &str) -> Result<T, String> {
    match w.parse::<T>() {
        Ok(v) if v >= lo && v <= hi => Ok(v),
        _ => Err(format!("bad {what} {w}")),
    }
}

impl Slot {
    /// As a saved project keeps it: whether it is on, the effect, then its
    /// settings as `name=value` (`on chord shape=m7 voicing=drop2 inv=0
    /// below=- vel=90 strum=8 scale=-`).
    pub fn to_words(&self) -> String {
        let mut s = String::from(if self.on { "on " } else { "off " });
        let _ = match self.fx {
            Fx::Transpose { semitones } => write!(s, "transpose st={semitones}"),
            Fx::Scale { scale, snap } => write!(
                s,
                "scale scale={} snap={}",
                scale_word(scale),
                word_of(&SNAPS, snap)
            ),
            Fx::Chord(c) => write!(
                s,
                "chord shape={} voicing={} inv={} below={} vel={} strum={} scale={}",
                shape_word(c.shape),
                word_of(&VOICINGS, c.voicing),
                c.inversion,
                opt_word(c.keep_below),
                c.velocity,
                c.strum_ms,
                c.scale.map_or_else(|| "-".to_string(), scale_word)
            ),
            Fx::Velocity(v) => write!(
                s,
                "velocity fixed={} curve={} min={} max={}",
                opt_word(v.fixed),
                v.curve,
                v.min,
                v.max
            ),
            Fx::Length { time } => write!(s, "length time={}", time_word(time)),
            Fx::Echo(e) => write!(
                s,
                "echo time={} repeats={} fb={} pitch={}",
                time_word(e.time),
                e.repeats,
                e.feedback,
                e.pitch
            ),
            Fx::Humanize { time_ms, velocity } => {
                write!(s, "humanize ms={time_ms} vel={velocity}")
            }
            Fx::Chance { percent } => write!(s, "chance pct={percent}"),
            Fx::Strum(st) => write!(
                s,
                "strum ms={} order={}",
                st.time_ms,
                word_of(&ORDERS, st.order)
            ),
            Fx::Latch { mode } => write!(s, "latch mode={}", word_of(&LATCHES, mode)),
            Fx::Randomize(r) => write!(
                s,
                "random pitch={} often={} vel={} longer={}",
                r.pitch, r.often, r.velocity, r.longer_ms
            ),
            Fx::CcScale(c) => write!(
                s,
                "ccscale cc={} low={} high={}",
                opt_word(c.cc),
                c.low,
                c.high
            ),
            Fx::CcSmooth(c) => write!(
                s,
                "ccsmooth cc={} ms={} note={}",
                opt_word(c.cc),
                c.ms,
                if c.glide_on_note { "glide" } else { "jump" }
            ),
        };
        s
    }

    /// Reads [`Slot::to_words`]. A setting it doesn't know is skipped and
    /// one left out keeps the effect's first value, so a newer build's
    /// words still read; a number out of its range is refused.
    pub fn from_words(text: &str) -> Result<Slot, String> {
        let mut w = text.split_whitespace();
        let on = match w.next() {
            Some("on") => true,
            Some("off") => false,
            other => return Err(format!("no on or off: {other:?}")),
        };
        let kind = w.next().ok_or("no effect")?;
        let mut fx = match kind {
            "transpose" => Fx::Transpose { semitones: 0 },
            "scale" => Fx::all()[0],
            "chord" => Fx::Chord(Chord::default()),
            "velocity" => Fx::Velocity(Velocity::default()),
            "length" => Fx::all()[4],
            "echo" => Fx::Echo(Echo::default()),
            "humanize" => Fx::all()[6],
            "chance" => Fx::all()[7],
            "strum" => Fx::Strum(Strum::default()),
            "latch" => Fx::Latch {
                mode: LatchMode::NextChord,
            },
            "random" => Fx::Randomize(Randomize::default()),
            "ccscale" => Fx::CcScale(CcScale::default()),
            "ccsmooth" => Fx::CcSmooth(CcSmooth::default()),
            k => return Err(format!("no effect {k}")),
        };
        for pair in w {
            let Some((k, v)) = pair.split_once('=') else {
                continue;
            };
            match &mut fx {
                Fx::Transpose { semitones } => {
                    if k == "st" {
                        *semitones = num(v, -48, 48, k)?;
                    }
                }
                Fx::Scale { scale, snap } => match k {
                    "scale" => *scale = scale_from(v)?,
                    "snap" => *snap = from_word(&SNAPS, v, k)?,
                    _ => {}
                },
                Fx::Chord(c) => match k {
                    "shape" => c.shape = shape_from(v)?,
                    "voicing" => c.voicing = from_word(&VOICINGS, v, k)?,
                    "inv" => c.inversion = num(v, 0, 3, k)?,
                    "below" => c.keep_below = opt_from(v, 0, 127, k)?,
                    "vel" => c.velocity = num(v, 1, 100, k)?,
                    "strum" => c.strum_ms = num(v, 0, 1000, k)?,
                    "scale" => c.scale = if v == "-" { None } else { Some(scale_from(v)?) },
                    _ => {}
                },
                Fx::Velocity(vel) => match k {
                    "fixed" => vel.fixed = opt_from(v, 1, 127, k)?,
                    "curve" => vel.curve = num(v, -100, 100, k)?,
                    "min" => vel.min = num(v, 1, 127, k)?,
                    "max" => vel.max = num(v, 1, 127, k)?,
                    _ => {}
                },
                Fx::Length { time } => {
                    if k == "time" {
                        *time = time_from(v)?;
                    }
                }
                Fx::Echo(e) => match k {
                    "time" => e.time = time_from(v)?,
                    "repeats" => e.repeats = num(v, 1, 16, k)?,
                    "fb" => e.feedback = num(v, 1, 100, k)?,
                    "pitch" => e.pitch = num(v, -12, 12, k)?,
                    _ => {}
                },
                Fx::Humanize { time_ms, velocity } => match k {
                    "ms" => *time_ms = num(v, 0, 100, k)?,
                    "vel" => *velocity = num(v, 0, 64, k)?,
                    _ => {}
                },
                Fx::Chance { percent } => {
                    if k == "pct" {
                        *percent = num(v, 0, 100, k)?;
                    }
                }
                Fx::Strum(st) => match k {
                    "ms" => st.time_ms = num(v, 1, 200, k)?,
                    "order" => st.order = from_word(&ORDERS, v, k)?,
                    _ => {}
                },
                Fx::Latch { mode } => {
                    if k == "mode" {
                        *mode = from_word(&LATCHES, v, k)?;
                    }
                }
                Fx::Randomize(r) => match k {
                    "pitch" => r.pitch = num(v, 0, 24, k)?,
                    "often" => r.often = num(v, 0, 100, k)?,
                    "vel" => r.velocity = num(v, 0, 64, k)?,
                    "longer" => r.longer_ms = num(v, 0, 1000, k)?,
                    _ => {}
                },
                Fx::CcScale(c) => match k {
                    "cc" => c.cc = opt_from(v, 0, CC_MOST, k)?,
                    "low" => c.low = num(v, 0, 127, k)?,
                    "high" => c.high = num(v, 0, 127, k)?,
                    _ => {}
                },
                Fx::CcSmooth(c) => match k {
                    "cc" => c.cc = opt_from(v, 0, CC_MOST, k)?,
                    "ms" => c.ms = num(v, 1, 2000, k)?,
                    "note" => {
                        c.glide_on_note = match v {
                            "glide" => true,
                            "jump" => false,
                            _ => return Err(format!("bad note {v}")),
                        }
                    }
                    _ => {}
                },
            }
        }
        Ok(Slot { fx, on })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effect_reads_back_as_written() {
        let mut list = Fx::all().to_vec();
        list.push(Fx::Chord(Chord {
            shape: Shape::InKey(4),
            voicing: Voicing::Drop2,
            inversion: 2,
            keep_below: Some(72),
            velocity: 90,
            strum_ms: 8,
            scale: Some(Scale::with_down(9, 0b1010_1101_1101, 0b0101_1010_1101)),
        }));
        list.push(Fx::Chord(Chord {
            shape: Shape::Custom(0b1001_0001),
            ..Chord::default()
        }));
        list.push(Fx::Velocity(Velocity {
            fixed: Some(100),
            curve: -40,
            min: 20,
            max: 110,
        }));
        list.push(Fx::Length {
            time: Time::Ms(120),
        });
        list.push(Fx::Echo(Echo {
            time: Time::Ms(250),
            repeats: 16,
            feedback: 1,
            pitch: -12,
        }));
        list.push(Fx::Transpose { semitones: -48 });
        list.push(Fx::Strum(Strum {
            time_ms: 200,
            order: StrumOrder::Played,
        }));
        list.push(Fx::Latch {
            mode: LatchMode::Toggle,
        });
        list.push(Fx::Randomize(Randomize {
            pitch: 24,
            often: 100,
            velocity: 64,
            longer_ms: 1000,
        }));
        list.push(Fx::CcScale(CcScale {
            cc: Some(74),
            low: 127,
            high: 0,
        }));
        list.push(Fx::CcSmooth(CcSmooth {
            cc: Some(0),
            ms: 2000,
            glide_on_note: true,
        }));
        for (i, fx) in list.into_iter().enumerate() {
            let slot = Slot { fx, on: i % 2 == 0 };
            let words = slot.to_words();
            assert_eq!(Slot::from_words(&words), Ok(slot), "{words}");
        }
    }

    #[test]
    fn unknown_settings_are_skipped_and_bad_numbers_refused() {
        let s = Slot::from_words("on echo repeats=5 colour=red time=240t").unwrap();
        assert_eq!(
            s.fx,
            Fx::Echo(Echo {
                repeats: 5,
                time: Time::Ticks(240),
                ..Echo::default()
            })
        );
        assert!(Slot::from_words("on echo repeats=17").is_err());
        assert!(Slot::from_words("on transpose st=49").is_err());
        assert!(Slot::from_words("on chord shape=m9").is_err());
        assert!(Slot::from_words("on flanger").is_err());
        assert!(Slot::from_words("maybe chance pct=5").is_err());
        assert!(Slot::from_words("on scale scale=12:1:1").is_err());
    }

    #[test]
    fn the_list_adds_above_the_arp_and_moves_effects_past_it() {
        let all = Fx::all();
        let mut t = TrackFx::default();
        assert_eq!(t.items(), [Item::Arp]);
        assert_eq!(t.add(all[0]), Some(0)); // Scale
        assert_eq!(t.add(all[1]), Some(1)); // Chord
        assert_eq!(t.add(all[6]), Some(2)); // Humanize
        assert_eq!(
            t.items(),
            [Item::Fx(0), Item::Fx(1), Item::Fx(2), Item::Arp]
        );
        // Humanize goes below the arp: board 7's list.
        assert!(t.move_row(2, 3));
        assert_eq!(
            t.items(),
            [Item::Fx(0), Item::Fx(1), Item::Arp, Item::Fx(2)]
        );
        assert_eq!(t.arp_at, 2);
        assert_eq!(t.slots[2].unwrap().fx, all[6]);
        // Each keeps its id when it moves.
        assert_eq!(t.ids, [1, 2, 3, 0, 0, 0, 0, 0]);
        // The arp moves to the top: every effect is below it.
        assert!(t.move_row(2, 0));
        assert_eq!(t.arp_at, 0);
        assert_eq!(t.row_of(Item::Fx(1)), Some(2));
        // Taking one out packs the places; the arp can't be taken out.
        assert!(!t.remove(0));
        assert!(t.remove(1));
        assert_eq!(t.count(), 2);
        assert_eq!(t.slots[0].unwrap().fx, all[1]);
        assert_eq!(t.slots[1].unwrap().fx, all[6]);
        assert_eq!((t.ids[0], t.ids[1], t.ids[2]), (2, 3, 0));
        assert_eq!(t.place_of(3), Some(1));
        assert!(!t.move_row(0, 9));
        for _ in 0..6 {
            assert!(t.add(all[7]).is_some());
        }
        assert_eq!(t.add(all[7]), None, "eight places");
    }

    #[test]
    fn the_arp_sits_after_its_places() {
        let mut t = TrackFx::default();
        assert!(t.is_empty());
        assert_eq!(t.arp_place(), 0);
        t.arp_at = 200;
        assert_eq!(t.arp_place(), MAX_FX);
        t.slots[3] = Some(Slot {
            fx: Fx::Chance { percent: 50 },
            on: false,
        });
        assert!(!t.is_empty());
    }

    #[test]
    fn an_effects_id_is_new_on_its_track_and_finds_its_place() {
        let mut t = TrackFx::default();
        assert_eq!(t.new_id(), 1);
        assert_eq!(t.place_of(0), None);
        let slot = Some(Slot {
            fx: Fx::Chance { percent: 50 },
            on: true,
        });
        (t.slots[2], t.ids[2]) = (slot, 7);
        (t.slots[5], t.ids[5]) = (slot, 3);
        assert_eq!(t.new_id(), 8);
        assert_eq!(t.place_of(3), Some(5));
        // An empty place's id names nothing.
        t.ids[0] = 9;
        assert_eq!(t.place_of(9), None);
        assert_eq!(t.new_id(), 8);
        // Past the top it wraps to the lowest free one past 0.
        (t.slots[1], t.ids[1]) = (slot, u16::MAX);
        assert_eq!(t.new_id(), 1);
    }

    #[test]
    fn a_setting_is_read_and_set_by_its_effects_id_and_kept_in_range() {
        let mut t = TrackFx::default();
        t.add(Fx::Chance { percent: 50 });
        t.add(Fx::Chord(crate::Chord::default()));
        let (chance, chord) = (t.ids[0], t.ids[1]);
        assert_eq!(t.value(chance, 0), Some(50));
        assert!(t.set_value(chance, 0, 80));
        assert_eq!(t.slots[0].unwrap().fx, Fx::Chance { percent: 80 });
        // Out of range is kept in it; a choice too.
        assert!(t.set_value(chance, 0, 400));
        assert_eq!(t.value(chance, 0), Some(100));
        assert!(t.set_value(chord, 2, 9));
        assert_eq!(t.value(chord, 2), Some(3), "inversion 0 to 3");
        assert!(t.set_value(chord, 0, -5));
        assert_eq!(t.value(chord, 0), Some(0));
        // No such effect or setting.
        assert!(!t.set_value(99, 0, 1));
        assert!(!t.set_value(chance, 1, 1));
        assert_eq!(t.value(chance, 1), None);
        assert_eq!(t.value(0, 0), None);
    }
}
