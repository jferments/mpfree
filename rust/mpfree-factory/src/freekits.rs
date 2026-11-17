//! Free drum kits made from public-domain (CC0) sample libraries.
//!
//! [`MANIFEST`] (`free-kits.txt`, next to this crate's `Cargo.toml`) lists
//! each kit: its library, the library's maker and licence, the commit it is
//! read from, and for each pad the sample files mixed into its one hit, each
//! with its SHA-256. The `mpfree-free-kits` tool fetches those files at that
//! commit, checks every hash and the licence text, and renders the kits into
//! a kits file ([`render`]); the release workflow puts that file next to
//! MPFree's programs as [`STICK_FILE`], and the stick builder adds the kits
//! after MPFree's own ([`stick_kits`]). Nothing here downloads: the caller
//! hands in each file's bytes.
//!
//! Each pad's hit is its layers (one per microphone, usually) decoded (WAV
//! or FLAC), at 44.1 kHz, each with its gain and pan, added together; then
//! trimmed of silence at both ends (below 60 dB under the hit's peak, and
//! at most 5 ms before the hit reaches 40 dB under it) and
//! cut to the kit's longest hit with a fade. Each hit is then scaled to peak
//! at about -1 dB, as a sample pack's one-shots are: the libraries record
//! each drum at its own level (VCSL's instruments in sessions of their own),
//! so the musician sets the balance with each pad's level.
//!
//! The manifest's lines (`#` starts a comment; spaces at the start are
//! ignored):
//!
//! ```text
//! kit NAME                  a kit; the lines after it describe it
//! maker NAME                who made the library
//! terms CC0 1.0             the only terms accepted
//! source OWNER/REPO COMMIT  the GitHub repository and the full commit
//! licence SHA256 PATH       the licence file in it, checked for CC0's text
//! seconds N                 the longest hit, 0.1 to 10 (default 6)
//! pad N NAME                pad N (1 to 16) and the sound's name
//! file SHA256 GAIN PAN PATH a layer of the last pad: gain in dB, pan -1 to 1
//! ```
//!
//! Write `-` for a file's hash to have `mpfree-free-kits --fill` download it
//! and write its hash in (for choosing new pads; review the file).

use crate::kitfile::{self, Sound, MAX_FILE, MAX_FRAMES, MAX_KITS, MAX_NAME, PAD_NOTES, RATE};
use sha2::{Digest, Sha256};

/// The kits MPFree ships, as `free-kits.txt` says.
pub const MANIFEST: &str = include_str!("../free-kits.txt");
/// The kits file's name in the folder of programs for the stick.
pub const STICK_FILE: &str = "free-kits";
/// The only terms a kit can have.
pub const TERMS: &str = "CC0 1.0";
/// What the licence file must say, for [`TERMS`].
pub const LICENCE_TEXT: &str = "CC0 1.0 Universal";
/// Where files are fetched from: `RAW/OWNER/REPO/COMMIT/PATH`.
pub const RAW: &str = "https://raw.githubusercontent.com";
/// Default longest hit, in seconds.
const SECONDS: f32 = 6.0;
/// Each hit peaks here (about -1 dB, as MPFree's own kits at most).
const PEAK: f32 = 0.89;
/// Silence at either end of a hit: this many dB under its peak.
const QUIET_DB: f32 = -60.0;
/// Where a hit starts: the first sound this many dB under its peak...
const ONSET_DB: f32 = -40.0;
/// ...with at most this much before it (5 ms), faded in, so a quiet
/// recording's raised noise doesn't delay the hit.
const PRE_ROLL: usize = 220;
/// A file is read for at most this long.
const READ_SECONDS: u32 = 60;

/// Gives the SHA-256 of a kit's file, by its path ([`fill`]).
pub type HashOf<'a> = dyn FnMut(&Kit, &str) -> Result<[u8; 32], String> + 'a;
/// Gives a kit's file's bytes ([`render`]).
pub type ReadFile<'a> = dyn FnMut(&Kit, &File) -> Result<Vec<u8>, String> + 'a;

