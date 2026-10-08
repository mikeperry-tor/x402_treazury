//! Bounded command queue for the single mutable wallet owner. Executing commands
//! retain their durable results when reply receivers disappear. Shutdown discards
//! queued work and cancels interruptible sync/submission according to their contracts.
use super::Treasury;
use crate::rotation::transaction::{
    PrepareRequest, PreparedTransaction, SubmissionOutcome, TransactionPreparer,
    TransactionPresence, TransactionSubmission,
};
use anyhow::{Context, Result};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

enum Command {
    RecoverFunding(
        String,
        u32,
        oneshot::Sender<Result<super::recovery::RecoveryOutcome>>,
    ),
    Refund(String, oneshot::Sender<Result<String>>),
    Sync(oneshot::Sender<Result<()>>),
    Prepare(PrepareRequest, oneshot::Sender<Result<PreparedTransaction>>),
    Submit(String, bool, oneshot::Sender<Result<SubmissionOutcome>>),
    Reconcile(String, oneshot::Sender<Result<TransactionPresence>>),
}
#[derive(Clone)]
pub struct TreasuryHandle {
    sender: mpsc::Sender<Command>,
}
pub struct TreasuryCommands {
    receiver: mpsc::Receiver<Command>,
}
pub fn channel() -> (TreasuryHandle, TreasuryCommands) {
    let (sender, receiver) = mpsc::channel(8);
    (TreasuryHandle { sender }, TreasuryCommands { receiver })
}
impl TreasuryHandle {
    pub async fn recover_funding(
        &self,
        id: String,
        max_attempts: u32,
    ) -> Result<super::recovery::RecoveryOutcome> {
        self.call(|reply| Command::RecoverFunding(id, max_attempts, reply))
            .await
    }

    async fn call<T>(&self, make: impl FnOnce(oneshot::Sender<Result<T>>) -> Command) -> Result<T> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(make(send))
            .await
            .map_err(|_| anyhow::anyhow!("treasury owner stopped"))?;
        recv.await.context("treasury owner stopped")?
    }
    pub async fn refund_address(&self, job: String) -> Result<String> {
        self.call(|reply| Command::Refund(job, reply)).await
    }
    pub async fn sync(&self) -> Result<()> {
        self.call(Command::Sync).await
    }
    pub async fn prepare(&self, request: PrepareRequest) -> Result<PreparedTransaction> {
        self.call(|reply| Command::Prepare(request, reply)).await
    }
    pub async fn submit(&self, id: String, retry: bool) -> Result<SubmissionOutcome> {
        self.call(|reply| Command::Submit(id, retry, reply)).await
    }
    pub async fn reconcile(&self, id: String) -> Result<TransactionPresence> {
        self.call(|reply| Command::Reconcile(id, reply)).await
    }
}
impl Treasury {
    pub async fn run_commands(
        mut self,
        mut commands: TreasuryCommands,
        mut submission: impl TransactionSubmission,
        stop: CancellationToken,
    ) -> Result<()> {
        let delay = self
            .sync_settings
            .as_ref()
            .context("sync endpoint not configured")?
            .max_age_seconds
            .saturating_div(2)
            .clamp(1, 60);
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(delay));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                command = commands.receiver.recv() => {
                    let Some(command) = command else { break };
                    match command {
                        Command::RecoverFunding(id, max_attempts, reply) => { let _ = reply.send(self.recover_funding_job(id, max_attempts, &mut submission, &stop).await); }

                        Command::Refund(job, reply) => { let _ = reply.send(self.refund_address(job).await); }
                        Command::Sync(reply) => { let _ = reply.send(self.sync_once(&stop).await); }
                        Command::Prepare(request, reply) => {
                            let result = match self.sync_once(&stop).await {
                                Ok(()) if !stop.is_cancelled() => self.prepare(request).await,
                                Ok(()) => Err(anyhow::anyhow!("treasury stopping")),
                                Err(error) if self.healthy => Err(error.context(crate::rotation::transaction::PreparationDeferred)),
                                Err(error) => Err(error),
                            };
                            let _ = reply.send(result);
                        }
                        Command::Submit(id, retry, reply) => { let _ = reply.send(self.submit_prepared(id, &mut submission, retry, &stop).await); }
                        Command::Reconcile(id, reply) => { let _ = reply.send(self.reconcile_prepared(id, &mut submission, &stop).await); }
                    }
                }
                _ = interval.tick() => {
                    if let Err(error) = self.sync_once(&stop).await
                        && !stop.is_cancelled() {
                            super::diagnostics::warn_sync_failure(&error);
                    }
                }
            }
            if !self.healthy {
                stop.cancel();
                commands.receiver.close();
                drop(commands);
                self.close().await?;
                anyhow::bail!("treasury requires restart after interrupted mutation");
            }
        }
        // Discard queued work; accepted mutations above have finished and persisted.
        commands.receiver.close();
        drop(commands);
        self.close().await
    }
}

