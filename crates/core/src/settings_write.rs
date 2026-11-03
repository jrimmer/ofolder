//! Writing the settings file from the settings window.
//!
//! The file is also edited by hand, so a write changes only the keys it was
//! asked to and keeps everything else: comments, order, blank lines and keys
//! this version does not know. It reads the file again just before writing,
//! so a hand edit made while the window was open is not lost, refuses to
//! touch a file it cannot parse, and replaces the file in one step so a
//! crash never leaves half of it.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, Item, Table, Value, value};

use crate::rails::{RailLayout, Side};

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
        }
    }
    Ok(doc.to_string())
}

/// Applies `edits` to the settings file at `path`, creating it when missing.
pub fn save(path: &Path, edits: &[Edit]) -> Result<(), WriteError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(WriteError::Read(e)),
    };
    let updated = apply(&text, edits)?;
    if updated == text && path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(WriteError::Write)?;
    }
    let temp = temp_path(path);
    let written = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(updated.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if written.is_err() {
        fs::remove_file(&temp).ok();
    }
    written.map_err(WriteError::Write)
}

/// A file beside `path`, so the rename stays on one filesystem.
fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings.toml".to_owned());
    path.with_file_name(format!(".{name}.{}.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
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
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["settings.toml"]);
        fs::remove_dir_all(dir).ok();
    }
}
