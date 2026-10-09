//! PrePoMax's CalculiX keyword editor (Model > Edit CalculiX Keywords).
//!
//! The editor shows the input file as the keyword tree the writer builds from the model. The
//! user adds own keywords anywhere in it, for features the model cannot express yet; they are
//! kept in the FE model with their place in the tree and inserted whenever the input file is
//! written. Generated keywords are read-only.

use std::ops::Range;

use plx_io::inp::{Keyword, KeywordKind};
use plx_model::{FeModel, UserKeyword};

/// Text of a new keyword, a comment until the user writes something.
const NEW_KEYWORD: &str = "** User keyword";
/// Generated keywords with more lines show only their first line when data is hidden.
const HIDE_LINES: usize = 10;
const HIDDEN_DATA: &str = "... hidden data ...";

/// A keyword of the tree with its expansion state.
struct Node {
    keyword: Keyword,
    children: Vec<Node>,
    expanded: bool,
}

impl Node {
    fn new(mut keyword: Keyword) -> Self {
        let children = std::mem::take(&mut keyword.children)
            .into_iter()
            .map(Node::new)
            .collect();
        Self {
            keyword,
            children,
            expanded: false,
        }
    }

    fn to_keyword(&self) -> Keyword {
        Keyword {
            children: self.children.iter().map(Node::to_keyword).collect(),
            ..self.keyword.clone()
        }
    }

    fn is_user(&self) -> bool {
        matches!(self.keyword.kind, KeywordKind::User { .. })
    }

    fn label(&self) -> String {
        match &self.keyword.kind {
            KeywordKind::Title(name) => name.clone(),
            _ => self
                .keyword
                .text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("(leer)")
                .trim()
                .to_owned(),
        }
    }

    /// Lines shown in the input file preview.
    fn preview(&self, hide: bool) -> Vec<String> {
        let output = self.keyword.output();
        let mut lines: Vec<String> = output.lines().map(str::to_owned).collect();
        if hide && self.keyword.kind == KeywordKind::Generated && lines.len() > HIDE_LINES {
            lines.truncate(1);
            lines.push(HIDDEN_DATA.to_owned());
        }
        lines
    }
}

/// The nodes under the node at `path`; the empty path is the root.
fn children_mut<'a>(nodes: &'a mut Vec<Node>, path: &[usize]) -> Option<&'a mut Vec<Node>> {
    match path.split_first() {
        None => Some(nodes),
        Some((&first, rest)) => children_mut(&mut nodes.get_mut(first)?.children, rest),
    }
}

fn node_mut<'a>(nodes: &'a mut [Node], path: &[usize]) -> Option<&'a mut Node> {
    let (&first, rest) = path.split_first()?;
    let node = nodes.get_mut(first)?;
    if rest.is_empty() {
        Some(node)
    } else {
        node_mut(&mut node.children, rest)
    }
}

fn node<'a>(nodes: &'a [Node], path: &[usize]) -> Option<&'a Node> {
    let (&first, rest) = path.split_first()?;
    let node = nodes.get(first)?;
    if rest.is_empty() {
        Some(node)
    } else {
        self::node(&node.children, rest)
    }
}

/// What the user decided in the editor.
pub enum EditorResult {
    Open,
    Ok(Vec<UserKeyword>),
    Cancel,
}

pub struct KeywordEditor {
    nodes: Vec<Node>,
    root_expanded: bool,
    /// User keywords whose place no longer exists; kept unchanged, like PrePoMax's
    /// suppressed keywords.
    unplaced: Vec<UserKeyword>,
    /// Path of the selected node; the empty path is the root.
    selected: Option<Vec<usize>>,
    hide_data: bool,
    preview: Vec<String>,
    /// Preview lines of the selected keyword.
    preview_selection: Option<Range<usize>>,
    preview_dirty: bool,
    scroll_to_selection: bool,
}

impl KeywordEditor {
    /// Opens the editor on the keyword tree of the model. Fails if the model cannot be written.
    pub fn new(mesh: &plx_mesh::FeMesh, model: &FeModel, heading: &str) -> Result<Self, String> {
        let mut tree =
            plx_io::inp::model_keywords(mesh, model, heading).map_err(|e| e.to_string())?;
        let placed = plx_io::inp::insert_user_keywords(&mut tree, &model.user_keywords);
        let unplaced = model
            .user_keywords
            .iter()
            .zip(placed)
            .filter(|(_, placed)| !placed)
            .map(|(keyword, _)| keyword.clone())
            .collect();
        let mut editor = Self {
            nodes: tree.into_iter().map(Node::new).collect(),
            root_expanded: true,
            unplaced,
            selected: None,
            hide_data: true,
            preview: Vec::new(),
            preview_selection: None,
            preview_dirty: true,
            scroll_to_selection: false,
        };
        editor.expand_user_keywords();
        Ok(editor)
    }