// Observe the real command boundary without substituting a fake FundingBackend.
#[cfg(test)]
pub(crate) enum ObservedCommand {
    RecoverFunding(String, u32),
    Prepare(PrepareRequest),
    Submit(String, bool),
    Reconcile(String),
}
#[cfg(test)]
impl TreasuryCommands {
    pub(crate) fn test_is_empty(&self) -> bool {
        self.receiver.is_empty()
    }
    pub(crate) async fn test_observe(&mut self) -> ObservedCommand {
        match self.receiver.recv().await.expect("command channel closed") {
            Command::RecoverFunding(id, limit, reply) => {
                let _ = reply.send(Ok(super::recovery::RecoveryOutcome::Waiting(
                    "waiting_for_transaction_expiry",
                )));
                ObservedCommand::RecoverFunding(id, limit)
            }
            Command::Prepare(request, reply) => {
                let _ = reply.send(Err(anyhow::anyhow!("fixture command observed")));
                ObservedCommand::Prepare(request)
            }
            Command::Submit(id, retry, reply) => {
                let _ = reply.send(Err(anyhow::anyhow!("fixture command observed")));
                ObservedCommand::Submit(id, retry)
            }
            Command::Reconcile(id, reply) => {
                let _ = reply.send(Err(anyhow::anyhow!("fixture command observed")));
                ObservedCommand::Reconcile(id)
            }
            _ => panic!("unexpected treasury command"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoSubmission;
    impl TransactionSubmission for NoSubmission {
        async fn submit(
            &mut self,
            _: crate::rotation::transaction::BroadcastTransaction,
        ) -> Result<SubmissionOutcome> {
            panic!("unexpected submission")
        }
        async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
            panic!("unexpected lookup")
        }
    }
    #[tokio::test]
    async fn dropped_receivers_preserve_mutations_but_shutdown_discards_queued_work() {
        for stopped in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut treasury=Treasury::create(dir.path().join("state"),dir.path().join("key"),2_000_000,Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
            treasury
                .ensure_pool("actor".into(), "5".into())
                .await
                .unwrap();
            let status = treasury.status().await.unwrap();
            let id = status.treasury_id;
            let job = status.funding_jobs[0].id.clone();
            treasury.configure_sync(
                super::super::SyncSettings::new("http://127.0.0.1:1".into(), 3, 300).unwrap(),
            );
            let (handle, commands) = channel();
            let stop = CancellationToken::new();
            let (send, recv) = oneshot::channel();
            handle
                .sender
                .send(Command::Refund(job.clone(), send))
                .await
                .unwrap();
            drop(recv);
            let (send, reply) = oneshot::channel();
            handle
                .sender
                .send(Command::Refund(job.clone(), send))
                .await
                .unwrap();
            if stopped {
                stop.cancel();
            }
            let task = tokio::spawn(treasury.run_commands(commands, NoSubmission, stop.clone()));
            let address = tokio::time::timeout(std::time::Duration::from_secs(5), reply)
                .await
                .unwrap();
            if stopped {
                assert!(address.is_err());
            } else {
                assert!(address.unwrap().unwrap().starts_with('t'));
            }
            stop.cancel();
            task.await.unwrap().unwrap();
            assert!(handle.refund_address(job.clone()).await.is_err());
            let restored = Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
                .await
                .unwrap();
            let saved = restored
                .store
                .call(move |s| s.refund_address(&job))
                .await
                .unwrap();
            assert_eq!(saved.is_some(), !stopped);
            assert!(
                restored
                    .status()
                    .await
                    .unwrap()
                    .treasury_operations
                    .is_empty()
            );
            restored.close().await.unwrap();
        }
    }
    #[tokio::test]
    async fn failed_sync_returns_typed_unstarted_response_without_reservation() {
        let dir = tempfile::tempdir().unwrap();
        let mut treasury = Treasury::create(
            dir.path().join("state"), dir.path().join("key"), 2_000_000,
            Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))
        ).await.unwrap();
        treasury.configure_sync(
            super::super::SyncSettings::new("http://127.0.0.1:1".into(), 3, 300).unwrap(),
        );
        let id = treasury.status().await.unwrap().treasury_id;
        let (handle, commands) = channel();
        let stop = CancellationToken::new();
        let request = PrepareRequest {
            allocation_limits: None,
            operation_id: uuid::Uuid::new_v4().to_string(),
            pool_id: None,
            daily_limit_zatoshis: 200000,
            deadline: u64::MAX,
            recipient: "not-used-before-sync".into(),
            amount_zatoshis: 1,
            max_fee_zatoshis: 10000,
            max_input_zatoshis: 20000,
        };
        let (send, reply) = oneshot::channel();
        handle
            .sender
            .send(Command::Prepare(request, send))
            .await
            .unwrap();
        let task = tokio::spawn(treasury.run_commands(commands, NoSubmission, stop.clone()));
        let result = reply.await.unwrap();
        assert!(
            result
                .err()
                .unwrap()
                .is::<crate::rotation::transaction::PreparationDeferred>()
        );
        stop.cancel();
        task.await.unwrap().unwrap();
        let restored = Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
            .await
            .unwrap();
        assert!(
            restored
                .status()
                .await
                .unwrap()
                .treasury_operations
                .is_empty()
        );
        restored.close().await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_before_queue_admission_leaves_no_command() {
        let (handle, commands) = channel();
        for _ in 0..8 {
            let (send, _) = oneshot::channel();
            handle.sender.send(Command::Sync(send)).await.unwrap();
        }
        let mut pending = Box::pin(handle.sync());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
                .await
                .is_err()
        );
        drop(pending);
        assert_eq!(commands.receiver.len(), 8);
    }
}
