use super::{RunEvent, RunResult, RunStatus};
use crate::RunId;
use crate::errors::RunError;
use crate::tool::{CancellationReason, CancellationToken};
use futures::stream::unfold;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::sync::{mpsc, watch};

#[derive(Clone)]
pub(super) struct AtomicRunStatus(Arc<AtomicU8>);
impl AtomicRunStatus {
    pub(super) fn new(status: RunStatus) -> Self {
        Self(Arc::new(AtomicU8::new(to_status_code(status))))
    }
    pub(super) fn get(&self) -> RunStatus {
        from_status_code(self.0.load(Ordering::SeqCst))
    }
    pub(super) fn set(&self, status: RunStatus) {
        self.0.store(to_status_code(status), Ordering::SeqCst);
    }
}

/// Handle for observing, cancelling, and awaiting a local run.
#[derive(Clone)]
pub struct RunHandle {
    pub(super) run_id: RunId,
    pub(super) status: Arc<AtomicRunStatus>,
    pub(super) cancellation_token: CancellationToken,
    pub(super) result_tx: watch::Sender<Option<Result<RunResult, RunError>>>,
    pub(super) events_tx: EventBus,
}
impl RunHandle {
    /// Opaque identity allocated for this run.
    #[must_use]
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }
    /// Current status snapshot.
    pub fn status(&self) -> RunStatus {
        self.status.get()
    }
    /// Request cancellation.
    pub fn cancel(&self) {
        self.cancellation_token.cancel(CancellationReason::User);
    }
    /// Request cancellation with a runtime-provided reason.
    #[allow(dead_code)] // Used by the optional delegation runtime.
    pub(crate) fn cancel_with(&self, reason: CancellationReason) {
        self.cancellation_token.cancel(reason);
    }
    /// Subscribe to lifecycle events.
    pub fn events(&self) -> impl futures::Stream<Item = RunEvent> {
        let receiver = self.events_tx.subscribe();
        unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|item| (item, receiver))
        })
    }
    /// Wait for terminal completion.
    pub async fn wait(&self) -> Result<RunResult, RunError> {
        let mut result = self.result_tx.subscribe();
        loop {
            if let Some(result) = result.borrow().clone() {
                return result;
            }
            result
                .changed()
                .await
                .map_err(|_| RunError::Provider("run task cancelled unexpectedly".to_string()))?;
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct EventBus {
    observers: Arc<std::sync::Mutex<Vec<mpsc::Sender<RunEvent>>>>,
    parent: Option<Arc<EventBus>>,
    child_run_id: Option<String>,
}
#[derive(Clone)]
pub(crate) struct RunObserver {
    events: EventBus,
}
impl EventBus {
    pub(super) fn observer(&self) -> RunObserver {
        RunObserver {
            events: self.clone(),
        }
    }
    pub(super) fn observed_by(observer: RunObserver, run_id: String) -> Self {
        Self {
            observers: Arc::new(std::sync::Mutex::new(Vec::new())),
            parent: Some(Arc::new(observer.events)),
            child_run_id: Some(run_id),
        }
    }
    pub(super) fn subscribe(&self) -> mpsc::Receiver<RunEvent> {
        let (sender, receiver) = mpsc::channel(256);
        if let Ok(mut observers) = self.observers.lock() {
            observers.push(sender);
        }
        receiver
    }
    pub(super) async fn emit(&self, event: RunEvent) {
        let observers = self
            .observers
            .lock()
            .map_or_else(|_| Vec::new(), |senders| senders.clone());
        for observer in observers {
            let _ = observer.send(event.clone()).await;
        }
        if let Ok(mut observers) = self.observers.lock() {
            observers.retain(|observer| !observer.is_closed());
        }
        if let (Some(parent), Some(run_id)) = (&self.parent, &self.child_run_id) {
            Box::pin(parent.emit(RunEvent::Child {
                run_id: run_id.clone(),
                event: Box::new(event),
            }))
            .await;
        }
    }
}
pub(super) async fn emit_event(events: &EventBus, event: RunEvent) {
    events.emit(event).await;
}
fn to_status_code(status: RunStatus) -> u8 {
    match status {
        RunStatus::Pending => 0,
        RunStatus::Running => 1,
        RunStatus::Completed => 2,
        RunStatus::Incomplete => 3,
        RunStatus::Failed => 4,
        RunStatus::Cancelled => 5,
    }
}
fn from_status_code(value: u8) -> RunStatus {
    match value {
        0 => RunStatus::Pending,
        1 => RunStatus::Running,
        2 => RunStatus::Completed,
        3 => RunStatus::Incomplete,
        4 => RunStatus::Failed,
        _ => RunStatus::Cancelled,
    }
}