    /// Opens the parents of every user keyword so the user sees what was added.
    fn expand_user_keywords(&mut self) {
        fn expand(nodes: &mut [Node]) -> bool {
            let mut any = false;
            for node in nodes {
                if expand(&mut node.children) {
                    node.expanded = true;
                    any = true;
                }
                any |= node.is_user();
            }
            any
        }
        expand(&mut self.nodes);
    }

    /// The user keywords in the order they are inserted again: those without a place first,
    /// as PrePoMax does.
    fn user_keywords(&self) -> Vec<UserKeyword> {
        let tree: Vec<Keyword> = self.nodes.iter().map(Node::to_keyword).collect();
        let mut keywords = self.unplaced.clone();
        keywords.extend(plx_io::inp::user_keywords(&tree));
        keywords
    }

    fn selected_node(&self) -> Option<&Node> {
        node(&self.nodes, self.selected.as_deref()?)
    }

    fn select(&mut self, path: Vec<usize>) {
        self.selected = Some(path);
        self.preview_dirty = true;
        self.scroll_to_selection = true;
    }

    /// Adds a keyword after the selected user keyword, or as the last child of the selected
    /// keyword or title.
    fn add(&mut self) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        let keyword = Node::new(Keyword::user(&UserKeyword {
            position: Vec::new(),
            text: NEW_KEYWORD.to_owned(),
            active: true,
        }));
        let new_path = if self.selected_node().is_some_and(Node::is_user) {
            let (&index, parent) = path.split_last().expect("a node has a parent");
            let Some(siblings) = children_mut(&mut self.nodes, parent) else {
                return;
            };
            siblings.insert(index + 1, keyword);
            [parent, &[index + 1]].concat()
        } else {
            match node_mut(&mut self.nodes, &path) {
                Some(node) => node.expanded = true,
                None => self.root_expanded = true,
            }
            let Some(children) = children_mut(&mut self.nodes, &path) else {
                return;
            };
            children.push(keyword);
            [&path[..], &[children.len() - 1]].concat()
        };
        self.select(new_path);
    }

    fn delete(&mut self) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        if !self.selected_node().is_some_and(Node::is_user) {
            return;
        }
        let (&index, parent) = path.split_last().expect("a node has a parent");
        if let Some(siblings) = children_mut(&mut self.nodes, parent) {
            siblings.remove(index);
            let next = if index < siblings.len() {
                Some([parent, &[index]].concat())
            } else if index > 0 {
                Some([parent, &[index - 1]].concat())
            } else {
                Some(parent.to_vec())
            };
            self.selected = next;
            self.preview_dirty = true;
        }
    }

    /// Where the selected user keyword goes when moved up or down, PrePoMax's way: it steps
    /// over plain siblings, into a neighbour that has children, and out of its parent at the
    /// first or last place. The path is valid after the keyword was removed.
    fn move_target(&self, up: bool) -> Option<Vec<usize>> {
        let path = self.selected.as_deref()?;
        if !self.selected_node()?.is_user() {
            return None;
        }
        let (&index, parent) = path.split_last()?;
        let siblings = match parent {
            [] => &self.nodes,
            _ => &node(&self.nodes, parent)?.children,
        };
        let enter = |n: &Node| !n.children.is_empty() && !n.is_user();
        let leave = |offset: usize| {
            let (&parent_index, grandparent) = parent.split_last()?;
            Some([grandparent, &[parent_index + offset]].concat())
        };
        if up {
            if index == 0 {
                return leave(0);
            }
            let previous = &siblings[index - 1];
            Some(if enter(previous) {
                [parent, &[index - 1, previous.children.len()]].concat()
            } else {
                [parent, &[index - 1]].concat()
            })
        } else {
            if index + 1 == siblings.len() {
                return leave(1);
            }
            Some(if enter(&siblings[index + 1]) {
                [parent, &[index, 0]].concat()
            } else {
                [parent, &[index + 1]].concat()
            })
        }
    }

    fn move_selected(&mut self, up: bool) {
        let (Some(target), Some(path)) = (self.move_target(up), self.selected.clone()) else {
            return;
        };
        let (&index, parent) = path.split_last().expect("checked by move_target");
        let Some(siblings) = children_mut(&mut self.nodes, parent) else {
            return;
        };
        let node = siblings.remove(index);
        let (&new_index, new_parent) = target.split_last().expect("a target has a parent");
        if let Some(parent) = node_mut(&mut self.nodes, new_parent) {
            parent.expanded = true;
        }
        if let Some(siblings) = children_mut(&mut self.nodes, new_parent) {
            siblings.insert(new_index, node);
            self.select(target);
        }
    }

    fn update_preview(&mut self) {
        fn add(
            nodes: &[Node],
            hide: bool,
            path: &mut Vec<usize>,
            selected: Option<&[usize]>,
            lines: &mut Vec<String>,
            selection: &mut Option<Range<usize>>,
        ) {
            for (index, node) in nodes.iter().enumerate() {
                path.push(index);
                let start = lines.len();
                lines.extend(node.preview(hide));
                if selected == Some(path.as_slice()) {
                    *selection = Some(start..lines.len());
                }
                add(&node.children, hide, path, selected, lines, selection);
                path.pop();
            }
        }
        self.preview.clear();
        self.preview_selection = None;
        let selected = self.selected.as_deref();
        add(
            &self.nodes,
            self.hide_data,
            &mut Vec::new(),
            selected,
            &mut self.preview,
            &mut self.preview_selection,
        );
        self.preview_dirty = false;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> EditorResult {
        if self.preview_dirty {
            self.update_preview();
        }
        let mut open = true;
        let mut result = EditorResult::Open;
        let screen = ctx.content_rect().size();
        let size = egui::vec2((screen.x * 0.9).min(1150.0), (screen.y * 0.85).min(760.0));
        egui::Window::new("CalculiX-Keyword-Editor")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size(size)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                let buttons_height = 30.0;
                let body = egui::vec2(size.x, size.y - buttons_height);
                ui.horizontal_top(|ui| {
                    let tree_width = (body.x * 0.36).max(220.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(tree_width, body.y),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.tree_side(ui, body.y),
                    );
                    ui.separator();
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), body.y),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.text_side(ui, body.y),
                    );
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if !self.unplaced.is_empty() {
                        ui.label(format!(
                            "{} Keyword(s) ohne gültigen Platz im geänderten Modell werden nicht geschrieben.",
                            self.unplaced.len()
                        ));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Abbrechen").clicked() {
                            result = EditorResult::Cancel;
                        }
                        if ui.button("OK").clicked() {
                            result = EditorResult::Ok(self.user_keywords());
                        }
                    });
                });
            });
        if !open {
            result = EditorResult::Cancel;
        }
        result
    }

    fn tree_side(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.strong("CalculiX-Keyword-Baum");
        let tree_height = height - 64.0;
        frame().show(ui, |ui| {
            ui.set_min_size(egui::vec2(ui.available_width(), tree_height));
            egui::ScrollArea::both()
                .id_salt("keyword tree")
                .max_height(tree_height)
                .auto_shrink(false)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let mut clicked = None;
                    let root_selected = self.selected.as_deref() == Some(&[]);
                    if tree_row(
                        ui,
                        0,
                        Some(&mut self.root_expanded),
                        root_selected,
                        egui::RichText::new("CalculiX inp file"),
                    )
                    .clicked()
                    {
                        clicked = Some(Vec::new());
                    }
                    if self.root_expanded {
                        let selected = self.selected.as_deref();
                        tree_ui(ui, &mut self.nodes, &mut Vec::new(), selected, &mut clicked);
                    }
                    if let Some(path) = clicked {
                        self.select(path);
                    }
                });
        });
        let user = self.selected_node().is_some_and(Node::is_user);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(self.selected.is_some(), egui::Button::new("Hinzufügen"))
                .on_hover_text("Neues Keyword unter dem gewählten Eintrag oder nach dem gewählten eigenen Keyword")
                .clicked()
            {
                self.add();
            }
            let up = self.move_target(true).is_some();
            if ui.add_enabled(up, egui::Button::new("Nach oben")).clicked() {
                self.move_selected(true);
            }
            let down = self.move_target(false).is_some();
            if ui.add_enabled(down, egui::Button::new("Nach unten")).clicked() {
                self.move_selected(false);
            }
            if ui.add_enabled(user, egui::Button::new("Löschen")).clicked() {
                self.delete();
            }
        });
    }

    fn text_side(&mut self, ui: &mut egui::Ui, height: f32) {
        let editor_height = (height * 0.38).max(120.0);
        ui.horizontal(|ui| {
            ui.strong("Gewähltes Keyword bearbeiten");
            let path = self.selected.clone().unwrap_or_default();
            if let Some(node) = node_mut(&mut self.nodes, &path)
                && let KeywordKind::User { active } = &mut node.keyword.kind
            {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.checkbox(active, "Aktiv").changed() {
                        self.preview_dirty = true;
                    }
                });
            }
        });
        let path = self.selected.clone().unwrap_or_default();
        let width = ui.available_width();
        egui::ScrollArea::vertical()
            .id_salt("keyword text")
            .max_height(editor_height)
            .min_scrolled_height(editor_height)
            .show(ui, |ui| match node_mut(&mut self.nodes, &path) {
                Some(node) if node.is_user() => {
                    let edit = egui::TextEdit::multiline(&mut node.keyword.text)
                        .code_editor()
                        .desired_width(width)
                        .min_size(egui::vec2(width, editor_height));
                    if ui.add(edit).changed() {
                        self.preview_dirty = true;
                    }
                }
                selected => {
                    let has_selection = selected.is_some();
                    let mut text = match selected {
                        Some(node) => node.preview(self.hide_data).join("\n"),
                        None => String::new(),
                    };
                    let hint = if has_selection {
                        "Erzeugte Keywords werden aus dem Modell geschrieben und sind hier nicht änderbar."
                    } else {
                        "Einen Eintrag im Baum wählen. Eigene Keywords mit \"Hinzufügen\" einfügen."
                    };
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .code_editor()
                            .interactive(false)
                            .hint_text(hint)
                            .desired_width(width)
                            .min_size(egui::vec2(width, editor_height)),
                    );
                }
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.strong("Eingabedatei (schreibgeschützt)");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .checkbox(&mut self.hide_data, "Daten ausblenden (schneller)")
                    .changed()
                {
                    self.preview_dirty = true;
                    self.scroll_to_selection = true;
                }
            });
        });
        if self.preview_dirty {
            self.update_preview();
        }
        self.preview_ui(ui);
    }

    /// The whole input file, drawn line by line so that large meshes stay fast.
    fn preview_ui(&mut self, ui: &mut egui::Ui) {
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let row_height = ui.fonts_mut(|f| f.row_height(&font));
        let mut area = egui::ScrollArea::both()
            .id_salt("inp preview")
            .auto_shrink(false);
        if std::mem::take(&mut self.scroll_to_selection)
            && let Some(selection) = &self.preview_selection
        {
            let offset = selection.start.saturating_sub(3) as f32 * row_height;
            area = area.vertical_scroll_offset(offset);
        }
        frame().show(ui, |ui| {
            // show_rows takes the row pitch from this spacing.
            ui.spacing_mut().item_spacing.y = 0.0;
            area.show_rows(ui, row_height, self.preview.len(), |ui, rows| {
                for row in rows {
                    let line = &self.preview[row];
                    let selected = self
                        .preview_selection
                        .as_ref()
                        .is_some_and(|s| s.contains(&row));
                    let mut text = egui::RichText::new(line).font(font.clone());
                    if selected {
                        text = text.background_color(SELECTED_LINE);
                    }
                    ui.add(egui::Label::new(text).extend());
                }
            });
        });
    }
}

