//! Shared APP-AGG-001 phase 1 contract.
//!
//! The desktop shell and the headless checker both call this crate.
//! Nothing here completes a real provider login or marks a model verified
//! from a catalog response.

#![forbid(unsafe_code)]

pub mod check;
pub mod codex;
pub mod corecfg;
pub mod http;
pub mod oauth;
pub mod pin;
pub mod platform;
pub mod redact;
pub mod registry;
pub mod supervisor;

pub use check::CoreCheckReport;
pub use platform::assess_platform;
pub use registry::ModelRegistry;
