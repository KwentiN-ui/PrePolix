//! Reader for CalculiX result files (`.frd`), ASCII and binary.
//!
//! An `.frd` file carries its own mesh (nodes and elements, shells and beams already expanded
//! to solids by CalculiX) followed by one block per result field and increment. Component names
//! follow PrePoMax: `D1` becomes `U1`, `SXX` becomes `S11` and so on.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use plx_mesh::{Element, ElementShape, FeMesh, Part};
use plx_results::{AnalysisKind, Component, Field, Increment, add_derived_components};

#[derive(Debug, thiserror::Error)]
pub enum FrdError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Zeile {line}: {message}")]
    Parse { line: usize, message: String },
}

#[derive(Debug, Default)]
pub struct FrdImport {
    pub mesh: FeMesh,
    pub increments: Vec<Increment>,
    /// Material names by CalculiX material number.
    pub materials: BTreeMap<i32, String>,
    /// Date and time of the analysis as CalculiX writes them (`1UDATE`, `1UTIME`).
    pub date: Option<String>,
    pub time: Option<String>,
    pub warnings: Vec<String>,
}

pub fn read_frd(path: &Path) -> Result<FrdImport, FrdError> {
    let bytes = std::fs::read(path).map_err(|source| FrdError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    read_frd_bytes(&bytes)
}

pub fn read_frd_bytes(bytes: &[u8]) -> Result<FrdImport, FrdError> {
    Reader {
        data: bytes,
        pos: 0,
        line: 0,
        import: FrdImport::default(),
        element_materials: BTreeMap::new(),
        pending: None,
        blocks: Vec::new(),
    }
    .run()
}

/// Header of the result block being read: step, increment and analysis.
#[derive(Clone, Copy, Default)]
struct StepHeader {
    step: u32,
    increment: u32,
    mode: Option<u32>,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    line: usize,
    import: FrdImport,
    element_materials: BTreeMap<i32, Vec<u32>>,
    pending: Option<StepHeader>,
    /// Result blocks found so far; their values are parsed in parallel at the end.
    blocks: Vec<ResultBlock<'a>>,
}

/// A result block whose header has been read, with its values still unparsed.
struct ResultBlock<'a> {
    step: u32,
    increment: u32,
    kind: AnalysisKind,
    value: f64,
    field_name: String,
    names: Vec<String>,
    /// Bytes per binary float, `None` for ASCII.
    binary: Option<usize>,
    body: &'a [u8],
}

impl<'a> Reader<'a> {
    fn run(mut self) -> Result<FrdImport, FrdError> {
        while let Some(line) = self.next_line() {
            if line.starts_with("    1UMAT") {
                self.material(&line);
            } else if let Some(date) = line.strip_prefix("    1UDATE") {
                self.import.date = Some(date.trim().to_string()).filter(|d| !d.is_empty());
            } else if let Some(time) = line.strip_prefix("    1UTIME") {
                self.import.time = Some(time.trim().to_string()).filter(|t| !t.is_empty());
            } else if line.starts_with("    2C") {
                self.nodes(&line)?;
            } else if line.starts_with("    3C") {
                self.elements(&line)?;
            } else if line.starts_with("    1PSTEP") {
                self.pending = Some(step_header(&line));
            } else if let Some(mode) = line.strip_prefix("    1PMODE") {
                if let Some(header) = &mut self.pending {
                    header.mode = mode.trim().parse().ok();
                }
            } else if line.starts_with("  100C") {
                self.results(&line)?;
            } else if line.starts_with("9999") {
                break;
            }
        }
        self.finish()
    }

