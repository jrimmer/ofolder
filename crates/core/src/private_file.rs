//! Writing the files that hold what ofolder remembers: privately, durably, and
//! without two writers losing each other's change.
//!
//! The state files (recents, dialog places, favorites, settings and the message
//! log) name folders and record what the user did, so only the user may read
//! them. A write goes to a temporary file in the same folder, created private
//! from its first byte, then replaces the old file in one rename; the folder is
//! flushed afterwards so the rename itself survives a power loss. A read followed
//! by a write is held under [`with_lock`], so two dialogs finishing at once
//! cannot each build on the same old file and drop one another's change.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A new state file: readable and writable by the user only.
const FILE_MODE: u32 = 0o600;
/// A new state folder: entered by the user only.
const DIR_MODE: u32 = 0o700;

/// Gives every temporary file in this process its own name, so two writers
/// never share one even when they write the same target at the same moment.
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// The folder `path` lives in. A bare file name lives in the current folder.
fn parent_of(path: &Path) -> &Path {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
}

/// Creates `dir` and any missing parents, each private. A folder that already
/// exists is left exactly as it is, mode included: the user chose it.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    DirBuilder::new().recursive(true).mode(DIR_MODE).create(dir)
}

/// A temporary file beside `path`, created readable by the user only. It is
/// never wider than that, not even for an instant, because the mode is given
/// at creation rather than changed afterwards.
fn create_temp(path: &Path) -> io::Result<(PathBuf, File)> {
    let name = path
        .file_name()
        .map_or_else(|| "file".to_owned(), |n| n.to_string_lossy().into_owned());
    let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let tmp = parent_of(path).join(format!(".{name}.{}.{n}.tmp", std::process::id()));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(&tmp)?;
    Ok((tmp, file))
}

/// Flushes a folder so a rename into it survives a crash.
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Replaces the file at `path` with `bytes`, creating it (and any missing
/// folders) private. On failure the old file is untouched and no temporary
/// file is left behind.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = parent_of(path);
    create_private_dir(dir)?;
    let (tmp, mut file) = create_temp(path)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written {
        fs::remove_file(&tmp).ok();
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, path) {
        fs::remove_file(&tmp).ok();
        return Err(e);
    }
    // The new file is in place; a failure to flush the folder is reported but
    // does not undo it.
    sync_dir(dir)
}

/// The advisory lock file beside `path`: `<name>.lock`. It is locked rather
/// than the state file itself, because a write replaces that file's inode and
/// a lock on the old inode would protect nothing.
fn lock_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(|| "file".to_owned(), |n| n.to_string_lossy().into_owned());
    parent_of(path).join(format!("{name}.lock"))
}

