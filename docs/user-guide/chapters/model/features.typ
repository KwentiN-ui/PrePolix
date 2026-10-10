#import "../../template.typ": *

== Features <features>

Under #ui("Features") in the tree you define reference points, coordinate systems and planes.
They are helpers for other functions (section views, plane results, paths, result
transformations) and are not written to the input file. The results have their own copy of the
features, which can be edited on the #ui("Results") tab.

Points can be typed in or picked: a node, or the centre of a clicked edge or face.

/ Reference Point: a named point.
/ Coordinate System: #ui("Rectangular") or #ui("Cylindrical"), defined by #ui("Origin"),
  #ui("Point on x-axis") and #ui("Point in xy-plane"). For a cylindrical system the z axis is the
  cylinder axis and the directions are r, θ and z.
/ Plane: by #ui("Coordinate system") (a plane XY, YZ or XZ of a system with an offset), by
  #ui("Three points") (normal by the right-hand rule) or by #ui("Point and normal"). A plane
  built on reference points follows them when they change.
