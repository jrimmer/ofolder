//! The item selected in a dialog's file list, for the preview panel.
//!
//! Each dialog says what is selected in its own way, and none of them says
//! it as a path: Flea gives the cursor row's name beside the folder, GTK and
//! Strata give a row's label. A name is only turned into a path when it is
//! one plain file name inside the folder the dialog shows and that path
//! exists; anything else is `None`.

use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::a11y::{Node, Role, walk};

/// `name` inside `folder`, when `name` is a single plain file name and the
/// result exists (a broken link counts: it is still the item selected).
pub fn name_in_folder(folder: &Path, name: &str) -> Option<PathBuf> {
    let mut parts = Path::new(name).components();
    let (Some(Component::Normal(_)), None) = (parts.next(), parts.next()) else {
        return None;
    };
    if !folder.is_absolute() {
        return None;
    }
    let path = folder.join(name);
    path.symlink_metadata().ok().map(|_| path)
}

/// What Flea's picker answers to its `snapshot` call: the folder, the
/// cursor row's name and the marked rows. A single mark wins over the cursor;
/// several marks are no single selection. Not checked against the disk.
pub fn flea_candidate(snapshot: &str) -> Option<PathBuf> {
    let value: Value = serde_json::from_str(snapshot).ok()?;
    let marks = value.get("marks")?.as_array()?;
    match marks.as_slice() {
        [] => {}
        [mark] => {
            let path = PathBuf::from(mark.get("path")?.as_str()?);
            return path.is_absolute().then_some(path);
        }
        _ => return None,
    }
    let folder = Path::new(value.get("path")?.as_str()?);
    let name = value.get("cursorName")?.as_str()?;
    // The Recent view's "folder" is not a folder, and its rows are not in one.
    if !folder.is_absolute() || name.is_empty() {
        return None;
    }
    let mut parts = Path::new(name).components();
    let (Some(Component::Normal(_)), None) = (parts.next(), parts.next()) else {
        return None;
    };
    Some(folder.join(name))
}

/// Asks a running Flea picker for its `snapshot`. The picker answers this
/// read-only call on its own IPC socket; nothing about the dialog changes.
pub trait PickerIpc: Send + Sync {
    fn snapshot(&self, pid: u32) -> Option<String>;
}

/// The picker's IPC through `qs ipc`, Quickshell's own client.
#[derive(Debug, Default, Clone, Copy)]
pub struct QsIpc;

/// Longer than a call has ever taken (about 18 ms), short enough that a
/// stuck picker cannot hold the poll up.
const IPC_LIMIT: Duration = Duration::from_millis(800);

impl PickerIpc for QsIpc {
    fn snapshot(&self, pid: u32) -> Option<String> {
        let mut child = Command::new("qs")
            .args([
                "ipc",
                "--pid",
                &pid.to_string(),
                "call",
                "fleapicker",
                "snapshot",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break,
                Ok(Some(_)) | Err(_) => return None,
                Ok(None) if start.elapsed() > IPC_LIMIT => {
                    child.kill().ok();
                    child.wait().ok();
                    return None;
                }
                Ok(None) => sleep(Duration::from_millis(5)),
            }
        }
        let output = child.wait_with_output().ok()?;
        String::from_utf8(output.stdout).ok()
    }
}

/// The one selected row's name in GTK's file table ("Files"): its rows are
/// runs of cells, one per column header, and the first cell of a row holds
/// the icon and then the name.
pub fn gtk_selected_name<N: Node>(chooser: &N) -> Option<String> {
    let mut table = None;
    walk(chooser, &mut |node| {
        if node.role() == Role::Table && node.name() == "Files" {
            table = Some(node.clone());
            return crate::a11y::Walk::Stop;
        }
        if node.role() == Role::List {
            crate::a11y::Walk::Over
        } else {
            crate::a11y::Walk::Into
        }
    });
    let children = table?.children();
    let columns = children
        .iter()
        .filter(|c| c.role() == Role::ColumnHeader)
        .count();
    if columns == 0 {
        return None;
    }
    let cells: Vec<&N> = children
        .iter()
        .filter(|c| c.role() == Role::TableCell)
        .collect();
    let mut names = cells
        .chunks(columns)
        .filter_map(|row| row.first())
        .filter(|first| first.selected())
        .map(|first| {
            first
                .children()
                .iter()
                .map(Node::name)
                .rfind(|n| !n.is_empty())
                .unwrap_or_default()
        });
    match (names.next(), names.next()) {
        (Some(name), None) if !name.is_empty() => Some(name),
        _ => None,
    }
}

