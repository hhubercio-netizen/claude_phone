// TM-CODE.2: no `unsafe` in this crate. Compile-time gate against any
// future contributor adding `unsafe` blocks here.
#![deny(unsafe_code)]

pub mod auth;
pub mod config;
pub mod error;
pub mod http;
pub mod logging;
pub mod routes;
pub mod session;
