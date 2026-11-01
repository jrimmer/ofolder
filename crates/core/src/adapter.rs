//! What every dialog adapter must provide, and what each known one can do.

use std::path::{Path, PathBuf};

/// What an adapter can do. The strip turns off whatever an adapter cannot,
/// rather than guessing: a missing capability is a normal answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// The dialog's current folder can be read from outside.
    pub read_folder: bool,
    /// The dialog can be moved to any folder without sending keystrokes.
    pub goto_without_keys: bool,
    /// The dialog has its own "new folder" button.
    pub native_new_folder: bool,
    /// The dialog's window opens tiled, so it must be floated before docking.
    pub opens_tiled: bool,
}

/// The dialog the calling application asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogRequest {
    pub folder: PathBuf,
    pub suggested_name: String,
    pub title: String,
}

/// A window as the compositor knows it. The address is deliberately not an
/// identity: Hyprland reuses addresses, so a new dialog can inherit a dead one's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRef {
    pub address: String,
    pub pid: u32,
    pub class: String,
    pub title: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("this dialog cannot {0}")]
    Unsupported(&'static str),
    #[error("the dialog is not running")]
    NotRunning,
    #[error("refused: {0}")]
    Refused(String),
    #[error("the backend failed: {0}")]
    Backend(String),
}

/// One dialog backend. Implementations hold all backend-specific knowledge;
/// the strip never reaches past this trait.
pub trait Adapter: Send {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;
    fn launch(&mut self, request: &DialogRequest) -> Result<(), AdapterError>;
    /// The window of the dialog this adapter launched, or `None`.
    fn window(&self) -> Option<WindowRef>;
    /// The folder the dialog is showing. `None` means unknown, never a guess.
    fn read_folder(&self) -> Option<PathBuf>;
    /// Asks the dialog to show `folder`. The caller verifies arrival with
    /// [`Adapter::read_folder`]; a request being issued is not arrival.
    fn goto(&mut self, folder: &Path) -> Result<(), AdapterError>;
    fn close(&mut self) -> Result<(), AdapterError>;
    fn alive(&self) -> bool;

    /// Called once a move has been seen to land. Some dialogs are moved by
    /// borrowing a field, and this is where it is given back.
    fn after_arrival(&mut self) {}
}

/// A reading is trusted once two in a row agree, so a dialog caught
/// mid-move shows as unknown instead of as a stale folder. Sleeps `gap`
/// between readings.
pub fn stable_folder(
    adapter: &dyn Adapter,
    tries: u32,
    gap: std::time::Duration,
) -> Option<PathBuf> {
    crate::readers::first_agreeing((0..tries).map(|i| {
        if i > 0 {
            std::thread::sleep(gap);
        }
        adapter.read_folder()
    }))
}

/// Moves the dialog to `folder` and waits until it is seen to be there, for
/// at most `within`. Issuing a move is not arriving: a dialog that is still
/// starting or has its path bar in an error state accepts the keys and goes
/// nowhere, so every caller that acts on arrival uses this. Dialogs report the
/// resolved path of a symlinked folder, so that counts as arrival too.
pub fn goto_verified(
    adapter: &mut dyn Adapter,
    folder: &Path,
    within: std::time::Duration,
) -> Result<std::time::Duration, AdapterError> {
    let start = std::time::Instant::now();
    adapter.goto(folder)?;
    let resolved = folder.canonicalize().ok();
    loop {
        let shown = stable_folder(adapter, 3, std::time::Duration::from_millis(20));
        if shown.as_deref() == Some(folder) || (shown.is_some() && shown == resolved) {
            adapter.after_arrival();
            return Ok(start.elapsed());
        }
        if start.elapsed() >= within {
            return Err(AdapterError::Backend(format!(
                "the dialog did not arrive at {} (it shows {})",
                folder.display(),
                shown.map_or_else(
                    || "nothing readable".to_owned(),
                    |p| p.display().to_string()
                )
            )));
        }
    }
}

