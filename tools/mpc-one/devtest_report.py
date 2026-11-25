#!/usr/bin/env python3
"""WORKSTATION ONLY: what the device round's reports show, in one page. Reads files; contacts nothing.

  python3 tools/mpc-one/devtest_report.py LOG [LOG ...] [--out FILE]

Give it the logs results.py kept for the round (~/mpfree-results/current/
NN-NAME.log) or the tools' own --report files: host_bench_mpc.py's,
host_devtest_mpc.py's and host_streamtest_mpc.py's, in any order. It finds
in them:

- the workflows (host_devtest_mpc.py): each run's exit code, the steps the
  app could not do (a control not on the page, a button with no number), the
  expects and expect-nots checked against the run's log (on the MPC, from the
  step's mark to the next mark: the app's threads write their lines a little
  after the step that caused them), and for every mark's part of a run its
  underruns, periods, slow periods, late parts on the sound cores, and the
  sampler's and synth's render means over that part (from the status lines
  the app writes at each mark); where takes play from their files (the
  stream test, D3), the takes on files and the seconds that went silent
  because the drive was late, over each part (any silence fails the run),
  and each drive's read and sync times at the run's end (the SD card plan's
  B2a);
  the screen's frames; your listening answer;
- every project open the runs made (live shows A4): how long the whole open
  took and each part of it (the library's read, the build with the plugins'
  starts, the swap into the audio loop, the kits, loops and takes), the
  sound files read and their time, and the memory once it was in; and each
  set's song change, from Go (or Play set) to the next song in;
- heavy-load on each sound-core setting side by side, with the setting the
  numbers favour;
- the benchmarks (host_bench_mpc.py): the NEON keep-or-drop A/B (every
  engine's p99 with the NEON kernels and without), the --afford rows the
  device round asked about (Wind, Keys, Drawbar, FM, Choir, grains) with
  each engine's cost per voice and the voices that fit, the grain budget
  line with GRAIN_BUDGET and VOICE_COST to set, and the presets whose note
  starts go over the synth budget.

It prints Markdown; --out also writes it to a file. Exit code 0 when every
workflow ran with every check met and no underrun, 1 otherwise, 2 when it
found nothing to read.
"""
import argparse
import json
import re
import sys

RUN_START = re.compile(r"^----- workflow (\S+) \(sound cores: (\S+)\) -----$")
RUN_END = re.compile(r"^----- end of workflow (\S+) \(sound cores: (\S+)\): exit code (-?\d+) -----$")
STEP = re.compile(r"^script: line (\d+) at (\d+\.\d+) s: (\S+)(?: (.*))?$")
NOT_DONE = re.compile(r"^script: line (\d+): (no control .*|.* has no button number; not pressed)$")
STATUS = re.compile(r"^status: (\d+) hits .*?, (\d+) underruns in (\d+) periods, at most (\d+) sounds at once")
SAMPLER = re.compile(r"sampler: render mean (\d+) us, max (\d+) us")
SYNTH = re.compile(r"^status: synth: .*?render mean (\d+) us, max (\d+) us")
CORES = re.compile(r"^status: sound cores (\S+): ")
LATE = re.compile(r"(\d+) late")
SLOW = re.compile(r"^control: slow period at ")
SCREEN = re.compile(r"^screen: (\d+) frames, mean ([\d.]+) ms, slowest ([\d.]+) ms")
# The audio tracks' status line while takes play from their files (the stream test's workflows, D3).
FILES = re.compile(r"^status: audio tracks: .*; from their files: (\d+) of (\d+) takes, ([\d.]+) s went silent, "
                   r"(\d+) of (\d+) region starts held")
JSON_START = re.compile(r"^----- (\S+)\.json -----$")
JSON_END = re.compile(r"^----- end of (\S+)\.json -----$")
BUILD = re.compile(r"^\[\d+/\d+\] The (\S+) build")
AFFORD_ROW = re.compile(r"^(.{36}) +(-?\d+\.\d) +(-|\d+\.\d\d) +(\S.*?) +(\d+\.\d) +(\d+) us +(.+)$")
GRAIN_COST = re.compile(r"^Grain budget \(R7 step A6\), .*: a grain ([\d.]+) us, a looping sample voice ([\d.]+) us "
                        r"\(about ([\d.]+) grains; the engine's VOICE_COST is (\d+)\)")
