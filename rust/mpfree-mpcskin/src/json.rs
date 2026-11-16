//! JSON as the MPC reads a skin: objects keep their keys in order and keep
//! duplicates (an overlay's data has two `version` keys, a legacy entry
//! inlines its remap's `version`), and a lookup takes the first occurrence.
//! Strict JSON otherwise, with a byte-order mark allowed in front.

use std::fmt;

/// Nesting deeper than this is refused (real skins nest about 20 deep).
const MAX_DEPTH: usize = 200;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// Keys in file order, duplicates kept.
    Obj(Vec<(String, Json)>),
}

/// Where and why a file failed to parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub what: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}: {}", self.line, self.col, self.what)
    }
}

static NULL: Json = Json::Null;

impl Json {
    /// The first value under `key` in an object; `None` for a missing key
    /// or a value that isn't an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// `get`, or `Null` when missing, for chained lookups.
    pub fn at(&self, key: &str) -> &Json {
        self.get(key).unwrap_or(&NULL)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    /// A string, or "" for anything else.
    pub fn str_or_empty(&self) -> &str {
        self.as_str().unwrap_or("")
    }

    /// A number, also from a string holding one (some generators quote
    /// numbers).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            Json::Str(s) => s.trim().parse().ok(),
            Json::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        self.as_f64()
            .filter(|n| n.is_finite())
            .map(|n| n.round() as i64)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            Json::Num(n) => Some(*n != 0.0),
            Json::Str(s) => match s.as_str() {
                "true" | "True" | "1" => Some(true),
                "false" | "False" | "0" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    /// The items of an array (none for anything else).
    pub fn items(&self) -> &[Json] {
        match self {
            Json::Arr(a) => a,
            _ => &[],
        }
    }

    /// The pairs of an object (none for anything else).
    pub fn pairs(&self) -> &[(String, Json)] {
        match self {
            Json::Obj(kv) => kv,
            _ => &[],
        }
    }

    /// A `[{"key": k, "value": v}, ...]` map, as the MPC writes dictionaries.
    pub fn key_values(&self) -> impl Iterator<Item = (&str, &Json)> {
        self.items()
            .iter()
            .filter_map(|e| Some((e.get("key")?.as_str()?, e.get("value")?)))
    }
}

/// Parses a whole file.
pub fn parse(text: &str) -> Result<Json, ParseError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("text after the end of the document"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> ParseError {
        let upto = &self.s[..self.i.min(self.s.len())];
        let line = upto.iter().filter(|&&b| b == b'\n').count() + 1;
        let col = upto.iter().rev().take_while(|&&b| b != b'\n').count() + 1;
        ParseError {
            line,
            col,
            what: what.to_string(),
        }
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.i) {
            self.i += 1;
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.s.get(self.i) == Some(&b) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nested too deeply"));
        }
        match self.s.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut kv = Vec::new();
                self.ws();
                if self.eat(b'}') {
                    return Ok(Json::Obj(kv));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return Err(self.err("expected a key in quotes"));
                    }
                    let k = self.string()?;
                    self.ws();
                    if !self.eat(b':') {
                        return Err(self.err("expected ':' after a key"));
                    }
                    self.ws();
                    let v = self.value(depth + 1)?;
                    kv.push((k, v));
                    self.ws();
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b'}') {
                        return Ok(Json::Obj(kv));
                    }
                    return Err(self.err("expected ',' or '}'"));
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.eat(b']') {
                    return Ok(Json::Arr(a));
                }
                loop {
                    self.ws();
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b']') {
                        return Ok(Json::Arr(a));
                    }
                    return Err(self.err("expected ',' or ']'"));
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
            None => Err(self.err("unexpected end of the document")),
        }
    }

    fn word(&mut self, w: &str, v: Json) -> Result<Json, ParseError> {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(self.err("unexpected word"))
        }
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.i;
        self.eat(b'-');
        let digits = |p: &mut Self| {
            let s = p.i;
            while let Some(b'0'..=b'9') = p.s.get(p.i) {
                p.i += 1;
            }
            p.i > s
        };
        if !digits(self) {
            return Err(self.err("expected a digit"));
        }
        if self.eat(b'.') && !digits(self) {
            return Err(self.err("expected a digit after '.'"));
        }
        if self.eat(b'e') || self.eat(b'E') {
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            if !digits(self) {
                return Err(self.err("expected an exponent"));
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("x");
        text.parse()
            .map(Json::Num)
            .map_err(|_| self.err("not a number"))
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let h = self
            .s
            .get(self.i..self.i + 4)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .ok_or_else(|| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(h)
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.i += 1; // the opening quote
        let mut out = Vec::new();
        loop {
            match self.s.get(self.i) {
                None => return Err(self.err("a string runs to the end of the document")),
                Some(b'"') => {
                    self.i += 1;
                    return String::from_utf8(out).map_err(|_| self.err("not UTF-8"));
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = *self.s.get(self.i).ok_or_else(|| self.err("bad escape"))?;
                    self.i += 1;
                    let ch = match c {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi)
                                && self.s.get(self.i..self.i + 2) == Some(b"\\u")
                            {
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                } else {
                                    0xFFFD
                                }
                            } else {
                                hi
                            };
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                        _ => return Err(self.err("bad escape")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                Some(&b) if b < 0x20 => return Err(self.err("a control character inside a string")),
                Some(&b) => {
                    out.push(b);
                    self.i += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_duplicate_keys_in_order_and_takes_the_first() {
        let j = parse(r#"{"version": 2, "version": 1, "actions": [], "x": "y"}"#).unwrap();
        let keys: Vec<&str> = j.pairs().iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["version", "version", "actions", "x"]);
        assert_eq!(j.at("version").as_i64(), Some(2));
        assert_eq!(j.at("x").as_str(), Some("y"));
        assert_eq!(j.at("missing"), &Json::Null);
    }

    #[test]
    fn reads_values_and_escapes() {
        let j = parse("\u{feff} {\"a\": [1, -2.5e1, true, false, null], \"s\": \"\\u00e9\\ud83d\\ude00\\n\\\"\"} ")
            .unwrap();
        let a = j.at("a").items();
        assert_eq!(a[0].as_f64(), Some(1.0));
        assert_eq!(a[1].as_f64(), Some(-25.0));
        assert_eq!(a[2].as_bool(), Some(true));
        assert_eq!(a[4], Json::Null);
        assert_eq!(j.at("s").as_str(), Some("é😀\n\""));
    }

    #[test]
    fn key_value_maps() {
        let j = parse(r#"[{"key": "Data", "value": "Parameter 2"}, {"key": "x"}]"#).unwrap();
        let m: Vec<(&str, &str)> = j
            .key_values()
            .map(|(k, v)| (k, v.as_str().unwrap()))
            .collect();
        assert_eq!(m, [("Data", "Parameter 2")]);
    }

    #[test]
    fn says_where_it_failed() {
        let e = parse("{\n  \"a\": 1,\n  \"b\" 2\n}").unwrap_err();
        assert_eq!((e.line, e.col), (3, 7));
        assert!(parse("[1, 2").is_err());
        assert!(parse("{} x").is_err());
        assert!(parse("\"a\u{1}b\"").is_err());
        let deep = "[".repeat(300) + &"]".repeat(300);
        assert!(parse(&deep).is_err());
    }
}
