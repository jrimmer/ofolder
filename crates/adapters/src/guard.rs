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
    /// asks the compositor to give it focus and waits for that to land, asking
    /// again twice if it does not (a pointer resting on another of our windows can
    /// take the keyboard back). `step` names where in a sequence the check is, for
    /// the line a refusal leaves in the journal.
    pub fn ensure_active(
        &self,
        step: &str,
        address: &str,
        focus: bool,
    ) -> Result<(), AdapterError> {
        if focus {
            self.compositor.focus(address).map_err(backend)?;
        }
        let tries = if focus { 8 } else { 1 };
        let pointer_before = self.compositor.cursor().ok();
        let mut asked_again = 0;
        let mut seen: Vec<Option<String>> = Vec::new();
        for attempt in 0..tries {
            let active = self.compositor.active_window().map_err(backend)?;
            if active.as_deref() == Some(address) {
                return Ok(());
            }
            seen.push(active);
            if attempt + 1 < tries {
                sleep(self.pace / 2);
                if focus && (attempt == 2 || attempt == 5) {
                    self.compositor.focus(address).map_err(backend)?;
                    asked_again += 1;
                }
            }
        }
        let pointer_after = self.compositor.cursor().ok();
        self.report_focus_miss(
            step,
            address,
            focus,
            &seen,
            asked_again,
            pointer_before.zip(pointer_after),
        );
        Err(AdapterError::Refused(
            "the dialog is not the active window, so nothing was typed".to_owned(),
        ))
    }

    /// One journal line saying which window held the keyboard instead of the dialog, by
    /// class only (never a title), so a refusal can be explained afterwards.
    fn report_focus_miss(
        &self,
        step: &str,
        address: &str,
        focus: bool,
        seen: &[Option<String>],
        asked_again: u32,
        pointer: Option<((i32, i32), (i32, i32))>,
    ) {
        let clients = self.compositor.clients().unwrap_or_default();
        let class_of = |addr: &str| {
            clients
                .iter()
                .find(|c| c.address == addr)
                .map_or("unknown", |c| c.class.as_str())
                .to_owned()
        };
        let mut holders: Vec<String> = Vec::new();
        for active in seen {
            let who = active
                .as_deref()
                .map_or_else(|| "none".to_owned(), class_of);
            if holders.last() != Some(&who) {
                holders.push(who);
            }
        }
        let pointer = match pointer {
            Some((a, b)) if a == b => "pointer still".to_owned(),
            Some(_) => "pointer moved".to_owned(),
            None => "pointer unknown".to_owned(),
        };
        eprintln!(
            "ofolder: focus check failed at {step}: wanted {} ({address}), keyboard held by [{}] over {} look(s), asked for focus: {focus} (and again {asked_again}x), {pointer}",
            class_of(address),
            holders.join(" > "),
            seen.len()
        );
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
        self.ensure_active("focus", &window().ok_or(AdapterError::NotRunning)?, true)?;
        for keys in [Keys::CtrlL, Keys::Text(text), Keys::Return] {
            self.ensure_active(
                "before keys",
                &window().ok_or(AdapterError::NotRunning)?,
                false,
            )?;
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
        self.ensure_active("focus", &window().ok_or(AdapterError::NotRunning)?, true)?;
        let send = |keys: &Keys| -> Result<(), AdapterError> {
            self.ensure_active(
                "before keys",
                &window().ok_or(AdapterError::NotRunning)?,
                false,
            )?;
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
        self.ensure_active("escape", address, true)?;
        self.typist.send(&Keys::Escape).map_err(backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ofolder_core::hypr::{Client, Monitor};
    use std::sync::Mutex;

    /// A compositor that ignores the first `ignore` requests for focus, as one does when a
    /// pointer resting on another window takes the keyboard straight back.
    struct Reluctant {
        ignore: usize,
        asked: Mutex<usize>,
        active: Mutex<Option<String>>,
    }

    impl Reluctant {
        fn new(ignore: usize) -> Arc<Self> {
            Arc::new(Self {
                ignore,
                asked: Mutex::new(0),
                active: Mutex::new(Some("0xrail".to_owned())),
            })
        }
        fn asked(&self) -> usize {
            *self.asked.lock().expect("lock")
        }
    }

    impl Compositor for Reluctant {
        fn clients(&self) -> Result<Vec<Client>, SystemError> {
            Ok(Vec::new())
        }
        fn monitors(&self) -> Result<Vec<Monitor>, SystemError> {
            Ok(Vec::new())
        }
        fn active_window(&self) -> Result<Option<String>, SystemError> {
            Ok(self.active.lock().expect("lock").clone())
        }
        fn focus(&self, address: &str) -> Result<(), SystemError> {
            let mut asked = self.asked.lock().expect("lock");
            *asked += 1;
            if *asked > self.ignore {
                *self.active.lock().expect("lock") = Some(address.to_owned());
            }
            Ok(())
        }
    }

    struct NoTypist;

    impl Typist for NoTypist {
        fn send(&self, _: &Keys) -> Result<(), SystemError> {
            Ok(())
        }
    }

    fn guard(compositor: Arc<Reluctant>) -> KeyGuard {
        KeyGuard::new(compositor, Arc::new(NoTypist), Duration::ZERO)
    }

    #[test]
    fn a_request_for_focus_that_did_not_hold_is_asked_again_before_giving_up() {
        let compositor = Reluctant::new(1);
        guard(compositor.clone())
            .ensure_active("test", "0xdialog", true)
            .expect("the second request lands");
        assert_eq!(compositor.asked(), 2);
    }

    #[test]
    fn focus_that_never_lands_is_refused_after_two_more_requests_and_nothing_else() {
        let compositor = Reluctant::new(usize::MAX);
        let err = guard(compositor.clone())
            .ensure_active("test", "0xdialog", true)
            .expect_err("must refuse");
        assert!(matches!(err, AdapterError::Refused(_)), "{err:?}");
        assert_eq!(compositor.asked(), 3, "the first request and two more");
    }

    #[test]
    fn a_check_that_does_not_ask_for_focus_never_asks() {
        let compositor = Reluctant::new(0);
        guard(compositor.clone())
            .ensure_active("test", "0xdialog", false)
            .expect_err("the rail holds focus");
        assert_eq!(compositor.asked(), 0);
    }
}
