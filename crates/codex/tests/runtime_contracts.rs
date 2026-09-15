//! Public contract tests over a deterministic fake App Server transport.

#![allow(clippy::expect_used)] // Assertions should panic with fixture-specific context.

use agentive::{
    CompiledInstructions, InstructionFragment, Message, ModelRequest, Tool, ToolCallFuture,
    ToolContext, ToolError, ToolName,
};
use agentive_codex::{CodexRuntime, CodexSession, CodexTransport};
use futures::future::BoxFuture;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct FakeTransport {
    sent: Arc<Mutex<Vec<Value>>>,
    incoming: Arc<Mutex<VecDeque<Value>>>,
}

impl FakeTransport {
    fn new() -> Self {
        Self {
            sent: Arc::new(Mutex::new(Vec::new())),
            incoming: Arc::new(Mutex::new(default_messages())),
        }
    }

    fn with_messages(messages: Vec<Value>) -> Self {
        Self {
            sent: Arc::new(Mutex::new(Vec::new())),
            incoming: Arc::new(Mutex::new(messages.into())),
        }
    }
}

impl CodexTransport for FakeTransport {
    fn open(&self) -> BoxFuture<'static, Result<Box<dyn CodexSession>, agentive::ProviderError>> {
        let sent = Arc::clone(&self.sent);
        let incoming = Arc::clone(&self.incoming);
        Box::pin(
            async move { Ok(Box::new(FakeSession { sent, incoming }) as Box<dyn CodexSession>) },
        )
    }
}

struct FakeSession {
    sent: Arc<Mutex<Vec<Value>>>,
    incoming: Arc<Mutex<VecDeque<Value>>>,
}

impl CodexSession for FakeSession {
    fn send(&mut self, message: Value) -> BoxFuture<'_, Result<(), agentive::ProviderError>> {
        self.sent.lock().expect("test mutex").push(message);
        Box::pin(async { Ok(()) })
    }

    fn receive(&mut self) -> BoxFuture<'_, Result<Option<Value>, agentive::ProviderError>> {
        let item = self.incoming.lock().expect("test mutex").pop_front();
        Box::pin(async move { Ok(item) })
    }
}

struct EchoTool {
    name: ToolName,
}

impl EchoTool {
    fn new() -> Self {
        Self {
            name: ToolName::parse("echo").expect("valid tool name"),
        }
    }
}

impl Tool for EchoTool {
    fn name(&self) -> &ToolName {
        &self.name
    }
    fn description(&self) -> &'static str {
        "Echoes input"
    }
    fn schema_json(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| json!({"type":"object"}))
    }
    fn call<'call>(
        &'call self,
        _context: &'call ToolContext,
        args: Value,
    ) -> ToolCallFuture<'call> {
        Box::pin(async move {
            if args.get("fail") == Some(&Value::Bool(true)) {
                return Err(ToolError::terminal("fixture_failed", "fixture failure"));
            }
            Ok(json!({"echo": args}))
        })
    }
}

#[tokio::test]
async fn fake_app_server_translates_text_stream_and_usage() {
    let transport = FakeTransport::new();
    let runtime = CodexRuntime::with_transport(transport.clone());
    let response = runtime.run(request()).await.expect("fake run succeeds");

    assert_eq!(response.text.as_deref(), Some("hello world"));
    assert_eq!(
        response.usage.as_ref().and_then(|usage| usage.input),
        Some(3)
    );
    assert_eq!(
        response.usage.as_ref().and_then(|usage| usage.output),
        Some(2)
    );
    let sent = transport.sent.lock().expect("test mutex");
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[1]["method"], "initialized");
    assert_eq!(sent[2]["method"], "thread/start");
    assert_eq!(sent[3]["method"], "turn/start");
    assert_eq!(
        sent[3]["params"]["input"][0],
        json!({"type": "text", "text": "hello"})
    );
}

#[tokio::test]
async fn tools_are_declared_to_app_server_before_the_transport_is_opened() {
    let transport = FakeTransport::new();
    let runtime = CodexRuntime::with_transport(transport.clone());
    let mut request = request();
    request.tools.push(agentive::ProviderToolDescriptor {
        name: agentive::ToolName::parse("echo").expect("valid name"),
        description: "Echo a value".to_owned(),
        schema: json!({"type": "object"}),
        idempotent: true,
    });

    runtime.run(request).await.expect("tools are declared");

    let sent = transport.sent.lock().expect("test mutex");
    assert_eq!(sent[2]["params"]["dynamicTools"][0]["name"], "echo");
}

