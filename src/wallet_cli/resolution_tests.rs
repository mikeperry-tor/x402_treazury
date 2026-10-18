use super::*;
use serde_json::{Value, json};
use std::{path::Path, process::Command as ProcessCommand};

fn command_value(command: Command) -> Value {
    match command {
        Command::Bootstrap { meta_config } => json!({"kind":"bootstrap","config":meta_config}),
        Command::Sync {
            meta_config,
            qualification_parent_stdin,
        } => json!({"kind":"sync","config":meta_config,"parent_stdin":qualification_parent_stdin}),
        Command::Reconcile {
            meta_config,
            operation_id,
            rebroadcast,
        } => {
            json!({"kind":"reconcile","config":meta_config,"operation":operation_id,"rebroadcast":rebroadcast})
        }
        Command::Recover { meta_config } => json!({"kind":"recover","config":meta_config}),
        Command::RecoverExpired {
            meta_config,
            operation_id,
        } => json!({"kind":"recover-expired","config":meta_config,"operation":operation_id}),
        Command::ShieldRefunds {
            meta_config,
            job_id,
        } => json!({"kind":"shield-refunds","config":meta_config,"job":job_id}),
        Command::Init {
            state_dir,
            key_file,
            birthday,
            indexer_url,
            mnemonic_file,
        } => {
            json!({"kind":"init","state":state_dir,"key":key_file,"birthday":birthday,"indexer":indexer_url,"mnemonic":mnemonic_file})
        }
        Command::Status {
            state_dir,
            key_file,
        } => json!({"kind":"status","state":state_dir,"key":key_file}),
        Command::Addresses {
            state_dir,
            key_file,
            treasury_id,
        } => json!({"kind":"addresses","state":state_dir,"key":key_file,"id":treasury_id}),
        Command::Address {
            state_dir,
            key_file,
            treasury_id,
        } => json!({"kind":"address","state":state_dir,"key":key_file,"id":treasury_id}),
        Command::Backup {
            state_dir,
            key_file,
            treasury_id,
            destination,
        } => {
            json!({"kind":"backup","state":state_dir,"key":key_file,"id":treasury_id,"destination":destination})
        }
        Command::RecoverUnprepared {
            state_dir,
            key_file,
            treasury_id,
            job_id,
        } => {
            json!({"kind":"recover-unprepared","state":state_dir,"key":key_file,"id":treasury_id,"job":job_id})
        }
    }
}

// Each child owns a fresh production network OnceLock and environment. No command
// execution occurs: even funded command variants are only inspected as values.
#[test]
fn resolution_child() {
    let Ok(spec) = std::env::var("X402_RESOLUTION_SPEC") else {
        return;
    };
    let spec: Value = serde_json::from_str(&spec).unwrap();
    if spec["preinstall"] == true {
        crate::network::install(Default::default()).unwrap();
    }
    let args: Vec<String> = serde_json::from_value(spec["args"].clone()).unwrap();
    let args =
        WalletArgs::try_parse_from(std::iter::once("wallet".to_owned()).chain(args)).unwrap();
    let mut category = None;
    let result = match resolve_args(args) {
        Ok(command) => json!({"ok":command_value(command)}),
        Err(error) => {
            category = if let Some(io) = error.downcast_ref::<std::io::Error>() {
                Some(format!("io:{:?}", io.kind()))
            } else if error.is::<toml::de::Error>() {
                Some("toml".into())
            } else if error.is::<uuid::Error>() {
                Some("uuid".into())
            } else {
                None
            };
            json!({"error":error.to_string()})
        }
    };
    let mut output = json!({"result":result});
    if let Some(category) = category {
        output["error_category"] = json!(category);
    }
    if spec["check_default"] == true {
        output["default_install"] = match crate::network::install(Default::default()) {
            Ok(()) => json!("ok"),
            Err(error) => json!(error.to_string()),
        };
    }
    std::fs::write(spec["output"].as_str().unwrap(), output.to_string()).unwrap();
}

