//! The process side of updates: running praeco, the portal's schedule of
//! checks, and the registration with `omarchy update`.
//!
//! The rules are in `ofolder_core::update`; this module only starts praeco
//! with the arguments those rules build, reads what it prints, and writes the
//! state file. praeco is never run through a shell, so a path or a version
//! with spaces or quotes reaches it unchanged.

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ofolder_core::settings;
use ofolder_core::update::{
    CheckResult, InstallPlan, POLL_SECS, RETRY_AFTER_FAILURE_SECS, Source, State, check_args,
    check_due, install_args, locate_praeco, parse_check, read_state, record_check, register_args,
    running_version, unregister_args,
};

/// How long a check may take. A slow network is reported as a failed check,
/// which the window does not show, rather than holding the portal's thread.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// Registering or removing takes a registry write and a version read.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(120);
/// `praeco install --detach` returns as soon as the copy that installs has started.
const DETACH_TIMEOUT: Duration = Duration::from_secs(60);
/// The first check comes this long after the portal starts, so that starting
/// a dialog is never slowed by a network request.
pub const FIRST_CHECK: Duration = Duration::from_secs(60);

/// What praeco printed and how it ended.
struct Ran {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Runs praeco with `args`, reading its output while it runs, and kills it if
/// it takes longer than `timeout`. No shell is involved.
fn run_praeco(praeco: &Path, args: &[OsString], timeout: Duration) -> Result<Ran, String> {
    let mut child = Command::new(praeco)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("praeco could not be started: {e}"))?;
    let stdout = child.stdout.take().map(read_all);
    let stderr = child.stderr.take().map(read_all);
    let start = Instant::now();
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                child.kill().ok();
                child.wait().ok();
                return Err("praeco took too long and was stopped".to_owned());
            }
            Err(e) => return Err(format!("praeco could not be waited for: {e}")),
        }
    };
    Ok(Ran {
        success,
        stdout: join_text(stdout),
        stderr: join_text(stderr),
    })
}

/// Reads a pipe to its end on a thread of its own, so a full pipe never stops praeco.
fn read_all<R: Read + Send + 'static>(mut pipe: R) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes).ok();
        bytes
    })
}

