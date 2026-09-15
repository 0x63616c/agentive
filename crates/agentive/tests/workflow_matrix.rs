#![allow(
    missing_docs,
    clippy::expect_used,
    clippy::manual_async_fn,
    clippy::unwrap_used,
    dead_code
)]

use agentive::{
    Agent, AgentRunBudget, AgentRunEffect, AgentRunState, Message, ModelCapabilities,
    ModelFinishReason, ModelResponse, ModelToolCall, ProviderToolDescriptor, RunEvent, RunOptions,
    RunStatus, Tool, ToolContext, ToolError, ToolName, ToolRuntimePolicy,
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

struct LargeContractTool {
    name: ToolName,
    description: &'static str,
    schema: Value,
}

impl LargeContractTool {
    fn new() -> Self {
        Self {
            name: "large_contract".parse().expect("fixture name"),
            description: Box::leak("description ".repeat(2_048).into_boxed_str()),
            schema: json!({
                "type": "object",
                "additionalProperties": false,
                "description": "schema ".repeat(2_048),
                "properties": {
                    "payload": { "type": "string", "description": "field ".repeat(2_048) }
                }
            }),
        }
    }
}

impl Tool for LargeContractTool {
    fn name(&self) -> &ToolName {
        &self.name
    }

    fn description(&self) -> &'static str {
        self.description
    }

    fn schema_json(&self) -> &Value {
        &self.schema
    }

    fn call<'a>(
        &'a self,
        _: &'a ToolContext,
        _: Value,
    ) -> impl std::future::Future<Output = Result<Value, ToolError>> + Send + 'a {
        async { Ok(json!({"unexpected": true})) }
    }
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
    fn call<'a>(
        &'a self,
        context: &'a ToolContext,
        args: Value,
    ) -> impl std::future::Future<Output = Result<Value, ToolError>> + Send + 'a {
        let calls = Arc::clone(&self.calls);
        let retry_once = self.retry_once;
        async move {
            let count = calls.fetch_add(1, Ordering::SeqCst);
            if retry_once && count == 0 {
                assert_eq!(context.attempt(), 1);
                return Err(ToolError::retryable("temporary", "retry safely"));
            }
            Ok(json!({"value": args["value"], "attempt": context.attempt()}))
        }
    }
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct Args {
    #[schemars(description = "value to record")]
    value: String,
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
async fn multiple_sequential_tools_from_one_model_round_all_complete_in_order() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = ScriptedProvider::new()
        .respond_with_tool_calls(vec![
            ("record", json!({"value":"one"})),
            ("record", json!({"value":"two"})),
        ])
        .respond_with_text("done");
    let agent = Agent::builder()
        .provider(provider.clone())
        .tool(RecordingTool::new("record", Arc::clone(&calls), false))
        .build()
        .expect("agent");

    let result = agent.run("start").await.expect("run");
    let values = result
        .history
        .iter()
        .filter(|message| message.role == agentive::MessageRole::Tool)
        .map(|message| match &message.content[0] {
            agentive::MessageContent::Text { text } => text.clone(),
            other => panic!("unexpected tool result: {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(values[0].contains("one"));
    assert!(values[1].contains("two"));
    provider.assert_finished();
}

#[tokio::test]
async fn manual_tool_arguments_are_validated_before_side_effects() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = ScriptedProvider::new()
        .respond_with_tool_calls(vec![("record", json!({"value": 42}))])
        .respond_with_text("repaired");
    let agent = Agent::builder()
        .provider(provider)
        .tool(RecordingTool::new("record", Arc::clone(&calls), false))
        .build()
        .expect("agent");

    let result = agent
        .run("start")
        .await
        .expect("model repairs invalid call");

    assert_eq!(result.text.as_deref(), Some("repaired"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(format!("{:?}", result.history).contains("invalid_arguments"));
}

#[tokio::test]
async fn duplicate_tool_call_ids_fail_before_any_side_effect() {
    let calls = Arc::new(AtomicUsize::new(0));
    let duplicate = ModelResponse {
        text: None,
        tool_calls: ["one", "two"]
            .map(|_| agentive::ModelToolCall {
                call_id: "duplicate".to_string(),
                name: "record".parse().expect("tool name"),
                arguments: json!({"value":"x"}),
                provider_call_id: None,
            })
            .into(),
        usage: None,
        finish_reason: ModelFinishReason::ToolCalls,
    };
    let agent = Agent::builder()
        .provider(ScriptedProvider::new().respond_with(duplicate))
        .tool(RecordingTool::new("record", Arc::clone(&calls), false))
        .build()
        .expect("agent");

    let handle = agent.start("start", Default::default());
    let events = handle.events();
    let error = handle.wait().await.expect_err("duplicate ids fail");

    assert!(matches!(error, agentive::RunError::ProviderProtocol(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(handle.status(), agentive::RunStatus::Failed);
    let failed_events = events
        .filter(|event| {
            futures::future::ready(matches!(
                event,
                RunEvent::StatusChanged {
                    status: RunStatus::Failed
                }
            ))
        })
        .count()
        .await;
    assert_eq!(failed_events, 1);
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
    let calls = Arc::new(AtomicUsize::new(0));
    let provider =
        ScriptedProvider::new().respond_with_tool_calls(vec![("record", json!({"value":"one"}))]);
    let agent = Agent::builder()
        .provider(provider)
        .tool(RecordingTool::new("record", Arc::clone(&calls), false))
        .build()
        .expect("agent");
    let options = RunOptions {
        model_call_limit: 1,
        ..RunOptions::default()
    };

    let result = agent.run_with_options("start", options).await.expect("run");

    assert_eq!(result.status, RunStatus::Incomplete);
    assert_eq!(result.records.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!result.usage.complete);
}

#[tokio::test]
async fn hard_token_limit_rejects_large_canonical_contract_before_provider_transport() {
    let provider = ScriptedProvider::new()
        .capabilities(ModelCapabilities {
            max_context_tokens: Some(1_000_000),
            ..ModelCapabilities::default()
        })
        .respond_with_text("must not run");
    let agent = Agent::builder()
        .provider(provider.clone())
        .agent_instructions("instruction ".repeat(2_048))
        .tool(LargeContractTool::new())
        .build()
        .expect("agent");

    let result = agent
        .run_with_options(
            "start",
            RunOptions {
                output_token_reserve: 1,
                token_limit: Some(1),
                ..RunOptions::default()
            },
        )
        .await
        .expect("budget exhaustion is a result");

    assert_eq!(result.status, RunStatus::Incomplete);
    assert!(result.records.is_empty());
    provider.assert_request_count(0);
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

    let handle = agent.start("start", options);
    let events = handle.events();
    let error = handle.wait().await.expect_err("invalid history");

    assert!(matches!(error, agentive::RunError::ProviderProtocol(_)));
    assert_eq!(handle.status(), RunStatus::Failed);
    let events = events.collect::<Vec<_>>().await;
    assert!(matches!(
        events.last(),
        Some(RunEvent::StatusChanged {
            status: RunStatus::Failed
        })
    ));
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

#[tokio::test]
async fn attached_events_capture_an_immediately_completed_run_and_then_close() {
    let provider = ScriptedProvider::new().respond_with_text("done");
    let agent = Agent::builder().provider(provider).build().expect("agent");
    let handle = agent.start("start", RunOptions::default());

    assert_eq!(
        handle.wait().await.expect("run").status,
        RunStatus::Completed
    );
    let events = handle.events().collect::<Vec<_>>().await;

    assert!(matches!(
        events.first(),
        Some(RunEvent::StatusChanged {
            status: RunStatus::Running
        })
    ));
    assert!(matches!(
        events.last(),
        Some(RunEvent::StatusChanged {
            status: RunStatus::Completed
        })
    ));
}

#[tokio::test]
async fn forged_effect_is_rejected_before_the_provider_is_called() {
    let provider = ScriptedProvider::new().respond_with_text("never called");
    let agent = Agent::builder()
        .provider(provider.clone())
        .build()
        .expect("agent");
    let state = agent
        .prepare_state("run", Vec::new(), &RunOptions::default())
        .expect("worker-bound state");
    let mut forged = state.next_effect().expect("provider effect");
    let AgentRunEffect::ProviderCall { request, .. } = &mut forged else {
        panic!("expected provider effect");
    };
    request.messages.push(Message::user("forged"));

    let error = agent
        .execute_effect(&state, forged, agentive::CancellationToken::new())
        .await
        .expect_err("forged effect must fail");
    assert!(matches!(error, agentive::RunError::ProviderProtocol(_)));
    provider.assert_request_count(0);
}

#[tokio::test]
async fn forged_tool_policy_is_rejected_before_it_reaches_the_provider() {
    let provider = ScriptedProvider::new().respond_with_text("never called");
    let tool = RecordingTool::new("record", Arc::new(AtomicUsize::new(0)), false);
    let definition = tool.metadata();
    let agent = Agent::builder()
        .provider(provider.clone())
        .tool(tool)
        .build()
        .expect("agent");
    let mut state = AgentRunState::new(
        "forged-policy",
        Vec::new(),
        AgentRunBudget {
            model_call_limit: 1,
        },
    );
    state.plan.tools.push(ToolRuntimePolicy {
        descriptor: ProviderToolDescriptor {
            name: definition.name,
            description: definition.description.to_owned(),
            schema: definition.schema.json,
            idempotent: definition.idempotent,
        },
        idempotent: true,
        max_attempts: 9,
        parallel_safe: true,
        delegation: false,
    });
    let effect = state.next_effect().expect("provider effect");

    let error = agent
        .execute_effect(&state, effect, agentive::CancellationToken::new())
        .await
        .expect_err("worker-bound tool policy must win");

    assert!(matches!(error, agentive::RunError::Config(_)));
    provider.assert_request_count(0);
}

#[tokio::test]
async fn restored_state_cannot_replace_worker_owned_instruction_fragments() {
    let provider = ScriptedProvider::new().respond_with_text("never called");
    let agent = Agent::builder()
        .provider(provider.clone())
        .agent_instructions("worker-owned agent instructions")
        .tool(RecordingTool::new(
            "record",
            Arc::new(AtomicUsize::new(0)),
            false,
        ))
        .build()
        .expect("agent");
    let state = agent
        .prepare_state(
            "restored-instructions",
            Vec::new(),
            &RunOptions {
                run_instructions: Some("caller-owned run instructions".into()),
                ..RunOptions::default()
            },
        )
        .expect("prepared state");
    let mut restored: AgentRunState =
        serde_json::from_value(serde_json::to_value(state).expect("serialize durable state"))
            .expect("deserialize durable state");
    restored.plan.instructions.core.text = "forged core instructions".into();
    restored.plan.instructions.agent = Some("forged agent instructions".into());
    restored.plan.instructions.features[0].text = "forged feature instructions".into();
    let effect = restored.next_effect().expect("provider effect");

    let error = agent
        .execute_effect(&restored, effect, agentive::CancellationToken::new())
        .await
        .expect_err("worker-owned instruction fragments must win");

    assert!(matches!(error, agentive::RunError::Config(_)));
    provider.assert_request_count(0);
}

#[tokio::test]
async fn direct_effect_execution_rejects_invalid_snapshot_history_before_provider_call() {
    let provider = ScriptedProvider::new().respond_with_text("never called");
    let agent = Agent::builder()
        .provider(provider.clone())
        .build()
        .expect("agent");
    let state = AgentRunState::new(
        "invalid-history",
        vec![Message::system("not canonical history")],
        AgentRunBudget {
            model_call_limit: 1,
        },
    );
    let effect = state.next_effect().expect("provider effect");

    let error = agent
        .execute_effect(&state, effect, agentive::CancellationToken::new())
        .await
        .expect_err("invalid snapshot history must be rejected");
    assert!(matches!(error, agentive::RunError::ProviderProtocol(_)));
    provider.assert_request_count(0);
}

#[tokio::test]
async fn direct_effect_execution_rejects_invalid_provider_response() {
    let provider = ScriptedProvider::new().respond_with(ModelResponse {
        text: Some("mixed response".into()),
        tool_calls: vec![ModelToolCall {
            call_id: "call".into(),
            name: "tool".parse().expect("fixture tool name"),
            arguments: json!({}),
            provider_call_id: None,
        }],
        usage: None,
        finish_reason: ModelFinishReason::ToolCalls,
    });
    let agent = Agent::builder()
        .provider(provider.clone())
        .build()
        .expect("agent");
    let state = agent
        .prepare_state("invalid-response", Vec::new(), &RunOptions::default())
        .expect("worker-bound state");
    let effect = state.next_effect().expect("provider effect");

    let error = agent
        .execute_effect(&state, effect, agentive::CancellationToken::new())
        .await
        .expect_err("invalid provider response must be rejected");
    assert!(matches!(error, agentive::RunError::ProviderProtocol(_)));
    provider.assert_finished();
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
