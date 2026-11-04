//! The `ofolder` command.

use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod doctor;
mod hooks;
mod keys;
mod live;
mod pause;
mod recording;
mod strip_cmd;

use ofolder_core::adapter::known_capabilities;
use ofolder_core::portal::{self, FILE_CHOOSER, dbus_name_for};
use ofolder_portal::hooks::PassThrough;

const USAGE: &str = "usage: ofolder <command>

commands:
  adapters     list the installed file-chooser backends and which ofolder can decorate
  strip-demo   open a sample strip window to see how it draws
  portal [--backend <flea|gtk|strata>] [--no-strip] [--check]
               serve a file-chooser proxy that forwards to a backend; --check only
               verifies that it could start, and claims nothing (exit 0 when it could,
               3 when not)
  pause | resume | status
               switch every dialog back to untouched, switch ofolder on again, or show the state
  strip --backend <id> --title <t>
               the strip for one open dialog (the proxy starts these)
  do <command>  run a rail command in the open dialog's strip (what a shortcut runs),
               e.g. places, find, new_folder, copy_path, settings
  forget       empty the recent folders and files, and list what went (favorites stay)
  unbind-all   remove every key binding ofolder made, if a crash left any
  live-check [flea|gtk|strata]
               run an adapter against the real dialog (opens a window briefly)
  doctor       check how ofolder is set up and say what is wrong and how to fix it
  repair [--undo] [--no-restart] [--watch on|off]
               put it right: the registration files if missing, and ofolder first in the
               file-chooser chain (a copy is kept; --undo goes back). --watch on installs
               a guard that does this by itself, and keeps dialogs working if ofolder is
               deleted; --watch off removes it
  version      print the version";

fn main() -> anyhow::Result<()> {
    match env::args().nth(1).as_deref() {
        Some("adapters") => {
            adapters();
            Ok(())
        }
        Some("strip-demo") => ofolder_strip::run_demo(),
        Some("portal") => portal(&env::args().skip(2).collect::<Vec<_>>()),
        Some("pause") => set_paused(true),
        Some("resume") => set_paused(false),
        Some("status") => status(),
        Some("strip") => strip_cmd::run(&env::args().skip(2).collect::<Vec<_>>()),
        Some("do") => do_command(env::args().nth(2).as_deref()),
        Some("doctor") => doctor::doctor(),
        Some("repair") => doctor::repair(&env::args().skip(2).collect::<Vec<_>>()),
        Some("unbind-all") => unbind_all(),
        Some("forget") => forget(),
        Some("live-check") => live::run(env::args().nth(2).as_deref().unwrap_or("flea")),
        Some("version" | "--version") => {
            println!("ofolder {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            println!("{USAGE}");
            Ok(())
        }
    }
}

/// The directories the portal frontend reads `*.portal` files from, user
/// first: the same order it uses, so the same backend wins.
fn portal_dirs() -> Vec<PathBuf> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let data_home = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".local/share")));
    let data_dirs =
        env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned());
    data_home
        .into_iter()
        .chain(data_dirs.split(':').map(PathBuf::from))
        .map(|d| d.join("xdg-desktop-portal/portals"))
        .collect()
}

fn adapters() {
    let backends: Vec<_> = portal::discover(&portal_dirs())
        .into_iter()
        .filter(portal::PortalFile::provides_file_chooser)
        .collect();
    if backends.is_empty() {
        println!("no {FILE_CHOOSER} backends found");
        return;
    }
    for backend in backends {
        match backend.adapter_id() {
            Some(id) => {
                let caps = known_capabilities(id);
                println!(
                    "{:<46} adapter {id:<7} read_folder={} keyless_goto={} native_new_folder={} opens_tiled={}",
                    backend.dbus_name,
                    caps.read_folder,
                    caps.goto_without_keys,
                    caps.native_new_folder,
                    caps.opens_tiled
                );
            }
            None => println!("{:<46} no adapter: left undecorated", backend.dbus_name),
        }
    }
}

/// The bus name this backend answers on. It is not in anyone's portal
/// configuration until the user adds it, so the frontend never calls it.
const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.ofolder";

/// The options of `portal`.
struct PortalArgs {
    backend: Option<String>,
    strip: bool,
    check: bool,
}

/// The exit status of a failed `portal --check`. The guard treats only this status as "this build
/// is broken". An older build does not know `--check` and exits 1, so that status must not make the
/// guard set the portal file aside.
const CHECK_FAILED: i32 = 3;

