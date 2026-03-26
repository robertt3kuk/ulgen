use std::fs;
use std::io::{BufRead, Write};
use std::path::Path;

#[cfg(unix)]
use std::io::BufReader;
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{MuxError, MuxRequest, MuxResponse, MuxRpc, SplitDirection, SyncScope};

pub const RPC_VERSION_V0: &str = "v0";
pub const DEFAULT_MAX_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RpcErrorCode {
    ParseError,
    InvalidRequest,
    UnsupportedVersion,
    MethodNotFound,
    InvalidParams,
    NotFound,
    InvalidState,
    RequestTooLarge,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcErrorBody {
    pub code: RpcErrorCode,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcResponseEnvelope {
    pub id: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcErrorBody>,
}

impl RpcResponseEnvelope {
    fn success(id: Option<String>, result: Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    fn failure(id: Option<String>, code: RpcErrorCode, message: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(RpcErrorBody {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct RpcRequestEnvelopeRaw {
    id: Option<String>,
    #[serde(default = "default_rpc_version")]
    v: String,
    method: Option<String>,
    #[serde(default = "default_params_object")]
    params: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketApiError {
    Io(String),
    Serialization(String),
}

impl std::fmt::Display for SocketApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => write!(f, "io error: {message}"),
            Self::Serialization(message) => write!(f, "serialization error: {message}"),
        }
    }
}

impl std::error::Error for SocketApiError {}

pub fn handle_rpc_line<R: MuxRpc>(
    rpc: &mut R,
    line: &str,
    max_request_bytes: usize,
) -> RpcResponseEnvelope {
    if line.len() > max_request_bytes {
        return RpcResponseEnvelope::failure(
            None,
            RpcErrorCode::RequestTooLarge,
            format!(
                "request exceeds max size ({} > {})",
                line.len(),
                max_request_bytes
            ),
        );
    }

    let request_json: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            return RpcResponseEnvelope::failure(
                None,
                RpcErrorCode::ParseError,
                format!("invalid json request: {error}"),
            );
        }
    };
    let request_id = request_json
        .get("id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let raw: RpcRequestEnvelopeRaw = match serde_json::from_value(request_json) {
        Ok(value) => value,
        Err(error) => {
            return RpcResponseEnvelope::failure(
                request_id,
                RpcErrorCode::InvalidRequest,
                format!("invalid request envelope: {error}"),
            );
        }
    };

    if raw.id.is_none() {
        return RpcResponseEnvelope::failure(
            None,
            RpcErrorCode::InvalidRequest,
            "request id must be a string",
        );
    }

    if raw.v != RPC_VERSION_V0 {
        return RpcResponseEnvelope::failure(
            raw.id,
            RpcErrorCode::UnsupportedVersion,
            format!("unsupported rpc version: {}", raw.v),
        );
    }

    let method = match raw.method {
        Some(method) if !method.trim().is_empty() => method,
        _ => {
            return RpcResponseEnvelope::failure(
                raw.id,
                RpcErrorCode::InvalidRequest,
                "request method is required",
            );
        }
    };

    let request = match map_method_to_request(&method, raw.params, raw.id.clone()) {
        Ok(request) => request,
        Err(response) => return response,
    };

    match rpc.handle(request) {
        Ok(response) => RpcResponseEnvelope::success(raw.id, map_response_to_result(response)),
        Err(error) => map_mux_error(raw.id, error),
    }
}

pub fn serve_connection<R: MuxRpc, Reader: BufRead, Writer: Write>(
    rpc: &mut R,
    reader: &mut Reader,
    writer: &mut Writer,
    max_request_bytes: usize,
) -> Result<(), SocketApiError> {
    loop {
        let line_bytes = match read_line_bounded(reader, max_request_bytes)? {
            BoundedReadResult::Eof => break,
            BoundedReadResult::RequestTooLarge => {
                let response = RpcResponseEnvelope::failure(
                    None,
                    RpcErrorCode::RequestTooLarge,
                    format!("request exceeds max size ({max_request_bytes})"),
                );
                write_response_line(writer, &response)?;
                continue;
            }
            BoundedReadResult::Line(line) => line,
        };

        let line = match std::str::from_utf8(&line_bytes) {
            Ok(line) => line.trim(),
            Err(error) => {
                let response = RpcResponseEnvelope::failure(
                    None,
                    RpcErrorCode::ParseError,
                    format!("request is not valid utf-8: {error}"),
                );
                write_response_line(writer, &response)?;
                continue;
            }
        };

        if line.is_empty() {
            continue;
        }

        let response = handle_rpc_line(rpc, line, max_request_bytes);
        write_response_line(writer, &response)?;
    }

    writer
        .flush()
        .map_err(|error| SocketApiError::Io(format!("flush response writer: {error}")))?;
    Ok(())
}

#[cfg(unix)]
pub fn serve_unix_socket_once<R: MuxRpc>(
    rpc: &mut R,
    socket_path: impl AsRef<Path>,
    max_request_bytes: usize,
) -> Result<(), SocketApiError> {
    let socket_path = socket_path.as_ref();

    if let Some(parent) = socket_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| {
                SocketApiError::Io(format!("create socket dir {}: {error}", parent.display()))
            })?;
            let mode = fs::metadata(parent)
                .map_err(|error| {
                    SocketApiError::Io(format!(
                        "inspect socket dir permissions {}: {error}",
                        parent.display()
                    ))
                })?
                .permissions()
                .mode();
            let group_or_other_writable = mode & 0o022 != 0;
            let sticky_bit_set = mode & 0o1000 != 0;
            if group_or_other_writable && !sticky_bit_set {
                return Err(SocketApiError::Io(format!(
                    "socket dir {} must be private (0700) or sticky-bit protected",
                    parent.display()
                )));
            }
        }
    }

    if socket_path.exists() {
        let metadata = fs::symlink_metadata(socket_path).map_err(|error| {
            SocketApiError::Io(format!(
                "inspect existing socket path {}: {error}",
                socket_path.display()
            ))
        })?;
        if !metadata.file_type().is_socket() {
            return Err(SocketApiError::Io(format!(
                "existing socket path {} is not a unix socket",
                socket_path.display()
            )));
        }

        if UnixStream::connect(socket_path).is_ok() {
            return Err(SocketApiError::Io(format!(
                "socket path {} is already active",
                socket_path.display()
            )));
        }

        fs::remove_file(socket_path).map_err(|error| {
            SocketApiError::Io(format!(
                "remove stale socket {}: {error}",
                socket_path.display()
            ))
        })?;
    }

    let listener = UnixListener::bind(socket_path).map_err(|error| {
        SocketApiError::Io(format!(
            "bind unix socket {}: {error}",
            socket_path.display()
        ))
    })?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        SocketApiError::Io(format!(
            "set socket permissions {}: {error}",
            socket_path.display()
        ))
    })?;

    let accept_result = listener
        .accept()
        .map_err(|error| SocketApiError::Io(format!("accept unix socket client: {error}")))?;
    let (mut stream, _) = accept_result;
    let read_stream = stream
        .try_clone()
        .map_err(|error| SocketApiError::Io(format!("clone unix socket stream: {error}")))?;
    let mut reader = BufReader::new(read_stream);

    let serve_result = serve_connection(rpc, &mut reader, &mut stream, max_request_bytes);
    let _ = fs::remove_file(socket_path);
    serve_result
}

