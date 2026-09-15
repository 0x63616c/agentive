use agentive::{tool, ToolError};

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct EchoArgs {
    text: String,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
struct EchoResult {
    output: String,
}

#[tool(name = "echo", description = "Echo text")]
async fn echo(args: EchoArgs) -> Result<EchoResult, ToolError> {
    Ok(EchoResult {
        output: args.text,
    })
}

fn main() {}
