//! Public contract tests over a deterministic fake App Server transport.

#![allow(clippy::expect_used, clippy::manual_async_fn)] // Explicit RPITIT signatures are fixtures.

use agentive::{
    CompiledInstructions, InstructionFragment, Message, ModelRequest, Tool, ToolContext,
    ToolDefinition, ToolError, ToolMetadata, ToolName, ToolSchema,
};
use agentive_codex::{CodexRuntime, CodexSession, CodexTransport};
use futures::future::BoxFuture;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Clone)]
struct FakeTransport {
    sent: Arc<Mutex<Vec<Value>>>,
    incoming: Arc<Mutex<VecDeque<Value>>>,
    shutdowns: Arc<AtomicUsize>,
}

impl FakeTransport {
    fn new() -> Self {
        Self {
            sent: Arc::new(Mutex::new(Vec::new())),
            incoming: Arc::new(Mutex::new(default_messages())),
            shutdowns: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn with_messages(messages: Vec<Value>) -> Self {
        Self {
            sent: Arc::new(Mutex::new(Vec::new())),
            incoming: Arc::new(Mutex::new(messages.into())),
            shutdowns: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl CodexTransport for FakeTransport {
    fn open(&self) -> BoxFuture<'static, Result<Box<dyn CodexSession>, agentive::ProviderError>> {
        let sent = Arc::clone(&self.sent);
        let incoming = Arc::clone(&self.incoming);
        let shutdowns = Arc::clone(&self.shutdowns);
        Box::pin(async move {
            Ok(Box::new(FakeSession {
                sent,
                incoming,
                shutdowns,
            }) as Box<dyn CodexSession>)
        })
    }
}

struct FakeSession {
    sent: Arc<Mutex<Vec<Value>>>,
    incoming: Arc<Mutex<VecDeque<Value>>>,
    shutdowns: Arc<AtomicUsize>,
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

    fn shutdown(&mut self) -> BoxFuture<'_, Result<(), agentive::ProviderError>> {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone)]
struct StallingTransport {
    sent: Arc<Notify>,
    shutdowns: Arc<AtomicUsize>,
}

impl StallingTransport {
    fn new() -> Self {
        Self {
            sent: Arc::new(Notify::new()),
            shutdowns: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl CodexTransport for StallingTransport {
    fn open(&self) -> BoxFuture<'static, Result<Box<dyn CodexSession>, agentive::ProviderError>> {
        let sent = Arc::clone(&self.sent);
        let shutdowns = Arc::clone(&self.shutdowns);
        Box::pin(async move {
            Ok(Box::new(StallingSession { sent, shutdowns }) as Box<dyn CodexSession>)
        })
    }
}

struct StallingSession {
    sent: Arc<Notify>,
    shutdowns: Arc<AtomicUsize>,
}

impl CodexSession for StallingSession {
    fn send(&mut self, _message: Value) -> BoxFuture<'_, Result<(), agentive::ProviderError>> {
        self.sent.notify_one();
        Box::pin(async { Ok(()) })
    }

    fn receive(&mut self) -> BoxFuture<'_, Result<Option<Value>, agentive::ProviderError>> {
        Box::pin(std::future::pending())
    }

    fn shutdown(&mut self) -> BoxFuture<'_, Result<(), agentive::ProviderError>> {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
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
        SCHEMA.get_or_init(|| {
            json!({
                "type":"object",
                "properties": {
                    "value": {"type":"string", "description":"value to echo"},
                    "fail": {"type":"boolean", "description":"whether to fail"}
                },
                "additionalProperties":false
            })
        })
    }
    fn call<'call>(
        &'call self,
        _context: &'call ToolContext,
        args: Value,
    ) -> impl std::future::Future<Output = Result<Value, ToolError>> + Send + 'call {
        async move {
            if args.get("fail") == Some(&Value::Bool(true)) {
                return Err(ToolError::terminal("fixture_failed", "fixture failure"));
            }
            Ok(json!({"echo": args}))
        }
    }
}

#[tokio::test]
async fn fake_app_server_translates_text_stream_and_usage() {
    let transport = FakeTransport::new();
    let runtime = CodexRuntime::with_transport(transport.clone());
    let response = runtime.run(request()).await.expect("fake run succeeds");

    assert_eq!(response.text.as_deref(), Some("hello world"));
    assert_eq!(transport.shutdowns.load(Ordering::SeqCst), 1);
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
async fn unsupported_structured_output_fails_before_opening_transport() {
    let transport = FakeTransport::new();
    let mut structured = request();
    structured.output_format = agentive::ModelOutputFormat::Json;

    let error = CodexRuntime::with_transport(transport.clone())
        .run(structured)
        .await
        .expect_err("runtime cannot preserve structured output");

    assert!(matches!(
        error.kind,
        agentive::ProviderErrorKind::InvalidRequest
    ));
    assert!(transport.sent.lock().expect("test mutex").is_empty());
}

#[tokio::test]
async fn unsupported_context_and_output_constraints_fail_before_transport() {
    let mutations: [fn(&mut ModelRequest); 2] = [
        |request: &mut ModelRequest| request.include_context = false,
        |request: &mut ModelRequest| request.max_output_tokens = 1,
    ];
    for mutate in mutations {
        let transport = FakeTransport::new();
        let mut unsupported = request();
        mutate(&mut unsupported);

        let error = CodexRuntime::with_transport(transport.clone())
            .run(unsupported)
            .await
            .expect_err("unsupported constraint must be explicit");

        assert!(matches!(
            error.kind,
            agentive::ProviderErrorKind::InvalidRequest
        ));
        assert!(transport.sent.lock().expect("test mutex").is_empty());
    }
}

#[tokio::test]
async fn completed_handle_result_is_retained_for_a_late_waiter() {
    let handle = CodexRuntime::with_transport(FakeTransport::new()).start(request());
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let response = tokio::time::timeout(std::time::Duration::from_secs(1), handle.wait())
        .await
        .expect("retained result remains observable")
        .expect("fake run succeeds");

    assert_eq!(response.text.as_deref(), Some("hello world"));
}

#[tokio::test]
async fn tools_are_declared_to_app_server_before_the_transport_is_opened() {
    let transport = FakeTransport::new();
    let runtime = CodexRuntime::with_transport(transport.clone()).tool(EchoTool::new());
    let mut request = request();
    let definition = EchoTool::new().metadata();
    request.tools.push(agentive::ProviderToolDescriptor {
        name: definition.name,
        description: definition.description.to_owned(),
        schema: definition.schema.json,
        idempotent: definition.idempotent,
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
    let runtime = CodexRuntime::with_transport(transport.clone()).tool(EchoTool::new());
    runtime
        .run(request_with_tool(&EchoTool::new()))
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
async fn immediate_run_handle_cancellation_returns_a_canonical_timeout() {
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
}

#[tokio::test]
async fn cancellation_interrupts_stalled_initialization_and_releases_the_session() {
    let transport = StallingTransport::new();
    let handle = CodexRuntime::with_transport(transport.clone()).start(request());
    transport.sent.notified().await;
    handle.cancel();

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), handle.wait())
        .await
        .expect("cancellation must not hang during initialization")
        .expect_err("cancelled setup fails");

    assert!(matches!(error.kind, agentive::ProviderErrorKind::Timeout));
    assert_eq!(transport.shutdowns.load(Ordering::SeqCst), 1);
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
    url_request.messages = vec![
        Message::image_url("https://example.test/image.png", "image/png").expect("valid URL image"),
    ];
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
    inline_request.messages =
        vec![Message::image_inline(vec![1, 2, 3], "image/png").expect("valid inline image")];
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
        .tool(EchoTool::new())
        .run(request_with_tool(&EchoTool::new()))
        .await
        .expect("failed tool response does not crash the transport");
    let sent = transport.sent.lock().expect("test mutex");
    let response = sent
        .iter()
        .find(|message| message["id"] == "failed")
        .expect("response");
    assert_eq!(response["result"]["success"], false);
    assert_eq!(tool_error_code(response), "fixture_failed");
    assert!(response.to_string().contains("fixture failure"));
}

#[tokio::test]
async fn frozen_metadata_is_published_and_dispatched_despite_later_tool_drift() {
    let messages = tool_call_messages("drift", json!({"value":"ok"}));
    let transport = FakeTransport::with_messages(messages);
    let runtime = CodexRuntime::with_transport(transport.clone()).tool(DriftTool::new());
    let mut declared = request();
    declared.tools.push(descriptor_from(&definition_for(
        &ToolName::parse("echo").expect("valid tool name"),
        "frozen description",
        value_schema(),
    )));

    runtime.run(declared).await.expect("frozen tool succeeds");

    let sent = transport.sent.lock().expect("test mutex");
    let declaration = &sent[2]["params"]["dynamicTools"][0];
    assert_eq!(declaration["description"], "frozen description");
    assert_eq!(
        declaration["inputSchema"]["properties"]["value"]["type"],
        "string"
    );
    let response = sent
        .iter()
        .find(|message| message["id"] == "drift")
        .expect("tool response");
    assert_eq!(response["result"]["success"], true);
}

#[tokio::test]
async fn callback_arguments_are_validated_against_the_frozen_schema_before_side_effects() {
    let calls = Arc::new(AtomicUsize::new(0));
    let messages = tool_call_messages("invalid", json!({"value":"ok", "extra":true}));
    let transport = FakeTransport::with_messages(messages);
    let runtime =
        CodexRuntime::with_transport(transport.clone()).tool(CountingTool::new(Arc::clone(&calls)));

    runtime
        .run(request_with_tool(&CountingTool::new(Arc::clone(&calls))))
        .await
        .expect("invalid call is repaired");

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let sent = transport.sent.lock().expect("test mutex");
    let response = sent
        .iter()
        .find(|message| message["id"] == "invalid")
        .expect("tool response");
    assert_eq!(tool_error_code(response), "invalid_arguments");
}

#[tokio::test]
async fn duplicate_or_mismatched_tool_declarations_fail_before_transport() {
    let definition = EchoTool::new().metadata();
    let duplicate_transport = FakeTransport::new();
    let mut duplicate = request();
    duplicate.tools = vec![descriptor_from(&definition), descriptor_from(&definition)];
    let duplicate_error = CodexRuntime::with_transport(duplicate_transport.clone())
        .run(duplicate)
        .await
        .expect_err("duplicate declarations are rejected");
    assert!(matches!(
        duplicate_error.kind,
        agentive::ProviderErrorKind::InvalidRequest
    ));
    assert!(
        duplicate_transport
            .sent
            .lock()
            .expect("test mutex")
            .is_empty()
    );

    let mismatch_transport = FakeTransport::new();
    let mut mismatch = request();
    let mut mismatched = descriptor_from(&definition);
    mismatched.description = "different description".to_owned();
    mismatch.tools.push(mismatched);
    let mismatch_error = CodexRuntime::with_transport(mismatch_transport.clone())
        .tool(EchoTool::new())
        .run(mismatch)
        .await
        .expect_err("mismatched declaration is rejected");
    assert!(matches!(
        mismatch_error.kind,
        agentive::ProviderErrorKind::InvalidRequest
    ));
    assert!(
        mismatch_transport
            .sent
            .lock()
            .expect("test mutex")
            .is_empty()
    );

    let missing_transport = FakeTransport::new();
    let mut missing = request();
    missing.tools.push(descriptor_from(&definition));
    let missing_error = CodexRuntime::with_transport(missing_transport.clone())
        .run(missing)
        .await
        .expect_err("declared tools without callbacks are rejected");
    assert!(matches!(
        missing_error.kind,
        agentive::ProviderErrorKind::InvalidRequest
    ));
    assert!(
        missing_transport
            .sent
            .lock()
            .expect("test mutex")
            .is_empty()
    );

    let extra_transport = FakeTransport::new();
    let extra_error = CodexRuntime::with_transport(extra_transport.clone())
        .tool(EchoTool::new())
        .run(request())
        .await
        .expect_err("callbacks absent from the request are rejected");
    assert!(matches!(
        extra_error.kind,
        agentive::ProviderErrorKind::InvalidRequest
    ));
    assert!(extra_transport.sent.lock().expect("test mutex").is_empty());
}

#[tokio::test]
async fn tool_panics_are_model_safe_for_construction_and_polling() {
    for (call_id, tool) in [
        ("construct", PanicTool::constructing()),
        ("poll", PanicTool::polling()),
    ] {
        let transport = FakeTransport::with_messages(tool_call_messages(call_id, json!({})));
        CodexRuntime::with_transport(transport.clone())
            .tool(tool)
            .run(request_with_tool(&PanicTool::polling()))
            .await
            .expect("panic is returned as a tool result");
        let sent = transport.sent.lock().expect("test mutex");
        let response = sent
            .iter()
            .find(|message| message["id"] == call_id)
            .expect("tool response");
        assert_eq!(tool_error_code(response), "tool_panicked");
        assert!(!response.to_string().contains("private panic detail"));
    }
}

#[tokio::test]
async fn cancellation_wins_against_a_noncooperative_tool() {
    let entered = Arc::new(Notify::new());
    let transport = FakeTransport::with_messages(vec![
        json!({"id":1,"result":{"serverInfo":{"version":"2"}}}),
        json!({"id":2,"result":{"thread":{"id":"thread-a"}}}),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":"cancelled","method":"item/tool/call","params":{"threadId":"thread-a","turnId":"turn-a","callId":"call-a","tool":"wait","arguments":{}}}),
        json!({"id":4,"result":{}}),
        json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"interrupted"}}}),
    ]);
    let handle = CodexRuntime::with_transport(transport.clone())
        .tool(WaitingTool::new(Arc::clone(&entered)))
        .start(request_with_tool(&WaitingTool::new(Arc::clone(&entered))));
    entered.notified().await;
    handle.cancel();

