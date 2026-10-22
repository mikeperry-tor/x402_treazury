//! Startup-only local connectivity discovery, independent of financial authority.
use super::*;

const RETRY: Duration = Duration::from_secs(5);
const ERROR_REPEAT: Duration = Duration::from_secs(60);

impl NetworkContext {
    /// Keep finite serving discovery pending until its local Tor prerequisite exists.
    /// This is one TCP observation, not a Tor bootstrap check or a permanent monitor.
    /// Drop the future with its startup owner; no detached task or HTTP replay exists.
    pub(crate) async fn wait_for_startup_socks(&self) {
        if self.policy.mode != Mode::Tor || crate::qualification::active() {
            return;
        }
        self.startup_socks
            .get_or_init(|| async {
                let endpoint = self.policy.socks_endpoint.expect("validated Tor policy");
                let mut last_error = None::<tokio::time::Instant>;
                loop {
                    let reason = match self.connect_startup_socks(endpoint).await {
                        Ok(()) => {
                            if last_error.is_some() {
                                tracing::info!(
                                    socks_endpoint = %endpoint,
                                    "Tor SOCKS endpoint is reachable; resuming pending startup discovery; Tor bootstrap and destination connectivity are not yet verified"
                                );
                            }
                            break;
                        }
                        Err(error) => error.kind(),
                    };
                    if last_error.is_none_or(|at| at.elapsed() >= ERROR_REPEAT) {
                        tracing::error!(
                            category = "tor_socks_unreachable",
                            socks_endpoint = %endpoint,
                            io_kind = ?reason,
                            retry_after_seconds = RETRY.as_secs(),
                            error_repeat_after_seconds = ERROR_REPEAT.as_secs(),
                            "Tor SOCKS endpoint is unreachable; start Tor Browser or the configured Tor daemon. Startup discovery will resume automatically; MCP serving remains available"
                        );
                        last_error = Some(tokio::time::Instant::now());
                    }
                    tokio::time::sleep(RETRY).await;
                }
            })
            .await;
    }
}

#[cfg(test)]
mod tests;
