//! `ofolder doctor` and `ofolder repair`: say what is wrong with how ofolder is set up, and put it
//! right, so that nothing about it has to be remembered.
//!
//! What can go wrong is small and all of it is in files: the program the portal starts is gone, the
//! two registration files are missing, or another program has taken ofolder out of the file-chooser
//! chain (Flea's `--picker` did, once). `doctor` only reads. `repair` writes the registration files
//! only if they are missing or point at nothing, puts ofolder first in the chain changing nothing
//! else, keeps a copy of what it replaced, and can undo the chain change.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use ofolder_core::portal::{self, put_first};

const PORTAL_FILE: &str = include_str!("../../../packaging/ofolder.portal");
const SERVICE_TEMPLATE: &str =
    include_str!("../../../packaging/org.freedesktop.impl.portal.desktop.ofolder.service");
const GUARD_SCRIPT: &str = include_str!("../../../packaging/ofolder-guard.sh");
const SERVICE_NAME: &str = "org.freedesktop.impl.portal.desktop.ofolder.service";

struct Dirs {
    home: PathBuf,
    data: PathBuf,
    config: PathBuf,
    state: PathBuf,
}

fn dirs() -> anyhow::Result<Dirs> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let pick = |var: &str, under: &str| {
        std::env::var_os(var)
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(under))
    };
    Ok(Dirs {
        data: pick("XDG_DATA_HOME", ".local/share"),
        config: pick("XDG_CONFIG_HOME", ".config"),
        state: pick("XDG_STATE_HOME", ".local/state"),
        home,
    })
}

impl Dirs {
    fn portal_file(&self) -> PathBuf {
        self.data.join("xdg-desktop-portal/portals/ofolder.portal")
    }

    fn service_file(&self) -> PathBuf {
        self.data.join("dbus-1/services").join(SERVICE_NAME)
    }

    fn backups(&self) -> PathBuf {
        self.state.join("ofolder/portal-backups")
    }
}

/// The program a D-Bus service file runs: the first word after `Exec=`.
pub fn service_exec(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("Exec="))
        .and_then(|rest| rest.split_whitespace().next())
        .map(str::to_owned)
}

/// The service file for the program at `exe`. A D-Bus service file is not shell quoted, so a path
/// with whitespace, quotes or a backslash cannot be written safely.
pub fn render_service(exe: &Path) -> Result<String, String> {
    let text = exe.to_str().ok_or("the program's path is not valid text")?;
    if text
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'))
    {
        return Err(format!(
            "{text} has whitespace, a quote or a backslash in it, which a D-Bus service file cannot hold; \
             install ofolder somewhere else"
        ));
    }
    Ok(SERVICE_TEMPLATE.replace("@OFOLDER@", text))
}

fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

fn a_strip_is_open() -> bool {
    fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|e| {
            fs::read("/proc/".to_owned() + &e.file_name().to_string_lossy() + "/cmdline")
                .is_ok_and(|c| c.windows(13).any(|w| w == b"ofolder\0strip"))
        })
}

struct Report {
    problems: u32,
}

impl Report {
    fn ok(&self, what: &str) {
        println!("ok       {what}");
    }

    fn problem(&mut self, what: &str, fix: &str) {
        self.problems += 1;
        println!("PROBLEM  {what}");
        println!("         fix: {fix}");
    }
}

