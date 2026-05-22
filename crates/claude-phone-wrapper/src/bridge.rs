use std::pin::Pin;

use futures::{SinkExt, Stream};
use tokio_tungstenite::tungstenite::Message;

use claude_phone_shared::protocol::{ControlMessage, Resize};

use crate::gateway_client::GatewayClient;
use crate::pty::PtySession;

/// A frame moving in either direction between PTY and the gateway WS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeFrame {
    Binary(Vec<u8>),
    Text(String),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close,
}

/// Source of frames coming from the gateway. Implemented for the real WS
/// stream and for `mpsc::Receiver<BridgeFrame>` in tests.
pub trait BridgeStream: Send + Unpin {
    fn poll_next_frame(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<BridgeFrame>>;
}

/// Sink for frames going back to the gateway.
#[async_trait::async_trait]
pub trait BridgeSink: Send + Unpin {
    async fn send_frame(&mut self, frame: BridgeFrame) -> anyhow::Result<()>;
}

/// PTY side abstraction.
#[async_trait::async_trait]
pub trait BridgePty: Send + Unpin {
    async fn read_chunk(&mut self) -> Option<Vec<u8>>;
    async fn write_chunk(&mut self, data: &[u8]) -> anyhow::Result<()>;
    fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()>;
}

/// Generic bridge loop. Returns when either side closes.
pub async fn run<S, K, P>(mut stream: S, mut sink: K, mut pty: P) -> anyhow::Result<()>
where
    S: BridgeStream,
    K: BridgeSink,
    P: BridgePty,
{
    loop {
        tokio::select! {
            chunk = pty.read_chunk() => {
                let Some(bytes) = chunk else { break };
                if sink.send_frame(BridgeFrame::Binary(bytes)).await.is_err() {
                    break;
                }
            }
            ws_msg = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next_frame(cx)) => {
                let Some(frame) = ws_msg else { break };
                match frame {
                    BridgeFrame::Binary(b) => {
                        let _ = pty.write_chunk(&b).await;
                    }
                    BridgeFrame::Text(t) => {
                        if let Ok(ControlMessage::Resize(Resize { cols, rows })) =
                            serde_json::from_str(&t)
                        {
                            let _ = pty.resize(cols, rows);
                        }
                    }
                    BridgeFrame::Ping(p) => {
                        let _ = sink.send_frame(BridgeFrame::Pong(p)).await;
                    }
                    BridgeFrame::Pong(_) => {}
                    BridgeFrame::Close => break,
                }
            }
        }
    }
    Ok(())
}

// ===== Real adapters =====

type WsTcpStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Wraps the real WS stream half of `GatewayClient` into `BridgeStream`.
pub struct GatewayStreamAdapter {
    inner: futures::stream::SplitStream<WsTcpStream>,
}

impl BridgeStream for GatewayStreamAdapter {
    fn poll_next_frame(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<BridgeFrame>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_next(cx) {
            std::task::Poll::Pending => std::task::Poll::Pending,
            std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
            std::task::Poll::Ready(Some(Err(_))) => std::task::Poll::Ready(None),
            std::task::Poll::Ready(Some(Ok(msg))) => {
                let mapped = match msg {
                    Message::Binary(b) => Some(BridgeFrame::Binary(b)),
                    Message::Text(t) => Some(BridgeFrame::Text(t)),
                    Message::Ping(p) => Some(BridgeFrame::Ping(p)),
                    Message::Pong(p) => Some(BridgeFrame::Pong(p)),
                    Message::Close(_) => Some(BridgeFrame::Close),
                    Message::Frame(_) => None,
                };
                match mapped {
                    Some(f) => std::task::Poll::Ready(Some(f)),
                    None => std::task::Poll::Pending,
                }
            }
        }
    }
}

/// Wraps the real WS sink half of `GatewayClient`.
pub struct GatewaySinkAdapter {
    inner: futures::stream::SplitSink<WsTcpStream, Message>,
}

#[async_trait::async_trait]
impl BridgeSink for GatewaySinkAdapter {
    async fn send_frame(&mut self, frame: BridgeFrame) -> anyhow::Result<()> {
        let msg = match frame {
            BridgeFrame::Binary(b) => Message::Binary(b),
            BridgeFrame::Text(t) => Message::Text(t),
            BridgeFrame::Ping(p) => Message::Ping(p),
            BridgeFrame::Pong(p) => Message::Pong(p),
            BridgeFrame::Close => Message::Close(None),
        };
        self.inner.send(msg).await?;
        Ok(())
    }
}

/// Wraps a locked `PtySession` guard.
pub struct PtyGuardAdapter {
    pub guard: tokio::sync::OwnedMutexGuard<PtySession>,
}

#[async_trait::async_trait]
impl BridgePty for PtyGuardAdapter {
    async fn read_chunk(&mut self) -> Option<Vec<u8>> {
        self.guard.read().await
    }
    async fn write_chunk(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.guard.write_all(data).await
    }
    fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.guard.resize(cols, rows)
    }
}

/// Backwards-compatible entry point used by `main.rs`.
pub async fn run_via_locked(
    client: GatewayClient,
    pty_guard: tokio::sync::OwnedMutexGuard<PtySession>,
) -> anyhow::Result<()> {
    let stream = GatewayStreamAdapter {
        inner: client.stream,
    };
    let sink = GatewaySinkAdapter { inner: client.sink };
    let pty = PtyGuardAdapter { guard: pty_guard };
    run(stream, sink, pty).await
}