    fn finish(mut self) -> Result<FrdImport, FrdError> {
        let blocks = std::mem::take(&mut self.blocks);
        let mesh = &self.import.mesh;
        let fields = parallel_map(&blocks, |block| {
            let mut field = Field {
                name: block.field_name.clone(),
                components: block
                    .names
                    .iter()
                    .zip(block_values(block, mesh))
                    .map(|(name, values)| Component {
                        name: name.clone(),
                        values,
                        derived: false,
                    })
                    .collect(),
            };
            add_derived_components(&mut field);
            field
        });
        for (block, field) in blocks.iter().zip(fields) {
            let target = self.import.increments.iter_mut().find(|i| {
                i.step == block.step && i.increment == block.increment && i.kind == block.kind
            });
            match target {
                Some(target) => target.fields.push(field),
                None => self.import.increments.push(Increment {
                    step: block.step,
                    increment: block.increment,
                    kind: block.kind,
                    value: block.value,
                    fields: vec![field],
                }),
            }
        }
        let materials = std::mem::take(&mut self.element_materials);
        for (material, elements) in materials {
            let name = self
                .import
                .materials
                .get(&material)
                .cloned()
                .unwrap_or_else(|| {
                    if material > 0 {
                        format!("MATERIAL-{material}")
                    } else {
                        "PART-1".to_string()
                    }
                });
            self.import.mesh.parts.push(Part { name, elements });
        }
        Ok(self.import)
    }

