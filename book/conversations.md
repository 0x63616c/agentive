# Text, images, and history

Use `RunOptions::history` to continue a conversation:

```rust,ignore
use agentive::{Message, RunOptions};

let options = RunOptions {
    history: vec![
        Message::user("What is in this image?"),
        Message::image_url("https://example.invalid/photo.png", "image/png"),
    ],
    ..RunOptions::default()
};
assert_eq!(options.history.len(), 2);
```

URL and inline images are separate forms. `Message::image_inline` copies bytes into the message; `Message::image_url` records a URL and media type. Providers without image support reject unsupported content rather than silently changing it. V1 supports text and images only: generic files, audio, and video are not content types.

History is not automatically summarized, truncated, compacted, or persisted. Context policy is all-or-error: the complete supplied history and the new user message must fit the selected provider/model limit using exact or conservative admission, otherwise the run fails before transport. `provider_enforced_limit_opt_out` skips only local admission; it never permits SDK truncation.

`RunUsage` aggregates known input, output, cached-input, reasoning, and provider-total measurements across model calls and child agents. Missing provider measurements remain `None` and mark the aggregate incomplete; they never become zero.
