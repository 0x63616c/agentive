# App Server conformance record

Observed against the locally installed `codex-cli` 0.153.4 and the
version-matched JSON Schema generated with `codex app-server
generate-json-schema` on 2026-09-14.

| Capability | Result | Boundary |
| --- | --- | --- |
| Subscription authentication | Supported | The spawned App Server owns cached ChatGPT authentication; this crate never reads credential files or accepts tokens. |
| Text | Supported | `turn/start` text items; incremental `item/agentMessage/delta` becomes canonical response text. |
| URL images | Supported | Canonical URL images map to App Server `image` items. |
| Inline images | Unsupported | The current public turn schema exposes URL and local-path image forms, not an inline byte representation. Rejected before process startup. |
| Tools | Experimental support | `dynamicTools` declares canonical tool schemas. Correlated `item/tool/call` requests execute only registered canonical `Tool` objects and receive `inputText` content items. Unsupported protocol versions are rejected. |
| Usage | Partially supported | Known App Server token fields map directly; omitted measurements remain `None`. |
| Cancellation | Supported | `CodexRunHandle::cancel` shares its canonical cancellation token with tool contexts, sends `turn/interrupt`, and ignores late text deltas while awaiting the terminal interruption. |
| Parallel isolation | Supported | Every `run` opens a dedicated App Server process/session and uses its returned thread and turn ids to filter notifications. |

The selected public name is `CodexRuntime`, not `CodexProvider`. App Server is a
rich agent runtime with persistent threads, turns, items, approvals, and its own
tool loop; describing it as a raw one-model-turn provider would create a hidden
nested Agentive loop.

## Live smoke test

The ignored smoke test is an explicit manual release gate and must be run by a developer who is
already authenticated in local Codex. It does not take an API key and never
prints account data. Hosted CI deliberately cannot run it because no developer
subscription session is installed there. Run it with:

```text
AGENTIVE_CODEX_LIVE=1 cargo test -p agentive-codex live_subscription_text_tool_usage_and_process_reaping -- --ignored
```

The prepared release-evidence test is
`live_subscription_text_tool_usage_and_process_reaping`. A read-only
`model/list` request to installed Codex Desktop 0.153.4 on 2026-09-14 listed
`gpt-5.6-luna` as “Fast and affordable”, with text/image capability and a fast
service tier. Subscription pricing is not exposed by App Server, so this is the
smallest/fastest metadata-based choice rather than a price claim.

Release evidence: the test passed on 2026-09-14 with `gpt-5.6-luna`. It completed
one authenticated subscription run, invoked the registered tool exactly once,
returned exactly `done`, reported usage, and released the child process. The
runtime now closes, terminates when necessary, and reaps every stdio session.

## OpenClaw comparison

The current [OpenClaw App Server handler](https://github.com/openclaw/openclaw/blob/01e2590b4b409d74c8cf5fca464bf964fab442a1/extensions/codex/src/app-server/run-attempt-server-requests.ts#L164-L204)
validates `threadId` and `turnId` before dispatching an `item/tool/call`, then
returns content items. Its [default response](https://github.com/openclaw/openclaw/blob/01e2590b4b409d74c8cf5fca464bf964fab442a1/extensions/codex/src/app-server/client.ts#L1150-L1164)
uses `inputText`, confirming this adapter's callback shape.

OpenClaw also has an explicit [auth bridge](https://github.com/openclaw/openclaw/blob/01e2590b4b409d74c8cf5fca464bf964fab442a1/extensions/codex/src/app-server/auth-bridge.ts#L695-L726)
which can inject prepared OAuth/API-key material using `account/login/start`.
Agentive deliberately does not implement that path: its spawned process removes
ambient API-key/token variables and relies only on the user's already-authenticated
local Codex subscription. This preserves the subscription-only boundary and never
copies, reads, or logs credentials.
