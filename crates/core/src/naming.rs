//! Names for folders the strip creates.

/// How many "New folder N" names are tried before giving up. A folder with
/// this many is not a place the strip should be adding another to.
const MAX_ATTEMPTS: u32 = 1000;

/// The first of "New folder", "New folder 2", "New folder 3", ... that is not
/// taken, so a second click never collides with the first.
pub fn next_folder_name(taken: impl Fn(&str) -> bool) -> Option<String> {
    if !taken("New folder") {
        return Some("New folder".to_owned());
    }
    (2..=MAX_ATTEMPTS)
        .map(|n| format!("New folder {n}"))
        .find(|name| !taken(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_folder_is_called_new_folder() {
        assert_eq!(next_folder_name(|_| false).as_deref(), Some("New folder"));
    }

    #[test]
    fn a_taken_name_gets_the_next_number() {
        let taken = |n: &str| n == "New folder";
        assert_eq!(next_folder_name(taken).as_deref(), Some("New folder 2"));
    }

    #[test]
    fn gaps_are_filled_in_order() {
        let taken = |n: &str| n == "New folder" || n == "New folder 2" || n == "New folder 4";
        assert_eq!(next_folder_name(taken).as_deref(), Some("New folder 3"));
    }

    #[test]
    fn it_gives_up_rather_than_looping_forever() {
        assert_eq!(next_folder_name(|_| true), None);
    }
}
