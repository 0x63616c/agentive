# Delegation

Delegation is an explicit model-visible tool, not hidden recursion. Register a configured child agent with `AgentBuilder::delegate`; the parent provider sees the declared name, description, and a single `task` argument. The child returns a compact structured result containing its agent name, run id, status, final text, and aggregate usage. Its history and diagnostics do not enter parent history.

```rust,ignore
use agentive::Agent;
use agentive_test::ScriptedProvider;

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let researcher = Agent::builder()
    .name("researcher")
    .provider(ScriptedProvider::new().respond_with_text("research complete"))
    .build()?;

let coordinator = Agent::builder()
    .name("coordinator")
    .provider(ScriptedProvider::new())
    .delegate(
        "research",
        "Ask the research agent one self-contained question.",
        researcher,
    )?
    .build()?;
# let _ = coordinator;
# Ok(())
# }
```

`delegate_parallel` is the explicit opt-in for independent child calls to run in bounded parallel batches. Ordinary tools and ordinary delegation are serial. The parent run's `max_parallel_tool_calls`, deadline, model-call and token budgets, and cancellation apply to the child tree.

Agentive rejects detectable static cycles during construction and returns safe tool results for depth or budget exhaustion. Child lifecycle events are attributed under the parent event stream, and child token usage is merged into the parent aggregate. There is no human-approval step in v1; applications decide which delegation tools to register.
