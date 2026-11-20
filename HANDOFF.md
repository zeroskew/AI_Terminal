# AI Terminal: handoff

A Windows terminal app for Claude Code / Codex / shell sessions, terminal-only (no chat
sessions), with pasted-image previews in the terminal. Rust +
egui/eframe 0.37, ~9,500 lines in `src/`. The user (Dima) runs the installed build
`dist\ai_terminal.exe` daily. Feature reference for users: `README.md`.

## Build, test, ship

```
build.cmd build                  dev build (VS 2022 env via vcvars64; plain cargo fails to link)
build.cmd test                   25 unit tests (+1 ignored live cost check)
dist.cmd                         release build, copied to dist\ (fails if dist\ai_terminal.exe is running)
build.cmd run --example mkicon   regenerate the icon (tools\mkicon.rs → assets\)
```

**Before `dist.cmd`, check the user isn't running the installed exe**
(`tasklist` → `(Get-Process -Id <pid>).Path`). If it's running, build with `cargo build --release`
(through vcvars) and ask them to close it.

## Testing rules (important)

- **Never** `taskkill /IM ai_terminal.exe`: the user's own instance has the same image name. One
  by-name kill already closed their window. Find your test instance's PID
  (`/proc/*/exename` + `/proc/<pid>/winpid` in Git Bash), and capture/kill by PID only.
- Always launch test builds with **`--test-instance`**. It uses separate settings
  (`%APPDATA%\AI Terminal Test`), so test runs never overwrite the user's projects or sessions.
- `tools\capture.ps1 out.png -ProcessId <pid>` screenshots a window (PrintWindow, no input).
- `tools\hover.ps1 -ProcessId <pid> -X <x> -Y <y>` posts WM_MOUSEMOVE (physical client px)
  without moving the real cursor. It worked once for a path hover, but not reliably.
- Test hooks (any order unless noted):
  - `--add-project <dir>` (also selects it, so `--start` opens there)
  - `--start <Claude|Codex|pwsh|powershell|cmd>`
  - `--type "<line>"` and `--paste-image <file>` and `--find <text>`: run after `--start`
  - `--options`, `--options-page <general|images|snippets|scripts|shortcuts|about>`, `--launch-dialog`
  - `--scripts` (opens the palette), `--run-script "<name>"` (runs a script in the selected project; `--type` after it feeds its input)
  - `--plugins`, `--plugins-sources`, `--view-image <file>`
  - `--debug-log` (log goes to `%LOCALAPPDATA%\AI_Terminal\logs\ai_terminal.log`)
  - `--no-previews`, `--pin <n>`
  - `--check-updates` (opens the Check for updates dialog; a test instance downloads to `update-test`). To test a real download + swap, build with `AI_TERMINAL_VERSION=2026.01.01-0000` and run a copy from outside `target`
  - `--home` (opens on the Home page)
  - `--search <text>` (opens the Search window and runs it)
- Real Claude end-to-end runs (`--start Claude --type "..."`) work; Claude opens in the selected
  project; pass `--add-project` with an already-trusted folder, or Claude stops at its trust prompt.

## Architecture (src/)

