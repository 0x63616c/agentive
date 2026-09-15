use super::DynModelProvider;
use crate::errors::RunError;
use crate::message::{Message, MessageRole};
use crate::model::{
    AllOrError, ModelFinishReason, ModelOutputFormat, ModelRequest, ModelResponse,
    StructuredOutputSupport, UsageEstimator,
};
use std::collections::VecDeque;
use std::sync::Arc;

pub(crate) fn validate_history(history: &[Message]) -> Result<(), RunError> {
    let pending_calls = pending_history_calls(history)?;
    if pending_calls.is_empty() {
        Ok(())
    } else {
        Err(RunError::ProviderProtocol(
            "history ends with unresolved tool calls".to_string(),
        ))
    }
}

/// Validates the canonical history carried by a resumable effect snapshot.
///
/// A snapshot may end with exactly the tool calls in `state.pending_tools`; every other history
/// ordering and correlation rule is the same as a completed canonical history.
pub(crate) fn validate_effect_state_history(state: &crate::AgentRunState) -> Result<(), RunError> {
    let pending_calls = pending_history_calls(&state.history)?;
    if pending_calls.len() != state.pending_tools.len()
        || state
            .pending_tools
            .iter()
            .zip(pending_calls)
            .any(|(call, (id, name))| call.call_id != id || call.name.as_str() != name)
    {
        return Err(RunError::ProviderProtocol(
            "state pending tools do not match canonical history".to_string(),
        ));
    }
    Ok(())
}

fn pending_history_calls(history: &[Message]) -> Result<VecDeque<(&str, &str)>, RunError> {
    let mut pending_calls = VecDeque::new();
    for message in history {
        validate_message_shape(message)?;
        match message.role {
            MessageRole::System => {
                return Err(RunError::ProviderProtocol(
                    "trusted instructions cannot appear in canonical history".to_string(),
                ));
            }
            MessageRole::Assistant => {
                if !pending_calls.is_empty() {
                    return Err(RunError::ProviderProtocol(
                        "assistant message crossed unresolved tool calls".to_string(),
                    ));
                }
                if let Some(calls) = &message.tool_calls {
                    for call in calls {
                        if pending_calls.iter().any(|(id, _)| *id == call.id) {
                            return Err(RunError::ProviderProtocol(
                                "duplicate tool call correlation id".to_string(),
                            ));
                        }
                        pending_calls.push_back((call.id.as_str(), call.name.as_str()));
                    }
                }
            }
            MessageRole::Tool => {
                let Some(id) = message.tool_call_id.as_deref() else {
                    return Err(RunError::ProviderProtocol(
                        "tool result is missing correlation id".to_string(),
                    ));
                };
                let Some((expected_id, expected_name)) = pending_calls.pop_front() else {
                    return Err(RunError::ProviderProtocol(
                        "tool result has unknown correlation id".to_string(),
                    ));
                };
                if id != expected_id {
                    return Err(RunError::ProviderProtocol(
                        "tool result did not resolve the next provider-ordered tool call"
                            .to_string(),
                    ));
                }
                if message.name.as_ref().map(crate::ids::ToolName::as_str) != Some(expected_name) {
                    return Err(RunError::ProviderProtocol(
                        "tool result name does not match correlated call".to_string(),
                    ));
                }
            }
            MessageRole::User if !pending_calls.is_empty() => {
                return Err(RunError::ProviderProtocol(
                    "user message crossed unresolved tool calls".to_string(),
                ));
            }
            MessageRole::User => {}
        }
    }
    Ok(pending_calls)
}

fn validate_message_shape(message: &Message) -> Result<(), RunError> {
    let invalid = || {
        RunError::ProviderProtocol(format!(
            "invalid canonical {:?} message shape",
            message.role
        ))
    };
    match message.role {
        MessageRole::System => Err(RunError::ProviderProtocol(
            "trusted instructions cannot appear in canonical history".to_string(),
        )),
        MessageRole::User => {
            if message.content.is_empty()
                || message.tool_calls.is_some()
                || message.tool_call_id.is_some()
                || message.name.is_some()
            {
                Err(invalid())
            } else {
                Ok(())
            }
        }
        MessageRole::Assistant => {
            let calls = message.tool_calls.as_deref().unwrap_or_default();
            let has_content = !message.content.is_empty();
            let has_calls = !calls.is_empty();
            if message.tool_call_id.is_some()
                || message.name.is_some()
                || has_content == has_calls
                || calls
                    .iter()
                    .any(|call| call.id.trim().is_empty() || !call.arguments.is_object())
            {
                Err(invalid())
            } else {
                Ok(())
            }
        }
        MessageRole::Tool => {
            let valid_content = matches!(
                message.content.as_slice(),
                [crate::MessageContent::Text { text }]
                    if serde_json::from_str::<serde_json::Value>(text).is_ok()
            );
            if !valid_content
                || message.tool_calls.is_some()
                || message
                    .tool_call_id
                    .as_deref()
                    .is_none_or(|id| id.trim().is_empty())
                || message.name.is_none()
            {
                Err(invalid())
            } else {
                Ok(())
            }
        }
    }
}

