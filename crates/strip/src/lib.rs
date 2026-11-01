//! The strip: two panels of chrome docked either side of a dialog, the left
//! one to go somewhere and the right one to do something here.
//!
//! The strip knows nothing about any dialog. It shows a folder, some
//! favorites and a few actions, and asks a [`StripActions`] to do the work
//! that touches the dialog. The logic lives in plain modules (`model`,
//! `favorites`, `paths`, `naming`) that need no display; `view` only draws.

mod demo;
mod error;
pub mod favorites;
mod field;
mod left;
pub mod model;
pub mod naming;
pub mod paths;
mod right;
mod view;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size,
};

pub use demo::DemoActions;
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
}

pub struct StripOptions {
    /// Short backend name shown in the header, e.g. "gtk".
    pub label: String,
    /// JSON file the favorites are saved in.
    pub favorites_path: PathBuf,
    /// For abbreviating paths as `~/...`.
    pub home: PathBuf,
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
    let StripOptions {
        label,
        favorites_path,
        home,
    } = options;
    let vm = ViewModel::new(label.clone(), home, favorites_path);
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
    let work = base.join("Work/ofolder");
    for sample in ["Documents", "Downloads", "Work/ofolder"] {
        std::fs::create_dir_all(base.join(sample))?;
    }
    let mut samples = favorites::Favorites::default();
    for sample in ["Documents", "Downloads", "Work/ofolder"] {
        samples.add(base.join(sample));
    }
    samples.add(base.join("Deleted"));
    let favorites_path = base.join("favorites.json");
    favorites::save(&favorites_path, &samples)?;
    let options = StripOptions {
        label: "demo".to_owned(),
        favorites_path,
        home: base.clone(),
    };
    let result = run_strip(options, Arc::new(DemoActions::new(work)));
    std::fs::remove_dir_all(&base).ok();
    result
}