#[cfg(not(unix))]
pub fn serve_unix_socket_once<R: MuxRpc>(
    _rpc: &mut R,
    _socket_path: impl AsRef<Path>,
    _max_request_bytes: usize,
) -> Result<(), SocketApiError> {
    Err(SocketApiError::Io(
        "unix socket transport is not available on this platform".to_string(),
    ))
}

fn default_rpc_version() -> String {
    RPC_VERSION_V0.to_string()
}

fn default_params_object() -> Value {
    json!({})
}

fn map_method_to_request(
    method: &str,
    params: Value,
    request_id: Option<String>,
) -> Result<MuxRequest, RpcResponseEnvelope> {
    match method {
        "workspace.list" => Ok(MuxRequest::WorkspaceList),
        "workspace.create" => {
            #[derive(Deserialize)]
            struct Params {
                name: String,
            }
            let params: Params = parse_params(params, method, request_id.clone())?;
            Ok(MuxRequest::WorkspaceCreate { name: params.name })
        }
        "workspace.select" => {
            #[derive(Deserialize)]
            struct Params {
                workspace_id: String,
            }
            let params: Params = parse_params(params, method, request_id.clone())?;
            Ok(MuxRequest::WorkspaceSelect {
                workspace_id: params.workspace_id,
            })
        }
        "pane.split" => {
            #[derive(Deserialize)]
            struct Params {
                direction: String,
            }
            let params: Params = parse_params(params, method, request_id.clone())?;
            let direction = match params.direction.as_str() {
                "left" => SplitDirection::Left,
                "right" => SplitDirection::Right,
                "up" => SplitDirection::Up,
                "down" => SplitDirection::Down,
                other => {
                    return Err(RpcResponseEnvelope::failure(
                        request_id,
                        RpcErrorCode::InvalidParams,
                        format!("unsupported split direction: {other}"),
                    ));
                }
            };
            Ok(MuxRequest::PaneSplit { direction })
        }
        "pane.focus" => {
            #[derive(Deserialize)]
            struct Params {
                pane_id: String,
            }
            let params: Params = parse_params(params, method, request_id.clone())?;
            Ok(MuxRequest::PaneFocus {
                pane_id: params.pane_id,
            })
        }
        "surface.send_text" => {
            #[derive(Deserialize)]
            struct Params {
                text: String,
            }
            let params: Params = parse_params(params, method, request_id.clone())?;
            Ok(MuxRequest::SurfaceSendText { text: params.text })
        }
        "session.detach" => {
            #[derive(Deserialize)]
            struct Params {
                session_id: String,
            }
            let params: Params = parse_params(params, method, request_id.clone())?;
            Ok(MuxRequest::SessionDetach {
                session_id: params.session_id,
            })
        }
        "session.attach" => {
            #[derive(Deserialize)]
            struct Params {
                session_id: String,
            }
            let params: Params = parse_params(params, method, request_id.clone())?;
            Ok(MuxRequest::SessionAttach {
                session_id: params.session_id,
            })
        }
        "sync.set_scope" => {
            #[derive(Deserialize)]
            struct Params {
                scope: Option<String>,
            }
            let params: Params = parse_params(params, method, request_id.clone())?;
            let scope = match params.scope.as_deref() {
                None => None,
                Some("current_tab") => Some(SyncScope::CurrentTab),
                Some("all_tabs") => Some(SyncScope::AllTabs),
                Some("all_workspaces") => Some(SyncScope::AllWorkspaces),
                Some(other) => {
                    return Err(RpcResponseEnvelope::failure(
                        request_id,
                        RpcErrorCode::InvalidParams,
                        format!("unsupported sync scope: {other}"),
                    ));
                }
            };
            Ok(MuxRequest::SyncSetScope { scope })
        }
        _ => Err(RpcResponseEnvelope::failure(
            request_id,
            RpcErrorCode::MethodNotFound,
            format!("unknown method: {method}"),
        )),
    }
}