fn join_text(reader: Option<JoinHandle<Vec<u8>>>) -> String {
    reader
        .and_then(|r| r.join().ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

/// The last line of praeco's error output, which is its plain-language reason.
fn reason(ran: &Ran) -> String {
    ran.stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .or_else(|| ran.stdout.lines().rev().find(|l| !l.trim().is_empty()))
        .map_or_else(|| "praeco did not say why".to_owned(), str::to_owned)
}

/// Whether a file is an executable file, which is what praeco must be.
fn usable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The praeco to run: `OFOLDER_PRAECO` when set, else the one shipped with
/// this ofolder. `None` means the update UI is off.
pub fn praeco_path(exe: &Path) -> Option<PathBuf> {
    let env = std::env::var_os("OFOLDER_PRAECO").map(PathBuf::from);
    let exe_dir = exe.parent().unwrap_or_else(|| Path::new("/"));
    locate_praeco(env.as_deref(), exe_dir, usable)
}

/// The ofolder program and the praeco that installs it, or why there is none.
pub fn praeco_and_exe() -> Result<(PathBuf, PathBuf), String> {
    let exe = crate::own_exe().map_err(|e| format!("the program cannot find its own path: {e}"))?;
    let praeco = praeco_path(&exe)
        .ok_or_else(|| "praeco was not found beside ofolder, so updates are off".to_owned())?;
    Ok((exe, praeco))
}

/// The release source the environment names, or the built-in one.
pub fn source_from_env() -> Result<Source, String> {
    Source::from_env_value(std::env::var("OFOLDER_UPDATE_SOURCE").ok().as_deref())
        .map_err(|e| format!("OFOLDER_UPDATE_SOURCE {e}"))
}

/// The version the check compares against: `OFOLDER_PRETEND_VERSION` when set.
pub fn running_from_env() -> String {
    running_version(
        std::env::var("OFOLDER_PRETEND_VERSION").ok().as_deref(),
        env!("CARGO_PKG_VERSION"),
    )
}

/// The result of one check that ran.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Checked(State),
    Failed(String),
}

/// Everything a check needs, all of it given rather than read from the
/// environment, so the tests can drive the same code with a fake praeco.
#[derive(Clone, Debug)]
pub struct Updates {
    pub praeco: PathBuf,
    pub source: Source,
    pub state_path: PathBuf,
    /// The ofolder program praeco checks and installs.
    pub exe: PathBuf,
    /// The version the check compares against.
    pub running: String,
    pub timeout: Duration,
}

impl Updates {
    /// Asks praeco for the latest release and records the answer. A failure
    /// records nothing, so the state keeps the last good answer.
    pub fn check_now(&self, now: i64) -> Outcome {
        let args = check_args(&self.source, &self.running, &self.exe);
        let ran = match run_praeco(&self.praeco, &args, self.timeout) {
            Ok(ran) => ran,
            Err(why) => return Outcome::Failed(why),
        };
        let result = parse_check(&ran.stdout);
        match &result {
            CheckResult::Failed { message, .. } => Outcome::Failed(message.clone()),
            CheckResult::Answer { .. } => {
                match record_check(&self.state_path, now, &self.running, &result) {
                    Some(Ok(state)) => Outcome::Checked(state),
                    Some(Err(e)) => Outcome::Failed(format!("the check could not be saved: {e}")),
                    None => Outcome::Failed("praeco gave no answer".to_owned()),
                }
            }
        }
    }

    /// Checks when one is due (see `check_due`), and does nothing otherwise.
    /// Returns `None` when no check was due.
    pub fn check_if_due(&self, now: i64, forced: bool) -> Option<Outcome> {
        let last = read_state(&self.state_path).checked_at;
        check_due(last, now, forced).then(|| self.check_now(now))
    }
}

/// Registers ofolder with `omarchy update` through praeco. Only the command and
/// the settings window ask for this; nothing registers on its own.
pub fn register(praeco: &Path, source: &Source, exe: &Path) -> Result<String, String> {
    let ran = run_praeco(praeco, &register_args(source, exe), REGISTER_TIMEOUT)?;
    if ran.success {
        Ok("ofolder is registered with omarchy update.".to_owned())
    } else {
        Err(format!("praeco did not register ofolder: {}", reason(&ran)))
    }
}

/// Takes the registration away. A registration that is not there is not an error for praeco.
pub fn unregister(praeco: &Path) -> Result<String, String> {
    let ran = run_praeco(praeco, &unregister_args(), REGISTER_TIMEOUT)?;
    if ran.success {
        Ok("ofolder is no longer registered with omarchy update.".to_owned())
    } else {
        Err(format!(
            "praeco did not remove the registration: {}",
            reason(&ran)
        ))
    }
}

/// The folder the progress files go in: the runtime folder, which is private
/// to the user. A new progress file is named for this process and the moment.
pub fn progress_file_for_install() -> io::Result<PathBuf> {
    let dir = crate::keys::runtime_dir()?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    Ok(dir.join(format!("update-{}-{stamp}.jsonl", std::process::id())))
}

/// Starts praeco's detached install of `version`, which writes its progress to `progress` (a file
/// that must not exist yet). The portal is stopped and restarted by praeco; the restart runs
/// `ofolder status`, which only proves the new program starts.
pub fn start_install(
    praeco: &Path,
    source: &Source,
    version: &str,
    exe: &Path,
    portal_pid: u32,
    progress: &Path,
    terminal: bool,
) -> Result<(), String> {
    let plan = InstallPlan {
        source,
        version,
        exe,
        restart_pid: portal_pid,
        progress_file: progress,
        restart_program: exe,
        restart_args: &["status"],
        terminal,
    };
    let ran = run_praeco(praeco, &install_args(&plan), DETACH_TIMEOUT)?;
    if ran.success {
        Ok(())
    } else {
        Err(format!("the update could not start: {}", reason(&ran)))
    }
}

/// The portal this strip belongs to: a strip is started by the portal, so its
/// parent is the portal. The parent is only named when its command line is
/// `ofolder portal ...`, since praeco stops that process.
pub fn parent_portal_pid() -> Option<u32> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // The command name is in parentheses and may hold spaces, so fields are
    // counted from the last closing parenthesis: state, then the parent's pid.
    let after = &stat[stat.rfind(')')? + 1..];
    let ppid: u32 = after.split_whitespace().nth(1)?.parse().ok()?;
    let cmdline = std::fs::read(format!("/proc/{ppid}/cmdline")).ok()?;
    let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
    let is_ofolder = args
        .first()
        .is_some_and(|program| program.ends_with(b"ofolder"));
    (is_ofolder && args.get(1) == Some(&b"portal".as_slice())).then_some(ppid)
}

