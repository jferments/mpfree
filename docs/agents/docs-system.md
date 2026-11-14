# How the docs stay current

**Status:** proposal by the docs thread, 2026-11-11, waiting for the owner's review in the pull request that adds it.

MPFree changes every day: screens get redrawn, tabs get renamed, buttons get new jobs. Documentation that is written once and fixed later goes stale within a week. These rules make keeping it current a small, routine part of every change, and let a machine find what went stale. They apply to people and to agents.

## 1. Where a page goes

| Audience | Folder | Holds |
|---|---|---|
| Musicians and producers | `docs/user/` | The user manual: what to press and why, in plain words. No crate names, no code, no device research. |
| Developers | `docs/dev/` | `project/` (architecture, roadmap, log, policy), `design/` (one page per engine or subsystem), `decisions/` (ADRs), `guides/` (build, test, device runbooks), `hardware/`, `milestones/`, `perf/`, `research/`. |
| AI agents | `docs/agents/` | How agents work in this repository: safety, process, checks, and this page. `CLAUDE.md` at the root points here. |

Notes that only matter to the people and agents running the project (thread handoffs, private memory, device logs, anything with a serial number) go in the private `jferments/mpfree-claude` repository, never here.

A new page goes in one of those folders and gets a line in that folder's index (`docs/user/src/SUMMARY.md`, `docs/dev/README.md` or `docs/agents/README.md`). `check_docs.py` fails if a page is in none of the indexes. Pages in a dated archive (`docs/dev/hardware/archive/`) are kept byte for byte: they count as listed when their archive's README links them, and their old relative links are not checked.

## 2. One fact, one place

The manual describes the same buttons, tabs and screens many times. If each mention is typed by hand, a rename means hunting through every chapter. So:

- **Screenshots are drawn from the code, never captured by hand.** `mpfree --preview DIR` draws every page of the app from the current source. The manual build runs it and uses those pictures, so a rebuilt manual always shows today's screens. A page asks for one with `{{shot: play-beats}}`. If a screen is renamed or removed, the build stops and names the page that asked for it. To add a picture the manual needs, add a screen to `gui::sample_screens` in `rust/mpfree-app/src/gui.rs` (the same list the golden screen checks use).
- **Button combinations live in one table:** `docs/user/src/reference/controls.md`. Chapters describe what a button is for in their own words; the exact chord ("hold SHIFT, hold ERASE, press TAP TEMPO") is written once there and linked.
- **Names live in one list:** `docs/user/terms.toml` holds the names of tabs, pages and controls, and the names that were retired (Clips is now Arrange, Sound is Synth, the Debug tab is Dev). `check_docs.py` fails on a retired name anywhere in the manual, so a rename is one edit to `terms.toml` and the check lists every page to fix.
- **"Coming from an MPC" notes are written where they belong** (in the chapter, as a `> [!MPC]` box) and the build collects every one of them into the appendix "Coming from an MPC". Nobody keeps that appendix by hand.

## 3. Every page says how fresh it is

Every page in `docs/user/src/` and `docs/dev/design/` starts with a header comment:

```markdown
<!--
status: changing
covers: rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/play_widgets.rs
checked: 2026-11-11 414faa1
-->
```

- **status** (user pages only): `ready` (works and is unlikely to move), `changing` (works, but the screen or controls are still being designed), `planned` (not built yet; the page says what is coming). The manual shows `changing` and `planned` as a banner at the top of the page, so a reader knows what to trust. Developer design pages keep their own `**Status:**` line, and may add the comment for `covers` and `checked`; without one, `doc_drift.py` takes the `rust/...` crates the page's title and first paragraphs name.
- **covers:** the source files or folders whose behaviour the page describes.
- **checked:** the date and commit at which someone last read the page against the code.

