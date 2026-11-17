//! Flattened device trees for the creator: a node's property by path (the
//! board file's `compatible` and the panel's `rotation`, which the screen
//! log on the stick is turned to match), and for the chain route the whole
//! tree, read and written, to build `kexec.dtb` as the builder does
//! (`tools/mpc-one/offline-dtb.py` kexec-dtb and `offline-kexec-stick.py`'s
//! build_kexec_dtb): the update's board file plus what U-Boot added on this
//! MPC, with the eMMC and SD card controllers disabled and the Ethernet
//! PHY's ID. `tests/data/kexec-dtb-example.json`, written by the builder's
//! side, holds trees and the bytes and notes this must give for them.

use crate::recipe::KexecDtb;
use std::collections::{HashMap, HashSet};
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

/// The largest tree read (the builder's MAX_SIZE).
pub const MAX_SIZE: usize = 4 << 20;
const MAX_DEPTH: usize = 64;

/// A node: its name, its properties in file order, its children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub props: Vec<(String, Vec<u8>)>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(name: &str) -> Node {
        Node {
            name: name.to_string(),
            props: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn prop(&self, name: &str) -> Option<&[u8]> {
        self.props
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_slice())
    }

    pub fn set_prop(&mut self, name: &str, value: Vec<u8>) {
        match self.props.iter_mut().find(|(n, _)| n == name) {
            Some(p) => p.1 = value,
            None => self.props.push((name.to_string(), value)),
        }
    }

    pub fn del_prop(&mut self, name: &str) {
        self.props.retain(|(n, _)| n != name);
    }
}

/// A whole tree: the root node, the memory reservations, the boot CPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    pub root: Node,
    pub reserved: Vec<(u64, u64)>,
    pub boot_cpuid: u32,
}

fn path_parts(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|p| !p.is_empty())
}

impl Tree {
    /// (path, node) for every node, parents first, level by level.
    pub fn walk(&self) -> Vec<(String, &Node)> {
        let mut out = vec![("/".to_string(), &self.root)];
        let mut i = 0;
        while i < out.len() {
            let (path, node) = (out[i].0.clone(), out[i].1);
            for c in &node.children {
                out.push((format!("{}/{}", path.trim_end_matches('/'), c.name), c));
            }
            i += 1;
        }
        out
    }

    pub fn find(&self, path: &str) -> Option<&Node> {
        let mut node = &self.root;
        for part in path_parts(path) {
            node = node.children.iter().find(|c| c.name == part)?;
        }
        Some(node)
    }

    pub fn find_mut(&mut self, path: &str) -> Option<&mut Node> {
        let mut node = &mut self.root;
        for part in path_parts(path) {
            node = node.children.iter_mut().find(|c| c.name == part)?;
        }
        Some(node)
    }
}