fn parse_params<T: for<'de> Deserialize<'de>>(
    params: Value,
    method: &str,
    request_id: Option<String>,
) -> Result<T, RpcResponseEnvelope> {
    serde_json::from_value(params).map_err(|error| {
        RpcResponseEnvelope::failure(
            request_id,
            RpcErrorCode::InvalidParams,
            format!("invalid params for {method}: {error}"),
        )
    })
}

fn map_response_to_result(response: MuxResponse) -> Value {
    match response {
        MuxResponse::WorkspaceList { workspaces } => json!({ "workspaces": workspaces }),
        MuxResponse::WorkspaceCreate { workspace } => json!({ "workspace": workspace }),
        MuxResponse::WorkspaceSelect { workspace_id } => json!({ "workspace_id": workspace_id }),
        MuxResponse::PaneSplit { pane_id } => json!({ "pane_id": pane_id }),
        MuxResponse::PaneFocus { pane_id } => json!({ "pane_id": pane_id }),
        MuxResponse::SurfaceSendText { .. }
        | MuxResponse::SessionDetach
        | MuxResponse::SessionAttach
        | MuxResponse::SyncSetScope => {
            json!({})
        }
    }
}

fn write_response_line<Writer: Write>(
    writer: &mut Writer,
    response: &RpcResponseEnvelope,
) -> Result<(), SocketApiError> {
    let payload = serde_json::to_string(response).map_err(|error| {
        SocketApiError::Serialization(format!("serialize response envelope: {error}"))
    })?;
    writer
        .write_all(payload.as_bytes())
        .map_err(|error| SocketApiError::Io(format!("write response payload: {error}")))?;
    writer
        .write_all(b"\n")
        .map_err(|error| SocketApiError::Io(format!("write response newline: {error}")))?;
    writer
        .flush()
        .map_err(|error| SocketApiError::Io(format!("flush response writer: {error}")))?;
    Ok(())
}

