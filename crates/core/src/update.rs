//! Keeping ofolder up to date: where releases are read from, when they are
//! checked, what the window shows, and the praeco commands that check and
//! install.
//!
//! praeco does the work (asking the release host, downloading, verifying,
//! installing and restarting); ofolder runs it and reads what it says. Nothing
//! here starts a process or reads the network: the functions take what they
//! need as arguments, so the rules (due or not, which version is offered, what
//! a praeco line means) are tested without either. The process side lives in
//! `ofolder`'s `update` module.

use std::cmp::Ordering;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::private_file;

/// The forge the releases are read from. This is the one place to change when
/// the releases get a public home; `OFOLDER_UPDATE_SOURCE` overrides it.
pub const DEFAULT_FORGEJO_BASE: &str = "https://forge.example";
pub const DEFAULT_REPO: &str = "jrimmer/ofolder";

/// The name praeco registers ofolder under, and the name of its registry file.
pub const REGISTRATION_NAME: &str = "ofolder";

/// A check is repeated once the last successful one is this old.
pub const CHECK_EVERY_SECS: i64 = 7 * 24 * 60 * 60;
/// After a failed check, the next is tried this much later, rather than every hour.
pub const RETRY_AFTER_FAILURE_SECS: i64 = 24 * 60 * 60;
/// How often the portal looks at the clock to see whether a check is due.
pub const POLL_SECS: u64 = 60 * 60;

/// Where the praeco binary comes from when there is no `OFOLDER_PRAECO`: it is
/// shipped beside ofolder, or one level up in `lib/ofolder`, never on `PATH`.
pub const SYSTEM_PRAECO: &str = "/usr/lib/ofolder/praeco";

/// Where release assets are read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Forgejo { base: String, repo: String },
    GitHub { repo: String },
}

impl Source {
    /// The source built into ofolder.
    pub fn default_source() -> Source {
        Source::Forgejo {
            base: DEFAULT_FORGEJO_BASE.to_owned(),
            repo: DEFAULT_REPO.to_owned(),
        }
    }

    /// Reads `forgejo BASE OWNER/REPO` or `github OWNER/REPO`, the forms praeco
    /// itself takes, so the environment variable reads the same as praeco's flags.
    pub fn parse(text: &str) -> Result<Source, String> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let usage = "expected `forgejo BASE OWNER/REPO` or `github OWNER/REPO`";
        match words.as_slice() {
            ["forgejo", base, repo] => {
                if !(base.starts_with("https://") || base.starts_with("http://")) {
                    return Err(format!("{base:?} is not a web address; {usage}"));
                }
                Ok(Source::Forgejo {
                    base: (*base).to_owned(),
                    repo: checked_repo(repo)?,
                })
            }
            ["github", repo] => Ok(Source::GitHub {
                repo: checked_repo(repo)?,
            }),
            _ => Err(usage.to_owned()),
        }
    }

    /// The source the environment asks for: `OFOLDER_UPDATE_SOURCE` when set
    /// and not empty, else the built-in one. A value that does not parse is an
    /// error, not a fallback, so a typo does not quietly check the wrong place.
    pub fn from_env_value(value: Option<&str>) -> Result<Source, String> {
        match value.map(str::trim).filter(|v| !v.is_empty()) {
            Some(text) => Source::parse(text),
            None => Ok(Source::default_source()),
        }
    }

    /// The praeco arguments that name this source, e.g. `--forgejo BASE OWNER/REPO`.
    pub fn praeco_args(&self) -> Vec<OsString> {
        match self {
            Source::Forgejo { base, repo } => {
                vec!["--forgejo".into(), base.into(), repo.into()]
            }
            Source::GitHub { repo } => vec!["--github".into(), repo.into()],
        }
    }
}

/// `OWNER/REPO` with exactly one slash and no empty half.
fn checked_repo(repo: &str) -> Result<String, String> {
    let mut parts = repo.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) if !owner.is_empty() && !name.is_empty() => {
            Ok(repo.to_owned())
        }
        _ => Err(format!("{repo:?} is not OWNER/REPO")),
    }
}