/// The exit status when the check could not be made at all: the session bus was not there to
/// ask. Nothing is known to be wrong with the build, so the guard leaves it alone.
const CHECK_UNKNOWN: i32 = 4;

/// Why a check did not pass: the build or its setup is broken, or the check itself could not be made.
#[derive(Debug, PartialEq, Eq)]
enum CheckFailure {
    Broken(String),
    CannotTell(String),
}

impl std::fmt::Display for CheckFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Broken(why) | Self::CannotTell(why) => f.write_str(why),
        }
    }
}

/// `--backend <id>`, `--no-strip` and `--check`, in any order. Without a backend, the one after
/// ofolder in the user's portal configuration is used.
fn portal_options(args: &[String]) -> anyhow::Result<PortalArgs> {
    let mut parsed = PortalArgs {
        backend: None,
        strip: true,
        check: false,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--backend" => {
                parsed.backend = Some(
                    it.next()
                        .ok_or_else(|| anyhow::anyhow!("--backend needs a value"))?
                        .clone(),
                )
            }
            "--no-strip" => parsed.strip = false,
            "--check" => parsed.check = true,
            other => anyhow::bail!(
                "unknown option {other:?}; usage: ofolder portal [--backend <flea|gtk|strata>] [--no-strip] [--check]"
            ),
        }
    }
    Ok(parsed)
}

/// The backend named in the portal configuration after ofolder's own entry.
/// The portal configuration the frontend would read for this desktop, and its text.
fn portal_conf(home: &Path) -> Option<(PathBuf, String)> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    portal_conf_in(&config_home, &current_desktops())
}

/// The desktop name or names the portal frontend goes by (`hyprland`), lower case. Taken from this
/// process's environment; a service started without one (the guard, under systemd) asks the
/// running portal frontend, whose environment is the one that decides which file it reads.
fn current_desktops() -> String {
    if let Ok(own) = env::var("XDG_CURRENT_DESKTOP")
        && !own.is_empty()
    {
        return own.to_lowercase();
    }
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let pid = entry.file_name();
        let Some(pid) = pid
            .to_str()
            .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        let is_frontend = std::fs::read(format!("/proc/{pid}/cmdline"))
            .ok()
            .and_then(|c| c.split(|b| *b == 0).next().map(<[u8]>::to_vec))
            .is_some_and(|argv0| argv0.ends_with(b"/xdg-desktop-portal"));
        if !is_frontend {
            continue;
        }
        if let Ok(environ) = std::fs::read(format!("/proc/{pid}/environ"))
            && let Some(value) = environ
                .split(|b| *b == 0)
                .find_map(|kv| kv.strip_prefix(b"XDG_CURRENT_DESKTOP="))
        {
            return String::from_utf8_lossy(value).to_lowercase();
        }
    }
    String::new()
}

/// The portal configuration the frontend would read for `desktops`, and its text. With the desktop
/// unknown, the one `<desktop>-portals.conf` there is wins over `portals.conf`, as it would for the
/// frontend; guessing wrong would have `repair` change a file the frontend never reads.
fn portal_conf_in(config_home: &Path, desktops: &str) -> Option<(PathBuf, String)> {
    let dir = config_home.join("xdg-desktop-portal");
    let mut files: Vec<PathBuf> = desktops
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| dir.join(format!("{d}-portals.conf")))
        .collect();
    if desktops.is_empty() {
        let mut specific: Vec<PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("-portals.conf"))
            })
            .collect();
        if specific.len() == 1 {
            files.append(&mut specific);
        }
    }
    files.push(dir.join("portals.conf"));
    files
        .into_iter()
        .find_map(|f| std::fs::read_to_string(&f).ok().map(|text| (f, text)))
}

/// The backend ofolder forwards to: the first one after it in the chain that is installed, and
/// failing that any installed one it knows. A chain that names a backend since uninstalled must
/// not leave the dialog forwarded to nothing: that would stop every file dialog.
fn detect_backend(home: &Path) -> Option<&'static str> {
    let installed: Vec<&'static str> = portal::discover(&portal_dirs())
        .iter()
        .filter(|p| p.provides_file_chooser())
        .filter_map(portal::PortalFile::adapter_id)
        .collect();
    portal_conf(home)
        .and_then(|(_, text)| {
            portal::backend_after_installed(&text, "ofolder", &|id| installed.contains(&id))
        })
        .or_else(|| {
            ["gtk", "flea", "strata"]
                .into_iter()
                .find(|id| installed.contains(id))
        })
}

