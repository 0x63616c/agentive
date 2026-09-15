use agentive::{tool, ToolError};

#[derive(serde::Deserialize, schemars::JsonSchema)] struct Args;
#[derive(serde::Serialize)] struct Output;

#[tool(name = "bad name", description = "invalid")]
async fn invalid(args: Args) -> Result<Output, ToolError> { let _ = args; Ok(Output) }

fn main() {}