/// The one selected item in Strata's column for `folder`: each column is a
/// list named after its folder, holding one item per file. Two columns with
/// that name (a folder inside one of the same name) are ambiguous.
pub fn strata_selected_name<N: Node>(root: &N, folder: &Path) -> Option<String> {
    let wanted = folder.file_name()?.to_str()?.to_owned();
    let mut lists = Vec::new();
    walk(root, &mut |node| {
        if node.role() == Role::List {
            if node.name() == wanted {
                lists.push(node.clone());
            }
            return crate::a11y::Walk::Over;
        }
        crate::a11y::Walk::Into
    });
    let [list] = lists.as_slice() else {
        return None;
    };
    let mut names = list
        .children()
        .into_iter()
        .filter(|item| item.role() == Role::ListItem && item.selected())
        .map(|item| item.name());
    match (names.next(), names.next()) {
        (Some(name), None) if !name.is_empty() => Some(name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a11y::fake::FakeNode;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ofolder-sel-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_plain_name_that_exists_in_the_folder_becomes_its_path() {
        let dir = scratch("plain");
        std::fs::write(dir.join("alpha.txt"), "x").unwrap();
        assert_eq!(
            name_in_folder(&dir, "alpha.txt"),
            Some(dir.join("alpha.txt"))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_name_that_is_not_there_or_not_a_plain_name_is_none() {
        let dir = scratch("odd");
        std::fs::write(dir.join("a"), "x").unwrap();
        assert_eq!(name_in_folder(&dir, "missing.txt"), None);
        for odd in ["", ".", "..", "a/b", "/etc/passwd", "../a"] {
            assert_eq!(name_in_folder(&dir, odd), None, "{odd:?}");
        }
        assert_eq!(name_in_folder(Path::new("relative"), "a"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flea_reads_the_cursor_row_inside_the_folder_when_nothing_is_marked() {
        let snap = r#"{"path":"/tmp/pick","cursor":1,"cursorName":"aether.png","marks":[]}"#;
        assert_eq!(
            flea_candidate(snap),
            Some(PathBuf::from("/tmp/pick/aether.png"))
        );
    }

    #[test]
    fn flea_prefers_a_single_mark_and_refuses_several() {
        let one = r#"{"path":"/tmp/pick","cursorName":"a","marks":[{"path":"/tmp/pick/b","uri":"file:///tmp/pick/b"}]}"#;
        assert_eq!(flea_candidate(one), Some(PathBuf::from("/tmp/pick/b")));
        let two = r#"{"path":"/tmp/pick","cursorName":"a","marks":[{"path":"/tmp/pick/b"},{"path":"/tmp/pick/c"}]}"#;
        assert_eq!(flea_candidate(two), None);
    }

    #[test]
    fn flea_gives_nothing_for_the_recent_view_an_empty_row_or_a_bad_reply() {
        for snap in [
            r#"{"path":"recent:///","cursorName":"home/me/a.txt","marks":[]}"#,
            r#"{"path":"/tmp/pick","cursorName":"","marks":[]}"#,
            r#"{"path":"/tmp/pick","cursorName":"../x","marks":[]}"#,
            r#"{"path":"/tmp/pick","cursorName":"a"}"#,
            "not json",
            "",
        ] {
            assert_eq!(flea_candidate(snap), None, "{snap}");
        }
    }

    /// The shape dumped from the real GTK dialog: four column headers, then
    /// four cells per row; the first cell holds the icon and the name.
    fn gtk_table(selected: &[usize]) -> FakeNode {
        let chooser = FakeNode::root(Role::FileChooser, "Open");
        let places = chooser.add(Role::List, "");
        places.add(Role::TableCell, "not-a-file").selected_on();
        let table = chooser.add(Role::Table, "Files");
        for h in ["Name", "Size", "Type", "Modified"] {
            table.add(Role::ColumnHeader, h);
        }
        for (i, name) in ["gamma.md", "pinentry.png", "omarchy.png"]
            .iter()
            .enumerate()
        {
            let pick = |n: FakeNode| {
                if selected.contains(&i) {
                    n.selected_on()
                } else {
                    n
                }
            };
            let first = pick(table.add(Role::TableCell, ""));
            first.add(Role::TableCell, "");
            first.add(Role::TableCell, name);
            pick(table.add(Role::TableCell, "1 byte"));
            pick(table.add(Role::TableCell, "Document"));
            let modified = pick(table.add(Role::TableCell, ""));
            modified.add(Role::TableCell, "00:29");
            modified.add(Role::TableCell, "00:29");
        }
        chooser
    }

    #[test]
    fn gtk_reads_the_name_cell_of_the_one_selected_row() {
        assert_eq!(
            gtk_selected_name(&gtk_table(&[1])),
            Some("pinentry.png".to_owned())
        );
    }

    #[test]
    fn gtk_gives_nothing_when_no_row_or_several_rows_are_selected() {
        assert_eq!(gtk_selected_name(&gtk_table(&[])), None);
        assert_eq!(gtk_selected_name(&gtk_table(&[0, 2])), None);
    }

    fn strata_columns(folder_lists: &[&str], selected: &[&str]) -> FakeNode {
        let root = FakeNode::root(Role::Other, "pick — Strata");
        for list_name in folder_lists {
            let list = root.add(Role::Other, "column").add(Role::List, list_name);
            for item in ["sub-folder", "aether.png", "alpha.txt"] {
                let node = list.add(Role::ListItem, item);
                node.add(Role::Label, item);
                if selected.contains(&item) {
                    node.selected_on();
                }
            }
        }
        root
    }

    #[test]
    fn strata_reads_the_selected_item_of_the_column_for_the_folder_shown() {
        // The parent's column has its own selection; only the folder's column counts.
        let root = strata_columns(&["tmp", "pick"], &["alpha.txt"]);
        assert_eq!(
            strata_selected_name(&root, Path::new("/tmp/pick")),
            Some("alpha.txt".to_owned())
        );
    }

    #[test]
    fn strata_gives_nothing_for_two_columns_of_the_same_name_or_no_selection() {
        let root = strata_columns(&["pick", "pick"], &["alpha.txt"]);
        assert_eq!(
            strata_selected_name(&root, Path::new("/tmp/pick/pick")),
            None
        );
        let root = strata_columns(&["pick"], &[]);
        assert_eq!(strata_selected_name(&root, Path::new("/tmp/pick")), None);
        let root = strata_columns(&["pick"], &["alpha.txt", "aether.png"]);
        assert_eq!(strata_selected_name(&root, Path::new("/tmp/pick")), None);
    }
}