enum BoundedReadResult {
    Eof,
    RequestTooLarge,
    Line(Vec<u8>),
}

fn read_line_bounded<Reader: BufRead>(
    reader: &mut Reader,
    max_request_bytes: usize,
) -> Result<BoundedReadResult, SocketApiError> {
    let hard_limit = max_request_bytes.saturating_add(2);
    let mut line = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| SocketApiError::Io(format!("read request bytes: {error}")))?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(BoundedReadResult::Eof)
            } else {
                Ok(BoundedReadResult::Line(line))
            };
        }

        let newline_pos = available.iter().position(|byte| *byte == b'\n');
        let chunk_len = newline_pos.map_or(available.len(), |index| index + 1);
        if line.len() + chunk_len > hard_limit {
            reader.consume(chunk_len);
            if newline_pos.is_none() {
                drain_until_newline(reader)?;
            }
            return Ok(BoundedReadResult::RequestTooLarge);
        }

        line.extend_from_slice(&available[..chunk_len]);
        reader.consume(chunk_len);

        if newline_pos.is_some() {
            break;
        }
    }

    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }

    if line.len() > max_request_bytes {
        return Ok(BoundedReadResult::RequestTooLarge);
    }

    Ok(BoundedReadResult::Line(line))
}

fn drain_until_newline<Reader: BufRead>(reader: &mut Reader) -> Result<(), SocketApiError> {
    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| SocketApiError::Io(format!("drain oversized request: {error}")))?;
        if available.is_empty() {
            return Ok(());
        }

        if let Some(position) = available.iter().position(|byte| *byte == b'\n') {
            reader.consume(position + 1);
            return Ok(());
        }

        let len = available.len();
        reader.consume(len);
    }
}

