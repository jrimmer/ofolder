//! The two rails: which icon sits where, which menu each opens, and how big
//! that menu should be. No window is involved, so all of it is tested here;
//! the drawing code places each icon at the position this module gives, so a
//! menu's anchor always matches the icon that opened it. The order, the
//! hidden icons and the divider lines come from the user's [`Layout`].

use ofolder_core::rails::{Item, RailLayout, Side};

/// The width of each rail, in logical pixels.
pub const RAIL_WIDTH: f32 = 52.0;
/// The side of an icon button.
pub const BUTTON: f32 = 36.0;
/// The space between two buttons.
const GAP: f32 = 4.0;
/// The space above the first button.
const TOP: f32 = 8.0;
/// The height a divider takes, line included.
pub const DIVIDER: f32 = 13.0;

/// Which rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailSide {
    Left,
    Right,
}

/// Every icon on the rails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    Back,
    Forward,
    Parent,
    Favorites,
    Places,
    RecentFolders,
    RecentFiles,
    NewFolder,
    Find,
    Favorite,
    CopyPath,
    FileManager,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Button(Button),
    Divider,
}

impl Button {
    /// Every icon, left rail first.
    pub const ALL: [Button; 13] = [
        Button::Back,
        Button::Forward,
        Button::Parent,
        Button::Favorites,
        Button::Places,
        Button::RecentFolders,
        Button::RecentFiles,
        Button::NewFolder,
        Button::Find,
        Button::Favorite,
        Button::CopyPath,
        Button::FileManager,
        Button::Terminal,
    ];

    /// Its id in the settings file's `[rails]` lists.
    pub fn key(self) -> &'static str {
        match self {
            Button::Back => "back",
            Button::Forward => "forward",
            Button::Parent => "parent",
            Button::Favorites => "favorites",
            Button::Places => "places",
            Button::RecentFolders => "recent_folders",
            Button::RecentFiles => "recent_files",
            Button::NewFolder => "new_folder",
            Button::Find => "find",
            Button::Favorite => "favorite",
            Button::CopyPath => "copy_path",
            Button::FileManager => "file_manager",
            Button::Terminal => "terminal",
        }
    }

    pub fn from_key(key: &str) -> Option<Button> {
        Button::ALL.into_iter().find(|b| b.key() == key)
    }

    /// Its name in the settings window.
    pub fn name(self) -> &'static str {
        match self {
            Button::Back => "Back",
            Button::Forward => "Forward",
            Button::Parent => "Parent folder",
            Button::Favorites => "Favorites",
            Button::Places => "Places and drives",
            Button::RecentFolders => "Recent folders",
            Button::RecentFiles => "Recent files",
            Button::NewFolder => "New folder",
            Button::Find => "Find a folder",
            Button::Favorite => "Add or remove favorite",
            Button::CopyPath => "Copy path",
            Button::FileManager => "Open in file manager",
            Button::Terminal => "Open a terminal",
        }
    }
}

/// The left rail, "go somewhere", top to bottom, as it comes.
pub const LEFT: &[Entry] = &[
    Entry::Button(Button::Back),
    Entry::Button(Button::Forward),
    Entry::Button(Button::Parent),
    Entry::Divider,
    Entry::Button(Button::Favorites),
    Entry::Button(Button::Places),
    Entry::Button(Button::RecentFolders),
    Entry::Button(Button::RecentFiles),
];

/// The right rail, "do something here", top to bottom, as it comes.
pub const RIGHT: &[Entry] = &[
    Entry::Button(Button::NewFolder),
    Entry::Button(Button::Find),
    Entry::Divider,
    Entry::Button(Button::Favorite),
    Entry::Button(Button::CopyPath),
    Entry::Divider,
    Entry::Button(Button::FileManager),
    Entry::Button(Button::Terminal),
];

/// The menus a rail icon opens. `Toast` is not opened by an icon: it shows a
/// message while no menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Favorites,
    Places,
    RecentFolders,
    RecentFiles,
    NewFolder,
    Find,
    Toast,
}

