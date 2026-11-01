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
            Keys::Return => vec!["-k", "Return"],
            Keys::Escape => vec!["-k", "Escape"],
        };
        let status = Command::new("wtype")
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
