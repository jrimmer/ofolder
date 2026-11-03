//! The rail icons, built into the binary so nothing has to be installed
//! beside it. They are drawn as masks in the theme's text colour.

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

/// Every icon file, by the path gpui asks for.
const ALL: &[&str] = &[
    "icons/back.svg",
    "icons/chevron-down.svg",
    "icons/chevron-right.svg",
    "icons/copy.svg",
    "icons/dots.svg",
    "icons/file.svg",
    "icons/find.svg",
    "icons/folder-open.svg",
    "icons/folder-plus.svg",
    "icons/forward.svg",
    "icons/info.svg",
    "icons/parent.svg",
    "icons/places.svg",
    "icons/recent.svg",
    "icons/star.svg",
    "icons/star-filled.svg",
    "icons/star-plus.svg",
    "icons/terminal.svg",
    "icons/warning.svg",
];

fn bytes(path: &str) -> Option<&'static [u8]> {
    Some(match path {
        "icons/back.svg" => include_bytes!("../assets/icons/back.svg"),
        "icons/chevron-down.svg" => include_bytes!("../assets/icons/chevron-down.svg"),
        "icons/chevron-right.svg" => include_bytes!("../assets/icons/chevron-right.svg"),
        "icons/copy.svg" => include_bytes!("../assets/icons/copy.svg"),
        "icons/dots.svg" => include_bytes!("../assets/icons/dots.svg"),
        "icons/file.svg" => include_bytes!("../assets/icons/file.svg"),
        "icons/find.svg" => include_bytes!("../assets/icons/find.svg"),
        "icons/folder-open.svg" => include_bytes!("../assets/icons/folder-open.svg"),
        "icons/folder-plus.svg" => include_bytes!("../assets/icons/folder-plus.svg"),
        "icons/forward.svg" => include_bytes!("../assets/icons/forward.svg"),
        "icons/info.svg" => include_bytes!("../assets/icons/info.svg"),
        "icons/parent.svg" => include_bytes!("../assets/icons/parent.svg"),
        "icons/places.svg" => include_bytes!("../assets/icons/places.svg"),
        "icons/recent.svg" => include_bytes!("../assets/icons/recent.svg"),
        "icons/star.svg" => include_bytes!("../assets/icons/star.svg"),
        "icons/star-filled.svg" => include_bytes!("../assets/icons/star-filled.svg"),
        "icons/star-plus.svg" => include_bytes!("../assets/icons/star-plus.svg"),
        "icons/terminal.svg" => include_bytes!("../assets/icons/terminal.svg"),
        "icons/warning.svg" => include_bytes!("../assets/icons/warning.svg"),
        _ => return None,
    })
}

pub struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(bytes(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ALL
            .iter()
            .filter(|p| p.starts_with(path))
            .map(|p| SharedString::from(*p))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rail::{Entry, RailSide, entries};

    #[test]
    fn every_rail_icon_is_built_in() {
        for side in [RailSide::Left, RailSide::Right] {
            for e in entries(side) {
                if let Entry::Button(b) = e {
                    assert!(bytes(b.icon()).is_some(), "{b:?}");
                }
            }
        }
        assert!(bytes("icons/star-filled.svg").is_some());
    }

    #[test]
    fn every_built_in_icon_is_listed_and_an_unknown_one_is_not_found() {
        for p in ALL {
            assert!(bytes(p).is_some(), "{p}");
        }
        assert!(Icons.load("icons/none.svg").unwrap().is_none());
        assert_eq!(Icons.list("icons/").unwrap().len(), ALL.len());
    }
}
