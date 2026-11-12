// The MPFree user manual, as a printable PDF.
// build_manual.py writes body.typ (the pages, converted by Pandoc) next to
// this file and compiles it with the commit and date as inputs.

#let commit = sys.inputs.at("commit", default: "unknown")
#let date = sys.inputs.at("date", default: "")

#import "lib.typ": *

#set document(title: "MPFree User Manual", author: "The MPFree contributors")
#set text(font: "Inter", size: 10pt, fill: ink, lang: "en")
#set par(justify: false, leading: 0.68em, spacing: 1.1em)
#show raw: set text(font: "JetBrains Mono", size: 0.92em)
#show link: set text(fill: accent)
#show strong: set text(weight: "semibold")

// ------------------------------------------------------------ cover

#page(margin: 0pt, fill: rgb("#0e1116"))[
  #place(top + left, dx: 2.2cm, dy: 3.2cm)[
    #text(fill: rgb("#6aa6ff"), size: 12pt, weight: "semibold", tracking: 0.2em)[FOR THE AKAI MPC ONE]
    #v(0.6cm)
    #text(fill: white, size: 54pt, weight: "semibold")[MPFree]
    #v(-0.4cm)
    #text(fill: rgb("#c9d1dc"), size: 24pt)[User Manual]
  ]
  #place(top + left, dx: 2.2cm, dy: 11.2cm)[
    #box(width: 16.6cm, clip: true, radius: 6pt, image("shots/play.png", width: 100%))
  ]
  #place(bottom + left, dx: 2.2cm, dy: -2.2cm)[
    #text(fill: rgb("#8b95a5"), size: 9.5pt)[
      Describes MPFree as of commit #raw(commit) #if date != "" [(#date)]. \
      Free, open-source software. Early test version: details will change.
    ]
  ]
]

// ------------------------------------------------------------ body pages

#let chapter-title() = {
  let before = query(heading.where(level: 1).before(here()))
  if before.len() > 0 { before.last().body } else { [] }
}

#set page(
  paper: "a4",
  margin: (top: 2.4cm, bottom: 2.2cm, inside: 2.4cm, outside: 2.2cm),
  header: context {
    if counter(page).get().first() > 2 {
      set text(size: 8.5pt, fill: muted)
      grid(columns: (1fr, auto), [MPFree User Manual], chapter-title())
      v(-0.3em)
      line(length: 100%, stroke: 0.5pt + rule-c)
    }
  },
  footer: context {
    set text(size: 8.5pt, fill: muted)
    align(center, counter(page).display())
  },
)
#counter(page).update(1)

#set heading(numbering: (..n) => {
  let n = n.pos()
  if n.first() > 0 and n.len() <= 2 { numbering("1.1", ..n) }
})
#show heading: set text(weight: "semibold")
#show heading.where(level: 1): it => {
  pagebreak(weak: true)
  v(1.2cm)
  if it.numbering != none {
    text(size: 11pt, fill: accent, weight: "semibold", tracking: 0.12em)[CHAPTER #counter(heading).display("1")]
    v(0.2cm)
  }
  text(size: 26pt, it.body)
  v(0.5cm)
}
#show heading.where(level: 2): it => {
  v(0.9em)
  text(size: 14pt, it)
  v(0.2em)
}
#show heading.where(level: 3): set text(size: 11.5pt)

#show figure.caption: set text(size: 8.5pt, fill: muted)
#set table(stroke: (x, y) => (bottom: 0.5pt + rule-c, top: if y == 0 { 0.8pt + ink } else { none }),
  inset: (x: 6pt, y: 5pt))
#show table.cell.where(y: 0): set text(weight: "semibold")
#show table: set text(size: 9pt)
#show table: set align(left)
#set list(indent: 0.4em, body-indent: 0.6em)

// ------------------------------------------------------------ contents

#page(header: none)[
  #text(size: 26pt, weight: "semibold")[Contents]
  #v(0.6cm)
  #show outline.entry: it => {
    if it.element.func() == figure {
      v(0.9em)
      text(size: 11pt, weight: "semibold", fill: accent, upper(it.element.caption.body))
    } else if it.level == 1 {
      v(0.25em)
      strong(it)
    } else { it }
  }
  #outline(title: none, depth: 2, indent: auto,
    target: heading.where(level: 1).or(heading.where(level: 2)).or(figure.where(kind: "part")))
]

#include "body.typ"
