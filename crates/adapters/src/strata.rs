//! Strata's file chooser (`strata --portal`).
//!
//! Read and driven over the accessibility bus. The folder is the breadcrumb
//! next to the "Copy path" button, the sidebar places and the parent button
//! can be clicked, and any other folder is reached with Ctrl+L, typed only
//! into the verified-active dialog. Its window opens tiled, so a float rule
//! is installed before it is launched.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use ofolder_core::adapter::{
    Adapter, AdapterError, Capabilities, DialogRequest, WindowRef, known_capabilities,
};
use ofolder_core::geometry::centered;
use ofolder_core::readers::strata_folder;
use ofolder_core::system::{Compositor, Typist};

use crate::a11y::{Node, Role, Tree, find_named, labels};
use crate::atspi_bus::AtspiTree;
use crate::guard::{KeyGuard, backend};
use crate::resident::ResidentDialog;
use crate::selection::{name_in_folder, strata_selected_name};

pub const BACKEND: &str = "org.freedesktop.impl.portal.desktop.strata";
pub const WINDOW_CLASS: &str = "io.github.lgse.Strata.FileChooser";
const APP_HINT: &str = "Strata.FileChooser";
const RULE: &str = "ofolder-strata-dialog";
const SIZE: (i32, i32) = (1000, 640);

/// The sidebar places the dialog offers, by the folder each leads to.
const PLACES: [(&str, &str); 6] = [
    ("Documents", "Documents"),
    ("Downloads", "Downloads"),
    ("Music", "Music"),
    ("Pictures", "Pictures"),
    ("Videos", "Videos"),
    ("Projects", "Projects"),
];

/// The dialog's accessibility root: Strata names its application after the window class.
pub fn root<T: Tree>(tree: &T) -> Option<T::N> {
    tree.applications()
        .into_iter()
        .find(|(name, _)| name.contains(APP_HINT))
        .map(|(_, app)| app)
}

/// The folder shown. The header holds the breadcrumb as a few labels
/// (`~`, `/`, `Work`) beside the "Copy path" button; the nearest container
/// whose labels read as a path is the breadcrumb.
pub fn folder<N: Node>(root: &N, home: &Path) -> Option<PathBuf> {
    let copy = find_named(root, Role::Button, "Copy path")?;
    let mut ancestor = copy.parent();
    for _ in 0..4 {
        let here = ancestor?;
        let pieces = labels(&here);
        let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
        if let Some(path) = strata_folder(&refs, home) {
            return Some(path);
        }
        ancestor = here.parent();
    }
    None
}

/// The sidebar label that leads to `folder`, if there is one.
pub fn place_label(home: &Path, folder: &Path) -> Option<&'static str> {
    if folder == home {
        return Some("Home");
    }
    PLACES
        .iter()
        .find(|(_, dir)| folder == home.join(dir))
        .map(|(label, _)| *label)
}

pub fn click_place<N: Node>(root: &N, label: &str) -> bool {
    find_named(root, Role::Button, label).is_some_and(|b| b.press())
}

pub fn cancel<N: Node>(root: &N) -> bool {
    find_named(root, Role::Button, "Cancel file selection (Escape)").is_some_and(|b| b.press())
}

/// Installs the rule that makes Strata's dialog float, centred, the moment it
/// opens. Its window would otherwise tile first and reflow the layout.
pub fn install_float_rule(compositor: &dyn Compositor) -> Result<(), AdapterError> {
    let bounds = compositor
        .monitors()
        .map_err(backend)?
        .first()
        .map(|m| m.usable_bounds())
        .ok_or_else(|| AdapterError::Backend("no monitor to centre the dialog on".to_owned()))?;
    compositor
        .install_float_rule(RULE, WINDOW_CLASS, SIZE, centered(bounds, SIZE), true)
        .map_err(backend)
}

/// Switches the rule off again. Harmless if it was never on.
pub fn remove_float_rule(compositor: &dyn Compositor) {
    compositor.remove_rule(RULE).ok();
}

