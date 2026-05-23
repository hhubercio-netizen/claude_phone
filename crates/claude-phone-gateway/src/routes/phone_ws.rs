use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};

use claude_phone_shared::protocol::{
    ControlMessage, ErrorCode, ErrorMessage, PeerStatus, ServerHello,
};
use claude_phone_shared::SessionToken;

use crate::rate_limit::{
    ConnRateLimiter, PHONE_TO_GW_MSG_PER_SEC, PONG_DEADLINE, SINK_SEND_TIMEOUT,
};
use crate::session::{Frame, SessionRegistry};

#[derive(Clone)]
pub struct PhoneWsState {
    pub registry: Arc<SessionRegistry>,
    /// When `Some`, browser-initiated WSes whose `Origin` header doesn't
    /// equal this value are rejected with 403. When `None`, no enforcement.
    pub public_origin: Option<String>,
}

/// Hard cap on a single WebSocket message. PTY stdout chunks are 8KB; phone
/// keystrokes are tiny. 64KB is way above what either side needs and well
/// below what an attacker could use to OOM the gateway.
const MAX_WS_MESSAGE_BYTES: usize = 64 * 1024;

pub async fn handler(
    ws: WebSocketUpgrade,
    Path(token_str): Path<String>,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<PhoneWsState>,
) -> Response {
    // Strict equality on token length — anything else is malformed and we
    // refuse to even allocate the upgrade. The previous 8..=64 band let
    // off-shape strings reach SessionToken::parse() unnecessarily.
    if token_str.len() != SessionToken::LEN {
        return StatusCode::BAD_REQUEST.into_response();
    }

    // Defense-in-depth Origin check. Only fires when the deployer set
    // `public_origin` in gateway.toml AND the client sent an `Origin`
    // header (browsers always do; non-browser clients may not).
    if let Some(expected) = state.public_origin.as_deref() {
        if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            if origin != expected {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    ws.max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, state, token_str, peer))
}

