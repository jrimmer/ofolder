//! The find box: ranking the folders the strip already knows as the user
//! types, and the state of a search below the current folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ofolder_core::fuzzy;
use ofolder_core::search::Ended;

/// How many ranked results the strip shows.
pub const SHOWN: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub label: String,
}

/// The last two parts of a path, so letters are not matched across a long
/// chain of parent folders (`Work/ofolder` for `/home/ann/Work/ofolder`).
fn tail(path: &Path) -> String {
    let parts: Vec<_> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let from = parts.len().saturating_sub(2);
    format!("/{}", parts[from..].join("/"))
}

/// The candidates that match `query`, best first, at most `limit`. A
/// candidate is matched on the end of its path and on its label, whichever
/// is better, so a favorite can be found by the name it was given.
pub fn rank_candidates(query: &str, candidates: &[Candidate], limit: usize) -> Vec<Candidate> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(i64, usize)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let by_path = fuzzy::score(query, &tail(&c.path));
            let by_label = fuzzy::score(query, &format!("/{}", c.label));
            by_path.max(by_label).map(|s| (s, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, i)| candidates[i].clone())
        .collect()
}

/// A search to run on a background thread.
#[derive(Clone, Debug)]
pub struct SearchJob {
    pub id: u64,
    pub root: PathBuf,
    pub query: String,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Debug, Default)]
pub enum SearchState {
    #[default]
    Idle,
    Running {
        id: u64,
        query: String,
        cancel: Arc<AtomicBool>,
    },
    Done {
        query: String,
        found: Vec<PathBuf>,
        ended: Ended,
    },
}

#[derive(Debug, Default)]
pub struct Finder {
    query: String,
    highlight: usize,
    search: SearchState,
    next_id: u64,
}

