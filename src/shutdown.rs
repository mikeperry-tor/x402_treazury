//! Process signal subscriptions retained across shutdown and accepted-work drain.

use std::io;

/// Register once before polling work, then receive successive operator signals.
/// A signal received while work is being cancelled remains available to drain.
pub struct ShutdownSignals {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(windows)]
    interrupt: tokio::signal::windows::CtrlC,
}

impl ShutdownSignals {
    pub fn new() -> io::Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                interrupt: signal(SignalKind::interrupt())?,
                terminate: signal(SignalKind::terminate())?,
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                interrupt: tokio::signal::windows::ctrl_c()?,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "operator shutdown signals are unsupported on this platform",
            ))
        }
    }

    pub async fn recv(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        let received = tokio::select! {
            received = self.interrupt.recv() => received,
            received = self.terminate.recv() => received,
        };
        #[cfg(windows)]
        let received = self.interrupt.recv().await;
        #[cfg(any(unix, windows))]
        {
            received.ok_or_else(|| io::Error::other("operator shutdown signal stream ended"))
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "operator shutdown signals are unsupported on this platform",
            ))
        }
    }
}
