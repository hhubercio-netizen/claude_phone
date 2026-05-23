use std::pin::Pin;
use std::task::{Context, Poll};

use async_trait::async_trait;
use claude_phone_wrapper::bridge::{run, BridgeFrame, BridgePty, BridgeSink, BridgeStream};
use tokio::sync::{mpsc, oneshot};

struct FakeStream {
    rx: mpsc::UnboundedReceiver<BridgeFrame>,
}

impl BridgeStream for FakeStream {
    fn poll_next_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<BridgeFrame>> {
        let this = self.get_mut();
        this.rx.poll_recv(cx)
    }
}

struct FakeSink {
    tx: mpsc::UnboundedSender<BridgeFrame>,
}

#[async_trait]
impl BridgeSink for FakeSink {
    async fn send_frame(&mut self, frame: BridgeFrame) -> anyhow::Result<()> {
        self.tx.send(frame).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(())
    }
}

struct FakePty {
    reads: mpsc::UnboundedReceiver<Option<Vec<u8>>>,
    writes: mpsc::UnboundedSender<Vec<u8>>,
    last_resize: std::sync::Arc<std::sync::Mutex<Option<(u16, u16)>>>,
}

#[async_trait]
impl BridgePty for FakePty {
    async fn read_chunk(&mut self) -> Option<Vec<u8>> {
        self.reads.recv().await.flatten()
    }
    async fn write_chunk(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.writes.send(data.to_vec()).unwrap();
        Ok(())
    }
    fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        *self.last_resize.lock().unwrap() = Some((cols, rows));
        Ok(())
    }
}

struct Harness {
    stream_tx: mpsc::UnboundedSender<BridgeFrame>,
    sink_rx: mpsc::UnboundedReceiver<BridgeFrame>,
    pty_in_tx: mpsc::UnboundedSender<Option<Vec<u8>>>,
    pty_out_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    resize: std::sync::Arc<std::sync::Mutex<Option<(u16, u16)>>>,
    stream: FakeStream,
    sink: FakeSink,
    pty: FakePty,
    shutdown_tx: oneshot::Sender<()>,
    shutdown_rx: oneshot::Receiver<()>,
}

