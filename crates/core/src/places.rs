//! Standard locations, mounted drives, and the history of folders visited.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
    pub kind: PlaceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceKind {
    Home,
    UserFolder,
    Drive,
    /// The folder a file-manager window is showing now.
    OpenWindow,
}

/// The standard folders in the user's directory configuration
/// (`user-dirs.dirs`), after the home folder. An entry the user has switched
/// off (set to the home folder itself) or one that is not an absolute path
/// is left out.
pub fn user_dirs(config: &str, home: &Path) -> Vec<Place> {
    const NAMES: [(&str, &str); 8] = [
        ("XDG_DESKTOP_DIR", "Desktop"),
        ("XDG_DOWNLOAD_DIR", "Downloads"),
        ("XDG_DOCUMENTS_DIR", "Documents"),
        ("XDG_MUSIC_DIR", "Music"),
        ("XDG_PICTURES_DIR", "Pictures"),
        ("XDG_VIDEOS_DIR", "Videos"),
        ("XDG_TEMPLATES_DIR", "Templates"),
        ("XDG_PUBLICSHARE_DIR", "Public"),
    ];
    let mut out = vec![Place {
        name: "Home".to_owned(),
        path: home.to_path_buf(),
        kind: PlaceKind::Home,
    }];
    for (key, name) in NAMES {
        let Some(value) = config
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#'))
            .find_map(|l| {
                l.strip_prefix(key)?
                    .trim_start()
                    .strip_prefix('=')
                    .map(str::trim)
            })
        else {
            continue;
        };
        let value = value.trim_matches('"');
        let expanded = if let Some(rest) = value.strip_prefix("$HOME") {
            home.join(rest.trim_start_matches('/'))
        } else {
            PathBuf::from(value)
        };
        if expanded.is_absolute() && expanded != home {
            out.push(Place {
                name: name.to_owned(),
                path: expanded,
                kind: PlaceKind::UserFolder,
            });
        }
    }
    out
}

