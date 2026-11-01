//! A stand-in for the dialog, to see the strip draw without one.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::StripActions;

/// A fake dialog sitting in a real folder. `go` and `new_folder` act on the
/// real filesystem below whatever folder it is given, so give it a scratch
/// folder.
pub struct DemoActions {
    folder: Mutex<Option<PathBuf>>,
}

impl DemoActions {
    pub fn new(start: PathBuf) -> DemoActions {
        DemoActions {
            folder: Mutex::new(Some(start)),
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
}
