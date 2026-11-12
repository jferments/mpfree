//! Scala tuning files: `.scl` scales and `.kbm` keyboard maps.
//!
//! The format is the Scala program's, and thousands of published tunings
//! come in it (<https://www.huygens-fokker.org/scala/scl_format.html>).
//!
//! **A scale** (`.scl`) lists its degrees above 1/1, one per line: in cents
//! (a number with a dot, `701.955`) or as a ratio (`3/2`, or a whole number
//! such as `2`). The last degree is the period, usually the octave (2/1);
//! the scale repeats at that interval. Lines starting with `!` are
//! comments; the first other line is the description, the next the number
//! of degrees. Anything after a degree's number on its line is ignored.
//!
//! **A keyboard map** (`.kbm`) says how keys reach degrees: the map's size
//! (0 for "every key the next degree"), the first and last keys to retune,
//! the middle key (it plays 1/1), the reference key and its frequency, the
//! degree one repeat of the map spans (0 for the scale's period), then one
//! degree per key of the map (`x` for a key with none).
//!
//! With no map, keys follow the degrees in order from middle C, with middle
//! C at its 12-TET pitch (261.6256 Hz), as Scala does ([`KeyMap::standard`]).
//!
//! Two choices differ from Scala, so a note never vanishes:
//!
//! - every key follows the map, whatever the first and last keys to retune
//!   say, since a sequenced or transposed note can land anywhere;
//! - a key with no degree (`x`) sounds between its neighbours instead of
//!   staying silent.

use crate::{Error, Tuning};

/// The most degrees a scale or entries a map may have.
pub const MAX_DEGREES: usize = 1024;

/// A scale: its degrees above 1/1 in cents, the last being the period.
#[derive(Clone, Debug, PartialEq)]
pub struct Scale {
    pub description: String,
    /// Degrees 1 to N in cents. Never empty; the last is above 0.
    pub cents: Vec<f64>,
}

impl Scale {
    /// The interval the scale repeats at, in cents (1200 for an octave).
    pub fn period(&self) -> f64 {
        self.cents.last().copied().unwrap_or(1200.0)
    }

    /// Degrees in one period (the count the file gives).
    pub fn len(&self) -> usize {
        self.cents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cents.is_empty()
    }

    /// The tuning with 1/1 on `key` at `hz` Hz and each key the next
    /// degree: what [`tuning`] gives with [`KeyMap::linear`]`(key, key, hz)`,
    /// to the bit, but without allocating, so the audio thread may call it
    /// to move a scale to another root or A4.
    pub fn rooted(&self, key: i32, hz: f64) -> Tuning {
        Tuning::linear(&self.cents, key, hz)
    }

    /// The pitch of any degree, in cents above 1/1: degree 0 is 1/1, degree
    /// N the period, negative degrees go down.
    pub fn degree_cents(&self, d: i64) -> f64 {
        let n = self.cents.len().max(1) as i64;
        let oct = d.div_euclid(n);
        let i = d.rem_euclid(n) as usize;
        let within = if i == 0 { 0.0 } else { self.cents[i - 1] };
        oct as f64 * self.period() + within
    }
}

/// A keyboard map: which keys play which degrees, and the tuning's pitch.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyMap {
    /// The first and last keys to retune (read, kept, but see the module
    /// docs: every key follows the map).
    pub first: i32,
    pub last: i32,
    /// The key that plays 1/1.
    pub middle: i32,
    /// The key tuned to [`KeyMap::ref_hz`].
    pub ref_key: i32,
    pub ref_hz: f64,
    /// The degree one repeat of the map spans; 0 for the scale's period.
    pub octave_degree: usize,
    /// One entry per key of a repeat, from the middle key up; `None` for a
    /// key with no degree. Empty: every key plays the next degree.
    pub map: Vec<Option<usize>>,
}

impl KeyMap {
    /// Scala's default: every key the next degree from middle C, and middle
    /// C at its 12-TET pitch for A4 = 440 Hz.
    pub fn standard() -> KeyMap {
        KeyMap::linear(60, 60, 440.0 * (-9.0f64 / 12.0).exp2())
    }

    /// Every key the next degree from `middle`, with `ref_key` at `ref_hz`.
    pub fn linear(middle: i32, ref_key: i32, ref_hz: f64) -> KeyMap {
        KeyMap {
            first: 0,
            last: 127,
            middle,
            ref_key,
            ref_hz,
            octave_degree: 0,
            map: Vec::new(),
        }
    }
}

