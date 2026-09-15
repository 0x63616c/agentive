#![allow(missing_docs, clippy::expect_used, clippy::manual_async_fn)]

use agentive::{Agent, Tool, ToolContext, ToolName};
use agentive_test::ScriptedProvider;
use serde_json::{Value, json};

struct ManualSchemaTool {
    name: ToolName,
    schema: Value,
}

impl ManualSchemaTool {
    fn new(schema: Value) -> Self {
        Self {
            name: "schema".parse().expect("valid test tool name"),
            schema,
        }
    }
}

impl Tool for ManualSchemaTool {
    fn name(&self) -> &ToolName {
        &self.name
    }

    fn description(&self) -> &'static str {
        "validates a manually supplied schema"
    }

    fn schema_json(&self) -> &Value {
        &self.schema
    }

    fn call(
        &self,
        _: &ToolContext,
        _: Value,
    ) -> impl std::future::Future<Output = Result<Value, agentive::ToolError>> + Send + '_ {
        async { Ok(json!({"ok": true})) }
    }
}

fn build(schema: Value) -> Result<Agent, agentive::RunError> {
    Agent::builder()
        .provider(ScriptedProvider::new())
        .tool(ManualSchemaTool::new(schema))
        .build()
}

#[test]
fn registration_rejects_malformed_json_schema() {
    assert!(build(json!({"type": 7})).is_err());
}

#[test]
fn registration_rejects_non_object_argument_schema() {
    assert!(build(json!({"type": "array", "items": {"type": "string"}})).is_err());
}

#[test]
fn registration_rejects_open_top_level_object() {
    assert!(build(json!({"type": "object", "additionalProperties": true})).is_err());
}