/// Reads a whole tree, with the builder's checks.
pub fn parse(blob: &[u8]) -> io::Result<Tree> {
    if blob.len() < 40 {
        return Err(bad("file too short for an FDT header"));
    }
    let h = |i: usize| be32(blob, i * 4).map(|v| v as usize);
    let (magic, total, off_struct, off_strings, off_rsv, version) =
        (h(0)?, h(1)?, h(2)?, h(3)?, h(4)?, h(5)?);
    let (boot_cpuid, size_strings, size_struct) = (h(7)? as u32, h(8)?, h(9)?);
    if magic as u32 != MAGIC {
        return Err(bad(format!(
            "not a device tree (starts {})",
            crate::update::hex(&blob[..4])
        )));
    }
    if total > blob.len() || total > MAX_SIZE {
        return Err(bad(format!(
            "header says {total} bytes, file has {}",
            blob.len()
        )));
    }
    if version < 16 {
        return Err(bad(format!("FDT version {version} is too old")));
    }
    for (off, len, what) in [
        (off_struct, size_struct, "structure"),
        (off_strings, size_strings, "strings"),
    ] {
        if off + len > total {
            return Err(bad(format!("FDT {what} block runs past the end")));
        }
    }
    let mut reserved = Vec::new();
    let mut pos = off_rsv;
    loop {
        if pos + 16 > total {
            return Err(bad("memory reservation map runs past the end"));
        }
        let q = |at: usize| u64::from_be_bytes(blob[at..at + 8].try_into().expect("8 bytes"));
        let (addr, size) = (q(pos), q(pos + 8));
        pos += 16;
        if addr == 0 && size == 0 {
            break;
        }
        reserved.push((addr, size));
    }
    let strings = &blob[off_strings..off_strings + size_strings];
    let end = off_struct + size_struct;
    let mut pos = off_struct;
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    while pos < end {
        let token = be32(blob, pos)?;
        pos += 4;
        match token {
            BEGIN_NODE => {
                let nul = blob[pos.min(end)..end]
                    .iter()
                    .position(|&c| c == 0)
                    .ok_or_else(|| bad(format!("unterminated node name at offset {pos}")))?;
                let name = String::from_utf8_lossy(&blob[pos..pos + nul]).into_owned();
                pos = (pos + nul + 1 + 3) & !3;
                if stack.is_empty() && root.is_some() {
                    return Err(bad("more than one root node"));
                }
                stack.push(Node::new(&name));
                if stack.len() > MAX_DEPTH {
                    return Err(bad("nesting is implausibly deep"));
                }
            }
            END_NODE => {
                let node = stack
                    .pop()
                    .ok_or_else(|| bad(format!("unbalanced end of node at offset {}", pos - 4)))?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            PROP => {
                if stack.is_empty() || pos + 8 > end {
                    return Err(bad(format!("bad property at offset {}", pos - 4)));
                }
                let len = be32(blob, pos)? as usize;
                let name_off = be32(blob, pos + 4)? as usize;
                pos += 8;
                let name_end = strings
                    .get(name_off..)
                    .and_then(|r| r.iter().position(|&c| c == 0));
                let Some(name_len) = name_end.filter(|_| name_off < strings.len()) else {
                    return Err(bad(format!(
                        "property name offset {name_off} is outside the strings block"
                    )));
                };
                if pos + len > end {
                    return Err(bad("property value runs past the structure block"));
                }
                let name = String::from_utf8_lossy(&strings[name_off..name_off + name_len]);
                stack
                    .last_mut()
                    .expect("checked above")
                    .props
                    .push((name.into_owned(), blob[pos..pos + len].to_vec()));
                pos = (pos + len + 3) & !3;
            }
            NOP => {}
            END => break,
            other => {
                return Err(bad(format!(
                    "unknown token 0x{other:x} at offset {}",
                    pos - 4
                )))
            }
        }
    }
    if !stack.is_empty() {
        return Err(bad("structure block ends inside a node"));
    }
    let root = root.ok_or_else(|| bad("no root node"))?;
    Ok(Tree {
        root,
        reserved,
        boot_cpuid,
    })
}

/// Writes a tree as an FDT version 17 blob, byte for byte as the builder does.
pub fn build(tree: &Tree) -> Vec<u8> {
    let mut strings: Vec<u8> = Vec::new();
    let mut offsets: HashMap<String, u32> = HashMap::new();
    let mut st: Vec<u8> = Vec::new();
    fn pad(st: &mut Vec<u8>) {
        while !st.len().is_multiple_of(4) {
            st.push(0);
        }
    }
    fn emit(
        node: &Node,
        st: &mut Vec<u8>,
        strings: &mut Vec<u8>,
        offsets: &mut HashMap<String, u32>,
    ) {
        st.extend_from_slice(&BEGIN_NODE.to_be_bytes());
        st.extend_from_slice(node.name.as_bytes());
        st.push(0);
        pad(st);
        for (name, value) in &node.props {
            let off = *offsets.entry(name.clone()).or_insert_with(|| {
                let at = strings.len() as u32;
                strings.extend_from_slice(name.as_bytes());
                strings.push(0);
                at
            });
            for v in [PROP, value.len() as u32, off] {
                st.extend_from_slice(&v.to_be_bytes());
            }
            st.extend_from_slice(value);
            pad(st);
        }
        for c in &node.children {
            emit(c, st, strings, offsets);
        }
        st.extend_from_slice(&END_NODE.to_be_bytes());
    }
    emit(&tree.root, &mut st, &mut strings, &mut offsets);
    st.extend_from_slice(&END.to_be_bytes());
    let mut rsv = Vec::new();
    for (a, s) in tree.reserved.iter().chain([&(0, 0)]) {
        rsv.extend_from_slice(&a.to_be_bytes());
        rsv.extend_from_slice(&s.to_be_bytes());
    }
    let off_rsv = 40usize;
    let off_struct = off_rsv + rsv.len();
    let off_strings = off_struct + st.len();
    let total = off_strings + strings.len();
    let mut out = Vec::with_capacity(total);
    for v in [
        MAGIC,
        total as u32,
        off_struct as u32,
        off_strings as u32,
        off_rsv as u32,
        17,
        16,
        tree.boot_cpuid,
        strings.len() as u32,
        st.len() as u32,
    ] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&rsv);
    out.extend_from_slice(&st);
    out.extend_from_slice(&strings);
    out
}