/// Reads a `.scl` scale.
pub fn parse_scl(text: &str) -> Result<Scale, Error> {
    let mut lines = text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.starts_with('!'))
        .map(|(i, l)| (i + 1, l.trim()));
    let err = |line: usize, why: &str| Error(format!("scale line {line}: {why}"));
    let (_, description) = lines
        .next()
        .ok_or_else(|| Error("scale: empty file".to_string()))?;
    let mut lines = lines.filter(|(_, l)| !l.is_empty());
    let (at, count) = lines
        .next()
        .ok_or_else(|| Error("scale: no count of notes".to_string()))?;
    let n: usize = first_token(count)
        .parse()
        .map_err(|_| err(at, "the count of notes is not a whole number"))?;
    if n == 0 || n > MAX_DEGREES {
        return Err(err(at, "the count of notes must be 1 to 1024"));
    }
    let mut cents = Vec::with_capacity(n);
    for _ in 0..n {
        let (at, line) = lines
            .next()
            .ok_or_else(|| Error(format!("scale: {n} notes promised, {} found", cents.len())))?;
        let c = pitch(first_token(line))
            .ok_or_else(|| err(at, "not cents (with a dot) or a ratio above 0"))?;
        cents.push(c);
    }
    if cents[n - 1] <= 0.0 {
        return Err(Error(
            "scale: the last note (the period) must be above 1/1".to_string(),
        ));
    }
    Ok(Scale {
        description: description.to_string(),
        cents,
    })
}

/// Reads a `.kbm` keyboard map.
pub fn parse_kbm(text: &str) -> Result<KeyMap, Error> {
    let mut lines = text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.starts_with('!'))
        .map(|(i, l)| (i + 1, first_token(l.trim())))
        .filter(|(_, l)| !l.is_empty());
    let mut next = |what: &str| {
        lines
            .next()
            .ok_or_else(|| Error(format!("keyboard map: no {what}")))
    };
    let err = |line: usize, why: &str| Error(format!("keyboard map line {line}: {why}"));
    let whole = |(at, s): (usize, &str), what: &str| -> Result<usize, Error> {
        s.parse::<usize>()
            .map_err(|_| err(at, &format!("the {what} is not a whole number")))
    };
    let key = |(at, s): (usize, &str), what: &str| -> Result<i32, Error> {
        match s.parse::<i32>() {
            Ok(k) if (0..=127).contains(&k) => Ok(k),
            _ => Err(err(at, &format!("the {what} must be a key from 0 to 127"))),
        }
    };
    let size = whole(next("map size")?, "map size")?;
    if size > MAX_DEGREES {
        return Err(Error(
            "keyboard map: the map size must be 0 to 1024".to_string(),
        ));
    }
    let first = key(next("first key")?, "first key")?;
    let last = key(next("last key")?, "last key")?;
    let middle = key(next("middle key")?, "middle key")?;
    let ref_key = key(next("reference key")?, "reference key")?;
    let (at, f) = next("reference frequency")?;
    let ref_hz: f64 = f
        .parse()
        .ok()
        .filter(|f: &f64| f.is_finite() && *f > 0.0)
        .ok_or_else(|| err(at, "the reference frequency must be a number above 0"))?;
    let octave_degree = whole(next("octave degree")?, "octave degree")?;
    let mut map = Vec::with_capacity(size);
    for _ in 0..size {
        // Entries left out are keys with no degree, as Scala reads them.
        let Some((at, e)) = lines.next() else {
            break;
        };
        map.push(if e.eq_ignore_ascii_case("x") {
            None
        } else {
            Some(whole((at, e), "degree")?)
        });
    }
    map.resize(size, None);
    Ok(KeyMap {
        first,
        last,
        middle,
        ref_key,
        ref_hz,
        octave_degree,
        map,
    })
}

/// The tuning a scale and a keyboard map give.
pub fn tuning(scale: &Scale, map: &KeyMap) -> Result<Tuning, Error> {
    if scale.is_empty() {
        return Err(Error("scale: no notes".to_string()));
    }
    let size = map.map.len() as i64;
    let octave = if map.octave_degree == 0 {
        scale.len() as i64
    } else {
        map.octave_degree as i64
    };
    let key_cents = |k: i32| -> Option<f64> {
        let off = i64::from(k) - i64::from(map.middle);
        if size == 0 {
            return Some(scale.degree_cents(off));
        }
        let repeat = off.div_euclid(size);
        let d = map.map[off.rem_euclid(size) as usize]? as i64;
        Some(repeat as f64 * scale.degree_cents(octave) + scale.degree_cents(d))
    };
    let r = key_cents(map.ref_key).ok_or_else(|| {
        Error(format!(
            "keyboard map: the reference key {} has no degree",
            map.ref_key
        ))
    })?;
    let base = map.ref_hz.log2();
    Tuning::from_log2(|k| key_cents(k).map(|c| base + (c - r) / 1200.0))
}

