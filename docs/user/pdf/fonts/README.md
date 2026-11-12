# Fonts for the PDF manual

The PDF uses the same two families as MPFree's screen, Inter and JetBrains Mono, but needs more of Inter than the GUI carries: an italic, Medium and Bold, and Inter Display (Inter drawn for large sizes) for the cover and chapter titles. `build_manual.py` gives Typst only this folder (`--ignore-system-fonts`), so the PDF looks the same on every machine.

| File | Made from |
| --- | --- |
| `Inter-Regular.ttf`, `Inter-Italic.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf`, `Inter-SemiBoldItalic.ttf`, `Inter-Bold.ttf` | Inter 4.1 (`Inter-4.1.zip` from github.com/rsms/inter releases, `extras/ttf/`) |
| `InterDisplay-Light.ttf`, `InterDisplay-SemiBold.ttf`, `InterDisplay-Bold.ttf` | the same release |
| `JetBrainsMono-Regular-Latin.ttf` | a copy of `rust/mpfree-ui/fonts/JetBrainsMono-Regular-Latin.ttf` |

All are under the SIL Open Font License 1.1 ([`OFL.txt`](OFL.txt)); neither family declares a Reserved Font Name. The Inter files are subsets (Latin, punctuation, arrows, a few symbols; every OpenType feature kept, for tabular figures), made with fontTools:

```bash
U="U+0020-007E,U+00A0-00FF,U+0131,U+0152-0153,U+02C6,U+02DA,U+02DC,U+2000-206F,U+2070-209F,U+20AC,U+2122,U+2190-21FF,U+2212,U+2215,U+2219,U+2248,U+2260,U+2264,U+2265,U+25A0-25FF,U+266D-266F,U+00D7"
pyftsubset extras/ttf/Inter-Regular.ttf --unicodes="$U" --layout-features='*' --output-file=Inter-Regular.ttf
```

(the same for the others).
