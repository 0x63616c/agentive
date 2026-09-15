use super::DynModelProvider;
use crate::errors::RunError;
use crate::message::{Message, MessageRole};
use crate::model::{AllOrError, ModelRequest, UsageEstimator};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) fn validate_history(history: &[Message]) -> Result<(), RunError> {
    let mut pending_calls = HashMap::new();
    for message in history {
        match message.role {
            MessageRole::System => {
                return Err(RunError::ProviderProtocol(
                    "trusted instructions cannot appear in canonical history".to_string(),
                ));
            }
            MessageRole::Assistant => {
                if let Some(calls) = &message.tool_calls {
                    for call in calls {
                        if pending_calls
                            .insert(call.id.as_str(), call.name.as_str())
                            .is_some()
                        {
                            return Err(RunError::ProviderProtocol(
                                "duplicate tool call correlation id".to_string(),
                            ));
                        }
                    }
                }
            }
            MessageRole::Tool => {
                let Some(id) = message.tool_call_id.as_deref() else {
                    return Err(RunError::ProviderProtocol(
                        "tool result is missing correlation id".to_string(),
                    ));
                };
                let Some(expected_name) = pending_calls.remove(id) else {
                    return Err(RunError::ProviderProtocol(
                        "tool result has unknown correlation id".to_string(),
                    ));
                };
                if message.name.as_ref().map(crate::ids::ToolName::as_str) != Some(expected_name) {
                    return Err(RunError::ProviderProtocol(
                        "tool result name does not match correlated call".to_string(),
                    ));
                }
            }
            MessageRole::User => {}
        }
    }
    if pending_calls.is_empty() {
        Ok(())
    } else {
        Err(RunError::ProviderProtocol(
            "history ends with unresolved tool calls".to_string(),
        ))
    }
}

pub(crate) fn validate_context(
    provider: &Arc<dyn DynModelProvider>,
    mode: AllOrError,
    provider_enforced_limit_opt_out: bool,
    request: &ModelRequest,
    estimator: &UsageEstimator,
) -> Result<(), RunError> {
    let capabilities = provider.capabilities();
    if !request.tools.is_empty() && !capabilities.supports_tool_calls {
        return Err(RunError::Capability(
            "tool calls are not supported".to_string(),
        ));
    }
    if request.messages.iter().any(|message| {
        message
            .content
            .iter()
            .any(|content| matches!(content, crate::MessageContent::Image { .. }))
    }) && !capabilities.supports_images
    {
        return Err(RunError::Capability("images are not supported".to_string()));
    }
    estimator
        .can_admit(
            mode,
            &request.messages,
            &capabilities,
            request.max_output_tokens,
            provider_enforced_limit_opt_out,
        )
        .map_err(RunError::ContextLimit)
}