/// Reads a scale and an optional keyboard map (Scala's default without
/// one) into a tuning.
pub fn load(scl: &str, kbm: Option<&str>) -> Result<Tuning, Error> {
    let scale = parse_scl(scl)?;
    let map = match kbm {
        Some(text) => parse_kbm(text)?,
        None => KeyMap::standard(),
    };
    tuning(&scale, &map)
}

fn first_token(line: &str) -> &str {
    line.split_whitespace().next().unwrap_or("")
}

/// A degree in cents: cents if it has a dot, else a ratio or whole number.
fn pitch(tok: &str) -> Option<f64> {
    if tok.contains('.') {
        return tok.parse::<f64>().ok().filter(|c| c.is_finite());
    }
    let (a, b) = match tok.split_once('/') {
        Some((a, b)) => (a.parse::<u64>().ok()?, b.parse::<u64>().ok()?),
        None => (tok.parse::<u64>().ok()?, 1),
    };
    if a == 0 || b == 0 {
        return None;
    }
    Some(1200.0 * (a as f64 / b as f64).log2())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Within 0.02 cents.
    fn close(a: f32, b: f32) -> bool {
        (a / b - 1.0).abs() < 1e-5
    }

    const TET12: &str = "! 12tet.scl\n!\n12 equal steps\n 12\n!\n 100.0\n 200.\n 300.0\n 400.0\n 500.0\n 600.0\n 700.0\n 800.0\n 900.0\n 1000.0\n 1100.0\n 2/1\n";

    #[test]
    fn reads_cents_ratios_comments_and_trailing_text() {
        let s = parse_scl(
            "! a comment\n! another\nA test scale\n 4\n!\n 701.955 a fifth\n5/4\n 3\n2/1   octave\n",
        )
        .unwrap();
        assert_eq!(s.description, "A test scale");
        assert_eq!(s.len(), 4);
        assert!((s.cents[0] - 701.955).abs() < 1e-9);
        assert!((s.cents[1] - 386.3137).abs() < 1e-3);
        assert!((s.cents[2] - 1901.955).abs() < 1e-3);
        assert_eq!(s.period(), 1200.0);
        // The description may be empty; Windows line ends are fine.
        let s = parse_scl("!x\r\n\r\n1\r\n2/1\r\n").unwrap();
        assert_eq!((s.description.as_str(), s.len()), ("", 1));
    }

    #[test]
    fn bad_scales_say_what_and_where() {
        for (text, why) in [
            ("", "empty"),
            ("desc\n", "no count"),
            ("desc\nlots\n", "line 2"),
            ("desc\n0\n", "1 to 1024"),
            ("desc\n2\n3/2\n", "2 notes promised"),
            ("desc\n2\n3/2\n0/1\n", "line 4"),
            ("desc\n1\nabc\n", "line 3"),
            ("desc\n1\n-1200.0\n", "period"),
        ] {
            let e = parse_scl(text).unwrap_err().to_string();
            assert!(e.contains(why), "{text:?}: {e}");
        }
    }

    #[test]
    fn a_12_tet_file_matches_the_standard_tuning() {
        let t = load(TET12, None).unwrap();
        let std = Tuning::standard();
        for k in -64..=191 {
            let (a, b) = (t.hz(k as f32), std.hz(k as f32));
            assert!((a / b - 1.0).abs() < 2e-6, "key {k}: {a} vs {b}");
        }
        assert!(!t.is_equal());
    }

    #[test]
    fn a_keyboard_map_places_degrees_and_pitch() {
        // A 7-note scale on the white keys from D, A4 = 432 Hz on key 69.
        let scl = "Dorian, just\n7\n9/8\n6/5\n4/3\n3/2\n5/3\n9/5\n2/1\n";
        let kbm = "! dorian.kbm\n12\n0\n127\n62\n69\n432.0\n7\n! mapping\n0\nx\n1\n2\nx\n3\nx\n4\nx\n5\n6\nx\n";
        let map = parse_kbm(kbm).unwrap();
        assert_eq!((map.middle, map.ref_key, map.octave_degree), (62, 69, 7));
        assert_eq!(map.map.len(), 12);
        assert_eq!(map.map[1], None);
        let t = load(scl, Some(kbm)).unwrap();
        assert!((t.hz(69.0) - 432.0).abs() < 1e-3);
        let d = t.hz(62.0);
        assert!(close(t.hz(69.0) / d, 1.5), "A is a fifth over D");
        assert!(close(t.hz(74.0) / d, 2.0), "the map repeats at degree 7");
        assert!(close(t.hz(65.0) / d, 1.2), "F is 6/5");
        // A key with no degree lies between its neighbours.
        let eb = t.hz(63.0);
        assert!((eb - (d * t.hz(64.0)).sqrt()).abs() < 1e-3);
    }

    #[test]
    fn short_maps_pad_with_keys_without_a_degree() {
        let kbm = "4\n0\n127\n60\n60\n261.6\n0\n0\n2\n";
        let m = parse_kbm(kbm).unwrap();
        assert_eq!(m.map, vec![Some(0), Some(2), None, None]);
        let scl = "four\n4\n300.\n600.\n900.\n1200.\n";
        let t = tuning(&parse_scl(scl).unwrap(), &m).unwrap();
        assert!((t.hz(60.0) - 261.6).abs() < 1e-3);
        assert!((t.hz(61.0) / t.hz(60.0) - 2f32.sqrt()).abs() < 1e-5);
        assert!(close(t.hz(64.0) / t.hz(60.0), 2.0));
    }

    #[test]
    fn bad_maps_say_what() {
        for (text, why) in [
            ("12\n0\n127\n60\n", "reference key"),
            ("12\n0\n128\n60\n69\n440\n12\n", "last key"),
            ("12\n0\n127\n60\n69\n-1\n12\n", "frequency"),
            ("5000\n0\n127\n60\n69\n440\n12\n", "map size"),
            ("2\n0\n127\n60\n69\n440\n12\n0\ny\n", "degree"),
        ] {
            let e = parse_kbm(text).unwrap_err().to_string();
            assert!(e.contains(why), "{text:?}: {e}");
        }
        // The reference key must have a degree.
        let m = parse_kbm("2\n0\n127\n60\n61\n440\n0\n0\nx\n").unwrap();
        let s = parse_scl(TET12).unwrap();
        assert!(tuning(&s, &m)
            .unwrap_err()
            .to_string()
            .contains("reference key 61"));
    }

    #[test]
    fn a_scale_can_repeat_at_any_interval() {
        // Bohlen-Pierce: 13 equal steps of a twelfth (3/1).
        let mut scl = String::from("Bohlen-Pierce\n13\n");
        for i in 1..13 {
            scl.push_str(&format!("{:.6}\n", 1200.0 * 3f64.log2() * i as f64 / 13.0));
        }
        scl.push_str("3/1\n");
        let t = load(&scl, None).unwrap();
        assert!(close(t.hz(73.0) / t.hz(60.0), 3.0));
        assert!(close(t.hz(47.0) / t.hz(60.0), 1.0 / 3.0));
    }

    #[test]
    fn a_rooted_scale_is_the_linear_map_on_that_root_to_the_bit() {
        let mut bp = String::from("Bohlen-Pierce\n13\n");
        for i in 1..13 {
            bp.push_str(&format!("{:.6}\n", 1200.0 * 3f64.log2() * i as f64 / 13.0));
        }
        bp.push_str("3/1\n");
        let scales = [
            TET12.to_string(),
            "Dorian, just\n7\n9/8\n6/5\n4/3\n3/2\n5/3\n9/5\n2/1\n".to_string(),
            "one\n1\n3/2\n".to_string(),
            bp,
        ];
        for text in &scales {
            let s = parse_scl(text).unwrap();
            for (key, hz) in [(60, 261.6256), (62, 290.0), (-10, 3.5), (127, 12_000.0)] {
                let want = tuning(&s, &KeyMap::linear(key, key, hz)).unwrap();
                assert_eq!(s.rooted(key, hz), want, "{} on key {key}", s.description);
            }
        }
        // Middle C at its usual pitch is Scala's default map.
        let s = parse_scl(TET12).unwrap();
        let std = KeyMap::standard();
        assert_eq!(s.rooted(60, std.ref_hz), tuning(&s, &std).unwrap());
    }
}
