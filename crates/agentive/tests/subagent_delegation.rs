//! Public delegation contracts through the scripted provider boundary.

#![allow(clippy::expect_used)]

use agentive::{
    Agent, DelegationTool, ModelFinishReason, ModelResponse, ProviderError, ProviderErrorKind,
    RunEvent, RunOptions, RunStatus, ToolName,
};
use agentive_test::ScriptedProvider;
use futures::StreamExt;
use serde_json::json;
use std::time::Duration;

fn tool_call(name: &str, arguments: serde_json::Value) -> ModelResponse {
    ModelResponse {
        text: None,
        tool_calls: vec![agentive::ModelToolCall {
            call_id: "delegate-1".to_string(),
            name: name.parse::<ToolName>().expect("valid fixture tool name"),
            arguments,
            provider_call_id: None,
        }],
        usage: Some(agentive::ModelTokenUsage {
            input: Some(3),
            output: Some(2),
            cached_input: None,
            reasoning: None,
            provider_total: Some(5),
        }),
        finish_reason: ModelFinishReason::ToolCalls,
    }
}

fn text_response(text: &str) -> ModelResponse {
    ModelResponse {
        text: Some(text.to_string()),
        tool_calls: Vec::new(),
        usage: Some(agentive::ModelTokenUsage {
            input: Some(3),
            output: Some(2),
            cached_input: None,
            reasoning: None,
            provider_total: Some(5),
        }),
        finish_reason: ModelFinishReason::Stop,
    }
}

fn parallel_delegations() -> ModelResponse {
    ModelResponse {
        text: None,
        tool_calls: ["one", "two"]
            .into_iter()
            .map(|call_id| agentive::ModelToolCall {
                call_id: call_id.to_string(),
                name: "delegate".parse().expect("valid fixture tool name"),
                arguments: json!({"task": call_id}),
                provider_call_id: None,
            })
            .collect(),
        usage: None,
        finish_reason: ModelFinishReason::ToolCalls,
    }
}

#[tokio::test]
async fn delegation_is_an_explicit_tool_and_keeps_child_history_private() {
    let child_provider = ScriptedProvider::new().respond_with(text_response("child answer"));
    let child = Agent::builder()
        .name("budget-child")
        .name("depth-child")
        .name("researcher")
        .provider(child_provider.clone())
        .build()
        .expect("child agent");
    let parent_provider = ScriptedProvider::new()
        .respond_with(tool_call("research", json!({"task":"find the answer"})))
        .respond_with(text_response("parent answer"));
    let parent = Agent::builder()
        .name("budget-parent")
        .name("depth-parent")
        .name("orchestrator")
        .provider(parent_provider.clone())
        .delegate(
            "research",
            "Ask the research agent a focused question.",
            child,
        )
        .expect("delegation tool")
        .build()
        .expect("parent agent");

    let result = parent.run("start").await.expect("parent run");

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text.as_deref(), Some("parent answer"));
    let tool_results: Vec<_> = result
        .history
        .iter()
        .filter(|message| message.role == agentive::MessageRole::Tool)
        .collect();
    assert_eq!(tool_results.len(), 1);
    let result_json: serde_json::Value = serde_json::from_str(match &tool_results[0].content[0] {
        agentive::MessageContent::Text { text } => text,
        other => panic!("unexpected tool content: {other:?}"),
    })
    .expect("delegation result JSON");
    assert_eq!(result_json["text"], "child answer");
    assert_eq!(result_json["agent"], "researcher");
    assert!(result_json["run_id"].as_str().is_some());
    assert!(result_json.get("history").is_none());
    assert_eq!(child_provider.recorded_requests().len(), 1);
    assert_eq!(parent_provider.recorded_requests().len(), 2);
    assert_eq!(result.usage.aggregate.provider_total, Some(15));
    assert_eq!(result.usage.model_calls.len(), 3);
    child_provider.assert_finished();
    parent_provider.assert_finished();
}

