//! Every control on every sample screen, measured against the layout spec's
//! touch rules (section 1a).
//!
//! `mpfree --touch-targets DIR` writes each sample page's controls: where
//! each answers a finger and what it declares about touch
//! (`mpfree_ui::touch`). This test reads them and finds three kinds of
//! rule break, each by kind of control (`widget/id`, the controls
//! catalogue's key):
//!
//! - **size**: a hit area whose short side is under the minimum of the
//!   class its declaration needs (`Target::needed_class`);
//! - **overlap**: two controls of one widget whose hit areas overlap, so a
//!   touch there belongs to two controls;
//! - **gap**: two neighbours (side by side or one above the other) closer
//!   than the dead gap between them should be: 24 px beside a control that
//!   loses material (Delete, Clear, Erase, Remove) unless it needs a hold,
//!   12 px between two Performance actions or a pair easily confused, 8 px
//!   between other independent actions.
//!
//! Many of today's controls break them: golden/touch.txt lists each break,
//! with its worst measure across every screen and a reason (what fixes it).
//! The test fails on a break not listed and on one worse than listed: a
//! control that shrank, one that needs a bigger class than its line says,
//! two controls that came closer. A line may get better; blessing writes
//! the better measure, and a line goes when its break does:
//!
//!     MPFREE_TOUCH_BLESS=1 cargo test -p mpfree-app --test touch_audit
//!
//! Give any line you add a reason (docs/dev/design/layout-spec.md, 1a).

use mpfree_ui::touch::Class;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What a line of the list says when it was written by blessing.
const NEW_REASON: &str =
    "before the touch audit (25 Nov 2026 baseline); until its screen is redone";

/// Kinds of control the spec lets go under their class, with the spec's
/// words: the smallest short side each may have.
const ALLOWED: &[(&str, i32, &str)] = &[(
    "header/tab",
    60,
    "spec 1a: the tabs answer over the whole 60 px top bar, the one Normal control allowed at 60 px",
)];

/// Pairs of controls (by id, in either order) easily confused, which need
/// 12 px between them wherever they meet (spec 1a, "Hit areas and gaps").
const CONFUSABLE: &[(&str, &str)] = &[
    ("mute", "solo"),
    ("fader", "mute"),
    ("value", "mute"),
    ("scene", "slot"),
];

/// Words that name a control that loses material.
const LOSES: &[&str] = &["delete", "clear", "erase", "remove"];

fn baseline_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden/touch.txt")
}

#[derive(Clone, Debug)]
struct Control {
    kind: String,
    id: String,
    label: String,
    needed: Class,
    consequence: String,
    commit: String,
    rect: (i32, i32, i32, i32),
}

impl Control {
    fn short_side(&self) -> i32 {
        self.rect.2.min(self.rect.3)
    }

    fn loses_material(&self) -> bool {
        if self.consequence == "Destructive" {
            return true;
        }
        let words = format!("{} {}", self.id, self.label).to_lowercase();
        words
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| LOSES.contains(&w))
    }

    fn performance(&self) -> bool {
        self.needed >= Class::Performance
    }

    fn describe(&self) -> String {
        let (x, y, w, h) = self.rect;
        format!("{} {:?} at {x},{y} {w}x{h}", self.kind, self.label)
    }
}

fn class_named(name: &str) -> Class {
    *Class::ALL
        .iter()
        .find(|c| c.name() == name)
        .unwrap_or_else(|| panic!("no class named {name:?}"))
}

/// One page's controls, from a NAME.touch.tsv file.
fn read_page(text: &str) -> Vec<Control> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.starts_with("widget\t") || line.is_empty() {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        assert!(c.len() >= 19, "short line in a touch file: {line:?}");
        let n = |i: usize| -> i32 { c[i].parse().unwrap() };
        out.push(Control {
            kind: format!("{}/{}", c[0], c[1]),
            id: c[1].to_string(),
            label: c[18].to_string(),
            needed: class_named(c[6]),
            consequence: c[8].to_string(),
            commit: c[10].to_string(),
            rect: (n(13), n(14), n(15), n(16)),
        });
    }
    out
}

/// A rule break, by what it is about: `size KIND`, `overlap A B` or
/// `gap A B` (A and B sorted).
type Key = (String, String, String);

/// What a break measures: (measure, needed), where `needed - measure` is how
/// far it falls short (0 for an overlap, which is either there or not).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Measure {
    got: i32,
    need: i32,
}

