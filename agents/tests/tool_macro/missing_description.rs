#[macro_use]
extern crate agents_macros;

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct Args {}

#[derive(serde::Serialize, schemars::JsonSchema)]
struct Out {}

#[tool(name = "no_description")]
async fn no_description(_args: Args) -> Result<Out, agents::ToolError> {
    Ok(Out {})
}

fn main() {}
