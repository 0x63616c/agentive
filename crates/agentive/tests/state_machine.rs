#![allow(missing_docs, clippy::expect_used, clippy::unwrap_used)]

use agentive::{
    AgentRunBudget, AgentRunEffect, AgentRunState, ModelFinishReason, ModelResponse, ModelToolCall,
    ToolName,
};
use proptest::prelude::*;
use serde_json::json;

#[test]
fn tool_effect_identity_is_stable_across_replay_and_round_trip() {
    let mut state = AgentRunState::new(
        "run-1",
        vec![],
        AgentRunBudget {
            model_call_limit: 2,
        },
    );
    state
        .commit_provider_response(
            "run-1:provider:0",
            ModelResponse {
                text: None,
                tool_calls: vec![ModelToolCall {
                    call_id: "call-1".into(),
                    name: ToolName::parse("echo").expect("name"),
                    arguments: json!({"value":1}),
                    provider_call_id: Some("provider-1".into()),
                }],
                usage: None,
                finish_reason: ModelFinishReason::ToolCalls,
            },
        )
        .expect("provider transition");
    let first = state.next_effect().expect("tool effect");
    let restored: AgentRunState =
        serde_json::from_str(&serde_json::to_string(&state).expect("serialize"))
            .expect("deserialize");
    assert_eq!(Some(first), restored.next_effect());
}

#[test]
fn provider_effect_carries_the_exact_serialized_request_and_stable_identity() {
    let state = AgentRunState::new(
        "run-plan",
        vec![agentive::Message::user("hello")],
        AgentRunBudget {
            model_call_limit: 1,
        },
    );
    let effect = state.next_effect().expect("provider effect");
    let AgentRunEffect::ProviderCall {
        effect_id,
        provider_call_id,
        request,
        round,
    } = effect
    else {
        panic!("expected provider effect");
    };
    assert_eq!(round, 0);
    assert_eq!(effect_id, provider_call_id);
    assert_eq!(request.invocation_id, effect_id);
    assert_eq!(request.messages, state.history);
    let restored: AgentRunState =
        serde_json::from_str(&serde_json::to_string(&state).expect("serialize"))
            .expect("deserialize");
    assert_eq!(restored.next_effect(), state.next_effect());
}

#[test]
fn parallel_safe_pending_calls_form_one_ordered_batch_effect() {
    let name = ToolName::parse("echo").expect("name");
    let plan = agentive::AgentRunPlan {
        instructions: agentive::CompiledInstructions {
            core: agentive::InstructionFragment::new("test", 1, "test"),
            agent: None,
            run: None,
            features: vec![],
        },
        tools: vec![agentive::ToolRuntimePolicy {
            descriptor: agentive::ProviderToolDescriptor {
                name: name.clone(),
                description: "echo".into(),
                schema: json!({}),
                idempotent: true,
            },
            idempotent: true,
            max_attempts: 2,
            parallel_safe: true,
            delegation: false,
        }],
        model: None,
        max_output_tokens: 8,
        provider_max_attempts: 1,
        max_parallel_tool_calls: 2,
        context_estimate: agentive::AllOrError::Exact,
        provider_enforced_limit_opt_out: false,
        deadline_ms: None,
        delegation_depth: 0,
        max_delegation_depth: 4,
        token_limit: None,
    };
    let mut state = AgentRunState::with_plan(
        "batch",
        vec![],
        AgentRunBudget {
            model_call_limit: 2,
        },
        plan,
    );
    state
        .commit_provider_response(
            "batch:provider:0",
            ModelResponse {
                text: None,
                tool_calls: vec![
                    ModelToolCall {
                        call_id: "one".into(),
                        name: name.clone(),
                        arguments: json!({}),
                        provider_call_id: None,
                    },
                    ModelToolCall {
                        call_id: "two".into(),
                        name,
                        arguments: json!({}),
                        provider_call_id: None,
                    },
                ],
                usage: None,
                finish_reason: ModelFinishReason::ToolCalls,
            },
        )
        .expect("provider");
    let AgentRunEffect::ToolBatch { effect_id, calls } = state.next_effect().expect("batch") else {
        panic!("expected batch")
    };
    assert_eq!(calls.len(), 2);
    state
        .commit_effect(
            &effect_id,
            agentive::AgentEffectOutcome::ToolBatch {
                outputs: vec![
                    agentive::ToolEffectResult {
                        output: json!({"position": 1}),
                        attempts: 1,
                        child_usage: None,
                    },
                    agentive::ToolEffectResult {
                        output: json!({"position": 2}),
                        attempts: 1,
                        child_usage: None,
                    },
                ],
            },
        )
        .expect("batch commit");
    let outputs = state
        .history
        .iter()
        .filter(|message| message.role == agentive::MessageRole::Tool)
        .collect::<Vec<_>>();
    assert_eq!(outputs.len(), 2);
    let agentive::MessageContent::Text { text } = &outputs[0].content[0] else {
        panic!("tool result must be text encoded JSON");
    };
    assert!(text.contains("position"));
}

