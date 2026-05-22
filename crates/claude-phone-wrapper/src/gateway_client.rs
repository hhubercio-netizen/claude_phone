use futures::{stream::SplitSink, stream::SplitStream, SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use claude_phone_shared::protocol::{ControlMessage, WrapperHello};
use claude_phone_shared::{ApiKey, SessionToken};

pub struct GatewayClientConfig {
    pub url: String,
    pub api_key: ApiKey,
    pub token: SessionToken,
    pub cols: u16,
    pub rows: u16,
}

pub struct GatewayClient {
    pub sink: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    pub stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    session_id: String,
}

impl GatewayClient {
    pub async fn connect(config: GatewayClientConfig) -> anyhow::Result<Self> {
        let (ws, _resp) = connect_async(&config.url).await?;
        let (mut sink, mut stream) = ws.split();

        let hello = ControlMessage::WrapperHello(WrapperHello {
            api_key: config.api_key,
            token: config.token,
            cols: config.cols,
            rows: config.rows,
            claude_version: None,
        });
        sink.send(Message::Text(serde_json::to_string(&hello)?))
            .await?;

        let first = stream
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("no server hello"))??;
        let session_id = match first {
            Message::Text(t) => {
                let msg: ControlMessage = serde_json::from_str(&t)?;
                match msg {
                    ControlMessage::ServerHello(h) => h.session_id,
                    ControlMessage::Error(e) => {
                        anyhow::bail!("gateway error: {:?} {}", e.code, e.message)
                    }
                    other => anyhow::bail!("unexpected first message: {other:?}"),
                }
            }
            _ => anyhow::bail!("expected text frame"),
        };

        Ok(Self {
            sink,
            stream,
            session_id,
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}