GRAIN_FIT = re.compile(r"^grain budget: (\d+) grains fit .*\(GRAIN_BUDGET is (\d+) now\)")
OVER = "over the synth budget"
ASKED = ("wind", "keys", "tine", "drawbar", "organ", "fm", "choir", "grain")
LISTEN = re.compile(r"^your answer: (.*)$")
# The open's own line (open_clock.rs) and a set's song in (gui.rs), live shows A4.
OPENED = re.compile(r"^project: (.+?) opened in (\d+\.\d+) s: (.*)$")
OPEN_READ = re.compile(r"^read in (\d+\.\d+) s")
OPEN_BUILT = re.compile(r"built in (\d+\.\d+) s(?: \((\d+) plugins? started in (\d+\.\d+) s\))?")
OPEN_AT = re.compile(r"(in the audio loop|kits|loops|takes) at (\d+\.\d+) s")
OPEN_FILES = re.compile(r"(\d+) sound files? \((\d+\.\d) MB\) read in (\d+\.\d+) s")
OPEN_MEMORY = re.compile(r"memory (\d+) MB in use(?: \(peak (\d+) MB\))?(?:, (\d+) MB free)?")
SONG_IN = re.compile(r"^show: (.+?) is in \((\w+)\), (\d+\.\d+) s after (Go|Play set)$")
# What Soundcheck kept for each song, and how it went (gui.rs, live shows B2b).
SOUNDCHECK_KEPT = re.compile(r"^show: soundcheck kept song (\d+) \((.*?)\): (.*)$")
SOUNDCHECK_OVER = re.compile(r"^show: soundcheck of (.+?) (done|stopped): (.*)$")
NEON_MARGIN = 0.02


class Run:
    def __init__(self, workflow, cores):
        self.workflow, self.cores = workflow, cores
        self.lines = []
        self.rc = None

    def label(self):
        return self.workflow if self.cores == "default" else f"{self.workflow} on {self.cores}"


def read(paths):
    """Every report line, without the tools' indents, file after file."""
    lines = []
    for path in paths:
        with open(path, encoding="utf-8", errors="replace") as fh:
            lines.extend(ln.rstrip("\n").strip() for ln in fh)
    return lines


def runs_in(lines):
    runs, run = [], None
    for ln in lines:
        m = RUN_START.match(ln)
        if m:
            run = Run(*m.groups())
            runs.append(run)
            continue
        m = RUN_END.match(ln)
        if m and run:
            run.rc = int(m.group(3))
            run = None
            continue
        if run:
            run.lines.append(ln)
    return runs


def totals(block):
    """(underruns, periods, sampler mean, synth mean, late, sampler max, synth max) of one status block."""
    t = {"underruns": None, "periods": None, "sampler": None, "synth": None, "late": 0,
         "sampler_max": None, "synth_max": None, "silent": None, "on_file": None}
    for ln in block:
        m = FILES.match(ln)
        if m:
            t["silent"] = float(m.group(3))
            t["on_file"] = f"{m.group(1)} of {m.group(2)}"
        m = STATUS.match(ln)
        if m:
            t["underruns"], t["periods"] = int(m.group(2)), int(m.group(3))
            s = SAMPLER.search(ln)
            if s:
                t["sampler"], t["sampler_max"] = int(s.group(1)), int(s.group(2))
        m = SYNTH.match(ln)
        if m:
            t["synth"], t["synth_max"] = int(m.group(1)), int(m.group(2))
        if CORES.match(ln):
            t["late"] = sum(int(x) for x in LATE.findall(ln))
    return t


def status_after(lines, i):
    """The status block that starts first after line i (the app writes one at each mark and at the end)."""
    j = i
    while j < len(lines) and not STATUS.match(lines[j]):
        j += 1
    if j == len(lines):
        return None
    k = j + 1
    while k < len(lines) and lines[k].startswith("status: ") and not STATUS.match(lines[k]):
        k += 1
    return totals(lines[j:k])


def window_mean(a, b, key):
    """A render mean over the part between two status blocks, from their running means."""
    if not a or not b or a[key] is None or b[key] is None or b["periods"] == a["periods"]:
        return None
    return (b[key] * b["periods"] - a[key] * a["periods"]) / (b["periods"] - a["periods"])


