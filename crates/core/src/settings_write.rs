//! Writing the settings file from the settings window.
//!
//! The file is also edited by hand, so a write changes only the keys it was
//! asked to and keeps everything else: comments, order, blank lines and keys
//! this version does not know. It reads the file again just before writing,
//! so a hand edit made while the window was open is not lost, refuses to
//! touch a file it cannot parse, and replaces the file in one durable step so
//! a crash never leaves half of it.

use std::fs;
use std::path::Path;

use toml_edit::{Array, DocumentMut, Item, Table, Value, value};

use crate::preview::PreviewSettings;
use crate::private_file;
use crate::rails::{RailLayout, Side};
use crate::shortcuts::{COMMANDS, Shortcuts};

/// One change to the settings file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Bool(&'static str, bool),
    Int(&'static str, i64),
    /// A command; `None` removes the key, so the default is detected again.
    Command(&'static str, Option<String>),
    /// The rail layout; the default layout removes the lists, so a later
    /// version's default applies.
    Rails(RailLayout),
    /// Every shortcut: a command without one has its key removed. Keys
    /// under `[shortcuts]` this version does not know are left alone.
    Shortcuts(Shortcuts),
    /// The preview panel's settings; the defaults remove the table.
    Preview(PreviewSettings),
    /// The applications left alone; an empty list removes the key, which is
    /// the default.
    ExcludedApps(Vec<String>),
    /// The folders never recorded as recent, as written in the file. `None`
    /// is the default list and removes the key, so a later version's default
    /// applies; `Some(vec![])` is a deliberate empty list.
    RecentsExcluded(Option<Vec<String>>),
    /// The folder a dialog starts in for one application; `None` removes
    /// that entry, and the table goes when it is empty. Other entries,
    /// including ones this version cannot read, are left alone.
    StartFolder(String, Option<String>),
}

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("the settings file could not be read: {0}")]
    Read(std::io::Error),
    #[error("the settings file has a mistake in it, so it was left alone: {0}")]
    Parse(String),
    #[error("the settings file could not be written: {0}")]
    Write(std::io::Error),
}

/// Sets `key` in `table`, keeping the comments around an existing value.
fn set(table: &mut Table, key: &str, new: Value) {
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(old) => {
            let decor = old.decor().clone();
            *old = new;
            *old.decor_mut() = decor;
        }
        None => {
            table.insert(key, value(new));
        }
    }
}

fn set_rails(doc: &mut DocumentMut, layout: &RailLayout) {
    if layout.is_default() {
        if let Some(rails) = doc.get_mut("rails").and_then(Item::as_table_mut) {
            rails.remove(Side::Left.key());
            rails.remove(Side::Right.key());
            if rails.is_empty() {
                doc.remove("rails");
            }
        }
        return;
    }
    if !doc.get("rails").is_some_and(Item::is_table) {
        doc.insert("rails", Item::Table(Table::new()));
    }
    let Some(rails) = doc.get_mut("rails").and_then(Item::as_table_mut) else {
        return;
    };
    for side in [Side::Left, Side::Right] {
        let list: Array = layout.ids(side).into_iter().collect();
        set(rails, side.key(), Value::Array(list));
    }
}

fn set_shortcuts(doc: &mut DocumentMut, shortcuts: &Shortcuts) {
    if !doc.get("shortcuts").is_some_and(Item::is_table) {
        if shortcuts.is_empty() {
            return;
        }
        doc.insert("shortcuts", Item::Table(Table::new()));
    }
    let Some(table) = doc.get_mut("shortcuts").and_then(Item::as_table_mut) else {
        return;
    };
    for def in COMMANDS {
        match shortcuts.get(def.id) {
            Some(combo) => set(table, def.id, Value::from(combo.to_setting())),
            None => {
                table.remove(def.id);
            }
        }
    }
    if table.is_empty() {
        doc.remove("shortcuts");
    }
}

fn set_preview(doc: &mut DocumentMut, preview: &PreviewSettings) {
    if *preview == PreviewSettings::default() {
        if let Some(table) = doc.get_mut("preview").and_then(Item::as_table_mut) {
            for key in ["enabled", "video_thumbnails", "max_mb"] {
                table.remove(key);
            }
            if table.is_empty() {
                doc.remove("preview");
            }
        }
        return;
    }
    if !doc.get("preview").is_some_and(Item::is_table) {
        doc.insert("preview", Item::Table(Table::new()));
    }
    let Some(table) = doc.get_mut("preview").and_then(Item::as_table_mut) else {
        return;
    };
    set(table, "enabled", Value::from(preview.enabled));
    set(
        table,
        "video_thumbnails",
        Value::from(preview.video_thumbnails),
    );
    set(
        table,
        "max_mb",
        Value::from(i64::try_from(preview.max_mb).unwrap_or(i64::MAX)),
    );
}

