//! Keep a blocked stdin read out of Tokio's shutdown-joined blocking pool.
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::mpsc,
};

pub fn transport() -> io::Result<(Input, tokio::io::Stdout)> {
    let (send, receive) = mpsc::channel(2);
    // A pipe may stay open after SIGTERM. This non-financial input thread has
    // bounded buffering and does not delay process exit after accepted work has
    // drained. It exits on EOF or the next read after the receiver closes.
    std::thread::Builder::new()
        .name("mcp-stdin".into())
        .spawn(move || {
            use io::Read;
            let input = io::stdin();
            let mut input = input.lock();
            loop {
                let mut bytes = vec![0; 8192];
                let result = match input.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => {
                        bytes.truncate(n);
                        Ok(bytes)
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => Err(e),
                };
                let failed = result.is_err();
                if send.blocking_send(result).is_err() || failed {
                    break;
                }
            }
        })?;
    Ok((
        Input {
            receive,
            bytes: Vec::new(),
            position: 0,
        },
        tokio::io::stdout(),
    ))
}

pub struct Input {
    receive: mpsc::Receiver<io::Result<Vec<u8>>>,
    bytes: Vec<u8>,
    position: usize,
}
impl AsyncRead for Input {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buffer.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if self.position == self.bytes.len() {
            match self.receive.poll_recv(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(e)),
                Poll::Ready(Some(Ok(bytes))) => {
                    self.bytes = bytes;
                    self.position = 0;
                }
            }
        }
        let n = buffer.remaining().min(self.bytes.len() - self.position);
        buffer.put_slice(&self.bytes[self.position..self.position + n]);
        self.position += n;
        Poll::Ready(Ok(()))
    }
}