/// `\040` style escapes in a mount point, which is how the kernel writes a space.
fn unescape_mount(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && let Ok(v) = u8::from_str_radix(&text[i + 1..i + 4], 8)
        {
            out.push(v);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Drives a person plugged in or mounted: mounts under the usual removable
/// locations, on a real filesystem. `mountinfo` is `/proc/self/mountinfo`.
pub fn mounted_drives(mountinfo: &str, user: &str) -> Vec<Place> {
    const PSEUDO: [&str; 12] = [
        "proc",
        "sysfs",
        "tmpfs",
        "devtmpfs",
        "devpts",
        "cgroup",
        "cgroup2",
        "overlay",
        "squashfs",
        "fuse.portal",
        "autofs",
        "securityfs",
    ];
    let roots = [
        format!("/run/media/{user}/"),
        "/media/".to_owned(),
        "/mnt/".to_owned(),
    ];
    let mut out: Vec<Place> = Vec::new();
    for line in mountinfo.lines() {
        let (before, after) = match line.split_once(" - ") {
            Some(parts) => parts,
            None => continue,
        };
        let mount_point = match before.split_whitespace().nth(4) {
            Some(m) => unescape_mount(m),
            None => continue,
        };
        let fstype = after.split_whitespace().next().unwrap_or("");
        if PSEUDO.contains(&fstype) || !roots.iter().any(|r| mount_point.starts_with(r.as_str())) {
            continue;
        }
        let path = PathBuf::from(&mount_point);
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if !out.iter().any(|p| p.path == path) {
            out.push(Place {
                name,
                path,
                kind: PlaceKind::Drive,
            });
        }
    }
    out
}

const HISTORY_LIMIT: usize = 100;

/// Folders the dialog has shown, with a position, so the strip can step back
/// and forward and go up a level.
#[derive(Debug, Default)]
pub struct History {
    entries: Vec<PathBuf>,
    index: usize,
    /// A step back or forward the strip asked for and has not yet seen land.
    pending: Option<Step>,
}

/// A step in flight. Until the dialog shows `target`, it still shows `from`,
/// and reading `from` again is not a new visit.
#[derive(Debug, Clone)]
struct Step {
    target: PathBuf,
    from: PathBuf,
    from_index: usize,
}

impl History {
    /// Notes the folder the dialog shows now. Seeing the folder already
    /// current, or the one a step back or forward asked for, adds nothing.
    /// A new folder after stepping back drops the forward entries.
    pub fn observe(&mut self, folder: &Path) {
        if let Some(step) = &self.pending {
            if step.target == folder {
                self.pending = None;
                return;
            }
            // The dialog has not moved yet: it still shows the folder being left.
            if step.from == folder {
                return;
            }
        }
        if self.entries.get(self.index).map(PathBuf::as_path) == Some(folder) {
            return;
        }
        self.pending = None;
        if !self.entries.is_empty() {
            self.entries.truncate(self.index + 1);
        }
        self.entries.push(folder.to_path_buf());
        if self.entries.len() > HISTORY_LIMIT {
            self.entries.remove(0);
        }
        self.index = self.entries.len() - 1;
    }

    pub fn can_go_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.index + 1 < self.entries.len()
    }

    /// The folder to go back to, and the position moves there. If the move
    /// then cannot be made, [`History::step_failed`] puts the position back.
    pub fn back(&mut self) -> Option<PathBuf> {
        if !self.can_go_back() {
            return None;
        }
        self.begin_step(self.index - 1)
    }

    pub fn forward(&mut self) -> Option<PathBuf> {
        if !self.can_go_forward() {
            return None;
        }
        self.begin_step(self.index + 1)
    }

    fn begin_step(&mut self, to: usize) -> Option<PathBuf> {
        let step = Step {
            target: self.entries.get(to)?.clone(),
            from: self.entries.get(self.index)?.clone(),
            from_index: self.index,
        };
        self.index = to;
        let target = step.target.clone();
        self.pending = Some(step);
        Some(target)
    }

    /// The step asked for did not happen: the dialog is still where it was,
    /// so the position goes back to it.
    pub fn step_failed(&mut self) {
        if let Some(step) = self.pending.take() {
            self.index = step.from_index;
        }
    }
}

/// The folder above `folder`, or `None` at the top.
pub fn parent_of(folder: &Path) -> Option<PathBuf> {
    folder
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/j";

    fn visited(folders: &[&str]) -> History {
        let mut h = History::default();
        for f in folders {
            h.observe(Path::new(f));
        }
        h
    }

    #[test]
    fn the_folder_being_left_read_again_during_a_step_does_not_rewrite_the_history() {
        let mut h = visited(&["/a", "/b", "/c"]);
        assert_eq!(h.back(), Some(PathBuf::from("/b")));
        // The dialog has not moved yet, so a poll still sees /c.
        h.observe(Path::new("/c"));
        h.observe(Path::new("/c"));
        h.observe(Path::new("/b"));
        assert_eq!(h.back(), Some(PathBuf::from("/a")));
        h.observe(Path::new("/b"));
        h.observe(Path::new("/a"));
        assert!(!h.can_go_back());
        assert_eq!(h.forward(), Some(PathBuf::from("/b")));
    }

    #[test]
    fn a_step_that_could_not_be_made_puts_the_position_back() {
        let mut h = visited(&["/a", "/b", "/c"]);
        assert_eq!(h.back(), Some(PathBuf::from("/b")));
        h.step_failed();
        assert!(!h.can_go_forward());
        assert_eq!(h.back(), Some(PathBuf::from("/b")));
        h.step_failed();
        h.step_failed();
        assert_eq!(h.back(), Some(PathBuf::from("/b")));
    }

    #[test]
    fn arriving_somewhere_other_than_the_step_target_is_a_new_visit() {
        let mut h = visited(&["/a", "/b", "/c"]);
        h.back();
        h.observe(Path::new("/elsewhere"));
        assert!(!h.can_go_forward());
        assert_eq!(h.back(), Some(PathBuf::from("/b")));
    }

    #[test]
    fn the_standard_folders_come_after_home_with_home_expanded() {
        let cfg = "# comment\nXDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\nXDG_DOCUMENTS_DIR=\"$HOME/Docs\"\nXDG_MUSIC_DIR=\"/data/music\"\n";
        let places = user_dirs(cfg, Path::new(HOME));
        let names: Vec<_> = places.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Home", "Downloads", "Documents", "Music"]);
        assert_eq!(places[2].path, PathBuf::from("/home/j/Docs"));
        assert_eq!(places[3].path, PathBuf::from("/data/music"));
        assert_eq!(places[0].kind, PlaceKind::Home);
    }

    #[test]
    fn a_folder_the_user_switched_off_or_that_is_not_absolute_is_left_out() {
        let cfg = "XDG_DESKTOP_DIR=\"$HOME/\"\nXDG_VIDEOS_DIR=\"relative/Videos\"\nXDG_PICTURES_DIR=\"$HOME/Pictures\"\n";
        let places = user_dirs(cfg, Path::new(HOME));
        let names: Vec<_> = places.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Home", "Pictures"]);
    }

    #[test]
    fn with_no_config_there_is_still_home() {
        assert_eq!(user_dirs("", Path::new(HOME)).len(), 1);
    }

    const MOUNTS: &str = "\
24 1 259:2 / / rw,relatime shared:1 - btrfs /dev/mapper/root rw
30 24 0:25 / /run rw,nosuid shared:5 - tmpfs tmpfs rw
85 24 8:17 / /run/media/j/My\\040Stick rw,relatime shared:60 - vfat /dev/sdb1 rw
86 24 8:33 / /mnt/backup rw,relatime shared:61 - ext4 /dev/sdc1 rw
87 24 0:40 / /run/media/j/ram rw - tmpfs tmpfs rw
88 24 0:41 / /run/media/other/theirs rw - ext4 /dev/sdd1 rw
";

    #[test]
    fn plugged_in_drives_are_listed_with_escaped_spaces_decoded() {
        let drives = mounted_drives(MOUNTS, "j");
        let names: Vec<_> = drives.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["My Stick", "backup"]);
        assert_eq!(drives[0].path, PathBuf::from("/run/media/j/My Stick"));
        assert!(drives.iter().all(|d| d.kind == PlaceKind::Drive));
    }

    #[test]
    fn another_users_drive_and_pseudo_filesystems_are_not_listed() {
        let paths: Vec<_> = mounted_drives(MOUNTS, "j")
            .into_iter()
            .map(|d| d.path)
            .collect();
        assert!(!paths.contains(&PathBuf::from("/run/media/other/theirs")));
        assert!(!paths.contains(&PathBuf::from("/run/media/j/ram")));
        assert!(!paths.contains(&PathBuf::from("/")));
    }

    #[test]
    fn a_drive_that_is_unplugged_is_gone_the_next_time_the_mounts_are_read() {
        assert_eq!(mounted_drives(MOUNTS, "j").len(), 2);
        assert_eq!(mounted_drives("", "j").len(), 0);
    }

    fn visit(h: &mut History, paths: &[&str]) {
        for p in paths {
            h.observe(Path::new(p));
        }
    }

    #[test]
    fn after_visiting_a_b_c_back_goes_to_b_then_a_and_forward_returns_to_b_then_c() {
        let mut h = History::default();
        visit(&mut h, &["/a", "/b", "/c"]);
        assert_eq!(h.back(), Some(PathBuf::from("/b")));
        h.observe(Path::new("/b"));
        assert_eq!(h.back(), Some(PathBuf::from("/a")));
        h.observe(Path::new("/a"));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), Some(PathBuf::from("/b")));
        h.observe(Path::new("/b"));
        assert_eq!(h.forward(), Some(PathBuf::from("/c")));
        h.observe(Path::new("/c"));
        assert_eq!(h.forward(), None);
    }

    #[test]
    fn stepping_back_does_not_record_the_arrival_as_a_new_visit() {
        let mut h = History::default();
        visit(&mut h, &["/a", "/b"]);
        h.back();
        h.observe(Path::new("/a"));
        assert!(h.can_go_forward(), "the forward entry survives the arrival");
    }

    #[test]
    fn going_somewhere_new_after_stepping_back_drops_the_forward_entries() {
        let mut h = History::default();
        visit(&mut h, &["/a", "/b", "/c"]);
        h.back();
        h.observe(Path::new("/b"));
        h.observe(Path::new("/z"));
        assert!(!h.can_go_forward());
        assert_eq!(h.back(), Some(PathBuf::from("/b")));
    }

    #[test]
    fn seeing_the_same_folder_again_adds_nothing() {
        let mut h = History::default();
        visit(&mut h, &["/a", "/a", "/a"]);
        assert!(!h.can_go_back());
    }

    #[test]
    fn the_history_is_bounded() {
        let mut h = History::default();
        for i in 0..250 {
            h.observe(Path::new(&format!("/d{i}")));
        }
        let mut steps = 0;
        while h.back().is_some() {
            steps += 1;
        }
        assert_eq!(steps, HISTORY_LIMIT - 1);
    }

    #[test]
    fn the_parent_of_a_nested_folder_is_the_one_above_and_the_root_has_none() {
        assert_eq!(parent_of(Path::new("/a/b")), Some(PathBuf::from("/a")));
        assert_eq!(parent_of(Path::new("/a")), Some(PathBuf::from("/")));
        assert_eq!(parent_of(Path::new("/")), None);
    }
}
