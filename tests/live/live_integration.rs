//! Explicit opt-in qualification tooling. Planning never opens wallets or networks.
// integration_preparation owns the shared runner tests. Cargo --all-targets also
// builds this example's test harness; keep it empty instead of repeating the suite.
// Ordinary example builds and all-target Clippy still compile the runner below.
#[cfg(not(test))]
include!("runner/driver.rs");
