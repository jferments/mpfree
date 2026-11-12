#!/usr/bin/env python3
"""Build the MPFree user manual as a website (mdBook) and a PDF (Pandoc + Typst).

    python3 docs/tools/build_manual.py                 # both, into docs/user/build/
    python3 docs/tools/build_manual.py --no-pdf        # website only
    python3 docs/tools/build_manual.py --shots DIR     # use screens already drawn

Screenshots come from the code: unless --shots is given, this builds
mpfree-app and runs `mpfree --preview` to draw every page as it is now.

Needs: Python 3 with Pillow (to shrink the screenshots), cargo, and for the
outputs mdbook (`cargo install mdbook`), pandoc 3.1 or newer and typst
(`cargo install typst-cli`). Output:
  docs/user/build/site/index.html      the website
  docs/user/build/mpfree-manual.pdf    the PDF
"""
import argparse
import os
import re
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(__file__))
import manual_pages as mp  # noqa: E402

OUT_DEFAULT = os.path.join(mp.USER, "build")
FONTS = os.path.join(mp.ROOT, "rust", "mpfree-ui", "fonts")


def run(cmd, **kw):
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, check=True, **kw)


def draw_shots(out):
    raw = os.path.join(out, "shots-raw")
    shutil.rmtree(raw, ignore_errors=True)
    run(["cargo", "build", "--manifest-path", os.path.join(mp.ROOT, "rust", "Cargo.toml"),
         "--release", "--locked", "-p", "mpfree-app"])
    exe = os.path.join(mp.ROOT, "rust", "target", "release", "mpfree")
    run([exe, "--preview", raw], stdout=subprocess.DEVNULL)
    return raw


