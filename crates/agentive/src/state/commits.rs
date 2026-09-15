//! Effect completion and terminal transitions.

use super::{AgentRunEffect, AgentRunState, DelegationBudget, StateTransitionError};
use crate::{
    AgentEffectOutcome, Message, ModelResponse, ProviderAttempt, ProviderError, RunStatus, RunUsage,
};
use serde_json::{Value, json};

impl AgentRunState {
    /// Commits a provider response after the matching provider effect completed exactly once.
    pub fn commit_provider_response(
        &mut self,
        effect_id: &str,
        response: ModelResponse,
    ) -> Result<(), StateTransitionError> {
        self.commit_provider_response_with_attempts(effect_id, response, Vec::new())
    }

    fn commit_provider_response_with_attempts(
        &mut self,
        effect_id: &str,
        response: ModelResponse,
        attempts: Vec<ProviderAttempt>,
    ) -> Result<(), StateTransitionError> {
        self.expect_effect(effect_id, true)?;
        self.status = RunStatus::Running;
        self.reserve_provider_attempt_tokens(&attempts);
        self.provider_calls_started = self
            .provider_calls_started
            .max(self.model_calls)
            .saturating_add(provider_call_count(&attempts));
        self.model_calls = self.model_calls.saturating_add(1);
        self.record_provider_attempts(attempts, response.usage.clone());
        if !response.tool_calls.is_empty() {
            self.history.push(Message::assistant_tool_calls(
                response
                    .tool_calls
                    .clone()
                    .into_iter()
                    .map(|call| crate::ToolCall {
                        id: call.call_id,
                        name: call.name,
                        arguments: call.arguments,
                    })
                    .collect(),
            ));
            self.pending_tools = response.tool_calls;
        } else if let Some(text) = response.text {
            self.history.push(Message::assistant_text(text.clone()));
            self.text = Some(text);
            self.status = RunStatus::Completed;
        } else {
            self.status = RunStatus::Incomplete;
            self.usage.complete = false;
        }
        Ok(())
    }

    /// Commits one typed outcome without requiring a runtime to branch on effect kind.
    pub fn commit_effect(
        &mut self,
        effect_id: &str,
        outcome: AgentEffectOutcome,
    ) -> Result<(), StateTransitionError> {
        match outcome {
            AgentEffectOutcome::Provider { response, attempts } => {
                self.commit_provider_response_with_attempts(effect_id, response, attempts)
            }
            AgentEffectOutcome::ProviderFailed { error, attempts } => {
                self.commit_provider_failure(effect_id, error, attempts)
            }
            AgentEffectOutcome::ProviderBudgetExhausted { .. } => {
                self.expect_effect(effect_id, true)?;
                self.status = RunStatus::Incomplete;
                self.usage.complete = false;
                Ok(())
            }
            AgentEffectOutcome::Tool {
                output,
                child_usage,
                ..
            } => self.commit_tool_result_with_usage(effect_id, output, child_usage),
            AgentEffectOutcome::ToolBatch { outputs } => self.commit_tool_batch(effect_id, outputs),
        }
    }

    /// Commits the first pending tool effect. Its result is correlated to the provider call.
    pub fn commit_tool_result(
        &mut self,
        effect_id: &str,
        output: Value,
    ) -> Result<(), StateTransitionError> {
        self.commit_tool_result_with_usage(effect_id, output, None)
    }

