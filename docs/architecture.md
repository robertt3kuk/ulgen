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

## Stability contracts

- `muxd` RPC methods are versioned.
- settings schema remains backward compatible per minor release.
- permission policy defaults are explicit and auditable.
