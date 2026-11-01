//! Remembering where each application's dialog was last left.
//!
//! Keyed by the application identity (see `identity`), so an app that cannot
//! be identified remembers nothing. A remembered place is used only if it is
//! a sensible size and lies wholly on a monitor now, since monitors come and go.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;

const MIN_WIDTH: i32 = 300;
const MIN_HEIGHT: i32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogMemory {
    #[serde(default)]
    places: BTreeMap<String, Saved>,
}

impl DialogMemory {
    /// Notes where the dialog ended up. Nothing is kept for an unknown app
    /// (empty key) or for a size too small to be a real dialog.
    pub fn remember(&mut self, app_key: &str, rect: Rect) -> bool {
        if app_key.is_empty() || rect.w < MIN_WIDTH || rect.h < MIN_HEIGHT {
            return false;
        }
        self.places.insert(
            app_key.to_owned(),
            Saved {
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: rect.h,
            },
        );
        true
    }

    /// Where this app's dialog was last left, if that place still lies wholly
    /// on `bounds`, or on any of them if `bounds` is `None` the answer is unchecked.
    pub fn recall(&self, app_key: &str, bounds: Option<Rect>) -> Option<Rect> {
        let s = self.places.get(app_key)?;
        let rect = Rect {
            x: s.x,
            y: s.y,
            w: s.w,
            h: s.h,
        };
        match bounds {
            Some(b)
                if rect.x < b.x
                    || rect.y < b.y
                    || rect.right() > b.right()
                    || rect.bottom() > b.bottom() =>
            {
                None
            }
            _ => Some(rect),
        }
    }

    pub fn forget(&mut self, app_key: &str) -> bool {
        self.places.remove(app_key).is_some()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("could not write the dialog memory: {0}")]
    Write(#[from] std::io::Error),
    #[error("could not encode the dialog memory: {0}")]
    Encode(#[from] serde_json::Error),
}

/// Reads the memory. A file that cannot be read gives an empty memory: losing
/// where dialogs were left is a small thing and must never block a dialog.
pub fn load(path: &Path) -> DialogMemory {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, memory: &DialogMemory) -> Result<(), MemoryError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(memory)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn memory_path(
    override_path: Option<PathBuf>,
    xdg_data: Option<PathBuf>,
    home: &Path,
) -> PathBuf {
    override_path.unwrap_or_else(|| {
        xdg_data
            .unwrap_or_else(|| home.join(".local/share"))
            .join("ofolder/dialogs.json")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        w: 4096,
        h: 1152,
    };

    fn dialog(x: i32) -> Rect {
        Rect {
            x,
            y: 100,
            w: 900,
            h: 600,
        }
    }

    #[test]
    fn a_dialog_left_somewhere_is_recalled_for_the_same_app() {
        let mut m = DialogMemory::default();
        assert!(m.remember("class:zen", dialog(1500)));
        assert_eq!(m.recall("class:zen", Some(SCREEN)), Some(dialog(1500)));
        assert_eq!(m.recall("class:other", Some(SCREEN)), None);
    }

    #[test]
    fn an_unknown_app_remembers_nothing() {
        let mut m = DialogMemory::default();
        assert!(!m.remember("", dialog(0)));
        assert_eq!(m.recall("", None), None);
    }

    #[test]
    fn a_window_too_small_to_be_a_real_dialog_is_not_remembered() {
        let mut m = DialogMemory::default();
        assert!(!m.remember(
            "app",
            Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 100
            }
        ));
    }

    #[test]
    fn a_place_that_is_no_longer_wholly_on_a_monitor_is_not_used() {
        let mut m = DialogMemory::default();
        m.remember("app", dialog(3800));
        assert_eq!(
            m.recall("app", Some(SCREEN)),
            None,
            "it would hang off the right edge"
        );
        let wider = Rect {
            x: 0,
            y: 0,
            w: 5120,
            h: 1152,
        };
        assert_eq!(m.recall("app", Some(wider)), Some(dialog(3800)));
    }

    #[test]
    fn the_latest_place_replaces_the_earlier_one() {
        let mut m = DialogMemory::default();
        m.remember("app", dialog(100));
        m.remember("app", dialog(900));
        assert_eq!(m.recall("app", None), Some(dialog(900)));
        assert!(m.forget("app"));
        assert!(!m.forget("app"));
    }

    #[test]
    fn the_memory_reads_back_after_saving_and_a_broken_file_is_just_empty() {
        let dir = std::env::temp_dir().join(format!("ofolder-dialogs-{}", std::process::id()));
        let file = dir.join("dialogs.json");
        let mut m = DialogMemory::default();
        m.remember("app", dialog(1500));
        save(&file, &m).expect("save");
        assert_eq!(load(&file), m);
        fs::write(&file, "not json").expect("write");
        assert_eq!(load(&file), DialogMemory::default());
        assert_eq!(load(&dir.join("missing.json")), DialogMemory::default());
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn the_memory_lives_in_the_data_folder_unless_overridden() {
        let home = Path::new("/home/j");
        assert_eq!(
            memory_path(None, None, home),
            PathBuf::from("/home/j/.local/share/ofolder/dialogs.json")
        );
        assert_eq!(
            memory_path(Some(PathBuf::from("/x")), None, home),
            PathBuf::from("/x")
        );
    }
}