/// What `status` says about the file-chooser chain: dialogs only reach ofolder
/// when it is first, and another program (Flea's `--picker`, for one) can
/// rewrite the line.
fn chain_report(conf: Option<(PathBuf, String)>) -> Vec<String> {
    let Some((path, text)) = conf else {
        return vec!["file chooser chain: no portal configuration found, so the desktop's default applies and dialogs bypass ofolder".to_owned()];
    };
    let Some(chain) = portal::file_chooser_chain(&text) else {
        return vec![format!(
            "warning: {} sets no file chooser chain, so dialogs bypass ofolder",
            path.display()
        )];
    };
    let shown = chain.join(";");
    match chain.iter().position(|e| e == "ofolder") {
        Some(0) => vec![format!("file chooser chain: {shown} (ofolder first)")],
        Some(_) => vec![format!(
            "warning: ofolder is not first in the file chooser chain ({shown} in {}): dialogs open without it",
            path.display()
        )],
        None => vec![format!(
            "warning: ofolder is not in the file chooser chain ({shown} in {}): dialogs open without it. \
             Put ofolder first, as in `ofolder;{shown}`",
            path.display()
        )],
    }
}

/// What `portal` serves, decided from the configuration before anything is claimed on the bus.
struct Plan {
    id: String,
    bus_name: &'static str,
    hooks: Arc<dyn ofolder_portal::hooks::Hooks>,
}

/// The backend and the hooks `portal` starts with. The server and `--check` both build them here,
/// so the check cannot pass for a setup the server would refuse. `runtime` is only asked for when
/// there is a strip, and it must not create anything: the check runs beside the real portal.
fn plan(
    chosen: Option<String>,
    strip: bool,
    home: &Path,
    runtime: impl FnOnce() -> Option<PathBuf>,
) -> anyhow::Result<Plan> {
    let id = match chosen {
        Some(id) => id,
        None => detect_backend(home)
            .ok_or_else(|| anyhow::anyhow!("no backend given and none follows ofolder in the portal configuration; use --backend"))?
            .to_owned(),
    };
    let bus_name = dbus_name_for(&id).ok_or_else(|| anyhow::anyhow!("no adapter named {id:?}"))?;
    let hooks: Arc<dyn ofolder_portal::hooks::Hooks> = if strip {
        Arc::new(hooks::StripHooks::new(
            own_exe()?,
            id.clone(),
            Arc::new(ofolder_adapters::hyprctl::Hyprctl),
            ofolder_core::settings::settings_path(
                env::var_os("OFOLDER_SETTINGS").map(PathBuf::from),
                env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
                home,
            ),
            ofolder_core::recents::recents_path(
                env::var_os("OFOLDER_RECENTS").map(PathBuf::from),
                env::var_os("XDG_DATA_HOME").map(PathBuf::from),
                home,
            ),
            home.to_path_buf(),
            runtime(),
        ))
    } else {
        Arc::new(PassThrough)
    };
    Ok(Plan {
        id,
        bus_name,
        hooks,
    })
}

fn portal(args: &[String]) -> anyhow::Result<()> {
    let parsed = portal_options(args)?;
    if parsed.check {
        return portal_check(parsed);
    }
    let home =
        PathBuf::from(env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?);
    let Plan {
        bus_name: backend,
        hooks,
        ..
    } = plan(parsed.backend, parsed.strip, &home, || {
        keys::runtime_dir().ok()
    })?;
    // The guard is on unless the maintainer turned it off: a first start puts it in place. Not waited on.
    std::thread::spawn(|| {
        if let Err(e) = doctor::reconcile_guard() {
            eprintln!("ofolder: the guard could not be put in order: {e}");
        }
    });
    ofolder_portal::block_on(async {
        let _connection = ofolder_portal::serve_session(BUS_NAME, backend, hooks).await?;
        println!(
            "ofolder portal: {BUS_NAME} forwards to {backend}{}",
            if parsed.strip {
                " with a strip per dialog"
            } else {
                ""
            }
        );
        std::future::pending::<()>().await;
        Ok(())
    })
}

