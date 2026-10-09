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
`--submit-key ctrl-enter` (or `BB_TUI_SUBMIT_KEY=ctrl-enter`, or `submit_key` in the settings file) makes Enter
insert a new line and `Alt-Enter`/`C-Enter` send. Bracketed paste and the kitty
keyboard protocol are enabled when the terminal supports them.

## Mentions and attachments

The composer and the new-thread prompt complete mentions like the web
promptbox (`client-core/src/prompt/mentions`):

| Trigger | Completes | Source |
|---|---|---|
| `@` | workspace files and folders, thread-storage files, threads, projects, sections, plugin items | `GET /environments/:id/paths` (or `/projects/:id/paths`), `GET /threads/:id/thread-storage/paths`, the sidebar, `GET /plugins/mentions/search` |
| `#`, `$`, `!`, `~` | plugin items of mention providers that registered the trigger | `GET /plugins/contributions` (`mentionProviders`), `GET /plugins/mentions/search?trigger=` |
| `/`, `$` | provider commands and skills, plus the provider's plan/goal prompt actions (`$` lists skills only) | `GET /projects/:id/commands?provider=` with the thread's environment |

A trigger opens after whitespace or `(`, `[`, `{`; `@` queries may contain
spaces. The popup groups and ranks results like the web menu (exact, prefix,
substring, then supporting-text matches; paths first when the query contains
`/`). Path and plugin searches wait 120 ms after the last keystroke and cancel
the previous request. `↑`/`↓` or `C-n`/`C-p` move, `Enter`/`Tab` insert, `Esc`
dismisses until the cursor leaves the trigger; with no rows (hint, loading)
`Enter` still sends. A `/` text that matches no command stays plain text.

An inserted mention is a chip: its serialized text (`@src/a.rs`,
`@thread:thr_…`, `/review`, `#Issue title`) is highlighted, the cursor jumps
over it, and `Backspace`/`Delete` or any edit that touches it removes the whole
chip. Mention ranges are kept and sent as `PromptTextMention` offsets in UTF-16
code units, so emoji (surrogate pairs, ZWJ sequences, flags), Cyrillic and CJK
text give the same `start`/`end` as the web editor.

Attachments: `C-o` (or `:attach` alone on the draft, or the palette's
"Attach file") opens a path prompt with `Tab` completion; `:attach <path…>` as
the whole draft attaches directly (quote paths with spaces). Pasting text that
is only existing file paths (absolute, `~/`, `file://` URIs, shell-quoted or
escaped) attaches them instead of inserting text. Files are uploaded with
`POST /projects/:id/attachments` (multipart field `file`, 35 MB limit checked
before reading) and shown as chips above the editor; uploads in progress show
as `… name`. `M-a` lists attachments: `j`/`k` select, `d`/`x`/`Del` remove.
Drafts keep mentions and attachments in the web draft format.
`--read-only` refuses uploads; completion only reads.

Not built yet: rendering a chip with its label instead of its serialized text,
image previews of attachments, thread-link paste conversion, and dragging
threads into the composer.

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

## Right panel

`i` shows or hides the right panel for the open thread, `D` opens it on the
Diff tab and focuses it (again closes it), `>`/`<` switch tabs and `Z`
maximizes it over the timeline. In the panel `1`/`2`/`3` pick the Info, Diff
and File tabs, `r` reloads, and `Esc` returns from a preview, restores a
maximized panel, then leaves the panel. The keys follow the web app's
`panel.toggle`, `diff.toggle`, `panel.nextTab`, `panel.previousTab` and
`panel.fullScreen.toggle` commands, so server keybinding overrides rebind them.
The panel reloads when the server reports a work-status or git-refs change of
the environment.

