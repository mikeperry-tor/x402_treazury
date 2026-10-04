//! Private parent-lifetime channel for meta-config serving, separate from MCP.
#[cfg(unix)]
use anyhow::Context;
use anyhow::{Result, ensure};

pub struct Parent {
    #[cfg(unix)]
    pipe: Option<tokio::net::unix::pipe::Receiver>,
}
impl Parent {
    /// Stdin must be a pipe owned by the supervisor. Never use Tokio's blocking
    /// stdin reader: its uncancellable thread can prevent shutdown after SIGTERM.
    pub fn from_stdin(enabled: bool) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::fd::AsFd;
            let pipe = if enabled {
                Some(
                    tokio::net::unix::pipe::Receiver::from_owned_fd(
                        std::io::stdin().as_fd().try_clone_to_owned()?,
                    )
                    .context("qualification parent control requires a readable stdin pipe")?,
                )
            } else {
                None
            };
            Ok(Self { pipe })
        }
        #[cfg(not(unix))]
        {
            ensure!(
                !enabled,
                "qualification parent control is supported only on Unix"
            );
            Ok(Self {})
        }
    }
    pub async fn closed(&mut self) -> Result<()> {
        #[cfg(unix)]
        if let Some(pipe) = &mut self.pipe {
            use tokio::io::AsyncReadExt;
            let mut byte = [0];
            let n = pipe
                .read(&mut byte)
                .await
                .context("qualification parent control read failed")?;
            tracing::warn!(
                "qualification supervisor connection closed or invalid; stopping startup or deliberately draining calls for transaction safety"
            );
            ensure!(
                n == 0,
                "qualification parent control accepts EOF only; received unexpected data"
            );
            return Ok(());
        }
        std::future::pending().await
    }
}
