// TM-CODE.2: opt back into `unsafe` for this module only. The crate root
// (`lib.rs`) sets `#![deny(unsafe_code)]`; the FFI to POSIX termios below
// is the single approved exception. Every `unsafe` block carries an
// inline `// SAFETY:` comment naming the precondition relied upon.
#![allow(unsafe_code)]

#[cfg(unix)]
pub struct RawTty {
    original: libc::termios,
}

#[cfg(unix)]
impl RawTty {
    pub fn enable() -> anyhow::Result<Self> {
        use std::os::fd::AsRawFd;
        let stdin_fd = std::io::stdin().as_raw_fd();
        // TM-CODE.2 / SAFETY: `libc::termios` is plain old data on the Unix
        // targets we support — all fields are integers, character arrays,
        // and bit-flags with no required invariants beyond being byte-init.
        // `tcgetattr` below overwrites every field before any read, so the
        // zero-initialised state is never observed.
        let mut original: libc::termios = unsafe {
            let uninit = std::mem::MaybeUninit::<libc::termios>::zeroed();
            uninit.assume_init()
        };
        // TM-CODE.2 / SAFETY: POSIX FFI — `stdin_fd` is a valid file
        // descriptor returned by `as_raw_fd()` on the process's stdin, and
        // `&mut original` is a unique, properly aligned pointer to a
        // `termios` storage location owned by this frame.
        if unsafe { libc::tcgetattr(stdin_fd, &mut original) } != 0 {
            anyhow::bail!("tcgetattr failed");
        }
        let mut raw = original;
        // TM-CODE.2 / SAFETY: POSIX FFI — `&mut raw` is a unique pointer to
        // a `termios` value initialised by the `tcgetattr` call above.
        unsafe {
            libc::cfmakeraw(&mut raw);
        }
        // TM-CODE.2 / SAFETY: POSIX FFI — same `stdin_fd` and a `&raw`
        // borrow of the locally-owned `termios` we just configured.
        if unsafe { libc::tcsetattr(stdin_fd, libc::TCSANOW, &raw) } != 0 {
            anyhow::bail!("tcsetattr failed");
        }
        Ok(Self { original })
    }
}

#[cfg(unix)]
impl Drop for RawTty {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        let stdin_fd = std::io::stdin().as_raw_fd();
        // TM-CODE.2 / SAFETY: POSIX FFI — restore the original termios on
        // the same stdin fd we read it from. `&self.original` is a unique
        // shared borrow during Drop.
        unsafe {
            libc::tcsetattr(stdin_fd, libc::TCSANOW, &self.original);
        }
    }
}

#[cfg(not(unix))]
pub struct RawTty;

#[cfg(not(unix))]
impl RawTty {
    pub fn enable() -> anyhow::Result<Self> {
        // No-op on Windows for v1.
        Ok(Self)
    }
}
