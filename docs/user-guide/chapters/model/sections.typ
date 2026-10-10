#import "../../template.typ": *

== Sections <sections>

A section assigns a material to elements. Every element needs a section, otherwise the model check
reports "Elements without material". Create sections with #menu("Model", "Create Section ...").

#screenshot("section.png", [Section dialog])

#fields(
  [Type], [#ui("Solid"), #ui("Truss") or #ui("Beam").],
  [Material], [One of the model's materials.],
  [Region], [#ui("Parts") (picked in the 3D view) or an #ui("Element Set") of an imported
    mesh.],
)

=== Solid section

For solid elements and the plane and axisymmetric elements of 2D models. In plane stress and plane
strain models the section has a #ui("Thickness") (default 1). Written as `*SOLID SECTION`.

=== Truss section

For line parts that carry only axial force. #ui("Cross-section area") must be positive. The lines
are written as `T3D2` truss elements.

#note[Trusses must form a stable truss structure. If they can move without resistance, the model
check reports "Truss structure is a mechanism".]

=== Beam section

#fields(
  [Profile], [#ui("Rectangle") (thicknesses a and b in the 1- and 2-direction), #ui("Circle")
    (radius), #ui("Pipe") (outer radius and wall thickness) or #ui("Box") (widths a and b and the
    four wall thicknesses).],
  [Normal (1-direction)], [#ui("Automatic") uses the global z axis, or x for beams parallel to z.
    #ui("Vector") takes the direction from x, y and z; it must not be parallel to a beam.],
  [Offset in 1-/2-direction], [Shifts the beam axis by a multiple of the section dimension.],
)

Beams are written as `*BEAM SECTION` with `B31` or `B32` elements. Pipe and box sections need
quadratic lines and are written as `B32R`, as CalculiX requires.

#limitation[There are no shell sections yet. Line elements can only be used in 3D models.]
