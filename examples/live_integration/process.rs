//! Owned child handles, bounded private evidence, EOF shutdown and bounded reaping.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, ChildStdin},
    task::JoinHandle,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

/// Install before starting any children, including Tor/bootstrap inspectors.
/// Dropping the scope aborts watchers; it never leaves a detached signal task.
pub struct StopSignals {
    pub stop: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
}
impl StopSignals {
    pub fn install() -> Result<Self> {
        let stop = CancellationToken::new();
        let signal_stop = stop.clone();
        #[cfg(unix)]
        let task = {
            let mut interrupt =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            tokio::spawn(async move {
                tokio::select! {_ = interrupt.recv()=>{}, _ = terminate.recv()=>{}}
                eprintln!(
                    "qualification interrupted: deliberately draining owned children and retaining incomplete evidence"
                );
                signal_stop.cancel();
            })
        };
        #[cfg(not(unix))]
        let task = tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            signal_stop.cancel();
        });
        Ok(Self {
            stop,
            tasks: vec![task],
        })
    }
    pub fn observe_faults(&mut self, faults: [CancellationToken; 2]) {
        let stop = self.stop.clone();
        self.tasks.push(tokio::spawn(async move {
            tokio::select! {_ = faults[0].cancelled()=>{}, _ = faults[1].cancelled()=>{}}
            eprintln!(
                "owned Tor/control evidence failed: stopping progression and draining children"
            );
            stop.cancel();
        }));
    }
}
impl Drop for StopSignals {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[derive(Clone, Default, Serialize)]
pub struct Output {
    pub bytes: Vec<u8>,
    pub limit_exceeded: bool,
    pub read_error: bool,
    pub pipe_incomplete: bool,
}
#[derive(Serialize)]
pub struct Evidence {
    pub pid: u32,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub forced_kill: bool,
    pub reason: String,
    pub stdout: Output,
    pub stderr: Output,
}
impl Evidence {
    pub fn valid_output(&self) -> bool {
        [&self.stdout, &self.stderr]
            .iter()
            .all(|o| !o.limit_exceeded && !o.read_error && !o.pipe_incomplete)
    }
}
pub struct Process {
    child: Child,
    stdin: Option<ChildStdin>,
    pid: u32,
    pub fault: CancellationToken,
    output: [(Arc<Mutex<Output>>, JoinHandle<()>); 2],
}

/// Own a process across a cancellable restart/drain await. The cleanup task is
/// retained here, so dropping a waiter does not drop (and kill) the owned child.
/// The supervisor must await `finish_shutdown` outside its cancellation select.
pub struct ProcessSlot {
    running: Option<Process>,
    draining: Option<JoinHandle<Result<Evidence>>>,
}
impl ProcessSlot {
    pub fn new(process: Process) -> Self {
        Self {
            running: Some(process),
            draining: None,
        }
    }
    pub fn running(&mut self) -> Result<&mut Process> {
        self.running
            .as_mut()
            .context("application is draining or stopped; dispatch refused")
    }
    pub fn begin_shutdown(&mut self, reason: &str, cleanup: Duration) -> Result<()> {
        ensure!(
            !cleanup.is_zero() && cleanup <= Duration::from_secs(86400),
            "invalid child cleanup deadline"
        );
        if self.draining.is_some() {
            return Ok(()); // retain the original reason, deadline and child handle
        }
        let process = self.running.take().context("application already stopped")?;
        let reason = reason.to_owned();
        self.draining = Some(tokio::spawn(async move {
            process.shutdown(&reason, cleanup).await
        }));
        Ok(())
    }
    pub async fn finish_shutdown(&mut self) -> Result<Evidence> {
        let result = self
            .draining
            .as_mut()
            .context("application shutdown not started")?
            .await;
        self.draining.take();
        result.context("application cleanup task failed")?
    }
}
impl Process {
    pub fn launch_confined(
        binary: &Path,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Path,
        limit: usize,
        profile: Option<&Path>,
    ) -> Result<Self> {
        if let Some(profile) = profile {
            ensure!(
                cfg!(target_os = "macos"),
                "macOS confinement is unavailable on this platform"
            );
            super::files::regular(profile)?;
            let mut wrapped = vec![
                "-f".into(),
                profile
                    .to_str()
                    .context("profile path must be UTF8")?
                    .into(),
                binary.to_str().context("binary path must be UTF8")?.into(),
            ];
            wrapped.extend_from_slice(args);
            Self::launch(
                Path::new("/usr/bin/sandbox-exec"),
                &wrapped,
                env,
                cwd,
                limit,
            )
        } else {
            Self::launch(binary, args, env, cwd, limit)
        }
    }
    pub fn launch(
        binary: &Path,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Path,
        limit: usize,
    ) -> Result<Self> {
        ensure!(
            (1..=super::files::CATALOG_BYTES).contains(&limit),
            "child output limit must be 1..{} bytes per stream",
            super::files::CATALOG_BYTES
        );
        let mut command = tokio::process::Command::new(binary);
        command
            .args(args)
            .env_clear()
            .envs(env)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().context("cannot start pinned child")?;
        let pid = child.id().context("child PID missing")?;
        let stdin = child.stdin.take();
        let fault = CancellationToken::new();
        let out = capture(
            child.stdout.take().context("child stdout missing")?,
            limit,
            "stdout",
            fault.clone(),
        );
        let err = capture(
            child.stderr.take().context("child stderr missing")?,
            limit,
            "stderr",
            fault.clone(),
        );
        Ok(Self {
            child,
            stdin,
            pid,
            fault,
            output: [out, err],
        })
    }
    pub fn exited(&mut self) -> Result<bool> {
        Ok(self.child.try_wait()?.is_some())
    }
    pub async fn wait(
        mut self,
        timeout: Duration,
        cleanup: Duration,
        stop: CancellationToken,
    ) -> Result<Evidence> {
        let reason = tokio::select! {
            result = self.child.wait() => { result?; "exited" },
            _ = self.fault.cancelled() => "output_failure",
            _ = stop.cancelled() => "cancelled",
            _ = tokio::time::sleep(timeout) => "deadline",
        };
        self.shutdown(reason, cleanup).await
    }
    pub async fn shutdown(mut self, reason: &str, cleanup: Duration) -> Result<Evidence> {
        ensure!(
            !cleanup.is_zero() && cleanup <= Duration::from_secs(86400),
            "invalid child cleanup deadline"
        );
        // EOF is the production child's normal drain request. Never signal an arbitrary stored PID.
        self.stdin.take();
        let deadline = Instant::now() + cleanup;
        let mut forced = false;
        let status = match tokio::time::timeout_at(deadline, self.child.wait()).await {
            Ok(status) => status?,
            Err(_) => {
                eprintln!(
                    "child cleanup deadline exceeded; terminating owned child; accepted work must remain uncertain"
                );
                forced = true;
                self.child.start_kill()?;
                tokio::time::timeout(Duration::from_secs(5), self.child.wait())
                    .await
                    .context("owned child could not be reaped within 5 seconds")??
            }
        };
        for (output, task) in &mut self.output {
            match tokio::time::timeout_at(deadline, &mut *task).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => {
                    eprintln!("child output collector failed; evidence is incomplete");
                    output.lock().expect("capture lock").read_error = true;
                }
                Err(_) => {
                    eprintln!(
                        "child output pipe remained open at cleanup deadline; evidence is incomplete"
                    );
                    output.lock().expect("capture lock").pipe_incomplete = true;
                    task.abort();
                }
            }
        }
        let stdout = self.output[0].0.lock().expect("capture lock").clone();
        let stderr = self.output[1].0.lock().expect("capture lock").clone();
        Ok(Evidence {
            pid: self.pid,
            exit_code: status.code(),
            success: status.success(),
            forced_kill: forced,
            reason: reason.into(),
            stdout,
            stderr,
        })
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        // Last-resort cancellation path; callers must use shutdown to allow transaction drain.
        // Child::kill_on_drop is tied to its unreaped handle, not a persisted/reusable PID.
        self.stdin.take();
        for (_, task) in &self.output {
            task.abort();
        }
    }
}
fn capture<R: AsyncRead + Unpin + Send + 'static>(
    mut pipe: R,
    limit: usize,
    label: &'static str,
    fault: CancellationToken,
) -> (Arc<Mutex<Output>>, JoinHandle<()>) {
    let output = Arc::new(Mutex::new(Output::default()));
    let saved = output.clone();
    let task = tokio::spawn(async move {
        let mut buffer = [0; 8192];
        loop {
            match pipe.read(&mut buffer).await {
                Ok(0) => break,
                Ok(n) => {
                    let mut state = saved.lock().expect("capture lock");
                    let keep = n.min(limit.saturating_sub(state.bytes.len()));
                    state.bytes.extend_from_slice(&buffer[..keep]);
                    if keep < n && !state.limit_exceeded {
                        state.limit_exceeded = true;
                        eprintln!(
                            "child {label} exceeded configured {limit}-byte capture limit; output is incomplete; stopping child"
                        );
                        fault.cancel();
                    }
                    // Continue draining after visible failure until shutdown closes the child.
                }
                Err(_) => {
                    saved.lock().expect("capture lock").read_error = true;
                    eprintln!("child {label} capture failed; output evidence is incomplete");
                    fault.cancel();
                    break;
                }
            }
        }
    });
    (output, task)
}
#[cfg(test)]
mod tests {
    use super::*;
    async fn shell(script: &str, limit: usize, timeout: Duration) -> Evidence {
        let dir = tempfile::tempdir().unwrap();
        let child = Process::launch(
            Path::new("/bin/sh"),
            &["-c".into(), script.into()],
            &BTreeMap::new(),
            dir.path(),
            limit,
        )
        .unwrap();
        child
            .wait(
                timeout,
                Duration::from_millis(200),
                CancellationToken::new(),
            )
            .await
            .unwrap()
    }
    #[tokio::test]
    async fn bounded_output_and_minimal_environment_are_explicit() {
        let e = shell(
            "printf '%s' \"${EVM_PRIVATE_KEY-unset}\"",
            128,
            Duration::from_secs(2),
        )
        .await;
        assert_eq!(e.stdout.bytes, b"unset");
        assert!(e.success && e.valid_output());
        let e = shell("printf '123456789'", 4, Duration::from_secs(2)).await;
        assert_eq!(e.stdout.bytes, b"1234");
        assert!(e.stdout.limit_exceeded);
        assert!(!e.valid_output());
    }
    #[tokio::test]
    async fn deadline_and_inherited_output_pipes_cannot_hang_supervision() {
        let e = shell("exec /bin/sleep 5", 128, Duration::from_millis(20)).await;
        assert!(e.forced_kill);
        assert_eq!(e.reason, "deadline");
        let e = shell("/bin/sleep 1 & exit 0", 128, Duration::from_secs(2)).await;
        assert!(e.stdout.pipe_incomplete || e.stderr.pipe_incomplete);
    }
    #[tokio::test]
    async fn cancelled_drain_wait_preserves_child_cleanup_and_original_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let process = Process::launch(
            Path::new("/bin/sh"),
            &[
                "-c".into(),
                "while read line; do :; done; /bin/sleep 0.1; printf drained".into(),
            ],
            &BTreeMap::new(),
            dir.path(),
            1024,
        )
        .unwrap();
        let mut slot = ProcessSlot::new(process);
        assert!(slot.begin_shutdown("invalid", Duration::ZERO).is_err());
        assert!(slot.running().is_ok());
        slot.begin_shutdown("restart_checkpoint", Duration::from_secs(3))
            .unwrap();
        assert!(slot.running().is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(10), slot.finish_shutdown())
                .await
                .is_err()
        );
        // A cancellation cleanup path must join the same task, not shorten its
        // deadline or start another shutdown against a recorded PID.
        slot.begin_shutdown("cancelled", Duration::from_millis(1))
            .unwrap();
        let evidence = slot.finish_shutdown().await.unwrap();
        assert!(evidence.success && !evidence.forced_kill && evidence.valid_output());
        assert_eq!(evidence.reason, "restart_checkpoint");
        assert_eq!(evidence.stdout.bytes, b"drained");
        assert!(slot.running().is_err());
        assert!(slot.finish_shutdown().await.is_err());
        assert!(
            slot.begin_shutdown("again", Duration::from_secs(1))
                .is_err()
        );
    }
}
