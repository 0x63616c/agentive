#![allow(missing_docs, clippy::expect_used, clippy::unwrap_used)]

#[tokio::test]
async fn scripted_text_response_completes() {
    use agentive::{Agent, RunStatus};
    use agentive_test::ScriptedProvider;

    let provider = ScriptedProvider::new().respond_with_text("Done");
    let agent = Agent::builder().provider(provider).build().unwrap();

    let result = agent.run("run this").await.unwrap();

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text, Some("Done".to_string()));
    assert_eq!(result.records.len(), 1);
    assert!(result.records[0].response.text.is_some());
    assert_eq!(result.history.len(), 2);
}

#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
struct EchoArgs {
    text: String,
}

struct EchoTool;

impl agentive::Tool for EchoTool {
    fn name(&self) -> &agentive::ToolName {
        static NAME: std::sync::OnceLock<agentive::ToolName> = std::sync::OnceLock::new();
        NAME.get_or_init(|| agentive::ToolName::parse("echo").expect("tool name must be valid"))
    }

    fn description(&self) -> &'static str {
        "Echo text"
    }

    fn schema_json(&self) -> &serde_json::Value {
        use schemars::schema_for;
        static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| serde_json::to_value(schema_for!(EchoArgs)).unwrap())
    }

    fn idempotent(&self) -> bool {
        false
    }

    fn call(
        &self,
        _context: &agentive::ToolContext,
        args: serde_json::Value,
    ) -> agentive::ToolCallFuture<'_> {
        Box::pin(async move {
            let args: EchoArgs = agentive::decode_tool_call_args(args).map_err(|err| {
                agentive::ToolError::terminal("invalid_arguments", err.to_string())
            })?;
            Ok(serde_json::json!({ "output": args.text }))
        })
    }
}

#[tokio::test]
async fn scripted_tool_call_halts_and_completes() {
    use agentive::{Agent, RunStatus};
    use agentive_test::ScriptedProvider;

    let provider = ScriptedProvider::new()
        .respond_with_tool_calls(vec![("echo", serde_json::json!({"text": "payload"}))])
        .respond_with_text("echoed");

    let agent = Agent::builder()
        .provider(provider)
        .tool(EchoTool)
        .build()
        .unwrap();

    let result = agent.run("please echo").await.unwrap();

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text, Some("echoed".to_string()));
    assert_eq!(result.history.len(), 4);
    assert_eq!(result.history[1].tool_calls.as_ref().unwrap().len(), 1);
    assert_eq!(result.history[2].role, agentive::MessageRole::Tool);
    assert_eq!(
        result.history[2].tool_call_id,
        Some(result.history[1].tool_calls.as_ref().unwrap()[0].id.clone())
    );
}

#[tokio::test]
async fn unknown_tool_returns_safe_error_to_model() {
    use agentive::{Agent, MessageContent, RunStatus};
    use agentive_test::ScriptedProvider;

    let provider = ScriptedProvider::new()
        .respond_with_tool_calls(vec![("missing", serde_json::json!({"x": 1}))])
        .respond_with_text("finished");

    let agent = Agent::builder().provider(provider).build().unwrap();

    let result = agent.run("please fail").await.unwrap();

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text, Some("finished".to_string()));
    assert_eq!(result.history.len(), 4);
    let tool_message = &result.history[2];
    assert_eq!(tool_message.role, agentive::MessageRole::Tool);
    let MessageContent::Text { text } = &tool_message.content[0] else {
        panic!("expected text tool result")
    };
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({"error": {"code": "tool_not_found", "message": "tool not found"}})
    );
}

#[tokio::test]
async fn start_and_cancel_marks_cancelled() {
    use agentive::{Agent, RunOptions, RunStatus};
    use agentive_test::ScriptedProvider;
    use std::time::Duration;
    use tokio::time::sleep;

    let provider = ScriptedProvider::new()
        .delay(Duration::from_millis(100))
        .respond_with_text("late");
    let agent = Agent::builder().provider(provider).build().unwrap();

    let handle = agent.start("slow", RunOptions::default());
    sleep(Duration::from_millis(20)).await;
    handle.cancel();

    let result = tokio::time::timeout(Duration::from_millis(50), handle.wait())
        .await
        .expect("cancellation must interrupt the provider attempt")
        .unwrap();
    assert_eq!(result.status, RunStatus::Cancelled);
    assert!(result.text.is_none());
    assert_eq!(result.history, vec![agentive::Message::user("slow")]);
}

#[tokio::test]
async fn unsupported_images_are_rejected_before_provider_transport() {
    use agentive::{Agent, Message, RunError, RunOptions};
    use agentive_test::ScriptedProvider;

    let provider = ScriptedProvider::new().respond_with_text("must not run");
    let agent = Agent::builder().provider(provider.clone()).build().unwrap();
    let error = agent
        .run_with_options(
            "describe",
            RunOptions {
                history: vec![Message::image_url(
                    "https://example.test/image.png",
                    "image/png",
                )],
                ..Default::default()
            },
        )
        .await
        .expect_err("unsupported image");
    assert!(matches!(error, RunError::Capability(message) if message.contains("images")));
    assert!(provider.recorded_requests().is_empty());
}
