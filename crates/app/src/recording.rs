//! Writing a confirmed dialog into the recents store.
//!
//! The proxy writes the store, and every dialog that finishes writes it too,
//! so the read, the change and the write are held under the store's lock
//! file: that covers two dialogs finishing at the same moment, in this process
//! or another. The strip processes only read it.

use std::path::{Path, PathBuf};

use ofolder_core::private_file;
use ofolder_core::recents::{self, Policy};
use ofolder_core::settings;

pub struct Recorder {
    pub recents_path: PathBuf,
    pub settings_path: PathBuf,
    pub home: PathBuf,
}

/// The portal's "everything went well" response code.
const CONFIRMED: u32 = 0;

impl Recorder {
    pub fn new(recents_path: PathBuf, settings_path: PathBuf, home: PathBuf) -> Self {
        Self {
            recents_path,
            settings_path,
            home,
        }
    }

    /// Records what a dialog returned. `app_key` is empty when the app is
    /// unknown; the folders are still recorded, with no app attached. Returns
    /// the warnings to show: nothing is recorded for a cancelled dialog, while
    /// paused, or when recording is off.
    pub fn record(
        &self,
        app_key: &str,
        response: u32,
        uris: &[String],
        now: u64,
        is_dir: &dyn Fn(&Path) -> bool,
    ) -> Vec<String> {
        if response != CONFIRMED {
            return Vec::new();
        }
        let loaded = settings::load(&self.settings_path, &self.home);
        let mut warnings = loaded.warnings;
        let s = loaded.settings;
        if s.paused || !s.recents_enabled {
            return warnings;
        }
        let paths = recents::uris_to_paths(uris);
        if paths.is_empty() {
            return warnings;
        }
        let recorded = private_file::with_lock(&self.recents_path, || {
            let mut store = recents::load(&self.recents_path);
            let mut notes: Vec<String> = store.warning.take().into_iter().collect();
            store.recents.prune(s.recents_days, now);
            store.recents.trim(s.recents_max);
            let policy = Policy {
                enabled: true,
                max: s.recents_max,
                excluded: &s.recents_excluded,
            };
            store
                .recents
                .record_confirmed(&paths, app_key, now, &policy, is_dir);
            if let Err(e) = recents::save(&self.recents_path, &store.recents) {
                notes.push(e.to_string());
            }
            notes
        });
        match recorded {
            Ok(notes) => warnings.extend(notes),
            Err(e) => warnings.push(format!("recents were not recorded: {e}")),
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    struct Rig {
        dir: PathBuf,
        recorder: Recorder,
    }

    fn rig(label: &str, settings: &str) -> Rig {
        let dir = std::env::temp_dir().join(format!("ofolder-rec-{label}-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        fs::write(dir.join("settings.toml"), settings).expect("settings");
        let recorder = Recorder::new(
            dir.join("recents.json"),
            dir.join("settings.toml"),
            PathBuf::from("/home/j"),
        );
        Rig { dir, recorder }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn uris(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| format!("file://{p}")).collect()
    }

    fn stored(r: &Rig) -> recents::Recents {
        recents::load(&r.recorder.recents_path).recents
    }

    /// Folders are the paths with no extension that are not named `file`: the tests' files are `x.txt`, `report.txt` and `file`.
    fn real_folders(p: &Path) -> bool {
        p.extension().is_none() && p.file_name().is_some_and(|n| n != "file")
    }

    #[test]
    fn a_confirmed_save_is_recorded_with_the_app_that_made_it() {
        let r = rig("save", "");
        let w = r
            .recorder
            .record("class:zen", 0, &uris(&["/docs/report.txt"]), 10, &|p| {
                p == Path::new("/docs")
            });
        assert!(w.is_empty(), "{w:?}");
        let s = stored(&r);
        assert_eq!(s.folders[0].path, "/docs");
        assert_eq!(s.folders[0].apps, ["class:zen"]);
        assert_eq!(s.files[0].path, "/docs/report.txt");
    }

    #[test]
    fn a_cancelled_dialog_records_nothing() {
        let r = rig("cancel", "");
        r.recorder
            .record("app", 1, &uris(&["/docs/x.txt"]), 1, &real_folders);
        r.recorder
            .record("app", 2, &uris(&["/docs/y.txt"]), 1, &real_folders);
        assert!(!r.recorder.recents_path.exists());
    }

    #[test]
    fn nothing_is_recorded_while_paused_or_when_recording_is_off() {
        for settings in ["paused = true", "recents_enabled = false"] {
            let r = rig("off", settings);
            r.recorder
                .record("app", 0, &uris(&["/docs/x.txt"]), 1, &real_folders);
            assert!(!r.recorder.recents_path.exists(), "{settings}");
        }
    }

    #[test]
    fn an_unknown_app_still_records_the_folder_but_attaches_no_app() {
        let r = rig("unknown", "");
        r.recorder
            .record("", 0, &uris(&["/docs/x.txt"]), 1, &real_folders);
        assert!(stored(&r).folders[0].apps.is_empty());
    }

    #[test]
    fn the_settings_limit_and_exclusions_are_honoured() {
        let r = rig(
            "limit",
            "recents_max = 2\nrecents_excluded = [\"/private\"]",
        );
        for (i, f) in ["/a", "/b", "/c", "/private/z"].iter().enumerate() {
            r.recorder.record(
                "app",
                0,
                &uris(&[&format!("{f}/file")]),
                i as u64,
                &real_folders,
            );
        }
        let s = stored(&r);
        let folders: Vec<_> = s.folders.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(folders, ["/c", "/b"]);
    }

    #[test]
    fn a_bad_settings_file_still_lets_a_dialog_be_recorded_and_says_why() {
        let r = rig("badsettings", "recents_max = \"many\"");
        let w = r
            .recorder
            .record("app", 0, &uris(&["/docs/x.txt"]), 1, &real_folders);
        assert_eq!(w.len(), 1);
        assert_eq!(stored(&r).folders.len(), 1);
    }

    #[test]
    fn a_corrupt_store_is_set_aside_with_a_warning_and_recording_continues() {
        let r = rig("corrupt", "");
        fs::write(&r.recorder.recents_path, "garbage").expect("write");
        let w = r
            .recorder
            .record("app", 0, &uris(&["/docs/x.txt"]), 1, &real_folders);
        assert!(w.iter().any(|m| m.contains("set aside")), "{w:?}");
        assert_eq!(stored(&r).folders.len(), 1);
    }

    #[test]
    fn uris_that_are_not_local_files_record_nothing() {
        let r = rig("remote", "");
        r.recorder.record(
            "app",
            0,
            &["https://example.org/x".to_owned()],
            1,
            &real_folders,
        );
        assert!(!r.recorder.recents_path.exists());
    }

    /// A folder that is a real folder for the sensitive-location tests.
    fn is_home_ssh(p: &Path) -> bool {
        p == Path::new("/home/j/.ssh")
    }

    #[test]
    fn a_save_in_a_sensitive_folder_is_not_recorded_under_the_defaults() {
        let r = rig("sensitive", "");
        r.recorder.record(
            "app",
            0,
            &uris(&["/home/j/.ssh/config.txt"]),
            1,
            &is_home_ssh,
        );
        let s = stored(&r);
        assert!(s.folders.is_empty(), "{:?}", s.folders);
        assert!(s.files.is_empty(), "{:?}", s.files);
    }

    #[test]
    fn the_same_save_is_recorded_once_the_file_sets_no_exclusions() {
        let r = rig("no-exclusions", "recents_excluded = []");
        r.recorder.record(
            "app",
            0,
            &uris(&["/home/j/.ssh/config.txt"]),
            1,
            &is_home_ssh,
        );
        assert_eq!(stored(&r).folders[0].path, "/home/j/.ssh");
    }

    #[test]
    fn a_list_in_the_file_replaces_the_defaults_for_recording() {
        let r = rig("replaced", "recents_excluded = [\"/private\"]");
        r.recorder.record(
            "app",
            0,
            &uris(&["/home/j/.ssh/config.txt"]),
            1,
            &is_home_ssh,
        );
        assert_eq!(stored(&r).folders[0].path, "/home/j/.ssh");
    }

    #[test]
    fn recent_entries_older_than_the_setting_are_dropped_when_recording() {
        let r = rig("expiry", "recents_days = 30");
        let day = 86_400;
        let mut old = recents::Recents::default();
        old.folders.push(recents::RecentFolder {
            path: "/old".to_owned(),
            apps: Vec::new(),
            at: 10 * day,
        });
        recents::save(&r.recorder.recents_path, &old).expect("seed");
        r.recorder
            .record("app", 0, &uris(&["/docs/x.txt"]), 100 * day, &real_folders);
        let folders: Vec<_> = stored(&r).folders.into_iter().map(|f| f.path).collect();
        assert_eq!(folders, ["/docs"]);
    }

    #[test]
    fn two_recorders_finishing_at_once_keep_every_folder() {
        let r = rig("concurrent", "recents_max = 500");
        let other = Recorder::new(
            r.recorder.recents_path.clone(),
            r.recorder.settings_path.clone(),
            r.recorder.home.clone(),
        );
        let always_a_folder = |_: &Path| true;
        std::thread::scope(|scope| {
            for (recorder, tag) in [(&r.recorder, "a"), (&other, "b")] {
                scope.spawn(move || {
                    for i in 0..200 {
                        let w = recorder.record(
                            "app",
                            0,
                            &uris(&[&format!("/{tag}/{i}")]),
                            i as u64,
                            &always_a_folder,
                        );
                        assert!(w.is_empty(), "{w:?}");
                    }
                });
            }
        });
        let s = stored(&r);
        assert_eq!(s.folders.len(), 400, "no update was lost");
    }
}
