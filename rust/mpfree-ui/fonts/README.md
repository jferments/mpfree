# Fonts built into MPFree's GUI

| File | Made from | Glyphs | SHA-256 |
| --- | --- | --- | --- |
| `Inter-Regular-Latin.ttf` | Inter Regular 4.001 (Google Fonts static TTF, SHA-256 `1b08e7fc…a683ba`) | 444 | `1a2da3102b295a7221c8ee8752e1c8436fb53731e29cca6f4a7e7931bc19b150` |
| `Inter-SemiBold-Latin.ttf` | Inter SemiBold 4.001 (Google Fonts static TTF, SHA-256 `e7a1aaf7…e8d4f7`) | 444 | `670e6c37f62cb4ff74892afbaa070102254016e8d2acdf069dc6b197ee9f6ca7` |
| `JetBrainsMono-Regular-Latin.ttf` | JetBrains Mono Regular 2.211 (Google Fonts static TTF, SHA-256 `44ce4a84…9d5dc3`) | 513 | `a5d71984fbd04687178a2a988a1949e6e8442e6c4c2d596ad35d4195cbdbdf61` |

All three are under the SIL Open Font License 1.1 ([`OFL.txt`](OFL.txt)), which allows bundling them with MIT software as long as this licence travels with them. The fonts stay under the OFL; the rest of MPFree stays MIT. Neither font declares a Reserved Font Name.

They are subsets (a "Modified Version" under the OFL): only Latin letters, digits, punctuation, arrows and a few symbols are kept, and hinting is dropped. The whole fonts cost the GUI about 22 MB of RAM once loaded (the rasteriser prepares every glyph); the subsets about 1 MB. They were made with fontTools:

```bash
U="U+0020-007E,U+00A0-017F,U+2010-2027,U+2030-203A,U+2190-2193,U+2212,U+25CF,U+2713,U+20AC"
pyftsubset Inter-Regular.ttf --unicodes="$U" --name-IDs='*' --name-languages='*' --name-legacy \
  --no-hinting --layout-features='kern,liga,calt' --output-file=Inter-Regular-Latin.ttf
```

(the same for the other two).

To try another font without rebuilding, programs that use `mpfree-ui` take `--font FILE`, `--bold-font FILE` and `--mono-font FILE` (any TTF or OTF). Only commit a font whose licence clearly allows redistribution.
