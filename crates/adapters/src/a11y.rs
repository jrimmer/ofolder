//! A small view of an accessibility tree.
//!
//! The GTK and Strata adapters read their dialogs through the accessibility
//! bus. They are written against these two traits, not against `atspi`, so
//! their logic can be tested on trees shaped like the real ones.

use std::time::Duration;

/// The kinds of node the adapters care about. Everything else is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Button,
    ToggleButton,
    Label,
    Text,
    List,
    ListItem,
    /// A file list laid out in columns, read row by row.
    Table,
    TableCell,
    ColumnHeader,
    FileChooser,
    Filler,
    Other,
}

pub trait Node: Clone {
    fn role(&self) -> Role;
    fn name(&self) -> String;
    fn checked(&self) -> bool;
    fn editable(&self) -> bool;
    /// Whether the node is a selected row or item of a list.
    fn selected(&self) -> bool;
    fn children(&self) -> Vec<Self>;
    fn parent(&self) -> Option<Self>;
    /// Performs the node's first action (a click, or an entry's "activate").
    fn press(&self) -> bool;
    fn text(&self) -> Option<String>;
    fn set_text(&self, text: &str) -> bool;
}

pub trait Tree {
    type N: Node;
    /// The applications on the accessibility bus, with their names.
    fn applications(&self) -> Vec<(String, Self::N)>;
}

/// What a visitor says about the node it was shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Walk {
    /// Look at its children too.
    Into,
    /// Leave its children alone: a file list can hold thousands of nodes.
    Over,
    /// Stop the whole walk.
    Stop,
}

/// Depth first, in document order. Returns `false` if the walk was stopped.
pub fn walk<N: Node>(root: &N, visit: &mut dyn FnMut(&N) -> Walk) -> bool {
    match visit(root) {
        Walk::Stop => return false,
        Walk::Over => return true,
        Walk::Into => {}
    }
    root.children().iter().all(|child| walk(child, visit))
}

/// The first node, in document order, that satisfies `matches`. File lists
/// are skipped over because the things the adapters look for are never in them.
pub fn find<N: Node>(root: &N, matches: impl Fn(&N) -> bool) -> Option<N> {
    let mut found = None;
    walk(root, &mut |node| {
        if matches(node) {
            found = Some(node.clone());
            Walk::Stop
        } else if is_file_list(node.role()) {
            Walk::Over
        } else {
            Walk::Into
        }
    });
    found
}

pub fn find_named<N: Node>(root: &N, role: Role, name: &str) -> Option<N> {
    find(root, |n| n.role() == role && n.name() == name)
}

/// Every label under `root`, in order, ignoring file lists.
pub fn labels<N: Node>(root: &N) -> Vec<String> {
    let mut out = Vec::new();
    walk(root, &mut |node| {
        if node.role() == Role::Label && !node.name().is_empty() {
            out.push(node.name());
        }
        if is_file_list(node.role()) {
            Walk::Over
        } else {
            Walk::Into
        }
    });
    out
}

/// Lists and tables hold the files, which can be thousands of nodes, and
/// none of the controls the adapters look for.
fn is_file_list(role: Role) -> bool {
    matches!(role, Role::List | Role::Table)
}

/// The wait between the two "activate" presses GTK needs after a move.
pub const ACTIVATE_GAP: Duration = Duration::from_millis(400);

#[cfg(test)]
pub(crate) mod fake {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{Node, Role, Tree};

    #[derive(Default)]
    pub struct Data {
        pub role: Vec<Role>,
        pub name: Vec<String>,
        pub checked: Vec<bool>,
        pub selected: Vec<bool>,
        pub editable: Vec<bool>,
        pub children: Vec<Vec<usize>>,
        pub parent: Vec<Option<usize>>,
        pub presses: Vec<u32>,
        pub text: Vec<Option<String>>,
    }

    #[derive(Clone)]
    pub struct FakeNode {
        pub id: usize,
        pub data: Rc<RefCell<Data>>,
    }

    impl FakeNode {
        pub fn root(role: Role, name: &str) -> Self {
            let node = Self {
                id: 0,
                data: Rc::new(RefCell::new(Data::default())),
            };
            node.push(role, name, None);
            node
        }

