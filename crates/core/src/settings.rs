//! The user's settings file.
//!
//! A settings file is edited by hand, so a mistake in it must never stop a
//! dialog from working. Every setting is read on its own: a value of the wrong
//! type, or out of range, is reported and ignored while the rest still apply,
//! and a file that cannot be parsed at all gives the defaults and one warning.
//! The file is read again for each dialog, so a change applies to the next one.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::preview::PreviewSettings;
use crate::rails::{RailLayout, Side};
use crate::shortcuts::Shortcuts;

pub const MAX_RECENTS_LIMIT: usize = 500;
/// The longest age, in days, recents are kept for: a century.
pub const MAX_RECENTS_DAYS: u32 = 36_500;
/// Recents kept for this many days when the file does not say otherwise.
pub const DEFAULT_RECENTS_DAYS: u32 = 90;

/// Folders under the home folder that hold keys, passwords, wallets, browser
/// profiles and deleted files. They are never recorded as recent unless the
/// settings file names its own list in `recents_excluded`.
const SENSITIVE_UNDER_HOME: [&str; 12] = [
    ".ssh",
    ".gnupg",
    ".password-store",
    ".local/share/keyrings",
    ".config/Bitwarden",
    ".config/1Password",
    ".config/KeePassXC",
    ".mozilla",
    ".config/google-chrome",
    ".config/chromium",
    ".config/BraveSoftware",
    ".local/share/Trash",
];

/// The folders excluded from recents when the settings file does not name
/// its own: see [`SENSITIVE_UNDER_HOME`].
pub fn default_recents_excluded(home: &Path) -> Vec<PathBuf> {
    SENSITIVE_UNDER_HOME
        .iter()
        .map(|rel| home.join(rel))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The command that opens a folder in a file manager. `None` means detect it.
    pub file_manager: Option<String>,
    /// The command that opens a terminal in a folder. `None` means detect it.
    pub terminal: Option<String>,
    /// Whether the strip stays open after "open in file manager".
    pub keep_dialog_after_file_manager: bool,
    pub recents_enabled: bool,
    /// Whether the guard is installed: it puts ofolder back in the file-chooser
    /// chain when another program removes it, and keeps dialogs working if the
    /// ofolder program is deleted. On unless the maintainer turns it off.
    pub guard: bool,
    pub recents_max: usize,
    /// How many days a recent folder or file is kept after its last use;
    /// zero keeps them for ever.
    pub recents_days: u32,
    /// Folders, and everything below them, that are never recorded as recent.
    /// When the settings file sets `recents_excluded`, its list replaces the
    /// defaults: an empty list excludes nothing, and a list none of whose
    /// entries is a usable folder keeps the defaults, so a typo cannot open a
    /// sensitive folder to recording. [`Settings::default`] has none, since it
    /// does not know the home folder; [`parse`] and [`load`] fill in the
    /// defaults for the home they are given.
    pub recents_excluded: Vec<PathBuf>,
    /// Applications by id for which the strip is not shown.
    pub excluded_apps: Vec<String>,
    /// The folder a dialog starts in, by application id, when the app gave none.
    pub start_folders: BTreeMap<String, PathBuf>,
    /// While set, nothing is shown, recorded or moved.
    pub paused: bool,
    /// Put the places panel on the right and the controls on the left.
    pub swap_sides: bool,
    /// Which icons each rail shows, in what order, with its divider lines.
    pub rails: RailLayout,
    /// Keyboard shortcuts for the rail icons; none until the user sets one.
    pub shortcuts: Shortcuts,
    /// The preview panel under the dialog.
    pub preview: PreviewSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            file_manager: None,
            terminal: None,
            keep_dialog_after_file_manager: true,
            recents_enabled: true,
            guard: true,
            recents_max: 20,
            recents_days: DEFAULT_RECENTS_DAYS,
            recents_excluded: Vec::new(),
            excluded_apps: Vec::new(),
            start_folders: BTreeMap::new(),
            paused: false,
            swap_sides: false,
            rails: RailLayout::default(),
            shortcuts: Shortcuts::default(),
            preview: PreviewSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub settings: Settings,
    /// One line for everything that was ignored, in the order found.
    pub warnings: Vec<String>,
}

