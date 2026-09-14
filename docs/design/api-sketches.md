# API sketches

These sketches preserve concrete syntax discussed during the design interview. Each section states whether it is accepted or proposed; none is compilable implementation until the relevant specification section is complete.

The proposed hardened split between stable `ToolInvocation` identity and ephemeral `ToolContext` controls is detailed in [Tool context design](./tool-context.md).

## Provider authoring

**Accepted.** Provider authors implement a native Rust trait. The agent performs lifetime-preserving type erasure internally.

```rust
pub trait ModelProvider: Send + Sync + 'static {
    fn generate(
        &self,
        request: ModelRequest,
    ) -> impl Future<Output = Result<ModelResponse, ProviderError>> + Send + '_;

    fn stream(
        &self,
        request: ModelRequest,
    ) -> impl Future<Output = Result<ModelStream, ProviderError>> + Send + '_;

    fn capabilities(&self) -> ProviderCapabilities;
}
```

The erased interface preserves the borrow of `self`; it does not incorrectly require a `'static` future.

```rust
trait ErasedModelProvider: Send + Sync {
    fn generate(
        &self,
        request: ModelRequest,
    ) -> BoxFuture<'_, Result<ModelResponse, ProviderError>>;

    fn stream(
        &self,
        request: ModelRequest,
    ) -> BoxFuture<'_, Result<ModelStream, ProviderError>>;
}

impl<P: ModelProvider> ErasedModelProvider for P {
    fn generate(
        &self,
        request: ModelRequest,
    ) -> BoxFuture<'_, Result<ModelResponse, ProviderError>> {
        Box::pin(ModelProvider::generate(self, request))
    }

    fn stream(
        &self,
        request: ModelRequest,
    ) -> BoxFuture<'_, Result<ModelStream, ProviderError>> {
        Box::pin(ModelProvider::stream(self, request))
    }
}
```

## Agent construction

**Accepted.** The provider is selected when the agent is built and remains stable for that agent's lifetime.

```rust
let researcher = Agent::builder()
    .name("researcher")
    .agent_instructions("Research carefully and cite evidence.")
    .provider(codex.clone())
    .build()?;

let writer = Agent::builder()
    .name("writer")
    .agent_instructions("Produce a concise final answer.")
    .provider(codex)
    .sub_agent(researcher)
    .build()?;

let result = writer
    .run("Research and explain durable execution.")
    .await?;
```

## Prompt input separation

**Accepted.** The normal run call contains one user message. Standing application behavior uses `AgentBuilder::agent_instructions`; trusted per-run behavior uses `RunOptions::run_instructions` rather than `RunInput`.

```rust
let agent = Agent::builder()
    .agent_instructions("Research carefully and cite factual claims.")
    .provider(provider)
    .build()?;

let result = agent
    .run("Research durable execution and keep it under 500 words.")
    .await?;
```

Possible advanced form:

```rust
let result = agent
    .run_with_options(
        "Research durable execution.",
        RunOptions::default().run_instructions(
            "Keep this run under 500 words.",
        ),
    )
    .await?;
```

This shape is intentionally excluded:

```rust,ignore
RunInput::user("Research durable execution.")
    .run_instructions("Keep this run under 500 words.");
```

**Accepted.** One minimal, versioned SDK core fragment is always present. Enabled features may add small, versioned instruction fragments; the compiled plan is inspectable and an unused optional feature contributes nothing.

```rust
let request = agent.compile_request("Please refund my last order.")?;

assert_eq!(
    request.instructions().fragment("tool_errors").version(),
    1,
);
assert_snapshot!(request);
```

## Provider routing and fallback

**Accepted.** Routing is provider composition, not per-run mutation of an agent.

```rust
let provider = FallbackProvider::new([
    DynProvider::new(primary),
    DynProvider::new(fallback),
]);

let agent = Agent::builder()
    .provider(provider)
    .build()?;
```

The initial API deliberately omits this shape:

