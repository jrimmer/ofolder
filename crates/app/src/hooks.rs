//! What the portal proxy does around each dialog: work out who asked, start a
//! strip for the dialog, make sure it is gone when the dialog is, and record
//! what the dialog returned.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ofolder_adapters::flea::picker_pids;
use ofolder_adapters::strata;
use ofolder_core::identity::{self, Identity};
use ofolder_core::settings;
use ofolder_core::system::Compositor;
use ofolder_portal::hooks::{Call, Hooks, Options};
use ofolder_portal::preseed;

use crate::keys;
use crate::recording::Recorder;

pub struct StripHooks {
    /// The `ofolder` binary to run for each strip.
    exe: PathBuf,
    /// The adapter id of the backend the proxy forwards to.
    backend: String,
    compositor: Arc<dyn Compositor>,
    settings_path: PathBuf,
    home: PathBuf,
    recorder: Recorder,
    /// Who asked for each open dialog, from `begin` to `end`.
    pending: Mutex<HashMap<String, Identity>>,
    /// Where strips record their key bindings, to remove those of a strip
    /// that had to be stopped. None leaves bindings to the next strip.
    runtime: Option<PathBuf>,
}

impl StripHooks {
    pub fn new(
        exe: PathBuf,
        backend: String,
        compositor: Arc<dyn Compositor>,
        settings_path: PathBuf,
        recents_path: PathBuf,
        home: PathBuf,
        runtime: Option<PathBuf>,
    ) -> Self {
        let recorder = Recorder::new(recents_path, settings_path.clone(), home.clone());
        Self {
            exe,
            backend,
            compositor,
            settings_path,
            home,
            recorder,
            pending: Mutex::new(HashMap::new()),
            runtime,
        }
    }

    /// The class of the window with keyboard focus right now, which is almost
    /// always the application whose Save button was just pressed.
    fn focused_class(&self) -> Option<String> {
        let active = self.compositor.active_window().ok()??;
        self.compositor
            .clients()
            .ok()?
            .into_iter()
            .find(|c| c.address == active)
            .map(|c| c.class)
    }
}

/// Held for the life of one dialog; dropping it stops the strip and undoes
/// anything installed for the dialog.
struct DialogGuard {
    strip: Option<Child>,
    compositor: Arc<dyn Compositor>,
    strata_rule: bool,
    runtime: Option<PathBuf>,
}

/// How long a strip has to notice its dialog closed and end by itself,
/// removing its key bindings, before it is stopped.
const STRIP_GRACE: Duration = Duration::from_millis(1500);

impl Drop for DialogGuard {
    fn drop(&mut self) {
        if let Some(mut strip) = self.strip.take() {
            // The strip exits by itself when the dialog closes; stopping it
            // covers a crash or a hang. Waited for off this thread, so the
            // application gets its answer at once.
            let compositor = self.compositor.clone();
            let runtime = self.runtime.clone();
            std::thread::spawn(move || {
                let start = Instant::now();
                while matches!(strip.try_wait(), Ok(None)) && start.elapsed() < STRIP_GRACE {
                    std::thread::sleep(Duration::from_millis(50));
                }
                if matches!(strip.try_wait(), Ok(None)) {
                    strip.kill().ok();
                }
                strip.wait().ok();
                // A strip that was stopped could not remove its key bindings.
                if let Some(dir) = runtime {
                    keys::release_for(&*compositor, &dir, strip.id());
                }
            });
        }
        if self.strata_rule {
            strata::remove_float_rule(&*self.compositor);
        }
    }
}

