use agentive::{ToolContext, ToolError, tool};

#[derive(schemars::JsonSchema, serde::Deserialize)]
struct Arguments;

#[derive(serde::Serialize)]
struct Output;

#[tool(name = "malformed_context", description = "Rejects malformed context markers.")]
async fn malformed_context_marker(
    #[tool(not_context)] _context: &ToolContext,
    _arguments: Arguments,
) -> Result<Output, ToolError> {
    Ok(Output)
}

fn main() {}
