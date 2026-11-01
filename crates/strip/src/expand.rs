//! Which folder rows are opened to show their subfolders, and what was
//! read for each. The reading itself happens elsewhere, off the UI thread;
//! this only records the state.

use std::collections::HashMap;
use std::path::PathBuf;

/// The lists a folder row can appear in. The same folder can be open in one
/// list and closed in another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Favorites,
    Places,
    RecentFolders,
    RecentFiles,
    Ranked,
    Found,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RowKey {
    pub section: Section,
    pub path: PathBuf,
}

impl RowKey {
    pub fn new(section: Section, path: impl Into<PathBuf>) -> RowKey {
        RowKey {
            section,
            path: path.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Loading,
    Loaded(Vec<PathBuf>),
    Failed(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Toggle {
    /// The row opened; its subfolders must be read now.
    Load,
    Closed,
}

#[derive(Debug, Default)]
pub struct Expansion {
    nodes: HashMap<RowKey, Node>,
}

impl Expansion {
    /// Opens a closed row (as loading) or closes an open one together with
    /// everything opened below it, so reopening reads fresh.
    pub fn toggle(&mut self, key: RowKey) -> Toggle {
        if self.nodes.contains_key(&key) {
            self.nodes
                .retain(|k, _| !(k.section == key.section && k.path.starts_with(&key.path)));
            return Toggle::Closed;
        }
        self.nodes.insert(key, Node::Loading);
        Toggle::Load
    }

    /// Stores what was read. An answer for a row closed in the meantime is
    /// dropped. Returns whether it was kept.
    pub fn finish(&mut self, key: &RowKey, result: Result<Vec<PathBuf>, String>) -> bool {
        match self.nodes.get_mut(key) {
            Some(node @ Node::Loading) => {
                *node = match result {
                    Ok(folders) => Node::Loaded(folders),
                    Err(why) => Node::Failed(why),
                };
                true
            }
            _ => false,
        }
    }

    pub fn node(&self, key: &RowKey) -> Option<&Node> {
        self.nodes.get(key)
    }

    pub fn is_open(&self, key: &RowKey) -> bool {
        self.nodes.contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(path: &str) -> RowKey {
        RowKey::new(Section::Favorites, path)
    }

    #[test]
    fn opening_a_row_asks_for_a_read_and_shows_loading_until_it_answers() {
        let mut e = Expansion::default();
        assert_eq!(e.toggle(key("/a")), Toggle::Load);
        assert_eq!(e.node(&key("/a")), Some(&Node::Loading));
        assert!(e.finish(&key("/a"), Ok(vec!["/a/b".into()])));
        assert_eq!(e.node(&key("/a")), Some(&Node::Loaded(vec!["/a/b".into()])));
    }

    #[test]
    fn a_read_error_is_kept_as_text_to_show() {
        let mut e = Expansion::default();
        e.toggle(key("/a"));
        e.finish(&key("/a"), Err("cannot read /a: denied".into()));
        assert_eq!(
            e.node(&key("/a")),
            Some(&Node::Failed("cannot read /a: denied".into()))
        );
    }

    #[test]
    fn closing_a_row_closes_the_rows_opened_below_it() {
        let mut e = Expansion::default();
        e.toggle(key("/a"));
        e.finish(&key("/a"), Ok(vec!["/a/b".into()]));
        e.toggle(key("/a/b"));
        e.toggle(key("/other"));
        assert_eq!(e.toggle(key("/a")), Toggle::Closed);
        assert!(!e.is_open(&key("/a/b")));
        assert!(e.is_open(&key("/other")));
    }

    #[test]
    fn a_late_answer_for_a_closed_row_is_dropped() {
        let mut e = Expansion::default();
        e.toggle(key("/a"));
        e.toggle(key("/a"));
        assert!(!e.finish(&key("/a"), Ok(vec![])));
        assert!(!e.is_open(&key("/a")));
    }

    #[test]
    fn the_same_folder_can_be_open_in_one_list_and_closed_in_another() {
        let mut e = Expansion::default();
        e.toggle(RowKey::new(Section::Places, "/a"));
        assert!(!e.is_open(&RowKey::new(Section::Favorites, "/a")));
    }

    #[test]
    fn a_sibling_with_a_shared_name_prefix_stays_open_when_another_closes() {
        let mut e = Expansion::default();
        e.toggle(key("/a"));
        e.toggle(key("/ab"));
        e.toggle(key("/a"));
        assert!(e.is_open(&key("/ab")));
    }
}
