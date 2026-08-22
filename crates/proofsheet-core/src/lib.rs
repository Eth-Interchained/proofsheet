//! # proofsheet-core
//!
//! Local-first browser control for two jobs that turn out to be the same job:
//! producing store-ready screenshots at exact dimensions, and running
//! deterministic tests against a real browser.
//!
//! Both need the same three things — a driver, a way to make the page stop
//! being nondeterministic, and a record of what happened that can be checked
//! later. So they share one core.
//!
//! ## Design commitments
//!
//! - **The model never authors code or selectors.** Actions are typed; the
//!   runtime composes them. A free-text field in a tool schema is
//!   grammar-constrained to *any* text, so the field is removed rather than
//!   validated after the fact.
//! - **Output pixels are the source of truth.** Stores specify pixels; the
//!   viewport is derived. See [`device`].
//! - **Determinism is injected, not hoped for.** See [`determinism`].
//! - **Nothing is claimed without a negative control.** A check that cannot
//!   fail on correct input is not evidence.
//!
//! ## Dependencies
//!
//! Deliberately small. No async runtime, no browser automation framework: a
//! tool people install should not drag a dependency tree behind it.

#![deny(rust_2018_idioms)]
#![warn(missing_debug_implementations)]

pub mod capture;
pub mod cdp;
pub mod determinism;
pub mod device;
pub mod error;
pub mod png;
mod ws;

pub use capture::{capture, Capture, CaptureRequest, Stability};
pub use cdp::{find_browser, Browser, LaunchOptions};
pub use determinism::Determinism;
pub use device::{builtin, by_id, for_store, Device, Requirement, Store};
pub use error::{Error, Result};

/// The crate version, surfaced so receipts can record which build produced
/// them.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_populated() {
        assert!(!super::VERSION.is_empty());
    }
}
