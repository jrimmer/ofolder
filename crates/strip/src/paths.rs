//! Showing paths in a strip that is 230 pixels wide.

use std::path::Path;

/// The path with the home folder written as `~`. A home of `/` or an empty
/// home is ignored, since abbreviating every path to `~/...` would be wrong.
pub fn abbreviate(path: &Path, home: &Path) -> String {
    let usable = home.components().count() > 1;
    if usable && let Ok(rest) = path.strip_prefix(home) {
        return if rest.as_os_str().is_empty() {
            "~".to_owned()
        } else {
            format!("~/{}", rest.display())
        };
    }
    path.display().to_string()
}

/// Cuts `text` to at most `max` characters by replacing its middle with an
/// ellipsis, so both ends stay readable.
pub fn shorten_middle(text: &str, max: usize) -> String {
    let max = max.max(3);
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_owned();
    }
    let keep = max - 1;
    let tail = keep / 2;
    let head = keep - tail;
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(&chars[chars.len() - tail..]);
    out
}

/// Cuts a path to at most `max` characters and always keeps the final
/// folder name, which is the part that tells folders apart. The cut falls
/// on a `/` when it can, giving `~/Work/…/last`.
pub fn shorten_path(path: &str, max: usize) -> String {
    let max = max.max(4);
    if path.chars().count() <= max {
        return path.to_owned();
    }
    let trimmed = path.trim_end_matches('/');
    let Some((_, last)) = trimmed.rsplit_once('/') else {
        return shorten_middle(trimmed, max);
    };
    let tail = format!("…/{last}");
    let tail_len = tail.chars().count();
    if tail_len >= max {
        return shorten_middle(last, max);
    }
    let budget = max - tail_len;
    let prefix: String = trimmed.chars().take(budget).collect();
    let prefix = match prefix.rfind('/') {
        Some(at) if at > 0 => prefix[..=at].to_owned(),
        _ => prefix,
    };
    format!("{prefix}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_is_written_as_a_tilde() {
        let home = Path::new("/home/ann");
        assert_eq!(abbreviate(Path::new("/home/ann"), home), "~");
        assert_eq!(abbreviate(Path::new("/home/ann/Work/x"), home), "~/Work/x");
    }

    #[test]
    fn paths_outside_home_and_lookalike_prefixes_are_left_alone() {
        let home = Path::new("/home/ann");
        assert_eq!(abbreviate(Path::new("/srv/data"), home), "/srv/data");
        assert_eq!(
            abbreviate(Path::new("/home/annabel/x"), home),
            "/home/annabel/x"
        );
    }

    #[test]
    fn a_root_home_is_not_used_for_abbreviation() {
        assert_eq!(abbreviate(Path::new("/etc"), Path::new("/")), "/etc");
        assert_eq!(abbreviate(Path::new("/etc"), Path::new("")), "/etc");
    }

    #[test]
    fn short_text_is_not_shortened() {
        assert_eq!(shorten_middle("abc", 10), "abc");
        assert_eq!(shorten_path("~/Work", 30), "~/Work");
    }

    #[test]
    fn middle_truncation_keeps_both_ends_and_the_limit() {
        let out = shorten_middle("abcdefghijklmnop", 9);
        assert_eq!(out.chars().count(), 9);
        assert!(out.starts_with("abcd") && out.ends_with("mnop") && out.contains('…'));
    }

    #[test]
    fn a_long_path_keeps_its_final_folder_name_visible() {
        let out = shorten_path("~/Work/clients/acme/projects/2026/reports/final-report", 28);
        assert!(out.chars().count() <= 28, "{out}");
        assert!(out.ends_with("/final-report"), "{out}");
        assert!(out.starts_with("~/"), "{out}");
    }

    #[test]
    fn a_300_character_path_fits_the_limit_and_keeps_the_last_name() {
        let path = format!("/{}/target-folder", "a".repeat(300));
        let out = shorten_path(&path, 30);
        assert!(out.chars().count() <= 30, "{out}");
        assert!(out.ends_with("/target-folder"), "{out}");
    }

    #[test]
    fn a_final_name_longer_than_the_limit_is_cut_in_the_middle() {
        let path = format!("/x/{}", "n".repeat(300));
        let out = shorten_path(&path, 20);
        assert_eq!(out.chars().count(), 20);
        assert!(out.contains('…'));
    }

    #[test]
    fn shortening_counts_characters_not_bytes() {
        let path = format!("/{}/é-dossier", "ü".repeat(100));
        let out = shorten_path(&path, 24);
        assert!(out.chars().count() <= 24, "{out}");
        assert!(out.ends_with("/é-dossier"), "{out}");
    }
}