fn resolve_in(
    dir: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    preinstall: bool,
    check_default: bool,
) -> Value {
    let output = dir.join("resolution-output.json");
    if output.exists() {
        std::fs::remove_file(&output).unwrap();
    }
    let child = ProcessCommand::new(std::env::current_exe().unwrap())
        .args(["--exact","wallet_cli::resolution_tests::resolution_child","--nocapture"])
        .current_dir(dir).env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE",v)))
        .envs(env.iter().copied())
        .env("X402_RESOLUTION_SPEC",json!({"args":args,"output":output,"preinstall":preinstall,"check_default":check_default}).to_string())
        .output().unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap()
}
fn resolved(dir: &Path, args: &[&str]) -> Value {
    resolve_in(dir, args, &[], false, false)["result"].clone()
}
fn config(dir: &Path, extra: &str) {
    std::fs::write(dir.join("deployment.toml"),format!("version=1\nservers={{}}\n[treasury]\nstate_dir='absent'\nmax_refund_shielding_fee_zec='0.001'\n{extra}\n[sources.unused]\nspec='https://unreachable.invalid/catalog'\n")).unwrap();
}

#[test]
fn configured_actions_return_exact_arguments_before_general_state_access() {
    let dir = tempfile::tempdir().unwrap();
    config(dir.path(), "");
    let cases = [
        (
            vec!["bootstrap"],
            json!({"kind":"bootstrap","config":"deployment.toml"}),
        ),
        (
            vec!["sync", "--qualification-parent-stdin"],
            json!({"kind":"sync","config":"deployment.toml","parent_stdin":true}),
        ),
        (
            vec!["sync"],
            json!({"kind":"sync","config":"deployment.toml","parent_stdin":false}),
        ),
        (
            vec!["reconcile", "--operation-id", "operation", "--rebroadcast"],
            json!({"kind":"reconcile","config":"deployment.toml","operation":"operation","rebroadcast":true}),
        ),
        (
            vec!["reconcile", "--operation-id", "operation"],
            json!({"kind":"reconcile","config":"deployment.toml","operation":"operation","rebroadcast":false}),
        ),
        (
            vec!["recover"],
            json!({"kind":"recover","config":"deployment.toml"}),
        ),
        (
            vec!["recover-expired", "--operation-id", "operation"],
            json!({"kind":"recover-expired","config":"deployment.toml","operation":"operation"}),
        ),
        (
            vec!["shield-refunds", "--job-id", "job"],
            json!({"kind":"shield-refunds","config":"deployment.toml","job":"job"}),
        ),
    ];
    for (mut args, expected) in cases {
        assert_eq!(
            resolved(dir.path(), &args),
            json!({"error":"this wallet command requires --config FILE"})
        );
        args.extend(["--config", "deployment.toml"]);
        assert_eq!(resolved(dir.path(), &args), json!({"ok":expected}));
        assert!(!dir.path().join("absent").exists());
    }
    assert_eq!(
        resolved(dir.path(), &["pool", "--config", "missing.toml"]),
        json!({"error":"wallet pool was removed; configure managed wallet reserves and explicit max_api_payment_usdc in the deployment, then use wallet bootstrap --config FILE"})
    );
    assert_eq!(
        resolved(dir.path(), &["pool", "--network-config", "missing.toml"])["error"],
        "wallet pool was removed; configure managed wallet reserves and explicit max_api_payment_usdc in the deployment, then use wallet bootstrap --config FILE"
    );
}

#[test]
fn settings_errors_and_network_install_keep_their_effect_order() {
    let dir = tempfile::tempdir().unwrap();
    for (text, error) in [
        ("version=2\nservers={}\n", "unsupported deployment version"),
        ("version=1\nservers={}\n", "missing [treasury]"),
        (
            "version=1\nservers={}\n[network]\nmode='tor'\n",
            "network.socks_endpoint is required in Tor mode",
        ),
    ] {
        std::fs::write(dir.path().join("deployment.toml"), text).unwrap();
        assert_eq!(
            resolved(dir.path(), &["bootstrap", "--config", "deployment.toml"]),
            json!({"error":error})
        );
    }
    // Missing treasury is returned only after a distinct valid policy is installed.
    std::fs::write(
        dir.path().join("deployment.toml"),
        "version=1\nservers={}\n[network]\nconnect_timeout_seconds=7\n",
    )
    .unwrap();
    let changed = resolve_in(
        dir.path(),
        &["bootstrap", "--config", "deployment.toml"],
        &[],
        false,
        true,
    );
    assert_eq!(
        changed,
        json!({"result":{"error":"missing [treasury]"},"default_install":"network policy cannot change after initialization; restart required"})
    );
    let changed = resolve_in(
        dir.path(),
        &["bootstrap", "--config", "deployment.toml"],
        &[],
        true,
        false,
    );
    assert_eq!(
        changed["result"],
        json!({"error":"network policy cannot change after initialization; restart required"})
    );
    config(dir.path(), "");
    assert_eq!(
        resolve_in(
            dir.path(),
            &["bootstrap", "--config", "deployment.toml"],
            &[],
            true,
            true
        )["default_install"],
        "ok"
    );
    std::fs::write(dir.path().join("network.toml"), "[network]\nmode='tor'\n").unwrap();
    assert_eq!(
        resolved(
            dir.path(),
            &["bootstrap", "--network-config", "network.toml"]
        )["error"],
        "network.socks_endpoint is required in Tor mode"
    );
    for filename in ["missing.toml", "broken.toml"] {
        if filename == "broken.toml" {
            std::fs::write(dir.path().join(filename), "invalid = [").unwrap();
        }
        let output = resolve_in(
            dir.path(),
            &["status", "--config", filename],
            &[],
            false,
            false,
        );
        assert_eq!(
            output["error_category"],
            if filename == "missing.toml" {
                "io:NotFound"
            } else {
                "toml"
            }
        );
    }
    config(dir.path(), "id='not-a-uuid'");
    assert_eq!(
        resolve_in(
            dir.path(),
            &["status", "--config", "deployment.toml"],
            &[],
            false,
            false
        )["error_category"],
        "uuid"
    );
    assert!(!dir.path().join("absent").exists());
}

