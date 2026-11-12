#!/usr/bin/env python3
"""Inspect MPC project files (.xpj) for the MPC importer: what they hold, and
whether the fields the importer will read are where the public notes say.

  python3 tools/mpc-import/inspect-xpj.py PATH [PATH ...] [--shape] [--out FILE]

PATH is a .xpj file, a folder (every .xpj under it), or a .zip such as the
mpc-projects.zip that tools/mpc-one/offline-mpc-projects.py writes.

For each project it prints:
- the format: MPC 3 (gzip, five header lines, then JSON) or MPC 2 (XML);
- a summary: tempo, tracks and their programs, pads and layers, inserts,
  sequences with their clips, notes and automation, songs, samples (and
  whether each sample's WAV is in the ProjectData folder);
- an importer checklist: each field the importer plans to read, present or
  missing, and whether its type matches the public notes for OS 3.7;
- with --shape, every field path in the file with its types and counts
  (list indices folded to [], so the schema is easy to compare between OS
  versions).

It only reads. It never prints an effect's or plugin's settings blob (only its
length), and never prints audio. Everything here about Akai's format comes from
community notes (docs/project/mpc-import.md), not from Akai: treat the type
names and route codes it shows as unconfirmed.
"""
import argparse
import gzip
import io
import json
import os
import re
import sys
import zipfile

PD_SUFFIX = "_[ProjectData]"
MAX_XPJ = 256 << 20

# Program types, per the public notes (unconfirmed for OS 3.6).
PROGRAM_TYPES = {0: "Drum", 1: "Keygroup", 3: "Plugin", 4: "MIDI", 5: "CV", 6: "Audio",
                 7: "Return", 8: "Submix", 9: "Output", 10: "Input"}
EVENT_TYPES = {1: "track automation", 2: "pad automation", 3: "note", 4: "audio"}
MODIFIER_SLOTS = ["Tuning (coarse)", "Tuning (fine)", "Filter", "Layer", "Attack", "Decay", "Slice/Pad"]
MODIFIER_DEFAULTS = [0.5, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0] + [0.0] * 9

# What the importer's stages 1 to 3 plan to read: (path, expected JSON type).
# Paths use [] for "every item of a list"; the first item that has the field is checked.
CHECKLIST = [
    ("data.version", "int"),
    ("data.masterTempo", "number"),
    ("data.masterTempoEnabled", "bool"),
    ("data.tracks", "list"),
    ("data.tracks[].name", "str"),
    ("data.tracks[].volume", "number"),
    ("data.tracks[].pan", "number"),
    ("data.tracks[].mute", "bool"),
    ("data.tracks[].program.type", "int"),
    ("data.tracks[].program.mixable.volume", "number"),
    ("data.tracks[].program.mixable.pan", "number"),
    ("data.tracks[].program.mixable.audioRoute.destination", "int"),
    ("data.tracks[].program.mixable.sends", "list"),
    ("data.tracks[].program.mixable.inserts.effects", "list"),
    ("data.tracks[].program.drum.instruments", "list"),
    ("data.tracks[].program.drum.instruments[].whichMuteGroup", "int"),
    ("data.tracks[].program.drum.instruments[].polyphony", "int"),
    ("data.tracks[].program.drum.instruments[].triggerMode", "int"),
    ("data.tracks[].program.drum.instruments[].coarseTune", "int"),
    ("data.tracks[].program.drum.instruments[].fineTune", "int"),
    ("data.tracks[].program.drum.instruments[].layersv", "list"),
    ("data.tracks[].program.drum.instruments[].layersv[].sampleName", "str"),
    ("data.tracks[].program.drum.instruments[].layersv[].sampleStart", "int"),
    ("data.tracks[].program.drum.instruments[].layersv[].sampleEnd", "int"),
    ("data.tracks[].program.drum.instruments[].layersv[].loop", "bool"),
    ("data.tracks[].program.drum.instruments[].layersv[].velocityStart", "number"),
    ("data.tracks[].program.drum.instruments[].layersv[].velocityEnd", "number"),
    ("data.tracks[].program.drum.instruments[].synthSection.ampEnvelope", "dict"),
    ("data.tracks[].program.drum.instruments[].synthSection.filterData", "dict"),
    ("data.tracks[].program.drum.instruments[].mixable.volume", "number"),
    ("data.tracks[].program.drum.instruments[].mixable.pan", "number"),
    ("data.tracks[].program.drum.padNoteMap", "dict"),
    ("data.tracks[].samples", "list"),
    ("data.tracks[].samples[].name", "str"),
    ("data.tracks[].samples[].path", "str"),
    ("data.sequences", "list"),
    ("data.sequences[].key", "int"),
    ("data.sequences[].value.name", "str"),
    ("data.sequences[].value.bpm", "number"),
    ("data.sequences[].value.lengthBars", "int"),
    ("data.sequences[].value.lengthPulses", "int"),
    ("data.sequences[].value.timeSignatureTrack.timeSignatures", "list"),
    ("data.sequences[].value.trackClipMaps", "list"),
    ("data.songs", "list"),
    ("data.samples", "list"),
    ("data.mixer", "dict"),
]


