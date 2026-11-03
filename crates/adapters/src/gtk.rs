//! The GTK portal's file chooser (`xdg-desktop-portal-gtk`).
//!
//! Everything is done over the accessibility bus, with no keystrokes: the
//! folder is the checked button in the path bar, and the dialog is moved by
//! putting a folder path into its name entry and activating it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use ofolder_core::adapter::{
    Adapter, AdapterError, Capabilities, DialogRequest, WindowRef, known_capabilities,
};
use ofolder_core::readers::{PathBarButton, gtk_folder};
use ofolder_core::system::Compositor;

use crate::a11y::{ACTIVATE_GAP, Node, Role, Tree, find, find_named};
use crate::atspi_bus::AtspiTree;
use crate::resident::ResidentDialog;
use crate::selection::{gtk_selected_name, name_in_folder};

pub const APP: &str = "xdg-desktop-portal-gtk";
pub const BACKEND: &str = "org.freedesktop.impl.portal.desktop.gtk";
pub const WINDOW_CLASS: &str = "xdg-desktop-portal-gtk";

/// The dialog with this title, among everything the portal process shows.
pub fn chooser<T: Tree>(tree: &T, title: &str) -> Option<T::N> {
    let (_, app) = tree
        .applications()
        .into_iter()
        .find(|(name, _)| name == APP)?;
    app.children()
        .into_iter()
        .find(|c| c.role() == Role::FileChooser && c.name() == title)
}

/// The folder shown, from the path bar: a row of toggle buttons, deepest
/// first, of which the checked one is the current folder. This is the shape
/// only; whether the folder exists is for the caller to check.
pub fn folder<N: Node>(chooser: &N) -> Option<PathBuf> {
    let bar = find_named(chooser, Role::Filler, "PathBar Layer")?;
    let row = bar.children().into_iter().next()?;
    let buttons: Vec<(String, bool)> = row
        .children()
        .into_iter()
        .filter(|b| b.role() == Role::ToggleButton)
        .map(|b| (b.name(), b.checked()))
        .collect();
    let view: Vec<PathBarButton<'_>> = buttons
        .iter()
        .map(|(name, checked)| PathBarButton {
            name,
            checked: *checked,
        })
        .collect();
    gtk_folder(&view)
}

pub fn name_entry<N: Node>(chooser: &N) -> Option<N> {
    find(chooser, |n| n.role() == Role::Text && n.editable())
}

/// Moves the dialog by writing `folder/` into its name entry and activating it
/// twice: the first activation is swallowed after a move. Returns the entry's
/// previous text, which is the file name the application suggested and which
/// GTK clears when the move lands.
pub fn move_to<N: Node>(chooser: &N, folder: &Path, gap: Duration) -> Result<String, AdapterError> {
    let text = folder
        .to_str()
        .ok_or_else(|| AdapterError::Refused("the folder is not valid UTF-8".to_owned()))?;
    let entry = name_entry(chooser)
        .ok_or_else(|| AdapterError::Backend("the dialog has no name entry".to_owned()))?;
    let saved = entry.text().unwrap_or_default();
    if !entry.set_text(&format!("{}/", text.trim_end_matches('/'))) {
        return Err(AdapterError::Backend(
            "could not write into the name entry".to_owned(),
        ));
    }
    entry.press();
    sleep(gap);
    entry.press();
    Ok(saved)
}

pub fn restore_name<N: Node>(chooser: &N, name: &str) -> bool {
    name_entry(chooser).is_some_and(|entry| entry.set_text(name))
}

pub fn cancel<N: Node>(chooser: &N) -> bool {
    find_named(chooser, Role::Button, "Cancel").is_some_and(|b| b.press())
}

pub struct GtkAdapter {
    tree: AtspiTree,
    dialog: ResidentDialog,
    gap: Duration,
    /// The application's suggested file name, held while a move is in flight.
    saved_name: Option<String>,
}

