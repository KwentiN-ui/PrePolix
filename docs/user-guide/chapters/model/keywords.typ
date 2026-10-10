#import "../../template.typ": *

== Keyword editor <keyword-editor>

#menu("Model", "Edit CalculiX Keywords ...") shows the input file PrePolix writes, as a tree of
keywords on the left and as text on the right. Here you add CalculiX keywords for features the
dialogs do not offer yet.

#screenshot("keyword-editor.png", [Keyword editor])

- Select a place in the tree and press #ui("Add") to insert a user keyword there. Type the
  keyword and its data lines under #ui("Edit Selected Keyword").
- #ui("Move Up") and #ui("Move Down") move a user keyword, #ui("Delete") removes it.
- #ui("Active") switches a user keyword off without deleting it; it is then written as a
  comment.
- Generated keywords are read-only. #ui("Hide data (faster)") shortens long data blocks in the
  preview.

Each step has empty places such as #ui("Controls"), #ui("Output frequency") and #ui("Defined
fields") for keywords like `*CONTROLS`, `*OUTPUT` or `*TEMPERATURE`.

User keywords are saved in the project and written every time the input file is written (run,
model check, export). If the model changes so that a keyword has no valid place any more,
PrePolix warns that it will not be written.
