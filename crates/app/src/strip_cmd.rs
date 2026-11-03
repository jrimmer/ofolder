//! `ofolder strip`: the strip for one dialog.
//!
//! One process per dialog. It attaches to a dialog the portal frontend
//! opened, docks a gpui strip beside it, drives the dialog through the
//! adapter, and exits when the dialog closes. The portal proxy starts it and
//! stops it; it can also be started by hand for a dialog that is already open.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use ofolder_adapters::flea::{FleaAdapter, find_new_picker};
use ofolder_adapters::gtk::GtkAdapter;
use ofolder_adapters::hyprctl::Hyprctl;
use ofolder_adapters::strata::StrataAdapter;
use ofolder_adapters::wtype::Wtype;
use ofolder_core::adapter::{Adapter, goto_verified, stable_folder};
use ofolder_core::dialog_memory;
use ofolder_core::geometry::{Panels, Rect, centre_in, place_menu, place_panels, place_tip};
use ofolder_core::launch::{self, Command};
use ofolder_core::settings;
use ofolder_core::system::{Compositor, Typist};
use ofolder_strip::{
    APP_ID_LEFT, APP_ID_MENU, APP_ID_RIGHT, APP_ID_SETTINGS, APP_ID_TIP, LEFT_WIDTH, MenuRequest,
    RIGHT_WIDTH, RailSide, SettingsRequest, StripActions, StripOptions, TipAnchor, TipRequest,
};

/// The space between the dialog and a rail: enough to keep the two readable as
/// separate things, no more.
const GAP: i32 = 3;
const RULE_LEFT: &str = "ofolder-strip-left";
const RULE_RIGHT: &str = "ofolder-strip-right";
const RULE_MENU: &str = "ofolder-strip-menu";
const RULE_TIP: &str = "ofolder-strip-tip";
const RULE_SETTINGS: &str = "ofolder-strip-settings";
/// The space between a rail and its tooltip.
const TIP_GAP: i32 = 4;

#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub backend: String,
    pub title: String,
    /// Picker pids that were open before the request, so the new one can be told apart.
    pub known_pids: Vec<u32>,
    /// Who asked for the dialog: a portal id, `class:<name>`, or empty when unknown.
    pub app_key: String,
}

pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let (mut backend, mut title, mut known, mut app_key) = (None, None, Vec::new(), String::new());
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--backend" => backend = Some(value.clone()),
            "--title" => title = Some(value.clone()),
            "--app-key" => app_key = value.clone(),
            "--known-pids" => {
                known = value
                    .split(',')
                    .filter(|p| !p.is_empty())
                    .map(|p| p.parse().map_err(|_| format!("bad pid {p:?}")))
                    .collect::<Result<_, _>>()?;
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(Args {
        backend: backend.ok_or("--backend is required")?,
        title: title.ok_or("--title is required")?,
        known_pids: known,
        app_key,
    })
}

/// A folder name typed into the strip, as a path below the dialog's folder.
/// `a/b` means nested folders; anything that could climb out, name the root
/// or hide in a component is refused.
pub fn safe_relative(name: &str) -> Result<PathBuf, String> {
    let path = Path::new(name.trim());
    if path.as_os_str().is_empty() {
        return Err("the folder needs a name".to_owned());
    }
    // `Path::components` folds an inner `.` away, so the raw segments are checked as well.
    if name
        .trim()
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(
            "the name may not start with '/' or contain '.', '..' or control characters".to_owned(),
        );
    }
    let ok = path.components().all(|c| match c {
        Component::Normal(part) => {
            part.len() <= 255 && !part.to_string_lossy().chars().any(char::is_control)
        }
        _ => false,
    });
    if ok {
        Ok(path.to_path_buf())
    } else {
        Err("the name may not start with '/' or contain '.', '..' or control characters".to_owned())
    }
}

