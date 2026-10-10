#import "../../template.typ": *

== Steps <steps>

A step is one analysis of CalculiX with its own boundary conditions, loads and outputs. Create it
with #menu("Model", "Create Step ...") or #ui("Create ...") on #ui("Steps"). A new step copies the
boundary conditions and loads of the last step, as far as its type accepts them. The type of a
step cannot be changed after it was created.

#note[Every step writes `OP=NEW` for its boundary conditions and loads. What applies in a step is
exactly what is listed under it in the tree.]

#screenshot("step.png", [Static step])

=== Order of the steps

CalculiX runs the steps in the order of the list, and a step starts from the state at the end of
the previous one. To change the order, right-click a step in the #ui("Steps") list and choose
#ui("Move Up") or #ui("Move Down"). The step keeps its boundary conditions, loads, defined
fields and outputs, and stays selected. #ui("Move Up") is disabled on the first step and
#ui("Move Down") on the last.

#screenshot("step-move-menu.png", [Context menu of a step])

Steps that build on the previous one, such as a step with #ui("Perturbation"), take the state of
whatever step now precedes them, so check the order after moving. The step numbers of existing
results refer to the order at the time of the run.

=== Solver

All step types have a #ui("Solver") field: #ui("Default (Pardiso if available)"),
#ui("Pardiso"), #ui("Spooles"), #ui("PaStiX"), #ui("Iterative scaling") or
#ui("Iterative Cholesky"). #ui("Default") uses Pardiso if the CalculiX executable contains it,
otherwise the default of CalculiX. Frequency and buckle steps need a direct solver.

=== Static

#fields(
  [Nonlinear geometry (Nlgeom)], [Large deformations (`NLGEOM`).],
  [Incrementation], [#ui("Default") leaves the incrementation to CalculiX. #ui("Automatic") uses
    the time and increment values below, #ui("Direct") uses fixed increments.],
  [Max. increments], [Largest number of increments (`INC`).],
  [Time period], [Length of the step (default 1).],
  [Initial / Min. / Max. increment], [Increment sizes for automatic incrementation.],
)

=== Frequency

Computes eigenfrequencies and mode shapes (`*FREQUENCY`). A frequency step takes no loads.

#fields(
  [Perturbation], [Uses the stress state of the previous step (prestress).],
  [Number of eigenfrequencies], [Default 10.],
  [Lower / Upper frequency bound], [Optional range of frequencies to compute.],
  [Store matrices and eigenmodes], [Writes the `.eig` file (`STORAGE=YES`).],
)

Materials need a density in a frequency step.

=== Buckle

Computes buckling factors and buckling modes (`*BUCKLE`). The loads of the step are the reference
load: the critical load is the buckling factor times these loads.

#fields(
  [Preload from previous step (Perturbation)], [Takes the state of the previous step as
    preload.],
  [Number of buckling factors], [Default 1.],
  [Accuracy], [Convergence tolerance of the eigenvalue solver (default 1e-4).],
)

=== Heat Transfer and Coupled Temperature-Displacement

#ui("Heat Transfer") computes temperatures only (`*HEAT TRANSFER`), #ui("Coupled
Temperature-Displacement") computes temperatures and displacements together.

#fields(
  [Steady state], [Steady-state solution (default on). Switch it off for a transient
    analysis; materials then need density and specific heat, and the model an initial
    temperature.],
  [Max. temperature change], [Largest temperature change per increment in a transient analysis
    (`DELTMX`).],
  [Nonlinear geometry], [Coupled steps only.],
)

Incrementation and solver fields are the same as for a static step.

=== Field outputs

Every step has a node and an element field output, which control what is written to the result
file (`.frd`). Open them with a double-click to choose the variables:

/ Node: `RF` (reaction forces), `U` (displacements), `NT` (temperatures), `RFL` (reaction heat
  flux).
/ Element: `S` (stresses), `E` (strains), `ME` (mechanical strains), `PEEQ` (equivalent plastic
  strain), `ENER` (energy density), `HFL` (heat flux).

The defaults depend on the step type. Field outputs cannot be added or deleted.

=== History outputs

History outputs write values of a region to the `.dat` file in every increment (`*NODE PRINT`,
`*EL PRINT`, `*CONTACT PRINT`). Create them with #ui("Create ...") on #ui("History Outputs") of a
step.

#fields(
  [Type], [#ui("Node Output") (region of nodes), #ui("Element Output") (parts, element set or
    selection) or #ui("Contact Output") (a contact pair).],
  [Variables], [Node: `RF U V NT RFL`. Element: `S E ME PEEQ HFL ENER ELSE ELKE EVOL EBHE`.
    Contact: `CDIS CSTR CELS CNUM CF`.],
  [Totals], [#ui("Yes") adds the sum over the region, #ui("Only") writes only the sum.],
)

When results are opened, the `.dat` file is read too, and the history outputs appear on the
#ui("Results") tab (@history-outputs).