pub struct StrataAdapter {
    tree: AtspiTree,
    dialog: ResidentDialog,
    compositor: Arc<dyn Compositor>,
    guard: KeyGuard,
    home: PathBuf,
    rule_installed: bool,
}

impl StrataAdapter {
    /// `title` must be unique among open dialogs: it is how this one is found.
    pub fn new(
        compositor: Arc<dyn Compositor>,
        typist: Arc<dyn Typist>,
        title: &str,
        home: PathBuf,
    ) -> Result<Self, AdapterError> {
        Ok(Self {
            tree: AtspiTree::connect().map_err(AdapterError::Backend)?,
            dialog: ResidentDialog::new(
                BACKEND,
                WINDOW_CLASS,
                title.to_owned(),
                compositor.clone(),
            ),
            guard: KeyGuard::new(compositor.clone(), typist, Duration::from_millis(250)),
            compositor,
            home,
            rule_installed: false,
        })
    }

    /// Watches a dialog the portal frontend opened, found by its title. The
    /// float rule for Strata's tiled window must already have been installed
    /// by whoever forwarded the request.
    pub fn attach(
        compositor: Arc<dyn Compositor>,
        typist: Arc<dyn Typist>,
        title: &str,
        home: PathBuf,
    ) -> Result<Self, AdapterError> {
        let mut adapter = Self::new(compositor, typist, title, home)?;
        adapter.dialog.set_attached();
        Ok(adapter)
    }

    fn root(&self) -> Option<<AtspiTree as Tree>::N> {
        root(&self.tree)
    }

    fn remove_rule(&mut self) {
        if self.rule_installed {
            remove_float_rule(&*self.compositor);
            self.rule_installed = false;
        }
    }
}

impl Drop for StrataAdapter {
    fn drop(&mut self) {
        self.remove_rule();
    }
}