impl GtkAdapter {
    /// `title` must be unique among open dialogs: it is how this one is found.
    pub fn new(compositor: Arc<dyn Compositor>, title: &str) -> Result<Self, AdapterError> {
        Ok(Self {
            tree: AtspiTree::connect().map_err(AdapterError::Backend)?,
            dialog: ResidentDialog::new(BACKEND, WINDOW_CLASS, title.to_owned(), compositor),
            gap: ACTIVATE_GAP,
            saved_name: None,
        })
    }

    /// Watches a dialog the portal frontend opened, found by its title.
    pub fn attach(compositor: Arc<dyn Compositor>, title: &str) -> Result<Self, AdapterError> {
        let mut adapter = Self::new(compositor, title)?;
        adapter.dialog.set_attached();
        Ok(adapter)
    }

    fn chooser(&self) -> Option<<AtspiTree as Tree>::N> {
        chooser(&self.tree, self.dialog.title())
    }

    /// The text of the dialog's name entry: the file name the application suggested.
    pub fn suggested_name(&self) -> Option<String> {
        name_entry(&self.chooser()?)?.text()
    }
}

impl Adapter for GtkAdapter {
    fn id(&self) -> &'static str {
        "gtk"
    }

    fn capabilities(&self) -> Capabilities {
        known_capabilities("gtk")
    }

    fn launch(&mut self, request: &DialogRequest) -> Result<(), AdapterError> {
        self.dialog.start(request)
    }

    fn window(&self) -> Option<WindowRef> {
        self.dialog.window()
    }

    fn read_folder(&self) -> Option<PathBuf> {
        folder(&self.chooser()?).filter(|p| p.is_dir())
    }

    fn goto(&mut self, target: &Path) -> Result<(), AdapterError> {
        let chooser = self.chooser().ok_or(AdapterError::NotRunning)?;
        let saved = move_to(&chooser, target, self.gap)?;
        // Keep the first suggestion if moves overlap: a later read would see the path we typed.
        self.saved_name.get_or_insert(saved);
        Ok(())
    }

    /// Read twice around the folder, so a row read while the dialog moved
    /// is not joined to the wrong folder.
    fn selection(&self) -> Option<PathBuf> {
        let chooser = self.chooser()?;
        let before = folder(&chooser).filter(|p| p.is_dir())?;
        let name = gtk_selected_name(&chooser)?;
        (folder(&chooser)? == before)
            .then(|| name_in_folder(&before, &name))
            .flatten()
    }

    fn after_arrival(&mut self) {
        if let (Some(name), Some(chooser)) = (self.saved_name.take(), self.chooser()) {
            restore_name(&chooser, &name);
        }
    }

    fn close(&mut self) -> Result<(), AdapterError> {
        if let Some(chooser) = self.chooser() {
            cancel(&chooser);
        }
        for _ in 0..50 {
            if !self.dialog.alive() {
                return Ok(());
            }
            sleep(Duration::from_millis(40));
        }
        Err(AdapterError::Backend("the dialog did not close".to_owned()))
    }

    fn alive(&self) -> bool {
        self.dialog.alive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a11y::fake::{FakeNode, FakeTree};

    /// The shape dumped from the real dialog: the file chooser, a header with
    /// the name entry and buttons, and the widget holding the path bar.
    fn dialog(title: &str, checked: &str) -> FakeNode {
        let app = FakeNode::root(Role::Other, APP);
        let ch = app.add(Role::FileChooser, title);
        ch.add(Role::ToggleButton, "");
        ch.add(Role::Button, "Save");
        ch.add(Role::Label, "Name");
        ch.add(Role::Text, "").editable_with("spike.txt");
        ch.add(Role::Button, "Cancel");
        let widget = ch.add(Role::FileChooser, "File Chooser Widget");
        let revealer = widget.add(Role::Other, "Browse Header Revealer");
        let stack = revealer.add(Role::Other, "Browse Header Stack");
        let bar = stack.add(Role::Filler, "PathBar Layer");
        let row = bar.add(Role::Other, "");
        for name in ["Documents", "user", "home", "File System Root"] {
            let b = row.add(Role::ToggleButton, name);
            if name == checked {
                b.checked_on();
            }
        }
        row.add(Role::Button, "Up Path");
        bar.add(Role::ToggleButton, "Create Folder");
        let files = widget.add(Role::List, "files");
        files.add(Role::Button, "a-button-in-the-file-list");
        app
    }

    #[test]
    fn the_dialog_is_found_by_its_title_among_several() {
        let app = dialog("one", "user");
        let other = app.add(Role::FileChooser, "two");
        other.add(Role::Button, "Cancel");
        let tree = FakeTree(vec![(APP.to_owned(), app)]);
        assert_eq!(
            chooser(&tree, "two").map(|n| n.name()),
            Some("two".to_owned())
        );
        assert!(chooser(&tree, "three").is_none());
    }

    #[test]
    fn another_application_with_a_dialog_of_the_same_title_is_not_picked() {
        let tree = FakeTree(vec![(
            "some-other-app".to_owned(),
            dialog("one", "user"),
        )]);
        assert!(chooser(&tree, "one").is_none());
    }

    #[test]
    fn the_folder_is_the_checked_path_bar_button_and_its_ancestors() {
        let app = dialog("t", "user");
        let ch = app.children().remove(0);
        assert_eq!(folder(&ch), Some(PathBuf::from("/home/user")));
    }

    #[test]
    fn a_path_bar_with_nothing_checked_reads_as_unknown() {
        let app = dialog("t", "nothing");
        assert_eq!(folder(&app.children().remove(0)), None);
    }

    #[test]
    fn a_dialog_without_a_path_bar_reads_as_unknown() {
        let app = FakeNode::root(Role::Other, APP);
        let ch = app.add(Role::FileChooser, "t");
        ch.add(Role::Button, "Cancel");
        assert_eq!(folder(&ch), None);
    }

    #[test]
    fn a_move_writes_the_folder_with_a_trailing_slash_and_activates_twice() {
        let app = dialog("t", "user");
        let ch = app.children().remove(0);
        let saved = move_to(&ch, Path::new("/home/user/Work"), Duration::ZERO).expect("moved");
        let entry = name_entry(&ch).expect("entry");
        assert_eq!(
            saved, "spike.txt",
            "the application's file name is handed back"
        );
        assert_eq!(entry.text().as_deref(), Some("/home/user/Work/"));
        assert_eq!(entry.presses(), 2);
    }

    #[test]
    fn the_applications_file_name_can_be_put_back_after_the_move() {
        let app = dialog("t", "user");
        let ch = app.children().remove(0);
        let saved = move_to(&ch, Path::new("/tmp"), Duration::ZERO).expect("moved");
        assert!(restore_name(&ch, &saved));
        assert_eq!(
            name_entry(&ch).and_then(|e| e.text()).as_deref(),
            Some("spike.txt")
        );
    }

    #[test]
    fn a_folder_that_is_not_utf8_is_refused_before_anything_is_written() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let app = dialog("t", "user");
        let ch = app.children().remove(0);
        let odd = PathBuf::from(OsStr::from_bytes(b"/h/\xff"));
        assert!(matches!(
            move_to(&ch, &odd, Duration::ZERO),
            Err(AdapterError::Refused(_))
        ));
        assert_eq!(
            name_entry(&ch).and_then(|e| e.text()).as_deref(),
            Some("spike.txt")
        );
    }

    #[test]
    fn cancel_presses_the_cancel_button_and_not_one_inside_the_file_list() {
        let app = dialog("t", "user");
        let ch = app.children().remove(0);
        assert!(cancel(&ch));
        assert_eq!(
            find_named(&ch, Role::Button, "Cancel").map(|b| b.presses()),
            Some(1)
        );
    }
}
