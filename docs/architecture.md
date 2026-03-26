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
- Startup loads previous window/workspace metadata when the file exists, otherwise bootstraps defaults.
- Command routing entrypoints are exposed through app-shell command ids (for example `window.new`, `workspace.new`).
- Keyboard routing resolves active profile defaults (`Warp`/`Tmux`, lowercase aliases accepted) plus user overrides, then dispatches to command ids.
- Conflicting key overrides are rejected deterministically and reported while valid mappings remain active.

## Stability contracts

- `muxd` RPC methods are versioned.
- settings schema remains backward compatible per minor release.
- permission policy defaults are explicit and auditable.
