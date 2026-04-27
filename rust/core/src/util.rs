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