    /// Next text line without the line break; `None` at the end of the data. Borrowed from
    /// the data unless it is not valid UTF-8.
    fn next_line(&mut self) -> Option<Cow<'a, str>> {
        if self.pos >= self.data.len() {
            return None;
        }
        let data: &'a [u8] = self.data;
        let rest = &data[self.pos..];
        let end = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
        self.pos += (end + 1).min(rest.len());
        self.line += 1;
        let text = &rest[..end];
        let text = text.strip_suffix(b"\r").unwrap_or(text);
        Some(String::from_utf8_lossy(text))
    }

    fn error(&self, message: impl Into<String>) -> FrdError {
        FrdError::Parse {
            line: self.line,
            message: message.into(),
        }
    }

    fn take_bytes(&mut self, count: usize) -> Result<&'a [u8], FrdError> {
        let end = self
            .pos
            .checked_add(count)
            .filter(|&end| end <= self.data.len())
            .ok_or_else(|| self.error("Binärdaten enden vorzeitig"))?;
        let data: &'a [u8] = self.data;
        self.pos = end;
        Ok(&data[end - count..end])
    }

    fn material(&mut self, line: &str) {
        // "    1UMAT    1STEEL": material number in five columns, then the name.
        let rest = line.get(9..).unwrap_or("");
        let (number, name) = rest.split_at(rest.len().min(5));
        if let Ok(number) = number.trim().parse() {
            self.import
                .materials
                .insert(number, name.trim().to_string());
        }
    }

    fn nodes(&mut self, header: &str) -> Result<(), FrdError> {
        let count = header_count(header, 6..36);
        match binary_size(header.get(36..)) {
            Some(size) => {
                let record = 4 + 3 * size;
                let block = self.take_bytes(count * record)?;
                let mut nodes = Vec::with_capacity(count);
                for chunk in block.chunks_exact(record) {
                    let id = i32_at(chunk, 0);
                    let xyz = [0, 1, 2].map(|k| float_at(chunk, 4 + k * size, size));
                    nodes.push((id, xyz));
                }
                for (id, xyz) in nodes {
                    self.add_node(id, xyz);
                }
            }
            None => {
                while let Some(line) = self.next_line() {
                    if line.starts_with(" -3") {
                        break;
                    }
                    if !line.starts_with(" -1") {
                        continue;
                    }
                    let id = int_field(&line, 3..13)
                        .ok_or_else(|| self.error(format!("Ungültige Knotenzeile: {line}")))?;
                    let values = fixed_floats(&line, 13, 3);
                    if values.len() < 3 {
                        return Err(self.error(format!("Knoten {id} hat keine drei Koordinaten")));
                    }
                    self.add_node(id, [values[0], values[1], values[2]]);
                }
            }
        }
        Ok(())
    }

    fn add_node(&mut self, id: i32, xyz: [f64; 3]) {
        if id > 0 && self.import.mesh.set_node(id as u32, xyz) {
            self.import
                .warnings
                .push(format!("Knoten {id} ist mehrfach definiert"));
        }
    }

    fn elements(&mut self, header: &str) -> Result<(), FrdError> {
        let count = header_count(header, 6..36);
        match binary_size(header.get(36..)) {
            Some(_) => {
                for _ in 0..count {
                    let head = self.take_bytes(16)?;
                    let [id, kind, _group, material] = [0, 4, 8, 12].map(|o| i32_at(head, o));
                    let node_count = element_kind(kind)
                        .map(|(shape, _)| shape.node_count())
                        .ok_or_else(|| self.error(format!("Unbekannter Elementtyp {kind}")))?;
                    let nodes = self.take_bytes(node_count * 4)?;
                    let nodes: Vec<i32> = (0..node_count).map(|k| i32_at(nodes, k * 4)).collect();
                    self.add_element(id, kind, material, nodes);
                }
            }
            None => {
                let mut current: Option<(i32, i32, i32, Vec<i32>)> = None;
                while let Some(line) = self.next_line() {
                    let tokens: Vec<i64> = line
                        .split_whitespace()
                        .map_while(|t| t.parse().ok())
                        .collect();
                    match tokens.first() {
                        Some(-1) => {
                            if let Some((id, kind, material, nodes)) = current.take() {
                                self.add_element(id, kind, material, nodes);
                            }
                            let [id, kind] = [1, 2].map(|k| tokens.get(k).copied().unwrap_or(0));
                            let material = tokens.get(4).copied().unwrap_or(-1);
                            current = Some((id as i32, kind as i32, material as i32, Vec::new()));
                        }
                        Some(-2) => {
                            if let Some((.., nodes)) = &mut current {
                                nodes.extend(tokens[1..].iter().map(|&n| n as i32));
                            }
                        }
                        Some(-3) => break,
                        _ => {}
                    }
                }
                if let Some((id, kind, material, nodes)) = current {
                    self.add_element(id, kind, material, nodes);
                }
            }
        }
        Ok(())
    }

    fn add_element(&mut self, id: i32, kind: i32, material: i32, frd_nodes: Vec<i32>) {
        let Some((shape, type_name)) = element_kind(kind) else {
            self.import.warnings.push(format!(
                "Element {id}: Elementtyp {kind} wird nicht unterstützt"
            ));
            return;
        };
        let count = shape.node_count();
        if frd_nodes.len() < count {
            self.import
                .warnings
                .push(format!("Element {id} ist unvollständig"));
            return;
        }
        let nodes = reorder_nodes(shape, &frd_nodes[..count])
            .into_iter()
            .map(|n| n as u32)
            .collect();
        let element = Element {
            id: id as u32,
            type_name: type_name.to_string(),
            shape,
            nodes,
        };
        match self.import.mesh.add_element(element) {
            Ok(()) => self
                .element_materials
                .entry(material)
                .or_default()
                .push(id as u32),
            Err(error) => self.import.warnings.push(error.to_string()),
        }
    }

    fn results(&mut self, header: &str) -> Result<(), FrdError> {
        let value_count = header_count(header, 24..36);
        let value = header
            .get(12..24)
            .and_then(|t| parse_float(t.trim()))
            .unwrap_or(0.0);
        let kind = match int_field(header, 56..58).unwrap_or(0) {
            0 => AnalysisKind::Static,
            1 => AnalysisKind::Dynamic,
            2 => AnalysisKind::Frequency,
            4 => AnalysisKind::Buckling,
            other => AnalysisKind::Other(other),
        };
        let binary = binary_size(header.get(73..75));
        let step = self.pending.take().unwrap_or(StepHeader {
            step: 1,
            increment: 1,
            mode: None,
        });

        let field_line = self
            .next_line()
            .ok_or_else(|| self.error("Ergebnisblock endet nach dem Kopf"))?;
        let field_name = field_line
            .split_whitespace()
            .nth(1)
            .unwrap_or("UNBEKANNT")
            .to_string();
        let declared = int_field(&field_line, 13..18).unwrap_or(0).max(0) as usize;
        let mut names = Vec::new();
        for _ in 0..declared {
            let line = self
                .next_line()
                .ok_or_else(|| self.error("Komponentenliste endet vorzeitig"))?;
            // Components flagged as existing elsewhere (like ALL) carry no values.
            if int_field(&line, 33..38).unwrap_or(0) == 0 {
                let name = line.get(5..13).unwrap_or("").trim();
                names.push(rename_component(name).to_string());
            }
        }

        let start = self.pos;
        match binary {
            Some(size) => {
                self.take_bytes(value_count * (4 + names.len() * size))?;
            }
            None => {
                // Up to the end of block line " -3".
                while let Some(line) = self.next_raw_line() {
                    if line.starts_with(b" -3") {
                        break;
                    }
                }
            }
        }
        let increment = match (kind, step.mode) {
            (AnalysisKind::Frequency, Some(mode)) => mode,
            _ => step.increment,
        };
        let data: &'a [u8] = self.data;
        self.blocks.push(ResultBlock {
            step: step.step,
            increment,
            kind,
            value,
            field_name,
            names,
            binary,
            body: &data[start..self.pos],
        });
        Ok(())
    }

    /// Like [`Reader::next_line`] without the text conversion, for skipping lines quickly.
    fn next_raw_line(&mut self) -> Option<&'a [u8]> {
        if self.pos >= self.data.len() {
            return None;
        }
        let data: &'a [u8] = self.data;
        let rest = &data[self.pos..];
        let end = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
        self.pos += (end + 1).min(rest.len());
        self.line += 1;
        Some(&rest[..end])
    }
}

