//! `cargo usb-creator`: builds the MPFree USB creator for this computer, with
//! this clone's MPFree for the MPC inside it, on Linux, macOS or Windows.
//!
//! ```text
//! cargo usb-creator
//! ```
//!
//! Run it in the clone's top folder (`.cargo/config.toml` there defines the
//! command). It ends with the creator as one file in `rust/target/usb-creator`
//! and the line that opens it. Run it again after a `git pull` to build a
//! newer one. It needs Rust (from rustup.rs) and an internet connection the
//! first time; nothing else, and no administrator rights. It writes only under
//! `rust/target` and the creator's own `rust/mpfree-creator/target`.
//!
//! In order: MPFree's three programs for the MPC, linked statically, as the
//! device rounds and releases build them; the download helper and the plugin
//! process, linked dynamically against the MPC's C library (glibc 2.39 at
//! most); the free drum kits; and the creator, with all of them inside it.
//! The programs for the MPC are compiled by Rust for 32-bit ARM and linked by
//! Rust's own linker (rust-lld) against Ubuntu 24.04's ARM libraries (see
//! [`libs`]), the C library the release's cross compiler links them with, so
//! no cross compiler, Linux or container is needed on any computer.
//! docs/dev/guides/build-usb-creator.md explains it.

mod elf;
mod libs;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const ARM: &str = "armv7-unknown-linux-gnueabihf";
/// The MPC's C library is glibc 2.39: a dynamically linked program may ask
/// for no newer version.
const MPC_GLIBC_MINOR: u32 = 39;
const MPC_INTERPRETER: &str = "/lib/ld-linux-armhf.so.3";

const USAGE: &str = "\
Builds the MPFree USB creator for this computer, with this clone's MPFree inside it:

  cargo usb-creator

Run it in the clone's top folder. It ends with the creator as one file in
rust/target/usb-creator, and the line to open it. Run it again after a
`git pull` to build a newer one. See docs/dev/guides/build-usb-creator.md.
";

/// Settings in the shell (or a cargo config) for other builds would change
/// what these builds make, or where; each build here sets its own.
const OTHER_BUILDS: [&str; 9] = [
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_TARGET_DIR",
    "CARGO_BUILD_TARGET",
    "CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER",
    "CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_RUSTFLAGS",
    "MPFREE_CREATOR_PROGRAMS",
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => {}
        [a] if a == "-h" || a == "--help" => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("\nSTOP: {why}");
            ExitCode::FAILURE
        }
    }
}

fn step(what: &str) {
    println!("\n== {what}");
}

/// The clone's top folder: where `cargo usb-creator` was run, which cargo
/// tells this program as the folder two above its own manifest.
fn repo_root() -> Result<PathBuf, String> {
    let here = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .and_then(|m| Some(m.parent()?.parent()?.to_path_buf()));
    let root = match here {
        Some(r) => r,
        None => std::env::current_dir().map_err(|e| format!("this folder: {e}"))?,
    };
    if !root.join("rust/mpfree-stick/src/stick.rs").is_file() {
        return Err(format!(
            "{} isn't an MPFree clone: run `cargo usb-creator` in the clone's top folder.",
            root.display()
        ));
    }
    Ok(root)
}

