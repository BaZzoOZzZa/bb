# bb-tui

Terminal client for bb, written in Rust (ratatui + crossterm + tokio). It is
a public client of the bb server: everything goes through the same REST API
and realtime WebSocket the web app uses, and the server keeps owning product
policy. The TUI never talks to the host daemon.

## Contents

- [Install](#install)
- [Run](#run)
- [Connecting to a server](#connecting-to-a-server)
- [Environment variables](#environment-variables)
- [Settings file](#settings-file)
- [Colors and mouse](#colors-and-mouse)
- [Read-only mode](#read-only-mode)
- [bb-tui doctor](#bb-tui-doctor)
- [Keys](#keys)
- Feature guides: [timeline](#thread-timeline), [composer](#composer),
  [mentions](#mentions-and-attachments), [requests](#pending-requests),
  [new thread](#new-thread), [palette](#command-palette-and-search),
  [right panel](#right-panel), [terminals](#terminals),
  [notifications](#notifications), [settings](#server-settings),
  [automations](#automations), [split panes](#split-panes)
- [Coverage compared with the web app](#coverage-compared-with-the-web-app)
- [Development](#development)

## Install

You need Rust 1.88 or newer and a running bb server
(the bb desktop app, `bb serve`, or a machine reachable through bb connect).

```sh
cd apps/tui
cargo install --path crates/bb-tui
cargo install --path crates/bb-tui --features keyring,notifications
```

The binary is `bb-tui`. Optional features:

| Feature | Adds |
|---|---|
| `keyring` | stores bb connect machine credentials in the OS keyring instead of `~/.config/bb-tui/credentials.json` |
| `notifications` | `desktop = "native"`: system notifications through D-Bus, macOS or Windows |

From the repository root the package also builds through Turbo:

```sh
pnpm exec turbo run build test lint typecheck --filter=@bb/tui
```

## Run

```sh
bb-tui
bb-tui --server-url http://127.0.0.1:38886
bb-tui --profile work
bb-tui --read-only
```

| Command | Does |
|---|---|
| `bb-tui` | opens the TUI against the discovered server |
| `bb-tui connect pair <code>` | redeems a bb connect machine code and saves a profile |
| `bb-tui connect list` | lists saved profiles |
| `bb-tui config` | prints the settings file path, any warnings and the effective settings |
| `bb-tui doctor` | checks discovery, the HTTP API and the WebSocket, then exits |
| `bb-tui --help`, `--version` | usage and version |

| Flag | Env | Meaning |
|---|---|---|
| `--server-url <url>` | `BB_SERVER_URL` | server to use (cannot be combined with `--profile`) |
| `--profile <name>` | `BB_TUI_PROFILE` | saved profile, matched by id, name, connect handle or URL |
| `--read-only` | `BB_TUI_READ_ONLY` | never send changes ([details](#read-only-mode)) |
| `--submit-key enter\|ctrl-enter` | `BB_TUI_SUBMIT_KEY` | what Enter does in the composer |
| `--appearance auto\|dark\|light` | `BB_TUI_APPEARANCE` | color scheme ([details](#colors-and-mouse)) |
| `--no-mouse` | `BB_TUI_NO_MOUSE` | do not capture the mouse |
| `--config <path>` | `BB_TUI_CONFIG` | settings file |

`q` quits, `?` lists the keys. On start the TUI checks `GET /health` and
`GET /api/v1/system/version`; an unreachable server, an old server (older
than 0.45) or a server that is moving machines shows as a toast, and the
banner at the top reports a lost realtime connection while it reconnects.

## Connecting to a server

Without `--profile`, the server URL is resolved in this order:

1. `--server-url`
2. `BB_SERVER_URL`
3. `serverUrl` from `$BB_DATA_DIR/bb-app-runtime.json` (default `~/.bb`), when
   the `pid` written there is still running (the bb desktop app writes it)
4. `http://127.0.0.1:38886`

`bb-tui doctor` prints each of these and the one that won.

### bb connect profiles

A remote bb server reachable through bb connect is paired once with a machine
code:

```sh
bb connect machine-code
bb-tui connect pair bbmc_…
bb-tui connect pair 'https://…?code=…'
bb-tui connect pair bbmc_… --name work --device-name laptop --no-activate
bb-tui connect list
bb-tui --profile work
```

`pair` accepts the bare code, the pairing link, or the JSON record the web app
copies. It saves the profile in `~/.config/bb-tui/servers.json`
(`$XDG_CONFIG_HOME/bb-tui/`), stores the `bbcm_` machine credential in the OS
keyring (with `--features keyring`) or in `~/.config/bb-tui/credentials.json`
(mode 0600), and makes the profile active unless `--no-activate` is given.
`connect list` marks the active profile; select one at startup with
`--profile` or `BB_TUI_PROFILE`. REST calls send the credential as
`x-bb-connect-machine`; WebSockets use a desktop-session cookie that is
renewed five minutes before it expires and after a 401/403.

The credential lets the holder act on the server as you. Do not share
`credentials.json` or pairing codes.

## Environment variables

| Variable | Effect |
|---|---|
| `BB_SERVER_URL` | server URL (after `--server-url`) |
| `BB_DATA_DIR` | bb data directory searched for `bb-app-runtime.json` (default `~/.bb`) |
| `BB_TUI_PROFILE` | saved profile to use (`--profile`) |
| `BB_TUI_READ_ONLY` | `1`/`true` turns on read-only mode |
| `BB_TUI_SUBMIT_KEY` | `enter` or `ctrl-enter` |
| `BB_TUI_APPEARANCE` | `auto`, `dark` or `light` |
| `BB_TUI_NO_MOUSE` | `1`/`true` leaves the mouse to the terminal |
| `BB_TUI_CONFIG` | settings file path |
| `BB_TUI_LOG` | append logs to this file (nothing is logged otherwise) |
| `RUST_LOG` | log filter for `BB_TUI_LOG`, e.g. `bb_client=debug` (default `info`) |
| `NO_COLOR` | any non-empty value turns colors off |
| `COLORFGBG` | read by `appearance = "auto"` to detect a light background |
| `XDG_CONFIG_HOME` | base for `bb-tui/config.toml`, `servers.json`, `credentials.json` |
| `XDG_STATE_HOME` | base for `bb-tui/drafts/` and `bb-tui/palette-recents.json` |
| `VISUAL`, `EDITOR` | editor for `e` in the right panel |
| `TMUX` | wraps OSC notifications for tmux passthrough |

## Settings file

Settings live in `~/.config/bb-tui/config.toml` (`$XDG_CONFIG_HOME/bb-tui/`).
Use `--config <path>` or `BB_TUI_CONFIG` to point somewhere else. The file is
optional, and every key has a default:

```toml
submit_key = "enter"
theme = "base16-ocean.dark"
appearance = "auto"
mouse = true

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
| `theme` | syntax highlighting theme for code blocks, diffs and previews: `base16-ocean.dark`, `base16-eighties.dark`, `base16-mocha.dark`, `base16-ocean.light`, `InspiredGitHub`, `Solarized (dark)`, `Solarized (light)`. Default `base16-ocean.dark`, or `InspiredGitHub` on a light background |
| `appearance` | `auto` (detect from `COLORFGBG`, else dark), `dark` or `light`. `--appearance` and `BB_TUI_APPEARANCE` override it |
| `mouse` | `true` captures the mouse for clicks and scrolling; `false` leaves it to the terminal. `--no-mouse` overrides it |
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

Other files the TUI writes: unsent drafts in `$XDG_STATE_HOME/bb-tui/drafts/`
(`~/.local/state/bb-tui/drafts`, the web app's draft format) and recently run
palette commands in `$XDG_STATE_HOME/bb-tui/palette-recents.json`.

## Colors and mouse

The UI uses the terminal's 16 named colors plus true-color diff tints and
syntax highlighting.

- `NO_COLOR` (any non-empty value, see no-color.org) removes every
  foreground and background color, including syntax highlighting and terminal
  output. Selections, toasts and banners stay visible as reverse video.
- `appearance = "light"` (or `auto` when `COLORFGBG` reports a light
  background, as rxvt, Konsole and some others do) swaps colors that vanish on
  a light background: white text becomes black, gray becomes dark gray,
  yellow and cyan get darker shades, the unfocused selection and the diff
  tints get light backgrounds, and the default syntax theme becomes
  `InspiredGitHub`. Terminals that do not set `COLORFGBG` need
  `appearance = "light"` or `--appearance light`.

The mouse is captured by default (`mouse = false` or `--no-mouse` turns it off,
which brings back the terminal's own text selection; most terminals also
select with `Shift` held while the mouse is captured):

| Where | Click | Wheel |
|---|---|---|
| Sidebar | a thread opens it; a project header folds it | moves the selection 3 rows |
| Timeline | focuses the pane (another split pane becomes the focused one) and selects the row | scrolls 3 lines; scrolling up stops following new output, scrolling to the end resumes it |
| Composer, request panel | focuses it | |
| Right panel | focuses it | moves 3 rows (Info, Diff, File); scrolls back 3 lines (Terminal) |
| Palette, pickers, settings, automations, dialogs | | moves the selection |

The mouse is released while `$EDITOR` runs and while a terminal is attached
fullscreen.

## Read-only mode

`--read-only` (or `BB_TUI_READ_ONLY=1`) never sends a change to the server.
Use it to look at a production server safely or to demo. Everything that
reads still works: the sidebar, timelines, diffs, file previews, search,
settings, automations, terminals (watched, not typed into) and
notifications. These are refused with a toast and nothing is sent:

- opening a thread does not mark it read; rename, archive, pin, read toggles
- sending, steering, queueing, stopping, editing queued messages
- answering, denying or cancelling requests
- creating threads, uploading attachments
- git actions (commit, merge, ready, draft)
- running plugin commands
- typing into, resizing, creating or closing terminals
- every change in the settings and automations views

A few reads are `POST` requests on the server (the diff patch batch, the
automations plugin's `automations_overview` RPC); they change nothing.

## bb-tui doctor

`bb-tui doctor` checks the connection without opening the UI and exits with
status 1 if a check fails. It only sends `GET` requests and opens (then
closes) the realtime WebSocket without subscribing to anything:

```
Discovery
       --server-url         not set
       BB_SERVER_URL        not set
       app runtime file     /home/me/.bb/bb-app-runtime.json → http://127.0.0.1:38886 (pid 4242 running)
       default              http://127.0.0.1:38886
  ok   using                http://127.0.0.1:38886 (from /home/me/.bb/bb-app-runtime.json)

Server
  ok   GET /health          ok in 2 ms
  ok   GET /system/version  0.45.0

Realtime
  ok   websocket            ws://127.0.0.1:38886/ws upgraded in 1 ms

Settings
       config file          /home/me/.config/bb-tui/config.toml (not created, defaults)
       profiles             0 saved, none active
       colors               dark (auto)
       mouse                captured
```

With `--profile` it reports the profile and checks the bb connect server it
points to; `--server-url`, `--config` and the other flags apply as usual.

## Keys

`?` shows the key list. The table below is the default
keymap of the main screen. Keys with a web command id follow the server's
keyboard overrides (Settings → Keyboard in the web app; `Cmd`-only shortcuts
are skipped), so a key you rebind in the web app is rebound here too. Each
feature section below has the keys that work inside that view.

| Keys | Action | Web command id |
|---|---|---|
| `C-k`, `:` | Command palette | `palette.open` |
| `C-p` | Quick open a file of this thread's workspace | `file.quickOpen` |
| `f` | Search threads (titles and messages) | `thread.search` |
| `P` | Switch project | |
| `j`/`k`, `↓`/`↑` | Move down / up | |
| `g`/`G`, `Home`/`End` | Go to top / bottom | |
| `PgDn`/`C-f`, `PgUp` | Page down / up | |
| `C-d`, `C-u` | Half page down / up | |
| `Enter` | Open thread / expand row | |
| `Space` | Fold / unfold group or row | |
| `h`/`←`, `l`/`→` | Collapse / expand group or row | |
| `]`, `[` | Next / previous user message | |
| `y` | Copy the selected row (OSC 52) | |
| `o` | Open the row's link or file | |
| `/` | Filter the sidebar | |
| `r` | Rename thread | `thread.rename` |
| `a` | Archive / unarchive thread | `thread.archive` |
| `u` | Undo last archive | |
| `m` | Mark read / unread | |
| `p` | Pin / unpin thread | |
| `J`, `K` | Open next / previous thread | `thread.next`, `thread.previous` |
| `M-1` … `M-9` | Open thread #1-9 | `thread.jump.1` … `thread.jump.9` |
| `n` | New thread | `thread.new` |
| `N` | New thread in this thread's environment | |
| `c` | Focus the composer | `composer.focus` |
| `C-o` | Attach a file | |
| `M-a` | Message attachments | |
| `S` | Stop the running turn | |
| `M-q` | Queued messages | |
| `M-t` | Toggle what Enter does while running (queue / steer) | |
| `M-m`, `M-e`, `M-p` | Model, reasoning and permission pickers | `modelPicker.toggle` (model) |
| `M-.`, `M-,` | Next / previous model | `modelPicker.cycleModel`, `modelPicker.cycleModelBackward` |
| `M-r`, `M-R` | Next / previous reasoning level | `modelPicker.cycleReasoning`, `modelPicker.cycleReasoningBackward` |
| `w`, `M-w` | Answer the next waiting request | |
| `!` | Notification center | |
| `Tab`, `S-Tab` | Focus next / previous pane | `pane.focus.next`, `pane.focus.previous` |
| `C-b` | Toggle sidebar | `sidebar.toggle` |
| `i` | Toggle right panel | `panel.toggle` |
| `D` | Show / hide the git diff | `diff.toggle` |
| `>`, `<` | Next / previous panel tab | `panel.nextTab`, `panel.previousTab` |
| `Z` | Maximize / restore the right panel | `panel.fullScreen.toggle` |
| `T` | Thread terminals | `terminal.open` |
| `,` | Settings | `settings.open` |
| `A` | Automations | |
| `V` | Open the selected thread in a split pane | |
| `M-h`/`M-←`, `M-l`/`M-→` | Focus the split pane to the left / right | `pane.focus.left`, `pane.focus.right` |
| — | Focus split pane 1-4 | `pane.focus.1` … `pane.focus.4` |
| `X` | Close the focused split pane | `pane.close` |
| `M-z` | Maximize / restore the focused split pane | `pane.maximize.toggle` |
| `C-r` | Reload data | `window.reload` |
| `Esc` | Back / clear filter / close | `app.back` |
| `?` | Help | |
| `q`, `C-c` | Quit (`C-c` in the composer clears the draft first) | |

Commands without a default key (commit, merge, mark ready, convert to draft,
new terminal, attach terminal fullscreen, new automation, settings sections)
run from the command palette. Inside the right panel `C`, `M`, `R` and `W`
run the git actions.

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
| Command, file change, permission, tool approval | `y` allow once · `s` allow for session · `d` deny · `1-3` by position · `e` show the whole command |
| Plan review | `y` approve plan · `d` keep planning |
| Questions (provider or the ask-user-question plugin) | `1-9` choose (the last number is `Other…`) · `j/k` + `Space` · `Tab`/`←→` switch question · `Enter` next / submit · typing goes to the `Other…` answer |
| Secret request (secrets plugin) | type into the masked field · `Tab`/`↑↓` field · `C-t` show/hide · `Enter` next empty field, then submit |
| Any other plugin request | shown as pretty JSON · `o` open the thread in the web app · `y` copy the link · `C-x` cancel |

`C-x` cancels question, secret and plugin requests (a provider question
stops the turn instead, like the web app); approvals and plan reviews cannot
be cancelled, only answered. `Esc` leaves the panel. As in the web app, a
denial or "keep planning" sends only the decision. Secret values are never
shown unless revealed, are redacted from debug output and cleared after
submission.

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
maximizes it over the timeline. In the panel `1`-`4` pick the Info, Diff,
File and Terminal tabs, `r` reloads, and `Esc` returns from a preview, restores a
maximized panel, then leaves the panel. When the window is too narrow for the
sidebar, the timeline (at least 40 columns) and the panel, the sidebar is hidden
until it gets focus. The keys follow the web app's
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

## Terminals

`T` (the web's `terminal.open`) opens the right panel on the **Terminal** tab
and focuses it. The tab lists the open thread's terminals like the web thread
view: `GET /terminals?threadId=` plus the environment's thread-less terminals
(`GET /terminals?environmentId=`); exited sessions are hidden. The selected
terminal is attached over `/ws/terminals/:id?sinceSeq=&outputAcks=1` with
`bb-client`'s `TerminalConnection` and rendered by a `vt100` emulator into the
panel: 256 and true colors, bold, dim, italic, underline, inverse, wide
characters, the cursor, the alternate screen and 2 000 lines of scrollback.
The program's window title (OSC 0/2) names the tab.

| Keys (Terminal tab) | Action |
|---|---|
| typing | Goes to the terminal while the tab is focused (cursor keys follow application-cursor mode, `Alt` sends `Esc`, F1-F12, paste is bracketed when the program asks for it) |
| `C-\` | Stop typing into the terminal (focus stays on the panel) |
| `S-PgUp`/`S-PgDn` | Scroll back while typing; any key returns to the live screen |
| `Enter`/`i` | Type into the terminal again |
| `j`/`k`, `PgUp`/`PgDn`, `C-u`/`C-d`, `g`/`G` | Scroll back / to the live screen |
| `]`/`[` | Next / previous terminal |
| `n` | Start a terminal in the thread (`POST /terminals`, thread target, the panel's size) |
| `x` | Close the terminal after confirmation (`POST /terminals/:id/close`, `mode: "force"`) |
| `A` | Attach fullscreen |
| `y` | Copy the visible screen |

Flow control follows the web client: the output of each chunk is
acknowledged (`ack` with `nextSeq`) only after the emulator has parsed it, and
the connection reports `visibility: false` while the tab is hidden, so a
hidden TUI never stalls the shell. The panel size is sent as `resize` whenever
it changes; the screen follows the size the server reports (`attached`,
`session-updated`), because only the most recently attached client may resize
a terminal. A dropped socket reconnects with `sinceSeq` set to the next
unseen chunk and the server replays from there; switching terminals keeps
each screen and resumes it the same way. If the server no longer has the
missing output, the screen is cleared with a notice, as in the web app.
Opening another thread closes the sockets.

`A` (or "Attach terminal fullscreen" in the palette) suspends the TUI like
`bb terminal attach`: the alternate screen is left, the terminal's current
screen is repainted, raw stdin bytes are sent as input and output bytes are
written straight to stdout, and the window size is propagated. `C-\` or
`C-b d` detaches; terminal modes the program set (mouse, bracketed paste,
application cursor, alternate screen) are reset and the TUI redraws.

`--read-only` lists and watches terminals but never types, resizes, creates or
closes them.

Not built yet: mouse reporting to the program, selecting text with the mouse,
opening links printed in the terminal, renaming and restarting terminals,
machine (host path) terminals, and environment terminals outside a thread.

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

## Server settings

`,` (the web's `settings.open`, or "Open settings" in the palette) opens the
settings view over the main area. It follows the web settings sections
(`components/settings/settings-sections.ts`) that work without a browser:

| Section | Shows | Changes |
|---|---|---|
| General | server URL, bb version (and the upgrade command when an update is out), data directory, server machine, then the general settings the server reports: followup behavior, Git changes, archive confirmation, history after a context clear, new branch prefix, machine server URL, streamer mode, usage data, diagnostic events | `Enter`/`Space` toggles; text fields open an input. The branch prefix is checked like the server does (at most 64 characters, must start a valid git branch name); the machine server URL must be an http(s) URL without credentials, empty means the server URL |
| Providers | `GET /system/providers/catalog`: enabled providers in the saved order, then disabled ones, with Default / Unavailable / Disabled badges and how finished turns display | `Space` enables or disables (`PUT /system/providers/:id/enabled`), `d` makes it the default, `c` toggles collapsing finished turns, `Enter` lists its models (`GET /system/execution-options?providerId=`); "Allow faster service tiers" toggles |
| Keyboard | every TUI command with its keys and web command id; `/` filters | `Enter` runs the command; rebinding is done in the web app (its overrides apply here) |
| Projects | sidebar projects with remote, how many machines have a checkout, thread count | `Enter` opens the project: name, remote, id, created, checkouts per machine (path or "Not set up", primary, offline), thread defaults (`GET /projects/:id/default-execution-options`). `r` renames (`PATCH /projects/:id`), `D` deletes after confirmation (`DELETE /projects/:id`) |
| Machines | bb version, then each machine with online/offline, server, lifecycle and needs-update badges and when it was last seen | `Enter` opens the machine: id, status, type, machine provider, permission limit, updates, projects with a checkout there, installed provider CLIs and their versions (`GET /hosts/:id/provider-clis/status`, only while online). `r` renames (`PATCH /hosts/:id`), `Enter` on the permission limit picks a new one (`PATCH /hosts/:id/permission-ceiling`) |
| Installed plugins | `GET /plugins`: name, id, version, status badge, whether it has settings; `/` filters | `Space` enables or disables (`POST /plugins/:id/enable` / `disable`). `Enter` opens the plugin; plugins with settings get a form built from `GET /plugins/:id/settings` |
| Archived threads | `GET /threads?archived=true` in pages of 50, grouped by project, `child` badge and when it was archived; `t` cycles all/root/child, `p` cycles the project, `/` searches the loaded titles | `Enter` opens the thread, `u` unarchives, `D`/`x`/`Del` deletes after confirmation (the dialog says how many child threads go too, from `GET /threads/:id/child-summary`); "Load more" fetches the next page |

The plugin settings form keeps the server's field order and renders each
descriptor type: `boolean` (`Enter`/`Space` toggles), `select` and `project`
(a picker; `1`-`9` choose), `number` (must be finite; empty clears it),
`string` (multiline strings are edited on one line with `\n` for a line break
and `\\` for a backslash), and secret strings (masked input, shown as `[set]`
or `[not set]`, an empty input keeps the stored secret, never logged). Each
change is sent alone as `PUT /plugins/:id/settings` with `{ values: { key:
value } }`; a value the plugin rejects shows its error under the field.
Descriptor types the TUI does not know are shown read-only. General settings
are saved like the web app: the whole settings object from `GET
/system/config` with the one change merged in, `PUT /settings/general`.

Keys: `j`/`k`, `PgUp`/`PgDn`, `g`/`G` move; `Tab`, `h`/`←` and `Esc` go to the
section list (`Esc` first closes an open project, machine or plugin and clears
a filter), where `j`/`k` or `1`-`7` pick a section and `Enter`/`l` return to
it (`1`-`7` also switch sections from a section's list); `y` copies ids, paths and URLs; `C-r` reloads the section; `C-k` opens the
palette; `q` or `,` close the view, and so does `Esc` in the section list. A row shows
`[saving…]` until the server answers, and a second change to the same row
waits for it. The palette has entries for each section ("Provider settings",
"Archived threads", …); running any other command from the palette closes the
settings first. `--read-only` refuses every change in the view; it only reads.

Not built yet: appearance, AI services, browser, files, mobile, environment
variables, updates, plugin marketplaces, experiments and community sections;
reordering providers; adding project checkouts on other machines, machine
setup, removal, suspend/resume and update retry; archived-thread search
through `GET /threads/search`; installing and updating plugins; rebinding keys.
Projects have no archive state on the server, so the project view offers
delete (as the web app does) rather than archive.

## Automations

`A` (or "Open automations" in the palette) opens the automations plugin
natively, over the main area. Everything goes through the plugin's RPC
contract (`plugins/automations/src/rpc.ts`) with `POST
/plugins/automations/rpc/:method`:

| View | Shows | Keys |
|---|---|---|
| List | `automations_overview`: every automation sorted by name with a status glyph (`●` active, `○` paused or finished, `◐` running, `✗` last run failed, `!` broken), project, cadence (`9AM Mon-Fri`, `Every 5 min`, a port of the web formatter; unusual expressions show the raw cron), next run or one-shot state, and `[paused]`, `[script]`, `[needs prompt]`, `[invalid]` badges | `j`/`k`, `g`/`G`, `PgUp`/`PgDn` move, `Enter` opens, `/` filters by name, project or cadence, `R`/`C-r` reloads, `Esc`/`q`/`A` close |
| Detail | `automations_get`: status, project, schedule, cron and timezone or one-shot time, last run, run count, last run thread, mode; for agents the provider, model, reasoning, service tier, permission mode, environment (project default, reuse, new worktree, workspace path, existing thread), target thread, origin and the prompt; for scripts the interpreter, timeout, working directory, env var names, script file and script; the last error. Below, `automations_runs` run history (time, status, duration, manual trigger, skip reason, error, exit code, last output line) | `Enter` on a run (or the last/target thread row) opens its thread; `m` or "Load more runs" fetches the next page; `Esc`/`h` back |

In both views `Space` pauses or resumes (`automations_pause` / `resume`; a
finished or expired one-shot cannot be toggled), `r` runs it now
(`automations_run`), `e` edits, `d`/`Del` deletes after confirmation
(`automations_delete`), and `n` opens the create form ("New automation" in the
palette does too).

The form (`automations_create` / `automations_update`) has name, project,
schedule (recurring cron with timezone, or one time), prompt, provider, model,
reasoning, permission mode, environment and "start paused". The cron
expression is checked field by field (five fields, ranges, steps, month and
day names) and its cadence is shown next to it; the timezone defaults to the
local one (`TZ` or `/etc/localtime`); a one-time run accepts `YYYY-MM-DD
HH:MM` local time, an ISO time or a delay like `30m`, `2h`, `1d`, and must be
in the future. Provider, model, reasoning and permission are prefilled from
the project's thread defaults and offered from `GET
/system/providers/catalog` and `GET /system/execution-options?providerId=`.
Environments: project default, a new worktree each run (on a connected
machine), or reusing an environment of one of the project's threads. New
automations are sent with `origin: "human"`. Editing sends only the changed
fields (`name`, `trigger`, and `agent` with the changed prompt, provider,
model, reasoning, permission or environment target); script automations can
change their name and schedule. `j`/`k` move, `Enter`/`Space` edit or choose
(`1`-`9` pick), `C-s` saves, `Esc` discards. The prompt is edited on one line
with `\n` for a line break.

The view refreshes on the plugin's realtime signals (`plugin-signal` on
channel `automations`): `automations-changed` refetches the list and the open
automation of that project, `automation-runs-changed` also its run history.
Refetches are coalesced while one is in flight. `--read-only` refuses every
change with a toast; reading still works.

Not built yet: creating script automations, editing a script's body,
interpreter, timeout or working directory, picking a target thread or an
explicit workspace path, service tiers, status/project filters and sort
modes, run output beyond its last line, and the web's "create via chat"
shortcut.

## Split panes

Several threads can sit side by side in the main area (vertical split, up to
4 panes). Each pane shows its thread's title (numbered), timeline and
composer; only the focused pane takes input, and the others keep following
their threads live (the TUI keeps watching every pane's thread).

| Keys | Action |
|---|---|
| `V` | Open the thread selected in the sidebar in a new pane right of the focused one (focuses it if it is already in a pane) |
| `M-h`/`M-l`, `M-←`/`M-→` | Focus the pane to the left / right (wraps); `pane.focus.left` / `pane.focus.right` |
| `Tab`/`S-Tab` | Cycle sidebar, each pane and its composer (or request panel), right panel |
| `X` | Close the focused pane (`pane.close`); one pane left ends the split |
| `M-z` | Show only the focused pane / restore the split (`pane.maximize.toggle`) |

Opening a thread from the sidebar, search or a notification replaces the
focused pane's thread, or focuses the pane that already shows it. The right
panel, palette and keys act on the focused pane's thread. `pane.focus.1`-`4`
have no default key but follow server keybinding overrides. A background pane
waiting for an answer shows a one-line notice; focus it to answer.

## Coverage compared with the web app

| Area | In the TUI | Missing or different |
|---|---|---|
| Sidebar | projects, Personal, pinned, nesting, unread and waiting markers, filter, rename, archive and undo, pin, read toggles, split panes. Projects with the same name are told apart by their remote (`owner/repo`), then the checkout path tail, then the id. The Personal group is hidden while it has no threads (unless there are no projects), like the web list; empty projects show `no threads` | sections and organization modes (by section, machine), sorting choices, drag and drop, hide/show groups |
| Timeline | Markdown, code highlighting, tables, work/tool/diff rows, turn summaries, lazy loading of older rows and collapsed turns, plan, goal and todo banners | images, Mermaid, KaTeX, inline-vis HTML, message directives (`::task` and others show a short placeholder) |
| Composer | send, queue, steer, stop, prompt history, model/reasoning/permission pickers, queued messages, drafts in the web format, mentions, `/` and `$` commands, attachments by path | voice input, image paste and previews, scheduled send, prompt library, chip labels (chips show their serialized text) |
| Requests | approvals, plan review, provider and plugin questions, secret requests, a JSON fallback for other plugin requests | plugin request renderers (only the generic fallback) |
| New thread | projects, environments, branches, providers, models, reasoning, permissions, reuse of an environment | service tiers, plugin input controls other than worktree/checkout, guided machine setup |
| Palette | commands, thread search, project switcher, quick open, plugin CLI commands | plugin palette commands registered from React, settings and plugin-page entries |
| Right panel | Info, Diff (lazy patches), File preview, Terminal, git commit/merge/ready/draft | merge base choice, file filter, context expansion, image previews, push and create-PR flows (no such server actions) |
| Terminals | list, attach, input, resize, replay, flow control, fullscreen attach | mouse reporting to programs, links, rename/restart, machine terminals |
| Notifications | toast, bell, OSC 9/777 or native desktop notifications, notification center, push-notifications plugin signals | web push |
| Settings | General, Providers, Keyboard (view), Projects, Machines, Installed plugins (with settings forms), Archived threads | appearance, AI services, browser, files, mobile, environment variables, updates, marketplaces, experiments, community; rebinding keys; installing plugins |
| Automations | list, detail, run history, pause/resume, run now, create/edit agent automations, delete | script automations, filters and sort modes |

Intentionally not supported, because they need a browser or a GUI:

- React UI contributed by plugins (nav panels, thread panel actions, settings
  sections, file openers, overlays, themes, icons); plugins work through their
  CLI commands, RPC, mention providers and settings descriptors instead
- the embedded browser, browser automation previews and agent annotations;
  links open in the system browser
- voice input, Monaco editor and PDF preview (files open in `$EDITOR` or the
  system opener)
- drag and drop, the web app's appearance settings, mobile QR pairing,
  desktop app updates and zoom
- side chat, docs, the tasks board, the GitHub browser, the provider usage
  card, the prompt library and drafts UI
- `/internal/*` routes and the host daemon protocol

Like the web app, denying an approval or keeping a plan in planning sends only
the decision (`POST /threads/:id/interactions/:id/resolve` with
`{ decision: "deny" }`); there is no reason or feedback field. Type a follow-up
in the composer afterwards if you want to explain.

## Development

- `crates/bb-wire` — serde mirrors of the server contract
- `crates/bb-client` — REST, realtime WebSocket, connect auth, discovery
- `crates/bb-store` — cache, invalidation, timeline merge
- `crates/bb-view` — view-model and formatting ported from `packages/thread-view`
- `crates/bb-tui` — the `bb-tui` binary: an Elm-style `update(state, action) ->
  effects` loop in `app.rs`, rendering in `*_ui.rs`, effects executed by
  `runtime.rs` through the `Api` and `Platform` traits so tests run against
  fake backends

```sh
cd apps/tui
cargo run -- --server-url http://127.0.0.1:38886 --read-only
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

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
screens). `renders_live_settings_read_only` walks every settings section and
`lists_live_thread_terminals_read_only` lists a thread's terminals; `bb-tui doctor`
against the live server is covered by `doctor_live_read_only`.