#[test]
fn transitions_commit_tool_results_before_the_next_provider_effect() {
    let mut state = AgentRunState::new(
        "run-2",
        vec![],
        AgentRunBudget {
            model_call_limit: 2,
        },
    );
    state
        .commit_provider_response(
            "run-2:provider:0",
            ModelResponse {
                text: None,
                tool_calls: vec![ModelToolCall {
                    call_id: "call-2".into(),
                    name: ToolName::parse("echo").expect("name"),
                    arguments: json!({}),
                    provider_call_id: None,
                }],
                usage: None,
                finish_reason: ModelFinishReason::ToolCalls,
            },
        )
        .expect("provider transition");
    assert!(matches!(
        state.next_effect(),
        Some(AgentRunEffect::ToolCall { .. })
    ));
    state
        .commit_tool_result("run-2:tool:call-2", json!({"ok":true}))
        .expect("tool transition");
    assert!(matches!(
        state.next_effect(),
        Some(AgentRunEffect::ProviderCall { round: 1, .. })
    ));
}

#[test]
fn stale_and_duplicate_effect_completions_are_rejected() {
    let mut state = AgentRunState::new(
        "run-3",
        vec![],
        AgentRunBudget {
            model_call_limit: 1,
        },
    );
    let response = ModelResponse {
        text: Some("done".into()),
        tool_calls: vec![],
        usage: None,
        finish_reason: ModelFinishReason::Stop,
    };
    let error = state
        .commit_provider_response("wrong", response.clone())
        .expect_err("wrong effect");
    assert!(matches!(
        error,
        agentive::StateTransitionError::UnexpectedEffect { .. }
    ));
    state
        .commit_provider_response("run-3:provider:0", response)
        .expect("correct effect");
    let error = state
        .commit_provider_response(
            "run-3:provider:0",
            ModelResponse {
                text: Some("again".into()),
                tool_calls: vec![],
                usage: None,
                finish_reason: ModelFinishReason::Stop,
            },
        )
        .expect_err("duplicate completion");
    assert!(matches!(
        error,
        agentive::StateTransitionError::UnexpectedEffect { .. }
    ));
}

#[test]
fn exhausted_budget_is_a_deterministic_incomplete_terminal_state() {
    let mut state = AgentRunState::new(
        "run-4",
        vec![],
        AgentRunBudget {
            model_call_limit: 0,
        },
    );
    assert!(state.finish_if_exhausted());
    assert_eq!(state.status, agentive::RunStatus::Incomplete);
    assert!(state.next_effect().is_none());
    assert!(!state.usage.complete);
}

#[test]
fn unified_effect_commit_and_terminal_failure_are_serializable() {
    let mut state = AgentRunState::new(
        "run-5",
        vec![],
        AgentRunBudget {
            model_call_limit: 1,
        },
    );
    state
        .commit_effect(
            "run-5:provider:0",
            agentive::AgentEffectOutcome::Provider {
                response: ModelResponse {
                    text: Some("done".into()),
                    tool_calls: vec![],
                    usage: None,
                    finish_reason: ModelFinishReason::Stop,
                },
            },
        )
        .expect("effect commit");
    assert_eq!(state.text.as_deref(), Some("done"));
    state.fail("safe failure");
    let restored: AgentRunState =
        serde_json::from_str(&serde_json::to_string(&state).expect("serialize"))
            .expect("deserialize");
    assert_eq!(restored.error.as_deref(), Some("safe failure"));
    assert_eq!(restored.status, agentive::RunStatus::Failed);
}

proptest! {
    #[test]
    fn state_json_round_trips(run in "[a-z0-9]{1,16}", limit in 0u32..8) {
        let state = AgentRunState::new(run, vec![], AgentRunBudget { model_call_limit: limit });
        let decoded: AgentRunState = serde_json::from_str(&serde_json::to_string(&state).expect("serialize")).expect("deserialize");
        prop_assert_eq!(state, decoded);
    }
}

#[tokio::test]
async fn scripted_local_text_turn_matches_direct_durable_transition() {
    use agentive::{Agent, RunOptions};
    use agentive_test::ScriptedProvider;

    let response = ModelResponse {
        text: Some("done".into()),
        tool_calls: vec![],
        usage: None,
        finish_reason: ModelFinishReason::Stop,
    };
    let local = Agent::builder()
        .provider(ScriptedProvider::new().respond_with(response.clone()))
        .build()
        .expect("agent")
        .run("start")
        .await
        .expect("local result");
    let mut durable = AgentRunState::new(
        "durable-run",
        vec![agentive::Message::user("start")],
        AgentRunBudget {
            model_call_limit: 6,
        },
    );
    durable
        .commit_provider_response("durable-run:provider:0", response)
        .expect("durable transition");

    assert_eq!(local.history, durable.history);
    assert_eq!(local.usage, durable.usage);
    assert_eq!(local.text, durable.text);
    let _ = RunOptions::default();
}