/// `portal --check`: prints one line and exits 0 when the portal could start, and exits with
/// [`CHECK_FAILED`] and says why when it could not. It never claims the bus name, so it is safe while
/// the real portal runs.
fn portal_check(parsed: PortalArgs) -> anyhow::Result<()> {
    let verdict = match env::var_os("HOME") {
        None => Err(CheckFailure::Broken("HOME is not set".to_owned())),
        Some(home) => {
            // Read, never created: the real portal owns this folder.
            let runtime =
                env::var_os("XDG_RUNTIME_DIR").map(|base| PathBuf::from(base).join("ofolder"));
            check_portal(
                parsed.backend,
                parsed.strip,
                Path::new(&home),
                runtime,
                &session_bus_names,
            )
        }
    };
    match verdict {
        Ok(line) => {
            println!("{line}");
            Ok(())
        }
        Err(why) => {
            eprintln!("ofolder portal --check failed: {why}");
            std::process::exit(match why {
                CheckFailure::Broken(_) => CHECK_FAILED,
                CheckFailure::CannotTell(_) => CHECK_UNKNOWN,
            });
        }
    }
}

/// The names on the session bus, or why they cannot be read.
fn session_bus_names() -> Result<Vec<String>, String> {
    ofolder_portal::block_on(ofolder_portal::session_bus_names())
        .map_err(|e| format!("session bus unavailable: {e}"))
}

/// Everything `portal` does before it claims its bus name, then whether the session bus knows the
/// backend's name. `session_names` only reads the bus.
fn check_portal(
    chosen: Option<String>,
    strip: bool,
    home: &Path,
    runtime: Option<PathBuf>,
    session_names: &dyn Fn() -> Result<Vec<String>, String>,
) -> Result<String, CheckFailure> {
    own_exe()
        .map_err(|e| CheckFailure::Broken(format!("the program cannot find its own path: {e}")))?;
    let plan = plan(chosen, strip, home, || runtime)
        .map_err(|e| CheckFailure::Broken(format!("{e:#}")))?;
    // A bus that cannot be asked is not a verdict on the build: it may be restarting.
    let names = session_names().map_err(CheckFailure::CannotTell)?;
    if !names.iter().any(|name| name == plan.bus_name) {
        return Err(CheckFailure::Broken(format!(
            "the session bus does not know {} (the name of {}), so the portal frontend could not reach it",
            plan.bus_name, plan.id
        )));
    }
    Ok(format!(
        "ok: ofolder would forward to {} ({}); the session bus knows that name, and nothing was claimed",
        plan.bus_name, plan.id
    ))
}

/// Empties the recent folders and files. What went is printed first, so the
/// owner sees it; favorites are a separate list and are not touched.
fn forget() -> anyhow::Result<()> {
    let home =
        PathBuf::from(env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?);
    let path = ofolder_core::recents::recents_path(
        env::var_os("OFOLDER_RECENTS").map(PathBuf::from),
        env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        &home,
    );
    let gone = ofolder_core::recents::forget_all(&path)?;
    for folder in &gone.folders {
        println!("folder  {folder}");
    }
    for file in &gone.files {
        println!("file    {file}");
    }
    if gone.unreadable {
        println!("the recents file could not be read, so it was replaced with an empty one");
    }
    println!("{} in {}", gone.summary(), path.display());
    Ok(())
}

fn settings_file() -> anyhow::Result<(PathBuf, PathBuf)> {
    let home =
        PathBuf::from(env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?);
    let path = ofolder_core::settings::settings_path(
        env::var_os("OFOLDER_SETTINGS").map(PathBuf::from),
        env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        &home,
    );
    Ok((path, home))
}

fn set_paused(paused: bool) -> anyhow::Result<()> {
    let (path, _) = settings_file()?;
    pause::set(&path, paused)?;
    println!(
        "ofolder is {} ({})",
        if paused {
            "paused: dialogs pass through untouched"
        } else {
            "on"
        },
        path.display()
    );
    Ok(())
}

fn status() -> anyhow::Result<()> {
    let (path, home) = settings_file()?;
    let loaded = ofolder_core::settings::load(&path, &home);
    println!("settings: {}", path.display());
    println!(
        "ofolder is {}",
        if loaded.settings.paused {
            "paused"
        } else {
            "on"
        }
    );
    for warning in loaded.warnings {
        println!("warning: {warning}");
    }
    for line in chain_report(portal_conf(&home)) {
        println!("{line}");
    }
    Ok(())
}

