#!/usr/bin/env python3
"""Check a built kernel's .config against MPFree's settings and rules.

Usage: check-config.py FRAGMENT DOTCONFIG

- Every `CONFIG_X=value` line of FRAGMENT (tools/kernel/mpfree.config) must be
  in DOTCONFIG exactly. Kconfig silently drops or changes a setting whose
  dependencies are missing, so a setting that did not take is an error.
- Every `# CONFIG_X is not set` line of FRAGMENT must be unset in DOTCONFIG.
- MPFree's own rules, whatever FRAGMENT says (RULES below): the eMMC and SD
  card drivers are never built in, so nothing probes them at start
  (docs/dev/hardware/sd-card-and-internal-drive.md); modules are never
  required to be signed; the kernel carries its own config.

Prints one line per problem, then `CONFIG-OK` or `CONFIG-FAILED: N problems`.
Exit status 0 when there are none, 1 when there are, 2 for a usage error.
Reads only.
"""
import re
import sys

SET = re.compile(r"^(CONFIG_[A-Za-z0-9_]+)=(.*)$")
UNSET = re.compile(r"^# (CONFIG_[A-Za-z0-9_]+) is not set$")

# (symbol, allowed values; None = unset). Checked after the fragment.
RULES = (
    ("CONFIG_MMC", {"m", None},
     "the MMC core must be a module or absent: built in, it would probe the eMMC and SD card at start"),
    ("CONFIG_MODULE_SIG_FORCE", {None}, "modules must not be required to be signed"),
    ("CONFIG_IKCONFIG_PROC", {"y"}, "the kernel must carry its config in /proc/config.gz"),
)


def parse(text):
    """{symbol: value or None} from .config text; later lines win, as in Kconfig."""
    out = {}
    for line in text.splitlines():
        line = line.strip()
        m = SET.match(line)
        if m:
            out[m.group(1)] = m.group(2)
            continue
        m = UNSET.match(line)
        if m:
            out[m.group(1)] = None
    return out


def check(fragment_text, config_text):
    """The list of problems, one sentence each."""
    wanted = parse(fragment_text)
    have = parse(config_text)
    problems = []
    if not have:
        return ["the .config has no CONFIG_ lines"]
    for sym, value in wanted.items():
        got = have.get(sym)
        if value is None:
            if got is not None:
                problems.append(f"{sym}: wanted unset, the build has {sym}={got}")
        elif got != value:
            shown = "unset" if got is None else f"{sym}={got}"
            problems.append(f"{sym}: wanted {sym}={value}, the build has {shown}")
    for sym, allowed, why in RULES:
        got = have.get(sym)
        if got not in allowed:
            shown = "unset" if got is None else f"{sym}={got}"
            problems.append(f"{sym}: {why} (the build has {shown})")
    return problems


def main(argv):
    if len(argv) != 3:
        print(__doc__.strip().splitlines()[2], file=sys.stderr)
        return 2
    try:
        with open(argv[1], encoding="utf-8") as fh:
            fragment = fh.read()
        with open(argv[2], encoding="utf-8") as fh:
            config = fh.read()
    except OSError as err:
        print(f"STOP: {err}", file=sys.stderr)
        return 2
    problems = check(fragment, config)
    for p in problems:
        print(p)
    if problems:
        print(f"CONFIG-FAILED: {len(problems)} problems")
        return 1
    print("CONFIG-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
