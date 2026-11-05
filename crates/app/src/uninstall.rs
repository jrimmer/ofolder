//! `ofolder uninstall`: undo what `repair` and the first dialog set up, and leave the program files
//! to `install.sh --uninstall`.
//!
//! Undoing is done in an order that never leaves a dialog worse off at any step: the guard goes
//! first (otherwise it would put ofolder straight back in the chain), then the chain, then the two
//! registration files, then the state ofolder made, and last the portal frontend is restarted.
//! Settings, favorites and recents are the user's own data and stay unless `--purge` is given.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use ofolder_core::portal;

use crate::doctor::{self, Dirs};

/// How long after a backup was taken the chain file may still have been last written by that
/// repair. The backup is stamped before the write, in whole seconds.
const WRITE_SLACK_SECS: u64 = 5;

/// What to do with the file-chooser chain file.
#[derive(Debug, PartialEq, Eq)]
enum ChainChange {
    /// ofolder is not in the chain: nothing to undo.
    None,
    /// Put the file back exactly as the last repair found it.
    Restore(String),
    /// Take ofolder's entry out and keep everything else, because the file was written by something
    /// else since the last repair and a restore would undo that too.
    Edit(String),
    /// What is left says nothing at all, so the file goes.
    Delete,
}

/// Decides how to undo the chain. `backup` is the newest copy a repair kept of this very file with
/// the second it was taken, and `written` is when the file was last written, in seconds. The backup
/// is used only when nothing but that repair has written the file since, and when it does not
/// itself name ofolder.
fn chain_change(current: &str, backup: Option<(&str, u64)>, written: u64) -> ChainChange {
    let Some(edited) = portal::without_entry(current, "ofolder") else {
        return ChainChange::None;
    };
    if let Some((text, stamp)) = backup
        && written <= stamp + WRITE_SLACK_SECS
        && !portal::file_chooser_chain(text).is_some_and(|c| c.iter().any(|e| e == "ofolder"))
    {
        return ChainChange::Restore(text.to_owned());
    }
    if portal::says_nothing(&edited) {
        ChainChange::Delete
    } else {
        ChainChange::Edit(edited)
    }
}

/// The newest backup a repair kept of the file called `name`, with the second it was taken.
fn newest_backup(d: &Dirs, name: &str) -> Option<(String, u64)> {
    let mut newest: Option<(u64, PathBuf)> = None;
    for entry in fs::read_dir(d.backups()).into_iter().flatten().flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if let Some((original, stamp)) = file.rsplit_once('.')
            && original == name
            && let Ok(stamp) = stamp.parse::<u64>()
            && newest.as_ref().is_none_or(|(s, _)| stamp > *s)
        {
            newest = Some((stamp, entry.path()));
        }
    }
    let (stamp, path) = newest?;
    Some((fs::read_to_string(path).ok()?, stamp))
}

fn modified_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(u64::MAX, |d| d.as_secs())
}

/// Removes a file, and says so; a file that is not there is not worth a word.
fn remove(path: &Path, said: &mut Vec<String>) -> anyhow::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => said.push(format!("removed {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("cannot remove {}", path.display())),
    }
    Ok(())
}

/// The path with `.lock` added: the lock file a writer keeps beside a file several dialogs write.
fn lock_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// The user's own data: the files `--purge` adds to what is removed.
fn own_data(d: &Dirs) -> anyhow::Result<Vec<PathBuf>> {
    let env_path = |var: &str| std::env::var_os(var).map(PathBuf::from);
    let (settings, _) = crate::settings_file()?;
    let favorites = crate::strip_cmd::favorites_path(
        env_path("OFOLDER_FAVORITES"),
        env_path("XDG_CONFIG_HOME"),
        &d.home,
    );
    let recents = ofolder_core::recents::recents_path(
        env_path("OFOLDER_RECENTS"),
        env_path("XDG_DATA_HOME"),
        &d.home,
    );
    Ok([settings, favorites, recents]
        .into_iter()
        .flat_map(|p| [lock_of(&p), p])
        .collect())
}

