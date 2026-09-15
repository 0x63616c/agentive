use agentive::{tool, ToolContext, ToolError};

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct AddArgs { value: i64 }

#[derive(serde::Serialize)]
struct AddResult { value: i64 }

struct Counter(i64);

#[tool(name = "counter_add", description = "Adds the supplied value")]
impl Counter {
    async fn call(&self, #[tool(context)] _context: &ToolContext, args: AddArgs) -> Result<AddResult, ToolError> {
        Ok(AddResult { value: self.0 + args.value })
    }
}

fn main() {}