const KNOWN: [&str; 15] = [
    "file_manager",
    "terminal",
    "keep_dialog_after_file_manager",
    "recents_enabled",
    "guard",
    "recents_max",
    "recents_days",
    "recents_excluded",
    "excluded_apps",
    "start_folders",
    "paused",
    "swap_sides",
    "rails",
    "shortcuts",
    "preview",
];

/// `~` and `~/...` mean the home folder. Anything else must already be absolute.
fn folder(text: &str, home: &Path) -> Option<PathBuf> {
    let path = if text == "~" {
        home.to_path_buf()
    } else if let Some(rest) = text.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(text)
    };
    path.is_absolute().then_some(path)
}

struct Reader<'a> {
    table: &'a Table,
    warnings: Vec<String>,
}

impl Reader<'_> {
    fn warn(&mut self, key: &str, why: &str) {
        self.warnings
            .push(format!("setting `{key}` ignored: {why}"));
    }

    fn boolean(&mut self, key: &str, into: &mut bool) {
        match self.table.get(key) {
            None => {}
            Some(Value::Boolean(b)) => *into = *b,
            Some(_) => self.warn(key, "expected true or false"),
        }
    }

    fn command(&mut self, key: &str, into: &mut Option<String>) {
        match self.table.get(key) {
            None => {}
            Some(Value::String(s)) if !s.trim().is_empty() => *into = Some(s.trim().to_owned()),
            Some(Value::String(_)) => self.warn(key, "it is empty"),
            Some(_) => self.warn(key, "expected a command as text"),
        }
    }

    fn strings(&mut self, key: &str) -> Option<Vec<String>> {
        match self.table.get(key)? {
            Value::Array(items) => {
                let texts: Option<Vec<String>> = items
                    .iter()
                    .map(|v| v.as_str().map(str::to_owned))
                    .collect();
                if texts.is_none() {
                    self.warn(key, "every entry must be text");
                }
                texts
            }
            _ => {
                self.warn(key, "expected a list of text");
                None
            }
        }
    }
}

/// The settings with every default that depends on the home folder filled in.
fn defaults_for(home: &Path) -> Settings {
    Settings {
        recents_excluded: default_recents_excluded(home),
        ..Settings::default()
    }
}

