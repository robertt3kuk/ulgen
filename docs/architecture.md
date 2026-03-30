# Ulgen Architecture (v1)

## Core layers

1. UI shell (`ulgen-app`): window lifecycle, sidebar, command palette, block views.
2. Domain (`ulgen-domain`): canonical entities for workspaces, tabs, panes, surfaces, blocks.
3. Mux daemon (`ulgen-muxd`): persistent multiplexer, detach/attach, socket RPC.
4. Terminal backend (`ulgen-pty`): OS-specific process control via PTY/ConPTY.
5. ACP bridge (`ulgen-acp`): external agent session lifecycle and terminal bridging.
6. Notifications (`ulgen-notify`): in-app event stream plus OS bridge adapters.

## Data flow

- User action in UI -> command registry action -> mux request -> backend result -> block update -> notification event.
- ACP action -> ACP terminal bridge -> mux request -> PTY output -> block + ACP update stream.

## App shell bootstrap and restore

- App shell state defaults to a user-scoped OS path (Linux: `XDG_STATE_HOME/ulgen` or `~/.local/state/ulgen`, macOS: `~/Library/Application Support/Ulgen`, Windows: `LOCALAPPDATA\\Ulgen`) with override via `ULGEN_STATE_PATH`.
- Startup loads previous window/workspace metadata and block history when the file exists, otherwise bootstraps defaults.
- Command routing entrypoints are exposed through app-shell command ids (for example `window.new`, `workspace.new`).
- Keyboard routing resolves active profile defaults (`Warp`/`Tmux`, lowercase aliases accepted) plus user overrides, then dispatches to command ids.
- Conflicting key overrides are rejected deterministically and reported while valid mappings remain active.

## Block engine lifecycle (M3-1)

- Block runs are persisted in app-shell state as append-only command history.
- Runtime index contracts:
  - block id -> block position lookup
  - session id -> ordered block id list
- Block lifecycle APIs in `ulgen-app`:
  - start command block for active/explicit session
  - append ordered output chunks (`chunk_id` monotonic per block)
  - finish with terminal status (`Succeeded`/`Failed`/`Cancelled`)
  - rerun original input or rerun with edited input
  - replay merged output and query session-scoped history

## Sidebar navigation contract (M3-2)

- Sidebar exposes active-window hierarchy as `workspace -> tabs -> panes`.
- Sidebar position is user-configurable and persisted (`Left` default, toggle to `Right`).
- Navigation capabilities:
  - select node by id (click-like targeting)
  - next/previous keyboard traversal across flattened tree order
  - fuzzy-match and jump by node title/id

## Command palette contract (M3-3)

- Palette unifies executable commands and quick-switch entities into a single searchable surface.
- Palette item ids are stable and typed:
  - `cmd:<command_id>` for registered command actions
  - `node:<sidebar_node_id>` for workspace/tab/pane quick switch targets
- Search ranking baseline:
  - exact match > prefix match > substring match
  - recent selections apply a deterministic recency boost
- Executing a palette item must route through existing command/sidebar handlers and record recency history.
- Recent palette selections are persisted in app state for restore and next-start discoverability.

## Block notification contract (M3-4)

- Block lifecycle publishes notification events on terminal status transitions:
  - `Succeeded` -> `TaskDone`
  - `Failed`/`Cancelled` -> `TaskFailed`
  - explicit approval gate -> `ApprovalRequired`
- Notification events carry `block_id` for deep-link resolution back into app shell context.
- App shell exposes deterministic block navigation resolution using `block_id -> session -> window/workspace/tab/pane`.
- Notification transport policy remains driven by settings (`in-app`, `os`, or both) via `NotificationBus`.

## ACP transport contract (M4-1)

- ACP server lifecycle gates all session operations behind `initialize`.
- Session lifecycle methods:
  - `session/new` -> create deterministic session identity
  - `session/load` -> load existing session metadata
  - `session/prompt` -> accept prompt and produce session update
  - `session/cancel` -> cancel session and emit cancellation update
  - `session/updates` -> drain update stream for a session
- Terminal lifecycle methods (M4-2):
  - `terminal/create` -> create a session-scoped terminal handle (`output_byte_limit` bounded, `outputByteLimit` alias accepted)
  - `terminal/input` -> submit raw input bytes with source attribution (`agent`/`user`)
  - `terminal/output` -> return bounded output plus optional exit status
  - `terminal/wait_for_exit` -> return terminal exit state when available
  - `terminal/kill` -> terminate terminal process for owning session
  - `terminal/release` -> kill (best effort for already-exited/not-found) then untrack ownership handle
- Interactive app mode (M4-3):
  - alternate-screen transitions (`?1049`, `?1048`, `?1047`, `?47` enter/exit) are detected from terminal output stream
  - user-sourced input can claim interactive control while in alternate screen
  - agent operations that would disrupt user control are denied while interactive mode is user-owned
- Permission gating baseline (M4-4):
  - ownership validation is evaluated before backend output reads for cross-session protection
  - user-owned interactive sessions deny agent `terminal/input`, `terminal/output`, `terminal/kill`, and `terminal/release`
  - denied operations surface as terminal operation errors (`-32005`) for auditable client handling
- JSON-RPC stdio transport accepts line-delimited request frames and returns one JSON response frame per request (notifications return no response frame).
- JSON-RPC `id` values support `string` and `number`; parse/invalid-request failures return `id: null`.
- Error contract:
  - parse errors (`-32700`)
  - invalid request/method/params (`-32600/-32601/-32602`)
  - ACP server state errors (`-32001` not initialized, `-32002` unsupported protocol, `-32004` session missing for session-scoped operations with unknown ids, `-32005` terminal operation failure including ownership/backend/runtime errors)

## Stability contracts

- `muxd` RPC methods are versioned.
- settings schema remains backward compatible per minor release.
- permission policy defaults are explicit and auditable.