/// The NUL-separated printable strings in `value`, or None if it is not a
/// string list (the builder's strings_of).
pub fn strings_of(value: &[u8]) -> Option<Vec<String>> {
    let body = value.strip_suffix(&[0])?;
    let parts: Vec<&[u8]> = body.split(|&c| c == 0).collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || p.iter().any(|&b| !(0x20..=0x7e).contains(&b)))
    {
        return None;
    }
    Some(
        parts
            .iter()
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect(),
    )
}

fn str_list(texts: &[String]) -> Vec<u8> {
    texts.iter().flat_map(|t| t.bytes().chain([0])).collect()
}

/// A value as the builder's report shows it: strings quoted, else cells, else bytes.
fn show(value: &[u8]) -> String {
    if let Some(t) = strings_of(value) {
        return t
            .iter()
            .map(|x| format!("\"{x}\""))
            .collect::<Vec<_>>()
            .join(", ");
    }
    if value.is_empty() {
        return "(empty)".into();
    }
    if value.len().is_multiple_of(4) {
        let cells: Vec<String> = value
            .chunks(4)
            .take(32)
            .map(|c| format!("0x{:x}", u32::from_be_bytes([c[0], c[1], c[2], c[3]])))
            .collect();
        let more = if value.len() / 4 > 32 { " ..." } else { "" };
        return format!("<{}{more}>", cells.join(" "));
    }
    let shown = &value[..value.len().min(64)];
    let more = if value.len() > 64 { "..." } else { "" };
    format!("[{}{more}]", crate::update::hex(shown))
}

/// The root's `compatible` list.
pub fn compatible(tree: &Tree) -> Vec<String> {
    tree.root
        .prop("compatible")
        .and_then(strings_of)
        .unwrap_or_default()
}

fn unit_address(name: &str) -> &str {
    name.split_once('@').map_or("", |(_, a)| a)
}

/// The paths of every node at this unit address.
fn nodes_at(tree: &Tree, address: &str) -> Vec<String> {
    tree.walk()
        .into_iter()
        .filter(|(_, n)| unit_address(&n.name).to_ascii_lowercase() == address)
        .map(|(p, _)| p)
        .collect()
}

fn status_of(node: &Node) -> String {
    node.prop("status")
        .and_then(strings_of)
        .and_then(|t| t.into_iter().next())
        .unwrap_or_else(|| "okay (no status property)".into())
}

fn parent_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/".into(),
        Some((p, _)) => p.into(),
    }
}

fn prop_path(path: &str, name: &str) -> String {
    format!("{}/{name}", path.trim_end_matches('/'))
}

/// {phandle: path} from every node's phandle (or linux,phandle).
fn phandles(tree: &Tree) -> HashMap<u32, String> {
    let mut found = HashMap::new();
    for (path, node) in tree.walk() {
        for name in ["phandle", "linux,phandle"] {
            if let Some(v) = node.prop(name).filter(|v| v.len() == 4) {
                found.insert(u32::from_be_bytes([v[0], v[1], v[2], v[3]]), path.clone());
            }
        }
    }
    found
}

