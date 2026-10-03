//! Opt-in consensus tests. Public mnemonic, disposable Docker state, no peers.
use super::*;
use crate::rotation::transaction::{
    PrepareRequest, SubmissionOutcome, TransactionPreparer, TransactionPresence,
};
use serde_json::{Value, json};
use std::process::Command;
use zcash_local_net::{rpc_client::RpcRequestClient, zebra_rpc};

const ZEBRA: &str = "docker.io/zfnd/zebra:6.0.0@sha256:78a10b7f24b83a86e6223d97e857094a353454e3268f84a87bd987e7140a33bb";
const ZAINO: &str = "docker.io/zingodevops/zaino:0.6.0-rc.1-no-tls@sha256:e48b133dbf53dbed77b872de74d65aa8a45357a3844b658ea472a340949feb7f";
const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

pub(super) fn heights() -> zingolib::ActivationHeights {
    zingolib::ActivationHeights::builder()
        .set_overwinter(Some(1))
        .set_sapling(Some(1))
        .set_blossom(Some(1))
        .set_heartwood(Some(1))
        .set_canopy(Some(1))
        .set_nu5(Some(2))
        .set_nu6(Some(2))
        .set_nu6_1(Some(5))
        .set_nu6_2(Some(5))
        .set_nu6_3(Some(5))
        .set_nu7(None)
        .build()
}
fn docker(args: &[&str]) -> Result<String> {
    let out = Command::new("docker").args(args).output()?;
    ensure!(
        out.status.success(),
        "docker {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut text = String::from_utf8(out.stdout)?;
    if args.first() == Some(&"logs") {
        text.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    Ok(text.trim().to_string())
}
fn budget(state: &Path, operation: &str) -> Result<(i64, i64)> {
    let db = rusqlite::Connection::open_with_flags(
        state.join("state.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    Ok(db.query_row(
        "SELECT reserved,consumed FROM budget_entries WHERE id=?1",
        [operation],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}
struct Chain {
    passed: bool,
    prefix: String,
    _dir: tempfile::TempDir,
    rpc: RpcRequestClient,
    fork: RpcRequestClient,
    endpoint: String,
}
impl Drop for Chain {
    fn drop(&mut self) {
        for suffix in ["indexer", "fork", "node"] {
            let name = format!("{}-{suffix}", self.prefix);
            if !self.passed
                && let Ok(logs) = docker(&["logs", &name])
            {
                eprintln!(
                    "{name}: {}",
                    logs.lines().take(25).collect::<Vec<_>>().join("\n")
                );
                let omitted = logs.lines().count().saturating_sub(25);
                if omitted > 0 {
                    eprintln!(
                        "{name}: [diagnostic log truncated at 25 lines; {omitted} lines omitted]"
                    );
                }
            }
            let _ = docker(&["rm", "-f", &name]);
        }
        let _ = docker(&["network", "rm", &self.prefix]);
    }
}
impl Chain {
    async fn launch(miner: &str) -> Result<Self> {
        let prefix = format!("x402-regtest-{}", uuid::Uuid::new_v4());
        let dir = tempfile::tempdir()?;
        // Install cleanup before creating any external resources.
        let mut chain = Self {
            passed: false,
            prefix,
            _dir: dir,
            rpc: RpcRequestClient::new("127.0.0.1:1".parse()?),
            fork: RpcRequestClient::new("127.0.0.1:1".parse()?),
            endpoint: String::new(),
        };
        docker(&["network", "create", &chain.prefix])?;
        let zebra_config = format!(
            r#"
[network]
network = "Regtest"
listen_addr = "127.0.0.1:0"
initial_mainnet_peers = []
initial_testnet_peers = []
[rpc]
listen_addr = "0.0.0.0:18232"
indexer_listen_addr = "127.0.0.1:18234"
enable_cookie_auth = false
[state]
cache_dir = "/tmp/chain"
[tracing]
filter = "warn"
[mining]
miner_address = "{miner}"
[network.testnet_parameters.activation_heights]
Canopy = 1
NU5 = 2
NU6 = 2
"NU6.1" = 5
"NU6.2" = 5
"NU6.3" = 5
[[network.testnet_parameters.lockbox_disbursements]]
address = "t2RnBRiqrN1nW4ecZs1Fj3WWjNdnSs4kiX8"
amount = 1
[network.testnet_parameters.post_nu6_funding_streams.height_range]
start = 2
end = 5
[[network.testnet_parameters.post_nu6_funding_streams.recipients]]
receiver = "Deferred"
numerator = 1
"#
        );
        std::fs::write(chain._dir.path().join("zebra.toml"), &zebra_config)?;
        std::fs::write(
            chain._dir.path().join("fork.toml"),
            zebra_config.replace(miner, "t2RnBRiqrN1nW4ecZs1Fj3WWjNdnSs4kiX8"),
        )?;
        std::fs::write(
            chain._dir.path().join("zaino.toml"),
            r#"
backend = "fetch"
network = "Regtest"
[grpc_settings]
listen_address = "0.0.0.0:18233"
[validator_settings]
validator_jsonrpc_listen_address = "127.0.0.1:18232"
validator_user = "xxxxxx"
validator_password = "xxxxxx"
[storage]
database.path = "/tmp/indexer"
"#,
        )?;
        let mount = format!("{}:/config:ro", chain._dir.path().display());
        for suffix in ["node", "fork"] {
            let name = format!("{}-{suffix}", chain.prefix);
            docker(&[
                "run",
                "--pull",
                "never",
                "-d",
                "--name",
                &name,
                "--network",
                &chain.prefix,
                "-p",
                "127.0.0.1::18232",
                "-p",
                "127.0.0.1::18233",
                "-v",
                &mount,
                "--entrypoint",
                "/usr/local/bin/zebrad",
                ZEBRA,
                "-c",
                if suffix == "node" {
                    "/config/zebra.toml"
                } else {
                    "/config/fork.toml"
                },
                "start",
            ])?;
            let port = docker(&["port", &name, "18232/tcp"])?;
            let rpc = RpcRequestClient::new(port.parse()?);
            if suffix == "node" {
                chain.rpc = rpc;
                chain.endpoint = format!("http://{}", docker(&["port", &name, "18233/tcp"])?);
            } else {
                chain.fork = rpc;
            }
        }
        chain.wait_rpc(&chain.rpc).await?;
        chain.wait_rpc(&chain.fork).await?;
        Self::mine(&chain.fork, 1).await?;
        Self::copy(&chain.fork, &chain.rpc, 1, 1).await?;
        // Shared network namespace; only node's loopback-published gRPC port is exposed.
        docker(&[
            "run",
            "--pull",
            "never",
            "-d",
            "--platform",
            "linux/amd64",
            "--name",
            &format!("{}-indexer", chain.prefix),
            "--network",
            &format!("container:{}-node", chain.prefix),
            "-v",
            &mount,
            "--entrypoint",
            "/usr/local/bin/zainod",
            ZAINO,
            "start",
            "--config",
            "/config/zaino.toml",
        ])?;
        Ok(chain)
    }
    async fn wait_rpc(&self, rpc: &RpcRequestClient) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(90), async {
            loop {
                if rpc
                    .json_result_from_call::<Value>("getblockchaininfo", "[]")
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .context("regtest validator startup timed out")
    }
    async fn mine(rpc: &RpcRequestClient, count: usize) -> Result<()> {
        for _ in 0..count {
            let block = tokio::time::timeout(
                Duration::from_secs(90),
                zebra_rpc::submit_template_block(
                    rpc,
                    &zcash_local_net::validator::regtest_test_activation_heights(),
                ),
            )
            .await??;
            let response: Value = serde_json::from_str(&block.response)?;
            ensure!(
                response["error"].is_null() && response["result"].is_null(),
                "mining rejected: {response}"
            );
        }
        Ok(())
    }
    async fn copy(
        from: &RpcRequestClient,
        to: &RpcRequestClient,
        start: u32,
        end: u32,
    ) -> Result<()> {
        for height in start..=end {
            let raw: String = from
                .json_result_from_call("getblock", json!([height.to_string(), 0]).to_string())
                .await?;
            let result: Value = to
                .json_result_from_call("submitblock", json!([raw]).to_string())
                .await?;
            ensure!(result.is_null(), "fork block rejected: {result}");
        }
        Ok(())
    }
    async fn indexer_ready(&self, height: u64) -> Result<()> {
        use zingo_netutils::{GrpcIndexer, Indexer, lightwallet_protocol::BlockId};
        tokio::time::timeout(Duration::from_secs(90), async {
            loop {
                if let Ok(mut client) = GrpcIndexer::new(self.endpoint.parse().unwrap()).await
                    && let Ok(info) = client.get_lightd_info(Duration::from_secs(2)).await
                    && info.block_height == height
                    && let Ok(tip) = client.get_latest_block(Duration::from_secs(2)).await
                    && tip.height == height
                    && let Ok(block) = client
                        .get_block(
                            BlockId {
                                height,
                                hash: vec![],
                            },
                            Duration::from_secs(2),
                        )
                        .await
                    && block.height == height
                {
                    assert_eq!(info.chain_name, TreasuryNetwork::Regtest.rpc_name());
                    return;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .context("regtest indexer startup/tip timed out")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Docker and the pinned Zebra/Zaino images; mines only an isolated regtest chain"]
async fn deposit_settlement_and_reorg_recovery() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(900), run())
        .await
        .context("regtest qualification timed out")?
}
async fn run() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn")
        .with_writer(std::io::stderr)
        .try_init();
    let dir = tempfile::tempdir()?;
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let mut treasury = Treasury::create_with_network(
        state.clone(),
        key.clone(),
        1,
        Some(Zeroizing::new(SEED.into())),
        TreasuryNetwork::Regtest,
    )
    .await?;
    let id = treasury.status().await?.treasury_id;
    let miner = treasury
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut chain = Chain::launch(&miner).await?;
    eprintln!("regtest validators ready; mining shielded funds");
    Chain::mine(&chain.rpc, 7).await?;
    Chain::copy(&chain.rpc, &chain.fork, 2, 8).await?;
    chain.indexer_ready(8).await?;
    let settings = SyncSettings::new(chain.endpoint.clone(), 2, 300)?;
    treasury.configure_sync(settings.clone());
    let stop = CancellationToken::new();
    treasury.sync_once(&stop).await?;
    assert!(
        treasury
            .status()
            .await?
            .sync
            .unwrap()
            .spendable_shielded_zatoshis
            > 100_000
    );
    let op = uuid::Uuid::new_v4().to_string();
    let identity_op = op.clone();
    treasury
        .store
        .call(move |s| {
            s.bind_operation_recipient(&identity_op, "0x0000000000000000000000000000000000000001")
        })
        .await?;
    let prepared = treasury
        .prepare(PrepareRequest {
            operation_id: op.clone(),
            pool_id: None,
            daily_limit_zatoshis: 1_000_000,
            deadline: now()? + 3600,
            recipient: "t2RnBRiqrN1nW4ecZs1Fj3WWjNdnSs4kiX8".into(),
            amount_zatoshis: 50_000,
            max_fee_zatoshis: 50_000,
            max_input_zatoshis: 100_000,
        })
        .await?;
    let facts = prepared.facts().unwrap();
    let cost = i64::try_from(facts.amount_zatoshis + facts.fee_zatoshis)?;
    assert_eq!(budget(&state, &op)?, (cost, 0));
    eprintln!("deposit prepared; checking durable restart");
    treasury.close().await?;
    assert!(
        Treasury::open(state.clone(), key.clone(), id.clone())
            .await
            .is_err(),
        "production open accepted regtest state"
    );
    let mut treasury = Treasury::open_with_network(
        state.clone(),
        key.clone(),
        id.clone(),
        TreasuryNetwork::Regtest,
    )
    .await?;
    treasury.configure_sync(settings.clone());
    let mut sender = submission::GrpcSubmission::with_network(
        chain.endpoint.clone(),
        chain.endpoint.clone(),
        TreasuryNetwork::Regtest,
    )?;
    assert_eq!(
        treasury
            .submit_prepared(op.clone(), &mut sender, false, &stop)
            .await?,
        SubmissionOutcome::Accepted
    );
    Chain::mine(&chain.rpc, 1).await?;
    chain.indexer_ready(9).await?;
    let error = treasury
        .reconcile_prepared(op.clone(), &mut sender, &stop)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("treasury_confirmation_pending"),
        "{error}"
    );
    assert!(treasury.status().await?.outgoing_pending);
    eprintln!("deposit mined; replacing its block with a longer branch");
    Chain::mine(&chain.fork, 3).await?;
    Chain::copy(&chain.fork, &chain.rpc, 9, 11).await?;
    chain.indexer_ready(11).await?;
    let presence = treasury
        .reconcile_prepared(op.clone(), &mut sender, &stop)
        .await?;
    assert!(!matches!(presence, TransactionPresence::Confirmed { .. }));
    assert!(treasury.status().await?.outgoing_pending);
    let recovered = treasury
        .store
        .call({
            let op = op.clone();
            move |s| crate::rotation::transaction::PreparedTransaction::load(s, &op)
        })
        .await?;
    assert_eq!(recovered.bytes(), prepared.bytes());
    assert_eq!(budget(&state, &op)?, (cost, 0));
    assert_eq!(
        treasury
            .submit_prepared(op.clone(), &mut sender, true, &stop)
            .await?,
        SubmissionOutcome::Accepted
    );
    Chain::mine(&chain.rpc, 2).await?;
    chain.indexer_ready(13).await?;
    assert!(matches!(
        treasury
            .reconcile_prepared(op.clone(), &mut sender, &stop)
            .await?,
        TransactionPresence::Confirmed { .. }
    ));
    assert!(!treasury.status().await?.outgoing_pending);
    assert_eq!(budget(&state, &op)?, (0, cost));
    eprintln!("deposit settled; testing a fork after accounting confirmation");
    Chain::mine(&chain.fork, 3).await?;
    Chain::copy(&chain.fork, &chain.rpc, 12, 14).await?;
    chain.indexer_ready(14).await?;
    let error = treasury.sync_once(&stop).await.unwrap_err();
    assert!(
        error.to_string().contains("treasury_confirmed_spend_reorg"),
        "{error}"
    );
    let status = treasury.status().await?;
    assert_eq!(status.sync.unwrap().phase, SyncPhase::Failed);
    assert_eq!(status.treasury_operations[0].submission, "CONFIRMED");
    assert_eq!(budget(&state, &op)?, (0, cost));
    treasury.close().await?;
    let mut restored =
        Treasury::open_with_network(state, key, id, TreasuryNetwork::Regtest).await?;
    restored.configure_sync(settings);
    assert!(
        restored
            .sync_once(&stop)
            .await
            .unwrap_err()
            .to_string()
            .contains("treasury_confirmed_spend_reorg")
    );
    restored.close().await?;
    chain.passed = true;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Docker; isolated refund discovery and per-address shielding"]
async fn high_index_refunds_and_separate_shielding() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(900), refunds_run()).await?
}
async fn refunds_run() -> Result<()> {
    use crate::rotation::store::funding::FundingPhase;
    let dir = tempfile::tempdir()?;
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let mut treasury = Treasury::create_with_network(
        state.clone(),
        key.clone(),
        1,
        Some(Zeroizing::new(SEED.into())),
        TreasuryNetwork::Regtest,
    )
    .await?;
    let id = treasury.status().await?.treasury_id;
    for n in 0..16 {
        treasury
            .ensure_pool(format!("refund_{n}"), "5.00".into())
            .await?;
    }
    let jobs = treasury.store.call(|s| s.funding_jobs()).await?;
    let mut addresses = vec![];
    for job in &jobs {
        addresses.push(treasury.refund_address(job.id.clone()).await?);
    }
    assert_eq!(
        addresses
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        32
    );
    let miner = treasury
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    treasury.close().await?;
    let mut chain = Chain::launch(&miner).await?;
    Chain::mine(&chain.rpc, 7).await?;
    chain.indexer_ready(8).await?;
    let settings = SyncSettings::new(chain.endpoint.clone(), 2, 300)?;
    let stop = CancellationToken::new();
    let mut treasury = Treasury::open_with_network(
        state.clone(),
        key.clone(),
        id.clone(),
        TreasuryNetwork::Regtest,
    )
    .await?;
    treasury.configure_sync(settings.clone());
    let mut sender = submission::GrpcSubmission::with_network(
        chain.endpoint.clone(),
        chain.endpoint.clone(),
        TreasuryNetwork::Regtest,
    )?;
    for n in [30usize, 31] {
        treasury.sync_once(&stop).await?;
        let job = jobs[n].clone();
        let jid = job.id.clone();
        treasury
            .store
            .call(move |s| {
                s.save_funding_quote(&jid, b"test quote")?;
                s.advance_funding(&jid, FundingPhase::Quoted, FundingPhase::Preparing)
            })
            .await?;
        treasury
            .prepare(PrepareRequest {
                operation_id: job.operation_id.clone(),
                pool_id: Some(job.pool_id),
                daily_limit_zatoshis: 1_000_000,
                deadline: now()? + 3600,
                recipient: addresses[n].clone(),
                amount_zatoshis: 50_000,
                max_fee_zatoshis: 50_000,
                max_input_zatoshis: 100_000,
            })
            .await?;
        assert_eq!(
            treasury
                .submit_prepared(job.operation_id.clone(), &mut sender, false, &stop)
                .await?,
            SubmissionOutcome::Accepted
        );
        Chain::mine(&chain.rpc, 2).await?;
        chain.indexer_ready(if n == 30 { 10 } else { 12 }).await?;
        treasury
            .reconcile_prepared(job.operation_id, &mut sender, &stop)
            .await?;
    }
    treasury.close().await?;
    let mut treasury = Treasury::open_with_network(
        state.clone(),
        key.clone(),
        id.clone(),
        TreasuryNetwork::Regtest,
    )
    .await?;
    treasury.configure_sync(settings.clone());
    treasury.sync_once(&stop).await?;
    assert_eq!(treasury.status().await?.refunds.len(), 2);
    for fail_commit in [false, true] {
        let before = treasury.status().await?.treasury_operations.len();
        let mempool: Value = chain
            .rpc
            .json_result_from_call("getrawmempool", "[]")
            .await?;
        let db = rusqlite::Connection::open(state.join("state.sqlite"))?;
        if fail_commit {
            db.execute_batch("CREATE TRIGGER reject_shield BEFORE INSERT ON outgoing BEGIN SELECT RAISE(ABORT,'fixture shield commit failure'); END;")?;
        }
        let error = treasury
            .shield_refund(
                jobs[30].id.clone(),
                1_000_000,
                if fail_commit { 30_000 } else { 1 },
                &stop,
            )
            .await
            .err()
            .expect("shielding failure was not injected");
        if !fail_commit {
            assert!(
                error.to_string().contains("shielding fee cap exceeded"),
                "{error}"
            );
        }
        if fail_commit {
            db.execute_batch("DROP TRIGGER reject_shield;")?;
        }
        assert_eq!(treasury.status().await?.treasury_operations.len(), before);
        assert!(!treasury.status().await?.outgoing_pending);
        let after: Value = chain
            .rpc
            .json_result_from_call("getrawmempool", "[]")
            .await?;
        assert_eq!(
            mempool, after,
            "calculate-only failure broadcast a transaction"
        );
        let abandoned: Vec<String> = db.prepare("SELECT id FROM budget_entries WHERE consumed=0 AND NOT EXISTS(SELECT 1 FROM outgoing WHERE outgoing.id=budget_entries.id)")?.query_map([],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
        drop(db);
        treasury.close().await?;
        treasury = Treasury::open_with_network(
            state.clone(),
            key.clone(),
            id.clone(),
            TreasuryNetwork::Regtest,
        )
        .await?;
        treasury.configure_sync(settings.clone());
        treasury
            .store
            .call(move |s| {
                for id in abandoned {
                    s.abandon_unprepared(&id)?;
                }
                Ok(())
            })
            .await?;
    }
    // Both unrelated transparent addresses are funded. Each shield must consume
    // exactly its selected address, even when the helper's default would combine.
    for (n, height) in [(30usize, 14), (31usize, 16)] {
        let prepared = treasury
            .shield_refund(jobs[n].id.clone(), 1_000_000, 30_000, &stop)
            .await?;
        let tx = submission::decode(prepared.bytes())?;
        assert_eq!(tx.transparent_bundle().unwrap().vin.len(), 1);
        assert!(tx.transparent_bundle().unwrap().vout.is_empty());
        assert_eq!(prepared.facts().unwrap().amount_zatoshis, 0);
        let op = prepared.operation_id().to_owned();
        assert_eq!(
            treasury
                .submit_prepared(op.clone(), &mut sender, false, &stop)
                .await?,
            SubmissionOutcome::Accepted
        );
        Chain::mine(&chain.rpc, 2).await?;
        chain.indexer_ready(height).await?;
        treasury
            .reconcile_prepared(op.clone(), &mut sender, &stop)
            .await?;
        assert_eq!(
            budget(&state, &op)?,
            (0, i64::try_from(prepared.facts().unwrap().fee_zatoshis)?)
        );
        assert_eq!(treasury.status().await?.refunds.len(), 2);
    }
    treasury.close().await?;
    chain.passed = true;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Docker; isolated expiry recovery with unspent-input proof"]
async fn expired_ambiguous_deposit_releases_only_after_chain_proof() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(900), expiry_run()).await?
}
async fn expiry_run() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let mut treasury = Treasury::create_with_network(
        state.clone(),
        key.clone(),
        1,
        Some(Zeroizing::new(SEED.into())),
        TreasuryNetwork::Regtest,
    )
    .await?;
    let id = treasury.status().await?.treasury_id;
    let miner = treasury
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut chain = Chain::launch(&miner).await?;
    Chain::mine(&chain.rpc, 7).await?;
    chain.indexer_ready(8).await?;
    let settings = SyncSettings::new(chain.endpoint.clone(), 2, 300)?;
    treasury.configure_sync(settings.clone());
    let stop = CancellationToken::new();
    treasury.sync_once(&stop).await?;
    let op = uuid::Uuid::new_v4().to_string();
    let identity_op = op.clone();
    treasury
        .store
        .call(move |s| {
            s.bind_operation_recipient(&identity_op, "0x0000000000000000000000000000000000000001")
        })
        .await?;
    let prepared = treasury
        .prepare(PrepareRequest {
            operation_id: op.clone(),
            pool_id: None,
            daily_limit_zatoshis: 1_000_000,
            deadline: now()? + 3600,
            recipient: "t2RnBRiqrN1nW4ecZs1Fj3WWjNdnSs4kiX8".into(),
            amount_zatoshis: 50_000,
            max_fee_zatoshis: 50_000,
            max_input_zatoshis: 100_000,
        })
        .await?;
    // Simulate a crash after committing the send intent, before handing bytes to
    // transport. Recovery must treat this exactly like a lost broadcast response.
    let operation = op.clone();
    treasury
        .store
        .call(move |s| {
            s.request_broadcast(&operation, now()?, 8, false)?;
            Ok(())
        })
        .await?;
    let mut sender = submission::GrpcSubmission::with_network(
        chain.endpoint.clone(),
        chain.endpoint.clone(),
        TreasuryNetwork::Regtest,
    )?;
    assert!(
        treasury
            .recover_expired(op.clone(), &mut sender, &stop)
            .await
            .unwrap_err()
            .to_string()
            .contains("expiry not buried")
    );
    assert!(budget(&state, &op)?.0 > 0);
    treasury.close().await?;
    let target = u64::from(prepared.facts().unwrap().expiry_height) + 2;
    ensure!(target < 200, "unexpected regtest expiry");
    Chain::mine(&chain.rpc, usize::try_from(target - 8)?).await?;
    chain.indexer_ready(target).await?;
    let mut treasury =
        Treasury::open_with_network(state.clone(), key, id, TreasuryNetwork::Regtest).await?;
    treasury.configure_sync(settings);
    treasury.sync_once(&stop).await?;
    // An unresolved/missing original input must block recovery even when the
    // transaction itself has expired. Exercise the same production verifier.
    {
        use pepper_sync::wallet::{IronwoodNote, OrchardNote, SaplingNote, TransparentCoin};
        let operation = op.clone();
        let bytes = treasury
            .store
            .call(move |s| s.prepared_snapshot(&operation))
            .await?;
        let original = LightClient::from_reader(
            bytes.as_slice(),
            config(
                treasury._scratch.path(),
                WalletConfig::Read,
                TreasuryNetwork::Regtest,
            )?,
        )
        .await?;
        let original = original.wallet().read().await;
        let txid = submission::decode(prepared.bytes())?.txid();
        let mut current = treasury.client.as_ref().unwrap().wallet().write().await;
        let checked = [
            reject_missing_input::<TransparentCoin>(&original, &mut current, &txid, target)?,
            reject_missing_input::<SaplingNote>(&original, &mut current, &txid, target)?,
            reject_missing_input::<OrchardNote>(&original, &mut current, &txid, target)?,
            reject_missing_input::<IronwoodNote>(&original, &mut current, &txid, target)?,
        ];
        assert!(checked.into_iter().any(|checked| checked));
    }
    treasury
        .recover_expired(op.clone(), &mut sender, &stop)
        .await?;
    assert_eq!(budget(&state, &op)?, (0, 0));
    assert_eq!(
        treasury.status().await?.treasury_operations[0].submission,
        "EXPIRED"
    );
    let operation = op.clone();
    assert!(
        treasury
            .store
            .call(move |s| crate::rotation::transaction::PreparedTransaction::load(s, &operation))
            .await
            .is_err()
    );
    assert!(
        treasury
            .submit_prepared(op, &mut sender, true, &stop)
            .await
            .is_err()
    );
    assert!(!treasury.status().await?.outgoing_pending);
    treasury.close().await?;
    chain.passed = true;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Docker; qualify indexer non-inclusion response"]
async fn indexer_non_inclusion_response() -> Result<()> {
    use zingo_netutils::{GrpcIndexer, Indexer, lightwallet_protocol::TxFilter};
    let mut chain = Chain::launch("t2RnBRiqrN1nW4ecZs1Fj3WWjNdnSs4kiX8").await?;
    chain.indexer_ready(1).await?;
    let mut client = GrpcIndexer::new(chain.endpoint.parse()?).await?;
    client.get_lightd_info(Duration::from_secs(15)).await?;
    let result = client
        .get_transaction(
            TxFilter {
                hash: vec![42; 32],
                ..Default::default()
            },
            Duration::from_secs(15),
        )
        .await;
    let error = result.unwrap_err();
    assert_eq!(error.code(), tonic::Code::Internal);
    assert_eq!(
        error.message(),
        "InternalServerError: error receiving data from backing node"
    );
    chain.passed = true;
    Ok(())
}

fn reject_missing_input<T: pepper_sync::wallet::OutputInterface>(
    original: &zingolib::wallet::LightWallet,
    current: &mut zingolib::wallet::LightWallet,
    txid: &zcash_primitives::transaction::TxId,
    height: u64,
) -> Result<bool> {
    let inputs = T::transaction_inputs(original.wallet_transactions.get(txid).unwrap());
    let Some(origin) = current
        .wallet_outputs::<T>()
        .into_iter()
        .find(|n| n.spend_link().is_some_and(|link| inputs.contains(&&link)))
        .map(|n| n.output_id().txid())
    else {
        return Ok(false);
    };
    let removed = current.wallet_transactions.remove(&origin).unwrap();
    assert!(super::expiry::check_inputs::<T>(original, current, txid, height, 2).is_err());
    current.wallet_transactions.insert(origin, removed);
    assert!(super::expiry::check_inputs::<T>(original, current, txid, height, 2)? > 0);
    Ok(true)
}

use crate::test_socks as socks;
#[tokio::test]
#[ignore = "requires Docker; run alone because it installs the process Tor policy"]
async fn tor_consensus_lifecycle() -> Result<()> {
    let proxy = socks::Socks::start(
        std::collections::BTreeMap::from([("loopback".into(), "127.0.0.1:1".parse()?)]),
        socks::Fault::None,
    )
    .await;
    crate::network::install(crate::network::NetworkPolicy {
        mode: crate::network::Mode::Tor,
        socks_endpoint: Some(proxy.address),
        ..Default::default()
    })?;
    run().await?;
    refunds_run().await?;
    expiry_run().await?;
    let records = proxy.records.lock().unwrap();
    ensure!(records.len() >= 6, "no proxied consensus activity");
    ensure!(
        records.iter().all(|r| r.user == "<torS0X>0"),
        "missing SOCKS isolation"
    );
    ensure!(
        records
            .iter()
            .map(|r| &r.password)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 3,
        "identities were not separated"
    );
    Ok(())
}