/// One file in a library: its path at the pinned commit and its SHA-256
/// (None for `-`, waiting for `--fill`).
#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub sha256: Option<[u8; 32]>,
    pub path: String,
}

/// One layer of a pad's hit.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub file: File,
    pub gain_db: f32,
    pub pan: f32,
}

/// One pad: 1 to 16, its sound's name, and its layers.
#[derive(Clone, Debug, PartialEq)]
pub struct Pad {
    pub pad: usize,
    pub name: String,
    pub layers: Vec<Layer>,
}

/// One kit, as the manifest describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Kit {
    pub name: String,
    pub maker: String,
    pub terms: String,
    /// `owner/repo` on GitHub.
    pub repo: String,
    /// The full commit (40 hex digits).
    pub commit: String,
    pub licence: File,
    pub seconds: f32,
    pub pads: Vec<Pad>,
}

/// The kits' names, as the manifest gives them (and the stick's kits file
/// and BROWSE's `Kits/MPFree/` folder name them).
pub fn kit_names() -> impl Iterator<Item = &'static str> {
    MANIFEST
        .lines()
        .filter_map(|l| l.trim().strip_prefix("kit "))
        .map(str::trim)
}

impl Kit {
    /// The address of `path` in this kit's library, at its commit.
    pub fn url(&self, path: &str) -> String {
        let path: Vec<String> = path.split('/').map(escape).collect();
        format!("{RAW}/{}/{}/{}", self.repo, self.commit, path.join("/"))
    }

    /// Every file this kit needs, the licence first.
    pub fn files(&self) -> Vec<&File> {
        let mut out = vec![&self.licence];
        for p in &self.pads {
            out.extend(p.layers.iter().map(|l| &l.file));
        }
        out
    }

    /// The library's page.
    pub fn page(&self) -> String {
        format!("https://github.com/{}", self.repo)
    }
}

/// `s` with every byte but letters, digits and `-._~` written as `%XX`.
fn escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// `h` as 64 lowercase hex digits.
pub fn hex(h: &[u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// A path inside a library: relative, `/` between plain parts, no `..`.
fn good_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 400
        && !p.contains('\\')
        && !p.chars().any(char::is_control)
        && p.split('/')
            .all(|s| !s.is_empty() && s != "." && s != ".." && s.trim() == s)
}

/// A name a musician sees, and a file name in a kit folder.
fn good_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= MAX_NAME.min(60)
        && n.trim() == n
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || " -'().,&".contains(c))
}

fn file(hash: &str, path: &str) -> Result<File, String> {
    let sha256 = match hash {
        "-" => None,
        h => Some(unhex(h).ok_or_else(|| format!("{h:?} is not a SHA-256"))?),
    };
    if !good_path(path) {
        return Err(format!("{path:?} is not a plain path inside the library"));
    }
    Ok(File {
        sha256,
        path: path.to_string(),
    })
}

fn number(s: &str, what: &str, lo: f32, hi: f32) -> Result<f32, String> {
    let v: f32 = s
        .parse()
        .map_err(|_| format!("{what} {s:?} is not a number"))?;
    if !(lo..=hi).contains(&v) {
        return Err(format!("{what} {v} is not between {lo} and {hi}"));
    }
    Ok(v)
}

