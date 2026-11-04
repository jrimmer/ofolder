//! What the panels show and what each user action does to it, with no
//! window involved. Actions that need the dialog or the filesystem (moving
//! the dialog, creating a folder, reading subfolders, searching, launching a
//! program) are returned as a job for the caller to run off the UI thread,
//! and the answer comes back through a `finish_*` method.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ofolder_core::places::{self, History, Place, PlaceKind};
use ofolder_core::recents::Recents;
use ofolder_core::search::Ended;

use crate::StripOptions;
use crate::expand::{Expansion, RowKey, Section, Toggle};
use crate::favorites::{self, AddOutcome, Favorites};
use crate::finder::{self, Candidate, Finder, SearchJob};
use crate::lines::{self, Context, Icon, Line, Root};
use crate::menu::{RowAction, RowMenu};
use crate::naming::validate_new_folder_name;
use crate::paths;
use crate::sources::{self, RecentsRead, Sources};

/// How long an informational message stays.
const INFO_TTL: Duration = Duration::from_millis(2500);
/// The shortest time an error stays; the user needs time to read it.
const ERROR_TTL: Duration = Duration::from_secs(10);
/// The longest an error stays without the pointer on it.
const ERROR_TTL_MAX: Duration = Duration::from_secs(30);
/// Reading time added per character past the first eighty.
const READ_PER_CHAR: Duration = Duration::from_millis(60);
/// What is left of a message's time when the pointer leaves it, at least.
const AFTER_HOVER: Duration = Duration::from_secs(3);
/// How long a problem found at startup stays.
const NOTICE_TTL: Duration = Duration::from_secs(30);
/// Rows a recents or search list shows before "Show all".
pub const LIST_CAP: usize = 15;

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
    /// While the pointer rests on the message it does not go.
    held: bool,
    /// What the user was doing when it was said.
    context: &'static str,
}

impl Message {
    fn expired(&self, now: Instant) -> bool {
        !self.held && now.saturating_duration_since(self.created) >= self.ttl
    }
}

/// How long an error stays on screen: long enough to read, longer for a
/// long one, within limits.
pub fn error_ttl(text: &str) -> Duration {
    let extra = u32::try_from(text.chars().count().saturating_sub(80)).unwrap_or(u32::MAX);
    (ERROR_TTL + READ_PER_CHAR.saturating_mul(extra)).min(ERROR_TTL_MAX)
}

/// A message as shown, for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub at: std::time::SystemTime,
    pub kind: MessageKind,
    /// What the user was doing, such as "copy-path".
    pub context: &'static str,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Browse,
    /// A text field asks for a new display name for one favorite.
    Rename(usize),
    /// A text field asks for the name of a new folder.
    NewFolder,
}

/// Which applications' recent folders are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    AllApps,
    ThisApp,
}

/// Work that must touch the dialog or start a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    Go(PathBuf),
    NewFolder(String),
    OpenFileManager(PathBuf),
    OpenTerminal(PathBuf),
}

/// What a click in a row's menu asks the caller to do besides the state change.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Job(Job),
    /// Text for the clipboard.
    Copy(String),
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
    history: History,
    places: Vec<Place>,
    /// Folders file-manager windows are showing, listed after the places.
    open_windows: Vec<Place>,
    recents: Recents,
    recents_enabled: bool,
    /// The rail layout the settings file gave when the dialog opened.
    rails: ofolder_core::rails::RailLayout,
    scope: Scope,
    app_key: String,
    expansion: Expansion,
    menu: RowMenu,
    finder: Finder,
    show_all: HashSet<Section>,
    rename_seed: Option<String>,
    sources: Sources,
    /// The last recents warning shown, so a file that stays unreadable is
    /// reported once and not at every poll.
    last_recents_warning: Option<String>,
    /// Messages shown and not yet written to the log.
    shown: Vec<Shown>,
    /// The preview panel's settings, as read when the dialog opened.
    preview: ofolder_core::preview::PreviewSettings,
}

/// Characters of path that fit a panel's width.
pub const PATH_CHARS: usize = 30;

impl ViewModel {
    pub fn new(options: &StripOptions) -> ViewModel {
        let loaded = favorites::load(&options.favorites_path);
        let settings = sources::read_settings(&options.settings_path, &options.home);
        let mut vm = ViewModel {
            label: options.label.clone(),
            home: options.home.clone(),
            favorites_path: options.favorites_path.clone(),
            favorites: loaded.favorites,
            folder: None,
            available: HashMap::new(),
            message: None,
            mode: Mode::Browse,
            busy: false,
            history: History::default(),
            places: Vec::new(),
            open_windows: Vec::new(),
            recents: Recents::default(),
            recents_enabled: settings.recents_enabled,
            rails: settings.rails,
            preview: settings.preview,
            scope: Scope::AllApps,
            app_key: options.app_key.clone(),
            expansion: Expansion::default(),
            menu: RowMenu::default(),
            finder: Finder::default(),
            show_all: HashSet::new(),
            rename_seed: None,
            last_recents_warning: None,
            shown: Vec::new(),
            sources: Sources {
                user_dirs_path: options.user_dirs_path.clone(),
                mountinfo_path: options.mountinfo_path.clone(),
                recents_path: options.recents_path.clone(),
                home: options.home.clone(),
                user: options.user.clone(),
            },
        };
        let notices: Vec<String> = loaded.notice.into_iter().chain(settings.warnings).collect();
        if !notices.is_empty() {
            vm.say(
                "startup",
                notices.join("; "),
                MessageKind::Error,
                NOTICE_TTL,
            );
        }
        vm
    }

    /// Where the lists are read from, for the polling thread.
    pub fn sources(&self) -> Sources {
        self.sources.clone()
    }

