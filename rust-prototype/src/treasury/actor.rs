//! Bounded command queue for the single mutable wallet owner. Once accepted,
//! a command runs to completion even if its caller drops the reply receiver.
use super::Treasury;
use crate::rotation::transaction::{
    PrepareRequest, PreparedTransaction, SubmissionOutcome, TransactionPreparer,
    TransactionPresence, TransactionSubmission,
};
use anyhow::{Context, Result};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

enum Command {
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
                        Command::Refund(job, reply) => { let _ = reply.send(self.refund_address(job).await); }
                        Command::Sync(reply) => { let _ = reply.send(self.sync_once(&stop).await); }
                        Command::Prepare(request, reply) => {
                            let result = match self.sync_once(&stop).await {
                                Ok(()) if !stop.is_cancelled() => self.prepare(request).await,
                                Ok(()) => Err(anyhow::anyhow!("treasury stopping")),
                                Err(error) => Err(error),
                            };
                            let _ = reply.send(result);
                        }
                        Command::Submit(id, retry, reply) => { let _ = reply.send(self.submit_prepared(id, &mut submission, retry, &stop).await); }
                        Command::Reconcile(id, reply) => { let _ = reply.send(self.reconcile_prepared(id, &mut submission, &stop).await); }
                    }
                }
                _ = interval.tick() => {
                    if self.sync_once(&stop).await.is_err() && !stop.is_cancelled() {
                        tracing::warn!("treasury sync unavailable");
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
