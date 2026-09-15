# Typed tools

The `#[tool]` macro creates a public `Tool` implementation, schema, strict argument decoding, and structured serialization from an explicit declaration. Arguments deserialize and implement `JsonSchema`; results serialize to canonical JSON.

```rust,ignore
use agentive::{ToolError, tool};

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct AddArgs {
    /// First signed integer.
    #[schemars(description = "First signed integer.")]
    left: i64,
    /// Second signed integer.
    #[schemars(description = "Second signed integer.")]
    right: i64,
}
#[derive(serde::Serialize, schemars::JsonSchema)]
struct AddResult { sum: i64 }

#[tool(name = "add", description = "Adds two signed integers")]
async fn add(args: AddArgs) -> Result<AddResult, ToolError> {
    Ok(AddResult { sum: args.left + args.right })
}
```

The macro leaves `add` directly callable and generates `AddTool`, the zero-sized `Tool` implementation. Register it with `.tool(AddTool)` on the agent builder. Names are explicit portable-ASCII contracts; duplicate effective names fail construction. The argument payload must be one typed structure (use an empty structure for no arguments), and runtime decoding rejects unknown fields before deserialization.

`ToolContext` is not model input. Place it before the payload and mark it explicitly:

```rust,ignore
use agentive::{tool, ToolContext, ToolError};

# #[derive(serde::Deserialize, schemars::JsonSchema)]
# struct LookupArgs { key: String }
#[tool(name = "lookup", description = "Looks up one public record")]
async fn lookup(
    #[tool(context)] context: &ToolContext,
    args: LookupArgs,
) -> Result<serde_json::Value, ToolError> {
    if context.is_cancelled() {
        return Err(ToolError::terminal("cancelled", "tool invocation was cancelled"));
    }
    Ok(serde_json::json!({"key": args.key}))
}
```

Return `ToolError::terminal(code, message)` or `ToolError::retryable(code, message)`. Both fields are model-visible contracts: keep them stable and safe; never include secrets, stack traces, or arbitrary client errors.

`ToolContext` exposes stable invocation identity, attempt, remaining time, cancellation, and delegation bounds only. Stateful tools own their own clients; the macro also supports an `impl` block with an async `call(&self, ...)` method. Retrying is runtime-owned and requires explicit `idempotent` declaration plus an applicable policy. Return `ToolError::terminal(code, message)` for a final model-safe failure or `ToolError::retryable(code, message)` only when the tool is idempotent and a retry can be safe.
