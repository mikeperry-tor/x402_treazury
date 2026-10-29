//! Syntactically valid empty V5 encoding for journal/identity fixtures.
//! This is not a funded or consensus-valid payment.
pub fn raw(expiry: u32) -> Vec<u8> {
    let mut raw = Vec::new();
    for word in [0x80000005u32, 0x26a7270a, 0xc2d6d0b4, 0, expiry] {
        raw.extend(word.to_le_bytes());
    }
    raw.extend([0, 0, 0, 0, 0]);
    raw
}

pub fn txid(expiry: u32) -> String {
    zcash_primitives::transaction::Transaction::read(
        raw(expiry).as_slice(),
        zcash_protocol::consensus::BranchId::Nu5,
    )
    .expect("syntactically valid V5 fixture")
    .txid()
    .to_string()
}
