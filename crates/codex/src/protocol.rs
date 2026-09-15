//! JSON-RPC request and notification framing.
use crate::{CodexSession, classify_server_error, protocol_error, transport_error};
use agentive::ProviderError;
use serde_json::{Value, json};
pub(crate) async fn request_response(
    session: &mut dyn CodexSession,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, ProviderError> {
    session
        .send(json!({"id": id, "method": method, "params": params}))
        .await?;
    loop {
        let message = session.receive().await?.ok_or_else(transport_error)?;
        if message.get("id").is_some() && message.get("id").and_then(Value::as_u64) != Some(id) {
            return Err(protocol_error());
        }
        if message.get("id").is_none() {
            continue;
        }
        if message.get("error").is_some() {
            return Err(classify_server_error(&message));
        }
        return message.get("result").cloned().ok_or_else(protocol_error);
    }
}
pub(crate) async fn notification(
    session: &mut dyn CodexSession,
    method: &str,
    params: Value,
) -> Result<(), ProviderError> {
    session
        .send(json!({"method": method, "params": params}))
        .await
}
