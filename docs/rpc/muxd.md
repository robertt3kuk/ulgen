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
