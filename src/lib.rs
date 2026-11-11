//! Headless double-entry accounting library.
//!
//! The model, rules and reports are specified in `docs/SPEC.md`. Callers in
//! other languages use the JSON API in [`api`] through the C ABI in [`ffi`].

pub mod api;
pub mod book;
pub mod ffi;
pub mod model;
pub mod reports;
pub mod time;

/// Crate version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