    fn say(&mut self, context: &'static str, text: String, kind: MessageKind, ttl: Duration) {
        if kind == MessageKind::Error {
            eprintln!("ofolder: {text}");
        }
        self.shown.push(Shown {
            at: std::time::SystemTime::now(),
            kind,
            context,
            text: text.clone(),
        });
        // A note from another action does not push an error off the screen
        // before it can be read; it is still in the log. The same action
        // succeeding on a second try does replace its own error.
        let error_up = self.message.as_ref().is_some_and(|m| {
            m.kind == MessageKind::Error && m.context != context && !m.expired(Instant::now())
        });
        if kind == MessageKind::Info && error_up {
            return;
        }
        self.message = Some(Message {
            text,
            kind,
            created: Instant::now(),
            ttl,
            held: false,
            context,
        });
    }

    fn info(&mut self, context: &'static str, text: impl Into<String>) {
        self.say(context, text.into(), MessageKind::Info, INFO_TTL);
    }

    fn error(&mut self, context: &'static str, text: impl Into<String>) {
        let text = text.into();
        let ttl = error_ttl(&text);
        self.say(context, text, MessageKind::Error, ttl);
    }

    /// Shows an error that arose outside the view-model, such as a copy to
    /// the clipboard that failed.
    pub fn show_error(&mut self, context: &'static str, text: &str) {
        self.error(context, text);
    }

    /// The messages shown since the last call, for the log.
    pub fn take_shown(&mut self) -> Vec<Shown> {
        std::mem::take(&mut self.shown)
    }

    /// The pointer came onto the message (`true`) or left it.
    pub fn hold_message(&mut self, held: bool) {
        if let Some(m) = self.message.as_mut() {
            if m.held && !held {
                // Leave a few seconds to move away and back.
                let now = Instant::now();
                let left = m
                    .ttl
                    .saturating_sub(now.saturating_duration_since(m.created));
                if left < AFTER_HOVER {
                    m.created = now;
                    m.ttl = AFTER_HOVER;
                }
            }
            m.held = held;
        }
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
        let over = self.message.as_ref().is_some_and(|m| m.expired(now));
        if over {
            self.message = None;
        }
        over
    }

    // ---- the current folder -------------------------------------------

    /// Records what the dialog shows now. `None` replaces any old value, so
    /// a folder that can no longer be read is never shown as current. A
    /// folder is added to the history only when it is new.
    pub fn set_folder(&mut self, folder: Option<PathBuf>) -> bool {
        if self.folder == folder {
            return false;
        }
        if let Some(f) = &folder {
            self.history.observe(f);
        }
        self.folder = folder;
        true
    }

    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    /// The folder for a tooltip: home as `~`, cut to fit, or "unknown".
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

    /// The text to place on the clipboard for the current folder, with a
    /// confirmation message. The caller writes it to the clipboard.
    pub fn copy_path(&mut self) -> Option<String> {
        let Some(folder) = self.folder.clone() else {
            self.error("copy-path", "Cannot copy: the folder is unknown");
            return None;
        };
        self.copy_text(&folder)
    }

    /// The same for any folder.
    pub fn copy_text(&mut self, path: &Path) -> Option<String> {
        let Some(text) = path.to_str() else {
            self.error("copy-path", "Cannot copy: the path is not valid text");
            return None;
        };
        let text = text.to_owned();
        self.info("copy-path", "Path copied");
        Some(text)
    }

    // ---- history ------------------------------------------------------

    pub fn can_go_back(&self) -> bool {
        self.history.can_go_back()
    }

    pub fn can_go_forward(&self) -> bool {
        self.history.can_go_forward()
    }

    pub fn can_go_up(&self) -> bool {
        self.folder.as_deref().and_then(places::parent_of).is_some()
    }

    pub fn go_back(&mut self) -> Option<Job> {
        if self.busy {
            return None;
        }
        let target = self.history.back()?;
        let job = self.go_to(&target);
        if job.is_none() {
            self.history.step_failed();
        }
        job
    }

    pub fn go_forward(&mut self) -> Option<Job> {
        if self.busy {
            return None;
        }
        let target = self.history.forward()?;
        let job = self.go_to(&target);
        if job.is_none() {
            self.history.step_failed();
        }
        job
    }

    pub fn go_up(&mut self) -> Option<Job> {
        let parent = self.folder.as_deref().and_then(places::parent_of)?;
        self.go_to(&parent)
    }

    // ---- favorites ----------------------------------------------------

