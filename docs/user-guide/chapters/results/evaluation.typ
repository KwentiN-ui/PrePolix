#import "../../template.typ": *

== Transformations <transformations>

The transformation button on the results toolbar shows the results of a symmetric or periodic
model as the full structure. Transformations only change the display.

#figure(
  grid(
    columns: (40%, 60%),
    gutter: 8pt,
    image("../../images/transformation.png"), image("../../images/symmetry.png"),
  ),
  caption: [Symmetry about the YZ plane and its result],
)

Move a type from #ui("Available") to #ui("Active") and set its properties:

/ Symmetry X/Y/Z: mirrors the model at the plane normal to the axis through the #ui("Symmetry
  point").
/ Linear pattern: copies the model #ui("Number of items") times along the line from the start to
  the end point.
/ Circular pattern: copies the model #ui("Number of items") times by #ui("Angle") about the axis
  through two points.

Several transformations are applied one after the other. Vector and tensor results are mirrored
or rotated with the copies, and legend and labels include them. #ui("Clear") removes all
transformations.

== Derived field outputs <derived-fields>

#ui("Create ...") on #ui("Field Outputs") computes new fields from the results.

#screenshot("field-output.png", [Create Field Output])

#fields(
  [Limit], [Divides a component by a limit value per part, per element set or for all elements.
    Gives `RATIO` (value / limit) and `SAFETY_FACTOR` (limit / value).],
  [Envelope], [`MAX`, `MIN` and `AVERAGE` of a component over all increments of all steps.],
  [Equation], [A formula of components, for example `=Abs([STRESS.S11]) / 235`, with the
    functions of PrePoMax (`Abs`, `Sqrt`, `Max`, `Min`, `Pow`, `Sin`, `If`, ...). #ui("Insert
    Component...") inserts a component name. #ui("Unit") is shown in the legend.],
  [Coordinate System Transformation], [Rotates a vector or tensor field into a coordinate system
    defined under #ui("Features"); cylindrical systems give radial, tangential and axial
    components.],
)

#limitation[Derived field outputs are lost when the result file is reloaded.]

== History outputs and tables <history-outputs>

#ui("History Outputs") on the #ui("Results") tab lists the history outputs from the `.dat` file
(for example the eigenfrequencies of a frequency step) and the ones you create with
#ui("Create ..."):

/ From Field Output: values of a field at selected nodes, one row per increment.
/ From History Output by Equation: a formula of other history outputs.
/ From Element Size: volume of elements or area of faces in every increment.

A double-click on a component opens its table. Click column headers to select them
(#key("Ctrl") adds, #key("Shift") selects a range) or drag over cells. #ui("Copy")
(#key("Ctrl+C")) copies the selection as tab-separated text, #ui("Plot") (#key("Ctrl+P")) draws
the selected columns over the first one.

#screenshot("history-table.png", [Eigenfrequencies from the `.dat` file])

== Paths <paths>

#ui("Create ...") on #ui("Paths") shows the displayed component along a straight line.

#screenshot("path.png", [Stress along a path through the hole])

#fields(
  [Start, End], [Coordinates (typed or picked with #ui("...")) or reference points.],
  [Points], [Number of points along the path (default 100).],
)

The dialog shows minimum, maximum and a plot of the value over the distance. #ui("Copy to
Clipboard") and #ui("Save as CSV...") export the values; #ui("Table") lists them. Points outside
the mesh have no value.

== Plane results

#ui("Create ...") on #ui("Plane Results") evaluates the displayed component on a plane defined
under #ui("Features"): minimum and maximum with their position, the area-weighted mean, and the
cut area. With its check box ticked, the 3D view shows only the cut, and the legend uses the
values on it.

== Hot spot stresses <hot-spots>

Hot spot stresses evaluate welded joints as recommended by the IIW: the stress at the weld toe is
extrapolated from read-out points on the plate surface.

#screenshot("hot-spot.png", [Create Hot Spot])

#fields(
  [Weld toe], [Nodes along the weld toe, for example picked as an edge.],
  [Extrapolation], [IIW type a, fine mesh, linear (0.4t / 1.0t, default) or quadratic
    (0.4t / 0.9t / 1.4t); type a, coarse mesh (0.5t / 1.5t); type b, fine (4 / 8 / 12 mm) or
    coarse (5 / 15 mm); or #ui("Custom read-out points").],
  [Plate thickness t], [For type a methods.],
  [Read-out points], [Shows the extrapolation formula.],
  [Stress], [#ui("Perpendicular to weld toe"), #ui("Max principal stress") or #ui("Signed max
    abs principal stress").],
  [Path direction], [Points away from the weld toe; PrePolix turns it into the plate surface.],
)

The evaluation runs for all increments and writes `<result>_hot_spots.csv` next to the result
file. #menu("Results", "Hot Spot Table") shows the read-out stresses and the hot spot stress per
node for the displayed increment, with the maximum in bold.

== Query <query>

#menu("Tools", "Query ...") reads values from the model by clicking: #ui("Vertex/Node"),
#ui("Facet/Element"), #ui("Part"), #ui("Assembly"), #ui("Bounding box size"), #ui("Distance")
(two nodes), #ui("Angle") (three nodes) and #ui("Circle") (centre and radius through three
nodes). The values appear in the output pane and as labels in the 3D view. On results, node
queries also show the deformed position, the displacement and the value of the displayed
component. #ui("Clear") removes the labels.

#screenshot("query.png", [Querying nodes on a result])