impl Button {
    pub fn side(self) -> RailSide {
        match self {
            Button::Back
            | Button::Forward
            | Button::Parent
            | Button::Favorites
            | Button::Places
            | Button::RecentFolders
            | Button::RecentFiles => RailSide::Left,
            _ => RailSide::Right,
        }
    }

    /// The menu this icon opens; None for an icon that acts at once.
    pub fn menu(self) -> Option<MenuKind> {
        match self {
            Button::Favorites => Some(MenuKind::Favorites),
            Button::Places => Some(MenuKind::Places),
            Button::RecentFolders => Some(MenuKind::RecentFolders),
            Button::RecentFiles => Some(MenuKind::RecentFiles),
            Button::NewFolder => Some(MenuKind::NewFolder),
            Button::Find => Some(MenuKind::Find),
            _ => None,
        }
    }

    /// The icon's file under the embedded assets.
    pub fn icon(self) -> &'static str {
        match self {
            Button::Back => "icons/back.svg",
            Button::Forward => "icons/forward.svg",
            Button::Parent => "icons/parent.svg",
            Button::Favorites => "icons/star.svg",
            Button::Places => "icons/places.svg",
            Button::RecentFolders => "icons/recent.svg",
            Button::RecentFiles => "icons/file.svg",
            Button::NewFolder => "icons/folder-plus.svg",
            Button::Find => "icons/find.svg",
            Button::Favorite => "icons/star-plus.svg",
            Button::CopyPath => "icons/copy.svg",
            Button::FileManager => "icons/folder-open.svg",
            Button::Terminal => "icons/terminal.svg",
        }
    }

    /// The hover text. The favorite toggle says what a click will do.
    pub fn tooltip(self, current_is_favorite: bool) -> &'static str {
        match self {
            Button::Back => "Back",
            Button::Forward => "Forward",
            Button::Parent => "Parent folder",
            Button::Favorites => "Favorites",
            Button::Places => "Places and drives",
            Button::RecentFolders => "Recent folders",
            Button::RecentFiles => "Recent files",
            Button::NewFolder => "New folder",
            Button::Find => "Find a folder",
            Button::Favorite if current_is_favorite => "Remove this folder from favorites",
            Button::Favorite => "Add this folder to favorites",
            Button::CopyPath => "Copy this folder's path",
            Button::FileManager => "Open this folder in the file manager",
            Button::Terminal => "Open a terminal in this folder",
        }
    }

    /// A stable element id for the icon.
    pub fn id(self) -> &'static str {
        match self {
            Button::Back => "rail-back",
            Button::Forward => "rail-forward",
            Button::Parent => "rail-parent",
            Button::Favorites => "rail-favorites",
            Button::Places => "rail-places",
            Button::RecentFolders => "rail-recent-folders",
            Button::RecentFiles => "rail-recent-files",
            Button::NewFolder => "rail-new-folder",
            Button::Find => "rail-find",
            Button::Favorite => "rail-favorite",
            Button::CopyPath => "rail-copy",
            Button::FileManager => "rail-file-manager",
            Button::Terminal => "rail-terminal",
        }
    }
}

impl MenuKind {
    pub fn side(self) -> RailSide {
        match self {
            MenuKind::Favorites
            | MenuKind::Places
            | MenuKind::RecentFolders
            | MenuKind::RecentFiles => RailSide::Left,
            MenuKind::NewFolder | MenuKind::Find | MenuKind::Toast => RailSide::Right,
        }
    }

    /// The icon that opens it.
    pub fn button(self) -> Option<Button> {
        match self {
            MenuKind::Favorites => Some(Button::Favorites),
            MenuKind::Places => Some(Button::Places),
            MenuKind::RecentFolders => Some(Button::RecentFolders),
            MenuKind::RecentFiles => Some(Button::RecentFiles),
            MenuKind::NewFolder => Some(Button::NewFolder),
            MenuKind::Find => Some(Button::Find),
            MenuKind::Toast => None,
        }
    }

    /// Menus that exist to be typed into take the keyboard when they open;
    /// the user has just clicked for one. Lists and the toast do not.
    pub fn takes_keyboard(self) -> bool {
        matches!(self, MenuKind::NewFolder | MenuKind::Find)
    }