/// `ofolder do <command>`: hands the command to the newest strip. Paused,
/// it does nothing; with no strip running it cleans up after any that died
/// and fails quietly, so a shortcut pressed with no dialog open is harmless.
fn do_command(id: Option<&str>) -> anyhow::Result<()> {
    let id = id.ok_or_else(|| anyhow::anyhow!("usage: ofolder do <command>"))?;
    let def =
        ofolder_core::shortcuts::command(id).ok_or_else(|| anyhow::anyhow!("no command {id:?}"))?;
    let (path, home) = settings_file()?;
    if ofolder_core::settings::load(&path, &home).settings.paused {
        return Ok(());
    }
    let dir = keys::runtime_dir()?;
    if keys::send(&dir, def.id, &keys::ofolder_alive)? {
        return Ok(());
    }
    keys::clean_stale(
        &ofolder_adapters::hyprctl::Hyprctl,
        &dir,
        &keys::ofolder_alive,
    );
    std::process::exit(1);
}

fn unbind_all() -> anyhow::Result<()> {
    let dir = keys::runtime_dir()?;
    let removed = keys::clear_ours(&ofolder_adapters::hyprctl::Hyprctl, &dir);
    println!("removed {removed} key binding(s) ofolder had made");
    Ok(())
}

/// The path to start this program again with. When the binary was replaced
/// while the portal was running, Linux reports the old file as
/// "<path> (deleted)", which cannot be started: the same path now holds the
/// new build, so use that.
pub(crate) fn own_exe() -> std::io::Result<PathBuf> {
    Ok(without_deleted_suffix(env::current_exe()?))
}

fn without_deleted_suffix(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_suffix(" (deleted)")) {
        Some(live) => PathBuf::from(live),
        None => path,
    }
}

#[cfg(test)]
mod own_exe_tests {
    use super::*;

    #[test]
    fn a_replaced_binary_is_started_from_its_path_without_the_deleted_marker() {
        assert_eq!(
            without_deleted_suffix(PathBuf::from("/home/u/.local/bin/ofolder (deleted)")),
            PathBuf::from("/home/u/.local/bin/ofolder")
        );
        assert_eq!(
            without_deleted_suffix(PathBuf::from("/home/u/.local/bin/ofolder")),
            PathBuf::from("/home/u/.local/bin/ofolder")
        );
    }
}