fn run() -> Result<(), String> {
    let root = repo_root()?;
    // Joined part by part, so Windows shows its own separators throughout.
    let out = root.join("rust").join("target").join("usb-creator");

    step("What it builds");
    let commit = git(&root, &["log", "-1", "--format=%h"]);
    println!(
        "{}",
        git(&root, &["log", "--oneline", "-1"]).unwrap_or_else(|| "(not a git clone)".into())
    );

    step("The tools");
    let rust = Rust::find()?;
    rust.add_arm_target()?;
    Command::new("curl")
        .arg("--version")
        .stdout(Stdio::null())
        .status()
        .map_err(|_| "curl isn't installed: install it (on Linux, your distribution's curl package), and run this again.".to_string())?;
    println!(
        "{}; MPFree's programs for the MPC are linked by Rust's own linker, {}",
        rust.version,
        rust.lld.display()
    );

    step("The MPC's C library, to link against (Ubuntu 24.04's, as the releases use)");
    let libdir = libs::ensure(&out)?;

    step("MPFree's programs for the MPC");
    let programs_target = out.join("arm-static");
    run_cargo(
        arm_build(&root, &rust, &libdir, Link::Static, &programs_target),
        "MPFree's programs for the MPC didn't build: the lines above say why.",
    )?;

    step("The download helper and the plugin process");
    let helpers_target = out.join("arm-dynamic");
    run_cargo(
        arm_build(&root, &rust, &libdir, Link::Dynamic, &helpers_target),
        "the download helper and the plugin process didn't build: the lines above say why.",
    )?;

    step("The free drum kits");
    free_kits(&root, &out)?;

    step("Collecting them for the creator");
    let names = Names::read(&root)?;
    let programs = out.join("stick-programs");
    collect(
        &names,
        &programs_target.join(ARM).join("release"),
        &helpers_target.join(ARM).join("release"),
        &out.join(&names.kits),
        &programs,
    )?;

    step("The USB creator, with them inside");
    let exe = creator(&root, &programs)?;
    let file = if cfg!(windows) {
        "mpfree-creator.exe"
    } else {
        "mpfree-creator"
    };
    let ready = out.join(file);
    std::fs::copy(&exe, &ready).map_err(|e| format!("{}: {e}", ready.display()))?;
    let open = if cfg!(windows) {
        format!(r"rust\target\usb-creator\{file}")
    } else {
        format!("rust/target/usb-creator/{file}")
    };
    println!(
        "\nDONE: the USB creator is ready, with MPFree {} inside:\n  {}\nOpen it with:\n  {open}",
        commit.as_deref().unwrap_or("(this folder)"),
        ready.display()
    );
    Ok(())
}

/// The first line git prints, if git is there and this is a clone.
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git")
        .args(args)
        .current_dir(root)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !s.is_empty()).then_some(s)
}

/// The Rust that runs this: its cargo, its version, and its linker.
struct Rust {
    cargo: OsString,
    version: String,
    sysroot: PathBuf,
    lld: PathBuf,
}

impl Rust {
    fn find() -> Result<Rust, String> {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let vv = Command::new("rustc")
            .arg("-vV")
            .output()
            .map_err(|_| "Rust isn't installed: install it from https://rustup.rs, open a new terminal, and run this again.".to_string())?;
        let vv = String::from_utf8_lossy(&vv.stdout).to_string();
        let field = |k: &str| {
            vv.lines()
                .find_map(|l| l.strip_prefix(k))
                .map(|v| v.trim().to_string())
        };
        let host = field("host:").ok_or("rustc -vV names no host")?;
        let version = vv.lines().next().unwrap_or("rustc").to_string();
        let sysroot = Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .map_err(|e| format!("rustc: {e}"))?;
        let sysroot = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim());
        let lld = sysroot
            .join("lib")
            .join("rustlib")
            .join(&host)
            .join("bin")
            .join(if cfg!(windows) {
                "rust-lld.exe"
            } else {
                "rust-lld"
            });
        if !lld.is_file() {
            return Err(format!(
                "this Rust has no linker of its own ({} isn't there). Install Rust from https://rustup.rs (not a system package), open a new terminal, and run this again.",
                lld.display()
            ));
        }
        Ok(Rust {
            cargo,
            version,
            sysroot,
            lld,
        })
    }

    /// Adds Rust's standard library for the MPC (32-bit ARM, hard-float
    /// Linux) when it isn't there yet.
    fn add_arm_target(&self) -> Result<(), String> {
        let has_arm = || {
            let lib = self
                .sysroot
                .join("lib")
                .join("rustlib")
                .join(ARM)
                .join("lib");
            std::fs::read_dir(lib).is_ok_and(|mut d| d.next().is_some())
        };
        if has_arm() {
            return Ok(());
        }
        println!("Adding Rust's standard library for the MPC ({ARM}):");
        let added = Command::new("rustup")
            .args(["target", "add", ARM])
            .status()
            .is_ok_and(|s| s.success());
        if !added || !has_arm() {
            return Err(format!(
                "this Rust has no standard library for the MPC ({ARM}), and rustup couldn't add it. Install Rust from https://rustup.rs (not a system package), open a new terminal, and run this again."
            ));
        }
        Ok(())
    }
}

