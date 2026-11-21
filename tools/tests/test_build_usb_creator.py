#!/usr/bin/env python3
"""Tests for tools/build-usb-creator.sh with stand-in tools: which way it links the
helpers, what it stops on and says, and that the creator it ends with carries every
program. The real builds are what the device rehearsal and the release workflow run;
here cargo, rustup, the cross compiler and readelf are small scripts that record how
they were called."""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "tools" / "build-usb-creator.sh"
STICK_RS = REPO / "rust" / "mpfree-stick" / "src" / "stick.rs"
ARM = "armv7-unknown-linux-gnueabihf"
# The real programs the script itself uses; everything else on PATH is a stand-in.
# They are taken from the system's own folders, not this shell's PATH, which may
# hold wrappers.
REAL = ["bash", "sh", "env", "sed", "grep", "tr", "sort", "tail", "head", "cut", "wc",
        "sha256sum", "cp", "rm", "mkdir", "cat", "dirname", "uname", "printf"]
SYSTEM_PATH = "/usr/local/bin:/usr/bin:/bin"

CARGO = r"""#!/usr/bin/env python3
import os, sys
args = sys.argv[1:]
log = os.environ["STUB_LOG"]
with open(log, "a") as f:
    f.write("cargo %s | RUSTFLAGS=%s | ZIGPY=%s | TARGET_DIR=%s\n" % (
        " ".join(args), os.environ.get("RUSTFLAGS", "-"),
        os.environ.get("CARGO_ZIGBUILD_PYTHON_PATH", "-"), os.environ.get("CARGO_TARGET_DIR", "-")))
def opt(name, default=None):
    return args[args.index(name) + 1] if name in args else default
def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
if args[:1] == ["--version"]:
    print("cargo 1.97.0 (stub)"); sys.exit(0)
if args[:1] == ["zigbuild"]:
    if "--help" in args:
        sys.exit(0 if os.environ.get("STUB_ZIGBUILD") == "1" else 101)
    target_dir = opt("--target-dir", "rust/target")
    for b in ("mpfree-fetch", "mpfree-plughost"):
        write(f"{target_dir}/ARM/release/{b}", b"zig-built " + b.encode())
    sys.exit(0)
if args[:1] == ["run"]:
    if os.environ.get("STUB_KITS_FAIL") == "1":
        print("error: couldn't fetch the samples", file=sys.stderr); sys.exit(1)
    write(opt("--out"), b"free kits\n")
    print("pad listing"); print("free-kits: 3 kits, 10 bytes"); sys.exit(0)
if args[:1] == ["build"]:
    manifest = opt("--manifest-path")
    if manifest == "rust/mpfree-creator/Cargo.toml":
        d = os.environ["MPFREE_CREATOR_PROGRAMS"]
        skip = os.environ.get("STUB_LEAVE_OUT", "")
        data = b"creator:" + b"".join(open(os.path.join(d, n), "rb").read()
                                      for n in sorted(os.listdir(d)) if n != skip)
        write("rust/mpfree-creator/target/release/mpfree-creator", data); sys.exit(0)
    target_dir = opt("--target-dir", "rust/target")
    pkgs = [args[i + 1] for i, a in enumerate(args) if a == "-p"]
    bins = [args[i + 1] for i, a in enumerate(args) if a == "--bin"]
    names = bins or [{"mpfree-app": "mpfree"}.get(p, p) for p in pkgs]
    for n in names:
        write(f"{target_dir}/ARM/release/{n}", b"gcc-built " + n.encode())
    sys.exit(0)
sys.exit("stub cargo: unexpected " + " ".join(args))
""".replace("ARM", ARM)

RUSTUP = """#!/bin/sh
echo "rustup $*" >> "$STUB_LOG"
[ "$1 $2 $3" = "target list --installed" ] && { echo x86_64-unknown-linux-gnu; [ "$STUB_HAS_TARGET" = 1 ] && echo ARM; }
exit 0
""".replace("ARM", ARM)

GCC = """#!/bin/sh
[ "$1" = -dumpversion ] && echo 13
"""

READELF = """#!/bin/sh
# -h: an ARM hard-float header; -l: the MPC's interpreter for the helpers; --version-info: their newest glibc.
case "$1" in
  -h) echo "  Machine:                           ARM"; echo "  Flags:                             0x5000400, Version5 EABI, hard-float ABI" ;;
  -l) echo "      [Requesting program interpreter: /lib/ld-linux-armhf.so.3]" ;;
  -W) echo "  0x0010:   Name: GLIBC_2.4  Flags: none  Version: 2"; echo "  0x0020:   Name: GLIBC_${STUB_GLIBC:-2.34}  Flags: none  Version: 3" ;;
esac
"""


class BuildUsbCreatorTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = Path(self.tmp.name)
        self.repo = t / "repo"
        (self.repo / "tools").mkdir(parents=True)
        shutil.copy2(SCRIPT, self.repo / "tools" / "build-usb-creator.sh")
        (self.repo / "rust" / "mpfree-stick" / "src").mkdir(parents=True)
        shutil.copy2(STICK_RS, self.repo / "rust" / "mpfree-stick" / "src" / "stick.rs")
        self.bin = t / "bin"
        self.bin.mkdir()
        for name in REAL:
            real = shutil.which(name, path=SYSTEM_PATH)
            self.assertIsNotNone(real, name)
            (self.bin / name).symlink_to(real)
        (self.bin / "python3").symlink_to(sys.executable)
        self.home = t / "home"
        self.home.mkdir()
        self.log = t / "calls.log"
        self.stub("cargo", CARGO)
        self.stub("rustup", RUSTUP)
        self.stub("arm-linux-gnueabihf-gcc", GCC)
        self.stub("arm-linux-gnueabihf-readelf", READELF)

    def tearDown(self):
        self.tmp.cleanup()

    def stub(self, name, text):
        p = self.bin / name
        p.write_text(text)
        p.chmod(p.stat().st_mode | stat.S_IXUSR)

    def run_script(self, args=(), **env):
        e = {"PATH": str(self.bin), "HOME": str(self.home), "STUB_LOG": str(self.log),
             "STUB_HAS_TARGET": "1", "LC_ALL": "C"}
        e.update(env)
        r = subprocess.run(["bash", "tools/build-usb-creator.sh", *args], cwd=self.repo, env=e,
                           capture_output=True, text=True, timeout=60)
        return r, r.stdout + r.stderr, self.log.read_text() if self.log.exists() else ""

    def creator(self):
        return self.repo / "rust" / "target" / "usb-creator" / "mpfree-creator"

    def test_with_zig_on_path_it_links_the_helpers_against_glibc_2_34_and_carries_everything(self):
        self.stub("zig", "#!/bin/sh\n")
        r, out, calls = self.run_script(STUB_ZIGBUILD="1")
        self.assertEqual(r.returncode, 0, out)
        self.assertIn("helpers linked with cargo-zigbuild", out)
        self.assertIn(f"zigbuild --manifest-path rust/Cargo.toml --release --locked --target-dir rust/target/helpers --target {ARM}.2.34", calls)
        self.assertIn("DONE: the USB creator is ready", out)
        self.assertIn("Open it with:\n  rust/target/usb-creator/mpfree-creator", out)
        data = self.creator().read_bytes()
        for piece in (b"gcc-built mpfree", b"gcc-built mpfree-startup", b"gcc-built mpfree-screen-log",
                      b"zig-built mpfree-fetch", b"zig-built mpfree-plughost", b"free kits"):
            self.assertIn(piece, data)
        staged = sorted(os.listdir(self.repo / "rust" / "target" / "usb-creator" / "stick-programs"))
        self.assertEqual(staged, ["free-kits", "mpfree", "mpfree-fetch", "mpfree-plughost",
                                  "mpfree-screen-log", "mpfree-startup"])

    def test_the_g1a_zig_folder_is_used_when_zig_is_not_on_path(self):
        venv = self.home / ".local" / "share" / "mpfree-zig" / "bin"
        venv.mkdir(parents=True)
        (venv / "python3").write_text("#!/bin/sh\n")
        (venv / "python3").chmod(0o755)
        r, out, calls = self.run_script(STUB_ZIGBUILD="1")
        self.assertEqual(r.returncode, 0, out)
        zig_line = next(l for l in calls.splitlines() if l.startswith("cargo zigbuild --manifest-path"))
        self.assertIn(f"ZIGPY={venv / 'python3'}", zig_line)

    def test_without_zig_the_cross_compiler_links_them_and_glibc_2_39_is_fine(self):
        r, out, calls = self.run_script(STUB_ZIGBUILD="0", STUB_GLIBC="2.39")
        self.assertEqual(r.returncode, 0, out)
        self.assertIn("helpers linked with the cross compiler", out)
        self.assertNotIn("cargo zigbuild --manifest", calls)
        self.assertIn(f"--target-dir rust/target/helpers --target {ARM} -p mpfree-mpcstore", calls)

    def test_a_cross_compiler_newer_than_the_mpcs_glibc_stops_with_the_line_that_installs_zig(self):
        r, out, calls = self.run_script(STUB_ZIGBUILD="0", STUB_GLIBC="2.43")
        self.assertEqual(r.returncode, 1, out)
        self.assertIn("STOP: mpfree-fetch asks for GLIBC_2.43, newer than the MPC's glibc 2.39", out)
        self.assertIn("cargo install --locked cargo-zigbuild && python3 -m venv", out)
        self.assertIn("ziglang==0.17.1", out)
        self.assertNotIn("mpfree-creator/Cargo.toml", calls)
        self.assertFalse(self.creator().exists())

    def test_no_cross_compiler_stops_before_building_and_names_the_packages(self):
        (self.bin / "arm-linux-gnueabihf-gcc").unlink()
        r, out, calls = self.run_script()
        self.assertEqual(r.returncode, 1, out)
        self.assertIn("STOP: the ARM cross compiler (arm-linux-gnueabihf-gcc) isn't installed", out)
        self.assertIn("sudo apt install gcc-arm-linux-gnueabihf libc6-dev-armhf-cross", out)
        self.assertNotIn("cargo build", calls)

    def test_no_rust_stops_and_points_at_rustup(self):
        (self.bin / "cargo").unlink()
        r, out, _ = self.run_script()
        self.assertEqual(r.returncode, 1, out)
        self.assertIn("STOP: Rust isn't installed. Install it from https://rustup.rs", out)

    def test_it_adds_the_arm_target_only_when_it_is_missing(self):
        r, out, calls = self.run_script(STUB_HAS_TARGET="0")
        self.assertEqual(r.returncode, 0, out)
        self.assertIn(f"rustup target add {ARM}", calls)
        self.log.unlink()
        r, out, calls = self.run_script(STUB_HAS_TARGET="1")
        self.assertEqual(r.returncode, 0, out)
        self.assertNotIn("rustup target add", calls)

    def test_the_shells_rustflags_and_target_dir_reach_none_of_its_builds(self):
        r, out, calls = self.run_script(RUSTFLAGS="-C opt-level=0", CARGO_TARGET_DIR="/elsewhere")
        self.assertEqual(r.returncode, 0, out)
        builds = [l for l in calls.splitlines() if l.startswith(("cargo build", "cargo run", "cargo zigbuild --manifest"))]
        self.assertEqual(len(builds), 4)
        self.assertTrue(all("TARGET_DIR=-" in l for l in builds), calls)
        self.assertIn("RUSTFLAGS=-C target-feature=+crt-static -C relocation-model=static", builds[0])
        self.assertTrue(all("RUSTFLAGS=-" in l for l in builds[1:]), calls)
        self.assertNotIn("opt-level=0", calls)

    def test_a_program_missing_from_the_creator_stops_it(self):
        r, out, _ = self.run_script(STUB_LEAVE_OUT="mpfree-plughost")
        self.assertEqual(r.returncode, 1, out)
        self.assertIn("STOP: not inside the creator: mpfree-plughost", out)
        self.assertFalse(self.creator().exists())

    def test_a_program_the_creator_needs_but_the_script_does_not_build_stops_it(self):
        stick = self.repo / "rust" / "mpfree-stick" / "src" / "stick.rs"
        text = stick.read_text()
        old = 'pub const PROGRAM_FILES: [&str; 3] = ["mpfree", "mpfree-screen-log", "mpfree-startup"];'
        self.assertIn(old, text)
        stick.write_text(text.replace(old, 'pub const PROGRAM_FILES: [&str; 4] = ["mpfree", "mpfree-screen-log", "mpfree-startup", "mpfree-new"];'))
        r, out, _ = self.run_script()
        self.assertEqual(r.returncode, 1, out)
        self.assertIn("STOP: the creator needs mpfree-new, which this script doesn't build", out)

    def test_the_kits_listing_goes_to_a_log_and_a_kits_failure_stops_with_its_lines(self):
        r, out, _ = self.run_script()
        self.assertEqual(r.returncode, 0, out)
        self.assertIn("free-kits: 3 kits, 10 bytes", out)
        self.assertNotIn("pad listing", out)
        self.assertIn("pad listing", (self.repo / "rust/target/usb-creator/free-kits.log").read_text())
        self.log.unlink()
        r, out, calls = self.run_script(STUB_KITS_FAIL="1")
        self.assertEqual(r.returncode, 1, out)
        self.assertIn("error: couldn't fetch the samples", out)
        self.assertIn("STOP: the free drum kits couldn't be made", out)
        self.assertNotIn("mpfree-creator/Cargo.toml", calls)

    def test_help_and_unknown_arguments(self):
        r, out, calls = self.run_script(["--help"])
        self.assertEqual(r.returncode, 0, out)
        self.assertIn("tools/build-usb-creator.sh", out)
        self.assertEqual(calls, "")
        r, out, _ = self.run_script(["--fast"])
        self.assertEqual(r.returncode, 2, out)


if __name__ == "__main__":
    unittest.main()
