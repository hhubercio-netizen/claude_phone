use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use claude_phone_shared::protocol::{ControlMessage, Resize};

use crate::gateway_client::GatewayClient;
use crate::pty::PtySession;

/// Bridge PTY ↔ Gateway WS in both directions.
pub async fn run_via_locked(
    client: GatewayClient,
    mut pty_guard: tokio::sync::OwnedMutexGuard<PtySession>,
) -> anyhow::Result<()> {
    let GatewayClient {
        mut sink,
        mut stream,
        ..
    } = client;

    loop {
        tokio::select! {
            chunk = pty_guard.read() => {
                let Some(bytes) = chunk else { break };
                if sink.send(Message::Binary(bytes)).await.is_err() {
                    break;
                }
            }
            ws_msg = stream.next() => {
                let Some(Ok(msg)) = ws_msg else { break };
                match msg {
                    Message::Binary(b) => {
                        let _ = pty_guard.write_all(&b).await;
                    }
                    Message::Text(t) => {
                        if let Ok(ControlMessage::Resize(Resize { cols, rows })) =
                            serde_json::from_str(&t)
                        {
                            let _ = pty_guard.resize(cols, rows);
                        }
                    }
                    Message::Close(_) => break,
                    Message::Ping(p) => {
                        let _ = sink.send(Message::Pong(p)).await;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
