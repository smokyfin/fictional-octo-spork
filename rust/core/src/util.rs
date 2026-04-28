use rand::Rng;

/// Allocate a random unused TCP port on `127.0.0.1` for use as an internal SOCKS port.
///
/// We bind, read the assigned port, then drop the listener. There is a small
/// race window before the consumer rebinds, but it is acceptable for our
/// localhost-only inter-component pipe (and the consumer will retry on
/// `EADDRINUSE`).
pub fn random_local_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

/// Generate a high-entropy random alphanumeric password for a SOCKS proxy.
pub fn random_password(len: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    (0..len)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Generate a random alphanumeric username (no `:` so it is SOCKS-safe).
pub fn random_username(len: usize) -> String {
    random_password(len)
}

/// Constant-time string comparison for IPC auth tokens.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Closes a raw POSIX file descriptor when dropped unless [`FdGuard::disarm`]
/// is called first. Used by the FFI entry points (`crate::start`) to guarantee
/// the platform-supplied TUN fd is closed exactly once if engine bring-up
/// fails — without this, the platform layer (Kotlin / Swift) would have to
/// guess whether Rust took ownership and close-or-not, which is racy.
///
/// FDs < 0 are treated as "no fd" (iOS uses `-1` because the Network Extension
/// doesn't expose a raw descriptor — packets flow via `packetFlow` instead).
#[cfg(unix)]
pub struct FdGuard(Option<i32>);

#[cfg(unix)]
impl FdGuard {
    pub fn new(fd: i32) -> Self {
        if fd < 0 {
            Self(None)
        } else {
            Self(Some(fd))
        }
    }
    pub fn disarm(mut self) {
        let _ = self.0.take();
    }
}

#[cfg(unix)]
impl Drop for FdGuard {
    fn drop(&mut self) {
        if let Some(fd) = self.0.take() {
            // SAFETY: fd was supplied by the platform layer with ownership
            // transferred to us; closing it here mirrors POSIX `close(2)`.
            unsafe {
                libc::close(fd);
            }
        }
    }
}

#[cfg(not(unix))]
pub struct FdGuard;

#[cfg(not(unix))]
impl FdGuard {
    pub fn new(_fd: i32) -> Self {
        Self
    }
    pub fn disarm(self) {}
}
