#import "../../template.typ": *

= The FE Model

The #ui("FE Model") tab holds everything CalculiX needs besides the mesh: materials, sections,
constraints, contacts, amplitudes, initial conditions and steps with their boundary conditions,
loads and outputs. Most items are created from the #ui("Model") and #ui("Interaction") menus or
with #ui("Create ...") in the context menu of their container in the tree.

#screenshot("fe-model-tree.png", [FE Model tree])

== Model properties <model-properties>

#menu("Model", "Model Properties ...") (or a double-click on #ui("Model") in the tree) sets
the basic properties of the model. The same dialog opens for #menu("File", "New ...").

#screenshot("model-properties.png", [Model properties])

=== Model space

/ 3D: solids, shells and lines.
/ 2D plane stress: plane elements (`CPS`), the section has a thickness.
/ 2D plane strain: plane elements (`CPE`), the section has a thickness.
/ 2D axisymmetric: axisymmetric elements (`CAX`). The y axis is the axis of rotation, the model
  must lie at x ≥ 0.

A change of the model space is refused if the mesh or geometry does not fit (for example solid
elements in a 2D model). 2D elements are retyped when the space changes between the 2D variants.

#note(title: "Axisymmetric models")[As in CalculiX, forces in an axisymmetric model act on the
full 360° circumference. Surface tractions are distributed according to the radius.]

=== Model type

#ui("General model") or #ui("Submodel"). A submodel takes the displacements at its cut
boundaries from a global model; choose its result file under #ui("Global results .frd")
(@submodel).

=== Unit system <unit-systems>

#fields(
  [Unitless], [No units. Numbers are taken as they are.],
  [m, kg, s, °C], [SI units.],
  [mm, ton, s, °C], [The default. Force in N, stress in MPa, density in t/mm³.],
  [m, ton, s, °C], [Force in kN, stress in kPa.],
  [in, lbf·s²/in, s, °F], [US units. Force in lbf, stress in psi.],
)

The dialog lists the base and derived units of the chosen system. When you change the unit system
of an existing model, #ui("Convert the model values") (default on) converts mesh, geometry,
materials, loads and all other values to the new units. Without it, the numbers stay the same and
are read in the new units.

=== Physical constants

#ui("Absolute zero") and #ui("Stefan-Boltzmann") are only needed for thermal radiation.
#ui("Defaults") fills them in for the selected unit system. They are written as
`*PHYSICAL CONSTANTS`.
