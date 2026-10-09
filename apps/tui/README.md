# bb-tui

Terminal client for bb, written in Rust (ratatui + crossterm + tokio).

## Layout

- `crates/bb-wire` — serde mirrors of the server contract
- `crates/bb-client` — REST, realtime WebSocket, connect auth
- `crates/bb-store` — cache, invalidation, timeline merge
- `crates/bb-view` — view-model and formatting ported from `packages/thread-view`
- `crates/bb-tui` — the `bb-tui` binary

## Build and run

```sh
cd apps/tui
cargo run -- --server-url http://127.0.0.1:3000
cargo build --release
```

`--server-url` falls back to `BB_SERVER_URL`. Press `q` to quit.

Through Turbo from the repository root:

```sh
pnpm exec turbo run build test lint typecheck --filter=@bb/tui
```

## Checks

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
