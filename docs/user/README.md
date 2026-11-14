# The MPFree user manual (source)

This folder is the source of MPFree's user manual, for musicians and producers. The same pages build a website and a printable PDF.

**To read the manual,** start at [src/index.md](src/index.md), or build it (below). The chapters are listed in [src/SUMMARY.md](src/SUMMARY.md).

**To write or update a page,** read [how the docs stay current](../agents/docs-system.md) and [writing the user manual](../agents/writing-the-manual.md).

## Layout

| Path | What |
|---|---|
| `src/SUMMARY.md` | The chapters, in order. Every page must be listed. |
| `src/getting-started/`, `src/making-music/`, `src/reference/` | The pages. |
| `terms.toml` | Current and retired names; `check_docs.py` fails on a retired one. |
| `book.toml`, `theme/` | The website (mdBook). |
| `pdf/` | The PDF: the Typst layout (`manual.typ` for the cover, page frame and chapter openers; `lib.typ` for colours, boxes, tables and the button keys), the Pandoc filter, and its fonts (`fonts/`, Inter and JetBrains Mono). |
| `build/` | Build output (not committed). |

## Building

```sh
cargo install --locked mdbook@0.5.4 typst-cli   # once (about 5 minutes); pandoc 3.1+ and Pillow from your OS
python3 docs/tools/build_manual.py              # draws the screens, then builds both
```

The website lands in `docs/user/build/site/index.html` and the PDF in `docs/user/build/mpfree-manual.pdf`. `--no-pdf` and `--no-site` build one of them; `--shots DIR` reuses screens already drawn with `mpfree --preview DIR`.

The build stops if a page asks for a screen that MPFree no longer draws, so a renamed or removed screen can't leave a stale picture behind.

In the PDF every heading's link target is named for its page as well as its title (`page--heading`), so two pages can use the same heading, and a link to a section that doesn't exist stops the PDF build. The website and the PDF come from the same pages, so check both after moving a section.

To check the pages without building, run `python3 docs/tools/check_docs.py` from the repository's root: it checks links, the page headers, `SUMMARY.md` and the retired names in `terms.toml`.
