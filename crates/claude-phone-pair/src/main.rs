// TM-CODE.2: no `unsafe` allowed in this binary.
#![deny(unsafe_code)]

use clap::Parser;
use serde::Deserialize;

#[derive(Parser, Debug)]
#[command(version, about = "Trigger pairing in the running claude-phone wrapper")]
struct Cli {
    /// Override wrapper RPC URL (otherwise read from $CLAUDE_PHONE_RPC_URL).
    #[arg(long, env = "CLAUDE_PHONE_RPC_URL")]
    rpc_url: Option<String>,

    /// Ephemeral bearer token issued by the wrapper at startup and exported
    /// to the child PTY env as CLAUDE_PHONE_RPC_TOKEN. Without it the RPC
    /// server returns 401 — that's the defense against any process that
    /// happens to find the listening loopback port.
    #[arg(long, env = "CLAUDE_PHONE_RPC_TOKEN", hide_env_values = true)]
    rpc_token: Option<String>,

    /// Output as JSON (machine-readable).
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Deserialize)]
struct PairResponse {
    url: String,
    token: String,
    qr_ascii: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let rpc_url = cli.rpc_url.ok_or_else(|| {
        anyhow::anyhow!(
            "CLAUDE_PHONE_RPC_URL not set. Run `claude-phone` instead of `claude` to enable phone bridging."
        )
    })?;
    let rpc_token = cli.rpc_token.ok_or_else(|| {
        anyhow::anyhow!(
            "CLAUDE_PHONE_RPC_TOKEN not set. The wrapper exports this to its child env; if you're seeing this, you ran `claude-phone-pair` outside the claude PTY started by `claude-phone`."
        )
    })?;

    let resp: PairResponse = reqwest::Client::new()
        .post(format!("{rpc_url}/pair"))
        .bearer_auth(&rpc_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    if cli.json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "url": resp.url,
                "token": resp.token,
            }))?
        );
    } else {
        println!();
        println!("{}", resp.qr_ascii);
        println!();
        println!("Open on your phone:");
        println!("  {}", resp.url);
        println!();
        println!("This URL expires when this Claude session ends.");
    }
    Ok(())
}
