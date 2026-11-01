//! Reading and rewriting the options a dialog starts with.
//!
//! The portal request already says which folder and file name the dialog
//! should start with, so changing where a dialog starts is a matter of
//! changing the request before it is forwarded. No dialog needs to be driven.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use zbus::zvariant::{OwnedValue, Value};

use crate::hooks::Options;

const CURRENT_FOLDER: &str = "current_folder";
const CURRENT_NAME: &str = "current_name";

/// The portal API sends folders as NUL-terminated byte strings (`ay`).
pub fn set_current_folder(options: &mut Options, folder: &Path) {
    let mut bytes = folder.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    if let Ok(value) = OwnedValue::try_from(Value::from(bytes)) {
        options.insert(CURRENT_FOLDER.to_owned(), value);
    }
}

pub fn current_folder(options: &Options) -> Option<PathBuf> {
    let value = options.get(CURRENT_FOLDER)?.try_clone().ok()?;
    let mut bytes = Vec::<u8>::try_from(value).ok()?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    Some(PathBuf::from(OsStr::from_bytes(&bytes).to_owned()))
}

pub fn current_name(options: &Options) -> Option<String> {
    let value = options.get(CURRENT_NAME)?.try_clone().ok()?;
    String::try_from(value).ok()
}

/// The extension of the suggested file name, lower-cased, without the dot.
pub fn suggested_extension(options: &Options) -> Option<String> {
    let name = current_name(options)?;
    let extension = Path::new(&name).extension()?.to_str()?;
    Some(extension.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::os::unix::ffi::OsStringExt;

    use super::*;

    fn options() -> Options {
        HashMap::new()
    }

    #[test]
    fn a_folder_set_on_the_request_reads_back_the_same() {
        let mut o = options();
        set_current_folder(&mut o, Path::new("/home/j/Work"));
        assert_eq!(current_folder(&o), Some(PathBuf::from("/home/j/Work")));
    }

    #[test]
    fn the_folder_is_sent_nul_terminated_as_the_portal_api_requires() {
        let mut o = options();
        set_current_folder(&mut o, Path::new("/a"));
        let bytes = Vec::<u8>::try_from(o[CURRENT_FOLDER].try_clone().expect("clone")).expect("ay");
        assert_eq!(bytes, b"/a\0");
    }

    #[test]
    fn a_folder_that_is_not_utf8_survives_the_round_trip() {
        let mut o = options();
        let odd = PathBuf::from(std::ffi::OsString::from_vec(b"/h/\xff".to_vec()));
        set_current_folder(&mut o, &odd);
        assert_eq!(current_folder(&o), Some(odd));
    }

    #[test]
    fn setting_a_folder_replaces_the_one_the_app_asked_for() {
        let mut o = options();
        set_current_folder(&mut o, Path::new("/first"));
        set_current_folder(&mut o, Path::new("/second"));
        assert_eq!(current_folder(&o), Some(PathBuf::from("/second")));
    }

    #[test]
    fn a_request_with_no_folder_reads_as_none() {
        assert_eq!(current_folder(&options()), None);
    }

    #[test]
    fn the_suggested_extension_is_lower_cased_and_needs_a_dot() {
        let mut o = options();
        let name = |s: &str| OwnedValue::try_from(Value::from(s)).expect("str value");
        o.insert(CURRENT_NAME.to_owned(), name("Photo.PNG"));
        assert_eq!(suggested_extension(&o).as_deref(), Some("png"));
        o.insert(CURRENT_NAME.to_owned(), name("README"));
        assert_eq!(suggested_extension(&o), None);
    }
}
