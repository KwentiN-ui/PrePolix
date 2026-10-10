#import "../../template.typ": *

= Geometry and Mesh <geometry>

== Importing geometry <geometry-import>

#menu("Geometry", "Import ...") or the toolbar button #ui("Import Geometry") reads STEP
(`.step`, `.stp`), IGES (`.iges`, `.igs`) and BREP (`.brep`) files. Several files can be
selected at once. Gmsh with OpenCASCADE reads the files, so Gmsh must be available
(@settings).

- If no model is open, #ui("Model Properties") opens first, so you can choose the model space
  (3D or 2D) and the unit system before the geometry is read (@model-properties).
- If the model already has geometry or a mesh, the imported parts are *added*. Existing parts keep
  their names, mesh settings and selections.
- STEP and IGES files are converted to the length unit of the model. BREP files have no unit and
  are read as they are.
- Default element sizes are derived from the size of the geometry: maximum 5 % and minimum 0.1 %
  of the bounding box diagonal.

#screenshot("geometry.png", [Imported geometry on the Geometry tab])

=== Parts

Every solid becomes a part. Faces that do not belong to a solid become shell parts, edges that do
not belong to a face become line parts (for beams and trusses). Part names are taken from the
CAD file where it has names, otherwise they are `SOLID-1`, `SHELL-1`, `LINE-1` and so on.

The context menu of a geometry part has #ui("Create Mesh"), #ui("Properties ..."),
#ui("Hide")/#ui("Show") and #ui("Delete"). Deleting a geometry part keeps a mesh that was already
created from it.

=== Geometry for 2D models

In a 2D model the geometry must consist of faces only and lie in the plane z = 0. For an
axisymmetric model it must lie at x ≥ 0, because the y axis is the axis of rotation. Otherwise
the import is refused with a message. Faces whose normal points in −z direction are flipped
when they are meshed.
