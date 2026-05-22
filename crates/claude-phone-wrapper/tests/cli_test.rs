use clap::Parser;
use claude_phone_wrapper::cli::Cli;

#[test]
fn parses_minimum_args() {
    let args = ["claude-phone"];
    let cli = Cli::try_parse_from(args).expect("default args parse");
    assert!(cli.config.is_none());
    assert_eq!(cli.claude_bin, "claude");
    assert!(cli.claude_args.is_empty());
}

#[test]
fn unknown_flags_are_forwarded_to_claude() {
    // The wrapper transparently forwards any unrecognized argument to
    // `claude` (trailing_var_arg + allow_hyphen_values). Pinning this so
    // an accidental `deny_unknown_args` change is caught.
    let args = ["claude-phone", "--definitely-not-a-real-flag"];
    let cli = Cli::try_parse_from(args).expect("trailing args parse");
    assert_eq!(cli.claude_args, vec!["--definitely-not-a-real-flag"]);
}

#[test]
fn forwards_trailing_claude_args() {
    let args = ["claude-phone", "--", "--model", "opus", "chat"];
    let cli = Cli::try_parse_from(args).expect("parses");
    assert_eq!(cli.claude_args, vec!["--model", "opus", "chat"]);
}

#[test]
fn override_claude_bin() {
    let args = ["claude-phone", "--claude-bin", "/usr/bin/claude"];
    let cli = Cli::try_parse_from(args).expect("parses");
    assert_eq!(cli.claude_bin, "/usr/bin/claude");
}
