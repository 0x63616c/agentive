//! Turn collection, input translation, and dynamic tool callbacks.
use crate::{
    CodexCancellation, CodexSession, classify_server_error, classify_turn_failure, protocol_error,
    transport_error,
};
use agentive::{
    IdempotencyKey, MessageContent, MessageRole, ModelFinishReason, ModelRequest, ModelResponse,
    ModelTokenUsage, ProviderError, ProviderErrorKind, Tool, ToolContext, ToolInvocation,
    ToolInvocationId,
};
use serde_json::{Value, json};
use std::sync::Arc;
pub(crate) async fn collect_turn(
    session: &mut dyn CodexSession,
    thread_id: &str,
    turn_id: &str,
    cancellation: &CodexCancellation,
    tools: &[Arc<dyn Tool>],
) -> Result<ModelResponse, ProviderError> {
    let mut text = String::new();
    let mut usage = None;
    let mut interrupted = false;
    loop {
        if cancellation.is_cancelled() && !interrupted {
            session
                .send(json!({
                    "id": 4,
                    "method": "turn/interrupt",
                    "params": {"threadId": thread_id, "turnId": turn_id},
                }))
                .await?;
            interrupted = true;
        }
        let message = tokio::select! {
            message = session.receive() => message?,
            _ = cancellation.token.cancelled(), if !interrupted => continue,
        }
        .ok_or_else(transport_error)?;
        if message.get("id").and_then(Value::as_u64) == Some(4) && interrupted {
            if message.get("error").is_some() {
                return Err(classify_server_error(&message));
            }
            continue;
        }
        if message.get("id").is_some() {
            if message.get("method").and_then(Value::as_str) == Some("item/tool/call") {
                respond_to_tool_request(session, &message, thread_id, turn_id, tools, cancellation)
                    .await?;
                continue;
            }
            return Err(protocol_error());
        }
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            continue;
        };
        let params = message.get("params").unwrap_or(&Value::Null);
        if string_at(params, "/threadId")
            .as_deref()
            .is_some_and(|id| id != thread_id)
        {
            continue;
        }
        if string_at(params, "/turnId")
            .as_deref()
            .is_some_and(|id| id != turn_id)
        {
            continue;
        }
        match method {
            "item/agentMessage/delta" if !interrupted => {
                if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                    text.push_str(delta);
                }
            }
            "thread/tokenUsage/updated" => usage = usage_from(params),
            "turn/completed" => {
                let status = string_at(params, "/turn/status").ok_or_else(protocol_error)?;
                return match status.as_str() {
                    "completed" if !interrupted => Ok(ModelResponse {
                        text: (!text.is_empty()).then_some(text),
                        tool_calls: Vec::new(),
                        usage,
                        finish_reason: ModelFinishReason::Stop,
                    }),
                    "interrupted" => Err(ProviderError::terminal(
                        ProviderErrorKind::Timeout,
                        "codex turn was interrupted",
                    )),
                    "failed" => Err(classify_turn_failure(params)),
                    _ => Err(protocol_error()),
                };
            }
            _ => {}
        }
    }
}

async fn respond_to_tool_request(
    session: &mut dyn CodexSession,
    request: &Value,
    expected_thread_id: &str,
    expected_turn_id: &str,
    tools: &[Arc<dyn Tool>],
    cancellation: &CodexCancellation,
) -> Result<(), ProviderError> {
    let id = request.get("id").cloned().ok_or_else(protocol_error)?;
    let params = request.get("params").ok_or_else(protocol_error)?;
    let tool_name = params
        .get("tool")
        .and_then(Value::as_str)
        .ok_or_else(protocol_error)?;
    let thread_id = params
        .get("threadId")
        .and_then(Value::as_str)
        .ok_or_else(protocol_error)?;
    let turn_id = params
        .get("turnId")
        .and_then(Value::as_str)
        .ok_or_else(protocol_error)?;
    let call_id = params
        .get("callId")
        .and_then(Value::as_str)
        .ok_or_else(protocol_error)?;
    if thread_id != expected_thread_id || turn_id != expected_turn_id || call_id.is_empty() {
        return Err(protocol_error());
    }
    let result = if let Some(tool) = tools.iter().find(|tool| tool.name().as_str() == tool_name) {
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
        let context = ToolContext::for_runtime(
            ToolInvocation {
                run_id: thread_id.to_owned(),
                invocation_id: ToolInvocationId::new(),
                provider_call_id: Some(call_id.to_owned()),
                tool_name: tool.name().clone(),
                model_round: 0,
                idempotency_key: IdempotencyKey::new(format!("{thread_id}:{turn_id}:{call_id}")),
            },
            1,
            None,
            cancellation.token.clone(),
        );
        match tool.call(&context, arguments).await {
            Ok(output) => json!({
                "contentItems": [{"type": "inputText", "text": output.to_string()}],
                "success": true,
            }),
            Err(error) => json!({
                "contentItems": [{"type": "inputText", "text": json!({"error": {"code": error.code, "message": error.message}}).to_string()}],
                "success": false,
            }),
        }
    } else {
        json!({
            "contentItems": [{"type": "inputText", "text": json!({"error": {"code": "tool_not_found", "message": "tool not found"}}).to_string()}],
            "success": false,
        })
    };
    session.send(json!({"id": id, "result": result})).await
}

pub(crate) fn compile_input(request: &ModelRequest) -> Result<Vec<Value>, ProviderError> {
    let mut input = Vec::with_capacity(request.messages.len());
    for message in &request.messages {
        if message.role != MessageRole::User
            || message.tool_calls.is_some()
            || message.tool_call_id.is_some()
        {
            return Err(unsupported("non-user history"));
        }
        for content in &message.content {
            match content {
                MessageContent::Text { text } => input.push(json!({"type": "text", "text": text})),
                MessageContent::Image {
                    url: Some(url),
                    bytes: None,
                    ..
                } => {
                    input.push(json!({"type": "image", "url": url}));
                }
                MessageContent::Image { .. } => return Err(unsupported("inline image")),
            }
        }
    }
    Ok(input)
}

pub(crate) fn dynamic_tools(request: &ModelRequest, tools: &[Arc<dyn Tool>]) -> Vec<Value> {
    let mut dynamic_tools: Vec<Value> = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name.as_str(),
                "description": tool.description,
                "inputSchema": tool.schema,
            })
        })
        .collect();
    for tool in tools {
        if !request
            .tools
            .iter()
            .any(|declared| declared.name == *tool.name())
        {
            dynamic_tools.push(json!({
                "type": "function",
                "name": tool.name().as_str(),
                "description": tool.description(),
                "inputSchema": tool.schema_json(),
            }));
        }
    }
    dynamic_tools
}

fn usage_from(params: &Value) -> Option<ModelTokenUsage> {
    let usage = params.get("tokenUsage")?;
    Some(ModelTokenUsage {
        input: number(usage, "inputTokens"),
        output: number(usage, "outputTokens"),
        cached_input: number(usage, "cachedInputTokens"),
        reasoning: number(usage, "reasoningTokens"),
        provider_total: number(usage, "totalTokens"),
    })
}

fn number(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

pub(crate) fn string_at(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn unsupported(capability: &str) -> ProviderError {
    ProviderError::terminal(
        ProviderErrorKind::InvalidRequest,
        format!("codex app server does not support lossless {capability} input"),
    )
}