/// Reads a manifest. Every kit is checked: its terms, its source, 1 to 16
/// pads with names of their own, each with at least one layer.
pub fn parse(text: &str) -> Result<Vec<Kit>, String> {
    let mut kits: Vec<Kit> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let at = |e: String| format!("free-kits.txt line {}: {e}", n + 1);
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        if word == "kit" {
            if !good_name(rest) {
                return Err(at(format!("{rest:?} can't be a kit's name")));
            }
            kits.push(Kit {
                name: rest.to_string(),
                maker: String::new(),
                terms: String::new(),
                repo: String::new(),
                commit: String::new(),
                licence: File {
                    sha256: None,
                    path: String::new(),
                },
                seconds: SECONDS,
                pads: Vec::new(),
            });
            continue;
        }
        let kit = kits
            .last_mut()
            .ok_or_else(|| at(format!("{word} before the first kit")))?;
        let mut parts = rest.splitn(2, ' ');
        let (a, b) = (
            parts.next().unwrap_or(""),
            parts.next().unwrap_or("").trim(),
        );
        match word {
            "maker" if good_name(rest) => kit.maker = rest.to_string(),
            "terms" => kit.terms = rest.to_string(),
            "source" => {
                let repo_ok = a.split('/').count() == 2
                    && a.split('/').all(|s| {
                        !s.is_empty()
                            && s != "."
                            && s != ".."
                            && s.bytes()
                                .all(|c| c.is_ascii_alphanumeric() || b"-._".contains(&c))
                    });
                let commit_ok = b.len() == 40
                    && b.bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c));
                if !repo_ok || !commit_ok {
                    return Err(at("source wants OWNER/REPO and a full commit".into()));
                }
                kit.repo = a.to_string();
                kit.commit = b.to_string();
            }
            "licence" => kit.licence = file(a, b).map_err(at)?,
            "seconds" => kit.seconds = number(rest, "seconds", 0.1, 10.0).map_err(at)?,
            "pad" => {
                let pad: usize = a
                    .parse()
                    .ok()
                    .filter(|p| (1..=16).contains(p))
                    .ok_or_else(|| at(format!("pad {a:?} is not 1 to 16")))?;
                if !good_name(b) {
                    return Err(at(format!("{b:?} can't be a sound's name")));
                }
                if kit.pads.iter().any(|p| p.pad == pad) {
                    return Err(at(format!("pad {pad} is there twice")));
                }
                if kit.pads.iter().any(|p| p.name.eq_ignore_ascii_case(b)) {
                    return Err(at(format!("two sounds are called {b:?}")));
                }
                kit.pads.push(Pad {
                    pad,
                    name: b.to_string(),
                    layers: Vec::new(),
                });
            }
            "file" => {
                let f: Vec<&str> = rest.splitn(4, ' ').collect();
                if f.len() != 4 {
                    return Err(at("file wants SHA256 GAIN PAN PATH".into()));
                }
                let layer = Layer {
                    file: file(f[0], f[3]).map_err(at)?,
                    gain_db: number(f[1], "gain", -40.0, 12.0).map_err(at)?,
                    pan: number(f[2], "pan", -1.0, 1.0).map_err(at)?,
                };
                kit.pads
                    .last_mut()
                    .ok_or_else(|| at("file before the kit's first pad".into()))?
                    .layers
                    .push(layer);
            }
            _ => return Err(at(format!("{line:?} is not a manifest line"))),
        }
    }
    for k in &kits {
        let e = |what: &str| format!("free-kits.txt, kit {:?}: {what}", k.name);
        if k.terms != TERMS {
            return Err(e(&format!("its terms are {:?}, not {TERMS}", k.terms)));
        }
        if k.maker.is_empty() || k.repo.is_empty() || k.licence.path.is_empty() {
            return Err(e("it needs maker, source and licence lines"));
        }
        if k.pads.is_empty() || k.pads.iter().any(|p| p.layers.is_empty()) {
            return Err(e("it needs pads, each with at least one file"));
        }
    }
    for (i, k) in kits.iter().enumerate() {
        if kits[..i]
            .iter()
            .any(|o| o.name.eq_ignore_ascii_case(&k.name))
        {
            return Err(format!("free-kits.txt: two kits are called {:?}", k.name));
        }
    }
    Ok(kits)
}

