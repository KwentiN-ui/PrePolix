#import "../../template.typ": *

== Boundary conditions <boundary-conditions>

Boundary conditions belong to a step. Create them with #ui("Create ...") on #ui("BCs") of a step,
or with #menu("Model", "Create Boundary Condition ...") for the last step. The dialog lists only
the types the step accepts. In the 3D view, supports are drawn as cones (translations) and
plates (rotations).

#screenshot("boundary-condition.png", [Fixed support on a face])

#fields(
  [Fixed], [All degrees of freedom held at zero. In 2D only the two translations are fixed.],
  [Displacement/Rotation], [Prescribed values for the ticked components `U1`, `U2`, `U3`
    (length) and `UR1`, `UR2`, `UR3` (angle in radians). Unticked components are free.],
  [Temperature], [Prescribed temperature (heat transfer and coupled steps).],
  [Submodel], [Displacements taken from the global model (static steps of a submodel,
    @submodel). #ui("Global step") selects the step of the global results, the check boxes the
    components.],
)

The region is a selection of faces, edges or nodes, a node set or a surface. Displacement and
temperature boundary conditions can follow an #ui("Amplitude") (@amplitudes).

== Loads <loads>

Loads are created like boundary conditions, with #ui("Create ...") on #ui("Loads") of a step or
#menu("Model", "Create Load ..."). A frequency step takes no loads.

#screenshot("selection.png", [Surface traction on the end face of a plate])

#fields(
  [Concentrated Force], [`F1`, `F2`, `F3` at *every* node of the region (`*CLOAD`). The force is
    not divided between the nodes; use #ui("Surface Traction") for a total force on a face.],
  [Pressure], [Pressure on faces; positive values push into the material (`*DLOAD`).],
  [Surface Traction], [A *total* force on faces. When the input file is written it is
    distributed to the nodes according to the face areas and the element shape functions
    (`*CLOAD`).],
  [Concentrated Flux], [Heat flow into every node of the region (`*CFLUX`).],
  [Surface Flux], [Heat flux density into faces (`*DFLUX`).],
  [Body Flux], [Heat generated per volume in parts or elements (`*DFLUX`, `BF`).],
  [Film (Convection)], [Convection to the surroundings with #ui("Sink temperature") and
    #ui("Film coefficient") (`*FILM`).],
  [Radiation], [Radiation to the surroundings with #ui("Sink temperature") and
    #ui("Emissivity") (`*RADIATE`). Needs the physical constants of the model
    (@model-properties).],
)

Every load can follow an #ui("Amplitude"). Film and radiation have a second amplitude for the film
coefficient or the emissivity.

In 2D models face loads act on element edges. In axisymmetric models forces act on the full
circumference.

#limitation[Line loads, bolt pre-tension and defined temperature fields are not available in
the dialogs yet. They can be added with the keyword
editor (@keyword-editor).]
