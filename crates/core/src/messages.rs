//! The log of messages the strip showed, so one that went by too fast can be
//! read again: in the settings window, or in the file itself.
//!
//! The log is small and private: the last [`KEEP`] lines, readable only by
//! the user, rewritten whole and swapped into place so a crash never leaves
//! half a file. A message is one line; it never carries a file's contents.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// Lines the log keeps; older ones are dropped as new ones arrive.
pub const KEEP: usize = 200;
/// Characters of one message kept in the log.
const TEXT_LIMIT: usize = 500;

/// One message as it was shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Seconds since the Unix epoch.
    pub at: i64,
    /// The local time zone's offset from UTC at that moment, in seconds.
    pub offset: i32,
    /// "info" or "error".
    pub kind: &'static str,
    /// What the user was doing, such as "copy-path" or "go".
    pub context: &'static str,
    pub text: String,
}

/// `$XDG_STATE_HOME/ofolder/messages.log`, or the override.
pub fn log_path(
    override_path: Option<PathBuf>,
    xdg_state: Option<PathBuf>,
    home: &Path,
) -> PathBuf {
    override_path.unwrap_or_else(|| {
        xdg_state
            .unwrap_or_else(|| home.join(".local/state"))
            .join("ofolder/messages.log")
    })
}

/// Year, month, day, hour, minute and second of `secs` after the epoch,
/// counted in the proleptic Gregorian calendar.
pub fn civil(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    // Howard Hinnant's days-to-civil algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        month,
        day,
        (rest / 3_600) as u32,
        (rest % 3_600 / 60) as u32,
        (rest % 60) as u32,
    )
}

/// `2026-11-03 01:23:45` in local time.
pub fn format_local(at: i64, offset: i32) -> String {
    let (y, mo, d, h, mi, s) = civil(at + i64::from(offset));
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

/// The line written for `entry`: time, kind, context, then the text on the
/// same line, with line breaks and other control characters made spaces.
pub fn format_line(entry: &Entry) -> String {
    let text: String = entry
        .text
        .chars()
        .take(TEXT_LIMIT)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    format!(
        "{} {} {}: {}",
        format_local(entry.at, entry.offset),
        entry.kind,
        entry.context,
        text.trim()
    )
}

/// Adds `entries` to the log at `path`, keeping only the last [`KEEP`]
/// lines. The folder is created private when missing.
pub fn append(path: &Path, entries: &[Entry]) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let old = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let mut lines: Vec<String> = old.lines().map(str::to_owned).collect();
    lines.extend(entries.iter().map(format_line));
    let skip = lines.len().saturating_sub(KEEP);
    let mut text = lines[skip..].join("\n");
    text.push('\n');
    let tmp = path.with_extension(format!("log.{}.tmp", std::process::id()));
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        // Only the temporary file is ours to clean; the log is untouched.
        fs::remove_file(&tmp).ok();
    }
    written
}

/// The last `n` lines of the log, newest first. A missing or unreadable
/// log reads as no lines.
pub fn read_last(path: &Path, n: usize) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines().rev().take(n).map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ofolder-messages-{label}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        dir
    }

    fn entry(text: &str) -> Entry {
        Entry {
            at: 1_793_697_766,
            offset: -8 * 3_600,
            kind: "error",
            context: "go",
            text: text.to_owned(),
        }
    }

    #[test]
    fn the_log_lives_under_the_state_folder() {
        let home = Path::new("/home/j");
        assert_eq!(
            log_path(None, None, home),
            PathBuf::from("/home/j/.local/state/ofolder/messages.log")
        );
        assert_eq!(
            log_path(None, Some(PathBuf::from("/s")), home),
            PathBuf::from("/s/ofolder/messages.log")
        );
        assert_eq!(
            log_path(Some(PathBuf::from("/x.log")), None, home),
            PathBuf::from("/x.log")
        );
    }

    #[test]
    fn times_are_written_in_local_time() {
        assert_eq!(format_local(0, 0), "1970-01-01 00:00:00");
        assert_eq!(format_local(951_782_400, 0), "2000-02-29 00:00:00");
        // 2026-11-03 09:22:46 UTC is 01:22:46 eight hours behind.
        assert_eq!(
            format_local(1_793_697_766, -8 * 3_600),
            "2026-11-03 01:22:46"
        );
        assert_eq!(format_local(-1, 0), "1969-12-31 23:59:59");
    }

    #[test]
    fn a_line_holds_time_kind_context_and_the_text_on_one_line() {
        assert_eq!(
            format_line(&entry("did not\narrive")),
            "2026-11-03 01:22:46 error go: did not arrive"
        );
        let long = "x".repeat(2_000);
        assert!(format_line(&entry(&long)).len() < 600);
    }

    #[test]
    fn the_log_keeps_only_the_newest_lines_and_is_private() {
        let dir = scratch("bounds");
        let path = dir.join("ofolder/messages.log");
        for i in 0..(KEEP + 30) {
            append(&path, &[entry(&format!("message {i}"))]).expect("append");
        }
        let text = fs::read_to_string(&path).expect("read");
        assert_eq!(text.lines().count(), KEEP);
        assert!(text.lines().next().expect("first").ends_with("message 30"));
        assert!(
            text.lines()
                .last()
                .expect("last")
                .ends_with(&format!("message {}", KEEP + 29))
        );
        let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = fs::metadata(path.parent().expect("dir"))
            .expect("meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        // No temporary file is left beside it.
        let names: Vec<_> = fs::read_dir(path.parent().expect("dir"))
            .expect("list")
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_newest_lines_are_read_first_and_a_missing_log_reads_as_none() {
        let dir = scratch("read");
        let path = dir.join("messages.log");
        assert!(read_last(&path, 5).is_empty());
        append(&path, &[entry("one"), entry("two"), entry("three")]).expect("append");
        let last = read_last(&path, 2);
        assert_eq!(last.len(), 2);
        assert!(last[0].ends_with("three") && last[1].ends_with("two"));
        append(&path, &[]).expect("nothing to add");
        assert_eq!(read_last(&path, 10).len(), 3);
        fs::remove_dir_all(&dir).ok();
    }
}
