//! The strip: two panels of chrome docked either side of a dialog, the left
//! one to go somewhere and the right one to do something here.
//!
//! The strip knows nothing about any dialog. It shows a folder, some
//! favorites and a few actions, and asks a [`StripActions`] to do the work
//! that touches the dialog. The logic lives in plain modules (`model`,
//! `favorites`, `paths`, `naming`) that need no display; `view` only draws.

mod colors;
mod demo;
mod error;
pub mod expand;
pub mod favorites;
mod field;
mod finder;
mod left;
mod lines;
mod menu;
pub mod model;
pub mod naming;
pub mod paths;
mod right;
mod rows;
mod sources;
mod view;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size,
};

pub use demo::{DemoActions, demo_setup};
pub use error::StripError;

use left::LeftPanel;
use model::ViewModel;
use right::RightPanel;
use view::Shared;

/// The window class of the left panel ("go somewhere"). Hyprland rules match
/// on it, so it must stay stable.
pub const APP_ID_LEFT: &str = "ofolder.strip.left";
/// The window class of the right panel ("do something here").
pub const APP_ID_RIGHT: &str = "ofolder.strip.right";

/// The left panel's width in logical pixels.
pub const LEFT_WIDTH: f32 = 200.0;
/// The right panel's width in logical pixels.
pub const RIGHT_WIDTH: f32 = 230.0;

/// The height the window opens with; the caller may resize it.
const HEIGHT: f32 = 480.0;

/// What the strip needs from whoever docks it.
pub trait StripActions: Send + Sync + 'static {
    /// The folder the dialog shows, or None when unknown. Called about five times a second from a background thread, so it must be cheap.
    fn current_folder(&self) -> Option<PathBuf>;
    /// Move the dialog to `folder` and confirm it arrived. An Err message is shown to the user in the strip.
    fn go(&self, folder: &Path) -> Result<(), String>;
    /// Create a folder called `name` (may contain '/', meaning nested folders) inside the dialog's current folder, then move into it. Returns the created path.
    fn new_folder(&self, name: &str) -> Result<PathBuf, String>;
    /// False once the dialog is gone; the strip then closes itself and the process ends.
    fn dialog_alive(&self) -> bool;
    /// Open `folder` in the user's file manager.
    fn open_in_file_manager(&self, folder: &Path) -> Result<(), String>;
    /// Open a terminal in `folder`.
    fn open_in_terminal(&self, folder: &Path) -> Result<(), String>;
}

pub struct StripOptions {
    /// Short backend name shown in the header, e.g. "gtk".
    pub label: String,
    /// JSON file the favorites are saved in.
    pub favorites_path: PathBuf,
    /// For abbreviating paths as `~/...`.
    pub home: PathBuf,
    /// Which application opened the dialog: a portal id, `class:<name>`, or
    /// empty when it is not known.
    pub app_key: String,
    /// The recent folders and files file.
    pub recents_path: PathBuf,
    /// The settings file.
    pub settings_path: PathBuf,
    /// The `user-dirs.dirs` file.
    pub user_dirs_path: PathBuf,
    /// Normally `/proc/self/mountinfo`.
    pub mountinfo_path: PathBuf,
    /// The login name, for finding the user's mounted drives.
    pub user: String,
    /// The desktop theme's colours; the default palette is the built-in dark look.
    pub palette: ofolder_core::theme::Palette,
}

fn window_options(app_id: &str, title: &str, width: f32, cx: &mut App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(width), px(HEIGHT)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_owned().into()),
            ..Default::default()
        }),
        app_id: Some(app_id.to_owned()),
        // The dialog keeps the keyboard until the user clicks a panel.
        focus: false,
        ..Default::default()
    }
}

/// Opens both panels and runs until the dialog is gone or either window is
/// closed.
pub fn run_strip(options: StripOptions, actions: Arc<dyn StripActions>) -> anyhow::Result<()> {
    run_strip_preview(options, actions, |_| {})
}

/// Like [`run_strip`], but lets the caller put the view-model in a state
/// first, so a screenshot can show open rows, menus and results. For the
/// demo only.
#[doc(hidden)]
pub fn run_strip_preview(
    options: StripOptions,
    actions: Arc<dyn StripActions>,
    prepare: impl FnOnce(&mut ViewModel),
) -> anyhow::Result<()> {
    colors::set(colors::Colors::from_palette(&options.palette));
    let mut vm = ViewModel::new(&options);
    prepare(&mut vm);
    let label = options.label;
    Application::new().run(move |cx| {
        let shared = cx.new(|cx| Shared::new(vm, actions, cx));
        let left_options = window_options(APP_ID_LEFT, "ofolder favorites", LEFT_WIDTH, cx);
        let left_shared = shared.clone();
        let left = cx.open_window(left_options, move |_, cx| {
            cx.new(|cx| LeftPanel::new(left_shared, cx))
        });
        let right_options = window_options(APP_ID_RIGHT, "ofolder", RIGHT_WIDTH, cx);
        let right = cx.open_window(right_options, move |_, cx| {
            cx.new(|cx| RightPanel::new(shared, label, cx))
        });
        if let Some(error) = left.err().or(right.err()) {
            eprintln!("ofolder: could not open a panel window: {error}");
            cx.quit();
            return;
        }
        // Either window closing ends the process, so no panel is left alone.
        cx.on_window_closed(|cx| cx.quit()).detach();
        // No `cx.activate`: that would take the keyboard from the dialog.
    });
    Ok(())
}

/// Opens both panels on their own, in a scratch folder under the system temporary
/// directory with a few sample favorites, to see how it draws. Real strips
/// are opened beside a dialog by the portal backend.
pub fn run_demo() -> anyhow::Result<()> {
    let base = std::env::temp_dir().join(format!("ofolder-demo-{}", std::process::id()));
    std::fs::create_dir_all(&base)?;
    let (options, work) = demo::demo_setup(&base, Default::default())?;
    let result = run_strip(options, Arc::new(DemoActions::new(work)));
    std::fs::remove_dir_all(&base).ok();
    result
}
