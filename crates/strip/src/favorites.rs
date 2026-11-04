//! The saved favorites: a list of folders, in the user's order, kept in one
//! JSON file. Nothing here knows about the window.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use ofolder_core::private_file;

use crate::error::StripError;
use crate::paths;

/// The file format version this build reads and writes.
const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Favorite {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Favorite {
    /// What the list shows: the chosen name, else the folder's own name,
    /// else the whole path (for `/`).
    pub fn label(&self, home: &Path) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        match self.path.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => paths::abbreviate(&self.path, home),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct FileFormat {
    #[serde(default = "default_version")]
    version: u32,
    favorites: Vec<Favorite>,
}

fn default_version() -> u32 {
    FORMAT_VERSION
}

#[derive(Debug, PartialEq, Eq)]
pub enum AddOutcome {
    Added,
    Duplicate,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Favorites {
    items: Vec<Favorite>,
}

/// What loading found: the list, and a message when something needs the
/// user to know (a corrupt file set aside, a file that cannot be read).
#[derive(Debug)]
pub struct Loaded {
    pub favorites: Favorites,
    pub notice: Option<String>,
}

impl Favorites {
    pub fn items(&self) -> &[Favorite] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.items.iter().any(|f| f.path == path)
    }

    pub fn add(&mut self, path: PathBuf) -> AddOutcome {
        if self.contains(&path) {
            return AddOutcome::Duplicate;
        }
        self.items.push(Favorite { path, name: None });
        AddOutcome::Added
    }

    pub fn remove(&mut self, index: usize) -> Option<Favorite> {
        (index < self.items.len()).then(|| self.items.remove(index))
    }

    pub fn remove_path(&mut self, path: &Path) -> bool {
        let before = self.items.len();
        self.items.retain(|f| f.path != path);
        self.items.len() != before
    }

    /// Sets the display name; a blank name clears it.
    pub fn rename(&mut self, index: usize, name: &str) -> bool {
        let name = name.trim();
        match self.items.get_mut(index) {
            Some(item) => {
                item.name = (!name.is_empty()).then(|| name.to_owned());
                true
            }
            None => false,
        }
    }

    pub fn move_up(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.items.len() {
            return false;
        }
        self.items.swap(index, index - 1);
        true
    }

    pub fn move_down(&mut self, index: usize) -> bool {
        if index + 1 >= self.items.len() {
            return false;
        }
        self.items.swap(index, index + 1);
        true
    }

    fn from_items(items: Vec<Favorite>) -> Favorites {
        let mut list = Favorites::default();
        for item in items {
            if !list.contains(&item.path) {
                list.items.push(item);
            }
        }
        list
    }
}

/// Reads the favorites. A missing file is an empty list. A file that cannot
/// be parsed is renamed to `<name>.corrupt` (never overwriting an earlier
/// one) and the list starts empty, with a notice for the user.
pub fn load(path: &Path) -> Loaded {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Loaded {
                favorites: Favorites::default(),
                notice: None,
            };
        }
        Err(e) => {
            let error = StripError::Read {
                path: path.to_owned(),
                source: e,
            };
            return Loaded {
                favorites: Favorites::default(),
                notice: Some(format!("Favorites not loaded: {error}")),
            };
        }
    };
    match serde_json::from_str::<FileFormat>(&text) {
        Ok(file) if file.version <= FORMAT_VERSION => Loaded {
            favorites: Favorites::from_items(file.favorites),
            notice: None,
        },
        _ => {
            let notice = match set_aside(path) {
                Ok(aside) => format!(
                    "Favorites file was unreadable; kept as {} and started empty",
                    aside.file_name().unwrap_or_default().to_string_lossy()
                ),
                Err(e) => format!("Favorites file was unreadable and could not be moved: {e}"),
            };
            Loaded {
                favorites: Favorites::default(),
                notice: Some(notice),
            }
        }
    }
}

