#import "../../template.typ": *

= The User Interface

== Main window <main-window>

The layout of the main window is fixed, as in PrePoMax. Only the separators between the panes can
be moved. PrePolix remembers the window size and position.

#screenshot("main-window.png", [Main window with a model on the FE Model tab])

From top to bottom the window contains:

/ Menu bar: #ui("File"), #ui("Edit"), #ui("View"), #ui("Geometry"), #ui("Mesh"),
  #ui("Model"), #ui("Interaction"), #ui("Analysis"), #ui("Results"), #ui("Tools"),
  #ui("Help"). #ui("Edit") and #ui("Help") have no entries yet.
/ Toolbar: two rows. The first row holds the file, view and display buttons, the second row the
  result controls (@results-toolbar). The result row is always there but greyed out outside the
  #ui("Results") tab, so the 3D view does not jump when you switch tabs.
/ Model tree: on the left, with the three tabs #ui("Geometry"), #ui("FE Model") and
  #ui("Results") (@model-tree).
/ 3D view: in the centre (@view).
/ Output pane: below the 3D view. It collects messages: what was loaded or saved, warnings,
  keywords of an imported `.inp` that were skipped ("Not yet evaluated: ..."), unit conversions
  and the output of the query tool.
/ Status bar: on the left what is loaded ("bracket.plx: 12345 nodes, 6789 elements, 3 parts") or
  what is running ("Meshing ..."), on the right the unit system and the model space.

=== Toolbar, first row

#fields(
  [New, Open, Save], [As in the #ui("File") menu (@files).],
  [Import Geometry], [Imports STEP, IGES or BREP files and adds them to the geometry
    (@geometry-import).],
  [Zoom to Fit], [Fits the visible parts into the view.],
  [Front, Back, Top, Bottom, Left, Right, Isometric], [Standard views (@view).],
  [Vertical View], [Rolls the view so that the axis closest to screen-up points straight up.],
  [Screenshot], [Copies the 3D view to the clipboard or saves it as PNG (@screenshot).],
  [Edges Only / Mesh Edges], [Show only the feature edges of the model, or all element edges.
    Exactly one of the two is active.],
  [Section View], [Opens the section view dialog. The button stays pressed while a section is
    active (@section-view).],
  [Exploded View], [A left click switches the exploded view on and off, a right click opens its
    settings (@exploded-view).],
)

Buttons that cannot be used at the moment are greyed out, for example #ui("Save") without a
model or #ui("Open") while a file is still loading.

== Model tree <model-tree>

The tree on the left has three tabs:

/ Geometry: imported CAD parts and the mesh setup (@geometry).
/ FE Model: mesh parts, materials, sections, constraints, contacts, steps and the analysis.
/ Results: the opened result files with their fields, history outputs, paths, planes and hot
  spots (@results).

#ui("Geometry") and #ui("FE Model") share one camera, #ui("Results") has its own. A model that
has no mesh yet opens on the #ui("Geometry") tab, opening a result file switches to
#ui("Results").

=== Working with tree items

- *Double-click* an item to open its dialog. Double-clicking a container (for example
  #ui("Materials")) creates a new item in it. Double-clicking #ui("Analysis-1") opens the
  monitor.
- *Right-click* an item for its context menu (see below).
- *Del* deletes the selected item after asking "Delete the selected item?". There is no undo.
- *Space* activates or deactivates the selected item while the mouse is over the tree.
- Selecting an item highlights its region in the 3D view. Master and slave regions of contacts and
  ties get different colours.
- Parts can be selected together: #key("Ctrl")+click toggles a part, #key("Shift")+click
  selects a range.

=== Context menus

/ Containers: #ui("Create ..."), #ui("Expand All"), #ui("Collapse All"). Some containers have
  more entries: #ui("Material Library ...") on #ui("Materials"), #ui("Default Mesh Parameters
  ...") and #ui("Mesh All Parts") on #ui("Mesh Setup"), #ui("Create Node Tie ...") on
  #ui("Contact Pairs"), #ui("Search Contact Pairs ...") on #ui("Constraints") and
  #ui("Contact Pairs"), #ui("Show Table") on #ui("Hot Spot Stresses"). A container that cannot
  take items explains why in a tooltip, for example "A frequency step has no loads."
/ Items: #ui("Edit ..."), #ui("Activate")/#ui("Deactivate"), #ui("Swap Master/Slave") (ties,
  spring connections and contact pairs), #ui("Delete").
/ Parts: #ui("Properties ..."), #ui("Hide")/#ui("Show"), #ui("Delete"); on the
  #ui("Geometry") tab also #ui("Create Mesh").

=== Deactivating items

Steps, boundary conditions, loads, constraints, contact pairs, node ties and initial conditions
can be deactivated with #ui("Deactivate") or #key("Space"). A deactivated item is shown grey
with a red no-entry sign; clicking the sign activates it again. Deactivated items stay in the
model but are written to the input file only as a comment, so CalculiX ignores them.

=== Renaming

Tree items are renamed in the #ui("Name") field of their dialog. Mesh parts are renamed in their
#ui("Properties") dialog. Names are converted to upper case and may contain letters (no umlauts),
digits, `_` and `-`, up to 80 characters. Items that refer to a renamed part, material or
amplitude follow the new name. Geometry parts cannot be renamed, because the mesh setup refers to
them by name.

=== Properties dialogs

A double-click on a part opens its properties: name, number of elements and nodes, element types,
colour and visibility. On the #ui("Results") tab, a double-click on #ui("Model") shows file,
load time, node and element counts, element types, the dimensions of the model and its sets and
surfaces. A double-click on a result component shows its maximum and minimum with the node where
they occur.
