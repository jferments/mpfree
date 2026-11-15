//! What the MPC shows for a plugin: parameter names and the page layout.
//!
//! The MPC draws a plugin's screen from JSON skins in its folder
//! (`Plugin Skins/TUI.json`, `Q-Links.json`). The inMusic SDK's
//! `tools/gen_ui.py` makes those from a short `ui.json`: pages of up to 16
//! controls, in a 4 x 4 grid, with the hardware knobs in blocks of four.
//! [`ui_json`] writes that `ui.json` from the node's own parameter groups,
//! the same groups MPFree's effect and synth pages use, so the MPC's pages
//! follow ours: one page per group, named after it.

use mpfree_node::{Curve, Node, ParamInfo};

/// Controls on one MPC page (its 4 x 4 grid).
pub const PAGE_CONTROLS: usize = 16;
/// Pages the MPC allows.
pub const MAX_PAGES: usize = 32;
/// Characters in a page's function-key label.
pub const PAGE_NAME_CHARS: usize = 8;

/// The node's parameters a plugin shows: every one but the meters.
pub fn host_params(node: &dyn Node) -> Vec<&'static ParamInfo> {
    node.params().iter().filter(|p| !p.readonly).collect()
}

/// Short form of a group for a name prefix: the first letter of each word
/// and every digit ("Band 1" gives "B1", "Slot 10" gives "S10").
fn group_tag(group: &str) -> String {
    let mut tag = String::new();
    for word in group.split_whitespace() {
        if word.chars().all(|c| c.is_ascii_digit()) {
            tag.push_str(word);
        } else if let Some(c) = word.chars().next() {
            tag.push(c.to_ascii_uppercase());
        }
    }
    tag
}

/// The names the host shows. Unique within the plugin, because the MPC's
/// skins find a parameter by its name: a name that several groups share
/// ("Freq" in each EQ band) gets its group's tag in front ("B1 Freq").
pub fn host_names(params: &[&ParamInfo]) -> Vec<String> {
    let mut names: Vec<String> = params
        .iter()
        .map(|p| {
            let shared = params.iter().filter(|q| q.name == p.name).count() > 1;
            if shared && !p.group.is_empty() {
                format!("{} {}", group_tag(p.group), p.name)
            } else {
                p.name.to_string()
            }
        })
        .collect();
    // Anything still doubled (same name and same group tag) gets a number.
    for i in 0..names.len() {
        let mut k = 2;
        while names[..i].contains(&names[i]) {
            names[i] = format!("{} {k}", names[i]);
            k += 1;
        }
    }
    names
}

fn page_name(group: &str, part: usize) -> String {
    let base: String = if group.is_empty() { "MAIN" } else { group }
        .to_uppercase()
        .chars()
        .take(PAGE_NAME_CHARS)
        .collect();
    if part == 0 {
        base
    } else {
        let suffix = format!(" {}", part + 1);
        let keep = PAGE_NAME_CHARS.saturating_sub(suffix.len());
        format!(
            "{}{suffix}",
            base.chars().take(keep).collect::<String>().trim_end()
        )
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One control in `ui.json` for a parameter.
fn control(p: &ParamInfo, name: &str) -> String {
    let n = json_str(name);
    match p.curve {
        Curve::Toggle => format!("{{ \"param\": {n}, \"widget\": \"switch\" }}"),
        Curve::Choice(_) | Curve::Stepped => {
            format!("{{ \"param\": {n}, \"widget\": \"slider\", \"overlay\": \"menu\" }}")
        }
        _ if p.min < 0.0 && p.max > 0.0 => {
            format!("{{ \"param\": {n}, \"widget\": \"knob\", \"polarity\": \"bipolar\" }}")
        }
        _ => format!("{{ \"param\": {n}, \"widget\": \"knob\" }}"),
    }
}

/// The SDK's `ui.json` for a node: one page per parameter group, in the
/// order the groups first appear, split into pages of 16. Parameters past
/// the MPC's 32 pages are left off the screen (still automatable and saved).
pub fn ui_json(node: &dyn Node) -> String {
    let params = host_params(node);
    let names = host_names(&params);
    let mut groups: Vec<&str> = Vec::new();
    for p in &params {
        if !groups.contains(&p.group) {
            groups.push(p.group);
        }
    }
    let mut pages = Vec::new();
    'groups: for g in groups {
        let members: Vec<usize> = (0..params.len())
            .filter(|&i| params[i].group == g)
            .collect();
        for (part, chunk) in members.chunks(PAGE_CONTROLS).enumerate() {
            if pages.len() == MAX_PAGES {
                break 'groups;
            }
            let controls: Vec<String> = chunk
                .iter()
                .map(|&i| format!("        {}", control(params[i], &names[i])))
                .collect();
            pages.push(format!(
                "    {{ \"name\": {},\n      \"controls\": [\n{}\n      ] }}",
                json_str(&page_name(g, part)),
                controls.join(",\n")
            ));
        }
    }
    format!(
        "{{\n  \"kit\": \"mpc-red\",\n  \"pages\": [\n{}\n  ]\n}}\n",
        pages.join(",\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_page_names() {
        assert_eq!(group_tag("Band 1"), "B1");
        assert_eq!(group_tag("Slot 10"), "S10");
        assert_eq!(page_name("Band 1", 0), "BAND 1");
        assert_eq!(page_name("Character", 0), "CHARACTE");
        assert_eq!(page_name("Character", 1), "CHARAC 2");
        assert_eq!(page_name("", 0), "MAIN");
        assert_eq!(json_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }
}
