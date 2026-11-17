#!/usr/bin/env python3
"""A built kernel's settings, compactly, with a hash on every line.

Usage: config-digest.py print DOTCONFIG
       config-digest.py read COPY

Agents can't reach the CI VM's store and read a job log only as text, so the
kernel workflow prints the built config this way and an agent copies it out of
the log. `read` checks the copy, line by line, and prints the config's set lines
again, sorted, so it can be compared with another kernel's config.

`print` writes:
    config-digest 1: N set (Y =y, M =m, V other); sha256 <hash of the set lines>
    y 001 <h> NAME NAME ...      the symbols set to y, CONFIG_ left off, sorted
    m 001 <h> NAME NAME ...      the symbols set to m
    v 001 <h> NAME=value         every other setting, one per line
    config-digest end
<h> is the first 8 hex digits of the sha256 of the line's kind and text, so a
mistake in a copied line shows which line it is in. The header's hash is the
sha256 of the set lines, `CONFIG_NAME=value`, sorted, one per line: the text
`read` prints. A symbol that is not listed is unset or absent.

`read` accepts the lines with or without the job log's time stamps. It prints
one line per problem to stderr and exits 1, or prints the set lines and exits 0.
Exit status 2 for a usage error. Reads only.
"""
import hashlib
import re
import sys

SET = re.compile(r"^(CONFIG_[A-Za-z0-9_]+)=(.*)$")
NAME = re.compile(r"^[A-Za-z0-9_]+$")
HEADER = re.compile(r"^config-digest 1: (\d+) set \((\d+) =y, (\d+) =m, (\d+) other\); sha256 ([0-9a-f]{64})$")
ROW = re.compile(r"^([ymv]) (\d{3,}) ([0-9a-f]{8}) (.+)$")
STAMP = re.compile(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z ")
END = "config-digest end"
PER_LINE = 12


def short(kind, text):
    return hashlib.sha256(f"{kind} {text}".encode()).hexdigest()[:8]


def set_lines(settings):
    """The canonical text: `CONFIG_NAME=value`, sorted, one per line."""
    return "".join(f"{k}={v}\n" for k, v in sorted(settings.items()))


def parse_config(text):
    """{CONFIG_NAME: value} for the set lines of a .config; later lines win."""
    out = {}
    for line in text.splitlines():
        m = SET.match(line.strip())
        if m:
            out[m.group(1)] = m.group(2)
    return out


def digest(settings):
    """The lines `print` writes, for {CONFIG_NAME: value}."""
    ys = sorted(k[len("CONFIG_"):] for k, v in settings.items() if v == "y")
    ms = sorted(k[len("CONFIG_"):] for k, v in settings.items() if v == "m")
    vs = sorted(f"{k[len('CONFIG_'):]}={v}" for k, v in settings.items() if v not in ("y", "m"))
    whole = hashlib.sha256(set_lines(settings).encode()).hexdigest()
    out = [f"config-digest 1: {len(settings)} set ({len(ys)} =y, {len(ms)} =m, {len(vs)} other); sha256 {whole}"]
    for kind, items, step in (("y", ys, PER_LINE), ("m", ms, PER_LINE), ("v", vs, 1)):
        for n, i in enumerate(range(0, len(items), step), 1):
            text = " ".join(items[i:i + step])
            out.append(f"{kind} {n:03d} {short(kind, text)} {text}")
    out.append(END)
    return out


def read(text):
    """(settings, problems) from a copy of `print`'s lines."""
    problems = []
    header = None
    settings = {}
    numbers = {"y": 0, "m": 0, "v": 0}
    counts = {"y": 0, "m": 0, "v": 0}
    ended = False
    for n, raw in enumerate(text.splitlines(), 1):
        line = STAMP.sub("", raw.rstrip("\r\n")).strip()
        if not line or ended:
            continue
        if header is None:
            m = HEADER.match(line)
            if m:
                header = m
            continue
        if line == END:
            ended = True
            continue
        m = ROW.match(line)
        if not m:
            problems.append(f"line {n}: not a digest line: {line[:60]!r}")
            continue
        kind, number, h, body = m.groups()
        if int(number) != numbers[kind] + 1:
            problems.append(f"line {n}: {kind} {number} follows {kind} {numbers[kind]:03d}")
        numbers[kind] = int(number)
        if short(kind, body) != h:
            problems.append(f"line {n}: {kind} {number}: the text does not match its hash {h}")
            continue
        if kind == "v":
            name, eq, value = body.partition("=")
            if not eq or not NAME.match(name):
                problems.append(f"line {n}: v {number}: not NAME=value")
                continue
            items = [(name, value)]
        else:
            names = body.split(" ")
            bad = [x for x in names if not NAME.match(x)]
            if bad:
                problems.append(f"line {n}: {kind} {number}: not a symbol name: {bad[0][:40]!r}")
                continue
            items = [(x, kind) for x in names]
        for name, value in items:
            key = "CONFIG_" + name
            if key in settings:
                problems.append(f"line {n}: {key} is listed twice")
            settings[key] = value
            counts[kind] += 1
    if header is None:
        return {}, ["no `config-digest 1:` line"]
    if not ended:
        problems.append(f"no `{END}` line: the copy is cut short")
    total, ny, nm, nv, whole = header.groups()
    if (counts["y"], counts["m"], counts["v"]) != (int(ny), int(nm), int(nv)):
        problems.append(f"the copy has {counts['y']} =y, {counts['m']} =m, {counts['v']} other; "
                        f"the header says {ny}, {nm}, {nv}")
    if len(settings) != int(total):
        problems.append(f"the copy has {len(settings)} settings, the header says {total}")
    if not problems and hashlib.sha256(set_lines(settings).encode()).hexdigest() != whole:
        problems.append("every line matches its hash but the whole does not match the header's sha256")
    return settings, problems


def main(argv):
    if len(argv) != 3 or argv[1] not in ("print", "read"):
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 2
    try:
        with open(argv[2], encoding="utf-8", errors="replace") as fh:
            text = fh.read()
    except OSError as e:
        print(f"STOP: {e}", file=sys.stderr)
        return 2
    if argv[1] == "print":
        settings = parse_config(text)
        if not settings:
            print("STOP: the .config has no CONFIG_ lines", file=sys.stderr)
            return 1
        print("\n".join(digest(settings)))
        return 0
    settings, problems = read(text)
    if problems:
        for p in problems:
            print(p, file=sys.stderr)
        print(f"DIGEST-FAILED: {len(problems)} problems", file=sys.stderr)
        return 1
    sys.stdout.write(set_lines(settings))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
