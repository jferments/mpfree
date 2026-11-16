//! Just enough JSON for a plugin folder's `plugin.json` and
//! `mpc-plugin.json`: the top level object's strings, numbers and
//! booleans. Nested objects and arrays are read past and left out.

use std::collections::HashMap;

/// A top-level value this reader keeps.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
    /// `null`, an object or an array: present, but not kept.
    Other,
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Some(*n as i64),
            _ => None,
        }
    }
}

/// The top-level object's members, or None if the text isn't one JSON
/// object.
pub fn object(text: &str) -> Option<HashMap<String, Value>> {
    let mut p = Parser {
        b: text.as_bytes(),
        i: 0,
    };
    p.ws();
    // A byte-order mark is allowed before the object.
    if p.b[p.i..].starts_with("\u{feff}".as_bytes()) {
        p.i += 3;
        p.ws();
    }
    let mut out = HashMap::new();
    p.eat(b'{')?;
    p.ws();
    if p.peek()? == b'}' {
        p.i += 1;
    } else {
        loop {
            p.ws();
            let k = p.string()?;
            p.ws();
            p.eat(b':')?;
            p.ws();
            let v = p.value(0)?;
            out.insert(k, v);
            p.ws();
            match p.next()? {
                b',' => continue,
                b'}' => break,
                _ => return None,
            }
        }
    }
    p.ws();
    (p.i == p.b.len()).then_some(out)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

/// Nesting deeper than this is refused (a hostile file can't overflow the
/// stack).
const MAX_DEPTH: usize = 64;

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.i += 1;
        Some(c)
    }

    fn eat(&mut self, c: u8) -> Option<()> {
        (self.next()? == c).then_some(())
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn lit(&mut self, word: &str, v: Value) -> Option<Value> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Some(v)
        } else {
            None
        }
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth > MAX_DEPTH {
            return None;
        }
        match self.peek()? {
            b'"' => self.string().map(Value::Str),
            b't' => self.lit("true", Value::Bool(true)),
            b'f' => self.lit("false", Value::Bool(false)),
            b'n' => self.lit("null", Value::Other),
            b'{' => self.container(b'{', b'}', depth),
            b'[' => self.container(b'[', b']', depth),
            _ => self.number(),
        }
    }

    fn container(&mut self, open: u8, close: u8, depth: usize) -> Option<Value> {
        self.eat(open)?;
        self.ws();
        if self.peek()? == close {
            self.i += 1;
            return Some(Value::Other);
        }
        loop {
            self.ws();
            if open == b'{' {
                self.string()?;
                self.ws();
                self.eat(b':')?;
                self.ws();
            }
            self.value(depth + 1)?;
            self.ws();
            match self.next()? {
                b',' => continue,
                c if c == close => return Some(Value::Other),
                _ => return None,
            }
        }
    }

    fn number(&mut self) -> Option<Value> {
        let start = self.i;
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        ) {
            self.i += 1;
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).ok()?;
        let n: f64 = s.parse().ok()?;
        n.is_finite().then_some(Value::Num(n))
    }

    fn hex4(&mut self) -> Option<u32> {
        let s = self.b.get(self.i..self.i + 4)?;
        self.i += 4;
        u32::from_str_radix(std::str::from_utf8(s).ok()?, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            match self.next()? {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let c = match self.next()? {
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
                            let code = if (0xd800..0xdc00).contains(&hi) {
                                self.eat(b'\\')?;
                                self.eat(b'u')?;
                                let lo = self.hex4()?;
                                if !(0xdc00..0xe000).contains(&lo) {
                                    return None;
                                }
                                0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00)
                            } else {
                                hi
                            };
                            char::from_u32(code)?
                        }
                        _ => return None,
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                c if c < 0x20 => return None,
                c => out.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_plugin_json() {
        let o = object(
            r#"{
  "name": "Wobble", "manufacturer": "Acme é\"x\"", "uid": "AcWb",
  "numOutputs": 2, "isInstrument": true, "tags": ["a", {"b": [1, 2]}],
  "cpu": null, "nested": {"x": {"y": 1.5e3}}
}"#,
        )
        .unwrap();
        assert_eq!(o["name"].as_str(), Some("Wobble"));
        assert_eq!(o["manufacturer"].as_str(), Some("Acme é\"x\""));
        assert_eq!(o["numOutputs"].as_i64(), Some(2));
        assert_eq!(o["isInstrument"].as_bool(), Some(true));
        assert_eq!(o["tags"], Value::Other);
        assert_eq!(o["cpu"], Value::Other);
        assert_eq!(o["nested"], Value::Other);
    }

    #[test]
    fn refuses_what_is_not_one_object() {
        for bad in [
            "",
            "[]",
            "{",
            r#"{"a" 1}"#,
            r#"{"a": 1,}"#,
            r#"{"a": 1} x"#,
            r#"{"a": "\q"}"#,
            r#"{"a": nul}"#,
            "{\"a\": \"line\nbreak\"}",
        ] {
            assert!(object(bad).is_none(), "{bad:?}");
        }
        let deep = format!("{{\"a\": {}{}}}", "[".repeat(200), "]".repeat(200));
        assert!(object(&deep).is_none());
        assert_eq!(object("\u{feff}{}").map(|o| o.len()), Some(0));
        assert_eq!(object(r#"{"e": "🎹"}"#).unwrap()["e"].as_str(), Some("🎹"));
    }
}
