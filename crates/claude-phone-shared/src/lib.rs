//! Shared types used by claude-phone wrapper, gateway, and pair helper.

// TM-CODE.2: this crate handles secret types — `unsafe` must never appear
// here. The deny lint is a compile-time gate against contributors adding
// any `unsafe` block.
#![deny(unsafe_code)]

pub mod protocol;
mod token;

pub use token::{ApiKey, SessionToken, TokenError};
