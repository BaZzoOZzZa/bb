# bb-tui evil keymap

This file is the single source of truth for bb-tui keys. Navigation follows
Doom Emacs with evil-mode: modal states, a `SPC` leader with a which-key popup,
`C-w` window commands, evil motions in every buffer and `:` ex commands. Code,
tests, the in-app help (`SPC h b b`), the palette's key column and the README
follow this file; when they disagree, this file wins and the code is fixed.

Contents:

1. [Principles](#1-principles)
2. [States and the modeline](#2-states-and-the-modeline)
3. [Buffers and windows](#3-buffers-and-windows)
4. [Global keys](#4-global-keys)
5. [Leader map (`SPC`)](#5-leader-map-spc)
6. [Window commands (`C-w`)](#6-window-commands-c-w)
7. [Motions, counts, search, marks, scrolling](#7-motions-counts-search-marks-scrolling)
8. [Sidebar buffer](#8-sidebar-buffer)
9. [Timeline buffer](#9-timeline-buffer)
10. [Composer buffer](#10-composer-buffer)
11. [Registers and the clipboard](#11-registers-and-the-clipboard)
12. [Minibuffers: palette, pickers, ex line](#12-minibuffers-palette-pickers-ex-line)
13. [Ex commands](#13-ex-commands)
14. [Right panel, terminals and special views](#14-right-panel-terminals-and-special-views)
15. [Which-key](#15-which-key)
16. [Server keybinding overrides](#16-server-keybinding-overrides)
17. [Mouse](#17-mouse)
18. [Russian layout](#18-russian-layout)
19. [Migration from the previous keymap](#19-migration-from-the-previous-keymap)
20. [Conflicts and resolutions](#20-conflicts-and-resolutions)
21. [Out of scope for the first cut](#21-out-of-scope-for-the-first-cut)
22. [Implementation notes](#22-implementation-notes)
23. [Implementation status](#23-implementation-status)

Notation: `C-x` is Ctrl+x, `M-x` is Alt+x, `S-x` is Shift+x, `RET` Enter,
`ESC` Escape, `TAB` Tab, `SPC` Space. `a b` is a key sequence. `{N}` is an
optional count, `{motion}` any motion of section 7, `{char}` any character,
`{reg}` a register name.

## 1. Principles

- Doom defaults first. A key does what it does in Doom Emacs with `(evil
  +everywhere)` and evil-collection, mapped onto the nearest bb concept:
  threads are buffers, split panes are windows, right-panel tabs are tabs,
  projects are workspaces, the command palette is `M-x`, file and thread
  pickers are vertico minibuffers.
- The user's Doom config (`~/.config/doom`) wins over stock Doom where it
  customises a key that has a bb equivalent: `C-h`/`C-j`/`C-k`/`C-l` move
  between windows in normal, visual and motion states; `SPC a` is the agent
  prefix with the user's Claude keys; `SPC 1`..`SPC 9` select windows
  (`window-select +numbers`); `SPC h ,` opens settings; `C-;` acts on the
  minibuffer candidate (embark); the evil state tag sits at the right end of
  the status line in mini-echo style; Cyrillic keys in normal state act as
  their Latin positions (`reverse-im`).
- No single letter performs a destructive or external action outside a
  dedicated special view. Quitting needs `SPC q q`, `:q` or `ZZ`/`ZQ`.
- Every command stays reachable three ways: a key (direct or sequence), the
  palette (`SPC :`) and, for commands with an argument or a mutation, an ex
  command. `--read-only` refuses mutations on every path with the same toast.
- Server keybinding overrides keep working for the web command ids the TUI
  maps (section 16).
- The mouse keeps working (section 17).

## 2. States and the modeline

| State | Tag | Color | Where | Entered by | Left by |
|---|---|---|---|---|---|
| normal | `<N>` | green | every buffer and special view | `ESC`, `C-[`, startup | an insert, visual or ex key |
| insert | `<I>` | blue | composer, new-thread prompt, terminal tab while typing | `i a I A o O s S c C gi`, `i`/`a`/`A`/`I`/`o`/`O` from the timeline, a mouse click in the composer | `ESC`, `C-[`; `C-\` in a terminal |
| visual | `<V>` | magenta | composer, timeline, sidebar, panel line views | `v`, `gv` | `ESC`, `v`, an operator |
| visual-line | `<L>` | magenta | same | `V` | `ESC`, `V`, an operator |
| operator-pending | `<O>` | cyan | composer (and `y`/`d` in read-only buffers) | `d c y > < gu gU g~ g? =` | a motion, a text object, `ESC` |
| replace | `<R>` | red | composer | `R` | `ESC` |
| command-line | `<E>` | yellow | ex line, search line | `:`, `/`, `?` | `RET`, `ESC`, `C-g`, `C-c` |

The tag is shown at the right end of the status bar (mini-echo layout: the one
place that never moves), bold, on the state's color derived from the theme
palette. Left of the tag the status bar shows, in order, the pending keys
(`showcmd`: count, register and the incomplete sequence, for example `"a2d`
or `SPC b`), the search match index (`[3/17]`) while a search is
highlighted, then the existing status items (waiting requests, notifications,
read-only marker, server). Messages (`E492: Not an editor command: foo`,
`search hit BOTTOM, continuing at TOP`, `SPC x y is undefined`) appear in the
echo area, the line under the status bar used by the ex and search lines,
until the next key. With no message and no ex or search line, the echo area
shows short key hints for the focused buffer in a dim color (the old status
bar hints moved there).

The cursor shape follows the state when the terminal supports DECSCUSR: block
in normal, visual and operator-pending, bar in insert and command-line,
underline in replace. Read-only buffers (sidebar, timeline, panel views) keep
a line highlight plus a block cursor at the cursor column.

## 3. Buffers and windows

| bb thing | Doom concept | Notes |
|---|---|---|
| a thread | buffer | `SPC ,` / `SPC b b` switches, `]b`/`[b` cycles, `SPC b k` archives ("kill") |
| the open thread's timeline | buffer contents | a read-only evil buffer over rendered lines |
| the composer under a timeline | an editable window below the timeline | modal (normal / insert / visual) |
| a split pane (up to 4) | window | `C-w` and `SPC w`; each pane has a timeline window and a composer window |
| the sidebar | treemacs-like side window, left | `SPC o p` toggles, `C-h` / `C-w h` reaches it |
| the right panel | side window, right | `SPC o i` toggles; its Info, Diff, File and Terminal tabs are tabs (`gt`/`gT`) |
| projects | workspaces | `SPC p p`, `SPC TAB TAB` |
| the command palette | `M-x` | `SPC :`, `M-x` |
| thread search, quick open, project switch, pickers | vertico minibuffers | section 12 |
| settings, automations, notifications, help, plugin output, file preview | special-mode buffers (evil-collection) | section 14 |
| the request panel, confirm dialogs, transient pickers | transients (magit popups) | own single-key maps, section 14 |

Window geometry for `C-w h/j/k/l` and `C-h/j/k/l`: sidebar on the left, the
panes left to right, each pane's timeline above its composer (or request
panel), the right panel on the right. Moving off an edge does nothing (no
wrap), except `C-w w`/`C-w W`, which cycle through every window in the order
sidebar, pane 1 timeline, pane 1 composer, pane 2 timeline, ..., right panel.
A hidden sidebar or panel is skipped; `C-w h` from the leftmost pane when
the sidebar is hidden only because the window is too narrow shows it (the old
"hidden until it gets focus" rule).

## 4. Global keys

Active in normal state of every buffer and special view, unless the buffer
section rebinds them.

| Keys | Action |
|---|---|
| `SPC` | Leader (section 5) |
| `M-SPC` | Leader from any state, including insert, minibuffers and transients |
| `SPC m`, `M-SPC m` | Local leader: actions of the focused thread (section 5) |
| `:` | Ex command line (section 13) |
| `M-x` | Command palette |
| `C-h` `C-j` `C-k` `C-l` | Focus the window left / below / above / right |
| `C-w` | Window prefix (section 6) |
| `M-h`/`M-←`, `M-l`/`M-→` | Focus the split pane left / right, wrapping (`pane.focus.left/right`) |
| `M-z` | Maximize / restore the focused window (`pane.maximize.toggle`) |
| `M-1` .. `M-9` | Open thread #1-9 (`thread.jump.N`) |
| `M-m`, `M-e`, `M-p` | Model, reasoning and permission pickers |
| `M-.`, `M-,` | Next / previous model |
| `M-r`, `M-R` | Next / previous reasoning level |
| `M-q` | Queued messages |
| `M-t` | Toggle what sending does while a turn runs (queue / steer) |
| `M-w` | Open the next thread waiting for an answer |
| `M-a` | Message attachments |
| `ESC`, `C-[` | Keyboard quit: cancel a count, register, operator or prefix; clear the search highlight; close the which-key popup, a minibuffer, the help or notification view; in a special view go back one level (`app.back`) |
| `C-g` | Same as `ESC` (Emacs keyboard-quit) |
| `F1` | Bindings of the focused buffer (`SPC h b b`) |
| `ZZ` | `:x` |
| `ZQ` | `:q!` |
| `C-c` | Prefix only: `C-c C-c` send, `C-c C-s` steer, `C-c C-k` stop the turn, `C-c C-a` attach a file. Never quits |
| `q` | Closes special views and popups; in the sidebar, timeline and composer it is unbound and the echo area says `Quit with SPC q q or :q` |

## 5. Leader map (`SPC`)

Group names are what which-key shows (`+buffer`). Keys marked "Doom" keep
the stock Doom meaning mapped to bb; "user" comes from `~/.config/doom`.

### Top level

| Keys | Action | Origin |
|---|---|---|
| `SPC SPC` | Quick open a file of the focused thread's workspace (`file.quickOpen`) | Doom find file in project |
| `SPC .` | Quick open a file (same picker, starting at the workspace root) | Doom find file |
| `SPC ,` | Switch thread within the focused project | Doom switch workspace buffer |
| `SPC <` | Switch thread, all projects | Doom switch buffer |
| `SPC /` | Search threads, titles and messages (`thread.search`) | Doom search project |
| `SPC :` | Command palette (`palette.open`) | Doom M-x |
| `SPC ;` | Ex command line | Doom eval expression |
| `SPC '` | Reopen the last minibuffer with its mode and query | Doom resume last search |
| `` SPC ` `` | Alternate thread (the previously open one) | Doom last buffer |
| `SPC 1` .. `SPC 4` | Focus split pane 1-4 (`pane.focus.N`) | user (window-select +numbers) |
| `SPC x` | New thread (scratch) | Doom scratch buffer |
| `SPC a` | +agent | user |
| `SPC b` | +buffer (threads) | Doom |
| `SPC f` | +file | Doom |
| `SPC g` | +git | Doom |
| `SPC h` | +help | Doom |
| `SPC m` | +thread (local leader) | Doom |
| `SPC n` | +notifications | bb |
| `SPC o` | +open | Doom |
| `SPC p` | +project | Doom |
| `SPC q` | +quit | Doom |
| `SPC s` | +search | Doom |
| `SPC t` | +toggle | Doom |
| `SPC w` | +window (same as `C-w`) | Doom |
| `SPC TAB` | +workspace (projects and thread slots) | Doom |

### `SPC a` +agent (user's Claude keys)

| Keys | Action |
|---|---|
| `SPC a a` | New thread (start an agent) |
| `SPC a t` | Toggle the composer: focus it in insert state, or return to the timeline |
| `SPC a c` | Continue: focus the composer of the open thread in insert state |
| `SPC a r` | Resume: switch thread (`SPC <`) |
| `SPC a p` | Send the composer draft (`:w`) |
| `SPC a m` | Mention a file: file picker, inserts an `@` mention at the composer cursor |
| `SPC a l` | Threads list (palette in Threads mode, most recent first) |
| `SPC a k` | Status: right panel on the Info tab |
| `SPC a q` | Stop the running turn |
| `SPC a .` | Command palette filtered to the Thread group |

### `SPC b` +buffer (threads)

| Keys | Action |
|---|---|
| `SPC b b` | Switch thread within the project |
| `SPC b B` | Switch thread, all projects |
| `SPC b n`, `SPC b ]` | Next thread (`thread.next`) |
| `SPC b p`, `SPC b [` | Previous thread (`thread.previous`) |
| `SPC b l` | Alternate thread |
| `SPC b N` | New thread (`thread.new`) |
| `SPC b c` | New thread in this thread's environment (clone) |
| `SPC b k`, `SPC b d` | Archive / unarchive the thread (`thread.archive`) |
| `SPC b u` | Undo the last archive |
| `SPC b R` | Rename the thread (`thread.rename`) |
| `SPC b r` | Reload data (`window.reload`) |
| `SPC b m` | Pin / unpin the thread (bookmark) |
| `SPC b !` | Mark read / unread |
| `SPC b y` | Yank the loaded timeline as plain text |
| `SPC b i` | Thread list (palette Threads mode) |

### `SPC f` +file

| Keys | Action |
|---|---|
| `SPC f f` | Quick open a file |
| `SPC f r` | Recent files of this session |
| `SPC f y` | Yank the path of the previewed file, or the selected file row |
| `SPC f e` | Open the previewed or selected file in `$VISUAL`/`$EDITOR` |
| `SPC f a` | Attach a file to the message (path prompt) |
| `SPC f A` | Message attachments |

### `SPC g` +git

| Keys | Action |
|---|---|
| `SPC g g` | Show / hide the git diff panel (`diff.toggle`) |
| `SPC g c c` | Commit uncommitted changes (confirm) |
| `SPC g p m` | Merge the pull request (confirm, then pick squash / merge / rebase) |
| `SPC g p r` | Mark the draft pull request ready (confirm) |
| `SPC g p d` | Convert the pull request to draft (confirm) |
| `SPC g o P` | Open the pull request in the browser (user: browse pull requests) |
| `SPC g ]`, `SPC g [` | Next / previous hunk in the diff panel (Doom: jump to hunk) |

### `SPC h` +help

| Keys | Action |
|---|---|
| `SPC h b b` | Bindings of the focused buffer and state, then the global ones |
| `SPC h k` | Describe key: the next key sequence shows its command instead of running it |
| `SPC h x` | Describe an ex command |
| `SPC h e` | Message history (`:messages`) |
| `SPC h ,` | Settings (`settings.open`) (user) |
| `SPC h ?` | This keymap overview, grouped like this file |

### `SPC m` +thread (local leader)

Acts on the focused pane's thread; in the sidebar, on the selected thread.

| Keys | Action |
|---|---|
| `SPC m s` | Send the draft (`:w`) |
| `SPC m S` | Steer the running turn (`:w!`) |
| `SPC m k` | Stop the running turn |
| `SPC m q` | Queued messages |
| `SPC m r` | Answer the next waiting request |
| `SPC m m` | Model picker (`modelPicker.toggle`) |
| `SPC m e` | Reasoning picker |
| `SPC m p` | Permission picker |
| `SPC m a` | Attach a file |
| `SPC m A` | Attachments |
| `SPC m n` | New thread in this thread's environment |
| `SPC m i` | Right panel on the Info tab |

### `SPC n` +notifications

| Keys | Action |
|---|---|
| `SPC n n` | Notification center |
| `SPC n r` | Open the next thread waiting for an answer, with its request panel focused |
| `SPC n c` | Clear all notifications |

### `SPC o` +open

| Keys | Action |
|---|---|
| `SPC o p` | Toggle the sidebar (`sidebar.toggle`) (Doom project sidebar) |
| `SPC o P` | Reveal the open thread in the sidebar and focus it |
| `SPC o i` | Toggle the right panel (`panel.toggle`) |
| `SPC o d` | Right panel on the Diff tab (`diff.toggle`) |
| `SPC o t` | Thread terminals (`terminal.open`) (Doom/user: terminal) |
| `SPC o T` | Start a new terminal (Doom: terminal here) |
| `SPC o f` | Attach the terminal fullscreen (Doom: frame) |
| `SPC o A` | Automations |
| `SPC o N` | New automation |
| `SPC o x` | Open the link or file under the cursor externally (`gx`) |

### `SPC p` +project

| Keys | Action |
|---|---|
| `SPC p p` | Switch project |
| `SPC p f` | Quick open a file of the project (project paths when no thread is open) |
| `SPC p n` | New thread in the project |

### `SPC q` +quit

| Keys | Action |
|---|---|
| `SPC q q` | Quit bb-tui |
| `SPC q Q` | Quit bb-tui (same; kept for Doom muscle memory) |
| `SPC q r` | Reload data (Doom: restart) |

### `SPC s` +search

| Keys | Action |
|---|---|
| `SPC s s`, `SPC s b` | Search the focused buffer (`/`) |
| `SPC s p` | Search threads (titles and messages) |
| `SPC s f` | Narrow the sidebar or the special view's list to a filter (`g/`) |
| `SPC s i` | Jump to a user message of this thread (imenu over user messages) |
| `SPC s r` | Jump to a mark (`:marks` picker) |
| `SPC s c` | Clear the search highlight (`:noh`) |

### `SPC t` +toggle

| Keys | Action |
|---|---|
| `SPC t s` | Sidebar |
| `SPC t p` | Right panel |
| `SPC t q` | What sending does while a turn runs: queue / steer |
| `SPC t l` | Relative line numbers in the timeline and composer |
| `SPC t h` | Search highlight (`hlsearch`) |
| `SPC t f` | Follow new output in the timeline |
| `SPC t z` | Zen: hide sidebar and panel, maximize the focused pane |

### `SPC w` +window

Every `C-w` key of section 6 also works as `SPC w` key (`SPC w v`), plus:

| Keys | Action |
|---|---|
| `SPC w m m` | Maximize / restore the focused window (Doom) |
| `SPC w d` | Close the focused window (Doom) |

### `SPC TAB` +workspace

| Keys | Action |
|---|---|
| `SPC TAB TAB`, `SPC TAB .` | Switch project |
| `SPC TAB 1` .. `SPC TAB 9` | Open thread #1-9 (same as `M-1`..`M-9`) |
| `SPC TAB ]`, `SPC TAB [` | Next / previous thread |
| `SPC TAB n` | New thread |
| `SPC TAB d` | Archive the thread |

## 6. Window commands (`C-w`)

| Keys | Action | Web command id |
|---|---|---|
| `C-w h`/`j`/`k`/`l` (also `C-w C-h` .. and arrows) | Focus the window in that direction | `pane.focus.left/right` for `h`/`l` between panes |
| `C-w w`, `C-w C-w` | Next window | `pane.focus.next` |
| `C-w W` | Previous window | `pane.focus.previous` |
| `C-w p` | Previously focused window | |
| `C-w t`, `C-w b` | Top-left window (sidebar or pane 1), bottom-right window | |
| `C-w v`, `C-w s` | Open the thread under the cursor (sidebar or palette) or the focused thread in a new split right of the focused pane (`s` gives the same vertical split; panes are always side by side) | |
| `C-w n` | New thread in a new split | |
| `C-w q`, `C-w c` | Close the focused window: a split pane (one pane left ends the split), the right panel, the sidebar (hides it), a special view. On the last window it echoes `E444: Cannot close last window` instead of quitting (quitting stays explicit: `:q`, `ZQ`, `SPC q q`) | `pane.close` |
| `C-w o` | Only: maximize the focused pane or the right panel, again restores. Nothing is closed | `pane.maximize.toggle`, `panel.fullScreen.toggle` |
| `C-w =` | Restore all windows (unmaximize, show the sidebar if it was hidden by `C-w o`) | |
| `C-w x`, `C-w r` | Swap the focused pane with the next one / rotate panes | |
| `C-w H`, `C-w L` | Move the focused pane to the far left / right | |
| `{N}C-w w` | Focus pane N | `pane.focus.N` |
| `C-w <`, `C-w >` | Narrow / widen the sidebar or panel by a count of columns (default 4) | |

## 7. Motions, counts, search, marks, scrolling

These apply to every buffer with a cursor: sidebar, timeline, composer,
panel views, special-view lists, terminal scrollback. Read-only buffers apply
them to rendered lines; the sidebar and list views only have lines, so
character motions are no-ops there.

### Counts

`{N}` before a motion or command repeats it (`5j`, `3]m`, `2dd`). A count
typed after an operator multiplies the first one (`2d3w` deletes six words).
`{N}G` and `:{N}` go to line N. `{N}%` goes to N percent. A count before a leader
key is ignored. The pending count shows in the status bar.

### Motions

| Keys | Motion |
|---|---|
| `h` `j` `k` `l`, arrows | Left, down, up, right |
| `gj` `gk` | Down / up one display line of a wrapped line (the composer moves by logical lines, because its wrapping is only known while drawing) |
| `w` `b` `e` `ge` / `W` `B` `E` `gE` | Word / WORD forward, back, end, end back |
| `0` `^` `$` `g_` | Line start, first non-blank, end, last non-blank |
| `gg` `G` | First line, last line (`G` in the timeline also resumes following) |
| `H` `M` `L` | Top, middle, bottom of the window |
| `f{char}` `F{char}` `t{char}` `T{char}` | Find / till a character in the line |
| `;` `,` | Repeat the last find forward / backward |
| `%` | Matching bracket or quote |
| `{` `}` | Previous / next paragraph (timeline: previous / next row) |
| `(` `)` | Previous / next sentence |
| `'{mark}` `` `{mark} `` | Line / position of a mark |
| `''` ` `` ` | Back to the position before the last jump |
| `C-o` `C-i` | Jump list back / forward (`C-i` only where the terminal tells it apart from `TAB`, kitty keyboard protocol) |
| `n` `N` | Next / previous search match |
| `*` `#` | Search the word under the cursor forward / backward |

### Search

| Keys | Action |
|---|---|
| `/{pattern}RET` | Search forward |
| `?{pattern}RET` | Search backward |
| `n`, `N` | Repeat in the same / opposite direction |
| `*`, `#`, `g*`, `g#` | Word under the cursor, whole-word / partial |
| `:noh`, `ESC` in normal state | Clear the highlight |

Search is incremental (the view moves to the first match while typing) and
smart-case (case-insensitive unless the pattern has an uppercase letter;
`\c`/`\C` force it). Patterns are regular expressions (the `regex` crate
syntax, very-magic like evil's default). All matches in the visible rows are
highlighted until `:noh`, `ESC` or a new search (`:set nohlsearch` hides,
`:set hlsearch` shows them again). Search wraps around with
`search hit BOTTOM, continuing at TOP`; `:set nowrapscan` disables wrapping.
In the timeline search covers the rendered text of loaded rows: expanded rows
fully, collapsed rows by their title line. A match inside a collapsed row's
deferred content is not found until the row is expanded. The sidebar
searches row labels (thread titles, project and section names), including
threads inside collapsed groups, which open to show the match. The search
line has history (`↑`/`↓`, `C-p`/`C-n`) and the minibuffer editing keys of
section 12.

### Marks

| Keys | Action |
|---|---|
| `m{a-z}` | Set a buffer-local mark at the cursor |
| `m{A-Z}` | Set a global mark: thread, row and position |
| `'{a-z}`, `` `{a-z} `` | Jump to the mark's line / position |
| `'{A-Z}` | Open the mark's thread in the focused pane and jump to the position |
| `''`, ` `` ` | Jump back |
| `'[` `']` `'<` `'>` `'.` `'^` | Last change start/end, last visual selection start/end, last change, last insert (composer) |
| `:marks` | List marks in a picker; `RET` jumps |
| `:delm {marks}`, `:delm!` | Delete marks / all lowercase marks of the buffer |

Timeline marks are anchored to a timeline row id plus a line offset inside the
row, so they survive reloads, newly loaded older rows and expanding rows
above them. Sidebar marks are anchored to the thread or group id. Global marks
persist in `$XDG_STATE_HOME/bb-tui/marks.json` (thread id, row id, offset);
buffer-local marks live for the session. Jumps (`G`, `gg`, searches, marks,
`]m`, `%`, `{N}G`, opening a thread) push the jump list (`C-o`/`C-i`).

### Scrolling

| Keys | Action |
|---|---|
| `C-d`, `C-u` | Half page down / up, cursor moves with the view |
| `C-f`, `C-b`, `PgDn`, `PgUp` | Page down / up |
| `C-e`, `C-y` | Scroll one line, cursor stays when possible |
| `zz`, `zt`, `zb` | Cursor line to the middle / top / bottom of the window |
| `z RET`, `z.`, `z-` | Same, cursor to the first non-blank |
| `Home`, `End` | First / last line |

Scrolling up in the timeline stops following new output; `G`, `End` or
`SPC t f` resume it. Reaching the top loads older rows as before.

### Folding

| Keys | Action |
|---|---|
| `TAB`, `za` | Toggle the fold at the cursor (group, row, diff file) |
| `zo`, `zc` | Open / close |
| `zO`, `zC` | Open / close recursively |
| `zR`, `zM` | Open / close all |
| `zj`, `zk` | Next / previous fold start |

## 8. Sidebar buffer

The sidebar is a normal-state, read-only, line buffer (treemacs-evil and
evil-collection dired conventions). Motions, counts, search, marks, scrolling
and folding work as in section 7.

| Keys | Action | Web command id |
|---|---|---|
| `RET` | Open the thread (focus stays) / toggle a group | |
| `l`, `→` | Expand a group or row; on a thread, open it | |
| `h`, `←` | Collapse; on a collapsed or leaf row, go to the parent | |
| `go` | Open the thread and focus its timeline | |
| `i`, `a` | Open the thread and focus its composer in insert state | `composer.focus` |
| `o` | New thread in the selected project | `thread.new` |
| `O` | New thread in the selected thread's environment | |
| `C-w v`, `C-w s` | Open the selected thread in a split pane | |
| `R` | Rename | `thread.rename` |
| `dd`, `x`, `Del` | Archive / unarchive (asks when the server's archive confirmation is on) | `thread.archive` |
| `u` | Undo the last archive | |
| `!` | Mark read / unread | |
| `+` | Pin / unpin | |
| `yy` | Yank the thread title (`gy`: the thread id) | |
| `g/` | Narrow to a filter (in-place filter line; `ESC` clears it) | |
| `gr` | Reload | `window.reload` |
| `]b`, `[b` | Open the next / previous thread | `thread.next`, `thread.previous` |
| `]w`, `[w` | Move to the next / previous thread waiting for an answer | |
| `]u`, `[u` | Move to the next / previous unread thread | |
| `V` then motions | Select rows; `d` archives them (one confirmation for all), `y` yanks their titles, `!` marks them read, `+` pins them | |

`gv` keeps its evil meaning (reselect the last visual selection).

## 9. Timeline buffer

The timeline is a normal-state, read-only buffer over its rendered lines with
a line and column cursor. Each timeline row (message, work row, turn summary)
spans one or more rendered lines; the row under the cursor is highlighted as
the old selection was, so row actions apply to it.

| Keys | Action |
|---|---|
| motions of section 7 | Move over rendered lines and characters |
| `{`, `}` | Previous / next row |
| `]m`, `[m` | Next / previous user message |
| `]]`, `[[` | Next / previous turn |
| `]e`, `[e` | Next / previous error or failed tool row |
| `]b`, `[b` | Next / previous thread |
| `]w`, `[w` | Next / previous waiting thread |
| `RET`, `TAB`, `za` | Toggle the row (expand / collapse); on a collapsed turn, load and expand it |
| `zo`, `zc`, `zO`, `zC`, `zR`, `zM` | Folds of section 7 applied to rows |
| `v`, `V` | Visual / visual-line selection over rendered text |
| `y{motion}`, `yy`, `Y` (= `y$`) | Yank rendered text (plain, no styling) |
| `gy` | Yank the row's source: the message Markdown, the command, the tool input, the patch (the old `y`) |
| `gx` | Open the link or file at the cursor, else the row's links (picker when several) |
| `gf` | Preview the file under the cursor in the right panel |
| `i`, `a`, `I`, `A`, `o`, `O` | Focus the composer in insert state (cursor at the end of the draft; `I` at its start) |
| `gi` | Focus the composer in insert state where insert was last left |
| `gr` | Reload |
| `/`, `?`, `n`, `N`, `*`, `#` | Search (section 7) |

Text objects usable after `y` and in visual state:

| Object | Inner (`i`) | Around (`a`) |
|---|---|---|
| `w`, `W` | word / WORD | with trailing space |
| `s` | sentence | with trailing space |
| `p` | rendered paragraph | with trailing blank line |
| `m` | the row's rendered body | the row including its header line |
| `` ` `` | inline code or the fenced code block content | with the fences |
| `"`, `'`, `(`, `[`, `{`, `<` | inside the pair | including the pair |

`yi`` copies the code block under the cursor.

## 10. Composer buffer

The composer (and the new-thread prompt, the queued-message editor and the
ex line's multi-line arguments) is a modal editor. It starts in insert state
when focused by an insert key, a click or `SPC a c`; `C-h/j/k/l` and `C-w`
focus keep the current state, defaulting to normal.

### Sending

| State | Keys | Action |
|---|---|---|
| normal | `RET` | Send (queue while a turn runs) |
| normal | `:w`, `SPC m s`, `SPC a p` | Send |
| normal | `:w!`, `SPC m S` | Steer the running turn; with an empty draft, send the first queued message as steer |
| any | `C-c C-c`, `C-RET`, `M-RET` | Send |
| any | `C-c C-s` | Steer |
| any | `C-c C-k` | Stop the running turn (the old `Esc Esc`) |
| insert | `RET` | New line (default `submit_key = "ctrl-enter"`); send when `submit_key = "enter"` |
| insert | `S-RET`, `C-j` | New line |

After a send the composer returns to insert state when the send started there,
otherwise stays in normal state. `M-t` and `SPC t q` swap queue and steer for
the send keys while a turn runs, as before.

### Insert state

| Keys | Action |
|---|---|
| `ESC`, `C-[` | Normal state (cursor moves one left, like vim) |
| `C-w` | Delete the word before the cursor |
| `C-u` | Delete to the start of the line (to the indentation first) |
| `C-h`, `Bksp` | Delete the character before the cursor |
| `C-a`, `C-e` | Line start (first non-blank) / end (Doom insert bindings) |
| `C-k` | Delete to the end of the line |
| `C-r {reg}` | Insert a register's contents |
| `C-o` | Run one normal-state command, then return to insert |
| `C-n`, `C-p` | Next / previous completion candidate while the mention popup is open; otherwise next / previous prompt history entry |
| `C-j`, `C-k` while the mention popup is open | Next / previous candidate (vertico/corfu style) |
| `TAB`, `S-TAB` | Accept / cycle the mention completion; otherwise insert a tab (in the new-thread prompt `TAB`/`S-TAB` keep moving between the form fields) |
| `↑`, `↓` on the first / last line | Prompt history |
| `C-c ...` | Send, steer, stop, attach (above; `C-c C-a` attaches a file) |
| `C-v` | Insert the next key literally |

Bracketed paste inserts as one change. In insert state it joins the insert
session; in normal state it inserts at the cursor as one undo step and leaves
the cursor on the last pasted character; in visual state it replaces the
selection. The pasted text also becomes the `"+` register (section 11).

### Normal state

| Keys | Action |
|---|---|
| motions of section 7 | Move |
| `i` `a` `I` `A` `o` `O` | Insert before / after the cursor, at line start / end, on a new line below / above |
| `gi` | Insert where insert was last left |
| `x`, `X` | Delete the character under / before the cursor |
| `s`, `S` | Substitute character / line |
| `r{char}` | Replace the character |
| `R` | Replace state: typed characters overwrite, `Bksp` restores the overwritten character, `RET` inserts a line break, `ESC` returns to normal |
| `~` | Toggle case and advance |
| `J`, `gJ` | Join lines with / without a space |
| `d{motion}`, `dd`, `D` | Delete |
| `c{motion}`, `cc`, `C` | Change |
| `y{motion}`, `yy`, `Y` | Yank (`Y` = `y$`, Doom `evil-want-Y-yank-to-eol`) |
| `>{motion}`, `>>`, `<{motion}`, `<<` | Indent / outdent by two spaces |
| `gu{motion}`, `gU{motion}`, `g~{motion}` (`guu`, `gUU`, `g~~`) | Lower, upper, toggle case |
| `p`, `P` | Put after / before; linewise registers put on a new line |
| `u`, `C-r` | Undo, redo |
| `U` | Undo all changes on the last changed line |
| `.` | Repeat the last change, including the text of an insert session, with a new count if given |
| `m{a-z}` | Set a composer mark |
| `'{mark}`, `` `{mark} `` | Go to a mark's line / position; also after an operator (`d'a`, `` y`a ``) |
| `"{reg}` | Use a register for the next delete, yank or put |
| `v`, `V`, `gv` | Visual, visual-line, reselect |
| `C-p`, `C-n` | Previous / next prompt history entry (comint style) |
| `RET` | Send |
| `C-k` | Focus the timeline above (window up) |

Text objects after `d c y > < gu gU g~` and in visual state: `iw aw iW aW is
as ip ap i" a" i' a' i` a` i( a( ib ab i[ a[ i{ a{ iB aB i< a<`. `(`/`)` are
the same as `b`, `{`/`}` the same as `B`.

Undo granularity: each normal-state command is one undo step; an insert
session from entering insert to `ESC` is one step (a send ends it). Undo and
redo also restore the composer's marks. `U` restores the last changed line to
how it was before the run of changes on it, as a new undoable change; a second
`U` swaps back. It refuses when lines were added or removed since.
Repeat (`.`) replays the last change command with its motion or text object
and, for `c`/`s`/`i`/`a`/`o`/`R`, the inserted text; a count given to `.`
replaces the original one (for `i`/`a`/`A`/`R` it repeats the text). Visual
operations repeat on the same number of characters (selections within one
line) or lines; multi-line characterwise selections and visual `p` are not
repeated.

Composer marks hold character positions and move with edits before them; a
mark inside deleted text moves to the start of the deletion. Besides `a`-`z`
the composer keeps `'[`/`']` (start / end of the last change), `'<`/`'>`
(last visual selection, also used by `gv`), `'.` (last change), `'^` (where
insert was left) and `''` (the position before the last jump: `gg`, `G`,
`%`, `{N}%`, `(`, `)`, `{`, `}`, mark jumps).

Character motions and `x X r ~ a s` treat a grapheme cluster (a letter with
combining marks, a ZWJ emoji sequence, a flag) as one character. Word motions
use vim's classes on Unicode: letters and digits of any script form words,
punctuation and emoji form their own runs.

Mention chips are atomic: a motion never lands inside a chip, a chip is one
WORD for `w`/`W`, and `x` or any delete over part of a chip removes the whole
chip. A yank over part of a chip yanks the whole chip text. Mention offsets
stay UTF-16 offsets into the draft text, as the server expects.

### Visual states

| Keys | Action |
|---|---|
| motions, text objects | Extend the selection |
| `o` | Go to the other end |
| `d`, `x` | Delete |
| `c`, `s` | Change |
| `y` | Yank |
| `>`, `<` | Indent / outdent |
| `u`, `U`, `~` | Lower, upper, toggle case |
| `r{char}` | Replace every selected character |
| `J` | Join the selected lines |
| `p`, `P` | Replace the selection with a register (`P` keeps the register) |
| `:` | Ex line with `'<,'>` (only commands that take a range use it) |
| `ESC`, `v`, `V` | Leave or switch the visual kind |

### The new-thread form

The rows (project, environment, branch, provider ...) are a list buffer:
`j`/`k`, `gg`/`G`, `RET` opens a row's picker (a minibuffer, section 12) and
moves to the next row after a pick, `i`/`a` focus the prompt in insert state,
`q`/`ESC`/`:q` close the form (the prompt is kept). The prompt is a composer:
sending creates the thread; `C-k` or `k` on its first line in normal state
returns to the rows.

## 11. Registers and the clipboard

| Register | Contents |
|---|---|
| `""` | Unnamed: the last delete, change or yank |
| `"0` | The last yank |
| `"1`..`"9` | The last nine linewise or multi-line deletes, shifted |
| `"-` | The last small (within a line) delete |
| `"a`..`"z` | Named; `"A`..`"Z` append |
| `"_` | Black hole |
| `"+`, `"*` | System clipboard |
| `".` | The last inserted text (read only) |
| `":` | The last ex command (read only) |
| `"/` | The last search pattern (read only) |

Yanks also go to the system clipboard (Doom's default, `clipboard=unnamedplus`
for yanks only; deletes and changes do not overwrite the clipboard). The
clipboard is written with OSC 52 (wrapped in a DCS passthrough inside tmux),
as the old `y` did. OSC 52 cannot be read back reliably, so `"+p` puts the
last text bb-tui itself wrote to the clipboard or the last bracketed paste it
received in a composer; text copied in other programs is pasted with the
terminal's paste key, which arrives as a bracketed paste. `".`, `":` and `"/`
can be put but not written (`E354`).
`:set clipboard=` turns the automatic clipboard write off; `"+y` still writes
it. `:reg` lists registers.

## 12. Minibuffers: palette, pickers, ex line

The command palette, thread search, quick open, project switcher, mention
popup, model / reasoning / permission pickers, the new-thread row pickers,
the attach-path prompt, rename and filter lines behave like vertico with
evil (Doom `:completion vertico`). They are always in insert state for the
input line; there is no normal state inside them.

| Keys | Action |
|---|---|
| typing | Filter |
| `C-j`, `C-n`, `↓`, `TAB` | Next candidate |
| `C-k`, `C-p`, `↑`, `S-TAB` | Previous candidate |
| `C-d`, `PgDn` / `PgUp` | Page down / up (`C-u` keeps its editing meaning below) |
| `M-<`, `M->` | First / last candidate |
| `RET` | Accept |
| `C-RET` | Accept the typed input as is (no candidate) where free text is allowed |
| `C-;` | Actions on the candidate (embark): for a thread `o` open, `v` open in a split, `R` rename, `d` archive, `y` yank the id; for a file `o` preview, `e` edit, `y` yank the path, `a` attach |
| `ESC`, `C-g`, `C-c` | Abort |
| `C-w` | Delete the word before the cursor (a path component in file pickers) |
| `C-u` | Delete to the start of the input |
| `C-h`, `Bksp` | Delete a character; on an empty query return to the command list (palette) |
| `C-a`, `C-e`, `←`, `→` | Move in the input |
| `C-r {reg}` | Insert a register |
| `M-p`, `M-n` | Previous / next input history entry of this minibuffer |

`SPC '` reopens the last minibuffer with its mode, query and selection.
The palette lists each command with its first key sequence (leader sequences
preferred, then direct chords, then the ex command) and keeps its groups,
recents and plugin commands.

## 13. Ex commands

`:` opens the ex line in the echo area. `TAB` completes command names and
arguments (thread titles for `:b`, workspace paths for `:e`, sections for
`:settings`, options for `:set`), `↑`/`↓` and `C-p`/`C-n` walk the ex
history, the minibuffer editing keys of section 12 apply, `RET` runs, `ESC`
aborts. An unknown command prints `E492: Not an editor command: <name>`.
Commands accept their shortest unique abbreviation shown in brackets. `!`
after a command is its forced variant. Mutating commands are refused in
`--read-only` mode. `@:` repeats the last ex command.

| Command | Action |
|---|---|
| `:q[uit]` | Close the focused window: a split pane, the right panel, the sidebar, a special view; with a single pane and nothing else to close, quit bb-tui |
| `:q[uit]!` | Same, without the archive / unsaved-form confirmations |
| `:qa[ll]`, `:qa!`, `:cq` | Quit bb-tui |
| `:w[rite]` | Send the focused composer's draft |
| `:w[rite]!` | Steer |
| `:wq`, `:x[it]` | Send, then `:q` |
| `:wa[ll]` | Send every pane's non-empty draft |
| `:e[dit] {path}` | Preview a workspace file in the right panel's File tab |
| `:e[dit]`, `:e!` | Reload data (`window.reload`) |
| `:b[uffer] {thread}` | Open a thread by fuzzy title, id or `#N` |
| `:b {N}` | Open thread #N (`thread.jump.N`) |
| `:bn[ext]`, `:bp[revious]`, `:bN` | Next / previous thread |
| `:b#`, `:e#` | Alternate thread |
| `:ls`, `:buffers`, `:files` | Thread list (palette Threads mode) |
| `:bd[elete] [thread]` | Archive the thread (the focused one by default) |
| `:bd! [thread]` | Archive without the confirmation |
| `:unarchive [thread]`, `:bu[ndo]` | Unarchive a thread / undo the last archive |
| `:sp[lit] [thread]`, `:vs[plit] [thread]` | Open a thread (default: the one under the cursor or the focused one) in a new split |
| `:new [prompt]`, `:vnew [prompt]` | New thread in a new split, prompt prefilled |
| `:ene[w] [prompt]` | New thread in the focused pane |
| `:ene[w]! [prompt]`, `:new! [prompt]` | New thread in this thread's environment |
| `:clo[se]` | `C-w c` |
| `:on[ly]` | `C-w o` |
| `:ren[ame] {title}` | Rename the focused thread |
| `:pin`, `:unpin`, `:read`, `:unread` | Thread flags |
| `:noh[lsearch]` | Clear the search highlight |
| `:h[elp] [topic]` | Help: `:help` overview, `:help {key}` describes a key, `:help :{cmd}` an ex command, `:help {view}` a view's keys |
| `:set {option}`, `:set no{option}`, `:set {option}!`, `:set {option}?`, `:set {option}={value}` | Options below; `:set` lists changed options |
| `:se[t] all` | Every option |
| `:mes[sages]` | Message history |
| `:marks`, `:delm[arks] {marks}`, `:delm!` | Marks |
| `:reg[isters]`, `:di[splay]` | Registers |
| `:{N}`, `:$` | Go to line N / the last line |
| `:pal[ette] [query]` | Command palette (`palette.open`) |
| `:find {query}`, `:fi`, `:files` | Quick open (`file.quickOpen`) |
| `:se[arch] {query}`, `:threads {query}` | Thread search (`thread.search`) |
| `:proj[ect] [name]` | Switch project |
| `:filter {text}`, `:filter` | Narrow the sidebar or the focused list; no argument clears |
| `:diff` | Diff panel (`diff.toggle`) |
| `:panel [info\|diff\|file\|terminal]` | Toggle the right panel / open it on a tab |
| `:sidebar` | Toggle the sidebar |
| `:term[inal]`, `:term new`, `:term attach`, `:term close` | Terminal tab, new terminal, attach fullscreen, close the selected terminal (confirm) |
| `:settings [section]`, `:sett` | Settings (`general`, `providers`, `keyboard`, `projects`, `machines`, `plugins`, `archived`) |
| `:automations`, `:auto`, `:auto new` | Automations, new automation |
| `:notif[ications]`, `:notif clear` | Notification center, clear all |
| `:attach [path]` | Attach a file (path prompt without an argument) |
| `:attachments` | Attachments |
| `:model [name]`, `:reasoning [level]`, `:permission [mode]` | Pick, or set directly with an argument |
| `:stop` | Stop the running turn |
| `:steer [text]` | Steer with the draft, or with the text |
| `:queue` | Queued messages |
| `:answer`, `:pending` | Answer the next waiting request |
| `:commit`, `:merge [squash\|merge\|rebase]`, `:ready`, `:draft` | Git actions (confirm) |
| `:reload` | Reload data |

`:set` options:

| Option | Default | Meaning |
|---|---|---|
| `hlsearch` / `hls` | on | Highlight search matches |
| `incsearch` / `is` | on | Move to the match while typing |
| `ignorecase` / `ic` | on | Case-insensitive search |
| `smartcase` / `scs` | on | Case-sensitive when the pattern has uppercase |
| `wrapscan` / `ws` | on | Searches wrap around |
| `number` / `nu` | off | Line numbers in the timeline and composer |
| `relativenumber` / `rnu` | off | Relative line numbers |
| `scrolloff` / `so` | 2 | Lines kept above and below the cursor |
| `clipboard` | `unnamedplus` | Yanks also write the system clipboard; empty turns it off |
| `submitkey` | from the settings file | `enter` or `ctrl-enter`, see section 10 |
| `whichkeydelay` | 300 | Milliseconds before the which-key popup |
| `follow` | on | Timeline follows new output at the bottom |
| `sidebar`, `panel` | layout | Show the sidebar / right panel |
| `mouse` | from the settings file | Mouse capture |

Options set with `:set` last for the session; the settings file (`[evil]`
table: `which_key_delay_ms`, `clipboard`, `relative_number`, `scrolloff`) sets
their startup values.

## 14. Right panel, terminals and special views

Special views are evil-collection style: normal state, the motion set of
section 7, `SPC`, `:`, `C-w`, `C-h/j/k/l`, `/`, `?`, `n`, `N`, marks, counts
and `q`/`ESC` keep their global meaning. Remaining single letters keep a
view-local meaning listed here. The convention for item lists: `RET` opens,
`o` creates a new item, `dd`/`x`/`Del` deletes or archives, `u` undoes,
`R` renames, `~` toggles on/off, `gr` reloads, `yy` yanks, `g/` narrows, `gt`/
`gT` and `{N}gt` switch tabs or sections, `]x`/`[x` jump between items of a
kind.

### Right panel

| Keys | Action |
|---|---|
| `gt`, `gT`, `{N}gt` | Next / previous tab, tab N (1 Info, 2 Diff, 3 File, 4 Terminal) (`panel.nextTab`, `panel.previousTab`) |
| `gr` | Reload |
| `C-w o` | Maximize / restore the panel (`panel.fullScreen.toggle`) |
| `q`, `ESC` | Back from a preview, restore a maximized panel, then leave the panel (focus returns to the timeline) |

Info tab: `j`/`k` move; `RET` opens a commit's diff, reveals a file in the
diff, copies the directory or branch or opens the pull request; `gf` previews
a file; `e` opens it in the editor; `yy` yanks the SHA, path or URL.

Diff tab:

| Keys | Action |
|---|---|
| `j`/`k`, `C-d`/`C-u`, `gg`/`G`, ... | Move (section 7) |
| `zh`, `zl`, `0` | Scroll sideways, back to the first column (`h`/`l` move the column cursor) |
| `RET`, `TAB`, `za` | Fold / unfold the file, load an on-demand diff |
| `zR`, `zM` | Unfold / fold all files |
| `]c`, `[c` | Next / previous hunk (vim diff) |
| `]f`, `[f` | Next / previous file |
| `s`, `S` | Next / previous scope |
| `gf` | Preview the file at the cursor line |
| `e` | Open the file at the cursor line in `$VISUAL`/`$EDITOR` |
| `yy`, `gy` | Yank the line / the path |
| `V` + `y` | Yank the selected diff lines |

File tab: read-only line buffer with motions, search and visual yank; `e`
edits, `gy` yanks the path, `zh`/`zl` scroll sideways.

Git confirmations and the merge-method choice are transients: `y`/`RET`
confirm, `ESC`/`n` cancel, `h`/`l` or `1`-`3` pick a merge method.

### Terminal tab

| Keys | Action |
|---|---|
| typing (insert state, `<I>`) | Goes to the terminal program, `ESC` and `C-w` included |
| `C-\`, `C-\ C-n` | Normal state (stop typing; focus stays on the panel) |
| `i`, `a`, `A`, `I`, `RET` | Insert state (type into the terminal) |
| `S-PgUp`/`S-PgDn` in insert | Scroll back while typing |
| motions, `C-u`/`C-d`, `gg`/`G`, search, `v`/`V` + `y` in normal | Scrollback; `G` returns to the live screen |
| `]t`, `[t` | Next / previous terminal |
| `o` | Start a new terminal |
| `dd`, `x` | Close the terminal (confirm) |
| `SPC o f` | Attach fullscreen; `C-\` or `C-b d` detaches |
| `yy`, `SPC b y` | Yank the line / the visible screen |

### Notification center

`j`/`k` move, `RET` opens the thread (request panel focused if it waits),
`dd`/`x`/`Del` delete one, visual `d` deletes the selection, `C` or
`:notif clear` clears all, `q`/`ESC` close.

### Settings

Two windows: the section list on the left, the section on the right.
`C-h`/`C-l` (or `h` in a list without columns / `l`, `RET`) move between
them; `gt`/`gT`/`{N}gt` switch sections; `q`, `ESC` in the section list, or
`:q` close the view. In section lists: `RET` opens or edits, `~` toggles
(was `Space`: enable a provider or plugin, flip a boolean), `R` renames a
project or machine, `dd`/`x`/`Del` delete a project or archived thread
(confirm), `u` unarchives, `gd` makes a provider the default, `zc` toggles
collapsing finished turns of a provider, `t`/`p` cycle the archived-thread
type and project filters, `g/` narrows, `yy` yanks ids, paths and URLs, `gr`
reloads. Plugin forms: `j`/`k` fields, `RET` edits, `~` toggles a boolean,
digits choose in an open picker.

### Automations

List: `RET` opens, `o` new automation, `~` pauses / resumes, `r` runs now,
`e` edits, `dd`/`x`/`Del` delete (confirm), `g/` narrows, `gr` reloads,
`q`/`ESC` close. Detail: same keys for the open automation, `RET` on a run
opens its thread, `gm` loads more runs, `q`/`ESC`/`h` back. Form: `j`/`k`
fields, `RET` edits or chooses, digits pick in an open picker, `:w`, `C-c
C-c` or `C-s` save, `ESC`/`:q!` discard.

### Help, plugin output and file preview panes

Read-only line buffers: motions, search, visual yank, `zh`/`zl` sideways,
`yy` yanks, `gy` yanks the command's stdout or the file path, `gx` opens the
previewed file, `q`/`ESC`/`RET` close.

### Request panel (transient)

The request panel keeps its single-key map because answering is its only
purpose; `M-SPC` reaches the leader, `C-h/j/k/l` leave it, `ESC` leaves it.

| Request | Keys |
|---|---|
| Approvals | `y` allow once, `s` allow for the session, `d` deny, `1`-`3` by position, `e` show the whole command |
| Plan review | `y` approve, `d` keep planning |
| Questions | `1`-`9` choose, `j`/`k` + `x` or `SPC` toggle (the leader is off here), `TAB`/`h`/`l` switch question, `RET` next / submit, typing goes to `Other…` |
| Secret | masked input (insert state), `TAB`/`↑↓` field, `C-t` reveal, `RET` next / submit |
| Other plugin request | `o` open in the web app, `y` copy the link |
| Any | `C-x` cancel where cancellable |

Queued messages, attachments and pickers opened from a thread are transients
too: `j`/`k`, `RET`, their own letters (`e` edit, `s` send now, `r` retry,
`dd`/`d` delete), `q`/`ESC` close.

## 15. Which-key

- Prefixes: `SPC` and every leader group, `M-SPC`, `C-w`, `C-c`, `g`, `z`,
  `[`, `]`, `"`, `m`, `'`, `` ` ``, `Z`, and operators waiting for a motion.
- After a prefix the TUI waits for the next key indefinitely. When no key
  arrives for `which_key_delay_ms` (300 ms, Doom's `which-key-idle-delay`),
  a popup opens at the bottom of the screen over the composer area, full
  width, at most half the screen high. Once open, the popup follows every
  further prefix immediately (no delay).
- Content: the current sequence as the title (`SPC b-`), then columns of
  `key → description`, groups shown as `key → +group` in a distinct color,
  sorted with plain letters first, then uppercase, then symbols, then
  modified keys. For `"` it lists non-empty registers with a preview; for
  `'` and `` ` `` the marks with their line.
- Paging: when entries do not fit, the title shows `page 1/3`; `C-n`/`C-p`
  (or `PgDn`/`PgUp`) page, other keys continue the sequence.
- `Bksp` goes back one level of the sequence; `ESC`/`C-g` cancel it.
- An undefined key ends the sequence with `SPC x y is undefined` in the echo
  area.
- The popup is built from the same key trie that dispatches keys, so its
  descriptions are the command descriptions shown in the palette.

## 16. Server keybinding overrides

The web app's `APP_COMMAND_IDS` overrides (Settings → Keyboard on the web,
`GET /system/config`) keep working:

- An override gives its command a direct chord. It replaces the command's
  default direct chords listed below; leader sequences, `C-w` sequences,
  bracket motions and ex commands for the command stay.
- A `null` shortcut removes the direct chords only.
- An override chord shadows the evil binding of the same key in normal,
  visual and special-view states (an override of `thread.archive` to `j`
  makes `j` archive there). In insert state and minibuffers it applies only
  when the chord has Ctrl or Alt and is not one of the reserved editing keys
  `C-w C-u C-h C-r C-o C-v C-j C-k C-n C-p C-a C-e C-c C-[`.
- `Cmd`-only shortcuts and other platforms are skipped, as before.

| Web command id | Command | Default direct chord | Sequence / ex |
|---|---|---|---|
| `app.back` | Back / keyboard quit | `ESC` | `C-g` |
| `palette.open` | Command palette | `M-x` | `SPC :`, `:palette` |
| `file.quickOpen` | Quick open | — | `SPC SPC`, `SPC f f`, `:find` |
| `thread.search` | Search threads | — | `SPC /`, `SPC s p`, `:search` |
| `thread.new` | New thread | — | `SPC b N`, `SPC a a`, sidebar `o`, `:enew` |
| `thread.rename` | Rename thread | — | `SPC b R`, sidebar `R`, `:rename` |
| `thread.archive` | Archive / unarchive | — | `SPC b k`, sidebar `dd`, `:bd` |
| `thread.next`, `thread.previous` | Next / previous thread | — | `]b`/`[b`, `SPC b n`/`SPC b p`, `:bn`/`:bp` |
| `thread.jump.1` .. `9` | Open thread #N | `M-1` .. `M-9` | `SPC TAB N`, `:b N` |
| `sidebar.toggle` | Toggle sidebar | — | `SPC o p`, `SPC t s`, `:sidebar` |
| `panel.toggle` | Toggle right panel | — | `SPC o i`, `SPC t p`, `:panel` |
| `diff.toggle` | Diff panel | — | `SPC g g`, `SPC o d`, `:diff` |
| `terminal.open` | Terminals | — | `SPC o t`, `:term` |
| `panel.nextTab`, `panel.previousTab` | Panel tabs | — | `gt`/`gT` |
| `panel.fullScreen.toggle` | Maximize panel | — | `C-w o` in the panel, `SPC w m m` |
| `pane.focus.next`, `pane.focus.previous` | Next / previous window | — | `C-w w`, `C-w W` |
| `pane.focus.left`, `pane.focus.right` | Pane left / right | `M-h`/`M-←`, `M-l`/`M-→` | `C-w h`/`C-w l`, `C-h`/`C-l` |
| `pane.focus.1` .. `4` | Focus pane N | — | `SPC N`, `{N}C-w w` |
| `pane.close` | Close pane | — | `C-w q`, `C-w c`, `:q` |
| `pane.maximize.toggle` | Maximize pane | `M-z` | `C-w o`, `SPC w m m` |
| `window.reload` | Reload | — | `SPC b r`, `gr`, `:e` |
| `settings.open` | Settings | — | `SPC h ,`, `:settings` |
| `composer.focus` | Focus composer | — | `i`/`a` in the timeline, `SPC a c` |
| `modelPicker.toggle` | Model picker | `M-m` | `SPC m m`, `:model` |
| `modelPicker.cycleModel`, `…Backward` | Cycle model | `M-.`, `M-,` | — |
| `modelPicker.cycleReasoning`, `…Backward` | Cycle reasoning | `M-r`, `M-R` | — |

## 17. Mouse

Mouse support is unchanged in what it reaches: clicking a sidebar row
selects it and opens a thread, clicking a timeline row moves the cursor there
(normal state) and toggles a row's fold glyph, clicking the composer focuses
it in insert state with the cursor at the click, clicking a panel tab or a
pane focuses it, the wheel scrolls the window under the pointer (moving the
cursor only when it would leave the view, `scrolloff` respected). Dragging in
the timeline or composer makes a visual selection; release keeps it, `y`
yanks it. Clicks never change the evil state of a window other than the
composer.

## 18. Russian layout

Following the user's `reverse-im` setup (`russian-computer`), in normal,
visual, operator-pending states, special views, transients and inside key
sequences (leader, `C-w`, `g`, `z`, ...) a Cyrillic letter is read as the
Latin letter on the same key of the PC layout, with Ctrl and Alt preserved:

```
й ц у к е н г ш щ з х ъ   ф ы в а п р о л д ж э   я ч с м и т ь б ю
q w e r t y u i o p [ ]   a s d f g h j k l ; '   z x c v b n m , .
```

and the uppercase rows likewise (`Л` is `K`, `П` is `G`). `ё`/`Ё` map to
`` ` ``/`~`. ASCII punctuation is never translated. Insert state, the
terminal tab, minibuffer inputs, `f{char}`/`t{char}`/`r{char}` arguments,
search and ex text keep the typed characters.

## 19. Migration from the previous keymap

Every command of the previous keymap and where it went:

| Old keys | Command | New keys |
|---|---|---|
| `q`, `C-c` | Quit | `SPC q q`, `:q` (last window), `:qa`, `ZZ`, `ZQ` |
| `?` | Help | `SPC h b b`, `F1`, `:help` (`?` searches backward) |
| `Esc` | Back | `ESC`, `C-g` |
| `Tab`, `S-Tab` | Focus next / previous pane | `C-w w`, `C-w W`, `C-h/j/k/l` (`TAB` folds) |
| `j`/`k`, arrows | Move | unchanged |
| `g`/`G`, `Home`/`End` | Top / bottom | `gg`/`G`, `Home`/`End` |
| `Enter` | Open / expand | `RET` (unchanged) |
| `/` | Filter sidebar | `g/`, `SPC s f`, `:filter` (`/` searches) |
| `h`/`←`, `l`/`→` | Collapse / expand | sidebar unchanged; timeline `zc`/`zo` (`h`/`l` move the column) |
| `Space` | Toggle group / row | `TAB`, `za`, `RET` (`SPC` is the leader) |
| `r` | Rename | sidebar `R`, `SPC b R`, `:rename` |
| `a` | Archive | sidebar `dd`/`x`, `SPC b k`, `:bd` |
| `u` | Undo archive | sidebar `u`, `SPC b u`, `:bu` |
| `m` | Read / unread | sidebar `!`, `SPC b !` (`m` sets marks) |
| `p` | Pin | sidebar `+`, `SPC b m` |
| `J`, `K` | Next / previous thread | `]b`, `[b`, `SPC b n`/`p` |
| `M-1`..`M-9` | Jump to thread | unchanged, plus `SPC TAB N` |
| `C-b` | Toggle sidebar | `SPC o p` (`C-b` pages up) |
| `i` | Toggle right panel | `SPC o i` (`i` inserts) |
| `D` | Diff panel | `SPC g g`, `SPC o d` |
| `>`, `<` | Panel tabs | `gt`, `gT` |
| `Z` | Maximize panel | `C-w o` in the panel, `SPC w m m` |
| `T` | Terminals | `SPC o t` |
| `,` | Settings | `SPC h ,`, `:settings` |
| `A` | Automations | `SPC o A`, `:auto` |
| `V` | Open in a split | `C-w v`, `:vs` (`V` is visual-line) |
| `M-h`/`M-l` | Pane left / right | unchanged, plus `C-w h/l`, `C-h/l` |
| `X` | Close pane | `C-w q`, `C-w c`, `:q` |
| `M-z` | Maximize pane | unchanged, plus `C-w o` |
| `C-r` | Reload | `SPC b r`, `gr`, `:e` (`C-r` redoes) |
| `PgDn`/`C-f`, `PgUp` | Page | unchanged, plus `C-b` |
| `C-d`, `C-u` | Half page | unchanged |
| `]`, `[` | Next / previous user message | `]m`, `[m` |
| `y` | Copy row | `gy` (row source), `y{motion}`/`yy` (rendered text) |
| `o` | Open link / file | `gx` |
| `c` | Focus composer | `i`, `a`, `A`, `SPC a c` |
| `S` | Stop the turn | `C-c C-k`, `SPC m k`, `SPC a q`, `:stop` |
| `M-q`, `M-t` | Queue list, submit mode | unchanged, plus `SPC m q`, `SPC t q` |
| `w`, `M-w` | Next waiting request | `SPC n r`, `SPC m r`, `M-w`, `]w` (`w` is a motion) |
| `!` | Notification center | `SPC n n`, `:notif` (sidebar `!` toggles read) |
| `n` | New thread | `SPC b N`, `SPC a a`, `SPC x`, sidebar `o`, `:enew` (`n` repeats search) |
| `N` | New thread in environment | `SPC b c`, `SPC m n`, sidebar `O`, `:enew!` |
| `C-k`, `:` | Palette | `SPC :`, `M-x` (`:` is ex, `C-k` is window up) |
| `C-p` | Quick open | `SPC SPC`, `SPC f f`, `:find` (`C-p` is history in the composer) |
| `f` | Search threads | `SPC /`, `SPC s p`, `:search` (`f` finds a character) |
| `P` | Switch project | `SPC p p`, `SPC TAB TAB`, `:project` (`P` puts) |
| `C-o` | Attach a file | `C-c C-a`, `SPC m a`, `SPC f a`, `:attach` (`C-o` jumps back) |
| `M-a` | Attachments | unchanged, plus `SPC m A` |
| `M-m`, `M-e`, `M-p` | Pickers | unchanged, plus `SPC m m/e/p` |
| `M-.`, `M-,`, `M-r`, `M-R` | Cycle model / reasoning | unchanged |
| composer `Enter` | Send | normal `RET`, `C-c C-c`, `C-RET`, `:w`; insert `RET` is a new line by default |
| composer `C-s` | Steer | `C-c C-s`, `:w!`, `SPC m S` |
| composer `Esc Esc` | Stop the turn | `C-c C-k` |
| composer `C-c` | Clear / stop / quit | `C-c` is a prefix; clear with `ggdG` or `S` in normal state |
| composer `↑`/`↓` | History | unchanged in insert on the first / last line; `C-p`/`C-n` |
| panel `1`-`4` | Tabs | `{N}gt` |
| panel `r` | Reload | `gr` |
| panel `C`, `M`, `R`, `W` | Git actions | `SPC g c c`, `SPC g p m`, `SPC g p r`, `SPC g p d` |
| diff `n`/`N` | Hunks | `]c`/`[c` |
| diff `]`/`[` | Files | `]f`/`[f` |
| diff `a` | Fold all | `zR`/`zM` |
| diff, info `p` | Preview file | `gf` |
| diff `h`/`l` | Scroll sideways | `zh`/`zl` |
| terminal `]`/`[` | Next terminal | `]t`/`[t` |
| terminal `n` | New terminal | `o`, `SPC o T` |
| terminal `x` | Close terminal | `dd`, `x` |
| terminal `A` | Attach fullscreen | `SPC o f` |
| terminal `y` | Copy screen | `SPC b y` (`yy` the line) |
| notifications `d` | Delete | `dd`, `x` |
| settings `1`-`7` | Sections | `{N}gt` |
| settings `Space` | Toggle | `~`, `RET` |
| settings `r`, `D`, `d`, `c` | Rename, delete, default provider, collapse turns | `R`, `dd`, `gd`, `zc` |
| settings `/` | Filter | `g/` |
| settings `,` | Close | `q`, `:q` |
| automations `Space`, `n`, `m`, `R`/`C-r`, `C-s` | Pause, new, more runs, reload, save | `~`, `o`, `gm`, `gr`, `:w`/`C-c C-c` (`C-s` kept in the form) |
| new thread `Tab`/`S-Tab` (prompt ↔ rows) | | `C-k`/`C-j`, `i` |

`submit_key` keeps its two values with a new default: `ctrl-enter` (insert
`RET` is a new line, `C-RET`/`M-RET` send) is the default; `enter` makes
insert `RET` send and `S-RET`/`C-j` insert a new line. Normal-state `RET`
always sends.

## 20. Conflicts and resolutions

| Conflict | Resolution |
|---|---|
| `SPC` toggled folds and options | `SPC` is the leader everywhere a list or buffer has focus; folds use `TAB`/`za`, toggles in special views use `~`; inside transients and text inputs `SPC` stays a character, the leader is `M-SPC` |
| Single-letter list actions (`r a m p n N f P c i w o D T A V X Z S`) collide with evil motions, operators and states | moved under `SPC`, the `g`/`z`/`]`/`[` prefixes, or evil-collection list letters (`R`, `dd`, `x`, `u`, `!`, `+`, `o`, `O`, `gr`, `gx`, `gy`, `gf`) |
| `C-h/j/k/l` | window motions (user config) in normal/visual/special views; insert keeps `C-h` backspace, `C-j` newline, `C-k` kill line; minibuffers use `C-j`/`C-k` for candidates |
| `C-k` palette | `SPC :`/`M-x`; servers that override `palette.open` to `Ctrl+K` win in normal state (section 16) |
| `C-p` quick open vs history | history in the composer, previous candidate in minibuffers; quick open is `SPC SPC` |
| `C-o` attach vs jump back | jump list; attach is `C-c C-a` / `SPC m a` |
| `C-r` reload vs redo | redo; reload is `SPC b r` / `gr` / `:e` |
| `C-b` sidebar vs page up | page up; sidebar is `SPC o p` |
| `C-c` quit / clear vs Doom `C-c` prefix | prefix only; quit is `SPC q q`; clearing a draft is an edit (`ggdG`, `S`) |
| `C-s` steer vs Doom search | steer is `C-c C-s`; `C-s` is unbound outside the automation form |
| `C-w` delete word (insert) vs window prefix | vim rule: insert and minibuffers delete a word, normal state is the window prefix |
| `TAB` folding vs `C-i` jump forward | they are the same byte in legacy terminals: `TAB` folds; `C-i` jumps forward only with the kitty keyboard protocol |
| `q` quit vs macro recording | `q` closes special views and popups; in main buffers it is unbound (hint shown); macros are out of scope |
| `/` filter vs search | `/` searches with highlight and `n`/`N`; narrowing moved to `g/` |
| `ESC` in the composer left it; `Esc Esc` stopped the turn | `ESC` goes to normal state; leaving is `C-k`/`C-w k`; stopping is `C-c C-k` |
| Digits picked tabs and sections | digits are counts; tabs and sections use `{N}gt`; transients and open pickers keep digit choices |
| `n` new vs search next | `o` creates items in lists, `SPC b N` creates threads |
| `M-p` permission picker vs comint history | picker keeps `M-p`; composer history is `C-p`/`C-n` and arrows; minibuffer history keeps `M-p`/`M-n` because pickers are not reachable there |
| `gv` | evil reselect; open-in-split is `C-w v` |
| Doom `SPC n` (notes), `SPC o A` (agenda), `SPC x` (scratch) | reused for bb notifications, automations and a new thread; no bb concept collides |
| Doom `SPC a` (embark / none) | user config: agent prefix |

## 21. Out of scope for the first cut

- Macros (`q{reg}`, `@{reg}`, `@@`); `q` stays unbound in main buffers.
- Visual-block state (`C-v`) and `:s`/`:%s` substitution in the composer.
- `C-a`/`C-x` number increment, `g?` rot13, `=` reformat (`=` is accepted as
  an operator and does nothing).
- Horizontal splits: `C-w s` and `:sp` open a vertical split.
- Reading the system clipboard over OSC 52.
- `SPC f r` beyond the files previewed in this session.
- Rebinding evil keys from the settings file; only web overrides apply.

## 22. Implementation notes

- One key trie per scope (global, leader, `C-w`, `g`, `z`, `[`, `]`,
  `C-c`, and one per buffer kind) maps sequences to `Command`s with
  descriptions; dispatch, which-key, `SPC h b b`, the palette key column and
  the settings Keyboard section read it.
- An evil state machine per focused window holds the state, pending count,
  register, operator and prefix; read-only buffers share motions over a
  `LineBuffer` trait (lines plus row anchors), the composer implements the
  editing subset on its text area.
- `Keymap::apply_overrides` keeps its contract for direct chords; tests that
  asserted the old single-letter defaults move to the new defaults, and new
  tests cover state transitions, operators with motions and text objects,
  counts, registers, `.` repeat, undo/redo, search, marks, which-key timing
  and override precedence over evil keys.
- The Russian translation happens once, when a key event enters the evil
  layer outside insert state.

## 23. Implementation status

What the first step built, and what later steps add. The tables above stay
the target; this list says which parts already run.

Built:

- The key engine: one trie per scope (global, leader, `C-w`, sidebar,
  timeline, composer normal / visual / insert, new-thread rows, each right
  panel tab, notifications, settings, automations, output pane), counts,
  registers, character arguments (`f t r "`), the which-key popup (delay,
  paging, `Bksp`, undefined-key message), `SPC h k`, `SPC h b b`, `SPC h ?`,
  `M-SPC` from every state, the Russian layout, direct chords with server
  overrides and their precedence rules (section 16).
- The modeline: state tag, pending keys, buffer name, position; the echo area
  with messages, the ex line and hints.
- The whole leader map of section 5 except the entries listed below, all of
  section 6, the global keys of section 4.
- The sidebar keys of section 8 except visual-line batches, `]b`/`[b` and
  `]w`/`[w` everywhere.
- Timeline rows: `j k { } gg G {N}G H M L C-d C-u C-f C-b C-e C-y zz zt zb`,
  `]m [m`, `RET TAB za zo zc zO zC zR zM`, `gy`, `gx`, `i a A o O I gi`, `gr`.
- The composer: normal, insert, visual and visual-line states, the motions of
  section 7 inside the draft (`h j k l w b e ge W B E gE 0 ^ $ g_ gg G { } %
  f F t T ; ,`), operators `d c y > < gu gU g~` with motions, text objects and
  doubled-key line forms, `x X s S r ~ J gJ D C Y p P u C-r v V`, registers of
  section 11 (named, numbered, small delete, black hole, clipboard), one undo
  step per command or insert session, insert keys `C-w C-u C-h C-n C-p C-j
  C-r C-o`, `C-c C-c/C-s/C-k/C-a`, `C-RET`/`M-RET`, normal `RET`, the new
  `submit_key` default, the new-thread prompt as the same modal editor.
- The composer as an evil buffer (second step): `.` repeat with insert text
  and new counts, `R` replace state (`<R>` tag), `U`, `gv`, composer marks
  (`m{a-z}`, `'`/`` ` `` jumps and operator targets, `'[ '] '< '> '. '^ ''`),
  sentence motions and objects (`( ) is as`), `{N}%`, counts after an
  operator (`2d3w`), grapheme-aware character motions and edits, chips that
  are one word and are deleted, changed and yanked whole, read-only `". ": "/`
  registers, bracketed paste as one change in every state and into `"+`,
  insert `C-a` (first non-blank), `C-v` (literal next key) and `TAB` (a tab),
  mouse click-to-position and drag selection in the thread composer.
- Read-only evil buffers (third step): the sidebar, the timeline, the Diff
  and File tabs, the terminal scrollback in normal state and output panes
  (help, registers, marks, plugin output) share one engine over their
  rendered text. Built: the line and column cursor (the sidebar has lines
  only), the motions of section 7 with counts, `C-d C-u C-f C-b C-e C-y zz
  zt zb z. z- z RET H M L`, incremental smart-case regex search `/ ? n N * #
  g* g#` with highlighting, wrap messages, `[i/N]` in the echo area, search
  history, the `"/` register, `:noh`, `:set [no]wrapscan`, `:set
  [no]hlsearch`; visual and visual-line selection (`v V gv o`, text objects
  in visual state) and yanks (`yy Y y{motion} y{object} y'{mark}`, visual
  `y`/`Y`) into the registers and `"+` (OSC 52); local marks, `''`, the
  jump list (`C-o C-i`), session-only global marks in the timeline,
  `:marks`, `:delm`; the timeline extras `]] [[ ]e [e gf`, `gx` on a URL
  under the cursor, `` yi` `` / `` ya` `` on rendered code blocks, `yim`
  / `yam`, `vim` / `vam`; sidebar visual-line batches (`d x Del ! + y`) and
  search that opens collapsed groups; diff `gy`; terminal `yy` (the line)
  and `SPC b y` (the screen); a mouse click puts the timeline cursor at the
  clicked character.
- Minibuffers: the vertico keys of section 12 except `C-;`, `C-r` and
  `M-p`/`M-n`; `SPC '`.
- Ex commands of section 13 except `:marks`, `:delm`, `:h {topic}`, `@:`,
  `:wq` ranges; `:set` for `whichkeydelay`, `clipboard`, `submitkey`,
  `sidebar`, `panel`, `follow`; the `[evil]` settings table with
  `which_key_delay_ms` and `clipboard`.
- Right panel, terminal, notifications, settings and automations keys of
  section 14, mapped onto the existing view actions.

Not built yet:

- Search, marks and the jump list in the composer, persisting global marks
  to `$XDG_STATE_HOME/bb-tui/marks.json` (they live for the session),
  `:marks` as a picker (it is a read-only listing pane), `zj`/`zk`, search
  inside the deferred content of collapsed timeline rows, revealing matches
  inside collapsed parent threads of the sidebar (only collapsed groups
  open).
- Evil buffers for the Info tab, notifications, settings and automations
  lists (they keep their list keys).
- Composer `gj`/`gk` by display line (they move by logical line), mouse
  click-to-position in the new-thread prompt.
- Which-key listings for registers and marks; `C-;` embark actions; minibuffer
  history (`M-p`/`M-n`).
- Cursor shape by state (DECSCUSR), relative line numbers (`SPC t l`,
  `:set rnu`), `scrolloff`, mouse drag selection in the timeline and the
  panel.
- Leader entries `SPC f r`, `SPC h x`, `SPC s s/b/i/r/c`, `SPC t h`,
  `SPC g o P`; `SPC ,` searches all projects instead of the focused one.
- Notification visual delete, request panel `x` toggle, automation form
  `:w`/`C-c C-c` (it keeps `C-s`).

Deviations of the third step, by design:

- Line-only buffers (the sidebar) search by line: `n` moves to the next line
  with a match, `[i/N]` counts lines, and `v` starts visual-line state.
- `*` and `#` follow smart-case like `/` instead of plain `ignorecase`.
- The first `ESC` in a read-only buffer with a search highlight or a visual
  selection only clears it; the next `ESC` does what section 4 lists.
- Search in the sidebar opens every collapsed group that holds a match when
  the search is submitted, not only the group of the match it lands on.
- Output panes: `yy` yanks the line under the cursor and `gy` the whole
  output (or the previewed file path), `q`/`RET`/`ESC` close.

