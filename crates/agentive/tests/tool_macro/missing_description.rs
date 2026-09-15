use agentive::tool;

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct Args {}

#[derive(serde::Serialize, schemars::JsonSchema)]
struct Out {}

#[tool(name = "no_description")]
async fn no_description(_args: Args) -> Result<Out, agentive::ToolError> {
    Ok(Out {})
}

fn main() {}
