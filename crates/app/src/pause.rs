//! `ofolder pause` and `ofolder resume`: the one switch that stops everything.
//!
//! While paused nothing is shown, recorded or moved. The switch is the
//! `paused` line in the settings file, which is read for every dialog, so it
//! takes effect on the next one with no restart. Only that one line is
//! touched; the rest of the file, comments included, is left as it was.

use std::fs;
use std::io;
use std::path::Path;

/// The file text with `paused = <value>` set: the existing `paused` line is
/// replaced in place, or one is added at the top.
pub fn with_paused(text: &str, paused: bool) -> String {
    let line = format!("paused = {paused}");
    let mut replaced = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            let key = l.split('=').next().unwrap_or("").trim();
            if !replaced && l.contains('=') && key == "paused" {
                replaced = true;
                line.clone()
            } else {
                l.to_owned()
            }
        })
        .collect();
    if !replaced {
        out.insert(0, line);
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Sets the switch, creating the file if it does not exist. Written through a
/// temporary file and a rename, so an interrupted write leaves the old file.
pub fn set(path: &Path, paused: bool) -> io::Result<()> {
    let existing = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, with_paused(&existing, paused))?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use ofolder_core::settings;

    use super::*;

    #[test]
    fn a_missing_line_is_added_and_the_rest_of_the_file_is_kept() {
        let out = with_paused("# my notes\nrecents_max = 5\n", true);
        assert_eq!(out, "paused = true\n# my notes\nrecents_max = 5\n");
    }

    #[test]
    fn an_existing_line_is_changed_in_place() {
        let out = with_paused("a = 1\npaused = true\nb = 2\n", false);
        assert_eq!(out, "a = 1\npaused = false\nb = 2\n");
    }

    #[test]
    fn a_key_that_merely_contains_the_word_is_left_alone() {
        let out = with_paused("unpaused_note = 1\nxpaused = 2\n", true);
        assert!(out.starts_with("paused = true\n"));
        assert!(out.contains("unpaused_note = 1") && out.contains("xpaused = 2"));
    }

    #[test]
    fn the_result_is_a_settings_file_the_reader_understands() {
        let out = with_paused("recents_max = 5\n", true);
        let loaded = settings::parse(&out, Path::new("/home/j"));
        assert!(loaded.settings.paused && loaded.warnings.is_empty());
        assert_eq!(loaded.settings.recents_max, 5);
    }

    #[test]
    fn setting_creates_the_file_and_toggles_it_back() {
        let dir = std::env::temp_dir().join(format!("ofolder-pause-{}", std::process::id()));
        let file = dir.join("nested/settings.toml");
        set(&file, true).expect("pause");
        assert!(settings::load(&file, Path::new("/home/j")).settings.paused);
        set(&file, false).expect("resume");
        assert!(!settings::load(&file, Path::new("/home/j")).settings.paused);
        fs::remove_dir_all(&dir).expect("cleanup");
    }
}
