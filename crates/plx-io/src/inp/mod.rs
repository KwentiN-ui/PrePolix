//! Reader for CalculiX/Abaqus input files (`.inp`).
//!
//! Currently reads the mesh: nodes, elements, node and element sets, and surfaces.
//! Every other keyword is skipped and reported in [`InpImport::skipped_keywords`].

mod lines;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use plx_mesh::{Element, ElementId, ElementShape, FeMesh, NodeId, Part, SurfaceDefinition};

use lines::{Keyword, SourceLine, fields, is_keyword, parse_f64};

#[derive(Debug, thiserror::Error)]
pub enum InpError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{file}:{line}: {message}")]
    Parse {
        file: PathBuf,
        line: usize,
        message: String,
    },
}

#[derive(Debug, Default)]
pub struct InpImport {
    pub mesh: FeMesh,
    /// Problems that did not stop the import, ready to show to the user.
    pub warnings: Vec<String>,
    /// Keywords not read yet, with how often they occurred.
    pub skipped_keywords: BTreeMap<String, usize>,
    /// The file itself followed by every included file.
    pub files: Vec<PathBuf>,
}

pub fn read_inp(path: &Path) -> Result<InpImport, InpError> {
    let mut files = Vec::new();
    let lines = lines::load(path, &mut files)?;
    Reader::new(files).run(&lines)
}

/// Reads input given as text; `*INCLUDE` paths resolve against `base_dir`.
pub fn read_inp_str(text: &str, base_dir: Option<&Path>) -> Result<InpImport, InpError> {
    let mut files = Vec::new();
    let lines = lines::expand(text, base_dir, PathBuf::from("<text>"), &mut files)?;
    Reader::new(files).run(&lines)
}

enum Block {
    Skip,
    Node {
        set: Option<String>,
    },
    Element {
        type_name: String,
        shape: ElementShape,
        set: String,
        pending: Vec<u32>,
        pending_line: usize,
    },
    NodeSet {
        name: String,
        generate: bool,
    },
    ElementSet {
        name: String,
        generate: bool,
    },
    Surface {
        name: String,
        by_nodes: bool,
    },
}

struct Reader {
    import: InpImport,
    unsupported_elements: BTreeSet<String>,
    duplicate_nodes: usize,
}

impl Reader {
    fn new(files: Vec<PathBuf>) -> Self {
        Self {
            import: InpImport {
                files,
                ..Default::default()
            },
            unsupported_elements: BTreeSet::new(),
            duplicate_nodes: 0,
        }
    }

    fn run(mut self, lines: &[SourceLine]) -> Result<InpImport, InpError> {
        let mut block = Block::Skip;
        for line in lines {
            if is_keyword(&line.text) {
                self.finish(&block, line)?;
                block = self.start(Keyword::parse(&line.text), line)?;
            } else {
                self.data(&mut block, line)?;
            }
        }
        if let Some(last) = lines.last() {
            self.finish(&block, last)?;
        }
        self.report();
        Ok(self.import)
    }

    fn error(&self, line: &SourceLine, message: impl Into<String>) -> InpError {
        InpError::Parse {
            file: self.import.files[line.file].clone(),
            line: line.number,
            message: message.into(),
        }
    }