#[test]
fn init_resolves_paths_and_endpoint_precedence_without_reading_or_creating_state() {
    let dir = tempfile::tempdir().unwrap();
    let basic = json!({"kind":"init","state":"missing","key":"missing/wallet.key","birthday":1,"indexer":null,"mnemonic":"unread-seed"});
    assert_eq!(
        resolved(
            dir.path(),
            &[
                "init",
                "--state-dir",
                "missing",
                "--birthday",
                "1",
                "--mnemonic-file",
                "unread-seed"
            ]
        ),
        json!({"ok":basic})
    );
    let base = ["init", "--state-dir", "missing"];
    assert_eq!(
        resolved(dir.path(), &base)["ok"],
        json!({"kind":"init","state":"missing","key":"missing/wallet.key","birthday":null,"indexer":null,"mnemonic":null})
    );
    for value in ["", "not-yet-validated", "https://indexer.example"] {
        let result = resolve_in(
            dir.path(),
            &[
                "init",
                "--state-dir",
                "missing",
                "--indexer-url-env",
                "INDEXER",
            ],
            &[("INDEXER", value)],
            false,
            false,
        );
        assert_eq!(result["result"]["ok"]["indexer"], value);
    }
    assert_eq!(
        resolved(
            dir.path(),
            &[
                "init",
                "--state-dir",
                "missing",
                "--indexer-url-env",
                "INDEXER"
            ]
        )["error"],
        "missing birthday indexer environment variable INDEXER"
    );
    assert_eq!(
        resolved(
            dir.path(),
            &[
                "init",
                "--state-dir",
                "missing",
                "--treasury-id",
                "assertion",
                "--indexer-url-env",
                "INDEXER"
            ]
        )["error"],
        "init creates a new treasury identity; omit the expected treasury ID"
    );
    std::fs::create_dir(dir.path().join("configs")).unwrap();
    config(
        &dir.path().join("configs"),
        "key_file='outside.key'\nindexer_url_env='INDEXER'",
    );
    let args = [
        "init",
        "--config",
        "configs/deployment.toml",
        "--birthday",
        "1",
    ];
    assert_eq!(
        resolved(dir.path(), &args)["ok"],
        json!({"kind":"init","state":"configs/absent","key":"configs/outside.key","birthday":1,"indexer":null,"mnemonic":null})
    );
    let args = ["init", "--config", "configs/deployment.toml"];
    assert_eq!(
        resolved(dir.path(), &args)["error"],
        "required endpoint environment variable INDEXER is missing or empty"
    );
    assert_eq!(
        resolve_in(dir.path(), &args, &[("INDEXER", "")], false, false)["result"]["error"],
        "required endpoint environment variable INDEXER is missing or empty"
    );
    assert_eq!(
        resolve_in(
            dir.path(),
            &args,
            &[("INDEXER", "https://indexer.example")],
            false,
            false
        )["result"]["ok"]["indexer"],
        "https://indexer.example"
    );
    assert_eq!(
        resolved(
            dir.path(),
            &[
                "init",
                "--config",
                "configs/deployment.toml",
                "--indexer-url-env",
                "INDEXER"
            ]
        )["error"],
        "configure the indexer in the deployment file"
    );
    config(
        &dir.path().join("configs"),
        "indexer_url='https://explicit.example'",
    );
    assert_eq!(
        resolved(dir.path(), &args)["ok"]["indexer"],
        "https://explicit.example"
    );
    config(&dir.path().join("configs"), "");
    let configured = resolved(dir.path(), &args);
    assert_eq!(configured["ok"]["key"], "configs/absent/wallet.key");
    assert_eq!(
        configured["ok"]["indexer"],
        crate::rotation::config::DEFAULT_ZCASH_INDEXER
    );
    assert!(!dir.path().join("missing").exists());
    assert!(!dir.path().join("configs/absent").exists());
}

