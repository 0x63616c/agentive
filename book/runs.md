# Runs, events, cancellation, and limits

Start a run when you need live observation:

```rust,ignore
use agentive::RunOptions;
use futures::StreamExt;

let handle = agent.start("Draft release notes", RunOptions::default());
let mut events = handle.events();
let _first = events.next().await;
let result = handle.wait().await?;
# Ok::<(), agentive::RunError>(())
```

Events include status transitions, model-call start/completion, and tool start/completion. Attached observers use a bounded backpressured buffer: events are not silently dropped or reordered. Dropping an observer detaches it; it does **not** cancel work.

`RunHandle::cancel()` explicitly requests cancellation and yields `RunStatus::Cancelled`. Dropping a handle or waiter only releases observation. `RunOptions::model_call_limit` is finite (default 6); exhaustion is `Incomplete`, never a hidden extra call. A `token_limit` or `deadline` similarly bounds the whole run tree. `RunResult` retains status, committed history, usage, model-call records, and any safe error text even when a run is incomplete, failed, or cancelled.