/// The praeco to run: the `OFOLDER_PRAECO` value when it is set (and then no
/// other place is searched, and a path that is not usable gives `None`, which
/// switches the update UI off), otherwise the first usable of
/// `<exe dir>/praeco`, `<exe dir>/../lib/ofolder/praeco` and
/// [`SYSTEM_PRAECO`]. `usable` says whether a path is an executable file.
pub fn locate_praeco(
    env_value: Option<&Path>,
    exe_dir: &Path,
    usable: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if let Some(path) = env_value.filter(|p| !p.as_os_str().is_empty()) {
        return usable(path).then(|| path.to_path_buf());
    }
    let candidates = [
        lexical_clean(&exe_dir.join("praeco")),
        lexical_clean(&exe_dir.join("../lib/ofolder/praeco")),
        PathBuf::from(SYSTEM_PRAECO),
    ];
    candidates.into_iter().find(|c| usable(c))
}

/// `path` with `.` and `..` folded away, so a location reads as it is named.
/// It does not follow links, which is what praeco itself is shown against.
fn lexical_clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The version the update check compares against: the `OFOLDER_PRETEND_VERSION`
/// override when given and not empty, else the real one.
pub fn running_version(pretend: Option<&str>, real: &str) -> String {
    match pretend.map(str::trim).filter(|p| !p.is_empty()) {
        Some(version) => version.to_owned(),
        None => real.to_owned(),
    }
}

/// The version as people read it: a leading `v` is dropped.
pub fn display_version(version: &str) -> String {
    version.trim().trim_start_matches(['v', 'V']).to_owned()
}

/// Compares two versions by their dotted numbers. A leading `v` is ignored, and
/// anything after a `-` or `+` too; a missing number counts as zero, so
/// `1.2` and `1.2.0` are equal, and a part that is not a number counts as zero.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let numbers = |v: &str| -> Vec<u64> {
        let v = v.trim().trim_start_matches(['v', 'V']);
        let core = v.split(['-', '+']).next().unwrap_or("");
        core.split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (x, y) = (numbers(a), numbers(b));
    for i in 0..x.len().max(y.len()) {
        let left = x.get(i).copied().unwrap_or(0);
        let right = y.get(i).copied().unwrap_or(0);
        match left.cmp(&right) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

/// Whether a check is due now. `last_checked` is the time of the last
/// successful check, in Unix seconds. A check is due when there has been none,
/// when the last one is a week old or more, or when the clock has gone back
/// behind it. `forced` (the test setting, `OFOLDER_UPDATE_CHECK_DELAY`) makes
/// it due whatever the last check was.
pub fn check_due(last_checked: Option<i64>, now: i64, forced: bool) -> bool {
    if forced {
        return true;
    }
    match last_checked {
        None => true,
        Some(last) if last > now => true,
        Some(last) => now - last >= CHECK_EVERY_SECS,
    }
}

/// One release as the update window shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Release {
    pub version: String,
    /// The day it was published, as praeco gives it (`2026-10-24T09:12:00Z`).
    pub published_at: String,
    pub url: String,
    /// Markdown, up to the `## Install` heading.
    pub notes: String,
}

/// What `praeco check --json` said about the install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallInfo {
    pub kind: String,
    pub supported: bool,
}

/// One answer from praeco's `check`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckResult {
    Answer {
        latest: Option<String>,
        update_available: bool,
        releases: Vec<Release>,
        install: Option<InstallInfo>,
    },
    Failed {
        code: String,
        message: String,
    },
}

/// Reads praeco's `check --json` line. Anything that is not an answer, or
/// reports `ok: false`, is a failure with praeco's own code and message, so a
/// caller can always say why nothing was shown.
pub fn parse_check(text: &str) -> CheckResult {
    let value: serde_json::Value = match serde_json::from_str(text.trim()) {
        Ok(value) => value,
        Err(_) => {
            return CheckResult::Failed {
                code: "json".to_owned(),
                message: "praeco did not answer with JSON".to_owned(),
            };
        }
    };
    if value.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        let error = value.get("error");
        let text_of = |key: &str, fallback: &str| {
            error
                .and_then(|e| e.get(key))
                .and_then(serde_json::Value::as_str)
                .unwrap_or(fallback)
                .to_owned()
        };
        return CheckResult::Failed {
            code: text_of("code", "unknown"),
            message: text_of("message", "The update check failed."),
        };
    }
    let text_of = |v: &serde_json::Value, key: &str| {
        v.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let releases = value
        .get("releases")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|r| Release {
                    version: text_of(r, "version"),
                    published_at: text_of(r, "published_at"),
                    url: text_of(r, "url"),
                    notes: text_of(r, "notes"),
                })
                .collect()
        })
        .unwrap_or_default();
    let install = value
        .get("install")
        .filter(|i| i.is_object())
        .map(|i| InstallInfo {
            kind: text_of(i, "kind"),
            supported: i
                .get("supported")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        });
    CheckResult::Answer {
        latest: value
            .get("latest")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        update_available: value
            .get("update_available")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        releases,
        install,
    }
}

