use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about = "Wrap `claude` with phone bridging support")]
pub struct Cli {
    /// Path to wrapper config TOML.
    #[arg(short, long, env = "CLAUDE_PHONE_WRAPPER_CONFIG")]
    pub config: Option<PathBuf>,

    /// Override the `claude` binary path.
    #[arg(long, env = "CLAUDE_PHONE_CLAUDE_BIN", default_value = "claude")]
    pub claude_bin: String,

    /// All remaining args are forwarded to `claude` as-is.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub claude_args: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CliError {
    #[error("--claude-bin is empty; refusing to spawn nothing")]
    ClaudeBinEmpty,
    #[error("--claude-bin contains a control character at byte {0}; refusing to spawn")]
    ClaudeBinControl(usize),
}

impl Cli {
    /// Parse and validate. Failures abort the process loudly — `claude_bin`
    /// is fed to a process spawn, and silently re-shaping the string could
    /// pivot an injected env var into a different binary.
    ///
    /// TM-INPUT.8: control characters in `--claude-bin` are rejected at
    /// startup. `portable-pty`'s `CommandBuilder` does NOT pass through a
    /// shell (`execve` directly on Unix, `CreateProcessW` on Windows), so
    /// meta-characters like `;` `|` `&` `$` are non-issues — they only land
    /// if the user explicitly typed them. The real concern is an
    /// environment-tampered `CLAUDE_PHONE_CLAUDE_BIN` containing embedded
    /// newlines (confuse log readers) or NUL bytes (truncate the spawn
    /// target on some platforms).
    pub fn parse_validated() -> Result<Self, CliError> {
        let cli = Self::parse();
        cli.validate()?;
        Ok(cli)
    }

    pub fn validate(&self) -> Result<(), CliError> {
        if self.claude_bin.is_empty() {
            return Err(CliError::ClaudeBinEmpty);
        }
        for (i, b) in self.claude_bin.bytes().enumerate() {
            // Reject all C0 control chars (0x00-0x1F) and DEL (0x7F).
            // Tab / newline / CR have no legitimate place in a binary path.
            if b < 0x20 || b == 0x7F {
                return Err(CliError::ClaudeBinControl(i));
            }
        }
        Ok(())
    }
}
