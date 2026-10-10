#import "../../template.typ": *

= Running CalculiX <analysis>

== Running an analysis

A model has one analysis, #ui("Analysis-1"). It uses the CalculiX settings under
#menu("Tools", "Settings ...", "CalculiX") (@settings).

#fields(
  [Run Analysis (#key("F5"))], [Writes `Analysis-1.inp` to the work directory and starts
    CalculiX. Old result files of the analysis in that directory are deleted first.],
  [Check Model], [Lets CalculiX only read and check the model (`*NO ANALYSIS`), without solving
    it.],
  [Kill Analysis], [Stops a running analysis.],
  [Monitor], [Shows the monitor window.],
  [Open Results], [Opens the result file of the last run.],
)

These commands are in the #ui("Analysis") menu and in the context menu of #ui("Analysis-1").
Before CalculiX starts, PrePolix refuses to run a model without mesh, without step or with all
steps deactivated, and it stops if the input file cannot be written (for example because a region
is empty).

=== Monitor

The monitor opens when a run starts. It shows the status and running time, the current step,
increment and iteration, and the output of CalculiX. #ui("Results") opens the results when the run
is finished.

#screenshot("monitor.png", [Monitor after a completed run])

The status at the end is #ui("completed"), #ui("finished with errors, results available"),
#ui("failed") or #ui("killed"). If CalculiX failed, PrePolix looks for known error messages
in the output and adds a line "Possible cause: ..." with a remedy, for example for distorted
elements, missing material data, rigid body motion or missing convergence.

== Model check <model-check>

PrePolix checks the model continuously while you work. Items with a problem get a warning sign in
the tree; items with an error and their containers are shown in red. Click the warning sign to see
what is wrong and how to fix it.

#screenshot("model-check.png", [Model check: parts without a section])

Errors include elements without material, materials without the data the step needs (elasticity,
density, conductivity, specific heat), invalid material constants, distorted elements, a missing
initial temperature in a transient heat transfer step, an increment larger than the step, rigid
body motion of a part in a static step, truss structures that are mechanisms and missing
references (a deleted material, part or set that an item still uses). Warnings include parts held
only by contact, conflicting boundary conditions, loads on fixed nodes, rotations without effect
and steps without load.

The checks do not stop a run; their errors are listed at the top of the monitor. Missing
references do stop it, because the input file cannot be written.

== Exporting the input file <export-inp>

#menu("File", "Export CalculiX Input File ...") writes the same input file as a run, for example
to run CalculiX by hand or on another computer. Deactivated items appear as comments.

== Submodels <submodel>

A submodel computes a detail of a larger model more precisely. Its cut boundaries follow the
displacements of the global model.

+ Compute the global model and keep its result file (`.frd`).
+ In the submodel, open #menu("Model", "Model Properties ...") and set #ui("Model Type") to
  #ui("Submodel"). Choose the global result file under #ui("Global results .frd").
+ In a static step, create a boundary condition of type #ui("Submodel") on the cut faces. Choose
  the #ui("Global step") and the displacement components.
+ Run the analysis. PrePolix copies the global results next to the input file and writes
  `*SUBMODEL` and `*BOUNDARY, SUBMODEL`.

#limitation[Only displacements can be transferred. The global result file must not be the result
file of the submodel's own analysis.]

== Exporting a deformed mesh <export-deformed>

#menu("File", "Export Deformed Mesh (.inp) ...") writes the mesh of the displayed result with its
nodes moved by the displacements times the current deformation scale factor. A typical use is an
imperfection for a buckling analysis: show the buckling mode, set #ui("Deformation") to #ui("User
defined") with the imperfection amplitude as factor, export, and open the file as the mesh of a new
model. The file contains only the mesh.
