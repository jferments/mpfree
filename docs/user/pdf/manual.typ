// The MPFree user manual, as a printable PDF.
// build_manual.py writes body.typ (the pages, converted by Pandoc) next to
// this file and compiles it with the commit and date as inputs.
//
// The design: A4, one 14.6 cm column of 10 pt Inter (about 80 characters a
// line), chapters opening on a page of their own with what's in them, parts
// on dark divider pages, buttons drawn as keys, and screens numbered within
// their chapter. lib.typ holds the colours and the pieces the pages use.

#let commit = sys.inputs.at("commit", default: "unknown")
#let date = sys.inputs.at("date", default: "")

#import "lib.typ": *

#set document(title: "MPFree User Manual", author: "The MPFree contributors",
  keywords: ("MPFree", "Akai MPC One", "manual"))
#set text(font: sans, size: 10pt, fill: body-ink, lang: "en", region: "GB",
  hyphenate: true, costs: (hyphenation: 120%, runt: 200%, widow: 300%, orphan: 300%))
#set par(justify: true, leading: 0.72em, spacing: 1.05em, linebreaks: "optimized")
#show raw: set text(font: mono, size: 0.9em)
#show raw.where(block: false): it => box(fill: tint, inset: (x: 2.5pt), outset: (y: 2pt), radius: 2pt, it)
#show raw.where(block: true): it => block(width: 100%, fill: tint, inset: 10pt, radius: 3pt, it)
// Links in the text are blue; the contents, part pages and chapter openers
// link by location and keep their own colours.
#show link: it => if type(it.dest) == location { it } else { text(fill: accent, it) }
#show strong: it => {
  let k = as-keys(it.body)
  if k != none { k } else { text(weight: "semibold", fill: ink, it.body) }
}
#show emph: set text(style: "italic")

// ------------------------------------------------------------ cover

#page(margin: 0pt, fill: night, header: none, footer: none)[
  // A thin accent stripe down the spine side.
  #place(top + left, rect(width: 0.5cm, height: 100%, fill: accent))
  #place(top + left, dx: 2.6cm, dy: 3.0cm, block(width: 15.8cm)[
    #set par(justify: false, leading: 0.3em)
    #text(font: display, fill: rgb("#6aa6ff"), size: 11pt, weight: "semibold", tracking: 0.26em)[FOR THE AKAI MPC ONE]
    #v(0.7cm)
    #text(font: display, fill: white, size: 68pt, weight: "bold", tracking: -0.02em)[MPFree]
    #v(0.5cm)
    #text(font: display, fill: rgb("#c9d1dc"), size: 28pt, weight: "light")[User Manual]
    #v(1.6cm)
    #box(width: 100%, clip: true, radius: 8pt, stroke: 1pt + rgb("#283040"),
      image("shots/play-all-loop.png", width: 100%))
    #v(0.2cm)
    #text(fill: rgb("#8b95a5"), size: 8.5pt)[The Play page in the middle of a jam: a beat, a three-layer loop, a throw held and the master meter moving.]
  ])
  #place(bottom + left, dx: 2.6cm, dy: -2.4cm)[
    #set text(fill: rgb("#8b95a5"), size: 9pt)
    #grid(columns: (auto, auto), column-gutter: 1.6cm, row-gutter: 0.45em,
      text(fill: white, weight: "semibold")[Preview edition],
      text(fill: white, weight: "semibold")[Free and open-source software],
      [Built from #text(font: mono, size: 8.5pt, commit)#if date != "" [ on #date]],
      [Early test version: details will change],
    )
  ]
]

// ------------------------------------------------------------ the inside cover