/// Rewrites the hash of each `file` line whose hash is `-` with the one
/// `hash(kit, path)` gives (for `--fill`); every other line is kept as it is.
pub fn fill(text: &str, hash: &mut HashOf) -> Result<String, String> {
    let kits = parse(text)?;
    let mut out = String::new();
    let mut kit = None::<usize>;
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("kit ") {
            kit = Some(kit.map_or(0, |k| k + 1));
        }
        let lead = &line[..line.len() - t.len()];
        let missing = |w: &str| -> Option<String> {
            let rest = t.strip_prefix(w)?;
            let rest = rest.strip_prefix("- ")?;
            Some(rest.to_string())
        };
        if let (Some(k), Some(rest)) = (kit, missing("file ")) {
            let path = rest.splitn(3, ' ').nth(2).unwrap_or("").trim().to_string();
            let h = hash(&kits[k], &path)?;
            out.push_str(&format!("{lead}file {} {rest}\n", hex(&h)));
        } else if let (Some(k), Some(rest)) = (kit, missing("licence ")) {
            let h = hash(&kits[k], rest.trim())?;
            out.push_str(&format!("{lead}licence {} {rest}\n", hex(&h)));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(out)
}

/// A decoded file at 44.1 kHz: left and right.
fn decode(path: &str, data: &[u8]) -> Result<(Vec<f32>, Vec<f32>), String> {
    let e = |x: String| format!("{path}: {x}");
    let (rate, left, right) = if data.starts_with(b"fLaC") {
        let d = mpfree_flac::decode(data, READ_SECONDS as usize * 192_000 * 2)
            .map_err(|x| e(x.to_string()))?;
        let ch = usize::from(d.info.channels);
        if !(1..=2).contains(&ch) {
            return Err(e(format!("{ch} channels")));
        }
        let left: Vec<f32> = d.samples.iter().step_by(ch).copied().collect();
        let right: Vec<f32> = d.samples.iter().skip(ch - 1).step_by(ch).copied().collect();
        (d.info.rate, left, right)
    } else {
        let d = crate::wav::decode(data, READ_SECONDS).map_err(e)?;
        if d.channels > 2 {
            return Err(e(format!("{} channels", d.channels)));
        }
        (d.rate, d.left, d.right)
    };
    Ok((
        crate::wav::resample(&left, rate),
        crate::wav::resample(&right, rate),
    ))
}

/// What [`render_kit`] made of one pad, for the tool's report.
#[derive(Clone, Debug, PartialEq)]
pub struct PadReport {
    pub pad: usize,
    pub name: String,
    pub seconds: f32,
    /// The gain that brought the hit's peak to about -1 dB, in dB: how far
    /// a quiet recording (and its noise) was raised.
    pub gain_db: f32,
    /// Cut at the kit's longest hit (faded out).
    pub cut: bool,
}

/// One pad's hit: its layers mixed, the silence at both ends trimmed, cut
/// at `seconds` with a fade. Interleaved stereo floats, and whether it was
/// cut.
fn mix_pad(
    pad: &Pad,
    seconds: f32,
    read: &mut dyn FnMut(&File) -> Result<Vec<u8>, String>,
) -> Result<(Vec<f32>, bool), String> {
    let mut left: Vec<f32> = Vec::new();
    let mut right: Vec<f32> = Vec::new();
    for layer in &pad.layers {
        let data = read(&layer.file)?;
        let (l, r) = decode(&layer.file.path, &data)?;
        let g = 10f32.powf(layer.gain_db / 20.0);
        // Balance: the far side is turned down, the near side kept.
        let (gl, gr) = (
            g * (1.0 - layer.pan).min(1.0),
            g * (1.0 + layer.pan).min(1.0),
        );
        if l.len() > left.len() {
            left.resize(l.len(), 0.0);
            right.resize(l.len(), 0.0);
        }
        for (i, (a, b)) in l.iter().zip(&r).enumerate() {
            left[i] += a * gl;
            right[i] += b * gr;
        }
    }
    let peak = left.iter().chain(&right).fold(0f32, |m, v| m.max(v.abs()));
    if peak == 0.0 {
        return Err(format!("pad {} ({}) is silent", pad.pad, pad.name));
    }
    let (l, r) = (&left, &right);
    let over = |db: f32| {
        let level = peak * 10f32.powf(db / 20.0);
        move |i: &usize| l[*i].abs() > level || r[*i].abs() > level
    };
    let onset = (0..left.len()).find(over(ONSET_DB)).unwrap_or(0);
    let first = (0..left.len())
        .find(over(QUIET_DB))
        .unwrap_or(0)
        .max(onset.saturating_sub(PRE_ROLL));
    let last = (0..left.len()).rev().find(over(QUIET_DB)).unwrap_or(0) + 1;
    let longest = ((seconds * RATE as f32) as usize).min(MAX_FRAMES);
    let cut = last - first > longest;
    let end = first + (last - first).min(longest);
    let n = end - first;
    // A short fade at a trimmed end; a long one where the hit was cut.
    let fade = if cut {
        (n / 4).min(RATE as usize)
    } else {
        (n / 4).min(RATE as usize / 100)
    };
    let curve = |x: f32| 0.5 - 0.5 * (std::f32::consts::PI * x).cos();
    let fade_in = onset - first;
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let left_of_end = n - i;
        let mut g = if left_of_end <= fade && fade > 0 {
            curve(left_of_end as f32 / fade as f32)
        } else {
            1.0
        };
        if i < fade_in {
            g *= curve(i as f32 / fade_in as f32);
        }
        out.push(left[first + i] * g);
        out.push(right[first + i] * g);
    }
    Ok((out, cut))
}