| File | What |
|---|---|
| `main.rs` | Entry; `--hook` / `--statusline` modes exit early (hooks.rs); drops inherited `CLAUDE_*` session env; `--test-instance` app name |
| `app.rs` (1.4k) | App state, types, `eframe::App` (frame loop), sessions, shortcuts, housekeeping; UI split into `app/*.rs` below |
| `app/*.rs` | `impl App` blocks by area: `topbar`, `sidebar`, `pages` (home / empty project), `launch`, `options`, `snippets`, `scripts` (palette for `<exe dir>\scripts`), `plugins_ui`, `mcp_servers_ui`, `status_bar` (usage limits; compact / context / clear (confirmation dialog) / mcp links), `viewer` (+ toasts), `mcp_api` (ai-terminal MCP tool handlers) |
| `view.rs` | Terminal widget: grid snapshot under the alacritty lock, painting, box-drawing as shapes, keys→bytes, mouse/selection, image refs + inline previews, color tags, Ctrl+links, find bar |
| `session.rs` | One PTY session: alacritty_terminal `Term` + ConPTY event loop, `Profile` detection, launch args, agent id / transcript title / cost polling, activity times |
| `agents.rs` | Claude/Codex launch flags (`build_args`), model catalogues (`claude --help`, `codex debug models`), transcript lookup (`~/.claude/projects/<mangled cwd>`, `~/.codex/sessions`, Antigravity CLI `~/.gemini/antigravity-cli/conversations/<id>.db`: SQLite, protobuf steps read by field number: type 14 user text 19.2, type 15 agent text 20.1, metadata 1 = timestamp, trajectory_metadata_blob 1.1 = workspace URI), titles; statics for hook settings / MCP config / color-tag prompt |
| `hooks.rs` | Claude hooks: `--settings hooks-<hash>.json` runs `ai_terminal.exe --hook <event>`, which writes `%LOCALAPPDATA%\AI_Terminal\events\<key>~<event>~<ns>.evt`; app polls; watcher thread wakes the UI. `SessionStart` → `start` event carrying `session_id` re-links the tab to the new conversation after `/clear` (`Session::set_conversation`). Also sets `statusLine` → `ai_terminal.exe --statusline`, which saves Claude's `rate_limits` to `%LOCALAPPDATA%\AI_Terminal\limits.json` (status bar) and chains the user's own statusLine |
| `mcp.rs` / `mcp_tools.rs` | Local MCP server (`ai-terminal`, HTTP 127.0.0.1:random, bearer token, header `x-ai-terminal-session`), 12 tools, run on the UI thread via a channel |
| `images.rs` | Clipboard image → `%TEMP%\AI_Terminal\paste\paste-<hash>.png`; texture cache (mipmaps + exact-size Lanczos copies); image-ref regex |
| `emoji.rs` | Color emoji and badge text via Direct2D/DirectWrite → textures |
| `colortags.rs` | `{red}…{/}`, `{bg:x}`, `{row:x}` tags in agent output; dark diff shades `#3d0100` / `#022800` (spans `#5c0200` / `#044700`) |
| `cost.rs` | Cost/tokens from transcripts (Claude: dedupe by message id, subagents included; Codex: token_count), `assets/pricing.json` (user override `%APPDATA%\AI Terminal\pricing.json`) |
| `plugins.rs` | Plugin catalogues: Claude Code (marketplace.json files + built-in directory cache, installed_plugins.json) and Codex (`codex plugin list --available --json` + `marketplace list`, descriptions from local `.codex-plugin/plugin.json`, loaded on a thread); background `claude` / `codex plugin …` jobs. The window (`app/plugins_ui.rs`) has a vertical tab per provider |
| `mcp_servers.rs` | Per-project MCP on/off: `claude mcp list` (parsed) + `~/.claude.json` `projects[<path>].disabledMcpServers` (what `/mcp` writes); built-in `claude-artifacts` (not in `mcp list`) via a `permissions.deny: ["Artifact"]` rule in `<project>/.claude/settings.local.json` (Claude Code 2.1.325 skips the server when the Artifact tool is denied; env `CLAUDE_CODE_ARTIFACT=0` also works); `describe()` = per-server help text |
| `search.rs` / `app/search_ui.rs` | Transcript search (user + agent text parts of Claude / Codex messages; raw-line prefilter, then JSON parse) on a thread; non-modal Search window (all sessions / current, days back), jump = select or resume + terminal find bar |
| `update.rs` / `app/update_ui.rs` / `tools/relauncher.rs` | Self-update. build.rs stamps `AI_TERMINAL_VERSION` (UTC `YYYY.MM.DD-HHMM`; `--version` prints it; Publish release.ps1 tags `v<version>`) and compiles `tools/relauncher.rs` (std only), embedded with `include_bytes!`. A thread asks the GitHub API for the latest release, downloads `ai_terminal.zip` (WinHTTP) to `%LOCALAPPDATA%\AI_Terminal\update\` and unzips the exe (miniz_oxide). At exit (Restart now, or any quit) main.rs writes the relauncher to `%TEMP%\AI_Terminal\` and starts it: it waits until the exe this process was launched from is unlocked, swaps it (`.old` backup, restored on failure), optionally starts it again; log `relauncher.log` beside it. Auto checks (Options > General > Update automatically) are off for exes under a `target` folder |
| `keys.rs` | Reassignable shortcuts (19 commands, stored as text overrides) |
| `links.rs` | URL / file:line detection, open in browser / editor |
| `winsys.rs` | Tray icon + toast (Shell_NotifyIcon), Run-key autostart, taskbar overlay badge, memory reading, sound |
| `fonts.rs` | CSS-style font list resolved against installed fonts by family name (default list in `fonts::DEFAULT_LIST`) |
| `term_theme.rs`, `theme.rs`, `widgets.rs`, `titlebar.rs`, `dlog.rs` | Colors/presets, egui style, painted icons/buttons, dark DWM title bar, debug log |

**App state and UI (`app.rs` + `app/`):**
- `Settings` (persisted by eframe, `version` field for migrations, currently 9)
- projects, live `Tab`s and `Dormant` (listed, not running) sessions
- top bar (project tabs, drag to reorder) and sidebar (sessions: day groups / pinned, live status, cost, rename, context menu)
- new-session menu and launch dialog; Options window (General / Terminal / Images / Snippets / Scripts / Shortcuts / About)
- plugins window, image viewer, snippets and scripts palettes
- `housekeeping` (tray, hooks → attention/notifications/badge, auto-close), `mcp_pump` (tool calls, message delivery)

## Session behaviour

- Nothing starts at launch. Saved Claude/Codex/Antigravity sessions are listed and resume on click
  (`claude --resume <id>`, `codex resume <id>`, `agy --conversation <id>`); shells aren't kept.
  Antigravity cost / model aren't read from its conversations (not implemented).
- **New Claude session:** `--permission-mode auto`.
  - **Unrestricted:** `--dangerously-skip-permissions`.
  - **With settings:** model, effort, CLI params with history, 1M context.
  - **Codex:** `--dangerously-bypass-approvals-and-sandbox`, `-c model_reasoning_effort=…`.
- **Session names:** new, unnamed agent sessions are asked to call `rename_session` after the first task (`agents::NAME_SESSION_TEXT`, Options > General > "Ask agents to name new sessions…"). Claude gets it via `--append-system-prompt`, Codex via `-c developer_instructions="..."` (skipped if the user's params already set `developer_instructions`). Other agents aren't wired to the MCP server, so they don't get it. Not sent on resume.
- **Conversation ids:** found by scanning for new transcripts after spawn (no `--session-id`); then kept current by the SessionStart hook (changes on `/clear`).
- **Hook events and attention:** events go through files (no named pipe).
- **Archive:** `SavedSession.archived` / `.used` (last run, set on save / park); `App::auto_archive`
  (sidebar.rs, every frame in housekeeping) archives listed, unpinned sessions past
  `Settings.keep_sessions` (default 20, per project, running ones count). "Restore to list" sets
  `used = now` so the limit doesn't re-archive it.
- **Paste:** Explorer files → quoted paths; bitmap → PNG path; else text (bracketed when multi-line).

## User preferences learned

- **Style:** ASM Test Bench's blue scheme (accent `#0078D4`); About page styled like ASM Test Bench's.
- **Never name the internal app this was originally modelled on**: not in code, comments, docs, UI, tests or commit messages (the repo is public).
- **Tool names:** MCP tools have no prefix (server `ai-terminal`).
- **Defaults:** inline previews "Never over text", max 320 px.
- **Workflow:** wants things working end to end; tests in their own instance and sends screenshots.