**Info** follows the web thread-info card (`ThreadMetadataContent`): status,
project, environment, machine (offline marker), directory, branch or detached
HEAD, merge base and how far ahead/behind it the branch is, the pull request
(number, state, checks/review next step, auto-merge), archived, provider,
model and reasoning level, context-window usage, created and updated times,
then the commits ahead of the merge base, the files they changed and the
uncommitted files with line counts. Data comes from `GET
/environments/:id/status` (with the environment's merge base branch, or the
default branch when it has none) and `GET /environments/:id/pull-request`.
`j`/`k` move, `Enter` opens a commit's diff, reveals a file in the diff, copies
the directory or branch, or opens the pull request; `p` previews a file, `e`
opens it in `$EDITOR`, `y` copies the SHA, path or URL.

**Diff** lists `GET /environments/:id/diff/files` for a scope: all changes
against the merge base, committed changes, uncommitted changes, or a single
commit (`s`/`S` cycle through the ones that exist, like the web selector).
With more than ten files every file starts collapsed (deleted files always
do). Patches load lazily: the server's inline patches are used first, then
`POST /environments/:id/diff/patch` fetches expanded `auto` files near the
viewport in batches of at most 50 paths; `on_demand` files (over 500 changed
lines or binary) load on `Enter`, `too_large` files (over 20 000 lines) are
never fetched and can only be previewed. Only the visible rows are rendered,
so a diff with hundreds of files opens as fast as the file list arrives. Lines
show old/new line numbers, `+`/`-` with a tinted background and syntect
highlighting.

| Keys (Diff) | Action |
|---|---|
| `j`/`k`, `PgUp`/`PgDn`, `C-d`/`C-u`, `g`/`G` | Move |
| `h`/`l`, `0` | Scroll sideways, back to the first column |
| `Enter` | Fold / unfold the file, load an on-demand diff |
| `n`/`N` | Next / previous hunk |
| `]`/`[` | Next / previous file |
| `a` | Fold / unfold all files |
| `s`/`S` | Next / previous scope |
| `p` | Preview the file at the selected line (`GET /environments/:id/diff/file`, new side; old side for deleted files) |
| `e` | Open the file at the selected line in `$VISUAL`/`$EDITOR` |
| `y` | Copy the line or the path |

**File** shows the previewed file with line numbers and highlighting (first
512 KiB; binary files are described, not shown). `e` opens it in the editor:
the TUI leaves the alternate screen, runs `$VISUAL`, `$EDITOR` or `vi` with
`+line` (or `--goto path:line` for VS Code-style editors) and redraws when the
editor exits. This only works when the workspace path exists on this machine.

Git actions use `POST /environments/:id/actions` and always ask for
confirmation (`y`/`Enter` confirm, `Esc` cancel): `C` commits all uncommitted
changes (the server stages them and writes the commit message), `M` merges the
open pull request (`←`/`→` or `1`-`3` pick squash, merge or rebase), `R` marks
a draft pull request ready and `W` converts an open one to draft. They are also
in the command palette. `--read-only` refuses them; everything else in the
panel only reads (the patch batch request is a `POST` but changes nothing).

Not built yet: choosing another merge base branch, the file filter, context
expansion around hunks, image previews, forks and thread-storage sections,
showing cost (the server does not report it), and the web's push/create-PR
flows (the actions API has no such actions).

## Notifications

bb-tui watches the thread list and notifies you when a thread in the
background needs you, following the push-notifications plugin's rules
(`plugins/push-notifications/sender.ts`):

| Event | Detected when | Body |
|---|---|---|
| Needs input | a thread gets a pending request | the request, e.g. `Approve command: cargo test` or the question (`GET /threads/:id/interactions`) |
| Error | a thread's status becomes `error` | `The thread hit an error` |
| Turn finished | a top-level thread goes from running to `idle` | `Finished and waiting for you` |

Events on one thread are coalesced for 2 seconds and only the most important
one is shown (needs input, then error, then finished). Nothing is shown when
the thread was read in the meantime (`lastReadAt` at or after the attention
time), when it is archived, deleted or hidden, or when it is the thread you
have open and the terminal has focus. Terminal focus comes from focus
reporting (`CSI ? 1004 h`); terminals that do not report it count as focused.

Signals from the push-notifications plugin (`plugin-signal` with channel
`notification`, sent when its web or desktop channel is enabled) are shown
too, unless the TUI already handled the same thread within 30 seconds; test
notifications (`bb push-notifications test desktop`) show up this way.

