use agentive::{tool, ToolError};

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct Args { value: String }

#[derive(serde::Serialize)]
struct Output { value: String }

struct Error;
impl From<Error> for ToolError {
    fn from(_: Error) -> Self { ToolError::terminal("custom", "safe") }
}

#[tool(name = "custom_error", description = "Tests Into ToolError")]
async fn custom(args: Args) -> Result<Output, Error> { Ok(Output { value: args.value }) }

fn main() {}
