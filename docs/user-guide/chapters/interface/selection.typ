#import "../../template.typ": *

== Selecting regions <selection>

Boundary conditions, loads, constraints, contacts, mesh settings and many result tools act on a
_region_ of the model. You define it by clicking in the 3D view while the item's dialog is open.

=== Region field

Each dialog with a region has a #ui("Region") field. Its combo box chooses where the region
comes from; the choices depend on the item:

/ Selection in the 3D view: picked nodes, element faces, edges, parts or CAD faces. The field shows
  what is selected ("12 element faces") and has a #ui("Clear Selection") button.
/ Parts: whole parts, picked in the 3D view.
/ Node Set, Element Set, Surface: sets and surfaces of an imported `.inp` mesh, or ones you
  defined yourself (@mesh-sets).

The #ui("...") button next to a field makes it the target of clicks in the 3D view; it is shown
pressed while it is active. Dialogs with two regions (master and slave) use one #ui("...")
button per region to switch between them.

=== Selection window

While a dialog picks, the #ui("Selection") window opens next to it and decides what a click
selects.

#screenshot("selection.png", [Picking faces for a load with the selection window])

/ Geometry based: #ui("Faces, edges and points") selects the smooth patch of faces under the
  cursor, or a feature edge or corner within a few pixels. #ui("Part") selects whole parts.
  #ui("Edge angle") and #ui("Surface angle") grow the selection across edges or faces as long as
  the angle between neighbours stays below the given value (default 30°).
/ Mesh based (#ui("More")): #ui("Node"), #ui("Element"), #ui("Edge"), #ui("Surface"),
  #ui("Part"), #ui("Edge angle"), #ui("Surface angle") and #ui("ID"). #ui("ID") takes a list such
  as `1, 5, 10-20` with #ui("Add") and #ui("Remove"); for face and edge regions the numbers are
  element numbers.

Modes that do not fit the region are disabled, for example #ui("Node") for a pressure load.
#ui("Undo") takes back the last pick, #ui("Clear") empties the selection, #ui("All") and
#ui("Invert") act on the visible items.

#note[On a mesh created from CAD geometry, geometry-based picks are stored as CAD faces, edges
and vertices. They survive remeshing. Picks of single nodes or element faces refer to the mesh
and have to be repeated after remeshing; the output pane says how many selections are affected.]

=== Modifier keys and box selection

- A plain click replaces the selection, #key("Shift") adds, #key("Ctrl") removes,
  #key("Shift")+#key("Ctrl") keeps only the intersection. A plain click into empty space
  clears the selection.
- Dragging with the left button draws a box. Dragged from left to right (blue) it takes what lies
  completely inside; dragged from right to left (green) it also takes what crosses the box.
- When the mouse rests for a moment, what a click would select is previewed in orange.
