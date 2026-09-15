use agentive::{ToolContext, ToolError, tool};

#[derive(schemars::JsonSchema, serde::Deserialize)]
struct Arguments;

#[derive(serde::Serialize)]
struct Output;

#[tool(name = "wrong_context_order", description = "Rejects a context after arguments.")]
async fn wrong_context_order(
    _arguments: Arguments,
    #[tool(context)] _context: &ToolContext,
) -> Result<Output, ToolError> {
    Ok(Output)
}

fn main() {}
