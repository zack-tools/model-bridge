//! Shared model-bridge phase 1 contract.
//!
//! The desktop shell and the headless checker both call this crate.
//! Nothing here completes a real provider login or marks a model verified
//! from a catalog response.

#![forbid(unsafe_code)]

pub mod attribution;
pub mod check;
pub mod codex;
pub mod corecfg;
pub mod http;
pub mod instance;
pub mod loopback_callback;
pub mod oauth;
pub mod oauth_client;
pub mod pin;
pub mod platform;
pub mod redact;
pub mod registry;
pub mod response_evidence;
pub mod supervisor;
pub mod vscode;

pub use check::CoreCheckReport;
pub use platform::assess_platform;
pub use registry::ModelRegistry;