/// What the update check last found, as the state file keeps it.
///
/// Only the portal writes this, after a successful check. The strips and the
/// settings window read it; the settings window also writes the choices the
/// owner makes (skip, later) through [`modify_state`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// When the last successful check ran, in Unix seconds.
    pub checked_at: Option<i64>,
    /// The version the check compared against.
    pub current: Option<String>,
    pub latest: Option<String>,
    pub update_available: bool,
    /// False when praeco cannot install the release on this kind of install,
    /// so the window offers the release page instead of Install.
    pub can_install: bool,
    /// The releases newer than `current`, newest first.
    pub notes: Vec<Release>,
    /// The version the maintainer chose to skip: nothing newer than it is shown.
    pub skipped: Option<String>,
    /// The `checked_at` the maintainer chose "Later" on. The pill stays hidden until
    /// a newer check, which has a different `checked_at`, replaces it.
    pub later_checked_at: Option<i64>,
}

/// The version to offer, if any: an update that is newer than the running
/// version, not skipped, not put off until the next check, and not for an
/// install praeco cannot update. The result is praeco's `latest`.
pub fn offered_version<'a>(state: &'a State, running: &str) -> Option<&'a str> {
    if !state.update_available {
        return None;
    }
    let latest = state.latest.as_deref()?;
    if compare_versions(latest, running) != Ordering::Greater {
        return None;
    }
    if let Some(skipped) = state.skipped.as_deref()
        && compare_versions(latest, skipped) != Ordering::Greater
    {
        return None;
    }
    if state.later_checked_at.is_some() && state.later_checked_at == state.checked_at {
        return None;
    }
    Some(latest)
}

/// The state file: `$XDG_STATE_HOME/ofolder/update.json`, or
/// `~/.local/state/ofolder/update.json`.
pub fn state_path(xdg_state: Option<PathBuf>, home: &Path) -> PathBuf {
    xdg_state
        .unwrap_or_else(|| home.join(".local/state"))
        .join("ofolder/update.json")
}

/// Reads the state file. A missing or unreadable file is no check yet, so the
/// window shows nothing rather than a failure.
pub fn read_state(path: &Path) -> State {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Changes the state file under its lock and writes it private and whole,
/// returning what was written. The read and the write are one step, so two
/// windows choosing at once cannot drop each other's change.
pub fn modify_state(path: &Path, change: impl FnOnce(&mut State)) -> io::Result<State> {
    private_file::with_lock(path, || -> io::Result<State> {
        let mut state = read_state(path);
        change(&mut state);
        let bytes = serde_json::to_vec_pretty(&state).map_err(io::Error::other)?;
        private_file::write_private_atomic(path, &bytes)?;
        Ok(state)
    })
    .and_then(|written| written)
}

/// Records a successful check. The check time is the one a later "Later" is
/// compared with, so each check brings the pill back.
pub fn record_check(
    path: &Path,
    now: i64,
    running: &str,
    result: &CheckResult,
) -> Option<io::Result<State>> {
    let CheckResult::Answer {
        latest,
        update_available,
        releases,
        install,
    } = result
    else {
        return None;
    };
    // praeco says so for a source build; the window does not offer those.
    let unsupported = install.as_ref().is_some_and(|i| i.kind == "unsupported");
    let can_install = install.as_ref().is_none_or(|i| i.supported);
    Some(modify_state(path, |state| {
        state.checked_at = Some(now);
        state.current = Some(running.to_owned());
        state.latest.clone_from(latest);
        state.update_available = *update_available && !unsupported;
        state.can_install = can_install;
        state.notes.clone_from(releases);
    }))
}

/// The arguments after `praeco` for a check.
pub fn check_args(source: &Source, current: &str, exe: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["check".into()];
    args.extend(source.praeco_args());
    args.extend([
        "--current".into(),
        current.into(),
        "--exe".into(),
        exe.into(),
        "--json".into(),
    ]);
    args
}

/// Everything an install needs, as praeco takes it.
pub struct InstallPlan<'a> {
    pub source: &'a Source,
    pub version: &'a str,
    pub exe: &'a Path,
    /// The portal, which praeco stops (SIGTERM) before it restarts ofolder.
    pub restart_pid: u32,
    pub progress_file: &'a Path,
    /// The program that is started after the install.
    pub restart_program: &'a Path,
    pub restart_args: &'a [&'a str],
    /// `--mode terminal`, for a system install that needs a password window.
    pub terminal: bool,
}

