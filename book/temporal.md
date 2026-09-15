# Temporal runtime

`agentive-temporal` ships a durable adapter that drives the same serializable `AgentRunState` state machine as local execution. A Temporal workflow selects one explicit provider or tool effect; `AgentiveActivities` executes that effect outside replay; the workflow commits the returned outcome and advances. Provider and tool code never runs in workflow replay.

The setup has three deliberate pieces:

1. Build one immutable `PayloadPipeline`, install `temporal_data_converter(pipeline.clone())` on every Temporal client and worker, and keep its configuration stable for a history.
2. Register `AgentiveWorkflow` plus `AgentiveActivities::new(agent)` using `runtime::worker_options`.
3. Start or reconnect with `TemporalRuntime`; its handle can `observe`, `cancel`, and await `result` across Continue-As-New.

```rust,ignore
use agentive::{Agent, AgentRunBudget, AgentRunState};
use agentive_temporal::{FilesystemStorage, PayloadPipeline, StorageId, temporal_data_converter};
use agentive_temporal::runtime::{AgentiveActivities, TemporalRunConfig, TemporalRuntime, worker_options};

# async fn configure(
#     agent: Agent,
#     client: temporalio_client::Client,
# ) -> Result<(), Box<dyn std::error::Error>> {
let pipeline = PayloadPipeline::single_store(
    FilesystemStorage::new("/var/lib/agentive-payloads")?,
    StorageId::new("production-filesystem")?,
)?;
let _converter = temporal_data_converter(pipeline.clone());
let _options = worker_options("agentive", AgentiveActivities::new(agent))?;

let state = AgentRunState::new("run-42", vec![], AgentRunBudget { model_call_limit: 6 });
let handle = TemporalRuntime::new(client)
    .start("agentive", state, TemporalRunConfig::new(250)?)
    .await?;
let _same_run = handle.workflow_id();
# Ok(())
# }
```

The snippet is schematic at the worker-construction boundary: use the Temporal SDK to build a worker from `_options` and install `_converter` on that worker's client as well. The critical invariant is that clients, workers, and Codec Server share an equivalent pipeline.

`TemporalRunConfig` carries a versioned adapter schema and a nonzero Continue-As-New threshold. The workflow checks its history patch marker, snapshots committed state for `observe`, and continues deterministically before the per-execution effect threshold. A `TemporalRunHandle` reconnects by workflow id; dropping it does not cancel work. Calling `cancel` reaches the canonical Agentive cancellation token inside a running activity and rejects a late effect result.

Activity-level Temporal retries are set to one attempt so they never multiply Agentive's canonical provider/tool retry policy. Durable replay coverage replays committed external-payload history through the configured converter; the ignored local-dev-server tests exercise filesystem payload round trips, reconnect, Continue-As-New, and cancellation. Run those release checks explicitly in an environment allowed to download and start the Temporal CLI dev server.

When operating it, retain filesystem payload objects longer than their Workflow History and archives. Agentive never infers reachability or garbage-collects them.
