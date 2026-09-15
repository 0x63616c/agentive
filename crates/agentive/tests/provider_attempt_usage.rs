#![allow(missing_docs, clippy::expect_used)]

use agentive::{
    Agent, AgentEffectOutcome, AgentRunBudget, AgentRunState, ModelCapabilities, ModelFinishReason,
    ModelProvider, ModelRequest, ModelResponse, ModelTokenUsage, ProviderAttempt,
    ProviderCallContext, ProviderError, ProviderErrorKind, RunOptions,
};
use agentive_test::ScriptedProvider;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
struct ExpandedWireProvider {
    calls: Arc<AtomicUsize>,
}

impl ModelProvider for ExpandedWireProvider {
    fn exact_context_token_count(&self, _request: &ModelRequest) -> Result<Option<u64>, String> {
        Ok(Some(4_096))
    }

    fn generate<'call>(
        &'call self,
        _request: ModelRequest,
        _context: &'call ProviderCallContext,
    ) -> impl std::future::Future<Output = Result<ModelResponse, ProviderError>> + Send + 'call
    {
        self.calls.fetch_add(1, Ordering::SeqCst);
        async {
            Ok(ModelResponse {
                text: Some("must not run".to_string()),
                tool_calls: Vec::new(),
                usage: None,
                finish_reason: ModelFinishReason::Stop,
            })
        }
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            max_context_tokens: Some(8_192),
            ..ModelCapabilities::default()
        }
    }
}

fn usage(input: u64, output: u64) -> ModelTokenUsage {
    ModelTokenUsage {
        input: Some(input),
        output: Some(output),
        cached_input: Some(0),
        reasoning: Some(0),
        provider_total: Some(input.saturating_add(output)),
    }
}

#[tokio::test]
async fn hard_token_budget_uses_the_provider_specific_wire_count_before_transport() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = Agent::builder()
        .provider(ExpandedWireProvider {
            calls: Arc::clone(&calls),
        })
        .build()
        .expect("agent");

    let result = agent
        .run_with_options(
            "start",
            RunOptions {
                token_limit: Some(1_024),
                output_token_reserve: 256,
                ..RunOptions::default()
            },
        )
        .await
        .expect("budget exhaustion is a retained result");

    assert_eq!(result.status, agentive::RunStatus::Incomplete);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(result.records.is_empty());
}

#[test]
fn provider_attempt_outcomes_round_trip_and_accept_pre_attempt_history() {
    let response = ModelResponse {
        text: Some("done".to_string()),
        tool_calls: Vec::new(),
        usage: Some(usage(2, 1)),
        finish_reason: ModelFinishReason::Stop,
    };
    let outcome = AgentEffectOutcome::Provider {
        response: response.clone(),
        attempts: vec![ProviderAttempt {
            attempt: 1,
            usage: Some(usage(2, 1)),
            reserved_tokens: 0,
        }],
    };
    let restored: AgentEffectOutcome =
        serde_json::from_value(serde_json::to_value(outcome).expect("serialize"))
            .expect("deserialize");
    assert!(matches!(
        restored,
        AgentEffectOutcome::Provider { attempts, .. } if attempts.len() == 1
    ));

    let legacy = serde_json::json!({"kind":"provider", "response": response});
    let restored: AgentEffectOutcome = serde_json::from_value(legacy).expect("legacy deserialize");
    assert!(matches!(
        restored,
        AgentEffectOutcome::Provider { attempts, .. } if attempts.is_empty()
    ));
}

#[test]
fn durable_state_retains_terminal_failed_attempt_usage() {
    let mut state = AgentRunState::new(
        "durable",
        Vec::new(),
        AgentRunBudget {
            model_call_limit: 1,
        },
    );
    state
        .commit_effect(
            "durable:provider:0",
            AgentEffectOutcome::ProviderFailed {
                error: ProviderError::terminal(ProviderErrorKind::Unavailable, "safe failure")
                    .with_usage(usage(4, 0)),
                attempts: vec![ProviderAttempt {
                    attempt: 1,
                    usage: Some(usage(4, 0)),
                    reserved_tokens: 0,
                }],
            },
        )
        .expect("matching effect commits");
    let restored: AgentRunState =
        serde_json::from_value(serde_json::to_value(state).expect("serialize durable state"))
            .expect("deserialize durable state");
    assert_eq!(restored.status, agentive::RunStatus::Failed);
    assert!(restored.usage.complete);
    assert_eq!(restored.usage.model_calls.len(), 1);
    assert_eq!(restored.usage.aggregate.provider_total, Some(4));
}