/// The builder's kexec_dtb: applies U-Boot's changes (`reference` to
/// `live`) to `target`, drops the properties in `drops`, puts each
/// `path=STRING` of `adds` first in that node's compatible list, and
/// disables the hidden controllers. Returns (notes, problems): notes name
/// every change (never a value); problems are reasons to stop.
pub fn kexec_dtb(
    target: &mut Tree,
    reference: &Tree,
    live: &Tree,
    drops: &[String],
    adds: &[String],
    k: &KexecDtb,
) -> (Vec<String>, Vec<String>) {
    let (mut notes, mut problems) = (Vec::new(), Vec::new());
    let ref_walk = reference.walk();
    let ref_nodes: HashMap<&str, &Node> = ref_walk.iter().map(|(p, n)| (p.as_str(), *n)).collect();
    let live_walk = live.walk();
    let live_paths: HashSet<&str> = live_walk.iter().map(|(p, _)| p.as_str()).collect();
    for (path, lnode) in &live_walk {
        let Some(rnode) = ref_nodes.get(path.as_str()) else {
            let parent = parent_of(path);
            if !ref_nodes.contains_key(parent.as_str()) {
                continue; // inside a node that is copied whole
            }
            if target.find(&parent).is_none() {
                problems.push(format!(
                    "U-Boot added node {path}, but the new tree has no {parent}"
                ));
            } else if target.find(path).is_some() {
                problems.push(format!(
                    "U-Boot added node {path}, but the new tree already has one"
                ));
            } else {
                let p = target.find_mut(&parent).expect("found above");
                p.children.push((*lnode).clone());
                notes.push(format!("copied node added by U-Boot: {path}"));
            }
            continue;
        };
        for (name, value) in &lnode.props {
            if rnode.prop(name) == Some(value.as_slice()) {
                continue;
            }
            if path == "/chosen" && k.chosen_not_copied.contains(name) {
                notes.push(format!(
                    "not copied (kexec sets it): {}",
                    prop_path(path, name)
                ));
                continue;
            }
            let Some(tnode) = target.find_mut(path) else {
                problems.push(format!(
                    "U-Boot set {}, but the new tree has no {path}",
                    prop_path(path, name)
                ));
                continue;
            };
            let what = if tnode.prop(name).is_some() {
                "replaced"
            } else {
                "added"
            };
            tnode.set_prop(name, value.clone());
            notes.push(format!(
                "{what} from the running tree ({} bytes): {}",
                value.len(),
                prop_path(path, name)
            ));
        }
        for (name, _) in &rnode.props {
            if lnode.prop(name).is_none() {
                problems.push(format!(
                    "U-Boot removed {}; decide by hand",
                    prop_path(path, name)
                ));
            }
        }
    }
    for (path, _) in &ref_walk {
        if !live_paths.contains(path.as_str()) && live_paths.contains(parent_of(path).as_str()) {
            problems.push(format!("U-Boot removed node {path}; decide by hand"));
        }
    }
    for item in drops {
        let (path, name) = (parent_of(item), item.rsplit('/').next().unwrap_or(""));
        match target.find_mut(&path) {
            Some(node) if node.prop(name).is_some() => {
                node.del_prop(name);
                notes.push(format!("dropped as asked: {item}"));
            }
            _ => problems.push(format!("--drop {item}: no such property in the result")),
        }
    }
    for item in adds {
        let (path, text) = item.split_once('=').unwrap_or((item, ""));
        let node = if path.starts_with('/') {
            target.find_mut(path)
        } else {
            None
        };
        let old = node
            .as_ref()
            .map(|n| n.prop("compatible").unwrap_or_default())
            .and_then(strings_of);
        let good_text = !text.is_empty()
            && strings_of(&str_list(&[text.to_string()])) == Some(vec![text.to_string()]);
        let (Some(node), Some(old), true) = (node, old, good_text) else {
            problems.push(format!(
                "--add-compatible {item}: no such node with a compatible list, or a bad string"
            ));
            continue;
        };
        if old.iter().any(|o| o == text) {
            problems.push(format!("--add-compatible {item}: the node already has it"));
            continue;
        }
        let mut new = vec![text.to_string()];
        new.extend(old);
        node.set_prop("compatible", str_list(&new));
        notes.push(format!(
            "compatible of {path} is now {}",
            show(node.prop("compatible").unwrap_or_default())
        ));
    }
    if live.reserved != target.reserved {
        notes.push(format!(
            "not copied: memory reservations (running tree {}, result keeps the new file's {})",
            live.reserved.len(),
            target.reserved.len()
        ));
    }
    for (label, address) in &k.hidden_controllers {
        let found = nodes_at(target, address);
        let [path] = found.as_slice() else {
            problems.push(format!(
                "expected one node at the {label} address {address} in the new tree, found {}",
                found.len()
            ));
            continue;
        };
        target
            .find_mut(path)
            .expect("walked")
            .set_prop("status", b"disabled\0".to_vec());
        notes.push(format!(
            "{label} controller disabled: {path} status = \"disabled\""
        ));
    }
    (notes, problems)
}

