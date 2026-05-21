//! Shared types used by claude-phone wrapper, gateway, and pair helper.

pub mod protocol;
mod token;

pub use token::{ApiKey, SessionToken, TokenError};
