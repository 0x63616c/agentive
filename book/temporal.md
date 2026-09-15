# Temporal runtime

`agentive-temporal` ships a durable adapter that drives the same serializable `AgentRunState` state machine as local execution. A Temporal workflow selects one explicit provider or tool effect; `AgentiveActivities` executes that effect outside replay; the workflow commits the returned outcome and advances. Provider and tool code never runs in workflow replay.

The setup has three deliberate pieces:

1. Build one immutable `PayloadPipeline`, install `temporal_data_converter(pipeline.clone())` on every Temporal client and worker, and keep its configuration stable for a history.
2. Register `AgentiveWorkflow` plus `AgentiveActivities::new(agent)` using `runtime::worker_options`.
3. Start or reconnect with `TemporalRuntime`; its handle can `observe`, `cancel`, and await `result` across Continue-As-New.

```rust,ignore
use agentive::{Agent, Message, RunOptions};
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
let state = agent.prepare_state(
    "run-42",
    vec![Message::user("Research durable execution")],
    &RunOptions::default(),
)?;
let _options = worker_options("agentive", AgentiveActivities::new(agent))?;

let handle = TemporalRuntime::new(client)
    .start("agentive", state, TemporalRunConfig::new(250)?)
    .await?;
let _same_run = handle.workflow_id();
# Ok(())
# }
```

The snippet is schematic at the worker-construction boundary: use the Temporal SDK to build a worker from `_options` and install `_converter` on that worker's client as well. The critical invariant is that clients, workers, and Codec Server share an equivalent pipeline.

`TemporalRunConfig` carries a versioned adapter schema, a validated workflow fingerprint, and a nonzero Continue-As-New threshold. The workflow rejects an unsupported fingerprint before effects, checks its history patch marker, snapshots committed state for `observe`, and continues deterministically before the per-execution effect threshold. A `TemporalRunHandle` reconnects by workflow id; dropping it does not cancel work. Calling `cancel` reaches the canonical Agentive cancellation token inside a running activity and rejects a late effect result.

Agentive owns classified provider/tool policy retries. Temporal activities are **at least once**, but a provider activity is scheduled with one attempt by default: provider redelivery needs an explicit `TemporalRunConfig::allow_durable_idempotent_provider()` contract that the provider deduplicates its stable Agentive provider-effect identity. A tool is rejected before scheduling unless `TemporalRunConfig` explicitly names it with `allow_durable_idempotent_tool`; that is the corresponding contract for its stable Agentive idempotency key. Only those explicit durable-safe providers and allowlisted tools receive the bounded two-attempt infrastructure-redelivery policy for lost workers and completion acknowledgements. Deterministic Agentive configuration/protocol errors remain non-retryable, and local retry idempotency alone is not a durable-redelivery contract. Cancellation commits and exposes the canonical `Cancelled` state through the workflow result and `observe` snapshot; a late activity outcome cannot replace it. The activity schedule-to-close timeout is the persisted run deadline remaining at scheduling time, so redelivery cannot reset the durable elapsed-time allowance. When the run has no deadline, the adapter uses the largest duration representable by Agentive's persisted millisecond contract because Temporal requires an activity timeout; it does not impose a short adapter-specific cap. After an outcome is committed, replay does not restart Agentive's inner retry sequence. Durable replay coverage replays committed external-payload history through the configured converter; the local-dev-server release workflow runs the ignored payload round trip, worker kill/restart recovery, retry composition, and cancellation scenarios.

When operating it, retain filesystem payload objects longer than their Workflow History and archives. Agentive never infers reachability or garbage-collects them.