    let error = handle.wait().await.expect_err("cancelled turn fails");
    assert!(matches!(error.kind, agentive::ProviderErrorKind::Timeout));
    let sent = transport.sent.lock().expect("test mutex");
    let response = sent
        .iter()
        .find(|message| message["id"] == "cancelled")
        .expect("tool cancellation response");
    assert_eq!(tool_error_code(response), "tool_cancelled");
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

fn tool_call_messages(call_id: &str, arguments: Value) -> Vec<Value> {
    vec![
        json!({"id":1,"result":{"serverInfo":{"version":"2"}}}),
        json!({"id":2,"result":{"thread":{"id":"thread-a"}}}),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":call_id,"method":"item/tool/call","params":{"threadId":"thread-a","turnId":"turn-a","callId":"call-a","tool":"echo","arguments":arguments}}),
        json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
    ]
}

fn descriptor_from(definition: &ToolDefinition) -> agentive::ProviderToolDescriptor {
    agentive::ProviderToolDescriptor {
        name: definition.name.clone(),
        description: definition.description.to_owned(),
        schema: definition.schema.json.clone(),
        idempotent: definition.idempotent,
    }
}

fn tool_error_code(response: &Value) -> String {
    let text = response["result"]["contentItems"][0]["text"]
        .as_str()
        .expect("tool error text");
    serde_json::from_str::<Value>(text).expect("tool error JSON")["error"]["code"]
        .as_str()
        .expect("tool error code")
        .to_owned()
}

struct DriftTool {
    name: ToolName,
    frozen: ToolDefinition,
    mutable_schema: Value,
}

impl DriftTool {
    fn new() -> Self {
        let name = ToolName::parse("echo").expect("valid tool name");
        Self {
            frozen: definition_for(&name, "frozen description", value_schema()),
            mutable_schema: json!({"type":"object","properties":{"changed":{"type":"string"}},"additionalProperties":true}),
            name,
        }
    }
}

impl Tool for DriftTool {
    fn name(&self) -> &ToolName {
        &self.name
    }
    fn description(&self) -> &'static str {
        "changed description"
    }
    fn schema_json(&self) -> &Value {
        &self.mutable_schema
    }
    fn metadata(&self) -> ToolDefinition {
        self.frozen.clone()
    }
    fn call<'call>(
        &'call self,
        _context: &'call ToolContext,
        args: Value,
    ) -> impl std::future::Future<Output = Result<Value, ToolError>> + Send + 'call {
        async move { Ok(json!({"echo":args})) }
    }
}

