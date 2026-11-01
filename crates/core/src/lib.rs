//! The adapter contract and the pure logic around it.
//!
//! This crate has no UI toolkit, no D-Bus and no accessibility code. Everything
//! that talks to a real dialog sits behind [`adapter::Adapter`], so the logic
//! that decides what is safe to do can be tested without opening a window.

pub mod adapter;
pub mod geometry;
pub mod hypr;
pub mod naming;
pub mod pathsafe;
pub mod portal;
pub mod readers;
pub mod system;