class Bad(Exception):
    pass


def jtype(v):
    if isinstance(v, bool):
        return "bool"
    if isinstance(v, int):
        return "int"
    if isinstance(v, float):
        return "float"
    if isinstance(v, str):
        return "str"
    if isinstance(v, list):
        return "list"
    if isinstance(v, dict):
        return "dict"
    return "null" if v is None else type(v).__name__


def type_ok(v, want):
    t = jtype(v)
    return t == want or (want == "number" and t in ("int", "float"))


def printable(s, n=80):
    s = str(s)
    return "".join(c if 32 <= ord(c) < 127 or ord(c) > 159 else "?" for c in s)[:n]


# ------------------------------------------------------------------ reading

def parse_xpj(raw):
    """(format, header lines, data or None, note)."""
    if raw[:2] == b"\x1f\x8b":
        try:
            text = gzip.decompress(raw).decode("utf-8", "replace")
        except (OSError, EOFError) as err:
            raise Bad(f"gzip, but it would not unpack: {err}")
        head, body = [], text
        for _ in range(5):
            line, sep, rest = body.partition("\n")
            if not sep or line.lstrip().startswith("{"):
                break
            head.append(line.rstrip("\r"))
            body = rest
        start = body.find("{")
        if start < 0:
            raise Bad("gzip, but no JSON after the header lines")
        try:
            doc = json.loads(body[start:])
        except json.JSONDecodeError as err:
            raise Bad(f"gzip, but the JSON does not parse: {err}")
        return "MPC 3 (gzip + JSON)", head, doc, None
    if raw.lstrip()[:5] == b"<?xml":
        text = raw[:1 << 20].decode("utf-8", "replace")
        tags = re.findall(r"<(File_Version|Application|Application_Version|Platform)>([^<]{0,60})</", text)
        return "MPC 2 (XML)", [f"{k}: {printable(v)}" for k, v in tags], None, \
            "MPC 2 projects are listed only; their parser comes later"
    raise Bad("not a project this tool knows: first bytes " + raw[:16].hex(" "))


def sources(paths):
    """Yield (label, xpj bytes, set of ProjectData file names relative to the folder, or None)."""
    for path in paths:
        if os.path.isdir(path):
            found = []
            for d, _, fs in os.walk(path):
                found += [os.path.join(d, f) for f in fs if f.lower().endswith(".xpj")]
            if not found:
                yield path, None, None
            for f in sorted(found):
                yield from sources([f])
        elif path.lower().endswith(".zip"):
            with zipfile.ZipFile(path) as z:
                names = z.namelist()
                xpjs = sorted(n for n in names if n.lower().endswith(".xpj"))
                if not xpjs:
                    yield path, None, None
                for n in xpjs:
                    if z.getinfo(n).file_size > MAX_XPJ:
                        yield f"{path}:{n}", b"", None
                        continue
                    pd = n[:-4] + PD_SUFFIX + "/"
                    files = {x[len(pd):] for x in names if x.startswith(pd) and not x.endswith("/")}
                    yield f"{path}:{n}", z.read(n), files if any(x.startswith(pd) for x in names) else None
        else:
            if os.path.getsize(path) > MAX_XPJ:
                yield path, b"", None
                continue
            with open(path, "rb") as fh:
                raw = fh.read()
            pd = path[:-4] + PD_SUFFIX
            files = None
            if os.path.isdir(pd):
                files = {os.path.relpath(os.path.join(d, f), pd).replace(os.sep, "/")
                         for d, _, fs in os.walk(pd) for f in fs}
            yield path, raw, files


