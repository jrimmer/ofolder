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
use std::process::{Command, Output};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use ofolder_core::exec::run_with_deadline;
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

    /// Present after `repair --undo`: the maintainer took ofolder out of the chain on purpose, so the
    /// guard must not put it back. `repair` run by hand removes it.
    fn chain_hold(&self) -> PathBuf {
        self.state.join("ofolder/chain-hold")
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
    lines: Vec<String>,
}

impl Report {
    fn ok(&mut self, what: &str) {
        self.lines.push(format!("ok       {what}"));
    }

    /// Something that is not a fault: it is said, and the exit status is not affected.
    fn note(&mut self, what: &str) {
        self.lines.push(format!("note     {what}"));
    }

    fn problem(&mut self, what: &str, fix: &str) {
        self.problems += 1;
        self.lines.push(format!("PROBLEM  {what}"));
        self.lines.push(format!("         fix: {fix}"));
    }
}

/// The text with the user's home folder written as `~`, and the user name and host name replaced,
/// so that a report can be pasted into a bug report without saying whose machine it came from.
/// Names shorter than three letters are left alone: replacing them would damage ordinary words.
pub(crate) fn redact(text: &str, home: &str, user: &str, host: &str) -> String {
    let mut out = text.to_owned();
    if home.len() > 1 {
        out = out.replace(home, "~");
    }
    if user.len() >= 3 {
        out = out.replace(user, "<user>");
    }
    if host.len() >= 3 {
        out = out.replace(host, "<host>");
    }
    out
}

/// First line of what `program args` prints, when it answers within a few seconds.
fn first_line(program: &str, args: &[&str]) -> Option<String> {
    let out = run_with_deadline(Command::new(program).args(args), Duration::from_secs(3)).ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.chars().take(120).collect())
}

/// What a bug report needs to know about the machine, as lines: versions and plain on/off
/// settings. No folder, file or application name is read for it.
fn report_header(d: &Dirs) -> Vec<String> {
    let settings = crate::settings_file()
        .map(|(path, _)| ofolder_core::settings::load(&path, &d.home).settings)
        .unwrap_or_default();
    let os = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| {
            t.lines().find_map(|l| {
                l.strip_prefix("PRETTY_NAME=")
                    .map(|v| v.trim_matches('"').to_owned())
            })
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let strata = d.home.join(".local/bin/strata");
    let strata = if strata.is_file() {
        strata.to_string_lossy().into_owned()
    } else {
        "strata".to_owned()
    };
    let version = |what: &str, found: Option<String>| {
        format!(
            "{what:<22}{}",
            found.unwrap_or_else(|| "not found".to_owned())
        )
    };
    let on_off = |b: bool| if b { "on" } else { "off" };
    vec![
        "ofolder report".to_owned(),
        version("ofolder", Some(env!("CARGO_PKG_VERSION").to_owned())),
        version("system", Some(os)),
        version(
            "desktop",
            Some(std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "not set".to_owned())),
        ),
        version(
            "Hyprland",
            first_line("Hyprland", &["--version"])
                .map(|l| l.split(" built").next().unwrap_or("").to_owned()),
        ),
        version(
            "portal frontend",
            first_line("/usr/lib/xdg-desktop-portal", &["--version"]),
        ),
        version("Flea", first_line("flea", &["--version"])),
        version("Strata", first_line(&strata, &["--version"])),
        format!(
            "settings              paused {}, guard {}, recents {}, updates {}, update with system {}",
            on_off(settings.paused),
            on_off(settings.guard),
            on_off(settings.recents_enabled),
            on_off(settings.updates),
            on_off(settings.update_with_system),
        ),
        "left out              folder and file names, recents, favorites, the messages log"
            .to_owned(),
        String::new(),
    ]
}