    pub fn width(self) -> i32 {
        match self {
            MenuKind::Find => 320,
            _ => 300,
        }
    }

    /// The menu's height for `rows` lines of content, kept within limits
    /// so a long list scrolls instead of running off the monitor.
    pub fn height(self, rows: usize) -> i32 {
        // A folder row with its path under the name, as measured on screen.
        const ROW: i32 = 48;
        const MIN: i32 = 90;
        const MAX: i32 = 520;
        let fixed = match self {
            MenuKind::NewFolder => return 120,
            // A message: its rows are lines of wrapped text.
            MenuKind::Toast => {
                let lines = i32::try_from(rows.max(1)).unwrap_or(i32::MAX / TOAST_LINE);
                return (TOAST_PAD + lines.saturating_mul(TOAST_LINE)).clamp(64, 260);
            }
            // The search box, the search button and a status line.
            MenuKind::Find => 144,
            // The All apps / This app switch, and the line saying which app is believed to ask.
            MenuKind::RecentFolders | MenuKind::RecentFiles => 92,
            MenuKind::Favorites | MenuKind::Places => 44,
        };
        let rows = i32::try_from(rows).unwrap_or(i32::MAX / ROW);
        (fixed + rows.saturating_mul(ROW)).clamp(MIN, MAX)
    }
}

/// One line of a message, and the space around the lines, in pixels.
const TOAST_LINE: i32 = 17;
const TOAST_PAD: i32 = 30;
/// Characters of a message that fit one line of the message window.
const TOAST_CHARS: usize = 36;

/// The lines a message takes in the message window, wrapping at spaces,
/// so the window is tall enough to show all of it.
pub fn toast_lines(text: &str) -> usize {
    let mut lines = 1;
    let mut used = 0;
    for word in text.split_whitespace() {
        let len = word.chars().count();
        let needed = if used == 0 { len } else { used + 1 + len };
        if needed <= TOAST_CHARS {
            used = needed;
            continue;
        }
        if used > 0 {
            lines += 1;
        }
        // A word longer than a line breaks over several.
        lines += len.saturating_sub(1) / TOAST_CHARS;
        used = (len.saturating_sub(1) % TOAST_CHARS) + 1;
    }
    lines
}

/// Where an entry sits in its rail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub entry: Entry,
    /// From the rail's top, in logical pixels.
    pub y: f32,
    pub h: f32,
}

impl RailSide {
    pub fn core(self) -> Side {
        match self {
            RailSide::Left => Side::Left,
            RailSide::Right => Side::Right,
        }
    }
}

/// What each rail shows, top to bottom: the user's choice of icons, order
/// and divider lines.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    left: Vec<Entry>,
    right: Vec<Entry>,
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            left: LEFT.to_vec(),
            right: RIGHT.to_vec(),
        }
    }
}

fn entry(item: &Item) -> Option<Entry> {
    match item {
        Item::Divider => Some(Entry::Divider),
        Item::Icon(key) => Button::from_key(key).map(Entry::Button),
    }
}

impl Layout {
    pub fn from_core(layout: &RailLayout) -> Layout {
        let side = |s: Side| layout.items(s).iter().filter_map(entry).collect();
        Layout {
            left: side(Side::Left),
            right: side(Side::Right),
        }
    }

    pub fn entries(&self, side: RailSide) -> &[Entry] {
        match side {
            RailSide::Left => &self.left,
            RailSide::Right => &self.right,
        }
    }
}

/// Every entry of a rail with its position.
pub fn slots(entries: &[Entry]) -> Vec<Slot> {
    let mut y = TOP;
    let mut out = Vec::new();
    for &entry in entries {
        let h = match entry {
            Entry::Button(_) => BUTTON,
            Entry::Divider => DIVIDER,
        };
        out.push(Slot { entry, y, h });
        y += h + GAP;
    }
    out
}

/// The top of the icon, from the top of its rail: where its menu's top goes.
pub fn anchor_y(layout: &Layout, button: Button) -> i32 {
    slots(layout.entries(button.side()))
        .into_iter()
        .find(|s| s.entry == Entry::Button(button))
        .map(|s| s.y.round() as i32)
        .unwrap_or(0)
}