struct CountingTool {
    name: ToolName,
    calls: Arc<AtomicUsize>,
    schema: Value,
}

impl CountingTool {
    fn new(calls: Arc<AtomicUsize>) -> Self {
        Self {
            name: ToolName::parse("echo").expect("valid tool name"),
            calls,
            schema: value_schema(),
        }
    }
}

impl Tool for CountingTool {
    fn name(&self) -> &ToolName {
        &self.name
    }
    fn description(&self) -> &'static str {
        "counts calls"
    }
    fn schema_json(&self) -> &Value {
        &self.schema
    }
    fn call<'call>(
        &'call self,
        _context: &'call ToolContext,
        _args: Value,
    ) -> impl std::future::Future<Output = Result<Value, ToolError>> + Send + 'call {
        self.calls.fetch_add(1, Ordering::SeqCst);
        async { Ok(json!({"called":true})) }
    }
}

#[derive(Clone, Copy)]
enum PanicMode {
    Constructing,
    Polling,
}

struct PanicTool {
    name: ToolName,
    mode: PanicMode,
    schema: Value,
}

impl PanicTool {
    fn constructing() -> Self {
        Self::new(PanicMode::Constructing)
    }
    fn polling() -> Self {
        Self::new(PanicMode::Polling)
    }
    fn new(mode: PanicMode) -> Self {
        Self {
            name: ToolName::parse("echo").expect("valid tool name"),
            mode,
            schema: empty_schema(),
        }
    }
}

