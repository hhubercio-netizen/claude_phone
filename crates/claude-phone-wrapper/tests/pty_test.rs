use std::time::Duration;

use claude_phone_wrapper::pty::PtySession;
use tokio::sync::broadcast::error::RecvError;

#[tokio::test]
async fn spawns_subprocess_and_reads_output() {
    let (prog, args): (&str, Vec<&str>) = if cfg!(windows) {
        ("cmd.exe", vec!["/c", "echo hi"])
    } else {
        ("sh", vec!["-c", "echo hi"])
    };
    let (_sess, mut rx) = PtySession::spawn(prog, &args, 80, 24, &[]).expect("spawn");

    let mut collected: Vec<u8> = Vec::new();
    // Read a few chunks; the subprocess writes "hi" then exits.
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Ok(bytes)) => {
                collected.extend_from_slice(&bytes);
                if collected.windows(2).any(|w| w == b"hi") {
                    return;
                }
            }
            Ok(Err(RecvError::Lagged(_))) => continue,
            Ok(Err(RecvError::Closed)) => break,
            Err(_) => break,
        }
    }
    panic!(
        "did not observe 'hi' in subprocess output; got: {:?}",
        String::from_utf8_lossy(&collected)
    );
}

#[tokio::test]
async fn resize_after_spawn_returns_ok() {
    let (prog, args): (&str, Vec<&str>) = if cfg!(windows) {
        ("cmd.exe", vec!["/c", "echo resize"])
    } else {
        ("sh", vec!["-c", "echo resize"])
    };
    let (sess, _rx) = PtySession::spawn(prog, &args, 80, 24, &[]).expect("spawn");
    assert!(sess.resize(120, 40).is_ok());
}

#[tokio::test]
async fn subscribe_after_spawn_sees_subsequent_output() {
    // Late subscribers should still observe whatever the child prints from
    // the point of subscribe forward. We use a tiny sleep on the *producer*
    // to give us time to subscribe before any output is generated.
    let (prog, args): (&str, Vec<&str>) = if cfg!(windows) {
        ("cmd.exe", vec!["/c", "ping -n 2 127.0.0.1 > NUL && echo late"])
    } else {
        ("sh", vec!["-c", "sleep 0.3; echo late"])
    };
    let (sess, _first) = PtySession::spawn(prog, &args, 80, 24, &[]).expect("spawn");
    let mut late = sess.subscribe();
    let mut collected: Vec<u8> = Vec::new();
    for _ in 0..40 {
        match tokio::time::timeout(Duration::from_secs(3), late.recv()).await {
            Ok(Ok(b)) => {
                collected.extend_from_slice(&b);
                if collected.windows(4).any(|w| w == b"late") {
                    return;
                }
            }
            Ok(Err(RecvError::Lagged(_))) => continue,
            Ok(Err(RecvError::Closed)) => break,
            Err(_) => break,
        }
    }
    panic!(
        "late subscriber missed output; got: {:?}",
        String::from_utf8_lossy(&collected)
    );
}

#[tokio::test]
async fn wait_exit_resolves_after_child_exits() {
    let (prog, args): (&str, Vec<&str>) = if cfg!(windows) {
        ("cmd.exe", vec!["/c", "echo bye"])
    } else {
        ("sh", vec!["-c", "echo bye"])
    };
    let (sess, _rx) = PtySession::spawn(prog, &args, 80, 24, &[]).expect("spawn");
    // `wait_exit` is signalled by the reader task on PTY EOF, which happens
    // once the child closes its end.
    tokio::time::timeout(Duration::from_secs(5), sess.wait_exit())
        .await
        .expect("child did not exit in time");
}