# ------------------------------------------------------------------ walking the JSON

def items(v):
    """A list, or an indexed dict {value0: .., value1: ..} as a list."""
    if isinstance(v, list):
        return v
    if isinstance(v, dict) and v and all(re.fullmatch(r"value\d+", k) for k in v):
        return [v[k] for k in sorted(v, key=lambda k: int(k[5:]))]
    return []


def lookup(root, path):
    """The first value at a [] path, or (False, None)."""
    parts = re.findall(r"[^.\[\]]+|\[\]", path)

    def go(v, i):
        if i == len(parts):
            return True, v
        p = parts[i]
        if p == "[]":
            for x in items(v):
                ok, got = go(x, i + 1)
                if ok:
                    return ok, got
            return False, None
        if isinstance(v, dict) and p in v:
            return go(v[p], i + 1)
        return False, None

    return go(root, 0)


def shape(v, path, out):
    t = jtype(v)
    seen = out.setdefault(path, {})
    seen[t] = seen.get(t, 0) + 1
    if isinstance(v, dict):
        for k, x in v.items():
            k2 = "valueN" if re.fullmatch(r"value\d+", k) else ("modifier*N" if re.fullmatch(r"modifier\w+?\d+", k) else k)
            shape(x, f"{path}.{k2}", out)
    elif isinstance(v, list):
        for x in v:
            shape(x, path + "[]", out)


def g(d, *keys, default=None):
    for k in keys:
        if not isinstance(d, dict) or k not in d:
            return default
        d = d[k]
    return d


def unwrap(v):
    """{key, value} pairs as used by sequences, or the item itself."""
    if isinstance(v, dict) and "value" in v and "key" in v:
        return v["key"], v["value"]
    return None, v


# ------------------------------------------------------------------ summary

def effects_line(mixable):
    out = []
    for e in items(g(mixable, "inserts", "effects", default=[])):
        name = g(e, "plugin", "description", "name") or g(e, "name") or "?"
        uid = g(e, "plugin", "description", "UID")
        state = g(e, "plugin", "state", default=None)
        if state is None:
            state = g(e, "state", default="")
        out.append(f"{printable(name, 40)}" + (f" (UID {uid})" if uid is not None else "")
                   + (" BYPASSED" if g(e, "bypass") else "")
                   + (f" [settings blob {len(state)} chars]" if isinstance(state, str) and state else ""))
    return out


def mix_line(m):
    if not isinstance(m, dict):
        return "no mixable"
    sends = [g(s, "volume") for s in items(g(m, "sends", default=[]))]
    vol = g(m, "volume")
    if isinstance(vol, dict):
        vol = vol.get("controlValue", vol.get("gainCoefficient"))
    bits = [f"vol {vol:.3f}" if isinstance(vol, (int, float)) else f"vol {vol!r}",
            f"pan {g(m, 'pan'):.3f}" if isinstance(g(m, "pan"), (int, float)) else f"pan {g(m, 'pan')!r}",
            f"route dest {g(m, 'audioRoute', 'destination')!r} out {printable(g(m, 'audioRoute', 'output'), 20)!r}"]
    if any(isinstance(s, (int, float)) and s > 0 for s in sends):
        bits.append("sends " + "/".join(f"{s:.2f}" if isinstance(s, (int, float)) else "?" for s in sends))
    if g(m, "mute"):
        bits.append("MUTE")
    if g(m, "solo"):
        bits.append("SOLO")
    return ", ".join(bits)