fn setup() -> Harness {
    let (stream_tx, stream_rx) = mpsc::unbounded_channel();
    let (sink_tx, sink_rx) = mpsc::unbounded_channel();
    let (pty_in_tx, pty_in_rx) = mpsc::unbounded_channel();
    let (pty_out_tx, pty_out_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let resize = std::sync::Arc::new(std::sync::Mutex::new(None));
    let stream = FakeStream { rx: stream_rx };
    let sink = FakeSink { tx: sink_tx };
    let pty = FakePty {
        reads: pty_in_rx,
        writes: pty_out_tx,
        last_resize: resize.clone(),
    };
    Harness {
        stream_tx,
        sink_rx,
        pty_in_tx,
        pty_out_rx,
        resize,
        stream,
        sink,
        pty,
        shutdown_tx,
        shutdown_rx,
    }
}

#[tokio::test]
async fn pty_bytes_forwarded_as_binary_frame() {
    let h = setup();
    h.pty_in_tx.send(Some(b"abc".to_vec())).unwrap();
    h.pty_in_tx.send(None).unwrap();

    let mut sink_rx = h.sink_rx;
    let handle = tokio::spawn(run(h.stream, h.sink, h.pty, h.shutdown_rx));
    let frame = sink_rx.recv().await.expect("sink got frame");
    assert_eq!(frame, BridgeFrame::Binary(b"abc".to_vec()));
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn ws_binary_forwarded_to_pty_write() {
    let h = setup();
    h.stream_tx
        .send(BridgeFrame::Binary(b"xyz".to_vec()))
        .unwrap();
    h.stream_tx.send(BridgeFrame::Close).unwrap();

    let mut pty_out_rx = h.pty_out_rx;
    let handle = tokio::spawn(run(h.stream, h.sink, h.pty, h.shutdown_rx));
    let written = pty_out_rx.recv().await.expect("pty got write");
    assert_eq!(written, b"xyz");
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn resize_text_dispatched_to_pty_resize() {
    let h = setup();
    let resize_json = r#"{"type":"resize","cols":100,"rows":40}"#.to_string();
    h.stream_tx.send(BridgeFrame::Text(resize_json)).unwrap();
    h.stream_tx.send(BridgeFrame::Close).unwrap();

    let resize = h.resize.clone();
    let handle = tokio::spawn(run(h.stream, h.sink, h.pty, h.shutdown_rx));
    handle.await.unwrap().unwrap();
    assert_eq!(*resize.lock().unwrap(), Some((100, 40)));
}

#[tokio::test]
async fn ping_replied_with_pong() {
    let h = setup();
    h.stream_tx.send(BridgeFrame::Ping(b"x".to_vec())).unwrap();
    h.stream_tx.send(BridgeFrame::Close).unwrap();

    let mut sink_rx = h.sink_rx;
    let handle = tokio::spawn(run(h.stream, h.sink, h.pty, h.shutdown_rx));
    let frame = sink_rx.recv().await.expect("got pong");
    assert_eq!(frame, BridgeFrame::Pong(b"x".to_vec()));
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn close_frame_terminates_run() {
    let h = setup();
    h.stream_tx.send(BridgeFrame::Close).unwrap();
    let handle = tokio::spawn(run(h.stream, h.sink, h.pty, h.shutdown_rx));
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn pty_eof_terminates_run() {
    let h = setup();
    h.pty_in_tx.send(None).unwrap();
    let handle = tokio::spawn(run(h.stream, h.sink, h.pty, h.shutdown_rx));
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn peer_disconnect_does_not_terminate_run() {
    // Sticky session model: when the phone goes away the gateway sends
    // `peer_status: connected=false`. The bridge MUST keep the PTY live so
    // the phone can re-enter the same link and pick up where it left off.
    // Output produced during the disconnect window is buffered in the
    // gateway (PhoneChannel) and replayed on reattach.
    let h = setup();
    let peer_down = r#"{"type":"peer_status","connected":false}"#.to_string();
    h.stream_tx.send(BridgeFrame::Text(peer_down)).unwrap();
    // The bridge should still be alive after this; close out via shutdown.
    let handle = tokio::spawn(run(h.stream, h.sink, h.pty, h.shutdown_rx));

    // Give it a moment to process the text frame; it must NOT exit yet.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!handle.is_finished(), "bridge exited on peer_disconnect");

    // Now actually shut it down so the test ends.
    let _ = h.shutdown_tx.send(());
    tokio::time::timeout(std::time::Duration::from_secs(2), handle)
        .await
        .expect("bridge did not exit on shutdown")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn peer_connect_does_not_terminate() {
    let h = setup();
    let peer_up = r#"{"type":"peer_status","connected":true}"#.to_string();
    h.stream_tx.send(BridgeFrame::Text(peer_up)).unwrap();
    h.stream_tx.send(BridgeFrame::Close).unwrap();
    let handle = tokio::spawn(run(h.stream, h.sink, h.pty, h.shutdown_rx));
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn shutdown_signal_terminates_run() {
    // Main triggers this when a new /pair arrives and the previous bridge
    // is still holding the PTY lock. Without this preemption hook the new
    // bridge would deadlock waiting for the lock.
    let h = setup();
    let handle = tokio::spawn(run(h.stream, h.sink, h.pty, h.shutdown_rx));
    // Bridge should be idle.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!handle.is_finished());

    let _ = h.shutdown_tx.send(());
    tokio::time::timeout(std::time::Duration::from_secs(2), handle)
        .await
        .expect("bridge did not exit on shutdown")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn malformed_text_does_not_crash() {
    let h = setup();
    h.stream_tx
        .send(BridgeFrame::Text("not json".into()))
        .unwrap();
    h.stream_tx.send(BridgeFrame::Close).unwrap();
    let resize = h.resize.clone();
    let handle = tokio::spawn(run(h.stream, h.sink, h.pty, h.shutdown_rx));
    handle.await.unwrap().unwrap();
    assert!(resize.lock().unwrap().is_none());
}
