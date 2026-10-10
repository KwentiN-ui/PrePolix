#import "../../template.typ": *

== Settings <settings>

#menu("Tools", "Settings ...") opens the settings window. The pages are listed on the left.
#ui("Defaults") resets all pages, #ui("Apply") applies without closing. Settings are stored per
user (`~/.local/share/prepolix` on Linux, `%APPDATA%\prepolix\data` on Windows).

#screenshot("settings.png", [Settings, CalculiX page])

=== Graphics

#fields(
  [Global axes at the origin], [Draws a coordinate triad at the global origin (default on).],
  [Coordinate triad in the corner], [The clickable triad in the lower right (default on).],
  [Scale bar], [Shows a scale bar (default on).],
)

=== Post-processing

#fields(
  [Show label at maximum / minimum], [Labels at the node with the largest / smallest value
    (default: maximum on, minimum off).],
  [Show status block], [The info block with file, step, increment and scale factor.],
  [Show undeformed outline], [Draws the undeformed model as a wireframe for newly opened
    results.],
  [Color levels], [Number of colour bands of the legend for newly opened results (2 to 24,
    default 9).],
)

=== CalculiX

#fields(
  [Executable], [Path to `ccx`. A bare name such as `ccx` is looked up on the system path.],
  [Threads], [Number of threads CalculiX uses (`OMP_NUM_THREADS`), default 1.],
  [Work directory], [Where input and result files of a run are written. Empty means a `Temp`
    folder next to the PrePolix program, or `<system temp>/prepolix` if that folder is not
    writable.],
  [Equation solvers], [The direct solvers found in the CalculiX executable, for example
    PARDISO. Steps with the solver #ui("Default") use Pardiso when it is available.],
  [Test CalculiX], [Starts CalculiX, computes a cantilever under a tip load and under pressure,
    compares the results with beam theory and checks which solvers are available. Each test shows
    OK or an error.],
)

=== Gmsh

#fields(
  [Library], [Path to the Gmsh library (`libgmsh.so` or `gmsh-4.15.dll`). Empty means: next to
    the program, then the system path. A changed library is loaded after a restart.],
  [Test Gmsh], [Loads Gmsh and meshes a cube.],
)
