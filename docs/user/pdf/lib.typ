// The PDF manual's design: colours, type, and the functions the pages call
// (body.typ imports this file; manual.typ sets up the pages around it).

// ------------------------------------------------------------ colours

#let ink = rgb("#161a21")
#let body-ink = rgb("#262b33")
#let muted = rgb("#5f6877")
#let faint = rgb("#8a93a2")
#let accent = rgb("#2f6fd6")
#let accent-soft = rgb("#e9f0fc")
#let mpc = rgb("#b8730c")
#let warn = rgb("#c4502c")
#let rule-c = rgb("#dde1e8")
#let tint = rgb("#f5f7fa")
#let night = rgb("#0e1116")

#let sans = ("Inter",)
#let display = ("Inter Display", "Inter")
#let mono = ("JetBrains Mono",)

// ------------------------------------------------------------ parts

#let part-counter = counter("mpf-part")

// A part opens on a dark page with its number, title and chapters.
#let part(title) = {
  pagebreak(weak: true)
  part-counter.step()
  page(header: none, footer: none, fill: night, margin: (x: 2.6cm, y: 2.6cm))[
    #set text(fill: white)
    #place(hide(figure(kind: "part", supplement: none, numbering: none, outlined: true,
      caption: title, [])))
    #v(1fr)
    #context text(font: display, size: 13pt, weight: "semibold", fill: rgb("#6aa6ff"),
      tracking: 0.24em)[PART #part-counter.display("I")]
    #v(0.35cm)
    #text(font: display, size: 44pt, weight: "semibold", tracking: -0.01em)[#title]
    #v(0.5cm)
    #line(length: 3.2cm, stroke: 2pt + rgb("#6aa6ff"))
    #v(0.8cm)
    // The chapters in this part, up to the next part page.
    #context {
      let here-loc = here()
      let next = query(figure.where(kind: "part")).filter(f => f.location().page() > here-loc.page())
      let stop = if next.len() > 0 { next.first().location().page() } else { 100000 }
      let chapters = query(heading.where(level: 1)).filter(h =>
        h.location().page() > here-loc.page() and h.location().page() < stop and h.numbering != none)
      set text(size: 10.5pt, fill: rgb("#c9d1dc"))
      grid(columns: (2.2em, 1fr, auto), row-gutter: 0.75em,
        ..chapters.map(h => (
          text(fill: rgb("#6aa6ff"), weight: "semibold", numbering("1", ..counter(heading).at(h.location()))),
          link(h.location(), h.body),
          text(fill: rgb("#8b95a5"), str(counter(page).at(h.location()).first())),
        )).flatten())
    }
    #v(2fr)
  ]
}

// ------------------------------------------------------------ boxes

// Boxes from the pages: > [!MPC], > [!NOTE], > [!TIP], > [!WARNING], > [!IMPORTANT].
#let box-style = (
  mpc: (mpc, "COMING FROM AN MPC"),
  note: (accent, "NOTE"),
  tip: (rgb("#2b8a5e"), "TIP"),
  warning: (warn, "CAREFUL"),
  important: (warn, "IMPORTANT"),
)
#let mpfbox(kind, body) = {
  let (c, label) = box-style.at(kind, default: (accent, upper(kind)))
  block(width: 100%, inset: (left: 14pt, right: 14pt, top: 10pt, bottom: 11pt), radius: (right: 3pt),
    fill: c.lighten(92%), stroke: (left: 2.5pt + c), breakable: true, above: 1.3em, below: 1.3em, {
      text(size: 7.5pt, weight: "bold", fill: c.darken(10%), tracking: 0.14em, label)
      v(0.45em, weak: true)
      set text(size: 9.25pt)
      set par(spacing: 0.8em)
      body
    })
}

// The banner under a page's title: changing or planned.
#let mpfstatus(kind, body) = {
  let c = if kind == "planned" { muted } else { warn }
  let label = if kind == "planned" { "PLANNED" } else { "CHANGING" }
  block(width: 100%, above: 0pt, below: 1.6em, grid(columns: (auto, 1fr), column-gutter: 10pt, align: horizon,
    box(inset: (x: 6pt, y: 3.5pt), radius: 2pt, fill: c,
      text(size: 7pt, weight: "bold", fill: white, tracking: 0.14em, label)),
    text(size: 8.5pt, fill: muted, body)))
}

// ------------------------------------------------------------ screens

// A screenshot drawn by mpfree --preview, numbered within its chapter.
#let shot(path, caption) = figure(
  box(radius: 5pt, clip: true, stroke: 0.6pt + rule-c, image(path, width: 100%)),
  caption: if caption == [] { none } else { caption },
  kind: image,
  supplement: [Screen],
)

// ------------------------------------------------------------ tables

// Tables come from filter.lua: the columns' widths, a header row, then the
// cells row by row. The header repeats when a table runs onto a new page.
#let mtable(widths, head, ..cells) = {
  set par(justify: false)
  block(width: 100%, above: 1.2em, below: 1.4em, table(
    columns: widths,
    inset: (x: 7pt, y: 5.5pt),
    align: left + top,
    stroke: (x, y) => (
      top: if y == 0 { none } else if y == 1 { 0.8pt + ink } else { 0.5pt + rule-c },
      bottom: none,
    ),
    fill: (x, y) => if y == 0 { none } else if calc.even(y) { tint } else { none },
    table.header(..head.map(c => text(size: 7.5pt, weight: "bold", fill: muted, tracking: 0.08em, upper(c)))),
    ..cells.pos(),
  ))
}

// ------------------------------------------------------------ buttons

// The MPC's buttons, drawn as keys when a page writes them in bold
// (**MENU**, **SHIFT + MENU**). Jacks, knobs and screen labels stay bold text.
#let buttons = (
  "MENU", "UNDO", "REC", "PLAY", "PLAY START", "STOP", "OVERDUB", "SHIFT", "NOTE REPEAT",
  "TAP TEMPO", "TRACK MIX", "TRACK MUTE", "MAIN", "STEP SEQ", "NEXT SEQ", "SAMPLER",
  "SAMPLE EDIT", "PROG EDIT", "Q-LINK", "FULL LEVEL", "16 LEVEL", "TC", "BROWSE", "ERASE",
  "COPY", "PAD BANK", "PAD BANK A", "PAD BANK B", "PAD BANK C", "PAD BANK D", "PAD BANK A–D",
  "PAD BANK A-D", "–", "+", "-",
)
#let key(name) = box(inset: (x: 3.2pt, y: 0pt), outset: (y: 2.4pt), radius: 2.2pt,
  fill: white, stroke: (paint: rgb("#b8bfca"), thickness: 0.6pt),
  text(size: 0.74em, weight: "semibold", fill: ink, tracking: 0.04em, name))
#let plain-text(it) = {
  if it.has("text") { it.text }
  else if it == [ ] { " " }
  else if it.has("children") {
    let parts = it.children.map(plain-text)
    if parts.any(p => p == none) { none } else { parts.join() }
  } else { none }
}
#let as-keys(body) = {
  let s = plain-text(body)
  if s == none { return none }
  let names = s.trim().split(" + ")
  if names.len() >= 1 and names.all(n => n in buttons) {
    names.map(key).join(h(2pt) + text(size: 0.85em, fill: muted, "+") + h(2pt))
  } else { none }
}
