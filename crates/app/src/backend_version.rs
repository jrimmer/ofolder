//! Learning which version of the backend ofolder is forwarding to, and saying once when it is one
//! nobody tried (`ofolder_core::known`).
//!
//! The portal asks the backend for its version once, when it starts and off the thread that serves
//! dialogs, and records the answer in a small file. A strip only reads that file, so opening a
//! dialog never waits on another program.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use ofolder_core::exec::run_with_deadline;
use ofolder_core::known;
use ofolder_core::private_file::write_private_atomic;

/// How long a backend may take to print its version.
const PROBE_LIMIT: Duration = Duration::from_secs(3);

fn state_file(state_dir: &Path) -> PathBuf {
    state_dir.join("ofolder/backend-version")
}

/// The program that reports the version of `backend`, when it has one.
fn program_for(backend: &str, home: &Path) -> Option<String> {
    match backend {
        "flea" => Some("flea".to_owned()),
        "strata" => {
            let own = home.join(".local/bin/strata");
            Some(if own.is_file() {
                own.to_string_lossy().into_owned()
            } else {
                "strata".to_owned()
            })
        }
        _ => None,
    }
}

/// The version of `backend` as the program itself reports it, when it has one to ask.
pub fn installed(backend: &str, home: &Path) -> Option<String> {
    ask(&program_for(backend, home)?)
}

/// What `program --version` says, as a version number, if it answers in time.
pub fn ask(program: &str) -> Option<String> {
    let out = run_with_deadline(Command::new(program).arg("--version"), PROBE_LIMIT).ok()?;
    known::version_of(&String::from_utf8_lossy(&out.stdout))
}

/// Records `backend` and its version, or removes the record when none can be learned, so a stale
/// answer about a backend that was since replaced is never shown.
pub fn record(state_dir: &Path, backend: &str, version: Option<&str>) {
    let path = state_file(state_dir);
    match version {
        Some(v) => {
            write_private_atomic(&path, format!("{backend} {v}\n").as_bytes()).ok();
        }
        None => {
            std::fs::remove_file(&path).ok();
        }
    }
}

/// Starts the one-off probe for the backend the portal forwards to.
pub fn start_probe(backend: String, home: PathBuf, state_dir: PathBuf) {
    if std::env::var_os("OFOLDER_NO_VERSION_PROBE").is_some() {
        return;
    }
    // The GTK portal has no program of its own to ask; its toolkit's launcher reports the GTK
    // version, which is as near as can be had and is only shown, never judged.
    let program = program_for(&backend, &home)
        .or_else(|| (backend == "gtk").then(|| "gtk4-launch".to_owned()));
    let Some(program) = program else {
        record(&state_dir, &backend, None);
        return;
    };
    std::thread::spawn(move || record(&state_dir, &backend, ask(&program).as_deref()));
}

/// The recorded `(backend, version)`.
pub fn recorded(state_dir: &Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(state_file(state_dir)).ok()?;
    let (backend, version) = text.trim().split_once(' ')?;
    Some((backend.to_owned(), version.to_owned()))
}

/// The line a strip for `label` should show once, and the file whose existence says it was shown.
pub fn note_for(label: &str, state_dir: &Path) -> Option<(String, PathBuf)> {
    let (backend, version) = recorded(state_dir)?;
    if backend != label {
        return None;
    }
    let note = known::untested_note(&backend, &version)?;
    let marker = state_dir.join(format!("ofolder/version-noted-{backend}-{version}"));
    Some((note, marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ofolder-bv-{name}-{}", std::process::id()));
        fs::remove_dir_all(&d).ok();
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn fake_program(dir: &Path, prints: &str) -> String {
        let path = dir.join("fake-backend");
        fs::write(&path, format!("#!/bin/sh\necho '{prints}'\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn the_version_a_program_prints_is_recorded_and_read_back() {
        let d = dir("record");
        let program = fake_program(&d, "flea 9.9.9");
        assert_eq!(ask(&program).as_deref(), Some("9.9.9"));
        record(&d, "flea", ask(&program).as_deref());
        assert_eq!(recorded(&d), Some(("flea".into(), "9.9.9".into())));
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_program_that_is_not_there_leaves_no_stale_record() {
        let d = dir("stale");
        record(&d, "flea", Some("0.4.0"));
        assert!(recorded(&d).is_some());
        assert_eq!(ask("/nonexistent/flea"), None);
        record(&d, "flea", None);
        assert_eq!(recorded(&d), None);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_strip_gets_a_note_only_for_its_own_backend_and_only_for_an_untried_version() {
        let d = dir("note");
        record(&d, "flea", Some("0.5.0"));
        let (note, marker) = note_for("flea", &d).unwrap();
        assert!(note.starts_with("Flea 0.5.0 has not been tried"), "{note}");
        assert!(marker.ends_with("ofolder/version-noted-flea-0.5.0"));
        assert!(
            note_for("strata", &d).is_none(),
            "a note for another backend's dialog"
        );
        record(&d, "flea", Some("0.4.0"));
        assert!(
            note_for("flea", &d).is_none(),
            "a note for a version that was tried"
        );
        fs::remove_dir_all(&d).ok();
    }
}
