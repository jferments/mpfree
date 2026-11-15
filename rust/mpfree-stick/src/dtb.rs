//! Just enough of a flattened device tree reader for the creator: a node's
//! property by path, to read the board file's `compatible` and the panel's
//! `rotation` (the screen log on the stick is turned to match, as
//! `offline-kexec-stick.py` does).

use std::io;

const MAGIC: u32 = 0xd00d_feed;
const BEGIN_NODE: u32 = 1;
const END_NODE: u32 = 2;
const PROP: u32 = 3;
const NOP: u32 = 4;
const END: u32 = 9;

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn be32(b: &[u8], at: usize) -> io::Result<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| bad("the device tree is cut short"))
}

/// A property's value: `path` is the node ("/" for the root), `name` the property.
pub fn prop(dtb: &[u8], path: &str, name: &str) -> io::Result<Option<Vec<u8>>> {
    if be32(dtb, 0)? != MAGIC {
        return Err(bad("not a device tree"));
    }
    let total = be32(dtb, 4)? as usize;
    let off_struct = be32(dtb, 8)? as usize;
    let off_strings = be32(dtb, 12)? as usize;
    if total > dtb.len() || off_struct >= total || off_strings >= total {
        return Err(bad("the device tree's header does not fit it"));
    }
    let want: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let mut stack: Vec<String> = Vec::new();
    let mut at = off_struct;
    let mut depth_seen = 0usize;
    loop {
        let token = be32(dtb, at)?;
        at += 4;
        match token {
            BEGIN_NODE => {
                let end = dtb[at..]
                    .iter()
                    .position(|&c| c == 0)
                    .ok_or_else(|| bad("a node name runs on"))?;
                let name = String::from_utf8_lossy(&dtb[at..at + end]).into_owned();
                at = (at + end + 1 + 3) & !3;
                if depth_seen > 64 {
                    return Err(bad("the device tree nests too deep"));
                }
                depth_seen += 1;
                stack.push(name);
            }
            END_NODE => {
                stack.pop();
                depth_seen = depth_seen.saturating_sub(1);
            }
            PROP => {
                let len = be32(dtb, at)? as usize;
                let nameoff = be32(dtb, at + 4)? as usize;
                at += 8;
                let value = dtb
                    .get(at..at + len)
                    .ok_or_else(|| bad("a property runs past the tree"))?;
                at = (at + len + 3) & !3;
                let s = off_strings + nameoff;
                let end = dtb
                    .get(s..)
                    .and_then(|r| r.iter().position(|&c| c == 0))
                    .ok_or_else(|| bad("a property name runs on"))?;
                let pname = &dtb[s..s + end];
                // The root node's own name is empty; the path below it must match.
                if stack.len() == want.len() + 1
                    && stack[1..].iter().zip(&want).all(|(a, b)| a == b)
                    && pname == name.as_bytes()
                {
                    return Ok(Some(value.to_vec()));
                }
            }
            NOP => {}
            END => return Ok(None),
            other => return Err(bad(format!("unknown device tree token {other}"))),
        }
    }
}

/// The NUL-separated strings of a property such as `compatible`.
pub fn strings(value: &[u8]) -> Vec<String> {
    value
        .split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// The panel's rotation in degrees (0 without one), as the builder reads it.
pub const PANEL: &str = "/mipi@ff960000/panel@0";

pub fn panel_rotation(dtb: &[u8]) -> io::Result<u32> {
    match prop(dtb, PANEL, "rotation")? {
        None => Ok(0),
        Some(v) if v.len() == 4 => {
            let d = u32::from_be_bytes([v[0], v[1], v[2], v[3]]);
            if [0, 90, 180, 270].contains(&d) {
                Ok(d)
            } else {
                Err(bad(format!(
                    "{PANEL}/rotation is {d}, not 0, 90, 180 or 270"
                )))
            }
        }
        Some(_) => Err(bad(format!("{PANEL}/rotation is not one number"))),
    }
}

/// A small tree for tests: root `compatible`, and the panel with `rotation`.
pub mod testtree {
    pub fn make(compatible: &[&str], rotation: Option<u32>) -> Vec<u8> {
        let mut strings = Vec::new();
        let name_off = |n: &str, strings: &mut Vec<u8>| {
            let at = strings.len() as u32;
            strings.extend_from_slice(n.as_bytes());
            strings.push(0);
            at
        };
        let mut s = Vec::new();
        let tok = |s: &mut Vec<u8>, t: u32| s.extend_from_slice(&t.to_be_bytes());
        let node = |s: &mut Vec<u8>, name: &str| {
            s.extend_from_slice(&1u32.to_be_bytes());
            s.extend_from_slice(name.as_bytes());
            s.push(0);
            while !s.len().is_multiple_of(4) {
                s.push(0);
            }
        };
        let prop = |s: &mut Vec<u8>, off: u32, v: &[u8]| {
            s.extend_from_slice(&3u32.to_be_bytes());
            s.extend_from_slice(&(v.len() as u32).to_be_bytes());
            s.extend_from_slice(&off.to_be_bytes());
            s.extend_from_slice(v);
            while !s.len().is_multiple_of(4) {
                s.push(0);
            }
        };
        node(&mut s, "");
        let c: Vec<u8> = compatible
            .iter()
            .flat_map(|x| x.bytes().chain([0]))
            .collect();
        let off = name_off("compatible", &mut strings);
        prop(&mut s, off, &c);
        node(&mut s, "mipi@ff960000");
        node(&mut s, "panel@0");
        if let Some(r) = rotation {
            let off = name_off("rotation", &mut strings);
            prop(&mut s, off, &r.to_be_bytes());
        }
        tok(&mut s, 2);
        tok(&mut s, 2);
        tok(&mut s, 2);
        tok(&mut s, 9);
        let header = 40usize;
        let rsv = 16usize;
        let off_struct = header + rsv;
        let off_strings = off_struct + s.len();
        let total = off_strings + strings.len();
        let mut out = Vec::new();
        for v in [
            0xd00d_feedu32,
            total as u32,
            off_struct as u32,
            off_strings as u32,
            header as u32,
            17,
            16,
            0,
            strings.len() as u32,
            s.len() as u32,
        ] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&[0u8; 16]);
        out.extend_from_slice(&s);
        out.extend_from_slice(&strings);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_compatible_and_rotation() {
        let t = testtree::make(
            &["inmusic,acva", "inmusic,az01", "rockchip,rk3288"],
            Some(270),
        );
        assert_eq!(
            strings(&prop(&t, "/", "compatible").unwrap().unwrap()),
            ["inmusic,acva", "inmusic,az01", "rockchip,rk3288"]
        );
        assert_eq!(panel_rotation(&t).unwrap(), 270);
        assert_eq!(panel_rotation(&testtree::make(&["x"], None)).unwrap(), 0);
        assert!(panel_rotation(&testtree::make(&["x"], Some(45))).is_err());
        assert!(prop(b"not a tree at all", "/", "x").is_err());
    }
}