def summarize(doc, pd_files, say):
    data = doc.get("data") if isinstance(doc, dict) else None
    if not isinstance(data, dict):
        say("  no 'data' object at the top level: the layout differs from the public notes")
        return
    say(f"  formatVersion {doc.get('formatVersion')!r}, data.version {data.get('version')!r}, "
        f"saved by {data.get('lastSavedProductIdentifier')!r}, created by {data.get('originalCreatorProductIdentifier')!r}")
    say(f"  master tempo {data.get('masterTempo')!r} (enabled {data.get('masterTempoEnabled')!r}), key {data.get('key')!r}, "
        f"engineMode {data.get('engineMode')!r}")

    tracks = items(data.get("tracks"))
    say(f"\n  tracks: {len(tracks)}")
    wanted = set()
    for i, t in enumerate(tracks):
        prog = g(t, "program", default={})
        ptype = g(prog, "type")
        tname = PROGRAM_TYPES.get(ptype, f"type {ptype!r}")
        if ptype in (7, 8, 9):
            fx = effects_line(g(prog, "mixable"))
            if fx or g(prog, "mixable", "mute"):
                say(f"   {i:3d}. {printable(g(t, 'name'), 30)} [{tname}] inserts: {', '.join(fx) or 'none'}")
            continue
        say(f"   {i:3d}. {printable(g(t, 'name'), 40)} [{tname}] program {printable(g(prog, 'name'), 40)!r}"
            + (" MUTED" if g(t, "mute") else ""))
        say(f"        track vol {g(t, 'volume')!r} pan {g(t, 'pan')!r}; program mix: {mix_line(g(prog, 'mixable'))}")
        for fx in effects_line(g(prog, "mixable")):
            say(f"        insert: {fx}")
        if ptype == 3:
            desc = g(prog, "plugin", "description", default={})
            say(f"        plugin: {printable(g(desc, 'name'))!r} by {printable(g(desc, 'manufacturer'))!r}"
                f" (settings: {'blob' if g(prog, 'plugin', 'state') else 'none found'})")
        insts = items(g(prog, "drum", "instruments", default=[]))
        if insts:
            pads = layers = 0
            groups, poly, fxpads = set(), {}, 0
            for inst in insts:
                lay = [x for x in items(g(inst, "layersv", default=[])) if g(x, "sampleName")]
                if lay:
                    pads += 1
                    layers += len(lay)
                    wanted.update(g(x, "sampleName") for x in lay)
                    mg = g(inst, "whichMuteGroup")
                    if mg:
                        groups.add(mg)
                    p = g(inst, "polyphony")
                    poly[p] = poly.get(p, 0) + 1
                    if effects_line(g(inst, "mixable")):
                        fxpads += 1
            say(f"        pads with samples: {pads} of {len(insts)}, layers {layers}, mute groups {sorted(groups) or 'none'}, "
                f"polyphony {dict(sorted(poly.items(), key=str))}, pads with inserts {fxpads}")
            if g(prog, "keygroup") is not None:
                say(f"        keygroup fields: {', '.join(sorted(g(prog, 'keygroup').keys())[:12]) if isinstance(g(prog, 'keygroup'), dict) else '?'}")
        pool = items(g(t, "samples", default=[]))
        if pool:
            say(f"        sample pool: {len(pool)}")
            wanted.update(g(s, "path") or g(s, "name") for s in pool)

    seqs = items(data.get("sequences"))
    say(f"\n  sequences: {len(seqs)}")
    auto_params = {}
    for s in seqs:
        key, v = unwrap(s)
        sigs = [(g(x, "beatsPerBar"), g(x, "beatLength"), g(x, "barStart"))
                for x in items(g(v, "timeSignatureTrack", "timeSignatures", default=[]))]
        say(f"   {key!r}: {printable(g(v, 'name'), 40)!r} bpm {g(v, 'bpm')!r}, bars {g(v, 'lengthBars')!r}, "
            f"pulses {g(v, 'lengthPulses')!r}, loop {g(v, 'loop')!r}, time sigs {sigs}")
        for lane in items(g(v, "trackClipMaps", default=[])):
            for entry in items(lane):
                cname, clip = unwrap(entry)
                evs = items(g(clip, "eventList", "events", default=[]))
                kinds = {}
                prob = ratch = invented = 0
                mods = {}
                for e in evs:
                    et = g(e, "type")
                    kinds[EVENT_TYPES.get(et, f"type {et!r}")] = kinds.get(EVENT_TYPES.get(et, f"type {et!r}"), 0) + 1
                    if g(e, "invented"):
                        invented += 1
                    n = g(e, "note")
                    if isinstance(n, dict):
                        if g(n, "probability") not in (None, 100):
                            prob += 1
                        if g(n, "ratchet") not in (None, 1):
                            ratch += 1
                        for k in range(16):
                            val = g(n, f"modifierValue{k}")
                            if g(n, f"modifierActiveState{k}") and isinstance(val, (int, float)) \
                                    and abs(val - MODIFIER_DEFAULTS[k]) > 1e-6:
                                label = MODIFIER_SLOTS[k] if k < len(MODIFIER_SLOTS) else f"slot {k}"
                                mods[label] = mods.get(label, 0) + 1
                    a = g(e, "automation")
                    if isinstance(a, dict):
                        auto_params[g(a, "parameter")] = auto_params.get(g(a, "parameter"), 0) + 1
                if evs:
                    say(f"      clip {printable(cname, 30)!r}: start {g(clip, 'startPulses')!r} end {g(clip, 'endPulses')!r}"
                        f" loop {g(clip, 'loopStartPulses')!r}-{g(clip, 'loopEndPulses')!r}; events {kinds}"
                        + (f", invented {invented}" if invented else "")
                        + (f", probability<100 on {prob}" if prob else "")
                        + (f", ratchets on {ratch}" if ratch else "")
                        + (f", modifiers {mods}" if mods else ""))
    if auto_params:
        say(f"   automation parameter ids seen: {dict(sorted(auto_params.items(), key=lambda kv: str(kv[0])))}")

    songs = items(data.get("songs"))
    used = [s for s in songs if items(g(s, "items", default=[]))]
    say(f"\n  songs: {len(songs)} slots, {len(used)} with items"
        + "".join(f"\n   {printable(g(s, 'name'))!r}: {len(items(g(s, 'items')))} items" for s in used))
    arr = sum(1 for t in tracks if items(g(t, "arrangementClipMap", default=[])))
    say(f"  tracks with arrangement clips: {arr}")

    pool = items(data.get("samples"))
    say(f"\n  project sample pool: {len(pool)}")
    wanted.update(g(s, "path") or g(s, "name") for s in pool)
    wanted.discard(None)
    if pd_files is None:
        say("  ProjectData folder: not found next to this .xpj")
    else:
        wavs = sorted(f for f in pd_files if f.lower().endswith(".wav"))
        base = {os.path.basename(f).lower() for f in pd_files}
        stem = {re.sub(r"\.wav$", "", f, flags=re.I) for f in base}
        names = {re.sub(r"\.wav$", "", os.path.basename(str(w)), flags=re.I) for w in wanted}
        missing = sorted((n for n in names if n.lower() not in stem), key=str.lower)
        say(f"  ProjectData: {len(pd_files)} files, {len(wavs)} WAV; samples named in the project but not found: {len(missing)}"
            + "".join(f"\n   missing: {printable(m)}" for m in missing[:20]))