/// What a click on a menu icon does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Close the open menu, if any, and open this one.
    Open(MenuKind),
    /// Close the open menu.
    Close,
}

/// Which menu is open; at most one.
#[derive(Debug, Default)]
pub struct Menus {
    open: Option<MenuKind>,
}

impl Menus {
    pub fn open(&self) -> Option<MenuKind> {
        self.open
    }

    /// Clicking the icon of the open menu closes it; any other opens its own
    /// menu in place of whatever was open.
    pub fn click(&mut self, kind: MenuKind) -> Change {
        if self.open == Some(kind) {
            self.open = None;
            Change::Close
        } else {
            self.open = Some(kind);
            Change::Open(kind)
        }
    }

    /// Shows a message: only when nothing else is open, since an open menu
    /// shows the message itself.
    pub fn show_toast(&mut self) -> bool {
        if self.open.is_none() {
            self.open = Some(MenuKind::Toast);
            true
        } else {
            false
        }
    }

    pub fn close(&mut self) -> Option<MenuKind> {
        self.open.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(side: RailSide) -> Vec<Entry> {
        Layout::default().entries(side).to_vec()
    }

    fn buttons(side: RailSide) -> Vec<Button> {
        entries(side)
            .iter()
            .filter_map(|e| match e {
                Entry::Button(b) => Some(*b),
                Entry::Divider => None,
            })
            .collect()
    }

    #[test]
    fn the_left_rail_holds_history_then_the_four_lists_in_order() {
        assert_eq!(
            buttons(RailSide::Left),
            [
                Button::Back,
                Button::Forward,
                Button::Parent,
                Button::Favorites,
                Button::Places,
                Button::RecentFolders,
                Button::RecentFiles
            ]
        );
        assert_eq!(LEFT[3], Entry::Divider);
    }

    #[test]
    fn the_right_rail_holds_the_actions_on_this_folder_in_order() {
        assert_eq!(
            buttons(RailSide::Right),
            [
                Button::NewFolder,
                Button::Find,
                Button::Favorite,
                Button::CopyPath,
                Button::FileManager,
                Button::Terminal
            ]
        );
    }

    #[test]
    fn every_icon_is_on_the_rail_its_side_names() {
        for side in [RailSide::Left, RailSide::Right] {
            for b in buttons(side) {
                assert_eq!(b.side(), side, "{b:?}");
            }
        }
    }

    #[test]
    fn the_lists_and_the_two_prompts_open_menus_and_everything_else_acts_at_once() {
        let with_menu: Vec<_> = buttons(RailSide::Left)
            .into_iter()
            .chain(buttons(RailSide::Right))
            .filter(|b| b.menu().is_some())
            .collect();
        assert_eq!(
            with_menu,
            [
                Button::Favorites,
                Button::Places,
                Button::RecentFolders,
                Button::RecentFiles,
                Button::NewFolder,
                Button::Find
            ]
        );
        for b in with_menu {
            let kind = b.menu().unwrap();
            assert_eq!(kind.button(), Some(b));
            assert_eq!(kind.side(), b.side());
        }
    }

    #[test]
    fn icons_are_stacked_without_overlap_and_fit_a_short_dialog() {
        for side in [RailSide::Left, RailSide::Right] {
            let s = slots(&entries(side));
            for pair in s.windows(2) {
                assert!(pair[1].y >= pair[0].y + pair[0].h, "{pair:?}");
            }
            let last = s.last().unwrap();
            // The shortest dialogs seen are about 400 pixels tall.
            assert!(last.y + last.h <= 380.0);
        }
    }

    #[test]
    fn a_menu_is_anchored_at_the_top_of_its_icon() {
        let layout = Layout::default();
        assert_eq!(anchor_y(&layout, Button::Back), 8);
        assert_eq!(anchor_y(&layout, Button::Forward), 48);
        // Three buttons and a divider above it.
        assert_eq!(anchor_y(&layout, Button::Favorites), 8 + 3 * 40 + 17);
        assert_eq!(anchor_y(&layout, Button::NewFolder), 8);
    }

    #[test]
    fn clicking_a_menus_icon_again_closes_it_and_another_icon_replaces_it() {
        let mut m = Menus::default();
        assert_eq!(m.click(MenuKind::Places), Change::Open(MenuKind::Places));
        assert_eq!(m.click(MenuKind::Find), Change::Open(MenuKind::Find));
        assert_eq!(m.open(), Some(MenuKind::Find));
        assert_eq!(m.click(MenuKind::Find), Change::Close);
        assert_eq!(m.open(), None);
    }

    #[test]
    fn a_toast_shows_only_while_no_menu_is_open_and_a_menu_click_replaces_it() {
        let mut m = Menus::default();
        m.click(MenuKind::Places);
        assert!(!m.show_toast());
        m.close();
        assert!(m.show_toast());
        assert_eq!(m.click(MenuKind::Places), Change::Open(MenuKind::Places));
    }

    #[test]
    fn only_the_menus_that_are_typed_into_take_the_keyboard() {
        assert!(MenuKind::Find.takes_keyboard());
        assert!(MenuKind::NewFolder.takes_keyboard());
        for k in [
            MenuKind::Favorites,
            MenuKind::Places,
            MenuKind::RecentFolders,
            MenuKind::RecentFiles,
            MenuKind::Toast,
        ] {
            assert!(!k.takes_keyboard(), "{k:?}");
        }
    }

    #[test]
    fn a_long_message_gets_a_window_tall_enough_to_read_it() {
        assert_eq!(toast_lines("Path copied"), 1);
        let long = "Could not move the dialog: the folder /home/alex/Projects/a did not read back within 5 s";
        assert!(toast_lines(long) >= 3);
        assert!(MenuKind::Toast.height(toast_lines(long)) > MenuKind::Toast.height(1));
        assert_eq!(MenuKind::Toast.height(1), 64);
        assert_eq!(MenuKind::Toast.height(1_000), 260);
        assert_eq!(toast_lines(&"x".repeat(100)), 3);
    }

    #[test]
    fn a_menu_grows_with_its_rows_within_limits() {
        let one = MenuKind::Places.height(1);
        let five = MenuKind::Places.height(5);
        assert!(five > one);
        assert_eq!(MenuKind::Places.height(0), 90);
        assert_eq!(MenuKind::Places.height(10_000), 520);
        assert_eq!(
            MenuKind::NewFolder.height(40),
            MenuKind::NewFolder.height(0)
        );
    }

    #[test]
    fn the_favorite_toggle_says_what_a_click_will_do() {
        assert!(Button::Favorite.tooltip(false).starts_with("Add"));
        assert!(Button::Favorite.tooltip(true).starts_with("Remove"));
    }

    #[test]
    fn every_icon_has_a_settings_id_on_its_own_rail_and_back() {
        for b in Button::ALL {
            assert_eq!(Button::from_key(b.key()), Some(b));
            assert!(b.side().core().icons().contains(&b.key()), "{b:?}");
        }
        for side in [RailSide::Left, RailSide::Right] {
            assert_eq!(side.core().icons().len(), buttons(side).len());
        }
    }

    #[test]
    fn the_default_layout_matches_the_rails_as_they_come() {
        assert_eq!(Layout::from_core(&RailLayout::default()), Layout::default());
    }

    #[test]
    fn a_changed_layout_moves_the_icons_and_their_menu_anchors() {
        let mut core = RailLayout::default();
        // Favorites to the top of the left rail, then a line below it.
        for i in (1..=4).rev() {
            core.move_up(Side::Left, i);
        }
        core.add_divider_after(Side::Left, 0);
        let layout = Layout::from_core(&core);
        assert_eq!(
            layout.entries(RailSide::Left)[..2],
            [Entry::Button(Button::Favorites), Entry::Divider]
        );
        assert_eq!(anchor_y(&layout, Button::Favorites), 8);
        assert_eq!(anchor_y(&layout, Button::Back), 8 + 40 + 17);
        core.hide(Side::Left, 2);
        let hidden = Layout::from_core(&core);
        assert!(
            !hidden
                .entries(RailSide::Left)
                .contains(&Entry::Button(Button::Back))
        );
    }
}