impl Tool for PanicTool {
    fn name(&self) -> &ToolName {
        &self.name
    }
    fn description(&self) -> &'static str {
        "panics"
    }
    fn schema_json(&self) -> &Value {
        &self.schema
    }
    fn call<'call>(
        &'call self,
        _context: &'call ToolContext,
        _args: Value,
    ) -> impl std::future::Future<Output = Result<Value, ToolError>> + Send + 'call {
        match self.mode {
            PanicMode::Constructing => panic!("private panic detail during construction"),
            PanicMode::Polling => async { panic!("private panic detail while polling") },
        }
    }
}

struct WaitingTool {
    name: ToolName,
    entered: Arc<Notify>,
    schema: Value,
}

impl WaitingTool {
    fn new(entered: Arc<Notify>) -> Self {
        Self {
            name: ToolName::parse("wait").expect("valid tool name"),
            entered,
            schema: empty_schema(),
        }
    }
}

impl Tool for WaitingTool {
    fn name(&self) -> &ToolName {
        &self.name
    }
    fn description(&self) -> &'static str {
        "waits indefinitely"
    }
    fn schema_json(&self) -> &Value {
        &self.schema
    }
    fn call<'call>(
        &'call self,
        _context: &'call ToolContext,
        _args: Value,
    ) -> impl std::future::Future<Output = Result<Value, ToolError>> + Send + 'call {
        self.entered.notify_one();
        std::future::pending()
    }
}

fn definition_for(name: &ToolName, description: &'static str, schema: Value) -> ToolDefinition {
    ToolMetadata {
        name: name.clone(),
        description,
        schema: ToolSchema { json: schema },
        idempotent: false,
    }
}

fn value_schema() -> Value {
    json!({
        "type":"object",
        "properties":{"value":{"type":"string","description":"value to process"}},
        "additionalProperties":false
    })
}

fn empty_schema() -> Value {
    json!({"type":"object","properties":{},"additionalProperties":false})
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
        max_output_tokens: 0,
        output_format: agentive::ModelOutputFormat::Text,
        invocation_id: "fixture".to_owned(),
    }
}

fn request_with_tool(tool: &impl Tool) -> ModelRequest {
    let mut request = request();
    request.tools.push(descriptor_from(&tool.metadata()));
    request
}
