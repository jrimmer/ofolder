//! A stand-in for the dialog, to see the strip draw without one.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ofolder_core::recents::{self, RecentFile, RecentFolder, Recents};
use ofolder_core::theme::Palette;

use crate::{MenuRequest, RailSide, StripActions, StripOptions, TipRequest, favorites};

/// A fake dialog sitting in a real folder. `go` and `new_folder` act on the
/// real filesystem below whatever folder it is given, so give it a scratch
/// folder.
pub struct DemoActions {
    folder: Mutex<Option<PathBuf>>,
    calls: Mutex<Vec<String>>,
}

impl DemoActions {
    pub fn new(start: PathBuf) -> DemoActions {
        DemoActions {
            folder: Mutex::new(Some(start)),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// The launcher calls so far, as `file-manager <path>` or `terminal <path>`.
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }

    fn record(&self, call: String) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    fn set(&self, folder: Option<PathBuf>) {
        if let Ok(mut slot) = self.folder.lock() {
            *slot = folder;
        }
    }
}

impl StripActions for DemoActions {
    fn current_folder(&self) -> Option<PathBuf> {
        self.folder.lock().ok().and_then(|f| f.clone())
    }

    fn go(&self, folder: &Path) -> Result<(), String> {
        if !folder.is_dir() {
            return Err(format!("{} is not a folder", folder.display()));
        }
        self.set(Some(folder.to_owned()));
        Ok(())
    }

    fn new_folder(&self, name: &str) -> Result<PathBuf, String> {
        let base = self.current_folder().ok_or("The folder is unknown")?;
        let target = base.join(name);
        if target.exists() {
            return Err(format!("{name} already exists"));
        }
        fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        self.set(Some(target.clone()));
        Ok(target)
    }

    fn dialog_alive(&self) -> bool {
        true
    }

    fn open_in_file_manager(&self, folder: &Path) -> Result<(), String> {
        self.record(format!("file-manager {}", folder.display()));
        Ok(())
    }

    fn open_in_terminal(&self, folder: &Path) -> Result<(), String> {
        self.record(format!("terminal {}", folder.display()));
        Ok(())
    }

    /// Without a compositor to ask, the menu opens wherever the desktop puts it.
    fn place_menu(&self, _: Option<MenuRequest>) {}

    fn place_tip(&self, _: Option<TipRequest>) {}

    fn pointer_on_rail(&self, _: RailSide) -> Option<bool> {
        None
    }
}

/// Writes sample favorites, recents, settings, user folders and drives under
/// `base` (a scratch folder) and returns the options that use them with the
/// folder the fake dialog starts in. The drive points at a folder that does
/// not exist, so clicking it shows an error.
pub fn demo_setup(base: &Path, palette: Palette) -> anyhow::Result<(StripOptions, PathBuf)> {
    let folders = [
        "Documents",
        "Downloads",
        "Work/ofolder/src",
        "Work/ofolder/docs/guides",
        "Work/a-very-long-folder-name-for-the-strip",
    ];
    for sample in folders {
        fs::create_dir_all(base.join(sample))?;
    }
    fs::create_dir_all(base.join("Work/ofolder/.git"))?;
    let mut samples = favorites::Favorites::default();
    for sample in [
        "Documents",
        "Downloads",
        "Work/ofolder",
        "Work/a-very-long-folder-name-for-the-strip",
    ] {
        samples.add(base.join(sample));
    }
    samples.add(base.join("Deleted"));
    let favorites_path = base.join("favorites.json");
    favorites::save(&favorites_path, &samples)?;

    let mut recent = Recents::default();
    for i in 0..20 {
        let path = base.join(format!("Projects/p{i:02}"));
        fs::create_dir_all(&path)?;
        recent.folders.push(RecentFolder {
            path: path.display().to_string(),
            apps: vec![
                if i % 2 == 0 {
                    "org.demo.App"
                } else {
                    "org.other.App"
                }
                .to_owned(),
            ],
            at: 100 - i,
        });
    }
    recent.files.push(RecentFile {
        path: base.join("Documents/report.txt").display().to_string(),
        apps: vec!["org.demo.App".to_owned()],
        at: 90,
    });
    recent.files.push(RecentFile {
        path: base.join("Downloads/photo.png").display().to_string(),
        apps: vec!["org.other.App".to_owned()],
        at: 80,
    });
    let recents_path = base.join("recents.json");
    recents::save(&recents_path, &recent)?;

    let settings_path = base.join("settings.toml");
    fs::write(&settings_path, "terminal = \"foot\"\n")?;
    let user_dirs_path = base.join("user-dirs.dirs");
    fs::write(
        &user_dirs_path,
        "XDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\nXDG_DOCUMENTS_DIR=\"$HOME/Documents\"\n",
    )?;
    let mountinfo_path = base.join("mountinfo");
    fs::write(
        &mountinfo_path,
        "36 25 8:1 / /mnt/demo-usb rw,relatime - vfat /dev/sdb1 rw\n",
    )?;
    let options = StripOptions {
        label: "demo".to_owned(),
        favorites_path,
        home: base.to_path_buf(),
        app_key: "org.demo.App".to_owned(),
        recents_path,
        settings_path,
        user_dirs_path,
        mountinfo_path,
        user: "demo".to_owned(),
        palette,
    };
    Ok((options, base.join("Work/ofolder")))
}
