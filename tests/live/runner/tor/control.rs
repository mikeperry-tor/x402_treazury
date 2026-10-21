//! One command at a time, with a continuously drained bounded event stream.
use super::audit;
use crate::files;
use anyhow::{Context, Result, ensure};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::tcp::OwnedWriteHalf,
    sync::mpsc,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
const LINE: usize = 1024 * 1024;
#[derive(Default)]
pub struct Events {
    pub lines: Vec<String>,
    pub error: Option<String>,
    bytes: usize,
}
pub struct Control {
    writer: OwnedWriteHalf,
    replies: mpsc::Receiver<Vec<String>>,
    reader: JoinHandle<()>,
    pub events: Arc<Mutex<Events>>,
    pub fault: CancellationToken,
    closing: CancellationToken,
}
impl Control {
    pub async fn connect(
        address: std::net::SocketAddr,
        cookie: &[u8],
        path: &Path,
    ) -> Result<Self> {
        ensure!(
            cookie.len() == 32,
            "Tor control cookie must have exactly 32 bytes"
        );
        let stream = x402_treazury::network::local_control(address, Duration::from_secs(5)).await?;
        let (read, writer) = stream.into_split();
        let file = files::create_file(path)?;
        let (send, replies) = mpsc::channel(1);
        let events = Arc::new(Mutex::new(Events::default()));
        let fault = CancellationToken::new();
        let closing = CancellationToken::new();
        let saved = events.clone();
        let failed = fault.clone();
        let closing_reader = closing.clone();
        let reader = tokio::spawn(async move {
            let result = observe(BufReader::new(read), file, send, &saved).await;
            if let Err(error) = result {
                eprintln!("Tor control evidence incomplete: {error:#}");
                saved.lock().expect("control events lock").error = Some(error.to_string());
                failed.cancel();
            } else if !closing_reader.is_cancelled() {
                eprintln!(
                    "Tor control connection closed unexpectedly; isolation evidence is incomplete"
                );
                saved.lock().expect("control events lock").error =
                    Some("unexpected control EOF".into());
                failed.cancel();
            }
        });
        let mut control = Self {
            writer,
            replies,
            reader,
            events,
            fault,
            closing,
        };
        let auth = format!("AUTHENTICATE {}", alloy_primitives::hex::encode(cookie));
        control.command(&auth).await?;
        Ok(control)
    }
    pub async fn command(&mut self, command: &str) -> Result<Vec<String>> {
        ensure!(
            command.len() <= 4096 && !command.contains(['\r', '\n']),
            "invalid/oversized Tor control command (4096-byte limit)"
        );
        ensure!(!self.fault.is_cancelled(), "Tor control observer failed");
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            self.writer.write_all(command.as_bytes()).await?;
            self.writer.write_all(b"\r\n").await?;
            let reply = self
                .replies
                .recv()
                .await
                .context("Tor control connection ended before reply")?;
            ensure!(
                reply.last().is_some_and(|s| s.starts_with("250 ")),
                "Tor rejected a control command"
            );
            Ok::<_, anyhow::Error>(reply)
        })
        .await
        .context("Tor control command exceeded 10-second deadline")
        .and_then(|r| r);
        if result.is_err() && !(self.closing.is_cancelled() && command == "SIGNAL SHUTDOWN") {
            self.fault.cancel();
        }
        result
    }
    /// Set only when deliberately stopping our owned Tor after live observations.
    pub fn expect_shutdown(&self) {
        self.closing.cancel();
    }
    pub async fn finish(mut self) -> Result<Vec<String>> {
        self.closing.cancel();
        self.writer.shutdown().await?;
        tokio::time::timeout(Duration::from_secs(5), &mut self.reader)
            .await
            .context("Tor control reader exceeded 5-second cleanup deadline")??;
        let state = self.events.lock().expect("control events lock");
        ensure!(state.error.is_none(), "Tor control evidence is incomplete");
        Ok(state.lines.clone())
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.reader.abort();
    }
}
async fn line<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Option<String>> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            ensure!(bytes.is_empty(), "Tor control EOF inside a line");
            return Ok(None);
        }
        let end = chunk.iter().position(|b| *b == b'\n').map(|p| p + 1);
        let n = end.unwrap_or(chunk.len());
        ensure!(
            n <= LINE.saturating_sub(bytes.len()),
            "Tor control line exceeds {LINE}-byte limit"
        );
        bytes.extend_from_slice(&chunk[..n]);
        reader.consume(n);
        if end.is_some() {
            ensure!(bytes.ends_with(b"\r\n"), "Tor control line lacks CRLF");
            bytes.truncate(bytes.len() - 2);
            return Ok(Some(
                String::from_utf8(bytes).context("Tor control line is not UTF8")?,
            ));
        }
    }
}
async fn observe<R: AsyncBufRead + Unpin>(
    mut reader: R,
    mut file: std::fs::File,
    replies: mpsc::Sender<Vec<String>>,
    events: &Arc<Mutex<Events>>,
) -> Result<()> {
    use std::io::Write;
    let mut reply = Vec::new();
    let mut reply_bytes = 0usize;
    let mut data = false;
    while let Some(text) = line(&mut reader).await? {
        if data {
            if text == "." {
                data = false;
                continue;
            }
            reply_bytes = reply_bytes
                .checked_add(text.len())
                .context("Tor reply length overflow")?;
            ensure!(
                reply_bytes <= files::DOCUMENT_BYTES,
                "Tor reply exceeds {}-byte limit",
                files::DOCUMENT_BYTES
            );
            reply.push(text.strip_prefix('.').unwrap_or(&text).to_owned());
            continue;
        }
        ensure!(
            text.len() >= 4 && text.as_bytes()[..3].iter().all(u8::is_ascii_digit),
            "invalid Tor control reply prefix"
        );
        let separator = text.as_bytes()[3];
        ensure!(
            b" -+".contains(&separator),
            "invalid Tor control reply separator"
        );
        if text.starts_with("650") {
            ensure!(
                separator == b' ',
                "unsupported multiline Tor event; evidence rejected"
            );
            let _ = audit::fields(&text)?;
            let mut state = events.lock().expect("control events lock");
            ensure!(
                state.lines.len() < 100000,
                "Tor observer exceeds 100000-event limit"
            );
            ensure!(
                text.len() < files::DOCUMENT_BYTES.saturating_sub(state.bytes),
                "Tor event evidence exceeds {}-byte limit",
                files::DOCUMENT_BYTES
            );
            file.write_all(text.as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_data()?;
            state.bytes += text.len() + 1;
            state.lines.push(text);
            continue;
        }
        reply_bytes = reply_bytes
            .checked_add(text.len())
            .context("Tor reply length overflow")?;
        ensure!(
            reply_bytes <= files::DOCUMENT_BYTES,
            "Tor reply exceeds {}-byte limit",
            files::DOCUMENT_BYTES
        );
        reply.push(text);
        data = separator == b'+';
        if separator == b' ' {
            replies.try_send(std::mem::take(&mut reply)).context(
                "unexpected/unconsumed Tor control reply; refusing ambiguous command evidence",
            )?;
            reply_bytes = 0;
        }
    }
    ensure!(reply.is_empty() && !data, "Tor control EOF inside a reply");
    file.sync_all()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn fragmented_lines_and_limits_are_not_silently_truncated() {
        let mut input = BufReader::new(&b"250 OK\r\n"[..]);
        assert_eq!(line(&mut input).await.unwrap(), Some("250 OK".into()));
        for bytes in [
            b"partial".to_vec(),
            b"250 OK\n".to_vec(),
            vec![b'x'; LINE + 1],
        ] {
            assert!(line(&mut BufReader::new(bytes.as_slice())).await.is_err());
        }
    }
    #[tokio::test]
    async fn multiline_replies_preserve_interleaved_events() {
        let tmp = tempfile::tempdir().unwrap();
        let state = Arc::new(Mutex::new(Events::default()));
        let (tx, mut rx) = mpsc::channel(1);
        let bytes = b"250+key=\r\n..escaped\r\n.\r\n650 CIRC 7 BUILT\r\n250 OK\r\n";
        observe(
            BufReader::new(&bytes[..]),
            files::create_file(&tmp.path().join("events")).unwrap(),
            tx,
            &state,
        )
        .await
        .unwrap();
        assert_eq!(
            rx.recv().await.unwrap(),
            vec!["250+key=", ".escaped", "250 OK"]
        );
        assert_eq!(state.lock().unwrap().lines, vec!["650 CIRC 7 BUILT"]);
    }
}

#[cfg(test)]
mod socket_tests {
    use super::*;
    #[tokio::test]
    async fn authenticated_control_keeps_events_separate_and_closes_cleanly() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut read = BufReader::new(read);
            assert_eq!(
                line(&mut read).await.unwrap().unwrap(),
                format!("AUTHENTICATE {}", "01".repeat(32))
            );
            write.write_all(b"250 OK\r\n").await.unwrap();
            assert_eq!(line(&mut read).await.unwrap().unwrap(), "GETINFO test");
            write.write_all(b"650 STREAM 1 NEW 0 provider.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=p\r\n250-test=value\r\n250 OK\r\n").await.unwrap();
            assert!(line(&mut read).await.unwrap().is_none());
        });
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events");
        let mut client = Control::connect(address, &[1; 32], &path).await.unwrap();
        assert_eq!(
            client.command("GETINFO test").await.unwrap(),
            vec!["250-test=value", "250 OK"]
        );
        client.expect_shutdown();
        let events = client.finish().await.unwrap();
        assert_eq!(events.len(), 1);
        server.await.unwrap();
        let raw = std::fs::read_to_string(path).unwrap();
        assert!(raw.starts_with("650 STREAM"));
        assert!(!raw.contains("AUTHENTICATE"));
    }
    #[tokio::test]
    async fn unsolicited_replies_invalidate_control_evidence() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut read = BufReader::new(read);
            let _ = line(&mut read).await.unwrap();
            write.write_all(b"250 OK\r\n250 OK\r\n").await.unwrap();
        });
        let tmp = tempfile::tempdir().unwrap();
        let mut client = Control::connect(address, &[1; 32], &tmp.path().join("events"))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), client.fault.cancelled())
            .await
            .unwrap();
        assert!(client.command("GETINFO test").await.is_err());
        assert!(client.finish().await.is_err());
        server.await.unwrap();
    }
}
