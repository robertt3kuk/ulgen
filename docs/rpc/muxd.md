# muxd RPC Contract (v0)

Transport: newline-delimited JSON (NDJSON) over local Unix domain socket.

Socket path policy:
- Existing non-socket paths are rejected.
- Existing active sockets are rejected.
- Parent directory must be private (`0700`) or sticky-bit protected (for example `/tmp`).

## Implemented methods (`v0`)

- `workspace.list`
- `workspace.create`
- `workspace.select`
- `pane.split`
- `pane.focus`
- `surface.send_text`
- `session.detach`
- `session.attach`
- `sync.set_scope`

Behavior notes:
- `session.detach` and `session.attach` are idempotent in `v0`; unknown sessions are treated as no-op for compatibility.
- `surface.send_text` resolves targets from the active `sync_scope` and excludes detached sessions internally.
- If scope resolution yields no attached sessions, `surface.send_text` returns `INVALID_STATE`.
- `surface.send_text` keeps the `v0` empty result shape (`{}`).
- `pane.split` direction is interpreted as insertion/focus order in `v0`:
  - `left`/`up` inserts before the active pane
  - `right`/`down` inserts after the active pane

## Planned (not implemented yet)

- `workspace.close`
- `tab.create`
- `tab.select`

Unimplemented methods currently return `METHOD_NOT_FOUND`.

## Request envelope

```json
{"id":"req-1","v":"v0","method":"workspace.list","params":{}}
```

Notes:
- `id` is required and must be a string.
- `v` defaults to `"v0"` when omitted.
- `params` defaults to `{}`.

## Success response envelope

```json
{"id":"req-1","ok":true,"result":{"workspaces":[]}}
```

## Error response envelope

```json
{"id":"req-1","ok":false,"error":{"code":"NOT_FOUND","message":"workspace not found"}}
```

## Error codes

- `PARSE_ERROR`
- `INVALID_REQUEST`
- `UNSUPPORTED_VERSION`
- `METHOD_NOT_FOUND`
- `INVALID_PARAMS`
- `NOT_FOUND`
- `INVALID_STATE`
- `REQUEST_TOO_LARGE`
- `INTERNAL`