/// The arguments after `praeco` for an install. The restart command comes
/// last, after `--`, as praeco requires.
pub fn install_args(plan: &InstallPlan<'_>) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["install".into()];
    args.extend(plan.source.praeco_args());
    args.extend([
        "--version".into(),
        plan.version.into(),
        "--exe".into(),
        plan.exe.into(),
        "--restart-pid".into(),
        plan.restart_pid.to_string().into(),
        "--detach".into(),
        "--progress-file".into(),
        plan.progress_file.into(),
    ]);
    if plan.terminal {
        args.extend(["--mode".into(), "terminal".into()]);
    }
    args.push("--restart".into());
    args.push("--".into());
    args.push(plan.restart_program.into());
    args.extend(plan.restart_args.iter().map(OsString::from));
    args
}

/// The arguments after `praeco` that register ofolder with the system update.
pub fn register_args(source: &Source, exe: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["init".into(), "--name".into(), REGISTRATION_NAME.into()];
    args.extend(source.praeco_args());
    args.extend(["--exe".into(), exe.into(), "--hook".into(), "auto".into()]);
    args
}

/// The arguments after `praeco` that take the registration away again.
pub fn unregister_args() -> Vec<OsString> {
    [
        "init",
        "--remove",
        "--name",
        REGISTRATION_NAME,
        "--hook",
        "auto",
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// One line of the install's progress file, in words a person can read.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Working {
        text: String,
        /// 0 to 1 while downloading, else `None`.
        fraction: Option<f64>,
    },
    /// praeco cannot ask for a password here: run the install again in a terminal.
    NeedsTerminal {
        message: String,
    },
    Done {
        restarted: bool,
    },
    Failed {
        code: String,
        message: String,
    },
    /// A line praeco wrote that this version does not show.
    Ignored,
}

/// What one JSON line of praeco's progress means.
pub fn parse_event(line: &str) -> Event {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
        return Event::Ignored;
    };
    let name = value
        .get("event")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let working = |text: &str| Event::Working {
        text: text.to_owned(),
        fraction: None,
    };
    match name {
        "resolving" => working("Getting ready"),
        "downloading" => {
            let total = value.get("total").and_then(serde_json::Value::as_f64);
            let bytes = value
                .get("bytes")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            match total {
                Some(total) if total > 0.0 => {
                    let fraction = (bytes / total).clamp(0.0, 1.0);
                    Event::Working {
                        text: format!("Downloading {}%", (fraction * 100.0).floor() as u32),
                        fraction: Some(fraction),
                    }
                }
                _ => working("Downloading"),
            }
        }
        "verifying" => working("Checking the download"),
        "installing" => working("Installing"),
        "installed" | "restarting" => working("Restarting"),
        "terminal_opened" => working("Finishing in a terminal window"),
        "done" => {
            let restarted = value
                .get("restarted")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            Event::Done { restarted }
        }
        "needs_terminal" => Event::NeedsTerminal {
            message: text("message"),
        },
        "error" => {
            let message = text("message");
            Event::Failed {
                code: text("code"),
                message: if message.is_empty() {
                    "The update did not finish.".to_owned()
                } else {
                    message
                },
            }
        }
        _ => Event::Ignored,
    }
}

/// Reads a progress file as it grows: each call takes the bytes after the last
/// one and returns the events of the lines they complete. A line still being
/// written waits for its newline, so nothing is read half-written.
#[derive(Debug, Default)]
pub struct Tail {
    offset: u64,
    pending: Vec<u8>,
}

impl Tail {
    /// How many bytes of the file have been taken so far: where the next read
    /// of the file starts.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Takes the next bytes of the file and returns the events they complete.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.offset += bytes.len() as u64;
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(newline) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=newline).collect();
            let text = String::from_utf8_lossy(&line);
            if text.trim().is_empty() {
                continue;
            }
            match parse_event(&text) {
                Event::Ignored => {}
                event => events.push(event),
            }
        }
        events
    }
}

