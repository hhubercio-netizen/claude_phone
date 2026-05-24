//! TM-INPUT.8 — forward-looking validation of the wrapper `--claude-bin`
//! argument. Each rejection case asserts the exact `CliError` variant so
//! a future refactor that loosens the validation (e.g. accidentally
//! short-circuits on `\t`) breaks the test rather than the user's trust
//! boundary at process-spawn time.

use std::path::PathBuf;

use claude_phone_wrapper::cli::{Cli, CliError};

/// Construct a `Cli` directly, bypassing clap's argv parser. The validate
/// path is the same logic clap eventually drives via `parse_validated`.
fn cli_with(claude_bin: &str) -> Cli {
    Cli {
        config: None,
        claude_bin: claude_bin.to_string(),
        claude_args: Vec::new(),
    }
}

#[test]
fn claude_bin_empty_rejects() {
    let cli = cli_with("");
    assert_eq!(cli.validate(), Err(CliError::ClaudeBinEmpty));
}

#[test]
fn claude_bin_with_nul_rejects() {
    let cli = cli_with("claude\0");
    assert_eq!(cli.validate(), Err(CliError::ClaudeBinControl(6)));
}

#[test]
fn claude_bin_with_newline_rejects() {
    let cli = cli_with("claude\n");
    assert!(matches!(cli.validate(), Err(CliError::ClaudeBinControl(_))));
}

#[test]
fn claude_bin_with_tab_rejects() {
    let cli = cli_with("claude\t");
    assert!(matches!(cli.validate(), Err(CliError::ClaudeBinControl(_))));
}

#[test]
fn claude_bin_with_del_rejects() {
    let cli = cli_with("claude\x7f");
    assert!(matches!(cli.validate(), Err(CliError::ClaudeBinControl(_))));
}

#[test]
fn claude_bin_accepts_normal_path() {
    let cli = cli_with("/usr/local/bin/claude");
    assert!(cli.validate().is_ok());
}

#[test]
fn claude_bin_accepts_relative_dot_dot_path() {
    // Documents the deliberate decision NOT to reject `..` in --claude-bin.
    // This is a user-typed CLI arg in the user's own trust domain; rejecting
    // `..` would break legitimate `./local-build/claude` workflows. A future
    // drift toward "block `..` in all path args" trips this test.
    let cli = cli_with("../bin/claude");
    assert!(cli.validate().is_ok());
}

#[test]
fn config_field_is_orthogonal_to_claude_bin() {
    // Sanity: `validate` does NOT touch `config` — that field has its own
    // path-existence check at load time. A future refactor that adds
    // cross-field validation must update this test deliberately.
    let cli = Cli {
        config: Some(PathBuf::from("/nonexistent")),
        claude_bin: "claude".to_string(),
        claude_args: Vec::new(),
    };
    assert!(cli.validate().is_ok());
}
