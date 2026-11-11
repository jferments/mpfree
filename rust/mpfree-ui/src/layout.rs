//! Page layouts as short text specs, so a page can be rearranged without
//! touching any widget.
//!
//! ```text
//! col(header:60, row(pads:*, col(kits:*, level:84):480):*, hint:28)
//! ```
//!
//! - `row(...)` places its children left to right, `col(...)` top to bottom,
//!   with the theme's gap between them.
//! - Any other name is a slot: the widget added to the `Ui` under that name.
//! - After a child, `:N` gives it N pixels, `:N*` a share of what is left
//!   (weight N), and `:*` or nothing a share of weight 1.

use crate::geom::Rect;

#[derive(Clone, Debug, PartialEq)]
pub enum Size {
    Px(i32),
    Weight(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Slot(String),
    Row(Vec<(Node, Size)>),
    Col(Vec<(Node, Size)>),
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn err(&self, what: &str) -> String {
        format!("layout: {what} at character {}", self.i + 1)
    }

    fn name(&mut self) -> Result<String, String> {
        self.skip();
        let start = self.i;
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_alphanumeric() || b"_-".contains(&self.s[self.i]))
        {
            self.i += 1;
        }
        if start == self.i {
            return Err(self.err("expected a name"));
        }
        Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    fn node(&mut self) -> Result<Node, String> {
        let name = self.name()?;
        if !self.eat(b'(') {
            return Ok(Node::Slot(name));
        }
        let mut kids = Vec::new();
        loop {
            let kid = self.node()?;
            let size = if self.eat(b':') {
                self.size()?
            } else {
                Size::Weight(1.0)
            };
            kids.push((kid, size));
            if self.eat(b',') {
                continue;
            }
            if self.eat(b')') {
                break;
            }
            return Err(self.err("expected ',' or ')'"));
        }
        match name.as_str() {
            "row" => Ok(Node::Row(kids)),
            "col" => Ok(Node::Col(kids)),
            _ => Err(format!("layout: '{name}(' is not row( or col(")),
        }
    }

    fn size(&mut self) -> Result<Size, String> {
        self.skip();
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.') {
            self.i += 1;
        }
        let num = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
        if self.eat(b'*') {
            let w = if num.is_empty() {
                1.0
            } else {
                num.parse::<f32>().map_err(|_| self.err("bad weight"))?
            };
            if !(w > 0.0 && w <= 1000.0) {
                return Err(self.err("weight must be above 0"));
            }
            return Ok(Size::Weight(w));
        }
        let px = num
            .parse::<i32>()
            .map_err(|_| self.err("expected a size such as 60, 2* or *"))?;
        if !(0..=10_000).contains(&px) {
            return Err(self.err("size out of range"));
        }
        Ok(Size::Px(px))
    }
}

/// Read a spec. Slot names must be unique within it.
pub fn parse(spec: &str) -> Result<Node, String> {
    let mut p = Parser {
        s: spec.as_bytes(),
        i: 0,
    };
    let node = p.node()?;
    if p.peek().is_some() {
        return Err(p.err("unexpected text after the layout"));
    }
    let mut names = slots(&node);
    names.sort();
    if let Some(w) = names.windows(2).find(|w| w[0] == w[1]) {
        return Err(format!("layout: slot '{}' appears twice", w[0]));
    }
    Ok(node)
}

/// Every slot name, in drawing order.
pub fn slots(node: &Node) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(n: &Node, out: &mut Vec<String>) {
        match n {
            Node::Slot(s) => out.push(s.clone()),
            Node::Row(k) | Node::Col(k) => k.iter().for_each(|(c, _)| walk(c, out)),
        }
    }
    walk(node, &mut out);
    out
}

/// Place every slot of `node` inside `r`, `gap` pixels apart.
pub fn solve(node: &Node, r: Rect, gap: i32) -> Vec<(String, Rect)> {
    let mut out = Vec::new();
    place(node, r, gap, &mut out);
    out
}

fn place(node: &Node, r: Rect, gap: i32, out: &mut Vec<(String, Rect)>) {
    let (kids, across) = match node {
        Node::Slot(name) => {
            out.push((name.clone(), r));
            return;
        }
        Node::Row(k) => (k, true),
        Node::Col(k) => (k, false),
    };
    let total = if across { r.w } else { r.h };
    let avail = (total - gap * (kids.len() as i32 - 1)).max(0);
    let fixed: i32 = kids
        .iter()
        .map(|(_, s)| if let Size::Px(p) = s { *p } else { 0 })
        .sum();
    let weights: f32 = kids
        .iter()
        .map(|(_, s)| if let Size::Weight(w) = s { *w } else { 0.0 })
        .sum();
    let mut rest = (avail - fixed).max(0);
    let last_weighted = kids.iter().rposition(|(_, s)| matches!(s, Size::Weight(_)));
    let shares: Vec<i32> = kids
        .iter()
        .enumerate()
        .map(|(i, (_, s))| match s {
            Size::Px(p) => *p,
            Size::Weight(w) => {
                if Some(i) == last_weighted {
                    0 // gets what is left, below
                } else {
                    let n = ((avail - fixed).max(0) as f32 * w / weights).floor() as i32;
                    rest -= n;
                    n
                }
            }
        })
        .collect();
    let mut pos = if across { r.x } else { r.y };
    let end = pos + total;
    for (i, (kid, _)) in kids.iter().enumerate() {
        let start = pos.min(end);
        let len = if Some(i) == last_weighted {
            rest
        } else {
            shares[i]
        }
        .min(end - start);
        let cell = if across {
            Rect::new(start, r.y, len, r.h)
        } else {
            Rect::new(r.x, start, r.w, len)
        };
        place(kid, cell, gap, out);
        pos += len + gap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_places() {
        let n = parse("col(header:60, row(pads:2*, side):*, hint:28)").unwrap();
        let s = solve(&n, Rect::new(0, 0, 1000, 500), 10);
        let get = |k: &str| s.iter().find(|(n, _)| n == k).unwrap().1;
        assert_eq!(get("header"), Rect::new(0, 0, 1000, 60));
        assert_eq!(get("hint"), Rect::new(0, 472, 1000, 28));
        // 500 - 60 - 28 - 2 gaps = 392 for the row; 990 across, 2:1.
        assert_eq!(get("pads"), Rect::new(0, 70, 660, 392));
        assert_eq!(get("side"), Rect::new(670, 70, 330, 392));
        assert_eq!(slots(&n), vec!["header", "pads", "side", "hint"]);
    }

    #[test]
    fn weights_fill_exactly() {
        let n = parse("row(a,b,c)").unwrap();
        let s = solve(&n, Rect::new(0, 0, 100, 10), 0);
        assert_eq!(s.iter().map(|(_, r)| r.w).sum::<i32>(), 100);
        assert_eq!(s[2].1.right(), 100);
    }

    #[test]
    fn too_little_room_never_overflows() {
        let n = parse("col(a:300, b:300, c)").unwrap();
        let s = solve(&n, Rect::new(0, 0, 50, 400), 10);
        for (_, r) in &s {
            assert!(r.bottom() <= 400 && r.h >= 0, "{r:?}");
        }
    }

    #[test]
    fn rejects_bad_specs() {
        for bad in [
            "",
            "row(",
            "row(a b)",
            "grid(a)",
            "row(a:x)",
            "row(a, a)",
            "row(a) b",
            "row(a:0*)",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert_eq!(parse(" pads ").unwrap(), Node::Slot("pads".into()));
    }
}
