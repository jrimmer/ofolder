//! Checking the name typed for a new folder before it is sent anywhere.

/// Longest single folder name on common Linux filesystems, in bytes.
const MAX_COMPONENT_BYTES: usize = 255;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("Type a folder name")]
    Empty,
    #[error("A name cannot start with /")]
    LeadingSlash,
    #[error("A name made of dots is not allowed")]
    Dots,
    #[error("Remove the empty part between slashes")]
    EmptyPart,
    #[error("A name cannot contain a null character")]
    Nul,
    #[error("A folder name is longer than 255 bytes")]
    TooLong,
}

/// Returns the name to create: trimmed, without a trailing slash. A name
/// may hold slashes to mean nested folders, but no part may be empty or
/// made only of dots, since `..` would leave the dialog's folder.
pub fn validate_new_folder_name(input: &str) -> Result<String, NameError> {
    let name = input.trim();
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.starts_with('/') {
        return Err(NameError::LeadingSlash);
    }
    if name.contains('\0') {
        return Err(NameError::Nul);
    }
    let name = name.trim_end_matches('/');
    for part in name.split('/') {
        if part.is_empty() {
            return Err(NameError::EmptyPart);
        }
        if part.chars().all(|c| c == '.') {
            return Err(NameError::Dots);
        }
        if part.len() > MAX_COMPONENT_BYTES {
            return Err(NameError::TooLong);
        }
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_name_is_accepted_and_trimmed() {
        assert_eq!(validate_new_folder_name("  Reports "), Ok("Reports".into()));
    }

    #[test]
    fn nested_names_are_accepted() {
        assert_eq!(validate_new_folder_name("a/b/c"), Ok("a/b/c".into()));
    }

    #[test]
    fn a_trailing_slash_is_dropped() {
        assert_eq!(validate_new_folder_name("a/b/"), Ok("a/b".into()));
    }

    #[test]
    fn an_empty_or_blank_name_is_rejected() {
        assert_eq!(validate_new_folder_name(""), Err(NameError::Empty));
        assert_eq!(validate_new_folder_name("   "), Err(NameError::Empty));
    }

    #[test]
    fn a_name_that_is_only_dots_is_rejected() {
        for name in [".", "..", "...", "...."] {
            assert_eq!(
                validate_new_folder_name(name),
                Err(NameError::Dots),
                "{name}"
            );
        }
    }

    #[test]
    fn a_dots_part_that_would_leave_the_folder_is_rejected() {
        assert_eq!(validate_new_folder_name("a/../b"), Err(NameError::Dots));
        assert_eq!(validate_new_folder_name("../x"), Err(NameError::Dots));
    }

    #[test]
    fn a_leading_slash_is_rejected() {
        assert_eq!(
            validate_new_folder_name("/etc"),
            Err(NameError::LeadingSlash)
        );
        assert_eq!(
            validate_new_folder_name(" /x"),
            Err(NameError::LeadingSlash)
        );
        assert_eq!(validate_new_folder_name("/"), Err(NameError::LeadingSlash));
    }

    #[test]
    fn an_empty_part_between_slashes_is_rejected() {
        assert_eq!(validate_new_folder_name("a//b"), Err(NameError::EmptyPart));
    }

    #[test]
    fn dotted_names_that_are_not_only_dots_are_accepted() {
        assert_eq!(validate_new_folder_name(".config"), Ok(".config".into()));
        assert_eq!(validate_new_folder_name("v1.2"), Ok("v1.2".into()));
    }

    #[test]
    fn a_null_character_and_an_overlong_part_are_rejected() {
        assert_eq!(validate_new_folder_name("a\0b"), Err(NameError::Nul));
        assert_eq!(
            validate_new_folder_name(&"x".repeat(256)),
            Err(NameError::TooLong)
        );
    }
}
