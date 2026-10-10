//! Material library: materials sorted into categories, kept outside of any project so that
//! they can be copied into every model, like PrePoMax's `materials.lib`.

use serde::{Deserialize, Serialize};

use crate::{Elastic, Material, UnitSystem};

/// Version of the library file format written by this build.
pub const LIBRARY_FORMAT: u32 = 1;

/// Name of the root category, as in PrePoMax.
pub const ROOT_NAME: &str = "Materials";

/// Default name of a new category.
pub const NEW_CATEGORY: &str = "NewCategory";

/// Place of a node in the library: the index of each node on the way down from the root's
/// children. The empty path is the root category.
pub type LibraryPath = Vec<usize>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaterialLibrary {
    pub format: u32,
    /// The units the library's values are in; materials are converted when they are copied
    /// between the library and a model. Libraries saved before this was stored are in
    /// PrePoMax's default "mm, ton, s, °C".
    #[serde(default)]
    pub units: UnitSystem,
    pub root: Category,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    /// Whether the category is shown expanded; remembered like in PrePoMax.
    #[serde(default)]
    pub expanded: bool,
    pub items: Vec<LibraryNode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LibraryNode {
    Category(Category),
    Material(Material),
}

impl LibraryNode {
    pub fn name(&self) -> &str {
        match self {
            LibraryNode::Category(c) => &c.name,
            LibraryNode::Material(m) => &m.name,
        }
    }

    fn set_name(&mut self, name: String) {
        match self {
            LibraryNode::Category(c) => c.name = name,
            LibraryNode::Material(m) => m.name = name,
        }
    }
}

impl Category {
    fn new(name: &str, items: Vec<LibraryNode>) -> Self {
        Self {
            name: name.into(),
            expanded: true,
            items,
        }
    }

    fn contains(&self, name: &str) -> bool {
        self.items
            .iter()
            .any(|n| n.name().eq_ignore_ascii_case(name))
    }
}

impl Default for MaterialLibrary {
    /// The library a new installation starts with. Values in the unit system mm, t, s, as
    /// PrePoMax's default "mm, ton, s, °C".
    fn default() -> Self {
        let s235 = Material {
            name: "S235".into(),
            density: Some(7.85e-9),
            elastic: Some(Elastic {
                young: 210_000.0,
                poisson: 0.3,
            }),
            ..Material::default()
        };
        let category = |name, items| LibraryNode::Category(Category::new(name, items));
        Self {
            format: LIBRARY_FORMAT,
            units: UnitSystem::MmTonSC,
            root: Category::new(
                ROOT_NAME,
                vec![category(
                    "Elastic_Models",
                    vec![category(
                        "Steel",
                        vec![category("Structural", vec![LibraryNode::Material(s235)])],
                    )],
                )],
            ),
        }
    }
}

impl MaterialLibrary {
    /// The node at a path; `None` for the root and for paths that lead nowhere.
    pub fn node(&self, path: &[usize]) -> Option<&LibraryNode> {
        let (&last, parent) = path.split_last()?;
        self.category(parent)?.items.get(last)
    }

    fn node_mut(&mut self, path: &[usize]) -> Option<&mut LibraryNode> {
        let (&last, parent) = path.split_last()?;
        self.category_mut(parent)?.items.get_mut(last)
    }

    /// The category at a path, the root for the empty path.
    pub fn category(&self, path: &[usize]) -> Option<&Category> {
        let mut category = &self.root;
        for &index in path {
            match category.items.get(index)? {
                LibraryNode::Category(c) => category = c,
                LibraryNode::Material(_) => return None,
            }
        }
        Some(category)
    }

    pub fn category_mut(&mut self, path: &[usize]) -> Option<&mut Category> {
        let mut category = &mut self.root;
        for &index in path {
            match category.items.get_mut(index)? {
                LibraryNode::Category(c) => category = c,
                LibraryNode::Material(_) => return None,
            }
        }
        Some(category)
    }

    pub fn material(&self, path: &[usize]) -> Option<&Material> {
        match self.node(path)? {
            LibraryNode::Material(m) => Some(m),
            LibraryNode::Category(_) => None,
        }
    }

    /// The category a new node goes into when `path` is selected: the selected category, or
    /// the category of the selected material.
    fn target_category(&self, path: &[usize]) -> Option<LibraryPath> {
        if self.category(path).is_some() {
            Some(path.to_vec())
        } else {
            self.material(path)?;
            Some(path[..path.len() - 1].to_vec())
        }
    }

    /// Adds an empty category to the selected category (PrePoMax adds none below a
    /// material) and returns its path.
    pub fn add_category(&mut self, selected: &[usize]) -> Option<LibraryPath> {
        let category = self.category_mut(selected)?;
        let name = free_name(category, NEW_CATEGORY, "-");
        category.expanded = true;
        category
            .items
            .push(LibraryNode::Category(Category::new(&name, Vec::new())));
        let mut path = selected.to_vec();
        path.push(category.items.len() - 1);
        Some(path)
    }

    /// Copies a material of the model into the selected category, or next to the selected
    /// material. A taken name gets PrePoMax's suffix `_Model-n`. Returns the new path.
    pub fn add_material(&mut self, selected: &[usize], material: &Material) -> Option<LibraryPath> {
        let target = self.target_category(selected)?;
        let category = self.category_mut(&target)?;
        let mut material = material.clone();
        material.name = free_name(category, &material.name, "_Model-");
        category.expanded = true;
        category.items.push(LibraryNode::Material(material));
        let mut path = target;
        path.push(category.items.len() - 1);
        Some(path)
    }

    /// Deletes a node with everything in it. Returns the node to select afterwards: the one
    /// that took its place, the one before it, or the parent.
    pub fn delete(&mut self, path: &[usize]) -> Option<LibraryPath> {
        let (&last, parent) = path.split_last()?;
        let category = self.category_mut(parent)?;
        if last >= category.items.len() {
            return None;
        }
        category.items.remove(last);
        let mut next = parent.to_vec();
        if last < category.items.len() {
            next.push(last);
        } else if last > 0 {
            next.push(last - 1);
        }
        Some(next)
    }

    /// Renames a category or material; the name must be free among its siblings.
    pub fn rename(&mut self, path: &[usize], name: &str) -> Result<(), String> {
        let name = name.trim();
        let (&last, parent) = path
            .split_last()
            .ok_or("The root of the library cannot be renamed.")?;
        if name.is_empty() {
            return Err("Please enter a name.".into());
        }
        let category = self.category(parent).ok_or("Entry not found.")?;
        let taken = (category.items.iter().enumerate())
            .any(|(i, n)| i != last && n.name().eq_ignore_ascii_case(name));
        if taken {
            return Err(format!(
                "{} already contains an entry named {name}.",
                category.name
            ));
        }
        self.node_mut(path)
            .ok_or("Entry not found.")?
            .set_name(name.into());
        Ok(())
    }

    /// Index a material would move to, one place up or down among its siblings. Categories
    /// keep their place, as in PrePoMax.
    pub fn move_target(&self, path: &[usize], up: bool) -> Option<usize> {
        self.material(path)?;
        let (&last, parent) = path.split_last()?;
        let count = self.category(parent)?.items.len();
        if up {
            last.checked_sub(1)
        } else {
            Some(last + 1).filter(|&t| t < count)
        }
    }

    /// Moves a material one place up or down and returns its new path.
    pub fn move_material(&mut self, path: &[usize], up: bool) -> Option<LibraryPath> {
        let target = self.move_target(path, up)?;
        let (&last, parent) = path.split_last()?;
        self.category_mut(parent)?.items.swap(last, target);
        let mut moved = parent.to_vec();
        moved.push(target);
        Some(moved)
    }

    /// Path of the first material in tree order, which PrePoMax selects on opening.
    pub fn first_material(&self) -> Option<LibraryPath> {
        fn find(category: &Category, path: &mut LibraryPath) -> bool {
            for (i, node) in category.items.iter().enumerate() {
                path.push(i);
                match node {
                    LibraryNode::Material(_) => return true,
                    LibraryNode::Category(c) if find(c, path) => return true,
                    LibraryNode::Category(_) => {}
                }
                path.pop();
            }
            false
        }
        let mut path = Vec::new();
        find(&self.root, &mut path).then_some(path)
    }
}

/// `name` if no item of the category has it, else `name` with `separator` and the first
/// free number.
fn free_name(category: &Category, name: &str, separator: &str) -> String {
    if !category.contains(name) {
        return name.into();
    }
    (1..)
        .map(|n| format!("{name}{separator}{n}"))
        .find(|candidate| !category.contains(candidate))
        .expect("unbounded range")
}

/// Name for a material copied into the model: PrePoMax appends `_Library-n` when the name
/// is taken.
pub fn name_for_model<'a>(name: &str, existing: impl IntoIterator<Item = &'a str>) -> String {
    let existing: Vec<&str> = existing.into_iter().collect();
    let taken = |candidate: &str| existing.iter().any(|e| e.eq_ignore_ascii_case(candidate));
    if !taken(name) {
        return name.into();
    }
    (1..)
        .map(|n| format!("{name}_Library-{n}"))
        .find(|candidate| !taken(candidate))
        .expect("unbounded range")
}