        fn push(&self, role: Role, name: &str, parent: Option<usize>) -> usize {
            let mut d = self.data.borrow_mut();
            d.role.push(role);
            d.name.push(name.to_owned());
            d.checked.push(false);
            d.selected.push(false);
            d.editable.push(false);
            d.children.push(Vec::new());
            d.parent.push(parent);
            d.presses.push(0);
            d.text.push(None);
            d.role.len() - 1
        }

        pub fn add(&self, role: Role, name: &str) -> FakeNode {
            let id = self.push(role, name, Some(self.id));
            self.data.borrow_mut().children[self.id].push(id);
            FakeNode {
                id,
                data: self.data.clone(),
            }
        }

        pub fn checked_on(self) -> Self {
            self.data.borrow_mut().checked[self.id] = true;
            self
        }

        pub fn selected_on(self) -> Self {
            self.data.borrow_mut().selected[self.id] = true;
            self
        }

        pub fn editable_with(self, text: &str) -> Self {
            let mut d = self.data.borrow_mut();
            d.editable[self.id] = true;
            d.text[self.id] = Some(text.to_owned());
            drop(d);
            self
        }

        pub fn presses(&self) -> u32 {
            self.data.borrow().presses[self.id]
        }
    }

    impl Node for FakeNode {
        fn role(&self) -> Role {
            self.data.borrow().role[self.id]
        }
        fn name(&self) -> String {
            self.data.borrow().name[self.id].clone()
        }
        fn checked(&self) -> bool {
            self.data.borrow().checked[self.id]
        }
        fn editable(&self) -> bool {
            self.data.borrow().editable[self.id]
        }
        fn selected(&self) -> bool {
            self.data.borrow().selected[self.id]
        }
        fn children(&self) -> Vec<Self> {
            self.data.borrow().children[self.id]
                .iter()
                .map(|&id| FakeNode {
                    id,
                    data: self.data.clone(),
                })
                .collect()
        }
        fn parent(&self) -> Option<Self> {
            self.data.borrow().parent[self.id].map(|id| FakeNode {
                id,
                data: self.data.clone(),
            })
        }
        fn press(&self) -> bool {
            self.data.borrow_mut().presses[self.id] += 1;
            true
        }
        fn text(&self) -> Option<String> {
            self.data.borrow().text[self.id].clone()
        }
        fn set_text(&self, text: &str) -> bool {
            self.data.borrow_mut().text[self.id] = Some(text.to_owned());
            true
        }
    }

    /// A bus holding the applications it was given.
    pub struct FakeTree(pub Vec<(String, FakeNode)>);

    impl Tree for FakeTree {
        type N = FakeNode;
        fn applications(&self) -> Vec<(String, FakeNode)> {
            self.0.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeNode;
    use super::*;

    fn sample() -> FakeNode {
        let root = FakeNode::root(Role::Other, "app");
        let list = root.add(Role::List, "files");
        list.add(Role::Button, "inside-list");
        root.add(Role::Button, "Cancel");
        root
    }

    #[test]
    fn find_returns_the_first_match_in_document_order() {
        let root = sample();
        assert_eq!(
            find_named(&root, Role::Button, "Cancel").map(|n| n.name()),
            Some("Cancel".to_owned())
        );
    }

    #[test]
    fn find_does_not_look_inside_file_lists() {
        assert!(find_named(&sample(), Role::Button, "inside-list").is_none());
    }

    #[test]
    fn find_does_not_look_inside_file_tables_either() {
        let root = FakeNode::root(Role::Other, "app");
        let table = root.add(Role::Table, "Files");
        table.add(Role::Button, "inside-table");
        assert!(find_named(&root, Role::Button, "inside-table").is_none());
    }

    #[test]
    fn labels_come_back_in_order_and_skip_empty_ones() {
        let root = FakeNode::root(Role::Other, "row");
        root.add(Role::Label, "~");
        root.add(Role::Label, "");
        root.add(Role::Label, "/");
        root.add(Role::Label, "Work");
        assert_eq!(labels(&root), ["~", "/", "Work"]);
    }
}
