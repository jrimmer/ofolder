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
use ofolder_core::geometry::place_strip;
use ofolder_core::system::{Compositor, Typist};
use ofolder_strip::{APP_ID, StripActions, StripOptions, WIDTH};

const GAP: i32 = 6;
const DOCK_RULE: &str = "ofolder-strip";

#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub backend: String,
    pub title: String,
    /// Picker pids that were open before the request, so the new one can be told apart.
    pub known_pids: Vec<u32>,
}

pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let (mut backend, mut title, mut known) = (None, None, Vec::new());
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--backend" => backend = Some(value.clone()),
            "--title" => title = Some(value.clone()),
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

/// Keeps the strip beside the dialog: flips at monitor edges, follows moves,
/// resizes and workspace changes. Runs until the process exits.
fn dock_forever(actions: Arc<Actions>, compositor: Arc<dyn Compositor>) {
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
        let (Some(dialog), Some(strip)) = (
            clients.iter().find(|c| c.address == window.address),
            clients.iter().find(|c| c.class == APP_ID && c.pid == me),
        ) else {
            continue;
        };
        let bounds = compositor
            .monitors()
            .ok()
            .and_then(|ms| ms.into_iter().find(|m| m.id == dialog.monitor))
            .map(|m| m.usable_bounds());
        // Once, when the strip first shows: it must not hold the keyboard the dialog needs.
        if !checked_focus {
            checked_focus = true;
            if compositor.active_window().ok().flatten().as_deref() == Some(strip.address.as_str())
            {
                compositor.focus(&dialog.address).ok();
            }
        }
        let place = place_strip(dialog.rect(), bounds, WIDTH as i32, GAP);
        let (size, at) = ((place.rect.w, place.rect.h), (place.rect.x, place.rect.y));
        if strip.workspace.id != dialog.workspace.id {
            compositor
                .send_to_workspace(&strip.address, &dialog.workspace.name)
                .ok();
        }
        if !strip.floating {
            compositor.float_window(&strip.address, size, at).ok();
        } else if strip.rect() != place.rect {
            compositor.resize_window(&strip.address, size).ok();
            compositor.move_window(&strip.address, at).ok();
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
    let actions = Arc::new(Actions {
        adapter: Mutex::new(adapter),
    });

    let window = wait_for(Duration::from_secs(8), || actions.window()).context(
        "the dialog's window never appeared, or more than one dialog has the same title",
    )?;
    note(&format!("dialog window {} found", window.address));
    // Position the strip before it opens, so it never appears tiled and then jumps.
    if let Some(dialog) = compositor
        .clients()
        .ok()
        .and_then(|cs| cs.into_iter().find(|c| c.address == window.address))
    {
        let bounds = compositor
            .monitors()
            .ok()
            .and_then(|ms| ms.into_iter().find(|m| m.id == dialog.monitor))
            .map(|m| m.usable_bounds());
        let place = place_strip(dialog.rect(), bounds, WIDTH as i32, GAP);
        compositor
            .install_float_rule(
                DOCK_RULE,
                APP_ID,
                (place.rect.w, place.rect.h),
                (place.rect.x, place.rect.y),
                false,
            )
            .ok();
    }
    {
        let (actions, compositor) = (actions.clone(), compositor.clone());
        std::thread::spawn(move || dock_forever(actions, compositor));
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
            home,
        },
        actions,
    );
    note(&format!("strip window closed: {result:?}"));
    compositor.remove_rule(DOCK_RULE).ok();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
        ]))
        .expect("parses");
        assert_eq!(
            a,
            Args {
                backend: "flea".into(),
                title: "Save as".into(),
                known_pids: vec![10, 20]
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
}
