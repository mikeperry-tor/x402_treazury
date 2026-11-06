//! Listener-local Host allowlists; independent of bearer authentication.
use anyhow::{Result, ensure};
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;

pub fn require_loopback(address: std::net::IpAddr) -> Result<()> {
    ensure!(
        address.is_loopback(),
        "this release requires a loopback listen address"
    );
    Ok(())
}

#[derive(Clone, Default)]
pub struct HostPolicy {
    allowed: Option<Vec<String>>,
}

impl HostPolicy {
    pub fn new(allowed: Option<Vec<String>>, disabled: bool) -> Result<Self> {
        ensure!(
            !disabled || allowed.is_none(),
            "allowed_hosts and disable_host_check cannot be combined"
        );
        if let Some(entries) = &allowed {
            for entry in entries {
                let valid = !entry.is_empty()
                    && entry.trim() == entry
                    && !entry.contains(['/', '@', '*', '?', '#'])
                    && (entry.parse::<std::net::IpAddr>().is_ok()
                        || entry
                            .parse::<axum::http::uri::Authority>()
                            .is_ok_and(|authority| {
                                !authority.host().is_empty()
                                    && (entry.ends_with(']')
                                        || !entry.contains(':')
                                        || entry.rsplit_once(':').is_some_and(|(_, port)| {
                                            !port.is_empty()
                                                && port.bytes().all(|byte| byte.is_ascii_digit())
                                                && port.parse::<u16>().is_ok()
                                        }))
                            }));
                ensure!(
                    valid,
                    "allowed_hosts entries must be hostnames or IP addresses, optionally with a port; URLs and wildcards are not supported"
                );
            }
        }
        Ok(Self {
            allowed: if disabled { Some(Vec::new()) } else { allowed },
        })
    }

    pub fn validate_auth(&self, token: Option<&str>) -> Result<()> {
        ensure!(
            token.is_none_or(|value| !value.trim().is_empty()),
            "HTTP bearer token is empty or whitespace-only"
        );
        self.validate_auth_enabled(token.is_some())
    }

    pub fn validate_auth_enabled(&self, enabled: bool) -> Result<()> {
        ensure!(
            enabled || !self.allowed.as_ref().is_some_and(Vec::is_empty),
            "HTTP authentication and Host checking cannot both be disabled"
        );
        Ok(())
    }

    pub(super) fn apply(
        &self,
        config: StreamableHttpServerConfig,
        listener: &str,
    ) -> StreamableHttpServerConfig {
        match &self.allowed {
            None => config,
            Some(hosts) if hosts.is_empty() => {
                tracing::warn!(
                    listener,
                    category = "http_host_check_disabled",
                    "MCP Host allowlist is disabled for this listener; bearer authentication policy is unchanged"
                );
                config.disable_allowed_hosts()
            }
            Some(hosts) => config.with_allowed_hosts(hosts.clone()),
        }
    }
}