/// The capabilities measured on 2026-11-01 (see `docs/adapters.md`). Unknown
/// backends get none, so they are left undecorated instead of mishandled.
pub fn known_capabilities(adapter_id: &str) -> Capabilities {
    match adapter_id {
        "flea" => Capabilities {
            read_folder: true,
            goto_without_keys: false,
            native_new_folder: false,
            opens_tiled: false,
        },
        "gtk" => Capabilities {
            read_folder: true,
            goto_without_keys: true,
            native_new_folder: true,
            opens_tiled: false,
        },
        // Places and the parent button are clickable, but an arbitrary path
        // needs Ctrl+L, so it is not keyless.
        "strata" => Capabilities {
            read_folder: true,
            goto_without_keys: false,
            native_new_folder: true,
            opens_tiled: true,
        },
        _ => Capabilities::default(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;

    /// An adapter whose folder reads come from a script.
    struct Scripted(Mutex<VecDeque<Option<PathBuf>>>);

    impl Scripted {
        fn new(readings: &[Option<&str>]) -> Self {
            Self(Mutex::new(
                readings.iter().map(|r| r.map(PathBuf::from)).collect(),
            ))
        }
    }

    impl Adapter for Scripted {
        fn id(&self) -> &'static str {
            "scripted"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        fn launch(&mut self, _: &DialogRequest) -> Result<(), AdapterError> {
            Ok(())
        }
        fn window(&self) -> Option<WindowRef> {
            None
        }
        fn read_folder(&self) -> Option<PathBuf> {
            self.0.lock().expect("lock").pop_front().flatten()
        }
        fn goto(&mut self, _: &Path) -> Result<(), AdapterError> {
            Ok(())
        }
        fn close(&mut self) -> Result<(), AdapterError> {
            Ok(())
        }
        fn alive(&self) -> bool {
            true
        }
    }

    /// A dialog that shows whatever it was last sent to, unless it is stuck.
    struct Mover {
        shown: Mutex<PathBuf>,
        stuck: bool,
    }

    impl Adapter for Mover {
        fn id(&self) -> &'static str {
            "mover"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        fn launch(&mut self, _: &DialogRequest) -> Result<(), AdapterError> {
            Ok(())
        }
        fn window(&self) -> Option<WindowRef> {
            None
        }
        fn read_folder(&self) -> Option<PathBuf> {
            Some(self.shown.lock().expect("lock").clone())
        }
        fn goto(&mut self, folder: &Path) -> Result<(), AdapterError> {
            if !self.stuck {
                *self.shown.lock().expect("lock") = folder.to_path_buf();
            }
            Ok(())
        }
        fn close(&mut self) -> Result<(), AdapterError> {
            Ok(())
        }
        fn alive(&self) -> bool {
            true
        }
    }

    #[test]
    fn a_move_that_lands_is_reported_with_how_long_it_took() {
        let mut dialog = Mover {
            shown: Mutex::new(PathBuf::from("/a")),
            stuck: false,
        };
        let took = goto_verified(&mut dialog, Path::new("/b"), Duration::from_secs(1));
        assert!(took.is_ok(), "{took:?}");
    }

    #[test]
    fn a_move_that_is_issued_but_never_lands_is_an_error_naming_where_the_dialog_is() {
        let mut dialog = Mover {
            shown: Mutex::new(PathBuf::from("/a")),
            stuck: true,
        };
        let err = goto_verified(&mut dialog, Path::new("/b"), Duration::from_millis(150))
            .expect_err("must not claim arrival");
        let message = err.to_string();
        assert!(
            message.contains("/b") && message.contains("/a"),
            "{message}"
        );
    }

    #[test]
    fn a_dialog_that_reports_the_resolved_path_of_a_symlinked_folder_has_arrived() {
        let dir = std::env::temp_dir().join(format!("ofolder-core-link-{}", std::process::id()));
        let (target, link) = (dir.join("target"), dir.join("link"));
        std::fs::create_dir_all(&target).expect("dir");
        std::os::unix::fs::symlink(&target, &link).expect("link");
        let resolved = target.canonicalize().expect("canonical");
        // A dialog that reports the resolved path whatever it was sent.
        struct Resolving(PathBuf);
        impl Adapter for Resolving {
            fn id(&self) -> &'static str {
                "resolving"
            }
            fn capabilities(&self) -> Capabilities {
                Capabilities::default()
            }
            fn launch(&mut self, _: &DialogRequest) -> Result<(), AdapterError> {
                Ok(())
            }
            fn window(&self) -> Option<WindowRef> {
                None
            }
            fn read_folder(&self) -> Option<PathBuf> {
                Some(self.0.clone())
            }
            fn goto(&mut self, _: &Path) -> Result<(), AdapterError> {
                Ok(())
            }
            fn close(&mut self) -> Result<(), AdapterError> {
                Ok(())
            }
            fn alive(&self) -> bool {
                true
            }
        }
        let result = goto_verified(&mut Resolving(resolved), &link, Duration::from_secs(1));
        std::fs::remove_dir_all(&dir).expect("cleanup");
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn a_folder_read_twice_the_same_is_trusted() {
        let a = Scripted::new(&[Some("/a"), Some("/a")]);
        assert_eq!(
            stable_folder(&a, 5, Duration::ZERO),
            Some(PathBuf::from("/a"))
        );
    }

    #[test]
    fn a_folder_still_changing_when_the_tries_run_out_is_unknown() {
        let a = Scripted::new(&[Some("/a"), Some("/b"), Some("/a"), Some("/b")]);
        assert_eq!(stable_folder(&a, 4, Duration::ZERO), None);
    }

    #[test]
    fn a_dialog_that_settles_after_a_blank_moment_is_trusted_once_it_does() {
        let a = Scripted::new(&[None, Some("/b"), Some("/b")]);
        assert_eq!(
            stable_folder(&a, 5, Duration::ZERO),
            Some(PathBuf::from("/b"))
        );
    }

    #[test]
    fn flea_needs_the_strips_new_folder_because_it_has_none() {
        assert!(!known_capabilities("flea").native_new_folder);
    }

    #[test]
    fn only_the_gtk_dialog_can_be_moved_to_any_folder_without_keys() {
        let keyless: Vec<_> = ["flea", "gtk", "strata"]
            .into_iter()
            .filter(|id| known_capabilities(id).goto_without_keys)
            .collect();
        assert_eq!(keyless, ["gtk"]);
    }

    #[test]
    fn strata_opens_tiled_and_the_others_float() {
        assert!(known_capabilities("strata").opens_tiled);
        assert!(!known_capabilities("flea").opens_tiled);
        assert!(!known_capabilities("gtk").opens_tiled);
    }

    #[test]
    fn an_unknown_backend_can_do_nothing() {
        assert_eq!(known_capabilities("kde"), Capabilities::default());
    }
}
