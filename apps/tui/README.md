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

## Pending requests

When the agent waits for you, the open thread replaces the composer with a
request panel (`c` or `Tab` focuses it; it takes focus by itself if you were in
the composer). The sidebar marks waiting threads with `?`, the status bar shows
`? N waiting`, and `w` (`M-w` from the composer) opens the next waiting thread
with its panel focused. With several requests on one thread they are answered
oldest first (`1 of N`).

| Request | Keys |
|---|---|
| Command, file change, permission, tool approval | `y` allow once · `s` allow for session · `d` deny · `1-3` by position · `r` deny and steer a reason to the agent · `e` show the whole command |
| Plan review | `y` approve plan · `d` keep planning · `f` keep planning and steer feedback |
| Questions (provider or the ask-user-question plugin) | `1-9` choose (the last number is `Other…`) · `j/k` + `Space` · `Tab`/`←→` switch question · `Enter` next / submit · typing goes to the `Other…` answer |
| Secret request (secrets plugin) | type into the masked field · `Tab`/`↑↓` field · `C-t` show/hide · `Enter` next empty field, then submit |
| Any other plugin request | shown as pretty JSON · `o` open the thread in the web app · `y` copy the link · `x` cancel |

`C-x` cancels question, secret and plugin requests (a provider request stops
the turn instead, like the web app) and `Esc` leaves the panel. Denial reasons
and plan feedback are not part of the resolve request, so they are sent as a
steer message after the denial succeeds. Secret values are never shown unless
revealed, are redacted from debug output and cleared after submission.

Through Turbo from the repository root:

```sh
pnpm exec turbo run build test lint typecheck --filter=@bb/tui
```

## New thread

`n` opens the new-thread form in the main pane for the selected project (the
sidebar selection, else the open thread's project). `N` starts a thread in the
open thread's environment: it reuses that environment, keeps the thread's
section and pin, and puts the cursor in the prompt. The form shows one row per
choice, in order:

| Row | What it picks |
|---|---|
| Project | any project, or Personal |
| Environment | a machine and an environment option (project checkout, git worktree, personal workspace, plugin providers), a hostless provider, or `Reuse existing environment…` (an environment other threads of the project use) |
| Branch | git worktree: the base branch (default or any local/remote branch); project checkout: keep the current checkout, check out a local branch, or create a new branch from one. Shown only for those two options |
| Provider · Model · Reasoning · Permissions | the execution options for the routed machine or environment |

Defaults follow the web app's new-thread composer: the project's saved
execution defaults, the project checkout on the primary machine when the
project has a source there, the personal workspace for Personal. Choices that
cannot work are listed but disabled with the reason: an option the machine
reports unavailable, a plugin option whose inputs need the plugin's web
control, an offline machine, a branch change on a dirty, detached or mid-rebase
checkout, a permission mode above the machine's limit. The line under the rows
says why the thread cannot be created yet, or that it is ready.

Keys: `j`/`k` (or `Tab`/`S-Tab`) move between rows, `Enter` opens a row's
picker (type to filter, `Enter` choose, `Esc` cancel) and picking moves to the
next row, `i` jumps to the prompt. In the prompt, `Enter` creates the thread
(`--submit-key` applies), `Tab`/`S-Tab` go back to the rows, `M-m`/`M-e`/`M-p`
open the model, reasoning and permission pickers, and `Esc` leaves the prompt.
`Esc` on the rows closes the form; the unsent prompt is kept for the next time
it opens. The created thread opens right away. `--read-only` refuses to create
threads.

Not built yet: reusing an existing git worktree by path, the service-tier
picker, guided machine setup, plugin input controls other than the git
worktree and project checkout ones, and remembering choices between sessions.

## Command palette and search

`C-k` (or `:`) opens the command palette, `C-p` quick-opens a file, `f`
searches threads and `P` switches project. `C-k` and `C-p` also work from the
composer, the request panel and the new-thread form. The palette keys follow
the web app's `APP_COMMAND_IDS` (`palette.open`, `file.quickOpen`,
`thread.search`), so keybinding overrides from the server settings rebind them
too; `/` stays the in-place sidebar filter.

| Mode | What it lists | Enter |
|---|---|---|
| Commands | every TUI command with its bound keys, grouped like the web palette, then the plugin CLI commands from `GET /plugins/contributions` | run the command; a plugin command first asks for its arguments |
| Threads | with an empty query the 20 most recently updated threads; otherwise local fuzzy title and project-name matches merged with `GET /threads/search` (title and message matches, an `Archived` group, excerpts around the match) | open the thread |
| Projects | sidebar projects | reveal and expand the project in the sidebar |
| Files | `GET /environments/:id/paths` for the open (or selected) thread's environment, else `GET /projects/:id/paths` | preview the file (`GET …/files/:path`) with syntax highlighting |

Type to filter; `↑`/`↓`, `Tab`/`S-Tab` or `C-n`/`C-p` move, `PgUp`/`PgDn`
page, `Esc` closes, and `Backspace` on an empty query returns to the command
list. `C-k` in another mode switches to the command list and closes the
palette when it is already there. Ranking ports
`packages/fuzzy-match` on top of nucleo: commands match their title (and their
group as an alias), ties go to recently run commands and then to the list
order; with an empty query the eight recently run commands come first.
Recents are stored in `$XDG_STATE_HOME/bb-tui/palette-recents.json`. Thread
search waits 150 ms and file search 120 ms after the last keystroke and cancels
the previous request; while file results are in flight the previous results are
re-ranked locally.

A plugin command opens an argument line prefilled with its subcommand (taken
from the command's usage, e.g. `bb thread-list prefs list`); quotes and
backslashes work like a shell. Enter runs `POST /plugins/:id/cli` with the
open thread and its project as context, and the output (stdout, then stderr in
yellow, with the exit code) is shown in a scrollable result pane. File
previews use the same pane.

| Keys (result pane) | Action |
|---|---|
| `j`/`k`, `PgUp`/`PgDn`/`Space`, `C-d`/`C-u`, `g`/`G` | Scroll |
| `h`/`l`, `0` | Scroll sideways, back to the first column |
| `y` | Copy the command's stdout, or the file's path |
| `o` | Open the previewed file with `xdg-open` / `open` (when its workspace path is known) |
| `Esc`, `q`, `Enter` | Close |

`--read-only` refuses to run plugin commands; searching and previews only read.
Not built yet: the web palette's settings and plugin-page entries, the
archived-thread recents list, sorting threads by date, the thread-view
submenu, and passing a working directory to plugin commands.

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

`renders_live_palette_read_only` loads the plugin commands, runs a thread
search for a word from a sidebar title, searches files of a thread's
environment and previews the first match; it never runs plugin commands.
`renders_live_pending_interactions_read_only` parses every listed thread's
pending interactions and renders the request panel of each waiting thread.
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
