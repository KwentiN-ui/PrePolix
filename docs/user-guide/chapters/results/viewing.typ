#import "../../template.typ": *

= Results <results>

== Opening results

Results are opened with #menu("Analysis", "Open Results") after a run, with #ui("Results") in the
monitor, or with #menu("File", "Open ...") for any `.frd` file (ASCII or binary). A `.dat` file
with the same name is read as well. Several result files can be open at the same time; the
#ui("Result") box on the toolbar switches between them. #menu("Results", "Close Current
Results") and #ui("Close All Results") close them.

When a result file is opened, PrePolix shows the #ui("Results") tab with the last increment.

#screenshot("results.png", [von Mises stress of a static step])

== Results tree

/ Model: the mesh of the results with its parts and sets, and the features (reference points,
  coordinate systems, planes) used for result evaluation.
/ Field Outputs: the fields of the result file with their components. Click a component to show
  it. Names follow PrePoMax (`U1`, `S11`, ...). PrePolix adds the displacement magnitude `ALL`,
  `MISES`, `TRESCA`, the principal stresses and the signed maximum absolute principal stress.
/ History Outputs: values from the `.dat` file and history outputs you create (@history-outputs).
/ Hot Spot Stresses, Paths, Plane Results: evaluations described in the following sections.

== Results toolbar <results-toolbar>

#screenshot("results-toolbar.png", [Results toolbar])

#fields(
  [Result], [The displayed result file.],
  [Deformation], [#ui("Undeformed"), #ui("True scale") (default), #ui("Automatic") with
    factors from 0.25 to 5, or #ui("User defined").],
  [Factor], [The deformation scale factor; editable for #ui("User defined").],
  [Show Undeformed], [Draws the undeformed outline behind the deformed shape.],
  [Color levels], [Number of colour bands (2 to 24).],
  [Transformations], [Symmetries and patterns (@transformations).],
  [Step, Increment], [The displayed increment, with buttons for the first, previous, next and
    last increment.],
  [Animation], [Opens the animation window (@animation).],
  [Sound of the mode shapes], [Opens the sound window for modes of a frequency step
    (@sound).],
)

== Contour plot, legend and labels

The legend shows the field, the unit and the range in nine colour bands from blue (minimum) to red
(maximum) by default. The info block in the upper right shows file, date, step, increment or mode
with its frequency or buckling factor, and the deformation scale factor. Labels mark the node with
the maximum (and optionally the minimum) value. Legend, info block and labels can be dragged.
Which of them appear is set under #menu("Tools", "Settings ...", "Post-processing").

#screenshot("results-mode.png", [First mode shape of a frequency step])

Increment 0 of a buckle step is the reference state; the modes follow as increments 1, 2, ...
