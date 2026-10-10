#import "../../template.typ": *

== Files and projects <files>

=== File menu

#fields(
  [New ... (#key("Ctrl+N"))], [Opens #ui("Model Properties") for a new, empty model
    (@model-properties). The space and units chosen last are preset. OK closes the current model
    and all results *without asking to save*.],
  [Open ... (#key("Ctrl+O"))], [Opens a project (`.plx`), a CalculiX mesh (`.inp`), a result file
    (`.frd`) or a CAD file (`.step`, `.stp`, `.iges`, `.igs`, `.brep`). Large files load in the
    background.],
  [Save (#key("Ctrl+S"))], [Saves the project. A model that was not opened from a `.plx` file
    is saved with #ui("Save As").],
  [Save As ... (#key("Ctrl+Shift+S"))], [Saves the project under a new name.],
  [Export CalculiX Input File ...], [Writes the input file exactly as #ui("Run Analysis")
    would (@export-inp).],
  [Export Deformed Mesh (.inp) ...], [Writes the displayed deformed mesh of the current result
    as a mesh-only input file (@export-deformed).],
  [Exit], [Closes prepolix. There is no prompt for unsaved changes.],
)

#note[prepolix has no undo and does not ask about unsaved changes. Save regularly.]

=== What each file type does

/ `.plx` (prepolix project): geometry, mesh setup, mesh and FE model in one file. Saving writes a
  temporary file first and then replaces the old one, so a crash during saving cannot destroy the
  project. A project written by a newer version of prepolix is refused.
/ `.inp` (CalculiX input): *only the mesh* is imported: nodes, elements, `*NSET`, `*ELSET`
  (also `GENERATE`), `*SURFACE` and files referenced with `*INCLUDE`. Every element set named in
  `*ELEMENT, ELSET=...` becomes a part. All other keywords (materials, steps, ...) are skipped and
  listed in the output pane. The model space follows the element types: `CPS`, `CPE` and `CAX`
  elements give a 2D model.
/ `.frd` (CalculiX results): added to the results on the #ui("Results") tab; the FE model is not
  touched. Opening the same file again reloads it and keeps its hot spots, paths and planes. A
  `.dat` file with the same name next to it is read as well (history output).
/ CAD files: #menu("File", "Open ...") replaces the model with the geometry. To *add* geometry
  to an existing model use #menu("Geometry", "Import ...") (@geometry-import).

Supported element types of `.inp` meshes: `C3D4`, `C3D6`, `C3D8`, `C3D10`, `C3D15`, `C3D20` with
their `R` and `I` variants, `S3`, `S4`, `S6`, `S8`, `M3D` membranes, `CPS`/`CPE`/`CAX` plane and
axisymmetric elements, `B31`, `B32`, `T3D2` and `T3D3`.

=== Drag and drop and command line

Files can be dropped onto the window. Several CAD files dropped together are combined into one
geometry; otherwise only the first dropped file is opened.

On the command line, `prepolix model.plx results.frd` opens all files in order. Without files,
prepolix reopens the last saved project that was open at exit.