const SELECTED_LINE: egui::Color32 = egui::Color32::from_rgb(255, 236, 160);
/// Colour of the user's keywords in the tree, like PrePoMax's added-keyword icon.
const USER_KEYWORD: egui::Color32 = egui::Color32::from_rgb(0, 110, 0);

pub(crate) fn frame() -> egui::Frame {
    egui::Frame::new()
        .fill(crate::style::WINDOW)
        .stroke(egui::Stroke::new(1.0, crate::style::BORDER))
        .inner_margin(2)
}

fn tree_ui(
    ui: &mut egui::Ui,
    nodes: &mut [Node],
    path: &mut Vec<usize>,
    selected: Option<&[usize]>,
    clicked: &mut Option<Vec<usize>>,
) {
    for (index, node) in nodes.iter_mut().enumerate() {
        path.push(index);
        let mut label = egui::RichText::new(node.label());
        let is_selected = selected == Some(path.as_slice());
        if node.is_user() {
            label = label.strong();
            if !is_selected {
                label = label.color(USER_KEYWORD);
            }
            if let KeywordKind::User { active: false } = node.keyword.kind {
                label = label.strikethrough();
            }
        }
        let has_children = !node.children.is_empty();
        let expanded = has_children.then_some(&mut node.expanded);
        if tree_row(
            ui,
            path.len(),
            expanded,
            selected == Some(path.as_slice()),
            label,
        )
        .clicked()
        {
            *clicked = Some(path.clone());
        }
        if has_children && node.expanded {
            tree_ui(ui, &mut node.children, path, selected, clicked);
        }
        path.pop();
    }
}

