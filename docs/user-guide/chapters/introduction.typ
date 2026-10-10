#import "../template.typ": *

= Introduction

PrePolix is a pre- and post-processor for the finite element solver
#link("http://www.calculix.de/")[CalculiX]. It runs on Linux and Windows. Its look, workflow
and feature set follow #link("https://prepomax.fs.um.si/")[PrePoMax], so users of PrePoMax will
find most dialogs where they expect them.

With PrePolix you

- import CAD geometry (STEP, IGES, BREP) and mesh it with Gmsh, or open an existing CalculiX mesh
  (`.inp`),
- define the finite element model: materials, sections, constraints, contacts, steps, boundary
  conditions and loads,
- run CalculiX and watch its progress,
- view and evaluate the results (`.frd`, `.dat`): contour plots, deformed shapes, animations,
  derived fields, history plots, paths, planes and hot spot stresses.

This guide is a reference. Each chapter describes one part of the program: where to find a
function, what every field of its dialog means, and what PrePolix writes to the CalculiX input
file. You do not need to read it from front to back.

== How PrePolix thinks about a model

You never edit node sets, element sets or surfaces directly. Instead you pick faces, edges, parts
or nodes in the 3D view, and the model stores your _intent_: "a pressure of 2 MPa on these
faces", "these two parts are tied". Only when the input file is written does PrePolix turn this
intent into CalculiX keywords: it creates the required node sets, element sets and surfaces with
generated names and splits a distributed load into what CalculiX needs.

This has two consequences you will notice:

- When you remesh a part, loads and boundary conditions that were picked on the CAD geometry
  stay where they are, because they refer to faces and edges of the geometry, not to element
  numbers.
- The `.inp` file contains sets you never named. You can inspect it at any time in the keyword
  editor (@keyword-editor) or export it (#menu("File", "Export CalculiX Input File ...")).

The project, including geometry, mesh and model, is saved as a PrePolix project file (`.plx`).

== Requirements

- *CalculiX* (`ccx`) is needed to run analyses. It is not part of PrePolix. Set its path under
  #menu("Tools", "Settings ...", "CalculiX") and press #ui("Test CalculiX") to check it
  (@settings).
- *Gmsh* is needed to import and mesh CAD geometry. The Windows installer includes it. Without
  Gmsh PrePolix still starts; only geometry import and meshing are unavailable. Its library path
  can be set under #menu("Tools", "Settings ...", "Gmsh").
- On Linux, PrePolix needs a Vulkan or OpenGL driver and `libxkbcommon`, which most desktop
  installations provide. The Gmsh library additionally needs `libGLU` and `libXft`.

== Installation

*Windows.* Run `prepolix-<version>-setup.exe` from the release page. It installs PrePolix with
Gmsh to `C:\Program Files\prepolix`, creates a start menu entry, optionally a desktop icon, and
can associate `.plx` project files with PrePolix. Uninstall it under _Apps_ in the Windows
settings. Because programs may not write to `C:\Program Files`, the CalculiX work directory then
defaults to `%TEMP%\prepolix`.

*Linux.* Build PrePolix from source with a current stable Rust toolchain (`cargo run --release`),
see the README of the repository.

== Starting PrePolix

Start PrePolix from the start menu, by double-clicking a `.plx` file (Windows, if associated), or
from a terminal. Files given on the command line are opened in order, so a model and its results
can be opened together:

```sh
prepolix bracket.plx bracket.frd
```

Several CAD files given together are combined into one geometry. Without command line arguments,
PrePolix reopens the project that was open when it was last closed, provided it was saved as a
`.plx` file and still exists.