/// Reasons the built tree must not be used (the builder's verify_kexec_dtb).
pub fn verify_kexec_dtb(tree: &Tree, board: &str, k: &KexecDtb) -> Vec<String> {
    let mut reasons = Vec::new();
    let compat = compatible(tree);
    if !compat.iter().any(|c| c == board) {
        reasons.push(format!(
            "the tree is not for {board}: {}",
            compat.join(", ")
        ));
    }
    for (label, address) in &k.hidden_controllers {
        let found = nodes_at(tree, address);
        let disabled = match found.as_slice() {
            [path] => {
                tree.find(path)
                    .and_then(|n| n.prop("status"))
                    .and_then(strings_of)
                    == Some(vec!["disabled".to_string()])
            }
            _ => false,
        };
        if !disabled {
            reasons.push(format!("the {label} controller is not disabled"));
        }
    }
    for (path, node) in tree.walk() {
        if (node.name == "memory" || node.name.starts_with("memory@"))
            && !node.prop("reg").unwrap_or(&[0]).iter().any(|&b| b != 0)
        {
            reasons.push(format!(
                "{path} has no memory size (U-Boot normally fills it in)"
            ));
        }
    }
    reasons
}

/// The Ethernet PHY node the MAC points to, checked to be the layout the
/// PHY ID is for (the builder's phy_node).
pub fn phy_node(tree: &Tree, k: &KexecDtb) -> io::Result<String> {
    let eth = &k.ethernet;
    let handle = tree
        .find(eth)
        .and_then(|g| g.prop("phy-handle"))
        .filter(|h| h.len() == 4)
        .map(|h| u32::from_be_bytes([h[0], h[1], h[2], h[3]]));
    let path = handle.and_then(|h| phandles(tree).remove(&h));
    let node = path.as_deref().and_then(|p| tree.find(p));
    let (Some(path), Some(node)) = (path.clone(), node) else {
        return Err(bad(format!(
            "the board file's {eth} has no phy-handle to a PHY node under it"
        )));
    };
    if !path.starts_with(&format!("{eth}/")) {
        return Err(bad(format!(
            "the board file's {eth} has no phy-handle to a PHY node under it"
        )));
    }
    if node.prop("reg") != Some(&1u32.to_be_bytes()[..])
        || node.prop("reset-gpios").is_none()
        || node.prop("compatible").and_then(strings_of)
            != Some(vec!["ethernet-phy-ieee802.3-c22".to_string()])
    {
        return Err(bad(format!(
            "the PHY node {path} is not at address 1 with compatible \"ethernet-phy-ieee802.3-c22\" and a reset-gpios line; this app does not know this layout"
        )));
    }
    Ok(path)
}

/// The chain's kexec.dtb, as built.
pub struct KexecTree {
    pub blob: Vec<u8>,
    /// What changed, by name (never a value: the running tree carries the
    /// MPC's serial number and MAC address).
    pub notes: Vec<String>,
    /// The Ethernet PHY's node.
    pub phy: String,
}

