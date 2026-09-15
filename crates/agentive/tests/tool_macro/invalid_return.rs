use agentive::tool;

#[derive(serde::Deserialize, schemars::JsonSchema)] struct Args;

#[tool(name = "invalid_return", description = "invalid")]
async fn invalid(args: Args) -> String { let _ = args; String::new() }

fn main() {}