/// One kit's sounds, on their pads' notes, each peaking at about -1 dB.
/// `read` gives a file's bytes (checked against its hash by the caller).
pub fn render_kit(
    kit: &Kit,
    read: &mut dyn FnMut(&File) -> Result<Vec<u8>, String>,
) -> Result<(Vec<Sound>, Vec<PadReport>), String> {
    let mut sounds = Vec::new();
    let mut report = Vec::new();
    for pad in &kit.pads {
        let (audio, cut) = mix_pad(pad, kit.seconds, read)
            .map_err(|e| format!("{}, pad {}: {e}", kit.name, pad.pad))?;
        let gain = PEAK / audio.iter().fold(0f32, |m, v| m.max(v.abs()));
        report.push(PadReport {
            pad: pad.pad,
            name: pad.name.clone(),
            seconds: audio.len() as f32 / 2.0 / RATE as f32,
            gain_db: 20.0 * gain.log10(),
            cut,
        });
        sounds.push(Sound {
            note: PAD_NOTES[pad.pad - 1],
            name: pad.name.clone(),
            audio: audio
                .iter()
                .map(|v| (v * gain * 32767.0).round().clamp(-32768.0, 32767.0) as i16)
                .collect(),
        });
    }
    // In pad order, as MPFree's own kits are.
    sounds.sort_by_key(|s| PAD_NOTES.iter().position(|n| *n == s.note));
    report.sort_by_key(|r| r.pad);
    Ok((sounds, report))
}

/// Every kit in `kits` rendered into one kits file, with each kit's report.
pub fn render(kits: &[Kit], read: &mut ReadFile) -> Result<(Vec<u8>, Vec<Vec<PadReport>>), String> {
    let mut entries = Vec::new();
    let mut reports = Vec::new();
    for kit in kits {
        let licence = read(kit, &kit.licence)?;
        if !String::from_utf8_lossy(&licence).contains(LICENCE_TEXT) {
            return Err(format!(
                "{}: {} doesn't say {LICENCE_TEXT}",
                kit.name, kit.licence.path
            ));
        }
        let (sounds, report) = render_kit(kit, &mut |f| read(kit, f))?;
        entries.push((kit.name.clone(), kitfile::kit_bytes(&sounds)));
        reports.push(report);
    }
    if entries.is_empty() {
        return Err("no kits".into());
    }
    let file = kitfile::kits_bytes(&entries);
    if entries.len() > MAX_KITS || file.len() > MAX_FILE {
        return Err(format!(
            "{} kits in {} bytes: more than a kits file holds",
            entries.len(),
            file.len()
        ));
    }
    Ok((file, reports))
}

/// Where each kit's sounds came from, for the release and the stick.
pub fn credits(kits: &[Kit]) -> String {
    let mut out = String::from(
        "MPFree's free drum kits\n\
         =======================\n\n\
         Made by MPFree's mpfree-free-kits tool from these public-domain\n\
         sample libraries. Each kit's sounds are its makers' recordings,\n\
         mixed to one hit per pad; their makers dedicated them to the public\n\
         domain (CC0 1.0), so the kits are free to use for anything.\n",
    );
    for k in kits {
        out.push_str(&format!(
            "\n{}\n  Library by {}, {}: {}\n  Read at commit {}, licence file {}\n",
            k.name,
            k.maker,
            k.terms,
            k.page(),
            k.commit,
            k.licence.path
        ));
        for p in &k.pads {
            out.push_str(&format!("  Pad {:>2}, {}:\n", p.pad, p.name));
            for l in &p.layers {
                out.push_str(&format!("    {}\n", l.file.path));
            }
        }
    }
    out
}

