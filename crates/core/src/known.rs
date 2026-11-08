//! Which versions of the programs ofolder docks beside were actually tried with it.
//!
//! ofolder reads what Flea and Strata show (their picker contract and accessibility labels are not
//! documented interfaces), so a version nobody tried may behave differently. This table is the
//! versions the end-to-end suite ran against; anything else is said once, in plain words, instead
//! of failing quietly. It says "not tried", never "broken": most patch releases are fine.

/// Backend id and the versions tried with it. The GTK portal has no version of its own to ask for,
/// and Flea 0.3 has no program that reports one, so neither is listed.
pub const TESTED: &[(&str, &[&str])] = &[("flea", &["0.4.0"]), ("strata", &["1.1.1"])];

/// The version number in what a program printed for `--version`: the first word of the first line
/// that starts with a digit, so both "0.4.0" and "strata 1.1.1" give their number.
pub fn version_of(output: &str) -> Option<String> {
    let line = output.lines().next()?;
    line.split_whitespace()
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()))
        .map(|w| {
            w.trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_owned()
        })
        .filter(|v| !v.is_empty() && v.len() <= 32)
}

/// The name a person knows the backend by.
fn display(backend: &str) -> String {
    if backend == "gtk" {
        return "GTK".to_owned();
    }
    let mut chars = backend.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

/// Says what ofolder sits beside, as far as it could be learned: "With Flea v0.4.0", or "With Flea"
/// when no version could be had. An empty backend says nothing.
pub fn describe(backend: &str, version: Option<&str>) -> String {
    if backend.is_empty() {
        return String::new();
    }
    match version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => format!("With {} v{}", display(backend), v.trim_start_matches('v')),
        None => format!("With {}", display(backend)),
    }
}

/// One line for a person when `version` of `backend` was not among those tried, else `None`.
pub fn untested_note(backend: &str, version: &str) -> Option<String> {
    let (_, tried) = TESTED.iter().find(|(id, _)| *id == backend)?;
    if tried.contains(&version) {
        return None;
    }
    Some(format!(
        "{} {version} has not been tried with ofolder (tried: {}). If the rails misbehave, \
         `ofolder pause` turns them off.",
        display(backend),
        tried.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_rails_sit_beside_is_said_with_its_version_when_known() {
        assert_eq!(describe("flea", Some("0.4.0")), "With Flea v0.4.0");
        assert_eq!(describe("strata", Some("v1.1.1")), "With Strata v1.1.1");
        assert_eq!(describe("gtk", Some("4.24.1")), "With GTK v4.24.1");
        assert_eq!(describe("flea", None), "With Flea");
        assert_eq!(describe("flea", Some("  ")), "With Flea");
        assert_eq!(describe("", Some("1")), "");
    }

    #[test]
    fn the_number_is_found_in_what_each_program_prints() {
        assert_eq!(version_of("0.4.0\n").as_deref(), Some("0.4.0"));
        assert_eq!(version_of("strata 1.1.1\n").as_deref(), Some("1.1.1"));
        assert_eq!(
            version_of("flea 0.5.0-beta.1 (abc)").as_deref(),
            Some("0.5.0-beta.1")
        );
        assert_eq!(version_of("no number here"), None);
        assert_eq!(version_of(""), None);
    }

    #[test]
    fn a_version_that_was_tried_says_nothing() {
        assert_eq!(untested_note("flea", "0.4.0"), None);
        assert_eq!(untested_note("strata", "1.1.1"), None);
    }

    #[test]
    fn a_version_that_was_not_tried_is_said_once_in_plain_words() {
        let note = untested_note("flea", "0.5.0").unwrap();
        assert!(note.starts_with("Flea 0.5.0 has not been tried"), "{note}");
        assert!(note.contains("tried: 0.4.0"), "{note}");
        assert!(note.contains("ofolder pause"), "{note}");
    }

    #[test]
    fn a_backend_without_a_version_to_ask_for_never_gets_a_note() {
        assert_eq!(untested_note("gtk", "1.15.3"), None);
        assert_eq!(untested_note("nonesuch", "1"), None);
    }
}