```rust,ignore
agent.run_with_provider(prompt, some_provider).await?;
```

## Scripted provider tests

**Accepted requirement; proposed method names.** The first-party test provider models an ordered interaction rather than exposing generated method expectations.

```rust
let provider = ScriptedProvider::new()
    .respond_with_tool_call("search", json!({ "query": "durable agents" }))
    .respond_with_text("Durable execution survives process failure.");

let agent = Agent::builder()
    .provider(provider.clone())
    .tool(search_tool)
    .build()?;

let result = agent.run("Explain durable execution.").await?;

assert_eq!(
    result.text(),
    "Durable execution survives process failure."
);
provider.assert_finished();
```

## External Storage identities

**Accepted behavior; exact constructor names remain proposed.** Public APIs use a validated `StorageId`, not free-form `&str` keys. The common single-store path hides identity and registry configuration.

```rust
let storage = ExternalStorage::single(
    FilesystemStorage::new("./payloads")?,
);
```

Multiple named drivers support migrations. New payloads use the one default writer; reads select the driver named in the reference.

```rust
const LOCAL_DEV: StorageId = StorageId::new_static("local-dev");
const FILES_OLD: StorageId = StorageId::new_static("files-old");
const FILES_PRIMARY: StorageId = StorageId::new_static("files-primary");

let storage = ExternalStorage::builder()
    .register(LOCAL_DEV, FilesystemStorage::new("./payloads"))?
    .register(FILES_OLD, FilesystemStorage::new("/mnt/payloads-old"))?
    .register(FILES_PRIMARY, FilesystemStorage::new("/mnt/payloads"))?
    .default_writer(FILES_PRIMARY)
    .build()?;
```

Runtime configuration parses and validates names once at the boundary:

```rust
let storage_id: StorageId = config.storage_id.parse()?;
```

An unknown `StorageId` in an existing reference is a typed error; the registry never guesses or silently falls back to its default writer.

**Accepted.** References use opaque object identities and carry integrity data. The exact checksum representation remains proposed.

```rust
ExternalReference {
    storage: FILES_PRIMARY,
    object: ObjectId::new(),
    size: 48_291,
    checksum: Sha256Digest::from_bytes(&encoded),
}
```

Objects are create-only. A repeated write of identical bytes under the same `ObjectId` is idempotent; different bytes under that identity fail. Reads verify both `size` and `checksum` before returning bytes.

**Accepted logical layout; exact trait method names remain proposed.** The shared External Storage module derives the entire validated relative key. A Storage Driver does not invent folder structure.

```rust
// ObjectId: 7f2ab981c4...
// ObjectKey: v1/objects/7f2/7f2ab981c4...
let key = ObjectKey::for_reference(
    ReferenceVersion::V1,
    object_id,
);

driver.create(&key, encoded).await?;
```

The three-character hexadecimal shard yields 4,096 possible top-level directories, created lazily. The v1 filesystem driver maps the `ObjectKey` beneath its configured root; future drivers receive the same validated logical key.

## Payload Codec ordering

**Accepted behavior; exact constructors remain proposed.** User-defined codec order is explicit. Encoding runs top-to-bottom and decoding reverses that order.

```rust
let codecs = PayloadCodecChain::builder()
    .push(ZstdCodec::new())
    .build()?;
```

The complete built-in v1 path is:

```text
encode: serialize -> compress -> checksum/size -> store
decode: load -> verify checksum/size -> decompress -> deserialize
```

The checksum describes the exact stored bytes for corruption detection and idempotent writes; it does not provide confidentiality or authenticate an untrusted store. V1 does not ship an authenticated-encryption codec or key manager. Applications can supply one through the same chain, conventionally after compression and before checksum calculation and storage. The SDK never silently reorders it.

**Accepted behavior; exact trait shape remains proposed.** A codec explicitly applies or declines. Applied envelopes carry typed identity and version metadata.