def shrink_shots(raw, dest, wanted):
    """Copy the screens the pages use, as small palette PNGs (about 10x smaller)."""
    from PIL import Image
    os.makedirs(dest, exist_ok=True)
    missing = []
    for name in sorted(wanted):
        src = os.path.join(raw, name + ".png")
        if not os.path.exists(src):
            missing.append(name)
            continue
        img = Image.open(src).convert("RGB")
        img = img.quantize(colors=256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
        img.save(os.path.join(dest, name + ".png"), optimize=True)
    return missing


# ---------------------------------------------------------------- pages


def banner(fields, target):
    status = (fields or {}).get("status", "ready")
    if status not in mp.BANNERS:
        return ""
    title, text = mp.BANNERS[status]
    if target == "html":
        return f'<div class="mpf-status {status}"><strong>{title}</strong> {text}</div>\n\n'
    return f"::: {{.mpfstatus .{status}}}\n**{title}.** {text}\n:::\n\n"


def box(kind, inner, target):
    title = mp.BOX_TITLES[kind]
    cls = kind.lower()
    if target == "html":
        return (f'<div class="mpf-box {cls}">\n<p class="mpf-box-title">{title}</p>\n\n'
                f"{inner}\n\n</div>\n")
    return f"::: {{.mpfbox .{cls}}}\n**{title}**\n\n{inner}\n:::\n"


def shot(m, path, target):
    name, caption = m.group(1), (m.group(2) or "")
    if target == "html":
        rel = os.path.relpath("shots", os.path.dirname(path) or ".")
        cap = f"<figcaption>{caption}</figcaption>" if caption else ""
        alt = caption.replace('"', "&quot;") or name
        return (f'<figure class="mpf-shot"><img src="{rel}/{name}.png" alt="{alt}" '
                f'loading="lazy">{cap}</figure>')
    return f"![{caption}](shots/{name}.png)"


def render(path, text, target, notes, labels):
    fields, body = mp.read_header(text)
    out = []
    for chunk in mp.split_boxes(body):
        out.append(chunk[1] if chunk[0] == "text" else box(chunk[1], chunk[2], target))
    body = "\n".join(out)
    # The status banner goes under the page's title.
    body = re.sub(r"^(# .+\n)", lambda m: m.group(1) + "\n" + banner(fields, target), body,
                  count=1, flags=re.M)
    body = mp.SHOT.sub(lambda m: shot(m, path, target), body)
    body = body.replace("{{build}}", mp.build_line())
    if "{{mpc-notes}}" in body:
        parts = []
        for src, title, note in notes:
            note = mp.relink(note, src, path)
            link = os.path.relpath(src, os.path.dirname(path) or ".")
            parts.append(f"## {title}\n\n{note}\n\n*From [{title}]({link}).*\n")
        body = body.replace("{{mpc-notes}}", "\n".join(parts) or "*None yet.*")
    if target == "pdf":
        body = pdf_links(body, path, labels)
        body = re.sub(r"^# (.+)$", lambda m: f"# {m.group(1)} {{#{mp.page_label(path)}}}",
                      body, count=1, flags=re.M)
    return body


def pdf_links(body, path, labels):
    """Links to other pages become links inside the PDF; outside links stay."""
    def fix(m):
        t = m.group(2)
        if re.match(r"^[a-z]+:", t) or t.startswith("#") or t.startswith("shots/"):
            return m.group(0)
        p = t.split("#")[0]
        full = os.path.normpath(os.path.join(os.path.dirname(path), p))
        if full in labels:
            return m.group(1) + "#" + labels[full] + m.group(3)
        # A link out of the manual (a developer page): point at GitHub.
        repo = os.path.normpath(os.path.join("docs/user/src", os.path.dirname(path), t))
        return m.group(1) + "https://github.com/jferments/mpfree/blob/main/" + repo + m.group(3)
    return mp.LINK.sub(fix, body)


def prepare(out, target, pages, notes):
    dest = os.path.join(out, f"src-{target}")
    shutil.rmtree(dest, ignore_errors=True)
    os.makedirs(dest)
    labels = {p: mp.page_label(p) for p in pages}
    shutil.copy(os.path.join(mp.SRC, "SUMMARY.md"), dest)
    for p in pages:
        with open(os.path.join(mp.SRC, p), encoding="utf-8") as f:
            text = f.read()
        os.makedirs(os.path.join(dest, os.path.dirname(p)), exist_ok=True)
        with open(os.path.join(dest, p), "w", encoding="utf-8") as f:
            f.write(render(p, text, target, notes, labels))
    return dest


# ---------------------------------------------------------------- outputs


def build_site(out, src_html, shots):
    book = os.path.join(out, "book")
    shutil.rmtree(book, ignore_errors=True)
    os.makedirs(book)
    with open(os.path.join(mp.USER, "book.toml"), encoding="utf-8") as f:
        toml = f.read().replace('src = "src"', 'src = "src-html"')
    with open(os.path.join(book, "book.toml"), "w", encoding="utf-8") as f:
        f.write(toml)
    shutil.copytree(src_html, os.path.join(book, "src-html"))
    shutil.copytree(shots, os.path.join(book, "src-html", "shots"))
    shutil.copytree(os.path.join(mp.USER, "theme"), os.path.join(book, "theme"))
    site = os.path.join(out, "site")
    shutil.rmtree(site, ignore_errors=True)
    run(["mdbook", "build", book, "--dest-dir", site])
    return site


def build_pdf(out, src_pdf, shots, toc):
    work = os.path.join(out, "pdf")
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    shutil.copytree(shots, os.path.join(work, "shots"))
    filt = os.path.join(mp.USER, "pdf", "filter.lua")
    body = []
    for kind, title, path in toc:
        if kind == "part":
            body.append(f'#part("{title}")\n')
            continue
        typ = subprocess.run(
            ["pandoc", "-f", "commonmark_x", "-t", "typst", "--wrap=none", "--lua-filter", filt,
             os.path.join(src_pdf, path)],
            check=True, capture_output=True, text=True).stdout
        if not any(k == "part" for k, _, _ in toc[:toc.index((kind, title, path))]):
            # Pages before the first part (the welcome) are not numbered chapters.
            typ = re.sub(r"^= (.+)\n<(page-[^>]+)>", r"#heading(level: 1, numbering: none)[\1] <\2>",
                         typ, count=1, flags=re.M)
        body.append(typ)
    with open(os.path.join(work, "body.typ"), "w", encoding="utf-8") as f:
        f.write("#import \"lib.typ\": *\n\n" + "\n".join(body))
    for name in ("manual.typ", "lib.typ"):
        shutil.copy(os.path.join(mp.USER, "pdf", name), work)
    commit = mp.git("rev-parse", "--short", "HEAD") or "unknown"
    date = mp.git("log", "-1", "--format=%cs") or ""
    pdf = os.path.join(out, "mpfree-manual.pdf")
    run(["typst", "compile", "--root", work, "--font-path", FONTS,
         "--input", f"commit={commit}", "--input", f"date={date}",
         os.path.join(work, "manual.typ"), pdf])
    return pdf


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", default=OUT_DEFAULT)
    ap.add_argument("--shots", help="a folder of screens from `mpfree --preview`")
    ap.add_argument("--no-site", action="store_true")
    ap.add_argument("--no-pdf", action="store_true")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)

    toc = mp.summary()
    pages = [p for k, _, p in toc if k == "page"]
    notes = mp.collect_mpc_notes(pages)
    wanted = set()
    for p in pages:
        with open(os.path.join(mp.SRC, p), encoding="utf-8") as f:
            wanted |= {m.group(1) for m in mp.SHOT.finditer(f.read())}
    raw = a.shots or draw_shots(out)
    shots = os.path.join(out, "shots")
    shutil.rmtree(shots, ignore_errors=True)
    missing = shrink_shots(raw, shots, wanted)
    if missing:
        sys.exit("STOP: these screens are asked for but mpfree --preview no longer draws "
                 f"them: {', '.join(missing)}. Fix the page or add the screen "
                 "(rust/mpfree-app/src/gui.rs, sample_screens).")
    if not a.no_site:
        site = build_site(out, prepare(out, "html", pages, notes), shots)
        print(f"website: {os.path.join(site, 'index.html')}")
    if not a.no_pdf:
        pdf = build_pdf(out, prepare(out, "pdf", pages, notes), shots, toc)
        print(f"PDF: {pdf}")


if __name__ == "__main__":
    main()