/// The values of a result block, one column per component, by node index of the mesh.
fn block_values(block: &ResultBlock, mesh: &FeMesh) -> Vec<Vec<f32>> {
    let names = block.names.len();
    let mut columns = vec![vec![f32::NAN; mesh.node_count()]; names];
    match block.binary {
        Some(size) => {
            for chunk in block.body.chunks_exact(4 + names * size) {
                let id = i32_at(chunk, 0);
                if let Some(index) = mesh.node_index(id as u32) {
                    for (k, column) in columns.iter_mut().enumerate() {
                        column[index] = float_at(chunk, 4 + k * size, size) as f32;
                    }
                }
            }
        }
        None => {
            let mut index = None;
            let mut filled = 0;
            let mut values = Vec::with_capacity(names);
            for line in block.body.split(|&b| b == b'\n') {
                let line = String::from_utf8_lossy(line.strip_suffix(b"\r").unwrap_or(line));
                if line.starts_with(" -1") {
                    index = int_field(&line, 3..13).and_then(|id| mesh.node_index(id as u32));
                    filled = 0;
                } else if !line.starts_with(" -2") {
                    continue;
                }
                let Some(index) = index else { continue };
                values.clear();
                fixed_floats_into(&line, 13, names - filled.min(names), &mut values);
                for &value in &values {
                    if let Some(column) = columns.get_mut(filled) {
                        column[index] = value as f32;
                    }
                    filled += 1;
                }
            }
        }
    }
    columns
}

/// Maps the items on all cores, keeping their order.
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(items.len().max(1));
    if threads <= 1 {
        return items.iter().map(f).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut results: Vec<(usize, R)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        done.push((i, f(item)));
                    }
                    done
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("result worker panicked"))
            .collect()
    });
    results.sort_unstable_by_key(|&(i, _)| i);
    results.into_iter().map(|(_, r)| r).collect()
}

/// `    1PSTEP   <data set> <increment> <step>`.
fn step_header(line: &str) -> StepHeader {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let number = |k: usize| tokens.get(k).and_then(|t| t.parse().ok());
    StepHeader {
        increment: number(2).unwrap_or(1),
        step: number(3).unwrap_or(1),
        mode: None,
    }
}

fn rename_component(name: &str) -> &str {
    match name {
        "D1" => "U1",
        "D2" => "U2",
        "D3" => "U3",
        "SXX" => "S11",
        "SYY" => "S22",
        "SZZ" => "S33",
        "SXY" => "S12",
        "SYZ" => "S23",
        "SZX" => "S13",
        "EXX" => "E11",
        "EYY" => "E22",
        "EZZ" => "E33",
        "EXY" => "E12",
        "EYZ" => "E23",
        "EZX" => "E13",
        "MEXX" => "ME11",
        "MEYY" => "ME22",
        "MEZZ" => "ME33",
        "MEXY" => "ME12",
        "MEYZ" => "ME23",
        "MEZX" => "ME13",
        "TEM(%)" => "TEM",
        "STR(%)" => "STR",
        other => other,
    }
}