/// How a program for the MPC is linked.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Link {
    /// MPFree's own programs: everything inside, as the device rounds build them.
    Static,
    /// The helpers that open the MPC's own libraries: against its C library.
    Dynamic,
}

/// A cargo command with no other build's settings.
fn cargo(root: &Path, rust: &Rust) -> Command {
    let mut c = Command::new(&rust.cargo);
    c.current_dir(root);
    for v in OTHER_BUILDS {
        c.env_remove(v);
    }
    c
}

/// The flags `arm-linux-gnueabihf-gcc` would hand the linker, for rust-lld:
/// the C runtime's start files around the program, from the MPC's libraries.
fn arm_rustflags(lib: &Path, link: Link) -> Vec<String> {
    let f = |name: &str| format!("-Clink-arg={}", lib.join(name).display());
    let mut flags = vec![
        "-Clinker-flavor=ld.lld".to_string(),
        format!("-Clink-arg=-L{}", lib.display()),
        "-Clink-arg=--build-id".to_string(),
        "-Clink-arg=--hash-style=gnu".to_string(),
    ];
    match link {
        Link::Static => {
            flags.insert(0, "-Ctarget-feature=+crt-static".into());
            flags.insert(1, "-Crelocation-model=static".into());
            flags.extend(["crt1.o", "crti.o", "crtbeginT.o", "crtend.o", "crtn.o"].map(f));
        }
        Link::Dynamic => {
            flags.push(format!("-Clink-arg=--dynamic-linker={MPC_INTERPRETER}"));
            flags.extend(["Scrt1.o", "crti.o", "crtbeginS.o", "crtendS.o", "crtn.o"].map(f));
        }
    }
    flags
}

/// The build of MPFree's programs (static) or the helpers (dynamic), each in
/// its own target folder so neither undoes the other's flags.
fn arm_build(root: &Path, rust: &Rust, lib: &Path, link: Link, target_dir: &Path) -> Command {
    let mut c = cargo(root, rust);
    c.args([
        "build",
        "--manifest-path",
        "rust/Cargo.toml",
        "--release",
        "--locked",
    ])
    .args(["--target", ARM])
    .arg("--target-dir")
    .arg(target_dir);
    match link {
        Link::Static => c.args([
            "-p",
            "mpfree-app",
            "-p",
            "mpfree-startup",
            "-p",
            "mpfree-screen-log",
        ]),
        Link::Dynamic => c.args([
            "-p",
            "mpfree-mpcstore",
            "--bin",
            "mpfree-fetch",
            "-p",
            "mpfree-plughost",
            "--bin",
            "mpfree-plughost",
        ]),
    };
    // The environment wins over any cargo config's linker or flags.
    c.env(
        "CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER",
        &rust.lld,
    )
    .env(
        "CARGO_ENCODED_RUSTFLAGS",
        arm_rustflags(lib, link).join("\x1f"),
    );
    c
}

fn run_cargo(mut c: Command, why: &str) -> Result<(), String> {
    let ok = c.status().map_err(|e| format!("cargo: {e}"))?.success();
    ok.then_some(()).ok_or_else(|| why.to_string())
}