#[tokio::test]
async fn dynamic_tool_callbacks_use_the_canonical_tool_and_correlated_response() {
    let messages = vec![
        json!({"id": 1, "result": {"serverInfo": {"version": "2"}}}),
        json!({"id": 2, "result": {"thread": {"id": "thread-a"}}}),
        json!({"id": 3, "result": {"turn": {"id": "turn-a"}}}),
        json!({"id": "server-tool", "method": "item/tool/call", "params": {"threadId":"thread-a", "turnId":"turn-a", "callId":"call-a", "tool":"echo", "arguments":{"value":"ok"}}}),
        json!({"method": "turn/completed", "params": {"threadId":"thread-a", "turn": {"id":"turn-a", "status":"completed"}}}),
    ];
    let transport = FakeTransport::with_messages(messages);
    let runtime = CodexRuntime::with_transport(transport.clone())
        .with_tools([Arc::new(EchoTool::new()) as Arc<dyn Tool>]);
    runtime
        .run(request())
        .await
        .expect("tool callback succeeds");
    let sent = transport.sent.lock().expect("test mutex");
    let response = sent
        .iter()
        .find(|message| message["id"] == "server-tool")
        .expect("correlated response");
    assert_eq!(response["result"]["success"], true);
    assert_eq!(response["result"]["contentItems"][0]["type"], "inputText");
    assert!(
        response["result"]["contentItems"][0]["text"]
            .as_str()
            .expect("text")
            .contains("echo")
    );
}

#[tokio::test]
async fn run_handle_interrupts_the_turn_and_returns_a_canonical_timeout() {
    let transport = FakeTransport::with_messages(vec![
        json!({"id": 1, "result": {"serverInfo": {"version": "2"}}}),
        json!({"id": 2, "result": {"thread": {"id": "thread-a"}}}),
        json!({"id": 3, "result": {"turn": {"id": "turn-a"}}}),
        json!({"id": 4, "result": {}}),
        json!({"method":"item/agentMessage/delta", "params":{"threadId":"thread-a", "turnId":"turn-a", "delta":"late"}}),
        json!({"method": "turn/completed", "params": {"threadId":"thread-a", "turn": {"id":"turn-a", "status":"interrupted"}}}),
    ]);
    let handle = CodexRuntime::with_transport(transport.clone()).start(request());
    handle.cancel();
    let error = handle.wait().await.expect_err("interrupted run fails");
    assert!(matches!(error.kind, agentive::ProviderErrorKind::Timeout));
    assert!(
        transport
            .sent
            .lock()
            .expect("test mutex")
            .iter()
            .any(|message| message["method"] == "turn/interrupt")
    );
}

#[tokio::test]
async fn protocol_auth_eof_and_version_failures_are_classified_without_server_text() {
    let cases = [
        (
            vec![json!({"id": 99, "result": {}})],
            agentive::ProviderErrorKind::Protocol,
        ),
        (
            vec![json!({"id": 1, "error": {"message": "unauthorized: secret"}})],
            agentive::ProviderErrorKind::Authentication,
        ),
        (Vec::new(), agentive::ProviderErrorKind::Transport),
        (
            vec![json!({"id": 1, "result": {"serverInfo": {"version": "999"}}})],
            agentive::ProviderErrorKind::Protocol,
        ),
    ];
    for (messages, expected) in cases {
        let error = CodexRuntime::with_transport(FakeTransport::with_messages(messages))
            .run(request())
            .await
            .expect_err("fixture fails");
        assert!(std::mem::discriminant(&error.kind) == std::mem::discriminant(&expected));
        assert!(!error.message.contains("secret"));
    }
}

#[tokio::test]
async fn url_images_map_losslessly_and_inline_images_fail_before_opening_transport() {
    let transport = FakeTransport::new();
    let mut url_request = request();
    url_request.messages = vec![Message::image_url(
        "https://example.test/image.png",
        "image/png",
    )];
    CodexRuntime::with_transport(transport.clone())
        .run(url_request)
        .await
        .expect("URL image maps");
    assert_eq!(
        transport.sent.lock().expect("test mutex")[3]["params"]["input"][0],
        json!({"type":"image", "url":"https://example.test/image.png"})
    );

    let unopened = FakeTransport::new();
    let mut inline_request = request();
    inline_request.messages = vec![Message::image_inline(vec![1, 2, 3], "image/png")];
    let error = CodexRuntime::with_transport(unopened.clone())
        .run(inline_request)
        .await
        .expect_err("inline is unsupported");
    assert!(matches!(
        error.kind,
        agentive::ProviderErrorKind::InvalidRequest
    ));
    assert!(unopened.sent.lock().expect("test mutex").is_empty());
}

