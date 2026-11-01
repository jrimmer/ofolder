//! A dialog that lives in a resident backend process.
//!
//! The GTK portal and Strata host every dialog inside one long-running
//! process, so there is no child to watch. The dialog is opened by calling the
//! backend over D-Bus, and its window is found by its class and its title,
//! which the caller makes unique.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use ofolder_core::adapter::{AdapterError, DialogRequest, WindowRef};
use ofolder_core::system::Compositor;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, Value};

const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.impl.portal.FileChooser";

static NEXT_HANDLE: AtomicU32 = AtomicU32::new(1);

pub struct ResidentDialog {
    backend: &'static str,
    class: &'static str,
    title: String,
    compositor: Arc<dyn Compositor>,
    /// Set when the D-Bus call returns, which is when the dialog is closed.
    done: Arc<AtomicBool>,
    response: Arc<Mutex<Option<u32>>>,
    thread: Option<JoinHandle<()>>,
    /// The dialog was opened by someone else (the portal frontend, through
    /// the proxy), so it is only watched, never launched or waited for.
    attached: bool,
}

impl ResidentDialog {
    pub fn new(
        backend: &'static str,
        class: &'static str,
        title: String,
        compositor: Arc<dyn Compositor>,
    ) -> Self {
        Self {
            backend,
            class,
            title,
            compositor,
            done: Arc::new(AtomicBool::new(true)),
            response: Arc::new(Mutex::new(None)),
            thread: None,
            attached: false,
        }
    }

    /// Makes this watch a dialog somebody else opened.
    pub fn set_attached(&mut self) {
        self.attached = true;
    }

    pub fn is_attached(&self) -> bool {
        self.attached
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Opens the dialog by calling the backend. The call blocks until the
    /// dialog is closed, so it runs on its own thread.
    pub fn start(&mut self, request: &DialogRequest) -> Result<(), AdapterError> {
        if self.attached {
            return Err(AdapterError::Refused(
                "this dialog was opened by someone else".to_owned(),
            ));
        }
        if self.thread.is_some() && !self.done.load(Ordering::SeqCst) {
            return Err(AdapterError::Refused("a dialog is already open".to_owned()));
        }
        let folder = request
            .folder
            .to_str()
            .ok_or_else(|| AdapterError::Refused("the folder is not valid UTF-8".to_owned()))?
            .to_owned();
        let (backend, title, name) = (
            self.backend,
            self.title.clone(),
            request.suggested_name.clone(),
        );
        let (done, response) = (self.done.clone(), self.response.clone());
        done.store(false, Ordering::SeqCst);
        let handle = format!(
            "{PORTAL_PATH}/request/ofolder/{}_{}",
            std::process::id(),
            NEXT_HANDLE.fetch_add(1, Ordering::Relaxed)
        );
        self.thread = Some(std::thread::spawn(move || {
            let code = call_save_file(backend, &handle, &title, &folder, &name);
            *response.lock().expect("response lock") = code;
            done.store(true, Ordering::SeqCst);
        }));
        Ok(())
    }

    /// The dialog's window. Two windows with the same class and title cannot
    /// be told apart, so then there is no answer: acting on the wrong dialog
    /// is worse than not acting.
    pub fn window(&self) -> Option<WindowRef> {
        let mut matches = self
            .compositor
            .clients()
            .ok()?
            .into_iter()
            .filter(|c| c.class == self.class && c.title == self.title);
        let found = matches.next()?;
        matches.next().is_none().then_some(WindowRef {
            address: found.address,
            pid: found.pid,
            class: found.class,
            title: found.title,
        })
    }

    pub fn alive(&self) -> bool {
        if self.attached {
            // Someone else opened it, so it is alive as long as its window is.
            return self.window().is_some();
        }
        self.thread.is_some() && !self.done.load(Ordering::SeqCst)
    }

    /// The portal response code once the dialog has closed.
    pub fn response(&self) -> Option<u32> {
        *self.response.lock().expect("response lock")
    }
}

impl Drop for ResidentDialog {
    fn drop(&mut self) {
        // The thread ends when the dialog closes; joining a dialog that is
        // still open would hang, so an open one is left to finish by itself.
        if self.done.load(Ordering::SeqCst)
            && let Some(thread) = self.thread.take()
        {
            thread.join().ok();
        }
    }
}

/// The backend's `SaveFile`, returning the portal response code, or `None`
/// if the call itself failed.
fn call_save_file(
    backend: &str,
    handle: &str,
    title: &str,
    folder: &str,
    name: &str,
) -> Option<u32> {
    let connection = Connection::session().ok()?;
    let proxy = Proxy::new(&connection, backend, PORTAL_PATH, FILE_CHOOSER).ok()?;
    let mut folder_bytes = folder.as_bytes().to_vec();
    folder_bytes.push(0);
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("current_name", Value::from(name));
    options.insert("current_folder", Value::from(folder_bytes));
    let handle = ObjectPath::try_from(handle).ok()?;
    let reply = proxy
        .call_method("SaveFile", &(handle, "ofolder", "", title, options))
        .ok()?;
    let (code, _results): (u32, HashMap<String, zbus::zvariant::OwnedValue>) =
        reply.body().deserialize().ok()?;
    Some(code)
}

#[cfg(test)]
mod tests {
    use ofolder_core::hypr::{Client, Monitor, parse_clients};
    use ofolder_core::system::SystemError;

