//! bukio-cli — agent-first, local-first double-entry bookkeeping for Dutch SMEs.
//!
//! Rust reimplementation of the bukio-cli Node.js application. The SQLite
//! schema (migrations/) is shared verbatim, so database files are 100%
//! compatible with the Node version. Money is integer cents everywhere.

pub mod audit;
pub mod core;
pub mod error;
pub mod fx;
pub mod report;
pub mod vat;

pub use error::{AppError, Result};
