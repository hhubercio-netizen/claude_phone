#[cfg(unix)]
pub struct RawTty {
    original: libc::termios,
}

#[cfg(unix)]
impl RawTty {
    pub fn enable() -> anyhow::Result<Self> {
        use std::os::fd::AsRawFd;
        let stdin_fd = std::io::stdin().as_raw_fd();
        let mut original: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(stdin_fd, &mut original) } != 0 {
            anyhow::bail!("tcgetattr failed");
        }
        let mut raw = original;
        unsafe {
            libc::cfmakeraw(&mut raw);
        }
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