def analyse(run):
    """A run's parts, failed steps and checks."""
    lines = run.lines
    marks = [(i, m.group(4) or "") for i, ln in enumerate(lines)
             for m in [STEP.match(ln)] if m and m.group(3) == "mark"]
    done = next((i for i, ln in enumerate(lines) if ln == "script: done"), len(lines))
    bounds = [i for i, _ in marks] + [done]
    parts = []
    for n, (i, name) in enumerate(marks):
        a, b = status_after(lines, i), status_after(lines, bounds[n + 1])
        inside = lines[i:bounds[n + 1]]
        part = {"name": name, "slow": sum(1 for ln in inside if SLOW.match(ln))}
        if a and b and a["underruns"] is not None and b["underruns"] is not None:
            part.update(underruns=b["underruns"] - a["underruns"], periods=b["periods"] - a["periods"],
                        late=b["late"] - a["late"], sampler=window_mean(a, b, "sampler"),
                        synth=window_mean(a, b, "synth"))
        if a and b and a["silent"] is not None and b["silent"] is not None:
            part.update(silent=b["silent"] - a["silent"], on_file=b["on_file"])
        parts.append(part)
    failed = [f"line {m.group(1)}: {m.group(2)}" for ln in lines for m in [NOT_DONE.match(ln)] if m]
    checks = []
    for i, ln in enumerate(lines):
        m = STEP.match(ln)
        if not m or m.group(3) not in ("expect", "expect-not", "expect-on-mpc"):
            continue
        start = max([b for b, _ in marks if b < i], default=0)
        end = min([b for b, _ in marks if b > i] + [len(lines)])
        text = m.group(4) or ""
        # Not the script's own lines, and not the status block a mark prints: its totals say "0 underruns" and
        # the like, so `expect-not underrun` would always be seen (the underruns column counts them instead).
        # The smoke session checks the same lines: what the control side and the screen said.
        seen = any(text in x for x in lines[start:end] if not x.startswith(("script: ", "status: ")))
        want = m.group(3) != "expect-not"
        checks.append((int(m.group(1)), m.group(3), text, seen == want))
    end = status_after(lines, done) or status_after(lines, 0)
    screen = next((m.groups() for ln in lines for m in [SCREEN.match(ln)] if m), None)
    return {"parts": parts, "failed": failed, "checks": checks, "end": end, "screen": screen,
            "done": done < len(lines)}


def drive_timings(lines):
    """Each drive's read and sync times from the run's last audio tracks status line that has them, or None."""
    for ln in reversed(lines):
        if FILES.match(ln):
            rest = ln.split("region starts held", 1)[1].lstrip("; ")
            if rest:
                return rest
    return None


def fmt(v, digits=0):
    if v is None:
        return "-"
    return f"{v:.{digits}f}"


