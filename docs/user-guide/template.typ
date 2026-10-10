// Layout of the PrePolix user guide. Only fonts embedded in Typst are used, so the PDF
// looks the same on every machine (`typst compile --ignore-system-fonts`).

#let accent = rgb("#2b5797")

// Menu path, e.g. #menu("File", "Open...").
#let menu(..items) = {
  let parts = items.pos().map(it => text(weight: "bold", it))
  parts.join([ #sym.arrow.r ])
}

// Name of a dialog, button or field as it appears in the program.
#let ui(name) = text(weight: "bold", name)

// Keyboard key or combination, e.g. #key("Ctrl+O").
#let key(name) = box(
  inset: (x: 3pt, y: 0pt),
  outset: (y: 2pt),
  stroke: 0.5pt + luma(150),
  radius: 2pt,
  fill: luma(245),
  text(size: 0.85em, font: "DejaVu Sans Mono", name),
)

// Short boxed remark: a hint, a limitation or a CalculiX detail.
#let note(title: "Note", body) = block(
  width: 100%,
  inset: 8pt,
  radius: 3pt,
  fill: rgb("#eef3fa"),
  stroke: (left: 2pt + accent),
  [#text(weight: "bold", fill: accent, title) #h(0.4em) #body],
)

#let limitation(body) = note(title: "Limitation", body)

// Table of dialog fields: pairs of (field, description).
#let fields(..rows) = table(
  columns: (30%, 1fr),
  stroke: (x, y) => if y == 0 { (bottom: 0.7pt) } else { (bottom: 0.3pt + luma(200)) },
  inset: (x: 5pt, y: 4pt),
  table.header([*Field*], [*Meaning*]),
  ..rows.pos().flatten(),
)

#let guide(version: "dev", body) = {
  set document(title: "PrePolix User Guide", author: "PrePolix contributors")
  set text(font: "Libertinus Serif", size: 10.5pt, lang: "en")
  set par(justify: true, leading: 0.6em)
  set heading(numbering: "1.1")
  show raw: set text(font: "DejaVu Sans Mono", size: 0.9em)
  show link: set text(fill: accent)

  show heading.where(level: 1): it => {
    pagebreak(weak: true)
    v(2em)
    text(size: 22pt, fill: accent, it)
    v(1em)
  }
  show heading.where(level: 2): set text(size: 14pt, fill: accent)
  show heading.where(level: 3): set text(size: 11.5pt)

  // Title page.
  page(margin: 3cm)[
    #v(25%)
    #text(size: 36pt, weight: "bold", fill: accent)[PrePolix]
    #v(0.2em)
    #text(size: 18pt)[User Guide]
    #v(1em)
    #text(size: 12pt)[A pre- and post-processor for CalculiX, modelled on PrePoMax]
    #v(1fr)
    #text(size: 11pt)[Version #version]
  ]

  set page(
    paper: "a4",
    margin: (x: 2.2cm, y: 2.5cm),
    numbering: "1",
    header: context {
      if counter(page).get().first() > 2 {
        set text(size: 9pt, fill: luma(110))
        [PrePolix User Guide #h(1fr) Version #version]
      }
    },
  )
  counter(page).update(1)

  outline(depth: 2, indent: auto)
  body
}

// A screenshot from images/, shown at the same scale as all others: 0.55 pt per pixel, at most
// the text width. Full-window shots therefore fill the line and dialogs stay readable.
#let screenshot(name, caption) = figure(
  layout(size => {
    let natural = measure(image("images/" + name)).width
    image("images/" + name, width: calc.min(natural * 0.55, size.width))
  }),
  caption: caption,
)