impl Adapter for StrataAdapter {
    fn id(&self) -> &'static str {
        "strata"
    }

    fn capabilities(&self) -> Capabilities {
        known_capabilities("strata")
    }

    fn launch(&mut self, request: &DialogRequest) -> Result<(), AdapterError> {
        if self.dialog.is_attached() {
            return Err(AdapterError::Refused(
                "this dialog was opened by someone else".to_owned(),
            ));
        }
        install_float_rule(&*self.compositor)?;
        self.rule_installed = true;
        let started = self.dialog.start(request);
        if started.is_err() {
            self.remove_rule();
        }
        started
    }

    fn window(&self) -> Option<WindowRef> {
        self.dialog.window()
    }

    fn read_folder(&self) -> Option<PathBuf> {
        folder(&self.root()?, &self.home).filter(|p| p.is_dir())
    }

    /// Read twice around the folder, so an item read while the dialog moved
    /// is not joined to the wrong folder.
    fn selection(&self) -> Option<PathBuf> {
        let root = self.root()?;
        let before = folder(&root, &self.home).filter(|p| p.is_dir())?;
        let name = strata_selected_name(&root, &before)?;
        (folder(&root, &self.home)? == before)
            .then(|| name_in_folder(&before, &name))
            .flatten()
    }

    fn goto(&mut self, target: &Path) -> Result<(), AdapterError> {
        if let Some(label) = place_label(&self.home, target)
            && let Some(root) = self.root()
            && click_place(&root, label)
        {
            return Ok(());
        }
        let window = || self.dialog.window().map(|w| w.address);
        self.guard.type_path(&window, target)
    }

    fn close(&mut self) -> Result<(), AdapterError> {
        if let Some(root) = self.root() {
            cancel(&root);
        }
        for _ in 0..50 {
            if !self.dialog.alive() {
                self.remove_rule();
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

    const HOME: &str = "/home/j";

    /// The shape dumped from the real dialog: sidebar places as buttons, and a
    /// header whose breadcrumb is a button and labels beside "Copy path".
    fn dialog(crumbs: &[&str]) -> FakeNode {
        let app = FakeNode::root(Role::Other, "io.github.lgse.Strata.FileChooser");
        let frame = app.add(Role::Other, "Strata spike save");
        let sidebar = frame.add(Role::Other, "");
        for place in ["Home", "Recent", "Documents", "Downloads"] {
            let b = sidebar.add(Role::Button, place);
            b.add(Role::Label, place);
        }
        let header = frame.add(Role::Other, "");
        header.add(Role::Button, crumbs.first().copied().unwrap_or("~"));
        for crumb in crumbs {
            header.add(Role::Label, crumb);
        }
        header.add(Role::Button, "Copy path");
        frame.add(Role::Button, "Cancel file selection (Escape)");
        let files = frame.add(Role::List, "files");
        let row = files.add(Role::Other, "row");
        row.add(Role::Label, "a-label-in-the-file-list");
        app
    }

    #[test]
    fn the_application_is_found_by_part_of_its_name() {
        let tree = FakeTree(vec![
            ("quickshell".to_owned(), FakeNode::root(Role::Other, "x")),
            (
                "io.github.lgse.Strata.FileChooser".to_owned(),
                dialog(&["~"]),
            ),
        ]);
        assert!(root(&tree).is_some());
        assert!(root(&FakeTree(Vec::new())).is_none());
    }

    #[test]
    fn at_home_the_breadcrumb_is_just_a_tilde() {
        assert_eq!(
            folder(&dialog(&["~"]), Path::new(HOME)),
            Some(PathBuf::from(HOME))
        );
    }

    #[test]
    fn a_folder_under_home_reads_from_the_breadcrumb_pieces() {
        let d = dialog(&["~", "/", "Work", "/", "ofolder"]);
        assert_eq!(
            folder(&d, Path::new(HOME)),
            Some(PathBuf::from("/home/j/Work/ofolder"))
        );
    }

    #[test]
    fn a_folder_outside_home_is_absolute() {
        let d = dialog(&["/", "usr", "/", "bin"]);
        assert_eq!(folder(&d, Path::new(HOME)), Some(PathBuf::from("/usr/bin")));
    }

    #[test]
    fn labels_in_the_file_list_never_leak_into_the_breadcrumb() {
        let d = dialog(&["~", "/", "Documents"]);
        assert_eq!(
            folder(&d, Path::new(HOME)),
            Some(PathBuf::from("/home/j/Documents"))
        );
    }

    #[test]
    fn a_dialog_with_no_copy_path_button_reads_as_unknown() {
        let app = FakeNode::root(Role::Other, "io.github.lgse.Strata.FileChooser");
        app.add(Role::Label, "~");
        assert_eq!(folder(&app, Path::new(HOME)), None);
    }

    #[test]
    fn a_breadcrumb_that_is_not_a_path_reads_as_unknown() {
        assert_eq!(folder(&dialog(&["Recent"]), Path::new(HOME)), None);
    }

    #[test]
    fn the_sidebar_places_are_found_by_folder() {
        let home = Path::new(HOME);
        assert_eq!(place_label(home, Path::new("/home/j")), Some("Home"));
        assert_eq!(
            place_label(home, Path::new("/home/j/Documents")),
            Some("Documents")
        );
        assert_eq!(place_label(home, Path::new("/home/j/Work")), None);
        assert_eq!(place_label(home, Path::new("/usr")), None);
    }

    #[test]
    fn clicking_a_place_presses_its_sidebar_button() {
        let d = dialog(&["~"]);
        assert!(click_place(&d, "Documents"));
        assert_eq!(
            find_named(&d, Role::Button, "Documents").map(|b| b.presses()),
            Some(1)
        );
        assert!(
            !click_place(&d, "Music"),
            "a place the sidebar lacks cannot be clicked"
        );
    }

    #[test]
    fn cancel_presses_the_cancel_button() {
        let d = dialog(&["~"]);
        assert!(cancel(&d));
    }
}
