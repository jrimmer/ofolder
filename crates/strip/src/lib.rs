//! The strip: two slim rails of icons docked either side of a dialog, the
//! left one to go somewhere and the right one to do something here. Icons
//! that need more room open a menu beside their rail.
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
mod hint;
mod icons;
mod left;
mod lines;
mod menu;
pub mod model;
pub mod naming;
pub mod paths;
mod popup;
mod preview;
pub mod rail;
mod right;
mod rows;
mod settings_window;
mod sources;
mod view;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ofolder_core::shortcuts::{Combo, Shortcuts};

use gpui::{
    AnyWindowHandle, App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions,
    prelude::*, px, size,
};

pub use demo::{DemoActions, demo_setup};
pub use error::StripError;

pub use hint::TipAnchor;
use left::LeftRail;
use model::ViewModel;
pub use rail::{MenuKind, RailSide};
use right::RightRail;
use view::Shared;

/// The window class of the left panel ("go somewhere"). Hyprland rules match
/// on it, so it must stay stable.
pub const APP_ID_LEFT: &str = "ofolder.strip.left";
/// The window class of the right panel ("do something here").
pub const APP_ID_RIGHT: &str = "ofolder.strip.right";
/// The window class of a rail's menu, and of the message shown while no menu
/// is open.
pub const APP_ID_MENU: &str = "ofolder.strip.menu";
/// The window class of a rail icon's tooltip.
pub const APP_ID_TIP: &str = "ofolder.strip.tip";
/// The window class of the settings window the gear opens.
pub const APP_ID_SETTINGS: &str = "ofolder.strip.settings";
/// The window class of the preview panel under the dialog.
pub const APP_ID_PREVIEW: &str = "ofolder.strip.preview";

/// The settings window about to open: its size, centred on the dialog's
/// monitor. It takes the keyboard, since the user clicked for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsRequest {
    pub width: i32,
    pub height: i32,
}

/// The left rail's width in logical pixels.
pub const LEFT_WIDTH: f32 = rail::RAIL_WIDTH;
/// The right rail's width in logical pixels.
pub const RIGHT_WIDTH: f32 = rail::RAIL_WIDTH;

/// A menu about to open, or open and changing size, beside a rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuRequest {
    /// The rail it belongs to.
    pub side: RailSide,
    /// Where its top goes, from the top of the rail; past the rail's foot
    /// means "end with the rail".
    pub anchor_y: i32,
    pub width: i32,
    pub height: i32,
    /// Whether the menu should get the keyboard when it opens.
    pub takes_keyboard: bool,
    /// True when the window is about to open, so a placement rule must be
    /// in force before it appears; false for a size change of an open menu.
    pub fresh: bool,
    /// Open on the rail's outer side, clear of the dialog: for a message,
    /// which must not cover the dialog's own buttons.
    pub away_from_dialog: bool,
}

/// A rail icon's tooltip about to open beside its rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TipRequest {
    pub side: RailSide,
    /// The icon's centre on its rail.
    pub anchor: TipAnchor,
    pub width: i32,
    pub height: i32,
}

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
    /// Where the menu window goes: `Some` before one opens and whenever it
    /// changes size, `None` once it is closed. Called on the UI thread, so
    /// it must return quickly.
    fn place_menu(&self, menu: Option<MenuRequest>);
    /// Where a tooltip window goes: `Some` just before one opens, `None`
    /// once it is closed. It must never take the keyboard. Called on the UI
    /// thread, so it must return quickly.
    fn place_tip(&self, tip: Option<TipRequest>);
    /// Whether the pointer is over the rail on `side`, or None when that
    /// cannot be told. Called from a background thread while a tooltip is
    /// up, about five times a second.
    fn pointer_on_rail(&self, side: RailSide) -> Option<bool>;
    /// Where the settings window goes: `Some` just before it opens, `None`
    /// once it is closed. Called on the UI thread, so it must return quickly.
    fn place_settings(&self, settings: Option<SettingsRequest>);
    /// Commands from keyboard shortcuts that arrived since the last call,
    /// oldest first, by command id. Called on the UI thread many times a
    /// second, so it must return at once. Without shortcuts, none.
    fn take_commands(&self) -> Vec<&'static str> {
        Vec::new()
    }
    /// The shortcuts that could not be bound when the strip started, by
    /// command id, with why.
    fn shortcut_problems(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Binds `shortcuts` in place of the ones bound now, after the settings
    /// window changed them; returns those that could not be bound.
    fn rebind(&self, _shortcuts: &Shortcuts) -> Vec<(String, String)> {
        Vec::new()
    }
    /// What the user's own compositor configuration does with `combo`, when
    /// it binds it, so a shortcut never shadows it.
    fn taken_by(&self, _combo: &Combo) -> Option<String> {
        None
    }
    /// Puts `text` on the clipboard without a window of the strip's own: a
    /// shortcut comes with no input event, and a Wayland window may only set
    /// the clipboard in answer to one.
    /// Puts the guard in place (`on`) or takes it away after its setting changed in the settings
    /// window; says what was done. Called from a background thread, since it can start programs.
    fn set_guard(&self, _on: bool) -> Result<String, String> {
        Ok(String::new())
    }
    /// The folders file-manager windows are showing now, as (program, folder), each from the
    /// window's own answer. Called from a background thread every few seconds.
    fn open_folders(&self) -> Vec<(String, PathBuf)> {
        Vec::new()
    }
    /// Empties the recent folders and files, as `ofolder forget` does, and
    /// says what went. Favorites are kept. Called from a background thread.
    fn forget_recents(&self) -> Result<String, String> {
        Ok(String::new())
    }
    /// The item selected in the dialog, when the dialog can say; for the
    /// preview panel. Called from a background thread a few times a second.
    fn selection(&self) -> Option<PathBuf> {
        None
    }
    /// The preview panel's size if it were shown under the dialog now, or
    /// `None` when there is no room below the dialog. Called from a
    /// background thread.
    fn preview_size(&self) -> Option<(i32, i32)> {
        None
    }
    /// The preview panel is about to open (`true`), so its placement must be
    /// in force before it maps, or it has closed (`false`). It never takes
    /// the keyboard. Called on the UI thread, so it must return quickly.
    fn place_preview(&self, _open: bool) {}
    fn copy_text(&self, _text: &str) -> Result<(), String> {
        Err("the clipboard cannot be set from a shortcut here".to_owned())
    }
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
    /// Where every message shown is logged, so one that went by can be read
    /// again; `None` keeps no log.
    pub messages_path: Option<PathBuf>,
}

