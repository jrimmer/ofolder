//! Typing into a dialog, with the checks that make it safe.
//!
//! `wtype` sends keys to whichever window has focus. Every group of keys is therefore preceded by
//! giving the dialog's window the keyboard and checking that it holds it, and the whole thing
//! stops the moment it does not.
//!
//! With focus-follows-mouse the keyboard follows the pointer, and while a click on a rail is being
//! answered the pointer sits on that rail. So the keyboard is taken right before each group and the
//! pointer is put back right after it, not once for the whole sequence: a pointer that moves on
//! between two groups takes the keyboard away, and the next group simply takes it again. Keeping
//! the pointer on the dialog throughout would turn a quick second click on the rail into a click
//! in the dialog's file list.

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

/// How long the pointer stays on the dialog after a group of keys is sent, so the compositor has
/// delivered them before the pointer, and with it the keyboard, goes back.
const SETTLE: Duration = Duration::from_millis(40);

/// How many times the keyboard is looked at before the dialog is given up on.
const LOOKS: usize = 8;

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

    fn settle(&self) -> Duration {
        self.pace.min(SETTLE)
    }

    /// Gives `address` the keyboard and succeeds only once it holds it, asking again twice if it
    /// does not (a pointer resting on another window of ours can take the keyboard back). Returns
    /// where the pointer was, for [`give_back`](Self::give_back). `step` names where in a sequence
    /// this is, for the line a refusal leaves in the journal. On a refusal the pointer is already
    /// back.
    fn aim(&self, step: &str, address: &str) -> Result<Option<(i32, i32)>, AdapterError> {
        let home = self
            .compositor
            .focus_leaving_pointer(address)
            .map_err(backend)?;
        match self.hold(step, address) {
            Ok(()) => Ok(home),
            Err(refused) => {
                self.give_back(home);
                Err(refused)
            }
        }
    }

    fn hold(&self, step: &str, address: &str) -> Result<(), AdapterError> {
        let mut asked_again = 0;
        let mut seen: Vec<Option<String>> = Vec::new();
        for attempt in 0..LOOKS {
            let active = self.compositor.active_window().map_err(backend)?;
            if active.as_deref() == Some(address) {
                return Ok(());
            }
            seen.push(active);
            if attempt + 1 < LOOKS {
                sleep(self.pace / 2);
                if attempt == 2 || attempt == 5 {
                    self.compositor
                        .focus_leaving_pointer(address)
                        .map_err(backend)?;
                    asked_again += 1;
                }
            }
        }
        self.report_focus_miss(step, address, &seen, asked_again);
        Err(AdapterError::Refused(
            "the dialog is not the active window, so nothing was typed".to_owned(),
        ))
    }

    /// Puts the pointer back where [`aim`](Self::aim) found it.
    fn give_back(&self, home: Option<(i32, i32)>) {
        if let Some((x, y)) = home {
            self.compositor.move_pointer(x, y).ok();
        }
    }

    /// One group of keys: the dialog is given the keyboard and checked, the keys go to it, and only
    /// then does the pointer go back.
    fn send_aimed(&self, step: &str, address: &str, keys: &Keys) -> Result<(), AdapterError> {
        let home = self.aim(step, address)?;
        let sent = self.typist.send(keys).map_err(backend);
        sleep(self.settle());
        self.give_back(home);
        sent
    }

    /// The wait that lets the dialog act on a group, after the time already spent settling.
    fn pause_after_keys(&self) {
        sleep(self.pace.saturating_sub(self.settle()));
    }

    /// One journal line saying which window held the keyboard instead of the dialog, by class only
    /// (never a title), so a refusal can be explained afterwards.
    fn report_focus_miss(
        &self,
        step: &str,
        address: &str,
        seen: &[Option<String>],
        asked_again: u32,
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
        eprintln!(
            "ofolder: focus check failed at {step}: wanted {} ({address}), keyboard held by [{}] over {} look(s), asked again {asked_again}x",
            class_of(address),
            holders.join(" > "),
            seen.len()
        );
    }

    /// Ctrl+L, the path, Return, into the window `window()` names, aiming at it before every
    /// group. `window` is asked each time so a dialog that vanishes mid-way is noticed.
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
        for keys in [Keys::CtrlL, Keys::Text(text), Keys::Return] {
            let address = window().ok_or(AdapterError::NotRunning)?;
            self.send_aimed("keys", &address, &keys)?;
            self.pause_after_keys();
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
        let send = |keys: &Keys| -> Result<(), AdapterError> {
            let address = window().ok_or(AdapterError::NotRunning)?;
            self.send_aimed("keys", &address, keys)
        };
        let mut seen = None;
        for attempt in [&[Keys::CtrlL][..], &[Keys::Tab, Keys::CtrlL][..]] {
            for keys in attempt {
                send(keys)?;
                self.pause_after_keys();
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
            self.pause_after_keys();
        }
        Ok(())
    }

    /// Escape, if and only if the dialog can be given the keyboard.
    pub fn escape(&self, address: &str) -> Result<(), AdapterError> {
        self.send_aimed("escape", address, &Keys::Escape)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ofolder_core::hypr::{Client, Monitor};
    use std::sync::Mutex;

    type Log = Arc<Mutex<Vec<String>>>;

    /// A compositor whose keyboard follows the pointer: putting the pointer back takes the keyboard
    /// from the dialog, as on a desktop with follow-mouse. The first `ignore` requests for focus do
    /// nothing, as when something else has just taken it.
    struct Follows {
        log: Log,
        ignore: usize,
        asked: Mutex<usize>,
        active: Mutex<Option<String>>,
    }

    impl Follows {
        fn new(log: &Log, ignore: usize) -> Arc<Self> {
            Arc::new(Self {
                log: log.clone(),
                ignore,
                asked: Mutex::new(0),
                active: Mutex::new(Some("0xrail".to_owned())),
            })
        }
        fn held(&self) -> String {
            self.active
                .lock()
                .expect("lock")
                .clone()
                .unwrap_or_default()
        }
    }

    impl Compositor for Follows {
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
            self.focus_leaving_pointer(address).map(|_| ())
        }
        fn focus_leaving_pointer(&self, address: &str) -> Result<Option<(i32, i32)>, SystemError> {
            self.log.lock().expect("lock").push("focus".to_owned());
            let mut asked = self.asked.lock().expect("lock");
            *asked += 1;
            if *asked > self.ignore {
                *self.active.lock().expect("lock") = Some(address.to_owned());
            }
            Ok(Some((7, 9)))
        }
        fn move_pointer(&self, x: i32, y: i32) -> Result<(), SystemError> {
            self.log
                .lock()
                .expect("lock")
                .push(format!("pointer {x},{y}"));
            *self.active.lock().expect("lock") = Some("0xrail".to_owned());
            Ok(())
        }
    }

    /// Notes each group of keys with the window that held the keyboard when it was sent.
    struct Notes {
        log: Log,
        compositor: Arc<Follows>,
    }

    impl Typist for Notes {
        fn send(&self, keys: &Keys) -> Result<(), SystemError> {
            self.log
                .lock()
                .expect("lock")
                .push(format!("keys {keys:?} held={}", self.compositor.held()));
            Ok(())
        }
    }

    fn guard(ignore: usize) -> (KeyGuard, Arc<Follows>, Log) {
        let log: Log = Arc::default();
        let compositor = Follows::new(&log, ignore);
        let typist = Arc::new(Notes {
            log: log.clone(),
            compositor: compositor.clone(),
        });
        (
            KeyGuard::new(compositor.clone(), typist, Duration::ZERO),
            compositor,
            log,
        )
    }

    fn dialog() -> Option<String> {
        Some("0xdialog".to_owned())
    }

    fn lines(log: &Log) -> Vec<String> {
        log.lock().expect("lock").clone()
    }

    #[test]
    fn every_group_of_keys_is_aimed_at_the_dialog_and_the_pointer_goes_back_only_after_it() {
        let (guard, _, log) = guard(0);
        guard
            .type_path(&dialog, Path::new("/tmp/x"))
            .expect("the dialog takes the keyboard each time");
        let want: Vec<String> = [
            "focus",
            "keys CtrlL held=0xdialog",
            "pointer 7,9",
            "focus",
            "keys Text(\"/tmp/x\") held=0xdialog",
            "pointer 7,9",
            "focus",
            "keys Return held=0xdialog",
            "pointer 7,9",
        ]
        .map(str::to_owned)
        .into();
        assert_eq!(lines(&log), want);
    }

    #[test]
    fn a_keyboard_taken_back_between_groups_is_taken_again_for_the_next() {
        let (guard, compositor, _) = guard(0);
        guard
            .type_path(&dialog, Path::new("/tmp/x"))
            .expect("typed");
        // The last pointer move took the keyboard to the rail, and no group was sent to it.
        assert_eq!(compositor.held(), "0xrail");
    }

    #[test]
    fn a_request_for_focus_that_did_not_hold_is_asked_again_before_giving_up() {
        let (guard, _, log) = guard(1);
        guard
            .type_path(&dialog, Path::new("/tmp/x"))
            .expect("the second request lands");
        let sent: Vec<String> = lines(&log)
            .into_iter()
            .filter(|l| l.starts_with("keys"))
            .collect();
        assert_eq!(sent.len(), 3);
        assert!(
            sent.iter().all(|l| l.ends_with("held=0xdialog")),
            "{sent:?}"
        );
    }

    #[test]
    fn focus_that_never_lands_sends_nothing_and_gives_the_pointer_back() {
        let (guard, _, log) = guard(usize::MAX);
        let err = guard
            .type_path(&dialog, Path::new("/tmp/x"))
            .expect_err("must refuse");
        assert!(matches!(err, AdapterError::Refused(_)), "{err:?}");
        assert_eq!(
            lines(&log),
            ["focus", "focus", "focus", "pointer 7,9"],
            "the first request, two more, and the pointer back; no keys"
        );
    }

    #[test]
    fn the_open_bar_route_aims_every_group_too() {
        let (guard, _, log) = guard(0);
        guard
            .type_path_via_open_bar(&dialog, Path::new("/tmp/x"), &|| Some(true))
            .expect("typed");
        let sent: Vec<String> = lines(&log)
            .into_iter()
            .filter(|l| l.starts_with("keys"))
            .collect();
        assert_eq!(sent.len(), 3, "{sent:?}");
        assert!(
            sent.iter().all(|l| l.ends_with("held=0xdialog")),
            "{sent:?}"
        );
    }

    #[test]
    fn escape_is_aimed_like_any_other_keys() {
        let (guard, _, log) = guard(0);
        guard.escape("0xdialog").expect("sent");
        assert_eq!(
            lines(&log),
            ["focus", "keys Escape held=0xdialog", "pointer 7,9"]
        );
    }
}