/// Starts the portal's checks: a first one `first_after` from now (with
/// `forced`, one regardless of the last), then a look every hour for one that
/// is due. Nothing is checked while the `updates` setting is off, and a
/// failed check is not tried again for a day.
pub fn spawn_schedule(
    updates: Updates,
    settings_path: PathBuf,
    home: PathBuf,
    first_after: Duration,
    forced: bool,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        std::thread::sleep(first_after);
        let mut forced = forced;
        let mut failed_at: Option<i64> = None;
        loop {
            let now = now_secs();
            let on = settings::load(&settings_path, &home).settings.updates;
            let retry_due =
                failed_at.is_none_or(|t| now < t || now - t >= RETRY_AFTER_FAILURE_SECS);
            if on && retry_due {
                match updates.check_if_due(now, forced) {
                    Some(Outcome::Failed(why)) => {
                        eprintln!("ofolder: the update check failed: {why}");
                        failed_at = Some(now);
                    }
                    Some(Outcome::Checked(_)) => failed_at = None,
                    None => {}
                }
            }
            forced = false;
            std::thread::sleep(Duration::from_secs(POLL_SECS));
        }
    })
}

/// Seconds since the epoch, as the state file keeps times.
pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The portal's update checks, set up from the environment: praeco, the source
/// and the delay. Nothing starts when praeco is missing, or the source
/// does not parse (which is reported, not guessed).
pub fn start_portal_checks(exe: &Path, home: &Path, settings_path: PathBuf) {
    let Some(praeco) = praeco_path(exe) else {
        return;
    };
    let source = match source_from_env() {
        Ok(source) => source,
        Err(why) => {
            eprintln!("ofolder: {why}; update checks are off");
            return;
        }
    };
    let (first_after, forced) = match std::env::var("OFOLDER_UPDATE_CHECK_DELAY") {
        Ok(text) => match text.trim().parse::<u64>() {
            Ok(secs) => (Duration::from_secs(secs), true),
            Err(_) => {
                eprintln!(
                    "ofolder: OFOLDER_UPDATE_CHECK_DELAY must be whole seconds; using the default"
                );
                (FIRST_CHECK, false)
            }
        },
        Err(_) => (FIRST_CHECK, false),
    };
    let updates = Updates {
        praeco,
        source,
        state_path: ofolder_core::update::state_path(
            std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
            home,
        ),
        exe: exe.to_path_buf(),
        running: running_from_env(),
        timeout: CHECK_TIMEOUT,
    };
    // Not joined: the schedule lives as long as the portal does.
    drop(spawn_schedule(
        updates,
        settings_path,
        home.to_path_buf(),
        first_after,
        forced,
    ));
}