pub(crate) fn validate_model_response(
    response: &ModelResponse,
    provider_call_id: &str,
) -> Result<(), RunError> {
    if response
        .text
        .as_deref()
        .is_some_and(|text| text.trim().is_empty())
    {
        return Err(RunError::ProviderProtocol(
            "provider response contained blank final text".to_string(),
        ));
    }
    if response.text.is_some() && !response.tool_calls.is_empty() {
        return Err(RunError::ProviderProtocol(
            "provider response mixed final text with tool calls".to_string(),
        ));
    }
    match (&response.finish_reason, response.tool_calls.is_empty()) {
        (ModelFinishReason::ToolCalls, true) => {
            return Err(RunError::ProviderProtocol(
                "tool-call finish reason contained no tool calls".to_string(),
            ));
        }
        (ModelFinishReason::Stop, false) | (ModelFinishReason::Error(_), _) => {
            return Err(RunError::ProviderProtocol(
                "provider finish reason did not match response content".to_string(),
            ));
        }
        (ModelFinishReason::Stop | ModelFinishReason::ToolCalls, _) => {}
    }
    if matches!(response.finish_reason, ModelFinishReason::Stop) && response.text.is_none() {
        return Err(RunError::ProviderProtocol(
            "provider stop response contained no final text".to_string(),
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for call in &response.tool_calls {
        if call.call_id.trim().is_empty() || !ids.insert(call.call_id.as_str()) {
            return Err(RunError::ProviderProtocol(
                "provider returned an invalid or duplicate tool-call id".to_string(),
            ));
        }
        if call
            .provider_call_id
            .as_deref()
            .is_some_and(|value| value != provider_call_id)
        {
            return Err(RunError::ProviderProtocol(
                "tool call was correlated to a different provider call".to_string(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_context(
    provider: &Arc<dyn DynModelProvider>,
    mode: AllOrError,
    provider_enforced_limit_opt_out: bool,
    request: &ModelRequest,
    estimator: &UsageEstimator,
) -> Result<Option<u64>, RunError> {
    let capabilities = provider.capabilities();
    if !request.tools.is_empty() && !capabilities.supports_tool_calls {
        return Err(RunError::Capability(
            "tool calls are not supported".to_string(),
        ));
    }
    let output_supported = matches!(
        (&request.output_format, provider.structured_output_support()),
        (ModelOutputFormat::Text, _)
            | (
                ModelOutputFormat::Json,
                StructuredOutputSupport::Json | StructuredOutputSupport::JsonSchema
            )
            | (
                ModelOutputFormat::JsonSchema { .. },
                StructuredOutputSupport::JsonSchema
            )
    );
    if !output_supported {
        return Err(RunError::Capability(
            "requested structured output is not supported".to_string(),
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
    let provider_prompt_token_bound = if provider_enforced_limit_opt_out {
        None
    } else {
        match mode {
            AllOrError::Exact => provider
                .exact_context_token_count(request)
                .map_err(RunError::ContextLimit)?,
            AllOrError::Conservative => provider
                .conservative_context_token_bound(request)
                .map_err(RunError::ContextLimit)?,
        }
    };
    estimator
        .can_admit(
            mode,
            request,
            &capabilities,
            provider_prompt_token_bound,
            provider_enforced_limit_opt_out,
        )
        .map_err(RunError::ContextLimit)?;
    Ok(provider_prompt_token_bound)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::validate_history;
    use crate::{Message, MessageContent, MessageRole, ToolCall, ToolName};
    use serde_json::json;

    #[test]
    fn role_dependent_message_shapes_are_rejected() {
        let name = ToolName::parse("lookup").expect("valid fixture name");
        let invalid = [
            Message {
                role: MessageRole::User,
                content: vec![MessageContent::Text { text: "hi".into() }],
                tool_calls: None,
                tool_call_id: Some("impossible".into()),
                name: None,
            },
            Message {
                role: MessageRole::Assistant,
                content: vec![MessageContent::Text {
                    text: "mixed".into(),
                }],
                tool_calls: Some(vec![ToolCall {
                    id: "call".into(),
                    name: name.clone(),
                    arguments: json!({}),
                }]),
                tool_call_id: None,
                name: None,
            },
            Message {
                role: MessageRole::Tool,
                content: vec![MessageContent::Text {
                    text: "not JSON".into(),
                }],
                tool_calls: None,
                tool_call_id: Some("call".into()),
                name: Some(name),
            },
        ];

        for history in invalid.map(|message| vec![message]) {
            assert!(validate_history(&history).is_err());
        }
    }

    #[test]
    fn history_requires_tool_results_before_later_user_or_assistant_messages() {
        let call = ToolCall {
            id: "call".into(),
            name: "lookup".parse().expect("valid fixture name"),
            arguments: json!({}),
        };
        for crossed_message in [
            Message::user("later user"),
            Message::assistant_text("later assistant"),
        ] {
            let history = vec![
                Message::assistant_tool_calls(vec![call.clone()]),
                crossed_message,
            ];
            assert!(validate_history(&history).is_err());
        }
    }

    #[test]
    fn history_requires_tool_results_in_provider_call_order() {
        let lookup: ToolName = "lookup".parse().expect("valid fixture name");
        let store: ToolName = "store".parse().expect("valid fixture name");
        let history = vec![
            Message::assistant_tool_calls(vec![
                ToolCall {
                    id: "lookup-call".into(),
                    name: lookup,
                    arguments: json!({}),
                },
                ToolCall {
                    id: "store-call".into(),
                    name: store.clone(),
                    arguments: json!({}),
                },
            ]),
            Message::tool_result("store-call", store, json!({ "ok": true })),
        ];

        assert!(validate_history(&history).is_err());
    }
}
