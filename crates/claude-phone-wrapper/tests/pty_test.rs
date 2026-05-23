use std::time::Duration;

use claude_phone_wrapper::pty::PtySession;

#[tokio::test]
async fn spawns_subprocess_and_reads_output() {
    let (prog, args): (&str, Vec<&str>) = if cfg!(windows) {
        ("cmd.exe", vec!["/c", "echo hi"])
    } else {
        ("sh", vec!["-c", "echo hi"])
    };
    let mut sess = PtySession::spawn(prog, &args, 80, 24, &[]).expect("spawn");

    let mut collected: Vec<u8> = Vec::new();
    // Read a few chunks; the subprocess writes "hi" then exits.
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_secs(3), sess.read()).await {
            Ok(Some(bytes)) => {
                collected.extend_from_slice(&bytes);
                if collected.windows(2).any(|w| w == b"hi") {
                    return;
                }
            }
            Ok(None) => break,
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
    let sess = PtySession::spawn(prog, &args, 80, 24, &[]).expect("spawn");
    assert!(sess.resize(120, 40).is_ok());
}
