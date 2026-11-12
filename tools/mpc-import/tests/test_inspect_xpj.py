#!/usr/bin/env python3
"""Tests for inspect-xpj.py on made-up projects shaped like the public notes
(our own values; no Akai files)."""
import gzip
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "inspect-xpj.py")
BLOB = "SECRETSETTINGSBLOB" * 4


def mixable(dest=2, effects=()):
    return {"volume": 0.7, "pan": 0.5, "mute": False, "solo": False,
            "audioRoute": {"destination": dest, "output": "Out 1/2"},
            "sends": [{"volume": 0.0}, {"volume": 0.25}, {"volume": 0.0}, {"volume": 0.0}],
            "inserts": {"effects": [{"bypass": i == 1, "plugin": {"description": {"name": n, "UID": 1000 + i},
                                                                  "state": BLOB}} for i, n in enumerate(effects)]}}


def layer(name):
    return {"active": True, "sampleName": name, "sampleStart": 0, "sampleEnd": 100, "loop": False,
            "velocityStart": 0.0, "velocityEnd": 1.0}


def instrument(layers=(), group=0):
    return {"whichMuteGroup": group, "polyphony": 1, "triggerMode": 0, "coarseTune": 0, "fineTune": 0,
            "layersv": [layer(x) for x in layers] + [{"active": False, "sampleName": ""}],
            "synthSection": {"ampEnvelope": {"Attack": {"value0": 0.0}}, "filterData": {"value0": {}}},
            "mixable": mixable()}


def note(t, pitch, prob=100, mod0=0.5):
    n = {"note": pitch, "velocity": 0.8, "length": 240, "probability": prob, "ratchet": 1}
    for k in range(16):
        n[f"modifierValue{k}"] = mod0 if k == 0 else 0.0
        n[f"modifierActiveState{k}"] = k == 0
    return {"time": t, "type": 3, "muted": False, "invented": False, "note": n}


def project():
    insts = [instrument() for _ in range(128)]
    insts[0] = instrument(["kick"], group=1)
    insts[1] = instrument(["snare", "snare soft"], group=1)
    insts[2] = instrument(["gone"])
    drum = {"name": "Beat Kit", "type": 0, "mixable": mixable(effects=["Reverb Test", "Delay Test"]),
            "drum": {"instruments": insts, "padNoteMap": {"noteForPad": {"value0": 36}}}}
    events = [note(0, 36), note(240, 37, prob=50), note(480, 36, mod0=0.75),
              {"time": 0, "type": 2, "muted": False, "invented": False,
               "automation": {"note": 36, "value": 0.3, "parameter": 518}}]
    seq = {"name": "Seq A", "bpm": 92.5, "lengthBars": 2, "lengthPulses": 7680, "loop": True,
           "timeSignatureTrack": {"timeSignatures": [{"beatsPerBar": 4, "beatLength": 960, "barStart": 0}]},
           "trackClipMaps": [[{"key": "Beat", "value": {"startPulses": 0, "endPulses": 7680, "loopStartPulses": 0,
                                                       "loopEndPulses": 7680, "eventList": {"events": events}}}]]}
    return {"formatVersion": 2, "data": {
        "version": 99, "masterTempo": 120.0, "masterTempoEnabled": False, "key": "C Major",
        "tracks": [{"name": "Beat", "volume": 1.0, "pan": 0.5, "mute": False, "program": drum,
                    "samples": [{"name": "kick", "path": "kick.wav"}]},
                   {"name": "Submix 1", "program": {"type": 8, "mixable": mixable(effects=["Comp Test"])}}],
        "sequences": [{"key": 0, "value": seq}],
        "songs": [{"name": "Song 1", "items": [{"sequence": 0}]}, {"name": "Song 2", "items": []}],
        "samples": [{"name": "snare", "path": "snare.wav"}],
        "mixer": {}}}


def xpj(doc, head=b"HDR\n9.9\nObj\njson\nOS\n"):
    return gzip.compress(head + json.dumps(doc).encode())


class InspectTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_tool(self, *args):
        return subprocess.run([sys.executable, TOOL, *args], capture_output=True, text=True, timeout=60)

    def write(self, rel, data):
        p = os.path.join(self.tmp, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "wb") as fh:
            fh.write(data)
        return p

    def test_summary_from_a_zip(self):
        z = os.path.join(self.tmp, "mpc-projects.zip")
        with zipfile.ZipFile(z, "w") as zf:
            zf.writestr("MPC Documents/Beat.xpj", xpj(project()))
            for w in ("Kick.WAV", "snare.wav", "sub/snare soft.wav"):
                zf.writestr(f"MPC Documents/Beat_[ProjectData]/{w}", b"RIFF")
        res = self.run_tool(z, "--shape")
        out = res.stdout
        self.assertEqual(res.returncode, 0, out + res.stderr)
        self.assertIn("format: MPC 3 (gzip + JSON)", out)
        self.assertIn("header: HDR", out)
        self.assertIn("data.version 99", out)
        self.assertIn("[Drum] program 'Beat Kit'", out)
        self.assertIn("pads with samples: 3 of 128, layers 4, mute groups [1]", out)
        self.assertIn("insert: Reverb Test (UID 1000) [settings blob 72 chars]", out)
        self.assertIn("insert: Delay Test (UID 1001) BYPASSED", out)
        self.assertIn("Submix 1 [Submix] inserts: Comp Test", out)
        self.assertIn("'Seq A' bpm 92.5, bars 2, pulses 7680", out)
        self.assertIn("events {'note': 3, 'pad automation': 1}, probability<100 on 1, modifiers {'Tuning (coarse)': 1}", out)
        self.assertIn("automation parameter ids seen: {518: 1}", out)
        self.assertIn("songs: 2 slots, 1 with items", out)
        self.assertIn("ProjectData: 3 files, 3 WAV; samples named in the project but not found: 1", out)
        self.assertIn("missing: gone", out)
        self.assertIn("46 of 46 fields as expected", out)
        self.assertIn(".data.tracks[].program.drum.instruments[].layersv[].sampleName: str x", out)
        self.assertNotIn("SECRETSETTINGSBLOB", out)

    def test_checklist_flags_missing_and_changed_fields(self):
        doc = project()
        doc["data"]["masterTempo"] = "fast"
        del doc["data"]["sequences"][0]["value"]["bpm"]
        p = self.write("x/Beat.xpj", xpj(doc))
        out = self.run_tool(p).stdout
        self.assertIn("TYPE     data.masterTempo: str, expected number", out)
        self.assertIn("MISSING  data.sequences[].value.bpm", out)
        self.assertIn("ProjectData folder: not found next to this .xpj", out)

    def test_other_formats_and_bad_files(self):
        self.write("f/old.xpj", b'<?xml version="1.0"?><MPCVObject><Version><File_Version>1.7</File_Version></Version></MPCVObject>')
        self.write("f/junk.xpj", b"\x00\x01junk")
        self.write("f/broken.xpj", gzip.compress(b"HDR\n{not json"))
        res = self.run_tool(os.path.join(self.tmp, "f"))
        self.assertEqual(res.returncode, 1)
        self.assertIn("format: MPC 2 (XML)", res.stdout)
        self.assertIn("header: File_Version: 1.7", res.stdout)
        self.assertIn("STOP: not a project this tool knows: first bytes 00 01", res.stdout)
        self.assertIn("STOP: gzip, but the JSON does not parse", res.stdout)
        os.makedirs(os.path.join(self.tmp, "empty"))
        res = self.run_tool(os.path.join(self.tmp, "empty"))
        self.assertIn("no .xpj here", res.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