#[tokio::test]
async fn retry_attempt_usage_is_retained_once_alongside_the_success() {
    let provider = ScriptedProvider::new()
        .respond_with_error(
            ProviderError::retryable(ProviderErrorKind::Transport, "safe retry")
                .with_usage(usage(3, 1)),
        )
        .respond_with(ModelResponse {
            text: Some("done".to_string()),
            tool_calls: Vec::new(),
            usage: Some(usage(5, 2)),
            finish_reason: ModelFinishReason::Stop,
        });
    let agent = Agent::builder().provider(provider).build().expect("agent");

    let result = agent
        .run_with_options(
            "start",
            RunOptions {
                provider_max_attempts: 2,
                ..RunOptions::default()
            },
        )
        .await
        .expect("retry succeeds");

    assert_eq!(result.usage.model_calls.len(), 2);
    assert!(result.usage.complete);
    assert_eq!(result.usage.aggregate.input, Some(8));
    assert_eq!(result.usage.aggregate.output, Some(3));
    assert_eq!(result.usage.aggregate.provider_total, Some(11));
}

#[tokio::test]
async fn hard_model_call_limit_caps_provider_retry_attempts() {
    let provider = ScriptedProvider::new()
        .respond_with_error(
            ProviderError::retryable(ProviderErrorKind::Unavailable, "retryable")
                .with_usage(usage(2, 0)),
        )
        .respond_with_text("must not run");
    let probe = provider.clone();
    let agent = Agent::builder().provider(provider).build().expect("agent");

    let result = agent
        .run_with_options(
            "start",
            RunOptions {
                model_call_limit: 1,
                provider_max_attempts: 3,
                ..RunOptions::default()
            },
        )
        .await
        .expect("terminal provider failure is a retained run result");

    assert_eq!(result.status, agentive::RunStatus::Failed);
    assert_eq!(result.records[0].attempts.len(), 1);
    probe.assert_request_count(1);
}

#[tokio::test]
async fn hard_token_limit_reserves_each_provider_retry_attempt() {
    let measuring = ScriptedProvider::new().respond_with_error(ProviderError::terminal(
        ProviderErrorKind::Unavailable,
        "measure one request",
    ));
    let measuring_probe = measuring.clone();
    let measuring_agent = Agent::builder().provider(measuring).build().expect("agent");
    let _ = measuring_agent.run("start").await;
    let request = measuring_probe.recorded_requests().remove(0);
    let one_attempt_limit = u64::try_from(
        serde_json::to_vec(&request)
            .expect("canonical request")
            .len(),
    )
    .expect("request length")
    .saturating_add(request.max_output_tokens);

    let provider = ScriptedProvider::new()
        .respond_with_error(ProviderError::retryable(
            ProviderErrorKind::Unavailable,
            "retryable",
        ))
        .respond_with_text("must not run");
    let probe = provider.clone();
    let agent = Agent::builder().provider(provider).build().expect("agent");

    let result = agent
        .run_with_options(
            "start",
            RunOptions {
                token_limit: Some(one_attempt_limit),
                provider_max_attempts: 2,
                ..RunOptions::default()
            },
        )
        .await
        .expect("terminal provider failure is a retained run result");

    assert_eq!(result.status, agentive::RunStatus::Failed);
    assert_eq!(result.records[0].attempts.len(), 1);
    probe.assert_request_count(1);
}

#[tokio::test]
async fn terminal_failed_attempt_usage_is_retained_once() {
    let provider = ScriptedProvider::new().respond_with_error(
        ProviderError::terminal(ProviderErrorKind::Authentication, "safe terminal failure")
            .with_usage(usage(7, 0)),
    );
    let agent = Agent::builder().provider(provider).build().expect("agent");
    let handle = agent.start("start", RunOptions::default());

    let result = handle.wait().await.expect("provider failure result");
    assert_eq!(result.status, agentive::RunStatus::Failed);
    assert_eq!(result.records[0].attempts.len(), 1);
    assert!(result.records[0].error.is_some());
    let usage = handle.usage();
    assert_eq!(usage.model_calls.len(), 1);
    assert!(usage.complete);
    assert_eq!(usage.aggregate.input, Some(7));
    assert_eq!(usage.aggregate.output, Some(0));
    assert_eq!(usage.aggregate.provider_total, Some(7));
}