/// Reads the setup and prints what is right and wrong with it. The exit status says which.
/// `--report` adds the versions a bug report needs and hides the home folder, user and host name.
pub fn doctor(args: &[String]) -> anyhow::Result<()> {
    let report = args.iter().any(|a| a == "--report");
    if let Some(other) = args.iter().find(|a| a.as_str() != "--report") {
        anyhow::bail!("usage: ofolder doctor [--report] (not {other})");
    }
    let d = dirs()?;
    let mut r = Report {
        problems: 0,
        lines: Vec::new(),
    };

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
    let held = d.chain_hold().is_file();
    for line in crate::chain_report(conf) {
        let problem = line
            .strip_prefix("warning: ")
            .or_else(|| line.contains("bypass").then_some(line.as_str()));
        match problem {
            Some(why) if held => r.ok(&format!(
                "{why} (left this way on purpose since you undid a repair; ofolder repair puts it back)"
            )),
            Some(why) => r.problem(why, "ofolder repair"),
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
        let enabled = systemctl_output(&["is-enabled", "ofolder-guard.path"])
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
    if let Some((id, version)) = crate::detect_backend(&d.home)
        .and_then(|id| crate::backend_version::installed(id, &d.home).map(|v| (id, v)))
    {
        match ofolder_core::known::untested_note(id, &version) {
            Some(note) => r.note(&note),
            None => r.ok(&format!(
                "{id} {version} is one of the versions tried with ofolder"
            )),
        }
    }
    match crate::own_exe().ok().and_then(|exe| crate::update::praeco_path(&exe)) {
        Some(path) => r.ok(&format!("praeco, which does the updates, is at {}", path.display())),
        None => r.note("praeco is not beside ofolder, so nothing is checked or installed (a note, not a problem)"),
    }
    let loaded = ofolder_core::settings::load(&settings_path, &d.home).settings;
    let registered = d.config.join("praeco/apps.d/ofolder.json").is_file();
    let checks = if loaded.updates { "on" } else { "off" };
    let system = match (loaded.update_with_system, registered) {
        (true, true) => "registered with omarchy update",
        (false, false) => "not registered with omarchy update, as chosen",
        (true, false) => {
            "set to update with omarchy update, but not registered (ofolder update register)"
        }
        (false, true) => {
            "registered with omarchy update, but the setting is off (ofolder update unregister)"
        }
    };
    let line = format!("updates: checks are {checks}; {system}");
    if loaded.update_with_system == registered {
        r.ok(&line);
    } else {
        r.note(&line);
    }
    r.lines.push(String::new());
    r.lines.push(if r.problems == 0 {
        "Nothing is wrong.".to_owned()
    } else {
        format!("{} thing(s) to fix.", r.problems)
    });
    let mut text = if report {
        report_header(&d)
    } else {
        Vec::new()
    };
    text.append(&mut r.lines);
    let mut text = text.join("\n") + "\n";
    if report {
        let user = std::env::var("USER").unwrap_or_default();
        let host = fs::read_to_string("/etc/hostname")
            .map(|h| h.trim().to_owned())
            .unwrap_or_default();
        text = redact(&text, &d.home.to_string_lossy(), &user, &host);
    }
    print!("{text}");
    if r.problems == 0 {
        Ok(())
    } else {
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
    let desktop = crate::current_desktops()
        .split(':')
        .next()
        .unwrap_or("")
        .to_owned();
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
    // A repair run by hand is the maintainer asking for ofolder back, which ends an undo's hold; the
    // guard runs it with --from-guard and must leave a held chain as it was left.
    let from_guard = args.iter().any(|a| a == "--from-guard");
    let held = d.chain_hold().is_file();
    if held && !from_guard {
        fs::remove_file(d.chain_hold()).ok();
    }
    let fixed = if held && from_guard {
        println!(
            "the chain was left without ofolder on purpose (you undid a repair): run ofolder repair to put it back"
        );
        None
    } else {
        put_first(&text, "ofolder", &others)
    };
    match fixed {
        None if held && from_guard => {}
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

    // The guard follows its setting too, so one `repair` leaves nothing for `doctor` to flag.
    let wanted = crate::settings_file()
        .ok()
        .map(|(settings, _)| {
            ofolder_core::settings::load(&settings, &d.home)
                .settings
                .guard
        })
        .unwrap_or(true);
    if wanted && !guard_installed(&d) {
        for line in apply_guard(&d, true, true)? {
            println!("{line}");
        }
        changed = true;
    }

    if changed && restart {
        if a_strip_is_open() {
            println!(
                "a dialog is open, so the portal was not restarted; close it and run: systemctl --user restart xdg-desktop-portal.service"
            );
        } else {
            let ok = systemctl_output(&["restart", "xdg-desktop-portal.service"])
                .is_some_and(|o| o.status.success());
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
    // More than ten runs in a minute means another program rewrites the chain on every launch, and
    // the limit makes systemd stop starting the guard instead of looping on it.
    let service = format!(
        "[Unit]\nDescription=ofolder guard\nStartLimitIntervalSec=60\nStartLimitBurst=10\n\n\
         [Service]\nType=oneshot\nExecStart={}\n\n\
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

/// How long a `systemctl --user` request may take. The user manager answers in well under a second;
/// a longer wait means it is wedged, and the doctor or repair would otherwise wait with it forever.
const SYSTEMCTL_LIMIT: Duration = Duration::from_secs(10);

/// Runs `systemctl --user` with `args`, and returns its output when it finished in time.
fn systemctl_output(args: &[&str]) -> Option<Output> {
    run_with_deadline(
        Command::new("systemctl").arg("--user").args(args),
        SYSTEMCTL_LIMIT,
    )
    .ok()
}

fn systemctl(args: &[&str]) -> bool {
    systemd_allowed() && systemctl_output(args).is_some_and(|o| o.status.success())
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
    // Without this the guard would put ofolder straight back, since the file just changed.
    atomic_write(
        &d.chain_hold(),
        "Set by `ofolder repair --undo`; `ofolder repair` removes it and puts ofolder back in the chain.\n",
    )?;
    println!(
        "the guard will leave the chain as it is now until you run ofolder repair; the change takes effect when the portal restarts: systemctl --user restart xdg-desktop-portal.service"
    );
    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_report_hides_the_home_folder_the_user_and_the_host() {
        let text =
            "the portal starts /home/user/.local/bin/ofolder\nwritten by user on host\n";
        let shown = redact(text, "/home/user", "user", "host");
        assert_eq!(
            shown,
            "the portal starts ~/.local/bin/ofolder\nwritten by <user> on <host>\n"
        );
        assert!(!shown.contains("user"));
    }

    #[test]
    fn names_too_short_to_replace_safely_are_left_alone() {
        // A user called "al" must not turn every "also" and "all" into "<user>so".
        assert_eq!(redact("all is well", "/home/al", "al", "pc"), "all is well");
        // A home of "/" would replace every slash.
        assert_eq!(redact("/usr/bin", "/", "root", "box"), "/usr/bin");
    }
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
    fn the_guard_service_stops_starting_after_ten_runs_in_a_minute() {
        let (_, service) = guard_units(
            Path::new("/home/u/.local/bin"),
            Path::new("/home/u/.config/xdg-desktop-portal"),
            Path::new("/home/u/.local/share/ofolder/ofolder-guard.sh"),
        );
        let unit_section = service
            .split("[Service]")
            .next()
            .expect("the unit text has a [Service] section after its [Unit] section");
        assert!(
            unit_section.contains("\nStartLimitIntervalSec=60\n"),
            "{unit_section}"
        );
        assert!(
            unit_section.contains("\nStartLimitBurst=10\n"),
            "{unit_section}"
        );
        assert!(
            !service.contains("[Service]\nStartLimit"),
            "the limit belongs in [Unit]"
        );
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