    fn start(&mut self, keyword: Keyword, line: &SourceLine) -> Result<Block, InpError> {
        let required = |key: &str| {
            keyword
                .param(key)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_ascii_uppercase())
                .ok_or_else(|| self.error(line, format!("*{} ohne {key}=", keyword.name)))
        };
        Ok(match keyword.name.as_str() {
            "NODE" => Block::Node {
                set: keyword.param("NSET").map(str::to_ascii_uppercase),
            },
            "ELEMENT" => {
                let type_name = required("TYPE")?;
                match ElementShape::from_type_name(&type_name) {
                    Some(shape) => Block::Element {
                        set: keyword
                            .param("ELSET")
                            .map_or_else(|| type_name.clone(), str::to_ascii_uppercase),
                        type_name,
                        shape,
                        pending: Vec::new(),
                        pending_line: line.number,
                    },
                    None => {
                        self.unsupported_elements.insert(type_name);
                        Block::Skip
                    }
                }
            }
            "NSET" => Block::NodeSet {
                name: required("NSET")?,
                generate: keyword.has_flag("GENERATE"),
            },
            "ELSET" => Block::ElementSet {
                name: required("ELSET")?,
                generate: keyword.has_flag("GENERATE"),
            },
            "SURFACE" => Block::Surface {
                name: required("NAME")?,
                by_nodes: keyword
                    .param("TYPE")
                    .is_some_and(|t| t.eq_ignore_ascii_case("NODE")),
            },
            _ => {
                *self
                    .import
                    .skipped_keywords
                    .entry(keyword.name)
                    .or_default() += 1;
                Block::Skip
            }
        })
    }

    fn data(&mut self, block: &mut Block, line: &SourceLine) -> Result<(), InpError> {
        match block {
            Block::Skip => {}
            Block::Node { set } => {
                let values: Vec<&str> = fields(&line.text).collect();
                let id = parse_id(values[0])
                    .ok_or_else(|| self.error(line, "Ungültige Knotennummer"))?;
                let mut coords = [0.0; 3];
                for (axis, value) in values[1..].iter().take(3).enumerate() {
                    if !value.is_empty() {
                        coords[axis] = parse_f64(value).ok_or_else(|| {
                            self.error(line, format!("Ungültige Koordinate '{value}'"))
                        })?;
                    }
                }
                let mesh = &mut self.import.mesh;
                if mesh.set_node(id, coords) {
                    self.duplicate_nodes += 1;
                }
                if let Some(set) = set {
                    mesh.node_sets.entry(set.clone()).or_default().push(id);
                }
            }
            Block::Element {
                type_name,
                shape,
                set,
                pending,
                pending_line,
            } => {
                if pending.is_empty() {
                    *pending_line = line.number;
                }
                for value in fields(&line.text) {
                    pending.push(
                        parse_id(value).ok_or_else(|| {
                            self.error(line, format!("Ungültige Nummer '{value}'"))
                        })?,
                    );
                }
                if pending.len() > shape.node_count() + 1 {
                    return Err(self.error(
                        line,
                        format!(
                            "Element {} vom Typ {type_name} hat mehr als {} Knoten",
                            pending[0],
                            shape.node_count()
                        ),
                    ));
                }
                if pending.len() == shape.node_count() + 1 {
                    let element = Element {
                        id: pending[0],
                        type_name: type_name.clone(),
                        shape: *shape,
                        nodes: pending[1..].to_vec(),
                    };
                    let id = element.id;
                    let added = self.import.mesh.add_element(element);
                    added.map_err(|e| self.error(line, e.to_string()))?;
                    pending.clear();
                    let mesh = &mut self.import.mesh;
                    mesh.element_sets.entry(set.clone()).or_default().push(id);
                    match mesh.parts.iter_mut().find(|p| p.name == *set) {
                        Some(part) => part.elements.push(id),
                        None => mesh.parts.push(Part {
                            name: set.clone(),
                            elements: vec![id],
                        }),
                    }
                }
            }
            Block::NodeSet { name, generate } => {
                let ids = self.set_members(line, *generate, |mesh, set| {
                    mesh.node_sets.get(set).cloned()
                })?;
                self.import
                    .mesh
                    .node_sets
                    .entry(name.clone())
                    .or_default()
                    .extend(ids);
            }
            Block::ElementSet { name, generate } => {
                let ids = self.set_members(line, *generate, |mesh, set| {
                    mesh.element_sets.get(set).cloned()
                })?;
                self.import
                    .mesh
                    .element_sets
                    .entry(name.clone())
                    .or_default()
                    .extend(ids);
            }
            Block::Surface { name, by_nodes } => self.surface_data(line, name, *by_nodes)?,
        }
        Ok(())
    }

    /// Ids listed on a set data line, either explicitly, as `GENERATE` ranges or via other sets.
    fn set_members(
        &mut self,
        line: &SourceLine,
        generate: bool,
        lookup: impl Fn(&FeMesh, &str) -> Option<Vec<u32>>,
    ) -> Result<Vec<u32>, InpError> {
        let values: Vec<&str> = fields(&line.text).filter(|v| !v.is_empty()).collect();
        if generate {
            let numbers = values
                .iter()
                .map(|v| parse_id(v))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| self.error(line, "GENERATE erwartet Zahlen"))?;
            let (start, end, step) = match numbers.as_slice() {
                [start, end] => (*start, *end, 1),
                [start, end, step] if *step > 0 => (*start, *end, *step),
                _ => return Err(self.error(line, "GENERATE erwartet Anfang, Ende[, Schritt]")),
            };
            return Ok((start..=end).step_by(step as usize).collect());
        }
        let mut ids = Vec::new();
        for value in values {
            if let Some(id) = parse_id(value) {
                ids.push(id);
            } else {
                match lookup(&self.import.mesh, &value.to_ascii_uppercase()) {
                    Some(members) => ids.extend(members),
                    None => self.warn(line, format!("Set '{value}' ist nicht definiert")),
                }
            }
        }
        Ok(ids)
    }

    fn surface_data(
        &mut self,
        line: &SourceLine,
        name: &str,
        by_nodes: bool,
    ) -> Result<(), InpError> {
        let values: Vec<&str> = fields(&line.text).collect();
        let mesh = &self.import.mesh;
        if by_nodes {
            let ids = match parse_id(values[0]) {
                Some(id) => vec![id],
                None => mesh
                    .node_sets
                    .get(&values[0].to_ascii_uppercase())
                    .cloned()
                    .unwrap_or_default(),
            };
            match self
                .import
                .mesh
                .surfaces
                .entry(name.to_string())
                .or_insert_with(|| SurfaceDefinition::Nodes(Vec::new()))
            {
                SurfaceDefinition::Nodes(nodes) => nodes.extend(ids),
                SurfaceDefinition::ElementFaces(_) => self.warn(
                    line,
                    format!("Surface {name} mischt Knoten und Elementflächen"),
                ),
            }
            return Ok(());
        }
        let face = values
            .get(1)
            .and_then(|label| face_number(label))
            .ok_or_else(|| self.error(line, "Elementfläche erwartet, z. B. 'EALL, S2'"))?;
        let elements: Vec<ElementId> = match parse_id(values[0]) {
            Some(id) => vec![id],
            None => match mesh.element_sets.get(&values[0].to_ascii_uppercase()) {
                Some(set) => set.clone(),
                None => {
                    self.warn(line, format!("Set '{}' ist nicht definiert", values[0]));
                    Vec::new()
                }
            },
        };
        match self
            .import
            .mesh
            .surfaces
            .entry(name.to_string())
            .or_insert_with(|| SurfaceDefinition::ElementFaces(Vec::new()))
        {
            SurfaceDefinition::ElementFaces(faces) => {
                faces.extend(elements.into_iter().map(|e| (e, face)))
            }
            SurfaceDefinition::Nodes(_) => self.warn(
                line,
                format!("Surface {name} mischt Knoten und Elementflächen"),
            ),
        }
        Ok(())
    }

    fn finish(&mut self, block: &Block, at: &SourceLine) -> Result<(), InpError> {
        if let Block::Element {
            pending,
            pending_line,
            type_name,
            ..
        } = block
            && !pending.is_empty()
        {
            let line = SourceLine {
                number: *pending_line,
                ..at.clone()
            };
            return Err(self.error(
                &line,
                format!(
                    "Element {} vom Typ {type_name} ist unvollständig",
                    pending[0]
                ),
            ));
        }
        Ok(())
    }

    fn warn(&mut self, line: &SourceLine, message: String) {
        let file = self.import.files[line.file]
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.import
            .warnings
            .push(format!("{file}:{}: {message}", line.number));
    }

    fn report(&mut self) {
        let warnings = &mut self.import.warnings;
        for type_name in &self.unsupported_elements {
            warnings.push(format!(
                "Elementtyp {type_name} wird noch nicht unterstützt und wurde übersprungen"
            ));
        }
        if self.duplicate_nodes > 0 {
            warnings.push(format!(
                "{} Knoten waren mehrfach definiert, die letzte Definition gilt",
                self.duplicate_nodes
            ));
        }
        let missing = self.import.mesh.missing_nodes();
        if let Some((element, node)) = missing.first() {
            warnings.push(format!(
                "{} Elementknoten verweisen auf fehlende Knoten (z. B. Element {element}, Knoten {node})",
                missing.len()
            ));
        }
    }
}

fn parse_id(field: &str) -> Option<NodeId> {
    field.parse().ok()
}

fn face_number(label: &str) -> Option<u8> {
    let label = label.trim().to_ascii_uppercase();
    let number: u8 = label.strip_prefix('S')?.parse().ok()?;
    (1..=6).contains(&number).then_some(number)
}

#[cfg(test)]
mod tests;