/// `ofolder update register|unregister`: the same registration the settings
/// window makes, run by hand, and recorded in the settings file.
pub fn command(args: &[String], exe: &Path, settings_path: &Path) -> anyhow::Result<()> {
    let praeco = praeco_path(exe).ok_or_else(|| {
        anyhow::anyhow!("praeco was not found beside ofolder (OFOLDER_PRAECO names no executable)")
    })?;
    let source = source_from_env().map_err(anyhow::Error::msg)?;
    let (message, on) = match args.first().map(String::as_str) {
        Some("register") => (register(&praeco, &source, exe), true),
        Some("unregister") => (unregister(&praeco), false),
        _ => anyhow::bail!("usage: ofolder update register | unregister"),
    };
    let message = message.map_err(anyhow::Error::msg)?;
    ofolder_core::settings_write::save(
        settings_path,
        &[ofolder_core::settings_write::Edit::Bool(
            "update_with_system",
            on,
        )],
    )?;
    println!("{message}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use ofolder_core::update::{Event, Tail, modify_state, offered_version, read_state};

    /// The moment the fixtures check at.
    const NOW: i64 = 1_800_000_000;

    /// One release newer than the running version, with notes, as praeco's check prints it.
    const OFFER: &str = r#"{"ok":true,"source":"forgejo:https://forge.example/jrimmer/ofolder","current":"0.1.0","latest":"0.2.0","update_available":true,"releases":[{"version":"0.2.0","tag":"v0.2.0","name":"ofolder 0.2.0","published_at":"2026-10-24T09:12:00Z","url":"https://forge.example/jrimmer/ofolder/releases/tag/v0.2.0","prerelease":false,"notes":"Fixed\n- A thing."}],"install":{"kind":"tarball","supported":true,"reason":"r","detail":null,"target":"/home/u/.local/bin/ofolder","asset":null}}"#;

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// A folder with a copy of the fake praeco and the answers a test sets. It is removed when the
    /// fixture is dropped, and nothing in it is the user's.
    struct Fixture {
        dir: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Fixture {
            let n = NEXT.fetch_add(1, AtomicOrdering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("ofolder-update-{label}-{}-{n}", std::process::id()));
            fs::remove_dir_all(&dir).ok();
            fs::create_dir_all(&dir).expect("fixture folder");
            // A link, not a copy: a script written just now can fail to run ("text file busy")
            // while another test thread still holds it open for writing. The fake keeps its files in
            // the folder it was started from, which for a link is this folder.
            let script =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/support/fake-praeco");
            std::os::unix::fs::symlink(&script, dir.join("praeco"))
                .expect("the fake praeco is linked");
            Fixture { dir }
        }

        fn praeco(&self) -> PathBuf {
            self.dir.join("praeco")
        }

        /// Writes one of the fake's answer files.
        fn answer(&self, file: &str, text: &str) {
            fs::write(self.dir.join(file), text).expect("answer file");
        }

        /// Every invocation the fake recorded, each as its list of arguments.
        fn calls(&self) -> Vec<Vec<String>> {
            let log = fs::read_to_string(self.dir.join("calls.log")).unwrap_or_default();
            let mut calls = Vec::new();
            let mut current = Vec::new();
            for line in log.lines() {
                if line == "%%END%%" {
                    calls.push(std::mem::take(&mut current));
                } else {
                    current.push(line.to_owned());
                }
            }
            calls
        }

        fn state_path(&self) -> PathBuf {
            self.dir.join("state/update.json")
        }

        fn updates(&self, timeout: Duration) -> Updates {
            Updates {
                praeco: self.praeco(),
                source: Source::default_source(),
                state_path: self.state_path(),
                exe: PathBuf::from("/home/u/.local/bin/ofolder"),
                running: "0.1.0".to_owned(),
                timeout,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn offered(f: &Fixture) -> Option<String> {
        let state = read_state(&f.state_path());
        offered_version(&state, "0.1.0").map(str::to_owned)
    }

    #[test]
    fn a_due_check_writes_the_state_once_and_a_second_look_does_not_check_again() {
        let f = Fixture::new("once");
        f.answer("check.json", OFFER);
        let u = f.updates(Duration::from_secs(10));
        let first = u.check_if_due(NOW, false);
        assert!(
            matches!(&first, Some(Outcome::Checked(s)) if s.latest.as_deref() == Some("0.2.0"))
        );
        assert_eq!(
            u.check_if_due(NOW + 60, false),
            None,
            "a fresh check is not due again"
        );
        assert_eq!(read_state(&f.state_path()).checked_at, Some(NOW));
        let calls = f.calls();
        assert_eq!(calls.len(), 1, "praeco was run once: {calls:?}");
        assert_eq!(calls[0][0], "check");
        assert!(
            calls.iter().all(|c| c[0] != "init" && c[0] != "install"),
            "a check never registers or installs"
        );
    }

    #[test]
    fn a_failed_check_leaves_no_pill_and_no_state_and_does_not_panic() {
        let f = Fixture::new("failed");
        let u = f.updates(Duration::from_secs(10));
        assert_eq!(
            u.check_if_due(NOW, false),
            Some(Outcome::Failed("offline".to_owned()))
        );
        assert!(!f.state_path().exists());
        assert_eq!(offered(&f), None);
    }

    #[test]
    fn a_check_praeco_does_not_finish_in_time_is_stopped_and_reported() {
        let f = Fixture::new("slow");
        f.answer("check.json", OFFER);
        f.answer("check.sleep", "5");
        let u = f.updates(Duration::from_millis(300));
        let started = Instant::now();
        let outcome = u.check_now(NOW);
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "the wait is bounded"
        );
        assert!(
            matches!(&outcome, Outcome::Failed(why) if why.contains("too long")),
            "{outcome:?}"
        );
        assert!(!f.state_path().exists());
    }

    #[test]
    fn a_forced_check_runs_even_when_one_was_made_this_week() {
        let f = Fixture::new("forced");
        f.answer("check.json", OFFER);
        let u = f.updates(Duration::from_secs(10));
        assert!(matches!(
            u.check_if_due(NOW, false),
            Some(Outcome::Checked(_))
        ));
        assert!(matches!(
            u.check_if_due(NOW + 1, true),
            Some(Outcome::Checked(_))
        ));
        assert_eq!(f.calls().len(), 2);
    }

    #[test]
    fn later_hides_the_offer_until_the_next_check_and_skip_hides_that_version() {
        let f = Fixture::new("choices");
        f.answer("check.json", OFFER);
        let u = f.updates(Duration::from_secs(10));
        u.check_if_due(NOW, false);
        assert_eq!(offered(&f).as_deref(), Some("0.2.0"));

        modify_state(&f.state_path(), |s| s.later_checked_at = s.checked_at).expect("later");
        assert_eq!(offered(&f), None, "Later hides it");
        u.check_now(NOW + 5);
        assert_eq!(
            offered(&f).as_deref(),
            Some("0.2.0"),
            "the next check shows it again"
        );

        modify_state(&f.state_path(), |s| s.skipped = s.latest.clone()).expect("skip");
        assert_eq!(offered(&f), None, "Skip hides this version");
        f.answer("check.json", &OFFER.replace("0.2.0", "0.2.1"));
        u.check_now(NOW + 10);
        assert_eq!(
            offered(&f).as_deref(),
            Some("0.2.1"),
            "a newer release is offered"
        );
    }

    #[test]
    fn the_install_is_started_with_exactly_the_arguments_praeco_expects() {
        let f = Fixture::new("install");
        f.answer(
            "install-dialog.jsonl",
            "{\"event\":\"resolving\",\"version\":\"0.2.0\"}\n{\"event\":\"done\",\"version\":\"0.2.0\",\"restarted\":true}\n",
        );
        fs::create_dir(f.dir.join("progress dir")).expect("progress folder");
        let progress = f.dir.join("progress dir/update \"1\".jsonl");
        let exe = PathBuf::from("/home/u/My Apps/ofolder");
        start_install(
            &f.praeco(),
            &Source::default_source(),
            "0.2.0",
            &exe,
            4321,
            &progress,
            false,
        )
        .expect("started");

        let calls = f.calls();
        assert_eq!(calls.len(), 1);
        let progress_text = progress.display().to_string();
        assert_eq!(
            calls[0],
            [
                "install",
                "--forgejo",
                "https://forge.example",
                "jrimmer/ofolder",
                "--version",
                "0.2.0",
                "--exe",
                "/home/u/My Apps/ofolder",
                "--restart-pid",
                "4321",
                "--detach",
                "--progress-file",
                progress_text.as_str(),
                "--restart",
                "--",
                "/home/u/My Apps/ofolder",
                "status",
            ]
        );
        let events = Tail::default().feed(&fs::read(&progress).expect("progress file"));
        assert_eq!(events.last(), Some(&Event::Done { restarted: true }));
    }

    #[test]
    fn a_terminal_install_passes_the_terminal_mode_and_a_password_need_is_read_back() {
        let f = Fixture::new("terminal");
        f.answer(
            "install-terminal.jsonl",
            "{\"event\":\"needs_terminal\",\"message\":\"No polkit agent.\"}\n",
        );
        let progress = f.dir.join("update.jsonl");
        let exe = PathBuf::from("/home/u/.local/bin/ofolder");
        start_install(
            &f.praeco(),
            &Source::default_source(),
            "0.2.0",
            &exe,
            9,
            &progress,
            true,
        )
        .expect("started");
        let call = &f.calls()[0];
        let mode = call.iter().position(|a| a == "--mode").expect("mode flag");
        assert_eq!(call[mode + 1], "terminal");
        let events = Tail::default().feed(&fs::read(&progress).expect("progress file"));
        assert_eq!(
            events,
            [Event::NeedsTerminal {
                message: "No polkit agent.".to_owned()
            }]
        );
    }

    #[test]
    fn an_install_praeco_refuses_names_its_reason() {
        let f = Fixture::new("refused");
        f.answer("install.status", "1");
        f.answer(
            "install.stderr",
            "The update needs a restart it cannot do.\n",
        );
        let exe = PathBuf::from("/home/u/.local/bin/ofolder");
        let err = start_install(
            &f.praeco(),
            &Source::default_source(),
            "0.2.0",
            &exe,
            9,
            &f.dir.join("p.jsonl"),
            false,
        )
        .expect_err("refused");
        assert!(err.contains("restart it cannot do"), "{err}");
    }

    #[test]
    fn registering_calls_init_with_the_expected_arguments_and_removing_calls_remove() {
        let f = Fixture::new("register");
        let exe = PathBuf::from("/home/u/.local/bin/ofolder");
        register(&f.praeco(), &Source::default_source(), &exe).expect("registered");
        unregister(&f.praeco()).expect("removed");
        let calls = f.calls();
        assert_eq!(
            calls,
            [
                vec![
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
                ],
                vec!["init", "--remove", "--name", "ofolder", "--hook", "auto"],
            ]
        );
    }

    #[test]
    fn a_registration_praeco_refuses_says_why() {
        let f = Fixture::new("refuse-register");
        f.answer("init.stderr", "No registry folder is writable.\n");
        let exe = PathBuf::from("/home/u/.local/bin/ofolder");
        let err = register(&f.praeco(), &Source::default_source(), &exe).expect_err("refused");
        assert!(err.contains("No registry folder is writable."), "{err}");
    }

    #[test]
    fn the_checks_never_register_so_only_the_owner_s_choice_does() {
        let f = Fixture::new("no-register");
        f.answer("check.json", OFFER);
        let u = f.updates(Duration::from_secs(10));
        u.check_if_due(NOW, false);
        u.check_if_due(NOW + 1, true);
        assert!(f.calls().iter().all(|c| c[0] == "check"));
    }
}
