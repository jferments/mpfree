#!/usr/bin/env python3
"""Move docs from the old layout (2026-11-11) to docs/user, docs/dev, docs/agents.

Safe to run more than once, and on any branch: it moves only files still at an
old path, then rewrites every Markdown link and every plain "docs/..." path
mention in the repository to the new place. A branch made before the move runs
it once after merging main; see docs/agents/docs-system.md.

    python3 docs/tools/migrate_doc_paths.py          # do it
    python3 docs/tools/migrate_doc_paths.py --check  # list what would change, exit 1 if anything
"""
import os
import re
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))

# Old folder (or file) -> new place. Order matters: first match wins.
PROJECT_LEVEL = {"architecture", "roadmap", "development-log", "factory-content-policy"}
DIR_MOVES = [
    ("docs/decisions/", "docs/dev/decisions/"),
    ("docs/guides/", "docs/dev/guides/"),
    ("docs/hardware/", "docs/dev/hardware/"),
    ("docs/milestones/", "docs/dev/milestones/"),
    ("docs/perf/", "docs/dev/perf/"),
    ("docs/research/", "docs/dev/research/"),
]


def new_path(old):
    """Repo-relative old path -> repo-relative new path (or the same path)."""
    if old.startswith("docs/project/"):
        rest = old[len("docs/project/"):]
        stem = rest.split("/")[0].removesuffix(".md")
        sub = "project" if stem in PROJECT_LEVEL else "design"
        return f"docs/dev/{sub}/{rest}"
    for a, b in DIR_MOVES:
        if old.startswith(a):
            return b + old[len(a):]
        if old == a.rstrip("/"):
            return b.rstrip("/")
    return old


def tracked():
    out = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True)
    return [p for p in out.stdout.splitlines() if os.path.exists(os.path.join(ROOT, p))]


def moves_needed(files):
    return [(f, new_path(f)) for f in files if new_path(f) != f]


LINK = re.compile(r"(\]\()([^)\s]+)(\))")
PLAIN = re.compile(r"docs/(project|decisions|guides|hardware|milestones|perf|research)/[A-Za-z0-9_./-]*")
TEXT_EXT = (".md", ".rs", ".py", ".sh", ".yml", ".yaml", ".toml", ".txt", ".json", ".html")


def rewrite_links(text, old_file, new_file, moved):
    """Rewrite relative Markdown links in a file that moved from old_file to new_file."""
    old_dir = os.path.dirname(old_file)
    new_dir = os.path.dirname(new_file)

    def fix(m):
        target = m.group(2)
        if re.match(r"^[a-z]+:", target) or target.startswith("#") or target.startswith("/"):
            return m.group(0)
        path, frag = (target.split("#", 1) + [""])[:2]
        if not path:
            return m.group(0)
        resolved = os.path.normpath(os.path.join(old_dir, path))
        if resolved.startswith(".."):
            return m.group(0)
        dest = moved.get(resolved, new_path(resolved))
        rel = os.path.relpath(dest, new_dir or ".")
        if path.endswith("/") and not rel.endswith("/"):
            rel += "/"
        out = rel + ("#" + frag if frag else "")
        return m.group(1) + out + m.group(3)

    return LINK.sub(fix, text)


def main():
    check = "--check" in sys.argv
    files = tracked()
    moves = moves_needed(files)
    moved = dict(moves)
    changes = []
    if check:
        for a, b in moves:
            changes.append(f"move {a} -> {b}")
    else:
        for a, b in moves:
            os.makedirs(os.path.join(ROOT, os.path.dirname(b)), exist_ok=True)
            subprocess.run(["git", "mv", a, b], cwd=ROOT, check=True)
    # Link and path rewrites: every text file, at its (new) place.
    old_of = {b: a for a, b in moves}
    for f in (new_path(p) for p in files):
        if not f.endswith(TEXT_EXT):
            continue
        cur = f if not check else old_of.get(f, f)
        full = os.path.join(ROOT, cur)
        if not os.path.exists(full):
            continue
        with open(full, encoding="utf-8", errors="surrogateescape") as fh:
            text = fh.read()
        new = text
        if f.endswith(".md"):
            new = rewrite_links(new, old_of.get(f, f), f, moved)
        new = PLAIN.sub(lambda m: new_path(m.group(0)), new)
        if new != text:
            changes.append(f"rewrite {f}")
            if not check:
                with open(full, "w", encoding="utf-8", errors="surrogateescape") as fh:
                    fh.write(new)
    for c in changes:
        print(c)
    if check:
        sys.exit(1 if changes else 0)


if __name__ == "__main__":
    main()