fn window_options(
    app_id: &str,
    title: &str,
    (width, height): (f32, f32),
    cx: &mut App,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(width), px(height)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_owned().into()),
            ..Default::default()
        }),
        app_id: Some(app_id.to_owned()),
        // The dialog keeps the keyboard until the user clicks a rail. A menu
        // that is typed into gets it through its placement rule instead.
        focus: false,
        ..Default::default()
    }
}

/// The options for a menu window of `size`.
pub(crate) fn menu_options(size: (i32, i32), cx: &mut App) -> WindowOptions {
    window_options(
        APP_ID_MENU,
        "ofolder menu",
        (size.0 as f32, size.1 as f32),
        cx,
    )
}

/// The options for a tooltip window of `size`.
pub(crate) fn tip_options(size: (i32, i32), cx: &mut App) -> WindowOptions {
    window_options(
        APP_ID_TIP,
        "ofolder tip",
        (size.0 as f32, size.1 as f32),
        cx,
    )
}

/// The options for the preview panel of `size`.
pub(crate) fn preview_options(size: (i32, i32), cx: &mut App) -> WindowOptions {
    window_options(
        APP_ID_PREVIEW,
        "ofolder preview",
        (size.0 as f32, size.1 as f32),
        cx,
    )
}

/// The options for the settings window of `size`.
pub(crate) fn settings_options(size: (i32, i32), cx: &mut App) -> WindowOptions {
    window_options(
        APP_ID_SETTINGS,
        "ofolder settings",
        (size.0 as f32, size.1 as f32),
        cx,
    )
}

/// Opens both panels and runs until the dialog is gone or either window is
/// closed.
pub fn run_strip(options: StripOptions, actions: Arc<dyn StripActions>) -> anyhow::Result<()> {
    run_strip_preview(options, actions, |_| None)
}

/// Like [`run_strip`], but lets the caller put the view-model in a state
/// first and name a menu to open, so a screenshot can show open rows, menus
/// and results. For the demo only.
#[doc(hidden)]
pub fn run_strip_preview(
    options: StripOptions,
    actions: Arc<dyn StripActions>,
    prepare: impl FnOnce(&mut ViewModel) -> Option<MenuKind>,
) -> anyhow::Result<()> {
    colors::set(colors::Colors::from_palette(&options.palette));
    let mut vm = ViewModel::new(&options);
    let open_first = prepare(&mut vm);
    let label = options.label;
    let settings_path = options.settings_path.clone();
    let home = options.home.clone();
    let messages_path = options.messages_path.clone();
    Application::new().with_assets(icons::Icons).run(move |cx| {
        let backend = label.clone();
        let shared = cx.new(|cx| {
            let mut shared = Shared::new(vm, actions, settings_path, home, cx);
            shared.label = backend;
            shared.messages_path = messages_path;
            shared
        });
        let left_options = window_options(APP_ID_LEFT, "ofolder go", (LEFT_WIDTH, HEIGHT), cx);
        let left_shared = shared.clone();
        let left = cx.open_window(left_options, move |window, cx| {
            cx.new(|cx| LeftRail::new(left_shared, window, cx))
        });
        let right_options = window_options(APP_ID_RIGHT, "ofolder do", (RIGHT_WIDTH, HEIGHT), cx);
        let right_shared = shared.clone();
        let right = cx.open_window(right_options, move |window, cx| {
            cx.new(|cx| RightRail::new(right_shared, window, cx))
        });
        let rails: Vec<AnyWindowHandle> = match (left, right) {
            (Ok(l), Ok(r)) => vec![l.into(), r.into()],
            (l, r) => {
                if let Some(error) = l.err().or(r.err()) {
                    eprintln!("ofolder: could not open a rail window: {error}");
                }
                cx.quit();
                return;
            }
        };
        // A rail closing ends the process, so no rail is left alone. A menu
        // closing only tells the shared state it has gone.
        let watched = shared.clone();
        cx.on_window_closed(move |cx| {
            let open: Vec<_> = cx.windows().iter().map(|w| w.window_id()).collect();
            if rails.iter().any(|r| !open.contains(&r.window_id())) {
                cx.quit();
                return;
            }
            watched.update(cx, |s, cx| {
                s.menu_window_closed(&open, cx);
                s.tip_window_closed(&open);
                s.settings_window_closed(&open, cx);
                s.preview_window_closed(&open, cx);
            });
        })
        .detach();
        if let Some(kind) = open_first {
            shared.update(cx, |s, cx| s.click_menu(kind, cx));
        }
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
