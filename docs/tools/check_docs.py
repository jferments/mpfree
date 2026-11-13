#!/usr/bin/env python3
"""Check the docs: links, indexes, user-manual page headers, retired names and
the manual's extensions. Needs only Python 3. Exit 1 on a problem.

    python3 docs/tools/check_docs.py
    python3 docs/tools/check_docs.py --shots DIR   # also check {{shot: ...}} names against
                                                   # the screens `mpfree --preview DIR` drew

The rules are in docs/agents/docs-system.md.
"""
import argparse
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(__file__))
import manual_pages as mp  # noqa: E402

ROOT = mp.ROOT
OLD_DIRS = ("project", "decisions", "guides", "hardware", "milestones", "perf", "research")
FENCE = re.compile(r"^(```|~~~).*?^\1", re.S | re.M)
# Dated archives keep old pages byte for byte (their blob SHAs are listed in
# each archive's README), so their old relative links are not checked, and a
# page there is listed when its archive's README links it.
ARCHIVES = ("docs/dev/hardware/archive/",)


def archived(f):
    return f.startswith(ARCHIVES) and os.path.basename(f) != "README.md"


def tracked_md():
    out = subprocess.run(["git", "ls-files", "-co", "--exclude-standard", "*.md"], cwd=ROOT,
                         capture_output=True, text=True, check=True).stdout.split()
    return [p for p in out if os.path.exists(os.path.join(ROOT, p))
            and "/target/" not in p and not p.startswith("docs/user/build/")]


def check_links(files, errors):
    for f in files:
        if archived(f):
            continue
        with open(os.path.join(ROOT, f), encoding="utf-8", errors="replace") as fh:
            text = FENCE.sub("", fh.read())
        text = re.sub(r"`[^`\n]*`", "", text)
        for m in re.finditer(r"\]\(([^)\s]+)\)", text):
            target = m.group(1)
            if re.match(r"^[a-z]+:", target) or target.startswith("#") or target.startswith("shots/"):
                continue
            path = target.split("#")[0]
            if not path:
                continue
            full = os.path.normpath(os.path.join(ROOT, os.path.dirname(f), path))
            if not os.path.exists(full):
                errors.append(f"{f}: broken link {target}")


def linked_from(index, folder):
    with open(os.path.join(ROOT, index), encoding="utf-8") as fh:
        text = fh.read()
    base = os.path.dirname(index)
    return {os.path.normpath(os.path.join(base, t.split("#")[0]))
            for t in re.findall(r"\]\(([^)\s#]+)", text)}


def check_layout(files, errors):
    for f in files:
        parts = f.split("/")
        if parts[0] != "docs":
            continue
        if len(parts) == 2 and f != "docs/README.md":
            errors.append(f"{f}: pages go in docs/user, docs/dev or docs/agents (docs-system.md, section 1)")
        if len(parts) > 2 and parts[1] in OLD_DIRS:
            errors.append(f"{f}: old docs path; run python3 docs/tools/migrate_doc_paths.py")
    for folder, index in (("docs/dev", "docs/dev/README.md"), ("docs/agents", "docs/agents/README.md")):
        linked = linked_from(index, folder)
        for readme in files:
            if readme.startswith(ARCHIVES) and readme.startswith(folder + "/") \
                    and os.path.basename(readme) == "README.md":
                linked |= linked_from(readme, os.path.dirname(readme))
        for f in files:
            if f.startswith(folder + "/") and f != index and f not in linked:
                errors.append(f"{f}: not listed in {index}")


def read_retired(path):
    """The [retired] table of terms.toml: "old name" = "what to use instead".
    (A small reader, so the check runs on any Python 3 without tomllib.)"""
    out, section = {}, None
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if line.startswith("["):
                section = line.strip("[]")
                continue
            m = re.match(r'^"([^"]+)"\s*=\s*"([^"]*)"$', line) or re.match(r'^([\w-]+)\s*=\s*"([^"]*)"$', line)
            if section == "retired" and m:
                out[m.group(1)] = m.group(2)
    return out


def check_manual(errors, warnings, shots_dir):
    toc = mp.summary()
    pages = [p for k, _, p in toc if k == "page"]
    for p in pages:
        if not os.path.exists(os.path.join(mp.SRC, p)):
            errors.append(f"docs/user/src/SUMMARY.md: {p} does not exist")
    for dirpath, _, names in os.walk(mp.SRC):
        for n in names:
            rel = os.path.relpath(os.path.join(dirpath, n), mp.SRC)
            if n.endswith(".md") and rel != "SUMMARY.md" and rel not in pages:
                errors.append(f"docs/user/src/{rel}: not listed in SUMMARY.md")

    retired = read_retired(os.path.join(mp.USER, "terms.toml"))
    drawn = None
    if shots_dir:
        drawn = {n[:-4] for n in os.listdir(shots_dir) if n.endswith(".png")}

    for p in pages:
        full = os.path.join(mp.SRC, p)
        if not os.path.exists(full):
            continue
        where = f"docs/user/src/{p}"
        with open(full, encoding="utf-8") as fh:
            text = fh.read()
        fields, body = mp.read_header(text)
        if fields is None:
            errors.append(f"{where}: no header comment (status, covers, checked)")
        else:
            if fields.get("status") not in mp.STATUSES:
                errors.append(f"{where}: status must be one of {', '.join(mp.STATUSES)}")
            for c in fields.get("covers", "").split():
                if not os.path.exists(os.path.join(ROOT, c)):
                    errors.append(f"{where}: covers {c}, which does not exist")
            if not re.match(r"^\d{4}-\d{2}-\d{2} [0-9a-f]{7,40}$", fields.get("checked", "")):
                errors.append(f"{where}: checked must be 'YYYY-MM-DD <commit>'")
        if not re.search(r"^# .+", body, re.M):
            errors.append(f"{where}: no '# Title' line")
        prose = FENCE.sub("", body)
        for old, new in retired.items():
            if re.search(r"(?<![\w-])" + re.escape(old) + r"(?![\w-])", prose):
                errors.append(f"{where}: '{old}' is a retired name; use {new}")
        for m in re.finditer(r"\{\{([^}]*)\}\}", prose):
            inner = m.group(1).strip()
            if inner in ("build", "mpc-notes"):
                continue
            sm = mp.SHOT.fullmatch(m.group(0))
            if not sm:
                errors.append(f"{where}: unknown {{{{{inner}}}}} (use shot:, build or mpc-notes)")
            elif drawn is not None and sm.group(1) not in drawn:
                errors.append(f"{where}: screen '{sm.group(1)}' is not drawn by mpfree --preview")
        for m in re.finditer(r"^>\s*\[!([A-Z]+)\]", prose, re.M):
            if m.group(1) not in mp.BOX_TITLES:
                errors.append(f"{where}: unknown box [!{m.group(1)}]")
        if re.search(r"\brust/mpfree-|\bcrate\b|`mpfree-", prose):
            warnings.append(f"{where}: mentions code; the manual is for musicians")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--shots")
    a = ap.parse_args()
    errors, warnings = [], []
    files = tracked_md()
    check_links(files, errors)
    check_layout(files, errors)
    check_manual(errors, warnings, a.shots)
    for w in warnings:
        print("warning:", w)
    for e in errors:
        print("error:", e)
    print(f"check_docs: {len(files)} pages, {len(errors)} errors, {len(warnings)} warnings")
    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
