#import "../../template.typ": *

=== Shell section <shell-section>

A #ui("Shell") section gives surface elements of a 3D model a thickness, so that a thin plate
or a sheet-metal part can be computed with shell elements (`S3`, `S4`, `S6`, `S8`) instead of
solids. Create it with #menu("Model", "Create Section ...") and choose #ui("Shell").

#screenshot("shell-section.png", [Shell section])

#fields(
  [Thickness], [The thickness of the shell (default 1); it must be greater than 0.],
  [Offset], [Position of the mesh within the thickness as a fraction of the thickness, from −1
    to 1: 0 (default) puts the mesh on the mid-surface, 0.5 on the top face, −0.5 on the bottom
    face. Written as `OFFSET=`.],
  [Material, Region], [As for the other sections. The region must contain surface elements
    only; a shell section on solid or line elements is reported as an invalid section.],
)

Written as `*SHELL SECTION, ELSET=..., MATERIAL=...` with the thickness on the next line. As soon
as the model has a shell section, the field outputs are written with `OUTPUT=2D`, so that the
results are reported at the shell nodes instead of the nodes CalculiX expands the shells to
internally, and the result file keeps the node numbers of the model.

Shell nodes have rotations: #ui("Displacement/Rotation") boundary conditions can hold `UR1`,
`UR2`, `UR3`, and #ui("Moment") loads can act on them.
