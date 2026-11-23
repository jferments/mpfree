//! The changes MPFree makes to a copy of Akai's root filesystem, as
//! `tools/mpc-one/creator-recipe.py` recorded them from
//! `offline-kexec-stick.py`'s own `build_rootfs` (`recipe.json`; a Python
//! test keeps it up to date), plus the creator stick's own setup files from
//! `tools/mpc-one/stick/`.

use serde_json::Value;
use std::collections::BTreeMap;

const JSON: &str = include_str!("recipe.json");

/// A file the recipe writes: its mode, and its text or a placeholder for
/// bytes the creator supplies (the pad player, the screen log, the host key).
#[derive(Clone, Debug)]
pub enum Content {
    Text(String),
    Placeholder(String),
}

#[derive(Clone, Debug)]
pub struct File {
    pub mode: u16,
    pub content: Content,
}

#[derive(Clone, Debug)]
pub struct AppUser {
    pub name: String,
    pub uid: u32,
    pub groups: Vec<String>,
    pub optional_groups: Vec<String>,
    pub programs: Vec<String>,
    pub gecos: String,
    pub home: String,
    pub shell: String,
}

/// `/boot/mpfree-kexec/SHA256SUMS` on a chain stick: the names it lists in
/// `dir` (a file the route writes, or a link to one of Akai's), hashed.
#[derive(Clone, Debug)]
pub struct KexecSums {
    pub path: String,
    pub dir: String,
    pub names: Vec<String>,
}

/// The chain's check of Akai's own kexec: it must stay, and be the one
/// systemd-shutdown runs (the builder's chain_kexec_check).
#[derive(Clone, Debug)]
pub struct ChainCheck {
    pub kexec_program: String,
    pub shutdown_program: String,
    pub kexec_paths: Vec<String>,
}

/// A developer route (`--kexec-file`, `--loadpin-off`, `--chain`): what
/// its stick has beyond the soft-reboot stick's recipe, and what it leaves
/// out. Recorded from the builder's own build_rootfs for that route.
#[derive(Clone, Debug)]
pub struct Route {
    pub files: BTreeMap<String, File>,
    pub links: BTreeMap<String, String>,
    /// The soft-reboot stick's links this route does not make.
    pub unlinked: Vec<String>,
    /// Stub programs this route leaves as Akai's.
    pub unstubbed: Vec<String>,
    pub mkdirs: Vec<String>,
    /// The hook's list, in place of the soft-reboot stick's.
    pub softreboot_listed: Vec<String>,
    pub kexec_sums: Option<KexecSums>,
    pub chain_check: Option<ChainCheck>,
}

/// What the chain's device tree (kexec.dtb) is built with, from the builder.
#[derive(Clone, Debug)]
pub struct KexecDtb {
    pub ethernet: String,
    pub phy_compatible: String,
    pub emmc_content: String,
    /// (label, unit address): the eMMC and SD controllers, disabled in it.
    pub hidden_controllers: Vec<(String, String)>,
    pub chosen_not_copied: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Recipe {
    pub files: BTreeMap<String, File>,
    pub links: BTreeMap<String, String>,
    pub mkdirs: Vec<String>,
    pub made_if_missing: Vec<String>,
    /// path -> "regular", "directory" or "" (anything)
    pub required: BTreeMap<String, String>,
    pub stub: String,
    pub stub_programs: Vec<String>,
    pub only_with_ssh: Vec<String>,
    pub softreboot_sums: String,
    pub softreboot_listed: Vec<String>,
    pub app_user: AppUser,
    pub data_name: String,
    pub data_start_sectors: u64,
    pub log_size: u64,
    pub kits_max: u64,
    pub placeholders: BTreeMap<String, String>,
    /// "kexec-file", "kexec-file-loadpin-off" and "chain".
    pub routes: BTreeMap<String, Route>,
    pub kexec_dtb: KexecDtb,
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(s).collect())
        .unwrap_or_default()
}

