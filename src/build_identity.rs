//! Self-description for operator-trusted, hash-pinned binaries, not remote attestation.
pub mod inputs;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildIdentity {
    pub version: u32,
    pub package: String,
    pub source_sha256: String,
    pub cargo_lock_sha256: String,
    pub source_revision: String,
    pub source_dirty: bool,
    pub target: String,
    pub profile: String,
    pub compiler: String,
    pub features: Vec<String>,
    pub qualification_protocol: u32,
}
pub fn current() -> BuildIdentity {
    BuildIdentity {
        version: 1,
        package: env!("CARGO_PKG_VERSION").into(),
        source_sha256: env!("TREAZURY_SOURCE_HASH").into(),
        cargo_lock_sha256: env!("TREAZURY_LOCK_HASH").into(),
        source_revision: env!("TREAZURY_BUILD_REVISION").into(),
        source_dirty: env!("TREAZURY_BUILD_DIRTY") == "true",
        target: env!("TREAZURY_BUILD_TARGET").into(),
        profile: env!("TREAZURY_BUILD_PROFILE").into(),
        compiler: env!("TREAZURY_BUILD_RUSTC").into(),
        features: [
            ("zcash", cfg!(feature = "zcash")),
            ("zcash-testutils", cfg!(feature = "zcash-testutils")),
            ("zcash-regtest", cfg!(feature = "zcash-regtest")),
        ]
        .into_iter()
        .filter(|(_, yes)| *yes)
        .map(|(name, _)| name.into())
        .collect(),
        qualification_protocol: 1,
    }
}
