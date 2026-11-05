//! Turning what each dialog exposes into a folder.
//!
//! These are the pure halves of the adapters' `read_folder`: the backend
//! specific code fetches raw material (accessibility nodes, file descriptors)
//! and these functions decide what folder, if any, that material proves.
//! Each returns `None` rather than a best guess.

use std::path::{Path, PathBuf};

/// One toggle button of the GTK dialog's path bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathBarButton<'a> {
    pub name: &'a str,
    pub checked: bool,
}

const GTK_ROOT_BUTTON: &str = "File System Root";

/// The GTK dialog's path bar lists buttons deepest first and ends at the
/// filesystem root. The checked one is the folder being shown; any buttons
/// before it are folders below it that were visited, not the current one.
pub fn gtk_folder(buttons: &[PathBarButton<'_>]) -> Option<PathBuf> {
    let mut checked = buttons.iter().enumerate().filter(|(_, b)| b.checked);
    let (at, _) = checked.next()?;
    if checked.next().is_some() {
        return None;
    }
    let root = buttons[at..]
        .iter()
        .position(|b| b.name == GTK_ROOT_BUTTON)?
        + at;
    let mut path = PathBuf::from("/");
    for button in buttons[at..root].iter().rev() {
        path.push(button.name);
    }
    Some(path)
}

/// Strata's header is a breadcrumb: the labels `~`, `/`, `Work` read together
/// as `~/Work`. A leading `~` is the home folder.
pub fn strata_folder(labels: &[&str], home: &Path) -> Option<PathBuf> {
    let text: String = labels.concat();
    if let Some(rest) = text.strip_prefix('~') {
        return match rest {
            "" => Some(home.to_path_buf()),
            _ if rest.starts_with('/') => Some(home.join(rest.trim_start_matches('/'))),
            // `~someone` is another user's home, which the header never shows.
            _ => None,
        };
    }
    text.starts_with('/').then(|| PathBuf::from(text))
}

/// Flea's `--backend` helper holds a directory open on the folder shown. Its
/// own state folders are not the folder shown, so they are set aside; the
/// reading counts only when exactly one candidate is left.
pub fn flea_folder(open_dirs: &[PathBuf], ignore_under: &[PathBuf]) -> Option<PathBuf> {
    let mut left = open_dirs
        .iter()
        .filter(|dir| !ignore_under.iter().any(|skip| dir.starts_with(skip)));
    let first = left.next()?;
    left.all(|other| other == first).then(|| first.clone())
}

/// A reading is trusted once two in a row agree. A dialog mid-move shows
/// nothing or something stale, and a single reading cannot tell the two apart.
pub fn first_agreeing(readings: impl IntoIterator<Item = Option<PathBuf>>) -> Option<PathBuf> {
    let mut previous: Option<PathBuf> = None;
    for reading in readings {
        if reading.is_some() && reading == previous {
            return reading;
        }
        previous = reading;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar<'a>(spec: &[(&'a str, bool)]) -> Vec<PathBarButton<'a>> {
        spec.iter()
            .map(|&(name, checked)| PathBarButton { name, checked })
            .collect()
    }

    #[test]
    fn the_gtk_path_bar_at_home_reads_as_the_home_folder() {
        let buttons = bar(&[("alex", true), ("home", false), ("File System Root", false)]);
        assert_eq!(gtk_folder(&buttons), Some(PathBuf::from("/home/alex")));
    }

    #[test]
    fn gtk_buttons_for_visited_subfolders_before_the_checked_one_are_ignored() {
        let buttons = bar(&[
            ("Work", false),
            ("alex", true),
            ("home", false),
            ("File System Root", false),
        ]);
        assert_eq!(gtk_folder(&buttons), Some(PathBuf::from("/home/alex")));
    }

    #[test]
    fn gtk_at_the_filesystem_root_is_the_root() {
        let buttons = bar(&[("File System Root", true)]);
        assert_eq!(gtk_folder(&buttons), Some(PathBuf::from("/")));
    }

    #[test]
    fn gtk_with_no_checked_button_is_unknown() {
        let buttons = bar(&[("alex", false), ("File System Root", false)]);
        assert_eq!(gtk_folder(&buttons), None);
    }

    #[test]
    fn gtk_with_two_checked_buttons_is_unknown() {
        let buttons = bar(&[("a", true), ("b", true), ("File System Root", false)]);
        assert_eq!(gtk_folder(&buttons), None);
    }

    #[test]
    fn gtk_whose_path_does_not_reach_the_root_is_unknown() {
        // A volume mounted somewhere else shows a different first button.
        let buttons = bar(&[("Documents", true), ("mybox", false)]);
        assert_eq!(gtk_folder(&buttons), None);
    }

    #[test]
    fn strata_tilde_alone_is_home() {
        assert_eq!(
            strata_folder(&["~"], Path::new("/home/j")),
            Some(PathBuf::from("/home/j"))
        );
    }

    #[test]
    fn strata_breadcrumb_pieces_join_into_a_path_under_home() {
        let labels = ["~", "/", "Work", "/", "ofolder"];
        assert_eq!(
            strata_folder(&labels, Path::new("/home/j")),
            Some(PathBuf::from("/home/j/Work/ofolder"))
        );
    }

    #[test]
    fn strata_breadcrumb_outside_home_is_absolute() {
        let labels = ["/", "usr", "/", "bin"];
        assert_eq!(
            strata_folder(&labels, Path::new("/home/j")),
            Some(PathBuf::from("/usr/bin"))
        );
    }

    #[test]
    fn strata_another_users_home_is_unknown() {
        assert_eq!(strata_folder(&["~root"], Path::new("/home/j")), None);
    }

    #[test]
    fn strata_nothing_recognisable_is_unknown() {
        assert_eq!(strata_folder(&["Recent"], Path::new("/home/j")), None);
        assert_eq!(strata_folder(&[], Path::new("/home/j")), None);
    }

    #[test]
    fn flea_one_open_folder_is_the_folder_shown() {
        let open = [PathBuf::from("/home/j/Work")];
        assert_eq!(flea_folder(&open, &[]), Some(PathBuf::from("/home/j/Work")));
    }

    #[test]
    fn flea_state_folders_are_set_aside() {
        let open = [
            PathBuf::from("/home/j/.local/state/flea"),
            PathBuf::from("/home/j/Work"),
        ];
        let skip = [PathBuf::from("/home/j/.local/state")];
        assert_eq!(
            flea_folder(&open, &skip),
            Some(PathBuf::from("/home/j/Work"))
        );
    }

    #[test]
    fn flea_two_different_open_folders_are_ambiguous() {
        let open = [PathBuf::from("/a"), PathBuf::from("/b")];
        assert_eq!(flea_folder(&open, &[]), None);
    }

    #[test]
    fn flea_the_same_folder_open_twice_is_still_one_folder() {
        let open = [PathBuf::from("/a"), PathBuf::from("/a")];
        assert_eq!(flea_folder(&open, &[]), Some(PathBuf::from("/a")));
    }

    #[test]
    fn flea_nothing_open_is_unknown() {
        assert_eq!(flea_folder(&[], &[]), None);
    }

    #[test]
    fn two_agreeing_readings_in_a_row_are_trusted() {
        let a = Some(PathBuf::from("/a"));
        let b = Some(PathBuf::from("/b"));
        assert_eq!(
            first_agreeing([a.clone(), b.clone(), b.clone()]),
            b,
            "the first pair that agrees wins"
        );
    }

    #[test]
    fn readings_that_keep_changing_are_never_trusted() {
        let r = |p: &str| Some(PathBuf::from(p));
        assert_eq!(first_agreeing([r("/a"), r("/b"), r("/a"), r("/b")]), None);
    }

    #[test]
    fn unknown_readings_never_agree_with_each_other() {
        assert_eq!(first_agreeing([None, None, None]), None);
    }
}