    /// Writes the list; on failure the message says so and the change stays
    /// in memory for this session.
    fn save_favorites(&mut self) -> bool {
        match favorites::save(&self.favorites_path, &self.favorites) {
            Ok(()) => true,
            Err(e) => {
                self.error("favorite", e.to_string());
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
            self.error("favorite", "Cannot save: the folder is unknown");
            return;
        };
        self.toggle_favorite(&folder);
    }

    /// Adds `path` to the favorites, or removes it when it is one.
    pub fn toggle_favorite(&mut self, path: &Path) {
        if self.favorites.remove_path(path) {
            if self.save_favorites() {
                self.info("favorite", "Removed from favorites");
            }
            return;
        }
        if self.favorites.add(path.to_path_buf()) == AddOutcome::Added && self.save_favorites() {
            self.info("favorite", "Added to favorites");
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

    /// Starts a move to `path`. A folder known to be missing reports itself
    /// and does not navigate.
    pub fn go_to(&mut self, path: &Path) -> Option<Job> {
        if self.is_missing(path) {
            let shown = paths::shorten_path(&paths::abbreviate(path, &self.home), 24);
            self.error("go", format!("{shown}: the folder is missing"));
            return None;
        }
        if self.busy {
            return None;
        }
        self.busy = true;
        Some(Job::Go(path.to_path_buf()))
    }

    /// Starts a move to favorite `index`.
    pub fn click_favorite(&mut self, index: usize) -> Option<Job> {
        let path = self.favorites.items().get(index)?.path.clone();
        self.go_to(&path)
    }

    pub fn finish_go(&mut self, result: Result<(), String>) {
        self.busy = false;
        if let Err(why) = result {
            self.history.step_failed();
            self.error("go", why);
        }
    }

    pub fn move_up(&mut self, index: usize) {
        if self.favorites.move_up(index) {
            self.save_favorites();
        }
    }

    pub fn move_down(&mut self, index: usize) {
        if self.favorites.move_down(index) {
            self.save_favorites();
        }
    }

    /// Opens the rename field. The text to start it with waits in
    /// [`ViewModel::take_rename_seed`].
    pub fn start_rename(&mut self, index: usize) -> bool {
        let Some(favorite) = self.favorites.items().get(index) else {
            return false;
        };
        self.rename_seed = Some(favorite.label(&self.home));
        self.mode = Mode::Rename(index);
        true
    }

    pub fn take_rename_seed(&mut self) -> Option<String> {
        self.rename_seed.take()
    }

    /// A blank name goes back to the folder's own name.
    pub fn submit_rename(&mut self, text: &str) {
        if let Mode::Rename(index) = self.mode {
            self.favorites.rename(index, text);
            self.mode = Mode::Browse;
            self.save_favorites();
        }
    }

    // ---- new folder ---------------------------------------------------

    pub fn start_new_folder(&mut self) -> bool {
        if self.folder.is_none() {
            self.error(
                "new-folder",
                "Cannot create a folder: the folder is unknown",
            );
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
            self.error(
                "new-folder",
                "Cannot create a folder: the folder is unknown",
            );
            return None;
        }
        match validate_new_folder_name(text) {
            Ok(name) => {
                self.busy = true;
                Some(Job::NewFolder(name))
            }
            Err(e) => {
                self.error("new-folder", e.to_string());
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
                self.info("new-folder", format!("Created {shown}"));
            }
            Err(why) => self.error("new-folder", why),
        }
    }

    /// Closes any open field.
    pub fn cancel(&mut self) {
        self.mode = Mode::Browse;
        self.rename_seed = None;
    }

    // ---- launching programs -------------------------------------------

    pub fn open_in_file_manager(&mut self, folder: &Path) -> Job {
        Job::OpenFileManager(folder.to_path_buf())
    }

    pub fn open_in_terminal(&mut self, folder: &Path) -> Job {
        Job::OpenTerminal(folder.to_path_buf())
    }

    /// For the buttons that act on the dialog's folder.
    pub fn open_current(&mut self, terminal: bool) -> Option<Job> {
        let Some(folder) = self.folder.clone() else {
            self.error("open", "Cannot open: the folder is unknown");
            return None;
        };
        Some(if terminal {
            self.open_in_terminal(&folder)
        } else {
            self.open_in_file_manager(&folder)
        })
    }

    pub fn finish_launch(&mut self, what: &str, result: Result<(), String>) {
        match result {
            Ok(()) => self.info("open", format!("Opened in {what}")),
            Err(why) => self.error("open", why),
        }
    }

    // ---- places and recents -------------------------------------------

    /// Stores the folders the file-manager windows show now. Each is named for its
    /// folder and the program, so two windows on different folders read apart.
    pub fn set_open_windows(&mut self, folders: Vec<(String, PathBuf)>) -> bool {
        let windows: Vec<Place> = folders
            .into_iter()
            .map(|(program, path)| Place {
                name: format!("{} (in {program})", folder_name(&path)),
                path,
                kind: PlaceKind::OpenWindow,
            })
            .collect();
        if self.open_windows == windows {
            return false;
        }
        self.open_windows = windows;
        true
    }

    pub fn set_places(&mut self, places: Vec<Place>) -> bool {
        if self.places == places {
            return false;
        }
        self.places = places;
        true
    }

    /// Stores a fresh read of the recents. A warning is shown once, when it
    /// first appears.
    pub fn set_recents(&mut self, read: RecentsRead) -> bool {
        if read.warning != self.last_recents_warning {
            self.last_recents_warning.clone_from(&read.warning);
            if let Some(warning) = read.warning {
                self.error("recents", warning);
            }
        }
        if self.recents == read.recents {
            return false;
        }
        self.recents = read.recents;
        true
    }

    pub fn recents_enabled(&self) -> bool {
        self.recents_enabled
    }

    /// The rail layout read when the dialog opened.
    pub fn rails(&self) -> &ofolder_core::rails::RailLayout {
        &self.rails
    }

    pub fn preview_settings(&self) -> ofolder_core::preview::PreviewSettings {
        self.preview
    }

    /// Shows a note that arose outside the view-model.
    pub fn show_info(&mut self, context: &'static str, text: &str) {
        self.info(context, text);
    }

    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// Whether recents can be limited to this application: only when it is
    /// known which application it is.
    pub fn can_filter_by_app(&self) -> bool {
        !self.app_key.is_empty()
    }

    pub fn set_scope(&mut self, scope: Scope) {
        if scope == Scope::ThisApp && !self.can_filter_by_app() {
            return;
        }
        self.scope = scope;
    }

    fn app_filter(&self) -> Option<&str> {
        (self.scope == Scope::ThisApp).then_some(self.app_key.as_str())
    }

    fn recent_folder_paths(&self) -> Vec<PathBuf> {
        self.recents
            .folders_for(self.app_filter())
            .into_iter()
            .map(|f| PathBuf::from(&f.path))
            .collect()
    }

    // ---- subfolders, rows and menus -----------------------------------

    /// Opens or closes a row's subfolders. Returns the row when its
    /// subfolders must be read now, off the UI thread.
    pub fn toggle_expand(&mut self, key: RowKey) -> Option<RowKey> {
        match self.expansion.toggle(key.clone()) {
            Toggle::Load => Some(key),
            Toggle::Closed => None,
        }
    }

    pub fn finish_browse(&mut self, key: &RowKey, result: Result<Vec<PathBuf>, String>) {
        if let Err(why) = &result {
            eprintln!("ofolder: {why}");
        }
        self.expansion.finish(key, result);
    }

    pub fn toggle_menu(&mut self, key: RowKey) {
        self.menu.toggle(key);
    }

    pub fn toggle_show_all(&mut self, section: Section) {
        if !self.show_all.remove(&section) {
            self.show_all.insert(section);
        }
    }

    /// Runs a menu action on a folder.
    pub fn row_action(&mut self, action: RowAction, path: &Path) -> Effect {
        let index = self.favorites.items().iter().position(|f| f.path == path);
        let effect = match action {
            RowAction::Go => self.go_to(path).map_or(Effect::None, Effect::Job),
            RowAction::CopyPath => self.copy_text(path).map_or(Effect::None, Effect::Copy),
            RowAction::OpenFileManager => Effect::Job(self.open_in_file_manager(path)),
            RowAction::OpenTerminal => Effect::Job(self.open_in_terminal(path)),
            RowAction::ToggleFavorite => {
                self.toggle_favorite(path);
                Effect::None
            }
            RowAction::MoveUp => {
                if let Some(i) = index {
                    self.move_up(i);
                }
                Effect::None
            }
            RowAction::MoveDown => {
                if let Some(i) = index {
                    self.move_down(i);
                }
                Effect::None
            }
            RowAction::Rename => {
                if let Some(i) = index {
                    self.start_rename(i);
                }
                Effect::None
            }
        };
        if !action.keeps_menu_open() {
            self.menu.close();
        }
        effect
    }

    /// Everything the finder looks through: favorites, then recent folders,
    /// then places, each folder once.
    fn candidates(&self) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = Vec::new();
        let mut add = |path: PathBuf, label: String| {
            if !out.iter().any(|c| c.path == path) {
                out.push(Candidate { path, label });
            }
        };
        for f in self.favorites.items() {
            add(f.path.clone(), f.label(&self.home));
        }
        for f in &self.recents.folders {
            let path = PathBuf::from(&f.path);
            let label = folder_name(&path);
            add(path, label);
        }
        for p in self.places.iter().chain(&self.open_windows) {
            add(p.path.clone(), p.name.clone());
        }
        out
    }

    fn roots_for(&self, section: Section) -> Vec<Root> {
        let plain = |path: PathBuf| Root {
            label: folder_name(&path),
            path,
            icon: Icon::Folder,
        };
        match section {
            Section::Favorites => self
                .favorites
                .items()
                .iter()
                .map(|f| Root {
                    path: f.path.clone(),
                    label: f.label(&self.home),
                    icon: Icon::Folder,
                })
                .collect(),
            Section::Places => self
                .places
                .iter()
                .chain(&self.open_windows)
                .map(|p| Root {
                    path: p.path.clone(),
                    label: p.name.clone(),
                    icon: match p.kind {
                        PlaceKind::Home => Icon::Home,
                        PlaceKind::UserFolder => Icon::Folder,
                        PlaceKind::Drive => Icon::Drive,
                        PlaceKind::OpenWindow => Icon::Folder,
                    },
                })
                .collect(),
            Section::RecentFolders => self.recent_folder_paths().into_iter().map(plain).collect(),
            Section::Ranked => self
                .find_results()
                .into_iter()
                .map(|c| Root {
                    path: c.path,
                    label: c.label,
                    icon: Icon::Folder,
                })
                .collect(),
            Section::Found => self.finder.found().iter().cloned().map(plain).collect(),
            Section::RecentFiles => Vec::new(),
        }
    }

    /// The lines of one list, with its open rows, menus and cap applied.
    pub fn section_lines(&self, section: Section) -> Vec<Line> {
        if section == Section::RecentFiles {
            return self.recent_file_lines();
        }
        let mut roots = self.roots_for(section);
        let capped = matches!(section, Section::RecentFolders | Section::Found);
        let total = roots.len();
        let showing_all = self.show_all.contains(&section);
        if capped && !showing_all {
            roots.truncate(LIST_CAP);
        }
        let renaming = match self.mode {
            Mode::Rename(i) if section == Section::Favorites => {
                self.favorites.items().get(i).map(|f| f.path.as_path())
            }
            _ => None,
        };
        let missing = |p: &Path| self.is_missing(p);
        let ctx = Context {
            section,
            expansion: &self.expansion,
            menu: &self.menu,
            favorites: &self.favorites,
            current: self.folder.as_deref(),
            home: &self.home,
            missing: &missing,
            renaming,
            highlight: if section == Section::Ranked {
                self.finder.highlight(roots.len())
            } else {
                None
            },
        };
        let mut out = lines::flatten(&roots, &ctx);
        if capped && total > LIST_CAP {
            out.push(Line::More {
                section,
                hidden: if showing_all { 0 } else { total - LIST_CAP },
            });
        }
        out
    }

    fn recent_file_lines(&self) -> Vec<Line> {
        let files = self.recents.files_for(self.app_filter());
        let total = files.len();
        let showing_all = self.show_all.contains(&Section::RecentFiles);
        let mut out: Vec<Line> = files
            .into_iter()
            .take(if showing_all { usize::MAX } else { LIST_CAP })
            .map(|f| {
                let path = PathBuf::from(&f.path);
                let folder = path.parent().map_or_else(String::new, |p| {
                    paths::shorten_path(&paths::abbreviate(p, &self.home), 26)
                });
                Line::File {
                    label: folder_name(&path),
                    subtitle: folder,
                    path,
                }
            })
            .collect();
        if total > LIST_CAP {
            out.push(Line::More {
                section: Section::RecentFiles,
                hidden: if showing_all { 0 } else { total - LIST_CAP },
            });
        }
        out
    }

    /// Opens the folder a recent file is in; the dialog cannot select the file.
    pub fn go_to_file_folder(&mut self, file: &Path) -> Option<Job> {
        let folder = file.parent()?.to_path_buf();
        self.go_to(&folder)
    }

    // ---- find ---------------------------------------------------------

    pub fn find_query(&self) -> &str {
        self.finder.query()
    }

    pub fn find_set_query(&mut self, text: &str) {
        self.finder.set_query(text);
    }

    pub fn find_move(&mut self, delta: isize) {
        let len = self.find_results().len();
        self.finder.move_highlight(delta, len);
    }

    pub fn find_results(&self) -> Vec<Candidate> {
        finder::rank_candidates(self.finder.query(), &self.candidates(), finder::SHOWN)
    }

    /// Enter in the find box: go to the highlighted result.
    pub fn find_enter(&mut self) -> Option<Job> {
        let results = self.find_results();
        let at = self.finder.highlight(results.len())?;
        self.go_to(&results[at].path)
    }

    /// Escape in the find box: stop a search, else clear the box.
    pub fn find_escape(&mut self) {
        if !self.finder.stop() {
            self.finder.clear();
        }
    }

    pub fn start_search(&mut self) -> Option<SearchJob> {
        match self.finder.begin(self.folder.as_deref()) {
            Ok(job) => Some(job),
            Err(why) => {
                self.error("find", why);
                None
            }
        }
    }

    pub fn stop_search(&mut self) {
        self.finder.stop();
    }

    pub fn finish_search(&mut self, id: u64, found: Vec<PathBuf>, ended: Ended) {
        self.finder.finish(id, found, ended);
    }

    pub fn search_running(&self) -> bool {
        self.finder.running()
    }

    pub fn search_status(&self) -> Option<String> {
        self.finder.status()
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ofolder_core::places::{Place, PlaceKind};
    use ofolder_core::recents::{RecentFile, RecentFolder};
    use std::fs;

    impl ViewModel {
        fn rows(&self) -> Vec<lines::FolderLine> {
            self.section_lines(Section::Favorites)
                .into_iter()
                .filter_map(|l| match l {
                    Line::Folder(f) => Some(f),
                    _ => None,
                })
                .collect()
        }

        fn rows_labels(&self) -> Vec<String> {
            self.rows().into_iter().map(|r| r.label).collect()
        }
    }

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

    fn options(dir: &Dir) -> StripOptions {
        StripOptions {
            label: "test".into(),
            favorites_path: dir.0.join("fav.json"),
            home: "/home/ann".into(),
            app_key: "org.app.A".into(),
            recents_path: dir.0.join("recents.json"),
            settings_path: dir.0.join("settings.toml"),
            user_dirs_path: dir.0.join("user-dirs.dirs"),
            mountinfo_path: dir.0.join("mountinfo"),
            user: "ann".into(),
            palette: Default::default(),
            messages_path: None,
        }
    }

    fn vm(dir: &Dir) -> ViewModel {
        ViewModel::new(&options(dir))
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
    fn a_long_error_stays_longer_within_a_limit() {
        assert_eq!(error_ttl("short"), Duration::from_secs(10));
        assert!(error_ttl(&"word ".repeat(40)) > Duration::from_secs(10));
        assert_eq!(error_ttl(&"x".repeat(5_000)), Duration::from_secs(30));
    }

    #[test]
    fn a_note_does_not_push_an_unread_error_off_the_screen_but_is_logged() {
        let d = Dir::new("keep-error");
        let mut vm = vm(&d);
        vm.copy_path();
        assert_eq!(vm.message().map(|m| m.kind), Some(MessageKind::Error));
        at(&mut vm, "/srv");
        vm.toggle_current_favorite();
        assert_eq!(vm.message().map(|m| m.kind), Some(MessageKind::Error));
        // The same action succeeding replaces its own error.
        vm.copy_path();
        assert_eq!(vm.message().map(|m| m.kind), Some(MessageKind::Info));
        let shown = vm.take_shown();
        let contexts: Vec<_> = shown.iter().map(|m| (m.kind, m.context)).collect();
        assert_eq!(
            contexts,
            [
                (MessageKind::Error, "copy-path"),
                (MessageKind::Info, "favorite"),
                (MessageKind::Info, "copy-path")
            ]
        );
        assert!(
            vm.take_shown().is_empty(),
            "each message is handed over once"
        );
    }

    #[test]
    fn a_message_under_the_pointer_stays_and_has_a_moment_after_it_leaves() {
        let d = Dir::new("hold");
        let mut vm = vm(&d);
        at(&mut vm, "/srv");
        vm.copy_path();
        vm.hold_message(true);
        assert!(!vm.expire_message(Instant::now() + Duration::from_secs(60)));
        vm.hold_message(false);
        assert!(!vm.expire_message(Instant::now() + Duration::from_secs(2)));
        assert!(vm.expire_message(Instant::now() + Duration::from_secs(4)));
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

    fn recents_with(folders: &[(&str, &[&str])], files: &[&str]) -> RecentsRead {
        RecentsRead {
            recents: Recents {
                folders: folders
                    .iter()
                    .map(|(p, apps)| RecentFolder {
                        path: (*p).into(),
                        apps: apps.iter().map(|a| (*a).into()).collect(),
                        at: 1,
                    })
                    .collect(),
                files: files
                    .iter()
                    .map(|p| RecentFile {
                        path: (*p).into(),
                        apps: vec!["org.app.A".into()],
                        at: 1,
                    })
                    .collect(),
            },
            warning: None,
        }
    }

    fn labels(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .filter_map(|l| match l {
                Line::Folder(f) => Some(f.label.clone()),
                Line::File { label, .. } => Some(label.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn rename_sets_the_display_name_through_the_seeded_field() {
        let d = Dir::new("rename");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/photos");
        vm.toggle_current_favorite();
        assert!(vm.start_rename(0));
        assert_eq!(vm.take_rename_seed().as_deref(), Some("photos"));
        vm.submit_rename("Pics");
        assert_eq!(vm.rows_labels(), ["Pics"]);
        assert_eq!(vm.mode(), Mode::Browse);
    }

    #[test]
    fn removing_a_favorite_from_its_menu_drops_it_and_closes_the_menu() {
        let d = Dir::new("remove");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/a");
        vm.toggle_current_favorite();
        vm.toggle_menu(RowKey::new(Section::Favorites, "/srv/a"));
        vm.row_action(RowAction::ToggleFavorite, Path::new("/srv/a"));
        assert!(vm.favorites().is_empty());
        assert!(!matches!(
            vm.section_lines(Section::Favorites).get(1),
            Some(Line::Menu { .. })
        ));
    }

    #[test]
    fn moving_a_favorite_from_its_menu_keeps_the_menu_open() {
        let d = Dir::new("move");
        let mut vm = vm(&d);
        for p in ["/srv/a", "/srv/b"] {
            at(&mut vm, p);
            vm.toggle_current_favorite();
        }
        let key = RowKey::new(Section::Favorites, "/srv/a");
        vm.toggle_menu(key);
        vm.row_action(RowAction::MoveDown, Path::new("/srv/a"));
        assert_eq!(vm.rows_labels(), ["b", "a"]);
        let lines = vm.section_lines(Section::Favorites);
        assert!(lines.iter().any(|l| matches!(l, Line::Menu { .. })));
    }

    #[test]
    fn cancel_closes_the_new_folder_field_and_a_rename() {
        let d = Dir::new("cancel");
        let mut vm = vm(&d);
        at(&mut vm, "/srv");
        vm.start_new_folder();
        vm.cancel();
        assert_eq!(vm.mode(), Mode::Browse);
        vm.toggle_current_favorite();
        vm.start_rename(0);
        vm.cancel();
        assert_eq!(vm.mode(), Mode::Browse);
        assert!(vm.take_rename_seed().is_none());
    }

    #[test]
    fn history_steps_back_and_forward_through_the_folders_the_dialog_showed() {
        let d = Dir::new("history");
        let mut vm = vm(&d);
        for p in ["/a", "/b", "/c"] {
            at(&mut vm, p);
        }
        assert!(vm.can_go_back() && !vm.can_go_forward());
        assert_eq!(vm.go_back(), Some(Job::Go("/b".into())));
        vm.finish_go(Ok(()));
        at(&mut vm, "/b");
        assert_eq!(vm.go_back(), Some(Job::Go("/a".into())));
        vm.finish_go(Ok(()));
        at(&mut vm, "/a");
        assert_eq!(vm.go_forward(), Some(Job::Go("/b".into())));
    }

    #[test]
    fn a_failed_back_step_leaves_the_history_where_the_dialog_still_is() {
        let d = Dir::new("history-failed");
        let mut vm = vm(&d);
        for p in ["/a", "/b"] {
            at(&mut vm, p);
        }
        assert_eq!(vm.go_back(), Some(Job::Go("/a".into())));
        vm.finish_go(Err("the dialog did not move".into()));
        assert!(vm.can_go_back() && !vm.can_go_forward());
        assert_eq!(vm.go_back(), Some(Job::Go("/a".into())));
    }

    #[test]
    fn a_back_step_whose_target_is_gone_does_not_move_the_history() {
        let d = Dir::new("history-gone");
        let mut vm = vm(&d);
        for p in ["/nowhere-ofolder/a", "/nowhere-ofolder/b"] {
            at(&mut vm, p);
        }
        // The folder to go back to is known to be missing, so no move is started.
        vm.set_availability(vec![(PathBuf::from("/nowhere-ofolder/a"), false)]);
        assert_eq!(vm.go_back(), None);
        assert!(vm.can_go_back() && !vm.can_go_forward());
    }

    #[test]
    fn the_old_folder_read_during_a_back_step_does_not_change_the_history() {
        let d = Dir::new("history-stale");
        let mut vm = vm(&d);
        for p in ["/a", "/b", "/c"] {
            at(&mut vm, p);
        }
        assert_eq!(vm.go_back(), Some(Job::Go("/b".into())));
        at(&mut vm, "/c");
        vm.finish_go(Ok(()));
        at(&mut vm, "/b");
        assert_eq!(vm.go_back(), Some(Job::Go("/a".into())));
    }

    #[test]
    fn arriving_after_a_back_step_is_not_recorded_as_a_new_visit() {
        let d = Dir::new("history-pending");
        let mut vm = vm(&d);
        for p in ["/a", "/b"] {
            at(&mut vm, p);
        }
        vm.go_back();
        vm.finish_go(Ok(()));
        at(&mut vm, "/a");
        assert!(vm.can_go_forward());
    }

    #[test]
    fn parent_goes_one_level_up_and_is_unavailable_with_no_folder_or_at_the_top() {
        let d = Dir::new("parent");
        let mut vm = vm(&d);
        assert!(!vm.can_go_up());
        assert_eq!(vm.go_up(), None);
        at(&mut vm, "/a/b");
        assert_eq!(vm.go_up(), Some(Job::Go("/a".into())));
        vm.finish_go(Ok(()));
        at(&mut vm, "/");
        assert!(!vm.can_go_up());
    }

    #[test]
    fn a_folder_the_dialog_shows_twice_is_one_history_entry() {
        let d = Dir::new("history-same");
        let mut vm = vm(&d);
        at(&mut vm, "/a");
        vm.set_folder(None);
        at(&mut vm, "/a");
        assert!(!vm.can_go_back());
    }

    #[test]
    fn folders_of_open_file_managers_follow_the_places_and_name_their_program() {
        let d = Dir::new("open-windows");
        let mut vm = vm(&d);
        vm.set_places(vec![Place {
            name: "Home".into(),
            path: "/home/ann".into(),
            kind: PlaceKind::Home,
        }]);
        assert!(vm.set_open_windows(vec![
            ("Flea".into(), "/srv/photos".into()),
            ("Flea".into(), "/srv/taxes".into()),
        ]));
        let lines = vm.section_lines(Section::Places);
        assert_eq!(
            labels(&lines),
            ["Home", "photos (in Flea)", "taxes (in Flea)"]
        );
    }

    #[test]
    fn an_unchanged_set_of_open_windows_reports_no_change_and_a_closed_window_goes() {
        let d = Dir::new("open-windows-change");
        let mut vm = vm(&d);
        let two = vec![
            ("Flea".to_owned(), PathBuf::from("/srv/a")),
            ("Flea".to_owned(), PathBuf::from("/srv/b")),
        ];
        assert!(vm.set_open_windows(two.clone()));
        assert!(!vm.set_open_windows(two));
        assert!(vm.set_open_windows(vec![("Flea".into(), "/srv/a".into())]));
        assert_eq!(labels(&vm.section_lines(Section::Places)), ["a (in Flea)"]);
        assert!(vm.set_open_windows(Vec::new()));
        assert!(vm.section_lines(Section::Places).is_empty());
    }

    #[test]
    fn places_are_listed_with_home_first_and_drives_marked() {
        let d = Dir::new("places");
        let mut vm = vm(&d);
        vm.set_places(vec![
            Place {
                name: "Home".into(),
                path: "/home/ann".into(),
                kind: PlaceKind::Home,
            },
            Place {
                name: "USB".into(),
                path: "/run/media/ann/USB".into(),
                kind: PlaceKind::Drive,
            },
        ]);
        let lines = vm.section_lines(Section::Places);
        assert_eq!(labels(&lines), ["Home", "USB"]);
        assert!(matches!(&lines[1], Line::Folder(f) if f.icon == Icon::Drive));
    }

    #[test]
    fn replacing_the_places_reports_whether_anything_changed() {
        let d = Dir::new("places-change");
        let mut vm = vm(&d);
        let one = vec![Place {
            name: "Home".into(),
            path: "/home/ann".into(),
            kind: PlaceKind::Home,
        }];
        assert!(vm.set_places(one.clone()));
        assert!(!vm.set_places(one));
        assert!(vm.set_places(Vec::new()));
    }

    #[test]
    fn recent_folders_list_newest_first_and_can_be_limited_to_this_app() {
        let d = Dir::new("recents-scope");
        let mut vm = vm(&d);
        vm.set_recents(recents_with(
            &[("/srv/b", &["org.app.B"]), ("/srv/a", &["org.app.A"])],
            &[],
        ));
        assert_eq!(
            labels(&vm.section_lines(Section::RecentFolders)),
            ["b", "a"]
        );
        vm.set_scope(Scope::ThisApp);
        assert_eq!(labels(&vm.section_lines(Section::RecentFolders)), ["a"]);
        vm.set_scope(Scope::AllApps);
        assert_eq!(vm.section_lines(Section::RecentFolders).len(), 2);
    }

    #[test]
    fn this_app_cannot_be_chosen_when_the_application_is_not_known() {
        let d = Dir::new("recents-unknown-app");
        let mut options = options(&d);
        options.app_key = String::new();
        let mut vm = ViewModel::new(&options);
        assert!(!vm.can_filter_by_app());
        vm.set_scope(Scope::ThisApp);
        assert_eq!(vm.scope(), Scope::AllApps);
    }

    #[test]
    fn recent_files_open_the_folder_they_are_in() {
        let d = Dir::new("recent-files");
        let mut vm = vm(&d);
        vm.set_recents(recents_with(&[], &["/srv/docs/report.txt"]));
        let lines = vm.section_lines(Section::RecentFiles);
        let Line::File {
            path,
            label,
            subtitle,
        } = &lines[0]
        else {
            panic!("expected a file line");
        };
        assert_eq!(label, "report.txt");
        assert_eq!(subtitle, "/srv/docs");
        assert_eq!(
            vm.go_to_file_folder(path),
            Some(Job::Go("/srv/docs".into()))
        );
    }

    #[test]
    fn a_long_recents_list_shows_fifteen_rows_then_all_on_request() {
        let d = Dir::new("recents-cap");
        let mut vm = vm(&d);
        let folders: Vec<(String, Vec<&str>)> =
            (0..40).map(|i| (format!("/srv/d{i}"), vec![])).collect();
        let refs: Vec<(&str, &[&str])> = folders
            .iter()
            .map(|(p, a)| (p.as_str(), a.as_slice()))
            .collect();
        vm.set_recents(recents_with(&refs, &[]));
        let lines = vm.section_lines(Section::RecentFolders);
        assert_eq!(lines.len(), LIST_CAP + 1);
        assert!(matches!(lines.last(), Some(Line::More { hidden: 25, .. })));
        vm.toggle_show_all(Section::RecentFolders);
        assert_eq!(vm.section_lines(Section::RecentFolders).len(), 41);
    }

    #[test]
    fn unchanged_recents_are_not_reported_as_a_change() {
        let d = Dir::new("recents-same");
        let mut vm = vm(&d);
        assert!(vm.set_recents(recents_with(&[("/srv/a", &[])], &[])));
        assert!(!vm.set_recents(recents_with(&[("/srv/a", &[])], &[])));
    }

    #[test]
    fn a_recents_warning_is_shown_in_the_strip() {
        let d = Dir::new("recents-warning");
        let mut vm = vm(&d);
        vm.set_recents(RecentsRead {
            recents: Recents::default(),
            warning: Some("recents were unreadable".into()),
        });
        assert_eq!(vm.message().unwrap().kind, MessageKind::Error);
    }

    #[test]
    fn settings_warnings_are_shown_at_startup() {
        let d = Dir::new("settings-warning");
        fs::write(d.0.join("settings.toml"), "recents_max = \"x\"\n").unwrap();
        let vm = vm(&d);
        assert!(text(&vm).contains("recents_max"));
    }

    #[test]
    fn opening_a_row_returns_it_for_reading_and_shows_the_answer_beneath() {
        let d = Dir::new("expand");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/a");
        vm.toggle_current_favorite();
        let key = RowKey::new(Section::Favorites, "/srv/a");
        assert_eq!(vm.toggle_expand(key.clone()), Some(key.clone()));
        assert!(matches!(
            vm.section_lines(Section::Favorites)[1],
            Line::Loading(1)
        ));
        vm.finish_browse(&key, Ok(vec!["/srv/a/x".into()]));
        assert_eq!(labels(&vm.section_lines(Section::Favorites)), ["a", "x"]);
        assert_eq!(vm.toggle_expand(key), None);
        assert_eq!(vm.section_lines(Section::Favorites).len(), 1);
    }

    #[test]
    fn a_subfolder_row_can_be_clicked_to_go_there() {
        let d = Dir::new("expand-go");
        let mut vm = vm(&d);
        assert_eq!(
            vm.row_action(RowAction::Go, Path::new("/srv/a/x")),
            Effect::Job(Job::Go("/srv/a/x".into()))
        );
    }

    #[test]
    fn the_menu_actions_copy_launch_and_toggle_the_favorite() {
        let d = Dir::new("menu-actions");
        let mut vm = vm(&d);
        let p = Path::new("/srv/a");
        assert_eq!(
            vm.row_action(RowAction::CopyPath, p),
            Effect::Copy("/srv/a".into())
        );
        assert_eq!(
            vm.row_action(RowAction::OpenFileManager, p),
            Effect::Job(Job::OpenFileManager("/srv/a".into()))
        );
        assert_eq!(
            vm.row_action(RowAction::OpenTerminal, p),
            Effect::Job(Job::OpenTerminal("/srv/a".into()))
        );
        vm.row_action(RowAction::ToggleFavorite, p);
        assert!(vm.favorites().contains(p));
        vm.row_action(RowAction::ToggleFavorite, p);
        assert!(!vm.favorites().contains(p));
    }

    #[test]
    fn a_menu_action_closes_the_menu() {
        let d = Dir::new("menu-closes");
        let mut vm = vm(&d);
        let key = RowKey::new(Section::Places, "/srv/a");
        vm.set_places(vec![Place {
            name: "a".into(),
            path: "/srv/a".into(),
            kind: PlaceKind::UserFolder,
        }]);
        vm.toggle_menu(key);
        assert!(
            vm.section_lines(Section::Places)
                .iter()
                .any(|l| matches!(l, Line::Menu { .. }))
        );
        vm.row_action(RowAction::CopyPath, Path::new("/srv/a"));
        assert!(
            !vm.section_lines(Section::Places)
                .iter()
                .any(|l| matches!(l, Line::Menu { .. }))
        );
    }

    #[test]
    fn opening_the_current_folder_in_a_program_is_refused_when_the_folder_is_unknown() {
        let d = Dir::new("launch-unknown");
        let mut vm = vm(&d);
        assert_eq!(vm.open_current(false), None);
        assert_eq!(vm.message().unwrap().kind, MessageKind::Error);
        at(&mut vm, "/srv/a");
        assert_eq!(
            vm.open_current(true),
            Some(Job::OpenTerminal("/srv/a".into()))
        );
    }

    #[test]
    fn a_failed_launch_shows_its_error_and_a_good_one_confirms() {
        let d = Dir::new("launch-result");
        let mut vm = vm(&d);
        vm.finish_launch("terminal", Err("no terminal found".into()));
        assert_eq!(text(&vm), "no terminal found");
        vm.finish_launch("terminal", Ok(()));
        assert_eq!(vm.message().unwrap().kind, MessageKind::Info);
    }

    #[test]
    fn typing_in_the_find_box_ranks_favorites_recents_and_places_together_without_repeats() {
        let d = Dir::new("find-rank");
        let mut vm = vm(&d);
        at(&mut vm, "/srv/tax-returns");
        vm.toggle_current_favorite();
        vm.set_recents(recents_with(
            &[("/srv/tax-returns", &[]), ("/srv/taxi", &[])],
            &[],
        ));
        vm.set_places(vec![Place {
            name: "Docs".into(),
            path: "/home/ann/Documents".into(),
            kind: PlaceKind::UserFolder,
        }]);
        vm.find_set_query("tax");
        let found: Vec<_> = vm.find_results().into_iter().map(|c| c.path).collect();
        assert_eq!(found.len(), 2);
        assert!(found.contains(&PathBuf::from("/srv/tax-returns")));
    }

    #[test]
    fn enter_in_the_find_box_goes_to_the_highlighted_result() {
        let d = Dir::new("find-enter");
        let mut vm = vm(&d);
        vm.set_recents(recents_with(
            &[("/srv/alpha", &[]), ("/srv/alps", &[])],
            &[],
        ));
        vm.find_set_query("alp");
        let ranked = vm.find_results();
        assert_eq!(ranked.len(), 2);
        assert_eq!(vm.find_enter(), Some(Job::Go(ranked[0].path.clone())));
        vm.finish_go(Ok(()));
        vm.find_move(1);
        assert_eq!(vm.find_enter(), Some(Job::Go(ranked[1].path.clone())));
    }

    #[test]
    fn enter_in_the_find_box_with_nothing_matching_does_nothing() {
        let d = Dir::new("find-nothing");
        let mut vm = vm(&d);
        vm.find_set_query("zzz");
        assert_eq!(vm.find_enter(), None);
    }

    #[test]
    fn a_search_is_refused_without_a_known_folder_and_says_why() {
        let d = Dir::new("search-unknown");
        let mut vm = vm(&d);
        vm.find_set_query("x");
        assert!(vm.start_search().is_none());
        assert_eq!(vm.message().unwrap().kind, MessageKind::Error);
    }

    #[test]
    fn search_results_are_listed_when_the_search_finishes_and_stop_ends_it_at_once() {
        let d = Dir::new("search-flow");
        let mut vm = vm(&d);
        at(&mut vm, "/srv");
        vm.find_set_query("x");
        let job = vm.start_search().unwrap();
        assert!(vm.search_running());
        vm.finish_search(job.id, vec!["/srv/x1".into()], Ended::Finished);
        assert_eq!(labels(&vm.section_lines(Section::Found)), ["x1"]);
        let job = vm.start_search().unwrap();
        vm.stop_search();
        assert!(!vm.search_running());
        vm.finish_search(job.id, vec!["/srv/late".into()], Ended::Finished);
        assert!(vm.section_lines(Section::Found).is_empty());
    }

    #[test]
    fn escape_in_the_find_box_stops_a_search_first_and_then_clears_the_box() {
        let d = Dir::new("find-escape");
        let mut vm = vm(&d);
        at(&mut vm, "/srv");
        vm.find_set_query("x");
        vm.start_search();
        vm.find_escape();
        assert!(!vm.search_running());
        assert_eq!(vm.find_query(), "x");
        vm.find_escape();
        assert_eq!(vm.find_query(), "");
    }

    #[test]
    fn a_found_folder_row_has_the_same_menu_as_any_other() {
        let d = Dir::new("found-menu");
        let mut vm = vm(&d);
        at(&mut vm, "/srv");
        vm.find_set_query("x");
        let job = vm.start_search().unwrap();
        vm.finish_search(job.id, vec!["/srv/x1".into()], Ended::Finished);
        vm.toggle_menu(RowKey::new(Section::Found, "/srv/x1"));
        assert!(
            vm.section_lines(Section::Found)
                .iter()
                .any(|l| matches!(l, Line::Menu { .. }))
        );
    }
}
