//! `Q-Links.json`: which parameter each of the 16 Q-Link knobs turns on each
//! tab, and how a unit with fewer knobs banks through them (section 8 of the
//! SDK's UI reference). The MPC One has four knobs in a column (`1by4`).

use crate::json::Json;
use crate::model::read_json;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BankDirection {
    /// Columns of the 4 x 4 grid, left to right, each read top to bottom.
    Column,
    ColumnBottomUp,
    /// Rows, bottom row first.
    Row,
    RowTopDown,
}

impl BankDirection {
    /// The four Q-Link numbers (1-based, the 4 x 4 grid numbered from the
    /// bottom left) in bank `b` (0 to 3), in the order a column of four
    /// knobs takes them, top first.
    pub fn bank(self, b: usize) -> [usize; 4] {
        let b = b.min(3);
        match self {
            BankDirection::Column => [13 + b, 9 + b, 5 + b, 1 + b],
            BankDirection::ColumnBottomUp => [1 + b, 5 + b, 9 + b, 13 + b],
            BankDirection::Row => [1 + 4 * b, 2 + 4 * b, 3 + 4 * b, 4 + 4 * b],
            BankDirection::RowTopDown => [13 - 4 * b, 14 - 4 * b, 15 - 4 * b, 16 - 4 * b],
        }
    }
}

/// One tab's knobs.
#[derive(Clone, Debug, PartialEq)]
pub struct TabKnobs {
    /// 1-based, as in the file.
    pub tab: usize,
    pub sub: usize,
    pub direction: BankDirection,
    /// Q-Link 1 to 16's parameter (index 0 is Q-Link 1).
    pub params: [Option<usize>; 16],
}

impl TabKnobs {
    /// The bank (0 to 3) holding `param`, if any.
    pub fn bank_of(&self, param: usize) -> Option<usize> {
        (0..4).find(|&b| {
            self.direction
                .bank(b)
                .iter()
                .any(|&q| self.params[q - 1] == Some(param))
        })
    }

