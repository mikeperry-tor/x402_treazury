//! Operator-authorized source registration; all paid work stays in the common dispatcher.
mod coinbase;
mod curated;
mod directory;
mod import;
mod local;
pub mod policy;
mod publication;
pub(crate) mod raw;
mod relevance;
pub(crate) mod source_details;
mod store;
pub mod tools;
use crate::{
    catalog_state::{BoundTool, CatalogSnapshot, CatalogState},
    deployment::ListenerConfig,
    payment::PaidClient,
};
use anyhow::{Context, Result, ensure};
pub use curated::AcceptedProvider;
pub use import::{Candidate, Lifetime, Selection, Visibility};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{OnceCell, Semaphore};
use uuid::Uuid;
const FORMAT: u32 = 1;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct FetchKey {
    url: String,
    transport: crate::network::HttpPolicy,
    timeout: Option<Duration>,
}
impl From<&str> for FetchKey {
    fn from(url: &str) -> Self {
        Self {
            url: url.into(),
            transport: Default::default(),
            timeout: None,
        }
    }
}
fn hash(bytes: &[u8]) -> String {
    alloy_primitives::hex::encode(Sha256::digest(bytes))
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    id: String,
    owner: String,
    candidate: Candidate,
    targets: Vec<String>,
    revision: u64,
    document: String,
    hash: String,
    format: u32,
    accepted_at: u64,
    updated_at: u64,
    removed: bool,
    disabled: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    generation: u64,
    records: BTreeMap<String, Record>,
    // Read old registration snapshots without retaining their unused result ledger.
    #[serde(
        default,
        rename = "receipts",
        deserialize_with = "ignore_registration_receipts",
        skip_serializing
    )]
    _legacy_receipts: (),
}
fn ignore_registration_receipts<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<(), D::Error> {
    serde::de::IgnoredAny::deserialize(deserializer).map(|_| ())
}
struct Inner {
    base: CatalogSnapshot,
    state: State,
    store: Option<store::Store>,
}
#[derive(Debug)]
struct Fetched {
    completed: Instant,
    result: std::result::Result<Arc<Vec<u8>>, &'static str>,
}
type FetchCell = Arc<OnceCell<Fetched>>;
pub struct Manager {
    #[cfg(test)]
    commit_barrier: Mutex<Option<Arc<tokio::sync::Barrier>>>,
    #[cfg(test)]
    fixture_endpoint: Mutex<Option<String>>,
    #[cfg(test)]
    fixture_directory: Mutex<Option<String>>,
    #[cfg(test)]
    fixture_coinbase: Mutex<Option<String>>,
    #[cfg(test)]
    fail_after_commit: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    crash_after_commit: std::sync::atomic::AtomicBool,
    pub policy: policy::Policy,
    search_relay: Option<Arc<crate::discovery_relay::SearchRelay>>,
    listeners: BTreeMap<String, ListenerConfig>,
    payers: BTreeMap<String, PaidClient>,
    catalog: Arc<CatalogState>,
    inner: Mutex<Inner>,
    fetches: Mutex<BTreeMap<FetchKey, FetchCell>>,
    global_import: Arc<Semaphore>,
    owner_import: BTreeMap<String, Arc<Semaphore>>,
    mutations: Arc<Semaphore>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Add {
    spec_url: Option<String>,
    provider_id: Option<String>,
    name: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Query {
    source_name: Option<String>,
    query: Option<String>,
    cursor: Option<String>,
    limit: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub tool_id: String,
    pub arguments: serde_json::Map<String, Value>,
}
impl Manager {
    pub fn enabled(&self, owner: &str) -> bool {
        self.listeners
            .get(owner)
            .is_some_and(|l| l.source_management)
    }
    fn targets(&self, owner: &str, c: &Candidate) -> Result<Vec<String>> {
        ensure!(self.enabled(owner), "source_management_disabled");
        import::validate_source_name(&c.name)?;
        ensure!(
            c.visibility == Visibility::Server && c.targets.is_empty(),
            "source_scope_must_be_endpoint_local"
        );
        ensure!(
            c.lifetime != Lifetime::Persistent || self.policy.registry_file.is_some(),
            "persistent_registry_missing"
        );
        ensure!(
            !self.listeners[owner].sources.contains(&c.name),
            "source_name_reserved"
        );
        Ok(vec![owner.into()])
    }
    pub fn find_tool(&self, owner: &str, tool_id: &str) -> Result<BoundTool> {
        ensure!(self.enabled(owner), "source_management_disabled");
        find_catalog_tool(&self.catalog, owner, tool_id)
    }

    fn source_client(&self, owner: &str) -> Result<PaidClient> {
        ensure!(self.enabled(owner), "source_management_disabled");
        let wallet = policy::wallet(&self.policy, &self.listeners[owner]);
        Ok(self
            .payers
            .get(wallet)
            .context("source management wallet unavailable")?
            .clone()
            .with_timeout(self.policy.read_timeout_seconds.map(Duration::from_secs))
            .with_download_limits(self.policy.max_response_bytes, self.policy.max_help_bytes))
    }

    pub fn directory(&self, owner: &str) -> Result<BoundTool> {
        let client = self
            .source_client(owner)?
            .with_source_identity("builtin:x402-list");
        let base = directory::BASE.to_owned();
        #[cfg(test)]
        if let Some(base) = self.fixture_directory.lock().unwrap().clone() {
            let client = if base.starts_with("https://") {
                client.public_destinations()
            } else {
                client
            };
            return Ok(BoundTool {
                tool: directory::tool("/services").clone(),
                client,
                base,
                source: None,
                help: Arc::new(OnceCell::new()),
            });
        }
        Ok(BoundTool {
            tool: directory::tool("/services").clone(),
            client: client.public_destinations(),
            base,
            source: None,
            help: Arc::new(OnceCell::new()),
        })
    }
    pub async fn invoke_directory(
        &self,
        owner: &str,
        name: &str,
        args: &serde_json::Map<String, Value>,
    ) -> Result<crate::output::ToolOutput> {
        let (tool, args) = directory::request(name, args)?;
        let mut bound = self.directory(owner)?;
        bound.tool = tool.clone();
        bound.invoke_output(&args).await
    }
    pub async fn new(
        policy: policy::Policy,
        listeners: BTreeMap<String, ListenerConfig>,
        payers: BTreeMap<String, PaidClient>,
        catalog: Arc<CatalogState>,
        protected: Vec<PathBuf>,
    ) -> Result<Arc<Self>> {
        Self::new_with_search(policy, listeners, payers, catalog, protected, None).await
    }
    pub(crate) async fn new_with_search(
        policy: policy::Policy,
        listeners: BTreeMap<String, ListenerConfig>,
        payers: BTreeMap<String, PaidClient>,
        catalog: Arc<CatalogState>,
        protected: Vec<PathBuf>,
        search_relay: Option<Arc<crate::discovery_relay::SearchRelay>>,
    ) -> Result<Arc<Self>> {
        let file = policy.registry_file.clone();
        let (store, state) = tokio::task::spawn_blocking(move || -> Result<_> {
            let store = file
                .map(|p| store::Store::open(&p, &protected))
                .transpose()?;
            let state = store
                .as_ref()
                .map(store::Store::load)
                .transpose()?
                .unwrap_or_default();
            Ok((store, state))
        })
        .await??;
        let owner_import = listeners
            .keys()
            .map(|id| (id.clone(), Arc::new(Semaphore::new(1))))
            .collect();
        let manager = Arc::new(Self {
            #[cfg(test)]
            fixture_endpoint: Mutex::new(None),
            #[cfg(test)]
            fixture_directory: Mutex::new(None),
            #[cfg(test)]
            fixture_coinbase: Mutex::new(None),
            #[cfg(test)]
            fail_after_commit: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            commit_barrier: Mutex::new(None),
            #[cfg(test)]
            crash_after_commit: std::sync::atomic::AtomicBool::new(false),
            policy,
            search_relay,
            listeners,
            payers,
            inner: Mutex::new(Inner {
                base: (*catalog.read()).clone(),
                state,
                store,
            }),
            catalog,
            fetches: Mutex::new(BTreeMap::new()),
            global_import: Arc::new(Semaphore::new(4)),
            owner_import,
            mutations: Arc::new(Semaphore::new(16)),
        });
        let m = manager.clone();
        tokio::task::spawn_blocking(move || m.restore()).await??;
        Ok(manager)
    }
    fn restore(&self) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let mut state = inner.state.clone();
        for record in state.records.values_mut().filter(|r| !r.removed) {
            let c = record.candidate.clone();
            record.disabled = if import::validate_source_name(&c.name).is_err() {
                Some("invalid_source_name".into())
            } else if record.format != FORMAT {
                Some("catalog_format_changed: explicit refresh required".into())
            } else if !self.listeners.contains_key(&record.owner) {
                Some("owner_removed".into())
            } else {
                self.targets(&record.owner, &c)
                    .and_then(|targets| {
                        ensure!(
                            targets == record.targets,
                            "saved source scope is not endpoint-local"
                        );
                        import::build(&self.policy, &record.candidate, record.document.as_bytes())
                            .map(|_| ())
                    })
                    .err()
                    .map(|_| "policy_or_catalog_invalid".into())
            };
        }
        let generation = state.generation.max(self.catalog.read().generation);
        let snapshot = match self.snapshot(&state, &inner.base, generation) {
            Ok(s) => s,
            Err(_) => {
                for r in state.records.values_mut().filter(|r| !r.removed) {
                    r.disabled = Some("registration_conflict_or_invalid_state".into());
                }
                self.snapshot(&state, &inner.base, generation)?
            }
        };
        inner.state = state;
        self.catalog.publish(snapshot);
        Ok(())
    }
    fn snapshot(
        &self,
        state: &State,
        base: &CatalogSnapshot,
        generation: u64,
    ) -> Result<CatalogSnapshot> {
        let live: Vec<_> = state.records.values().filter(|r| !r.removed).collect();
        crate::limits::check(
            live.iter().map(|r| r.document.len()).sum::<usize>(),
            128 * 1024 * 1024,
            "registry_document_limit",
            "hardcoded_registry_document_bytes",
        )?;
        let mut result = base.clone();
        result.generation = generation;
        let previous = self.catalog.read();
        for record in live.into_iter().filter(|r| r.disabled.is_none()) {
            let target = &record.owner;
            let cfg = self.listeners.get(target).context("target_missing")?;
            let built = import::build_visible(
                &self.policy,
                &record.candidate,
                record.document.as_bytes(),
                cfg,
            )?;
            let wallet = policy::wallet(&self.policy, cfg);
            let payer = self
                .payers
                .get(wallet)
                .context("dynamic wallet unavailable")?
                .clone()
                .with_transport(
                    record
                        .candidate
                        .provider
                        .as_ref()
                        .map(|p| p.settings.transport())
                        .unwrap_or_default(),
                )
                .with_timeout(import::read_timeout(
                    &self.policy,
                    record.candidate.provider.as_ref(),
                ))
                .with_download_limits(self.policy.max_response_bytes, self.policy.max_help_bytes)
                .public_destinations();
            let view = result.views.get_mut(target).context("target_missing")?;
            let initial_count = view.len();
            for tool in &built.tools {
                ensure!(
                    !view.iter().any(|t| t.tool.name == tool.name),
                    "tool_name_collision"
                );
                view.push(BoundTool {
                    tool: tool.clone(),
                    client: payer.clone(),
                    base: built.base.clone(),
                    source: Some((record.id.clone(), record.revision)),
                    help: previous
                        .views
                        .get(target)
                        .and_then(|tools| {
                            tools.iter().find(|bound| {
                                bound.tool.name == tool.name
                                    && bound.source.as_ref()
                                        == Some(&(record.id.clone(), record.revision))
                            })
                        })
                        .map(|bound| bound.help.clone())
                        .unwrap_or_else(|| Arc::new(OnceCell::new())),
                });
            }
            if view.len() > initial_count
                && cfg.instructions_text.is_none()
                && let Some(instructions) = built.instructions.as_ref().filter(|s| !s.is_empty())
            {
                let text = result
                    .instructions
                    .entry(target.clone())
                    .or_default()
                    .get_or_insert_default();
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(&format!("[{}]\n{instructions}", record.candidate.name));
            }
        }
        Ok(result)
    }
    fn result(&self, r: &Record, caller: &str, snapshot: &CatalogSnapshot) -> Value {
        let mut result = json!({
            "name": r.candidate.name,
            "tool_prefix": import::tool_prefix(&r.candidate.name).ok(),
            "tool_count": snapshot.views.get(caller).into_iter().flatten().filter(|t| t.source.as_ref().is_some_and(|s| s.0 == r.id)).count(),
            "status": if r.disabled.is_some() { "disabled" } else { "available" },
            "next": "Search tools with source_name=name on this same MCP endpoint, then invoke tool_id with arguments through x402_treazury_tool_call. Optional advertised provider help may explain prices and request contracts before a paid call. Unknown cost is not free; registration and help do not establish paid-delivery qualification."
        });
        if let Some(provider) = &r.candidate.provider {
            result["provider_id"] = json!(provider.id);
            result["schema"] = json!({"kind": if r.candidate.remote_spec().is_some() { "remote" } else { "bundled" }, "reference": provider.settings.spec});
        }
        if let Some(reason) = &r.disabled {
            result["reason"] = json!(reason);
        }
        result
    }

    #[cfg(test)]
    async fn document(&self, owner: &str, url: &str) -> Result<Arc<Vec<u8>>> {
        self.document_with(owner, url, None).await
    }
    async fn source_document(&self, owner: &str, candidate: &Candidate) -> Result<Arc<Vec<u8>>> {
        match candidate.remote_spec() {
            Some(url) => {
                self.document_with(owner, url, candidate.provider.as_ref())
                    .await
            }
            None => Ok(Arc::new(curated::local_document(
                candidate.provider.as_ref().context("provider_required")?,
            )?)),
        }
    }
    async fn document_with(
        &self,
        owner: &str,
        url: &str,
        provider: Option<&curated::AcceptedProvider>,
    ) -> Result<Arc<Vec<u8>>> {
        let canonical = import::endpoint(&self.policy, url)?.to_string();
        let url = canonical.as_str();
        let key = FetchKey {
            url: canonical.clone(),
            transport: provider.map(|p| p.settings.transport()).unwrap_or_default(),
            timeout: import::read_timeout(&self.policy, provider),
        };
        let cell = {
            let mut cache = self.fetches.lock().unwrap();
            cache.retain(|_, cell| {
                if cell.get().is_none() {
                    return Arc::strong_count(cell) > 1;
                }
                cell.get().is_none_or(|done| {
                    let retention = if done.result.is_ok() { 300 } else { 1 };
                    done.completed.elapsed() < Duration::from_secs(retention)
                })
            });
            let max = (128 * 1024 * 1024 / self.policy.max_spec_bytes).min(8);
            if !cache.contains_key(&key) && cache.len() >= max {
                // Never evict active work. Completed retention is optional;
                // capacity churn must not cause duplicate in-flight fetches.
                let oldest = cache
                    .iter()
                    .filter_map(|(key, cell)| cell.get().map(|done| (key.clone(), done.completed)))
                    .min_by_key(|(_, time)| *time)
                    .map(|(key, _)| key);
                if let Some(key) = oldest {
                    cache.remove(&key);
                } else {
                    anyhow::bail!(crate::limits::busy("imports_busy", max));
                }
            }
            cache
                .entry(key)
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };
        let done = cell
            .get_or_init(|| async {
                // Only the initializer consumes import capacity; aliases merely
                // waiting for its result cannot starve independent imports.
                let result = async {
                    let owner = self.owner_import.get(owner).ok_or("unknown_owner")?;
                    let _owner = owner
                        .try_acquire()
                        .map_err(|_| crate::limits::busy("owner_import_busy", 1))?;
                    let _global = self
                        .global_import
                        .try_acquire()
                        .map_err(|_| crate::limits::busy("imports_busy", 4))?;
                    self.fetch_document(url, provider)
                        .await
                        .map(Arc::new)
                        .map_err(|_| "source_fetch_failed_or_rejected")
                }
                .await;
                Fetched {
                    completed: Instant::now(),
                    result,
                }
            })
            .await;
        done.result.clone().map_err(anyhow::Error::msg)
    }

    async fn fetch_document(
        &self,
        url: &str,
        provider: Option<&curated::AcceptedProvider>,
    ) -> Result<Vec<u8>> {
        #[cfg(test)]
        {
            let fixture = self.fixture_endpoint.lock().unwrap().clone();
            if let Some(url) = fixture {
                return import::fixture_fetch(&self.policy, &url).await;
            }
        }
        import::fetch_with(&self.policy, url, provider).await
    }
    pub async fn invoke(self: &Arc<Self>, owner: &str, name: &str, args: Value) -> Result<Value> {
        crate::limits::check(
            serde_json::to_vec(&args)?.len(),
            65536,
            "management_request_too_large",
            "hardcoded_management_request_bytes",
        )?;
        ensure!(self.enabled(owner), "source_management_disabled");
        match name {
            "x402_treazury_source_add" => self.add_source(owner, args).await,
            local::NAME => local::list(args.as_object().context("arguments must be an object")?),
            "x402_treazury_tools_search" => self.search_tools(owner, name, args),
            _ => anyhow::bail!("unknown management tool"),
        }
    }
    fn existing_registration(
        &self,
        owner: &str,
        key: import::RegistrationKey<'_>,
    ) -> Option<Value> {
        let inner = self.inner.lock().unwrap();
        inner
            .state
            .records
            .values()
            .find(|record| {
                !record.removed
                    && record.owner == owner
                    && record.candidate.registration_key() == key
            })
            .map(|record| self.result(record, owner, &self.catalog.read()))
    }

    async fn add_source(self: &Arc<Self>, owner: &str, args: Value) -> Result<Value> {
        ensure!(self.enabled(owner), "source_management_disabled");
        match (args.get("provider_id"), args.get("spec_url")) {
            (Some(Value::String(_)), None) | (None, Some(Value::String(_))) => {}
            _ => anyhow::bail!("provide exactly one string provider_id or spec_url"),
        }
        let a: Add = serde_json::from_value(args)?;
        let spec_url = a
            .spec_url
            .as_deref()
            .map(|url| import::endpoint(&self.policy, url).map(|url| url.to_string()))
            .transpose()?;
        let key = match a.provider_id.as_deref() {
            Some(id) => import::RegistrationKey::Provider(id),
            None => import::RegistrationKey::Specification(spec_url.as_deref().unwrap()),
        };
        // Preserve accepted generic URL imports and stable provider identities;
        // re-adding never refreshes configuration or schema bytes implicitly.
        if let Some(result) = self.existing_registration(owner, key) {
            return Ok(result);
        }
        let provider = match a.provider_id.as_deref() {
            Some(id) => Some(curated::AcceptedProvider::current(id)?),
            None => {
                let matches = crate::providers::matching_spec(spec_url.as_deref().unwrap())?;
                match matches.as_slice() {
                    [] => None,
                    [provider] => Some(curated::AcceptedProvider::current(provider.id)?),
                    _ => anyhow::bail!(
                        "ambiguous specification; select provider_id from: {}",
                        matches.iter().map(|p| p.id).collect::<Vec<_>>().join(", ")
                    ),
                }
            }
        };
        let generated_name = a.name.is_none();
        let name = a.name.unwrap_or_else(|| {
            provider
                .as_ref()
                .map(|p| p.id.clone())
                .unwrap_or_else(|| import::default_source_name(spec_url.as_deref().unwrap()))
        });
        let candidate = Candidate {
            name,
            spec_url: provider
                .as_ref()
                .map(|p| {
                    if p.settings.spec.starts_with("https://") {
                        p.settings.spec.clone()
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_else(|| spec_url.unwrap()),
            base_url: None,
            provider,
            selection: Selection::default(),
            visibility: Visibility::Server,
            targets: vec![],
            lifetime: if self.policy.registry_file.is_some() {
                Lifetime::Persistent
            } else {
                Lifetime::Process
            },
        };
        if let Some(result) = self.existing_registration(owner, candidate.registration_key()) {
            return Ok(result);
        }
        if !generated_name {
            self.targets(owner, &candidate)?;
        }
        let document = self.source_document(owner, &candidate).await?;
        let id = Uuid::new_v4().to_string();
        import::build(&self.policy, &candidate, &document)?;
        let record = Record {
            id,
            owner: owner.into(),
            candidate,
            targets: vec![],
            revision: 1,
            document: String::from_utf8((*document).clone())?,
            hash: hash(&document),
            format: FORMAT,
            accepted_at: epoch(),
            updated_at: epoch(),
            removed: false,
            disabled: None,
        };
        self.commit(owner.to_owned(), generated_name, record).await
    }

    fn search_tools(self: &Arc<Self>, owner: &str, name: &str, args: Value) -> Result<Value> {
        let query: Query = serde_json::from_value(args)?;
        if let Some(name) = &query.source_name {
            import::validate_source_name(name)?;
            ensure!(
                self.inner
                    .lock()
                    .unwrap()
                    .state
                    .records
                    .values()
                    .any(|r| !r.removed && r.owner == owner && &r.candidate.name == name),
                "source_name_unknown"
            );
        }
        search_catalog_query(&self.catalog, owner, name, query)
    }

    async fn commit(
        self: &Arc<Self>,
        owner: String,
        generated_name: bool,
        mut record: Record,
    ) -> Result<Value> {
        #[cfg(test)]
        {
            let barrier = self.commit_barrier.lock().unwrap().clone();
            if let Some(barrier) = barrier {
                tokio::time::timeout(Duration::from_secs(10), barrier.wait())
                    .await
                    .context("test commit barrier deadline")?;
            }
        }
        let permit = self
            .mutations
            .clone()
            .try_acquire_owned()
            .map_err(|_| anyhow::anyhow!(crate::limits::busy("mutation_queue_full", 16)))?;
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut inner = this.inner.lock().unwrap();
            // Recheck duplicate URLs under the publication lock, including racing adds.
            if let Some(r) = inner.state.records.values().find(|r| {
                !r.removed
                    && r.owner == owner
                    && r.candidate.registration_key() == record.candidate.registration_key()
            }) {
                return Ok(this.result(r, &owner, &this.catalog.read()));
            }
            let mut state = inner.state.clone();
            if generated_name {
                let base = record.candidate.name.clone();
                let mut suffix = 1usize;
                while this.listeners[&owner]
                    .sources
                    .contains(&record.candidate.name)
                    || state.records.values().any(|r| {
                        !r.removed && r.owner == owner && r.candidate.name == record.candidate.name
                    })
                {
                    suffix += 1;
                    let tail = format!("-{suffix}");
                    record.candidate.name =
                        format!("{}{}", &base[..base.len().min(64 - tail.len())], tail);
                }
            }
            ensure!(
                !state.records.values().any(|r| !r.removed
                    && r.owner == owner
                    && r.candidate.name == record.candidate.name),
                "source_name_reserved"
            );
            record.targets = this.targets(&owner, &record.candidate)?;
            let id = record.id.clone();
            state.records.insert(id.clone(), record);
            state.generation = state
                .generation
                .max(this.catalog.read().generation)
                .checked_add(1)
                .context("generation exhausted")?;
            let snapshot = this.snapshot(&state, &inner.base, state.generation)?;
            let result = this.result(&state.records[&id], &owner, &snapshot);
            if let Some(store) = &mut inner.store {
                let mut saved = state.clone();
                saved
                    .records
                    .retain(|_, r| r.candidate.lifetime == Lifetime::Persistent);
                store.save(&saved)?;
            }
            #[cfg(test)]
            if this
                .crash_after_commit
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                std::process::exit(43);
            }
            #[cfg(test)]
            ensure!(
                !this
                    .fail_after_commit
                    .load(std::sync::atomic::Ordering::SeqCst),
                "injected crash after durable commit"
            );
            inner.state = state;
            this.catalog.publish(snapshot);
            Ok(result)
        })
        .await?
    }
}
pub(crate) fn find_catalog_tool(
    catalog: &CatalogState,
    owner: &str,
    tool_id: &str,
) -> Result<BoundTool> {
    crate::catalog_state::find(&catalog.read(), owner, tool_id).cloned()
}
pub(crate) fn search_catalog(
    catalog: &CatalogState,
    owner: &str,
    name: &str,
    args: Value,
) -> Result<Value> {
    crate::limits::check(
        serde_json::to_vec(&args)?.len(),
        65536,
        "management_request_too_large",
        "hardcoded_management_request_bytes",
    )?;
    let q: Query = serde_json::from_value(args)?;
    ensure!(q.source_name.is_none(), "source_name_unknown");
    search_catalog_query(catalog, owner, name, q)
}

// Resolve source names before projection without reserializing the caller's request.
fn search_catalog_query(
    catalog: &CatalogState,
    owner: &str,
    name: &str,
    q: Query,
) -> Result<Value> {
    let snapshot = catalog.read();
    let candidates: Vec<_> = snapshot
        .views
        .get(owner)
        .into_iter()
        .flatten()
        .filter(|t| {
            q.source_name.as_ref().is_none_or(|name| {
                t.source.is_some()
                    && t.tool
                        .name
                        .strip_prefix("dyn_")
                        .and_then(|n| n.rsplit_once("__"))
                        .is_some_and(|(registered, _)| registered == name)
            })
        })
        .collect();
    let available = candidates.len();
    let values = relevance::rank(candidates, q.query.as_deref().unwrap_or_default());
    page(
        values,
        &q,
        snapshot.generation,
        available,
        &format!(
            "{}:{owner}:{name}:{:?}:{:?}",
            catalog.instance(),
            q.source_name,
            q.query
        ),
    )
}
fn epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn page(
    values: Vec<&crate::catalog_state::BoundTool>,
    query: &Query,
    generation: u64,
    available: usize,
    scope: &str,
) -> Result<Value> {
    let limit = query
        .limit
        .unwrap_or(crate::limits::DEFAULT_PAGE_SIZE as u64);
    ensure!(limit > 0, "invalid page size");
    let limit = limit.min(values.len() as u64) as usize;
    let prefix = format!("{}:{generation}:", hash(scope.as_bytes()));
    let offset = if let Some(cursor) = &query.cursor {
        cursor
            .strip_prefix(&prefix)
            .context("stale_cursor: restart listing")?
            .parse::<usize>()?
    } else {
        0
    };
    ensure!(offset <= values.len(), "invalid cursor");
    let end = offset.saturating_add(limit).min(values.len());
    let items: Vec<_> = values[offset..end].iter().map(|t| json!({
        "tool_id":t.tool.name, "description":t.tool.description, "input_schema":t.tool.input_schema
    })).collect();
    let mut result = json!({
        "items":items,
        "available":available,
        "total":values.len(),
        "catalog_generation":generation,
        "next_cursor":(end<values.len()).then(||format!("{prefix}{end}")),
    });
    if values.is_empty()
        && available > 0
        && query.query.as_ref().is_some_and(|q| !q.trim().is_empty())
    {
        result["hint"] = json!(
            "No tools matched in this scope. Try different keywords, or omit query to enumerate available tools and page with next_cursor."
        );
    }
    Ok(result)
}
pub async fn inspect_cli(meta_config: &std::path::Path) -> Result<()> {
    let cfg: crate::deployment::MetaConfig =
        toml::from_str(&std::fs::read_to_string(meta_config)?)?;
    let mut policy = cfg
        .source_management
        .context("source management not configured")?;
    policy.resolve(meta_config);
    let path = policy
        .registry_file
        .context("persistent registry not configured")?;
    let value = tokio::task::spawn_blocking(move || store::inspect(&path)).await??;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// Operator-only registry maintenance. Exclusive ownership requires serving to be stopped.
pub async fn maintain_cli(
    path: &std::path::Path,
    owner: &str,
    source_id: &str,
    refresh: bool,
) -> Result<()> {
    maintain_cli_with_network(path, owner, source_id, refresh, None).await
}
pub async fn maintain_cli_with_network(
    path: &std::path::Path,
    owner: &str,
    source_id: &str,
    refresh: bool,
    network_override: Option<&crate::network::NetworkPolicy>,
) -> Result<()> {
    crate::deployment::Deployment::show_config_with_network(path, network_override).await?;
    let mut cfg = crate::deployment::MetaConfig::parse_with_network(
        &std::fs::read_to_string(path)?,
        network_override,
    )?;
    if let Some(t) = &mut cfg.treasury {
        t.resolve(path);
    }
    let mut policy = cfg
        .source_management
        .clone()
        .context("source management not configured")?;
    policy.resolve(path);
    let file = policy
        .registry_file
        .as_ref()
        .context("persistent registry not configured")?;
    ensure!(file.is_file(), "source registry does not exist");
    let mut store = store::Store::open(file, &policy::protected_paths(&cfg, path))?;
    let mut state = store.load()?;
    let record = state
        .records
        .get_mut(source_id)
        .filter(|r| !r.removed && r.owner == owner)
        .context("source_not_manageable")?;
    if refresh {
        ensure!(
            cfg.servers.get(owner).is_some_and(|s| s.source_management),
            "source_management_disabled"
        );
        ensure!(
            record.targets == [owner] && record.candidate.visibility == Visibility::Server,
            "source_scope_must_be_endpoint_local"
        );
        crate::network::install(cfg.network.clone())?;
        let mut candidate = record.candidate.clone();
        if let Some(provider) = &candidate.provider {
            candidate.provider = Some(curated::AcceptedProvider::current(&provider.id)?);
        }
        let bytes = match candidate.remote_spec() {
            Some(url) => import::fetch_with(&policy, url, candidate.provider.as_ref()).await?,
            None => {
                curated::local_document(candidate.provider.as_ref().context("provider_required")?)?
            }
        };
        import::build(&policy, &candidate, &bytes)?;
        candidate.spec_url = candidate.remote_spec().unwrap_or_default().to_owned();
        record.candidate = candidate;
        record.hash = hash(&bytes);
        record.document = String::from_utf8(bytes)?;
        record.format = FORMAT;
        record.disabled = None;
    } else {
        record.removed = true;
        record.document.clear();
        record.candidate.provider = None;
    }
    record.revision = record
        .revision
        .checked_add(1)
        .context("revision exhausted")?;
    record.updated_at = epoch();
    let result = json!({"source_id":record.id,"server":owner,"revision":record.revision,"removed":record.removed});
    crate::limits::check(
        state
            .records
            .values()
            .map(|r| r.document.len())
            .sum::<usize>(),
        128 * 1024 * 1024,
        "registry_document_limit",
        "hardcoded_registry_document_bytes",
    )?;
    state.generation = state
        .generation
        .checked_add(1)
        .context("generation exhausted")?;
    store.save(&state)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

#[cfg(test)]
mod tests;