/// The stick's kits file: MPFree's own kits, then the kits in `free` (a
/// kits file made by [`render`]) when there is one. Fails on a damaged
/// file, a kit named as one of MPFree's own, or too many kits.
pub fn stick_kits(free: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut entries: Vec<(String, Vec<u8>)> = crate::drums::kits()
        .into_iter()
        .map(|(n, s)| (n, kitfile::kit_bytes(&s)))
        .collect();
    if let Some(free) = free {
        let extra = kitfile::parse_kits(free).map_err(|e| format!("{STICK_FILE}: {e}"))?;
        for (name, sounds) in extra {
            if entries.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                return Err(format!("{STICK_FILE}: {name:?} is there twice"));
            }
            entries.push((name, kitfile::kit_bytes(&sounds)));
        }
    }
    let file = kitfile::kits_bytes(&entries);
    if entries.len() > MAX_KITS || file.len() > MAX_FILE {
        return Err(format!(
            "{} kits in {} bytes: more than a kits file holds",
            entries.len(),
            file.len()
        ));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::wav_bytes;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn manifest(hash: &str, licence: &str) -> String {
        format!(
            "# test\n\
             kit Test Kit\n  maker Someone\n  terms CC0 1.0\n\
             \x20 source someone/drums {COMMIT}\n\
             \x20 licence {licence} LICENSE\n  seconds 0.5\n\
             \x20 pad 2 Kick\n    file {hash} 0 0 Samples/kick one.wav\n\
             \x20   file {hash} -6 0.5 Samples/kick oh.wav\n\
             \x20 pad 1 Side stick\n    file {hash} 0 0 Samples/ss.wav\n"
        )
    }

    fn hit(frames: usize, start: usize, level: f32) -> Vec<u8> {
        // Silence, then a decaying hit, then silence again.
        let mut s = vec![0i32; frames];
        for (i, v) in s.iter_mut().enumerate().skip(start) {
            let t = (i - start) as f32 / 44_100.0;
            *v = (level * (-t * 3.0).exp() * (i as f32 * 0.3).sin() * 32767.0) as i32;
        }
        wav_bytes(44_100, 1, 16, &s)
    }

    #[test]
    fn the_shipped_manifest_reads_and_is_whole() {
        let kits = parse(MANIFEST).unwrap();
        assert!(!kits.is_empty());
        for k in &kits {
            assert_eq!(k.terms, TERMS);
            assert!(k.licence.sha256.is_some(), "{}: licence hash", k.name);
            for p in &k.pads {
                for l in &p.layers {
                    assert!(l.file.sha256.is_some(), "{} pad {}: hash", k.name, p.pad);
                }
            }
        }
        // MPFree's own kits and these fit in one kits file.
        assert!(crate::drums::kits().len() + kits.len() <= MAX_KITS);
        let names: Vec<&str> = kit_names().collect();
        assert_eq!(
            names,
            kits.iter().map(|k| k.name.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn manifests_are_checked() {
        let h = "ab".repeat(32);
        let kits = parse(&manifest(&h, &h)).unwrap();
        assert_eq!(kits.len(), 1);
        let k = &kits[0];
        assert_eq!((k.name.as_str(), k.seconds), ("Test Kit", 0.5));
        assert_eq!(k.pads[0].layers[1].gain_db, -6.0);
        assert_eq!(k.pads[0].layers[1].file.path, "Samples/kick oh.wav");
        assert_eq!(
            k.url("Samples/kick oh.wav"),
            format!("{RAW}/someone/drums/{COMMIT}/Samples/kick%20oh.wav")
        );
        assert_eq!(k.files().len(), 4);
        let bad = |from: &str, to: &str| parse(&manifest(&h, &h).replacen(from, to, 1));
        assert!(bad("CC0 1.0", "CC BY 4.0").unwrap_err().contains("terms"));
        assert!(bad("Samples/ss.wav", "../ss.wav").is_err());
        assert!(bad("Samples/ss.wav", "/etc/passwd").is_err());
        assert!(bad("Samples/ss.wav", "a//b.wav").is_err());
        assert!(bad("pad 1 Side stick", "pad 2 Snare").is_err());
        assert!(bad("pad 1 Side stick", "pad 17 Snare").is_err());
        assert!(bad("pad 1 Side stick", "pad 1 Kick").is_err());
        assert!(bad("pad 1 Side stick", "pad 1 Kick/../x").is_err());
        assert!(bad(COMMIT, "main").is_err());
        assert!(bad("someone/drums", "someone/../drums").is_err());
        assert!(bad("0 0 Samples/ss", "0 2 Samples/ss").is_err());
        assert!(bad("0 0 Samples/ss", "x 0 Samples/ss").is_err());
        assert!(bad("  maker Someone\n", "").is_err());
        assert!(bad("    file", "    flie").is_err());
        assert!(bad("kit Test Kit", "kit").is_err());
        assert!(parse(&format!("{}{}", manifest(&h, &h), manifest(&h, &h))).is_err());
        assert!(parse("maker Someone\n").is_err());
        // A missing hash reads, as `-`.
        assert!(parse(&manifest("-", "-")).unwrap()[0]
            .licence
            .sha256
            .is_none());
    }

    #[test]
    fn fill_writes_only_the_missing_hashes() {
        let h = "cd".repeat(32);
        let text = manifest(&h, "-").replacen(
            &format!("file {h} 0 0 Samples/ss"),
            "file - 0 0 Samples/ss",
            1,
        );
        let mut asked = Vec::new();
        let out = fill(&text, &mut |k, p| {
            asked.push((k.name.clone(), p.to_string()));
            Ok([0x11; 32])
        })
        .unwrap();
        assert_eq!(
            asked,
            [
                ("Test Kit".to_string(), "LICENSE".to_string()),
                ("Test Kit".to_string(), "Samples/ss.wav".to_string())
            ]
        );
        let ones = "11".repeat(32);
        assert!(out.contains(&format!("  licence {ones} LICENSE\n")));
        assert!(out.contains(&format!("    file {ones} 0 0 Samples/ss.wav\n")));
        assert!(out.contains(&format!("    file {h} -6 0.5 Samples/kick oh.wav\n")));
        assert_eq!(out.lines().count(), text.lines().count());
        assert!(parse(&out).unwrap()[0]
            .files()
            .iter()
            .all(|f| f.sha256.is_some()));
    }

    #[test]
    fn kits_render_trimmed_cut_and_scaled_together() {
        let h = "ab".repeat(32);
        let kits = parse(&manifest(&h, &h)).unwrap();
        let mut read = |_: &Kit, f: &File| -> Result<Vec<u8>, String> {
            Ok(match f.path.as_str() {
                "LICENSE" => b"Creative Commons Legal Code\n\nCC0 1.0 Universal\n".to_vec(),
                // 2 s long, hit at 0.1 s: cut at 0.5 s.
                "Samples/kick one.wav" => hit(88_200, 4410, 0.5),
                "Samples/kick oh.wav" => hit(88_200, 4410, 0.25),
                // A quiet, short hit after 0.2 s of low noise (50 dB under it).
                _ => {
                    let mut s: Vec<i32> = (0..8820)
                        .map(|i| if i % 2 == 0 { 10 } else { -10 })
                        .collect();
                    for (i, v) in s.iter_mut().enumerate().skip(8820 - 441) {
                        *v = (((i as f32) * 0.5).sin() * 0.1 * 32767.0) as i32;
                    }
                    wav_bytes(44_100, 1, 16, &s)
                }
            })
        };
        let (file, reports) = render(&kits, &mut read).unwrap();
        let parsed = kitfile::parse_kits(&file).unwrap();
        assert_eq!(parsed.len(), 1);
        let (name, sounds) = &parsed[0];
        assert_eq!(name, "Test Kit");
        // Pad order: pad 1 (37) before pad 2 (36).
        assert_eq!(sounds.iter().map(|s| s.note).collect::<Vec<_>>(), [37, 36]);
        let kick = &sounds[1];
        assert_eq!(kick.audio.len(), 2 * 22_050, "cut at the kit's 0.5 s");
        // It peaks at about -1 dB; the start is the hit, not silence.
        let peak = kick.audio.iter().map(|v| v.unsigned_abs()).max().unwrap();
        assert!((29_000..=29_300).contains(&peak), "{peak}");
        assert!(kick.audio[..20].iter().any(|v| *v != 0));
        // Faded out to nothing where it was cut.
        assert!(kick.audio[kick.audio.len() - 4..]
            .iter()
            .all(|v| v.abs() < 50));
        // Panned right: the right side is louder.
        let side = |c: usize| {
            kick.audio
                .iter()
                .skip(c)
                .step_by(2)
                .map(|v| (*v as i64).abs())
                .sum::<i64>()
        };
        assert!(side(1) > side(0));
        // The quiet hit is trimmed to itself and 5 ms of the noise before it,
        // faded in, and raised to peak at -1 dB too.
        let ss = &sounds[0];
        assert!(ss.audio.len() <= 2 * (441 + PRE_ROLL) && ss.audio.len() > 2 * 441);
        assert_eq!(&ss.audio[..2], [0, 0]);
        let peak = ss.audio.iter().map(|v| v.unsigned_abs()).max().unwrap();
        assert!((29_000..=29_300).contains(&peak), "{peak}");
        assert_eq!(reports[0][0].pad, 1);
        assert!(reports[0][1].cut && !reports[0][0].cut);
        // The quiet hit (0.1, -20 dB) was raised by about 19 dB.
        assert!(
            (reports[0][0].gain_db - 19.0).abs() < 0.2,
            "{}",
            reports[0][0].gain_db
        );
        // A licence that isn't CC0's text stops it.
        let mut not_cc0 = |k: &Kit, f: &File| -> Result<Vec<u8>, String> {
            if f.path == "LICENSE" {
                Ok(b"All rights reserved".to_vec())
            } else {
                read(k, f)
            }
        };
        assert!(render(&kits, &mut not_cc0)
            .unwrap_err()
            .contains("CC0 1.0 Universal"));
    }

    #[test]
    fn credits_name_every_file() {
        let h = "ab".repeat(32);
        let kits = parse(&manifest(&h, &h)).unwrap();
        let c = credits(&kits);
        assert!(
            c.contains("Test Kit\n  Library by Someone, CC0 1.0: https://github.com/someone/drums")
        );
        assert!(c.contains(COMMIT));
        assert!(c.contains("  Pad  2, Kick:\n    Samples/kick one.wav\n    Samples/kick oh.wav\n"));
    }

    #[test]
    fn the_stick_gets_mpfrees_own_kits_then_the_free_ones() {
        assert_eq!(stick_kits(None).unwrap(), crate::drums::kits_file());
        let s = Sound {
            note: 36,
            name: "Kick".into(),
            audio: vec![100, -100, 50, -50],
        };
        let free = kitfile::kits_bytes(&[(
            "Big Rusty Drums".into(),
            kitfile::kit_bytes(std::slice::from_ref(&s)),
        )]);
        let all = kitfile::parse_kits(&stick_kits(Some(&free)).unwrap()).unwrap();
        let own = crate::drums::kits().len();
        assert_eq!(all.len(), own + 1);
        assert!(all[0].0.starts_with("MPFree "));
        assert_eq!(all[own], ("Big Rusty Drums".to_string(), vec![s.clone()]));
        // A damaged file, or one of MPFree's own names again, is refused.
        assert!(stick_kits(Some(&free[..free.len() - 1])).is_err());
        let first = all[0].0.clone();
        let twice = kitfile::kits_bytes(&[(first, kitfile::kit_bytes(&[s]))]);
        assert!(stick_kits(Some(&twice)).unwrap_err().contains("twice"));
    }
}
