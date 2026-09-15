#!/usr/bin/env bash
set -euo pipefail

cargo +1.94.0 fmt --all --check
cargo +1.94.0 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.0 nextest run --workspace --all-features
cargo +1.94.0 test --workspace --all-features --doc
RUSTDOCFLAGS="-D warnings" cargo +1.94.0 doc --workspace --all-features --no-deps

cargo +stable clippy --workspace --all-targets --all-features -- -D warnings
cargo +stable nextest run --workspace --all-features
cargo +stable test --workspace --all-features --doc

cargo deny check
cargo hack check --workspace --feature-powerset --depth 1