fn warn(messages: &[String]) {
    for m in messages {
        eprintln!("ofolder: {m}");
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn uris_in(results: &Options) -> Vec<String> {
    results
        .get("uris")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .unwrap_or_default()
}

impl Hooks for StripHooks {
    fn begin(&self, call: &Call, options: &mut Options) -> Box<dyn Send> {
        let loaded = settings::load(&self.settings_path, &self.home);
        warn(&loaded.warnings);
        let who = identity::resolve(&call.app_id, self.focused_class().as_deref());
        let supplied = preseed::current_folder(options);
        let plan = identity::plan(&loaded.settings, &who, supplied.as_deref(), &|p: &Path| {
            p.is_dir()
        });
        if let Some(folder) = &plan.seed_folder {
            preseed::set_current_folder(options, folder);
        }
        let app_key = who.key.clone();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(call.handle.clone(), who);
        }
        if loaded.settings.paused {
            // Paused means untouched: not even the float rule for Strata's dialog.
            return Box::new(DialogGuard {
                strip: None,
                compositor: self.compositor.clone(),
                strata_rule: false,
                runtime: None,
            });
        }
        // Strata's dialog tiles unless a rule says otherwise, and the rule has to exist before it opens.
        let strata_rule =
            self.backend == "strata" && strata::install_float_rule(&*self.compositor).is_ok();
        let strip = plan.show_strip.then(|| {
            let known: Vec<String> = if self.backend == "flea" {
                picker_pids(&*self.compositor)
                    .iter()
                    .map(u32::to_string)
                    .collect()
            } else {
                Vec::new()
            };
            Command::new(&self.exe)
                .args(["strip", "--backend", &self.backend, "--title", &call.title])
                .args(["--known-pids", &known.join(","), "--app-key", &app_key])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| eprintln!("ofolder: could not start the strip: {e}"))
                .ok()
        });
        Box::new(DialogGuard {
            strip: strip.flatten(),
            compositor: self.compositor.clone(),
            strata_rule,
            runtime: self.runtime.clone(),
        })
    }

    fn end(&self, call: &Call, response: u32, results: &Options) {
        let who = self
            .pending
            .lock()
            .ok()
            .and_then(|mut p| p.remove(&call.handle))
            .unwrap_or_else(Identity::unknown);
        warn(
            &self
                .recorder
                .record(&who.key, response, &uris_in(results), now(), &|p: &Path| {
                    p.is_dir()
                }),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ofolder_core::hypr::{Client, Monitor, parse_clients};
    use ofolder_core::recents;
    use ofolder_core::system::SystemError;
    use ofolder_portal::hooks::Method;
    use zbus::zvariant::{OwnedValue, Value};

    use super::*;

    struct Desktop {
        active: Option<String>,
        clients: Vec<Client>,
    }

    impl Compositor for Desktop {
        fn clients(&self) -> Result<Vec<Client>, SystemError> {
            Ok(self.clients.clone())
        }
        fn monitors(&self) -> Result<Vec<Monitor>, SystemError> {
            Ok(Vec::new())
        }
        fn active_window(&self) -> Result<Option<String>, SystemError> {
            Ok(self.active.clone())
        }
        fn focus(&self, _: &str) -> Result<(), SystemError> {
            Ok(())
        }
    }

    fn desktop_with_focus(class: &str) -> Arc<Desktop> {
        let json = format!(
            r#"[{{"address":"0x1","class":"{class}","title":"x","pid":1,"at":[0,0],"size":[1,1],
            "floating":false,"workspace":{{"id":1,"name":"1"}},"monitor":0}}]"#
        );
        Arc::new(Desktop {
            active: Some("0x1".to_owned()),
            clients: parse_clients(&json).expect("sample"),
        })
    }

    struct Rig {
        dir: PathBuf,
        hooks: StripHooks,
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn rig(label: &str, settings: &str, focused: &str) -> Rig {
        let dir =
            std::env::temp_dir().join(format!("ofolder-hooks-{label}-{}", std::process::id()));
        fs::create_dir_all(dir.join("docs")).expect("dirs");
        fs::create_dir_all(dir.join("work")).expect("dirs");
        fs::write(
            dir.join("settings.toml"),
            settings.replace("{DIR}", &dir.to_string_lossy()),
        )
        .expect("settings");
        let hooks = StripHooks::new(
            PathBuf::from("/bin/true"),
            "gtk".to_owned(),
            desktop_with_focus(focused),
            dir.join("settings.toml"),
            dir.join("recents.json"),
            PathBuf::from("/home/j"),
            None,
        );
        Rig { dir, hooks }
    }

    fn call(app_id: &str) -> Call {
        Call {
            method: Method::SaveFile,
            handle: "/request/1".to_owned(),
            app_id: app_id.to_owned(),
            parent_window: String::new(),
            title: "Save".to_owned(),
        }
    }

    fn results_with(uris: &[String]) -> Options {
        let mut o = Options::new();
        o.insert(
            "uris".to_owned(),
            OwnedValue::try_from(Value::from(uris.to_vec())).expect("uris"),
        );
        o
    }

    fn folder_of(options: &Options) -> Option<PathBuf> {
        preseed::current_folder(options)
    }

    #[test]
    fn a_confirmed_save_is_recorded_against_the_window_that_had_focus() {
        let r = rig("record", "", "zen");
        let c = call("");
        drop(r.hooks.begin(&c, &mut Options::new()));
        let file = format!("file://{}/docs/report.txt", r.dir.display());
        r.hooks.end(&c, 0, &results_with(&[file]));
        let store = recents::load(&r.dir.join("recents.json")).recents;
        assert_eq!(store.folders[0].path, format!("{}/docs", r.dir.display()));
        assert_eq!(store.folders[0].apps, ["class:zen"]);
    }

    #[test]
    fn a_cancelled_dialog_records_nothing_and_a_real_app_id_wins_over_the_focused_window() {
        let r = rig("cancel", "", "zen");
        let cancelled = call("");
        drop(r.hooks.begin(&cancelled, &mut Options::new()));
        r.hooks.end(
            &cancelled,
            1,
            &results_with(&[format!("file://{}/docs/x.txt", r.dir.display())]),
        );
        assert!(!r.dir.join("recents.json").exists());

        let sandboxed = call("org.example.App");
        drop(r.hooks.begin(&sandboxed, &mut Options::new()));
        r.hooks.end(
            &sandboxed,
            0,
            &results_with(&[format!("file://{}/docs/y.txt", r.dir.display())]),
        );
        let store = recents::load(&r.dir.join("recents.json")).recents;
        assert_eq!(store.folders[0].apps, ["org.example.App"]);
    }

    #[test]
    fn a_start_folder_rule_seeds_a_dialog_that_named_no_folder() {
        let r = rig(
            "seed",
            "[start_folders]\n\"class:zen\" = \"{DIR}/work\"",
            "zen",
        );
        let mut options = Options::new();
        drop(r.hooks.begin(&call(""), &mut options));
        assert_eq!(folder_of(&options), Some(r.dir.join("work")));
    }

    #[test]
    fn an_app_that_named_its_own_folder_keeps_it_even_when_a_rule_exists() {
        let r = rig(
            "keep",
            "[start_folders]\n\"class:zen\" = \"{DIR}/work\"",
            "zen",
        );
        let mut options = Options::new();
        preseed::set_current_folder(&mut options, Path::new("/the/documents/own/folder"));
        drop(r.hooks.begin(&call(""), &mut options));
        assert_eq!(
            folder_of(&options),
            Some(PathBuf::from("/the/documents/own/folder"))
        );
    }

    #[test]
    fn a_rule_keyed_by_a_portal_id_does_not_fire_for_a_window_class_that_happens_to_match() {
        let r = rig("nofire", "[start_folders]\n\"zen\" = \"{DIR}/work\"", "zen");
        let mut options = Options::new();
        drop(r.hooks.begin(&call(""), &mut options));
        assert_eq!(folder_of(&options), None);
    }

    #[test]
    fn while_paused_nothing_is_moved_and_nothing_is_recorded() {
        let r = rig(
            "paused",
            "paused = true\n[start_folders]\n\"class:zen\" = \"{DIR}/work\"",
            "zen",
        );
        let mut options = Options::new();
        let c = call("");
        drop(r.hooks.begin(&c, &mut options));
        assert_eq!(folder_of(&options), None, "no start folder while paused");
        r.hooks.end(
            &c,
            0,
            &results_with(&[format!("file://{}/docs/x.txt", r.dir.display())]),
        );
        assert!(!r.dir.join("recents.json").exists());
    }

    #[test]
    fn a_dialog_whose_focused_window_is_another_dialog_is_not_attributed_to_it() {
        let r = rig("anotherdialog", "", "xdg-desktop-portal-gtk");
        let c = call("");
        drop(r.hooks.begin(&c, &mut Options::new()));
        r.hooks.end(
            &c,
            0,
            &results_with(&[format!("file://{}/docs/x.txt", r.dir.display())]),
        );
        let store = recents::load(&r.dir.join("recents.json")).recents;
        assert!(store.folders[0].apps.is_empty());
    }
}