#[cfg(test)]
mod tests {
    use super::*;

    const S235: [usize; 4] = [0, 0, 0, 0];

    #[test]
    fn default_library_holds_s235_in_prepomax_categories() {
        let library = MaterialLibrary::default();
        assert_eq!(library.first_material(), Some(S235.to_vec()));
        let s235 = library.material(&S235).unwrap();
        assert_eq!(s235.name, "S235");
        assert_eq!(s235.density, Some(7.85e-9));
        let names: Vec<&str> = (1..4)
            .map(|n| library.node(&S235[..n]).unwrap().name())
            .collect();
        assert_eq!(names, ["Elastic_Models", "Steel", "Structural"]);
    }

    #[test]
    fn categories_are_added_with_free_names() {
        let mut library = MaterialLibrary::default();
        assert_eq!(library.add_category(&[]), Some(vec![1]));
        assert_eq!(library.add_category(&[]), Some(vec![2]));
        assert_eq!(library.node(&[2]).unwrap().name(), "NewCategory-1");
        // Not below a material.
        assert_eq!(library.add_category(&S235), None);
    }

    #[test]
    fn model_materials_go_next_to_the_selected_material() {
        let mut library = MaterialLibrary::default();
        let s235 = library.material(&S235).unwrap().clone();
        let path = library.add_material(&S235, &s235).unwrap();
        assert_eq!(path, [0, 0, 0, 1]);
        assert_eq!(library.material(&path).unwrap().name, "S235_Model-1");
        let path = library.add_material(&[], &s235).unwrap();
        assert_eq!(path, [1]);
        assert_eq!(library.material(&path).unwrap().name, "S235");
    }

