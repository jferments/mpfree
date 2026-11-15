//! A small XML reader, enough for MPC program files: elements, attributes,
//! text, the five named entities and numeric ones, comments, CDATA, the
//! XML declaration and a doctype (both skipped). No namespaces, no DTD.
//!
//! It is bounded: at most [`MAX_BYTES`] of input, [`MAX_DEPTH`] levels and
//! [`MAX_ELEMENTS`] elements, so a hostile file can't take the machine.

/// The largest file read.
pub const MAX_BYTES: usize = 8 << 20;
/// The deepest nesting.
pub const MAX_DEPTH: usize = 64;
/// The most elements in one file.
pub const MAX_ELEMENTS: usize = 1 << 20;

/// One element: its name, attributes, the text directly inside it
/// (trimmed, CDATA included) and its child elements in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<Element>,
}

impl Element {
    /// The value of attribute `name`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The first child named `name`.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    /// The text of the first child named `name`, trimmed.
    pub fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.as_str())
    }

    /// This element and every element below it, depth first, in document order.
    pub fn descendants(&self) -> Vec<&Element> {
        let mut out = Vec::new();
        let mut stack = vec![self];
        while let Some(e) = stack.pop() {
            out.push(e);
            stack.extend(e.children.iter().rev());
        }
        out
    }
}