pub fn parse(text: &str, home: &Path) -> Loaded {
    let mut out = defaults_for(home);
    let table: Table = match text.parse() {
        Ok(t) => t,
        Err(error) => {
            let first_line = error.to_string().lines().next().unwrap_or("").to_owned();
            return Loaded {
                settings: out,
                warnings: vec![format!(
                    "settings file could not be read, defaults used: {first_line}"
                )],
            };
        }
    };
    let mut r = Reader {
        table: &table,
        warnings: Vec::new(),
    };

    r.command("file_manager", &mut out.file_manager);
    r.command("terminal", &mut out.terminal);
    r.boolean(
        "keep_dialog_after_file_manager",
        &mut out.keep_dialog_after_file_manager,
    );
    r.boolean("recents_enabled", &mut out.recents_enabled);
    r.boolean("guard", &mut out.guard);
    r.boolean("paused", &mut out.paused);
    r.boolean("swap_sides", &mut out.swap_sides);

    match table.get("recents_max") {
        None => {}
        Some(Value::Integer(n)) if (1..=MAX_RECENTS_LIMIT as i64).contains(n) => {
            out.recents_max = usize::try_from(*n).unwrap_or(out.recents_max);
        }
        Some(Value::Integer(_)) => r.warn(
            "recents_max",
            &format!("it must be between 1 and {MAX_RECENTS_LIMIT}"),
        ),
        Some(_) => r.warn("recents_max", "expected a whole number"),
    }

    match table.get("recents_days") {
        None => {}
        Some(Value::Integer(n)) if (0..=i64::from(MAX_RECENTS_DAYS)).contains(n) => {
            out.recents_days = u32::try_from(*n).unwrap_or(out.recents_days);
        }
        Some(Value::Integer(_)) => r.warn(
            "recents_days",
            &format!("it must be between 0 and {MAX_RECENTS_DAYS} (0 keeps them for ever)"),
        ),
        Some(_) => r.warn("recents_days", "expected a whole number of days"),
    }

    if let Some(items) = r.strings("recents_excluded") {
        let mut listed = Vec::new();
        for item in &items {
            match folder(item, home) {
                Some(path) => listed.push(path),
                None => r.warn(
                    "recents_excluded",
                    &format!("{item:?} is not an absolute folder"),
                ),
            }
        }
        if listed.is_empty() && !items.is_empty() {
            r.warn(
                "recents_excluded",
                "no entry is a usable folder, so the default exclusions stay",
            );
        } else {
            out.recents_excluded = listed;
        }
    }
    if let Some(items) = r.strings("excluded_apps") {
        out.excluded_apps = items.into_iter().filter(|a| !a.trim().is_empty()).collect();
    }

    match table.get("start_folders") {
        None => {}
        Some(Value::Table(map)) => {
            for (app, value) in map {
                match value.as_str().and_then(|s| folder(s, home)) {
                    Some(path) => {
                        out.start_folders.insert(app.clone(), path);
                    }
                    None => r.warn(
                        "start_folders",
                        &format!("the folder for {app:?} is not an absolute folder"),
                    ),
                }
            }
        }
        Some(_) => r.warn(
            "start_folders",
            "expected a table of application id to folder",
        ),
    }

    match table.get("rails") {
        None => {}
        Some(Value::Table(rails)) => {
            let mut list = |side: Side| -> Option<Vec<String>> {
                let key = format!("rails.{}", side.key());
                match rails.get(side.key())? {
                    Value::Array(items) => {
                        let texts: Option<Vec<String>> = items
                            .iter()
                            .map(|v| v.as_str().map(str::to_owned))
                            .collect();
                        if texts.is_none() {
                            r.warn(&key, "every entry must be text");
                        }
                        texts
                    }
                    _ => {
                        r.warn(&key, "expected a list of icon names");
                        None
                    }
                }
            };
            let left = list(Side::Left);
            let right = list(Side::Right);
            for key in rails
                .keys()
                .filter(|k| *k != Side::Left.key() && *k != Side::Right.key())
            {
                r.warnings.push(format!(
                    "setting `rails.{key}` is not known and was ignored"
                ));
            }
            out.rails = RailLayout::from_lists(left.as_deref(), right.as_deref(), &mut r.warnings);
        }
        Some(_) => r.warn("rails", "expected a table with `left` and `right` lists"),
    }

    match table.get("shortcuts") {
        None => {}
        Some(Value::Table(map)) => {
            out.shortcuts = Shortcuts::from_entries(
                map.iter().map(|(id, v)| (id.as_str(), v.as_str())),
                &mut r.warnings,
            );
        }
        Some(_) => r.warn("shortcuts", "expected a table of command to keys"),
    }

    match table.get("preview") {
        None => {}
        Some(Value::Table(preview)) => {
            out.preview = PreviewSettings::from_table(preview, &mut r.warnings);
        }
        Some(_) => r.warn("preview", "expected a table"),
    }

    for key in table.keys().filter(|k| !KNOWN.contains(&k.as_str())) {
        r.warnings
            .push(format!("setting `{key}` is not known and was ignored"));
    }
    Loaded {
        settings: out,
        warnings: r.warnings,
    }
}

