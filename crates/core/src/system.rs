//! The two outside systems an adapter drives, as traits, so the rules about
//! when it is safe to act can be tested with fakes instead of a desktop.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::hypr::{Client, Monitor};
use crate::keybind::Bind;
use crate::shortcuts::Combo;
use crate::wake::Wake;

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

    /// Starts telling `wake` whenever windows change (one opens, closes, moves between
    /// workspaces, takes focus, floats). The flag returned is true while that is working. A
    /// compositor that cannot say returns a flag that stays false, and the caller keeps polling.
    /// Events only hurry the caller up: it must still look now and then, since not every change
    /// (a window dragged to a new place) is announced.
    fn watch_events(&self, _wake: Arc<Wake>) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

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

    /// Where the pointer is, in layout pixels.
    fn cursor(&self) -> Result<(i32, i32), SystemError> {
        Err(unsupported("read the pointer position"))
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

    /// Every key binding the compositor holds now.
    fn binds(&self) -> Result<Vec<Bind>, SystemError> {
        Err(unsupported("list key bindings"))
    }

    /// Binds `combo` to run `command_line`, described as `title`.
    fn bind_key(
        &self,
        _combo: &Combo,
        _command_line: &str,
        _title: &str,
    ) -> Result<(), SystemError> {
        Err(unsupported("bind a key"))
    }

    /// Removes the bindings of `combo`. One that is not bound is not an error.
    fn unbind_key(&self, _combo: &Combo) -> Result<(), SystemError> {
        Err(unsupported("unbind a key"))
    }
}

/// What can be typed into whichever window has focus. The adapter, not the
/// typist, is responsible for checking that the right window has focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keys {
    CtrlL,
    Tab,
    Text(String),
    Return,
    Escape,
}

pub trait Typist: Send + Sync {
    fn send(&self, keys: &Keys) -> Result<(), SystemError>;
}
