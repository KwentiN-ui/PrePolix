// prepolix User Guide. Build with:
//   typst compile --ignore-system-fonts docs/user-guide/main.typ prepolix-user-guide.pdf
// The release workflow passes the version with `--input version=v1.2.3`.
// Each topic lives in its own file under chapters/, so that changes to different features do
// not touch the same file. See README.md in this folder.

#import "template.typ": *

#show: guide.with(version: sys.inputs.at("version", default: "dev"))

#include "chapters/introduction.typ"
#include "chapters/first-analysis.typ"

#include "chapters/interface/main-window.typ"
#include "chapters/interface/files.typ"
#include "chapters/interface/view.typ"
#include "chapters/interface/selection.typ"
#include "chapters/interface/input.typ"
#include "chapters/interface/settings.typ"

#include "chapters/geometry-mesh/geometry.typ"
#include "chapters/geometry-mesh/mesh.typ"

#include "chapters/model/model-properties.typ"
#include "chapters/model/materials.typ"
#include "chapters/model/sections.typ"
#include "chapters/model/shell-section.typ"
#include "chapters/model/features.typ"
#include "chapters/model/interactions.typ"
#include "chapters/model/amplitudes.typ"
#include "chapters/model/initial-conditions.typ"
#include "chapters/model/steps.typ"
#include "chapters/model/bcs-loads.typ"
#include "chapters/model/keywords.typ"

#include "chapters/analysis/run.typ"

#include "chapters/results/viewing.typ"
#include "chapters/results/animation.typ"
#include "chapters/results/evaluation.typ"
