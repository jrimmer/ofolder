//! What may be typed into a dialog on the user's behalf.
//!
//! `wtype` sends keystrokes to whichever window has focus, so a path that is
//! typed must survive being typed: it has to arrive character for character.

use std::path::Path;

/// The longest path typed into a dialog. Longer ones are refused, not cut.
pub const MAX_TYPED_PATH: usize = 4000;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("the path is not absolute")]
    NotAbsolute,
    #[error("the path is not valid UTF-8, so it cannot be typed faithfully")]
    NotUtf8,
    #[error("the path contains a control character, which would act as a key")]
    ControlCharacter,
    #[error("the path is longer than {MAX_TYPED_PATH} bytes")]
    TooLong,
}

pub fn check_typeable(path: &Path) -> Result<(), Refusal> {
    if !path.is_absolute() {
        return Err(Refusal::NotAbsolute);
    }
    let text = path.to_str().ok_or(Refusal::NotUtf8)?;
    if text.chars().any(char::is_control) {
        return Err(Refusal::ControlCharacter);
    }
    if text.len() > MAX_TYPED_PATH {
        return Err(Refusal::TooLong);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn awkward_but_typeable_names_are_accepted() {
        // Every one of these was typed into a real dialog and arrived intact.
        for name in [
            "/h/with space",
            "/h/ünïcödé 日本語",
            "/h/quo'te\"d",
            "/h/-dash",
            "/h/.hidden",
            "/h/$(touch PWNED);`id`;|&",
        ] {
            assert_eq!(check_typeable(Path::new(name)), Ok(()), "{name}");
        }
    }

    #[test]
    fn a_relative_path_is_refused() {
        assert_eq!(
            check_typeable(Path::new("Documents")),
            Err(Refusal::NotAbsolute)
        );
    }

    #[test]
    fn a_newline_is_refused_because_typing_it_would_press_return() {
        assert_eq!(
            check_typeable(Path::new("/h/a\nb")),
            Err(Refusal::ControlCharacter)
        );
    }

    #[test]
    fn an_escape_character_is_refused() {
        assert_eq!(
            check_typeable(Path::new("/h/a\u{1b}b")),
            Err(Refusal::ControlCharacter)
        );
    }

    #[test]
    fn a_path_that_is_not_utf8_is_refused() {
        let path = PathBuf::from(OsStr::from_bytes(b"/h/\xff\xfe"));
        assert_eq!(check_typeable(&path), Err(Refusal::NotUtf8));
    }

    #[test]
    fn a_path_over_the_limit_is_refused_and_one_at_the_limit_is_not() {
        let at_limit = format!("/{}", "x".repeat(MAX_TYPED_PATH - 1));
        assert_eq!(check_typeable(Path::new(&at_limit)), Ok(()));
        let over = format!("{at_limit}x");
        assert_eq!(check_typeable(Path::new(&over)), Err(Refusal::TooLong));
    }
}
