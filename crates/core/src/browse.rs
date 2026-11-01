//! Listing the folders inside a folder, for browsing down a tree.

use std::fs;
use std::path::{Path, PathBuf};

/// The subfolders of `dir`, sorted without regard to case. Hidden folders are
/// left out unless asked for. A folder that cannot be read is an error to
/// show, never a panic or a hang of its own: callers read on a background
/// thread, since a slow mount can take a long time to answer.
pub fn subfolders(dir: &Path, show_hidden: bool) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| show_hidden || !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    out.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    fn temp(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ofolder-browse-{label}-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn subfolders_are_listed_sorted_ignoring_case_and_files_are_left_out() {
        let d = temp("sorted");
        for n in ["beta", "Alpha", "gamma"] {
            fs::create_dir(d.join(n)).expect("dir");
        }
        fs::write(d.join("a-file.txt"), "x").expect("file");
        let names: Vec<_> = subfolders(&d, false)
            .expect("ok")
            .iter()
            .map(|p| p.file_name().expect("name").to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Alpha", "beta", "gamma"]);
        fs::remove_dir_all(&d).expect("cleanup");
    }

    #[test]
    fn hidden_folders_show_only_when_asked_for() {
        let d = temp("hidden");
        fs::create_dir(d.join(".secret")).expect("dir");
        fs::create_dir(d.join("open")).expect("dir");
        assert_eq!(subfolders(&d, false).expect("ok").len(), 1);
        assert_eq!(subfolders(&d, true).expect("ok").len(), 2);
        fs::remove_dir_all(&d).expect("cleanup");
    }

    #[test]
    fn a_symlink_to_a_folder_is_not_listed_as_a_folder_so_loops_cannot_form() {
        let d = temp("link");
        fs::create_dir(d.join("real")).expect("dir");
        symlink(&d, d.join("loop")).expect("link");
        let names: Vec<_> = subfolders(&d, false)
            .expect("ok")
            .iter()
            .map(|p| p.file_name().expect("name").to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["real"]);
        fs::remove_dir_all(&d).expect("cleanup");
    }

    #[test]
    fn an_unreadable_folder_is_an_error_message_and_does_not_stop_anything_else() {
        let d = temp("unreadable");
        let locked = d.join("locked");
        fs::create_dir(&locked).expect("dir");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
        let result = subfolders(&locked, false);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore");
        assert!(result.is_err());
        assert!(subfolders(&d, false).is_ok());
        fs::remove_dir_all(&d).expect("cleanup");
    }

    #[test]
    fn a_folder_that_does_not_exist_is_an_error() {
        assert!(subfolders(Path::new("/definitely/not/here"), false).is_err());
    }
}
