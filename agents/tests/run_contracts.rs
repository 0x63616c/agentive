#[tokio::test]
async fn scripted_text_response_completes() {
    use agents::{Agent, RunStatus};
    use agents_test::ScriptedProvider;

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

impl agents::Tool for EchoTool {
    fn name(&self) -> &agents::ToolName {
        static NAME: std::sync::OnceLock<agents::ToolName> = std::sync::OnceLock::new();
        NAME.get_or_init(|| agents::ToolName::parse("echo").expect("tool name must be valid"))
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
        _context: &agents::ToolContext,
        args: serde_json::Value,
    ) -> agents::ToolCallFuture<'_> {
        Box::pin(async move {
            let args: EchoArgs = agents::decode_tool_call_args(args)
                .map_err(|err| agents::ToolError::terminal("invalid_arguments", err.to_string()))?;
            Ok(serde_json::json!({ "output": args.text }))
        })
    }
}

#[tokio::test]
async fn scripted_tool_call_halts_and_completes() {
    use agents::{Agent, RunStatus};
    use agents_test::ScriptedProvider;

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
    assert_eq!(result.history[2].role, agents::MessageRole::Tool);
    assert_eq!(
        result.history[2].tool_call_id,
        Some(result.history[1].tool_calls.as_ref().unwrap()[0].id.clone())
    );
}

#[tokio::test]
async fn unknown_tool_returns_safe_error_to_model() {
    use agents::{Agent, MessageContent, RunStatus};
    use agents_test::ScriptedProvider;

    let provider = ScriptedProvider::new()
        .respond_with_tool_calls(vec![("missing", serde_json::json!({"x": 1}))])
        .respond_with_text("finished");

    let agent = Agent::builder().provider(provider).build().unwrap();

    let result = agent.run("please fail").await.unwrap();

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text, Some("finished".to_string()));
    assert_eq!(result.history.len(), 4);
    let tool_message = &result.history[2];
    assert_eq!(tool_message.role, agents::MessageRole::Tool);
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
    use agents::{Agent, RunOptions, RunStatus};
    use agents_test::ScriptedProvider;
    use std::time::Duration;
    use tokio::time::sleep;

    let provider = ScriptedProvider::new()
        .delay(Duration::from_millis(100))
        .respond_with_text("late");
    let agent = Agent::builder().provider(provider).build().unwrap();

    let handle = agent.start("slow", RunOptions::default());
    sleep(Duration::from_millis(20)).await;
    handle.cancel();

    let result = handle.wait().await.unwrap();
    assert_eq!(result.status, RunStatus::Cancelled);
}
