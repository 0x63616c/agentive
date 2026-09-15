#![allow(missing_docs, clippy::expect_used, clippy::unwrap_used, dead_code)]

use agentive::{
    Agent, Message, ModelFinishReason, ModelResponse, RunEvent, RunOptions, RunStatus, Tool,
    ToolCallFuture, ToolContext, ToolError, ToolName,
};
use agentive_test::ScriptedProvider;
use futures::StreamExt;
use schemars::schema_for;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct RecordingTool {
    name: ToolName,
    calls: Arc<AtomicUsize>,
    retry_once: bool,
}

impl RecordingTool {
    fn new(name: &str, calls: Arc<AtomicUsize>, retry_once: bool) -> Self {
        Self {
            name: name.parse().expect("fixture name"),
            calls,
            retry_once,
        }
    }
}

impl Tool for RecordingTool {
    fn name(&self) -> &ToolName {
        &self.name
    }
    fn description(&self) -> &'static str {
        "records calls"
    }
    fn schema_json(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| serde_json::to_value(schema_for!(Args)).expect("schema"))
    }
    fn idempotent(&self) -> bool {
        self.retry_once
    }
    fn call<'a>(&'a self, context: &'a ToolContext, args: Value) -> ToolCallFuture<'a> {
        let calls = Arc::clone(&self.calls);
        let retry_once = self.retry_once;
        Box::pin(async move {
            let count = calls.fetch_add(1, Ordering::SeqCst);
            if retry_once && count == 0 {
                assert_eq!(context.attempt(), 1);
                return Err(ToolError::retryable("temporary", "retry safely"));
            }
            Ok(json!({"value": args["value"], "attempt": context.attempt()}))
        })
    }
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct Args {
    #[schemars(description = "value to record")]
    value: String,
}

fn response_without_terminal_content() -> ModelResponse {
    ModelResponse {
        text: None,
        tool_calls: vec![],
        usage: None,
        finish_reason: ModelFinishReason::Stop,
    }
}

#[tokio::test]
async fn repeated_tool_turns_commit_correlated_results_in_provider_order() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = ScriptedProvider::new()
        .respond_with_tool_calls(vec![("record", json!({"value":"one"}))])
        .respond_with_tool_calls(vec![("record", json!({"value":"two"}))])
        .respond_with_text("done");
    let agent = Agent::builder()
        .provider(provider.clone())
        .tool(RecordingTool::new("record", Arc::clone(&calls), false))
        .build()
        .expect("agent");

    let result = agent.run("start").await.expect("run");

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(result.records.len(), 3);
    assert_eq!(
        result
            .history
            .iter()
            .filter(|message| message.role == agentive::MessageRole::Tool)
            .count(),
        2
    );
    provider.assert_finished();
}

#[tokio::test]
async fn idempotent_retry_preserves_one_logical_tool_turn() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = ScriptedProvider::new()
        .respond_with_tool_calls(vec![("retry", json!({"value":"one"}))])
        .respond_with_text("done");
    let agent = Agent::builder()
        .provider(provider)
        .tool(RecordingTool::new("retry", Arc::clone(&calls), true))
        .build()
        .expect("agent");

    let result = agent.run("start").await.expect("run");

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        result
            .history
            .iter()
            .filter(|message| message.role == agentive::MessageRole::Tool)
            .count(),
        1
    );
}

#[tokio::test]
async fn hard_model_call_limit_is_incomplete_and_preserves_attempt_records() {
    let provider = ScriptedProvider::new().respond_with(response_without_terminal_content());
    let agent = Agent::builder().provider(provider).build().expect("agent");
    let options = RunOptions {
        model_call_limit: 1,
        ..RunOptions::default()
    };

    let result = agent.run_with_options("start", options).await.expect("run");

    assert_eq!(result.status, RunStatus::Incomplete);
    assert_eq!(result.records.len(), 1);
    assert!(!result.usage.complete);
}

#[tokio::test]
async fn invalid_caller_history_is_rejected_before_provider_transport() {
    let provider = ScriptedProvider::new().respond_with_text("never called");
    let agent = Agent::builder()
        .provider(provider.clone())
        .build()
        .expect("agent");
    let options = RunOptions {
        history: vec![Message::system("injected")],
        ..RunOptions::default()
    };

    let error = agent
        .run_with_options("start", options)
        .await
        .expect_err("invalid history");

    assert!(matches!(error, agentive::RunError::ProviderProtocol(_)));
    provider.assert_request_count(0);
}

#[tokio::test]
async fn events_are_ordered_and_terminal_status_is_unique_for_multiple_observers() {
    let provider = ScriptedProvider::new().respond_with_text("done");
    let agent = Agent::builder().provider(provider).build().expect("agent");
    let handle = agent.start("start", RunOptions::default());
    let first = handle.events();
    let second = handle.events();
    futures::pin_mut!(first);
    futures::pin_mut!(second);
    let result = handle.wait().await.expect("run");

    let mut first_events = Vec::new();
    let mut second_events = Vec::new();
    for _ in 0..4 {
        first_events.push(first.next().await.expect("event"));
    }
    for _ in 0..4 {
        second_events.push(second.next().await.expect("event"));
    }
    for events in [&first_events, &second_events] {
        assert!(matches!(
            events.first(),
            Some(RunEvent::StatusChanged {
                status: RunStatus::Running
            })
        ));
        assert!(matches!(
            events.get(1),
            Some(RunEvent::ModelCallStarted { round: 0 })
        ));
        assert!(matches!(
            events.get(2),
            Some(RunEvent::ModelCallCompleted { round: 0, .. })
        ));
        assert!(matches!(
            events.last(),
            Some(RunEvent::StatusChanged {
                status: RunStatus::Completed
            })
        ));
    }
    assert_eq!(result.status, RunStatus::Completed);
}

#[test]
fn compiled_instruction_layers_are_preserved_and_xml_escaped() {
    let provider = ScriptedProvider::new();
    let agent = Agent::builder()
        .provider(provider)
        .agent_instructions("agent <trusted>")
        .tool(RecordingTool::new(
            "record",
            Arc::new(AtomicUsize::new(0)),
            false,
        ))
        .build()
        .expect("agent");
    let compiled = agent.compile_request(
        "user <untrusted>",
        &RunOptions {
            run_instructions: Some("run & scope".into()),
            ..RunOptions::default()
        },
    );
    assert_eq!(compiled.messages, vec![Message::user("user <untrusted>")]);
    assert_eq!(compiled.instructions.features.len(), 1);
    assert_eq!(
        compiled.instructions.agent.as_deref(),
        Some("agent <trusted>")
    );
    assert!(
        compiled
            .instructions
            .render_xml()
            .contains("agent &lt;trusted&gt;")
    );
    assert!(
        compiled
            .instructions
            .render_xml()
            .contains("run &amp; scope")
    );
}
