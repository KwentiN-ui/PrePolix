#import "../../template.typ": *

== Meshing <meshing>

PrePolix meshes each geometry part separately with Gmsh. Parts do not share nodes; connect them
with ties, contacts or node ties (@constraints).

- #menu("Mesh", "Mesh All Parts") meshes every part.
- #ui("Create Mesh") in the context menu of a part meshes only this part. Its old mesh is
  replaced; the other parts keep their node and element numbers.

The output pane lists each part with its element count and size range. Afterwards PrePolix
switches to the #ui("FE Model") tab.

=== Element types

#table(
  columns: (auto, 1fr),
  stroke: (x, y) => if y == 0 { (bottom: 0.7pt) } else { (bottom: 0.3pt + luma(200)) },
  inset: (x: 5pt, y: 4pt),
  table.header([*Part*], [*Elements*]),
  [Solid (3D models)], [Tetrahedra `C3D4` (linear) or `C3D10` (quadratic)],
  [Face (2D models)], [Triangles, or quad-dominated with quadrilaterals; written as `CPS`,
    `CPE` or `CAX` elements depending on the model space],
  [Line (3D models)], [`B31` or `B32` lines; the section decides whether they are written as
    beams or trusses (@sections)],
)

Faces can only be meshed in 2D models and solids and lines only in 3D models. Shell meshes of
3D models are not available yet.

=== Meshing errors

If Gmsh fails, the output pane shows "Meshing failed: ..." with the message of Gmsh. The faces,
edges and points the message names are marked red on the #ui("Geometry") tab, so you can see
where the problem is. The marking disappears with the next meshing.

== Mesh setup <mesh-setup>

The mesh is controlled by the #ui("Mesh Setup") items on the #ui("Geometry") tab. Create them
with #menu("Mesh", "Create Mesh Setup Item ...") or #ui("Create ...") on the #ui("Mesh Setup")
container.

=== Default mesh parameters

#menu("Mesh", "Default Mesh Parameters ...") sets the parameters for every part that is not
covered by a #ui("Meshing Parameters") item. #ui("Mesh All Parts") in this dialog applies the
values and meshes at once.

#screenshot("mesh-parameters.png", [Default mesh parameters])

#fields(
  [Max. element size], [Largest element edge length.],
  [Min. element size], [Smallest element edge length.],
  [Elements per curvature radius], [Refines the mesh on curved faces: about this many elements
    per radius (default 2). 0 switches curvature refinement off.],
  [Second order], [Quadratic elements with midside nodes (default on).],
  [Midside nodes on geometry], [Places midside nodes on the curved geometry instead of on the
    straight edge between the corner nodes (only with second order).],
  [Optimize mesh (Netgen)], [Improves the element quality after meshing (default on).],
  [Quad-dominated mesh (2D)], [Creates mainly quadrilaterals on faces of 2D models.],
)

=== Mesh setup items

/ Meshing Parameters: the fields above for selected parts. If several items name the same part,
  the last one counts.
/ Local Mesh Size: an element size on selected CAD faces and edges. Click them in the 3D view;
  #key("Shift") adds, #key("Ctrl") removes. The default is a quarter of the maximum element
  size.
/ Tetrahedral Gmsh: chooses the Gmsh algorithms for selected parts. #ui("Surface algorithm"):
  Frontal-Delaunay (default), Delaunay, MeshAdapt or Automatic. #ui("Volume algorithm"):
  Delaunay (default), Frontal or HXT.

#screenshot("mesh-setup-item.png", [Creating a mesh setup item])

== Using an existing mesh

A CalculiX mesh can be opened with #menu("File", "Open ...") (@files). Its node sets, element
sets and surfaces appear under #ui("Mesh") in the #ui("FE Model") tree and can be used as
regions. Such a mesh has no geometry, so regions are picked on the mesh.