fn map_mux_error(request_id: Option<String>, error: MuxError) -> RpcResponseEnvelope {
    match error {
        MuxError::NotFound(message) => {
            RpcResponseEnvelope::failure(request_id, RpcErrorCode::NotFound, message)
        }
        MuxError::InvalidState(message) => {
            RpcResponseEnvelope::failure(request_id, RpcErrorCode::InvalidState, message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MuxState;
    use std::io::Cursor;

    fn parse_response(line: &str) -> RpcResponseEnvelope {
        serde_json::from_str(line).expect("response envelope")
    }

    #[test]
    fn line_handler_can_create_workspace_then_list() {
        let mut mux = MuxState::new();
        let create = handle_rpc_line(
            &mut mux,
            r#"{"id":"req-1","v":"v0","method":"workspace.create","params":{"name":"api"}}"#,
            DEFAULT_MAX_REQUEST_BYTES,
        );
        assert!(create.ok);
        assert_eq!(create.id.as_deref(), Some("req-1"));
        assert_eq!(
            create.result.unwrap()["workspace"]["name"].as_str(),
            Some("api")
        );

        let list = handle_rpc_line(
            &mut mux,
            r#"{"id":"req-2","v":"v0","method":"workspace.list","params":{}}"#,
            DEFAULT_MAX_REQUEST_BYTES,
        );
        assert!(list.ok);
        assert_eq!(
            list.result.unwrap()["workspaces"].as_array().unwrap().len(),
            2
        );
    }

    #[test]
    fn line_handler_rejects_invalid_json() {
        let mut mux = MuxState::new();
        let response = handle_rpc_line(&mut mux, r#"{"id":"req-1""#, DEFAULT_MAX_REQUEST_BYTES);
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, RpcErrorCode::ParseError);
    }

    #[test]
    fn line_handler_rejects_unsupported_version() {
        let mut mux = MuxState::new();
        let response = handle_rpc_line(
            &mut mux,
            r#"{"id":"req-1","v":"v9","method":"workspace.list","params":{}}"#,
            DEFAULT_MAX_REQUEST_BYTES,
        );
        assert!(!response.ok);
        assert_eq!(
            response.error.unwrap().code,
            RpcErrorCode::UnsupportedVersion
        );
    }

    #[test]
    fn line_handler_rejects_unknown_method() {
        let mut mux = MuxState::new();
        let response = handle_rpc_line(
            &mut mux,
            r#"{"id":"req-1","v":"v0","method":"tab.create","params":{}}"#,
            DEFAULT_MAX_REQUEST_BYTES,
        );
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, RpcErrorCode::MethodNotFound);
    }

    #[test]
    fn line_handler_rejects_invalid_params() {
        let mut mux = MuxState::new();
        let response = handle_rpc_line(
            &mut mux,
            r#"{"id":"req-1","v":"v0","method":"workspace.create","params":{"missing":"name"}}"#,
            DEFAULT_MAX_REQUEST_BYTES,
        );
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, RpcErrorCode::InvalidParams);
    }

    #[test]
    fn line_handler_accepts_missing_params_for_sync_set_scope() {
        let mut mux = MuxState::new();
        let response = handle_rpc_line(
            &mut mux,
            r#"{"id":"req-1","v":"v0","method":"sync.set_scope"}"#,
            DEFAULT_MAX_REQUEST_BYTES,
        );
        assert!(response.ok);
    }

    #[test]
    fn line_handler_supports_pane_focus() {
        let mut mux = MuxState::new();
        let _ = handle_rpc_line(
            &mut mux,
            r#"{"id":"req-1","v":"v0","method":"pane.split","params":{"direction":"right"}}"#,
            DEFAULT_MAX_REQUEST_BYTES,
        );
        let response = handle_rpc_line(
            &mut mux,
            r#"{"id":"req-2","v":"v0","method":"pane.focus","params":{"pane_id":"pane-3"}}"#,
            DEFAULT_MAX_REQUEST_BYTES,
        );
        assert!(response.ok);
        assert_eq!(response.result.unwrap()["pane_id"].as_str(), Some("pane-3"));
    }

    #[test]
    fn line_handler_surface_send_text_keeps_v0_empty_result_shape() {
        let mut mux = MuxState::new();
        let _ = handle_rpc_line(
            &mut mux,
            r#"{"id":"req-1","v":"v0","method":"pane.split","params":{"direction":"right"}}"#,
            DEFAULT_MAX_REQUEST_BYTES,
        );
        let _ = handle_rpc_line(
            &mut mux,
            r#"{"id":"req-2","v":"v0","method":"sync.set_scope","params":{"scope":"current_tab"}}"#,
            DEFAULT_MAX_REQUEST_BYTES,
        );

        let response = handle_rpc_line(
            &mut mux,
            r#"{"id":"req-3","v":"v0","method":"surface.send_text","params":{"text":"echo hi"}}"#,
            DEFAULT_MAX_REQUEST_BYTES,
        );
        assert!(response.ok);
        assert_eq!(response.result.unwrap(), json!({}));
    }

    #[test]
    fn line_handler_session_detach_unknown_session_is_noop_in_v0() {
        let mut mux = MuxState::new();
        let response = handle_rpc_line(
            &mut mux,
            r#"{"id":"req-1","v":"v0","method":"session.detach","params":{"session_id":"session-missing"}}"#,
            DEFAULT_MAX_REQUEST_BYTES,
        );
        assert!(response.ok);
        assert_eq!(response.result.unwrap(), json!({}));
    }

    #[test]
    fn serve_connection_handles_multiple_requests() {
        let mut mux = MuxState::new();
        let input = r#"{"id":"req-1","v":"v0","method":"workspace.create","params":{"name":"api"}}
{"id":"req-2","v":"v0","method":"workspace.select","params":{"workspace_id":"ws-6"}}
{"id":"req-3","v":"v0","method":"workspace.list","params":{}}
"#;
        let mut reader = Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        serve_connection(
            &mut mux,
            &mut reader,
            &mut output,
            DEFAULT_MAX_REQUEST_BYTES,
        )
        .unwrap();

        let payload = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = payload.lines().collect();
        assert_eq!(lines.len(), 3);

        let r1 = parse_response(lines[0]);
        let r2 = parse_response(lines[1]);
        let r3 = parse_response(lines[2]);
        assert!(r1.ok);
        assert!(r2.ok);
        assert!(r3.ok);
        assert_eq!(
            r3.result.unwrap()["workspaces"].as_array().unwrap().len(),
            2
        );
    }

    #[test]
    fn serve_connection_rejects_oversized_requests_without_failing_followups() {
        let mut mux = MuxState::new();
        let oversized = format!(
            r#"{{"id":"req-1","v":"v0","method":"surface.send_text","params":{{"text":"{}"}}}}"#,
            "x".repeat(2048)
        );
        let input = format!(
            "{oversized}\n{{\"id\":\"req-2\",\"v\":\"v0\",\"method\":\"workspace.list\",\"params\":{{}}}}\n"
        );
        let mut reader = Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        serve_connection(&mut mux, &mut reader, &mut output, 512).unwrap();

        let payload = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = payload.lines().collect();
        assert_eq!(lines.len(), 2);

        let r1 = parse_response(lines[0]);
        let r2 = parse_response(lines[1]);
        assert!(!r1.ok);
        assert_eq!(r1.error.unwrap().code, RpcErrorCode::RequestTooLarge);
        assert!(r2.ok);
    }

    #[test]
    fn serve_connection_allows_exact_size_payload_with_newline_delimiter() {
        let mut mux = MuxState::new();
        let line = r#"{"id":"req-1","v":"v0","method":"workspace.list","params":{}}"#;
        let input = format!("{line}\n");
        let mut reader = Cursor::new(input.as_bytes());
        let mut output = Vec::new();

        serve_connection(&mut mux, &mut reader, &mut output, line.len()).unwrap();

        let payload = String::from_utf8(output).unwrap();
        let response = parse_response(payload.trim());
        assert!(response.ok);
        assert_eq!(response.id.as_deref(), Some("req-1"));
    }

    #[test]
    fn serve_connection_allows_exact_size_payload_with_crlf_delimiter() {
        let mut mux = MuxState::new();
        let line = r#"{"id":"req-1","v":"v0","method":"workspace.list","params":{}}"#;
        let input = format!("{line}\r\n");
        let mut reader = Cursor::new(input.as_bytes());
        let mut output = Vec::new();

        serve_connection(&mut mux, &mut reader, &mut output, line.len()).unwrap();

        let payload = String::from_utf8(output).unwrap();
        let response = parse_response(payload.trim());
        assert!(response.ok);
        assert_eq!(response.id.as_deref(), Some("req-1"));
    }

    #[test]
    fn serve_connection_allows_exact_size_payload_with_fragmented_crlf() {
        let mut mux = MuxState::new();
        let line = r#"{"id":"req-1","v":"v0","method":"workspace.list","params":{}}"#;
        let input = format!("{line}\r\n");
        let cursor = Cursor::new(input.as_bytes());
        let mut reader = std::io::BufReader::with_capacity(1, cursor);
        let mut output = Vec::new();

        serve_connection(&mut mux, &mut reader, &mut output, line.len()).unwrap();

        let payload = String::from_utf8(output).unwrap();
        let response = parse_response(payload.trim());
        assert!(response.ok);
        assert_eq!(response.id.as_deref(), Some("req-1"));
    }
}
