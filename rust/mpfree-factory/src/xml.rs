//! Just enough XML for Akai's program files (.xpm): elements, attributes,
//! text, entities, comments and CDATA. No namespaces, no DTDs (a DOCTYPE is
//! skipped). Bounded: at most 64 levels deep and 200,000 elements.

/// An element: its name, attributes, child elements and its own text.
#[derive(Debug, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    /// The element's direct text, joined.
    pub text: String,
}

impl Element {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// This element and every element below it named `name`, in document
    /// order (as ElementTree's `iter`).
    pub fn iter<'a>(&'a self, name: &str) -> Vec<&'a Element> {
        let mut out = Vec::new();
        self.collect(name, &mut out);
        out
    }

    fn collect<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        if self.name == name {
            out.push(self);
        }
        for c in &self.children {
            c.collect(name, out);
        }
    }

    /// The text of the first child named `name` ("" if it has none), or
    /// None without such a child (as ElementTree's `findtext`).
    pub fn find_text(&self, name: &str) -> Option<&str> {
        self.children
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.text.as_str())
    }
}

const MAX_DEPTH: usize = 64;
const MAX_ELEMENTS: usize = 200_000;

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Parses a document; returns its root element.
pub fn parse(text: &str) -> Result<Element, String> {
    let s = text.strip_prefix('\u{feff}').unwrap_or(text);
    let b = s.as_bytes();
    let mut i = 0usize;
    let mut stack: Vec<Element> = vec![Element::default()]; // a holder for the root
    let mut count = 0usize;
    while i < b.len() {
        if b[i] != b'<' {
            let end = s[i..].find('<').map_or(b.len(), |j| i + j);
            let t = decode_entities(&s[i..end]);
            if stack.len() > 1 {
                stack.last_mut().unwrap().text.push_str(&t);
            } else if !t.trim().is_empty() {
                return Err("text outside the root element".into());
            }
            i = end;
            continue;
        }
        let rest = &s[i..];
        if rest.starts_with("<?") {
            i += rest.find("?>").ok_or("an unclosed <?")? + 2;
        } else if rest.starts_with("<!--") {
            i += rest.find("-->").ok_or("an unclosed comment")? + 3;
        } else if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").ok_or("an unclosed CDATA")?;
            if stack.len() > 1 {
                stack.last_mut().unwrap().text.push_str(&rest[9..end]);
            }
            i += end + 3;
        } else if rest.starts_with("<!") {
            i += rest.find('>').ok_or("an unclosed <!")? + 1;
        } else if rest.starts_with("</") {
            let end = rest.find('>').ok_or("an unclosed end tag")?;
            let name = rest[2..end].trim();
            if stack.len() < 2 || stack.last().unwrap().name != name {
                return Err(format!("</{name}> does not close the open element"));
            }
            let el = stack.pop().unwrap();
            stack.last_mut().unwrap().children.push(el);
            i += end + 1;
        } else {
            // A start tag: name, attributes, maybe "/>".
            let mut j = i + 1;
            while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
                j += 1;
            }
            let name = s[i + 1..j].to_string();
            if name.is_empty() {
                return Err("a tag with no name".into());
            }
            let mut el = Element {
                name,
                ..Default::default()
            };
            let closed;
            loop {
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= b.len() {
                    return Err("an unclosed start tag".into());
                }
                if b[j] == b'>' {
                    closed = false;
                    j += 1;
                    break;
                }
                if b[j] == b'/' {
                    if b.get(j + 1) != Some(&b'>') {
                        return Err("a stray / in a tag".into());
                    }
                    closed = true;
                    j += 2;
                    break;
                }
                let k0 = j;
                while j < b.len() && b[j] != b'=' && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                    j += 1;
                }
                let key = s[k0..j].to_string();
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if b.get(j) != Some(&b'=') {
                    return Err(format!("attribute {key} has no value"));
                }
                j += 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                let q = *b.get(j).ok_or("an unclosed start tag")?;
                if q != b'"' && q != b'\'' {
                    return Err(format!("attribute {key}'s value is not quoted"));
                }
                let v0 = j + 1;
                let v1 = s[v0..]
                    .find(q as char)
                    .ok_or("an unclosed attribute value")?
                    + v0;
                el.attrs.push((key, decode_entities(&s[v0..v1])));
                j = v1 + 1;
            }
            count += 1;
            if count > MAX_ELEMENTS {
                return Err("too many elements".into());
            }
            if closed {
                stack.last_mut().unwrap().children.push(el);
            } else {
                if stack.len() > MAX_DEPTH {
                    return Err("elements nest too deep".into());
                }
                stack.push(el);
            }
            i = j;
        }
    }
    if stack.len() != 1 {
        return Err("the document ends inside an element".into());
    }
    let mut holder = stack.pop().unwrap();
    if holder.children.len() != 1 {
        return Err("not exactly one root element".into());
    }
    Ok(holder.children.pop().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_program_shape() {
        let doc = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- c -->\n<MPCVObject>\n  <Program type='Drum'>\n    <ProgramName>Kit &amp; Co &#233;</ProgramName>\n    <Empty/>\n    <Instruments><Instrument number=\"1\"><![CDATA[x<y]]></Instrument></Instruments>\n  </Program>\n</MPCVObject>\n";
        let root = parse(doc).unwrap();
        assert_eq!(root.name, "MPCVObject");
        let progs = root.iter("Program");
        assert_eq!(progs[0].attr("type"), Some("Drum"));
        assert_eq!(progs[0].find_text("ProgramName"), Some("Kit & Co é"));
        assert_eq!(progs[0].find_text("Empty"), Some(""));
        assert_eq!(progs[0].find_text("Nope"), None);
        assert_eq!(root.iter("Instrument")[0].text, "x<y");
    }

    #[test]
    fn rejects_broken_documents() {
        assert!(parse("<a><b></a>").is_err());
        assert!(parse("<a>").is_err());
        assert!(parse("<a x=1/>").is_err());
        assert!(parse("<a/><b/>").is_err());
        let deep = "<a>".repeat(100) + &"</a>".repeat(100);
        assert!(parse(&deep).is_err());
    }
}
