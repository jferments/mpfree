//! The Open MPC catalog (`https://open-mpc.github.io/catalog.json`): what
//! each plugin is, its newest release for 32-bit ARM, and whether MPFree on
//! an MPC One can run it.

use mpfree_mpcskin::json::{self, Json};

/// Where the catalog lives. The old address serves the same file.
pub const URLS: [&str; 2] = [
    "https://open-mpc.github.io/catalog.json",
    "https://sd88me.github.io/mpc-vst-plugins/catalog.json",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Catalog {
    /// When the catalog was made (ISO 8601, UTC).
    pub generated: String,
    pub plugins: Vec<Plugin>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub author: String,
    /// `instrument`, `effect` or `addin`.
    pub kind: String,
    pub license: String,
    pub summary: String,
    /// A picture of the plugin's screen (a PNG on GitHub), or empty.
    pub screenshot: String,
    pub style: String,
    pub tags: Vec<String>,
    pub repo: String,
    pub downloads: u64,
    pub updated: String,
    /// The release MPFree would install: the catalog's latest stable
    /// version when it has a 32-bit ARM zip, else the newest stable one
    /// that has.
    pub release: Option<Release>,
    pub fit: Fit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub date: String,
    /// The 32-bit ARM zip.
    pub url: String,
    pub sha256: String,
    pub size: u64,
    /// What the plugin needs that MPFree doesn't give it (a ROM, a network
    /// connection), in the maker's words; empty when nothing.
    pub requires: String,
    /// Other catalog plugins it needs, by id.
    pub depends: Vec<String>,
    /// The catalog's CPU bench verdict (`PASS`, `WARN`, `FAIL`) or empty.
    pub cpu: String,
}

/// Whether MPFree on an MPC One can run a plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fit {
    Yes,
    /// An add-in for Akai's own app, not a plugin.
    AddIn,
    /// Built only for 64-bit (Gen2) MPCs.
    Gen2Only,
    /// No release with a zip to install (a build-yourself entry).
    NoRelease,
}

impl Fit {
    /// Why not, in a few words for the screen; empty when it fits.
    pub fn why_not(&self) -> &'static str {
        match self {
            Fit::Yes => "",
            Fit::AddIn => "an add-in for Akai's app, not a plugin",
            Fit::Gen2Only => "for 64-bit MPCs only",
            Fit::NoRelease => "no release to install (build it yourself)",
        }
    }
}

fn strings(j: &Json) -> Vec<String> {
    j.items()
        .iter()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect()
}

fn release(v: &Json) -> Option<Release> {
    if v.at("yanked").as_bool() == Some(true) || v.at("gen2_only").as_bool() == Some(true) {
        return None;
    }
    let a = v.at("assets").at("armv7");
    let url = a.at("url").str_or_empty();
    let sha = a.at("sha256").str_or_empty().to_ascii_lowercase();
    if !url.starts_with("https://")
        || sha.len() != 64
        || !sha.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(Release {
        version: v.at("version").str_or_empty().to_string(),
        date: v.at("date").str_or_empty().to_string(),
        url: url.to_string(),
        sha256: sha,
        size: a.at("size").as_f64().unwrap_or(0.0).max(0.0) as u64,
        requires: v.at("requires").str_or_empty().trim().to_string(),
        depends: v
            .at("depends")
            .items()
            .iter()
            .filter_map(|d| d.at("id").as_str().map(str::to_string))
            .collect(),
        cpu: v.at("cpu").at("verdict").str_or_empty().to_string(),
    })
}

fn plugin(p: &Json) -> Option<Plugin> {
    let id = p.at("id").as_str()?.to_string();
    let kind = p.at("kind").str_or_empty().to_string();
    let latest = p.at("latest").str_or_empty();
    let stable = |v: &&Json| v.at("channel").str_or_empty() != "beta";
    let versions = p.at("versions").items();
    // The catalog lists versions newest first.
    let release = versions
        .iter()
        .filter(stable)
        .find(|v| v.at("version").str_or_empty() == latest)
        .and_then(release)
        .or_else(|| versions.iter().filter(stable).find_map(release));
    let any_arm32 = versions.iter().any(|v| {
        v.at("assets").get("armv7").is_some() && v.at("gen2_only").as_bool() != Some(true)
    });
    let fit = if kind == "addin" {
        Fit::AddIn
    } else if release.is_some() {
        Fit::Yes
    } else if !versions.is_empty() && !any_arm32 {
        Fit::Gen2Only
    } else {
        Fit::NoRelease
    };
    Some(Plugin {
        name: p.at("name").as_str().unwrap_or(&id).to_string(),
        id,
        author: p.at("author").str_or_empty().to_string(),
        kind,
        license: p.at("license").str_or_empty().to_string(),
        summary: p.at("summary").str_or_empty().to_string(),
        screenshot: p
            .at("screenshot")
            .as_str()
            .filter(|s| s.starts_with("https://"))
            .unwrap_or("")
            .to_string(),
        style: p.at("style").str_or_empty().to_string(),
        tags: strings(p.at("tags")),
        repo: p.at("repo").str_or_empty().to_string(),
        downloads: p.at("downloads").as_f64().unwrap_or(0.0).max(0.0) as u64,
        updated: p.at("updated").str_or_empty().to_string(),
        release: if fit == Fit::Yes { release } else { None },
        fit,
    })
}

