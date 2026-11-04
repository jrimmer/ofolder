//! Folders and files recently used in dialogs.
//!
//! Recorded when a dialog is confirmed with a result, never on browsing. A
//! folder appears once however often it is used, and moves to the top when it
//! is used again. Each entry remembers which applications used it, so the
//! strip can show everyone's recents or only one application's.
//!
//! The store is a JSON file. Writes are atomic, and a file that cannot be
//! read is set aside and replaced by an empty store, with a warning, so the
//! history is never silently lost and never blocks a dialog.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::private_file;

const SECONDS_PER_DAY: u64 = 86_400;

/// What the settings allow the store to record.
#[derive(Debug, Clone, Copy)]
pub struct Policy<'a> {
    pub enabled: bool,
    pub max: usize,
    /// A folder under any of these is never recorded.
    pub excluded: &'a [PathBuf],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentFolder {
    pub path: String,
    /// Application ids that used it, in the order they first did.
    pub apps: Vec<String>,
    /// Unix seconds of the last use.
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentFile {
    pub path: String,
    pub apps: Vec<String>,
    pub at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recents {
    #[serde(default)]
    pub folders: Vec<RecentFolder>,
    #[serde(default)]
    pub files: Vec<RecentFile>,
}

fn utf8(path: &Path) -> Option<String> {
    path.to_str().map(str::to_owned)
}

fn note_app(apps: &mut Vec<String>, app: &str) {
    if !app.is_empty() && !apps.iter().any(|a| a == app) {
        apps.push(app.to_owned());
    }
}

impl Recents {
    fn allowed(path: &Path, policy: &Policy<'_>) -> bool {
        policy.enabled && !policy.excluded.iter().any(|skip| path.starts_with(skip))
    }

    /// Records a folder. Returns whether it was recorded: nothing is when
    /// recording is off, the folder is excluded, it is not valid UTF-8, or it
    /// is not a folder that exists.
    pub fn record_folder(
        &mut self,
        folder: &Path,
        app: &str,
        now: u64,
        policy: &Policy<'_>,
        is_dir: &dyn Fn(&Path) -> bool,
    ) -> bool {
        let Some(text) = utf8(folder) else {
            return false;
        };
        if !Self::allowed(folder, policy) || !is_dir(folder) {
            return false;
        }
        let mut apps = Vec::new();
        if let Some(at) = self.folders.iter().position(|f| f.path == text) {
            apps = self.folders.remove(at).apps;
        }
        note_app(&mut apps, app);
        self.folders.insert(
            0,
            RecentFolder {
                path: text,
                apps,
                at: now,
            },
        );
        self.folders.truncate(policy.max);
        true
    }

    /// Records a file, which may not exist yet: a saved file is created by the
    /// application after the dialog closes. Excluded locations are still
    /// honoured.
    pub fn record_file(&mut self, file: &Path, app: &str, now: u64, policy: &Policy<'_>) -> bool {
        let Some(text) = utf8(file) else { return false };
        if !Self::allowed(file, policy) {
            return false;
        }
        let mut apps = Vec::new();
        if let Some(at) = self.files.iter().position(|f| f.path == text) {
            apps = self.files.remove(at).apps;
        }
        note_app(&mut apps, app);
        self.files.insert(
            0,
            RecentFile {
                path: text,
                apps,
                at: now,
            },
        );
        self.files.truncate(policy.max);
        true
    }

    /// Records what a confirmed dialog returned. A path that is a folder is
    /// recorded as a folder. Anything else is a file, recorded as a file along
    /// with the folder it is in.
    pub fn record_confirmed(
        &mut self,
        chosen: &[PathBuf],
        app: &str,
        now: u64,
        policy: &Policy<'_>,
        is_dir: &dyn Fn(&Path) -> bool,
    ) {
        for path in chosen {
            if is_dir(path) {
                self.record_folder(path, app, now, policy, is_dir);
            } else {
                self.record_file(path, app, now, policy);
                if let Some(parent) = path.parent() {
                    self.record_folder(parent, app, now, policy, is_dir);
                }
            }
        }
    }

    /// Folders newest first, everyone's or only `app`'s.
    pub fn folders_for(&self, app: Option<&str>) -> Vec<&RecentFolder> {
        self.folders
            .iter()
            .filter(|f| app.is_none_or(|a| f.apps.iter().any(|x| x == a)))
            .collect()
    }

    pub fn files_for(&self, app: Option<&str>) -> Vec<&RecentFile> {
        self.files
            .iter()
            .filter(|f| app.is_none_or(|a| f.apps.iter().any(|x| x == a)))
            .collect()
    }

    pub fn remove_folder(&mut self, folder: &Path) -> bool {
        let Some(text) = utf8(folder) else {
            return false;
        };
        let before = self.folders.len();
        self.folders.retain(|f| f.path != text);
        self.folders.len() != before
    }

    pub fn clear(&mut self) {
        self.folders.clear();
        self.files.clear();
    }

    /// Cuts the lists down when the limit was lowered after they were written.
    pub fn trim(&mut self, max: usize) {
        self.folders.truncate(max);
        self.files.truncate(max);
    }

    /// Drops entries last used more than `days` days before `now`. Zero keeps
    /// everything, as the setting's documentation says.
    pub fn prune(&mut self, days: u32, now: u64) {
        if days == 0 {
            return;
        }
        let cutoff = now.saturating_sub(u64::from(days) * SECONDS_PER_DAY);
        self.folders.retain(|f| f.at >= cutoff);
        self.files.retain(|f| f.at >= cutoff);
    }
}

/// The paths named by a list of `file://` URIs. Anything that is not a local
/// file URI, or is malformed, is skipped.
pub fn uris_to_paths(uris: &[String]) -> Vec<PathBuf> {
    uris.iter().filter_map(|u| uri_to_path(u)).collect()
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    if out.contains(&0) {
        return None;
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

#[derive(Debug, thiserror::Error)]
pub enum RecentsError {
    #[error("could not write the recents file: {0}")]
    Write(#[from] std::io::Error),
    #[error("could not encode the recents: {0}")]
    Encode(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct Loaded {
    pub recents: Recents,
    /// Set when the file could not be used and was set aside.
    pub warning: Option<String>,
}

/// Reads the store. A missing file is an empty store. A file that cannot be
/// read is renamed to `<name>.corrupt` (or `.corrupt.1` and so on, so an
/// earlier one is never overwritten) and an empty store is returned.
pub fn load(path: &Path) -> Loaded {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Loaded {
                recents: Recents::default(),
                warning: None,
            };
        }
        Err(e) => {
            return Loaded {
                recents: Recents::default(),
                warning: Some(format!("recents could not be read: {e}")),
            };
        }
    };
    match serde_json::from_str::<Recents>(&text) {
        Ok(recents) => Loaded {
            recents,
            warning: None,
        },
        Err(error) => {
            let aside = set_aside(path);
            Loaded {
                recents: Recents::default(),
                warning: Some(format!(
                    "recents were unreadable ({error}); {} and started again",
                    aside.map_or_else(
                        || "the file could not be moved".to_owned(),
                        |p| format!("set aside as {}", p.display())
                    )
                )),
            }
        }
    }
}

fn set_aside(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    for n in 0..100 {
        let suffix = if n == 0 {
            ".corrupt".to_owned()
        } else {
            format!(".corrupt.{n}")
        };
        let target = path.with_file_name(format!("{name}{suffix}"));
        if !target.exists() {
            return fs::rename(path, &target).ok().map(|()| target);
        }
    }
    None
}

/// Writes the store private and whole: see [`private_file::write_private_atomic`].
pub fn save(path: &Path, recents: &Recents) -> Result<(), RecentsError> {
    let bytes = serde_json::to_vec_pretty(recents)?;
    private_file::write_private_atomic(path, &bytes)?;
    Ok(())
}

/// What [`forget_all`] removed, by path, for the maintainer to see.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Forgotten {
    pub folders: Vec<String>,
    pub files: Vec<String>,
    /// Set when the file could not be read; it was replaced by an empty store
    /// anyway, since forgetting is what was asked.
    pub unreadable: bool,
}

impl Forgotten {
    /// One line for the maintainer: how many entries went.
    pub fn summary(&self) -> String {
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        if self.folders.is_empty() && self.files.is_empty() {
            return "there were no recent folders or files to forget".to_owned();
        }
        format!(
            "forgot {} and {}",
            plural(self.folders.len(), "folder"),
            plural(self.files.len(), "file")
        )
    }
}

/// Empties the store, folders and files alike, and says what was in it. A
/// store that does not exist is left that way, and an unreadable one is
/// replaced with an empty store. Favorites are not recents and are not touched.
pub fn forget_all(path: &Path) -> Result<Forgotten, RecentsError> {
    if !path.exists() {
        return Ok(Forgotten::default());
    }
    private_file::with_lock(path, || {
        let bytes = fs::read(path)?;
        let (recents, unreadable) = match serde_json::from_slice::<Recents>(&bytes) {
            Ok(recents) => (recents, false),
            Err(_) => (Recents::default(), true),
        };
        let forgotten = Forgotten {
            folders: recents.folders.iter().map(|f| f.path.clone()).collect(),
            files: recents.files.iter().map(|f| f.path.clone()).collect(),
            unreadable,
        };
        save(path, &Recents::default())?;
        Ok(forgotten)
    })?
}

/// Where the store lives: an override, else the user's data folder.
pub fn recents_path(
    override_path: Option<PathBuf>,
    xdg_data: Option<PathBuf>,
    home: &Path,
) -> PathBuf {
    override_path.unwrap_or_else(|| {
        xdg_data
            .unwrap_or_else(|| home.join(".local/share"))
            .join("ofolder/recents.json")
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn policy(max: usize, excluded: &[PathBuf]) -> Policy<'_> {
        Policy {
            enabled: true,
            max,
            excluded,
        }
    }

    fn dirs(real: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<PathBuf> = real.iter().map(PathBuf::from).collect();
        move |p: &Path| set.contains(p)
    }

    fn temp(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ofolder-recents-{label}-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_saved_to_folder_is_first_in_the_list() {
        let mut r = Recents::default();
        let is_dir = dirs(&["/a", "/b"]);
        r.record_folder(Path::new("/a"), "app", 1, &policy(10, &[]), &is_dir);
        r.record_folder(Path::new("/b"), "app", 2, &policy(10, &[]), &is_dir);
        assert_eq!(r.folders[0].path, "/b");
    }

    #[test]
    fn a_folder_used_twice_appears_once_and_moves_to_the_top() {
        let mut r = Recents::default();
        let is_dir = dirs(&["/a", "/b"]);
        for (path, at) in [("/a", 1), ("/b", 2), ("/a", 3)] {
            r.record_folder(Path::new(path), "app", at, &policy(10, &[]), &is_dir);
        }
        let paths: Vec<_> = r.folders.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["/a", "/b"]);
        assert_eq!(r.folders[0].at, 3);
    }

    #[test]
    fn the_eleventh_distinct_folder_pushes_out_the_oldest() {
        let mut r = Recents::default();
        let names: Vec<String> = (0..11).map(|i| format!("/d{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let is_dir = dirs(&refs);
        for (i, n) in names.iter().enumerate() {
            r.record_folder(Path::new(n), "app", i as u64, &policy(10, &[]), &is_dir);
        }
        assert_eq!(r.folders.len(), 10);
        assert!(
            r.folders.iter().all(|f| f.path != "/d0"),
            "the oldest is gone"
        );
        assert_eq!(r.folders[0].path, "/d10");
    }

    #[test]
    fn only_folders_that_exist_are_recorded() {
        let mut r = Recents::default();
        assert!(!r.record_folder(Path::new("/gone"), "app", 1, &policy(10, &[]), &dirs(&[])));
        assert!(r.folders.is_empty());
    }

    #[test]
    fn nothing_is_recorded_when_recording_is_switched_off() {
        let mut r = Recents::default();
        let off = Policy {
            enabled: false,
            max: 10,
            excluded: &[],
        };
        assert!(!r.record_folder(Path::new("/a"), "app", 1, &off, &dirs(&["/a"])));
        assert!(!r.record_file(Path::new("/a/f.txt"), "app", 1, &off));
        assert_eq!(r, Recents::default());
    }

    #[test]
    fn a_folder_under_an_excluded_location_is_never_recorded_and_neither_is_the_location() {
        let mut r = Recents::default();
        let excluded = [PathBuf::from("/private")];
        let is_dir = dirs(&["/private", "/private/deep/er", "/public"]);
        assert!(!r.record_folder(
            Path::new("/private"),
            "app",
            1,
            &policy(10, &excluded),
            &is_dir
        ));
        assert!(!r.record_folder(
            Path::new("/private/deep/er"),
            "app",
            1,
            &policy(10, &excluded),
            &is_dir
        ));
        assert!(r.record_folder(
            Path::new("/public"),
            "app",
            1,
            &policy(10, &excluded),
            &is_dir
        ));
        assert_eq!(r.folders.len(), 1);
    }

    #[test]
    fn a_sibling_that_only_shares_a_name_prefix_with_an_excluded_folder_is_still_recorded() {
        let mut r = Recents::default();
        let excluded = [PathBuf::from("/priv")];
        assert!(r.record_folder(
            Path::new("/private"),
            "app",
            1,
            &policy(10, &excluded),
            &dirs(&["/private"])
        ));
    }

    #[test]
    fn a_folder_used_by_two_apps_shows_in_each_apps_view_and_in_everyones() {
        let mut r = Recents::default();
        let is_dir = dirs(&["/shared", "/only-a"]);
        r.record_folder(Path::new("/shared"), "app.a", 1, &policy(10, &[]), &is_dir);
        r.record_folder(Path::new("/only-a"), "app.a", 2, &policy(10, &[]), &is_dir);
        r.record_folder(Path::new("/shared"), "app.b", 3, &policy(10, &[]), &is_dir);
        assert_eq!(r.folders_for(None).len(), 2);
        let b: Vec<_> = r
            .folders_for(Some("app.b"))
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert_eq!(b, ["/shared"]);
        let a: Vec<_> = r
            .folders_for(Some("app.a"))
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert_eq!(a, ["/shared", "/only-a"]);
    }

    #[test]
    fn a_confirmed_save_records_the_file_and_its_folder_and_a_chosen_folder_only_the_folder() {
        let mut r = Recents::default();
        let is_dir = dirs(&["/docs", "/pics"]);
        r.record_confirmed(
            &[PathBuf::from("/docs/new.txt"), PathBuf::from("/pics")],
            "app",
            5,
            &policy(10, &[]),
            &is_dir,
        );
        let folders: Vec<_> = r.folders.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(folders, ["/pics", "/docs"]);
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files[0].path, "/docs/new.txt");
    }

    #[test]
    fn a_file_that_does_not_exist_yet_is_still_recorded_because_the_app_creates_it_after_the_dialog()
     {
        let mut r = Recents::default();
        assert!(r.record_file(Path::new("/docs/soon.txt"), "app", 1, &policy(10, &[])));
    }

    #[test]
    fn an_individual_entry_can_be_removed_and_the_whole_list_cleared() {
        let mut r = Recents::default();
        let is_dir = dirs(&["/a", "/b"]);
        r.record_folder(Path::new("/a"), "app", 1, &policy(10, &[]), &is_dir);
        r.record_folder(Path::new("/b"), "app", 2, &policy(10, &[]), &is_dir);
        assert!(r.remove_folder(Path::new("/a")));
        assert!(!r.remove_folder(Path::new("/a")));
        r.record_file(Path::new("/b/f"), "app", 3, &policy(10, &[]));
        r.clear();
        assert_eq!(r, Recents::default());
    }

    #[test]
    fn lowering_the_limit_cuts_the_lists_on_trim() {
        let mut r = Recents::default();
        let is_dir = dirs(&["/a", "/b", "/c"]);
        for (p, at) in [("/a", 1), ("/b", 2), ("/c", 3)] {
            r.record_folder(Path::new(p), "app", at, &policy(10, &[]), &is_dir);
        }
        r.trim(2);
        assert_eq!(r.folders.len(), 2);
        assert_eq!(r.folders[0].path, "/c");
    }

    #[test]
    fn file_uris_become_paths_with_spaces_and_unicode_decoded() {
        let uris = vec![
            "file:///home/j/with%20space/a.txt".to_owned(),
            "file:///home/j/%C3%BCn%C3%AF.txt".to_owned(),
            "file://localhost/tmp/x".to_owned(),
        ];
        let paths = uris_to_paths(&uris);
        assert_eq!(
            paths,
            [
                PathBuf::from("/home/j/with space/a.txt"),
                PathBuf::from("/home/j/ünï.txt"),
                PathBuf::from("/tmp/x")
            ]
        );
    }

    #[test]
    fn uris_that_are_not_local_files_or_are_malformed_are_skipped() {
        let uris: Vec<String> = [
            "http://example.org/a",
            "file://otherhost/x",
            "file:///bad%2",
            "file:///nul%00",
            "file:relative",
            "",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert!(uris_to_paths(&uris).is_empty());
    }

    #[test]
    fn saved_recents_read_back_the_same() {
        let dir = temp("roundtrip");
        let file = dir.join("recents.json");
        let mut r = Recents::default();
        r.record_folder(Path::new("/a"), "app", 1, &policy(10, &[]), &dirs(&["/a"]));
        save(&file, &r).expect("save");
        let loaded = load(&file);
        assert_eq!(loaded.recents, r);
        assert!(loaded.warning.is_none());
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_missing_file_is_an_empty_store_with_no_warning() {
        let dir = temp("missing");
        let loaded = load(&dir.join("nothing.json"));
        assert_eq!(loaded.recents, Recents::default());
        assert!(loaded.warning.is_none());
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_corrupt_file_is_set_aside_not_deleted_and_never_overwrites_an_earlier_one() {
        let dir = temp("corrupt");
        let file = dir.join("recents.json");
        fs::write(&file, "{ not json").expect("write");
        let first = load(&file);
        assert!(first.warning.expect("warned").contains("set aside"));
        assert!(dir.join("recents.json.corrupt").exists());
        assert!(!file.exists());
        fs::write(&file, "also bad").expect("write");
        load(&file);
        assert!(dir.join("recents.json.corrupt.1").exists());
        assert_eq!(
            fs::read_to_string(dir.join("recents.json.corrupt")).expect("read"),
            "{ not json"
        );
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_failed_write_leaves_the_previous_file_whole() {
        let dir = temp("atomic");
        let file = dir.join("recents.json");
        let mut r = Recents::default();
        r.record_folder(Path::new("/a"), "app", 1, &policy(10, &[]), &dirs(&["/a"]));
        save(&file, &r).expect("save");
        // A folder that cannot be written to makes the write fail before the old file is touched.
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).expect("lock folder");
        r.record_folder(Path::new("/b"), "app", 2, &policy(10, &[]), &dirs(&["/b"]));
        let failed = save(&file, &r).is_err();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).expect("unlock folder");
        assert!(failed);
        let loaded = load(&file);
        assert_eq!(
            loaded.recents.folders.len(),
            1,
            "the earlier file is intact"
        );
        assert!(loaded.warning.is_none());
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn the_store_lives_in_the_data_folder_unless_overridden() {
        let home = Path::new("/home/j");
        assert_eq!(
            recents_path(None, None, home),
            PathBuf::from("/home/j/.local/share/ofolder/recents.json")
        );
        assert_eq!(
            recents_path(Some(PathBuf::from("/x.json")), None, home),
            PathBuf::from("/x.json")
        );
    }

    #[test]
    fn entries_not_used_within_the_limit_are_pruned_and_zero_keeps_everything() {
        let mut r = Recents::default();
        let is_dir = dirs(&["/old", "/new"]);
        let day = 86_400;
        let now = 100 * day;
        r.record_folder(
            Path::new("/old"),
            "app",
            now - 91 * day,
            &policy(10, &[]),
            &is_dir,
        );
        r.record_folder(
            Path::new("/new"),
            "app",
            now - 89 * day,
            &policy(10, &[]),
            &is_dir,
        );
        r.record_file(
            Path::new("/old/f.txt"),
            "app",
            now - 91 * day,
            &policy(10, &[]),
        );
        let mut kept = r.clone();
        kept.prune(0, now);
        assert_eq!(kept, r, "zero days keeps everything");
        r.prune(90, now);
        let folders: Vec<_> = r.folders.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(folders, ["/new"]);
        assert!(r.files.is_empty());
    }

    #[test]
    fn a_store_is_written_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("private");
        let file = dir.join("recents.json");
        save(&file, &Recents::default()).expect("save");
        let mode = fs::metadata(&file).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn forgetting_empties_folders_and_files_and_says_what_it_removed() {
        let dir = temp("forget");
        let file = dir.join("recents.json");
        let mut r = Recents::default();
        r.record_folder(Path::new("/a"), "app", 1, &policy(10, &[]), &dirs(&["/a"]));
        r.record_file(Path::new("/a/f.txt"), "app", 1, &policy(10, &[]));
        save(&file, &r).expect("save");
        let gone = forget_all(&file).expect("forget");
        assert_eq!(gone.folders, ["/a"]);
        assert_eq!(gone.files, ["/a/f.txt"]);
        assert!(!gone.unreadable);
        assert_eq!(load(&file).recents, Recents::default());
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn forgetting_an_unreadable_store_leaves_a_valid_empty_one() {
        let dir = temp("forget-bad");
        let file = dir.join("recents.json");
        for garbage in [&b"{ not json"[..], &[0xff, 0xfe, 0x00][..]] {
            fs::write(&file, garbage).expect("write");
            let gone = forget_all(&file).expect("forget");
            assert!(gone.unreadable, "{garbage:?}");
            assert!(gone.folders.is_empty() && gone.files.is_empty());
            assert_eq!(load(&file).recents, Recents::default());
            assert!(
                load(&file).warning.is_none(),
                "the file is valid JSON again"
            );
        }
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn the_forget_summary_counts_folders_and_files() {
        let one = Forgotten {
            folders: vec!["/a".to_owned()],
            files: vec!["/a/x".to_owned(), "/a/y".to_owned()],
            unreadable: false,
        };
        assert_eq!(one.summary(), "forgot 1 folder and 2 files");
        assert_eq!(
            Forgotten::default().summary(),
            "there were no recent folders or files to forget"
        );
    }

    #[test]
    fn forgetting_when_there_is_no_store_creates_nothing() {
        let dir = temp("forget-none");
        let file = dir.join("recents.json");
        assert_eq!(forget_all(&file).expect("forget"), Forgotten::default());
        assert!(!file.exists());
        assert!(!dir.join("recents.json.lock").exists());
        fs::remove_dir_all(&dir).expect("cleanup");
    }
}