/// Parses a document; returns its root element.
pub fn parse(bytes: &[u8]) -> Result<Element, String> {
    if bytes.len() > MAX_BYTES {
        return Err(format!(
            "{} bytes; at most {MAX_BYTES} are read",
            bytes.len()
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(|e| format!("not UTF-8: {e}"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut p = Parser {
        s: text.as_bytes(),
        src: text,
        i: 0,
        count: 0,
    };
    p.skip_misc()?;
    if p.peek() != Some(b'<') {
        return Err("no root element".into());
    }
    let root = p.element(0)?;
    p.skip_misc()?;
    if p.i < p.s.len() {
        return Err(format!("text after the root element at byte {}", p.i));
    }
    Ok(root)
}

struct Parser<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
    count: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn starts(&self, pat: &str) -> bool {
        self.s[self.i..].starts_with(pat.as_bytes())
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }

    fn skip_past(&mut self, end: &str, what: &str) -> Result<(), String> {
        match self.src[self.i..].find(end) {
            Some(k) => {
                self.i += k + end.len();
                Ok(())
            }
            None => Err(format!("{what} at byte {} never ends", self.i)),
        }
    }

    /// Whitespace, comments, processing instructions and a doctype.
    fn skip_misc(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            if self.starts("<?") {
                self.skip_past("?>", "a processing instruction")?;
            } else if self.starts("<!--") {
                self.skip_past("-->", "a comment")?;
            } else if self.starts("<!DOCTYPE") {
                self.skip_past(">", "a doctype")?;
            } else {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> Result<String, String> {
        let start = self.i;
        while self.peek().is_some_and(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':') || c >= 0x80
        }) {
            self.i += 1;
        }
        if self.i == start {
            return Err(format!("a name was expected at byte {start}"));
        }
        Ok(self.src[start..self.i].to_string())
    }

    fn element(&mut self, depth: usize) -> Result<Element, String> {
        if depth >= MAX_DEPTH {
            return Err(format!("nested deeper than {MAX_DEPTH} levels"));
        }
        self.count += 1;
        if self.count > MAX_ELEMENTS {
            return Err(format!("more than {MAX_ELEMENTS} elements"));
        }
        self.i += 1; // '<'
        let mut el = Element {
            name: self.name()?,
            ..Element::default()
        };
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'/') if self.starts("/>") => {
                    self.i += 2;
                    return Ok(el);
                }
                Some(b'>') => {
                    self.i += 1;
                    break;
                }
                Some(_) => {
                    let k = self.name()?;
                    self.skip_ws();
                    if self.peek() != Some(b'=') {
                        return Err(format!("attribute {k} has no value at byte {}", self.i));
                    }
                    self.i += 1;
                    self.skip_ws();
                    let q = self.peek().filter(|&c| c == b'"' || c == b'\'');
                    let Some(q) = q else {
                        return Err(format!("attribute {k} is not quoted at byte {}", self.i));
                    };
                    self.i += 1;
                    let start = self.i;
                    while self.peek().is_some_and(|c| c != q) {
                        self.i += 1;
                    }
                    if self.peek().is_none() {
                        return Err(format!("attribute {k} never ends"));
                    }
                    let v = unescape(&self.src[start..self.i])?;
                    self.i += 1;
                    el.attrs.push((k, v));
                }
                None => return Err(format!("<{}> never ends", el.name)),
            }
        }
        let mut text = String::new();
        loop {
            match self.peek() {
                None => return Err(format!("<{}> is never closed", el.name)),
                Some(b'<') => {
                    if self.starts("</") {
                        self.i += 2;
                        let n = self.name()?;
                        if n != el.name {
                            return Err(format!("</{n}> closes <{}>", el.name));
                        }
                        self.skip_ws();
                        if self.peek() != Some(b'>') {
                            return Err(format!("</{n}> has no '>'"));
                        }
                        self.i += 1;
                        el.text = text.trim().to_string();
                        return Ok(el);
                    } else if self.starts("<!--") {
                        self.skip_past("-->", "a comment")?;
                    } else if self.starts("<![CDATA[") {
                        let start = self.i + 9;
                        self.skip_past("]]>", "a CDATA section")?;
                        text.push_str(&self.src[start..self.i - 3]);
                    } else if self.starts("<?") {
                        self.skip_past("?>", "a processing instruction")?;
                    } else {
                        el.children.push(self.element(depth + 1)?);
                    }
                }
                Some(_) => {
                    let start = self.i;
                    while self.peek().is_some_and(|c| c != b'<') {
                        self.i += 1;
                    }
                    text.push_str(&unescape(&self.src[start..self.i])?);
                }
            }
        }
    }
}

/// Text with its entities replaced.
fn unescape(s: &str) -> Result<String, String> {
    if !s.contains('&') {
        return Ok(s.to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(k) = rest.find('&') {
        out.push_str(&rest[..k]);
        rest = &rest[k..];
        let end = rest
            .find(';')
            .ok_or_else(|| format!("an entity in {s:?} never ends"))?;
        let ent = &rest[1..end];
        let c = match ent {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let n = if let Some(h) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                    u32::from_str_radix(h, 16).ok()
                } else if let Some(d) = ent.strip_prefix('#') {
                    d.parse().ok()
                } else {
                    None
                };
                n.and_then(char::from_u32)
                    .ok_or_else(|| format!("unknown entity &{ent};"))?
            }
        };
        out.push(c);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_elements_attributes_and_text() {
        let doc = br#"<?xml version="1.0" encoding="UTF-8"?>
<!-- a comment -->
<A x="1" y='two &amp; three'>
  <B>hello &lt;world&gt; &#65;&#x42;</B>
  <C/>
  <D><![CDATA[raw <text>]]></D>
</A>
"#;
        let a = parse(doc).unwrap();
        assert_eq!(a.name, "A");
        assert_eq!(a.attr("y"), Some("two & three"));
        assert_eq!(a.child_text("B"), Some("hello <world> AB"));
        assert_eq!(a.child("C").unwrap().children.len(), 0);
        assert_eq!(a.child_text("D"), Some("raw <text>"));
        let names: Vec<_> = a.descendants().iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, ["A", "B", "C", "D"]);
    }

    #[test]
    fn a_byte_order_mark_is_skipped() {
        let mut doc = "\u{feff}".as_bytes().to_vec();
        doc.extend_from_slice(b"<A>1</A>");
        assert_eq!(parse(&doc).unwrap().text, "1");
    }

    #[test]
    fn broken_documents_are_refused() {
        for bad in [
            &b"<A><B></A>"[..],
            b"<A>",
            b"<A x=1></A>",
            b"<A>&nope;</A>",
            b"<A></A>trailing",
            b"",
            b"\xff\xfe",
        ] {
            assert!(parse(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn deep_nesting_is_refused() {
        let doc = "<a>".repeat(MAX_DEPTH + 1) + &"</a>".repeat(MAX_DEPTH + 1);
        assert!(parse(doc.as_bytes()).unwrap_err().contains("deeper"));
    }
}