#[tokio::test]
async fn unknown_and_malformed_dynamic_tools_are_safe_protocol_results() {
    let base = vec![
        json!({"id":1,"result":{"serverInfo":{"version":"2"}}}),
        json!({"id":2,"result":{"thread":{"id":"thread-a"}}}),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
    ];
    let mut unknown = base.clone();
    unknown.push(json!({"id":"unknown","method":"item/tool/call","params":{"threadId":"thread-a","turnId":"turn-a","callId":"call-a","tool":"missing","arguments":{}}}));
    unknown.push(json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}));
    let transport = FakeTransport::with_messages(unknown);
    CodexRuntime::with_transport(transport.clone())
        .run(request())
        .await
        .expect("unknown call gets a response");
    {
        let sent = transport.sent.lock().expect("test mutex");
        let response = sent
            .iter()
            .find(|message| message["id"] == "unknown")
            .expect("response");
        assert_eq!(response["result"]["success"], false);
        assert!(!response.to_string().contains("secret"));
    }

    let mut malformed = base;
    malformed.push(json!({"id":"bad","method":"item/tool/call","params":{"threadId":"thread-a","tool":"missing"}}));
    let error = CodexRuntime::with_transport(FakeTransport::with_messages(malformed))
        .run(request())
        .await
        .expect_err("missing correlation is invalid");
    assert!(matches!(error.kind, agentive::ProviderErrorKind::Protocol));
}

#[tokio::test]
async fn dynamic_tool_failures_are_returned_as_safe_failed_results() {
    let messages = vec![
        json!({"id":1,"result":{"serverInfo":{"version":"2"}}}),
        json!({"id":2,"result":{"thread":{"id":"thread-a"}}}),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":"failed","method":"item/tool/call","params":{"threadId":"thread-a","turnId":"turn-a","callId":"call-a","tool":"echo","arguments":{"fail":true}}}),
        json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
    ];
    let transport = FakeTransport::with_messages(messages);
    CodexRuntime::with_transport(transport.clone())
        .with_tools([Arc::new(EchoTool::new()) as Arc<dyn Tool>])
        .run(request())
        .await
        .expect("failed tool response does not crash the transport");
    let sent = transport.sent.lock().expect("test mutex");
    let response = sent
        .iter()
        .find(|message| message["id"] == "failed")
        .expect("response");
    assert_eq!(response["result"]["success"], false);
    assert!(
        response["result"]["contentItems"][0]["text"]
            .as_str()
            .expect("text")
            .contains("fixture_failed")
    );
}

#[tokio::test]
async fn concurrent_runs_keep_their_transports_and_thread_ids_isolated() {
    let left = FakeTransport::with_messages(messages_for("left-thread", "left-turn", "left"));
    let right = FakeTransport::with_messages(messages_for("right-thread", "right-turn", "right"));
    let left_runtime = CodexRuntime::with_transport(left.clone());
    let right_runtime = CodexRuntime::with_transport(right.clone());
    let (left_result, right_result) =
        tokio::join!(left_runtime.run(request()), right_runtime.run(request()),);
    assert_eq!(left_result.expect("left").text.as_deref(), Some("left"));
    assert_eq!(right_result.expect("right").text.as_deref(), Some("right"));
    assert_eq!(
        left.sent.lock().expect("test mutex")[3]["params"]["threadId"],
        "left-thread"
    );
    assert_eq!(
        right.sent.lock().expect("test mutex")[3]["params"]["threadId"],
        "right-thread"
    );
}

fn messages_for(thread_id: &str, turn_id: &str, text: &str) -> Vec<Value> {
    vec![
        json!({"id":1,"result":{"serverInfo":{"version":"2"}}}),
        json!({"id":2,"result":{"thread":{"id":thread_id}}}),
        json!({"id":3,"result":{"turn":{"id":turn_id}}}),
        json!({"method":"item/agentMessage/delta","params":{"threadId":thread_id,"turnId":turn_id,"delta":text}}),
        json!({"method":"turn/completed","params":{"threadId":thread_id,"turn":{"id":turn_id,"status":"completed"}}}),
    ]
}

fn default_messages() -> VecDeque<Value> {
    VecDeque::from([
        json!({"id": 1, "result": {"serverInfo": {"version": "2"}}}),
        json!({"id": 2, "result": {"thread": {"id": "thread-a"}}}),
        json!({"id": 3, "result": {"turn": {"id": "turn-a"}}}),
        json!({"method": "item/agentMessage/delta", "params": {"threadId": "thread-a", "turnId": "turn-a", "delta": "hello "}}),
        json!({"method": "item/agentMessage/delta", "params": {"threadId": "thread-a", "turnId": "turn-a", "delta": "world"}}),
        json!({"method": "thread/tokenUsage/updated", "params": {"threadId": "thread-a", "tokenUsage": {"inputTokens": 3, "outputTokens": 2}}}),
        json!({"method": "turn/completed", "params": {"threadId": "thread-a", "turn": {"id": "turn-a", "status": "completed"}}}),
    ])
}

fn request() -> ModelRequest {
    ModelRequest {
        instructions: CompiledInstructions {
            core: InstructionFragment::new("core", 1, "trusted core"),
            agent: Some("trusted agent".to_owned()),
            run: None,
            features: Vec::new(),
        },
        messages: vec![Message::user("hello")],
        tools: Vec::new(),
        include_context: true,
        model: Some("gpt-5.6-terra".to_owned()),
        max_output_tokens: 64,
        invocation_id: "fixture".to_owned(),
    }
}
