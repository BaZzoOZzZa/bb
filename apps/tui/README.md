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

`--server-url` falls back to `BB_SERVER_URL`. Press `q` to quit and `?` for
the key list. `--read-only` (or `BB_TUI_READ_ONLY=1`) never sends changes to
the server: opening a thread does not mark it read, and rename, archive, pin
and read toggles are refused.

## Thread timeline

Opening a thread shows its timeline in the main pane. Conversation messages are
rendered as Markdown (code blocks highlighted with syntect, tables, lists,
links shown as `text (url)`); work, system, step/bundle summary and turn rows
are one-line titles that expand in place. The view follows new output while it
is scrolled to the bottom and stops following as soon as you scroll up; older
messages load when you reach the top. Collapsed turns and deferred command,
tool and diff content are fetched when expanded.

| Keys | Action |
|---|---|
| `j`/`k`, `g`/`G` | Select row, jump to top / bottom (bottom resumes following) |
| `PgUp`/`PgDn`, `C-u`/`C-d` | Scroll a page / half page |
| `Enter`, `Space`, `l`/`h` | Toggle, expand, collapse (or go to parent) |
| `[`/`]` | Previous / next user message |
| `y` | Copy the selected row to the clipboard (OSC 52) |
| `o` | Open the row's link or file with `xdg-open` / `open` (picker if several) |

Through Turbo from the repository root:

```sh
pnpm exec turbo run build test lint typecheck --filter=@bb/tui
```

## Server connection (`bb-client`)

The server URL is resolved in this order: `--server-url`, `BB_SERVER_URL`,
`serverUrl` from `$BB_DATA_DIR/bb-app-runtime.json` (default `~/.bb`) when its
`pid` is still running, then `http://127.0.0.1:38886`. The client then probes
`GET /health` and `GET /api/v1/system/version`; an old or unknown server
version is a warning, not an error.

Saved servers live in `~/.config/bb-tui/servers.json` (or
`$XDG_CONFIG_HOME/bb-tui/`). A bb connect profile is paired from a code minted
by `bb connect machine-code`; its `bbcm_` machine credential is stored in the
OS keyring when built with `--features keyring`, otherwise in
`~/.config/bb-tui/credentials.json` with mode 0600. REST calls send it as
`x-bb-connect-machine`; WebSockets use a desktop-session cookie that is renewed
five minutes before expiry and after a 401/403.

The live integration tests are ignored by default and only send GET requests:

```sh
cargo test -p bb-client --test live -- --ignored --nocapture
cargo test -p bb-tui --test live -- --ignored --nocapture
```

`renders_live_timelines_read_only` parses the latest timeline and every
collapsed turn of the most recently updated threads (`BB_TUI_LIVE_THREADS`,
default 10), reports unknown row kinds and server fields the wire types do not
model, and renders each thread fully expanded (`BB_TUI_LIVE_PRINT=1` prints the
screens).

## Checks

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
