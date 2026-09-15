//! Versioned Temporal-only input and observation contracts.

use agentive::{AgentRunEffect, AgentRunState, ToolName};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The only supported durable adapter wire schema.
pub const ADAPTER_SCHEMA_VERSION: u16 = 1;
/// Fingerprint of the deterministic workflow behavior shipped by this release.
pub const WORKFLOW_FINGERPRINT: &str = "agentive-temporal-workflow-v1";

/// Validated durable workflow implementation identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkflowFingerprint(String);

impl WorkflowFingerprint {
    /// Parses a portable fingerprint recorded in workflow input.
    pub fn parse(value: impl Into<String>) -> Result<Self, TemporalRunConfigError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(TemporalRunConfigError::InvalidWorkflowFingerprint);
        }
        Ok(Self(value))
    }

    /// Returns the workflow identity supported by this binary.
    #[must_use]
    pub fn current() -> Self {
        Self(WORKFLOW_FINGERPRINT.to_string())
    }

    /// Borrows the portable fingerprint.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for WorkflowFingerprint {
    fn default() -> Self {
        Self::current()
    }
}

/// Immutable Temporal-specific policy carried beside canonical Agentive state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalRunConfig {
    /// Version of the durable adapter contract.
    pub schema_version: u16,
    /// Deterministic workflow implementation expected by this history.
    #[serde(default)]
    pub workflow_fingerprint: WorkflowFingerprint,
    /// Maximum completed effects in one Temporal execution before Continue-As-New.
    pub continue_as_new_after_effects: u32,
    /// Tool names whose implementations explicitly honor the stable Agentive idempotency key
    /// across Temporal activity redelivery.
    #[serde(default)]
    pub durable_idempotent_tools: BTreeSet<String>,
    /// Whether the configured provider durably deduplicates the stable provider-effect identity
    /// across Temporal activity redelivery.
    ///
    /// This remains opt-in because a provider call can have externally visible effects even when
    /// Agentive's local retry classifier permits another transport attempt.
    #[serde(default)]
    pub durable_idempotent_provider: bool,
}

impl Default for TemporalRunConfig {
    fn default() -> Self {
        Self {
            schema_version: ADAPTER_SCHEMA_VERSION,
            workflow_fingerprint: WorkflowFingerprint::current(),
            continue_as_new_after_effects: u32::MAX,
            durable_idempotent_tools: BTreeSet::new(),
            durable_idempotent_provider: false,
        }
    }
}

impl TemporalRunConfig {
    /// Creates a supported deterministic continuation policy.
    pub fn new(continue_as_new_after_effects: u32) -> Result<Self, TemporalRunConfigError> {
        if continue_as_new_after_effects == 0 {
            return Err(TemporalRunConfigError::ZeroContinuationThreshold);
        }
        Ok(Self {
            schema_version: ADAPTER_SCHEMA_VERSION,
            workflow_fingerprint: WorkflowFingerprint::current(),
            continue_as_new_after_effects,
            durable_idempotent_tools: BTreeSet::new(),
            durable_idempotent_provider: false,
        })
    }

    /// Explicitly permits a tool implementation that durably deduplicates its invocation key.
    #[must_use]
    pub fn allow_durable_idempotent_tool(mut self, name: ToolName) -> Self {
        self.durable_idempotent_tools.insert(name.to_string());
        self
    }

    /// Explicitly permits provider redelivery when the provider durably deduplicates the stable
    /// Agentive provider-effect identity across Temporal activity attempts.
    #[must_use]
    pub fn allow_durable_idempotent_provider(mut self) -> Self {
        self.durable_idempotent_provider = true;
        self
    }

    #[must_use]
    pub(crate) fn activity_max_attempts(&self, effect: &AgentRunEffect) -> u32 {
        match effect {
            AgentRunEffect::ProviderCall { .. } if !self.durable_idempotent_provider => 1,
            AgentRunEffect::ProviderCall { .. }
            | AgentRunEffect::ToolCall { .. }
            | AgentRunEffect::ToolBatch { .. } => 2,
        }
    }