async fn handle_socket(
    mut socket: WebSocket,
    state: PhoneWsState,
    token_str: String,
    peer: SocketAddr,
) {
    let token = match SessionToken::parse(&token_str) {
        Ok(t) => t,
        Err(_) => {
            send_error(
                &mut socket,
                ErrorCode::InvalidToken,
                "bad token format".into(),
            )
            .await;
            return;
        }
    };

    let handle = match state.registry.attach_phone(&token).await {
        Ok(h) => h,
        Err(_) => {
            send_error(
                &mut socket,
                ErrorCode::InvalidToken,
                "no such session".into(),
            )
            .await;
            return;
        }
    };

    let session_id = handle.session.id.clone();
    let server_hello = ControlMessage::ServerHello(ServerHello {
        session_id: session_id.clone(),
        peer_connected: true,
    });
    // TM-CODE.3: ServerHello is a derive(Serialize) struct — infallible.
    if socket
        .send(Message::Text(
            serde_json::to_string(&server_hello).expect("ServerHello serializes (static struct)"),
        ))
        .await
        .is_err()
    {
        return;
    }

    let peer_up = ControlMessage::PeerStatus(PeerStatus { connected: true });
    // TM-CODE.3: PeerStatus is a derive(Serialize) struct with a single bool.
    let _ = handle
        .session
        .to_wrapper
        .send(Frame::Text(
            serde_json::to_string(&peer_up).expect("PeerStatus serializes (static struct)"),
        ))
        .await;

    tracing::info!(session_id = %session_id, "phone attached");

    let (mut sink, mut stream) = socket.split();
    let mut rx_from_wrapper = handle.rx;
    let to_wrapper = handle.session.to_wrapper.clone();
    let cancel = handle.session.cancel.clone();

    // TM-RATE.7 — shared "millis since this socket opened" reference for the
    // no-pong watchdog. See wrapper_ws for the full explanation; the same
    // shape is needed here so a phone with a dead reverse-channel doesn't
    // hold an FD indefinitely.
    let socket_start = Instant::now();
    let last_pong_ms = Arc::new(AtomicU64::new(0));
    let last_pong_outgoing = last_pong_ms.clone();

    let cancel_outgoing = cancel.clone();
    let cancel_for_flood = cancel.clone();
    let outgoing_task = tokio::spawn(async move {
        // TM-RATE.3 — per-connection sliding-window cap on phone→wrapper
        // traffic. Direction is keystrokes/touches, so PHONE_TO_GW_MSG_PER_SEC
        // (100/s) is generous for a human while still ruling out automated
        // floods. Exceeding the cap cancels the whole session, not just this
        // task — a flooding phone has no business holding the session open.
        let mut conn_rate = ConnRateLimiter::new(PHONE_TO_GW_MSG_PER_SEC);
        loop {
            let cancelled = cancel_outgoing.cancelled();
            tokio::pin!(cancelled);
            tokio::select! {
                biased;
                _ = &mut cancelled => break,
                msg = stream.next() => {
                    let Some(Ok(msg)) = msg else { break };
                    let frame = match msg {
                        Message::Binary(b) => Frame::Binary(b),
                        Message::Text(t) => Frame::Text(t),
                        Message::Close(_) => break,
                        // TM-RATE.7: stamp last_pong on every received Pong.
                        Message::Pong(_) => {
                            last_pong_outgoing.store(
                                socket_start.elapsed().as_millis() as u64,
                                Ordering::Relaxed,
                            );
                            continue;
                        }
                        _ => continue,
                    };
                    if !conn_rate.check(Instant::now()) {
                        tracing::warn!(
                            peer = %peer,
                            cap = PHONE_TO_GW_MSG_PER_SEC,
                            "TM-RATE.3: phone exceeded per-connection msg rate, closing"
                        );
                        cancel_for_flood.cancel();
                        break;
                    }
                    if to_wrapper.send(frame).await.is_err() { break; }
                }
            }
        }
    });

    let cancel_incoming = cancel.clone();
    let last_pong_incoming = last_pong_ms.clone();
    let incoming_task = tokio::spawn(async move {
        // 30s server-initiated Ping keeps the phone's WebSocket alive across
        // NAT and Cloudflare's idle-connection drop (~100s). The reply Pong
        // (handled in outgoing_task) stamps `last_pong_incoming`; this task
        // checks the age on every tick (TM-RATE.7).
        let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(30));
        keepalive.tick().await; // skip immediate tick

        loop {
            let cancelled = cancel_incoming.cancelled();
            tokio::pin!(cancelled);
            tokio::select! {
                biased;
                _ = &mut cancelled => break,
                frame = rx_from_wrapper.recv() => {
                    let Some(frame) = frame else { break };
                    let msg = match frame {
                        Frame::Binary(b) => Message::Binary(b),
                        Frame::Text(t) => Message::Text(t),
                    };
                    // TM-RATE.6: same slow-write defense as wrapper_ws.
                    // A phone holding its read buffer indefinitely would
                    // otherwise pin the writer forever once the bounded
                    // channel filled up. On stall we cancel the session
                    // so outgoing_task tears down too.
                    match tokio::time::timeout(SINK_SEND_TIMEOUT, sink.send(msg)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) => {
                            cancel_incoming.cancel();
                            break;
                        }
                        Err(_) => {
                            tracing::warn!(
                                peer = %peer,
                                timeout_secs = SINK_SEND_TIMEOUT.as_secs(),
                                "TM-RATE.6: phone sink send stalled, closing"
                            );
                            cancel_incoming.cancel();
                            break;
                        }
                    }
                }
                _ = keepalive.tick() => {
                    // TM-RATE.7: check elapsed since last Pong BEFORE sending
                    // the next Ping. See wrapper_ws for the full reasoning.
                    let now_ms = socket_start.elapsed().as_millis() as u64;
                    let last_ms = last_pong_incoming.load(Ordering::Relaxed);
                    let age_ms = now_ms.saturating_sub(last_ms);
                    if age_ms > PONG_DEADLINE.as_millis() as u64 {
                        tracing::warn!(
                            peer = %peer,
                            age_ms,
                            deadline_ms = PONG_DEADLINE.as_millis() as u64,
                            "TM-RATE.7: phone no-pong deadline exceeded, closing"
                        );
                        cancel_incoming.cancel();
                        break;
                    }
                    // TM-RATE.6: bounded keepalive write — see wrapper_ws.
                    match tokio::time::timeout(
                        SINK_SEND_TIMEOUT,
                        sink.send(Message::Ping(Vec::new())),
                    )
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) | Err(_) => {
                            cancel_incoming.cancel();
                            break;
                        }
                    }
                }
            }
        }
    });

    let _ = tokio::join!(outgoing_task, incoming_task);

    {
        let mut slot = handle.session.to_phone.lock().await;
        slot.detach();
    }
    // Reset the idle clock — sweeper measures elapsed time from THIS instant
    // for sessions that are currently phone-less.
    handle.session.touch_phone().await;
    let peer_down = ControlMessage::PeerStatus(PeerStatus { connected: false });
    // TM-CODE.3: PeerStatus is a derive(Serialize) struct with a single bool.
    let _ = handle
        .session
        .to_wrapper
        .send(Frame::Text(
            serde_json::to_string(&peer_down).expect("PeerStatus serializes (static struct)"),
        ))
        .await;
    tracing::info!(session_id = %session_id, "phone detached");
}

async fn send_error(socket: &mut WebSocket, code: ErrorCode, message: String) {
    let err = ControlMessage::Error(ErrorMessage { code, message });
    // TM-CODE.3: ErrorMessage is a derive(Serialize) struct — infallible.
    let _ = socket
        .send(Message::Text(
            serde_json::to_string(&err).expect("ErrorMessage serializes (static struct)"),
        ))
        .await;
}
