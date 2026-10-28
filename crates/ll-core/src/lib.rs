//! ll-core: shared vocabulary for Ledgerlens. Pure data and pure functions; no I/O, no async.
#![forbid(unsafe_code)]

#[macro_use]
mod macros;

pub mod clock;
pub mod config;
pub mod currency;
pub mod date;
pub mod document;
pub mod error;
pub mod fields;
pub mod geometry;
pub mod ids;
pub mod money;
pub mod ports;
pub mod prelude;
pub mod record;
pub mod text;

pub use prelude::*;
