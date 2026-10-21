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
    #[serde(default)]
    pub required_targets: BTreeSet<String>,
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
    target: Option<String>,
    success: bool,
}
pub fn verify(expected: &Identities, events: &[String]) -> Result<Value> {
    verify_with_scopes(expected, &[], events)
}
pub fn verify_with_scopes(
    expected: &Identities,
    scopes: &[x402_treazury::network::ScopeObservation],
    events: &[String],
) -> Result<Value> {
    ensure!(!expected.is_empty(), "empty Tor identity map");
    ensure!(
        events.len() <= 100000,
        "Tor event audit exceeds 100000-event limit"
    );
    let mut labels = identity_labels(expected)?;
    scope_labels(expected, scopes, &mut labels)?;
    let mut audit = StreamAudit::default();
    for line in events {
        audit.apply_event(expected, &labels, line)?;
    }
    audit.finish(expected)
}

type Credential = (String, String);
struct Label<'a> {
    owner: &'a String,
    scope: Option<(String, String)>,
}

fn identity_labels(expected: &Identities) -> Result<BTreeMap<Credential, Label<'_>>> {
    let mut labels = BTreeMap::new();
    for (label, id) in expected {
        ensure!(
            ["discovery", "evm", "treasury"].contains(&id.kind.as_str()),
            "unknown Tor identity kind"
        );
        ensure!(
            labels
                .insert(
                    (id.user.clone(), id.password.clone()),
                    Label {
                        owner: label,
                        scope: None
                    }
                )
                .is_none(),
            "distinct identity labels share SOCKS credentials"
        );
        ensure!(
            id.kind != "discovery" || !id.targets.is_empty(),
            "discovery identity lacks destination constraint"
        );
    }
    for id in expected.values() {
        ensure!(
            id.required_targets.is_subset(&id.targets),
            "required destinations exceed identity permissions"
        );
    }
    Ok(labels)
}
fn scope_labels<'a>(
    expected: &'a Identities,
    scopes: &[x402_treazury::network::ScopeObservation],
    labels: &mut BTreeMap<Credential, Label<'a>>,
) -> Result<()> {
    let canonical: BTreeMap<_, _> = expected
        .iter()
        .map(|(label, id)| ((id.user.clone(), id.password.clone()), label))
        .collect();
    for observation in scopes {
        let owner = *canonical
            .get(&observation.canonical)
            .context("HTTP scope has an unknown canonical identity")?;
        ensure!(
            !canonical.contains_key(&observation.scope),
            "HTTP scope collides with canonical credentials"
        );
        let origin = reqwest::Url::parse(&observation.origin)?;
        ensure!(
            matches!(origin.scheme(), "http" | "https")
                && origin.origin().ascii_serialization() == observation.origin,
            "HTTP scope origin is not canonical"
        );
        let target = format!(
            "{}:{}",
            origin
                .domain()
                .context("HTTP scope requires a remote hostname")?,
            origin
                .port_or_known_default()
                .context("HTTP scope origin lacks port")?
        );
        ensure!(
            expected[owner].targets.contains(&target),
            "HTTP scope destination exceeds canonical identity permissions"
        );
        match labels.get(&observation.scope) {
            Some(previous) => ensure!(
                previous.owner == owner
                    && previous
                        .scope
                        .as_ref()
                        .is_some_and(|(origin, previous_target)| origin == &observation.origin
                            && previous_target == &target),
                "HTTP scope credentials have conflicting owner or origin"
            ),
            None => {
                labels.insert(
                    observation.scope.clone(),
                    Label {
                        owner,
                        scope: Some((observation.origin.clone(), target)),
                    },
                );
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct StreamAudit<'a> {
    observed_targets: BTreeMap<&'a String, BTreeSet<String>>,
    streams: BTreeMap<String, Stream>,
    circuits: BTreeMap<&'a String, BTreeSet<String>>,
    owners: BTreeMap<String, Credential>,
    successes: BTreeMap<&'a String, usize>,
}

impl<'a> StreamAudit<'a> {
    fn apply_event(
        &mut self,
        expected: &'a Identities,
        labels: &BTreeMap<Credential, Label<'a>>,
        line: &str,
    ) -> Result<()> {
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
            return Ok(());
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
        let state = self.streams.entry(stream.clone()).or_default();
        observe_credentials(state, &attrs, labels)?;
        if status == "NEW" && attrs.get("PURPOSE") == Some(&"USER") {
            ensure!(
                state.credential.is_some(),
                "Tor USER stream lacks isolation credentials"
            );
        }
        let Some(credential) = state.credential.clone() else {
            return Ok(());
        };
        let binding = &labels[&credential];
        let label = binding.owner;
        let identity = &expected[label];
        ensure!(
            identity.permitted,
            "known Tor identity is not permitted in this phase"
        );
        if status == "NEW" {
            observe_destination(state, identity, target)?;
            ensure!(
                binding
                    .scope
                    .as_ref()
                    .is_none_or(|(_, target)| state.target.as_ref() == Some(target)),
                "HTTP scope used another origin"
            );
        }
        if circuit != "0" {
            if let Some(other) = self.owners.insert(circuit.clone(), credential.clone()) {
                ensure!(
                    other == credential,
                    "distinct identities shared a Tor circuit"
                );
            }
            self.circuits
                .entry(label)
                .or_default()
                .insert(circuit.clone());
        }
        if status == "SUCCEEDED" {
            ensure!(
                circuit != "0" && state.remote_host,
                "successful stream lacks remote-host/circuit evidence"
            );
            if !state.success {
                *self.successes.entry(label).or_default() += 1;
                state.success = true;
                self.observed_targets
                    .entry(label)
                    .or_default()
                    .insert(state.target.clone().context("stream destination missing")?);
            }
        }
        Ok(())
    }

    fn finish(self, expected: &Identities) -> Result<Value> {
        let mut result = BTreeMap::new();
        for (label, id) in expected {
            let count = self.successes.get(label).copied().unwrap_or(0);
            ensure!(
                !id.required || count > 0,
                "required Tor identity {label} has no successful remote-host stream"
            );
            ensure!(
                id.required_targets.iter().all(|t| self
                    .observed_targets
                    .get(label)
                    .is_some_and(|seen| seen.contains(t))),
                "required Tor identity {label} lacks a successful stream to a required destination"
            );
            result.insert(label,json!({"kind":id.kind,"required":id.required,"permitted":id.permitted,"successful_streams":count,"required_destinations":id.required_targets.len(),"circuits":self.circuits.get(label).map_or(0,BTreeSet::len),"status":if count>0 {"observed"}else{"not_observed"}}));
        }
        Ok(
            json!({"identities":result,"scope":"Tor STREAM identity separation; not payment, settlement or complete unlinkability"}),
        )
    }
}

fn observe_credentials(
    state: &mut Stream,
    attrs: &BTreeMap<&str, &str>,
    labels: &BTreeMap<Credential, Label<'_>>,
) -> Result<()> {
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
    Ok(())
}

fn observe_destination(state: &mut Stream, identity: &Identity, target: &str) -> Result<()> {
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
        identity.targets.is_empty() || identity.targets.contains(&target.to_ascii_lowercase()),
        "Tor identity used an unexpected destination"
    );
    let target = target.to_ascii_lowercase();
    ensure!(
        state.target.as_ref().is_none_or(|old| old == &target),
        "Tor stream destination changed"
    );
    state.target = Some(target);
    state.remote_host = true;
    Ok(())
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
                    required_targets: BTreeSet::new(),
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
    fn scope(password: &str, origin: &str) -> x402_treazury::network::ScopeObservation {
        x402_treazury::network::ScopeObservation {
            canonical: ("u".into(), "old".into()),
            scope: ("fresh-user".into(), password.into()),
            origin: origin.into(),
        }
    }
    fn scoped_ids() -> Identities {
        let mut ids = optional_identities();
        let payer = ids.get_mut("old").unwrap();
        payer.targets =
            BTreeSet::from(["provider.example:443".into(), "second.example:443".into()]);
        payer.required = true;
        payer.required_targets = payer.targets.clone();
        ids
    }
    fn scoped_events() -> Vec<String> {
        vec![
            "650 STREAM 1 NEW 0 provider.example:443 SOCKS_USERNAME=fresh-user SOCKS_PASSWORD=scope-a".into(),
            "650 STREAM 1 SUCCEEDED 10 ignored-target".into(),
            "650 STREAM 2 NEW 0 second.example:443 SOCKS_USERNAME=fresh-user SOCKS_PASSWORD=scope-b".into(),
            "650 STREAM 2 SUCCEEDED 20 ignored-target".into(),
        ]
    }
    #[test]
    fn fresh_scopes_aggregate_canonical_requirements_without_learning_from_streams() {
        let ids = scoped_ids();
        let scopes = [
            scope("scope-a", "https://provider.example"),
            scope("scope-b", "https://second.example"),
        ];
        let events = scoped_events();
        assert!(
            verify(&ids, &events).is_err(),
            "STREAM credentials alone never authorize an alias"
        );
        let result = verify_with_scopes(&ids, &scopes, &events).unwrap();
        assert_eq!(result["identities"]["old"]["successful_streams"], 2);
        assert_eq!(result["identities"]["old"]["circuits"], 2);
        for private in ["scope-a", "scope-b", "fresh-user", "provider.example"] {
            assert!(!result.to_string().contains(private));
        }
        let duplicated = [scopes[0].clone(), scopes[0].clone(), scopes[1].clone()];
        assert_eq!(
            verify_with_scopes(&ids, &duplicated, &events).unwrap(),
            result,
            "reobserving the same pooled scope is idempotent"
        );
        assert!(
            verify_with_scopes(&ids, &scopes, &[]).is_err(),
            "scope observations alone cannot satisfy successful-stream requirements"
        );
    }
    #[test]
    fn fresh_scopes_cannot_share_circuits_or_broaden_target_or_owner_authority() {
        let mut ids = scoped_ids();
        let scopes = [
            scope("scope-a", "https://provider.example"),
            scope("scope-b", "https://second.example"),
        ];
        let events = scoped_events();
        let mut shared = events.clone();
        shared[3] = shared[3].replace("20", "10");
        assert!(
            verify_with_scopes(&ids, &scopes, &shared).is_err(),
            "two fresh scopes of one payer still require separate circuits"
        );
        let mut crossed = events.clone();
        crossed[0] = crossed[0].replace("provider.example", "second.example");
        let mut no_required = ids.clone();
        no_required.get_mut("old").unwrap().required = false;
        no_required.get_mut("old").unwrap().required_targets.clear();
        assert!(
            verify_with_scopes(&no_required, &scopes, &crossed).is_err(),
            "an alias is restricted to its observed origin even if the owner permits another"
        );
        ids.get_mut("old").unwrap().permitted = false;
        assert!(
            verify_with_scopes(&ids, &scopes, &events).is_err(),
            "observations never make a historical owner permitted"
        );
        ids.get_mut("old").unwrap().permitted = true;
        ids.get_mut("old").unwrap().required_targets.clear();
        ids.get_mut("old").unwrap().required = false;
        assert_eq!(
            verify_with_scopes(&ids, &scopes, &[]).unwrap()["identities"]["old"]["status"],
            "not_observed"
        );
    }
    #[test]
    fn malformed_unknown_colliding_and_conflicting_scope_declarations_fail() {
        let mut ids = scoped_ids();
        ids.get_mut("old").unwrap().required = false;
        ids.get_mut("old").unwrap().required_targets.clear();
        ids.get_mut("new").unwrap().targets = BTreeSet::from(["provider.example:443".into()]);
        let a = scope("scope-a", "https://provider.example");
        assert!(verify_with_scopes(&ids, std::slice::from_ref(&a), &[]).is_ok());
        for origin in [
            "https://forbidden.example",
            "https://provider.example/path",
            "https://provider.example?query=value",
            "https://user@provider.example",
            "https://192.0.2.1",
        ] {
            assert!(verify_with_scopes(&ids, &[scope("scope-a", origin)], &[]).is_err());
        }
        let mut unknown = a.clone();
        unknown.canonical.1 = "unknown".into();
        assert!(verify_with_scopes(&ids, &[unknown], &[]).is_err());
        let mut collision = a.clone();
        collision.scope = ("u".into(), "new".into());
        assert!(verify_with_scopes(&ids, &[collision], &[]).is_err());
        let mut changed_owner = a.clone();
        changed_owner.canonical.1 = "new".into();
        assert!(verify_with_scopes(&ids, &[a.clone(), changed_owner], &[]).is_err());
        assert!(
            verify_with_scopes(
                &ids,
                &[a.clone(), scope("scope-a", "https://second.example")],
                &[]
            )
            .is_err()
        );
        assert!(
            verify_with_scopes(
                &ids,
                &[a, scope("scope-a", "http://provider.example:443")],
                &[]
            )
            .is_err(),
            "different full origins conflict even when their host and port coincide"
        );
    }
    #[test]
    fn actual_factory_observations_feed_the_strict_scope_auditor() {
        if std::env::var_os("TREAZURY_SCOPE_AUDIT_CHILD").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .env_clear()
                .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
                .env("TREAZURY_SCOPE_AUDIT_CHILD", "1")
                .args([
                    "--exact",
                    "tor::audit::tests::actual_factory_observations_feed_the_strict_scope_auditor",
                    "--nocapture",
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "isolated exact test must execute"
            );
            return;
        }
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use std::sync::{Arc,Mutex};
            use x402_treazury::network::{self,HttpPolicy,IsolationId,Mode,NetworkContext,NetworkPolicy,ScopeObservation};
            let observations=Arc::new(Mutex::new(Vec::<ScopeObservation>::new()));
            let saved=observations.clone();
            network::install_http_scope_observer(move |scope| {saved.lock().unwrap().push(scope.clone());Ok(())}).unwrap();
            let network=NetworkContext::new(NetworkPolicy {mode:Mode::Tor,socks_endpoint:Some("127.0.0.1:9150".parse().unwrap()),..Default::default()}).unwrap();
            let owner=IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap();
            let canonical=network.credentials(&owner);
            let endpoint="https://provider.example/path";
            let primary=network.http_participating(&owner,endpoint,None,false,HttpPolicy::default()).unwrap();
            let retry=network.http_candidate(&primary).unwrap();
            assert!(network.promote_http(&primary,&retry));
            let later=network.http_participating(&owner,endpoint,None,false,HttpPolicy::default()).unwrap();
            assert!(Arc::ptr_eq(&later,&retry));
            let scopes=observations.lock().unwrap().clone();
            assert_eq!(scopes.len(),2,"cache hits never invent another observed scope");
            assert_eq!(scopes[0].canonical,canonical);
            assert_eq!(scopes[1].canonical,canonical);
            assert_eq!(scopes[0].scope,network.credentials(&primary.scope));
            assert_eq!(scopes[1].scope,network.credentials(&retry.scope));
            assert_ne!(scopes[0].scope,scopes[1].scope);
            let expected=Identities::from([("payer".into(),Identity {user:canonical.0,password:canonical.1,kind:"evm".into(),targets:BTreeSet::from(["provider.example:443".into()]),required:true,required_targets:BTreeSet::from(["provider.example:443".into()]),permitted:true})]);
            let mut events=Vec::new();
            for (index,scope) in scopes.iter().enumerate() {
                events.push(format!("650 STREAM {} NEW 0 provider.example:443 SOCKS_USERNAME=\"{}\" SOCKS_PASSWORD=\"{}\"",index+1,scope.scope.0,scope.scope.1));
                events.push(format!("650 STREAM {} {} {} ignored-target",index+1,if index==0 {"CLOSED"} else {"SUCCEEDED"},index+10));
            }
            let report=verify_with_scopes(&expected,&scopes,&events).unwrap();
            assert_eq!(report["identities"]["payer"]["successful_streams"],1);
            assert_eq!(report["identities"]["payer"]["circuits"],2);
            let mut reused=events;reused[3]=reused[3].replace("SUCCEEDED 11","SUCCEEDED 10");
            assert!(verify_with_scopes(&expected,&scopes,&reused).is_err(),"distinct actual fresh scopes cannot share one circuit even for the same canonical payer");
        });
    }
    include!("audit_tests.rs");
}
