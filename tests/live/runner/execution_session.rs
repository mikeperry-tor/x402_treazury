//! Every application incarnation gets a distinct registry-bound session. Launch
//! inputs remain pinned; a new session cannot extend execution/funding authority.
use super::*;
use crate::process::ProcessSlot;
use std::path::PathBuf;
use x402_treazury::qualification::Binding;

pub(super) struct Launcher<'a> {
    pub binary: &'a Path,
    pub directory: &'a Path,
    pub frozen: &'a Path,
    pub environment: &'a BTreeMap<String, String>,
    pub profile: Option<&'a Path>,
    pub managed: bool,
    pub new_funding: bool,
    pub expires_at: i64,
}
pub(super) struct Session {
    pub child: ProcessSlot,
    pub directory: PathBuf,
    pub binding: Binding,
    closed: Option<Value>,
}
pub(super) struct Application<'a> {
    launcher: Launcher<'a>,
    pub current: Session,
    completed: Vec<Value>,
}
impl<'a> Application<'a> {
    pub(super) fn start(
        launcher: Launcher<'a>,
        registry: &Registry,
        manifest: &Manifest,
    ) -> Result<Self> {
        let current = launcher.launch(registry, manifest, None)?;
        Ok(Self {
            launcher,
            current,
            completed: Vec::new(),
        })
    }
    pub(super) fn running(&mut self) -> Result<&mut Process> {
        self.current.child.running()
    }
    pub(super) fn config(&self) -> Result<MetaConfig> {
        ensure!(
            files::hash_file(self.launcher.frozen)? == self.current.binding.config_sha256,
            "application configuration changed during run"
        );
        Ok(toml::from_str(std::str::from_utf8(&files::read(
            self.launcher.frozen,
        )?)?)?)
    }
    pub(super) async fn close(
        &mut self,
        registry: &Registry,
        manifest: &Manifest,
        config: &MetaConfig,
        reason: &str,
    ) -> Result<Value> {
        if let Some(closed) = &self.current.closed {
            return Ok(closed.clone());
        }
        self.current
            .child
            .begin_shutdown(reason, Duration::from_secs(manifest.limits.cleanup_seconds))?;
        let result = self.current.child.finish_shutdown().await;
        // No await after consuming cleanup evidence: a cancelled waiter either
        // retains the drain task or finishes publishing this session as a unit.
        self.current.closed = Some(
            json!({"session":self.current.binding.session,"complete":false,"cover":{"status":"invalid_or_incomplete"}}),
        );
        let evidence = result?;
        save_process(&self.current.directory, &evidence)?;
        let process = json!({"session":self.current.binding.session,"success":evidence.success,
            "pid":evidence.pid,"exit_code":evidence.exit_code,"reason":evidence.reason,
            "forced_kill":evidence.forced_kill,"valid_output":evidence.valid_output()});
        registry.event(&manifest.run_id, "child_finished", &process, now()?)?;
        let cover = cover_report::collect(config, manifest, &evidence);
        let cover_value = cover
            .as_ref()
            .cloned()
            .unwrap_or_else(|_| json!({"status":"invalid_or_incomplete"}));
        registry.event(
            &manifest.run_id,
            "cover_observed",
            &json!({"session":self.current.binding.session,"cover":cover_value}),
            now()?,
        )?;
        let closed = json!({"session":self.current.binding.session,
            "complete":evidence.success && !evidence.forced_kill && evidence.valid_output() && cover.is_ok(),
            "process":process,"cover":cover_value});
        self.current.closed = Some(closed.clone());
        Ok(closed)
    }
    pub(super) fn reopen(&mut self, registry: &Registry, manifest: &Manifest) -> Result<()> {
        let closed = self
            .current
            .closed
            .as_ref()
            .context("application must drain before restart")?;
        ensure!(
            closed["complete"] == true,
            "incomplete application cleanup prevents restart"
        );
        ensure!(
            self.completed.len() < manifest.phases.len(),
            "application restarts exceed declared phase bound"
        );
        let pins = registry.pins(&manifest.run_id)?;
        let catalogs = pins
            .catalogs
            .as_ref()
            .context("restart catalog pins missing")?;
        catalogs.verify(manifest, &pins)?;
        if let Some(profile) = self.launcher.profile {
            ensure!(
                catalogs.files.get("client.sb") == Some(&files::hash_file(profile)?),
                "restart confinement profile changed"
            );
        }
        let next = self
            .launcher
            .launch(registry, manifest, Some(&self.current.binding))?;
        self.completed.push(closed.clone());
        self.current = next;
        Ok(())
    }
    pub(super) fn summaries(&self) -> Vec<Value> {
        self.completed
            .iter()
            .chain(self.current.closed.iter())
            .cloned()
            .collect()
    }
}
impl Launcher<'_> {
    fn arguments(&self, binding_file: &Path) -> Vec<String> {
        let mut args = vec![
            "serve".into(),
            "--config".into(),
            self.frozen.to_string_lossy().into_owned(),
            if self.managed {
                "--qualification-managed"
            } else {
                "--qualification-unsigned"
            }
            .into(),
            "--qualification-parent-stdin".into(),
            "--qualification-binding".into(),
            binding_file.to_string_lossy().into_owned(),
        ];
        if !self.new_funding {
            args.push("--qualification-no-new-funding".into());
        }
        args
    }
    fn launch(
        &self,
        registry: &Registry,
        manifest: &Manifest,
        previous: Option<&Binding>,
    ) -> Result<Session> {
        ensure!(
            now()? < self.expires_at,
            "application authority expired; no process started"
        );
        // The caller verifies executable/catalog pins before invoking this factory.
        // Process::launch_confined clears the environment; ambient keys/proxies and
        // dotenv paths must never enter the child through this refactoring.
        let directory = self
            .directory
            .join(format!("session-{}", uuid::Uuid::new_v4()));
        files::create_dir(&directory)?;
        let binding = registry.application_binding(
            &manifest.run_id,
            directory
                .file_name()
                .context("session name missing")?
                .to_str()
                .context("session name must be UTF8")?,
            files::hash_file(self.frozen)?,
            self.expires_at,
        )?;
        if let Some(previous) = previous {
            unchanged_authority(previous, &binding)?;
        }
        let binding_file = directory.join("binding.json");
        files::publish(&binding_file, &serde_json::to_vec(&binding)?)?;
        registry.event(
            &manifest.run_id,
            "application_session",
            &serde_json::to_value(&binding)?,
            now()?,
        )?;
        registry.event(
            &manifest.run_id,
            "funding_execution",
            &json!({"session":binding.session,
            "enabled":self.new_funding,"new_job_limit":manifest.limits.new_funding_jobs,
            "source_limit_zec":manifest.limits.source_exposure_zec}),
            now()?,
        )?;
        let process = Process::launch_confined(
            self.binary,
            &self.arguments(&binding_file),
            self.environment,
            self.directory,
            files::DOCUMENT_BYTES,
            self.profile,
        )?;
        Ok(Session {
            child: ProcessSlot::new(process),
            directory,
            binding,
            closed: None,
        })
    }
}
fn unchanged_authority(previous: &Binding, next: &Binding) -> Result<()> {
    ensure!(
        previous.session != next.session
            && previous.version == next.version
            && previous.run == next.run
            && previous.registry == next.registry
            && previous.pin_digest == next.pin_digest
            && previous.config_sha256 == next.config_sha256
            && previous.expires_at == next.expires_at,
        "restart changed authority/pins or reused an application session"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restarted_binding_changes_only_session_identity() {
        let previous = Binding {
            version: 1,
            run: "run".into(),
            session: "old".into(),
            registry: "registry.sqlite".into(),
            pin_digest: "pins".into(),
            config_sha256: "config".into(),
            expires_at: 100,
        };
        let mut next = previous.clone();
        assert!(unchanged_authority(&previous, &next).is_err());
        next.session = "new".into();
        unchanged_authority(&previous, &next).unwrap();
        for (field, value) in [
            ("expires_at", json!(101)),
            ("run", json!("another")),
            ("pin_digest", json!("new")),
            ("config_sha256", json!("new")),
            ("registry", json!("elsewhere")),
            ("version", json!(2)),
        ] {
            let mut changed = serde_json::to_value(&next).unwrap();
            changed[field] = value;
            assert!(
                unchanged_authority(&previous, &serde_json::from_value(changed).unwrap()).is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn every_incarnation_keeps_the_parent_pipe_mode_and_funding_restriction() {
        let environment = BTreeMap::new();
        for managed in [false, true] {
            for new_funding in [false, true] {
                let launcher = Launcher {
                    binary: Path::new("app"),
                    directory: Path::new("private"),
                    frozen: Path::new("frozen.toml"),
                    environment: &environment,
                    profile: None,
                    managed,
                    new_funding,
                    expires_at: 100,
                };
                for name in ["first-binding.json", "second-binding.json"] {
                    let args = launcher.arguments(Path::new(name));
                    assert!(args.iter().any(|a| a == "--qualification-parent-stdin"));
                    assert_eq!(args.iter().any(|a| a == "--qualification-managed"), managed);
                    assert_eq!(
                        args.iter().any(|a| a == "--qualification-unsigned"),
                        !managed
                    );
                    assert_eq!(
                        args.iter().any(|a| a == "--qualification-no-new-funding"),
                        !new_funding
                    );
                    let index = args
                        .iter()
                        .position(|a| a == "--qualification-binding")
                        .unwrap();
                    assert_eq!(args[index + 1], name);
                    assert_eq!(&args[..3], &["serve", "--config", "frozen.toml"]);
                }
            }
        }
    }
}
