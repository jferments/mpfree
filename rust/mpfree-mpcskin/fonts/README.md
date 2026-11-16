# Fonts for MPC plugin skins

An MPC plugin's skin names its fonts: "Titillium Web" (Akai's own screens' font) or "Roboto", in weights from Regular to Bold. MPC OS has them built in, so skins never carry them. MPFree draws skins with these subsets.

| File | Made from | Glyphs | SHA-256 |
| --- | --- | --- | --- |
| `TitilliumWeb-Regular-Latin.ttf` | Titillium Web Regular 1.002 (google/fonts `ofl/titilliumweb`, SHA-256 `7b6b4452…517ea5`) | 329 | `252ca5d63698e0ff5a677e71a5bbf3a582c6c26d42ec12f9aa6f51dec62d0d6f` |
| `TitilliumWeb-SemiBold-Latin.ttf` | Titillium Web SemiBold 1.002 (SHA-256 `7c92f153…648c81`) | 329 | `e717a42bda7f1ea3a8ad8664e2bea0525ba6a94bf5071fdbd491cf0846605677` |
| `TitilliumWeb-Bold-Latin.ttf` | Titillium Web Bold 1.002 (SHA-256 `e2e8e181…e13093`) | 329 | `b9d18d69d4e448c2c06f2302fb180647ee741c1543f24a53400f4d79ff3bd276` |
| `Roboto-Regular-Latin.ttf` | Roboto 3.015 variable font (google/fonts `ofl/roboto/Roboto[wdth,wght].ttf`, SHA-256 `d7598e12…72a134`) at weight 400, width 100 | 367 | `5897b40e11dafdd8d5988e868522445234576ddb3ec330c197911d12f91112fc` |
| `Roboto-Medium-Latin.ttf` | the same at weight 500 | 367 | `6381a31ffcb7cce44e4f56c8c84a784f11b6ca7f9cfdf85f13e97b6c73ae361d` |
| `Roboto-SemiBold-Latin.ttf` | the same at weight 600 | 367 | `9820ba00856719bfb8cbfb44da853baabd1465931cbeb402982987acd64f0ee6` |
| `Roboto-Bold-Latin.ttf` | the same at weight 700 | 367 | `97a5b4e63c73c80507febea9fad77665bf66bf9ed00ccf337789f4cfb7f46732` |

Both families are under the SIL Open Font License 1.1 ([`OFL.txt`](OFL.txt)), which allows bundling them with MIT software as long as this licence travels with them. Neither declares a Reserved Font Name. The fonts stay under the OFL; the rest of MPFree stays MIT.

They are subsets (a "Modified Version" under the OFL), made as the GUI's own fonts are ([`mpfree-ui/fonts/README.md`](../../mpfree-ui/fonts/README.md)): the Roboto weights first instanced from the variable font with `fonttools varLib.instancer Roboto[wdth,wght].ttf wght=400 wdth=100`, then

```bash
U="U+0020-007E,U+00A0-017F,U+2010-2027,U+2030-203A,U+2190-2193,U+2212,U+25CF,U+2713,U+20AC"
pyftsubset TitilliumWeb-Regular.ttf --unicodes="$U" --name-IDs='*' --name-languages='*' --name-legacy \
  --no-hinting --layout-features='kern,liga,calt' --output-file=TitilliumWeb-Regular-Latin.ttf
```

A skin that names another font (Akai's "Nexa", for one) is drawn in Titillium Web, at the same height.
