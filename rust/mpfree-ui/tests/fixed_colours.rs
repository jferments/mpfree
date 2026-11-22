//! No new fixed colours in drawing code.
//!
//! Screens ask the theme for colours (its core colours, its roles, or a
//! content colour through `Theme::swatch`), so every theme can restyle
//! them. A colour written into the code (`0x6EA8FE`, `Color::rgb(0)`) looks
//! right in one theme only. This test counts such literals in the code of
//! every crate that draws (mpfree-ui, mpfree-app, mpfree-mpcskin and every
//! mpfree-*-ui), tests left out, and holds each file to the count in
//! tests/fixed-colours.txt. The counts may only go down: a file with more
//! fails, naming its lines; a file with fewer passes, and the theme work
//! lowers its count as it goes. Write the counts with
//!
//!     MPFREE_LINT_BLESS=1 cargo test -p mpfree-ui --test fixed_colours
//!
//! A file listed with `*` is left out: where colours are meant to be
//! written down (the built-in themes) or where hex numbers aren't colours.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn rust_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn list_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixed-colours.txt")
}

/// The crates that draw.
fn ui_crates(rust: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(rust)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| {
            n == "mpfree-ui"
                || n == "mpfree-app"
                || n == "mpfree-mpcskin"
                || (n.starts_with("mpfree-") && n.ends_with("-ui"))
        })
        .filter(|n| rust.join(n).join("src").is_dir())
        .collect();
    v.sort();
    v
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The fixed colours in `src`, by line: hex numbers of six or eight digits
/// (`0xRRGGBB`, `0xAARRGGBB`, underscores allowed) that aren't one side of
/// a bit operation, and `Color::rgb(0)`. Comments, strings and anything
/// under `#[cfg(test)]` are skipped.
fn fixed_colours(src: &str) -> Vec<(usize, String)> {
    let b = src.as_bytes();
    let (mut i, mut line) = (0usize, 1usize);
    let mut depth = 0i32;
    // After #[cfg(test)]: until its item's `;`, or its `{ ... }` closes.
    let mut pending = false;
    let mut skip: Option<i32> = None;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut d = 1;
            i += 2;
            while i < b.len() && d > 0 {
                if b[i] == b'\n' {
                    line += 1;
                }
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    d += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    d -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // Raw strings: r"..", r#".."#, br"..".
        if (c == b'r' || (c == b'b' && b.get(i + 1) == Some(&b'r'))) && (i == 0 || !ident(b[i - 1]))
        {
            let mut k = if c == b'b' { i + 2 } else { i + 1 };
            let mut hashes = 0;
            while b.get(k) == Some(&b'#') {
                hashes += 1;
                k += 1;
            }
            if b.get(k) == Some(&b'"') {
                k += 1;
                while k < b.len() {
                    if b[k] == b'\n' {
                        line += 1;
                    }
                    if b[k] == b'"'
                        && b[k + 1..].iter().take_while(|&&x| x == b'#').count() >= hashes
                    {
                        k += 1 + hashes;
                        break;
                    }
                    k += 1;
                }
                i = k;
                continue;
            }
        }
        if c == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                if i < b.len() && b[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == b'\'' {
            // A char literal, or a lifetime.
            if b.get(i + 1) == Some(&b'\\') {
                i += 2;
                while i < b.len() && b[i] != b'\'' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            let len = match b.get(i + 1) {
                Some(&x) if x >= 0xF0 => 4,
                Some(&x) if x >= 0xE0 => 3,
                Some(&x) if x >= 0xC0 => 2,
                _ => 1,
            };
            if b.get(i + 1 + len) == Some(&b'\'') {
                i += 2 + len;
            } else {
                i += 1;
            }
            continue;
        }
        if src[i..].starts_with("#[cfg(test)]") {
            if skip.is_none() {
                pending = true;
            }
            i += "#[cfg(test)]".len();
            continue;
        }
        match c {
            b'{' => {
                if pending && skip.is_none() {
                    skip = Some(depth);
                    pending = false;
                }
                depth += 1;
                i += 1;
                continue;
            }
            b'}' => {
                depth -= 1;
                if skip == Some(depth) {
                    skip = None;
                }
                i += 1;
                continue;
            }
            b';' => {
                pending = false;
                i += 1;
                continue;
            }
            _ => {}
        }
        let counting = skip.is_none() && !pending;
        if c == b'0' && b.get(i + 1) == Some(&b'x') && (i == 0 || !ident(b[i - 1])) {
            let start = i;
            i += 2;
            let mut digits = 0;
            while i < b.len() && (b[i].is_ascii_hexdigit() || b[i] == b'_') {
                if b[i] != b'_' {
                    digits += 1;
                }
                i += 1;
            }
            let before = src[..start].trim_end();
            let bitwise = before.ends_with(['&', '|', '^'])
                || before.ends_with("<<")
                || before.ends_with(">>");
            let after = src[i..].trim_start();
            let bitwise = bitwise
                || (after.starts_with(['&', '|', '^'])
                    && !after.starts_with("&&")
                    && !after.starts_with("||"));
            if counting && (digits == 6 || digits == 8) && !bitwise {
                out.push((line, src[start..i].to_string()));
            }
            continue;
        }
        if counting && src[i..].starts_with("Color::rgb(0)") {
            out.push((line, "Color::rgb(0)".into()));
            i += "Color::rgb(0)".len();
            continue;
        }
        i += 1;
    }
    out
}

/// file to (allowed, reason); None allowed means left out.
type List = BTreeMap<String, (Option<usize>, String)>;

fn read_list(text: &str) -> List {
    let mut list = List::new();
    for (n, l) in text.lines().enumerate() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let (fields, reason) = l.split_once(" # ").unwrap_or((l, ""));
        let mut f = fields.split_whitespace();
        let (Some(path), Some(count), None) = (f.next(), f.next(), f.next()) else {
            panic!(
                "tests/fixed-colours.txt line {}: expected PATH COUNT # REASON",
                n + 1
            );
        };
        let count = match count {
            "*" => None,
            c => Some(c.parse().unwrap_or_else(|_| {
                panic!(
                    "tests/fixed-colours.txt line {}: {c:?} is not a count",
                    n + 1
                )
            })),
        };
        list.insert(path.to_string(), (count, reason.trim().to_string()));
    }
    list
}

#[test]
fn no_new_fixed_colours_in_drawing_code() {
    let rust = rust_dir();
    let mut got: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    for krate in ui_crates(&rust) {
        let mut files = Vec::new();
        rs_files(&rust.join(&krate).join("src"), &mut files);
        files.sort();
        for f in files {
            let rel = f
                .strip_prefix(&rust)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let src = std::fs::read_to_string(&f).unwrap();
            let found = fixed_colours(&src);
            if !found.is_empty() {
                got.insert(rel, found);
            }
        }
    }
    let old = std::fs::read_to_string(list_path())
        .map(|t| read_list(&t))
        .unwrap_or_default();
    if std::env::var_os("MPFREE_LINT_BLESS").is_some() {
        let mut text = String::from(
            "# Fixed colours each drawing file may still have: PATH COUNT # REASON, or\n\
             # PATH * # REASON for a file left out. Checked by tests/fixed_colours.rs;\n\
             # counts may only go down. Write them with\n\
             #     MPFREE_LINT_BLESS=1 cargo test -p mpfree-ui --test fixed_colours\n",
        );
        let mut paths: Vec<&String> = got.keys().chain(old.keys()).collect();
        paths.sort();
        paths.dedup();
        for p in paths {
            match (old.get(p), got.get(p)) {
                (Some((None, reason)), _) => text.push_str(&format!("{p} * # {reason}\n")),
                (old_line, Some(found)) => {
                    let reason = old_line
                        .map(|(_, r)| r.clone())
                        .filter(|r| !r.is_empty())
                        .unwrap_or_else(|| "before theme roles (phase 0 baseline)".into());
                    text.push_str(&format!("{p} {} # {reason}\n", found.len()));
                }
                _ => {}
            }
        }
        std::fs::write(list_path(), text).unwrap();
        eprintln!("wrote {}", list_path().display());
        return;
    }
    let mut problems = Vec::new();
    for (path, found) in &got {
        match old.get(path) {
            Some((None, _)) => {}
            entry => {
                let allowed = entry.and_then(|e| e.0).unwrap_or(0);
                if found.len() > allowed {
                    let lines: Vec<String> = found
                        .iter()
                        .map(|(n, lit)| format!("    {path}:{n} {lit}"))
                        .collect();
                    problems.push(format!(
                        "{path}: {} fixed colours, {allowed} allowed. Use the theme \
                         (a core colour, a role, or theme.swatch(colour) for a content \
                         colour) in place of the new one:\n{}",
                        found.len(),
                        lines.join("\n")
                    ));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{}\nTo write the new counts: MPFREE_LINT_BLESS=1 cargo test -p mpfree-ui --test fixed_colours\n\
         (see docs/dev/design/ui-themes.md, \"Checks\")",
        problems.join("\n")
    );
}

#[test]
fn the_scanner_finds_colours_and_skips_the_rest() {
    let src = r##"
// A comment 0x6EA8FE
/* a block /* nested 0x123456 */ 0x654321 */
const A: Color = Color::rgb(0x6EA8FE);
const B: u32 = 0xFF_4D5E;
fn f(p: u32) -> u32 { p & 0xFF_FFFF }
fn g(c: Color) { let s = "0x3DDC97"; let r = r#"0x3DDC97 "quoted""#; let ch = '{'; }
fn h<'a>(x: &'a str) -> Color { Color::rgb(0) }
const ID: u64 = 0xC1_0000_0000;
const SMALL: u8 = 0x1F;
#[cfg(test)]
mod tests {
    const T: u32 = 0xABCDEF;
    fn inner() { let x = { 0x111111 }; }
}
#[cfg(test)]
const U: u32 = 0x222222;
const LAST: u32 = 0xFFFFFFFF;
"##;
    let found: Vec<(usize, String)> = fixed_colours(src);
    let lits: Vec<&str> = found.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(
        lits,
        ["0x6EA8FE", "0xFF_4D5E", "Color::rgb(0)", "0xFFFFFFFF"],
        "{found:?}"
    );
    assert_eq!(found[0].0, 4);
    assert_eq!(found[2].0, 8);
    assert_eq!(found[3].0, 18);
}
