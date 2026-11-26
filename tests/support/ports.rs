use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};

/// Pick a free localhost port outside Linux (32768..60999) and macOS
/// (49152..65535) ephemeral ranges so parallel `127.0.0.1:0` tests cannot
/// reclaim the address during drop-and-rebind assertions.
pub async fn non_ephemeral_address() -> SocketAddr {
    const BASE: u16 = 20_000;
    const SPAN: u16 = 9_000;
    static NEXT: AtomicU16 = AtomicU16::new(0);
    let seed = (std::process::id() as u16).wrapping_mul(137) % SPAN;
    for _ in 0..SPAN {
        let offset = NEXT.fetch_add(1, Ordering::Relaxed) % SPAN;
        let port = BASE + (seed + offset) % SPAN;
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        if tokio::net::TcpListener::bind(address).await.is_ok() {
            return address;
        }
    }
    panic!(
        "no free non-ephemeral localhost port found in {BASE}..{}",
        BASE + SPAN
    );
}