fn files(v: &Value) -> BTreeMap<String, File> {
    v.as_object()
        .map(|o| {
            o.iter()
                .map(|(path, f)| {
                    let content = match f.get("placeholder") {
                        Some(p) => Content::Placeholder(s(p)),
                        None => Content::Text(s(&f["text"])),
                    };
                    (
                        path.clone(),
                        File {
                            mode: f["mode"].as_u64().unwrap_or(0o644) as u16,
                            content,
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn map(v: &Value) -> BTreeMap<String, String> {
    v.as_object()
        .map(|o| {
            o.iter()
                .map(|(k, x)| (k.clone(), if x.is_null() { String::new() } else { s(x) }))
                .collect()
        })
        .unwrap_or_default()
}

fn route(v: &Value) -> Route {
    Route {
        files: files(&v["files"]),
        links: map(&v["links"]),
        unlinked: strings(&v["unlinked"]),
        unstubbed: strings(&v["unstubbed"]),
        mkdirs: strings(&v["mkdirs"]),
        softreboot_listed: strings(&v["softreboot_listed"]),
        kexec_sums: v["kexec_sums"].as_object().map(|_| KexecSums {
            path: s(&v["kexec_sums"]["path"]),
            dir: s(&v["kexec_sums"]["dir"]),
            names: strings(&v["kexec_sums"]["names"]),
        }),
        chain_check: v["chain_check"].as_object().map(|_| ChainCheck {
            kexec_program: s(&v["chain_check"]["kexec_program"]),
            shutdown_program: s(&v["chain_check"]["shutdown_program"]),
            kexec_paths: strings(&v["chain_check"]["kexec_paths"]),
        }),
    }
}

impl Recipe {
    pub fn load() -> Recipe {
        let v: Value = serde_json::from_str(JSON).expect("recipe.json is valid JSON");
        let map = |key: &str| map(&v[key]);
        let k = &v["kexec_dtb"];
        let u = &v["app_user"];
        Recipe {
            files: files(&v["files"]),
            links: map("links"),
            mkdirs: strings(&v["mkdirs"]),
            made_if_missing: strings(&v["made_if_missing"]),
            required: map("required"),
            stub: s(&v["stub"]),
            stub_programs: strings(&v["stub_programs"]),
            only_with_ssh: strings(&v["only_with_ssh"]),
            softreboot_sums: s(&v["softreboot_sums"]),
            softreboot_listed: strings(&v["softreboot_listed"]),
            app_user: AppUser {
                name: s(&u["name"]),
                uid: u["uid"].as_u64().unwrap_or(0) as u32,
                groups: strings(&u["groups"]),
                optional_groups: strings(&u["optional_groups"]),
                programs: strings(&u["programs"]),
                gecos: s(&u["gecos"]),
                home: s(&u["home"]),
                shell: s(&u["shell"]),
            },
            data_name: s(&v["data_name"]),
            data_start_sectors: v["data_start_sectors"].as_u64().unwrap_or(0),
            log_size: v["log_size"].as_u64().unwrap_or(0),
            kits_max: v["kits_max"].as_u64().unwrap_or(0),
            placeholders: map("placeholders"),
            routes: v["routes"]
                .as_object()
                .map(|o| o.iter().map(|(name, r)| (name.clone(), route(r))).collect())
                .unwrap_or_default(),
            kexec_dtb: KexecDtb {
                ethernet: s(&k["ethernet"]),
                phy_compatible: s(&k["phy_compatible"]),
                emmc_content: s(&k["emmc_content"]),
                hidden_controllers: k["hidden_controllers"]
                    .as_array()
                    .map(|a| a.iter().map(|pair| (s(&pair[0]), s(&pair[1]))).collect())
                    .unwrap_or_default(),
                chosen_not_copied: strings(&k["chosen_not_copied"]),
            },
        }
    }
}

impl Recipe {
    /// The recipe for a developer route: the soft-reboot stick's with the
    /// route's files, links and folders added, its unlinked links and
    /// unstubbed programs left out, and its list for the hook. None for an
    /// unknown name.
    pub fn with_route(&self, name: &str) -> Option<(Recipe, Route)> {
        let route = self.routes.get(name)?.clone();
        let mut r = self.clone();
        for (path, f) in &route.files {
            if r.files.insert(path.clone(), f.clone()).is_some() {
                return None; // recorded as what differs: never both
            }
        }
        r.links.retain(|path, _| !route.unlinked.contains(path));
        r.links.extend(route.links.clone());
        r.stub_programs.retain(|p| !route.unstubbed.contains(p));
        r.mkdirs.extend(route.mkdirs.clone());
        r.softreboot_listed = route.softreboot_listed.clone();
        Some((r, route))
    }
}

/// The creator stick's own files on partition 1 (path, mode, text), from
/// tools/mpc-one/stick/. `@DATA_UUID@` is filled in when the stick is made;
/// a missing folder (the splash unit's drop-in folder) is made.
pub const SETUP_FILES: &[(&str, u16, &str)] = &[
    (
        "/etc/mpfree-test/setup-backup.sh",
        0o755,
        include_str!("../../../tools/mpc-one/stick/setup-backup.sh"),
    ),
    (
        "/etc/mpfree-test/setup-format.sh",
        0o755,
        include_str!("../../../tools/mpc-one/stick/setup-format.sh"),
    ),
    (
        "/etc/mpfree-test/setup-sd-card.sh",
        0o755,
        include_str!("../../../tools/mpc-one/stick/setup-sd-card.sh"),
    ),
    (
        "/etc/systemd/system/mpfree-setup-backup.service",
        0o644,
        include_str!("../../../tools/mpc-one/stick/mpfree-setup-backup.service"),
    ),
    (
        "/etc/systemd/system/mpfree-setup-backup.path",
        0o644,
        include_str!("../../../tools/mpc-one/stick/mpfree-setup-backup.path"),
    ),
    (
        "/etc/systemd/system/mpfree-setup-format.service",
        0o644,
        include_str!("../../../tools/mpc-one/stick/mpfree-setup-format.service"),
    ),
    (
        "/etc/systemd/system/mpfree-setup-sd-card.service",
        0o644,
        include_str!("../../../tools/mpc-one/stick/mpfree-setup-sd-card.service"),
    ),
    (
        "/etc/systemd/system/mpfree-setup-sd-card.path",
        0o644,
        include_str!("../../../tools/mpc-one/stick/mpfree-setup-sd-card.path"),
    ),
    (
        "/etc/systemd/system/mpfree-test-splash.service.d/mpfree-setup.conf",
        0o644,
        include_str!("../../../tools/mpc-one/stick/mpfree-test-splash-setup.conf"),
    ),
];

/// The programs the stick's own scripts run, from each one's `NEEDS='...'`
/// line, sorted: the creator makes no stick from a system that lacks one
/// (Akai's has no `blockdev`, for one).
pub fn setup_programs() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = SETUP_FILES
        .iter()
        .flat_map(|(_, _, text)| text.lines())
        .filter_map(|l| l.strip_prefix("NEEDS='")?.strip_suffix('\''))
        .flat_map(str::split_whitespace)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Links that start them at boot.
pub const SETUP_LINKS: &[(&str, &str)] = &[
    (
        "/etc/systemd/system/sysinit.target.wants/mpfree-setup-backup.path",
        "/etc/systemd/system/mpfree-setup-backup.path",
    ),
    (
        "/etc/systemd/system/sysinit.target.wants/mpfree-setup-format.service",
        "/etc/systemd/system/mpfree-setup-format.service",
    ),
    (
        "/etc/systemd/system/sysinit.target.wants/mpfree-setup-sd-card.path",
        "/etc/systemd/system/mpfree-setup-sd-card.path",
    ),
];

/// The setup partition's files (name, text), from tools/mpc-one/stick/ and
/// the hook's own installer and remover, byte for byte. `@RELEASE@` in
/// autorun.sh is filled in.
pub const SETUP_DRIVE: &[(&str, &str)] = &[
    (
        "autorun.sh",
        include_str!("../../../tools/mpc-one/stick/autorun.sh"),
    ),
    (
        "hook-install.sh",
        include_str!("../../../tools/mpc-one/hook-install.sh"),
    ),
    (
        "hook-remove.sh",
        include_str!("../../../tools/mpc-one/hook-remove.sh"),
    ),
    (
        "mode.txt",
        include_str!("../../../tools/mpc-one/stick/mode.txt"),
    ),
    (
        "README.txt",
        include_str!("../../../tools/mpc-one/stick/README.txt"),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_recipe_loads_and_names_the_soft_reboot_pieces() {
        let r = Recipe::load();
        assert!(r.files.contains_key("/etc/mpfree-test/internal-off.sh"));
        assert!(r.files.contains_key("/boot/mpfree-softreboot/PROTOCOL"));
        assert!(
            matches!(r.files["/etc/mpfree-test/mpfree"].content, Content::Placeholder(ref p) if p == "@PAD_PLAYER@")
        );
        assert_eq!(r.links["/etc/systemd/system/acvs.service"], "/dev/null");
        assert_eq!(r.app_user.uid, 960);
        assert_eq!(r.data_start_sectors, 2 * 1024 * 1024);
        assert_eq!(r.data_name, "mpfree-test-data");
        assert!(r
            .only_with_ssh
            .contains(&"/etc/mpfree-test/sshd_config".to_string()));
        assert!(r.stub_programs.contains(&"/usr/sbin/kexec".to_string()));
        for (path, _, text) in SETUP_FILES {
            assert!(!r.files.contains_key(*path), "{path}");
            assert!(
                text.starts_with("R=''\n")
                    || text.starts_with("[Unit]\n")
                    || text.starts_with("[Service]\n"),
                "{path}"
            );
        }
        assert_eq!(SETUP_DRIVE[0].1.matches("@RELEASE@").count(), 1);
    }

    #[test]
    fn the_recipe_has_the_developer_routes() {
        let r = Recipe::load();
        let names: Vec<&str> = r.routes.keys().map(String::as_str).collect();
        assert_eq!(names, ["chain", "kexec-file", "kexec-file-loadpin-off"]);
        let chain = &r.routes["chain"];
        assert_eq!(chain.unstubbed, ["/usr/sbin/kexec"]);
        assert!(chain.kexec_sums.is_some() && chain.chain_check.is_some());
        assert!(matches!(
            chain.files["/boot/mpfree-kexec/kexec.dtb"].content,
            Content::Placeholder(ref p) if *p == r.placeholders["kexec_dtb"]
        ));
        let kf = &r.routes["kexec-file"];
        assert!(kf.kexec_sums.is_none() && kf.chain_check.is_none() && kf.links.is_empty());
        assert_eq!(
            kf.unlinked,
            ["/etc/systemd/system/az0x-loadpin-trustpoints.service"]
        );
        assert!(r.links.contains_key(&kf.unlinked[0]));
        assert_eq!(r.kexec_dtb.hidden_controllers.len(), 2);
        assert_eq!(r.kexec_dtb.hidden_controllers[0].0, "eMMC");
    }

    #[test]
    fn a_route_adds_to_the_soft_reboot_recipe() {
        let r = Recipe::load();
        assert!(r.with_route("no-such-route").is_none());
        let (c, route) = r.with_route("chain").unwrap();
        assert!(c.files.contains_key("/etc/mpfree-test/chain.sh"));
        assert!(c.files.contains_key("/etc/mpfree-test/internal-off.sh"));
        assert!(!c
            .links
            .contains_key("/etc/systemd/system/az0x-loadpin-trustpoints.service"));
        assert_eq!(c.links["/boot/mpfree-kexec/zImage"], "../zImage-@RELEASE@");
        assert!(!c.stub_programs.contains(&"/usr/sbin/kexec".to_string()));
        assert!(c
            .stub_programs
            .contains(&"/usr/sbin/az01-update".to_string()));
        assert!(c.mkdirs.contains(&"/boot/mpfree-kexec".to_string()));
        assert_eq!(c.softreboot_listed, route.softreboot_listed);
        let (k, _) = r.with_route("kexec-file").unwrap();
        assert_eq!(k.stub_programs, r.stub_programs);
        assert_eq!(k.files.len(), r.files.len() + 3);
    }

    #[test]
    fn setup_programs_come_from_every_script() {
        let p = setup_programs();
        for name in [
            "udevadm",
            "sha256sum",
            "dd",
            "mke2fs",
            "od",
            "mount",
            "umount",
            "dmesg",
            "rmdir",
        ] {
            assert!(p.contains(&name), "{name}");
        }
        assert!(!p.contains(&"blockdev"), "Akai's system has none");
        assert!(p.windows(2).all(|w| w[0] < w[1]), "sorted, once each");
    }
}
