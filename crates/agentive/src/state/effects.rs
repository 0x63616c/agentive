//! Effect selection and admission transitions.

use super::{
    AgentRunBudget, AgentRunEffect, AgentRunPlan, AgentRunState, DelegationBudget, ToolEffectCall,
};
use crate::{
    AllOrError, CompiledInstructions, IdempotencyKey, InstructionFragment, Message, ModelRequest,
    ModelToolCall, RunStatus, RunUsage, ToolInvocation, ToolInvocationId,
};

impl AgentRunState {
    /// Creates a new state whose first requested effect is a provider call.
    #[must_use]
    pub fn new(run_id: impl Into<String>, history: Vec<Message>, budget: AgentRunBudget) -> Self {
        Self::with_plan(
            run_id,
            history,
            budget,
            AgentRunPlan {
                instructions: CompiledInstructions {
                    core: InstructionFragment::new("sdk_core", 1, ""),
                    agent: None,
                    run: None,
                    features: Vec::new(),
                },
                tools: Vec::new(),
                model: None,
                max_output_tokens: 256,
                output_format: crate::ModelOutputFormat::Text,
                provider_max_attempts: 1,
                max_parallel_tool_calls: 4,
                context_estimate: AllOrError::Exact,
                provider_enforced_limit_opt_out: false,
                deadline_ms: None,
                delegation_depth: 0,
                max_delegation_depth: 4,
                token_limit: None,
            },
        )
    }

    /// Creates a state with an explicit immutable execution plan.
    #[must_use]
    pub fn with_plan(
        run_id: impl Into<String>,
        history: Vec<Message>,
        budget: AgentRunBudget,
        plan: AgentRunPlan,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            plan,
            history,
            usage: RunUsage::new(),
            budget,
            model_calls: 0,
            provider_calls_started: 0,
            elapsed_ms: 0,
            reserved_child_model_calls: 0,
            reserved_tokens: 0,
            pending_tools: Vec::new(),
            status: RunStatus::Pending,
            text: None,
            error: None,
        }
    }

    /// Returns the next effect without mutating state. Repeating this call returns the same ID.
    #[must_use]
    pub fn next_effect(&self) -> Option<AgentRunEffect> {
        if !matches!(self.status, RunStatus::Pending | RunStatus::Running) {
            return None;
        }
        if let Some(call) = self.pending_tools.first() {
            let batchable = self.pending_tools.len() > 1
                && self.pending_tools.iter().all(|pending| {
                    self.plan
                        .tool_policy(&pending.name)
                        .is_some_and(|policy| policy.parallel_safe)
                });
            if batchable {
                return Some(AgentRunEffect::ToolBatch {
                    effect_id: format!("{}:tool-batch:{}", self.run_id, call.call_id),
                    calls: self
                        .pending_tools
                        .iter()
                        .map(|pending| self.tool_effect_call(pending))
                        .collect(),
                });
            }
            let effect_id = format!("{}:tool:{}", self.run_id, call.call_id);
            return Some(AgentRunEffect::ToolCall {
                effect_id,
                invocation: self.tool_effect_call(call).invocation,
                arguments: call.arguments.clone(),
                policy: self.plan.tool_policy(&call.name).cloned(),
                delegation_budget: self.delegation_budget_for(call),
            });
        }
        if self.no_provider_budget() {
            return None;
        }
        let round = self.model_calls;
        let effect_id = format!("{}:provider:{round}", self.run_id);
        Some(AgentRunEffect::ProviderCall {
            provider_call_id: effect_id.clone(),
            effect_id: effect_id.clone(),
            round,
            request: self.next_model_request(effect_id),
        })
    }

    /// Transitions a non-terminal run to incomplete when its provider budget is exhausted.
    /// Returns whether this call performed the terminal transition.
    pub fn finish_if_exhausted(&mut self) -> bool {
        if matches!(self.status, RunStatus::Pending | RunStatus::Running)
            && self
                .plan
                .deadline_ms
                .is_some_and(|deadline| self.elapsed_ms >= deadline)
        {
            self.cancel();
            return true;
        }
        if matches!(self.status, RunStatus::Pending | RunStatus::Running)
            && self.pending_tools.is_empty()
            && self.no_provider_budget()
        {
            self.status = RunStatus::Incomplete;
            self.usage.complete = false;
            true
        } else {
            false
        }
    }

    /// Adds runtime-measured time after one effect completes.
    ///
    /// Durable runtimes must report the same externally measured duration on replay.
    pub fn record_elapsed(&mut self, elapsed_ms: u64) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(elapsed_ms);
    }

    pub(super) fn tool_effect_call(&self, call: &ModelToolCall) -> ToolEffectCall {
        ToolEffectCall {
            invocation: ToolInvocation {
                run_id: self.run_id.clone(),
                invocation_id: ToolInvocationId::from_stable(format!(
                    "{}:tool:{}",
                    self.run_id, call.call_id
                )),
                provider_call_id: call.provider_call_id.clone(),
                tool_name: call.name.clone(),
                model_round: self.model_calls.saturating_sub(1),
                idempotency_key: IdempotencyKey::new(format!("{}:{}", self.run_id, call.call_id)),
            },
            arguments: call.arguments.clone(),
            policy: self.plan.tool_policy(&call.name).cloned(),
            delegation_budget: self.delegation_budget_for(call),
        }
    }

    pub(super) fn delegation_budget_for(&self, call: &ModelToolCall) -> Option<DelegationBudget> {
        self.plan
            .tool_policy(&call.name)
            .filter(|policy| policy.delegation)?;
        let child_count = self
            .pending_tools
            .iter()
            .filter(|pending| {
                self.plan
                    .tool_policy(&pending.name)
                    .is_some_and(|policy| policy.delegation)
            })
            .count()
            .saturating_add(1);
        let divisor = u32::try_from(child_count).unwrap_or(u32::MAX);
        Some(DelegationBudget {
            model_call_limit: self.budget.model_call_limit.saturating_sub(
                self.provider_calls_started
                    .max(self.model_calls)
                    .saturating_add(self.reserved_child_model_calls),
            ) / divisor,
            token_limit: self
                .plan
                .token_limit
                .map(|limit| limit.saturating_sub(self.reserved_tokens) / u64::from(divisor)),
        })
    }

    pub(super) fn no_provider_budget(&self) -> bool {
        self.provider_calls_started
            .max(self.model_calls)
            .saturating_add(self.reserved_child_model_calls)
            >= self.budget.model_call_limit
            || self
                .plan
                .token_limit
                .is_some_and(|limit| self.reserved_tokens >= limit)
    }

    pub(crate) fn next_model_token_reservation(&self) -> u64 {
        let request =
            self.next_model_request(format!("{}:provider:{}", self.run_id, self.model_calls));
        let input_upper_bound = serde_json::to_vec(&request)
            .map(|bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .unwrap_or(u64::MAX);
        input_upper_bound.saturating_add(self.plan.max_output_tokens)
    }

    fn next_model_request(&self, effect_id: String) -> ModelRequest {
        ModelRequest {
            instructions: self.plan.instructions.clone(),
            messages: self.history.clone(),
            tools: self
                .plan
                .tools
                .iter()
                .map(|policy| policy.descriptor.clone())
                .collect(),
            include_context: true,
            model: self.plan.model.clone(),
            max_output_tokens: self.plan.max_output_tokens,
            output_format: self.plan.output_format.clone(),
            invocation_id: effect_id,
        }
    }
}
