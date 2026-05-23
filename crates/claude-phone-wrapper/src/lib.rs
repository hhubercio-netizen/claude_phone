// TM-CODE.2: deny new `unsafe` workspace-wide. The single legitimate
// `unsafe` surface in this crate (`tty.rs`) opts back in via its own
// `#![allow(unsafe_code)]` and inline `// SAFETY:` justifications.
#![deny(unsafe_code)]

pub mod bridge;
pub mod cli;
pub mod config;
pub mod gateway_client;
pub mod local_term;
pub mod pty;
pub mod qr;
pub mod rpc;
pub mod session;
pub mod tty;