/// Reads the settings file. A missing file is normal and gives the defaults
/// with no warning; one that cannot be read gives the defaults and a warning.
pub fn load(path: &Path, home: &Path) -> Loaded {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text, home),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Loaded {
            settings: defaults_for(home),
            warnings: Vec::new(),
        },
        Err(e) => Loaded {
            settings: defaults_for(home),
            warnings: vec![format!(
                "settings file could not be read, defaults used: {e}"
            )],
        },
    }
}

/// Where the settings file lives: an override, else the user's config folder.
pub fn settings_path(
    override_path: Option<PathBuf>,
    xdg_config: Option<PathBuf>,
    home: &Path,
) -> PathBuf {
    override_path.unwrap_or_else(|| {
        xdg_config
            .unwrap_or_else(|| home.join(".config"))
            .join("ofolder/settings.toml")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/j";

    fn parse_ok(text: &str) -> Loaded {
        parse(text, Path::new(HOME))
    }

    /// What no settings file gives for this home folder.
    fn defaults() -> Settings {
        Settings {
            recents_excluded: default_recents_excluded(Path::new(HOME)),
            ..Settings::default()
        }
    }

    #[test]
    fn the_guard_is_on_unless_the_file_turns_it_off() {
        let none = parse_ok("");
        assert!(none.settings.guard);
        let off = parse_ok("guard = false\n");
        assert!(!off.settings.guard);
        assert!(off.warnings.is_empty());
        let bad = parse_ok("guard = \"no\"\n");
        assert!(
            bad.settings.guard,
            "a bad value is ignored, so the default stands"
        );
        assert_eq!(bad.warnings.len(), 1);
    }

    #[test]
    fn an_empty_file_gives_the_defaults_and_no_warnings() {
        let l = parse_ok("");
        assert_eq!(l.settings, defaults());
        assert!(l.warnings.is_empty());
    }

    #[test]
    fn every_setting_can_be_given() {
        let l = parse_ok(
            r#"
            file_manager = "flea"
            terminal = "ghostty --working-directory"
            keep_dialog_after_file_manager = false
            recents_enabled = false
            recents_max = 50
            recents_excluded = ["~/Private", "/mnt/secret"]
            excluded_apps = ["org.example.Kiosk"]
            paused = true
            swap_sides = true
            [start_folders]
            "org.example.Editor" = "~/Work"
            "#,
        );
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
        let s = l.settings;
        assert_eq!(s.file_manager.as_deref(), Some("flea"));
        assert_eq!(s.terminal.as_deref(), Some("ghostty --working-directory"));
        assert!(
            !s.keep_dialog_after_file_manager && !s.recents_enabled && s.paused && s.swap_sides
        );
        assert_eq!(s.recents_max, 50);
        assert_eq!(
            s.recents_excluded,
            [
                PathBuf::from("/home/j/Private"),
                PathBuf::from("/mnt/secret")
            ]
        );
        assert_eq!(s.excluded_apps, ["org.example.Kiosk"]);
        assert_eq!(
            s.start_folders["org.example.Editor"],
            PathBuf::from("/home/j/Work")
        );
    }

    #[test]
    fn a_value_of_the_wrong_type_is_reported_and_the_rest_still_apply() {
        let l = parse_ok(
            "recents_max = \"lots\"\nswap_sides = true\npaused = 3\nrecents_enabled = false",
        );
        assert!(l.settings.swap_sides && !l.settings.recents_enabled);
        assert_eq!(l.settings.recents_max, 20, "the default stays");
        assert!(!l.settings.paused);
        assert_eq!(l.warnings.len(), 2, "{:?}", l.warnings);
        assert!(l.warnings.iter().any(|w| w.contains("recents_max")));
        assert!(l.warnings.iter().any(|w| w.contains("paused")));
    }

    #[test]
    fn a_recents_limit_out_of_range_is_ignored() {
        for text in [
            "recents_max = 0",
            "recents_max = -4",
            "recents_max = 100000",
        ] {
            let l = parse_ok(text);
            assert_eq!(l.settings.recents_max, 20, "{text}");
            assert_eq!(l.warnings.len(), 1, "{text}");
        }
        assert_eq!(parse_ok("recents_max = 500").settings.recents_max, 500);
        assert_eq!(parse_ok("recents_max = 1").settings.recents_max, 1);
    }

    #[test]
    fn a_relative_folder_is_refused_and_a_good_one_beside_it_is_kept() {
        let l = parse_ok("recents_excluded = [\"relative/dir\", \"/ok\"]");
        assert_eq!(l.settings.recents_excluded, [PathBuf::from("/ok")]);
        assert_eq!(l.warnings.len(), 1);
    }

    #[test]
    fn a_start_folder_that_is_not_absolute_is_ignored_on_its_own() {
        let l = parse_ok("[start_folders]\na = \"nope\"\nb = \"/ok\"\nc = 4");
        assert_eq!(l.settings.start_folders.len(), 1);
        assert_eq!(l.settings.start_folders["b"], PathBuf::from("/ok"));
        assert_eq!(l.warnings.len(), 2);
    }

    #[test]
    fn an_unknown_setting_is_reported_not_silently_dropped() {
        let l = parse_ok("colour = \"blue\"");
        assert_eq!(l.settings, defaults());
        assert_eq!(l.warnings.len(), 1);
        assert!(l.warnings[0].contains("colour"));
    }

    #[test]
    fn a_file_that_does_not_parse_never_stops_the_dialogs() {
        let l = parse_ok("this is = = not toml [");
        assert_eq!(l.settings, defaults());
        assert_eq!(l.warnings.len(), 1);
        assert!(l.warnings[0].contains("defaults used"));
    }

    #[test]
    fn an_empty_command_is_ignored() {
        let l = parse_ok("terminal = \"  \"");
        assert_eq!(l.settings.terminal, None);
        assert_eq!(l.warnings.len(), 1);
    }

    #[test]
    fn a_missing_file_is_normal_and_a_directory_in_its_place_is_a_warning() {
        let dir = std::env::temp_dir().join(format!("ofolder-settings-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let missing = load(&dir.join("settings.toml"), Path::new(HOME));
        assert!(missing.warnings.is_empty());
        assert_eq!(missing.settings, defaults());
        let unreadable = load(&dir, Path::new(HOME));
        assert_eq!(unreadable.warnings.len(), 1);
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_change_to_the_file_applies_the_next_time_it_is_read() {
        let dir =
            std::env::temp_dir().join(format!("ofolder-settings-reload-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("settings.toml");
        fs::write(&file, "paused = false").expect("write");
        assert!(!load(&file, Path::new(HOME)).settings.paused);
        fs::write(&file, "paused = true").expect("write");
        assert!(load(&file, Path::new(HOME)).settings.paused);
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn the_sensitive_folders_under_home_are_excluded_from_recents_when_nothing_is_set() {
        let l = parse_ok("");
        for rel in [
            ".ssh",
            ".gnupg",
            ".password-store",
            ".config/Bitwarden",
            ".mozilla",
        ] {
            let folder = PathBuf::from(HOME).join(rel);
            assert!(l.settings.recents_excluded.contains(&folder), "{rel}");
        }
        assert_eq!(l.settings.recents_excluded.len(), 12);
    }

    #[test]
    fn a_list_in_the_file_replaces_the_default_exclusions() {
        let l = parse_ok("recents_excluded = [\"/private\"]");
        assert_eq!(l.settings.recents_excluded, [PathBuf::from("/private")]);
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    }

    #[test]
    fn an_empty_list_in_the_file_excludes_nothing() {
        let l = parse_ok("recents_excluded = []");
        assert!(l.settings.recents_excluded.is_empty());
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    }

    #[test]
    fn a_list_of_no_usable_folder_keeps_the_defaults_and_says_so() {
        let l = parse_ok("recents_excluded = [\"relative\", \"~nobody\"]");
        assert_eq!(l.settings.recents_excluded, defaults().recents_excluded);
        assert_eq!(
            l.warnings.len(),
            3,
            "two refused entries and the fallback: {:?}",
            l.warnings
        );
    }

    #[test]
    fn a_tilde_in_an_exclusion_is_expanded_against_the_given_home() {
        let l = parse(
            "recents_excluded = [\"~/Secret\", \"~\"]",
            Path::new("/srv/home/ann"),
        );
        assert_eq!(
            l.settings.recents_excluded,
            [
                PathBuf::from("/srv/home/ann/Secret"),
                PathBuf::from("/srv/home/ann")
            ]
        );
    }

    #[test]
    fn recents_are_kept_ninety_days_unless_the_file_says_otherwise() {
        assert_eq!(parse_ok("").settings.recents_days, 90);
        assert_eq!(parse_ok("recents_days = 0").settings.recents_days, 0);
        assert_eq!(parse_ok("recents_days = 365").settings.recents_days, 365);
        let bad = parse_ok("recents_days = -1");
        assert_eq!(bad.settings.recents_days, 90);
        assert_eq!(bad.warnings.len(), 1);
        let text = parse_ok("recents_days = \"week\"");
        assert_eq!(text.settings.recents_days, 90);
        assert_eq!(text.warnings.len(), 1);
    }

    #[test]
    fn the_settings_file_lives_in_the_config_folder_unless_overridden() {
        let home = Path::new(HOME);
        assert_eq!(
            settings_path(None, None, home),
            PathBuf::from("/home/j/.config/ofolder/settings.toml")
        );
        assert_eq!(
            settings_path(None, Some(PathBuf::from("/xdg")), home),
            PathBuf::from("/xdg/ofolder/settings.toml")
        );
        assert_eq!(
            settings_path(Some(PathBuf::from("/x.toml")), None, home),
            PathBuf::from("/x.toml")
        );
    }

    #[test]
    fn the_rail_layout_is_read_from_the_rails_table() {
        let loaded = parse_ok("[rails]\nleft = [\"places\", \"-\", \"back\"]\n");
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(
            loaded.settings.rails.ids(Side::Left),
            ["places", "-", "back"]
        );
        // The right rail was not given, so it keeps its default.
        assert_eq!(
            loaded.settings.rails.items(Side::Right),
            RailLayout::default().items(Side::Right)
        );
    }

    #[test]
    fn a_rails_setting_of_the_wrong_shape_is_reported_and_the_default_kept() {
        let loaded = parse_ok("rails = 3\n");
        assert!(loaded.settings.rails.is_default());
        assert!(loaded.warnings[0].contains("`rails` ignored"));
        let loaded = parse_ok("[rails]\nleft = \"back\"\nright = [1]\nmiddle = []\n");
        assert!(loaded.settings.rails.is_default());
        assert_eq!(loaded.warnings.len(), 3, "{:?}", loaded.warnings);
        assert!(loaded.warnings.iter().any(|w| w.contains("rails.middle")));
    }

    #[test]
    fn shortcuts_are_read_from_their_table_and_mistakes_are_reported() {
        let text = "[shortcuts]\nplaces = \"SUPER+ALT+P\"\nfind = 3\nnope = \"SUPER+ALT+N\"\n";
        let loaded = parse(text, Path::new("/h"));
        let places = loaded
            .settings
            .shortcuts
            .get("places")
            .map(|c| c.to_setting());
        assert_eq!(places.as_deref(), Some("SUPER+ALT+P"));
        assert_eq!(loaded.settings.shortcuts.get("find"), None);
        assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
        let wrong = parse("shortcuts = 1\n", Path::new("/h"));
        assert!(wrong.settings.shortcuts.is_empty());
        assert_eq!(wrong.warnings.len(), 1);
    }
}
