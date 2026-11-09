//! Executable preview contracts using public unfunded state and absent credentials.
use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};
use x402_treazury::rotation::{config::ReservesConfig, store::Store};

fn run(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_x402_treazury"));
    command
        .current_dir(dir)
        .env_clear()
        .args(args)
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)));
    command.envs(env.iter().copied()).output().unwrap()
}
fn write_config(dir: &Path, static_collision: bool) {
    let wallet = if static_collision {
        "mode='static'\nprivate_key_env='ABSENT_PREVIEW_STATIC_KEY'"
    } else {
        "mode='zcash_rotation'\nmax_api_payment_usdc='1'\nreserves={small={funding_amount_usdc='2'}}"
    };
    std::fs::write(
        dir.join("deployment.toml"),
        format!(
            r#"version=1
[treasury]
state_dir='wallet'
key_file='never-created.key'
indexer_url_env='ABSENT_PREVIEW_INDEXER'
[funding]
auto_fund=false
near_user_session_env='ABSENT_PREVIEW_NEAR'
base_rpc_url_env='ABSENT_PREVIEW_BASE'
[wallets.group]
{wallet}
[sources.unread]
provider='missing-provider.toml'
[servers.endpoint]
listen='127.0.0.1:0'
bearer_token_env='ABSENT_PREVIEW_TOKEN'
wallet='group'
sources=['unread']
"#
        ),
    )
    .unwrap();
}
fn fixture(dir: &Path) -> Store {
    let key = dir.join("wallet/wallet.key");
    let mut store = Store::create(
        &dir.join("wallet"),
        &key,
        1,
        b"synthetic bytes; intentionally not an SDK wallet",
    )
    .unwrap();
    let reserves: ReservesConfig = toml::from_str("small={funding_amount_usdc='2'}").unwrap();
    store
        .ensure_capacity_group("group", &reserves, None, 0)
        .unwrap();
    std::fs::remove_file(key).unwrap();
    store
}
fn success_json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn preview_json_human_and_environment_config_need_no_keys_or_catalogs() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let store = fixture(dir);
    let before = serde_json::to_value(store.status().unwrap()).unwrap();
    write_config(dir, false);
    let args = ["wallet", "preview", "--config", "deployment.toml", "--json"];
    let explicit = run(dir, &args, &[("X402_TREAZURY_CONFIG", "absent.toml")]);
    let json = success_json(&explicit);
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["managed"]["group"]["action"], "reuse");
    assert_eq!(json["saved"]["treasury_id"], before["treasury_id"]);
    assert_eq!(json["bindings"]["endpoint"]["unread"]["wallet"], "group");
    assert!(json["startup_conflicts"].as_array().unwrap().is_empty());
    let from_env = run(
        dir,
        &["wallet", "preview", "--json"],
        &[("X402_TREAZURY_CONFIG", "deployment.toml")],
    );
    assert_eq!(json, success_json(&from_env));
    let human = run(
        dir,
        &["wallet", "preview", "--config", "deployment.toml"],
        &[],
    );
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(text.contains("Wallet configuration preview"));
    assert!(text.contains("Managed group: reuse"));
    assert!(text.contains("advisory observations"));
    assert_eq!(
        before,
        serde_json::to_value(store.status().unwrap()).unwrap()
    );
    assert!(!dir.join("never-created.key").exists());
    assert!(!dir.join("missing-provider.toml").exists());
}

#[test]
fn preview_outputs_conflict_report_then_returns_failure_for_disabled_name() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let mut store = fixture(dir);
    store.disable_pool("group").unwrap();
    let before = serde_json::to_value(store.status().unwrap()).unwrap();
    write_config(dir, true);
    let output = run(
        dir,
        &["wallet", "preview", "--config", "deployment.toml", "--json"],
        &[],
    );
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["startup_conflicts"].as_array().unwrap().len(), 1);
    assert!(
        report["startup_conflicts"][0]
            .as_str()
            .unwrap()
            .contains("group")
    );
    assert_eq!(
        report["omitted"]["group"]["action"],
        "retained_unchanged_static_only"
    );
    let output = run(
        dir,
        &["wallet", "preview", "--config", "deployment.toml"],
        &[],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Startup conflict:")
    );
    assert_eq!(
        before,
        serde_json::to_value(store.status().unwrap()).unwrap()
    );
}

#[test]
fn preview_requires_deployment_and_never_initializes_missing_state() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let no_config = run(dir, &["wallet", "preview", "--state-dir", "wallet"], &[]);
    assert!(!no_config.status.success());
    assert!(no_config.stdout.is_empty());
    assert!(String::from_utf8_lossy(&no_config.stderr).contains("--config"));
    write_config(dir, false);
    let missing = run(
        dir,
        &["wallet", "preview", "--config", "deployment.toml", "--json"],
        &[],
    );
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("wallet init"));
    assert!(!dir.join("wallet").exists());
    assert!(!dir.join("never-created.key").exists());
}
