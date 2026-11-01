//! The small set of actions behind a folder row's "..." button.

use crate::expand::{RowKey, Section};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    Go,
    CopyPath,
    OpenFileManager,
    OpenTerminal,
    ToggleFavorite,
    MoveUp,
    MoveDown,
    Rename,
}

impl RowAction {
    /// Whether the menu stays open after the action, so a favorite can be
    /// moved several places in a row.
    pub fn keeps_menu_open(self) -> bool {
        matches!(self, RowAction::MoveUp | RowAction::MoveDown)
    }
}

/// The buttons of a row's menu. Only favorites can be moved and renamed.
pub fn actions_for(section: Section, is_favorite: bool) -> Vec<(RowAction, &'static str)> {
    let mut items = vec![
        (RowAction::Go, "Go"),
        (RowAction::CopyPath, "Copy path"),
        (RowAction::OpenFileManager, "File manager"),
        (RowAction::OpenTerminal, "Terminal"),
        (
            RowAction::ToggleFavorite,
            if is_favorite {
                "Remove favorite"
            } else {
                "Add favorite"
            },
        ),
    ];
    if section == Section::Favorites {
        items.push((RowAction::MoveUp, "Up"));
        items.push((RowAction::MoveDown, "Down"));
        items.push((RowAction::Rename, "Rename"));
    }
    items
}

/// Which row's menu is open; at most one.
#[derive(Debug, Default)]
pub struct RowMenu {
    open: Option<RowKey>,
}

impl RowMenu {
    pub fn toggle(&mut self, key: RowKey) {
        self.open = if self.open.as_ref() == Some(&key) {
            None
        } else {
            Some(key)
        };
    }

    pub fn close(&mut self) {
        self.open = None;
    }

    pub fn is_open(&self, key: &RowKey) -> bool {
        self.open.as_ref() == Some(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_folder_row_offers_go_copy_launchers_and_the_favorite_toggle() {
        let items = actions_for(Section::Places, false);
        let actions: Vec<_> = items.iter().map(|i| i.0).collect();
        assert_eq!(
            actions,
            [
                RowAction::Go,
                RowAction::CopyPath,
                RowAction::OpenFileManager,
                RowAction::OpenTerminal,
                RowAction::ToggleFavorite
            ]
        );
    }

    #[test]
    fn the_favorite_toggle_is_worded_for_what_it_will_do() {
        assert_eq!(actions_for(Section::Places, false)[4].1, "Add favorite");
        assert_eq!(actions_for(Section::Places, true)[4].1, "Remove favorite");
    }

    #[test]
    fn only_favorites_can_be_moved_and_renamed() {
        let favorites = actions_for(Section::Favorites, true);
        assert!(favorites.iter().any(|i| i.0 == RowAction::Rename));
        assert!(favorites.iter().any(|i| i.0 == RowAction::MoveUp));
        let places = actions_for(Section::Places, true);
        assert!(!places.iter().any(|i| i.0 == RowAction::Rename));
    }

    #[test]
    fn opening_one_menu_closes_the_other_and_toggling_closes_it() {
        let mut m = RowMenu::default();
        let a = RowKey::new(Section::Places, "/a");
        let b = RowKey::new(Section::Places, "/b");
        m.toggle(a.clone());
        m.toggle(b.clone());
        assert!(!m.is_open(&a) && m.is_open(&b));
        m.toggle(b.clone());
        assert!(!m.is_open(&b));
    }
}