`docs/tools/doc_drift.py` compares each page's `covers` with what changed since its `checked` commit (or with a pull request's diff) and lists the pages that may now be wrong. CI prints that list on every pull request. It is a reminder, not a failure: a change to a covered file often needs no doc change, and then the author just updates `checked`.

## 4. Docs change in the same pull request as the code

- A pull request that changes what a musician sees or does (a screen, a button, a sound, a name) updates the user manual page that covers it, or says in its description why no page needs to change. A pull request that adds a new feature adds a page, even a short `changing` one.
- A pull request that changes how an engine works updates its design page under `docs/dev/design/`.
- The feature's own thread writes these updates, because it knows the change best. The docs thread owns the structure, the style, the tools and the PDF, and reviews manual pages for plain language.
- Layout and button changes still need the owner's approval in the layout spec first ([layout spec](../dev/design/layout-spec.md)); the manual follows the spec, never leads it.

## 5. Write so it survives change

For pages marked `changing`:

- Lead with **what something is for** and **when you'd use it** ("the looper records what you play, in time, so you can layer parts"), which stays true across redesigns. Keep pixel-level detail ("the third card on the right") to a minimum.
- Point at the screenshot rather than describing the layout in words; the screenshot updates itself.
- Name a control by its printed label (REC, NOTE REPEAT) or its on-screen name, never by position.
- Write short pages. A short page is quick to check when its `covers` change.

## 6. The checks

```sh
python3 docs/tools/check_docs.py            # links, headers, indexes, terms, screenshot names
python3 docs/tools/doc_drift.py --base origin/main   # pages whose covered code changed
python3 docs/tools/build_manual.py          # the manual as a website and a PDF (needs mdbook, typst)
```

`check_docs.py` and `doc_drift.py` need only Python 3, and CI runs them on every pull request. `build_manual.py` is described in [docs/user/README.md](../user/README.md).

### The PDF's design

The PDF is typeset by Typst from the same pages, so a page never needs PDF-only markup. What decides how it looks:

- `docs/user/pdf/manual.typ`: the cover, the inside cover (how the manual shows things, and the colophon), the page frame (A4, running heads, page numbers on the outer edge), chapter openers with their "In this chapter" list, the contents, and screens numbered per chapter and floated to the top or bottom of a page.
- `docs/user/pdf/lib.typ`: colours, part pages, the callout boxes and status banners, tables, and the **buttons drawn as keys**. A bold run of words is drawn as keys only when every name in it (split on ` + `) is in the `buttons` list there, so `**SHIFT + MENU**` becomes two keys while `**INPUTS**` (a jack) stays bold text. Add a new hardware button to that list when a page starts naming it.
- `docs/tools/build_manual.py` gives every section in the PDF an id made of its page and its anchor, so the same heading on two pages (Pad pressure, Coming next) is two places, and a link to `page.md#section` lands on that section. A link to a section that doesn't exist stops the PDF build, which makes the build the check for section links.
- `docs/user/pdf/filter.lua`: turns boxes, banners, screenshots and tables into calls to `lib.typ`. A table's column widths follow the length of its text; a first column of short labels gets the width it needs.
- `docs/user/pdf/fonts/`: Inter (with italic and the Display cuts) and JetBrains Mono. Typst sees only this folder, so the PDF is the same on every machine.

After changing any of these, build the PDF and look at every page (for example `pdftoppm -r 50 -png` and a contact sheet): a gap at the foot of a page, a table squeezed into one narrow column, or a key that wraps are the usual faults.

## 7. Branches made before the move (2026-11-11)

The docs moved from `docs/{project,guides,hardware,milestones,perf,research,decisions}` to `docs/dev/...`. A branch made earlier runs, after merging main:

```sh
python3 docs/tools/migrate_doc_paths.py
```

It moves any page still at an old path and rewrites links and `docs/...` mentions in code comments. It is safe to run twice. `docs/project/*.md` pages land in `docs/dev/design/`, except architecture, roadmap, development-log and factory-content-policy, which go to `docs/dev/project/`.