/// Where the favorites are kept: an override for tests, else the user's config folder.
pub fn favorites_path(
    override_path: Option<PathBuf>,
    xdg_config: Option<PathBuf>,
    home: &Path,
) -> PathBuf {
    override_path.unwrap_or_else(|| {
        xdg_config
            .unwrap_or_else(|| home.join(".config"))
            .join("ofolder/favorites.json")
    })
}

struct Actions {
    adapter: Mutex<Box<dyn Adapter>>,
    settings_path: PathBuf,
    home: PathBuf,
    compositor: Arc<dyn Compositor>,
    dock: Mutex<Dock>,
}

/// What the docking thread and the strip's menu share. The docker keeps the
/// dialog's place current, so placing a menu never waits on the adapter,
/// which a move into another folder can hold for seconds.
#[derive(Default)]
struct Dock {
    dialog: Option<Rect>,
    bounds: Option<Rect>,
    menu: Option<MenuRequest>,
    /// A menu that is typed into was just asked for: give it the keyboard
    /// once its window appears. The user clicked for it, so this is the
    /// one time the strip takes the keyboard on its own.
    focus_menu: bool,
    tip: Option<TipRequest>,
    settings: Option<SettingsRequest>,
    /// The settings window was just asked for: give it the keyboard once
    /// it appears, as for a menu that is typed into.
    focus_settings: bool,
}

/// Where the settings window opens: centred on the dialog's monitor, or on
/// the dialog when the monitor is not known.
fn settings_rect(dialog: Rect, bounds: Option<Rect>, settings: &SettingsRequest) -> Rect {
    centre_in(bounds.unwrap_or(dialog), (settings.width, settings.height))
}

/// Where a menu belongs: beside its rail, which stands where the docker puts it.
fn menu_rect(dialog: Rect, bounds: Option<Rect>, menu: &MenuRequest) -> Rect {
    let panels = place_panels(dialog, bounds, LEFT_WIDTH as i32, RIGHT_WIDTH as i32, GAP);
    let rail = match menu.side {
        RailSide::Left => panels.left,
        RailSide::Right => panels.right,
    };
    place_menu(
        rail,
        dialog,
        bounds,
        menu.anchor_y,
        (menu.width, menu.height),
        !menu.away_from_dialog,
    )
}

/// Where a tooltip belongs: beside its rail, centred on its icon.
fn tip_rect(dialog: Rect, bounds: Option<Rect>, tip: &TipRequest) -> Rect {
    let panels = place_panels(dialog, bounds, LEFT_WIDTH as i32, RIGHT_WIDTH as i32, GAP);
    let rail = match tip.side {
        RailSide::Left => panels.left,
        RailSide::Right => panels.right,
    };
    let centre = match tip.anchor {
        TipAnchor::Top(y) => rail.y + y,
        TipAnchor::Bottom(y) => rail.bottom() - y,
    };
    place_tip(
        rail,
        dialog,
        bounds,
        centre,
        (tip.width, tip.height),
        TIP_GAP,
    )
}

/// Starts `command` on its own, with no input or output attached and in its own
/// process group, so closing the dialog or the strip never takes it down.
fn spawn_detached(command: &Command) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    let mut process = std::process::Command::new(&command.program);
    process
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    if let Some(cwd) = &command.cwd {
        process.current_dir(cwd);
    }
    let mut child = process
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", command.program))?;
    // Reaped on a thread so a long-lived program never leaves a zombie behind.
    std::thread::spawn(move || {
        child.wait().ok();
    });
    Ok(())
}

/// The text of the desktop entry `id`, searched in the user's and the system's
/// application folders in the order the desktop uses.
fn read_desktop_entry(id: &str, home: &Path) -> Option<String> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let data_dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned());
    std::iter::once(data_home)
        .chain(
            data_dirs
                .split(':')
                .filter(|d| !d.is_empty())
                .map(PathBuf::from),
        )
        .find_map(|dir| std::fs::read_to_string(dir.join("applications").join(id)).ok())
}

