#import "../../template.typ": *

== Materials <materials>

#menu("Model", "Create Material ...") creates a material. Each property is switched on with its
check box. A new material starts with #ui("Elasticity") switched on.

#screenshot("material.png", [Material dialog])

#fields(
  [Density], [Needed for frequency steps and transient heat transfer.
    Written as `*DENSITY`.],
  [Elasticity], [#ui("Young's modulus") and #ui("Poisson's ratio") (0 to 0.5). Written as
    `*ELASTIC`.],
  [Thermal conductivity], [Written as `*CONDUCTIVITY`.],
  [Specific heat], [Needed for transient heat transfer. Written as `*SPECIFIC HEAT`.],
  [Thermal expansion], [#ui("Expansion coefficient") and #ui("Reference temperature") (default
    20). Written as `*EXPANSION`.],
)

Values can be typed in any unit of the right kind, for example `210 GPa` or `7.85 g/cm³`
(@numeric-input).

=== Material library

#ui("Material Library ...") in the context menu of #ui("Materials") opens the material library.
On the left are the library materials in categories, on the right the materials of the model.

#screenshot("material-library.png", [Material library])

- #ui("Copy to Model") and #ui("Copy to Library") copy the selected material. Values are
  converted between the unit system of the library and that of the model. A name that already
  exists gets a suffix.
- #ui("Add Category"), #ui("Rename"), #ui("Delete"), #ui("Move Up") and #ui("Move Down")
  organise the library.
- #ui("Preview Material Properties") shows the values of the selected material.

The library is stored per user in `materials.ron` in the settings folder (@settings). When the
dialog is closed, PrePolix asks whether to save changes to the library.
