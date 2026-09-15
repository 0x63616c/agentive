//! Versioned Temporal-only input and observation contracts.

use agentive::AgentRunState;
use serde::{Deserialize, Serialize};

/// The only supported durable adapter wire schema.
pub const ADAPTER_SCHEMA_VERSION: u16 = 1;

/// Immutable Temporal-specific policy carried beside canonical Agentive state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalRunConfig {
    /// Version of the durable adapter contract.
    pub schema_version: u16,
    /// Maximum completed effects in one Temporal execution before Continue-As-New.
    pub continue_as_new_after_effects: u32,
}

impl Default for TemporalRunConfig {
    fn default() -> Self {
        Self {
            schema_version: ADAPTER_SCHEMA_VERSION,
            continue_as_new_after_effects: u32::MAX,
        }
    }
}

impl TemporalRunConfig {
    /// Creates a supported deterministic continuation policy.
    pub const fn new(continue_as_new_after_effects: u32) -> Result<Self, TemporalRunConfigError> {
        if continue_as_new_after_effects == 0 {
            return Err(TemporalRunConfigError::ZeroContinuationThreshold);
        }
        Ok(Self {
            schema_version: ADAPTER_SCHEMA_VERSION,
            continue_as_new_after_effects,
        })
    }

    pub(crate) const fn validate(&self) -> Result<(), TemporalRunConfigError> {
        if self.schema_version != ADAPTER_SCHEMA_VERSION {
            return Err(TemporalRunConfigError::UnsupportedSchemaVersion {
                received: self.schema_version,
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
mod tests {
    use super::{TemporalRunConfig, TemporalRunConfigError};

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
}