## Open items / known gaps

- **Hover on `[Image #N]`:** the simulated-mouse test didn't show the tooltip (path hover did). Unconfirmed; ask the user what they hovered if they report it.
- **Untested:**
  - Ctrl+click links and the editor launch commands (`code -g`, `rider --line`)
  - drag-reorder of tabs and pinned sessions
  - snippet palette insertion
  - plugin install / uninstall / marketplace add and remove (these change the user's real setup; do them only on request)
  - toast, sound, taskbar badge when unfocused
  - messaging and handoff MCP tools (only `rename_session` and `list_sessions` were run end to end)
  - Codex sessions in general
- **Not built:** Notes and Drafts plugins (with their MCP tools); mobile bridge; chat sessions (out of scope).
- **Cost:** sessions saved before cost tracking show no cost until resumed. Prices are only as current as `assets/pricing.json`.
- **Hidden window:** MCP tool calls are answered on the UI thread; a window hidden in the tray may not tick, so calls could time out (20 s).
- **Unbuilt ideas the user picked from earlier but deferred:** split view (#4), capture-region-and-paste (#5), Notes/Drafts (#8).
- **Framework:** user asked GPUI vs egui; recommendation was to stay on egui.

## Handy paths

- Settings: `%APPDATA%\AI Terminal\data\app.ron` (test: `AI Terminal Test`)
- Runtime: `%LOCALAPPDATA%\AI_Terminal\` (events, hooks-*.json, mcp.json, logs)
- Pasted images: `%TEMP%\AI_Terminal\paste\`
