//! Which icons the two rails show, in what order, and where the divider
//! lines go. The settings window edits a [`RailLayout`]; the settings file
//! keeps it as two lists of icon ids under `[rails]`, `"-"` meaning a
//! divider and a hidden icon simply left out.
//!
//! The left rail is for going somewhere and the right one for doing
//! something here, so an icon only ever moves within its own rail.

/// A divider line in a rail's list.
pub const DIVIDER: &str = "-";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Every icon of the left rail, by id.
pub const LEFT_ICONS: &[&str] = &[
    "back",
    "forward",
    "parent",
    "favorites",
    "places",
    "recent_folders",
    "recent_files",
];

/// Every icon of the right rail, by id.
pub const RIGHT_ICONS: &[&str] = &[
    "new_folder",
    "find",
    "favorite",
    "copy_path",
    "file_manager",
    "terminal",
];

const LEFT_DEFAULT: &[&str] = &[
    "back",
    "forward",
    "parent",
    DIVIDER,
    "favorites",
    "places",
    "recent_folders",
    "recent_files",
];

const RIGHT_DEFAULT: &[&str] = &[
    "new_folder",
    "find",
    DIVIDER,
    "favorite",
    "copy_path",
    DIVIDER,
    "file_manager",
    "terminal",
];

impl Side {
    pub fn icons(self) -> &'static [&'static str] {
        match self {
            Side::Left => LEFT_ICONS,
            Side::Right => RIGHT_ICONS,
        }
    }

    fn default_ids(self) -> &'static [&'static str] {
        match self {
            Side::Left => LEFT_DEFAULT,
            Side::Right => RIGHT_DEFAULT,
        }
    }

    fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    /// The key of its list in the `[rails]` table.
    pub fn key(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// An icon, by its id from [`LEFT_ICONS`] or [`RIGHT_ICONS`].
    Icon(&'static str),
    Divider,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailLayout {
    left: Vec<Item>,
    right: Vec<Item>,
}

fn defaults(side: Side) -> Vec<Item> {
    side.default_ids()
        .iter()
        .map(|id| {
            if *id == DIVIDER {
                Item::Divider
            } else {
                Item::Icon(id)
            }
        })
        .collect()
}

impl Default for RailLayout {
    fn default() -> Self {
        RailLayout {
            left: defaults(Side::Left),
            right: defaults(Side::Right),
        }
    }
}

/// One rail's list as written in the settings file, read leniently: an id
/// that is not one of this rail's icons, or one listed twice, is reported
/// and skipped, and a list left with no icon at all gives the default.
fn resolve(side: Side, list: &[String], warnings: &mut Vec<String>) -> Vec<Item> {
    let key = format!("rails.{}", side.key());
    let mut items = Vec::new();
    let mut seen: Vec<&'static str> = Vec::new();
    for raw in list {
        let id = raw.trim();
        if id == DIVIDER {
            items.push(Item::Divider);
        } else if let Some(&known) = side.icons().iter().find(|k| **k == id) {
            if seen.contains(&known) {
                warnings.push(format!(
                    "setting `{key}`: {id:?} is listed twice; the second was ignored"
                ));
            } else {
                seen.push(known);
                items.push(Item::Icon(known));
            }
        } else if side.other().icons().contains(&id) {
            warnings.push(format!(
                "setting `{key}`: {id:?} belongs on the {} rail and was ignored",
                side.other().key()
            ));
        } else {
            warnings.push(format!(
                "setting `{key}`: {id:?} is not a rail icon and was ignored"
            ));
        }
    }
    if seen.is_empty() {
        warnings.push(format!(
            "setting `{key}` ignored: it names no icon, so the default is used"
        ));
        return defaults(side);
    }
    items
}

impl RailLayout {
    /// The layout from the two lists in the settings file; a missing list
    /// means that rail's default. Problems are added to `warnings`.
    pub fn from_lists(
        left: Option<&[String]>,
        right: Option<&[String]>,
        warnings: &mut Vec<String>,
    ) -> RailLayout {
        RailLayout {
            left: left.map_or_else(
                || defaults(Side::Left),
                |l| resolve(Side::Left, l, warnings),
            ),
            right: right.map_or_else(
                || defaults(Side::Right),
                |r| resolve(Side::Right, r, warnings),
            ),
        }
    }

    pub fn items(&self, side: Side) -> &[Item] {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    fn items_mut(&mut self, side: Side) -> &mut Vec<Item> {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    /// The list as the settings file keeps it.
    pub fn ids(&self, side: Side) -> Vec<String> {
        self.items(side)
            .iter()
            .map(|item| match item {
                Item::Icon(id) => (*id).to_owned(),
                Item::Divider => DIVIDER.to_owned(),
            })
            .collect()
    }

    /// The rail's icons that are not shown, in their default order.
    pub fn hidden(&self, side: Side) -> Vec<&'static str> {
        let shown = self.items(side);
        side.icons()
            .iter()
            .copied()
            .filter(|id| !shown.contains(&Item::Icon(id)))
            .collect()
    }

    fn icon_count(&self, side: Side) -> usize {
        self.items(side)
            .iter()
            .filter(|i| matches!(i, Item::Icon(_)))
            .count()
    }

    pub fn can_move_up(&self, _side: Side, index: usize) -> bool {
        index > 0
    }

    pub fn can_move_down(&self, side: Side, index: usize) -> bool {
        index + 1 < self.items(side).len()
    }

    /// Swaps the entry at `index` with the one above it.
    pub fn move_up(&mut self, side: Side, index: usize) -> bool {
        if !self.can_move_up(side, index) || index >= self.items(side).len() {
            return false;
        }
        self.items_mut(side).swap(index - 1, index);
        true
    }

    /// Swaps the entry at `index` with the one below it.
    pub fn move_down(&mut self, side: Side, index: usize) -> bool {
        if !self.can_move_down(side, index) {
            return false;
        }
        self.items_mut(side).swap(index, index + 1);
        true
    }

    /// Whether the icon at `index` may be hidden: a rail keeps at least one
    /// icon, since a rail with none would be read back as the default.
    pub fn can_hide(&self, side: Side, index: usize) -> bool {
        matches!(self.items(side).get(index), Some(Item::Icon(_))) && self.icon_count(side) > 1
    }

    pub fn hide(&mut self, side: Side, index: usize) -> bool {
        if !self.can_hide(side, index) {
            return false;
        }
        self.items_mut(side).remove(index);
        true
    }

    /// Shows a hidden icon again, at the end of its rail.
    pub fn show(&mut self, side: Side, id: &str) -> bool {
        let Some(&known) = side.icons().iter().find(|k| **k == id) else {
            return false;
        };
        if self.items(side).contains(&Item::Icon(known)) {
            return false;
        }
        self.items_mut(side).push(Item::Icon(known));
        true
    }

    /// Puts a divider line just below the entry at `index`.
    pub fn add_divider_after(&mut self, side: Side, index: usize) -> bool {
        if index >= self.items(side).len() {
            return false;
        }
        self.items_mut(side).insert(index + 1, Item::Divider);
        true
    }

    pub fn remove_divider(&mut self, side: Side, index: usize) -> bool {
        if self.items(side).get(index) != Some(&Item::Divider) {
            return false;
        }
        self.items_mut(side).remove(index);
        true
    }

    pub fn is_default(&self) -> bool {
        *self == RailLayout::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| (*s).to_owned()).collect()
    }

    fn read(left: &[&str], right: &[&str]) -> (RailLayout, Vec<String>) {
        let mut warnings = Vec::new();
        let layout = RailLayout::from_lists(Some(&list(left)), Some(&list(right)), &mut warnings);
        (layout, warnings)
    }

    #[test]
    fn missing_lists_give_the_default_rails_with_their_dividers() {
        let mut warnings = Vec::new();
        let layout = RailLayout::from_lists(None, None, &mut warnings);
        assert!(warnings.is_empty());
        assert!(layout.is_default());
        assert_eq!(layout.ids(Side::Left), list(LEFT_DEFAULT));
        assert_eq!(layout.ids(Side::Right), list(RIGHT_DEFAULT));
    }

    #[test]
    fn a_list_is_read_in_its_order_with_dividers_and_left_out_icons_are_hidden() {
        let (layout, warnings) = read(&["places", "-", "back"], &["terminal", "find"]);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            layout.items(Side::Left),
            [Item::Icon("places"), Item::Divider, Item::Icon("back")]
        );
        assert_eq!(
            layout.hidden(Side::Left),
            [
                "forward",
                "parent",
                "favorites",
                "recent_folders",
                "recent_files"
            ]
        );
        assert_eq!(layout.ids(Side::Right), list(&["terminal", "find"]));
    }

    #[test]
    fn unknown_doubled_and_wrong_rail_ids_are_reported_and_skipped() {
        let (layout, warnings) = read(&["back", "nonsense", "back", "find"], &["copy_path"]);
        assert_eq!(layout.items(Side::Left), [Item::Icon("back")]);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].contains("not a rail icon"));
        assert!(warnings[1].contains("listed twice"));
        assert!(warnings[2].contains("belongs on the right rail"));
    }

    #[test]
    fn a_list_with_no_icon_falls_back_to_the_default_with_a_warning() {
        let (layout, warnings) = read(&[], &["-", "junk"]);
        assert_eq!(layout.ids(Side::Left), list(LEFT_DEFAULT));
        assert_eq!(layout.ids(Side::Right), list(RIGHT_DEFAULT));
        assert!(warnings.iter().any(|w| w.contains("rails.left")));
        assert!(warnings.iter().any(|w| w.contains("rails.right")));
    }

    #[test]
    fn moving_swaps_neighbours_and_stops_at_the_ends() {
        let mut layout = RailLayout::default();
        assert!(!layout.move_up(Side::Left, 0));
        assert!(layout.move_down(Side::Left, 0));
        assert_eq!(layout.ids(Side::Left)[..2], list(&["forward", "back"])[..]);
        assert!(layout.move_up(Side::Left, 1));
        assert_eq!(layout.ids(Side::Left), list(LEFT_DEFAULT));
        let last = layout.items(Side::Left).len() - 1;
        assert!(!layout.move_down(Side::Left, last));
        assert!(!layout.move_up(Side::Left, last + 5));
    }

    #[test]
    fn dividers_are_added_below_a_row_and_only_dividers_are_removed() {
        let mut layout = RailLayout::default();
        assert!(layout.add_divider_after(Side::Right, 0));
        assert_eq!(layout.items(Side::Right)[1], Item::Divider);
        assert!(!layout.remove_divider(Side::Right, 0));
        assert!(layout.remove_divider(Side::Right, 1));
        assert!(layout.is_default());
        assert!(!layout.add_divider_after(Side::Right, 99));
    }

    #[test]
    fn hiding_and_showing_an_icon_and_the_last_icon_stays() {
        let mut layout = RailLayout::default();
        assert!(layout.hide(Side::Right, 0));
        assert_eq!(layout.hidden(Side::Right), ["new_folder"]);
        assert!(layout.show(Side::Right, "new_folder"));
        assert_eq!(layout.ids(Side::Right).last().unwrap(), "new_folder");
        assert!(!layout.show(Side::Right, "new_folder"));
        assert!(!layout.show(Side::Right, "back"));
        // A divider is not hidden, and the last icon cannot be.
        assert_eq!(layout.items(Side::Right)[1], Item::Divider);
        assert!(!layout.hide(Side::Right, 1));
        let (mut one, _) = read(&["back"], &["find"]);
        assert!(!one.can_hide(Side::Left, 0));
        assert!(!one.hide(Side::Left, 0));
    }
}
