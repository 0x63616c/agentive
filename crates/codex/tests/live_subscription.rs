//! Deliberately opt-in evidence for one locally authenticated subscription run.

#![allow(clippy::expect_used)] // Assertions should panic with fixture-specific context.

use agentive::{
    CompiledInstructions, InstructionFragment, Message, ModelRequest, Tool, ToolCallFuture,
    ToolContext, ToolName,
};
use agentive_codex::CodexRuntime;
use serde_json::{Value, json};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};

struct OnceTool {
    name: ToolName,
    calls: Arc<AtomicUsize>,
}

impl Tool for OnceTool {
    fn name(&self) -> &ToolName {
        &self.name
    }
    fn description(&self) -> &'static str {
        "Returns the exact word pong. Use exactly once when asked."
    }
    fn schema_json(&self) -> &Value {
        static SCHEMA: OnceLock<Value> = OnceLock::new();
        SCHEMA.get_or_init(|| json!({"type":"object","additionalProperties":false}))
    }
    fn call<'call>(
        &'call self,
        _context: &'call ToolContext,
        _args: Value,
    ) -> ToolCallFuture<'call> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(json!({"value":"pong"})) })
    }
}

#[tokio::test]
#[ignore = "requires an already-authenticated local Codex subscription and spends one bounded turn"]
async fn live_subscription_text_tool_usage_and_process_lifecycle() {
    assert_eq!(
        std::env::var("AGENTIVE_CODEX_LIVE").as_deref(),
        Ok("1"),
        "set AGENTIVE_CODEX_LIVE=1 deliberately"
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = Arc::new(OnceTool {
        name: ToolName::parse("once_ping").expect("valid tool name"),
        calls: Arc::clone(&calls),
    });
    let request = ModelRequest {
        instructions: CompiledInstructions {
            core: InstructionFragment::new("core", 1, "Follow trusted application instructions."),
            agent: None,
            run: None,
            features: Vec::new(),
        },
        messages: vec![Message::user(
            "Use once_ping exactly once, then reply with only the word done.",
        )],
        tools: Vec::new(),
        include_context: true,
        // Read-only `model/list` discovery on 2026-09-14 reported this installed
        // model as "Fast and affordable". App Server exposes no subscription price.
        model: Some("gpt-5.6-luna".to_owned()),
        max_output_tokens: 16,
        invocation_id: "agentive-codex-live-one-shot".to_owned(),
    };
    let response = CodexRuntime::new()
        .with_tools([tool as Arc<dyn Tool>])
        .run(request)
        .await
        .expect("authenticated subscription turn succeeds");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the model must invoke the trivial tool exactly once"
    );
    assert_eq!(response.text.as_deref().map(str::trim), Some("done"));
    assert!(
        response.usage.is_some(),
        "App Server must return usage for release evidence"
    );
}