#[tokio::test]
async fn parent_cancellation_cancels_an_active_child_run() {
    let child = Agent::builder()
        .name("slow-child")
        .provider(
            ScriptedProvider::new()
                .delay(Duration::from_secs(10))
                .respond_with_text("too late"),
        )
        .build()
        .expect("child agent");
    let parent = Agent::builder()
        .provider(
            ScriptedProvider::new().respond_with(tool_call("delegate", json!({"task":"wait"}))),
        )
        .delegate("delegate", "Delegate one bounded task to the child.", child)
        .expect("delegation declaration")
        .build()
        .expect("parent agent");

    let handle = parent.start("start", agentive::RunOptions::default());
    tokio::time::sleep(Duration::from_millis(20)).await;
    handle.cancel();
    let result = tokio::time::timeout(Duration::from_secs(1), handle.wait())
        .await
        .expect("parent cancellation must complete")
        .expect("cancelled run is an outcome");

    assert_eq!(result.status, RunStatus::Cancelled);
}

#[tokio::test]
async fn delegation_depth_limit_returns_a_safe_tool_result_without_starting_child() {
    let child_provider = ScriptedProvider::new().respond_with_text("must not run");
    let child = Agent::builder()
        .name("depth-child")
        .provider(child_provider.clone())
        .build()
        .expect("child");
    let parent = Agent::builder()
        .name("depth-parent")
        .provider(
            ScriptedProvider::new()
                .respond_with(tool_call("delegate", json!({"task":"nested"})))
                .respond_with_text("repaired"),
        )
        .delegate("delegate", "Delegate exactly one task.", child)
        .expect("delegate")
        .build()
        .expect("parent");
    let result = parent
        .run_with_options(
            "start",
            agentive::RunOptions {
                max_delegation_depth: 0,
                ..Default::default()
            },
        )
        .await
        .expect("run");
    assert_eq!(result.text.as_deref(), Some("repaired"));
    let tool_result = result
        .history
        .iter()
        .find(|message| message.role == agentive::MessageRole::Tool)
        .expect("tool repair");
    assert!(format!("{tool_result:?}").contains("delegation_depth_exceeded"));
    assert!(child_provider.recorded_requests().is_empty());
}

#[test]
fn delegation_registry_rejects_name_collisions_and_static_name_cycles() {
    let child = Agent::builder()
        .name("root")
        .provider(ScriptedProvider::new())
        .build()
        .expect("child");
    let cycle = Agent::builder()
        .name("root")
        .provider(ScriptedProvider::new())
        .delegate("child", "Child.", child)
        .expect("delegate")
        .build();
    assert!(matches!(cycle, Err(agentive::RunError::Config(message)) if message.contains("cycle")));
    let child = Agent::builder()
        .provider(ScriptedProvider::new())
        .build()
        .expect("child");
    let collision = Agent::builder()
        .provider(ScriptedProvider::new())
        .delegate("same", "Child.", child.clone())
        .expect("first")
        .delegate("same", "Child.", child)
        .expect("second")
        .build();
    assert!(
        matches!(collision, Err(agentive::RunError::DuplicateToolName(name)) if name == "same")
    );
}

#[tokio::test]
async fn child_failure_is_a_safe_correlated_tool_result() {
    let child = Agent::builder()
        .name("failing-child")
        .provider(
            ScriptedProvider::new().respond_with_error(ProviderError::terminal(
                ProviderErrorKind::Authentication,
                "private child credential diagnostic",
            )),
        )
        .build()
        .expect("child agent");
    let parent_provider = ScriptedProvider::new()
        .respond_with(tool_call("delegate", json!({"task":"attempt"})))
        .respond_with_text("recovered");
    let parent = Agent::builder()
        .provider(parent_provider.clone())
        .tool(
            DelegationTool::new("delegate", "Delegate a focused task.", child)
                .expect("delegation tool"),
        )
        .build()
        .expect("parent agent");

    let result = parent.run("start").await.expect("parent run");

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text.as_deref(), Some("recovered"));
    let follow_up = parent_provider
        .recorded_requests()
        .into_iter()
        .nth(1)
        .expect("follow-up");
    let tool_text = match &follow_up.messages.last().expect("tool result").content[0] {
        agentive::MessageContent::Text { text } => text,
        other => panic!("unexpected tool content: {other:?}"),
    };
    assert!(tool_text.contains("delegation_failed"));
    assert!(!tool_text.contains("private child credential diagnostic"));
}