```rust
pub trait PayloadCodec: Send + Sync {
    fn identity(&self) -> CodecIdentity;

    async fn encode(
        &self,
        payload: Payload,
    ) -> Result<CodecOutcome, CodecError>;

    async fn decode(
        &self,
        payload: Payload,
    ) -> Result<CodecOutcome, CodecError>;
}

pub enum CodecOutcome {
    Applied(Payload),
    NotHandled(Payload),
}

pub struct CodecIdentity {
    pub id: CodecId,
    pub version: CodecVersion,
}
```

`NotHandled` forwards the unchanged payload. Plain payloads may pass through the entire chain, but a malformed, unsupported, or unresolved SDK-owned envelope is a typed terminal error after decoding.

**Accepted behavior; exact descriptor syntax remains proposed.** One codec may write a current version and retain readers for older versions.

```rust
CodecDescriptor {
    id: ZSTD,
    encode_version: V2,
    decode_versions: versions![V1, V2],
}
```

Two descriptors claiming the same `(CodecId, CodecVersion)` decode pair cause chain construction to fail. The chain never resolves duplicate ownership by registration order.

## Stateless tool macro

**Accepted.** The macro preserves the original function for direct unit testing and generates the dynamic tool adapter. Every tool has exactly one typed arguments struct. Every named model-visible property, including nested properties, has a non-empty description; Rust field doc comments are the normal source.

```rust
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    /// First number to add.
    left: i64,

    /// Second number to add.
    right: i64,
}

#[derive(Serialize)]
struct AddOutput {
    sum: i64,
}

/// Add two numbers.
#[tool(
    name = "add",
    description = "Add two integers and return their sum."
)]
async fn add(args: AddArgs) -> Result<AddOutput, ToolError> {
    Ok(AddOutput {
        sum: args.left + args.right,
    })
}
```

**Accepted.** Dynamic invocation rejects unknown fields before entering `add`, and the generated schema advertises the same closed object shape. An explicitly modeled catch-all field remains valid.

```rust
#[derive(Deserialize, JsonSchema)]
struct LabelsArgs {
    /// Natural-language query used to select labels.
    query: String,

    /// Additional caller-defined labels keyed by label name.
    #[serde(flatten)]
    labels: BTreeMap<String, String>,
}
```

Tool construction or agent registration rejects a completed schema with an undescribed named property. Examples and constraints such as `#[schemars(length(...))]`, `#[schemars(range(...))]`, and `#[schemars(example = ...)]` are optional hints.

## Stateful tool macro

**Accepted signature shape; ToolContext semantics remain proposed.** The same macro supports tools that own clients or application services. Context is optional, explicitly marked, placed before the single arguments struct, and omitted from the model-visible schema.

```rust
pub struct WeatherTool {
    client: WeatherClient,
}

/// Application-facing implementation details for the weather integration.
#[tool(
    name = "weather",
    description = "Fetch the current weather for a city."
)]
impl WeatherTool {
    async fn call(
        &self,
        #[tool(context)] context: &ToolContext,
        args: WeatherArgs,
    ) -> Result<WeatherReport, ToolError> {
        tokio::select! {
            result = self.client.fetch(&args.city) => {
                result.map_err(ToolError::from)
            }
            reason = context.cancelled() => {
                Err(ToolError::cancelled(reason))
            }
        }
    }
}
```

## Manual tool escape hatch

**Accepted architecture; exact names remain proposed.** Protocol adapters and unusual tools can implement the typed authoring trait directly. Registries contain `DynTool`, which erases associated types and the returned future.

```rust
impl Tool for WeatherTool {
    type Args = WeatherArgs;
    type Output = WeatherReport;

    fn call<'a>(
        &'a self,
        context: &'a ToolContext,
        args: WeatherArgs,
    ) -> impl Future<Output = Result<WeatherReport, ToolError>> + Send + 'a {
        async move {
            tokio::select! {
                result = self.client.fetch(&args.city) => {
                    result.map_err(ToolError::from)
                }
                reason = context.cancelled() => {
                    Err(ToolError::cancelled(reason))
                }
            }
        }
    }
}
```

