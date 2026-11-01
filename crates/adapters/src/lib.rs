//! The real adapters: the code that talks to Hyprland, the keyboard, `/proc`
//! and each dialog. The rules about when it is safe to act are in
//! `ofolder-core`; this crate supplies the hands.

pub mod a11y;
pub mod atspi_bus;
pub mod flea;
pub mod gtk;
pub mod guard;
pub mod hyprctl;
pub mod procfs;
pub mod resident;
pub mod strata;
pub mod wtype;
