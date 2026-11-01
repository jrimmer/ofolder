//! What the strip shows and what each user action does to it, with no
//! window involved. Actions that need the dialog (moving it, creating a
//! folder) are returned as a [`Job`] for the caller to run off the UI
//! thread, and the answer comes back through `finish_*`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::favorites::{self, AddOutcome, Favorites};
use crate::naming::validate_new_folder_name;
use crate::paths;

/// How long an informational message stays.
const INFO_TTL: Duration = Duration::from_millis(2500);
/// How long an error stays; the user needs time to read it.
const ERROR_TTL: Duration = Duration::from_secs(10);
/// How long a problem found at startup stays.
const NOTICE_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Error,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub text: String,
    pub kind: MessageKind,
    created: Instant,
    ttl: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Browse,
    /// The edit controls of one favorite are open.
    Manage(usize),
    /// A text field asks for a new display name for one favorite.
    Rename(usize),
    /// A text field asks for the name of a new folder.
    NewFolder,
}

/// Work that must touch the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    Go(PathBuf),
    NewFolder(String),
}

/// One line of the favorites list, ready to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub index: usize,
    pub label: String,
    /// The path shortened to fit, shown under the label.
    pub subtitle: String,
    /// The full path, for hover.
    pub full_path: String,
    pub missing: bool,
    pub is_current: bool,
}

pub struct ViewModel {
    label: String,
    home: PathBuf,
    favorites_path: PathBuf,
    favorites: Favorites,
    folder: Option<PathBuf>,
    /// Whether each favorite's folder was last seen to exist. A path not
    /// listed is assumed present until a check says otherwise.
    available: HashMap<PathBuf, bool>,
    message: Option<Message>,
    mode: Mode,
    busy: bool,
}

/// Characters of path that fit the strip's width.
pub const PATH_CHARS: usize = 30;

impl ViewModel {
    pub fn new(label: String, home: PathBuf, favorites_path: PathBuf) -> ViewModel {
        let loaded = favorites::load(&favorites_path);
        let mut vm = ViewModel {
            label,
            home,
            favorites_path,
            favorites: loaded.favorites,
            folder: None,
            available: HashMap::new(),
            message: None,
            mode: Mode::Browse,
            busy: false,
        };
        if let Some(notice) = loaded.notice {
            vm.say(notice, MessageKind::Error, NOTICE_TTL);
        }
        vm
    }

    fn say(&mut self, text: String, kind: MessageKind, ttl: Duration) {
        if kind == MessageKind::Error {
            eprintln!("ofolder: {text}");
        }
        self.message = Some(Message {
            text,
            kind,
            created: Instant::now(),
            ttl,
        });
    }

    fn info(&mut self, text: impl Into<String>) {
        self.say(text.into(), MessageKind::Info, INFO_TTL);
    }