/// The free drum kits, from the samples `free-kits.txt` pins (fetched once,
/// then kept in `out/downloads/free-kits`). It lists every pad of every kit:
/// that goes to `out/free-kits.log`, and only its last line here.
fn free_kits(root: &Path, out: &Path) -> Result<(), String> {
    let rust = Rust::find()?;
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("(the first time, this builds the kit maker and fetches the samples: a few minutes)");
    let log_path = out.join("free-kits.log");
    let log =
        std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let err = log
        .try_clone()
        .map_err(|e| format!("{}: {e}", log_path.display()))?;
    let ok = cargo(root, &rust)
        .args([
            "run",
            "--manifest-path",
            "rust/Cargo.toml",
            "--release",
            "--locked",
        ])
        .args(["-p", "mpfree-factory", "--bin", "mpfree-free-kits", "--"])
        .arg("--cache")
        .arg(out.join("downloads").join("free-kits"))
        .arg("--out")
        .arg(out.join("free-kits"))
        .stdout(log)
        .stderr(err)
        .status()
        .map_err(|e| format!("cargo: {e}"))?
        .success();
    let text = std::fs::read_to_string(&log_path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    if !ok {
        for l in &lines[lines.len().saturating_sub(20)..] {
            eprintln!("{l}");
        }
        return Err(format!(
            "the free drum kits couldn't be made: the lines above say why, and {} has them all.",
            log_path.display()
        ));
    }
    println!("{}", lines.last().unwrap_or(&""));
    Ok(())
}

/// The file names the creator needs, read from its source, so a program
/// added there can't be left out here.
#[derive(Debug, PartialEq)]
struct Names {
    programs: Vec<String>,
    helpers: Vec<String>,
    kits: String,
}

impl Names {
    fn read(root: &Path) -> Result<Names, String> {
        let read = |p: &str| std::fs::read_to_string(root.join(p)).map_err(|e| format!("{p}: {e}"));
        Names::parse(
            &read("rust/mpfree-stick/src/stick.rs")?,
            &read("rust/mpfree-factory/src/freekits.rs")?,
        )
    }

    fn parse(stick_rs: &str, freekits_rs: &str) -> Result<Names, String> {
        let quoted = |s: &str| -> Vec<String> {
            s.split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_string)
                .collect()
        };
        let line = |src: &str, start: &str| -> Option<String> {
            src.lines()
                .find(|l| l.starts_with(start))
                .map(str::to_string)
        };
        let programs = line(stick_rs, "pub const PROGRAM_FILES:")
            .map(|l| quoted(l.split_once('=').map_or("", |(_, v)| v)))
            .filter(|p| !p.is_empty())
            .ok_or("couldn't read PROGRAM_FILES from rust/mpfree-stick/src/stick.rs.")?;
        let one = |src: &str, name: &str, file: &str| {
            line(src, &format!("pub const {name}: &str = "))
                .and_then(|l| quoted(&l).into_iter().next())
                .ok_or(format!("couldn't read {name} from {file}."))
        };
        let helpers = vec![
            one(stick_rs, "FETCH_FILE", "rust/mpfree-stick/src/stick.rs")?,
            one(stick_rs, "PLUGHOST_FILE", "rust/mpfree-stick/src/stick.rs")?,
        ];
        let kits = one(
            freekits_rs,
            "STICK_FILE",
            "rust/mpfree-factory/src/freekits.rs",
        )?;
        Ok(Names {
            programs,
            helpers,
            kits,
        })
    }
}

