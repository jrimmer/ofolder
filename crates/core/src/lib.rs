//! The adapter contract and the pure logic around it.
//!
//! This crate has no UI toolkit, no D-Bus and no accessibility code. Everything
//! that talks to a real dialog sits behind [`adapter::Adapter`], so the logic
//! that decides what is safe to do can be tested without opening a window.

pub mod adapter;
pub mod browse;
pub mod dialog_memory;
pub mod fuzzy;
pub mod geometry;
pub mod hypr;
pub mod identity;
pub mod keybind;
pub mod launch;
pub mod messages;
pub mod naming;
pub mod pathsafe;
pub mod places;
pub mod portal;
pub mod preview;
pub mod private_file;
pub mod rails;
pub mod readers;
pub mod recents;
pub mod search;
pub mod settings;
pub mod settings_write;
pub mod shortcuts;
pub mod system;
pub mod theme;
