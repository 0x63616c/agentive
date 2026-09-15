#![allow(missing_docs, clippy::expect_used)]

use agentive::{Agent, FallbackProvider, ProviderError, ProviderErrorKind, RunStatus};
use agentive_test::ScriptedProvider;

#[tokio::test]
async fn fallback_is_a_runtime_counted_attempt_not_a_hidden_retry() {
    let primary = ScriptedProvider::new().respond_with_error(ProviderError::retryable(
        ProviderErrorKind::Unavailable,
        "primary unavailable",
    ));
    let fallback = ScriptedProvider::new().respond_with_text("fallback answer");
    let primary_probe = primary.clone();
    let fallback_probe = fallback.clone();
    let agent = Agent::builder()
        .provider(FallbackProvider::new(primary, fallback))
        .build()
        .expect("agent");

    let result = agent.run("start").await.expect("fallback succeeds");

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text.as_deref(), Some("fallback answer"));
    assert_eq!(result.usage.model_calls.len(), 2);
    primary_probe.assert_request_count(1);
    fallback_probe.assert_request_count(1);
}

#[tokio::test]
async fn terminal_primary_failure_does_not_route_to_fallback() {
    let primary = ScriptedProvider::new().respond_with_error(ProviderError::terminal(
        ProviderErrorKind::Authentication,
        "authentication failed",
    ));
    let fallback = ScriptedProvider::new().respond_with_text("must not run");
    let fallback_probe = fallback.clone();
    let agent = Agent::builder()
        .provider(FallbackProvider::new(primary, fallback))
        .build()
        .expect("agent");

    let result = agent.run("start").await.expect("terminal failure result");

    assert_eq!(result.status, RunStatus::Failed);
    assert!(result.records[0].error.is_some());
    fallback_probe.assert_request_count(0);
}