/// Element shape and an Abaqus-style type name for an frd element type number.
fn element_kind(kind: i32) -> Option<(ElementShape, &'static str)> {
    Some(match kind {
        1 => (ElementShape::Hex8, "C3D8"),
        2 => (ElementShape::Wedge6, "C3D6"),
        3 => (ElementShape::Tet4, "C3D4"),
        4 => (ElementShape::Hex20, "C3D20"),
        5 => (ElementShape::Wedge15, "C3D15"),
        6 => (ElementShape::Tet10, "C3D10"),
        7 => (ElementShape::Tri3, "S3"),
        8 => (ElementShape::Tri6, "S6"),
        9 => (ElementShape::Quad4, "S4"),
        10 => (ElementShape::Quad8, "S8"),
        11 => (ElementShape::Line2, "B31"),
        12 => (ElementShape::Line3, "B32"),
        _ => return None,
    })
}

/// frd lists the vertical edge mid nodes of quadratic hexahedra and wedges before the top ones.
fn reorder_nodes(shape: ElementShape, frd: &[i32]) -> Vec<i32> {
    let mut nodes = frd.to_vec();
    match shape {
        ElementShape::Hex20 => {
            nodes[12..16].copy_from_slice(&frd[16..20]);
            nodes[16..20].copy_from_slice(&frd[12..16]);
        }
        ElementShape::Wedge15 => {
            nodes[9..12].copy_from_slice(&frd[12..15]);
            nodes[12..15].copy_from_slice(&frd[9..12]);
        }
        _ => {}
    }
    nodes
}

fn header_count(line: &str, columns: std::ops::Range<usize>) -> usize {
    line.get(columns)
        .and_then(|t| t.trim().parse::<usize>().ok())
        .unwrap_or(0)
}

/// Size in bytes of a binary float for format flags 2 (f32) and 3 (f64); `None` for ASCII.
fn binary_size(flag: Option<&str>) -> Option<usize> {
    match flag.map(str::trim) {
        Some("2") => Some(4),
        Some("3") => Some(8),
        _ => None,
    }
}

fn int_field(line: &str, columns: std::ops::Range<usize>) -> Option<i32> {
    line.get(columns)?.trim().parse().ok()
}

fn parse_float(text: &str) -> Option<f64> {
    text.parse()
        .ok()
        .or_else(|| text.replace(['D', 'd'], "E").parse().ok())
}

/// Reads up to `count` numbers of 12 columns each from `start` on. Older CalculiX builds wrote
/// three-digit exponents, which makes negative numbers 13 columns wide; a field followed by a
/// digit is read with 13 columns.
fn fixed_floats(line: &str, start: usize, count: usize) -> Vec<f64> {
    let mut values = Vec::with_capacity(count);
    fixed_floats_into(line, start, count, &mut values);
    values
}

fn fixed_floats_into(line: &str, start: usize, count: usize, values: &mut Vec<f64>) {
    let bytes = line.as_bytes();
    let target = values.len() + count;
    let mut pos = start;
    while values.len() < target && pos < bytes.len() {
        let field = |width: usize| {
            line.get(pos..(pos + width).min(bytes.len()))
                .and_then(|t| parse_float(t.trim()))
        };
        let width = if next_starts_mid_number(bytes, pos + 12) {
            13
        } else {
            12
        };
        match field(width) {
            Some(value) => values.push(value),
            None => values.push(f64::NAN),
        }
        pos += width;
    }
}

/// True if the byte at `pos` continues a number (a digit right after the field boundary).
fn next_starts_mid_number(bytes: &[u8], pos: usize) -> bool {
    bytes.get(pos).is_some_and(|b| b.is_ascii_digit())
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn float_at(bytes: &[u8], offset: usize, size: usize) -> f64 {
    if size == 8 {
        f64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
    } else {
        f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as f64
    }
}

#[cfg(test)]
mod tests;
