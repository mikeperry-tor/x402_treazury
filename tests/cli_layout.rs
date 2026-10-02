//! Public naming and portable config paths must survive repository layout changes.
use serde_json::Value;
use std::{path::Path, process::Command};
#[test]
fn executable_name_version_and_example_inspection_are_portable() {
    let dir = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_treazure");
    let run = |args: &[&str]| {
        Command::new(binary)
            .env_clear()
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let version = run(&["--version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        concat!("treazure ", env!("CARGO_PKG_VERSION"))
    );
    let help = run(&["--help"]);
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("Usage: treazure")
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for example in [
        "servers.toml",
        "socialfetch.toml",
        "servers-auto-wallets.toml",
        "servers-managed.toml",
        "public-swap-demo.toml",
        "public-payment-demo.toml",
    ] {
        let path = root.join("examples").join(example);
        let shown = run(&["--meta-config", path.to_str().unwrap(), "--show-config"]);
        assert!(
            shown.status.success(),
            "{example}: {}",
            String::from_utf8_lossy(&shown.stderr)
        );
        let value: Value = serde_json::from_slice(&shown.stdout).unwrap();
        for source in value["sources"].as_object().unwrap().values() {
            let spec = source["settings"]["spec"].as_str().unwrap();
            if !spec.starts_with("http") {
                assert!(Path::new(spec).is_absolute());
                assert!(Path::new(spec).is_file(), "{example}: {spec}");
            }
        }
    }
}