def workflow_section(runs, out, answer):
    bad = False
    out.append("## Workflows\n")
    streams = any(p.get("silent") is not None for run in runs for p in analyse(run)["parts"])
    out.append("| workflow | exit | ran to its end | checks met | steps not done | underruns | slow periods | late parts |"
               + (" went silent (s) |" if streams else ""))
    out.append("|---|---|---|---|---|---|---|---|" + ("---|" if streams else ""))
    found = []
    for run in runs:
        a = analyse(run)
        found.append((run, a))
        met = sum(1 for c in a["checks"] if c[3])
        under = sum(p.get("underruns", 0) or 0 for p in a["parts"])
        slow = sum(p["slow"] for p in a["parts"])
        late = sum(p.get("late", 0) or 0 for p in a["parts"])
        silent = [p["silent"] for p in a["parts"] if p.get("silent") is not None]
        ok = (run.rc == 0 and a["done"] and met == len(a["checks"]) and not a["failed"] and under == 0
              and sum(silent) == 0)
        bad |= not ok
        out.append(f"| {run.label()} | {run.rc if run.rc is not None else 'cut off'} | {'yes' if a['done'] else 'NO'} "
                   f"| {met} of {len(a['checks'])} | {len(a['failed'])} | {under} | {slow} | {late} |"
                   + (f" {fmt(sum(silent), 2) if silent else '-'} |" if streams else ""))
    out.append("")
    if answer is not None:
        out.append(f"Your listening answer: {answer or '(nothing wrong)'}\n")
    for run, a in found:
        misses = [c for c in a["checks"] if not c[3]]
        if not (misses or a["failed"]):
            continue
        out.append(f"### {run.label()}: what did not happen\n")
        for line, verb, text, _ in misses:
            out.append(f"- line {line}: {verb} {text!r} " + ("was seen" if verb == "expect-not" else "never showed"))
        for f in a["failed"]:
            out.append(f"- {f}")
        out.append("")
    out.append("## Each part of each workflow\n")
    out.append("Underruns, periods and late parts are counted over the part; render means are the part's own "
               "(µs a period); slow periods are the app's `control: slow period` lines.\n")
    if streams:
        out.append("Where takes play from their files (the stream test's workflows), takes on files is the count at "
                   "the part's end and went silent the seconds of takes that played as silence in it because the "
                   "drive was late.\n")
    for run, a in found:
        files = any(p.get("silent") is not None for p in a["parts"])
        out.append(f"### {run.label()}\n")
        out.append("| part | underruns | periods | slow periods | late parts | sampler render µs | synth render µs |"
                   + (" takes on files | went silent (s) |" if files else ""))
        out.append("|---|---|---|---|---|---|---|" + ("---|---|" if files else ""))
        for p in a["parts"]:
            out.append(f"| {p['name']} | {fmt(p.get('underruns'))} | {fmt(p.get('periods'))} | {p['slow']} "
                       f"| {fmt(p.get('late'))} | {fmt(p.get('sampler'))} | {fmt(p.get('synth'))} |"
                       + (f" {p.get('on_file') or '-'} | {fmt(p.get('silent'), 2)} |" if files else ""))
        timings = drive_timings(run.lines)
        if timings:
            out.append(f"Drive timings at the run's end: {timings}.\n")
        e = a["end"]
        if e and e.get("sampler_max") is not None:
            out.append(f"\nWorst period of the run: sampler {e['sampler_max']} µs, synth {fmt(e.get('synth_max'))} µs.")
        if a["screen"]:
            out.append(f"Screen: {a['screen'][0]} frames, mean {a['screen'][1]} ms, slowest {a['screen'][2]} ms.")
        out.append("")
    return found, bad


def open_line(ln):
    """One open's numbers from its line, or None."""
    m = OPENED.match(ln)
    if not m:
        return None
    name, total, rest = m.group(1), float(m.group(2)), m.group(3)
    o = {"song": name, "total": total, "read": None, "built": None, "plugins": 0, "plugins_s": None,
         "in the audio loop": None, "kits": None, "loops": None, "takes": None, "files": 0, "mb": None,
         "files_s": None, "in_use": None, "peak": None, "free": None}
    r = OPEN_READ.match(rest)
    if r:
        o["read"] = float(r.group(1))
    b = OPEN_BUILT.search(rest)
    if b:
        o["built"] = float(b.group(1))
        if b.group(2):
            o["plugins"], o["plugins_s"] = int(b.group(2)), float(b.group(3))
    for part, at in OPEN_AT.findall(rest):
        o[part] = float(at)
    f = OPEN_FILES.search(rest)
    if f:
        o["files"], o["mb"], o["files_s"] = int(f.group(1)), float(f.group(2)), float(f.group(3))
    mem = OPEN_MEMORY.search(rest)
    if mem:
        o["in_use"], o["peak"], o["free"] = (int(x) if x else None for x in mem.groups())
    return o