    #[test]
    fn rename_rejects_names_of_siblings() {
        let mut library = MaterialLibrary::default();
        let new = library.add_category(&[0, 0, 0]).unwrap();
        assert!(library.rename(&new, "s235").is_err());
        assert!(library.rename(&new, " ").is_err());
        assert!(library.rename(&[], "Root").is_err());
        library.rename(&new, "Custom").unwrap();
        assert_eq!(library.node(&new).unwrap().name(), "Custom");
    }

    #[test]
    fn delete_selects_the_neighbour_or_the_parent() {
        let mut library = MaterialLibrary::default();
        let s235 = library.material(&S235).unwrap().clone();
        library.add_material(&S235, &s235);
        assert_eq!(library.delete(&[0, 0, 0, 1]), Some(vec![0, 0, 0, 0]));
        assert_eq!(library.delete(&S235), Some(vec![0, 0, 0]));
        assert_eq!(library.first_material(), None);
        assert_eq!(library.delete(&[]), None);
    }

    #[test]
    fn only_materials_move() {
        let mut library = MaterialLibrary::default();
        let s235 = library.material(&S235).unwrap().clone();
        library.add_material(&S235, &s235);
        assert_eq!(library.move_material(&S235, true), None);
        assert_eq!(library.move_material(&S235, false), Some(vec![0, 0, 0, 1]));
        assert_eq!(library.material(&S235).unwrap().name, "S235_Model-1");
        assert_eq!(library.move_material(&[0, 0, 0, 1], false), None);
        assert_eq!(library.move_material(&[0], false), None);
    }

    #[test]
    fn copies_into_the_model_get_free_names() {
        assert_eq!(name_for_model("S235", ["Steel"]), "S235");
        assert_eq!(
            name_for_model("S235", ["s235", "S235_Library-1"]),
            "S235_Library-2"
        );
    }
}