/// Checks each program and puts it, with the kits, in `to`, which then holds
/// only them, printing each file's SHA-256. A file that is already there with
/// the same bytes is left alone, so an unchanged program doesn't make the
/// creator build again (its build script watches the folder).
fn collect(
    names: &Names,
    programs: &Path,
    helpers: &Path,
    kits: &Path,
    to: &Path,
) -> Result<(), String> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut take = |from: &Path, name: &str, link: Option<Link>| -> Result<(), String> {
        let data = std::fs::read(from).map_err(|_| match link {
            Some(Link::Static) => format!("the creator needs {name}, which this doesn't build: add its package to the programs' build in rust/mpfree-creator-build/src/main.rs."),
            _ => format!("{name} wasn't built ({}).", from.display()),
        })?;
        if let Some(link) = link {
            check_program(name, &data, link)?;
        }
        files.push((name.to_string(), data));
        Ok(())
    };
    for n in &names.programs {
        take(&programs.join(n), n, Some(Link::Static))?;
    }
    for n in &names.helpers {
        take(&helpers.join(n), n, Some(Link::Dynamic))?;
    }
    take(kits, &names.kits, None)?;
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(to).map_err(|e| format!("{}: {e}", to.display()))? {
        let path = entry.map_err(|e| format!("{}: {e}", to.display()))?.path();
        let keep = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| files.iter().any(|(f, _)| f == n));
        if !keep {
            std::fs::remove_file(&path)
                .or_else(|_| std::fs::remove_dir_all(&path))
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    for (name, data) in &files {
        let path = to.join(name);
        if std::fs::read(&path).ok().as_deref() != Some(data.as_slice()) {
            std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        if *name != names.kits {
            executable(&path)?;
        }
        println!("{}  {name}", libs::sha256_hex(data));
    }
    Ok(())
}

/// Marks a program runnable (for qemu or a copy to the MPC; the creator
/// carries the bytes and sets the stick's own modes). Leaves its contents and
/// modification time alone.
fn executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// A program for the MPC: ARM hard-float; static, or dynamic against the
/// MPC's C library and no newer glibc than its 2.39.
fn check_program(name: &str, data: &[u8], link: Link) -> Result<(), String> {
    let arm = elf::read(data).map_err(|e| format!("{name}: {e}"))?;
    match (link, arm.interpreter.as_deref()) {
        (Link::Static, None) => Ok(()),
        (Link::Static, Some(i)) => Err(format!(
            "{name} is linked dynamically (against {i}); it must be static."
        )),
        (Link::Dynamic, Some(MPC_INTERPRETER)) => match arm.newest_glibc {
            Some(n) if n > MPC_GLIBC_MINOR => Err(format!(
                "{name} asks for GLIBC_2.{n}, newer than the MPC's glibc 2.{MPC_GLIBC_MINOR}."
            )),
            _ => Ok(()),
        },
        (Link::Dynamic, other) => Err(format!(
            "{name} isn't linked against the MPC's C library ({MPC_INTERPRETER}): {}.",
            other.map_or("it's static".to_string(), |i| format!("it asks for {i}"))
        )),
    }
}

/// Builds the creator with the programs inside, checks each is inside byte for
/// byte, and returns where it is.
fn creator(root: &Path, programs: &Path) -> Result<PathBuf, String> {
    let rust = Rust::find()?;
    let mut c = cargo(root, &rust);
    c.args([
        "build",
        "--manifest-path",
        "rust/mpfree-creator/Cargo.toml",
        "--release",
        "--locked",
    ])
    .env("MPFREE_CREATOR_PROGRAMS", programs);
    if cfg!(windows) {
        // The C runtime linked in, as the release builds it, so the creator
        // runs on a Windows without the Visual C++ package.
        c.env("CARGO_ENCODED_RUSTFLAGS", "-Ctarget-feature=+crt-static");
    }
    run_cargo(c, "the creator didn't build: the lines above say why.")?;
    let exe = root
        .join("rust")
        .join("mpfree-creator")
        .join("target")
        .join("release")
        .join(if cfg!(windows) {
            "mpfree-creator.exe"
        } else {
            "mpfree-creator"
        });
    let built = std::fs::read(&exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    check_inside(&built, programs)?;
    Ok(exe)
}

/// Every file in `programs` is inside `exe`, byte for byte.
fn check_inside(exe: &[u8], programs: &Path) -> Result<(), String> {
    let mut missing = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(programs)
        .map_err(|e| format!("{}: {e}", programs.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    for n in names {
        let data = std::fs::read(programs.join(&n)).map_err(|e| format!("{n}: {e}"))?;
        if !contains(exe, &data) {
            missing.push(n);
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("not inside the creator: {}", missing.join(", ")))
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    let Some((&first, _)) = needle.split_first() else {
        return true;
    };
    let mut at = 0;
    while let Some(i) = hay
        .get(at..)
        .and_then(|h| h.iter().position(|&b| b == first))
    {
        let start = at + i;
        if hay.get(start..start + needle.len()) == Some(needle) {
            return true;
        }
        at = start + 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use elf::tests::{program, HARD};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "mpfree-creator-build-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn names() -> Names {
        Names {
            programs: vec!["mpfree".into(), "mpfree-startup".into()],
            helpers: vec!["mpfree-fetch".into(), "mpfree-plughost".into()],
            kits: "free-kits".into(),
        }
    }

    fn fake_rust() -> Rust {
        Rust {
            cargo: "cargo".into(),
            version: "rustc 1.97.0".into(),
            sysroot: PathBuf::from("/sysroot"),
            lld: PathBuf::from("/sysroot/lib/rustlib/host/bin/rust-lld"),
        }
    }

    /// The start file a `-Clink-arg=` names in the folder `lib`, by its name
    /// alone (on Windows, `Path::join` puts a backslash before it).
    fn start_file<'a>(flag: &'a str, lib: &str) -> Option<&'a str> {
        let path = flag.strip_prefix("-Clink-arg=")?;
        let name = path.rsplit(['/', '\\']).next()?;
        (Path::new(lib).join(name).display().to_string() == path).then_some(name)
    }

    #[test]
    fn the_names_come_from_the_creators_own_source() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let n = Names::read(&root).unwrap();
        assert!(n.programs.contains(&"mpfree".to_string()), "{n:?}");
        assert_eq!(n.helpers, ["mpfree-fetch", "mpfree-plughost"]);
        assert_eq!(n.kits, "free-kits");
        assert!(Names::parse("nothing here", "")
            .unwrap_err()
            .contains("PROGRAM_FILES"));
    }

    #[test]
    fn the_static_build_links_with_the_static_start_files_and_no_shell_settings() {
        let lib = Path::new("/libs");
        let c = arm_build(
            Path::new("/repo"),
            &fake_rust(),
            lib,
            Link::Static,
            Path::new("/t"),
        );
        let args: Vec<_> = c
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(args.windows(2).any(|w| w == ["--target", ARM]), "{args:?}");
        assert!(args.contains(&"mpfree-app".to_string()));
        let env: Vec<_> = c.get_envs().collect();
        let get = |k: &str| {
            env.iter()
                .find(|(n, _)| *n == k)
                .and_then(|(_, v)| v.map(|v| v.to_string_lossy().to_string()))
        };
        for v in ["RUSTFLAGS", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET"] {
            assert!(
                env.iter().any(|(n, val)| *n == v && val.is_none()),
                "{v} isn't cleared"
            );
        }
        assert_eq!(
            get("CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER").as_deref(),
            Some("/sysroot/lib/rustlib/host/bin/rust-lld")
        );
        let flags = get("CARGO_ENCODED_RUSTFLAGS").unwrap();
        let flags: Vec<&str> = flags.split('\x1f').collect();
        assert!(flags.contains(&"-Ctarget-feature=+crt-static"));
        assert!(flags.contains(&"-Crelocation-model=static"));
        let files: Vec<&str> = flags
            .iter()
            .filter_map(|f| start_file(f, "/libs"))
            .collect();
        assert_eq!(
            files,
            ["crt1.o", "crti.o", "crtbeginT.o", "crtend.o", "crtn.o"]
        );
    }

    #[test]
    fn the_dynamic_build_asks_for_the_mpcs_interpreter_and_the_pie_start_files() {
        let flags = arm_rustflags(Path::new("/libs"), Link::Dynamic);
        assert!(!flags
            .iter()
            .any(|f| f.contains("crt-static") || f.contains("relocation-model")));
        assert!(flags.contains(&"-Clink-arg=--dynamic-linker=/lib/ld-linux-armhf.so.3".to_string()));
        let files: Vec<&str> = flags
            .iter()
            .filter_map(|f| start_file(f, "/libs"))
            .collect();
        assert_eq!(
            files,
            ["Scrt1.o", "crti.o", "crtbeginS.o", "crtendS.o", "crtn.o"]
        );
    }

    #[test]
    fn each_program_is_checked_for_how_it_is_linked() {
        let stat = program(HARD, None, &[]);
        let dynamic = program(HARD, Some(MPC_INTERPRETER), &["GLIBC_2.4", "GLIBC_2.34"]);
        let too_new = program(HARD, Some(MPC_INTERPRETER), &["GLIBC_2.41"]);
        let other = program(HARD, Some("/lib/ld-linux.so.3"), &[]);
        assert!(check_program("mpfree", &stat, Link::Static).is_ok());
        assert!(check_program("mpfree", &dynamic, Link::Static)
            .unwrap_err()
            .contains("must be static"));
        assert!(check_program("mpfree-fetch", &dynamic, Link::Dynamic).is_ok());
        assert!(check_program("mpfree-fetch", &too_new, Link::Dynamic)
            .unwrap_err()
            .contains("GLIBC_2.41"));
        assert!(check_program("mpfree-fetch", &other, Link::Dynamic)
            .unwrap_err()
            .contains("/lib/ld-linux.so.3"));
        assert!(check_program("mpfree-fetch", &stat, Link::Dynamic)
            .unwrap_err()
            .contains("static"));
    }

    #[test]
    fn collect_copies_the_checked_programs_and_the_kits() {
        let d = temp("collect");
        let (p, h) = (d.join("p"), d.join("h"));
        std::fs::create_dir_all(&p).unwrap();
        std::fs::create_dir_all(&h).unwrap();
        std::fs::write(p.join("mpfree"), program(HARD, None, &[])).unwrap();
        std::fs::write(p.join("mpfree-startup"), program(HARD, None, &[])).unwrap();
        for n in ["mpfree-fetch", "mpfree-plughost"] {
            std::fs::write(
                h.join(n),
                program(HARD, Some(MPC_INTERPRETER), &["GLIBC_2.34"]),
            )
            .unwrap();
        }
        std::fs::write(d.join("free-kits"), b"kits").unwrap();
        let to = d.join("out");
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(to.join("stale"), b"from an earlier run").unwrap();
        collect(&names(), &p, &h, &d.join("free-kits"), &to).unwrap();
        let mut got: Vec<_> = std::fs::read_dir(&to)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                "free-kits",
                "mpfree",
                "mpfree-fetch",
                "mpfree-plughost",
                "mpfree-startup"
            ]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode =
                |n: &str| std::fs::metadata(to.join(n)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode("mpfree"), 0o755);
            assert_eq!(mode("mpfree-fetch"), 0o755);
        }
        // Run again with nothing changed, no file is written again; a changed one is.
        let written = |n: &str| std::fs::metadata(to.join(n)).unwrap().modified().unwrap();
        let (mpfree_before, kits_before) = (written("mpfree"), written("free-kits"));
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(d.join("free-kits"), b"new kits").unwrap();
        collect(&names(), &p, &h, &d.join("free-kits"), &to).unwrap();
        assert_eq!(written("mpfree"), mpfree_before);
        assert_ne!(written("free-kits"), kits_before);
        assert_eq!(std::fs::read(to.join("free-kits")).unwrap(), b"new kits");
        // A program the creator needs that this doesn't build stops it, by name,
        // and leaves the folder as it was.
        let mut more = names();
        more.programs.push("mpfree-new".into());
        let e = collect(&more, &p, &h, &d.join("free-kits"), &to).unwrap_err();
        assert!(
            e.contains("mpfree-new") && e.contains("doesn't build"),
            "{e}"
        );
        assert_eq!(std::fs::read(to.join("free-kits")).unwrap(), b"new kits");
        // So does one that isn't an ARM hard-float program.
        std::fs::write(p.join("mpfree-startup"), b"#!/bin/sh\n").unwrap();
        assert!(collect(&names(), &p, &h, &d.join("free-kits"), &to)
            .unwrap_err()
            .contains("mpfree-startup"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_program_missing_from_the_creator_is_named() {
        let d = temp("inside");
        std::fs::write(d.join("mpfree"), b"\x7fELF one program").unwrap();
        std::fs::write(d.join("free-kits"), b"kits kits").unwrap();
        let exe = b"header \x7fELF \x7fELF one program and kits kits, end".to_vec();
        assert!(check_inside(&exe, &d).is_ok());
        std::fs::write(d.join("mpfree-startup"), b"\x7fELF another").unwrap();
        assert_eq!(
            check_inside(&exe, &d).unwrap_err(),
            "not inside the creator: mpfree-startup"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn contains_finds_a_needle_after_false_starts() {
        assert!(contains(b"aab aabc", b"aabc"));
        assert!(!contains(b"aab aab", b"aabc"));
        assert!(contains(b"x", b""));
        assert!(!contains(b"", b"x"));
    }
}