/// Reads the setup and prints what is right and wrong with it. The exit status says which.
pub fn doctor() -> anyhow::Result<()> {
    let d = dirs()?;
    let mut r = Report { problems: 0 };

    match fs::read_to_string(d.service_file()) {
        Err(_) => r.problem(
            &format!("the D-Bus service file is missing ({})", d.service_file().display()),
            "ofolder repair",
        ),
        Ok(text) => match service_exec(&text) {
            Some(exe) if runnable(Path::new(&exe)) => {
                r.ok(&format!("the portal starts {exe}"));
            }
            Some(exe) => r.problem(
                &format!("the portal would start {exe}, which is missing or not runnable, so file dialogs would not open at all"),
                "ofolder repair (or put ofolder back at that path)",
            ),
            None => r.problem("the D-Bus service file has no Exec line", "ofolder repair"),
        },
    }
    if d.portal_file().is_file() {
        r.ok("the portal file is in place");
    } else {
        r.problem(
            &format!("the portal file is missing ({})", d.portal_file().display()),
            "ofolder repair",
        );
    }
    let conf = crate::portal_conf(&d.home);
    for line in crate::chain_report(conf) {
        match line.strip_prefix("warning: ") {
            Some(problem) => r.problem(problem, "ofolder repair"),
            None if line.contains("bypass") => r.problem(&line, "ofolder repair"),
            None => r.ok(&line),
        }
    }
    match crate::detect_backend(&d.home) {
        Some(id) => r.ok(&format!("dialogs are forwarded to {id}")),
        None => r.problem(
            "no file-chooser backend that ofolder knows (Flea, GTK or Strata) is installed",
            "install xdg-desktop-portal-gtk, or Flea",
        ),
    }
    let (settings, _) = crate::settings_file()?;
    let paused = ofolder_core::settings::load(&settings, &d.home)
        .settings
        .paused;
    if paused {
        r.problem(
            "ofolder is paused: dialogs pass through without the rails",
            "ofolder resume",
        );
    } else {
        r.ok("ofolder is on");
    }
    let (settings_path, _) = crate::settings_file()?;
    let wants_guard = ofolder_core::settings::load(&settings_path, &d.home)
        .settings
        .guard;
    if !wants_guard {
        r.ok("the guard is turned off in the settings, as chosen");
    } else if !guard_installed(&d) {
        r.problem(
            "the guard is on in the settings but not installed, so nothing would put ofolder back in the chain",
            "ofolder repair --watch on",
        );
    } else if !systemd_allowed() {
        r.ok("the guard's files are in place (systemd is not asked here)");
    } else {
        let enabled = Command::new("systemctl")
            .args(["--user", "is-enabled", "ofolder-guard.path"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        match enabled.as_str() {
            "enabled" => r.ok("the guard is on"),
            "" => r.ok("the guard's files are in place (systemd could not be asked)"),
            other => r.problem(
                &format!("the guard's files are in place but systemd says it is {other}"),
                "ofolder repair --watch on",
            ),
        }
    }
    for tool in ["hyprctl", "wtype", "wl-copy"] {
        if on_path(tool) {
            r.ok(&format!("{tool} is installed"));
        } else {
            r.problem(
                &format!("{tool} is not installed, and ofolder needs it"),
                &format!("install {tool}"),
            );
        }
    }
    if r.problems == 0 {
        println!("\nNothing is wrong.");
        Ok(())
    } else {
        println!("\n{} thing(s) to fix.", r.problems);
        std::process::exit(1);
    }
}

fn atomic_write(path: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("ofolder-tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Puts the setup right, and says what it did. `--undo` puts the chain back as it was before the
/// last repair; `--no-restart` leaves the portal frontend running as it is.
pub fn repair(args: &[String]) -> anyhow::Result<()> {
    let d = dirs()?;
    if args.iter().any(|a| a == "--undo") {
        return undo(&d);
    }
    if let Some(at) = args.iter().position(|a| a == "--watch") {
        let on = match args.get(at + 1).map(String::as_str) {
            Some("on") => true,
            Some("off") => false,
            _ => anyhow::bail!("usage: ofolder repair --watch on|off [--no-enable]"),
        };
        return watch(&d, on, !args.iter().any(|a| a == "--no-enable"));
    }
    let restart = !args.iter().any(|a| a == "--no-restart");
    let mut changed = false;

    // The two registration files, only when missing or pointing at nothing.
    let exec_ok = fs::read_to_string(d.service_file())
        .ok()
        .and_then(|t| service_exec(&t))
        .is_some_and(|exe| runnable(Path::new(&exe)));
    if !exec_ok {
        let exe = crate::own_exe().context("cannot tell where ofolder is installed")?;
        let text = render_service(&exe).map_err(|e| anyhow::anyhow!(e))?;
        atomic_write(&d.service_file(), &text)?;
        println!(
            "wrote {} (the portal will start {})",
            d.service_file().display(),
            exe.display()
        );
        changed = true;
    }
    if !d.portal_file().is_file() {
        atomic_write(&d.portal_file(), PORTAL_FILE)?;
        println!("wrote {}", d.portal_file().display());
        changed = true;
    }

    // The chain.
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or("")
        .to_lowercase();
    let (path, text) = crate::portal_conf(&d.home).unwrap_or_else(|| {
        let name = if desktop.is_empty() {
            "portals.conf".to_owned()
        } else {
            format!("{desktop}-portals.conf")
        };
        (
            d.config.join("xdg-desktop-portal").join(name),
            String::new(),
        )
    });
    let installed: Vec<&str> = portal::discover(&crate::portal_dirs())
        .iter()
        .filter(|p| p.provides_file_chooser())
        .filter_map(portal::PortalFile::adapter_id)
        .collect();
    let others: Vec<&str> = ["flea", "gtk"]
        .into_iter()
        .filter(|id| installed.contains(id))
        .collect();
    match put_first(&text, "ofolder", &others) {
        None => println!("the file-chooser chain already starts with ofolder: left alone"),
        Some(fixed) => {
            if path.is_file() {
                fs::create_dir_all(d.backups())?;
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let backup = d.backups().join(format!("{name}.{}", now()));
                fs::copy(&path, &backup)?;
                println!("kept a copy of {} at {}", path.display(), backup.display());
            }
            atomic_write(&path, &fixed)?;
            println!(
                "put ofolder first in the file-chooser chain in {}",
                path.display()
            );
            changed = true;
        }
    }

    if changed && restart {
        if a_strip_is_open() {
            println!(
                "a dialog is open, so the portal was not restarted; close it and run: systemctl --user restart xdg-desktop-portal.service"
            );
        } else {
            let ok = Command::new("systemctl")
                .args(["--user", "restart", "xdg-desktop-portal.service"])
                .status()
                .is_ok_and(|s| s.success());
            println!(
                "{}",
                if ok {
                    "restarted the portal so the change takes effect"
                } else {
                    "could not restart the portal; run: systemctl --user restart xdg-desktop-portal.service"
                }
            );
        }
    } else if changed {
        println!(
            "the change takes effect when the portal restarts: systemctl --user restart xdg-desktop-portal.service"
        );
    } else {
        println!("nothing needed repairing");
    }
    Ok(())
}

/// The systemd units that run the guard when the program or the portal configuration changes.
pub fn guard_units(exe_dir: &Path, portal_conf_dir: &Path, script: &Path) -> (String, String) {
    let path = format!(
        "[Unit]\nDescription=Keep ofolder in front of file dialogs, and dialogs working without it\n\n\
         [Path]\nPathChanged={}\nPathChanged={}\nUnit=ofolder-guard.service\n\n\
         [Install]\nWantedBy=default.target\n",
        exe_dir.display(),
        portal_conf_dir.display()
    );
    let service = format!(
        "[Unit]\nDescription=ofolder guard\n\n[Service]\nType=oneshot\nExecStart={}\n\n\
         [Install]\nWantedBy=default.target\n",
        script.display()
    );
    (path, service)
}

/// Whether the guard is in place as it should be: the script as packaged, and both units.
fn guard_installed(d: &Dirs) -> bool {
    let script = d.data.join("ofolder/ofolder-guard.sh");
    let units = d.config.join("systemd/user");
    fs::read_to_string(&script).is_ok_and(|t| t == GUARD_SCRIPT)
        && units.join("ofolder-guard.path").is_file()
        && units.join("ofolder-guard.service").is_file()
}

/// Whether systemd is to be used at all. A test, which has a session of its own, sets
/// `OFOLDER_NO_SYSTEMD` so that nothing it does can reach the real user's units.
fn systemd_allowed() -> bool {
    std::env::var_os("OFOLDER_NO_SYSTEMD").is_none()
}

fn systemctl(args: &[&str]) -> bool {
    systemd_allowed()
        && Command::new("systemctl")
            .arg("--user")
            .args(args)
            .status()
            .is_ok_and(|s| s.success())
}

/// Puts the guard in place (`on`) or takes it away, and says what it did. `enable` is false when
/// only the files are wanted (tests, and anyone who wants to read them first).
fn apply_guard(d: &Dirs, on: bool, enable: bool) -> anyhow::Result<Vec<String>> {
    let mut said = Vec::new();
    let script = d.data.join("ofolder/ofolder-guard.sh");
    let unit_dir = d.config.join("systemd/user");
    let path_unit = unit_dir.join("ofolder-guard.path");
    let service_unit = unit_dir.join("ofolder-guard.service");
    if on {
        let exe = fs::read_to_string(d.service_file())
            .ok()
            .and_then(|t| service_exec(&t))
            .map(PathBuf::from)
            .or_else(|| crate::own_exe().ok())
            .context("cannot tell where ofolder is installed: run ofolder repair first")?;
        let exe_dir = exe.parent().context("the program has no folder")?;
        let conf_dir = d.config.join("xdg-desktop-portal");
        atomic_write(&script, GUARD_SCRIPT)?;
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
        }
        let (path_text, service_text) = guard_units(exe_dir, &conf_dir, &script);
        atomic_write(&path_unit, &path_text)?;
        atomic_write(&service_unit, &service_text)?;
        said.push(format!(
            "wrote {} and the units in {}",
            script.display(),
            unit_dir.display()
        ));
        if enable {
            let ok = systemctl(&["daemon-reload"])
                && systemctl(&["enable", "--now", "ofolder-guard.path"])
                && systemctl(&["start", "ofolder-guard.service"]);
            said.push(
                if ok {
                    "the guard is on: it runs at login and when ofolder or the portal configuration changes"
                } else {
                    "the units are written but systemd did not start them"
                }
                .to_owned(),
            );
        }
    } else {
        if enable {
            systemctl(&["disable", "--now", "ofolder-guard.path"]);
        }
        let held = d.data.join("ofolder/ofolder.portal.held");
        if held.is_file() && !d.portal_file().is_file() {
            fs::create_dir_all(d.portal_file().parent().context("no folder")?)?;
            fs::rename(&held, d.portal_file())?;
            said.push("put the portal file the guard had set aside back".to_owned());
        }
        for f in [&path_unit, &service_unit, &script] {
            match fs::remove_file(f) {
                Ok(()) => said.push(format!("removed {}", f.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        if enable {
            systemctl(&["daemon-reload"]);
        }
        said.push("the guard is off".to_owned());
    }
    Ok(said)
}

/// For the settings window: puts the guard in place or takes it away after the setting changed.
pub fn set_guard(on: bool) -> Result<String, String> {
    let d = dirs().map_err(|e| e.to_string())?;
    apply_guard(&d, on, true)
        .map(|said| said.join("; "))
        .map_err(|e| e.to_string())
}

/// Makes the guard match the setting: it is on by default, so a first start installs it, and a
/// guard the maintainer turned off is not put back. Run when the portal starts.
pub fn reconcile_guard() -> anyhow::Result<()> {
    let d = dirs()?;
    let (settings, _) = crate::settings_file()?;
    let wanted = ofolder_core::settings::load(&settings, &d.home)
        .settings
        .guard;
    if wanted != guard_installed(&d) {
        for line in apply_guard(&d, wanted, true)? {
            eprintln!("ofolder: {line}");
        }
    }
    Ok(())
}

/// `repair --watch on|off`: the same, and the setting follows, so the command line and the
/// settings window never disagree.
fn watch(d: &Dirs, on: bool, enable: bool) -> anyhow::Result<()> {
    for line in apply_guard(d, on, enable)? {
        println!("{line}");
    }
    let (settings, _) = crate::settings_file()?;
    ofolder_core::settings_write::save(
        &settings,
        &[ofolder_core::settings_write::Edit::Bool("guard", on)],
    )?;
    if on {
        println!("Turn it off with: ofolder repair --watch off, or in the settings window.");
    }
    Ok(())
}

fn undo(d: &Dirs) -> anyhow::Result<()> {
    let mut newest: Option<(u64, PathBuf, String)> = None;
    for entry in fs::read_dir(d.backups()).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some((original, stamp)) = name.rsplit_once('.')
            && let Ok(stamp) = stamp.parse::<u64>()
            && newest.as_ref().is_none_or(|(s, _, _)| stamp > *s)
        {
            newest = Some((stamp, entry.path(), original.to_owned()));
        }
    }
    let Some((_, backup, original)) = newest else {
        println!("there is no earlier copy to go back to");
        return Ok(());
    };
    let target = d.config.join("xdg-desktop-portal").join(&original);
    atomic_write(&target, &fs::read_to_string(&backup)?)?;
    println!(
        "put {} back as it was (from {})",
        target.display(),
        backup.display()
    );
    println!(
        "the change takes effect when the portal restarts: systemctl --user restart xdg-desktop-portal.service"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_a_service_file_runs_is_its_first_word() {
        assert_eq!(
            service_exec("[D-BUS Service]\nName=x\nExec=/home/u/.local/bin/ofolder portal\n")
                .as_deref(),
            Some("/home/u/.local/bin/ofolder")
        );
        assert_eq!(service_exec("[D-BUS Service]\nName=x\n"), None);
    }

    #[test]
    fn a_service_file_names_the_program_and_runs_the_portal() {
        let text = render_service(Path::new("/home/u/.local/bin/ofolder")).unwrap();
        assert!(text.contains("Exec=/home/u/.local/bin/ofolder portal"));
        assert!(!text.contains("@OFOLDER@"));
        assert_eq!(
            service_exec(&text).as_deref(),
            Some("/home/u/.local/bin/ofolder")
        );
    }

    #[test]
    fn the_guard_units_watch_the_program_and_the_portal_configuration() {
        let (path, service) = guard_units(
            Path::new("/home/u/.local/bin"),
            Path::new("/home/u/.config/xdg-desktop-portal"),
            Path::new("/home/u/.local/share/ofolder/ofolder-guard.sh"),
        );
        assert!(path.contains("PathChanged=/home/u/.local/bin\n"));
        assert!(path.contains("PathChanged=/home/u/.config/xdg-desktop-portal\n"));
        assert!(path.contains("Unit=ofolder-guard.service"));
        assert!(service.contains("ExecStart=/home/u/.local/share/ofolder/ofolder-guard.sh\n"));
        assert!(service.contains("Type=oneshot"));
    }

    #[test]
    fn a_path_a_service_file_cannot_hold_is_refused() {
        for bad in [
            "/home/a b/ofolder",
            "/home/a\"b/ofolder",
            "/home/a'b/ofolder",
            "/home/a\\b/ofolder",
        ] {
            assert!(render_service(Path::new(bad)).is_err(), "{bad}");
        }
    }
}