#[test]
fn administrative_identity_resolution_preserves_saved_state_and_optional_assertions() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        resolved(dir.path(), &["status"])["error"],
        "supply --config FILE or --state-dir PATH"
    );
    assert_eq!(
        resolved(dir.path(), &["status", "--state-dir", "missing"])["ok"],
        json!({"kind":"status","state":"missing","key":"missing/wallet.key"})
    );
    assert_eq!(
        resolved(dir.path(), &["addresses", "--state-dir", "missing"])["ok"],
        json!({"kind":"addresses","state":"missing","key":"missing/wallet.key","id":null})
    );
    for command in ["status", "addresses", "address"] {
        assert!(
            resolved(
                dir.path(),
                &[
                    command,
                    "--state-dir",
                    "missing",
                    "--treasury-id",
                    "expected"
                ]
            )
            .get("error")
            .is_some()
        );
    }
    for mut args in [
        vec!["address"],
        vec!["backup", "--destination", "backup"],
        vec!["recover-unprepared", "--job-id", "job"],
    ] {
        args.extend(["--state-dir", "missing"]);
        assert!(resolved(dir.path(), &args).get("error").is_some());
    }
    let store = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("external.key"),
        1,
        b"synthetic wallet",
    )
    .unwrap();
    let id = store.id().to_owned();
    drop(store);
    let database = std::fs::read(dir.path().join("state/state.sqlite")).unwrap();
    let key = std::fs::read(dir.path().join("external.key")).unwrap();
    for (action, extra) in [
        ("status", vec![]),
        ("addresses", vec![]),
        ("address", vec![]),
        ("backup", vec!["--destination", "backup"]),
        ("recover-unprepared", vec!["--job-id", "job"]),
    ] {
        for expected in [None, Some(id.as_str()), Some("wrong")] {
            let mut args = vec![action, "--state-dir", "state", "--key-file", "external.key"];
            args.extend(extra.iter().copied());
            if let Some(expected) = expected {
                args.extend(["--treasury-id", expected]);
            }
            let result = resolved(dir.path(), &args);
            if expected == Some("wrong") {
                assert_eq!(
                    result["error"],
                    "treasury ID does not match state (state identity mismatch); use the treasury_id from `wallet status` (it is a UUID, not an account number)"
                );
                continue;
            }
            let mut value = json!({"kind":action,"state":"state","key":"external.key"});
            if action != "status" {
                value["id"] = if action == "addresses" && expected.is_none() {
                    Value::Null
                } else {
                    json!(id)
                };
            }
            if action == "backup" {
                value["destination"] = json!("backup");
            }
            if action == "recover-unprepared" {
                value["job"] = json!("job");
            }
            assert_eq!(result, json!({"ok":value}));
        }
    }
    assert_eq!(
        resolved(
            dir.path(),
            &[
                "init",
                "--state-dir",
                "state",
                "--key-file",
                "external.key",
                "--birthday",
                "1"
            ]
        )["ok"]["kind"],
        "init"
    );
    assert_eq!(
        std::fs::read(dir.path().join("state/state.sqlite")).unwrap(),
        database
    );
    assert_eq!(std::fs::read(dir.path().join("external.key")).unwrap(), key);
    std::fs::write(dir.path().join("state/state.sqlite"), b"malformed sqlite").unwrap();
    for command in ["status", "addresses"] {
        assert!(
            resolved(dir.path(), &[command, "--state-dir", "state"])
                .get("ok")
                .is_some()
        );
        assert_eq!(
            resolved(
                dir.path(),
                &[command, "--state-dir", "state", "--treasury-id", &id]
            )["error"],
            "file is not a database"
        );
    }
    for mut args in [
        vec!["address"],
        vec!["backup", "--destination", "backup"],
        vec!["recover-unprepared", "--job-id", "job"],
    ] {
        args.extend(["--state-dir", "state"]);
        assert_eq!(
            resolved(dir.path(), &args)["error"],
            "file is not a database"
        );
    }
    assert!(!dir.path().join("backup").exists());
    assert!(!dir.path().join("missing").exists());
}