    /// Commits a safe tool failure as a correlated repair result.
    pub fn commit_tool_error(
        &mut self,
        effect_id: &str,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<(), StateTransitionError> {
        self.commit_tool_result(
            effect_id,
            json!({ "error": { "code": code.into(), "message": message.into() } }),
        )
    }

    /// Records cancellation as a terminal transition.
    pub fn cancel(&mut self) {
        if matches!(self.status, RunStatus::Pending | RunStatus::Running) {
            self.status = RunStatus::Cancelled;
            self.usage.complete = false;
        }
    }

    /// Records a safe terminal runtime failure.
    pub fn fail(&mut self, message: impl Into<String>) {
        if matches!(self.status, RunStatus::Pending | RunStatus::Running) {
            self.status = RunStatus::Failed;
            self.error = Some(message.into());
        }
    }

    fn commit_provider_failure(
        &mut self,
        effect_id: &str,
        error: ProviderError,
        attempts: Vec<ProviderAttempt>,
    ) -> Result<(), StateTransitionError> {
        self.expect_effect(effect_id, true)?;
        self.reserve_provider_attempt_tokens(&attempts);
        self.provider_calls_started = self
            .provider_calls_started
            .max(self.model_calls)
            .saturating_add(provider_call_count(&attempts));
        self.model_calls = self.model_calls.saturating_add(1);
        self.record_provider_attempts(attempts, error.usage().cloned());
        self.fail(error.message);
        Ok(())
    }

    fn record_provider_attempts(
        &mut self,
        attempts: Vec<ProviderAttempt>,
        legacy_or_final_usage: Option<crate::ModelTokenUsage>,
    ) {
        if attempts.is_empty() {
            self.usage.add_call("model", legacy_or_final_usage);
        } else {
            for attempt in attempts {
                self.usage.add_call("model", attempt.usage);
            }
        }
    }

    fn reserve_provider_attempt_tokens(&mut self, attempts: &[ProviderAttempt]) {
        let provider_derived = (!attempts.is_empty())
            .then(|| {
                attempts.iter().try_fold(0_u64, |total, attempt| {
                    (attempt.reserved_tokens > 0)
                        .then(|| total.saturating_add(attempt.reserved_tokens))
                })
            })
            .flatten();
        let reservation = provider_derived.unwrap_or_else(|| {
            self.next_model_token_reservation()
                .saturating_mul(u64::from(provider_call_count(attempts)))
        });
        self.reserved_tokens = self.reserved_tokens.saturating_add(reservation);
    }

    fn commit_tool_batch(
        &mut self,
        effect_id: &str,
        outputs: Vec<crate::ToolEffectResult>,
    ) -> Result<(), StateTransitionError> {
        let reservations = match self.next_effect() {
            Some(AgentRunEffect::ToolBatch { calls, .. }) => calls
                .into_iter()
                .map(|call| call.delegation_budget)
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        self.expect_batch(effect_id, outputs.len())?;
        for (output, reservation) in outputs.into_iter().zip(reservations) {
            let Some(call) = self.pending_tools.first().cloned() else {
                return Err(StateTransitionError::UnexpectedEffect {
                    received: effect_id.into(),
                    expected: None,
                });
            };
            if let Some(child_usage) = output.child_usage {
                self.usage.merge_child(&child_usage);
            }
            self.reserve_child(reservation);
            self.history
                .push(Message::tool_result(call.call_id, call.name, output.output));
            self.pending_tools.remove(0);
        }
        Ok(())
    }

    fn commit_tool_result_with_usage(
        &mut self,
        effect_id: &str,
        output: Value,
        child_usage: Option<RunUsage>,
    ) -> Result<(), StateTransitionError> {
        let reservation = self
            .pending_tools
            .first()
            .and_then(|call| self.delegation_budget_for(call));
        self.expect_effect(effect_id, false)?;
        let Some(call) = self.pending_tools.first().cloned() else {
            return Err(StateTransitionError::UnexpectedEffect {
                received: effect_id.to_string(),
                expected: None,
            });
        };
        if let Some(child_usage) = child_usage {
            self.usage.merge_child(&child_usage);
        }
        self.reserve_child(reservation);
        self.history
            .push(Message::tool_result(call.call_id, call.name, output));
        self.pending_tools.remove(0);
        Ok(())
    }

    fn reserve_child(&mut self, reservation: Option<DelegationBudget>) {
        if let Some(reservation) = reservation {
            self.reserved_child_model_calls = self
                .reserved_child_model_calls
                .saturating_add(reservation.model_call_limit);
            self.reserved_tokens = self
                .reserved_tokens
                .saturating_add(reservation.token_limit.unwrap_or(0));
        }
    }

    fn expect_effect(&self, received: &str, provider: bool) -> Result<(), StateTransitionError> {
        let effect = self.next_effect();
        let expected = effect.as_ref().map(effect_id);
        let valid = matches!((&effect, provider), (Some(AgentRunEffect::ProviderCall { effect_id, .. }), true) if effect_id == received)
            || matches!((&effect, provider), (Some(AgentRunEffect::ToolCall { effect_id, .. }), false) if effect_id == received);
        if valid {
            Ok(())
        } else {
            Err(StateTransitionError::UnexpectedEffect {
                received: received.into(),
                expected,
            })
        }
    }

    fn expect_batch(&self, received: &str, count: usize) -> Result<(), StateTransitionError> {
        let effect = self.next_effect();
        let expected = effect.as_ref().map(effect_id);
        if matches!(&effect, Some(AgentRunEffect::ToolBatch { effect_id, calls }) if effect_id == received && calls.len() == count)
        {
            Ok(())
        } else {
            Err(StateTransitionError::UnexpectedEffect {
                received: received.into(),
                expected,
            })
        }
    }
}

fn effect_id(effect: &AgentRunEffect) -> String {
    match effect {
        AgentRunEffect::ProviderCall { effect_id, .. }
        | AgentRunEffect::ToolCall { effect_id, .. }
        | AgentRunEffect::ToolBatch { effect_id, .. } => effect_id.clone(),
    }
}

fn provider_call_count(attempts: &[ProviderAttempt]) -> u32 {
    u32::try_from(attempts.len()).unwrap_or(u32::MAX).max(1)
}