def opens_section(runs, out):
    """Every open and set song change the workflows made (live shows A4), in the order they came."""
    opens = [(run, o) for run in runs for ln in run.lines for o in [open_line(ln)] if o]
    songs = [(run, m.groups()) for run in runs for ln in run.lines for m in [SONG_IN.match(ln)] if m]
    if not opens and not songs:
        return
    out.append("## Opens and song changes\n")
    out.append("Each open from its own line: the whole open (the library's read, then the last part in), the "
               "read, the build (with the plugins' starts and their time), then when each part was in, counted "
               "from the read's end; the sound files the kits and loops read and the time in the reads alone; "
               "the memory once it was in. The round's library is in RAM on the MPC, so the reads are RAM's, "
               "not the stick's.\n")
    if opens:
        out.append("| workflow | song | whole open s | read s | built s | plugins (s) | in the audio loop s | kits s "
                   "| loops s | takes s | sound files (MB, s) | in use MB | peak MB | free MB |")
        out.append("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
        for run, o in opens:
            plugins = f"{o['plugins']} ({fmt(o['plugins_s'], 2)})" if o["plugins"] else "0"
            files = f"{o['files']} ({fmt(o['mb'], 1)}, {fmt(o['files_s'], 2)})" if o["files"] else "0"
            out.append(f"| {run.label()} | {o['song']} | {fmt(o['total'], 2)} | {fmt(o['read'], 2)} "
                       f"| {fmt(o['built'], 2)} | {plugins} | {fmt(o['in the audio loop'], 2)} | {fmt(o['kits'], 2)} "
                       f"| {fmt(o['loops'], 2)} | {fmt(o['takes'], 2)} | {files} | {fmt(o['in_use'])} "
                       f"| {fmt(o['peak'])} | {fmt(o['free'])} |")
        slowest = max(opens, key=lambda ro: ro[1]["total"])
        peak = max((o["peak"] for _, o in opens if o["peak"] is not None), default=None)
        out.append(f"\nSlowest open: {slowest[1]['song']} in {slowest[1]['total']:.2f} s ({slowest[0].label()})."
                   + (f" Highest peak memory: {peak} MB." if peak is not None else ""))
        out.append("")
    if songs:
        out.append("A set's song change, from Go (the song's ending, its recovery copy, the next song's open) or "
                   "from Play set for the first song, to the song in:\n")
        out.append("| workflow | song | starts | s after | from |")
        out.append("|---|---|---|---|---|")
        for run, (song, starts, secs, frm) in songs:
            out.append(f"| {run.label()} | {song} | {starts} | {float(secs):.2f} | {frm} |")
        out.append("")


def soundcheck_section(runs, out):
    """What Soundcheck measured for each song on this MPC, and how each Soundcheck went (live shows B2b)."""
    kept = [(run, m.groups()) for run in runs for ln in run.lines for m in [SOUNDCHECK_KEPT.match(ln)] if m]
    over = [(run, m.groups()) for run in runs for ln in run.lines for m in [SOUNDCHECK_OVER.match(ln)] if m]
    if not kept and not over:
        return
    out.append("## Soundcheck\n")
    out.append("Each song as Soundcheck kept it in the set: its cold open, its memory at its peak, the CPU at its "
               "worst in its busiest part, the longest a clip waited for its audio, misses, xruns, and the change "
               "into the next song (its ending, then the next song's cold open).\n")
    if kept:
        out.append("| workflow | song | what it measured |")
        out.append("|---|---|---|")
        for run, (k, name, line) in kept:
            out.append(f"| {run.label()} | {k} {name} | {line} |")
        out.append("")
    for run, (set_name, how, words) in over:
        out.append(f"{run.label()}: {set_name} {how}: {words}.")
    if over:
        out.append("")


def cores_section(found, out):
    heavy = [(run, a) for run, a in found if run.workflow == "heavy-load" and run.cores != "default"]
    if not heavy:
        return
    out.append("## Sound cores: heavy-load on each setting\n")
    out.append("| setting | underruns | slow periods | late parts | steady part: sampler µs | steady part: synth µs | worst sampler µs |")
    out.append("|---|---|---|---|---|---|---|")
    scored = []
    for run, a in heavy:
        under = sum(p.get("underruns", 0) or 0 for p in a["parts"])
        slow = sum(p["slow"] for p in a["parts"])
        late = sum(p.get("late", 0) or 0 for p in a["parts"])
        steady = next((p for p in a["parts"] if p["name"] == "steady"), {})
        worst = (a["end"] or {}).get("sampler_max")
        out.append(f"| {run.cores} | {under} | {slow} | {late} | {fmt(steady.get('sampler'))} "
                   f"| {fmt(steady.get('synth'))} | {fmt(worst)} |")
        if run.rc == 0:
            scored.append((under, late, slow, worst if worst is not None else 1e9, run.cores))
    out.append("")
    if scored:
        best = min(scored)
        out.append(f"The numbers favour **{best[4]}**: fewest underruns, then late parts (sound a lane missed), then slow periods, "
                   "then the worst period. A tie is the owner's pick.\n")


def bench_reports(lines):
    """{name: json} from the bench report's JSON blocks."""
    found, name, buf = {}, None, []
    for ln in lines:
        m = JSON_START.match(ln)
        if m:
            name, buf = m.group(1), []
            continue
        m = JSON_END.match(ln)
        if m and name:
            try:
                found[name] = json.loads("\n".join(buf))
            except ValueError:
                pass
            name = None
            continue
        if name:
            buf.append(ln)
    return found


def neon_section(reports, out):
    a, b = reports.get("default"), reports.get("default-plain")
    if not a or not b:
        return None
    pa = {c["name"]: c for c in a.get("cases", [])}
    pb = {c["name"]: c for c in b.get("cases", [])}
    rows = []
    for name in sorted(set(pa) & set(pb)):
        x, y = pa[name].get("p99_ns"), pb[name].get("p99_ns")
        if x and y:
            rows.append((x / y, name, x, y))
    faster = [r for r in rows if r[0] < 1 - NEON_MARGIN]
    slower = [r for r in rows if r[0] > 1 + NEON_MARGIN]
    out.append("## NEON: keep or drop\n")
    out.append(f"{len(rows)} engines timed both ways; with the NEON kernels {len(faster)} are faster by more than "
               f"{NEON_MARGIN:.0%}, {len(slower)} slower, the rest the same.\n")
    if faster or slower:
        out.append("| engine | p99 with NEON µs | p99 plain µs | NEON / plain |")
        out.append("|---|---|---|---|")
        for ratio, name, x, y in sorted(faster) + sorted(slower):
            out.append(f"| {name} | {x / 1e3:.1f} | {y / 1e3:.1f} | {ratio:.2f} |")
        out.append("")
    if faster and not slower:
        verdict = "keep NEON: faster where it differs, nowhere slower"
    elif slower and not faster:
        verdict = "drop NEON: slower where it differs"
    elif not faster and not slower:
        verdict = "no difference on the MPC: keep the plain code's simplicity or NEON, the owner's pick"
    else:
        verdict = "mixed: keep NEON for the engines it speeds up and look at the slower ones above"
    out.append(f"Verdict: **{verdict}**.\n")
    return verdict


def afford_section(lines, out):
    build, rows, grain, fit, starts_over = None, [], None, None, []
    for ln in lines:
        m = BUILD.match(ln)
        if m:
            build = m.group(1)
        m = AFFORD_ROW.match(ln)
        if m and m.group(1).strip() != "engine":
            name = m.group(1).strip()
            if any(k in name.lower() for k in ASKED):
                rows.append((build, name, m.group(2), m.group(3), m.group(6), m.group(7)))
        m = GRAIN_COST.match(ln)
        if m and build == "default":
            grain = m.groups()
        m = GRAIN_FIT.match(ln)
        if m and build == "default":
            fit = m.groups()
        if ln.endswith(OVER) and build == "default":
            starts_over.append(re.split(r"\s{2,}", ln)[0])
    if not (rows or grain or fit or starts_over):
        return
    out.append("## What a voice costs on the MPC (the device round's asks)\n")
    if rows:
        out.append("| build | engine | fixed µs | µs a voice | budget µs | voices that fit |")
        out.append("|---|---|---|---|---|---|")
        for r in rows:
            out.append("| " + " | ".join(r) + " |")
        out.append("")
    if grain or fit:
        out.append("### Grain budget\n")
        if grain:
            out.append(f"A grain costs {grain[0]} µs and a looping sample voice {grain[1]} µs on the MPC, so a voice is "
                       f"about {grain[2]} grains: set `VOICE_COST` to **{round(float(grain[2]))}** (now {grain[3]}).")
        if fit:
            out.append(f"{fit[0]} grains fit beside the sample voices: set `GRAIN_BUDGET` to **{fit[0]}** (now {fit[1]}).")
        out.append("")
    if starts_over:
        out.append("### Note starts over the synth budget\n")
        for s in starts_over:
            out.append(f"- {s}")
        out.append("")


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("logs", nargs="+")
    p.add_argument("--out")
    args = p.parse_args(argv)
    lines = read(args.logs)
    runs = runs_in(lines)
    reports = bench_reports(lines)
    answer = next((m.group(1) for ln in lines for m in [LISTEN.match(ln)] if m), None)
    if answer == "(none)":
        answer = ""
    out = ["# Device round: what the reports show\n"]
    bad = False
    if runs:
        found, bad = workflow_section(runs, out, answer)
        opens_section(runs, out)
        soundcheck_section(runs, out)
        cores_section(found, out)
    verdict = neon_section(reports, out)
    afford_section(lines, out)
    if not runs and not reports and len(out) == 1:
        print("devtest_report: no workflow runs or benchmark reports in these files")
        return 2
    text = "\n".join(out).rstrip() + "\n"
    print(text, end="")
    if args.out:
        with open(args.out, "w") as fh:
            fh.write(text)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
