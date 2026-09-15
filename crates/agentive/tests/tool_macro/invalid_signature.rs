use agentive::{tool, ToolError};

#[derive(serde::Deserialize, schemars::JsonSchema)] struct Args;
#[derive(serde::Serialize)] struct Output;

#[tool(name = "invalid_signature", description = "invalid")]
fn invalid(args: Args) -> Result<Output, ToolError> { let _ = args; Ok(Output) }

fn main() {}
