#!/usr/bin/env python3
"""List docs pages whose code changed but the page didn't. A reminder, not a
failure: it exits 0 unless --strict. Needs only Python 3 and git.

    python3 docs/tools/doc_drift.py --base origin/main   # this branch's changes
    python3 docs/tools/doc_drift.py                      # since each page's 'checked' commit

What a page covers comes from its header comment (`covers:`, see
docs/agents/docs-system.md). A developer design page without one covers the
`rust/...` crates named in its title and first paragraphs
(as rust/mpfree-x or `mpfree-x`).
With --summary FILE the report is appended there as Markdown (CI passes
$GITHUB_STEP_SUMMARY).
"""
import argparse
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(__file__))
import manual_pages as mp  # noqa: E402

ROOT = mp.ROOT


def git(*args):
    r = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else None


def pages():
    """{page path: (covers list, checked commit or None)}."""
    out = {}
    for base in ("docs/user/src", "docs/dev/design"):
        for dirpath, _, names in os.walk(os.path.join(ROOT, base)):
            for n in sorted(names):
                if not n.endswith(".md") or n == "SUMMARY.md":
                    continue
                full = os.path.join(dirpath, n)
                rel = os.path.relpath(full, ROOT)
                with open(full, encoding="utf-8") as fh:
                    text = fh.read()
                fields, body = mp.read_header(text)
                if fields and fields.get("covers"):
                    covers = fields["covers"].split()
                    checked = (fields.get("checked", "").split() or [None])[-1]
                else:
                    head = "\n\n".join(body.split("\n\n")[:4])
                    names = set(re.findall(r"rust/(mpfree-[a-z0-9-]+)", head))
                    names |= set(re.findall(r"`(mpfree-[a-z0-9-]+)`", head))
                    covers = sorted(f"rust/{n}" for n in names
                                    if os.path.isdir(os.path.join(ROOT, "rust", n)))
                    checked = None
                covers = [c for c in covers if not c.startswith("docs/")]
                if covers:
                    out[rel] = (covers, checked)
    return out


def touches(changed, covers):
    hits = []
    for c in covers:
        c = c.rstrip("/")
        hits += [f for f in changed if f == c or f.startswith(c + "/")]
    return sorted(set(hits))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", help="compare this branch with BASE (merge base), as in a pull request")
    ap.add_argument("--summary", help="append the report to this file as Markdown")
    ap.add_argument("--strict", action="store_true", help="exit 1 if any page may be stale")
    a = ap.parse_args()

    report = []
    if a.base:
        mb = git("merge-base", a.base, "HEAD")
        if not mb:
            sys.exit(f"doc_drift: can't find the merge base with {a.base}")
        changed = (git("diff", "--name-only", mb) or "").splitlines()
        untracked = (git("ls-files", "--others", "--exclude-standard") or "").splitlines()
        changed = set(changed) | set(untracked)
        for page, (covers, _) in pages().items():
            hits = touches(changed, covers)
            if hits and page not in changed:
                report.append((page, hits))
        title = f"Docs that may need updating for this change (against {a.base})"
    else:
        for page, (covers, checked) in pages().items():
            if not checked or git("cat-file", "-e", checked + "^{commit}") is None:
                continue
            changed = (git("diff", "--name-only", checked, "HEAD", "--", *covers) or "").splitlines()
            if changed:
                report.append((page, changed))
        title = "Docs whose code changed since they were last checked"

    lines = [f"### {title}", ""]
    if not report:
        lines.append("None.")
    else:
        lines.append("Each page below covers code that changed. Update the page, or if it is still "
                     "right, just update its `checked:` line. See docs/agents/docs-system.md.")
        lines.append("")
        for page, hits in report:
            shown = ", ".join(f"`{h}`" for h in hits[:4]) + (f" and {len(hits) - 4} more" if len(hits) > 4 else "")
            lines.append(f"- `{page}`: {shown}")
    text = "\n".join(lines) + "\n"
    print(text, end="")
    if a.summary:
        with open(a.summary, "a", encoding="utf-8") as fh:
            fh.write(text)
    sys.exit(1 if (a.strict and report) else 0)


if __name__ == "__main__":
    main()