impl Measure {
    fn short_by(self) -> i32 {
        self.need - self.got
    }
}

/// A break as found: its worst measure, and where (page and controls).
#[derive(Clone, Debug)]
struct Found {
    measure: Measure,
    need_class: Option<Class>,
    page: String,
    what: String,
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// The dead space between two hit areas that don't overlap, when they sit
/// side by side or one above the other (None for diagonal neighbours and
/// for areas that overlap).
fn gap_between(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> Option<i32> {
    let dx = (a.0 - (b.0 + b.2)).max(b.0 - (a.0 + a.2));
    let dy = (a.1 - (b.1 + b.3)).max(b.1 - (a.1 + a.3));
    match (dx >= 0, dy >= 0) {
        (true, false) => Some(dx),
        (false, true) => Some(dy),
        _ => None,
    }
}

fn overlap(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// The dead gap two neighbours need (spec 1a's table), and why.
fn gap_needed(a: &Control, b: &Control) -> i32 {
    let held = |c: &Control| c.commit == "Hold";
    if (a.loses_material() && !held(a)) || (b.loses_material() && !held(b)) {
        return 24;
    }
    let confusable = CONFUSABLE
        .iter()
        .any(|&(x, y)| (a.id == x && b.id == y) || (a.id == y && b.id == x));
    if confusable || (a.performance() && b.performance()) {
        return 12;
    }
    8
}

fn widget(kind: &str) -> &str {
    kind.split('/').next().unwrap_or(kind)
}

/// Keep the worse of two findings for one break.
fn keep(found: &mut BTreeMap<Key, Found>, key: Key, f: Found) {
    match found.get(&key) {
        Some(old) if old.measure.short_by() >= f.measure.short_by() => {}
        _ => {
            found.insert(key, f);
        }
    }
}

/// Every break on every page in `dir`.
fn measure(dir: &Path) -> BTreeMap<Key, Found> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".touch.tsv"))
        .collect();
    names.sort();
    assert!(names.len() > 200, "only {} pages written", names.len());
    let mut found = BTreeMap::new();
    for n in names {
        let page = n.trim_end_matches(".touch.tsv").to_string();
        let controls = read_page(&std::fs::read_to_string(dir.join(&n)).unwrap());
        for c in &controls {
            let min = ALLOWED
                .iter()
                .find(|(k, _, _)| *k == c.kind)
                .map_or(c.needed.min_px(), |&(_, px, _)| px);
            if c.short_side() < min {
                keep(
                    &mut found,
                    ("size".into(), c.kind.clone(), String::new()),
                    Found {
                        measure: Measure {
                            got: c.short_side(),
                            need: min,
                        },
                        need_class: Some(c.needed),
                        page: page.clone(),
                        what: c.describe(),
                    },
                );
            }
        }
        for (i, a) in controls.iter().enumerate() {
            for b in &controls[i + 1..] {
                let (ka, kb) = pair(&a.kind, &b.kind);
                if overlap(a.rect, b.rect) {
                    if widget(&a.kind) == widget(&b.kind) {
                        keep(
                            &mut found,
                            ("overlap".into(), ka, kb),
                            Found {
                                measure: Measure { got: 0, need: 0 },
                                need_class: None,
                                page: page.clone(),
                                what: format!("{} and {}", a.describe(), b.describe()),
                            },
                        );
                    }
                    continue;
                }
                if a.kind == b.kind {
                    continue;
                }
                let Some(gap) = gap_between(a.rect, b.rect) else {
                    continue;
                };
                let need = gap_needed(a, b);
                if gap < need {
                    keep(
                        &mut found,
                        ("gap".into(), ka, kb),
                        Found {
                            measure: Measure { got: gap, need },
                            need_class: None,
                            page: page.clone(),
                            what: format!("{} and {}", a.describe(), b.describe()),
                        },
                    );
                }
            }
        }
    }
    found
}

/// A line of the list: what it measured and its reason.
#[derive(Clone, Debug, PartialEq)]
struct Line {
    measure: Measure,
    class: Option<String>,
    reason: String,
}

type List = BTreeMap<Key, Line>;

fn bad_line(n: usize) -> ! {
    panic!(
        "golden/touch.txt line {n}: expected `size KIND SHORT CLASS`, \
         `overlap A B` or `gap A B GAP NEED`, then # REASON"
    )
}