fn set_aside(path: &Path) -> io::Result<PathBuf> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".corrupt");
    let mut target = path.with_file_name(&name);
    let mut n = 1;
    while target.exists() {
        let mut numbered = name.clone();
        numbered.push(format!(".{n}"));
        target = path.with_file_name(numbered);
        n += 1;
    }
    fs::rename(path, &target)?;
    Ok(target)
}

/// Writes the favorites private and whole, under the file's lock. The list is
/// saved from memory, so a second window that saved since this one loaded
/// is overwritten: the last save wins, and nothing merges the two lists.
pub fn save(path: &Path, favorites: &Favorites) -> Result<(), StripError> {
    let write_error = |source| StripError::Write {
        path: path.to_owned(),
        source,
    };
    let file = FileFormat {
        version: FORMAT_VERSION,
        favorites: favorites.items.clone(),
    };
    let mut text = serde_json::to_string_pretty(&file)?;
    text.push('\n');
    private_file::with_lock(path, || {
        private_file::write_private_atomic(path, text.as_bytes())
    })
    .map_err(write_error)?
    .map_err(write_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> tempdir::Dir {
        tempdir::Dir::new()
    }

    /// A tiny self-cleaning temporary folder; the crate has no tempfile dependency.
    mod tempdir {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU32, Ordering};

        static NEXT: AtomicU32 = AtomicU32::new(0);

        pub struct Dir(PathBuf);

        impl Dir {
            pub fn new() -> Dir {
                let n = NEXT.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("ofolder-strip-test-{}-{n}", std::process::id()));
                std::fs::create_dir_all(&path).unwrap();
                Dir(path)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).ok();
            }
        }
    }

    fn list(paths: &[&str]) -> Favorites {
        let mut f = Favorites::default();
        for p in paths {
            f.add(PathBuf::from(p));
        }
        f
    }

    fn names(f: &Favorites) -> Vec<String> {
        f.items()
            .iter()
            .map(|i| i.path.display().to_string())
            .collect()
    }

    #[test]
    fn adding_the_same_folder_twice_keeps_one_entry() {
        let mut f = list(&["/a"]);
        assert_eq!(f.add(PathBuf::from("/a")), AddOutcome::Duplicate);
        assert_eq!(f.add(PathBuf::from("/a/")), AddOutcome::Duplicate);
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn entries_move_up_and_down_and_stop_at_the_ends() {
        let mut f = list(&["/a", "/b", "/c"]);
        assert!(f.move_up(2));
        assert_eq!(names(&f), ["/a", "/c", "/b"]);
        assert!(f.move_down(0));
        assert_eq!(names(&f), ["/c", "/a", "/b"]);
        assert!(!f.move_up(0));
        assert!(!f.move_down(2));
        assert!(!f.move_down(9));
    }

    #[test]
    fn removing_by_index_and_by_path_works() {
        let mut f = list(&["/a", "/b", "/c"]);
        assert_eq!(f.remove(1).map(|x| x.path), Some(PathBuf::from("/b")));
        assert!(f.remove(5).is_none());
        assert!(f.remove_path(Path::new("/a")));
        assert!(!f.remove_path(Path::new("/a")));
        assert_eq!(names(&f), ["/c"]);
    }

    #[test]
    fn renaming_sets_a_display_name_and_a_blank_name_clears_it() {
        let home = Path::new("/home/x");
        let mut f = list(&["/srv/photos"]);
        assert_eq!(f.items()[0].label(home), "photos");
        assert!(f.rename(0, "  Pictures "));
        assert_eq!(f.items()[0].label(home), "Pictures");
        assert!(f.rename(0, "   "));
        assert_eq!(f.items()[0].label(home), "photos");
        assert!(!f.rename(3, "x"));
    }

    #[test]
    fn the_root_folder_is_labelled_by_its_path() {
        let f = list(&["/"]);
        assert_eq!(f.items()[0].label(Path::new("/home/x")), "/");
    }

    #[test]
    fn saved_favorites_load_back_with_order_and_names() {
        let dir = scratch();
        let file = dir.path().join("fav.json");
        let mut f = list(&["/b", "/a"]);
        f.rename(1, "Alpha");
        save(&file, &f).unwrap();
        let loaded = load(&file);
        assert_eq!(loaded.favorites, f);
        assert!(loaded.notice.is_none());
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_list_without_a_notice() {
        let dir = scratch();
        let loaded = load(&dir.path().join("none.json"));
        assert!(loaded.favorites.is_empty());
        assert!(loaded.notice.is_none());
    }

    #[test]
    fn saving_creates_the_parent_folder_and_leaves_no_temp_file() {
        let dir = scratch();
        let file = dir.path().join("deep/er/fav.json");
        save(&file, &list(&["/a"])).unwrap();
        let mut entries: Vec<_> = fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        entries.sort();
        assert_eq!(entries, ["fav.json", "fav.json.lock"]);
    }

    #[test]
    fn saved_favorites_are_readable_by_the_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let file = dir.path().join("fav.json");
        save(&file, &list(&["/a"])).unwrap();
        let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn saving_over_an_existing_file_replaces_it_whole() {
        let dir = scratch();
        let file = dir.path().join("fav.json");
        save(&file, &list(&["/a", "/b"])).unwrap();
        save(&file, &list(&["/c"])).unwrap();
        assert_eq!(load(&file).favorites, list(&["/c"]));
    }

    #[test]
    fn a_failed_save_leaves_the_previous_file_intact() {
        let dir = scratch();
        let file = dir.path().join("fav.json");
        save(&file, &list(&["/a"])).unwrap();
        // A folder that cannot be written to makes the save fail before the old file is touched.
        use std::os::unix::fs::PermissionsExt;
        let folder = dir.path();
        fs::set_permissions(folder, fs::Permissions::from_mode(0o500)).unwrap();
        let failed = save(&file, &list(&["/zzz"])).is_err();
        fs::set_permissions(folder, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(failed);
        assert_eq!(load(&file).favorites, list(&["/a"]));
    }

    #[test]
    fn a_corrupt_file_is_renamed_aside_and_the_list_starts_empty_with_a_notice() {
        let dir = scratch();
        let file = dir.path().join("fav.json");
        fs::write(&file, "{ not json").unwrap();
        let loaded = load(&file);
        assert!(loaded.favorites.is_empty());
        assert!(loaded.notice.unwrap().contains("fav.json.corrupt"));
        assert!(!file.exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("fav.json.corrupt")).unwrap(),
            "{ not json"
        );
    }

    #[test]
    fn a_second_corrupt_file_does_not_overwrite_the_first_one_set_aside() {
        let dir = scratch();
        let file = dir.path().join("fav.json");
        fs::write(&file, "first").unwrap();
        load(&file);
        fs::write(&file, "second").unwrap();
        load(&file);
        assert_eq!(
            fs::read_to_string(dir.path().join("fav.json.corrupt")).unwrap(),
            "first"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("fav.json.corrupt.1")).unwrap(),
            "second"
        );
    }

    #[test]
    fn a_file_from_a_newer_format_is_set_aside_rather_than_overwritten() {
        let dir = scratch();
        let file = dir.path().join("fav.json");
        fs::write(&file, r#"{"version": 99, "favorites": []}"#).unwrap();
        let loaded = load(&file);
        assert!(loaded.notice.is_some());
        assert!(dir.path().join("fav.json.corrupt").exists());
    }

    #[test]
    fn duplicates_in_the_file_are_dropped_on_load() {
        let dir = scratch();
        let file = dir.path().join("fav.json");
        fs::write(
            &file,
            r#"{"version":1,"favorites":[{"path":"/a"},{"path":"/b","name":"B"},{"path":"/a"}]}"#,
        )
        .unwrap();
        assert_eq!(names(&load(&file).favorites), ["/a", "/b"]);
    }
}