fn set_string_list(doc: &mut DocumentMut, key: &str, items: Option<&[String]>) {
    match items {
        Some(items) => {
            let list: Array = items.iter().map(String::as_str).collect();
            set(doc.as_table_mut(), key, Value::Array(list));
        }
        None => {
            doc.remove(key);
        }
    }
}

fn set_start_folder(doc: &mut DocumentMut, app: &str, folder: Option<&str>) {
    let Some(folder) = folder else {
        if let Some(table) = doc.get_mut("start_folders").and_then(Item::as_table_mut) {
            table.remove(app);
            if table.is_empty() {
                doc.remove("start_folders");
            }
        }
        return;
    };
    if !doc.get("start_folders").is_some_and(Item::is_table) {
        doc.insert("start_folders", Item::Table(Table::new()));
    }
    if let Some(table) = doc.get_mut("start_folders").and_then(Item::as_table_mut) {
        set(table, app, Value::from(folder));
    }
}

/// `text` with `edits` applied.
pub fn apply(text: &str, edits: &[Edit]) -> Result<String, WriteError> {
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
        WriteError::Parse(e.to_string().lines().next().unwrap_or("").to_owned())
    })?;
    for edit in edits {
        match edit {
            Edit::Bool(key, on) => set(doc.as_table_mut(), key, Value::from(*on)),
            Edit::Int(key, n) => set(doc.as_table_mut(), key, Value::from(*n)),
            Edit::Command(key, Some(command)) => {
                set(doc.as_table_mut(), key, Value::from(command.as_str()))
            }
            Edit::Command(key, None) => {
                doc.remove(key);
            }
            Edit::Rails(layout) => set_rails(&mut doc, layout),
            Edit::Shortcuts(shortcuts) => set_shortcuts(&mut doc, shortcuts),
            Edit::Preview(preview) => set_preview(&mut doc, preview),
            Edit::ExcludedApps(apps) => {
                set_string_list(
                    &mut doc,
                    "excluded_apps",
                    Some(apps.as_slice()).filter(|a| !a.is_empty()),
                );
            }
            Edit::RecentsExcluded(folders) => {
                set_string_list(&mut doc, "recents_excluded", folders.as_deref());
            }
            Edit::StartFolder(app, folder) => set_start_folder(&mut doc, app, folder.as_deref()),
        }
    }
    Ok(doc.to_string())
}