/// Runs `body` while holding an exclusive lock on `path`'s lock file, so no
/// other process or thread holding the same lock runs at the same time. The
/// lock is released when `body` returns. The error is only for taking the
/// lock; whatever `body` itself reports is its own return value.
pub fn with_lock<T>(path: &Path, body: impl FnOnce() -> T) -> io::Result<T> {
    create_private_dir(parent_of(path))?;
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(FILE_MODE)
        .open(lock_path(path))?;
    // Blocks until the other holder lets go. Locks taken through separate
    // opens conflict even inside one process, so threads are covered too.
    file.lock()?;
    Ok(body())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::AtomicUsize;

    use super::*;

    /// A folder that is removed when dropped, and made writable first so a
    /// test that made it read-only can still clean up.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!(
                "ofolder-private-{label}-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::remove_dir_all(&dir).ok();
            fs::create_dir_all(&dir).expect("scratch folder");
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700)).ok();
            fs::remove_dir_all(&self.0).ok();
        }
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    #[test]
    fn a_written_file_is_readable_by_the_user_only() {
        let s = Scratch::new("mode");
        let file = s.0.join("state.json");
        write_private_atomic(&file, b"{}").expect("write");
        assert_eq!(fs::read(&file).expect("read"), b"{}");
        assert_eq!(mode(&file), 0o600);
    }

    #[test]
    fn a_rewritten_file_becomes_private_and_holds_the_new_bytes() {
        let s = Scratch::new("rewrite");
        let file = s.0.join("state.json");
        fs::write(&file, "old").expect("old file");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).expect("chmod");
        write_private_atomic(&file, b"new").expect("write");
        assert_eq!(fs::read_to_string(&file).expect("read"), "new");
        assert_eq!(mode(&file), 0o600);
    }

    #[test]
    fn a_created_parent_is_private_and_a_pre_existing_one_keeps_its_mode() {
        let s = Scratch::new("parents");
        let existing = s.0.join("shared");
        fs::create_dir(&existing).expect("existing folder");
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o755)).expect("chmod");
        write_private_atomic(&existing.join("a.json"), b"x").expect("write beside it");
        assert_eq!(mode(&existing), 0o755, "the user's folder is left alone");

        let created = s.0.join("ofolder").join("nested");
        write_private_atomic(&created.join("b.json"), b"y").expect("write into new folders");
        assert_eq!(mode(&s.0.join("ofolder")), 0o700);
        assert_eq!(mode(&created), 0o700);
    }

    #[test]
    fn the_temporary_file_is_private_from_the_moment_it_exists() {
        let s = Scratch::new("temp");
        let target = s.0.join("state.json");
        let (tmp, file) = create_temp(&target).expect("temp");
        // Checked before anything is written, when a wider mode would already leak.
        assert_eq!(mode(&tmp), 0o600);
        drop(file);
        fs::remove_file(&tmp).expect("cleanup");
    }

    #[test]
    fn two_temporary_files_for_one_target_never_share_a_name() {
        let s = Scratch::new("names");
        let target = s.0.join("state.json");
        let (first, _a) = create_temp(&target).expect("first");
        let (second, _b) = create_temp(&target).expect("second");
        assert_ne!(first, second);
    }

    #[test]
    fn a_failed_write_leaves_the_old_file_intact_and_no_temporary_file() {
        let s = Scratch::new("failed");
        let file = s.0.join("state.json");
        fs::write(&file, "keep me").expect("old file");
        // A folder that cannot be written to makes the temporary file fail to open.
        fs::set_permissions(&s.0, fs::Permissions::from_mode(0o500)).expect("chmod");
        assert!(write_private_atomic(&file, b"lost").is_err());
        fs::set_permissions(&s.0, fs::Permissions::from_mode(0o700)).expect("chmod back");
        assert_eq!(fs::read_to_string(&file).expect("read"), "keep me");
        assert_eq!(fs::read_dir(&s.0).expect("list").count(), 1);
    }

    #[test]
    fn a_rename_that_fails_removes_its_temporary_file() {
        let s = Scratch::new("rename");
        // A non-empty folder cannot be replaced by a file, so the rename fails
        // after the temporary file was written.
        let target = s.0.join("state.json");
        fs::create_dir(&target).expect("blocking folder");
        fs::write(target.join("inside"), "x").expect("content");
        assert!(write_private_atomic(&target, b"data").is_err());
        let names: Vec<_> = fs::read_dir(&s.0)
            .expect("list")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(names, ["state.json"], "no temporary file is left");
    }

    #[test]
    fn the_lock_file_is_private_and_sits_beside_the_state_file() {
        let s = Scratch::new("lockfile");
        let file = s.0.join("recents.json");
        with_lock(&file, || ()).expect("lock");
        assert_eq!(mode(&s.0.join("recents.json.lock")), 0o600);
    }

    #[test]
    fn two_holders_of_the_lock_never_run_at_the_same_time() {
        let s = Scratch::new("exclusive");
        let file = s.0.join("recents.json");
        let inside = AtomicUsize::new(0);
        let most_at_once = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..20 {
                        with_lock(&file, || {
                            let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                            most_at_once.fetch_max(now, Ordering::SeqCst);
                            std::thread::sleep(std::time::Duration::from_millis(1));
                            inside.fetch_sub(1, Ordering::SeqCst);
                        })
                        .expect("lock");
                    }
                });
            }
        });
        assert_eq!(most_at_once.load(Ordering::SeqCst), 1);
    }
}
