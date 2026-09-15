//! The non-deterministic activity boundary for one canonical Agentive effect.

use std::{sync::Arc, time::Duration};

use agentive::{
    Agent, AgentEffectOutcome, AgentRunEffect, AgentRunState, CancellationReason, CancellationToken,
};
use serde::{Deserialize, Serialize};
use temporalio_macros::activities;
use temporalio_sdk::activities::{ActivityContext, ActivityError};

const CANCELLATION_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const PROVIDER_CANCELLATION_ACK_TIMEOUT: Duration = Duration::from_secs(1);

/// The complete deterministic input to one durable effect activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectActivityInput {
    /// State snapshot that selected the effect.
    pub state: AgentRunState,
    /// Exact effect selected from that snapshot.
    pub effect: AgentRunEffect,
}

/// Activities bound to one immutable Agentive provider and tool definition.
pub struct AgentiveActivities {
    agent: Arc<Agent>,
}

impl AgentiveActivities {
    /// Binds activity execution to one configured Agentive agent.
    #[must_use]
    pub fn new(agent: Agent) -> Self {
        Self {
            agent: Arc::new(agent),
        }
    }
}

#[activities]
impl AgentiveActivities {
    /// Executes exactly one selected provider or tool effect outside workflow replay.
    #[allow(missing_docs)] // The Temporal macro generates an associated activity constant.
    #[activity]
    pub async fn execute_effect(
        self: Arc<Self>,
        context: ActivityContext,
        input: EffectActivityInput,
    ) -> Result<AgentEffectOutcome, ActivityError> {
        let cancellation = CancellationToken::new();
        let execution = self
            .agent
            .execute_effect(&input.state, input.effect, cancellation.clone());
        tokio::pin!(execution);
        let mut heartbeat = tokio::time::interval(CANCELLATION_HEARTBEAT_INTERVAL);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                () = context.cancelled() => {
                    cancellation.cancel(CancellationReason::Runtime);
                    // Give a cancellation-aware provider or tool one bounded opportunity to
                    // observe Agentive's token. Its result is deliberately discarded: Temporal
                    // cancellation, not a late effect outcome, owns this terminal transition.
                    let _ =
                        tokio::time::timeout(PROVIDER_CANCELLATION_ACK_TIMEOUT, &mut execution)
                            .await;
                    return Err(ActivityError::cancelled());
                }
                outcome = &mut execution => return outcome.map_err(ActivityError::from),
                _ = heartbeat.tick() => {
                    if let Err(error) = context.record_heartbeat(()).await {
                        if context.is_cancelled() {
                            cancellation.cancel(CancellationReason::Runtime);
                            let _ = tokio::time::timeout(
                                PROVIDER_CANCELLATION_ACK_TIMEOUT,
                                &mut execution,
                            )
                            .await;
                            return Err(ActivityError::cancelled());
                        }
                        return Err(error.into());
                    }
                }
            }
        }
    }
}
