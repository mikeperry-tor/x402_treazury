//! Operator-authorized source registration; all paid work stays in the common dispatcher.
mod directory;
mod import;
pub mod policy;
mod store;
pub mod tools;
use crate::{
    catalog_state::{BoundTool, CatalogSnapshot, CatalogState},
    deployment::ListenerConfig,
    payment::PaidClient,
};
use anyhow::{Context, Result, ensure};
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
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    hash: String,
    result: Value,
    persistent: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    generation: u64,
    records: BTreeMap<String, Record>,
    receipts: BTreeMap<String, Receipt>,
}
struct Inner {
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
    fail_after_commit: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    crash_after_commit: std::sync::atomic::AtomicBool,
    pub policy: policy::Policy,
    listeners: BTreeMap<String, ListenerConfig>,
    payers: BTreeMap<String, PaidClient>,
    catalog: Arc<CatalogState>,
    base: CatalogSnapshot,
    inner: Mutex<Inner>,
    fetches: Mutex<BTreeMap<String, FetchCell>>,
    global_import: Arc<Semaphore>,
    owner_import: BTreeMap<String, Arc<Semaphore>>,
    mutations: Arc<Semaphore>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Add {
    spec_url: String,
    name: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Query {
    source_id: Option<String>,
    query: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub tool_ref: String,
    pub arguments: serde_json::Map<String, Value>,
}
impl Manager {
    pub fn enabled(&self, owner: &str) -> bool {
        self.listeners
            .get(owner)
            .is_some_and(|l| l.source_management)
    }
    pub fn accepts(&self, owner: &str) -> bool {
        self.enabled(owner)
    }
    fn targets(&self, owner: &str, c: &Candidate) -> Result<Vec<String>> {
        ensure!(self.enabled(owner), "source_management_disabled");
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
    pub fn tool_reference(&self, owner: &str, bound: &BoundTool) -> String {
        catalog_reference(&self.catalog, owner, bound)
    }
    pub fn find_reference(&self, owner: &str, reference: &str) -> Result<BoundTool> {
        ensure!(self.enabled(owner), "source_management_disabled");
        find_catalog_reference(&self.catalog, owner, reference)
    }

    pub fn directory(&self, owner: &str) -> Result<BoundTool> {
        ensure!(self.enabled(owner), "source_management_disabled");
        let wallet = policy::wallet(&self.policy, &self.listeners[owner]);
        let client = self
            .payers
            .get(wallet)
            .context("directory wallet unavailable")?
            .clone()
            .with_timeout(self.policy.read_timeout_seconds.map(Duration::from_secs))
            .with_download_limits(self.policy.max_response_bytes, self.policy.max_help_bytes);
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
            fail_after_commit: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            commit_barrier: Mutex::new(None),
            #[cfg(test)]
            crash_after_commit: std::sync::atomic::AtomicBool::new(false),
            policy,
            listeners,
            payers,
            base: (*catalog.read()).clone(),
            catalog,
            inner: Mutex::new(Inner { state, store }),
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
            record.disabled = if record.format != FORMAT {
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
                        import::build(
                            &self.policy,
                            &record.candidate,
                            &record.id,
                            record.document.as_bytes(),
                        )
                        .map(|_| ())
                    })
                    .err()
                    .map(|_| "policy_or_catalog_invalid".into())
            };
        }
        let snapshot = match self.snapshot(&state) {
            Ok(s) => s,
            Err(_) => {
                for r in state.records.values_mut().filter(|r| !r.removed) {
                    r.disabled = Some("registration_conflict_or_quota".into());
                }
                self.snapshot(&state)?
            }
        };
        inner.state = state;
        self.catalog.publish(snapshot);
        Ok(())
    }
    fn snapshot(&self, state: &State) -> Result<CatalogSnapshot> {
        let live: Vec<_> = state.records.values().filter(|r| !r.removed).collect();
        // Disabled records occupy registration quota, but a revoked quota must not prevent startup.
        if live.iter().any(|r| r.disabled.is_none()) {
            ensure!(
                live.len() <= self.policy.max_sources,
                "source_quota_exceeded"
            );
        }
        ensure!(
            live.iter().map(|r| r.document.len()).sum::<usize>() <= 128 * 1024 * 1024,
            "registry_document_limit"
        );
        let mut result = self.base.clone();
        result.generation = state.generation;
        for record in live.into_iter().filter(|r| r.disabled.is_none()) {
            let built = import::build(
                &self.policy,
                &record.candidate,
                &record.id,
                record.document.as_bytes(),
            )?;
            let doc: Value = serde_json::from_str(&record.document)?;
            let ops = crate::catalog::operations(&doc, None)?;
            for target in &record.targets {
                let cfg = self.listeners.get(target).context("target_missing")?;
                let wallet = policy::wallet(&self.policy, cfg);
                let payer = self
                    .payers
                    .get(wallet)
                    .context("dynamic wallet unavailable")?
                    .clone()
                    .with_timeout(self.policy.read_timeout_seconds.map(Duration::from_secs))
                    .with_download_limits(
                        self.policy.max_response_bytes,
                        self.policy.max_help_bytes,
                    )
                    .public_destinations();
                let view = result.views.get_mut(target).context("target_missing")?;
                for tool in &built.tools {
                    if !crate::catalog::matches_operation_tags(
                        &ops,
                        tool,
                        &cfg.tags,
                        &cfg.exclude_tags,
                    ) || !import::matches(&tool.name, &cfg.include_tools, &cfg.exclude_tools)?
                    {
                        continue;
                    }
                    ensure!(
                        !view.iter().any(|t| t.tool.name == tool.name),
                        "tool_name_collision"
                    );
                    view.push(BoundTool {
                        tool: tool.clone(),
                        client: payer.clone(),
                        base: built.base.clone(),
                        source: Some((record.id.clone(), record.revision)),
                        help: Arc::new(OnceCell::new()),
                    });
                }
                ensure!(
                    view.iter().filter(|t| t.source.is_some()).count()
                        <= self.policy.max_tools_per_server,
                    "server_tool_quota_exceeded"
                );
            }
        }
        Ok(result)
    }
    fn result(&self, r: &Record, state: &State, caller: &str, snapshot: &CatalogSnapshot) -> Value {
        let targets: Vec<_> = r
            .targets
            .iter()
            .filter(|t| r.owner == caller || t.as_str() == caller)
            .cloned()
            .collect();
        let wallets: BTreeMap<_, _> = targets
            .iter()
            .filter_map(|t| {
                self.listeners
                    .get(t)
                    .map(|listener| (t.clone(), policy::wallet(&self.policy, listener).to_owned()))
            })
            .collect();
        json!({"source_id":r.id,"name":r.candidate.name,"revision":r.revision,"catalog_generation":state.generation,"targets":targets,"lifetime":r.candidate.lifetime,"disabled":r.disabled,"removed":r.removed,"wallet_profiles":wallets,"readiness":"payment readiness checked at invocation; registration does not fund wallets","spec_hash":r.hash,"accepted_at":r.accepted_at,"updated_at":r.updated_at,"tool_count":snapshot.views.get(caller).into_iter().flatten().filter(|t|t.source.as_ref().is_some_and(|s|s.0==r.id)).count(),"next":"Use x402_treazury_tools_search and x402_treazury_tool_call, or explicitly refresh your client's tool list."})
    }
    async fn document(&self, owner: &str, url: &str, refresh: bool) -> Result<Arc<Vec<u8>>> {
        let canonical = import::endpoint(&self.policy, url)?.to_string();
        let url = canonical.as_str();
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
            if refresh && cache.get(url).is_some_and(|cell| cell.get().is_some()) {
                cache.remove(url);
            }
            let max = (128 * 1024 * 1024 / self.policy.max_spec_bytes).min(8);
            if !cache.contains_key(url) && cache.len() >= max {
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
                    anyhow::bail!("imports_busy");
                }
            }
            cache
                .entry(url.into())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };
        let done = cell
            .get_or_init(|| async {
                // Only the initializer consumes import capacity; aliases merely
                // waiting for its result cannot starve independent imports.
                let result = async {
                    let owner = self.owner_import.get(owner).ok_or("unknown_owner")?;
                    let _owner = owner.try_acquire().map_err(|_| "owner_import_busy")?;
                    let _global = self
                        .global_import
                        .try_acquire()
                        .map_err(|_| "imports_busy")?;
                    self.fetch_document(url)
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

    async fn fetch_document(&self, url: &str) -> Result<Vec<u8>> {
        #[cfg(test)]
        {
            let fixture = self.fixture_endpoint.lock().unwrap().clone();
            if let Some(url) = fixture {
                return import::fixture_fetch(&self.policy, &url).await;
            }
        }
        import::fetch(&self.policy, url).await
    }
    pub async fn invoke(self: &Arc<Self>, owner: &str, name: &str, args: Value) -> Result<Value> {
        ensure!(
            serde_json::to_vec(&args)?.len() <= 65536,
            "management_request_too_large"
        );
        ensure!(
            self.enabled(owner) || self.accepts(owner),
            "source_management_disabled"
        );
        match name {
            "x402_treazury_source_add" => self.add_source(owner, args).await,
            "x402_treazury_tools_search" => self.search_tools(owner, name, args),
            _ => anyhow::bail!("unknown management tool"),
        }
    }
    async fn add_source(self: &Arc<Self>, owner: &str, args: Value) -> Result<Value> {
        let mut a: Add = serde_json::from_value(args)?;
        ensure!(self.enabled(owner), "source_management_disabled");
        a.spec_url = import::endpoint(&self.policy, &a.spec_url)?.to_string();
        let removed = {
            let inner = self.inner.lock().unwrap();
            if let Some(r) = inner
                .state
                .records
                .values()
                .find(|r| r.owner == owner && !r.removed && r.candidate.spec_url == a.spec_url)
            {
                return Ok(self.result(r, &inner.state, owner, &self.catalog.read()));
            }
            inner
                .state
                .records
                .values()
                .filter(|r| r.owner == owner && r.removed && r.candidate.spec_url == a.spec_url)
                .count()
        };
        let args = serde_json::to_value(&a)?;
        let key = format!("add:{}:{removed}", hash(a.spec_url.as_bytes()));
        let candidate = Candidate {
            name: a
                .name
                .unwrap_or_else(|| format!("api_{}", &hash(a.spec_url.as_bytes())[..16])),
            spec_url: a.spec_url,
            base_url: None,
            selection: Selection::default(),
            visibility: Visibility::Server,
            targets: vec![],
            lifetime: if self.policy.registry_file.is_some() {
                Lifetime::Persistent
            } else {
                Lifetime::Process
            },
        };
        self.targets(owner, &candidate)?;
        let document = self.document(owner, &candidate.spec_url, false).await?;
        let id = Uuid::new_v4().to_string();
        import::build(&self.policy, &candidate, &id, &document)?;
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
        self.commit(owner.to_owned(), key, args, record).await
    }

    fn search_tools(self: &Arc<Self>, owner: &str, name: &str, args: Value) -> Result<Value> {
        search_catalog(&self.catalog, owner, name, args)
    }

    async fn commit(
        self: &Arc<Self>,
        owner: String,
        key: String,
        args: Value,
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
            .context("mutation_queue_full")?;
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut inner = this.inner.lock().unwrap();
            // Recheck duplicate URLs under the publication lock, including racing adds.
            if let Some(r) = inner.state.records.values().find(|r| {
                !r.removed && r.owner == owner && r.candidate.spec_url == record.candidate.spec_url
            }) {
                return Ok(this.result(r, &inner.state, &owner, &this.catalog.read()));
            }
            let mut state = inner.state.clone();
            ensure!(
                !state.records.values().any(|r| !r.removed
                    && r.owner == owner
                    && r.candidate.name == record.candidate.name),
                "source_name_reserved"
            );
            record.targets = this.targets(&owner, &record.candidate)?;
            let id = record.id.clone();
            let persistent = record.candidate.lifetime == Lifetime::Persistent;
            state.records.insert(id.clone(), record);
            state.generation = state
                .generation
                .checked_add(1)
                .context("generation exhausted")?;
            let snapshot = this.snapshot(&state)?;
            let result = this.result(&state.records[&id], &state, &owner, &snapshot);
            ensure!(state.receipts.len() < 10000, "idempotency_capacity_reached");
            state.receipts.insert(
                receipt_key(&owner, &key)?,
                Receipt {
                    hash: hash(&serde_json::to_vec(&args)?),
                    result: result.clone(),
                    persistent,
                },
            );
            if let Some(store) = &mut inner.store {
                let mut saved = state.clone();
                saved
                    .records
                    .retain(|_, r| r.candidate.lifetime == Lifetime::Persistent);
                saved.receipts.retain(|_, r| r.persistent);
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
pub(crate) fn catalog_reference(catalog: &CatalogState, owner: &str, bound: &BoundTool) -> String {
    hash(
        &serde_json::to_vec(&(catalog.instance(), owner, &bound.tool.name, &bound.source))
            .expect("serializable tool reference"),
    )
}
pub(crate) fn find_catalog_reference(
    catalog: &CatalogState,
    owner: &str,
    reference: &str,
) -> Result<BoundTool> {
    let snapshot = catalog.read();
    snapshot
        .views
        .get(owner)
        .into_iter()
        .flatten()
        .find(|b| catalog_reference(catalog, owner, b) == reference)
        .cloned()
        .context("tool_reference_stale_or_unknown: search tools again")
}
pub(crate) fn search_catalog(
    catalog: &CatalogState,
    owner: &str,
    name: &str,
    args: Value,
) -> Result<Value> {
    ensure!(
        serde_json::to_vec(&args)?.len() <= 65536,
        "management_request_too_large"
    );
    let q: Query = serde_json::from_value(args)?;
    let snapshot = catalog.read();
    let query = q.query.as_ref().map(|q| q.to_lowercase());
    let values = snapshot
        .views
        .get(owner)
        .into_iter()
        .flatten()
        .filter(|t| {
            q.source_id
                .as_ref()
                .is_none_or(|id| t.source.as_ref().is_some_and(|s| &s.0 == id))
        })
        .filter(|t| {
            query.as_ref().is_none_or(|q| {
                format!("{} {}", t.tool.name, t.tool.description)
                    .to_lowercase()
                    .contains(q)
            })
        })
        .map(|t| {
            json!({
                "tool_ref": catalog_reference(catalog, owner, t),
                "tool_id": t.tool.name,
                "source_id": t.source.as_ref().map(|s| &s.0),
                "revision": t.source.as_ref().map_or(0, |s| s.1),
                "description": t.tool.description,
                "input_schema": t.tool.input_schema
            })
        })
        .collect();
    page(
        values,
        &q,
        snapshot.generation,
        &format!(
            "{}:{owner}:{name}:{:?}:{:?}",
            catalog.instance(),
            q.source_id,
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
fn receipt_key(owner: &str, key: &str) -> Result<String> {
    ensure!(
        !key.is_empty() && key.len() <= 256,
        "invalid idempotency key"
    );
    Ok(format!("{}:{owner}{key}", owner.len()))
}
fn page(values: Vec<Value>, query: &Query, generation: u64, scope: &str) -> Result<Value> {
    let limit = query.limit.unwrap_or(20);
    ensure!((1..=100).contains(&limit), "invalid page size");
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
    Ok(
        json!({"items":values[offset..end],"catalog_generation":generation,"next_cursor":(end<values.len()).then(||format!("{prefix}{end}"))}),
    )
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
    crate::deployment::Deployment::show_config(path).await?;
    let mut cfg: crate::deployment::MetaConfig = toml::from_str(&std::fs::read_to_string(path)?)?;
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
        let bytes = import::fetch(&policy, &record.candidate.spec_url).await?;
        import::build(&policy, &record.candidate, &record.id, &bytes)?;
        record.hash = hash(&bytes);
        record.document = String::from_utf8(bytes)?;
        record.format = FORMAT;
        record.disabled = None;
    } else {
        record.removed = true;
        record.document.clear();
    }
    record.revision = record
        .revision
        .checked_add(1)
        .context("revision exhausted")?;
    record.updated_at = epoch();
    let result = json!({"source_id":record.id,"server":owner,"revision":record.revision,"removed":record.removed});
    ensure!(
        state
            .records
            .values()
            .map(|r| r.document.len())
            .sum::<usize>()
            <= 128 * 1024 * 1024,
        "registry_document_limit"
    );
    if refresh {
        let listener = &cfg.servers[owner];
        let mut tools = 0;
        for r in state
            .records
            .values()
            .filter(|r| !r.removed && r.disabled.is_none() && r.owner == owner)
        {
            let built = import::build(&policy, &r.candidate, &r.id, r.document.as_bytes())?;
            let document: Value = serde_json::from_str(&r.document)?;
            let operations = crate::catalog::operations(&document, None)?;
            for tool in &built.tools {
                if crate::catalog::matches_operation_tags(
                    &operations,
                    tool,
                    &listener.tags,
                    &listener.exclude_tags,
                ) && import::matches(
                    &tool.name,
                    &listener.include_tools,
                    &listener.exclude_tools,
                )? {
                    tools += 1;
                }
            }
        }
        ensure!(
            tools <= policy.max_tools_per_server,
            "server_tool_quota_exceeded"
        );
    }
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
