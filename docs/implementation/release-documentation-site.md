# Release gate: documentation website

## Outcome

Rust developers can discover, install, and use every shipped v1 capability from a polished static documentation site whose source, examples, and deployment live in this repository. GitHub Pages serves the site and the repository README links to it prominently.

## Stack and hosting

- Use mdBook as the Rust-native documentation generator.
- Deploy static output with the official GitHub Pages Actions flow.
- Target `https://0x63616c.github.io/agentive/` and set the same canonical link in the README after deployment succeeds.
- Keep the site source in the repository and require no database, application server, analytics, or third-party runtime.
- Publish generated rustdoc separately through links rather than copying reference documentation into tutorials.

## Information architecture

1. Overview and project status.
2. Installation and five-minute quickstart.
3. Core concepts and canonical vocabulary.
4. Building agents and instruction layers.
5. Text and image conversations.
6. Defining stateless and stateful tools with `#[tool]`.
7. Tool context, safe errors, idempotency, and retries.
8. Scripted provider testing and provider conformance.
9. Runs, streaming events, cancellation, limits, context admission, and usage.
10. Codex subscription setup and the proven provider/runtime interface.
11. Sub-agent delegation.
12. Filesystem External Storage, codec pipeline, and Codec Server embedding.
13. Temporal workflows, crash/replay guarantees, Continue-As-New, and operations.
14. Troubleshooting, compatibility/MSRV, security defaults, and explicit non-goals.
15. API reference links and contribution guide.

Navigation includes only pages backed by shipped behavior. Later-slice pages remain absent rather than presenting aspirational APIs as available.

## Documentation seams and executable examples

- Every quickstart and feature walkthrough uses public crate interfaces only.
- Reusable snippets live as compiled examples or doctests and are included in prose where practical, preventing documentation-only code from drifting.
- Each feature guide contains: when to use it, the smallest complete example, observable result, common failure mode, and link to relevant public reference types.
- Testing guides use `ScriptedProvider` rather than hand-waved network calls.
- Temporal guides distinguish agent state, model context, Workflow History, and External Storage explicitly.
- Codex guidance never instructs users to read or copy cached credentials.

## Visual direction

A restrained technical reference: crisp high-contrast typography, compact navigation, excellent code readability, strong light/dark themes, and one small Rust/agent visual motif. The first viewport prioritizes installation and a working agent example rather than marketing copy. The site must remain usable on mobile, keyboard navigation, and 200% text enlargement.

## Test-first and release checks

1. A link checker fails on broken internal navigation and reference URLs under repository control.
2. All included Rust examples compile against the workspace.
3. mdBook builds with warnings treated as failures where supported.
4. A local static-server smoke test loads the root and representative nested pages under the `/agentive/` base path.
5. Automated accessibility checks cover headings, landmarks, link names, contrast, keyboard navigation, and overflow on representative pages.
6. GitHub Pages deployment completes successfully from `main` using the repository's configured Pages environment.
7. The live URL loads the deployed commit and the README link resolves to it.

## Final review gate

The final Sol review covers both code and documentation. It compares every claimed feature and snippet with public behavior, flags missing acceptance criteria and weak tests, and rejects claims unsupported by passing evidence.