pub fn run(args: &[String]) -> anyhow::Result<()> {
    let mut purge = false;
    let mut keep = false;
    let mut restart = true;
    for arg in args {
        match arg.as_str() {
            "--purge" => purge = true,
            "--keep-settings" => keep = true,
            "--no-restart" => restart = false,
            other => anyhow::bail!(
                "usage: ofolder uninstall [--keep-settings | --purge] [--no-restart] (not {other})"
            ),
        }
    }
    if purge && keep {
        anyhow::bail!("--purge and --keep-settings contradict each other: choose one");
    }
    let d = doctor::dirs()?;
    let data = own_data(&d)?;
    let mut said: Vec<String> = Vec::new();
    let mut changed = false;

    if purge {
        println!("--purge also removes your own data:");
        for p in data.iter().filter(|p| p.is_file()) {
            println!("  {}", p.display());
        }
    } else {
        println!("Your settings, favorites and recents are kept (--purge removes them).");
    }

    // The guard first: while it is on, it puts ofolder back in the chain.
    let guard_was_on = d.config.join("systemd/user/ofolder-guard.path").is_file()
        || d.data.join("ofolder/ofolder-guard.sh").is_file();
    if guard_was_on {
        said.extend(doctor::apply_guard(&d, false, doctor::systemd_allowed())?);
        changed = true;
    }
    if let Some(stamp) = std::env::var_os("XDG_RUNTIME_DIR") {
        remove(&PathBuf::from(stamp).join("ofolder-guard.stamp"), &mut said)?;
    }

    // The chain, before the files: a chain that names a backend the frontend cannot find is skipped,
    // but one that is already right needs no help.
    if let Some((conf, text)) = crate::portal_conf(&d.home) {
        let name = conf
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let backup = newest_backup(&d, &name);
        let change = chain_change(
            &text,
            backup.as_ref().map(|(t, s)| (t.as_str(), *s)),
            modified_secs(&conf),
        );
        match change {
            ChainChange::None => said.push(format!(
                "ofolder is not in the file-chooser chain in {}: left alone",
                conf.display()
            )),
            ChainChange::Restore(original) => {
                doctor::atomic_write(&conf, &original)?;
                said.push(format!(
                    "put {} back as it was before the last repair",
                    conf.display()
                ));
                changed = true;
            }
            ChainChange::Edit(edited) => {
                doctor::atomic_write(&conf, &edited)?;
                said.push(format!(
                    "took ofolder out of the file-chooser chain in {} and changed nothing else, because the file was edited after the last repair",
                    conf.display()
                ));
                changed = true;
            }
            ChainChange::Delete => {
                fs::remove_file(&conf)?;
                said.push(format!(
                    "removed {}, which said nothing once ofolder was out of the chain",
                    conf.display()
                ));
                changed = true;
            }
        }
    } else {
        said.push("there is no portal configuration file: nothing to put back".to_owned());
    }

    // A registration with `omarchy update` points at a program that is going.
    if d.config.join("praeco/apps.d/ofolder.json").is_file() {
        match crate::own_exe()
            .ok()
            .and_then(|exe| crate::update::praeco_path(&exe))
        {
            Some(praeco) => match crate::update::unregister(&praeco) {
                Ok(line) => said.push(line),
                Err(e) => said.push(format!(
                    "could not remove the registration with omarchy update ({e}); run: ofolder update unregister"
                )),
            },
            None => said.push(
                "ofolder is registered with omarchy update but praeco was not found to take that back; remove ~/.config/praeco/apps.d/ofolder.json"
                    .to_owned(),
            ),
        }
    }

    // The two registration files.
    for file in [d.portal_file(), d.service_file()] {
        if file.is_file() {
            changed = true;
        }
        remove(&file, &mut said)?;
    }

    // State ofolder made: the placement memory, and the whole state folder (the messages log, the
    // update check, the marks for notes already shown, the backups and the undo hold).
    let dialogs = ofolder_core::dialog_memory::memory_path(
        std::env::var_os("OFOLDER_DIALOGS").map(PathBuf::from),
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        &d.home,
    );
    remove(&dialogs, &mut said)?;
    remove(&lock_of(&dialogs), &mut said)?;
    let state = d.state.join("ofolder");
    if state.is_dir() {
        fs::remove_dir_all(&state).with_context(|| format!("cannot remove {}", state.display()))?;
        said.push(format!("removed {}", state.display()));
    }
    if purge {
        for p in &data {
            remove(p, &mut said)?;
        }
    }
    // Folders only ofolder used go once they are empty; one that still holds something is not ours.
    for dir in [d.data.join("ofolder"), d.config.join("ofolder")] {
        if fs::remove_dir(&dir).is_ok() {
            said.push(format!("removed {}", dir.display()));
        }
    }

    for line in &said {
        println!("{line}");
    }
    if changed && restart && doctor::systemd_allowed() {
        if doctor::a_strip_is_open() {
            println!(
                "a dialog is open, so the portal was not restarted; close it and run: systemctl --user restart xdg-desktop-portal.service"
            );
        } else if doctor::systemctl_output(&["restart", "xdg-desktop-portal.service"])
            .is_some_and(|o| o.status.success())
        {
            println!("restarted the portal so the change takes effect");
        } else {
            println!(
                "could not restart the portal; run: systemctl --user restart xdg-desktop-portal.service"
            );
        }
    } else if changed {
        println!(
            "the change takes effect when the portal restarts: systemctl --user restart xdg-desktop-portal.service"
        );
    } else {
        println!("ofolder was not set up: nothing to undo");
    }
    println!("The program files are still in place; install.sh --uninstall removes them.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAIN: &str = "org.freedesktop.impl.portal.FileChooser";

    #[test]
    fn a_chain_without_ofolder_is_left_alone() {
        let conf = format!("[preferred]\n{CHAIN}=flea;gtk\n");
        assert_eq!(chain_change(&conf, None, 0), ChainChange::None);
    }

    #[test]
    fn the_chain_goes_back_to_the_backup_when_only_the_repair_wrote_it() {
        let backup = "[preferred]\ndefault=hyprland\n";
        let now = format!("[preferred]\ndefault=hyprland\n{CHAIN}=ofolder;gtk\n");
        assert_eq!(
            chain_change(&now, Some((backup, 1000)), 1002),
            ChainChange::Restore(backup.to_owned())
        );
    }

    #[test]
    fn a_file_edited_after_the_repair_only_loses_ofolders_entry() {
        let backup = "[preferred]\n";
        let now = format!("[preferred]\nextra=1\n{CHAIN}=ofolder;gtk\n");
        assert_eq!(
            chain_change(&now, Some((backup, 1000)), 5000),
            ChainChange::Edit(format!("[preferred]\nextra=1\n{CHAIN}=gtk\n"))
        );
    }

    #[test]
    fn a_backup_that_names_ofolder_is_never_restored() {
        let backup = format!("[preferred]\n{CHAIN}=ofolder;gtk\n");
        let now = format!("[preferred]\n{CHAIN}=ofolder;gtk\n");
        assert_eq!(
            chain_change(&now, Some((&backup, 1000)), 1000),
            ChainChange::Edit(format!("[preferred]\n{CHAIN}=gtk\n"))
        );
    }

    #[test]
    fn no_backup_and_nothing_else_in_the_file_removes_the_file() {
        let now = format!("[preferred]\n{CHAIN}=ofolder\n");
        assert_eq!(chain_change(&now, None, 0), ChainChange::Delete);
    }

    #[test]
    fn the_lock_file_is_the_name_with_lock_added() {
        assert_eq!(
            lock_of(Path::new("/h/.config/ofolder/favorites.json")),
            PathBuf::from("/h/.config/ofolder/favorites.json.lock")
        );
    }
}