#page(header: none, footer: none, margin: (x: 3.2cm, top: 3.2cm, bottom: 3cm))[
  #set par(justify: false)
  #set text(size: 9pt)
  #v(1fr)
  #text(size: 8pt, weight: "bold", fill: muted, tracking: 0.14em)[HOW THIS MANUAL SHOWS THINGS]
  #v(0.8em)
  #grid(columns: (4.2cm, 1fr), row-gutter: 1.1em, column-gutter: 0.6cm, align: (left + horizon, left + horizon),
    [#key("SHIFT") #h(2pt) #text(size: 0.85em, fill: muted)[+] #h(2pt) #key("MENU")],
    [A button on the MPC. With a #text(fill: muted)[+], hold the first button, then press the second.],
    text(weight: "semibold", fill: ink)[Play, Tempo],
    [Something on the screen: a tab, a button you tap, a setting.],
    text(fill: accent)[Your first beat],
    [A link to another part of the manual. Click it in the PDF.],
    box(inset: (x: 6pt, y: 3.5pt), radius: 2pt, fill: warn,
      text(size: 7pt, weight: "bold", fill: white, tracking: 0.14em)[CHANGING]),
    [The chapter describes a part of MPFree that works but is still being designed.],
    box(width: 100%, inset: (left: 8pt, y: 5pt), fill: mpc.lighten(92%), stroke: (left: 2.5pt + mpc),
      text(size: 7pt, weight: "bold", fill: mpc.darken(10%), tracking: 0.14em)[COMING FROM AN MPC]),
    [Where MPFree differs from the MPC's own software. All of them are collected at the back.],
  )
  #v(2cm)
  #line(length: 100%, stroke: 0.5pt + rule-c)
  #v(0.6em)
  #text(fill: muted, size: 8.5pt)[
    *MPFree User Manual*, preview edition. Describes MPFree as of commit #raw(commit)#if date != "" [ (#date)]. The screens in it are drawn by MPFree itself from the same commit. \
    MPFree is free software under the MIT licence. Akai Professional and MPC are trademarks of their owners; MPFree is not made, approved or supported by Akai Professional. \
    Set in Inter and JetBrains Mono.
  ]
]

// ------------------------------------------------------------ body pages

// The chapter a page belongs to: the last chapter heading on or before it.
#let chapter-here() = {
  let before = query(heading.where(level: 1).before(here()))
  if before.len() > 0 { before.last() } else { none }
}
#let opens-chapter() = {
  let p = here().page()
  query(heading.where(level: 1)).any(h => h.location().page() == p)
}

#set page(
  paper: "a4",
  margin: (top: 2.6cm, bottom: 2.6cm, inside: 3.1cm, outside: 3.3cm),
  header-ascent: 40%,
  footer-descent: 40%,
  header: context {
    if counter(page).get().first() >= 1 and not opens-chapter() {
      let h = chapter-here()
      if h != none {
        set text(size: 7.5pt, fill: faint, tracking: 0.08em)
        let n = if h.numbering != none { numbering("1", ..counter(heading).at(h.location())) + "   " } else { "" }
        let label = upper(n + plain-text(h.body))
        let odd = calc.odd(here().page())
        grid(columns: (1fr, 1fr),
          align(left, if odd { [MPFREE USER MANUAL] } else { label }),
          align(right, if odd { label } else { [MPFREE USER MANUAL] }))
        v(-0.4em)
        line(length: 100%, stroke: 0.4pt + rule-c)
      }
    }
  },
  footer: context {
    set text(size: 8.5pt, fill: muted, weight: "medium", number-type: "lining")
    let n = counter(page).display()
    if calc.odd(here().page()) { align(right, n) } else { align(left, n) }
  },
)
#counter(page).update(1)

// Headings
#set heading(numbering: (..n) => {
  let n = n.pos()
  if n.first() > 0 and n.len() <= 2 { numbering("1.1", ..n) }
})
#show heading: set text(font: sans, fill: ink, weight: "semibold", hyphenate: false)
#show heading: set par(justify: false)

