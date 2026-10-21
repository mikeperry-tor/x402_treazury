//! A dedicated installed Tor, with persistent data and an owned child handle.
use super::control::Control;
use crate::{
    files,
    process::{Evidence, Process},
};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, path::Path, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use x402_treazury::network::{Mode, NetworkPolicy};
pub struct Tor {
    process: Process,
    pub control: Control,
    _ownership: files::Ownership,
}
impl Tor {
    #[cfg(test)]
    pub async fn start(
        binary: &Path,
        data: &Path,
        evidence: &Path,
        policy: &NetworkPolicy,
    ) -> Result<Self> {
        Self::start_with_cancel(binary, data, evidence, policy, CancellationToken::new()).await
    }
    pub async fn start_with_cancel(
        binary: &Path,
        data: &Path,
        evidence: &Path,
        policy: &NetworkPolicy,
        stop: CancellationToken,
    ) -> Result<Self> {
        policy.validate()?;
        ensure!(policy.mode == Mode::Tor, "owned Tor requires a Tor policy");
        files::directory(evidence)?;
        // Children run from the evidence directory. Resolve CLI-relative paths
        // before launch so persistent data and control files keep their identity.
        let evidence_path = evidence.canonicalize()?;
        let evidence = evidence_path.as_path();
        let binary_path = binary.canonicalize()?;
        let binary = binary_path.as_path();
        if !data.try_exists()? {
            files::create_dir(data)?;
        }
        files::directory(data)?;
        let data_path = data.canonicalize()?;
        let data = data_path.as_path();
        let lock = data.join("supervisor.lock");
        if !lock.try_exists()? {
            files::create_file(&lock)?.sync_all()?;
        }
        let ownership = files::lock(&lock)?;
        let version = Process::launch(
            binary,
            &["--version".into()],
            &BTreeMap::new(),
            evidence,
            16384,
        )?
        .wait(
            Duration::from_secs(10),
            Duration::from_secs(5),
            stop.clone(),
        )
        .await?;
        save(evidence, "tor-version", &version)?;
        ensure!(
            version.success
                && version.valid_output()
                && !version.forced_kill
                && !stop.is_cancelled(),
            "cannot qualify installed Tor version"
        );
        files::publish(
            &evidence.join("tor-binary.sha256"),
            files::hash_file(binary)?.as_bytes(),
        )?;
        let config = evidence.join("torrc");
        files::publish(
            &config,
            b"# Dedicated integration instance; no browser configuration.\n",
        )?;
        let port = evidence.join("control.port");
        let cookie = evidence.join("control.cookie");
        ensure!(
            !port.try_exists()? && !cookie.try_exists()?,
            "Tor control artifacts already exist; refusing stale ownership"
        );
        let socks = policy.socks_endpoint.context("SOCKS endpoint missing")?;
        let args = [
            "-f".into(),
            config.to_string_lossy().into_owned(),
            "--defaults-torrc".into(),
            config.to_string_lossy().into_owned(),
            "--DataDirectory".into(),
            data.to_string_lossy().into_owned(),
            "--ClientOnly".into(),
            "1".into(),
            "--SocksPort".into(),
            format!("{socks} IsolateSOCKSAuth"),
            "--ControlPort".into(),
            "auto".into(),
            "--ControlPortWriteToFile".into(),
            port.to_string_lossy().into_owned(),
            "--CookieAuthentication".into(),
            "1".into(),
            "--CookieAuthFile".into(),
            cookie.to_string_lossy().into_owned(),
            "--DNSPort".into(),
            "0".into(),
            "--TransPort".into(),
            "0".into(),
            "--HTTPTunnelPort".into(),
            "0".into(),
            "--ORPort".into(),
            "0".into(),
            "--Log".into(),
            "notice stdout".into(),
        ];
        let mut process = Process::launch(
            binary,
            &args,
            &BTreeMap::new(),
            evidence,
            files::DOCUMENT_BYTES,
        )?;
        let result = tokio::select! {
            _ = stop.cancelled() => Err(anyhow::anyhow!("Tor bootstrap cancelled; incomplete qualification")),
            result = tokio::time::timeout(
            Duration::from_secs(300),
            bootstrap(&mut process, &port, &cookie, evidence, socks),
        ) => result
        .context("Tor bootstrap exceeded its single 300-second budget")
        .and_then(|r| r),
        };
        match result {
            Ok(control) => Ok(Self {
                process,
                control,
                _ownership: ownership,
            }),
            Err(error) => {
                let output = process
                    .shutdown("tor_bootstrap_failure", Duration::from_secs(5))
                    .await?;
                save(evidence, "tor", &output)?;
                Err(error)
            }
        }
    }
    pub fn healthy(&mut self) -> Result<()> {
        ensure!(
            !self.process.exited()?
                && !self.process.fault.is_cancelled()
                && !self.control.fault.is_cancelled(),
            "owned Tor/control observer stopped or lost evidence"
        );
        Ok(())
    }
    pub fn faults(&self) -> [CancellationToken; 2] {
        [self.process.fault.clone(), self.control.fault.clone()]
    }
    pub async fn stop(mut self, evidence: &Path) -> Result<Vec<String>> {
        let previously_healthy = self.healthy().is_ok();
        self.control.expect_shutdown();
        // Tor may close immediately instead of sending a final 250 reply.
        let _ = self.control.command("SIGNAL SHUTDOWN").await;
        let output = self
            .process
            .shutdown("owned_tor_shutdown", Duration::from_secs(10))
            .await?;
        save(evidence, "tor", &output)?;
        let events = self.control.finish().await;
        ensure!(
            previously_healthy && output.success && output.valid_output() && !output.forced_kill,
            "owned Tor did not stop with complete qualification evidence"
        );
        events
    }
}
async fn bootstrap(
    process: &mut Process,
    port: &Path,
    cookie: &Path,
    evidence: &Path,
    socks: std::net::SocketAddr,
) -> Result<Control> {
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        ensure!(
            !process.exited()? && !process.fault.is_cancelled(),
            "Tor exited before control readiness; inspect private Tor output"
        );
        if port.try_exists()? && cookie.try_exists()? {
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "Tor control readiness exceeded 300-second bootstrap budget"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    files::regular(port)?;
    files::regular(cookie)?;
    let port_text = String::from_utf8(files::read(port)?)?;
    let address = port_text
        .trim()
        .strip_prefix("PORT=")
        .context("unrecognized Tor control endpoint file")?
        .parse()?;
    let mut control = Control::connect(
        address,
        &files::read(cookie)?,
        &evidence.join("tor-events.log"),
    )
    .await?;
    let settings = control.command("GETCONF SocksPort").await?;
    ensure!(
        settings.iter().any(|line| line
            .split_ascii_whitespace()
            .any(|field| field.eq_ignore_ascii_case("IsolateSOCKSAuth"))),
        "Tor SOCKS authentication isolation is not enabled"
    );
    let listeners = control.command("GETINFO net/listeners/socks").await?;
    let listener = listeners
        .iter()
        .find_map(|line| line.strip_prefix("250-net/listeners/socks="))
        .context("Tor SOCKS listener evidence missing")?;
    let addresses = super::audit::fields(listener)?;
    ensure!(
        addresses.len() == 1 && addresses[0].parse::<std::net::SocketAddr>()? == socks,
        "Tor opened unexpected SOCKS listeners"
    );
    let mut last = None;
    loop {
        ensure!(
            Instant::now() < deadline,
            "Tor bootstrap exceeded its single 300-second budget"
        );
        ensure!(
            !process.exited()? && !process.fault.is_cancelled(),
            "Tor exited during bootstrap"
        );
        let status = control.command("GETINFO status/bootstrap-phase").await?;
        let progress = status
            .iter()
            .flat_map(|line| line.split_ascii_whitespace())
            .find_map(|s| s.strip_prefix("PROGRESS="))
            .context("Tor bootstrap progress missing")?
            .parse::<u8>()?;
        ensure!(progress <= 100, "invalid Tor bootstrap percentage");
        if last != Some(progress) {
            eprintln!(
                "dedicated Tor bootstrap: {progress}% (one 300-second budget; persistent state retained)"
            );
            last = Some(progress);
        }
        if progress == 100 {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    control.command("SETEVENTS STREAM CIRC").await?;
    Ok(control)
}
fn save(directory: &Path, label: &str, output: &Evidence) -> Result<()> {
    files::publish(
        &directory.join(format!("{label}.stdout")),
        &output.stdout.bytes,
    )?;
    files::publish(
        &directory.join(format!("{label}.stderr")),
        &output.stderr.bytes,
    )?;
    let record = serde_json::json!({"success":output.success,"exit_code":output.exit_code,"forced_kill":output.forced_kill,"reason":output.reason,"valid_output":output.valid_output(),
        "stdout_limit_exceeded":output.stdout.limit_exceeded,"stderr_limit_exceeded":output.stderr.limit_exceeded,
        "stdout_read_error":output.stdout.read_error,"stderr_read_error":output.stderr.read_error,"stdout_pipe_incomplete":output.stdout.pipe_incomplete,"stderr_pipe_incomplete":output.stderr.pipe_incomplete});
    files::publish(
        &directory.join(format!("{label}.process.json")),
        &serde_json::to_vec_pretty(&record)?,
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    #[tokio::test]
    #[ignore = "subprocess-only Tor control fixture; invoked by its parent"]
    async fn fake_tor() {
        let Ok(root) = std::env::var("FAKE_TOR_DATA") else {
            return;
        };
        let data = Path::new(&root);
        assert!(
            data.is_absolute(),
            "Tor data path must survive child cwd changes"
        );
        let counter = data.join("fixture-starts");
        let n = std::fs::read_to_string(&counter)
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0)
            + 1;
        std::fs::write(counter, n.to_string()).unwrap();
        let control = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socks: std::net::SocketAddr = std::env::var("FAKE_TOR_SOCKS")
            .unwrap()
            .split_ascii_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let _socks = tokio::net::TcpListener::bind(socks).await.unwrap();
        let port = std::env::var("FAKE_TOR_PORT").unwrap();
        let cookie = std::env::var("FAKE_TOR_COOKIE").unwrap();
        files::publish(Path::new(&cookie), &[7; 32]).unwrap();
        files::publish(
            Path::new(&port),
            format!("PORT={}\n", control.local_addr().unwrap()).as_bytes(),
        )
        .unwrap();
        let (stream, _) = control.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        while let Some(command) = lines.next_line().await.unwrap() {
            let reply = if command.starts_with("AUTHENTICATE ") {
                "250 OK\r\n".to_owned()
            } else if command == "GETCONF SocksPort" {
                format!("250 SocksPort={socks} IsolateSOCKSAuth\r\n")
            } else if command == "GETINFO net/listeners/socks" {
                format!("250-net/listeners/socks=\"{socks}\"\r\n250 OK\r\n")
            } else if command == "GETINFO status/bootstrap-phase" {
                if data.join("hold-bootstrap").exists() {
                    std::fs::write(data.join("bootstrap-waiting"), b"waiting").unwrap();
                    "250-status/bootstrap-phase=NOTICE BOOTSTRAP PROGRESS=0 TAG=starting SUMMARY=\"Starting\"\r\n250 OK\r\n".into()
                } else {
                    "250-status/bootstrap-phase=NOTICE BOOTSTRAP PROGRESS=100 TAG=done SUMMARY=\"Done\"\r\n250 OK\r\n".into()
                }
            } else if command == "SETEVENTS STREAM CIRC" {
                "250 OK\r\n".into()
            } else if command == "SIGNAL SHUTDOWN" {
                write.write_all(b"250 OK\r\n").await.unwrap();
                break;
            } else {
                panic!("unexpected fake Tor command");
            };
            write.write_all(reply.as_bytes()).await.unwrap();
        }
    }
    fn shell_quote(path: &Path) -> String {
        format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"))
    }
    #[tokio::test]
    async fn owned_process_persists_data_and_control_shutdown_reaps_only_itself() {
        use std::os::unix::fs::PermissionsExt;
        let cwd = std::env::current_dir().unwrap();
        let tmp = tempfile::tempdir_in(cwd.join("target")).unwrap();
        let root = tmp.path().strip_prefix(&cwd).unwrap();
        assert!(root.is_relative());
        let script = root.join("tor-fixture");
        let contents = format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then printf 'Tor fixture\n'; exit 0; fi
while [ "$#" -gt 0 ]; do
  case "$1" in
    --DataDirectory) export FAKE_TOR_DATA="$2"; shift ;;
    --ControlPortWriteToFile) export FAKE_TOR_PORT="$2"; shift ;;
    --CookieAuthFile) export FAKE_TOR_COOKIE="$2"; shift ;;
    --SocksPort) export FAKE_TOR_SOCKS="$2"; shift ;;
  esac
  shift
done
exec {} --exact tor::owned::tests::fake_tor --ignored --nocapture
"#,
            shell_quote(&std::env::current_exe().unwrap())
        );
        std::fs::write(&script, contents).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socks = listener.local_addr().unwrap();
        drop(listener);
        let policy = NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(socks),
            ..Default::default()
        };
        let data = root.join("persistent");
        for n in 1..=2 {
            let evidence = root.join(format!("session-{n}"));
            files::create_dir(&evidence).unwrap();
            let mut tor = Tor::start(&script, &data, &evidence, &policy)
                .await
                .unwrap();
            tor.healthy().unwrap();
            assert_eq!(
                std::fs::read_to_string(data.join("fixture-starts")).unwrap(),
                n.to_string()
            );
            assert!(tor.stop(&evidence).await.unwrap().is_empty());
            // The owned listener is closed, and the next instance can reuse the data/port.
            let probe = tokio::net::TcpListener::bind(socks).await.unwrap();
            drop(probe);
            let result: serde_json::Value =
                serde_json::from_slice(&files::read(&evidence.join("tor.process.json")).unwrap())
                    .unwrap();
            assert_eq!(result["forced_kill"], false);
            assert_eq!(result["success"], true);
        }
        // Cancellation during bootstrap must still reap the owned process and
        // retain process evidence, rather than abandoning a Tor listener.
        std::fs::write(data.join("hold-bootstrap"), b"hold").unwrap();
        let evidence = root.join("cancelled");
        files::create_dir(&evidence).unwrap();
        let stop = CancellationToken::new();
        let cancel = async {
            tokio::time::timeout(Duration::from_secs(3), async {
                while !data.join("bootstrap-waiting").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            stop.cancel();
        };
        let (result, ()) = tokio::join!(
            Tor::start_with_cancel(&script, &data, &evidence, &policy, stop.clone()),
            cancel
        );
        assert!(result.err().unwrap().to_string().contains("cancelled"));
        let probe = tokio::net::TcpListener::bind(socks).await.unwrap();
        drop(probe);
        let result: serde_json::Value =
            serde_json::from_slice(&files::read(&evidence.join("tor.process.json")).unwrap())
                .unwrap();
        assert_eq!(result["forced_kill"], false);
        assert_eq!(result["valid_output"], true);
    }
}
