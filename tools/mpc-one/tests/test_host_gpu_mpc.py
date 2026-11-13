#!/usr/bin/env python3
"""Tests for host_gpu_mpc.py against a fake test build. No device, no GPU.

As in test_host_bench_mpc.py, a fake ssh runs the tool's real remote scripts
under BusyBox applets against a fake root folder (setting the scripts' R=''
root prefix to it), accepts only the test build's host key alias, and only
while the fake MPC is "on the test build". Running the benchmark is the one
other change: a shell stand-in logs its arguments and prints what the real
one would.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import hashlib
import importlib.util
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
HELPER = os.path.join(TOOLS, "host_gpu_mpc.py")
_spec = importlib.util.spec_from_file_location("test_app_mpc", os.path.join(HERE, "test_app_mpc.py"))
th = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(th)
_spec = importlib.util.spec_from_file_location("host_gpu_mpc", HELPER)
gpu = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gpu)

FAKE_SSH = th.FAKE_SSH.replace(
    "program = 'exec \"$R/run/mpfree-app/mpfree\"'\n"
    "if program in script:\n"
    "    script = script.replace(program, 'exec \"$FAKE_PROGRAM\"')",
    "program = 'exec \"$R/run/mpfree-gpu/mpfree-gpu-bench\"'\n"
    "if program in script:\n"
    "    script = script.replace(program, 'exec \"$FAKE_PROGRAM\"')")
assert FAKE_SSH != th.FAKE_SSH, "the fake ssh's program line moved in test_app_mpc.py"

FAKE_GPU_BENCH = r'''#!/bin/sh
echo "bench: $*" >> "$FAKE_LOG"
[ -x ./mpfree-gpu-bench ] || { echo "STOP: not copied"; exit 1; }
case "$1" in
  describe) echo "gpu: $FAKE_RENDERER; OpenGL ES 3.1 Mesa 24.0.7; EGL 1.4 (Mesa Project)"
            echo "gpu: OpenGL ES 3.1 (compute): yes" ;;
  compute) echo "compute: 128 frames x 64 taps: gpu p50 400 p99 900 max 1200; cpu (one core) p50 60" ;;
  screens) n=$(ls pages | grep -c '\.mpdl$'); echo "screens: $n pages, 30 frames each timed"
           echo "summary: gpu against software: at most 2 levels apart"; exit "$FAKE_SCREENS_RC" ;;
  panel) n=$(ls pages | grep -c '\.mpdl$'); echo "panel: $n pages drawn on the GPU and shown 1500 ms each"
         echo "panel: the framebuffer holds what was written: yes"
         echo "summary: panel showed $n pages from the GPU"; exit "$FAKE_PANEL_RC" ;;
  flip) n=$(ls pages | grep -c '\.mpdl$'); echo "flip: /dev/dri/card0: connector 33 on CRTC 31, mode 800x1280 at 60 Hz"
        [ "$FAKE_FLIP_RC" = 2 ] && { echo "STOP: the test card is not in the screen buffer"; exit 2; }
        echo "flip: SCREEN CHANGES NOW: $n pages drawn on the GPU straight into the buffers the panel shows, 1500 ms each"
        echo "summary: flip gpu whole page: whole p50 9.10"
        [ "$FAKE_FLIP_BACK" = 1 ] && echo "flip: the screen is back as it was (CRTC 31 showing framebuffer 40); 200 page flips in all"
        exit "$FAKE_FLIP_RC" ;;
esac
'''


def dynamic_elf(loader=b"/lib/ld-linux-armhf.so.3", needed=(b"libgcc_s.so.1", b"libc.so.6"),
                glibc=b"GLIBC_2.34", static=False):
    """A small ARM EABI5 hard-float PIE: PT_LOAD, PT_INTERP and PT_DYNAMIC with DT_NEEDED and DT_STRTAB."""
    base, interp_at, str_at, dyn_at = 0x10000, 160, 200, 320
    strtab = b"\0"
    offs = []
    for n in needed:
        offs.append(len(strtab))
        strtab += n + b"\0"
    strtab += glibc + b"\0"
    entries = [(1, o) for o in offs] + [(5, base + str_at), (0, 0)]
    dyn = b"".join(struct.pack("<iI", t, v) for t, v in entries)
    size = 4096
    data = bytearray(size)
    data[:7] = b"\x7fELF\x01\x01\x01"
    heads = [(1, 0, base, base, size, size)]
    if not static:
        heads += [(3, interp_at, base + interp_at, base + interp_at, len(loader) + 1, len(loader) + 1),
                  (2, dyn_at, base + dyn_at, base + dyn_at, len(dyn), len(dyn))]
    struct.pack_into("<HHI", data, 16, 3, 40, 1)
    struct.pack_into("<I", data, 28, 52)
    struct.pack_into("<I", data, 36, 0x05000400)
    struct.pack_into("<HHH", data, 40, 52, 32, len(heads))
    for i, h in enumerate(heads):
        struct.pack_into("<IIIIII", data, 52 + 32 * i, *h)
    data[interp_at:interp_at + len(loader)] = loader
    data[str_at:str_at + len(strtab)] = strtab
    data[dyn_at:dyn_at + len(dyn)] = dyn
    return bytes(data)


class ElfTests(unittest.TestCase):
    def test_takes_only_a_program_for_the_mpcs_c_library(self):
        self.assertIsNone(gpu.arm_dynamic_problem(dynamic_elf()))
        self.assertIn("statically linked", gpu.arm_dynamic_problem(dynamic_elf(static=True)))
        self.assertIn("not for the loader", gpu.arm_dynamic_problem(dynamic_elf(loader=b"/lib/ld-linux.so.3")))
        self.assertIn("linked against libEGL.so.1",
                      gpu.arm_dynamic_problem(dynamic_elf(needed=(b"libc.so.6", b"libEGL.so.1"))))
        self.assertIn("GLIBC_2.38", gpu.arm_dynamic_problem(dynamic_elf(glibc=b"GLIBC_2.38")))
        self.assertIn("not an ARM executable", gpu.arm_dynamic_problem(th.arm_elf(machine=62)))
        self.assertIn("not EABI5 hard-float", gpu.arm_dynamic_problem(th.arm_elf(flags=0x05000200)))
        self.assertIn("plausible size", gpu.arm_dynamic_problem(b"\x7fELF"))


class GpuBenchTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-gpu-test-")
        t = self.tmp
        self.work, self.log, self.stage = os.path.join(t, "work"), os.path.join(t, "log"), os.path.join(t, "stage")
        self.root, self.report = os.path.join(t, "root310"), os.path.join(t, "report.txt")
        th.write(self.stage, "310\n")
        th.write(self.log, "")
        fakebin, remote = os.path.join(t, "fakebin"), os.path.join(t, "remote-bin")
        th.write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        os.makedirs(remote)
        for name in th.APPLETS:
            os.symlink(self.busybox, os.path.join(remote, name))
        th.write(os.path.join(remote, "systemctl"), th.FAKE_SYSTEMCTL, 0o755)
        screen, app = os.path.join(t, "screen-state"), os.path.join(t, "app-state")
        th.write(screen, "inactive\n")
        th.write(app, "active\n")
        standin = os.path.join(t, "bench-standin")
        th.write(standin, FAKE_GPU_BENCH, 0o755)
        self.env = {"PATH": fakebin + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"), "HOME": t,
                    "MPFREE_APP_TEST": "1", "MPFREE_ACCEPT_GPU_BENCH": "YES", "FAKE_STAGE": self.stage,
                    "FAKE_LOG": self.log, "FAKE_ROOT": self.root, "FAKE_REMOTE_PATH": remote, "FAKE_SCREEN": screen,
                    "FAKE_APP": app, "FAKE_ACVS": "inactive", "FAKE_PROGRAM": standin,
                    "FAKE_RENDERER": "Mali-T760 (Panfrost)", "FAKE_SCREENS_RC": "0", "FAKE_PANEL_RC": "0",
                    "FAKE_FLIP_RC": "0", "FAKE_FLIP_BACK": "1"}
        self.key = os.path.join(t, "key")
        th.write(self.key, "fake key\n", 0o600)
        self.program = os.path.join(t, "build", "mpfree-gpu-bench")
        th.write(self.program, dynamic_elf(), 0o755)
        self.pages = os.path.join(t, "pages")
        for name in ("play", "sample-wave", "synth-fm"):
            th.write(os.path.join(self.pages, f"{name}.mpdl"), b"MPDL" + os.urandom(3000))
        folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        th.write(os.path.join(folder, "stick.img"), b"\x00" * 4096)
        th.write(os.path.join(folder, "known_hosts"), "mpfree-kexec-test ssh-ed25519 AAAAfake\n")
        th.write(os.path.join(folder, "report.txt"), f"its kernel release: {th.NEW}\n")
        r = self.root
        th.write(f"{r}/proc/sys/kernel/osrelease", th.NEW + "\n")
        th.write(f"{r}/proc/sys/kernel/kexec_load_disabled", "1\n")
        os.makedirs(f"{r}/sys/block/sda")
        th.write(f"{r}/proc/mounts", "/dev/root / ext4 ro,relatime 0 0\ntmpfs /run tmpfs rw,nosuid,nodev,mode=755 0 0\n")
        os.makedirs(f"{r}/run")
        for f in ("dev/dri/renderD128", "dev/dri/card0", "lib/ld-linux-armhf.so.3", "lib/libc.so.6", "lib/libgcc_s.so.1",
                  "usr/lib/libEGL.so.1", "usr/lib/libGLESv2.so.2"):
            th.write(os.path.join(r, f), "")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_helper(self, answers="\n" * 5, pages=True, extra=(), **env):
        cmd = [sys.executable, "-I", HELPER, "--ip", "192.0.2.141", "--key", self.key, "--work", self.work,
               "--report", self.report, "--program", self.program, *extra]
        if pages:
            cmd += ["--pages", self.pages]
        return subprocess.run(cmd, input=answers, env=dict(self.env, **env), text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def lines(self, prefix):
        with open(self.log) as fh:
            return [ln[len(prefix):].strip() for ln in fh.read().splitlines() if ln.startswith(prefix)]

    def test_copies_the_benchmark_and_pages_and_runs_only_the_fixed_arguments(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("links only the C library, glibc <= 2.34", "pages: 3 in", "CHECKS-OK",
                     "render node: /dev/dri/renderD128", "found: /usr/lib/libEGL.so.1", "GPU-OK",
                     "copied the benchmark and 3 pages; every hash on the MPC matches",
                     "gpu: Mali-T760 (Panfrost)", "compute: 128 frames x 64 taps", "screens: 3 pages",
                     "at most 2 levels apart", "panel: 3 pages drawn on the GPU",
                     "the framebuffer holds what was written: yes", "summary: panel showed 3 pages",
                     "the screen keeps the last page", "pad player: active (started again)",
                     "RESULT: the GPU benchmark ran"):
            self.assertIn(text, res.stdout)
        self.assertEqual(self.lines("bench: "), ["describe", "compute --iterations 500",
                                                 "screens pages --frames 30",
                                                 "panel pages --hold-ms 1500 --frames 60"])
        # Services: only our own pad player is stopped for the runs and started again after.
        changes = [c for c in self.lines("systemctl: ") if not c.startswith("is-active ")]
        self.assertEqual(changes, ["stop mpfree-test-app.service", "start mpfree-test-app.service"])
        d = f"{self.root}/run/mpfree-gpu"
        with open(self.program, "rb") as a, open(f"{d}/mpfree-gpu-bench", "rb") as b:
            self.assertEqual(hashlib.sha256(a.read()).digest(), hashlib.sha256(b.read()).digest())
        self.assertEqual(os.stat(f"{d}/mpfree-gpu-bench").st_mode & 0o777, 0o700)
        self.assertEqual(sorted(os.listdir(f"{d}/pages")), ["play.mpdl", "sample-wave.mpdl", "synth-fm.mpdl"])
        self.assertEqual(sorted(os.listdir(f"{self.root}/run")), ["mpfree-gpu"])
        with open(self.report) as fh:
            self.assertIn("RESULT: the GPU benchmark ran", fh.read())

    def test_needs_the_opt_in_and_refuses_stock(self):
        res = self.run_helper(MPFREE_ACCEPT_GPU_BENCH="")
        self.assertEqual(res.returncode, 1)
        self.assertIn("opt-in required", res.stdout)
        th.write(self.stage, "36\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the test build did not answer", res.stdout)
        self.assertEqual(self.lines("bench: "), [])
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-gpu"))

    def test_stops_before_copying_without_the_gpu_or_its_libraries(self):
        os.remove(f"{self.root}/usr/lib/libEGL.so.1")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1)
        self.assertIn("PROBLEM: missing libEGL.so.1", res.stdout)
        self.assertIn("the GPU or its libraries are not there", res.stdout)
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-gpu"))
        self.assertEqual([c for c in self.lines("systemctl: ") if not c.startswith("is-active ")], [])

    def test_stops_on_a_bad_program_or_pages_before_any_ssh(self):
        th.write(self.program, dynamic_elf(static=True), 0o755)
        res = self.run_helper()
        self.assertEqual(res.returncode, 1)
        self.assertIn("statically linked", res.stdout)
        self.assertEqual(self.lines("ssh["), [])
        th.write(self.program, dynamic_elf(), 0o755)
        th.write(os.path.join(self.pages, "Bad Name.mpdl"), b"MPDL")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1)
        self.assertIn("not a page file this tool copies", res.stdout)
        self.assertEqual(self.lines("ssh["), [])

    def test_stops_when_mesa_answers_without_the_mali(self):
        res = self.run_helper(FAKE_RENDERER="llvmpipe (LLVM 17.0.6, 128 bits)")
        self.assertEqual(res.returncode, 1)
        self.assertIn("Mesa answered without the Mali", res.stdout)
        self.assertEqual(self.lines("bench: "), ["describe"])
        # The pad player is started again.
        changes = [c for c in self.lines("systemctl: ") if not c.startswith("is-active ")]
        self.assertEqual(changes, ["stop mpfree-test-app.service", "start mpfree-test-app.service"])

    def test_a_failed_run_still_starts_the_pad_player_again(self):
        res = self.run_helper(FAKE_SCREENS_RC="3")
        self.assertEqual(res.returncode, 1)
        self.assertIn("screens ended with exit code 3", res.stdout)
        self.assertIn("(started again)", res.stdout)
        self.assertNotIn("panel pages", self.lines("bench: "))

    def test_the_screen_step_can_be_skipped_or_fail_and_the_log_display_comes_back(self):
        th.write(os.path.join(self.tmp, "screen-state"), "active\n")
        res = self.run_helper(answers="\n\n\ns\n")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.lines("bench: ")[-1], "screens pages --frames 30")
        res = self.run_helper(FAKE_PANEL_RC="1")
        self.assertEqual(res.returncode, 1)
        self.assertIn("panel ended with exit code 1", res.stdout)
        self.assertNotIn("the screen keeps the last page", res.stdout)
        changes = [c for c in self.lines("systemctl: ") if not c.startswith("is-active ")]
        self.assertIn("start mpfree-test-screen.service", changes)
        self.assertEqual(changes[-1:], ["start mpfree-test-screen.service"])

    def test_the_flip_step_runs_only_with_the_flag_and_with_the_display_free(self):
        res = self.run_helper(extra=["--flip"], answers="\n" * 6)
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("[7/7] The GPU drawing straight into the panel's own buffers", "display: /dev/dri/card0",
                     "CARD-FREE", "STATE-CHANGING, RAM only", "SCREEN CHANGES NOW",
                     "flip: the screen is back as it was", "pad player: active (started again)",
                     "RESULT: the GPU benchmark ran"):
            self.assertIn(text, res.stdout)
        self.assertEqual(self.lines("bench: ")[-2:], ["panel pages --hold-ms 1500 --frames 60",
                                                      "flip pages --hold-ms 1500 --frames 60"])
        # Without the flag: six steps, no flip.
        res = self.run_helper(answers="\n" * 6)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertNotIn("[7/", res.stdout)
        self.assertEqual(self.lines("bench: ").count("flip pages --hold-ms 1500 --frames 60"), 1)
        # Skipped at its prompt: it does not run.
        res = self.run_helper(extra=["--flip"], answers="\n" * 4 + "s\n")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.lines("bench: ").count("flip pages --hold-ms 1500 --frames 60"), 1)

    def test_the_flip_step_is_refused_while_another_program_has_the_display(self):
        os.makedirs(f"{self.root}/proc/4242/fd")
        th.write(f"{self.root}/proc/4242/comm", "compositor\n")
        os.symlink("/dev/dri/card0", f"{self.root}/proc/4242/fd/7")
        res = self.run_helper(extra=["--flip"], answers="\n" * 6)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("PROBLEM: /dev/dri/card0 is open in pid 4242 (compositor)", res.stdout)
        self.assertIn("another program has it open; the screen was not changed", res.stdout)
        self.assertNotIn("flip pages --hold-ms 1500 --frames 60", self.lines("bench: "))
        self.assertIn("pad player: active (started again)", res.stdout)

    def test_a_flip_that_fails_says_whether_the_screen_changed_and_how_to_undo(self):
        res = self.run_helper(extra=["--flip"], answers="\n" * 6, FAKE_FLIP_RC="2", FAKE_FLIP_BACK="0")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("flip ended with exit code 2 (before the screen changed)", res.stdout)
        self.assertNotIn("did not say the screen is back", res.stdout)
        res = self.run_helper(extra=["--flip"], answers="\n" * 6, FAKE_FLIP_RC="1", FAKE_FLIP_BACK="0")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("The program did not say the screen is back", res.stdout)
        self.assertIn("switch the MPC off with its power switch and on again", res.stdout)
        self.assertIn("flip ended with exit code 1", res.stdout)
        self.assertIn("pad player: active (started again)", res.stdout)

    def test_q_stops_before_the_copy(self):
        res = self.run_helper(answers="q\n")
        self.assertEqual(res.returncode, 1)
        self.assertIn("STOP: stopped by you", res.stdout)
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-gpu"))
        self.assertEqual([c for c in self.lines("systemctl: ") if not c.startswith("is-active ")], [])

    def test_makes_the_pages_with_the_workstations_app(self):
        fake_app = os.path.join(self.tmp, "host", "mpfree")
        th.write(fake_app, "#!/bin/sh\n[ \"$1\" = --display-lists ] || exit 2\nmkdir -p \"$2\"\n"
                           "printf 'MPDLxxxx' > \"$2/play.mpdl\"\necho \"display lists written: 1 pages in $2\"\n", 0o755)
        cmd = [sys.executable, "-I", HELPER, "--ip", "192.0.2.141", "--key", self.key, "--work", self.work,
               "--report", self.report, "--program", self.program, "--host-app"]
        res = subprocess.run(cmd + [os.path.join(self.tmp, "no-such-app")], input="q\n", env=self.env, text=True,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)
        self.assertEqual(res.returncode, 1)
        self.assertIn("build the app for this workstation first", res.stdout)
        self.assertEqual(self.lines("ssh["), [])
        res = subprocess.run(cmd + [fake_app], input="q\n", env=self.env, text=True, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, timeout=120)
        self.assertIn("making the pages:", res.stdout)
        self.assertIn("pages: 1 in", res.stdout)
        made = [d for d in os.listdir(self.work) if d.startswith("gpu-pages-")]
        self.assertEqual(len(made), 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