/// The words for a release's version on the pill and in the window title.
pub fn pill_text(latest: &str) -> String {
    format!("{} available", display_version(latest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    fn release(version: &str) -> Release {
        Release {
            version: version.to_owned(),
            published_at: "2026-10-24T09:12:00Z".to_owned(),
            url: format!("https://forge.example/jrimmer/ofolder/releases/tag/v{version}"),
            notes: "Fixed\n- A thing.".to_owned(),
        }
    }

    fn answer(latest: &str, kind: &str, supported: bool) -> CheckResult {
        CheckResult::Answer {
            latest: Some(latest.to_owned()),
            update_available: true,
            releases: vec![release(latest)],
            install: Some(InstallInfo {
                kind: kind.to_owned(),
                supported,
            }),
        }
    }

    #[test]
    fn the_built_in_source_is_forgejo_on_the_owners_forge() {
        assert_eq!(
            Source::from_env_value(None),
            Ok(Source::Forgejo {
                base: "https://forge.example".to_owned(),
                repo: "jrimmer/ofolder".to_owned(),
            })
        );
        assert_eq!(
            Source::from_env_value(Some("  ")),
            Source::from_env_value(None)
        );
    }

    #[test]
    fn the_environment_source_reads_praeco_s_own_forms() {
        assert_eq!(
            Source::parse("github jrimmer/ofolder"),
            Ok(Source::GitHub {
                repo: "jrimmer/ofolder".to_owned()
            })
        );
        assert_eq!(
            Source::parse("forgejo http://127.0.0.1:3000 jrimmer/ofolder"),
            Ok(Source::Forgejo {
                base: "http://127.0.0.1:3000".to_owned(),
                repo: "jrimmer/ofolder".to_owned(),
            })
        );
    }

    #[test]
    fn a_source_that_does_not_parse_is_refused_rather_than_guessed() {
        assert!(Source::from_env_value(Some("gitlab jrimmer/ofolder")).is_err());
        assert!(Source::parse("github user").is_err());
        assert!(Source::parse("github jrimmer/ofolder/extra").is_err());
        assert!(Source::parse("github /ofolder").is_err());
        assert!(Source::parse("forgejo forge.example jrimmer/ofolder").is_err());
        assert!(checked_repo("a/b").is_ok());
    }

    #[test]
    fn the_source_becomes_praeco_flags_with_each_part_as_its_own_argument() {
        let forge = Source::default_source().praeco_args();
        assert_eq!(
            forge,
            ["--forgejo", "https://forge.example", "jrimmer/ofolder"]
        );
        let hub = Source::GitHub {
            repo: "a/b".to_owned(),
        };
        assert_eq!(hub.praeco_args(), ["--github", "a/b"]);
    }

    #[test]
    fn praeco_is_found_beside_ofolder_then_one_level_up_then_in_the_system_folder() {
        let exe_dir = p("/home/u/.local/bin");
        let beside = p("/home/u/.local/bin/praeco");
        let up = p("/home/u/.local/lib/ofolder/praeco");
        let system = p(SYSTEM_PRAECO);
        let all = [beside.clone(), up.clone(), system.clone()];
        let usable = |path: &Path| all.iter().any(|c| c == path);
        assert_eq!(locate_praeco(None, &exe_dir, usable), Some(beside.clone()));
        let only_up = |path: &Path| path == up;
        assert_eq!(locate_praeco(None, &exe_dir, only_up), Some(up));
        let only_system = |path: &Path| path == system;
        assert_eq!(locate_praeco(None, &exe_dir, only_system), Some(system));
        assert_eq!(locate_praeco(None, &exe_dir, |_| false), None);
    }

    #[test]
    fn an_override_is_the_only_praeco_and_an_unusable_one_switches_the_ui_off() {
        let exe_dir = p("/home/u/.local/bin");
        let beside = p("/home/u/.local/bin/praeco");
        let usable = |path: &Path| path == beside;
        let over = p("/opt/test/praeco");
        assert_eq!(locate_praeco(Some(&over), &exe_dir, usable), None);
        assert_eq!(
            locate_praeco(Some(&beside), &exe_dir, usable),
            Some(beside.clone())
        );
        assert_eq!(
            locate_praeco(Some(Path::new("")), &exe_dir, usable),
            Some(beside)
        );
    }

    #[test]
    fn the_praeco_search_names_each_place_without_its_dots() {
        let seen = std::cell::RefCell::new(Vec::new());
        let record = |path: &Path| {
            seen.borrow_mut().push(path.to_path_buf());
            false
        };
        locate_praeco(None, &p("/home/u/.local/bin"), record);
        assert_eq!(
            seen.borrow().as_slice(),
            [
                p("/home/u/.local/bin/praeco"),
                p("/home/u/.local/lib/ofolder/praeco"),
                p(SYSTEM_PRAECO),
            ]
        );
    }

    #[test]
    fn the_running_version_is_the_pretend_one_only_when_one_is_given() {
        assert_eq!(running_version(Some("0.0.9"), "0.1.0"), "0.0.9");
        assert_eq!(running_version(Some(" "), "0.1.0"), "0.1.0");
        assert_eq!(running_version(None, "0.1.0"), "0.1.0");
        assert_eq!(display_version("v1.2.3"), "1.2.3");
        assert_eq!(pill_text("v1.2.3"), "1.2.3 available");
    }

    #[test]
    fn versions_compare_by_their_numbers_not_their_text() {
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("v1.2", "1.2.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.2.0-rc1", "1.2.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.2.0", "1.10.0"), Ordering::Less);
    }

    #[test]
    fn a_check_is_due_when_none_was_made_or_the_last_is_a_week_old() {
        let now = 1_800_000_000;
        assert!(check_due(None, now, false));
        assert!(!check_due(Some(now - 60), now, false));
        assert!(!check_due(Some(now - CHECK_EVERY_SECS + 1), now, false));
        assert!(check_due(Some(now - CHECK_EVERY_SECS), now, false));
    }

    #[test]
    fn a_check_is_due_when_the_clock_has_gone_back_past_the_last_one() {
        assert!(check_due(Some(2_000), 1_000, false));
    }

    #[test]
    fn a_forced_check_is_due_even_when_one_was_made_this_week() {
        assert!(check_due(Some(1_000), 1_001, true));
    }

    #[test]
    fn a_check_answer_with_an_update_parses_its_releases_and_install() {
        let text = r#"{"ok":true,"source":"forgejo:x","current":"0.1.0","latest":"0.2.0",
            "update_available":true,"releases":[{"version":"0.2.0","tag":"v0.2.0",
            "name":"ofolder 0.2.0","published_at":"2026-10-24T09:12:00Z","url":"https://x/v0.2.0",
            "prerelease":false,"notes":"Fixed\n- A thing."}],
            "install":{"kind":"tarball","supported":true,"reason":"r","detail":null,"target":"/x","asset":null}}"#;
        match parse_check(text) {
            CheckResult::Answer {
                latest,
                update_available,
                releases,
                install,
            } => {
                assert_eq!(latest.as_deref(), Some("0.2.0"));
                assert!(update_available);
                assert_eq!(releases.len(), 1);
                assert_eq!(releases[0].version, "0.2.0");
                assert_eq!(releases[0].url, "https://x/v0.2.0");
                assert_eq!(releases[0].notes, "Fixed\n- A thing.");
                assert_eq!(
                    install,
                    Some(InstallInfo {
                        kind: "tarball".to_owned(),
                        supported: true
                    })
                );
            }
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    #[test]
    fn an_up_to_date_answer_offers_nothing() {
        let text = r#"{"ok":true,"source":"github:a/b","current":"0.2.0","latest":"0.2.0","update_available":false,"releases":[]}"#;
        assert_eq!(
            parse_check(text),
            CheckResult::Answer {
                latest: Some("0.2.0".to_owned()),
                update_available: false,
                releases: Vec::new(),
                install: None,
            }
        );
    }

    #[test]
    fn a_failure_object_gives_praeco_s_code_and_message() {
        let text =
            r#"{"ok":false,"error":{"code":"network","message":"Could not reach the host."}}"#;
        assert_eq!(
            parse_check(text),
            CheckResult::Failed {
                code: "network".to_owned(),
                message: "Could not reach the host.".to_owned(),
            }
        );
    }

    #[test]
    fn output_that_is_not_json_is_a_failure_not_an_answer() {
        assert!(matches!(
            parse_check("Segmentation fault"),
            CheckResult::Failed { code, .. } if code == "json"
        ));
        assert!(matches!(
            parse_check(r#"{"ok":false}"#),
            CheckResult::Failed { code, .. } if code == "unknown"
        ));
    }

    #[test]
    fn a_recorded_check_is_read_back_with_its_time_and_versions() {
        let dir =
            std::env::temp_dir().join(format!("ofolder-update-record-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        let path = dir.join("update.json");
        let written = record_check(&path, 1_000, "0.1.0", &answer("0.2.0", "tarball", true))
            .expect("an answer is recorded")
            .expect("written");
        assert_eq!(read_state(&path), written);
        assert_eq!(written.checked_at, Some(1_000));
        assert_eq!(written.current.as_deref(), Some("0.1.0"));
        assert!(written.update_available && written.can_install);
        assert_eq!(written.notes.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_check_records_nothing() {
        let dir = std::env::temp_dir().join(format!("ofolder-update-fail-{}", std::process::id()));
        let path = dir.join("update.json");
        let failed = CheckResult::Failed {
            code: "network".to_owned(),
            message: "offline".to_owned(),
        };
        assert!(record_check(&path, 1_000, "0.1.0", &failed).is_none());
        assert!(!path.exists(), "no state file is written for a failure");
    }

    #[test]
    fn a_source_build_is_never_offered_an_install() {
        let dir = std::env::temp_dir().join(format!("ofolder-update-unsup-{}", std::process::id()));
        let path = dir.join("update.json");
        let written = record_check(&path, 5, "0.1.0", &answer("0.2.0", "unsupported", false))
            .expect("recorded")
            .expect("written");
        assert!(
            !written.update_available,
            "praeco's unsupported kind hides the pill"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_asset_still_offers_the_update_but_not_an_install() {
        let state = State {
            checked_at: Some(5),
            latest: Some("0.2.0".to_owned()),
            update_available: true,
            can_install: false,
            ..State::default()
        };
        assert_eq!(offered_version(&state, "0.1.0"), Some("0.2.0"));
    }

    #[test]
    fn a_skipped_version_hides_its_pill_and_a_newer_one_shows_again() {
        let mut state = State {
            checked_at: Some(5),
            latest: Some("0.2.0".to_owned()),
            update_available: true,
            can_install: true,
            skipped: Some("0.2.0".to_owned()),
            ..State::default()
        };
        assert_eq!(offered_version(&state, "0.1.0"), None);
        state.latest = Some("0.2.1".to_owned());
        assert_eq!(offered_version(&state, "0.1.0"), Some("0.2.1"));
    }

    #[test]
    fn later_hides_the_pill_until_the_next_check_brings_it_back() {
        let mut state = State {
            checked_at: Some(5),
            latest: Some("0.2.0".to_owned()),
            update_available: true,
            can_install: true,
            later_checked_at: Some(5),
            ..State::default()
        };
        assert_eq!(offered_version(&state, "0.1.0"), None);
        state.checked_at = Some(9);
        assert_eq!(offered_version(&state, "0.1.0"), Some("0.2.0"));
    }

    #[test]
    fn nothing_is_offered_once_the_running_version_has_caught_up() {
        let state = State {
            checked_at: Some(5),
            latest: Some("0.2.0".to_owned()),
            update_available: true,
            can_install: true,
            ..State::default()
        };
        assert_eq!(offered_version(&state, "0.2.0"), None);
        assert_eq!(offered_version(&state, "0.3.0"), None);
        assert_eq!(offered_version(&State::default(), "0.1.0"), None);
    }

    #[test]
    fn the_state_file_lives_in_the_state_folder_unless_it_is_named() {
        let home = p("/home/u");
        assert_eq!(
            state_path(None, &home),
            p("/home/u/.local/state/ofolder/update.json")
        );
        assert_eq!(
            state_path(Some(p("/run/state")), &home),
            p("/run/state/ofolder/update.json")
        );
    }

    #[test]
    fn a_state_file_that_is_damaged_reads_as_no_check_yet() {
        let dir =
            std::env::temp_dir().join(format!("ofolder-update-damaged-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("folder");
        let path = dir.join("update.json");
        fs::write(&path, "{not json").expect("write");
        assert_eq!(read_state(&path), State::default());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_modified_state_keeps_what_the_change_did_not_touch() {
        let dir =
            std::env::temp_dir().join(format!("ofolder-update-modify-{}", std::process::id()));
        let path = dir.join("update.json");
        modify_state(&path, |s| s.latest = Some("0.2.0".to_owned())).expect("first");
        modify_state(&path, |s| s.skipped = Some("0.2.0".to_owned())).expect("second");
        let state = read_state(&path);
        assert_eq!(state.latest.as_deref(), Some("0.2.0"));
        assert_eq!(state.skipped.as_deref(), Some("0.2.0"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_check_command_names_the_source_and_asks_for_json() {
        let args = check_args(
            &Source::default_source(),
            "0.1.0",
            &p("/home/u/.local/bin/ofolder"),
        );
        assert_eq!(
            args,
            [
                "check",
                "--forgejo",
                "https://forge.example",
                "jrimmer/ofolder",
                "--current",
                "0.1.0",
                "--exe",
                "/home/u/.local/bin/ofolder",
                "--json",
            ]
        );
    }

    #[test]
    fn the_install_command_keeps_values_with_spaces_and_quotes_as_single_arguments() {
        let source = Source::GitHub {
            repo: "a/b".to_owned(),
        };
        let progress = p("/run/user/1000/ofolder/update 1 \"x\".jsonl");
        let exe = p("/home/u/My Apps/ofolder");
        let plan = InstallPlan {
            source: &source,
            version: "0.2.0",
            exe: &exe,
            restart_pid: 4321,
            progress_file: &progress,
            restart_program: &exe,
            restart_args: &["status"],
            terminal: false,
        };
        let args = install_args(&plan);
        assert_eq!(
            args,
            [
                "install",
                "--github",
                "a/b",
                "--version",
                "0.2.0",
                "--exe",
                "/home/u/My Apps/ofolder",
                "--restart-pid",
                "4321",
                "--detach",
                "--progress-file",
                "/run/user/1000/ofolder/update 1 \"x\".jsonl",
                "--restart",
                "--",
                "/home/u/My Apps/ofolder",
                "status",
            ]
        );
    }

    #[test]
    fn a_terminal_install_adds_the_mode_before_the_restart_command() {
        let source = Source::default_source();
        let exe = p("/x/ofolder");
        let progress = p("/run/p.jsonl");
        let plan = InstallPlan {
            source: &source,
            version: "0.2.0",
            exe: &exe,
            restart_pid: 7,
            progress_file: &progress,
            restart_program: &exe,
            restart_args: &["status"],
            terminal: true,
        };
        let args = install_args(&plan);
        let mode = args.iter().position(|a| a == "--mode").expect("mode");
        assert_eq!(args[mode + 1], "terminal");
        let dashes = args.iter().position(|a| a == "--").expect("separator");
        assert!(mode < dashes, "the restart command must come last");
    }

    #[test]
    fn registration_and_removal_name_ofolder_and_the_automatic_hook() {
        let source = Source::default_source();
        assert_eq!(
            register_args(&source, &p("/home/u/.local/bin/ofolder")),
            [
                "init",
                "--name",
                "ofolder",
                "--forgejo",
                "https://forge.example",
                "jrimmer/ofolder",
                "--exe",
                "/home/u/.local/bin/ofolder",
                "--hook",
                "auto",
            ]
        );
        assert_eq!(
            unregister_args(),
            ["init", "--remove", "--name", "ofolder", "--hook", "auto"]
        );
    }

    #[test]
    fn each_progress_line_becomes_the_words_a_person_reads() {
        assert_eq!(
            parse_event(r#"{"event":"downloading","asset":"a","bytes":50,"total":200}"#),
            Event::Working {
                text: "Downloading 25%".to_owned(),
                fraction: Some(0.25),
            }
        );
        assert_eq!(
            parse_event(r#"{"event":"downloading","asset":"a","bytes":0,"total":null}"#),
            Event::Working {
                text: "Downloading".to_owned(),
                fraction: None,
            }
        );
        assert_eq!(
            parse_event(r#"{"event":"done","version":"0.2.0","restarted":true}"#),
            Event::Done { restarted: true }
        );
        assert_eq!(
            parse_event(r#"{"event":"needs_terminal","message":"No polkit agent."}"#),
            Event::NeedsTerminal {
                message: "No polkit agent.".to_owned()
            }
        );
        assert_eq!(
            parse_event(r#"{"event":"error","code":"checksum_mismatch","message":""}"#),
            Event::Failed {
                code: "checksum_mismatch".to_owned(),
                message: "The update did not finish.".to_owned(),
            }
        );
        assert_eq!(parse_event(r#"{"event":"detached"}"#), Event::Ignored);
        assert_eq!(parse_event("not json"), Event::Ignored);
    }

    #[test]
    fn a_progress_file_read_in_pieces_gives_each_event_once_and_waits_for_a_whole_line() {
        let mut tail = Tail::default();
        assert!(tail.feed(br#"{"event":"resolving""#).is_empty());
        let events = tail.feed(b"}\n{\"event\":\"done\",\"restarted\":false}\n");
        assert_eq!(
            events,
            [
                Event::Working {
                    text: "Getting ready".to_owned(),
                    fraction: None,
                },
                Event::Done { restarted: false },
            ]
        );
        assert!(tail.feed(b"").is_empty());
        let fed = br#"{"event":"resolving""#.len()
            + b"}\n{\"event\":\"done\",\"restarted\":false}\n".len();
        assert_eq!(
            tail.offset(),
            fed as u64,
            "every byte taken counts, even an unfinished line"
        );
    }
}
