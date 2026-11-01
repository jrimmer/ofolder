//! Finding a folder by typing a few of its letters.
//!
//! Every typed letter must appear in the candidate, in order, with anything
//! allowed in between. Better matches rank first: fewer gaps, letters
//! matching in case, letters at the start of a word or of the folder's name,
//! and matches in the last part of the path rather than the front of it.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Position of the candidate in the list that was searched.
    pub index: usize,
    pub score: i64,
}

fn is_boundary(prev: Option<char>) -> bool {
    prev.is_none_or(|c| matches!(c, '/' | '-' | '_' | ' ' | '.'))
}

/// How well `query` matches `candidate`, or `None` if it does not. Higher is
/// better. The letters are aligned to the candidate in the best way, not the
/// first way: `tr` matches the start of `tax-returns` in
/// `/home/j/Documents/tax-returns` rather than the `t` in `Documents`.
pub fn score(query: &str, candidate: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    const NONE: i64 = i64::MIN / 4;
    let first_lower = |ch: char| ch.to_lowercase().next().unwrap_or(ch);
    let q: Vec<char> = query.chars().collect();
    let c: Vec<char> = candidate.chars().collect();
    let ql: Vec<char> = q.iter().map(|&x| first_lower(x)).collect();
    let cl: Vec<char> = c.iter().map(|&x| first_lower(x)).collect();
    let last_part = c.iter().rposition(|&ch| ch == '/').map_or(0, |i| i + 1);
    let n = c.len();

    // What one letter earns at one position, apart from how it joins the one before.
    let letter = |i: usize, j: usize| -> i64 {
        let mut s = 10;
        if c[j] == q[i] {
            s += 4;
        }
        if is_boundary(j.checked_sub(1).map(|k| c[k])) {
            s += 10;
        }
        if j == last_part {
            s += 8;
        }
        if j >= last_part {
            s += 6;
        }
        s
    };

    let mut prev = vec![NONE; n];
    for j in 0..n {
        if cl[j] == ql[0] {
            let distance = j.saturating_sub(last_part);
            prev[j] = letter(0, j) - i64::try_from(distance).unwrap_or(6).min(6);
        }
    }
    for (i, &want) in ql.iter().enumerate().skip(1) {
        let mut cur = vec![NONE; n];
        let mut far = NONE;
        for j in 0..n {
            if j >= 10 {
                far = far.max(prev[j - 10]);
            }
            if cl[j] != want {
                continue;
            }
            let mut best = NONE;
            if j >= 1 && prev[j - 1] > NONE {
                best = prev[j - 1] + 14;
            }
            let (lo, hi) = (j.saturating_sub(9), j.saturating_sub(1));
            for (offset, &earlier) in prev[lo..hi].iter().enumerate() {
                if earlier > NONE {
                    let gap = i64::try_from(j - (lo + offset) - 1).unwrap_or(8).min(8);
                    best = best.max(earlier - gap);
                }
            }
            if far > NONE {
                best = best.max(far - 8);
            }
            if best > NONE {
                cur[j] = best + letter(i, j);
            }
        }
        prev = cur;
    }
    let best = prev.into_iter().max().filter(|&b| b > NONE)?;
    // Shorter candidates win a tie: the match covers more of them.
    Some(best * 4 - i64::try_from(n).unwrap_or(0))
}

/// The candidates that match, best first. Equal scores keep their order.
pub fn rank<S: AsRef<str>>(query: &str, candidates: &[S]) -> Vec<Hit> {
    let mut hits: Vec<Hit> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, c)| {
            Some(Hit {
                index,
                score: score(query, c.as_ref())?,
            })
        })
        .collect();
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(query: &str, list: &[&str]) -> Vec<String> {
        rank(query, list)
            .iter()
            .map(|h| list[h.index].to_owned())
            .collect()
    }

    #[test]
    fn the_initials_of_a_folders_name_list_it_first() {
        let list = [
            "/home/j/Documents/tax-returns",
            "/home/j/Downloads",
            "/home/j/Work/open-folder",
            "/home/j/Pictures",
        ];
        assert_eq!(order("tr", &list)[0], "/home/j/Documents/tax-returns");
        assert_eq!(order("of", &list)[0], "/home/j/Work/open-folder");
    }

    #[test]
    fn a_candidate_missing_a_typed_letter_does_not_match() {
        assert_eq!(score("xyz", "/home/j/Documents"), None);
        assert_eq!(score("sd", "/ds"), None, "letters must come in order");
    }

    #[test]
    fn letters_may_have_gaps_between_them() {
        assert!(score("dcmts", "/home/j/Documents").is_some());
    }

    #[test]
    fn a_match_in_the_last_part_beats_the_same_letters_earlier_in_the_path() {
        let list = ["/photos/misc/old", "/old/misc/photos"];
        assert_eq!(order("photos", &list)[0], "/old/misc/photos");
    }

    #[test]
    fn contiguous_letters_beat_scattered_ones() {
        let list = ["/a/d-o-w-n-l-o-a-d", "/a/download"];
        assert_eq!(order("down", &list)[0], "/a/download");
    }

    #[test]
    fn matching_case_wins_a_tie() {
        let list = ["/a/docs", "/a/Docs"];
        assert_eq!(order("Docs", &list)[0], "/a/Docs");
    }

    #[test]
    fn letters_at_the_start_of_words_beat_letters_buried_in_them() {
        let list = ["/x/abcdefgh", "/x/ab-cd-ef-gh"];
        assert_eq!(order("aceg", &list)[0], "/x/ab-cd-ef-gh");
    }

    #[test]
    fn an_empty_query_matches_everything_in_the_original_order() {
        assert_eq!(order("", &["/b", "/a"]), ["/b", "/a"]);
    }

    #[test]
    fn typing_is_case_insensitive_for_matching() {
        assert!(score("DOCS", "/home/j/documents").is_some());
    }

    #[test]
    fn unicode_names_match_by_letter_not_by_byte() {
        assert!(score("ünï", "/home/j/ünïcödé").is_some());
    }

    #[test]
    fn five_hundred_candidates_rank_well_inside_a_hundred_milliseconds() {
        let list: Vec<String> = (0..500)
            .map(|i| format!("/home/j/projects/client-{i}/documents/report-{}", i * 7))
            .collect();
        let start = std::time::Instant::now();
        let hits = rank("cdr", &list);
        assert!(!hits.is_empty());
        assert!(
            start.elapsed() < std::time::Duration::from_millis(100),
            "{:?}",
            start.elapsed()
        );
    }
}
