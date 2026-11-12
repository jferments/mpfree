// Functions the manual's pages call (body.typ imports this file).

#let ink = rgb("#1b1f27")
#let muted = rgb("#5d6675")
#let accent = rgb("#3d7be0")
#let mpc = rgb("#d9921a")
#let warn = rgb("#e0663e")
#let rule-c = rgb("#d6dbe3")

// Parts: a divider page.
#let part(title) = {
  pagebreak(weak: true)
  page(header: none, footer: none)[
    #v(1fr)
    #text(size: 11pt, fill: accent, weight: "semibold", tracking: 0.2em)[PART]
    #v(0.2cm)
    #text(size: 34pt, weight: "semibold")[#title]
    #place(hide(figure(kind: "part", supplement: none, numbering: none, outlined: true,
      caption: title, [])))
    #v(2fr)
  ]
}

// Boxes from the pages: > [!MPC], > [!NOTE], > [!TIP], > [!WARNING].
#let mpfbox(kind, body) = {
  let c = if kind == "mpc" { mpc } else if kind == "warning" { warn } else { accent }
  block(width: 100%, inset: (x: 12pt, y: 10pt), radius: 4pt, fill: c.lighten(88%),
    stroke: (left: 3pt + c), breakable: true, body)
}
// The banner under a page's title: changing or planned.
#let mpfstatus(kind, body) = {
  let c = if kind == "planned" { muted } else { warn }
  block(width: 100%, inset: (x: 12pt, y: 8pt), radius: 4pt, fill: c.lighten(90%),
    stroke: (left: 3pt + c), text(size: 9pt, body))
}
// A screenshot drawn by mpfree --preview.
#let shot(path, caption) = figure(
  box(radius: 4pt, clip: true, stroke: 0.5pt + rule-c, image(path, width: 100%)),
  caption: if caption == [] { none } else { caption },
  supplement: [Screen],
)

// What Pandoc's Typst writer calls.
#let blockquote(body) = block(inset: (left: 12pt), stroke: (left: 2pt + rule-c), body)
#let horizontalrule = line(length: 100%, stroke: 0.5pt + rule-c)
