//! The `ofolder` command.

use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
  portal [--backend <flea|gtk|strata>] [--no-strip]
               serve a file-chooser proxy that forwards to a backend
  pause | resume | status
               switch every dialog back to untouched, switch ofolder on again, or show the state
  strip --backend <id> --title <t>
               the strip for one open dialog (the proxy starts these)
  do <command>  run a rail command in the open dialog's strip (what a shortcut runs),
               e.g. places, find, new_folder, copy_path, settings
  unbind-all   remove every key binding ofolder made, if a crash left any
  live-check [flea|gtk|strata]
               run an adapter against the real dialog (opens a window briefly)
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
        Some("unbind-all") => unbind_all(),
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

/// `--backend <id>` and `--no-strip`, in any order. Without a backend, the one
/// after ofolder in the user's portal configuration is used.
fn portal_options(args: &[String]) -> anyhow::Result<(Option<String>, bool)> {
    let (mut backend, mut strip) = (None, true);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--backend" => {
                backend = Some(
                    it.next()
                        .ok_or_else(|| anyhow::anyhow!("--backend needs a value"))?
                        .clone(),
                )
            }
            "--no-strip" => strip = false,
            other => anyhow::bail!(
                "unknown option {other:?}; usage: ofolder portal [--backend <flea|gtk|strata>] [--no-strip]"
            ),
        }
    }
    Ok((backend, strip))
}

/// The backend named in the portal configuration after ofolder's own entry.
fn detect_backend(home: &Path) -> Option<&'static str> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let desktops = env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();
    let mut files: Vec<PathBuf> = desktops
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| config_home.join(format!("xdg-desktop-portal/{d}-portals.conf")))
        .collect();
    files.push(config_home.join("xdg-desktop-portal/portals.conf"));
    files
        .iter()
        .find_map(|f| std::fs::read_to_string(f).ok())
        .and_then(|text| portal::backend_after(&text, "ofolder"))
}

fn portal(args: &[String]) -> anyhow::Result<()> {
    let home =
        PathBuf::from(env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?);
    let (chosen, strip) = portal_options(args)?;
    let id: String = match chosen {
        Some(id) => id,
        None => detect_backend(&home)
            .ok_or_else(|| anyhow::anyhow!("no backend given and none follows ofolder in the portal configuration; use --backend"))?
            .to_owned(),
    };
    let id = id.as_str();
    let backend = dbus_name_for(id).ok_or_else(|| anyhow::anyhow!("no adapter named {id:?}"))?;
    let hooks: Arc<dyn ofolder_portal::hooks::Hooks> = if strip {
        let home =
            PathBuf::from(env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?);
        Arc::new(hooks::StripHooks::new(
            own_exe()?,
            id.to_owned(),
            Arc::new(ofolder_adapters::hyprctl::Hyprctl),
            ofolder_core::settings::settings_path(
                env::var_os("OFOLDER_SETTINGS").map(PathBuf::from),
                env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
                &home,
            ),
            ofolder_core::recents::recents_path(
                env::var_os("OFOLDER_RECENTS").map(PathBuf::from),
                env::var_os("XDG_DATA_HOME").map(PathBuf::from),
                &home,
            ),
            home,
        ))
    } else {
        Arc::new(PassThrough)
    };
    ofolder_portal::block_on(async {
        let _connection = ofolder_portal::serve_session(BUS_NAME, backend, hooks).await?;
        println!(
            "ofolder portal: {BUS_NAME} forwards to {backend}{}",
            if strip {
                " with a strip per dialog"
            } else {
                ""
            }
        );
        std::future::pending::<()>().await;
        Ok(())
    })
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
