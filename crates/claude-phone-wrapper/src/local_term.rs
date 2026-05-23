use std::io::Write;
use std::sync::Arc;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::broadcast;

use crate::pty::PtySession;

/// RAII guard that puts the controlling terminal into raw mode for the
/// lifetime of the value and restores cooked mode on drop. Required so the
/// wrapped `claude` TUI receives keystrokes byte-by-byte (no line buffering,
/// no local echo) just as it would if invoked directly.
///
/// On Windows, `crossterm` additionally enables `VIRTUAL_TERMINAL_PROCESSING`
/// on stdout and `VIRTUAL_TERMINAL_INPUT` on stdin so ANSI escape sequences
/// pass through cleanly in either direction.
pub struct RawModeGuard;

impl RawModeGuard {
    pub fn enable() -> anyhow::Result<Self> {
        // crossterm::enable_raw_mode on Windows only flips stdin into VT-input
        // mode; it does NOT enable ENABLE_VIRTUAL_TERMINAL_PROCESSING on
        // stdout. Without that flag, the child's ANSI escape sequences
        // (cursor moves, SGR colour codes, screen clears) are written through
        // to the console literally, so the user sees `←[2J←[H` text instead
        // of a rendered TUI. `crossterm::ansi_support::supports_ansi()` is
        // the public hook that, as a side-effect, enables VT processing on
        // the current stdout handle (once per process via `Once`).
        let _ = crossterm::ansi_support::supports_ansi();
        crossterm::terminal::enable_raw_mode()
            .map_err(|e| anyhow::anyhow!("enable_raw_mode failed: {e}"))?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Translate a semantic crossterm `KeyEvent` into the VT byte sequence the
/// downstream PTY (and therefore the `claude` TUI) expects. Returns `None`
/// for events we deliberately drop — KeyEventKind::Release on Windows, and
/// keys that have no portable VT encoding (CapsLock, NumLock, etc.).
///
/// Why this exists: raw stdin reads on Windows give inconsistent byte
/// sequences for a number of keys. The two that bit users hardest:
///   * Backspace → `0x08` (BS) instead of `0x7f` (DEL). readline (which
///     claude's prompt uses) treats BS as cursor-back-without-delete and
///     DEL as delete-prev-char. Some shells additionally bind ESC+BS to
///     "backward-kill-word", which is what produced the "deletes a whole
///     tab-sized chunk" behaviour the user reported.
///   * `?` (Shift+/) on non-US keyboard layouts can travel through
///     ENABLE_VIRTUAL_TERMINAL_INPUT as an unexpected sequence depending on
///     the active code page. Going through the event API gives us
///     `KeyCode::Char('?')` unambiguously.
fn key_to_bytes(ev: KeyEvent) -> Option<Vec<u8>> {
    // Windows fires both Press AND Release for every keystroke. If we acted
    // on Release too, every character would arrive twice. On Unix Crossterm
    // only emits Press by default, so this branch is a no-op there.
    if matches!(ev.kind, KeyEventKind::Release) {
        return None;
    }

    let mods = ev.modifiers;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    // SHIFT is already baked into `KeyCode::Char` for printable characters
    // (Shift+'/' arrives as `Char('?')`), so we ignore the bit here.

    let bytes: Vec<u8> = match ev.code {
        KeyCode::Char(c) => {
            if ctrl {
                // Ctrl+A..Z → 0x01..0x1A. Lower-case the char so Ctrl+Shift+X
                // folds onto the same control byte as Ctrl+X (matching every
                // mainstream terminal emulator).
                let lc = c.to_ascii_lowercase();
                if !lc.is_ascii() {
                    return None;
                }
                let b = lc as u8 & 0x1f;
                if alt {
                    vec![0x1b, b]
                } else {
                    vec![b]
                }
            } else if alt {
                // Alt+<char> is conventionally ESC-prefixed ("meta prefix")
                // — the way bash/readline/vim distinguish Meta keys.
                let mut v = vec![0x1b];
                let mut buf = [0u8; 4];
                v.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                v
            } else {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }
        KeyCode::Enter => vec![0x0d],
        // claude's prompt is readline-style: 0x7f (DEL) = delete-prev-char,
        // 0x08 (BS) = backward-char-without-delete. Always send DEL — that
        // matches what the web InputBar emits as well.
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![0x09],
        KeyCode::BackTab => vec![0x1b, b'[', b'Z'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Null => vec![0x00],
        KeyCode::Up => vec![0x1b, b'[', b'A'],
        KeyCode::Down => vec![0x1b, b'[', b'B'],
        KeyCode::Right => vec![0x1b, b'[', b'C'],
        KeyCode::Left => vec![0x1b, b'[', b'D'],
        KeyCode::Home => vec![0x1b, b'[', b'H'],
        KeyCode::End => vec![0x1b, b'[', b'F'],
        KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
        KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
        KeyCode::Insert => vec![0x1b, b'[', b'2', b'~'],
        KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],
        KeyCode::F(n) => match n {
            1 => vec![0x1b, b'O', b'P'],
            2 => vec![0x1b, b'O', b'Q'],
            3 => vec![0x1b, b'O', b'R'],
            4 => vec![0x1b, b'O', b'S'],
            5 => vec![0x1b, b'[', b'1', b'5', b'~'],
            6 => vec![0x1b, b'[', b'1', b'7', b'~'],
            7 => vec![0x1b, b'[', b'1', b'8', b'~'],
            8 => vec![0x1b, b'[', b'1', b'9', b'~'],
            9 => vec![0x1b, b'[', b'2', b'0', b'~'],
            10 => vec![0x1b, b'[', b'2', b'1', b'~'],
            11 => vec![0x1b, b'[', b'2', b'3', b'~'],
            12 => vec![0x1b, b'[', b'2', b'4', b'~'],
            _ => return None,
        },
        // CapsLock / NumLock / ScrollLock / Pause / Menu / Modifier-only
        // events have no portable VT encoding — drop them.
        _ => return None,
    };
    Some(bytes)
}

/// Drives the host stdin → PTY and PTY → host stdout pumps.
///
/// `first_rx` is the broadcast receiver handed back by `PtySession::spawn`.
/// Using *that* particular receiver (rather than calling `subscribe()` here)
/// guarantees no early child output is lost between spawn time and the
/// moment this task starts polling.
///
/// Returns when either pump observes EOF, or the PTY exits. The caller is
/// expected to hold a [`RawModeGuard`] across the lifetime of this call.
pub async fn run(pty: Arc<PtySession>, first_rx: broadcast::Receiver<Vec<u8>>) {
    // PTY → stdout. We run inside spawn_blocking so writes can hit a busy
    // console without parking the tokio worker pool, and we drive the
    // broadcast receiver via Handle::block_on inside the same thread.
    let mut rx = first_rx;
    let pty_exit = pty.clone();
    let stdout_task = tokio::task::spawn_blocking(move || {
        let handle = tokio::runtime::Handle::current();
        let mut stdout = std::io::stdout();
        loop {
            match handle.block_on(rx.recv()) {
                Ok(bytes) => {
                    if stdout.write_all(&bytes).is_err() {
                        break;
                    }
                    let _ = stdout.flush();
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    // stdin → PTY. crossterm::event::read() blocks until the next terminal
    // event arrives (key, paste, resize, focus, mouse). We deliberately do
    // NOT use raw stdin.read() any more — on Windows the byte stream coming
    // out of ENABLE_VIRTUAL_TERMINAL_INPUT has enough quirks (Backspace as
    // 0x08, code-page-dependent encodings of `?`, Meta+key sequences that
    // readline interprets as word-kill) to confuse claude's TUI. The event
    // API gives us semantic keys we re-emit through `key_to_bytes` as VT
    // bytes that match what the web client sends.
    let pty_for_stdin = pty.clone();
    let stdin_task = tokio::task::spawn_blocking(move || {
        let handle = tokio::runtime::Handle::current();
        loop {
            let ev = match event::read() {
                Ok(ev) => ev,
                Err(_) => break,
            };
            match ev {
                Event::Key(k) => {
                    if let Some(bytes) = key_to_bytes(k) {
                        if handle.block_on(pty_for_stdin.write_all(&bytes)).is_err() {
                            break;
                        }
                    }
                }
                // Paste events only fire if bracketed paste is enabled with
                // EnableBracketedPaste; we don't enable it (cmd.exe support
                // is uneven), but handle the variant so plain text paste
                // works if the terminal sends one.
                Event::Paste(text) => {
                    if handle
                        .block_on(pty_for_stdin.write_all(text.as_bytes()))
                        .is_err()
                    {
                        break;
                    }
                }
                // Forward host-window resizes to the PTY so the child redraws
                // at the new size. We don't currently emit a resize control
                // back to a paired phone — local-only feature for v1.
                Event::Resize(cols, rows) => {
                    let _ = pty_for_stdin.resize(cols, rows);
                }
                Event::Mouse(_) | Event::FocusGained | Event::FocusLost => {
                    // claude TUI doesn't drive these locally — drop.
                }
            }
        }
    });

    // Watch PTY exit. When the child exits, the reader broadcast closes,
    // which itself terminates stdout_task — but we still want to give the
    // user a clean return path even if their terminal happens to be wired
    // such that stdout_task lingers.
    tokio::select! {
        _ = pty_exit.wait_exit() => {}
        _ = stdout_task => {}
        // stdin_task only terminates when event::read() errors, which on
        // Windows happens once the console handle is closed by the OS.
        _ = stdin_task => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn k(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn question_mark_maps_to_ascii_0x3f() {
        // The original bug: `?` not making it to claude on Windows cmd.
        assert_eq!(
            key_to_bytes(k(KeyCode::Char('?'), KeyModifiers::NONE)),
            Some(vec![0x3f]),
        );
        // Shift+'/' arrives as Char('?') with SHIFT set; we still want 0x3f.
        assert_eq!(
            key_to_bytes(k(KeyCode::Char('?'), KeyModifiers::SHIFT)),
            Some(vec![0x3f]),
        );
    }

    #[test]
    fn backspace_maps_to_del_not_bs() {
        // The other half of the original bug: claude's readline-style prompt
        // treats 0x08 as "backward-char-without-delete", and some bindings
        // additionally consume ESC+BS as "backward-kill-word" — which is how
        // a single Backspace was eating an entire tab-sized chunk.
        assert_eq!(
            key_to_bytes(k(KeyCode::Backspace, KeyModifiers::NONE)),
            Some(vec![0x7f]),
        );
    }

    #[test]
    fn release_events_are_dropped() {
        // Windows fires Press + Release for every key. Without this filter
        // every typed character would arrive twice.
        let ev = KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        };
        assert_eq!(key_to_bytes(ev), None);
    }

    #[test]
    fn enter_maps_to_cr() {
        assert_eq!(
            key_to_bytes(k(KeyCode::Enter, KeyModifiers::NONE)),
            Some(vec![0x0d]),
        );
    }

    #[test]
    fn ctrl_c_maps_to_etx() {
        assert_eq!(
            key_to_bytes(k(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(vec![0x03]),
        );
    }

    #[test]
    fn ctrl_shift_x_folds_onto_ctrl_x() {
        let mods = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert_eq!(
            key_to_bytes(k(KeyCode::Char('X'), mods)),
            Some(vec![0x18]),
        );
    }

    #[test]
    fn alt_char_uses_meta_prefix() {
        assert_eq!(
            key_to_bytes(k(KeyCode::Char('b'), KeyModifiers::ALT)),
            Some(vec![0x1b, b'b']),
        );
    }

    #[test]
    fn arrow_keys_emit_csi_sequences() {
        let cases = [
            (KeyCode::Up, vec![0x1b, b'[', b'A']),
            (KeyCode::Down, vec![0x1b, b'[', b'B']),
            (KeyCode::Right, vec![0x1b, b'[', b'C']),
            (KeyCode::Left, vec![0x1b, b'[', b'D']),
        ];
        for (code, expected) in cases {
            assert_eq!(key_to_bytes(k(code, KeyModifiers::NONE)), Some(expected));
        }
    }

    #[test]
    fn multibyte_char_encodes_as_utf8() {
        // ó = U+00F3 → 0xC3 0xB3. Matches what the web InputBar sends so
        // both inputs land at claude as the same byte stream.
        assert_eq!(
            key_to_bytes(k(KeyCode::Char('ó'), KeyModifiers::NONE)),
            Some(vec![0xc3, 0xb3]),
        );
    }

    #[test]
    fn function_keys_use_canonical_vt_encodings() {
        assert_eq!(
            key_to_bytes(k(KeyCode::F(1), KeyModifiers::NONE)),
            Some(vec![0x1b, b'O', b'P']),
        );
        assert_eq!(
            key_to_bytes(k(KeyCode::F(5), KeyModifiers::NONE)),
            Some(vec![0x1b, b'[', b'1', b'5', b'~']),
        );
        assert_eq!(key_to_bytes(k(KeyCode::F(99), KeyModifiers::NONE)), None);
    }
}
