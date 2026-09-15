use crate::ids::{IdempotencyKey, ToolInvocationId, ToolName};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;
use tokio::sync::Notify;

/// A shared, runtime-neutral cancellation signal for a run or tool invocation.
#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: std::sync::Arc<AtomicBool>,
    notifier: std::sync::Arc<Notify>,
    reason: std::sync::Arc<std::sync::Mutex<Option<CancellationReason>>>,
}
impl CancellationToken {
    /// Creates a token that has not been cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cancelled: std::sync::Arc::new(AtomicBool::new(false)),
            notifier: std::sync::Arc::new(Notify::new()),
            reason: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }
    /// Marks the token cancelled and wakes all waiters.
    pub fn cancel(&self, reason: CancellationReason) {
        if let Ok(mut recorded_reason) = self.reason.lock()
            && recorded_reason.is_none()
        {
            *recorded_reason = Some(reason);
            self.cancelled.store(true, Ordering::Release);
            drop(recorded_reason);
            self.notifier.notify_waiters();
        }
    }
    /// Returns whether cancellation has already been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    /// Waits for cancellation and returns its recorded reason.
    pub async fn cancelled(&self) -> CancellationReason {
        loop {
            let notified = self.notifier.notified();
            if self.is_cancelled() {
                return self
                    .reason
                    .lock()
                    .ok()
                    .and_then(|reason| reason.clone())
                    .unwrap_or(CancellationReason::Runtime);
            }
            notified.await;
        }
    }
    pub(crate) fn flag(&self) -> std::sync::Arc<AtomicBool> {
        std::sync::Arc::clone(&self.cancelled)
    }
    pub(crate) fn notifier(&self) -> std::sync::Arc<Notify> {
        std::sync::Arc::clone(&self.notifier)
    }
}
impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable identity and policy context for a tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolInvocation {
    /// Stable identity of the owning run.
    pub run_id: String,
    /// Stable logical identity of this invocation.
    pub invocation_id: ToolInvocationId,
    /// Provider-supplied call identity, when available.
    pub provider_call_id: Option<String>,
    /// Validated name of the invoked tool.
    pub tool_name: ToolName,
    /// Zero-based provider model round that requested the invocation.
    pub model_round: u32,
    /// Caller-supplied key used to deduplicate the logical invocation.
    pub idempotency_key: IdempotencyKey,
}

/// Cause supplied when a running operation is cancelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancellationReason {
    /// A user requested cancellation.
    User,
    /// The parent run cancelled this operation.
    Parent,
    /// The operation exhausted its deadline.
    Timeout,
    /// The owning runtime cancelled the operation.
    Runtime,
}

#[cfg(test)]
mod tests {
    use super::{CancellationReason, CancellationToken};
    use std::sync::Arc;

    #[tokio::test]
    async fn cancellation_wakes_waiters_and_retains_the_first_reason() {
        let token = CancellationToken::new();
        let waiting = token.clone();
        let waiter = tokio::spawn(async move { waiting.cancelled().await });
        tokio::task::yield_now().await;
        token.cancel(CancellationReason::Parent);
        token.cancel(CancellationReason::User);
        match waiter.await {
            Ok(reason) => assert_eq!(reason, CancellationReason::Parent),
            Err(error) => panic!("waiter failed: {error}"),
        }
        assert_eq!(token.cancelled().await, CancellationReason::Parent);
    }

    #[tokio::test]
    async fn racing_cancellers_publish_one_canonical_reason() {
        let token = CancellationToken::new();
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let first = {
            let token = token.clone();
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                token.cancel(CancellationReason::User);
            })
        };
        let second = {
            let token = token.clone();
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                token.cancel(CancellationReason::Timeout);
            })
        };
        barrier.wait().await;
        assert!(first.await.is_ok());
        assert!(second.await.is_ok());

        let reason = token.cancelled().await;
        assert!(matches!(
            reason,
            CancellationReason::User | CancellationReason::Timeout
        ));
        assert_eq!(token.cancelled().await, reason);
    }
}

/// Runtime context supplied to a tool invocation.
#[derive(Debug, Clone)]
pub struct ToolContext {
    invocation: ToolInvocation,
    attempt: u8,
    remaining_time: Option<std::time::Duration>,
    cancellation: CancellationToken,
    delegation_depth: u8,
    max_delegation_depth: u8,
}
impl ToolContext {
    pub(crate) fn new(
        invocation: ToolInvocation,
        attempt: u8,
        remaining_time: Option<std::time::Duration>,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            invocation,
            attempt,
            remaining_time,
            cancellation,
            delegation_depth: 0,
            max_delegation_depth: 4,
        }
    }
    /// Creates the canonical context supplied by a local or external runtime.
    #[must_use]
    pub fn for_runtime(
        invocation: ToolInvocation,
        attempt: u8,
        remaining_time: Option<std::time::Duration>,
        cancellation: CancellationToken,
    ) -> Self {
        Self::new(invocation, attempt, remaining_time, cancellation)
    }
    pub(crate) fn for_runtime_with_delegation_depth(
        invocation: ToolInvocation,
        attempt: u8,
        remaining_time: Option<std::time::Duration>,
        cancellation: CancellationToken,
        delegation_depth: u8,
        max_delegation_depth: u8,
    ) -> Self {
        let mut context = Self::new(invocation, attempt, remaining_time, cancellation);
        context.delegation_depth = delegation_depth;
        context.max_delegation_depth = max_delegation_depth;
        context
    }
    pub(crate) fn delegation_depth(&self) -> u8 {
        self.delegation_depth
    }
    pub(crate) fn max_delegation_depth(&self) -> u8 {
        self.max_delegation_depth
    }
    /// Invocation identity.
    pub fn invocation(&self) -> &ToolInvocation {
        &self.invocation
    }
    /// Current retry attempt, starting at one.
    pub fn attempt(&self) -> u8 {
        self.attempt
    }
    /// Remaining runtime budget, if bounded.
    pub fn remaining(&self) -> Option<std::time::Duration> {
        self.remaining_time
    }
    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    /// Wait for cancellation.
    pub async fn cancelled(&self) -> CancellationReason {
        self.cancellation.cancelled().await
    }
    /// Records cancellation for this context and wakes waiters.
    pub async fn set_cancelled(&self, reason: CancellationReason) {
        self.cancellation.cancel(reason);
    }
}

/// Error returned while validating model-supplied tool arguments.
#[derive(Debug, Error)]
pub enum ToolDecodeError {
    #[error("tool argument decode failed: {0}")]
    /// The arguments contain fields not accepted by the tool.
    UnknownFields(String),
    #[error("tool schema validation failed: {0}")]
    /// The arguments do not satisfy the declared tool schema.
    InvalidSchema(String),
}
