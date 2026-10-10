#import "../../template.typ": *

== Initial conditions <initial-conditions>

#menu("Model", "Create Initial Condition ...") sets a temperature before the first step, for
example as reference for thermal expansion or as start value of a transient heat transfer
analysis. The only type is #ui("Temperature") (default 20). The region can be a selection,
parts, a node set or a surface. Written as `*INITIAL CONDITIONS, TYPE=TEMPERATURE`.
