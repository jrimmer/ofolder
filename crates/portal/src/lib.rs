//! ofolder's portal backend: a file-chooser proxy.
//!
//! The portal frontend hands every Open and Save request to a backend over
//! D-Bus. This backend takes the request, lets [`hooks::Hooks`] look at it and
//! rewrite it, forwards it to the real backend (Flea, GTK, Strata), and hands
//! the answer back unchanged. While the real dialog is open, the hooks are
//! where the strip is shown.
//!
//! Registering this backend with the user's portal configuration is the
//! user's decision and is not done here; see `docs/adapters.md`.

pub mod hooks;
pub mod preseed;
mod service;

pub use service::{PORTAL_PATH, serve, serve_session};

/// Runs a future to completion on the thread, for the binary that serves the portal.
pub use zbus::block_on;

#[derive(Debug, thiserror::Error)]
pub enum PortalError {
    #[error("D-Bus: {0}")]
    Bus(#[from] zbus::Error),
}