def checklist(doc, say):
    say("\n  importer checklist (fields stages 1 to 3 will read; expected types from the public 3.7 notes):")
    bad = 0
    for path, want in CHECKLIST:
        ok, v = lookup(doc, path)
        if not ok:
            say(f"   MISSING  {path}")
            bad += 1
        elif not type_ok(v, want):
            say(f"   TYPE     {path}: {jtype(v)}, expected {want}")
            bad += 1
    say(f"   {len(CHECKLIST) - bad} of {len(CHECKLIST)} fields as expected"
        + (" (a missing one may just be unused here, e.g. no drum track)" if bad else ""))


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("paths", nargs="+")
    p.add_argument("--shape", action="store_true", help="also list every field path with its types and counts")
    p.add_argument("--out", help="also write the report to this file")
    args = p.parse_args(argv)
    lines = []

    def say(text=""):
        print(text, flush=True)
        lines.append(text)

    status = 0
    for label, raw, pd_files in sources(args.paths):
        say(f"=== {label}")
        if raw is None:
            say("  no .xpj here")
            status = 1
            continue
        if not raw:
            say(f"  skipped: larger than {MAX_XPJ >> 20} MiB")
            status = 1
            continue
        try:
            fmt, head, doc, note = parse_xpj(raw)
        except Bad as err:
            say(f"  STOP: {err}")
            status = 1
            continue
        say(f"  format: {fmt}; {len(raw)} bytes")
        for h in head:
            say(f"  header: {printable(h)}")
        if note:
            say(f"  {note}")
            continue
        summarize(doc, pd_files, say)
        checklist(doc, say)
        if args.shape:
            out = {}
            shape(doc, "", out)
            say(f"\n  shape: {len(out)} field paths")
            for path in sorted(out):
                say(f"   {path or '(root)'}: " + ", ".join(f"{t} x{n}" for t, n in sorted(out[path].items())))
        say()
    if args.out:
        with open(args.out, "w") as fh:
            fh.write("\n".join(lines) + "\n")
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
