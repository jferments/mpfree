"""Shared helpers for the user manual tools: reading SUMMARY.md, page headers,
and turning the manual's few extensions into HTML (mdBook) or Pandoc Markdown
(the PDF).

The extensions, all in docs/user/src:
  <!-- status/covers/checked header -->   at the top of every page
  {{shot: NAME}} or {{shot: NAME | Caption}}   a screen drawn by mpfree --preview
  {{build}}                                 the commit and date this copy was built from
  {{mpc-notes}}                             every "Coming from an MPC" box, collected
  > [!MPC] / [!NOTE] / [!TIP] / [!WARNING]  boxes (GitHub shows the last three natively)
"""
import os
import re
import subprocess

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
USER = os.path.join(ROOT, "docs", "user")
SRC = os.path.join(USER, "src")

STATUSES = ("ready", "changing", "planned")
BANNERS = {
    "changing": ("Changing", "This part of MPFree works, but it is still being designed. "
                 "The screens and controls may differ in a newer version."),
    "planned": ("Planned", "This part of MPFree is not built yet. "
                "This page says what is coming."),
}
BOX_TITLES = {
    "MPC": "Coming from an MPC",
    "NOTE": "Note",
    "TIP": "Tip",
    "WARNING": "Careful",
    "IMPORTANT": "Important",
}

HEADER = re.compile(r"\A<!--\s*\n(.*?)\n-->\s*\n", re.S)
SHOT = re.compile(r"\{\{\s*shot:\s*([a-z0-9-]+)\s*(?:\|\s*(.*?)\s*)?\}\}")
LINK = re.compile(r"(\[[^\]]*\]\()([^)\s]+)(\))")


def summary():
    """[(kind, title, path)] in book order: kind is 'part' or 'page'."""
    out = []
    with open(os.path.join(SRC, "SUMMARY.md"), encoding="utf-8") as f:
        for line in f:
            line = line.rstrip()
            if line.startswith("# ") and line != "# Summary":
                out.append(("part", line[2:].strip(), None))
                continue
            m = re.match(r"^\s*(?:- )?\[(.+?)\]\((.+?)\)\s*$", line)
            if m:
                out.append(("page", m.group(1), m.group(2)))
    return out


def read_header(text):
    """(fields dict, body without the header) or (None, text)."""
    m = HEADER.match(text)
    if not m:
        return None, text
    fields = {}
    for line in m.group(1).splitlines():
        if ":" in line:
            k, v = line.split(":", 1)
            fields[k.strip()] = v.strip()
    return fields, text[m.end():]


def git(*args):
    try:
        return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True,
                              check=True).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return ""


def build_line():
    commit = git("rev-parse", "--short", "HEAD") or "unknown"
    date = git("log", "-1", "--format=%cs") or "unknown date"
    dirty = " with local changes" if git("status", "--porcelain", "--", "docs/user", "rust") else ""
    return (f"This copy describes MPFree as of commit `{commit}` ({date}){dirty}. "
            "Screenshots were drawn by that same version.")


def split_boxes(body):
    """Yield ('text', str) and ('box', kind, str) chunks."""
    lines = body.split("\n")
    i = 0
    buf = []
    while i < len(lines):
        m = re.match(r"^>\s*\[!([A-Z]+)\]\s*$", lines[i])
        if m and m.group(1) in BOX_TITLES:
            if buf:
                yield ("text", "\n".join(buf))
                buf = []
            kind = m.group(1)
            i += 1
            inner = []
            while i < len(lines) and lines[i].startswith(">"):
                inner.append(re.sub(r"^> ?", "", lines[i]))
                i += 1
            yield ("box", kind, "\n".join(inner).strip())
            continue
        buf.append(lines[i])
        i += 1
    if buf:
        yield ("text", "\n".join(buf))


def first_heading(body):
    m = re.search(r"^# (.+)$", body, re.M)
    return m.group(1).strip() if m else ""


def page_label(path):
    """A stable label for a page in the PDF: page-getting-started-first-beat."""
    return "page-" + re.sub(r"[^a-z0-9]+", "-", path.removesuffix(".md").lower()).strip("-")


def collect_mpc_notes(pages):
    """[(page path, page title, note markdown)] from every page, in book order."""
    notes = []
    for path in pages:
        with open(os.path.join(SRC, path), encoding="utf-8") as f:
            _, body = read_header(f.read())
        for chunk in split_boxes(body):
            if chunk[0] == "box" and chunk[1] == "MPC":
                notes.append((path, first_heading(body), chunk[2]))
    return notes


def relink(md, from_path, to_path):
    """Rewrite relative links in markdown written for from_path to work from to_path."""
    def fix(m):
        t = m.group(2)
        if re.match(r"^[a-z]+:", t) or t.startswith("#"):
            return m.group(0)
        p, _, frag = t.partition("#")
        full = os.path.normpath(os.path.join(os.path.dirname(from_path), p))
        rel = os.path.relpath(full, os.path.dirname(to_path) or ".")
        return m.group(1) + rel + ("#" + frag if frag else "") + m.group(3)
    return LINK.sub(fix, md)
