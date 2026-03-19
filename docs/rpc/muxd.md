# muxd RPC Contract (Draft v0)

Transport: newline-delimited JSON over local Unix domain socket (or Windows named pipe equivalent).

## Methods

- `workspace.list`
- `workspace.create`
- `workspace.select`
- `workspace.close`
- `tab.create`
- `tab.select`
- `pane.split`
- `pane.focus`
- `surface.send_text`
- `session.detach`
- `session.attach`
- `sync.set_scope`

## Request envelope

```json
{"id":"req-1","method":"workspace.list","params":{}}
```

## Response envelope

```json
{"id":"req-1","ok":true,"result":{"workspaces":[]}}
```

## Error envelope

```json
{"id":"req-1","ok":false,"error":{"code":"NOT_FOUND","message":"workspace not found"}}
```