// A chapter opens on a new page: its number, its title, and what's in it.
#show heading.where(level: 1): it => {
  pagebreak(weak: true)
  counter(figure.where(kind: image)).update(0)
  v(2.2cm)
  if it.numbering != none {
    text(font: display, size: 64pt, weight: "light", fill: accent,
      numbering("1", ..counter(heading).at(it.location())))
    v(0.1cm)
  }
  block(text(font: display, size: 30pt, weight: "semibold", tracking: -0.01em, it.body))
  v(0.25cm)
  line(length: 2.4cm, stroke: 2pt + accent)
  v(0.5cm)
  // In this chapter: its sections, with page numbers.
  context {
    let me = counter(heading).at(it.location()).first()
    let sections = query(heading.where(level: 2))
      .filter(h => counter(heading).at(h.location()).first() == me)
    if sections.len() >= 3 {
      set text(size: 8.5pt)
      set par(justify: false)
      block(width: 100%, inset: (y: 0.2em), below: 1.8em, {
        text(size: 7pt, weight: "bold", fill: faint, tracking: 0.14em)[IN THIS CHAPTER]
        v(0.5em)
        let entry(s) = block(link(s.location(), grid(columns: (1fr, auto), column-gutter: 6pt,
          text(fill: body-ink, weight: "regular", s.body), text(fill: faint, str(counter(page).at(s.location()).first())))))
        let half = calc.ceil(sections.len() / 2)
        grid(columns: (1fr, 1fr), column-gutter: 1.2cm,
          stack(spacing: 0.62em, ..sections.slice(0, half).map(entry)),
          stack(spacing: 0.62em, ..sections.slice(half).map(entry)))
      })
    } else { v(0.6cm) }
  }
}
#show heading.where(level: 2): it => {
  block(above: 1.9em, below: 0.85em, sticky: true, {
    if it.numbering != none {
      box(width: 0pt, place(right, dx: -0.6em,
        text(size: 12.5pt, weight: "medium", fill: accent, numbering("1.1", ..counter(heading).at(it.location())))))
    }
    text(size: 13.5pt, it.body)
  })
}
#show heading.where(level: 3): it => block(above: 1.6em, below: 0.7em, sticky: true,
  text(size: 10.5pt, it.body))
#show heading.where(level: 4): it => block(above: 1.3em, below: 0.6em, sticky: true,
  text(size: 10pt, fill: muted, it.body))

// Screens: numbered within the chapter, floated to the top or bottom of a
// page when they don't fit where they are, so pages don't end in a gap.
#set figure(placement: auto, gap: 0.8em)
#set figure(numbering: n => {
  let c = counter(heading).get().first()
  if c > 0 { numbering("1.1", c, n) } else { numbering("1", n) }
})
#show figure.where(kind: image): set block(above: 1.4em, below: 1.4em)
#set place(clearance: 1.6em)
#show figure.caption: it => {
  set text(size: 8.25pt, fill: muted)
  set par(justify: false)
  let label = if it.numbering == none { [] } else {
    [#text(weight: "semibold", fill: ink)[#it.supplement #context it.counter.display(it.numbering)] #h(0.4em)]
  }
  block(width: 100%, align(left, label + it.body))
}

#set list(indent: 0.2em, body-indent: 0.7em, spacing: 0.75em, marker: (text(fill: accent, "•"), text(fill: faint, "–")))
#set enum(indent: 0.2em, body-indent: 0.6em, spacing: 0.75em,
  numbering: n => text(fill: accent, weight: "semibold", str(n) + "."))
#show terms: set terms(separator: [: ])

// ------------------------------------------------------------ contents

#page(header: none)[
  #v(1.4cm)
  #text(font: display, size: 30pt, weight: "semibold")[Contents]
  #v(0.25cm)
  #line(length: 2.4cm, stroke: 2pt + accent)
  #v(0.8cm)
  #set par(justify: false)
  #show outline.entry: it => {
    if it.element.func() == figure {
      v(1.2em)
      text(size: 7.5pt, weight: "bold", fill: accent, tracking: 0.16em, upper(it.element.caption.body))
      v(0.1em)
    } else if it.level == 1 {
      v(0.55em)
      link(it.element.location(), grid(columns: (2em, 1fr, auto), align: (left, left, right),
        text(weight: "semibold", fill: accent, it.prefix()),
        text(weight: "semibold", fill: ink, it.body()),
        text(weight: "semibold", fill: ink, it.page())))
    } else if plain-text(query(heading.where(level: 1).before(it.element.location())).last().body) in ("Coming from an MPC", "Glossary") {
      // The appendix's sections repeat the chapter names: leave them out.
      []
    } else {
      set text(size: 8.75pt, fill: muted)
      link(it.element.location(), grid(columns: (2em, 2.8em, 1fr, auto), align: (left, left, left, right),
        [], it.prefix(), [#it.body() #box(width: 1fr, repeat(gap: 0.3em)[#text(fill: rule-c)[.]])], h(0.4em) + it.page()))
    }
  }
  #outline(title: none, depth: 2, indent: 0pt,
    target: heading.where(level: 1).or(heading.where(level: 2)).or(figure.where(kind: "part")))
]

#include "body.typ"
