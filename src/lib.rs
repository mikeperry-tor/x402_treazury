pub mod catalog;
pub mod catalog_state;
pub mod config;
pub mod deployment;
pub mod mcp_wire;
pub mod network;
pub mod payment;
pub(crate) mod pricing;
pub mod provider_status;
pub mod providers;
pub mod qualification;
mod relevance;
pub mod rotation;
pub mod server;
pub mod treasury;
pub mod wallet_cli;

#[cfg(test)]
#[path = "../tests/support/h2_capture.rs"]
mod test_h2_capture;
#[cfg(test)]
#[path = "../tests/support/socks.rs"]
mod test_socks;
#[cfg(test)]
#[path = "../tests/support/tls.rs"]
mod test_tls;

pub mod discovery;

#[cfg(test)]
#[path = "../tests/support/signatures.rs"]
mod test_signatures;

pub mod limits;

pub mod output;

pub mod supervision;

pub mod build_identity;

pub mod cover;

mod http_cache;

pub mod discovery_relay;

#[cfg(test)]
#[path = "../tests/support/funding_quote.rs"]
mod test_funding_quote;

#[cfg(test)]
#[path = "../tests/support/log_capture.rs"]
mod test_log_capture;