    /// The parameters of bank `b`, top knob first.
    pub fn bank_params(&self, b: usize) -> [Option<usize>; 4] {
        self.direction.bank(b).map(|q| self.params[q - 1])
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct QLinks {
    pub tabs: Vec<TabKnobs>,
    pub program: [Option<usize>; 16],
    /// A `PartialDescription` file's offset.
    pub partial: Option<usize>,
}

fn sixteen(j: &Json) -> [Option<usize>; 16] {
    let mut out = [None; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = j
            .at(&format!("Q-Link {}", i + 1))
            .as_i64()
            .filter(|&p| p >= 0)
            .map(|p| p as usize);
    }
    out
}

impl QLinks {
    pub fn from_json(j: &Json) -> QLinks {
        let mut q = QLinks {
            program: sixteen(j.at("Program Mode Q-Links")),
            ..QLinks::default()
        };
        if j.at("info").at("type").str_or_empty() == "PartialDescription" {
            q.partial = j
                .at("info")
                .at("offset")
                .as_i64()
                .filter(|&o| o > 0)
                .map(|o| o as usize);
        }
        for e in j.at("Screen Mode Q-Links").at("map").items() {
            q.tabs.push(TabKnobs {
                tab: e.at("Tab").as_i64().unwrap_or(1).max(1) as usize,
                sub: e.at("SubTab").as_i64().unwrap_or(1).max(1) as usize,
                direction: match e.at("Bank Direction").str_or_empty() {
                    "ColumnBottomUp" => BankDirection::ColumnBottomUp,
                    "Row" => BankDirection::Row,
                    "RowTopDown" => BankDirection::RowTopDown,
                    _ => BankDirection::Column,
                },
                params: sixteen(e.at("Q-Links")),
            });
        }
        q
    }

    pub fn read(path: &Path) -> Result<QLinks, String> {
        Ok(QLinks::from_json(&read_json(path)?))
    }

    /// The MPC's fallback without a file (or with a broken one): parameter
    /// `i` on page `i / 16` at Q-Link `i % 16 + 1`, banked by column.
    pub fn sequential(params: usize) -> QLinks {
        let mut q = QLinks::default();
        for page in 0..params.div_ceil(16).min(32) {
            let mut p = [None; 16];
            for (k, slot) in p.iter_mut().enumerate() {
                let i = page * 16 + k;
                if i < params {
                    *slot = Some(i);
                }
            }
            q.tabs.push(TabKnobs {
                tab: page + 1,
                sub: 1,
                direction: BankDirection::Column,
                params: p,
            });
        }
        for (k, slot) in q.program.iter_mut().enumerate() {
            *slot = (k < params).then_some(k);
        }
        q
    }

    /// The knobs for tab `key` (0-based function key), sub-tab `sub`
    /// (0-based). A partial file's one entry repeats with shifted indices.
    pub fn for_tab(&self, key: usize, sub: usize) -> Option<TabKnobs> {
        if let Some(off) = self.partial {
            let first = self.tabs.first()?;
            let mut t = first.clone();
            for p in t.params.iter_mut().flatten() {
                *p += key * off;
            }
            t.tab = key + 1;
            return Some(t);
        }
        self.tabs
            .iter()
            .find(|t| t.tab == key + 1 && t.sub == sub + 1)
            .cloned()
    }
}

/// The skin files to try for a unit, most specific first: product name,
/// family, Q-Link layout, then the plain file. For the MPC One:
/// `TUI - MPC One.json`, `TUI - MPC.json`, `TUI - 1by4.json`, `TUI.json`.
pub fn variants(base: &str, product: &str, family: &str, layout: &str) -> Vec<String> {
    let mut v = Vec::new();
    for s in [product, family, layout] {
        if !s.is_empty() {
            v.push(format!("{base} - {s}.json"));
        }
    }
    v.push(format!("{base}.json"));
    v
}

/// The first of `variants` that exists in `dir`.
pub fn pick(dir: &Path, names: &[String]) -> Option<std::path::PathBuf> {
    names.iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    #[test]
    fn banks_follow_the_direction() {
        assert_eq!(BankDirection::Column.bank(0), [13, 9, 5, 1]);
        assert_eq!(BankDirection::Column.bank(3), [16, 12, 8, 4]);
        assert_eq!(BankDirection::Row.bank(1), [5, 6, 7, 8]);
        assert_eq!(BankDirection::RowTopDown.bank(0), [13, 14, 15, 16]);
        assert_eq!(BankDirection::ColumnBottomUp.bank(2), [3, 7, 11, 15]);
    }

    #[test]
    fn reads_a_file_and_finds_banks() {
        let j = json::parse(
            r#"{"version": 4, "info": {"version": 1, "type": "CompleteDescription"},
            "Screen Mode Q-Links": {"version": 4, "map": [
              {"Tab": 1, "SubTab": 2, "Bank Direction": "Column",
               "Q-Links": {"Q-Link 1": 3, "Q-Link 2": 7, "Q-Link 3": -1, "Q-Link 4": 15, "Q-Link 5": 2, "Q-Link 6": 6,
                 "Q-Link 7": 10, "Q-Link 8": 14, "Q-Link 9": 1, "Q-Link 10": 5, "Q-Link 11": 9, "Q-Link 12": 13,
                 "Q-Link 13": 0, "Q-Link 14": 4, "Q-Link 15": 8, "Q-Link 16": 12}}]},
            "Program Mode Q-Links": {"Q-Link 1": 0, "Q-Link 2": 1}}"#,
        )
        .unwrap();
        let q = QLinks::from_json(&j);
        assert_eq!(q.program[0], Some(0));
        assert_eq!(q.program[2], None);
        assert!(q.for_tab(0, 0).is_none());
        let t = q.for_tab(0, 1).unwrap();
        assert_eq!(t.bank_params(0), [Some(0), Some(1), Some(2), Some(3)]);
        assert_eq!(t.bank_params(2), [Some(8), Some(9), Some(10), None]);
        assert_eq!(t.bank_of(13), Some(3));
        assert_eq!(t.bank_of(99), None);
    }

    #[test]
    fn the_sequential_fallback_and_variants() {
        let q = QLinks::sequential(20);
        assert_eq!(q.tabs.len(), 2);
        assert_eq!(q.tabs[1].params[3], Some(19));
        assert_eq!(q.tabs[1].params[4], None);
        assert_eq!(
            variants("TUI", "MPC One", "MPC", "1by4"),
            [
                "TUI - MPC One.json",
                "TUI - MPC.json",
                "TUI - 1by4.json",
                "TUI.json"
            ]
        );
    }
}
