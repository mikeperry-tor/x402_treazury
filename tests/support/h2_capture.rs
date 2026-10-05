//! Test-only decrypted client-frame capture; never linked into production egress.
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
pub type Capture = Arc<Mutex<Vec<Vec<u8>>>>;
pub struct Tap<T> {
    pub io: T,
    pub capture: Capture,
    pub index: usize,
}
impl<T: AsyncRead + Unpin> AsyncRead for Tap<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.io).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) {
            let mut all = self.capture.lock().unwrap();
            let bytes = &mut all[self.index];
            assert!(
                bytes.len() + buf.filled().len() - before <= 1_048_576,
                "fixture plaintext capture exceeds 1 MiB; evidence incomplete"
            );
            bytes.extend_from_slice(&buf.filled()[before..]);
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for Tap<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}
pub fn tls() -> tokio_rustls::TlsAcceptor {
    use tokio_rustls::rustls::{
        self,
        pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    };
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(
            include_bytes!("../fixtures/tls/server.der").to_vec(),
        )],
        PrivatePkcs8KeyDer::from(include_bytes!("../fixtures/tls/server-key.der").to_vec()).into(),
    )
    .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec()];
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}
/// Extract complete HEADERS/CONTINUATION blocks from this controlled fixture.
pub fn blocks(bytes: &[u8]) -> Vec<Vec<u8>> {
    assert!(bytes.starts_with(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"));
    let mut pos = 24;
    let mut current = Vec::new();
    let mut result = Vec::new();
    while pos + 9 <= bytes.len() {
        let len = ((bytes[pos] as usize) << 16)
            | ((bytes[pos + 1] as usize) << 8)
            | bytes[pos + 2] as usize;
        let kind = bytes[pos + 3];
        let flags = bytes[pos + 4];
        pos += 9;
        assert!(pos + len <= bytes.len(), "incomplete fixture HTTP2 frame");
        let mut data = &bytes[pos..pos + len];
        pos += len;
        if kind == 1 || kind == 9 {
            if kind == 1 {
                assert!(current.is_empty());
                if flags & 8 != 0 {
                    let pad = data[0] as usize;
                    data = &data[1..data.len() - pad];
                }
                if flags & 32 != 0 {
                    data = &data[5..];
                }
            }
            current.extend_from_slice(data);
            if flags & 4 != 0 {
                result.push(std::mem::take(&mut current));
            }
        }
    }
    assert_eq!(pos, bytes.len(), "incomplete fixture frame header");
    assert!(current.is_empty(), "incomplete header block");
    result
}
fn integer(bytes: &[u8], pos: &mut usize, bits: u8) -> usize {
    let mask = (1u8 << bits) - 1;
    let mut n = (bytes[*pos] & mask) as usize;
    *pos += 1;
    if n < mask as usize {
        return n;
    }
    let mut shift = 0;
    loop {
        let b = bytes[*pos];
        *pos += 1;
        n += ((b & 127) as usize) << shift;
        if b & 128 == 0 {
            return n;
        }
        shift += 7;
        assert!(shift < 56);
    }
}
fn string<'a>(bytes: &'a [u8], pos: &mut usize) -> (bool, &'a [u8]) {
    let huffman = bytes[*pos] & 128 != 0;
    let n = integer(bytes, pos, 7);
    let value = &bytes[*pos..*pos + n];
    *pos += n;
    (huffman, value)
}
/// Parse representations independently of h2. No decompressor needed to identify
/// the known fixture name: its RFC7541 Huffman bytes are pinned by the test.
pub fn never_indexed(block: &[u8]) -> Vec<(bool, Vec<u8>, bool, usize)> {
    let mut pos = 0;
    let mut result = Vec::new();
    while pos < block.len() {
        let first = block[pos];
        if first & 128 != 0 {
            integer(block, &mut pos, 7);
            continue;
        }
        if first & 224 == 32 {
            integer(block, &mut pos, 5);
            continue;
        }
        let incremental = first & 64 != 0;
        let sensitive = first & 240 == 16;
        let index = integer(block, &mut pos, if incremental { 6 } else { 4 });
        let name = if index == 0 {
            let (h, s) = string(block, &mut pos);
            (h, s.to_vec())
        } else {
            (false, vec![])
        };
        let (h, value) = string(block, &mut pos);
        if sensitive {
            result.push((name.0, name.1, h, value.len()));
        }
    }
    result
}
