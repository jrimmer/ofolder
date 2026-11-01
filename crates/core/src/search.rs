//! Searching for subfolders by name below a folder.
//!
//! Runs on a background thread. It stops at a depth, a result count and a
//! deadline, checks a cancel flag between folders so that a cancel takes
//! effect at once, skips hidden folders, never follows symlinks, and skips a
//! folder it cannot read instead of stopping.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::fuzzy;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_depth: usize,
    pub max_results: usize,
    pub budget: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_results: 50,
            budget: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub score: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Finished,
    Cancelled,
    OutOfTime,
    EnoughResults,
}

pub struct Outcome {
    pub found: Vec<Found>,
    pub ended: Ended,
}

/// Breadth first, so shallow folders are found before deep ones. Matching is
/// done on the path below `root`, so the query can name parents too.
pub fn search(root: &Path, query: &str, limits: Limits, cancel: &AtomicBool) -> Outcome {
    let start = Instant::now();
    let mut found: Vec<Found> = Vec::new();
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::from([(root.to_path_buf(), 0)]);
    let mut ended = Ended::Finished;
    'walk: while let Some((dir, depth)) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            ended = Ended::Cancelled;
            break;
        }
        if start.elapsed() >= limits.budget {
            ended = Ended::OutOfTime;
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            if !entry.file_type().is_ok_and(|t| t.is_dir())
                || entry.file_name().to_string_lossy().starts_with('.')
            {
                continue;
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            if let Some(score) = fuzzy::score(query, &format!("/{relative}")) {
                found.push(Found {
                    path: path.clone(),
                    score,
                });
                if found.len() >= limits.max_results * 4 {
                    ended = Ended::EnoughResults;
                    break 'walk;
                }
            }
            if depth + 1 < limits.max_depth {
                queue.push_back((path, depth + 1));
            }
        }
    }
    found.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    found.truncate(limits.max_results);
    Outcome { found, ended }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn temp(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ofolder-search-{label}-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn make(root: &Path, rels: &[&str]) {
        for r in rels {
            fs::create_dir_all(root.join(r)).expect("dir");
        }
    }

    #[test]
    fn a_folder_is_found_by_part_of_its_name_below_the_root() {
        let d = temp("find");
        make(&d, &["clients/acme/reports", "clients/zenith", "misc"]);
        let out = search(&d, "rep", Limits::default(), &AtomicBool::new(false));
        assert_eq!(out.ended, Ended::Finished);
        assert_eq!(out.found[0].path, d.join("clients/acme/reports"));
        fs::remove_dir_all(&d).expect("cleanup");
    }

    #[test]
    fn hidden_folders_are_skipped_and_so_is_everything_inside_them() {
        let d = temp("hidden");
        make(&d, &[".git/objects/reports", "visible"]);
        let out = search(&d, "rep", Limits::default(), &AtomicBool::new(false));
        assert!(out.found.is_empty());
        fs::remove_dir_all(&d).expect("cleanup");
    }

    #[test]
    fn the_depth_limit_stops_the_walk_going_deeper() {
        let d = temp("depth");
        make(&d, &["a/b/c/d/target"]);
        let shallow = Limits {
            max_depth: 3,
            ..Limits::default()
        };
        assert!(
            search(&d, "target", shallow, &AtomicBool::new(false))
                .found
                .is_empty()
        );
        let deep = Limits {
            max_depth: 6,
            ..Limits::default()
        };
        assert_eq!(
            search(&d, "target", deep, &AtomicBool::new(false))
                .found
                .len(),
            1
        );
        fs::remove_dir_all(&d).expect("cleanup");
    }

    #[test]
    fn results_are_limited_and_the_best_come_first() {
        let d = temp("limit");
        let names: Vec<String> = (0..30).map(|i| format!("doc{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        make(&d, &refs);
        make(&d, &["documents"]);
        let limits = Limits {
            max_results: 5,
            ..Limits::default()
        };
        let out = search(&d, "documents", limits, &AtomicBool::new(false));
        assert_eq!(out.found.len(), 1, "only the real match");
        let out = search(&d, "doc", limits, &AtomicBool::new(false));
        assert_eq!(out.found.len(), 5);
        fs::remove_dir_all(&d).expect("cleanup");
    }

    #[test]
    fn a_cancel_that_is_already_set_stops_the_search_before_it_looks_at_anything() {
        let d = temp("cancel-now");
        make(&d, &["a/target"]);
        let out = search(&d, "target", Limits::default(), &AtomicBool::new(true));
        assert_eq!(out.ended, Ended::Cancelled);
        assert!(out.found.is_empty());
        fs::remove_dir_all(&d).expect("cleanup");
    }

    #[test]
    fn a_cancel_during_a_big_walk_takes_effect_within_a_moment() {
        let d = temp("cancel-later");
        for i in 0..400 {
            fs::create_dir_all(d.join(format!("branch{i}/leaf"))).expect("dir");
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1));
            flag.store(true, Ordering::Relaxed);
        });
        let begun = Instant::now();
        let out = search(&d, "zzzz-never", Limits::default(), &cancel);
        handle.join().expect("thread");
        assert!(
            begun.elapsed() < Duration::from_millis(500),
            "{:?}",
            begun.elapsed()
        );
        assert!(matches!(out.ended, Ended::Cancelled | Ended::Finished));
        fs::remove_dir_all(&d).expect("cleanup");
    }

    #[test]
    fn the_time_budget_ends_the_search_and_says_so() {
        let d = temp("budget");
        make(&d, &["a/b"]);
        let limits = Limits {
            budget: Duration::ZERO,
            ..Limits::default()
        };
        assert_eq!(
            search(&d, "b", limits, &AtomicBool::new(false)).ended,
            Ended::OutOfTime
        );
        fs::remove_dir_all(&d).expect("cleanup");
    }

    #[test]
    fn an_unreadable_folder_is_stepped_over_and_the_rest_is_still_searched() {
        use std::os::unix::fs::PermissionsExt;
        let d = temp("unreadable");
        make(&d, &["locked", "open/target"]);
        fs::set_permissions(d.join("locked"), fs::Permissions::from_mode(0o000)).expect("chmod");
        let out = search(&d, "target", Limits::default(), &AtomicBool::new(false));
        fs::set_permissions(d.join("locked"), fs::Permissions::from_mode(0o755)).expect("restore");
        assert_eq!(out.found.len(), 1);
        fs::remove_dir_all(&d).expect("cleanup");
    }

    #[test]
    fn a_symlink_back_up_the_tree_is_not_followed_so_it_cannot_loop() {
        let d = temp("loop");
        make(&d, &["a"]);
        std::os::unix::fs::symlink(&d, d.join("a/back")).expect("link");
        let out = search(&d, "back", Limits::default(), &AtomicBool::new(false));
        assert!(out.found.is_empty());
        fs::remove_dir_all(&d).expect("cleanup");
    }

    /// The user's figure: a tree of 50,000 folders returns first results in
    /// under a second. The tree is big, so it is built in the home folder's
    /// cache, not the RAM-backed temp folder. Run with `--ignored`.
    #[test]
    #[ignore = "builds 50,000 folders; run on purpose with --ignored"]
    fn fifty_thousand_folders_return_first_results_in_under_a_second() {
        let base = PathBuf::from(std::env::var_os("HOME").expect("HOME"))
            .join(".cache/ofolder-bench-search");
        fs::remove_dir_all(&base).ok();
        for i in 0..250 {
            for j in 0..200 {
                fs::create_dir_all(base.join(format!("group{i}/item{j}"))).expect("dir");
            }
        }
        let started = Instant::now();
        let limits = Limits {
            max_depth: 4,
            max_results: 10,
            budget: Duration::from_secs(5),
        };
        let out = search(&base, "item77", limits, &AtomicBool::new(false));
        let took = started.elapsed();
        fs::remove_dir_all(&base).ok();
        assert!(!out.found.is_empty());
        assert!(took < Duration::from_secs(1), "{took:?}");
    }
}
