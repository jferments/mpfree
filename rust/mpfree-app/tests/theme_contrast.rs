//! Every sample screen in every built-in theme, checked for unreadable text.
//!
//! `mpfree --display-lists DIR --theme all` draws each sample page in each
//! built-in theme; `mpfree_ui::lint::text_runs` measures every run of text
//! against what it was drawn on. A run under 3:1 is unreadable.
//!
//! Some screens have such runs today: golden/theme-contrast.txt lists how
//! many each theme's page may have, each line with its reason. A page with
//! more fails, showing its runs; a page with fewer passes, and the theme
//! work lowers the list as it goes, so it only ever shrinks. Write the
//! counts with
//!
//!     MPFREE_LINT_BLESS=1 cargo test -p mpfree-app --test theme_contrast
//!
//! and give any line you add a reason (docs/dev/design/ui-themes.md).

use mpfree_ui::lint::{text_runs, Run, FLOOR};
use mpfree_ui::record::Frame;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What a line of the list says when it was written by blessing.
const NEW_REASON: &str = "before the theme refactor (phase 0 baseline)";

fn baseline_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden/theme-contrast.txt")
}

/// (theme, page) to (count, reason).
type List = BTreeMap<(String, String), (usize, String)>;

fn read_list(text: &str) -> List {
    let mut list = List::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (fields, reason) = line.split_once('#').unwrap_or((line, ""));
        let f: Vec<&str> = fields.split_whitespace().collect();
        let [theme, page, count] = f[..] else {
            panic!(
                "golden/theme-contrast.txt line {}: expected THEME PAGE COUNT # REASON",
                n + 1
            );
        };
        let count = count.parse().unwrap_or_else(|_| {
            panic!(
                "golden/theme-contrast.txt line {}: {count:?} is not a count",
                n + 1
            )
        });
        list.insert(
            (theme.to_string(), page.to_string()),
            (count, reason.trim().to_string()),
        );
    }
    list
}

fn write_list(list: &List) {
    let mut text = String::from(
        "# Runs of text under 3:1 contrast that a sample screen may have, per built-in\n\
         # theme: THEME PAGE COUNT # REASON. Checked by tests/theme_contrast.rs; the\n\
         # counts may only go down. Write them with\n\
         #     MPFREE_LINT_BLESS=1 cargo test -p mpfree-app --test theme_contrast\n",
    );
    for ((theme, page), (count, reason)) in list {
        text.push_str(&format!("{theme} {page} {count} # {reason}\n"));
    }
    std::fs::write(baseline_path(), text).unwrap();
}

/// Each theme's pages' unreadable runs, from the display lists in `dir`.
fn measure(dir: &Path) -> BTreeMap<(String, String), Vec<Run>> {
    let mut out = BTreeMap::new();
    for theme in mpfree_ui::theme::BUILTIN {
        let mut names: Vec<String> = std::fs::read_dir(dir.join(theme))
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".mpdl"))
            .collect();
        names.sort();
        assert!(
            names.len() > 100,
            "{theme}: only {} pages drawn",
            names.len()
        );
        for n in names {
            let bytes = std::fs::read(dir.join(theme).join(&n)).unwrap();
            let frames = Frame::decode_all(&bytes).unwrap();
            let bad: Vec<Run> = text_runs(&frames[0])
                .into_iter()
                .filter(|r| r.ratio < FLOOR)
                .collect();
            out.insert(
                (theme.to_string(), n.trim_end_matches(".mpdl").to_string()),
                bad,
            );
        }
    }
    out
}

fn describe(r: &Run) -> String {
    format!(
        "#{:06X} on #{:06X} at {},{} ({} glyphs, {:.2}:1)",
        r.ink.xrgb(),
        r.under.xrgb(),
        r.x,
        r.y,
        r.glyphs,
        r.ratio
    )
}

#[test]
fn no_screen_in_any_built_in_theme_gets_harder_to_read() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("mpfree-theme-contrast");
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_mpfree"))
        .args(["--display-lists", dir.to_str().unwrap(), "--theme", "all"])
        .output()
        .expect("run mpfree");
    assert!(
        out.status.success(),
        "mpfree --display-lists --theme all failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let got = measure(&dir);
    let old = std::fs::read_to_string(baseline_path())
        .map(|t| read_list(&t))
        .unwrap_or_default();
    if std::env::var_os("MPFREE_LINT_BLESS").is_some() {
        let mut list = List::new();
        for (k, runs) in &got {
            if !runs.is_empty() {
                let reason = old
                    .get(k)
                    .map(|(_, r)| r.clone())
                    .filter(|r| !r.is_empty())
                    .unwrap_or_else(|| NEW_REASON.to_string());
                list.insert(k.clone(), (runs.len(), reason));
            }
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
    for ((theme, page), runs) in &got {
        let allowed = old
            .get(&(theme.clone(), page.clone()))
            .map_or(0, |(n, _)| *n);
        if runs.len() > allowed {
            let mut s = format!(
                "{theme} {page}: {} runs of text under {FLOOR}:1, {allowed} allowed:",
                runs.len()
            );
            for r in runs.iter().take(8) {
                s.push_str(&format!("\n    {}", describe(r)));
            }
            problems.push(s);
        }
    }
    assert!(
        problems.is_empty(),
        "{} pages have text that got harder to read than golden/theme-contrast.txt allows \
         (display lists in {}):\n{}\n\
         Draw the text in a theme colour that reads on its fill (theme.ink_on(fill), or \
         theme.swatch(colour).text for text in a content colour). If the run is meant, write \
         the list and give its line a reason:\n\
         MPFREE_LINT_BLESS=1 cargo test -p mpfree-app --test theme_contrast",
        problems.len(),
        dir.display(),
        problems.join("\n")
    );
}

#[test]
fn the_list_reads_back_what_it_wrote() {
    let text = "# comment\n\nconsole looper-tab 3 # a reason, with words\ndefault x 1 #\n";
    let list = read_list(text);
    assert_eq!(
        list.get(&("console".into(), "looper-tab".into())),
        Some(&(3, "a reason, with words".into()))
    );
    assert_eq!(
        list.get(&("default".into(), "x".into())),
        Some(&(1, String::new()))
    );
}