#[cfg(test)]
mod tests {
    /// A scratch folder that removes itself.
    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }
    fn config(label: &str, files: &[&str]) -> Dir {
        let dir = std::env::temp_dir().join(format!("ofolder-conf-{}-{label}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("xdg-desktop-portal")).unwrap();
        for f in files {
            std::fs::write(dir.join("xdg-desktop-portal").join(f), format!("# {f}\n")).unwrap();
        }
        Dir(dir)
    }

    #[test]
    fn the_configuration_is_found_by_desktop_name() {
        let d = config("by-name", &["hyprland-portals.conf", "portals.conf"]);
        let (path, _) = portal_conf_in(&d.0, "hyprland").unwrap();
        assert!(path.ends_with("hyprland-portals.conf"));
    }

    #[test]
    fn with_no_desktop_name_the_one_desktop_file_beats_portals_conf() {
        let d = config("one-specific", &["hyprland-portals.conf", "portals.conf"]);
        let (path, _) = portal_conf_in(&d.0, "").unwrap();
        assert!(
            path.ends_with("hyprland-portals.conf"),
            "the frontend would read this one, so this is the one to repair: {path:?}"
        );
    }

    #[test]
    fn with_no_desktop_name_and_several_desktop_files_it_does_not_guess_between_them() {
        let d = config(
            "two-specific",
            &["hyprland-portals.conf", "sway-portals.conf"],
        );
        assert!(portal_conf_in(&d.0, "").is_none());
        let d = config(
            "two-and-plain",
            &["hyprland-portals.conf", "sway-portals.conf", "portals.conf"],
        );
        let (path, _) = portal_conf_in(&d.0, "").unwrap();
        assert_eq!(path.file_name().unwrap(), "portals.conf");
    }

    use super::*;

    fn conf(line: &str) -> Option<(PathBuf, String)> {
        Some((
            PathBuf::from("/c/hyprland-portals.conf"),
            format!("[preferred]\n{FILE_CHOOSER}={line}\n"),
        ))
    }

    #[test]
    fn status_is_quiet_when_ofolder_leads_the_chain() {
        assert_eq!(
            chain_report(conf("ofolder;flea;gtk")),
            ["file chooser chain: ofolder;flea;gtk (ofolder first)"]
        );
    }

    #[test]
    fn status_warns_when_another_program_took_the_line() {
        let report = chain_report(conf("flea;gtk"));
        assert_eq!(report.len(), 1);
        assert!(report[0].starts_with("warning: ofolder is not in the file chooser chain"));
        assert!(report[0].contains("ofolder;flea;gtk"), "{}", report[0]);
    }

    #[test]
    fn status_warns_when_ofolder_is_not_first() {
        let report = chain_report(conf("flea;ofolder;gtk"));
        assert!(
            report[0].starts_with("warning: ofolder is not first"),
            "{}",
            report[0]
        );
    }

    #[test]
    fn status_warns_when_there_is_no_chain_or_no_file() {
        let none = chain_report(Some((PathBuf::from("/c/p.conf"), "[preferred]\n".into())));
        assert!(none[0].starts_with("warning:") && none[0].contains("bypass"));
        assert!(chain_report(None)[0].contains("bypass ofolder"));
    }
}

#[cfg(test)]
mod portal_check_tests {
    use super::*;

    const GTK: &str = "org.freedesktop.impl.portal.desktop.gtk";

    /// A folder under the system temporary folder, removed on drop. It stands in for `HOME`.
    struct Home(PathBuf);

    impl Home {
        fn new(label: &str) -> Self {
            let dir = env::temp_dir().join(format!("ofolder-check-{label}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("the temporary folder can be created");
            Self(dir)
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn names(known: &[&str]) -> impl Fn() -> Result<Vec<String>, String> + use<> {
        let known: Vec<String> = known.iter().map(|s| (*s).to_owned()).collect();
        move || Ok(known.clone())
    }

    #[test]
    fn a_check_passes_when_the_session_bus_knows_the_backend() {
        let home = Home::new("known");
        let line = check_portal(
            Some("gtk".to_owned()),
            false,
            &home.0,
            None,
            &names(&[":1.4", GTK]),
        )
        .expect("a known backend passes the check");
        assert!(line.starts_with("ok:"), "{line}");
        assert!(line.contains(GTK), "{line}");
    }

    #[test]
    fn a_check_fails_and_names_the_backend_when_the_bus_does_not_know_it() {
        let home = Home::new("unknown");
        let why = check_portal(
            Some("gtk".to_owned()),
            false,
            &home.0,
            None,
            &names(&["org.freedesktop.impl.portal.desktop.flea"]),
        )
        .expect_err("an absent backend fails the check");
        assert!(matches!(why, CheckFailure::Broken(_)), "{why}");
        let why = why.to_string();
        assert!(why.contains(GTK), "{why}");
        assert!(why.contains("does not know"), "{why}");
    }

    #[test]
    fn a_check_says_plainly_when_there_is_no_session_bus() {
        let home = Home::new("no-bus");
        let why = check_portal(Some("gtk".to_owned()), false, &home.0, None, &|| {
            Err("session bus unavailable: no address".to_owned())
        })
        .expect_err("no bus means the check cannot be made");
        assert_eq!(
            why,
            CheckFailure::CannotTell("session bus unavailable: no address".to_owned()),
            "a bus that cannot be asked is not a verdict on the build"
        );
    }

    #[test]
    fn a_check_fails_for_an_adapter_that_does_not_exist() {
        let home = Home::new("no-adapter");
        let why = check_portal(Some("nope".to_owned()), true, &home.0, None, &names(&[GTK]))
            .expect_err("an unknown adapter fails the check")
            .to_string();
        assert!(why.contains("no adapter named"), "{why}");
    }

    #[test]
    fn a_check_with_the_strip_builds_the_same_hooks_and_passes() {
        let home = Home::new("strip");
        let runtime = home.0.join("runtime");
        check_portal(
            Some("gtk".to_owned()),
            true,
            &home.0,
            Some(runtime.clone()),
            &names(&[GTK]),
        )
        .expect("the strip's hooks can be built");
        assert!(
            !runtime.exists(),
            "the check must not create the runtime folder"
        );
    }

    #[test]
    fn the_check_flag_is_read_and_a_bad_option_is_still_a_usage_error() {
        let parsed = portal_options(&["--check".to_owned(), "--no-strip".to_owned()])
            .expect("the flags parse");
        assert!(parsed.check);
        assert!(!parsed.strip);
        assert!(portal_options(&["--bogus".to_owned()]).is_err());
        assert!(!portal_options(&[]).expect("no options").check);
    }
}
