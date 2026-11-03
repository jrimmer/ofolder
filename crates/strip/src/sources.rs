//! Reading the files the left panel lists from: the user's folders, the
//! mounted drives, the recents and the settings. Each read is a plain
//! function of file paths so it can run on a background thread.

use std::fs;
use std::path::{Path, PathBuf};

use ofolder_core::places::{self, Place};
use ofolder_core::recents::{self, Recents};
use ofolder_core::settings;

#[derive(Clone, Debug)]
pub struct Sources {
    pub user_dirs_path: PathBuf,
    pub mountinfo_path: PathBuf,
    pub recents_path: PathBuf,
    pub home: PathBuf,
    pub user: String,
}

pub struct RecentsRead {
    pub recents: Recents,
    pub warning: Option<String>,
}

impl Sources {
    /// Home, the standard folders and the mounted drives. A file that
    /// cannot be read just contributes nothing.
    pub fn read_places(&self) -> Vec<Place> {
        let dirs = fs::read_to_string(&self.user_dirs_path).unwrap_or_default();
        let mounts = fs::read_to_string(&self.mountinfo_path).unwrap_or_default();
        let mut out = places::user_dirs(&dirs, &self.home);
        out.extend(places::mounted_drives(&mounts, &self.user));
        out
    }

    pub fn read_recents(&self) -> RecentsRead {
        let loaded = recents::load(&self.recents_path);
        RecentsRead {
            recents: loaded.recents,
            warning: loaded.warning,
        }
    }
}

/// What the settings file says that the strip uses, and what is wrong with it.
pub struct SettingsRead {
    pub recents_enabled: bool,
    pub rails: ofolder_core::rails::RailLayout,
    pub warnings: Vec<String>,
}

pub fn read_settings(path: &Path, home: &Path) -> SettingsRead {
    let loaded = settings::load(path, home);
    SettingsRead {
        recents_enabled: loaded.settings.recents_enabled,
        rails: loaded.settings.rails,
        warnings: loaded.warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ofolder_core::places::PlaceKind;

    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Dir {
            let p = std::env::temp_dir().join(format!("ofolder-src-{}-{tag}", std::process::id()));
            fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    fn sources(d: &Dir) -> Sources {
        Sources {
            user_dirs_path: d.0.join("user-dirs.dirs"),
            mountinfo_path: d.0.join("mountinfo"),
            recents_path: d.0.join("recents.json"),
            home: "/home/ann".into(),
            user: "ann".into(),
        }
    }

    #[test]
    fn places_list_home_then_standard_folders_then_drives() {
        let d = Dir::new("places");
        fs::write(
            d.0.join("user-dirs.dirs"),
            "XDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\n",
        )
        .unwrap();
        fs::write(
            d.0.join("mountinfo"),
            "36 25 8:1 / /run/media/ann/USB rw - vfat /dev/sdb1 rw\n",
        )
        .unwrap();
        let places = sources(&d).read_places();
        let kinds: Vec<_> = places.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            [PlaceKind::Home, PlaceKind::UserFolder, PlaceKind::Drive]
        );
    }

    #[test]
    fn a_drive_that_is_unplugged_disappears_on_the_next_read() {
        let d = Dir::new("unplug");
        let s = sources(&d);
        fs::write(
            d.0.join("mountinfo"),
            "36 25 8:1 / /run/media/ann/USB rw - vfat /dev/sdb1 rw\n",
        )
        .unwrap();
        assert!(s.read_places().iter().any(|p| p.kind == PlaceKind::Drive));
        fs::write(d.0.join("mountinfo"), "").unwrap();
        assert!(!s.read_places().iter().any(|p| p.kind == PlaceKind::Drive));
    }

    #[test]
    fn missing_files_leave_just_home() {
        let d = Dir::new("nofiles");
        let places = sources(&d).read_places();
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].kind, PlaceKind::Home);
    }

    #[test]
    fn recents_are_read_from_the_file_and_a_missing_file_is_empty() {
        let d = Dir::new("recents");
        let s = sources(&d);
        assert!(s.read_recents().recents.folders.is_empty());
        fs::write(
            d.0.join("recents.json"),
            r#"{"folders":[{"path":"/srv/a","apps":["x"],"at":5}],"files":[]}"#,
        )
        .unwrap();
        assert_eq!(s.read_recents().recents.folders.len(), 1);
    }

    #[test]
    fn corrupt_recents_come_back_empty_with_a_warning() {
        let d = Dir::new("corrupt");
        fs::write(d.0.join("recents.json"), "not json").unwrap();
        let read = sources(&d).read_recents();
        assert!(read.recents.folders.is_empty());
        assert!(read.warning.is_some());
    }

    #[test]
    fn settings_that_turn_recents_off_are_noticed_and_bad_values_are_reported() {
        let d = Dir::new("settings");
        let path = d.0.join("settings.toml");
        fs::write(&path, "recents_enabled = false\nrecents_max = \"x\"\n").unwrap();
        let read = read_settings(&path, Path::new("/home/ann"));
        assert!(!read.recents_enabled);
        assert_eq!(read.warnings.len(), 1);
    }

    #[test]
    fn a_missing_settings_file_leaves_recents_on_without_warnings() {
        let d = Dir::new("nosettings");
        let read = read_settings(&d.0.join("none.toml"), Path::new("/home/ann"));
        assert!(read.recents_enabled && read.warnings.is_empty());
    }
}
