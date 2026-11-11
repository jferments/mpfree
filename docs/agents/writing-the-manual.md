# Writing the user manual

**Status:** 2026-11-11. How to write pages in `docs/user/src/`. The mechanics (headers, checks, builds) are in [how the docs stay current](docs-system.md).

## Who reads it

A musician who has never used an MPC, holding one. They want to make music, not learn the software's insides. Someone who knows the factory MPC software should still find it quick to skim.

## Voice

- Plain words and full sentences. Say "you". Present tense: "REC records a clip", not "REC will record".
- Lead with what something is for and when you'd use it, then how.
- One idea per paragraph. Short pages over long ones.
- No code, crate names, file paths, thread names or development history. A musician doesn't need to know which round added a feature.
- Name controls by their printed label in bold capitals (**REC**, **NOTE REPEAT**, **SHIFT + MENU**) and screen items by their on-screen name (the Beats view, the Q-Link column).
- Don't promise dates. A thing that isn't built yet goes on a `planned` page or in a sentence marked "coming".

## Structure of a page

1. The header comment (`status`, `covers`, `checked`).
2. `# Title`, in sentence case, naming what the reader wants to do ("Recording loops").
3. One or two sentences on what the page is for.
4. A screenshot, if the page is about a screen: `{{shot: NAME | Caption}}`.
5. Short sections (`##`) in the order a musician would do things. Steps as numbered lists.
6. Exact button combinations live in [Buttons and controls](../user/src/reference/controls.md); a page can name a button, but the full table is there.

## Boxes

```markdown
> [!MPC]
> On the MPC, ... In MPFree, ...
```

- `[!MPC]` "Coming from an MPC": where MPFree differs from the factory software. Write one wherever a difference matters, in the chapter it belongs to; the build collects them all into the appendix.
- `[!TIP]`, `[!NOTE]`, `[!WARNING]`: use sparingly. A warning is for something that can lose work or be loud.

## Screens

Screens come from `mpfree-hello --preview`, which draws each page with sample data. To show a screen the manual needs (a menu open, a view with data in it), add it to `gui::sample_screens` in `rust/mpfree-hello/src/gui.rs`, in the same pull request. Never commit a hand-made screenshot or photo of the screen: it goes stale the next time the page is redrawn.
