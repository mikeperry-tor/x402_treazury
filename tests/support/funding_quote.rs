//! Exact accepted destination terms for unfunded journal fixtures.
pub fn bound(amount: &str, recipient: &str, label: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "request": { "amount": amount, "recipient": recipient },
        "fixture_label": label
    }))
    .unwrap()
}
