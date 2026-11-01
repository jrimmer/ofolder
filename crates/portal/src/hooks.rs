//! What the rest of ofolder can do to a request as it passes through.

use std::collections::HashMap;

use zbus::zvariant::OwnedValue;

/// The `a{sv}` dictionary the portal API uses for options and results.
pub type Options = HashMap<String, OwnedValue>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    OpenFile,
    SaveFile,
    SaveFiles,
}

impl Method {
    pub fn dbus_name(self) -> &'static str {
        match self {
            Self::OpenFile => "OpenFile",
            Self::SaveFile => "SaveFile",
            Self::SaveFiles => "SaveFiles",
        }
    }
}

/// One request, as the frontend sent it.
#[derive(Debug, Clone)]
pub struct Call {
    pub method: Method,
    /// The object path of the request, which the frontend uses to cancel it.
    pub handle: String,
    pub app_id: String,
    pub parent_window: String,
    pub title: String,
}

/// The portal API's response codes.
pub mod response {
    pub const SUCCESS: u32 = 0;
    pub const CANCELLED: u32 = 1;
    /// Anything else, including the real backend being unreachable.
    pub const OTHER: u32 = 2;
}

pub trait Hooks: Send + Sync + 'static {
    /// Called before the request is forwarded. It may rewrite `options`,
    /// for example to start the dialog in a different folder. The returned
    /// guard is dropped when the dialog is done, which is where a strip
    /// shown for this dialog is closed.
    fn begin(&self, _call: &Call, _options: &mut Options) -> Box<dyn Send> {
        Box::new(())
    }

    /// Called with the real backend's answer, before it is passed on.
    fn end(&self, _call: &Call, _response: u32, _results: &Options) {}
}

/// Forwards everything unchanged.
#[derive(Debug, Default, Clone, Copy)]
pub struct PassThrough;

impl Hooks for PassThrough {}
