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

## Composer

The bottom of the thread view is the composer (`c` or `Tab` focuses it). It
follows the web app's follow-up rules from `client-core/src/prompt`: an idle
thread gets `POST /threads/:id/send` with `mode: "queue-if-active"`; while the
thread is active, starting, provisioning, stopping or waiting for its host,
Enter adds the message to the server-side queue instead; the steer key sends
`mode: "steer-if-active"`, or with an empty draft sends the first queued
message with `mode: "steer"`. Sending is blocked while the agent waits for an
answer to a pending request, and on an idle thread until its default execution
options have loaded.

| Keys (composer focused) | Action |
|---|---|
| `Enter` | Send (queue while running) |
| `Shift-Enter`, `Alt-Enter`, `C-j` | New line |
| `C-s` (`C-Enter` where the terminal reports it) | Steer the running turn |
| `↑`/`↓` on the first/last line | Prompt history (`GET /threads/:id/prompt-history`) |
| `Esc Esc` | Stop the running turn (a single `Esc` leaves the composer when idle) |
| `C-c` | Clear the draft; on an empty draft stop the turn, or quit when idle |

| Keys (thread open) | Action |
|---|---|
| `M-m`, `M-e`, `M-p` | Model, reasoning level and permission mode pickers |
| `M-.`/`M-,`, `M-r`/`M-R` | Cycle model, cycle reasoning level |
| `M-t` | Toggle what Enter does while running: queue (default) or steer |
| `M-q` | Queued messages: `j`/`k`, `Enter`/`e` edit, `s` send now, `r` retry a failed one, `d` delete |
| `S` | Stop the running turn |

Pickers use `GET /system/execution-options?providerId=&environmentId=` and
`GET /threads/:id/default-execution-options`: only the provider's permission
modes are offered, modes above the machine's permission ceiling are disabled,
the reasoning level is reconciled when the model changes, and an active model
fallback is used until you pick another model. Picked values are sent as
`executionInputSources: "explicit"`.

Editing a queued message holds it on the server (`POST
/queued-messages/:id/edit-hold`, renewed four times per lease) and saves with
`PATCH` and `expectedUpdatedAt`; `Esc` cancels and restores your draft.
Above the editor the composer shows the plan-mode, goal, todo and model-fallback
banners, the queue, and a footer with the model, reasoning level, permission
mode and context-window usage.

Unsent drafts are saved per thread under `$XDG_STATE_HOME/bb-tui/drafts`
(`~/.local/state/bb-tui/drafts`) in the web app's draft format.
`--submit-key ctrl-enter` (or `BB_TUI_SUBMIT_KEY=ctrl-enter`) makes Enter
insert a new line and `Alt-Enter`/`C-Enter` send. Bracketed paste and the kitty
keyboard protocol are enabled when the terminal supports them.

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

`renders_live_composer_read_only` parses the execution options, default
execution options, prompt history and queue of the same threads and renders
the composer for each. `renders_live_timelines_read_only` parses the latest timeline and every
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