fn read_list(text: &str) -> List {
    let mut list = List::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (fields, reason) = line.split_once('#').unwrap_or((line, ""));
        let f: Vec<&str> = fields.split_whitespace().collect();
        let num = |s: &str| -> i32 { s.parse().unwrap_or_else(|_| bad_line(n + 1)) };
        let (key, measure, class) = match f[..] {
            ["size", kind, short, class] => {
                let need = ALLOWED
                    .iter()
                    .find(|(k, _, _)| *k == kind)
                    .map_or(class_named(class).min_px(), |&(_, px, _)| px);
                (
                    ("size".to_string(), kind.to_string(), String::new()),
                    Measure {
                        got: num(short),
                        need,
                    },
                    Some(class.to_string()),
                )
            }
            ["overlap", a, b] => (
                ("overlap".to_string(), a.to_string(), b.to_string()),
                Measure { got: 0, need: 0 },
                None,
            ),
            ["gap", a, b, gap, need] => (
                ("gap".to_string(), a.to_string(), b.to_string()),
                Measure {
                    got: num(gap),
                    need: num(need),
                },
                None,
            ),
            _ => bad_line(n + 1),
        };
        list.insert(
            key,
            Line {
                measure,
                class,
                reason: reason.trim().to_string(),
            },
        );
    }
    list
}

fn line_text(key: &Key, line: &Line) -> String {
    let fields = match key.0.as_str() {
        "size" => format!(
            "size {} {} {}",
            key.1,
            line.measure.got,
            line.class.as_deref().unwrap_or("normal")
        ),
        "overlap" => format!("overlap {} {}", key.1, key.2),
        _ => format!(
            "gap {} {} {} {}",
            key.1, key.2, line.measure.got, line.measure.need
        ),
    };
    format!("{fields} # {}\n", line.reason)
}

fn write_list(list: &List) {
    let mut text = String::from(
        "# Today's breaks of the layout spec's touch rules (section 1a), by kind of\n\
         # control (widget/id), each at its worst across the sample screens:\n\
         #     size KIND SHORT_SIDE CLASS_NEEDED # REASON\n\
         #     overlap KIND KIND # REASON          (two controls of one widget)\n\
         #     gap KIND KIND GAP NEEDED # REASON   (dead space between neighbours, px)\n\
         # Checked by tests/touch_audit.rs: a line may only get better. Write it with\n\
         #     MPFREE_TOUCH_BLESS=1 cargo test -p mpfree-app --test touch_audit\n",
    );
    for (key, line) in list {
        text.push_str(&line_text(key, line));
    }
    std::fs::write(baseline_path(), text).unwrap();
}

fn key_text(key: &Key) -> String {
    match key.0.as_str() {
        "size" => format!("size {}", key.1),
        k => format!("{k} {} {}", key.1, key.2),
    }
}

#[test]
fn no_control_on_any_sample_screen_gets_harder_to_touch() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("mpfree-touch-audit");
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_mpfree"))
        .args(["--touch-targets", dir.to_str().unwrap()])
        .output()
        .expect("run mpfree");
    assert!(
        out.status.success(),
        "mpfree --touch-targets failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let got = measure(&dir);
    let old = std::fs::read_to_string(baseline_path())
        .map(|t| read_list(&t))
        .unwrap_or_default();
    if std::env::var_os("MPFREE_TOUCH_BLESS").is_some() {
        let mut list = List::new();
        for (key, f) in &got {
            let reason = old
                .get(key)
                .map(|l| l.reason.clone())
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| NEW_REASON.to_string());
            list.insert(
                key.clone(),
                Line {
                    measure: f.measure,
                    class: f.need_class.map(|c| c.name().to_string()),
                    reason,
                },
            );
        }
        write_list(&list);
        eprintln!(
            "wrote {} lines to {}",
            list.len(),
            baseline_path().display()
        );
        return;
    }
    let mut problems = Vec::new();
    let mut better = 0;
    for (key, f) in &got {
        match old.get(key) {
            None => problems.push(format!(
                "{}: not listed ({}), on {}: {}",
                key_text(key),
                match key.0.as_str() {
                    "overlap" => "their hit areas overlap".to_string(),
                    "gap" => format!("{} px apart, {} needed", f.measure.got, f.measure.need),
                    _ => format!("{} px, {} needed", f.measure.got, f.measure.need),
                },
                f.page,
                f.what
            )),
            Some(l) if f.measure.short_by() > l.measure.short_by() => problems.push(format!(
                "{}: worse than listed ({} where {} needs {}; listed {} of {}), on {}: {}",
                key_text(key),
                f.measure.got,
                f.need_class.map_or("it", |c| c.name()),
                f.measure.need,
                l.measure.got,
                l.measure.need,
                f.page,
                f.what
            )),
            Some(l) if f.measure.short_by() < l.measure.short_by() => better += 1,
            Some(_) => {}
        }
    }
    let gone = old.keys().filter(|k| !got.contains_key(*k)).count();
    if better + gone > 0 {
        eprintln!(
            "touch audit: {better} lines of golden/touch.txt got better and {gone} breaks went; \
             bless to write it (MPFREE_TOUCH_BLESS=1)"
        );
    }
    assert!(
        problems.is_empty(),
        "{} controls got harder to touch than golden/touch.txt allows (touch files in {}):\n{}\n\
         Make the control meet the class its declaration needs (layout spec 1a: a hit area as \
         big as its class, and dead gaps by what a wrong hit costs), or declare what the spec \
         allows (Dense for a row of one kind, with an alternative; see mpfree_ui::touch). If the \
         break is meant, write the list and give its line a reason:\n\
         MPFREE_TOUCH_BLESS=1 cargo test -p mpfree-app --test touch_audit",
        problems.len(),
        dir.display(),
        problems.join("\n")
    );
}