    pub(crate) fn validates_effect(
        &self,
        effect: &AgentRunEffect,
    ) -> Result<(), TemporalRunConfigError> {
        let calls: Vec<_> = match effect {
            AgentRunEffect::ProviderCall { .. } => return Ok(()),
            AgentRunEffect::ToolCall { invocation, .. } => vec![&invocation.tool_name],
            AgentRunEffect::ToolBatch { calls, .. } => calls
                .iter()
                .map(|call| &call.invocation.tool_name)
                .collect(),
        };
        for name in calls {
            if !self.durable_idempotent_tools.contains(name.as_str()) {
                return Err(TemporalRunConfigError::UnsafeDurableTool {
                    name: name.to_string(),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<(), TemporalRunConfigError> {
        if self.schema_version != ADAPTER_SCHEMA_VERSION {
            return Err(TemporalRunConfigError::UnsupportedSchemaVersion {
                received: self.schema_version,
            });
        }
        if self.workflow_fingerprint.as_str() != WORKFLOW_FINGERPRINT {
            return Err(TemporalRunConfigError::UnsupportedWorkflowFingerprint {
                received: self.workflow_fingerprint.as_str().to_string(),
            });
        }
        if self.continue_as_new_after_effects == 0 {
            return Err(TemporalRunConfigError::ZeroContinuationThreshold);
        }
        Ok(())
    }
}

/// Typed error for an invalid durable adapter contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemporalRunConfigError {
    /// A history requests an adapter version this binary does not understand.
    #[error("unsupported Temporal adapter schema version {received}")]
    UnsupportedSchemaVersion {
        /// Version found in durable workflow input.
        received: u16,
    },
    /// A zero threshold would never make forward progress.
    #[error("continue-as-new threshold must be nonzero")]
    ZeroContinuationThreshold,
    /// A tool cannot run durably unless the caller explicitly opts into its stable-key deduplication.
    #[error("tool {name} is not declared durably idempotent")]
    UnsafeDurableTool {
        /// Stable tool name omitted from the explicit durable-idempotency allowlist.
        name: String,
    },
    /// A fingerprint contains characters that cannot be persisted portably.
    #[error("workflow fingerprint is invalid")]
    InvalidWorkflowFingerprint,
    /// A history targets workflow behavior not supported by this binary.
    #[error("unsupported workflow fingerprint {received}")]
    UnsupportedWorkflowFingerprint {
        /// Fingerprint found in durable workflow input.
        received: String,
    },
}

/// Full durable workflow input; the Agentive state remains the single source of agent truth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalWorkflowInput {
    /// Canonical state machine snapshot.
    pub state: AgentRunState,
    /// Immutable Temporal-specific adapter policy.
    pub config: TemporalRunConfig,
    /// Count of effects committed across this Continue-As-New chain.
    pub completed_effects: u64,
    /// Count of effects committed in the current Temporal execution.
    #[serde(default)]
    pub effects_in_execution: u32,
}

impl TemporalWorkflowInput {
    /// Starts a durable run with no prior committed Temporal effect.
    #[must_use]
    pub fn new(state: AgentRunState, config: TemporalRunConfig) -> Self {
        Self {
            state,
            config,
            completed_effects: 0,
            effects_in_execution: 0,
        }
    }
}

/// Opaque monotonic observation cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TemporalRunCursor(u64);

impl TemporalRunCursor {
    pub(crate) const fn from_completed_effects(completed_effects: u64) -> Self {
        Self(completed_effects)
    }

    pub(crate) const fn terminal() -> Self {
        Self(u64::MAX)
    }
}

/// Point-in-time durable state returned by the workflow query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalRunSnapshot {
    /// Canonical state at the cursor.
    pub state: AgentRunState,
    /// Opaque monotonically increasing committed-progress cursor.
    pub cursor: TemporalRunCursor,
}

#[cfg(test)]
#[allow(clippy::expect_used)] // Fixture construction keeps the contract assertion legible.
mod tests {
    use super::{TemporalRunConfig, TemporalRunConfigError, WorkflowFingerprint};
    use agentive::{
        AgentRunBudget, AgentRunState, ModelFinishReason, ModelResponse, ModelToolCall,
    };

    #[test]
    fn unsupported_durable_adapter_version_is_rejected() {
        let config = TemporalRunConfig {
            schema_version: 2,
            ..TemporalRunConfig::default()
        };
        assert_eq!(
            config.validate(),
            Err(TemporalRunConfigError::UnsupportedSchemaVersion { received: 2 })
        );
    }

    #[test]
    fn unsupported_workflow_fingerprint_is_rejected() {
        let config = TemporalRunConfig {
            workflow_fingerprint: WorkflowFingerprint("agentive-temporal-workflow-v2".to_string()),
            ..TemporalRunConfig::default()
        };
        assert_eq!(
            config.validate(),
            Err(TemporalRunConfigError::UnsupportedWorkflowFingerprint {
                received: "agentive-temporal-workflow-v2".to_string(),
            })
        );
    }

    #[test]
    fn tool_effect_requires_explicit_durable_idempotency() {
        let mut state = AgentRunState::new(
            "durable-tool",
            Vec::new(),
            AgentRunBudget {
                model_call_limit: 1,
            },
        );
        state
            .commit_provider_response(
                "durable-tool:provider:0",
                ModelResponse {
                    text: None,
                    tool_calls: vec![ModelToolCall {
                        call_id: "call".into(),
                        name: "write".parse().expect("fixture tool name"),
                        arguments: serde_json::json!({}),
                        provider_call_id: None,
                    }],
                    usage: None,
                    finish_reason: ModelFinishReason::ToolCalls,
                },
            )
            .expect("provider response should select the tool");
        let effect = state.next_effect().expect("tool effect");
        assert_eq!(
            TemporalRunConfig::default().validates_effect(&effect),
            Err(TemporalRunConfigError::UnsafeDurableTool {
                name: "write".into()
            })
        );
        let config = TemporalRunConfig::default()
            .allow_durable_idempotent_tool("write".parse().expect("fixture tool name"));
        assert!(config.validates_effect(&effect).is_ok());
        assert_eq!(config.activity_max_attempts(&effect), 2);
    }

    #[test]
    fn provider_redelivery_requires_explicit_durable_idempotency() {
        let state = AgentRunState::new(
            "durable-provider",
            Vec::new(),
            AgentRunBudget {
                model_call_limit: 1,
            },
        );
        let effect = state.next_effect().expect("provider effect");
        assert_eq!(
            TemporalRunConfig::default().activity_max_attempts(&effect),
            1
        );
        assert_eq!(
            TemporalRunConfig::default()
                .allow_durable_idempotent_provider()
                .activity_max_attempts(&effect),
            2
        );
    }
}