/// Applies `edits` to the settings file at `path`, creating it when missing.
/// The read and the write are held under the file's lock, so a write from
/// another window cannot be built on the same old text and lost.
pub fn save(path: &Path, edits: &[Edit]) -> Result<(), WriteError> {
    private_file::with_lock(path, || -> Result<(), WriteError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(WriteError::Read(e)),
        };
        let updated = apply(&text, edits)?;
        if updated == text && path.exists() {
            return Ok(());
        }
        private_file::write_private_atomic(path, updated.as_bytes()).map_err(WriteError::Write)
    })
    .map_err(WriteError::Write)?
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::settings::parse;

    const HAND: &str = "# my settings\npaused = false # keep it on\n\n\
                        future_option = \"x\"\n\n[start_folders]\n\"class:zen\" = \"~/Downloads\"\n";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ofolder-settings-write-{}-{name}",
            std::process::id()
        ));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_change_keeps_comments_order_and_keys_this_version_does_not_know() {
        let out = apply(
            HAND,
            &[
                Edit::Bool("paused", true),
                Edit::Int("recents_max", 40),
                Edit::Command("terminal", Some("foot".to_owned())),
            ],
        )
        .unwrap();
        assert!(
            out.starts_with("# my settings\npaused = true # keep it on\n"),
            "{out}"
        );
        assert!(out.contains("future_option = \"x\""));
        assert!(out.contains("\"class:zen\" = \"~/Downloads\""));
        let read = parse(&out, Path::new("/home/j"));
        assert!(read.settings.paused);
        assert_eq!(read.settings.recents_max, 40);
        assert_eq!(read.settings.terminal.as_deref(), Some("foot"));
        assert_eq!(read.settings.start_folders.len(), 1);
    }

    #[test]
    fn a_blank_command_removes_the_key_so_it_is_detected_again() {
        let out = apply("terminal = \"foot\"\n", &[Edit::Command("terminal", None)]).unwrap();
        assert_eq!(parse(&out, Path::new("/h")).settings.terminal, None);
    }

    #[test]
    fn a_rail_layout_round_trips_and_the_default_removes_the_lists() {
        let mut layout = RailLayout::default();
        layout.hide(Side::Left, 0);
        layout.add_divider_after(Side::Right, 0);
        let text = "# rails below\n[rails]\n# mine\nleft = []\nkept = 1\n";
        let out = apply(text, &[Edit::Rails(layout.clone())]).unwrap();
        assert!(
            out.contains("# rails below") && out.contains("# mine") && out.contains("kept = 1")
        );
        let read = parse(&out, Path::new("/h"));
        assert_eq!(read.settings.rails, layout);
        let back = apply(&out, &[Edit::Rails(RailLayout::default())]).unwrap();
        assert!(
            !back.contains("left =") && back.contains("kept = 1"),
            "{back}"
        );
        let bare = apply(
            "[rails]\nleft = [\"back\"]\n",
            &[Edit::Rails(RailLayout::default())],
        )
        .unwrap();
        assert!(!bare.contains("rails"), "{bare}");
    }

    #[test]
    fn shortcuts_round_trip_keep_unknown_keys_and_an_empty_set_removes_the_table() {
        use crate::shortcuts::Combo;
        let mut keys = Shortcuts::default();
        keys.set("places", Some(Combo::parse("SUPER+ALT+P").unwrap()));
        keys.set("find", Some(Combo::parse("super+alt+f9").unwrap()));
        let text = "# keys\n[shortcuts]\n# mine\nback = \"SUPER+ALT+B\"\nfuture = \"x\"\n";
        let out = apply(text, &[Edit::Shortcuts(keys.clone())]).unwrap();
        assert!(
            out.contains("# keys") && out.contains("future = \"x\""),
            "{out}"
        );
        assert!(out.contains("places = \"SUPER+ALT+P\""), "{out}");
        assert!(out.contains("find = \"SUPER+ALT+F9\""), "{out}");
        assert!(
            !out.contains("back ="),
            "a cleared command loses its key: {out}"
        );
        assert_eq!(parse(&out, Path::new("/h")).settings.shortcuts, keys);
        let cleared = apply(&out, &[Edit::Shortcuts(Shortcuts::default())]).unwrap();
        assert!(cleared.contains("future = \"x\""), "{cleared}");
        let bare = apply(
            "[shortcuts]\nback = \"SUPER+ALT+B\"\n",
            &[Edit::Shortcuts(Shortcuts::default())],
        )
        .unwrap();
        assert!(!bare.contains("shortcuts"), "{bare}");
        assert_eq!(
            apply("", &[Edit::Shortcuts(Shortcuts::default())]).unwrap(),
            ""
        );
    }

    #[test]
    fn preview_settings_round_trip_keeping_comments_and_the_defaults_remove_the_table() {
        let text = "# mine\npaused = false\n\n[preview]\n# keep this\nenabled = true\nshiny = 1\n";
        let off = PreviewSettings {
            enabled: false,
            video_thumbnails: false,
            max_mb: 16,
        };
        let written = apply(text, &[Edit::Preview(off)]).unwrap();
        assert!(
            written.contains("# keep this") && written.contains("shiny = 1"),
            "{written}"
        );
        let read = crate::settings::parse(&written, std::path::Path::new("/home/j"));
        assert_eq!(read.settings.preview, off);
        let back = apply(&written, &[Edit::Preview(PreviewSettings::default())]).unwrap();
        assert!(
            back.contains("shiny = 1") && !back.contains("max_mb"),
            "{back}"
        );
        assert_eq!(
            apply("", &[Edit::Preview(PreviewSettings::default())]).unwrap(),
            ""
        );
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone() {
        let dir = scratch("broken");
        let path = dir.join("settings.toml");
        fs::write(&path, "paused = = true\n").unwrap();
        let err = save(&path, &[Edit::Bool("paused", true)]).unwrap_err();
        assert!(matches!(err, WriteError::Parse(_)), "{err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "paused = = true\n");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saving_creates_a_missing_file_rereads_a_hand_edit_and_leaves_no_temp_file() {
        let dir = scratch("save");
        let path = dir.join("sub/settings.toml");
        save(&path, &[Edit::Bool("recents_enabled", false)]).unwrap();
        // A hand edit made between two saves survives the second one.
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("# added by hand\n");
        fs::write(&path, &text).unwrap();
        save(&path, &[Edit::Int("recents_max", 7)]).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("# added by hand") && after.contains("recents_max = 7"));
        let mut names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["settings.toml", "settings.toml.lock"],
            "no temporary file"
        );
        fs::remove_dir_all(dir).ok();
    }

    const LISTS: &str = "# mine\nexcluded_apps = [\"class:old\"] # why\nfuture = 1\n\n\
                         [start_folders]\n# zen\n\"class:zen\" = \"~/Downloads\"\nbroken = \"nope\"\n";

    #[test]
    fn excluded_apps_round_trip_keep_comments_and_an_empty_list_removes_the_key() {
        let apps = vec!["class:kiosk".to_owned(), "org.example.App".to_owned()];
        let out = apply(LISTS, &[Edit::ExcludedApps(apps.clone())]).unwrap();
        assert!(
            out.contains("# mine") && out.contains("future = 1"),
            "{out}"
        );
        assert!(
            out.contains("# why"),
            "the comment after the value stays: {out}"
        );
        assert_eq!(parse(&out, Path::new("/h")).settings.excluded_apps, apps);
        let none = apply(&out, &[Edit::ExcludedApps(Vec::new())]).unwrap();
        assert!(
            !none.contains("excluded_apps") && none.contains("future = 1"),
            "{none}"
        );
        assert_eq!(apply("", &[Edit::ExcludedApps(Vec::new())]).unwrap(), "");
        let fresh = apply("", &[Edit::ExcludedApps(apps.clone())]).unwrap();
        assert_eq!(parse(&fresh, Path::new("/h")).settings.excluded_apps, apps);
    }

    #[test]
    fn recents_excluded_round_trips_and_the_default_removes_the_key() {
        let home = Path::new("/home/j");
        let text = "# mine\nrecents_excluded = [\"~/.ssh\"]\nfuture = 1\n";
        let list = vec!["~/Private".to_owned(), "/mnt/secret".to_owned()];
        let out = apply(text, &[Edit::RecentsExcluded(Some(list))]).unwrap();
        assert!(
            out.contains("# mine") && out.contains("future = 1"),
            "{out}"
        );
        assert_eq!(
            parse(&out, home).settings.recents_excluded,
            [
                PathBuf::from("/home/j/Private"),
                PathBuf::from("/mnt/secret")
            ]
        );
        let empty = apply(&out, &[Edit::RecentsExcluded(Some(Vec::new()))]).unwrap();
        assert!(
            parse(&empty, home).settings.recents_excluded.is_empty(),
            "{empty}"
        );
        let back = apply(&empty, &[Edit::RecentsExcluded(None)]).unwrap();
        assert!(
            !back.contains("recents_excluded") && back.contains("future = 1"),
            "{back}"
        );
        assert_eq!(
            parse(&back, home).settings.recents_excluded,
            crate::settings::default_recents_excluded(home)
        );
    }

    #[test]
    fn a_start_folder_is_set_and_removed_one_entry_at_a_time() {
        let home = Path::new("/home/j");
        let out = apply(
            LISTS,
            &[
                Edit::StartFolder("class:kiosk".to_owned(), Some("/srv/kiosk".to_owned())),
                Edit::StartFolder("class:zen".to_owned(), Some("~/Pictures".to_owned())),
            ],
        )
        .unwrap();
        assert!(
            out.contains("# zen") && out.contains("broken = \"nope\""),
            "{out}"
        );
        let read = parse(&out, home).settings.start_folders;
        assert_eq!(read["class:kiosk"], PathBuf::from("/srv/kiosk"));
        assert_eq!(read["class:zen"], PathBuf::from("/home/j/Pictures"));
        let less = apply(&out, &[Edit::StartFolder("class:kiosk".to_owned(), None)]).unwrap();
        assert!(!less.contains("kiosk") && less.contains("broken"), "{less}");
        // The last entry takes the table with it; a missing one is not an error.
        let gone = apply(
            "[start_folders]\n\"a\" = \"/x\"\n",
            &[
                Edit::StartFolder("a".to_owned(), None),
                Edit::StartFolder("b".to_owned(), None),
            ],
        )
        .unwrap();
        assert_eq!(gone, "");
        assert_eq!(
            apply("", &[Edit::StartFolder("a".to_owned(), None)]).unwrap(),
            ""
        );
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone_by_the_list_edits_too() {
        let dir = scratch("broken-lists");
        let path = dir.join("settings.toml");
        for edit in [
            Edit::ExcludedApps(vec!["a".to_owned()]),
            Edit::RecentsExcluded(None),
            Edit::StartFolder("a".to_owned(), Some("/x".to_owned())),
        ] {
            fs::write(&path, "excluded_apps = [\n").unwrap();
            let err = save(&path, &[edit]).unwrap_err();
            assert!(matches!(err, WriteError::Parse(_)), "{err}");
            assert_eq!(fs::read_to_string(&path).unwrap(), "excluded_apps = [\n");
        }
        fs::remove_dir_all(dir).ok();
    }
}