The erased registry interface works only with JSON values and boxed futures.

```rust
trait ErasedTool: Send + Sync {
    fn definition(&self) -> &ToolDefinition;

    fn call_json<'a>(
        &'a self,
        context: &'a ToolContext,
        arguments: serde_json::Value,
    ) -> BoxFuture<'a, Result<ToolOutput, ToolError>>;
}

#[derive(Clone)]
pub struct DynTool(Arc<dyn ErasedTool>);
```

## Tool errors

**Accepted shape; exact kinds remain proposed.** Macro-authored tools may use the convenient SDK error directly or convert a domain error explicitly. Arbitrary `Display` and debug output never becomes model-visible automatically.

```rust
pub type ToolResult<T> = Result<T, ToolError>;

#[tool(
    name = "search",
    description = "Search indexed documents for relevant passages.",
    idempotent
)]
async fn search(args: SearchArgs) -> ToolResult<SearchOutput> {
    search_index(args)
        .await
        .map_err(|source| {
            ToolError::retryable(
                "search.backend_unavailable",
                "Search is temporarily unavailable.",
            )
                .with_source(source)
        })
}
```

Custom errors remain idiomatic when they define an explicit conversion boundary.

```rust
impl From<SearchError> for ToolError {
    fn from(error: SearchError) -> Self {
        match error {
            error @ SearchError::InvalidQuery { .. } => {
                ToolError::terminal(
                    "search.invalid_query",
                    "The query is invalid.",
                )
                    .with_source(error)
            }
            error => ToolError::retryable(
                "search.backend_unavailable",
                "Search is temporarily unavailable.",
            )
            .with_source(error),
        }
    }
}
```

The error code is open to downstream domains; retry execution is not. The runtime repeats a call only when all gates agree.

```rust
let should_retry = tool.semantics().is_idempotent()
    && error.is_retryable()
    && policy.has_attempts_remaining()
    && !context.is_cancelled();
```

The default is a terminal error, a tool with no idempotency declaration, and one attempt. Exact retry-policy builder names remain proposed.

## Tool outputs

**Accepted.** Successful outputs implement `serde::Serialize` and become canonical `serde_json::Value` values. Provider adapters perform wire conversion. There is no `Display` or debug fallback.

```rust
#[derive(Serialize)]
struct SearchOutput {
    results: Vec<SearchResult>,
    total: usize,
}

#[tool(
    name = "search",
    description = "Search indexed documents for relevant passages.",
    idempotent
)]
async fn search(args: SearchArgs) -> ToolResult<SearchOutput> {
    let results = find_results(args).await?;
    let total = results.len();
    Ok(SearchOutput { results, total })
}
```

Binary and streaming tool results are outside the first release. A tool returns structured resource references when needed.

**Accepted.** A logical tool call contributes exactly one final model-visible result. Retry counts, timing, invocation IDs, idempotency keys, and diagnostic sources remain in SDK execution records, events, and tracing rather than being injected into the result.

```rust
pub struct ToolExecutionRecord {
    pub invocation: ToolInvocation,
    pub attempts: Vec<ToolAttemptRecord>,
    pub outcome: ToolOutcome,
    pub elapsed: Duration,
}
```

## Duplicate tool names

**Accepted; exact error type remains proposed.** Provider-facing tool names are unique within an agent. Static duplicates fail agent construction; dynamic insertion rejects the new tool atomically and preserves the existing registry.

```rust
let error = Agent::builder()
    .tool(search_documents)
    .tool(search_web)
    .build()
    .unwrap_err();

assert!(matches!(
    error,
    BuildError::DuplicateToolName { ref name } if name == "search"
));
```

## Proposed quality commands

These remain proposed until the MSRV and CI decisions are accepted.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --workspace --all-features --profile ci
cargo test --doc --workspace
cargo +1.85 check --workspace --all-targets
cargo hack check --feature-powerset
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo deny check
cargo semver-checks
cargo llvm-cov
```