fn in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

impl Actions {
    fn window(&self) -> Option<ofolder_core::adapter::WindowRef> {
        self.adapter.lock().ok()?.window()
    }
}

impl StripActions for Actions {
    fn current_folder(&self) -> Option<PathBuf> {
        self.adapter.lock().ok()?.read_folder()
    }

    fn go(&self, folder: &Path) -> Result<(), String> {
        let mut adapter = self
            .adapter
            .lock()
            .map_err(|_| "the dialog is busy".to_owned())?;
        goto_verified(&mut **adapter, folder, Duration::from_secs(5))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn new_folder(&self, name: &str) -> Result<PathBuf, String> {
        let relative = safe_relative(name)?;
        let mut adapter = self
            .adapter
            .lock()
            .map_err(|_| "the dialog is busy".to_owned())?;
        let base = stable_folder(&**adapter, 4, Duration::from_millis(40))
            .ok_or("the dialog's folder is not known, so nothing was created")?;
        let target = base.join(relative);
        if target.exists() {
            return Err("that folder already exists".to_owned());
        }
        std::fs::create_dir_all(&target).map_err(|e| format!("could not create it: {e}"))?;
        goto_verified(&mut **adapter, &target, Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        Ok(target)
    }

    fn open_in_file_manager(&self, folder: &Path) -> Result<(), String> {
        let loaded = settings::load(&self.settings_path, &self.home);
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join(".config"));
        let mimeapps = std::fs::read_to_string(config.join("mimeapps.list")).unwrap_or_default();
        let command = launch::file_manager_command(
            &loaded.settings,
            &mimeapps,
            &|id| read_desktop_entry(id, &self.home),
            folder,
        )?;
        spawn_detached(&command)?;
        if !loaded.settings.keep_dialog_after_file_manager
            && let Ok(mut adapter) = self.adapter.lock()
        {
            adapter.close().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn open_in_terminal(&self, folder: &Path) -> Result<(), String> {
        let loaded = settings::load(&self.settings_path, &self.home);
        let env_terminal = std::env::var("TERMINAL").ok();
        let command =
            launch::terminal_command(&loaded.settings, env_terminal.as_deref(), &in_path, folder)?;
        spawn_detached(&command)
    }

    fn place_menu(&self, menu: Option<MenuRequest>) {
        let placed = self.dock.lock().ok().and_then(|mut dock| {
            dock.menu = menu;
            dock.focus_menu = menu.is_some_and(|m| m.fresh && m.takes_keyboard)
                || (dock.focus_menu && menu.is_some());
            let menu = menu?;
            Some((menu, menu_rect(dock.dialog?, dock.bounds, &menu)))
        });
        // The rule must be in force before the window maps, or it opens tiled
        // and then jumps. A menu that is typed into takes the keyboard.
        if let Some((menu, rect)) = placed
            && menu.fresh
        {
            self.compositor
                .install_float_rule(
                    RULE_MENU,
                    APP_ID_MENU,
                    (rect.w, rect.h),
                    (rect.x, rect.y),
                    menu.takes_keyboard,
                )
                .ok();
        }
    }

    fn place_tip(&self, tip: Option<TipRequest>) {
        let placed = self.dock.lock().ok().and_then(|mut dock| {
            dock.tip = tip;
            let tip = tip?;
            Some(tip_rect(dock.dialog?, dock.bounds, &tip))
        });
        // In force before the window maps, so it never opens tiled, and it
        // never takes the keyboard.
        if let Some(rect) = placed {
            self.compositor
                .install_tip_rule(RULE_TIP, APP_ID_TIP, (rect.w, rect.h), (rect.x, rect.y))
                .ok();
        }
    }

    fn place_settings(&self, settings: Option<SettingsRequest>) {
        let placed = self.dock.lock().ok().and_then(|mut dock| {
            dock.settings = settings;
            dock.focus_settings = settings.is_some();
            let settings = settings?;
            Some(settings_rect(dock.dialog?, dock.bounds, &settings))
        });
        // In force before the window maps, so it opens where it belongs; it
        // takes the keyboard, since the user clicked the gear for it.
        if let Some(rect) = placed {
            self.compositor
                .install_float_rule(
                    RULE_SETTINGS,
                    APP_ID_SETTINGS,
                    (rect.w, rect.h),
                    (rect.x, rect.y),
                    true,
                )
                .ok();
        }
    }

    fn pointer_on_rail(&self, side: RailSide) -> Option<bool> {
        let (dialog, bounds) = self.dock.lock().ok().map(|d| (d.dialog, d.bounds))?;
        let panels = place_panels(dialog?, bounds, LEFT_WIDTH as i32, RIGHT_WIDTH as i32, GAP);
        let rail = match side {
            RailSide::Left => panels.left,
            RailSide::Right => panels.right,
        };
        Some(rail.contains(self.compositor.cursor().ok()?))
    }

    fn dialog_alive(&self) -> bool {
        let alive = self.adapter.lock().is_ok_and(|a| a.alive());
        if !alive {
            note("the dialog is gone, so the strip closes");
        }
        alive
    }
}

fn wait_for<T>(limit: Duration, mut probe: impl FnMut() -> Option<T>) -> Option<T> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if let Some(found) = probe() {
            return Some(found);
        }
        sleep(Duration::from_millis(50));
    }
    None
}

fn build_adapter(
    args: &Args,
    compositor: &Arc<dyn Compositor>,
    typist: &Arc<dyn Typist>,
    home: &Path,
) -> anyhow::Result<Box<dyn Adapter>> {
    Ok(match args.backend.as_str() {
        "flea" => {
            let pid = wait_for(Duration::from_secs(8), || {
                find_new_picker(&**compositor, &args.known_pids)
            })
            .context("no new Flea picker appeared")?;
            Box::new(FleaAdapter::new(compositor.clone(), typist.clone()).adopting(pid))
        }
        "gtk" => Box::new(
            GtkAdapter::attach(compositor.clone(), &args.title)
                .map_err(|e| anyhow::anyhow!("{e}"))?,
        ),
        "strata" => Box::new(
            StrataAdapter::attach(
                compositor.clone(),
                typist.clone(),
                &args.title,
                home.to_path_buf(),
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?,
        ),
        other => bail!("no adapter named {other:?}"),
    })
}

/// The usable area of the dialog's monitor.
fn bounds_for(compositor: &dyn Compositor, dialog: &ofolder_core::hypr::Client) -> Option<Rect> {
    compositor
        .monitors()
        .ok()
        .and_then(|ms| ms.into_iter().find(|m| m.id == dialog.monitor))
        .map(|m| m.usable_bounds())
}

/// Where the two panels belong around `dialog`, on its own monitor.
fn panels_for(compositor: &dyn Compositor, dialog: &ofolder_core::hypr::Client) -> Panels {
    let bounds = bounds_for(compositor, dialog);
    place_panels(
        dialog.rect(),
        bounds,
        LEFT_WIDTH as i32,
        RIGHT_WIDTH as i32,
        GAP,
    )
}

/// Keeps both panels around the dialog: on their own sides, packed to one
/// side at a monitor edge, following moves, resizes and workspace changes.
/// Runs until the process exits.
fn dock_forever(
    actions: Arc<Actions>,
    compositor: Arc<dyn Compositor>,
    last_seen: Arc<Mutex<Option<Rect>>>,
) {
    let me = std::process::id();
    let mut checked_focus = false;
    loop {
        sleep(Duration::from_millis(150));
        let Some(window) = actions.window() else {
            continue;
        };
        let Ok(clients) = compositor.clients() else {
            continue;
        };
        let Some(dialog) = clients.iter().find(|c| c.address == window.address) else {
            continue;
        };
        if let Ok(mut seen) = last_seen.lock() {
            *seen = Some(dialog.rect());
        }
        let bounds = bounds_for(&*compositor, dialog);
        let (menu, focus_menu, tip, settings, focus_settings) = actions
            .dock
            .lock()
            .ok()
            .map(|mut dock| {
                dock.dialog = Some(dialog.rect());
                dock.bounds = bounds;
                (
                    dock.menu,
                    dock.focus_menu,
                    dock.tip,
                    dock.settings,
                    dock.focus_settings,
                )
            })
            .unwrap_or((None, false, None, None, false));
        let left = clients
            .iter()
            .find(|c| c.class == APP_ID_LEFT && c.pid == me);
        let right = clients
            .iter()
            .find(|c| c.class == APP_ID_RIGHT && c.pid == me);
        // Once, when the panels first show: they must not hold the keyboard the dialog needs.
        if !checked_focus && left.is_some() && right.is_some() {
            checked_focus = true;
            let active = compositor.active_window().ok().flatten();
            if [left, right]
                .into_iter()
                .flatten()
                .any(|p| active.as_deref() == Some(p.address.as_str()))
            {
                compositor.focus(&dialog.address).ok();
            }
        }
        let panels = panels_for(&*compositor, dialog);
        let menu_window = clients
            .iter()
            .find(|c| c.class == APP_ID_MENU && c.pid == me);
        let menu_place = menu.map(|m| menu_rect(dialog.rect(), bounds, &m));
        // Only the menu that was asked for: one being replaced may still be on screen.
        if focus_menu
            && let (Some(window), Some(wanted)) = (menu_window, menu_place)
            && (window.at[0] - wanted.x).abs() <= 2
            && (window.at[1] - wanted.y).abs() <= 2
        {
            compositor.focus(&window.address).ok();
            if let Ok(mut dock) = actions.dock.lock() {
                dock.focus_menu = false;
            }
        }
        let settings_window = clients
            .iter()
            .find(|c| c.class == APP_ID_SETTINGS && c.pid == me);
        if focus_settings && let Some(window) = settings_window {
            compositor.focus(&window.address).ok();
            if let Ok(mut dock) = actions.dock.lock() {
                dock.focus_settings = false;
            }
        }
        // The settings window is placed once, when it opens, and only kept
        // floating and on the dialog's workspace after that, so the user may
        // move it.
        if let (Some(window), Some(settings)) = (settings_window, settings) {
            if window.workspace.id != dialog.workspace.id {
                compositor
                    .send_to_workspace(&window.address, &dialog.workspace.name)
                    .ok();
            }
            if !window.floating {
                let wanted = settings_rect(dialog.rect(), bounds, &settings);
                compositor
                    .float_window(&window.address, (wanted.w, wanted.h), (wanted.x, wanted.y))
                    .ok();
            }
        }
        let tip_window = clients
            .iter()
            .find(|c| c.class == APP_ID_TIP && c.pid == me);
        let tip_place = tip.map(|t| tip_rect(dialog.rect(), bounds, &t));
        for (panel, wanted) in [
            (left, Some(panels.left)),
            (right, Some(panels.right)),
            (menu_window, menu_place),
            (tip_window, tip_place),
        ] {
            let (Some(panel), Some(wanted)) = (panel, wanted) else {
                continue;
            };
            let (size, at) = ((wanted.w, wanted.h), (wanted.x, wanted.y));
            if panel.workspace.id != dialog.workspace.id {
                compositor
                    .send_to_workspace(&panel.address, &dialog.workspace.name)
                    .ok();
            }
            if !panel.floating {
                compositor.float_window(&panel.address, size, at).ok();
            } else if panel.rect() != wanted {
                compositor.resize_window(&panel.address, size).ok();
                compositor.move_window(&panel.address, at).ok();
            }
        }
    }
}

/// Progress notes on stderr when `OFOLDER_DEBUG` is set, to see where a strip stopped.
fn note(message: &str) {
    if std::env::var_os("OFOLDER_DEBUG").is_some() {
        eprintln!("ofolder strip: {message}");
    }
}

pub fn run(raw: &[String]) -> anyhow::Result<()> {
    let args = parse_args(raw).map_err(|e| anyhow::anyhow!("{e}"))?;
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let compositor: Arc<dyn Compositor> = Arc::new(Hyprctl);
    let typist: Arc<dyn Typist> = Arc::new(Wtype);
    let adapter = build_adapter(&args, &compositor, &typist, &home)?;
    note("adapter attached");
    let settings_path = settings::settings_path(
        std::env::var_os("OFOLDER_SETTINGS").map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        &home,
    );
    let recents_path = ofolder_core::recents::recents_path(
        std::env::var_os("OFOLDER_RECENTS").map(PathBuf::from),
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        &home,
    );
    let actions = Arc::new(Actions {
        adapter: Mutex::new(adapter),
        settings_path: settings_path.clone(),
        home: home.clone(),
        compositor: compositor.clone(),
        dock: Mutex::new(Dock::default()),
    });

    let window = wait_for(Duration::from_secs(8), || actions.window()).context(
        "the dialog's window never appeared, or more than one dialog has the same title",
    )?;
    note(&format!("dialog window {} found", window.address));
    let dialogs_path = dialog_memory::memory_path(
        std::env::var_os("OFOLDER_DIALOGS").map(PathBuf::from),
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        &home,
    );
    // Put the dialog where this app last left it, if that place is still on a monitor,
    // before the panels are positioned, so they are placed once, around its final place.
    if !args.app_key.is_empty()
        && let Some(dialog) = compositor
            .clients()
            .ok()
            .and_then(|cs| cs.into_iter().find(|c| c.address == window.address))
    {
        let bounds = compositor
            .monitors()
            .ok()
            .and_then(|ms| ms.into_iter().find(|m| m.id == dialog.monitor))
            .map(|m| m.usable_bounds());
        if let Some(wanted) = dialog_memory::load(&dialogs_path).recall(&args.app_key, bounds)
            && wanted != dialog.rect()
        {
            compositor
                .resize_window(&dialog.address, (wanted.w, wanted.h))
                .ok();
            compositor
                .move_window(&dialog.address, (wanted.x, wanted.y))
                .ok();
            sleep(Duration::from_millis(120));
        }
    }
    // Position the panels before they open, so they never appear tiled and then jump.
    if let Some(dialog) = compositor
        .clients()
        .ok()
        .and_then(|cs| cs.into_iter().find(|c| c.address == window.address))
    {
        let panels = panels_for(&*compositor, &dialog);
        for (rule, class, rect) in [
            (RULE_LEFT, APP_ID_LEFT, panels.left),
            (RULE_RIGHT, APP_ID_RIGHT, panels.right),
        ] {
            compositor
                .install_float_rule(rule, class, (rect.w, rect.h), (rect.x, rect.y), false)
                .ok();
        }
    }
    let last_seen: Arc<Mutex<Option<Rect>>> = Arc::new(Mutex::new(None));
    {
        let (actions, compositor, last_seen) =
            (actions.clone(), compositor.clone(), last_seen.clone());
        std::thread::spawn(move || dock_forever(actions, compositor, last_seen));
    }
    let favorites = favorites_path(
        std::env::var_os("OFOLDER_FAVORITES").map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        &home,
    );
    note("opening the strip window");
    let result = ofolder_strip::run_strip(
        StripOptions {
            label: args.backend.clone(),
            favorites_path: favorites,
            app_key: args.app_key.clone(),
            recents_path,
            settings_path,
            user_dirs_path: std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("user-dirs.dirs"),
            mountinfo_path: PathBuf::from("/proc/self/mountinfo"),
            user: std::env::var("USER").unwrap_or_default(),
            palette: ofolder_core::theme::load_current(
                std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
                &home,
            ),
            home,
        },
        actions,
    );
    note(&format!("strip window closed: {result:?}"));
    // Remember where the dialog was when it closed, for the next time this app opens one.
    let left_at = last_seen.lock().ok().and_then(|seen| *seen);
    if let Some(rect) = left_at {
        let mut memory = dialog_memory::load(&dialogs_path);
        if memory.remember(&args.app_key, rect)
            && let Err(e) = dialog_memory::save(&dialogs_path, &memory)
        {
            eprintln!("ofolder: {e}");
        }
    }
    compositor.remove_rule(RULE_LEFT).ok();
    compositor.remove_rule(RULE_RIGHT).ok();
    compositor.remove_rule(RULE_MENU).ok();
    compositor.remove_rule(RULE_TIP).ok();
    compositor.remove_rule(RULE_SETTINGS).ok();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(side: RailSide, anchor_y: i32) -> MenuRequest {
        MenuRequest {
            side,
            anchor_y,
            width: 300,
            height: 200,
            takes_keyboard: false,
            fresh: true,
            away_from_dialog: false,
        }
    }

    #[test]
    fn a_menu_opens_beside_its_own_rail_over_the_dialog_at_its_icon() {
        let dialog = Rect {
            x: 1000,
            y: 300,
            w: 900,
            h: 600,
        };
        let screen = Some(Rect {
            x: 0,
            y: 0,
            w: 4096,
            h: 1152,
        });
        let left = menu_rect(dialog, screen, &request(RailSide::Left, 100));
        // The left rail ends a gap before the dialog; the menu starts there.
        assert_eq!((left.x, left.y), (dialog.x - GAP, 400));
        let right = menu_rect(dialog, screen, &request(RailSide::Right, 0));
        assert_eq!(right.right(), dialog.right() + GAP);
        assert_eq!(right.y, 300);
    }

    #[test]
    fn a_tooltip_opens_beside_its_rail_over_the_dialog_centred_on_its_icon() {
        let dialog = Rect {
            x: 1000,
            y: 300,
            w: 900,
            h: 600,
        };
        let screen = Some(Rect {
            x: 0,
            y: 0,
            w: 4096,
            h: 1152,
        });
        let tip = |side, anchor| TipRequest {
            side,
            anchor,
            width: 200,
            height: 30,
        };
        let left = tip_rect(dialog, screen, &tip(RailSide::Left, TipAnchor::Top(100)));
        // The left rail ends a gap before the dialog; the tooltip a gap after it.
        assert_eq!((left.x, left.y), (dialog.x - GAP + TIP_GAP, 385));
        let right = tip_rect(dialog, screen, &tip(RailSide::Right, TipAnchor::Bottom(26)));
        assert_eq!(right.right(), dialog.right() + GAP - TIP_GAP);
        assert_eq!(right.y + right.h / 2, dialog.bottom() - 26);
    }

    fn command(program: &str, args: &[&str], cwd: Option<&Path>) -> Command {
        Command {
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            cwd: cwd.map(Path::to_path_buf),
        }
    }

    #[test]
    fn a_program_that_does_not_exist_is_reported_by_name() {
        let error = spawn_detached(&command("ofolder-no-such-program", &[], None)).unwrap_err();
        assert!(error.contains("ofolder-no-such-program"), "{error}");
    }

    #[test]
    fn a_launched_program_starts_in_the_folder_it_was_given() {
        let scratch = std::env::temp_dir().join(format!("ofolder-spawn-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let real = scratch.canonicalize().unwrap();
        // The shell is only the test's way to observe the working directory.
        spawn_detached(&command("sh", &["-c", "pwd > where.txt"], Some(&scratch))).unwrap();
        let marker = scratch.join("where.txt");
        let seen = wait_for(Duration::from_secs(5), || {
            std::fs::read_to_string(&marker)
                .ok()
                .filter(|t| !t.is_empty())
        });
        std::fs::remove_dir_all(&scratch).ok();
        let seen = seen.expect("the program ran");
        assert_eq!(Path::new(seen.trim()), real);
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_arguments_the_proxy_passes_are_understood() {
        let a = parse_args(&v(&[
            "--backend",
            "flea",
            "--title",
            "Save as",
            "--known-pids",
            "10,20",
            "--app-key",
            "class:zen",
        ]))
        .expect("parses");
        assert_eq!(
            a,
            Args {
                backend: "flea".into(),
                title: "Save as".into(),
                known_pids: vec![10, 20],
                app_key: "class:zen".into(),
            }
        );
    }

    #[test]
    fn known_pids_are_optional_and_may_be_empty() {
        let a = parse_args(&v(&[
            "--backend",
            "gtk",
            "--title",
            "t",
            "--known-pids",
            "",
        ]))
        .expect("parses");
        assert!(a.known_pids.is_empty());
    }

    #[test]
    fn missing_or_unknown_arguments_are_errors_not_guesses() {
        assert!(parse_args(&v(&["--title", "t"])).is_err());
        assert!(parse_args(&v(&["--backend", "gtk"])).is_err());
        assert!(parse_args(&v(&["--backend", "gtk", "--title", "t", "--bogus", "x"])).is_err());
        assert!(parse_args(&v(&["--backend", "gtk", "--title"])).is_err());
        assert!(
            parse_args(&v(&[
                "--backend",
                "gtk",
                "--title",
                "t",
                "--known-pids",
                "x"
            ]))
            .is_err()
        );
    }

    #[test]
    fn a_nested_folder_name_is_accepted() {
        assert_eq!(safe_relative("a/b/c").expect("ok"), PathBuf::from("a/b/c"));
        assert_eq!(
            safe_relative("  Reports  ").expect("trimmed"),
            PathBuf::from("Reports")
        );
        assert!(safe_relative("ünïcödé 日本語").is_ok());
    }

    #[test]
    fn a_folder_name_that_could_climb_out_or_name_the_root_is_refused() {
        for bad in ["", "   ", "/etc", "../x", "a/../../b", ".", "a/./b", "a\nb"] {
            assert!(safe_relative(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_component_over_the_filesystem_limit_is_refused() {
        assert!(safe_relative(&"x".repeat(256)).is_err());
        assert!(safe_relative(&"x".repeat(255)).is_ok());
    }

    #[test]
    fn favorites_live_in_the_config_folder_unless_overridden() {
        let home = Path::new("/home/j");
        assert_eq!(
            favorites_path(None, None, home),
            PathBuf::from("/home/j/.config/ofolder/favorites.json")
        );
        assert_eq!(
            favorites_path(None, Some(PathBuf::from("/xdg")), home),
            PathBuf::from("/xdg/ofolder/favorites.json")
        );
        assert_eq!(
            favorites_path(
                Some(PathBuf::from("/tmp/f.json")),
                Some(PathBuf::from("/xdg")),
                home
            ),
            PathBuf::from("/tmp/f.json")
        );
    }

    #[test]
    fn the_settings_window_opens_centred_on_the_dialogs_monitor() {
        let dialog = Rect {
            x: 1611,
            y: 289,
            w: 875,
            h: 600,
        };
        let monitor = Rect {
            x: 0,
            y: 26,
            w: 4096,
            h: 1126,
        };
        let request = SettingsRequest {
            width: 780,
            height: 560,
        };
        assert_eq!(
            settings_rect(dialog, Some(monitor), &request),
            Rect {
                x: 1658,
                y: 309,
                w: 780,
                h: 560
            }
        );
        // With no monitor known it is centred on the dialog instead.
        let on_dialog = settings_rect(dialog, None, &request);
        assert_eq!((on_dialog.x, on_dialog.w), (1658, 780));
    }
}