Each notification goes to the notification center, an in-app toast, the
terminal bell, and a desktop notification:

- `osc9` writes `ESC ] 9 ; title: body BEL` (iTerm2, WezTerm, kitty, Ghostty).
- `osc777` writes `ESC ] 777 ; notify ; title ; body BEL` (foot, urxvt, Ghostty).
- `native` uses the system notification service (D-Bus, macOS, Windows). It
  needs a build with `--features notifications`.
- `auto` picks `native` in such a build, otherwise detects the terminal from
  `TERM_PROGRAM`, `LC_TERMINAL`, `TERM`, `KITTY_WINDOW_ID` and
  `WEZTERM_PANE`, and falls back to the bell only.

Inside tmux the OSC sequences are wrapped in a DCS passthrough, so they need
`set -g allow-passthrough on`. By default desktop notifications are sent only
while the terminal is not focused.

`!` (or "Show notifications" in the palette) opens the notification center,
newest first, and marks everything read. In the center, `j`/`k` move,
`Enter` opens the thread (with the request panel focused if it waits for
you), `d`/`x`/`Del` delete one, `C` clears all and `Esc` closes. Opening a
thread marks its notifications read. The status bar shows `! N new` for
unread notifications, `⚑ N` for threads that need attention, and `⚑` alone
when `GET /system/attention` reports attention outside the listed threads.

## Settings file

Settings live in `~/.config/bb-tui/config.toml` (`$XDG_CONFIG_HOME/bb-tui/`).
Use `--config <path>` or `BB_TUI_CONFIG` to point somewhere else. The file is
optional, and every key has a default:

```toml
submit_key = "enter"
theme = "base16-ocean.dark"

[notifications]
enabled = true
turn_finished = true
needs_input = true
errors = true
toast = true
bell = true
desktop = "auto"
desktop_when = "unfocused"
plugin_signals = true
terminal_title = false
```

| Key | Values |
|---|---|
| `submit_key` | `enter` or `ctrl-enter`. `--submit-key` and `BB_TUI_SUBMIT_KEY` override it |
| `theme` | syntax highlighting theme for code blocks, diffs and previews: `base16-ocean.dark`, `base16-eighties.dark`, `base16-mocha.dark`, `base16-ocean.light`, `InspiredGitHub`, `Solarized (dark)`, `Solarized (light)` |
| `notifications.enabled` | turns all thread notifications on or off |
| `notifications.turn_finished`, `needs_input`, `errors` | which events notify |
| `notifications.toast`, `bell` | the in-app toast and the terminal bell |
| `notifications.desktop` | `auto`, `osc9`, `osc777`, `native`, `off` |
| `notifications.desktop_when` | `unfocused` (default) or `always` |
| `notifications.plugin_signals` | show push-notifications plugin signals |
| `notifications.terminal_title` | set the terminal title to `bb`, `(N) bb` with N threads needing attention, or `● bb` when the server reports attention elsewhere. The previous title is restored on exit (xterm title stack) |

Unknown keys and invalid values are reported as toasts at startup, and the
default is used instead. `bb-tui config` prints the file path, any warnings and
the effective settings.

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

`renders_live_panel_read_only` opens up to `BB_TUI_LIVE_THREADS` (default 5)
threads with an environment, loads their Info, Diff (file list and inline
patches only) and the preview of the first changed file through the GET
endpoints, and never runs git actions, patch batches or an editor.
`renders_live_mentions_read_only` opens a thread with an environment, types
an `@` mention after Cyrillic and emoji text, inserts the first result as a
chip, opens the `/` command list and prints the draft JSON; it never sends,
uploads or saves.
`renders_live_palette_read_only` loads the plugin commands, runs a thread
search for a word from a sidebar title, searches files of a thread's
environment and previews the first match; it never runs plugin commands.
`renders_live_notifications_read_only` loads the thread list and
`/system/attention`, simulates a turn finishing on a local copy of one idle
thread, checks that it notifies and renders the notification center, and
describes the pending requests of up to three waiting threads; it sends
nothing.
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
