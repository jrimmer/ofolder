//! Typing into a dialog, with the checks that make it safe.
//!
//! `wtype` sends keys to whichever window has focus. Every step of typing is
//! therefore preceded by a check that the dialog's window is the active one,
//! and the whole thing stops the moment it is not.

use std::path::Path;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use ofolder_core::adapter::AdapterError;
use ofolder_core::pathsafe::check_typeable;
use ofolder_core::system::{Compositor, Keys, SystemError, Typist};

pub fn backend(error: SystemError) -> AdapterError {
    AdapterError::Backend(error.0)
}

pub struct KeyGuard {
    compositor: Arc<dyn Compositor>,
    typist: Arc<dyn Typist>,
    /// The wait between steps, so the dialog can act on each.
    pace: Duration,
}

impl KeyGuard {
    pub fn new(compositor: Arc<dyn Compositor>, typist: Arc<dyn Typist>, pace: Duration) -> Self {
        Self {
            compositor,
            typist,
            pace,
        }
    }

    pub fn pace(&self) -> Duration {
        self.pace
    }

    pub fn set_pace(&mut self, pace: Duration) {
        self.pace = pace;
    }

    /// Succeeds only when `address` has keyboard focus. With `focus` it first
    /// asks the compositor to give it focus and waits for that to land.
    pub fn ensure_active(&self, address: &str, focus: bool) -> Result<(), AdapterError> {
        if focus {
            self.compositor.focus(address).map_err(backend)?;
        }
        let tries = if focus { 8 } else { 1 };
        for attempt in 0..tries {
            let active = self.compositor.active_window().map_err(backend)?;
            if active.as_deref() == Some(address) {
                return Ok(());
            }
            if attempt + 1 < tries {
                sleep(self.pace / 2);
            }
        }
        Err(AdapterError::Refused(
            "the dialog is not the active window, so nothing was typed".to_owned(),
        ))
    }

    /// Ctrl+L, the path, Return, into the window `window()` names, checking
    /// before every step. `window` is asked each time so a dialog that
    /// vanishes mid-way is noticed.
    pub fn type_path(
        &self,
        window: &dyn Fn() -> Option<String>,
        folder: &Path,
    ) -> Result<(), AdapterError> {
        check_typeable(folder).map_err(|r| AdapterError::Refused(r.to_string()))?;
        let text = folder
            .to_str()
            .ok_or_else(|| AdapterError::Refused("the path is not valid UTF-8".to_owned()))?
            .to_owned();
        self.ensure_active(&window().ok_or(AdapterError::NotRunning)?, true)?;
        for keys in [Keys::CtrlL, Keys::Text(text), Keys::Return] {
            self.ensure_active(&window().ok_or(AdapterError::NotRunning)?, false)?;
            self.typist.send(&keys).map_err(backend)?;
            sleep(self.pace);
        }
        Ok(())
    }

    /// Like [`type_path`](Self::type_path), but the path is typed only once
    /// `bar_open` says the path bar is open. Some dialogs keep the keyboard in
    /// a file name field that swallows Ctrl+L, and a path typed there would
    /// replace the name. So: Ctrl+L, look; if still closed, Tab (which hands
    /// the keyboard to the list) and Ctrl+L once more, look again. A bar that
    /// is still closed, or a dialog that does not say, ends in a refusal with
    /// nothing typed.
    pub fn type_path_via_open_bar(
        &self,
        window: &dyn Fn() -> Option<String>,
        folder: &Path,
        bar_open: &dyn Fn() -> Option<bool>,
    ) -> Result<(), AdapterError> {
        check_typeable(folder).map_err(|r| AdapterError::Refused(r.to_string()))?;
        let text = folder
            .to_str()
            .ok_or_else(|| AdapterError::Refused("the path is not valid UTF-8".to_owned()))?
            .to_owned();
        self.ensure_active(&window().ok_or(AdapterError::NotRunning)?, true)?;
        let send = |keys: &Keys| -> Result<(), AdapterError> {
            self.ensure_active(&window().ok_or(AdapterError::NotRunning)?, false)?;
            self.typist.send(keys).map_err(backend)
        };
        let mut seen = None;
        for attempt in [&[Keys::CtrlL][..], &[Keys::Tab, Keys::CtrlL][..]] {
            for keys in attempt {
                send(keys)?;
                sleep(self.pace);
            }
            for _ in 0..6 {
                seen = bar_open();
                if seen == Some(true) {
                    break;
                }
                sleep(self.pace / 2);
            }
            if seen == Some(true) {
                break;
            }
        }
        match seen {
            Some(true) => {}
            Some(false) => {
                return Err(AdapterError::Refused(
                    "the dialog's path bar did not open, so no path was typed".to_owned(),
                ));
            }
            None => {
                return Err(AdapterError::Refused(
                    "the dialog does not say whether its path bar is open, so no path was typed"
                        .to_owned(),
                ));
            }
        }
        for keys in [Keys::Text(text), Keys::Return] {
            send(&keys)?;
            sleep(self.pace);
        }
        Ok(())
    }

    /// Escape, if and only if the dialog is the active window.
    pub fn escape(&self, address: &str) -> Result<(), AdapterError> {
        self.ensure_active(address, true)?;
        self.typist.send(&Keys::Escape).map_err(backend)
    }
}