    use super::*;

    struct Windows(Vec<Client>);

    impl Compositor for Windows {
        fn clients(&self) -> Result<Vec<Client>, SystemError> {
            Ok(self.0.clone())
        }
        fn monitors(&self) -> Result<Vec<Monitor>, SystemError> {
            Ok(Vec::new())
        }
        fn active_window(&self) -> Result<Option<String>, SystemError> {
            Ok(None)
        }
        fn focus(&self, _: &str) -> Result<(), SystemError> {
            Ok(())
        }
    }

    fn client(address: &str, class: &str, title: &str) -> Client {
        let json = format!(
            r#"[{{"address":"{address}","class":"{class}","title":"{title}","pid":1,
            "at":[0,0],"size":[1,1],"floating":true,"workspace":{{"id":1,"name":"1"}},"monitor":0}}]"#
        );
        parse_clients(&json).expect("sample is valid").remove(0)
    }

    fn dialog(windows: Vec<Client>) -> ResidentDialog {
        ResidentDialog::new(
            "x.backend",
            "the.class",
            "Save as".to_owned(),
            Arc::new(Windows(windows)),
        )
    }

    #[test]
    fn the_window_is_found_by_class_and_title() {
        let d = dialog(vec![
            client("0xa", "other.class", "Save as"),
            client("0xb", "the.class", "Save as"),
        ]);
        assert_eq!(d.window().map(|w| w.address), Some("0xb".to_owned()));
    }

    #[test]
    fn two_windows_with_the_same_class_and_title_give_no_answer() {
        let d = dialog(vec![
            client("0xa", "the.class", "Save as"),
            client("0xb", "the.class", "Save as"),
        ]);
        assert_eq!(d.window(), None);
    }

    #[test]
    fn an_attached_dialog_is_alive_exactly_as_long_as_its_window_is() {
        let mut d = dialog(vec![client("0xa", "the.class", "Save as")]);
        d.set_attached();
        assert!(d.alive());
        let mut gone = dialog(Vec::new());
        gone.set_attached();
        assert!(!gone.alive());
    }

    #[test]
    fn a_dialog_that_was_never_launched_is_not_alive_even_if_a_window_matches() {
        assert!(!dialog(vec![client("0xa", "the.class", "Save as")]).alive());
    }

    #[test]
    fn an_attached_dialog_cannot_be_launched() {
        let mut d = dialog(Vec::new());
        d.set_attached();
        let request = DialogRequest {
            folder: std::path::PathBuf::from("/tmp"),
            suggested_name: String::new(),
            title: String::new(),
        };
        assert!(matches!(d.start(&request), Err(AdapterError::Refused(_))));
    }
}