/// Reads catalog.json. Entries without an id are left out.
pub fn parse(text: &str) -> Result<Catalog, String> {
    let j = json::parse(text).map_err(|e| format!("the catalog doesn't read: {e}"))?;
    if j.at("plugins").items().is_empty() {
        return Err("the catalog lists no plugins".into());
    }
    Ok(Catalog {
        generated: j.at("generated").str_or_empty().to_string(),
        plugins: j.at("plugins").items().iter().filter_map(plugin).collect(),
    })
}

impl Catalog {
    /// The plugin and release whose zip has this SHA-256.
    pub fn by_sha256(&self, sum: &str) -> Option<(&Plugin, &Release)> {
        self.plugins.iter().find_map(|p| {
            p.release
                .as_ref()
                .filter(|r| r.sha256.eq_ignore_ascii_case(sum))
                .map(|r| (p, r))
        })
    }

    pub fn get(&self, id: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.id == id)
    }

    /// The plugins MPFree can run on an MPC One.
    pub fn runnable(&self) -> impl Iterator<Item = &Plugin> {
        self.plugins.iter().filter(|p| p.fit == Fit::Yes)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = r#"{
      "schema": 1, "generated": "2026-11-15T12:34:21Z",
      "plugins": [
        {"id": "density2", "name": "Density2", "author": "Airwindows", "kind": "effect",
         "license": "MIT", "summary": "Saturation.", "downloads": 12,
         "screenshot": "https://raw.githubusercontent.com/x/y/z.png", "tags": ["drive"],
         "latest": "1.0.0", "versions": [
           {"version": "1.1.0-beta", "channel": "beta", "assets": {"armv7": {"url": "https://e/b.zip", "sha256": "bb", "size": 1}}},
           {"version": "1.0.0", "channel": "stable", "date": "2026-10-01", "requires": "",
            "cpu": {"verdict": "PASS"}, "depends": [{"id": "helper", "rules": []}],
            "assets": {"armv7": {"url": "https://e/d.zip", "sha256": "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789", "size": 92100},
                       "aarch64": {"url": "https://e/d64.zip", "sha256": "00", "size": 2}}}
         ]},
        {"id": "big", "name": "Big", "kind": "instrument", "latest": "2.0",
         "versions": [{"version": "2.0", "channel": "stable", "gen2_only": true,
           "assets": {"aarch64": {"url": "https://e/x.zip", "sha256": "11", "size": 3}}}]},
        {"id": "link", "name": "Link", "kind": "addin", "latest": "1.0",
         "versions": [{"version": "1.0", "channel": "stable",
           "assets": {"armv7": {"url": "https://e/l.zip", "sha256": "1111111111111111111111111111111111111111111111111111111111111111", "size": 3}}}]},
        {"id": "old", "name": "Old", "kind": "effect", "latest": "2.0",
         "versions": [
           {"version": "2.0", "channel": "stable", "yanked": true,
            "assets": {"armv7": {"url": "https://e/o2.zip", "sha256": "2222222222222222222222222222222222222222222222222222222222222222", "size": 3}}},
           {"version": "1.0", "channel": "stable",
            "assets": {"armv7": {"url": "https://e/o1.zip", "sha256": "3333333333333333333333333333333333333333333333333333333333333333", "size": 3}}}]},
        {"name": "no id"}
      ]}"#;

    #[test]
    fn reads_plugins_and_picks_the_arm32_release() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(c.generated, "2026-11-15T12:34:21Z");
        assert_eq!(c.plugins.len(), 4, "the entry with no id is left out");
        let d = c.get("density2").unwrap();
        assert_eq!(d.fit, Fit::Yes);
        let r = d.release.as_ref().unwrap();
        assert_eq!(r.version, "1.0.0", "never the beta");
        assert_eq!(r.url, "https://e/d.zip");
        assert_eq!(r.size, 92100);
        assert_eq!(r.depends, ["helper"]);
        assert_eq!(r.cpu, "PASS");
        assert_eq!(c.get("big").unwrap().fit, Fit::Gen2Only);
        assert_eq!(c.get("link").unwrap().fit, Fit::AddIn);
        assert!(c.get("link").unwrap().release.is_none());
        let o = c.get("old").unwrap();
        assert_eq!(
            o.release.as_ref().unwrap().version,
            "1.0",
            "a yanked release is skipped"
        );
        assert_eq!(c.runnable().count(), 2);
        let sum = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        assert_eq!(c.by_sha256(sum).unwrap().0.id, "density2");
        assert!(parse("{}").is_err());
    }
}