impl Finder {
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, query: &str) {
        if self.query != query {
            self.query = query.to_owned();
            self.highlight = 0;
        }
    }

    /// Empties the query and drops the results of any finished search.
    pub fn clear(&mut self) {
        self.query.clear();
        self.highlight = 0;
        self.stop();
        self.search = SearchState::Idle;
    }

    /// The highlighted position in a list of `len` results.
    pub fn highlight(&self, len: usize) -> Option<usize> {
        (len > 0).then(|| self.highlight.min(len - 1))
    }

    pub fn move_highlight(&mut self, delta: isize, len: usize) {
        if len == 0 {
            return;
        }
        let at = self.highlight.min(len - 1);
        self.highlight = at.saturating_add_signed(delta).min(len - 1);
    }

    /// Starts a search below `folder`. Never without a known folder or
    /// something to look for. A search already running is cancelled first.
    pub fn begin(&mut self, folder: Option<&Path>) -> Result<SearchJob, String> {
        let Some(root) = folder else {
            return Err("Cannot search: the folder is unknown".to_owned());
        };
        let query = self.query.trim().to_owned();
        if query.is_empty() {
            return Err("Type something to search for".to_owned());
        }
        self.stop();
        self.next_id += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.search = SearchState::Running {
            id: self.next_id,
            query: query.clone(),
            cancel: cancel.clone(),
        };
        Ok(SearchJob {
            id: self.next_id,
            root: root.to_path_buf(),
            query,
            cancel,
        })
    }

    /// Cancels a running search at once. What the thread returns later is
    /// ignored. Returns whether one was running.
    pub fn stop(&mut self) -> bool {
        if let SearchState::Running { query, cancel, .. } = &self.search {
            cancel.store(true, Ordering::Relaxed);
            self.search = SearchState::Done {
                query: query.clone(),
                found: Vec::new(),
                ended: Ended::Cancelled,
            };
            return true;
        }
        false
    }

    /// Stores what the thread found, unless that search was stopped or
    /// replaced meanwhile.
    pub fn finish(&mut self, id: u64, found: Vec<PathBuf>, ended: Ended) -> bool {
        match &self.search {
            SearchState::Running {
                id: current, query, ..
            } if *current == id => {
                self.search = SearchState::Done {
                    query: query.clone(),
                    found,
                    ended,
                };
                true
            }
            _ => false,
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.search, SearchState::Running { .. })
    }

    /// The folders a finished search found.
    pub fn found(&self) -> &[PathBuf] {
        match &self.search {
            SearchState::Done { found, .. } => found,
            _ => &[],
        }
    }

    /// A line of status under the search button.
    pub fn status(&self) -> Option<String> {
        match &self.search {
            SearchState::Idle => None,
            SearchState::Running { query, .. } => Some(format!("Searching for \"{query}\"...")),
            SearchState::Done {
                query,
                found,
                ended,
            } => Some(match ended {
                Ended::Cancelled => "Search stopped".to_owned(),
                Ended::OutOfTime => format!("Stopped at the time limit: {} found", found.len()),
                Ended::EnoughResults => format!("Showing the best {} for \"{query}\"", found.len()),
                Ended::Finished if found.is_empty() => format!("Nothing found for \"{query}\""),
                Ended::Finished => format!("{} found for \"{query}\"", found.len()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(path: &str, label: &str) -> Candidate {
        Candidate {
            path: path.into(),
            label: label.into(),
        }
    }

    #[test]
    fn typing_the_initials_of_a_folder_lists_it_first() {
        let list = [
            cand("/home/a/Documents", "Documents"),
            cand("/home/a/tax-returns", "tax-returns"),
            cand("/home/a/Downloads", "Downloads"),
        ];
        assert_eq!(rank_candidates("tr", &list, 8)[0].label, "tax-returns");
    }

    #[test]
    fn a_favorite_is_found_by_the_name_it_was_given() {
        let list = [cand("/srv/x1", "Invoices"), cand("/srv/x2", "Photos")];
        let out = rank_candidates("inv", &list, 8);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, PathBuf::from("/srv/x1"));
    }

    #[test]
    fn letters_are_not_matched_across_a_long_chain_of_parent_folders() {
        let list = [cand("/tmp/demo-one/Projects/p00", "p00")];
        assert!(rank_candidates("doc", &list, 8).is_empty());
    }

    #[test]
    fn a_query_can_name_the_parent_folder_too() {
        let list = [cand("/home/ann/Work/ofolder", "ofolder")];
        assert_eq!(rank_candidates("work/of", &list, 8).len(), 1);
    }

    #[test]
    fn a_blank_query_ranks_nothing() {
        assert!(rank_candidates("  ", &[cand("/a", "a")], 8).is_empty());
    }

    #[test]
    fn ranking_keeps_at_most_the_limit() {
        let list: Vec<_> = (0..30)
            .map(|i| cand(&format!("/a/dir{i}"), "dir"))
            .collect();
        assert_eq!(rank_candidates("dir", &list, SHOWN).len(), SHOWN);
    }

    #[test]
    fn the_highlight_moves_within_the_results_and_stops_at_the_ends() {
        let mut f = Finder::default();
        assert_eq!(f.highlight(3), Some(0));
        f.move_highlight(1, 3);
        f.move_highlight(1, 3);
        f.move_highlight(1, 3);
        assert_eq!(f.highlight(3), Some(2));
        f.move_highlight(-5, 3);
        assert_eq!(f.highlight(3), Some(0));
        assert_eq!(f.highlight(0), None);
    }

    #[test]
    fn changing_the_query_puts_the_highlight_back_on_the_top_result() {
        let mut f = Finder::default();
        f.set_query("a");
        f.move_highlight(1, 3);
        f.set_query("ab");
        assert_eq!(f.highlight(3), Some(0));
    }

    #[test]
    fn a_search_never_starts_without_a_known_folder() {
        let mut f = Finder::default();
        f.set_query("x");
        assert!(f.begin(None).is_err());
        assert!(!f.running());
    }

    #[test]
    fn a_search_never_starts_without_a_query() {
        let mut f = Finder::default();
        f.set_query("   ");
        assert!(f.begin(Some(Path::new("/a"))).is_err());
    }

    #[test]
    fn stopping_a_search_raises_its_cancel_flag_and_shows_stopped_at_once() {
        let mut f = Finder::default();
        f.set_query("x");
        let job = f.begin(Some(Path::new("/a"))).unwrap();
        assert!(f.stop());
        assert!(job.cancel.load(Ordering::Relaxed));
        assert!(!f.running());
        assert_eq!(f.status().as_deref(), Some("Search stopped"));
    }

    #[test]
    fn the_answer_of_a_stopped_search_is_ignored() {
        let mut f = Finder::default();
        f.set_query("x");
        let job = f.begin(Some(Path::new("/a"))).unwrap();
        f.stop();
        assert!(!f.finish(job.id, vec!["/a/x".into()], Ended::Finished));
        assert!(f.found().is_empty());
    }

    #[test]
    fn a_finished_search_keeps_its_results_and_says_how_many() {
        let mut f = Finder::default();
        f.set_query("x");
        let job = f.begin(Some(Path::new("/a"))).unwrap();
        assert!(f.finish(job.id, vec!["/a/x".into()], Ended::Finished));
        assert_eq!(f.found(), [PathBuf::from("/a/x")]);
        assert_eq!(f.status().as_deref(), Some("1 found for \"x\""));
    }

    #[test]
    fn starting_a_second_search_cancels_the_first_and_ignores_its_answer() {
        let mut f = Finder::default();
        f.set_query("x");
        let first = f.begin(Some(Path::new("/a"))).unwrap();
        let second = f.begin(Some(Path::new("/a"))).unwrap();
        assert!(first.cancel.load(Ordering::Relaxed));
        assert!(!f.finish(first.id, vec![], Ended::Finished));
        assert!(f.finish(second.id, vec![], Ended::Finished));
    }

    #[test]
    fn a_search_that_ran_out_of_time_says_so() {
        let mut f = Finder::default();
        f.set_query("x");
        let job = f.begin(Some(Path::new("/a"))).unwrap();
        f.finish(job.id, vec!["/a/x".into()], Ended::OutOfTime);
        assert!(f.status().unwrap().contains("time limit"));
    }

    #[test]
    fn clearing_the_box_stops_a_search_and_forgets_its_results() {
        let mut f = Finder::default();
        f.set_query("x");
        let job = f.begin(Some(Path::new("/a"))).unwrap();
        f.clear();
        assert!(job.cancel.load(Ordering::Relaxed));
        assert_eq!(f.query(), "");
        assert!(f.status().is_none());
    }
}