#[test]
fn the_list_reads_back_what_it_wrote() {
    let text = "# comment\n\n\
                size mixer/mute 36 performance # the Perform mixer makes it 85\n\
                overlap browser/row browser/star # the star sits on its row\n\
                gap leditor/clear leditor/send 6 24 #\n\
                size header/tab 44 normal # tabs\n";
    let list = read_list(text);
    let mute = &list[&("size".into(), "mixer/mute".into(), String::new())];
    assert_eq!(mute.measure, Measure { got: 36, need: 76 });
    assert_eq!(mute.reason, "the Perform mixer makes it 85");
    let tab = &list[&("size".into(), "header/tab".into(), String::new())];
    assert_eq!(tab.measure.need, 60, "the tabs' allowance");
    let gap = &list[&("gap".into(), "leditor/clear".into(), "leditor/send".into())];
    assert_eq!(gap.measure, Measure { got: 6, need: 24 });
    assert_eq!(gap.reason, "");
    for (k, l) in &list {
        assert_eq!(read_list(&line_text(k, l)).get(k), Some(l), "{k:?}");
    }
}

#[test]
fn neighbours_need_the_gap_their_cost_calls_for() {
    let c = |id: &str, label: &str, needed: Class, commit: &str| Control {
        kind: format!("w/{id}"),
        id: id.into(),
        label: label.into(),
        needed,
        consequence: "Minor".into(),
        commit: commit.into(),
        rect: (0, 0, 64, 64),
    };
    let n = Class::Normal;
    let p = Class::Performance;
    assert_eq!(
        gap_needed(
            &c("a", "Copy", n, "LiftInside"),
            &c("b", "Paste", n, "LiftInside")
        ),
        8
    );
    assert_eq!(
        gap_needed(
            &c("a", "Copy", n, "LiftInside"),
            &c("clear", "Clear", n, "LiftInside")
        ),
        24
    );
    assert_eq!(
        gap_needed(
            &c("a", "Copy", n, "LiftInside"),
            &c("b", "Delete take", n, "Hold")
        ),
        8,
        "a hold guards it"
    );
    assert_eq!(
        gap_needed(&c("mute", "M", p, "Press"), &c("solo", "S", p, "Press")),
        12
    );
    assert_eq!(
        gap_needed(
            &c("value", "0.0", n, "LiftInside"),
            &c("mute", "M", p, "Press")
        ),
        12
    );
    assert_eq!(
        gap_needed(&c("a", "x", p, "Press"), &c("b", "y", p, "Press")),
        12
    );
    assert_eq!(
        gap_between((0, 0, 10, 10), (14, 2, 10, 10)),
        Some(4),
        "side by side"
    );
    assert_eq!(
        gap_between((0, 0, 10, 10), (0, 10, 10, 10)),
        Some(0),
        "touching"
    );
    assert_eq!(
        gap_between((0, 0, 10, 10), (20, 20, 10, 10)),
        None,
        "diagonal"
    );
    assert!(overlap((0, 0, 10, 10), (9, 9, 10, 10)));
    assert!(!overlap((0, 0, 10, 10), (10, 0, 10, 10)));
}
