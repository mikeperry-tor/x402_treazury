//! Strict Tor STREAM evidence; raw credentials/targets stay in private artifacts.
//! Wire grammar: https://spec.torproject.org/control-spec/replies.html
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub user: String,
    pub password: String,
    pub kind: String,
    pub targets: BTreeSet<String>,
    pub required: bool,
    /// Known historical identities may be exported without authorizing their use.
    pub permitted: bool,
}
pub type Identities = BTreeMap<String, Identity>;
/// Parse Tor's quoted event fields without interpreting them as shell commands.
/// Unsupported escapes and malformed/ambiguous tokens invalidate qualification.
pub fn fields(line: &str) -> Result<Vec<String>> {
    ensure!(
        line.len() <= 1024 * 1024,
        "Tor control line exceeds 1048576-byte limit"
    );
    let mut result = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' if quoted => match chars.next().context("incomplete Tor quoted escape")? {
                '"' => token.push('"'),
                '\\' => token.push('\\'),
                'n' => token.push('\n'),
                'r' => token.push('\r'),
                't' => token.push('\t'),
                _ => anyhow::bail!("unsupported Tor quoted escape; cannot qualify event"),
            },
            ' ' | '\t' | '\r' | '\n' if !quoted => {
                if !token.is_empty() {
                    result.push(std::mem::take(&mut token));
                }
            }
            '\r' | '\n' => anyhow::bail!("unescaped newline in Tor quoted field"),
            _ => token.push(c),
        }
    }
    ensure!(!quoted, "unterminated Tor quoted field");
    if !token.is_empty() {
        result.push(token);
    }
    ensure!(
        result.len() <= 1000,
        "Tor control event exceeds 1000-field limit"
    );
    Ok(result)
}
#[derive(Default)]
struct Stream {
    credential: Option<(String, String)>,
    remote_host: bool,
    success: bool,
}
pub fn verify(expected: &Identities, events: &[String]) -> Result<Value> {
    ensure!(!expected.is_empty(), "empty Tor identity map");
    ensure!(
        events.len() <= 100000,
        "Tor event audit exceeds 100000-event limit"
    );
    let mut labels = BTreeMap::new();
    for (label, id) in expected {
        ensure!(
            ["discovery", "evm", "treasury"].contains(&id.kind.as_str()),
            "unknown Tor identity kind"
        );
        ensure!(
            labels
                .insert((id.user.clone(), id.password.clone()), label)
                .is_none(),
            "distinct identity labels share SOCKS credentials"
        );
        ensure!(
            id.kind != "discovery" || !id.targets.is_empty(),
            "discovery identity lacks destination constraint"
        );
    }
    let mut streams: BTreeMap<String, Stream> = BTreeMap::new();
    let mut circuits: BTreeMap<&String, BTreeSet<String>> = BTreeMap::new();
    let mut owners = BTreeMap::new();
    let mut successes: BTreeMap<&String, usize> = BTreeMap::new();
    for line in events {
        let f = fields(line)?;
        if f.first().map(String::as_str) != Some("650") {
            anyhow::bail!("non-event in Tor event evidence");
        }
        ensure!(
            matches!(f.get(1).map(String::as_str), Some("STREAM" | "CIRC")),
            "unrequested or malformed Tor event"
        );
        if f[1] == "CIRC" {
            ensure!(f.len() >= 4, "malformed Tor CIRC event");
            canonical_id(&f[2], false)?;
            continue;
        }
        ensure!(f.len() >= 6, "malformed Tor STREAM event");
        let stream = &f[2];
        let status = &f[3];
        let circuit = &f[4];
        let target = &f[5];
        canonical_id(stream, false)?;
        canonical_id(circuit, true)?;
        let mut attrs = BTreeMap::new();
        for field in &f[6..] {
            if let Some((key, value)) = field.split_once('=') {
                ensure!(
                    attrs.insert(key, value).is_none(),
                    "duplicate Tor STREAM attribute"
                );
            }
        }
        let state = streams.entry(stream.clone()).or_default();
        let user = attrs.get("SOCKS_USERNAME");
        let password = attrs.get("SOCKS_PASSWORD");
        if user.is_some() || password.is_some() {
            let credential = (
                user.context("Tor stream missing SOCKS username")?
                    .to_string(),
                password
                    .context("Tor stream missing SOCKS password")?
                    .to_string(),
            );
            ensure!(
                state
                    .credential
                    .as_ref()
                    .is_none_or(|old| *old == credential),
                "Tor stream credentials changed"
            );
            ensure!(
                labels.contains_key(&credential),
                "authenticated Tor stream has an unknown identity"
            );
            state.credential = Some(credential);
        }
        if status == "NEW" && attrs.get("PURPOSE") == Some(&"USER") {
            ensure!(
                state.credential.is_some(),
                "Tor USER stream lacks isolation credentials"
            );
        }
        let Some(credential) = &state.credential else {
            continue;
        };
        let label = labels[credential];
        let identity = &expected[label];
        ensure!(
            identity.permitted,
            "known Tor identity is not permitted in this phase"
        );
        if status == "NEW" {
            let (host, port) = target
                .rsplit_once(':')
                .context("Tor stream target lacks port")?;
            ensure!(
                !host.is_empty() && port.parse::<u16>().is_ok_and(|p| p != 0),
                "invalid Tor stream target"
            );
            let destination = reqwest::Url::parse(&format!("https://{target}/"))?;
            ensure!(
                destination.username().is_empty()
                    && destination.password().is_none()
                    && destination
                        .domain()
                        .is_some_and(|name| name == host.to_ascii_lowercase()),
                "Tor stream used a locally resolved address or noncanonical hostname"
            );
            ensure!(
                identity.targets.is_empty()
                    || identity.targets.contains(&target.to_ascii_lowercase()),
                "Tor identity used an unexpected destination"
            );
            state.remote_host = true;
        }
        if circuit != "0" {
            if let Some(other) = owners.insert(circuit.clone(), label) {
                ensure!(other == label, "distinct identities shared a Tor circuit");
            }
            circuits.entry(label).or_default().insert(circuit.clone());
        }
        if status == "SUCCEEDED" {
            ensure!(
                circuit != "0" && state.remote_host,
                "successful stream lacks remote-host/circuit evidence"
            );
            if !state.success {
                *successes.entry(label).or_default() += 1;
                state.success = true;
            }
        }
    }
    let mut result = BTreeMap::new();
    for (label, id) in expected {
        let count = successes.get(label).copied().unwrap_or(0);
        ensure!(
            !id.required || count > 0,
            "required Tor identity {label} has no successful remote-host stream"
        );
        result.insert(label,json!({"kind":id.kind,"required":id.required,"permitted":id.permitted,"successful_streams":count,"circuits":circuits.get(label).map_or(0,BTreeSet::len),"status":if count>0 {"observed"}else{"not_observed"}}));
    }
    Ok(
        json!({"identities":result,"scope":"Tor STREAM identity separation; not payment, settlement or complete unlinkability"}),
    )
}
fn canonical_id(value: &str, zero: bool) -> Result<()> {
    let parsed: u64 = value.parse().context("invalid Tor identifier")?;
    ensure!(
        (zero || parsed != 0) && parsed.to_string() == value,
        "noncanonical Tor identifier"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Identities, Vec<String>) {
        let mut ids = Identities::new();
        let mut events = Vec::new();
        for (n, (label, kind)) in [
            ("old", "evm"),
            ("new", "evm"),
            ("treasury", "treasury"),
            ("catalog", "discovery"),
        ]
        .into_iter()
        .enumerate()
        {
            ids.insert(
                label.into(),
                Identity {
                    user: "u".into(),
                    password: label.into(),
                    kind: kind.into(),
                    targets: if kind == "discovery" {
                        BTreeSet::from(["provider.example:443".into()])
                    } else {
                        BTreeSet::new()
                    },
                    required: kind != "discovery",
                    permitted: true,
                },
            );
            events.push(format!("650 STREAM {} NEW 0 provider.example:443 SOCKS_USERNAME=\"u\" SOCKS_PASSWORD=\"{label}\"",n+1));
            events.push(format!(
                "650 STREAM {} SUCCEEDED {} 192.0.2.1:443",
                n + 1,
                n + 1
            ));
        }
        (ids, events)
    }
    #[test]
    fn inherited_rotation_observer_cases_and_redaction() {
        let (ids, events) = fixture();
        let result = verify(&ids, &events).unwrap();
        assert_eq!(result["identities"]["old"]["successful_streams"], 1);
        assert!(!result.to_string().contains("password"));
        assert!(!result.to_string().contains("provider.example"));
        assert!(verify(&ids, &events[2..]).is_err());
        let changed: Vec<_> = events
            .iter()
            .map(|e| e.replace("2 SUCCEEDED 2", "2 SUCCEEDED 1"))
            .collect();
        assert!(verify(&ids, &changed).is_err());
        let result = verify(&ids, &events[..6]).unwrap();
        assert_eq!(result["identities"]["catalog"]["status"], "not_observed");
    }
    #[test]
    fn unknown_missing_changed_credentials_and_local_dns_fail() {
        let (ids, events) = fixture();
        for bad in [
            "650",
            "650 UNKNOWN",
            "650 CIRC",
            "650 STREAM 01 SUCCEEDED 1 provider.example:443",
            "650 STREAM 2 SUCCEEDED 01 provider.example:443",
            "650 STREAM 0 NEW 0 provider.example:443",
            "650 STREAM 18446744073709551616 NEW 0 provider.example:443",
            "650 STREAM 99 NEW 0 provider.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=unknown",
            "650 STREAM 99 NEW 0 provider.example:443 PURPOSE=USER",
            "650 STREAM 1 CLOSED 1 provider.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=new",
            "650 STREAM 99 NEW 0 provider.example:443 SOCKS_USERNAME=u",
        ] {
            let mut changed = events.clone();
            changed.push(bad.into());
            assert!(verify(&ids, &changed).is_err(), "{bad}");
        }
        for host in [
            "192.0.2.1",
            "[::1]",
            "different.example",
            "2130706433",
            "127%2e0%2e0%2e1",
        ] {
            let changed: Vec<_> = events
                .iter()
                .map(|e| e.replace("NEW 0 provider.example", &format!("NEW 0 {host}")))
                .collect();
            assert!(verify(&ids, &changed).is_err());
        }
    }
    #[test]
    fn required_identities_and_quoted_fields_are_explicit() {
        let (mut ids, events) = fixture();
        ids.get_mut("old").unwrap().required = false;
        assert_eq!(
            verify(&ids, &events[2..]).unwrap()["identities"]["old"]["status"],
            "not_observed"
        );
        assert_eq!(
            fields("a KEY=\"two words\" ESC=\"a\\\"b\"").unwrap(),
            vec!["a", "KEY=two words", "ESC=a\"b"]
        );
        assert!(fields("KEY=\"unfinished").is_err());
        assert!(fields(&"x".repeat(1024 * 1024 + 1)).is_err());
    }
}
