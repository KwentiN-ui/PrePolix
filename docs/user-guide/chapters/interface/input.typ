#import "../../template.typ": *

== Entering values <numeric-input>

Numeric fields accept a decimal point or a comma, scientific notation (`2.1e5`, `2,1e5`) and the
typographic minus sign. Thousands separators are not allowed, and the fields do not evaluate
arithmetic expressions. You can also drag a field with the mouse to change its value.

Fields of physical quantities show the unit of the model's unit system behind the value
(@unit-systems). You may type a value in another unit and PrePolix converts it: in a model in
mm, `3 cm` becomes 30 mm; other examples are `200 GPa`, `7.85 g/cm³`, `kN·m`, `N/mm²`,
`W/(m²·K)`, `5°`, `rpm`, `psi` and `°F`. The unit must belong to the quantity of the field. In a
unitless model, enter numbers only.

PrePolix never rounds what you typed: values are shown with all significant digits, very small or
very large values in scientific notation.