    fn error(&mut self, text: impl Into<String>) {
        self.say(text.into(), MessageKind::Error, ERROR_TTL);
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn message(&self) -> Option<&Message> {
        self.message.as_ref()
    }

    pub fn favorites(&self) -> &Favorites {
        &self.favorites
    }

    pub fn dismiss_message(&mut self) {
        self.message = None;
    }

    /// Drops a message whose time is up. Returns whether one went.
    pub fn expire_message(&mut self, now: Instant) -> bool {
        let over = self
            .message
            .as_ref()
            .is_some_and(|m| now.saturating_duration_since(m.created) >= m.ttl);
        if over {
            self.message = None;
        }
        over
    }

    // ---- the current folder -------------------------------------------

    /// Records what the dialog shows now. `None` replaces any old value, so
    /// a folder that can no longer be read is never shown as current.
    pub fn set_folder(&mut self, folder: Option<PathBuf>) -> bool {
        if self.folder == folder {
            return false;
        }
        self.folder = folder;
        true
    }

    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    /// The folder for the header: home as `~`, cut to fit, or "unknown".
    pub fn folder_display(&self) -> String {
        match &self.folder {
            Some(f) => paths::shorten_path(&paths::abbreviate(f, &self.home), PATH_CHARS),
            None => "unknown".to_owned(),
        }
    }

    /// The full path for hover.
    pub fn folder_full(&self) -> Option<String> {
        self.folder.as_ref().map(|f| f.display().to_string())
    }

    /// The text to place on the clipboard, with a confirmation message. The
    /// caller writes it to the clipboard.
    pub fn copy_path(&mut self) -> Option<String> {
        let Some(folder) = self.folder.clone() else {
            self.error("Cannot copy: the folder is unknown");
            return None;
        };
        let Some(text) = folder.to_str() else {
            self.error("Cannot copy: the path is not valid text");
            return None;
        };
        let text = text.to_owned();
        self.info("Path copied");
        Some(text)
    }

    // ---- favorites ----------------------------------------------------

    /// Writes the list; on failure the message says so and the change stays
    /// in memory for this session.
    fn save_favorites(&mut self) -> bool {
        match favorites::save(&self.favorites_path, &self.favorites) {
            Ok(()) => true,
            Err(e) => {
                self.error(e.to_string());
                false
            }
        }
    }

    pub fn current_is_favorite(&self) -> bool {
        self.folder
            .as_deref()
            .is_some_and(|f| self.favorites.contains(f))
    }

    /// Adds the current folder, or removes it when it is already a favorite.
    pub fn toggle_current_favorite(&mut self) {
        let Some(folder) = self.folder.clone() else {
            self.error("Cannot save: the folder is unknown");
            return;
        };
        if self.favorites.remove_path(&folder) {
            self.mode = Mode::Browse;
            if self.save_favorites() {
                self.info("Removed from favorites");
            }
            return;
        }
        if self.favorites.add(folder) == AddOutcome::Added && self.save_favorites() {
            self.info("Added to favorites");
        }
    }

    /// Replaces what is known about which favorites exist.
    pub fn set_availability(&mut self, seen: Vec<(PathBuf, bool)>) -> bool {
        let next: HashMap<PathBuf, bool> = seen.into_iter().collect();
        if next == self.available {
            return false;
        }
        self.available = next;
        true
    }

    pub fn favorite_paths(&self) -> Vec<PathBuf> {
        self.favorites
            .items()
            .iter()
            .map(|f| f.path.clone())
            .collect()
    }

    fn is_missing(&self, path: &Path) -> bool {
        self.available.get(path) == Some(&false)
    }

    pub fn rows(&self) -> Vec<Row> {
        self.favorites
            .items()
            .iter()
            .enumerate()
            .map(|(index, f)| Row {
                index,
                label: f.label(&self.home),
                subtitle: paths::shorten_path(&paths::abbreviate(&f.path, &self.home), PATH_CHARS),
                full_path: f.path.display().to_string(),
                missing: self.is_missing(&f.path),
                is_current: self.folder.as_deref() == Some(f.path.as_path()),
            })
            .collect()
    }

    /// Starts a move to favorite `index`. A missing folder reports itself
    /// and does not navigate.
    pub fn click_favorite(&mut self, index: usize) -> Option<Job> {
        let favorite = self.favorites.items().get(index)?.clone();
        if self.is_missing(&favorite.path) {
            let label = favorite.label(&self.home);
            self.error(format!("{label}: the folder is missing"));
            return None;
        }
        if self.busy {
            return None;
        }
        self.busy = true;
        Some(Job::Go(favorite.path))
    }

    pub fn finish_go(&mut self, result: Result<(), String>) {
        self.busy = false;
        if let Err(why) = result {
            self.error(why);
        }
    }

    pub fn toggle_manage(&mut self, index: usize) {
        self.mode = match self.mode {
            Mode::Manage(open) if open == index => Mode::Browse,
            _ if index < self.favorites.len() => Mode::Manage(index),
            other => other,
        };
    }

    pub fn move_up(&mut self, index: usize) {
        if self.favorites.move_up(index) {
            self.mode = Mode::Manage(index - 1);
            self.save_favorites();
        }
    }

    pub fn move_down(&mut self, index: usize) {
        if self.favorites.move_down(index) {
            self.mode = Mode::Manage(index + 1);
            self.save_favorites();
        }
    }

    pub fn remove(&mut self, index: usize) {
        if self.favorites.remove(index).is_some() {
            self.mode = Mode::Browse;
            self.save_favorites();
        }
    }

    /// Opens the rename field; returns the text to start it with.
    pub fn start_rename(&mut self, index: usize) -> Option<String> {
        let favorite = self.favorites.items().get(index)?;
        let initial = favorite.label(&self.home);
        self.mode = Mode::Rename(index);
        Some(initial)
    }

    /// A blank name goes back to the folder's own name.
    pub fn submit_rename(&mut self, text: &str) {
        if let Mode::Rename(index) = self.mode {
            self.favorites.rename(index, text);
            self.mode = Mode::Manage(index);
            self.save_favorites();
        }
    }

    // ---- new folder ---------------------------------------------------

    pub fn start_new_folder(&mut self) -> bool {
        if self.folder.is_none() {
            self.error("Cannot create a folder: the folder is unknown");
            return false;
        }
        self.mode = Mode::NewFolder;
        true
    }

    /// Checks the typed name. A bad name stays in the field with a message.
    pub fn submit_new_folder(&mut self, text: &str) -> Option<Job> {
        if self.mode != Mode::NewFolder || self.busy {
            return None;
        }
        if self.folder.is_none() {
            self.error("Cannot create a folder: the folder is unknown");
            return None;
        }
        match validate_new_folder_name(text) {
            Ok(name) => {
                self.busy = true;
                Some(Job::NewFolder(name))
            }
            Err(e) => {
                self.error(e.to_string());
                None
            }
        }
    }

    pub fn finish_new_folder(&mut self, result: Result<PathBuf, String>) {
        self.busy = false;
        match result {
            Ok(created) => {
                self.mode = Mode::Browse;
                let shown = paths::shorten_path(&paths::abbreviate(&created, &self.home), 24);
                self.info(format!("Created {shown}"));
            }
            Err(why) => self.error(why),
        }
    }

    /// Closes any open field or edit controls.
    pub fn cancel(&mut self) {
        self.mode = match self.mode {
            Mode::Rename(i) => Mode::Manage(i),
            _ => Mode::Browse,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Dir {
            let p =
                std::env::temp_dir().join(format!("ofolder-model-{}-{tag}", std::process::id()));
            fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    fn vm(dir: &Dir) -> ViewModel {
        ViewModel::new("test".into(), "/home/ann".into(), dir.0.join("fav.json"))
    }

    fn at(vm: &mut ViewModel, folder: &str) {
        vm.set_folder(Some(PathBuf::from(folder)));
    }

    fn text(vm: &ViewModel) -> String {
        vm.message().map(|m| m.text.clone()).unwrap_or_default()
    }

    #[test]
    fn an_unknown_folder_is_shown_as_unknown_and_never_as_the_old_value() {
        let d = Dir::new("unknown");
        let mut vm = vm(&d);
        at(&mut vm, "/home/ann/Work");
        assert_eq!(vm.folder_display(), "~/Work");
        vm.set_folder(None);
        assert_eq!(vm.folder_display(), "unknown");
        assert!(vm.folder_full().is_none());
    }

    #[test]
    fn a_long_current_folder_is_shortened_but_its_full_path_is_kept() {
        let d = Dir::new("long");
        let mut vm = vm(&d);
        let long = format!("/srv/{}/final", "x".repeat(300));
        at(&mut vm, &long);
        assert!(vm.folder_display().chars().count() <= PATH_CHARS);
        assert!(vm.folder_display().ends_with("/final"));
        assert_eq!(vm.folder_full().unwrap(), long);
    }

    #[test]
    fn copying_returns_the_absolute_path_and_confirms() {
        let d = Dir::new("copy");
        let mut vm = vm(&d);
        at(&mut vm, "/home/ann/My Docs/é");
        assert_eq!(vm.copy_path().as_deref(), Some("/home/ann/My Docs/é"));
        assert_eq!(vm.message().unwrap().kind, MessageKind::Info);
    }

    #[test]
    fn copying_with_an_unknown_folder_gives_nothing_and_an_error() {
        let d = Dir::new("copy-none");
        let mut vm = vm(&d);
        assert!(vm.copy_path().is_none());
        assert_eq!(vm.message().unwrap().kind, MessageKind::Error);
    }

    #[test]
    fn adding_the_current_folder_twice_in_a_row_toggles_it_off_leaving_no_duplicate() {
        let d = Dir::new("toggle");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/a");
        vm.toggle_current_favorite();
        assert!(vm.current_is_favorite());
        assert_eq!(vm.favorites().len(), 1);
        vm.toggle_current_favorite();
        assert!(!vm.current_is_favorite());
        assert!(vm.favorites().is_empty());
    }

    #[test]
    fn favorites_are_saved_at_once_and_load_in_the_next_run() {
        let d = Dir::new("persist");
        let mut a = vm(&d);
        at(&mut a, "/srv/a");
        a.toggle_current_favorite();
        at(&mut a, "/srv/b");
        a.toggle_current_favorite();
        a.move_up(1);
        let b = vm(&d);
        let paths: Vec<_> = b.rows().into_iter().map(|r| r.full_path).collect();
        assert_eq!(paths, ["/srv/b", "/srv/a"]);
    }

    #[test]
    fn saving_without_a_known_folder_reports_it_and_adds_nothing() {
        let d = Dir::new("save-unknown");
        let mut vm = vm(&d);
        vm.toggle_current_favorite();
        assert!(vm.favorites().is_empty());
        assert_eq!(vm.message().unwrap().kind, MessageKind::Error);
    }

    #[test]
    fn clicking_a_favorite_asks_to_go_there() {
        let d = Dir::new("click");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/a");
        vm.toggle_current_favorite();
        assert_eq!(vm.click_favorite(0), Some(Job::Go(PathBuf::from("/srv/a"))));
        assert!(vm.busy());
    }

    #[test]
    fn a_failed_move_shows_the_error_text_and_frees_the_strip() {
        let d = Dir::new("go-fail");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/a");
        vm.toggle_current_favorite();
        vm.click_favorite(0);
        vm.finish_go(Err("dialog did not follow".into()));
        assert_eq!(text(&vm), "dialog did not follow");
        assert!(!vm.busy());
    }

    #[test]
    fn a_missing_favorite_is_marked_and_does_not_navigate() {
        let d = Dir::new("missing");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/gone");
        vm.toggle_current_favorite();
        vm.set_availability(vec![(PathBuf::from("/srv/gone"), false)]);
        assert!(vm.rows()[0].missing);
        assert_eq!(vm.click_favorite(0), None);
        assert!(text(&vm).contains("missing"));
        assert!(!vm.busy());
    }

    #[test]
    fn a_favorite_not_yet_checked_is_not_marked_missing() {
        let d = Dir::new("unchecked");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/a");
        vm.toggle_current_favorite();
        assert!(!vm.rows()[0].missing);
    }

    #[test]
    fn a_second_click_while_a_move_is_running_is_ignored() {
        let d = Dir::new("busy");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/a");
        vm.toggle_current_favorite();
        assert!(vm.click_favorite(0).is_some());
        assert!(vm.click_favorite(0).is_none());
    }

    #[test]
    fn rename_sets_the_display_name_and_returns_to_the_edit_controls() {
        let d = Dir::new("rename");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/photos");
        vm.toggle_current_favorite();
        vm.toggle_manage(0);
        assert_eq!(vm.start_rename(0).as_deref(), Some("photos"));
        vm.submit_rename("Pics");
        assert_eq!(vm.rows()[0].label, "Pics");
        assert_eq!(vm.mode(), Mode::Manage(0));
    }

    #[test]
    fn removing_a_favorite_drops_it_and_closes_the_edit_controls() {
        let d = Dir::new("remove");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/a");
        vm.toggle_current_favorite();
        vm.toggle_manage(0);
        vm.remove(0);
        assert!(vm.favorites().is_empty());
        assert_eq!(vm.mode(), Mode::Browse);
    }

    #[test]
    fn moving_a_favorite_keeps_the_edit_controls_on_it() {
        let d = Dir::new("move");
        let mut vm = vm(&d);
        for p in ["/srv/a", "/srv/b"] {
            at(&mut vm, p);
            vm.toggle_current_favorite();
        }
        vm.toggle_manage(0);
        vm.move_down(0);
        assert_eq!(vm.mode(), Mode::Manage(1));
        assert_eq!(vm.rows()[1].full_path, "/srv/a");
    }

    #[test]
    fn new_folder_is_refused_when_the_folder_is_unknown() {
        let d = Dir::new("nf-unknown");
        let mut vm = vm(&d);
        assert!(!vm.start_new_folder());
        assert_eq!(vm.mode(), Mode::Browse);
        assert_eq!(vm.message().unwrap().kind, MessageKind::Error);
    }

    #[test]
    fn new_folder_rejects_a_bad_name_and_keeps_the_field_open() {
        let d = Dir::new("nf-bad");
        let mut vm = vm(&d);
        at(&mut vm, "/srv");
        vm.start_new_folder();
        for bad in ["", "..", "/abs"] {
            assert_eq!(vm.submit_new_folder(bad), None, "{bad:?}");
            assert_eq!(vm.mode(), Mode::NewFolder);
            assert_eq!(vm.message().unwrap().kind, MessageKind::Error);
        }
        assert!(!vm.busy());
    }

    #[test]
    fn new_folder_with_a_good_name_asks_for_it_and_closes_the_field_on_success() {
        let d = Dir::new("nf-ok");
        let mut vm = vm(&d);
        at(&mut vm, "/srv");
        vm.start_new_folder();
        assert_eq!(
            vm.submit_new_folder(" a/b "),
            Some(Job::NewFolder("a/b".into()))
        );
        vm.finish_new_folder(Ok(PathBuf::from("/srv/a/b")));
        assert_eq!(vm.mode(), Mode::Browse);
        assert!(text(&vm).contains("/srv/a/b"));
    }

    #[test]
    fn new_folder_failure_shows_the_error_and_keeps_the_field_open() {
        let d = Dir::new("nf-fail");
        let mut vm = vm(&d);
        at(&mut vm, "/srv");
        vm.start_new_folder();
        vm.submit_new_folder("x");
        vm.finish_new_folder(Err("Permission denied".into()));
        assert_eq!(vm.mode(), Mode::NewFolder);
        assert_eq!(text(&vm), "Permission denied");
        assert!(!vm.busy());
    }

    #[test]
    fn cancel_closes_the_field_and_a_rename_falls_back_to_the_edit_controls() {
        let d = Dir::new("cancel");
        let mut vm = vm(&d);
        at(&mut vm, "/srv");
        vm.start_new_folder();
        vm.cancel();
        assert_eq!(vm.mode(), Mode::Browse);
        vm.toggle_current_favorite();
        vm.toggle_manage(0);
        vm.start_rename(0);
        vm.cancel();
        assert_eq!(vm.mode(), Mode::Manage(0));
    }

    #[test]
    fn messages_expire_after_their_time_and_errors_outlast_info() {
        let d = Dir::new("expire");
        let mut vm = vm(&d);
        at(&mut vm, "/srv");
        vm.copy_path();
        let now = Instant::now();
        assert!(!vm.expire_message(now));
        assert!(vm.expire_message(now + Duration::from_secs(3)));
        vm.set_folder(None);
        vm.copy_path();
        assert!(!vm.expire_message(Instant::now() + Duration::from_secs(3)));
        assert!(vm.expire_message(Instant::now() + Duration::from_secs(11)));
    }

    #[test]
    fn a_corrupt_favorites_file_starts_empty_with_a_visible_message() {
        let d = Dir::new("corrupt");
        fs::write(d.0.join("fav.json"), "garbage").unwrap();
        let vm = vm(&d);
        assert!(vm.favorites().is_empty());
        assert!(text(&vm).contains("fav.json.corrupt"));
        assert!(d.0.join("fav.json.corrupt").exists());
    }

    #[test]
    fn a_save_failure_is_reported_in_the_strip() {
        let d = Dir::new("save-fail");
        // The favorites path is a folder, so renaming over it fails.
        fs::create_dir_all(d.0.join("fav.json")).unwrap();
        let mut vm = vm(&d);
        at(&mut vm, "/srv/a");
        vm.toggle_current_favorite();
        assert_eq!(vm.message().unwrap().kind, MessageKind::Error);
        assert!(text(&vm).contains("could not save"));
    }

    #[test]
    fn the_current_folder_is_flagged_in_the_rows() {
        let d = Dir::new("current-row");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/a");
        vm.toggle_current_favorite();
        assert!(vm.rows()[0].is_current);
        at(&mut vm, "/srv/other");
        assert!(!vm.rows()[0].is_current);
    }
}
