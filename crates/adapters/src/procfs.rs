//! Reading process information from `/proc`, with the root injectable so the
//! logic can be tested against a made-up tree.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ProcFs {
    root: PathBuf,
}

impl Default for ProcFs {
    fn default() -> Self {
        Self::with_root("/proc")
    }
}

/// The parent pid out of a `/proc/<pid>/stat` line. The command name sits in
/// parentheses and may itself contain spaces and parentheses, so the fields
/// are counted from the last closing parenthesis.
pub fn parse_ppid(stat: &str) -> Option<u32> {
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(1)?.parse().ok()
}

/// The state letter out of a `/proc/<pid>/stat` line (`R`, `S`, `Z`...).
pub fn parse_state(stat: &str) -> Option<char> {
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().next()?.chars().next()
}

impl ProcFs {
    /// Whether `pid` is a process that has not exited: it is listed and is
    /// neither a zombie waiting to be reaped nor dead.
    pub fn is_running(&self, pid: u32) -> bool {
        fs::read_to_string(self.root.join(pid.to_string()).join("stat"))
            .ok()
            .and_then(|stat| parse_state(&stat))
            .is_some_and(|state| !matches!(state, 'Z' | 'X' | 'x'))
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn pids(&self) -> Vec<u32> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .collect()
    }

    /// Every process below `root_pid`, children and their children.
    pub fn descendants(&self, root_pid: u32) -> Vec<u32> {
        let parents: Vec<(u32, u32)> = self
            .pids()
            .into_iter()
            .filter_map(|pid| {
                let stat = fs::read_to_string(self.root.join(pid.to_string()).join("stat")).ok()?;
                Some((pid, parse_ppid(&stat)?))
            })
            .collect();
        let mut found = Vec::new();
        let mut frontier = vec![root_pid];
        while let Some(parent) = frontier.pop() {
            for &(pid, ppid) in &parents {
                if ppid == parent && !found.contains(&pid) {
                    found.push(pid);
                    frontier.push(pid);
                }
            }
        }
        found.sort_unstable();
        found
    }

    /// The command line with its separators turned into spaces.
    pub fn cmdline(&self, pid: u32) -> Option<String> {
        let raw = fs::read(self.root.join(pid.to_string()).join("cmdline")).ok()?;
        Some(
            String::from_utf8_lossy(&raw)
                .replace('\0', " ")
                .trim()
                .to_owned(),
        )
    }

    /// Directories the process holds open through a file descriptor. Its
    /// working directory is not listed there, so a folder that happens to be
    /// the working directory still counts when the process really holds it
    /// open: Flea's helper runs from the home folder, and home is a folder a
    /// dialog shows.
    pub fn open_dirs(&self, pid: u32) -> Vec<PathBuf> {
        let base = self.root.join(pid.to_string());
        let Ok(fds) = fs::read_dir(base.join("fd")) else {
            return Vec::new();
        };
        let mut dirs: Vec<PathBuf> = fds
            .filter_map(Result::ok)
            .filter_map(|fd| fs::read_link(fd.path()).ok())
            .filter(|target| target.is_absolute() && Path::is_dir(target))
            .collect();
        dirs.sort();
        dirs
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// A temporary directory removed when dropped.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(label: &str) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("ofolder-{label}-{}-{n}", std::process::id()));
            fs::create_dir_all(&dir).expect("temp dir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            // Cleanup only: a leftover temp directory must not fail a test.
            fs::remove_dir_all(&self.0).ok();
        }
    }

    /// Adds `/proc/<pid>` with a command line, a parent, a working directory
    /// and some open directories.
    pub fn add_process(
        root: &Path,
        pid: u32,
        ppid: u32,
        cmdline: &str,
        cwd: &Path,
        open: &[&Path],
    ) {
        let dir = root.join(pid.to_string());
        fs::create_dir_all(dir.join("fd")).expect("fd dir");
        fs::write(
            dir.join("stat"),
            format!("{pid} (fake proc) S {ppid} 1 1 0"),
        )
        .expect("stat");
        fs::write(dir.join("cmdline"), cmdline.replace(' ', "\0")).expect("cmdline");
        symlink(cwd, dir.join("cwd")).expect("cwd link");
        for (i, target) in open.iter().enumerate() {
            symlink(target, dir.join("fd").join((i + 3).to_string())).expect("fd link");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{TempDir, add_process};
    use super::*;

    #[test]
    fn the_parent_is_found_even_when_the_command_name_has_spaces_and_parentheses() {
        assert_eq!(parse_ppid("42 (a (b) c) S 7 42 42 0"), Some(7));
    }

    #[test]
    fn a_stat_line_with_no_parenthesis_has_no_parent() {
        assert_eq!(parse_ppid("garbage"), None);
    }

    #[test]
    fn descendants_include_grandchildren_and_exclude_unrelated_processes() {
        let tmp = TempDir::new("desc");
        let here = tmp.0.join("here");
        fs::create_dir_all(&here).expect("dir");
        add_process(&tmp.0, 10, 1, "qs -p picker.qml", &here, &[]);
        add_process(&tmp.0, 11, 10, "flea --backend", &here, &[]);
        add_process(&tmp.0, 12, 11, "worker", &here, &[]);
        add_process(&tmp.0, 20, 1, "unrelated", &here, &[]);
        assert_eq!(ProcFs::with_root(&tmp.0).descendants(10), [11, 12]);
    }

    #[test]
    fn open_dirs_keep_a_held_folder_even_when_it_is_the_working_directory_and_leave_out_files() {
        let tmp = TempDir::new("dirs");
        let (cwd, shown) = (tmp.0.join("cwd"), tmp.0.join("shown"));
        fs::create_dir_all(&cwd).expect("dir");
        fs::create_dir_all(&shown).expect("dir");
        let file = tmp.0.join("a-file");
        fs::write(&file, "x").expect("file");
        add_process(
            &tmp.0,
            11,
            10,
            "flea --backend",
            &cwd,
            &[&shown, &cwd, &file],
        );
        let mut expected = vec![cwd, shown];
        expected.sort();
        assert_eq!(ProcFs::with_root(&tmp.0).open_dirs(11), expected);
    }

    #[test]
    fn the_command_line_is_readable_with_spaces() {
        let tmp = TempDir::new("cmd");
        add_process(&tmp.0, 11, 10, "/usr/bin/flea --backend", &tmp.0, &[]);
        assert_eq!(
            ProcFs::with_root(&tmp.0).cmdline(11).as_deref(),
            Some("/usr/bin/flea --backend")
        );
    }

    #[test]
    fn a_process_is_running_until_it_is_a_zombie_or_gone() {
        let tmp = TempDir::new("running");
        add_process(&tmp.0, 11, 1, "qs", &tmp.0, &[]);
        let procfs = ProcFs::with_root(&tmp.0);
        assert!(procfs.is_running(11));
        fs::write(tmp.0.join("11").join("stat"), "11 (a (b) c) Z 1 1 1 0").expect("stat");
        assert!(!procfs.is_running(11));
        assert!(!procfs.is_running(12));
        assert_eq!(parse_state("11 (a (b) c) Z 1 1"), Some('Z'));
    }

    #[test]
    fn a_missing_process_reads_as_nothing() {
        let tmp = TempDir::new("none");
        let procfs = ProcFs::with_root(&tmp.0);
        assert_eq!(procfs.cmdline(999), None);
        assert!(procfs.open_dirs(999).is_empty());
        assert!(procfs.descendants(999).is_empty());
    }
}