#[tokio::test]
async fn delegation_reserves_the_parent_tree_budget_and_forwards_attributed_events() {
    let child_provider = ScriptedProvider::new()
        .respond_with(text_response("child first"))
        .respond_with(text_response("must not start"));
    let child = Agent::builder()
        .name("budgeted-child")
        .provider(child_provider.clone())
        .build()
        .expect("child");
    let parent = Agent::builder()
        .provider(
            ScriptedProvider::new()
                .respond_with(tool_call("delegate", json!({"task":"one"})))
                .respond_with_text("parent done"),
        )
        .delegate("delegate", "Delegate one bounded task.", child)
        .expect("delegate")
        .build()
        .expect("parent");

    let handle = parent.start(
        "start",
        RunOptions {
            model_call_limit: 3,
            token_limit: Some(1_000),
            ..RunOptions::default()
        },
    );
    let events = handle.events();
    futures::pin_mut!(events);
    let waiter = handle.clone();
    let wait = tokio::spawn(async move { waiter.wait().await });
    let mut received = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(1), events.next())
            .await
            .expect("event stream must advance")
            .expect("event stream remains attached");
        let terminal = matches!(
            event,
            RunEvent::StatusChanged {
                status: RunStatus::Completed
                    | RunStatus::Incomplete
                    | RunStatus::Failed
                    | RunStatus::Cancelled
            }
        );
        received.push(event);
        if terminal {
            break;
        }
    }
    let result = wait.await.expect("wait task").expect("parent result");

    assert_eq!(result.status, RunStatus::Completed);
    child_provider.assert_request_count(1);
    assert!(received.iter().any(|event| matches!(
        event,
        RunEvent::Child { run_id, event }
            if !run_id.is_empty() && matches!(event.as_ref(), RunEvent::ModelCallStarted { round: 0 })
    )));
}

#[tokio::test]
async fn exhausted_tree_reservation_does_not_start_a_child() {
    let child_provider = ScriptedProvider::new().respond_with_text("must not run");
    let child = Agent::builder()
        .name("reservation-child")
        .provider(child_provider.clone())
        .build()
        .expect("child");
    let parent = Agent::builder()
        .name("reservation-parent")
        .provider(
            ScriptedProvider::new()
                .respond_with(tool_call("delegate", json!({"task":"one"})))
                .respond_with_text("repaired"),
        )
        .delegate("delegate", "Delegate one bounded task.", child)
        .expect("delegate")
        .build()
        .expect("parent");

    let result = parent
        .run_with_options(
            "start",
            RunOptions {
                model_call_limit: 2,
                ..RunOptions::default()
            },
        )
        .await
        .expect("parent result");

    assert_eq!(result.status, RunStatus::Completed);
    child_provider.assert_request_count(0);
    assert!(format!("{:?}", result.history).contains("delegation_budget_exceeded"));
}

#[tokio::test]
async fn token_budget_rejects_a_child_before_provider_transport() {
    let child_provider = ScriptedProvider::new().respond_with_text("must not run");
    let child = Agent::builder()
        .name("token-child")
        .provider(child_provider.clone())
        .build()
        .expect("child");
    let parent = Agent::builder()
        .name("token-parent")
        .provider(
            ScriptedProvider::new()
                .respond_with(tool_call("delegate", json!({"task":"one"})))
                .respond_with_text("repaired"),
        )
        .delegate("delegate", "Delegate one bounded task.", child)
        .expect("delegate")
        .build()
        .expect("parent");

    let result = parent
        .run_with_options(
            "start",
            RunOptions {
                output_token_reserve: 1,
                token_limit: Some(5),
                ..RunOptions::default()
            },
        )
        .await
        .expect("parent result");

    assert_eq!(result.status, RunStatus::Incomplete);
    child_provider.assert_request_count(0);
}

#[tokio::test]
async fn parallel_delegations_receive_deterministic_bounded_reservations() {
    let child_provider = ScriptedProvider::new()
        .respond_with_text("first")
        .respond_with_text("second");
    let child = Agent::builder()
        .name("parallel-child")
        .provider(child_provider.clone())
        .build()
        .expect("child");
    let parent = Agent::builder()
        .name("parallel-parent")
        .provider(
            ScriptedProvider::new()
                .respond_with(parallel_delegations())
                .respond_with_text("parent done"),
        )
        .delegate_parallel("delegate", "Delegate independent tasks.", child)
        .expect("delegate")
        .build()
        .expect("parent");

    let result = parent
        .run_with_options(
            "start",
            RunOptions {
                model_call_limit: 4,
                ..RunOptions::default()
            },
        )
        .await
        .expect("parent result");

    assert_eq!(result.status, RunStatus::Completed);
    child_provider.assert_request_count(2);
}
