#import "../../template.typ": *

== Rigid bodies, moments and reference points <rigid-body>

A #ui("Rigid Body") constraint makes the nodes of a region move as one rigid body, driven by a
reference point: a bolt head, a bearing seat or a face a lever acts on. Create it with
#menu("Interaction", "Create Constraint ...") and choose #ui("Rigid Body").

#screenshot("rigid-body.png", [Rigid body constraint])

#fields(
  [Region], [The nodes that form the rigid body: a selection of faces, edges or nodes, parts, a
    node set or a surface.],
  [Reference point], [One of the reference points under #ui("Features") (@features). It is the
    point the body is driven at; create it first.],
)

Written as `*RIGID BODY, NSET=..., REF NODE=..., ROT NODE=...`: PrePolix adds a reference node at
the point for the translations and a rotation node for the rotations, as CalculiX requires.

=== Loads and supports on a reference point

Boundary conditions and loads can act on a reference point instead of on nodes: choose
#ui("Reference Point") as the region in their dialogs. The point must drive an active rigid
body, otherwise the model check (@model-check) reports "Reference point without rigid body".

- A #ui("Fixed") or #ui("Displacement/Rotation") boundary condition on the point holds or moves
  the whole body; `UR1`, `UR2`, `UR3` turn it about the point.
- A #ui("Concentrated Force") on the point pulls the whole body with the given force.
- A #ui("Moment") load (`M1`, `M2`, `M3`, written as `*CLOAD` on the rotational degrees of
  freedom 4 to 6) turns the body about the point. Moments can also act on nodes of beams or
  shells, which have rotations of their own, and need a 3D model.

#screenshot("moment-load.png", [Moment on the reference point of a rigid body])

The reaction forces and moments of a held reference point can be requested with a history
output (@history-outputs) of `RF` on the point.