/// One row of the tree with its expand arrow; returns whether the label was clicked.
/// Row of a tree drawn from data: indentation, expand button for nodes with children and a
/// selectable label, whose response is returned.
pub(crate) fn tree_row(
    ui: &mut egui::Ui,
    depth: usize,
    expanded: Option<&mut bool>,
    selected: bool,
    label: egui::RichText,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add_space(depth as f32 * 14.0);
        let size = egui::vec2(12.0, ui.spacing().interact_size.y);
        match expanded {
            Some(expanded) => {
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                if response.clicked() {
                    *expanded = !*expanded;
                }
                let icon = egui::Rect::from_center_size(rect.center(), egui::vec2(9.0, 9.0));
                let response = response.with_new_rect(icon);
                egui::collapsing_header::paint_default_icon(
                    ui,
                    if *expanded { 1.0 } else { 0.0 },
                    &response,
                );
            }
            None => {
                ui.add_space(size.x);
            }
        }
        let response =
            ui.add(egui::Button::selectable(selected, label).wrap_mode(egui::TextWrapMode::Extend));
        if selected && response.gained_focus() {
            response.scroll_to_me(None);
        }
        response
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> KeywordEditor {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/kragbalken_c3d8.inp");
        let mesh = plx_io::inp::read_inp(&path).unwrap().mesh;
        let mut model = FeModel::default();
        model.materials.push(plx_model::Material {
            name: "Steel".into(),
            density: Some(7.85e-9),
            elastic: None,
            ..Default::default()
        });
        model.steps.push(plx_model::Step::new_static("Step-1"));
        KeywordEditor::new(&mesh, &model, "Test").unwrap()
    }

    fn title(editor: &KeywordEditor, name: &str) -> usize {
        editor
            .nodes
            .iter()
            .position(|n| n.keyword.kind == KeywordKind::Title(name.into()))
            .unwrap()
    }

    #[test]
    fn added_keywords_come_back_with_their_place() {
        let mut editor = editor();
        let amplitudes = title(&editor, "Amplitudes");
        editor.select(vec![amplitudes]);
        editor.add();
        assert_eq!(editor.selected, Some(vec![amplitudes, 0]));
        // A second one goes after the selected user keyword.
        editor.add();
        assert_eq!(editor.selected, Some(vec![amplitudes, 1]));
        editor.delete();
        assert_eq!(editor.selected, Some(vec![amplitudes, 0]));
        let keywords = editor.user_keywords();
        assert_eq!(
            keywords,
            [UserKeyword {
                position: vec![amplitudes, 0],
                text: NEW_KEYWORD.into(),
                active: true,
            }]
        );
        editor.update_preview();
        let selection = editor.preview_selection.clone().unwrap();
        assert_eq!(editor.preview[selection], [NEW_KEYWORD]);
    }

    #[test]
    fn moving_steps_into_and_out_of_neighbours() {
        let mut editor = editor();
        let materials = title(&editor, "Materials");
        editor.select(vec![materials]);
        editor.add();
        assert_eq!(editor.selected, Some(vec![materials, 1]));
        // Up enters the material, which has a *Density child, behind its children.
        editor.move_selected(true);
        assert_eq!(editor.selected, Some(vec![materials, 0, 1]));
        editor.move_selected(true);
        assert_eq!(editor.selected, Some(vec![materials, 0, 0]));
        // At the first place it leaves the material, then the Materials title.
        editor.move_selected(true);
        assert_eq!(editor.selected, Some(vec![materials, 0]));
        editor.move_selected(true);
        assert_eq!(editor.selected, Some(vec![materials]));
        // The title before Materials is empty, so the keyword steps over it.
        editor.move_selected(true);
        assert_eq!(editor.selected, Some(vec![materials - 1]));
        editor.move_selected(false);
        assert_eq!(editor.selected, Some(vec![materials]));
        // Down enters the Materials title before its first child.
        editor.move_selected(false);
        assert_eq!(editor.selected, Some(vec![materials, 0]));
        assert!(editor.selected_node().unwrap().is_user());
    }

    #[test]
    fn large_generated_keywords_are_hidden_in_the_preview() {
        let mut editor = editor();
        editor.update_preview();
        assert!(editor.preview.iter().any(|l| l == HIDDEN_DATA));
        assert!(editor.preview.iter().any(|l| l == "*End step"));
        let hidden = editor.preview.len();
        editor.hide_data = false;
        editor.update_preview();
        assert!(editor.preview.len() > hidden);
    }
}