/// kexec.dtb for the chain, as offline-kexec-stick.py builds it for an MPC
/// running the stick's own release: `board` is the update's board file
/// (the stick's kernel's, and the one U-Boot started the MPC with) and
/// `live` the MPC's running tree (collect-over-ssh.sh --kexec-preflight's
/// live.dtb), read on the installed system.
pub fn kexec_tree(board: &[u8], live: &[u8], k: &KexecDtb) -> io::Result<KexecTree> {
    let board_t = parse(board).map_err(|e| bad(format!("the update's board file: {e}")))?;
    let live_t = parse(live).map_err(|e| bad(format!("the running tree: {e}")))?;
    if compatible(&live_t) != compatible(&board_t) {
        return Err(bad(format!(
            "the running tree is for {}, not for the update's board file ({})",
            compatible(&live_t).join(", "),
            compatible(&board_t).join(", ")
        )));
    }
    // It must be the tree U-Boot gave the installed system, not one a test
    // build started by kexec has (its eMMC disabled: our own changes would
    // look like U-Boot's).
    let emmc = k
        .hidden_controllers
        .iter()
        .find(|(label, _)| label == "eMMC")
        .map(|(_, a)| nodes_at(&live_t, a))
        .unwrap_or_default();
    let enabled = match emmc.as_slice() {
        [path] => live_t.find(path).map(status_of).as_deref() != Some("disabled"),
        _ => false,
    };
    if !enabled {
        return Err(bad("the running tree has no enabled eMMC controller: it was read on a stick's system, not on the installed one. Read it again on the installed system (collect-over-ssh.sh --kexec-preflight, MPC_PORT=22 for developer mode)"));
    }
    let mut drops = Vec::new();
    let flag = k.emmc_content.rsplit('/').next().unwrap_or("");
    if board_t
        .find(&parent_of(&k.emmc_content))
        .and_then(|n| n.prop(flag))
        .is_some()
    {
        drops.push(k.emmc_content.clone());
    }
    let phy = phy_node(&board_t, k)?;
    let board_name = compatible(&live_t)
        .into_iter()
        .next()
        .unwrap_or_else(|| "?".into());
    if !compatible(&board_t).contains(&board_name) {
        return Err(bad(format!(
            "the update's board file is not a {board_name} tree"
        )));
    }
    let mut target = board_t.clone();
    let (notes, problems) = kexec_dtb(
        &mut target,
        &board_t,
        &live_t,
        &drops,
        &[format!("{phy}={}", k.phy_compatible)],
        k,
    );
    let blob = build(&target);
    let mut reasons = problems;
    reasons.extend(verify_kexec_dtb(&parse(&blob)?, &board_name, k));
    if !reasons.is_empty() {
        return Err(bad(format!(
            "kexec.dtb can't be built: {}",
            reasons.join("; ")
        )));
    }
    let first = parse(&blob)?
        .find(&phy)
        .and_then(|n| n.prop("compatible"))
        .and_then(strings_of)
        .and_then(|t| t.into_iter().next());
    if first.as_deref() != Some(k.phy_compatible.as_str()) {
        return Err(bad(format!(
            "{phy} in kexec.dtb does not start with {}",
            k.phy_compatible
        )));
    }
    Ok(KexecTree { blob, notes, phy })
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

    fn unhex(t: &str) -> Vec<u8> {
        (0..t.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&t[i..i + 2], 16).unwrap())
            .collect()
    }

    struct Example {
        board: Vec<u8>,
        live: Vec<u8>,
        kexec_dtb: Vec<u8>,
        notes: Vec<String>,
        phy: String,
    }

    fn example() -> Example {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/kexec-dtb-example.json")).unwrap();
        let h = |k: &str| unhex(v[k].as_str().unwrap());
        Example {
            board: h("board"),
            live: h("live"),
            kexec_dtb: h("kexec_dtb"),
            notes: v["notes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_str().unwrap().to_string())
                .collect(),
            phy: v["phy"].as_str().unwrap().to_string(),
        }
    }

    #[test]
    fn trees_written_by_the_builder_read_and_write_back_byte_for_byte() {
        let e = example();
        for blob in [&e.board, &e.live, &e.kexec_dtb] {
            let t = parse(blob).unwrap();
            assert_eq!(build(&t), *blob);
            assert_eq!(parse(&build(&t)).unwrap(), t);
        }
        let t = parse(&e.live).unwrap();
        assert_eq!(t.reserved.len(), 2);
        assert_eq!(
            t.find("/ethernet@ff290000/mdio/ethernet-phy@1")
                .unwrap()
                .prop("reg"),
            Some(&[0, 0, 0, 1][..])
        );
        let walked: Vec<String> = t.walk().into_iter().map(|(p, _)| p).collect();
        assert_eq!(walked[0], "/");
        assert!(walked.contains(&"/reserved-memory/ramoops@8000000".to_string()));
        for broken in [
            &e.board[..30],
            &e.board[..e.board.len() - 8],
            b"\0\0\0\0 not a device tree at all, no".as_slice(),
        ] {
            assert!(parse(broken).is_err());
        }
    }

    #[test]
    fn builds_the_same_kexec_dtb_as_the_builder() {
        let e = example();
        let k = crate::recipe::Recipe::load().kexec_dtb;
        let made = kexec_tree(&e.board, &e.live, &k).unwrap();
        assert_eq!(made.notes, e.notes);
        assert_eq!(made.phy, e.phy);
        assert_eq!(
            made.blob, e.kexec_dtb,
            "byte for byte the builder's kexec.dtb"
        );
        let t = parse(&made.blob).unwrap();
        for path in ["/dwmmc@ff0f0000", "/dwmmc@ff0c0000"] {
            assert_eq!(
                t.find(path).unwrap().prop("status"),
                Some(&b"disabled\0"[..])
            );
        }
        for note in &made.notes {
            assert!(!note.contains("EXAMPLE-SERIAL") && !note.contains("020000000001"));
        }
    }

    #[test]
    fn kexec_dtb_refuses_what_the_builder_refuses() {
        let e = example();
        let k = crate::recipe::Recipe::load().kexec_dtb;
        let live = parse(&e.live).unwrap();
        let board = parse(&e.board).unwrap();
        let refused = |board: &Tree, live: &Tree| match kexec_tree(&build(board), &build(live), &k)
        {
            Ok(_) => panic!("built"),
            Err(err) => err.to_string(),
        };
        // A running tree read on a stick's system: its eMMC is disabled.
        let mut stick = live.clone();
        stick
            .find_mut("/dwmmc@ff0f0000")
            .unwrap()
            .set_prop("status", b"disabled\0".to_vec());
        assert!(refused(&board, &stick).contains("no enabled eMMC controller"));
        // Another board.
        let mut other = live.clone();
        other
            .root
            .set_prop("compatible", str_list(&["inmusic,acvb".into()]));
        assert!(refused(&board, &other).contains("the running tree is for inmusic,acvb"));
        // U-Boot took a property away: decided by hand, not here.
        let mut removed = live.clone();
        removed.find_mut("/chosen").unwrap().del_prop("stdout-path");
        assert!(refused(&board, &removed)
            .contains("U-Boot removed /chosen/stdout-path; decide by hand"));
        // No memory size.
        let mut no_size = live.clone();
        no_size
            .find_mut("/memory@0")
            .unwrap()
            .set_prop("reg", vec![0; 8]);
        assert!(refused(&board, &no_size).contains("/memory@0 has no memory size"));
        // A PHY this layout is not for, then none.
        let (mut b, mut l) = (board.clone(), live.clone());
        for t in [&mut b, &mut l] {
            t.find_mut("/ethernet@ff290000/mdio/ethernet-phy@1")
                .unwrap()
                .set_prop("reg", 2u32.to_be_bytes().to_vec());
        }
        assert!(refused(&b, &l).contains("is not at address 1"));
        for t in [&mut b, &mut l] {
            t.find_mut("/ethernet@ff290000")
                .unwrap()
                .del_prop("phy-handle");
        }
        assert!(refused(&b, &l).contains("has no phy-handle"));
        // No SD controller in the board file.
        let (mut b, mut l) = (board.clone(), live.clone());
        for t in [&mut b, &mut l] {
            t.root.children.retain(|c| c.name != "dwmmc@ff0c0000");
        }
        assert!(refused(&b, &l).contains(
            "expected one node at the SD card address ff0c0000 in the new tree, found 0"
        ));
    }
}
