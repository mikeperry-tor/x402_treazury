//! Operator-authorized source registration; all paid work stays in the common dispatcher.
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
struct Preview {
    owner: String,
    candidate: Candidate,
    document: Arc<Vec<u8>>,
    created: Instant,
    id: String,
}
type FetchCell = Arc<OnceCell<std::result::Result<Arc<Vec<u8>>, String>>>;
pub struct Manager {
    #[cfg(test)]
    commit_barrier: Mutex<Option<Arc<tokio::sync::Barrier>>>,
    #[cfg(test)]
    fixture_endpoint: Mutex<Option<String>>,
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
    previews: Mutex<BTreeMap<String, Preview>>,
    fetches: Mutex<BTreeMap<String, (Instant, FetchCell)>>,
    global_import: Arc<Semaphore>,
    owner_import: BTreeMap<String, Arc<Semaphore>>,
    mutations: Arc<Semaphore>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Add {
    candidate: Candidate,
    idempotency_key: String,
    preview_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    name: Option<String>,
    source_id: String,
    expected_revision: u64,
    idempotency_key: String,
    selection: Option<Selection>,
    visibility: Option<Visibility>,
    targets: Option<Vec<String>>,
    lifetime: Option<Lifetime>,
    #[serde(default)]
    refresh_spec: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Remove {
    source_id: String,
    expected_revision: u64,
    idempotency_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewRequest {
    candidate: Option<Candidate>,
    preview_id: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
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
    pub tool_id: String,
    pub arguments: serde_json::Map<String, Value>,
    pub expected_revision: u64,
}
impl Manager {
    fn grant(&self, owner: &str) -> policy::Grant {
        self.listeners
            .get(owner)
            .and_then(|l| l.source_management.clone())
            .unwrap_or_default()
    }
    pub fn accepts(&self, id: &str) -> bool {
        self.grant(id).accept_sources
    }
    pub fn enabled(&self, id: &str) -> bool {
        self.grant(id).enabled
    }
    fn targets(&self, owner: &str, c: &Candidate) -> Result<Vec<String>> {
        ensure!(
            !self.listeners.values().any(|l| l.sources.contains(&c.name)),
            "source_name_reserved"
        );
        let g = self.grant(owner);
        ensure!(g.enabled, "source_management_disabled");
        ensure!(
            c.lifetime != Lifetime::Persistent
                || (g.allow_persistence && self.policy.registry_file.is_some()),
            "persistence_not_authorized"
        );
        let mut targets = match c.visibility {
            Visibility::Server => {
                ensure!(c.targets.is_empty(), "server visibility takes no targets");
                vec![owner.into()]
            }
            Visibility::Servers => {
                ensure!(!c.targets.is_empty(), "targets required");
                c.targets.clone()
            }
            Visibility::Process => {
                ensure!(
                    g.allow_process_scope && c.targets.is_empty(),
                    "process_scope_not_authorized"
                );
                self.listeners
                    .keys()
                    .filter(|id| self.accepts(id))
                    .cloned()
                    .collect()
            }
        };
        let allowed = g.allowed_targets.unwrap_or_else(|| vec![owner.into()]);
        ensure!(
            targets
                .iter()
                .all(|t| allowed.contains(t) && self.accepts(t)),
            "target_not_authorized"
        );
        targets.sort();
        targets.dedup();
        ensure!(!targets.is_empty(), "no eligible targets");
        Ok(targets)
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
            previews: Mutex::new(BTreeMap::new()),
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
            let mut c = record.candidate.clone();
            c.visibility = Visibility::Servers;
            c.targets = record.targets.clone();
            record.disabled = if record.format != FORMAT {
                Some("catalog_format_changed: explicit refresh required".into())
            } else if !self.listeners.contains_key(&record.owner) {
                Some("owner_removed".into())
            } else {
                self.targets(&record.owner, &c)
                    .and_then(|_| {
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
            for owner in self.listeners.keys() {
                ensure!(
                    live.iter().filter(|r| r.owner == *owner).count()
                        <= self.grant(owner).max_owned_sources,
                    "owner_source_quota_exceeded"
                );
            }
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
                let g = self.grant(target);
                let wallet = policy::wallet(&self.policy, &g);
                let payer = self
                    .payers
                    .get(wallet)
                    .context("dynamic wallet unavailable")?
                    .clone()
                    .with_timeout(Duration::from_secs(self.policy.fetch_timeout_seconds))
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
            .map(|t| {
                let g = self.grant(t);
                (t.clone(), policy::wallet(&self.policy, &g).to_owned())
            })
            .collect();
        json!({"source_id":r.id,"name":r.candidate.name,"revision":r.revision,"catalog_generation":state.generation,"targets":targets,"lifetime":r.candidate.lifetime,"disabled":r.disabled,"removed":r.removed,"can_manage":r.owner==caller&&self.enabled(caller),"wallet_profiles":wallets,"readiness":"payment readiness checked at invocation; registration does not fund wallets","spec_hash":r.hash,"accepted_at":r.accepted_at,"updated_at":r.updated_at,"tool_count":snapshot.views.get(caller).into_iter().flatten().filter(|t|t.source.as_ref().is_some_and(|s|s.0==r.id)).count(),"next":"Use treazury_tools_search and treazury_tool_call, or explicitly refresh your client's tool list."})
    }
    async fn document(&self, owner: &str, url: &str, refresh: bool) -> Result<Arc<Vec<u8>>> {
        let canonical = import::endpoint(&self.policy, url)?.to_string();
        let url = canonical.as_str();
        let _owner = self
            .owner_import
            .get(owner)
            .context("unknown owner")?
            .clone()
            .try_acquire_owned()
            .context("owner_import_busy")?;
        let _global = self
            .global_import
            .clone()
            .try_acquire_owned()
            .context("imports_busy")?;
        let cell = {
            let mut cache = self.fetches.lock().unwrap();
            cache.retain(|_, (time, _)| time.elapsed() < Duration::from_secs(300));
            if refresh && cache.get(url).is_some_and(|(_, cell)| cell.get().is_some()) {
                cache.remove(url);
            }
            let max = (128 * 1024 * 1024 / self.policy.max_spec_bytes).min(8);
            if !cache.contains_key(url) && cache.len() >= max {
                cache.clear();
            }
            cache
                .entry(url.into())
                .or_insert_with(|| (Instant::now(), Arc::new(OnceCell::new())))
                .1
                .clone()
        };
        cell.get_or_init(|| async {
            self.fetch_document(url)
                .await
                .map(Arc::new)
                .map_err(|_| "source_fetch_failed_or_rejected".into())
        })
        .await
        .clone()
        .map_err(anyhow::Error::msg)
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
            "treazury_sources_list" => self.list_sources(owner, name, args),
            "treazury_source_preview" => self.preview_source(owner, args).await,
            "treazury_source_add" => self.add_source(owner, name, args).await,
            "treazury_source_update" => self.update_source(owner, name, args).await,
            "treazury_source_remove" => self.remove_source(owner, name, args).await,
            "treazury_tools_search" => self.search_tools(owner, name, args),
            _ => anyhow::bail!("unknown management tool"),
        }
    }

    fn list_sources(self: &Arc<Self>, owner: &str, name: &str, args: Value) -> Result<Value> {
        let q: Query = serde_json::from_value(args)?;
        let inner = self.inner.lock().unwrap();
        let values = inner
            .state
            .records
            .values()
            .filter(|r| !r.removed && (r.owner == owner || r.targets.iter().any(|t| t == owner)))
            .filter(|r| q.source_id.as_ref().is_none_or(|id| id == &r.id))
            .filter(|r| {
                q.query
                    .as_ref()
                    .is_none_or(|q| r.candidate.name.to_lowercase().contains(&q.to_lowercase()))
            })
            .map(|r| self.result(r, &inner.state, owner, &self.catalog.read()))
            .collect();
        page(
            values,
            &q,
            inner.state.generation,
            &format!(
                "{}:{owner}:{name}:{:?}:{:?}",
                self.catalog.instance(),
                q.source_id,
                q.query
            ),
        )
    }

    async fn preview_source(self: &Arc<Self>, owner: &str, args: Value) -> Result<Value> {
        let request: PreviewRequest = serde_json::from_value(args)?;
        let preview_id = if let Some(id) = request.preview_id {
            ensure!(request.candidate.is_none(), "preview_id excludes candidate");
            id
        } else {
            ensure!(request.cursor.is_none(), "cursor requires preview_id");
            let candidate = request.candidate.context("candidate required")?;
            self.targets(owner, &candidate)?;
            let document = self.document(owner, &candidate.spec_url, false).await?;
            let id = Uuid::new_v4().to_string();
            import::build(&self.policy, &candidate, &id, &document)?;
            let preview_id = Uuid::new_v4().to_string();
            let mut cache = self.previews.lock().unwrap();
            cache.retain(|_, p| p.created.elapsed() < Duration::from_secs(300));
            while cache.len() >= 8
                || cache.values().map(|p| p.document.len()).sum::<usize>() + document.len()
                    > 128 * 1024 * 1024
            {
                let key = cache.keys().next().cloned().context("preview_too_large")?;
                cache.remove(&key);
            }
            cache.insert(
                preview_id.clone(),
                Preview {
                    owner: owner.into(),
                    candidate,
                    document,
                    created: Instant::now(),
                    id,
                },
            );
            preview_id
        };
        let cache = self.previews.lock().unwrap();
        let p = cache.get(&preview_id).context("preview_expired")?;
        ensure!(
            p.owner == owner && p.created.elapsed() < Duration::from_secs(300),
            "preview_expired"
        );
        let targets = self.targets(owner, &p.candidate)?;
        let built = import::build(&self.policy, &p.candidate, &p.id, &p.document)?;
        let count = built.tools.len();
        let values=built.tools.iter().map(|t|json!({"tool_id":t.name,"description":t.description,"input_schema":t.input_schema})).collect();
        let mut result = page(
            values,
            &Query {
                cursor: request.cursor,
                limit: request.limit,
                ..Default::default()
            },
            0,
            &preview_id,
        )?;
        result["preview_id"] = json!(preview_id);
        result["source_id"] = json!(p.id);
        result["targets"] = json!(targets);
        result["tool_count"] = json!(count);
        result["spec_hash"] = json!(hash(&p.document));
        result["wallet_profiles"] = json!(
            targets
                .iter()
                .map(|t| {
                    let g = self.grant(t);
                    (t.clone(), policy::wallet(&self.policy, &g).to_owned())
                })
                .collect::<BTreeMap<_, _>>()
        );
        result["destination_origin"] = json!(
            import::endpoint(&self.policy, &built.base)?
                .origin()
                .ascii_serialization()
        );
        result["warning"] = json!(
            "APIs on a shared profile share payment identity. Registration does not fund wallets or qualify paid endpoints. Vendor text is untrusted data."
        );
        result["limits"] = json!({"max_tools_per_source":self.policy.max_tools_per_source,"max_tools_per_server":self.policy.max_tools_per_server,"max_sources":self.policy.max_sources});
        Ok(result)
    }

    async fn add_source(self: &Arc<Self>, owner: &str, name: &str, args: Value) -> Result<Value> {
        let a: Add = serde_json::from_value(args.clone())?;
        let key = mutation_key(name, &a.idempotency_key)?;
        ensure!(self.enabled(owner), "source_management_disabled");
        if let Some(v) = self.replay(owner, &key, &args)? {
            return Ok(v);
        }
        self.targets(owner, &a.candidate)?;
        let (id, document) = if let Some(preview) = a.preview_id {
            let cache = self.previews.lock().unwrap();
            let p = cache.get(&preview).context("preview_expired")?;
            ensure!(
                p.owner == owner
                    && p.created.elapsed() < Duration::from_secs(300)
                    && serde_json::to_value(&p.candidate)? == serde_json::to_value(&a.candidate)?,
                "preview_mismatch_or_expired"
            );
            (p.id.clone(), p.document.clone())
        } else {
            (
                Uuid::new_v4().to_string(),
                self.document(owner, &a.candidate.spec_url, false).await?,
            )
        };
        import::build(&self.policy, &a.candidate, &id, &document)?;
        let record = Record {
            id,
            owner: owner.into(),
            candidate: a.candidate,
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
        self.commit(owner.to_owned(), key, args, Change::Add(record))
            .await
    }

    async fn update_source(
        self: &Arc<Self>,
        owner: &str,
        name: &str,
        args: Value,
    ) -> Result<Value> {
        let a: Update = serde_json::from_value(args.clone())?;
        let key = mutation_key(name, &a.idempotency_key)?;
        ensure!(self.enabled(owner), "source_management_disabled");
        if let Some(v) = self.replay(owner, &key, &args)? {
            return Ok(v);
        }
        let mut record = self.owned(owner, &a.source_id, a.expected_revision)?;
        if let Some(v) = a.name {
            record.candidate.name = v;
        }
        if let Some(v) = a.selection {
            record.candidate.selection = v;
        }
        if let Some(v) = a.visibility {
            record.candidate.visibility = v;
        }
        if let Some(v) = a.targets {
            record.candidate.targets = v;
        }
        if let Some(v) = a.lifetime {
            record.candidate.lifetime = v;
        }
        // Existing resolved process targets are stable until visibility/targets explicitly change.
        let mut authorized = record.candidate.clone();
        if args.get("visibility").is_none() && args.get("targets").is_none() {
            authorized.visibility = Visibility::Servers;
            authorized.targets = record.targets.clone();
        }
        self.targets(owner, &authorized)?;
        if a.refresh_spec {
            let doc = self
                .document(owner, &record.candidate.spec_url, true)
                .await?;
            record.document = String::from_utf8((*doc).clone())?;
            record.hash = hash(&doc);
            record.format = FORMAT;
        }
        ensure!(
            record.format == FORMAT,
            "catalog_format_changed: refresh required"
        );
        record.disabled = None;
        self.commit(
            owner.into(),
            key,
            args,
            Change::Update(record, a.expected_revision),
        )
        .await
    }

    async fn remove_source(
        self: &Arc<Self>,
        owner: &str,
        name: &str,
        args: Value,
    ) -> Result<Value> {
        let a: Remove = serde_json::from_value(args.clone())?;
        let key = mutation_key(name, &a.idempotency_key)?;
        ensure!(self.enabled(owner), "source_management_disabled");
        self.commit(
            owner.into(),
            key,
            args,
            Change::Remove(a.source_id, a.expected_revision),
        )
        .await
    }

    fn search_tools(self: &Arc<Self>, owner: &str, name: &str, args: Value) -> Result<Value> {
        let q: Query = serde_json::from_value(args)?;
        let snapshot = self.catalog.read();
        let values=snapshot.views.get(owner).into_iter().flatten().filter(|t|q.source_id.as_ref().is_none_or(|id|t.source.as_ref().is_some_and(|s|&s.0==id))).filter(|t|q.query.as_ref().is_none_or(|q|format!("{} {}",t.tool.name,t.tool.description).to_lowercase().contains(&q.to_lowercase()))).map(|t|json!({"tool_id":t.tool.name,"source_id":t.source.as_ref().map(|s|&s.0),"revision":t.source.as_ref().map_or(0,|s|s.1),"description":t.tool.description,"input_schema":t.tool.input_schema})).collect();
        page(
            values,
            &q,
            snapshot.generation,
            &format!(
                "{}:{owner}:{name}:{:?}:{:?}",
                self.catalog.instance(),
                q.source_id,
                q.query
            ),
        )
    }

    fn owned(&self, owner: &str, id: &str, revision: u64) -> Result<Record> {
        let inner = self.inner.lock().unwrap();
        owned(&inner.state, owner, id, revision).cloned()
    }
    fn replay(&self, owner: &str, key: &str, args: &Value) -> Result<Option<Value>> {
        let inner = self.inner.lock().unwrap();
        replay(&inner.state, owner, key, args)
    }
    async fn commit(
        self: &Arc<Self>,
        owner: String,
        key: String,
        args: Value,
        change: Change,
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
            if let Some(v) = replay(&inner.state, &owner, &key, &args)? {
                return Ok(v);
            }
            let mut state = inner.state.clone();
            let (id, persistent) = match change {
                Change::Add(mut r) => {
                    ensure!(
                        !state
                            .records
                            .values()
                            .any(|v| v.owner == owner && v.candidate.name == r.candidate.name),
                        "source_name_reserved"
                    );
                    r.targets = this.targets(&owner, &r.candidate)?;
                    let id = r.id.clone();
                    let persistent = r.candidate.lifetime == Lifetime::Persistent;
                    state.records.insert(id.clone(), r);
                    (id, persistent)
                }
                Change::Update(mut r, revision) => {
                    ensure!(
                        !state.records.values().any(|v| v.id != r.id
                            && v.owner == owner
                            && v.candidate.name == r.candidate.name),
                        "source_name_reserved"
                    );
                    let old = owned(&state, &owner, &r.id, revision)?;
                    let persistent = old.candidate.lifetime == Lifetime::Persistent
                        || r.candidate.lifetime == Lifetime::Persistent;
                    if args.get("visibility").is_some() || args.get("targets").is_some() {
                        r.targets = this.targets(&owner, &r.candidate)?;
                    } else {
                        let mut c = r.candidate.clone();
                        c.visibility = Visibility::Servers;
                        c.targets = r.targets.clone();
                        this.targets(&owner, &c)?;
                    }
                    r.updated_at = epoch();
                    r.revision = revision.checked_add(1).context("revision exhausted")?;
                    let id = r.id.clone();
                    state.records.insert(id.clone(), r);
                    (id, persistent)
                }
                Change::Remove(id, revision) => {
                    let r = owned(&state, &owner, &id, revision)?;
                    let persistent = r.candidate.lifetime == Lifetime::Persistent;
                    let r = state.records.get_mut(&id).unwrap();
                    r.removed = true;
                    r.document.clear();
                    r.updated_at = epoch();
                    r.revision = revision.checked_add(1).context("revision exhausted")?;
                    (id, persistent)
                }
            };
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
                // Persistent-to-process withdrawal must retain its UUID/name tombstone, not active access.
                for r in state
                    .records
                    .values()
                    .filter(|r| r.candidate.lifetime == Lifetime::Process)
                {
                    if state
                        .receipts
                        .values()
                        .any(|v| v.persistent && v.result["source_id"] == r.id)
                    {
                        let mut tombstone = r.clone();
                        tombstone.removed = true;
                        tombstone.document.clear();
                        saved.records.insert(r.id.clone(), tombstone);
                    }
                }
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
enum Change {
    Add(Record),
    Update(Record, u64),
    Remove(String, u64),
}
fn owned<'a>(state: &'a State, owner: &str, id: &str, revision: u64) -> Result<&'a Record> {
    let r = state
        .records
        .get(id)
        .filter(|r| r.owner == owner && !r.removed)
        .context("source_not_manageable")?;
    ensure!(r.revision == revision, "source_revision_conflict");
    Ok(r)
}
fn epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn mutation_key(operation: &str, key: &str) -> Result<String> {
    ensure!(
        !key.is_empty() && key.len() <= 128,
        "invalid idempotency key"
    );
    Ok(format!("{operation}:{key}"))
}
fn receipt_key(owner: &str, key: &str) -> Result<String> {
    ensure!(
        !key.is_empty() && key.len() <= 256,
        "invalid idempotency key"
    );
    Ok(format!("{}:{owner}{key}", owner.len()))
}
fn replay(state: &State, owner: &str, key: &str, args: &Value) -> Result<Option<Value>> {
    if let Some(r) = state.receipts.get(&receipt_key(owner, key)?) {
        ensure!(
            r.hash == hash(&serde_json::to_vec(args)?),
            "idempotency_conflict"
        );
        return Ok(Some(r.result.clone()));
    }
    Ok(None)
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
pub async fn inspect_cli() -> Result<()> {
    use clap::Parser;
    #[derive(Parser)]
    struct Args {
        #[command(subcommand)]
        command: Command,
    }
    #[derive(clap::Subcommand)]
    enum Command {
        Inspect {
            #[arg(long = "config", alias = "meta-config")]
            meta_config: PathBuf,
        },
    }
    let Args {
        command: Command::Inspect { meta_config },
    } = Args::parse_from(
        std::iter::once("x402_treazury sources".to_owned()).chain(std::env::args().skip(2)),
    );
    let cfg: crate::deployment::MetaConfig =
        toml::from_str(&std::fs::read_to_string(&meta_config)?)?;
    let mut policy = cfg
        .source_management
        .context("source management not configured")?;
    policy.resolve(&meta_config);
    let path = policy
        .registry_file
        .context("persistent registry not configured")?;
    let value = tokio::task::spawn_blocking(move || store::inspect(&path)).await??;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

#[cfg(test)]
mod tests;
