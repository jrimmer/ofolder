//! The two outside systems an adapter drives, as traits, so the rules about
//! when it is safe to act can be tested with fakes instead of a desktop.

use crate::hypr::{Client, Monitor};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SystemError(pub String);

fn unsupported(what: &str) -> SystemError {
    SystemError(format!("this compositor driver cannot {what}"))
}

/// The compositor: what windows exist, which one is active, and moving focus.
pub trait Compositor: Send + Sync {
    fn clients(&self) -> Result<Vec<Client>, SystemError>;
    fn monitors(&self) -> Result<Vec<Monitor>, SystemError>;
    /// The address of the window that has keyboard focus, if any.
    fn active_window(&self) -> Result<Option<String>, SystemError>;
    fn focus(&self, address: &str) -> Result<(), SystemError>;

    /// Makes the window float at `size` and `at`. For a dialog that opened tiled.
    fn float_window(
        &self,
        _address: &str,
        _size: (i32, i32),
        _at: (i32, i32),
    ) -> Result<(), SystemError> {
        Err(unsupported("float a window"))
    }

    fn resize_window(&self, _address: &str, _size: (i32, i32)) -> Result<(), SystemError> {
        Err(unsupported("resize a window"))
    }

    fn move_window(&self, _address: &str, _at: (i32, i32)) -> Result<(), SystemError> {
        Err(unsupported("move a window"))
    }

    /// Sends the window to a workspace without following it there.
    fn send_to_workspace(&self, _address: &str, _workspace: &str) -> Result<(), SystemError> {
        Err(unsupported("send a window to a workspace"))
    }

    /// Installs a named rule so every window of `class` that opens from now
    /// on floats at `size` and `at`, instead of tiling first and jumping.
    fn install_float_rule(
        &self,
        _name: &str,
        _class: &str,
        _size: (i32, i32),
        _at: (i32, i32),
        _take_focus: bool,
    ) -> Result<(), SystemError> {
        Err(unsupported("install a window rule"))
    }

    /// Like [`Compositor::install_float_rule`], for a window that must
    /// never take the keyboard: a tooltip.
    fn install_tip_rule(
        &self,
        _name: &str,
        _class: &str,
        _size: (i32, i32),
        _at: (i32, i32),
    ) -> Result<(), SystemError> {
        Err(unsupported("install a window rule"))
    }

    /// Switches a rule off. A rule that does not exist is not an error.
    fn remove_rule(&self, _name: &str) -> Result<(), SystemError> {
        Err(unsupported("remove a window rule"))
    }
}

/// What can be typed into whichever window has focus. The adapter, not the
/// typist, is responsible for checking that the right window has focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keys {
    CtrlL,
    Text(String),
    Return,
    Escape,
}

pub trait Typist: Send + Sync {
    fn send(&self, keys: &Keys) -> Result<(), SystemError>;
}
