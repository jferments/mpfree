#!/usr/bin/env python3
"""Tests for sd-card-round.py, the SD card round's two commands. No device: a stand-in program takes every
step's place (MPFREE_ROUND_FAKE) and says what the real one would, the MPC's two SSH ports are sockets on
this computer that open and close as the MPC would (Akai's system on one, the stick's system on the other),
and sudo and ssh are stand-ins on PATH. The results go to a temporary folder.

They check the order of the steps and what each is given, every stop (a stick that isn't MPFree's, partition
4 changed, a card that needs a repair, the card in the MPC under Akai's system, a boot-critical hash that
changed), resuming with --from, the hands-off round's variables, the clean power-off request, and the zip.
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
TOOL = os.path.join(TOOLS, "sd-card-round.py")
REPO = os.path.dirname(os.path.dirname(TOOLS))

HASHES = {p: f"{i:x}" * 64 for i, p in enumerate(("mmcblk0p1", "mmcblk0p2", "mmcblk0p3", "mmcblk0p4", "mmcblk0p5",
                                                   "mmcblk0boot0", "mmcblk0boot1", "mmcblk0p6"), start=1)}


def boot_map(hashes=None, card=False):
    hashes = dict(HASHES, **(hashes or {}))
    table = " 179        0    7636800 mmcblk0\n 179        8    2921775 mmcblk0p8\n"
    if card:
        table += " 179       96   62367744 mmcblk1\n 179       97   62351360 mmcblk1p1\n"
    return ("MPFREE BOOT/RECOVERY MAP - READ ONLY\n\n=== Kernel and command line ===\nLinux version 6.18.44\n\n"
            "=== Partition table (sysfs, names and geometry only) ===\n" + table +
            "\n=== Boot-critical partition hashes (read via dd, no output file) ===\n" +
            "".join(f"{p} {h}\n" for p, h in hashes.items()) +
            "\n=== Rockchip loader signatures (first 16 bytes only) ===\n...\n"
            "\nEND MPFREE BOOT/RECOVERY MAP (no writes were requested)\n")


INSPECT = ("partition 1: mpfree-test-root        444 MiB at 1 MiB\n"
           "partition 4: mpfree-test-data      58128 MiB at 1024 MiB\n"
           "made: MPFree stick, made by MPFree USB creator 0.1.0; developer SSH: on (port 2222, key only)\n"
           "partition 4: MPFREE-DATA ext4\nbackup: mpc-one-emmc-20261116T014409Z\n")

FAKE = r'''#!/usr/bin/env python3
import json, os, sys
name, cmd = sys.argv[1], sys.argv[2:]
d = os.environ["FAKE_DIR"]
env = {k: v for k, v in os.environ.items()
       if k == "MPFREE_ROUND" or (k.startswith("MPFREE_ACCEPT_") and k != "MPFREE_ACCEPT_ROUND") or k == "MPC_PORT"}
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write(json.dumps({"name": name, "cmd": cmd, "env": env}) + "\n")
def given(ext):
    p = os.path.join(d, f"{name}.{ext}")
    return open(p).read() if os.path.exists(p) else None
DEFAULTS = %(defaults)s
out = given("out")
sys.stdout.write(DEFAULTS.get(name, "") if out is None else out)
report = given("report")
if report is None and name.startswith("boot-map"):
    report = DEFAULTS["boot-map"]
if report is not None:
    with open(os.path.join(os.environ["MPC_REPORT_DIR"], f"mpc-one-boot-map-{name}.txt"), "w") as fh:
        fh.write(report)
if name == "power-off":
    open(os.path.join(d, "power-off.done"), "w").close()
sys.exit(int(given("rc") or 0))
'''

FAKE_SUDO = r'''#!/bin/sh
echo "sudo $*" >> "$FAKE_SUDO_LOG"
'''

FAKE_SSH = r'''#!/usr/bin/env python3
import json, os, sys
with open(os.environ["FAKE_SSH_LOG"], "w") as fh:
    json.dump({"args": sys.argv[1:], "stdin": sys.stdin.read()}, fh)
print("asked the stick's system for a clean power-off")
'''


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Mpc:
    """The MPC's two SSH ports on this computer. Before the stand-in power-off step: Akai's system answers in
    prepare, the stick's system in run. After it, Akai's system again (switched on without the stick)."""

    def __init__(self, fake_dir):
        self.fake_dir = fake_dir
        self.akai, self.stick = free_port(), free_port()
        self.mode = "prepare"
        self.mpfree_boots = True
        self.sockets = {}
        self.on = True
        self.thread = threading.Thread(target=self.loop, daemon=True)
        self.thread.start()

    def want(self):
        off = os.path.exists(os.path.join(self.fake_dir, "power-off.done"))
        if self.mode == "prepare" or off:
            return {self.akai}
        # A stick that doesn't start leaves Akai's system to come up instead.
        return {self.stick} if self.mpfree_boots else {self.akai}

    def loop(self):
        while self.on:
            want = self.want()
            for port in list(self.sockets):
                if port not in want:
                    self.sockets.pop(port).close()
            for port in want - set(self.sockets):
                s = socket.socket()
                s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                s.bind(("127.0.0.1", port))
                s.listen(16)
                s.setblocking(False)
                self.sockets[port] = s
            for s in self.sockets.values():
                try:
                    s.accept()[0].close()
                except OSError:
                    pass
            time.sleep(0.01)
        for s in self.sockets.values():
            s.close()

    def stop(self):
        self.on = False
        self.thread.join()


class RoundTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-round-test-")
        t = self.tmp
        self.results, self.work, self.research = (os.path.join(t, n) for n in ("results", "work", "research"))
        self.fake_dir, self.fakebin = os.path.join(t, "fake"), os.path.join(t, "bin")
        for d in (self.work, self.research, self.fake_dir, self.fakebin, os.path.join(self.work, "akai-update-files")):
            os.makedirs(d)
        self.key = os.path.join(t, "mpc_key")
        for path in (self.key, self.key + ".pub"):
            with open(path, "w") as fh:
                fh.write("key\n")
        old, new = (os.path.join(self.work, f"dev-tools-{n}") for n in ("old", "new"))
        os.makedirs(old)
        os.makedirs(new)
        os.utime(old, (1, 1))
        self.dev_tools = new
        self.fake = os.path.join(t, "fake-step")
        defaults = {"local-checks": "LOCAL-CHECKS-OK\n", "stick-before": INSPECT, "stick-after": INSPECT,
                    "write-stick": "done the stick is ready: put it in the MPC\n", "boot-map": boot_map()}
        self.write(self.fake, FAKE % {"defaults": repr(defaults)}, 0o755)
        self.write(os.path.join(self.fakebin, "sudo"), FAKE_SUDO, 0o755)
        self.write(os.path.join(self.fakebin, "ssh"), FAKE_SSH, 0o755)
        self.mpc = Mpc(self.fake_dir)
        self.env = {"PATH": self.fakebin + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"), "HOME": t,
                    "MPFREE_ACCEPT_ROUND": "YES", "MPFREE_ROUND_FAKE": self.fake, "FAKE_DIR": self.fake_dir,
                    "FAKE_LOG": os.path.join(t, "steps.log"), "FAKE_SUDO_LOG": os.path.join(t, "sudo.log"),
                    "FAKE_SSH_LOG": os.path.join(t, "ssh.log"), "MPFREE_RESULTS_DIR": self.results,
                    "MPFREE_WORK_DIR": self.work, "MPC_REPORT_DIR": self.research, "MPC_KEY": self.key,
                    "MPC_IP": "127.0.0.1", "MPC_PORT": str(self.mpc.akai), "MPC_TEST_PORT": str(self.mpc.stick),
                    "MPFREE_ROUND_POLL": "0.05", "MPFREE_ROUND_SETTLE": "0", "MPFREE_ROUND_BOOT_WAIT": "5",
                    "MPFREE_ROUND_OFF_WAIT": "5", "MPFREE_ROUND_WINDOW_QUIET": "0"}

    def tearDown(self):
        self.mpc.stop()
        shutil.rmtree(self.tmp, ignore_errors=True)

    def write(self, path, text, mode=0o644):
        with open(path, "w") as fh:
            fh.write(text)
        os.chmod(path, mode)

    def given(self, name, rc=None, out=None, report=None):
        for ext, value in (("rc", rc), ("out", out), ("report", report)):
            if value is not None:
                self.write(os.path.join(self.fake_dir, f"{name}.{ext}"), str(value))

    def tool(self, *args, **env):
        return subprocess.run([sys.executable, "-I", TOOL, *args], env=dict(self.env, **env), text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120, stdin=subprocess.DEVNULL)

    def steps(self):
        path = self.env["FAKE_LOG"]
        if not os.path.exists(path):
            return []
        with open(path) as fh:
            return [json.loads(line) for line in fh]

    def names(self):
        return [s["name"] for s in self.steps()]

    def step(self, name):
        return [s for s in self.steps() if s["name"] == name][-1]

    def current(self, *parts):
        return os.path.join(self.results, "current", *parts)

    def prepared(self, *args):
        res = self.tool("prepare", *args)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("PREPARE-OK", res.stdout)
        os.remove(self.env["FAKE_LOG"])
        self.mpc.mode = "run"
        return res

    # prepare

    def test_prepare_runs_every_step_in_order_with_one_sudo_password(self):
        self.given("card-check", rc=3)
        res = self.tool("prepare", "--repair-card")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.names(), ["local-checks", "build-arm", "build-helpers", "build-creator", "stick-before",
                                        "write-stick", "stick-after", "card-check", "card-repair", "boot-map-before"])
        # The local checks: every test file of the round's tools, then the disk test's own.
        checks = self.step("local-checks")["cmd"][2]
        for name in ("test_sd_card_round.py", "test_sd_card_write.py", "test_repair_sd_card.py",
                     "test_host_disktest_mpc.py", "--test disktest_sd_card", "echo LOCAL-CHECKS-OK"):
            self.assertIn(name, checks)
        # The stick: partition 4 kept, the newest developer tools, the round's key, recorded for the tools.
        write = self.step("write-stick")["cmd"]
        self.assertEqual(write[:3], ["sudo", "rust/target/release/mpfree-stick", "write"])
        for pair in (["--device", "mpfree"], ["--yes-erase", "mpfree"], ["--dev-tools", self.dev_tools],
                     ["--ssh-key", self.key + ".pub"], ["--record", os.path.realpath(self.work)]):
            self.assertIn(pair, [write[i:i + 2] for i in range(len(write) - 1)], write)
        self.assertIn("--keep-data", write)
        self.assertNotIn("--fresh-data", write)
        self.assertEqual(self.step("stick-before")["cmd"],
                         ["sudo", "rust/target/release/mpfree-stick", "inspect", "--device", "mpfree"])
        # The card: checked, then repaired without a typed word only because --repair-card asked.
        self.assertEqual(self.step("card-check")["cmd"], ["python3", "tools/mpc-one/repair-sd-card.py"])
        self.assertEqual(self.step("card-repair")["cmd"],
                         ["python3", "tools/mpc-one/repair-sd-card.py", "--repair", "--yes"])
        # Akai's system: the boot map on its port.
        self.assertEqual(self.step("boot-map-before")["cmd"], ["bash", "tools/mpc-one/collect-over-ssh.sh",
                                                               "--boot-map"])
        self.assertEqual(self.step("boot-map-before")["env"], {"MPC_PORT": str(self.mpc.akai)})
        with open(self.env["FAKE_SUDO_LOG"]) as fh:
            self.assertEqual(fh.read(), "sudo -v\n")
        for text in ("partition 4 and its backups: as before", "the card is repaired, clean and unmarked",
                     "NOTHING in its USB port or SD slot", "PREPARE-OK", "NEXT: run command 2"):
            self.assertIn(text, res.stdout)
        with open(self.current("round-state.txt")) as fh:
            state = fh.read()
        self.assertIn("prepare=ok\n", state)
        self.assertIn("done=boot-map\n", state)
        with open(self.current("round-prepare.txt")) as fh:
            self.assertIn("PREPARE-OK", fh.read())
        self.assertTrue(any(n.endswith("-boot-map-before--mpc-one-boot-map-boot-map-before.txt")
                            for n in os.listdir(self.current())))

    def test_a_stick_that_isnt_an_mpfree_stick_is_never_written(self):
        for out in (INSPECT.replace("developer SSH: on", "developer SSH: off"),
                    INSPECT.replace("MPFREE-DATA ext4", "MYDISK vfat")):
            with self.subTest(out=out[:40]):
                self.given("stick-before", out=out)
                res = self.tool("prepare")
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("STOP: the stick is not an MPFree stick", res.stdout)
                self.assertNotIn("write-stick", self.names())
                with open(self.current("round-prepare.txt")) as fh:
                    self.assertIn("STOP: the stick is not an MPFree stick", fh.read())

    def test_partition_4_changed_stops_the_round(self):
        self.given("stick-after", out=INSPECT.replace("backup: mpc-one-emmc-20261116T014409Z\n", ""))
        res = self.tool("prepare")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("doesn't show partition 4 and its backups as before", res.stdout)
        self.assertNotIn("card-check", self.names())

    def test_a_card_that_needs_a_repair_stops_and_from_card_carries_on(self):
        self.given("card-check", rc=3)
        res = self.tool("prepare")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("run prepare --from card --repair-card", res.stdout)
        self.assertNotIn("card-repair", self.names())
        os.remove(self.env["FAKE_LOG"])
        res = self.tool("prepare", "--from", "card", "--repair-card")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.names(), ["card-check", "card-repair", "boot-map-before"])
        # The same results folder: the first attempt's logs stay with this round.
        self.assertTrue(any(n.endswith("-write-stick.log") for n in os.listdir(self.current())))

    def test_from_needs_the_steps_before_at_this_commit(self):
        res = self.tool("prepare", "--from", "card")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("--from card: the steps before it ran at another commit", res.stdout)
        self.assertEqual(self.names(), [])

    def test_a_failed_repair_or_check_stops(self):
        self.given("card-check", rc=3)
        self.given("card-repair", rc=1)
        res = self.tool("prepare", "--repair-card")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the card repair didn't leave it clean and unmarked", res.stdout)
        self.assertNotIn("boot-map-before", self.names())

    def test_no_card_in_this_computer_is_skipped(self):
        self.given("card-check", rc=5)
        res = self.tool("prepare", "--repair-card")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("no SD card in this computer: skipped", res.stdout)
        self.assertNotIn("card-repair", self.names())

    def test_the_card_in_the_mpc_under_akais_system_stops_the_boot_map(self):
        self.given("boot-map-before", report=boot_map(card=True))
        res = self.tool("prepare", "--repair-card")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the SD card is in the MPC while Akai's system runs", res.stdout)
        self.assertIn("prepare --from card --repair-card", res.stdout)

    def test_a_boot_map_without_every_hash_stops(self):
        text = boot_map().replace(f"mmcblk0boot1 {HASHES['mmcblk0boot1']}\n", "mmcblk0boot1: no block device\n")
        self.given("boot-map-before", report=text)
        res = self.tool("prepare")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the boot map has no hash for mmcblk0boot1; then run prepare --from boot-map", res.stdout)

    def test_a_failed_build_changes_nothing(self):
        self.given("build-helpers", rc=101)
        res = self.tool("prepare")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the helpers didn't build", res.stdout)
        self.assertEqual(self.names(), ["local-checks", "build-arm", "build-helpers"])

    def test_earlier_files_go_into_their_own_zip_first(self):
        os.makedirs(self.current())
        self.write(self.current("01-old.log"), "old run\n")
        res = self.tool("prepare")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("Earlier files in the results folder, not this round's", res.stdout)
        zips = [n for n in os.listdir(self.results) if n.endswith(".zip")]
        self.assertEqual(len(zips), 1)
        self.assertNotIn("01-old.log", os.listdir(self.current()))

    def test_needs_the_opt_in_and_no_sudo(self):
        res = self.tool("prepare", MPFREE_ACCEPT_ROUND="")
        self.assertEqual(res.returncode, 1)
        self.assertIn("opt-in required", res.stdout)
        res = self.tool("run", MPFREE_ACCEPT_ROUND="")
        self.assertIn("MPFREE_ACCEPT_ROUND=YES python3 tools/mpc-one/sd-card-round.py run", res.stdout)
        self.assertEqual(self.names(), [])
        self.assertFalse(os.path.exists(self.env["FAKE_SUDO_LOG"]))

    # run

    def test_run_does_every_test_hands_off_and_zips_it_all(self):
        self.prepared()
        res = self.tool("run")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.names(), ["sd-card", "stream-read", "stream-recorder", "stream-app", "heat",
                                        "power-off", "boot-map-after"])
        sd = self.step("sd-card")
        self.assertEqual(sd["cmd"], ["python3", "tools/mpc-one/host_disktest_mpc.py", "--sd-card"])
        self.assertEqual(sd["env"], {"MPFREE_ACCEPT_DISKTEST": "YES", "MPFREE_ROUND": "1",
                                     "MPC_PORT": str(self.mpc.akai)})
        self.assertEqual(self.step("stream-read")["env"]["MPFREE_ACCEPT_DISKTEST"], "READ-ONLY")
        self.assertEqual(self.step("stream-app")["cmd"], ["python3", "tools/mpc-one/host_streamtest_mpc.py"])
        self.assertEqual(self.step("stream-app")["env"]["MPFREE_ACCEPT_STREAMTEST"], "YES")
        self.assertTrue(all(s["env"].get("MPFREE_ROUND") == "1" for s in self.steps()[:5]))
        self.assertEqual(self.step("power-off")["cmd"][1:], [TOOL, "power-off"])
        for text in ("PUT THE SD CARD IN THE MPC'S SLOT NOW", "the MPC is off",
                     "Take the stick AND the SD card out", "mmcblk0p1     same", "boot map: every boot-critical "
                     "hash as before", "sd-card: ran", "heat: ran", "ROUND-OK", "UPLOAD THIS ONE FILE"):
            self.assertIn(text, res.stdout)
        zips = [n for n in os.listdir(self.results) if n.endswith(".zip")]
        self.assertEqual(len(zips), 1)
        with zipfile.ZipFile(os.path.join(self.results, zips[0])) as zf:
            names = [n.split("/", 1)[1] for n in zf.namelist()]
            for want in ("round-prepare.txt", "round-run.txt", "round-state.txt", "MANIFEST.txt"):
                self.assertIn(want, names)
            for step in ("write-stick", "boot-map-before", "sd-card", "heat", "boot-map-after"):
                self.assertTrue(any(n.endswith(f"-{step}.log") for n in names), step)
            run_text = zf.read(next(n for n in zf.namelist() if n.endswith("round-run.txt"))).decode()
            self.assertIn("ROUND-OK", run_text)

    def test_sd_only_and_a_changed_hash_fail_the_round_but_still_zip(self):
        self.prepared()
        self.given("boot-map-after", report=boot_map({"mmcblk0p2": "f" * 64}))
        self.given("sd-card", rc=1)
        res = self.tool("run", "--sd-only")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertEqual(self.names(), ["sd-card", "power-off", "boot-map-after"])
        for text in ("mmcblk0p2     DIFFERENT", "boot map: CHANGED: mmcblk0p2", "sd-card: exit code 1",
                     "ROUND-NOT-OK", "UPLOAD THIS ONE FILE"):
            self.assertIn(text, res.stdout)

    def test_a_failed_sd_card_test_lets_the_windows_close_before_the_stick_tests(self):
        self.prepared()
        self.given("sd-card", rc=1)
        res = self.tool("run")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("letting the SD card's windows close by themselves first", res.stdout)
        self.assertEqual(self.names()[:2], ["sd-card", "stream-read"])

    def test_mpfree_not_answering_runs_nothing_on_it_but_takes_the_boot_map(self):
        self.prepared()
        # The stick doesn't start, so Akai's system answers from the start (no clock to race).
        self.mpc.mpfree_boots = False
        res = self.tool("run", MPFREE_ROUND_BOOT_WAIT="0.5")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("MPFree didn't answer: nothing was run on the MPC.", res.stdout)
        self.assertIn("mpfree: did not answer", res.stdout)
        self.assertEqual(self.names(), ["boot-map-after"])

    def test_a_card_left_in_for_the_last_boot_map_is_noted(self):
        self.prepared()
        self.given("boot-map-after", report=boot_map(card=True))
        res = self.tool("run", "--sd-only")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("note: the SD card was in the MPC under Akai's system", res.stdout)

    def test_run_needs_prepare_at_this_commit(self):
        res = self.tool("run")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("command 1 (prepare) hasn't finished", res.stdout)
        self.prepared()
        with open(self.current("round-state.txt"), "a") as fh:
            fh.write("commit=0000000000000000000000000000000000000000\n")
        res = self.tool("run")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("this clone is at another commit", res.stdout)
        self.assertEqual(self.names(), [])

    # The power-off request.

    def test_power_off_asks_the_sticks_system_over_its_own_ssh(self):
        folder = os.path.join(self.work, "creator-stick-20261124T050000Z")
        os.makedirs(folder)
        self.write(os.path.join(folder, "report.txt"), "report\n")
        self.write(os.path.join(folder, "known_hosts"), "mpfree-kexec-test ssh-ed25519 AAAA\n")
        res = self.tool("power-off")
        self.assertEqual(res.returncode, 0, res.stdout)
        with open(self.env["FAKE_SSH_LOG"]) as fh:
            ssh = json.load(fh)
        args = ssh["args"]
        self.assertIn(["-p", str(self.mpc.stick)], [args[i:i + 2] for i in range(len(args) - 1)])
        for opt in (f"UserKnownHostsFile={folder}/known_hosts", "StrictHostKeyChecking=yes", "BatchMode=yes"):
            self.assertIn(opt, args)
        self.assertEqual(args[-2:], ["root@127.0.0.1", "sh -s"])
        self.assertTrue(ssh["stdin"].startswith("R=''\n"))
        # The script itself, on a stand-in root: one word, written whole, nothing else.
        busybox = shutil.which("busybox") or shutil.which("sh")
        root = os.path.join(self.tmp, "root")
        os.makedirs(os.path.join(root, "run/mpfree-app"))
        script = ssh["stdin"].replace("R=''", f"R='{root}'", 1)
        res = subprocess.run([busybox, "sh"] if busybox.endswith("busybox") else [busybox], input=script, text=True,
                             capture_output=True)
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(os.listdir(os.path.join(root, "run/mpfree-app")), ["power-request"])
        with open(os.path.join(root, "run/mpfree-app/power-request")) as fh:
            self.assertEqual(fh.read(), "poweroff\n")
        # Not the stick's system: nothing written.
        shutil.rmtree(os.path.join(root, "run"))
        res = subprocess.run([busybox, "sh"] if busybox.endswith("busybox") else [busybox], input=script, text=True,
                             capture_output=True)
        self.assertEqual(res.returncode, 1)
        self.assertFalse(os.path.exists(os.path.join(root, "run")))


if __name__ == "__main__":
    unittest.main(verbosity=2)
