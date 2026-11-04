//! Keystrokes, through the `wtype` command.

use std::process::Command;

use ofolder_core::system::{Keys, SystemError, Typist};

#[derive(Debug, Default, Clone, Copy)]
pub struct Wtype;

impl Typist for Wtype {
    fn send(&self, keys: &Keys) -> Result<(), SystemError> {
        let args: Vec<&str> = match keys {
            Keys::CtrlL => vec!["-M", "ctrl", "l", "-m", "ctrl"],
            // `--` so a path that starts with a dash is text, not an option.
            Keys::Text(text) => vec!["--", text.as_str()],
            Keys::Tab => vec!["-k", "Tab"],
            Keys::Return => vec!["-k", "Return"],
            Keys::Escape => vec!["-k", "Escape"],
        };
        // wtype decodes its arguments with the process locale, and fails ("Failed to deencode
        // input argv") on any non-ASCII character when there is none: a portal started by D-Bus
        // may have none. C.UTF-8 is always present and decodes every folder name.
        let status = Command::new("wtype")
            .env("LC_ALL", "C.UTF-8")
            .args(&args)
            .status()
            .map_err(|e| SystemError(format!("wtype: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(SystemError(format!("wtype exited with {status}")))
        }
    }
}
