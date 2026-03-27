use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{json, Number, Value};
use ulgen_pty::{TerminalExitStatus, TerminalId};

pub const ACP_PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientCapabilities {
    pub terminal: bool,
    pub fs_read_text_file: bool,
    pub fs_write_text_file: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializeRequest {
    pub protocol_version: u32,
    pub client_capabilities: ClientCapabilities,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializeResponse {
    pub protocol_version: u32,
    pub terminal_supported: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub cwd: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    Ask,
    Allow,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalCreateRequest {
    pub session_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub output_byte_limit: usize,
}

pub trait AcpTerminalBridge {
    fn terminal_create(&mut self, request: TerminalCreateRequest) -> Result<TerminalId, String>;
    fn terminal_output(
        &self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<(String, Option<TerminalExitStatus>), String>;
    fn terminal_wait_for_exit(
        &self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<Option<TerminalExitStatus>, String>;
    fn terminal_kill(&mut self, session_id: &str, terminal_id: &TerminalId) -> Result<(), String>;
    fn terminal_release(
        &mut self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionUpdate {
    SessionCreated {
        session_id: String,
        cwd: String,
    },
    PromptReceived {
        session_id: String,
        prompt_id: String,
        prompt: String,
    },
    SessionCancelled {
        session_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcpServerError {
    NotInitialized,
    UnsupportedProtocol { requested: u32, supported: u32 },
    SessionMissing(String),
}

impl std::fmt::Display for AcpServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInitialized => write!(f, "ACP server is not initialized"),
            Self::UnsupportedProtocol {
                requested,
                supported,
            } => write!(
                f,
                "unsupported ACP protocol version: requested={requested} supported={supported}"
            ),
            Self::SessionMissing(session_id) => write!(f, "session missing: {session_id}"),
        }
    }
}

impl std::error::Error for AcpServerError {}

#[derive(Default)]
pub struct SessionRegistry {
    sessions: BTreeMap<String, Session>,
    updates: BTreeMap<String, VecDeque<SessionUpdate>>,
    next_session_seq: u64,
    next_prompt_seq: u64,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_session(&mut self, cwd: String) -> Session {
        self.next_session_seq += 1;
        let session_id = format!("sess-{}", self.next_session_seq);
        let session = Session {
            session_id: session_id.clone(),
            cwd: cwd.clone(),
        };
        self.sessions.insert(session_id.clone(), session.clone());
        self.updates
            .entry(session_id.clone())
            .or_default()
            .push_back(SessionUpdate::SessionCreated { session_id, cwd });
        session
    }

    pub fn load_session(&self, session_id: &str) -> Result<Session, AcpServerError> {
        self.sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| AcpServerError::SessionMissing(session_id.to_string()))
    }

    pub fn prompt_session(
        &mut self,
        session_id: &str,
        prompt: String,
    ) -> Result<String, AcpServerError> {
        if !self.sessions.contains_key(session_id) {
            return Err(AcpServerError::SessionMissing(session_id.to_string()));
        }

        self.next_prompt_seq += 1;
        let prompt_id = format!("prompt-{}", self.next_prompt_seq);
        self.updates
            .entry(session_id.to_string())
            .or_default()
            .push_back(SessionUpdate::PromptReceived {
                session_id: session_id.to_string(),
                prompt_id: prompt_id.clone(),
                prompt,
            });
        Ok(prompt_id)
    }

    pub fn cancel_session(&mut self, session_id: &str) -> Result<(), AcpServerError> {
        if self.sessions.remove(session_id).is_none() {
            return Err(AcpServerError::SessionMissing(session_id.to_string()));
        }

        self.updates
            .entry(session_id.to_string())
            .or_default()
            .push_back(SessionUpdate::SessionCancelled {
                session_id: session_id.to_string(),
            });
        Ok(())
    }

    pub fn drain_updates(
        &mut self,
        session_id: &str,
    ) -> Result<Vec<SessionUpdate>, AcpServerError> {
        if !self.sessions.contains_key(session_id) && !self.updates.contains_key(session_id) {
            return Err(AcpServerError::SessionMissing(session_id.to_string()));
        }
        let Some(queue) = self.updates.get_mut(session_id) else {
            return Ok(Vec::new());
        };

        let mut drained = Vec::new();
        while let Some(update) = queue.pop_front() {
            drained.push(update);
        }
        let should_remove_queue = queue.is_empty();
        if should_remove_queue {
            // Keep update map bounded under high session churn.
            self.updates.remove(session_id);
        }
        Ok(drained)
    }
}

#[derive(Default)]
pub struct AcpServer {
    initialized: bool,
    sessions: SessionRegistry,
}

impl AcpServer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn initialize(
        &mut self,
        request: InitializeRequest,
    ) -> Result<InitializeResponse, AcpServerError> {
        if request.protocol_version != ACP_PROTOCOL_VERSION {
            return Err(AcpServerError::UnsupportedProtocol {
                requested: request.protocol_version,
                supported: ACP_PROTOCOL_VERSION,
            });
        }

        self.initialized = true;
        Ok(InitializeResponse {
            protocol_version: ACP_PROTOCOL_VERSION,
            terminal_supported: true,
        })
    }

    pub fn new_session(&mut self, cwd: String) -> Result<Session, AcpServerError> {
        self.ensure_initialized()?;
        Ok(self.sessions.create_session(cwd))
    }

    pub fn load_session(&self, session_id: &str) -> Result<Session, AcpServerError> {
        self.ensure_initialized()?;
        self.sessions.load_session(session_id)
    }

    pub fn prompt_session(
        &mut self,
        session_id: &str,
        prompt: String,
    ) -> Result<String, AcpServerError> {
        self.ensure_initialized()?;
        self.sessions.prompt_session(session_id, prompt)
    }

    pub fn cancel_session(&mut self, session_id: &str) -> Result<(), AcpServerError> {
        self.ensure_initialized()?;
        self.sessions.cancel_session(session_id)
    }

    pub fn drain_session_updates(
        &mut self,
        session_id: &str,
    ) -> Result<Vec<SessionUpdate>, AcpServerError> {
        self.ensure_initialized()?;
        self.sessions.drain_updates(session_id)
    }

    fn ensure_initialized(&self) -> Result<(), AcpServerError> {
        if !self.initialized {
            return Err(AcpServerError::NotInitialized);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<JsonRpcId>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Null,
    Number(Number),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    fn success(id: JsonRpcId, result: Value) -> Self {
        Self {
            jsonrpc: jsonrpc_v2(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: JsonRpcId, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: jsonrpc_v2(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionNewRequest {
    pub cwd: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLoadRequest {
    pub session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPromptRequest {
    pub session_id: String,
    pub prompt: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPromptResponse {
    pub session_id: String,
    pub prompt_id: String,
    pub accepted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCancelRequest {
    pub session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCancelResponse {
    pub session_id: String,
    pub cancelled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUpdatesRequest {
    pub session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUpdatesResponse {
    pub session_id: String,
    pub updates: Vec<SessionUpdate>,
}

#[derive(Default)]
pub struct JsonRpcStdioTransport {
    server: AcpServer,
}

impl JsonRpcStdioTransport {
    pub fn new(server: AcpServer) -> Self {
        Self { server }
    }

    pub fn server(&self) -> &AcpServer {
        &self.server
    }

    pub fn server_mut(&mut self) -> &mut AcpServer {
        &mut self.server
    }

    pub fn handle_line(&mut self, line: &str) -> String {
        let parsed = match serde_json::from_str::<Value>(line) {
            Ok(value) => value,
            Err(error) => {
                return serialize_response(&JsonRpcResponse::error(
                    JsonRpcId::Null,
                    -32700,
                    format!("parse error: {error}"),
                ))
            }
        };

        let response_id = extract_response_id(&parsed);
        let request = match serde_json::from_value::<JsonRpcRequest>(parsed) {
            Ok(request) => request,
            Err(error) => {
                return serialize_response(&JsonRpcResponse::error(
                    response_id,
                    -32600,
                    format!("invalid request: {error}"),
                ))
            }
        };

        let request_id = request.id.clone();
        if request.jsonrpc != jsonrpc_v2() {
            return serialize_response(&JsonRpcResponse::error(
                request_id.unwrap_or(JsonRpcId::Null),
                -32600,
                "invalid request: jsonrpc must be '2.0'",
            ));
        }

        let is_notification = request.id.is_none();
        let response_id = request.id.clone().unwrap_or(JsonRpcId::Null);
        let response = match self.dispatch(request) {
            Ok(result) => JsonRpcResponse::success(response_id.clone(), result),
            Err(error) => JsonRpcResponse::error(response_id, error.code, error.message),
        };
        if is_notification {
            return String::new();
        }
        serialize_response(&response)
    }

    fn dispatch(&mut self, request: JsonRpcRequest) -> Result<Value, JsonRpcError> {
        match request.method.as_str() {
            "initialize" => {
                let params: InitializeRequest = parse_params(request.params)?;
                let response = self
                    .server
                    .initialize(params)
                    .map_err(server_error_to_rpc_error)?;
                Ok(json!(response))
            }
            "session/new" => {
                let params: SessionNewRequest = parse_params(request.params)?;
                let session = self
                    .server
                    .new_session(params.cwd)
                    .map_err(server_error_to_rpc_error)?;
                Ok(json!(session))
            }
            "session/load" => {
                let params: SessionLoadRequest = parse_params(request.params)?;
                let session = self
                    .server
                    .load_session(&params.session_id)
                    .map_err(server_error_to_rpc_error)?;
                Ok(json!({ "session": session }))
            }
            "session/prompt" => {
                let params: SessionPromptRequest = parse_params(request.params)?;
                let prompt_id = self
                    .server
                    .prompt_session(&params.session_id, params.prompt)
                    .map_err(server_error_to_rpc_error)?;
                Ok(json!(SessionPromptResponse {
                    session_id: params.session_id,
                    prompt_id,
                    accepted: true,
                }))
            }
            "session/cancel" => {
                let params: SessionCancelRequest = parse_params(request.params)?;
                self.server
                    .cancel_session(&params.session_id)
                    .map_err(server_error_to_rpc_error)?;
                Ok(json!(SessionCancelResponse {
                    session_id: params.session_id,
                    cancelled: true,
                }))
            }
            "session/updates" => {
                let params: SessionUpdatesRequest = parse_params(request.params)?;
                let updates = self
                    .server
                    .drain_session_updates(&params.session_id)
                    .map_err(server_error_to_rpc_error)?;
                Ok(json!(SessionUpdatesResponse {
                    session_id: params.session_id,
                    updates,
                }))
            }
            _ => Err(JsonRpcError {
                code: -32601,
                message: format!("method not found: {}", request.method),
            }),
        }
    }
}

fn parse_params<T: for<'de> Deserialize<'de>>(params: Value) -> Result<T, JsonRpcError> {
    serde_json::from_value(params).map_err(|error| JsonRpcError {
        code: -32602,
        message: format!("invalid params: {error}"),
    })
}

fn server_error_to_rpc_error(error: AcpServerError) -> JsonRpcError {
    match error {
        AcpServerError::NotInitialized => JsonRpcError {
            code: -32001,
            message: error.to_string(),
        },
        AcpServerError::UnsupportedProtocol { .. } => JsonRpcError {
            code: -32002,
            message: error.to_string(),
        },
        AcpServerError::SessionMissing(_) => JsonRpcError {
            code: -32004,
            message: error.to_string(),
        },
    }
}

fn serialize_response(response: &JsonRpcResponse) -> String {
    serde_json::to_string(response)
        .unwrap_or_else(|_| "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32603,\"message\":\"internal serialization error\"}}".to_string())
}

fn jsonrpc_v2() -> String {
    "2.0".to_string()
}

fn extract_response_id(parsed: &Value) -> JsonRpcId {
    parsed
        .get("id")
        .and_then(|id| serde_json::from_value::<JsonRpcId>(id.clone()).ok())
        .unwrap_or(JsonRpcId::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulgen_pty::{create_runtime_backend, CommandSpec};

    fn rpc_request(id: Value, method: &str, params: Value) -> String {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        })
        .to_string()
    }

    fn assert_number_id(id: &JsonRpcId, expected: u64) {
        match id {
            JsonRpcId::Number(number) => assert_eq!(number.as_u64(), Some(expected)),
            _ => panic!("expected numeric id {expected}, got {id:?}"),
        }
    }

    fn parse_rpc_response(line: &str) -> JsonRpcResponse {
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn can_create_and_load_session() {
        let mut registry = SessionRegistry::new();
        let session = registry.create_session("/tmp/project".to_string());
        let loaded = registry.load_session(&session.session_id).unwrap();
        assert_eq!(loaded, session);
    }

    #[test]
    fn session_registry_tracks_updates_and_unique_ids() {
        let mut registry = SessionRegistry::new();
        let first = registry.create_session("/tmp/one".to_string());
        assert_eq!(first.session_id, "sess-1");

        let prompt_id = registry
            .prompt_session(&first.session_id, "build".to_string())
            .unwrap();
        assert_eq!(prompt_id, "prompt-1");

        assert!(registry.cancel_session(&first.session_id).is_ok());
        assert!(matches!(
            registry.cancel_session("sess-missing"),
            Err(AcpServerError::SessionMissing(_))
        ));

        let updates = registry.drain_updates(&first.session_id).unwrap();
        assert_eq!(updates.len(), 3);
        assert!(matches!(updates[0], SessionUpdate::SessionCreated { .. }));
        assert!(matches!(updates[1], SessionUpdate::PromptReceived { .. }));
        assert!(matches!(updates[2], SessionUpdate::SessionCancelled { .. }));

        assert!(matches!(
            registry.drain_updates("sess-missing"),
            Err(AcpServerError::SessionMissing(_))
        ));

        let second = registry.create_session("/tmp/two".to_string());
        assert_eq!(second.session_id, "sess-2");
    }

    #[test]
    fn server_requires_initialize_before_session_ops() {
        let mut server = AcpServer::new();
        let err = server.new_session("/tmp/project".to_string()).unwrap_err();
        assert!(matches!(err, AcpServerError::NotInitialized));
    }

    #[test]
    fn server_initialize_and_lifecycle_flow() {
        let mut server = AcpServer::new();
        let init = server
            .initialize(InitializeRequest {
                protocol_version: ACP_PROTOCOL_VERSION,
                client_capabilities: ClientCapabilities {
                    terminal: true,
                    fs_read_text_file: true,
                    fs_write_text_file: false,
                },
            })
            .unwrap();
        assert_eq!(init.protocol_version, ACP_PROTOCOL_VERSION);
        assert!(server.is_initialized());

        let session = server.new_session("/tmp/agent".to_string()).unwrap();
        let loaded = server.load_session(&session.session_id).unwrap();
        assert_eq!(loaded, session.clone());

        let prompt_id = server
            .prompt_session(&session.session_id, "run tests".to_string())
            .unwrap();
        assert_eq!(prompt_id, "prompt-1");

        let updates = server.drain_session_updates(&session.session_id).unwrap();
        assert_eq!(updates.len(), 2);

        server.cancel_session(&session.session_id).unwrap();
        assert!(matches!(
            server.load_session(&session.session_id),
            Err(AcpServerError::SessionMissing(_))
        ));

        let post_cancel_updates = server.drain_session_updates(&session.session_id).unwrap();
        assert_eq!(post_cancel_updates.len(), 1);
        assert!(matches!(
            post_cancel_updates[0],
            SessionUpdate::SessionCancelled { .. }
        ));
    }

    #[test]
    fn json_rpc_transport_supports_initialize_and_session_lifecycle() {
        let mut transport = JsonRpcStdioTransport::new(AcpServer::new());

        let init_line = rpc_request(
            json!(1),
            "initialize",
            json!({
                "protocol_version": ACP_PROTOCOL_VERSION,
                "client_capabilities": {
                    "terminal": true,
                    "fs_read_text_file": true,
                    "fs_write_text_file": true
                }
            }),
        );
        let init_response = parse_rpc_response(&transport.handle_line(&init_line));
        assert!(init_response.error.is_none());
        assert_number_id(&init_response.id, 1);

        let new_line = rpc_request(json!(2), "session/new", json!({ "cwd": "/tmp/acp" }));
        let new_response = parse_rpc_response(&transport.handle_line(&new_line));
        assert!(new_response.error.is_none());
        assert_number_id(&new_response.id, 2);
        let session: Session = serde_json::from_value(new_response.result.unwrap()).unwrap();

        let load_line = rpc_request(
            json!(3),
            "session/load",
            json!({ "session_id": session.session_id }),
        );
        let load_response = parse_rpc_response(&transport.handle_line(&load_line));
        assert!(load_response.error.is_none());
        assert_number_id(&load_response.id, 3);
        let loaded: Value = load_response.result.unwrap();
        assert_eq!(
            loaded
                .get("session")
                .and_then(|value| value.get("cwd"))
                .unwrap(),
            "/tmp/acp"
        );

        let prompt_line = rpc_request(
            json!(4),
            "session/prompt",
            json!({ "session_id": session.session_id, "prompt": "build" }),
        );
        let prompt_response = parse_rpc_response(&transport.handle_line(&prompt_line));
        assert!(prompt_response.error.is_none());
        assert_number_id(&prompt_response.id, 4);
        let prompt: SessionPromptResponse =
            serde_json::from_value(prompt_response.result.unwrap()).unwrap();
        assert_eq!(prompt.prompt_id, "prompt-1");

        let updates_line = rpc_request(
            json!(5),
            "session/updates",
            json!({ "session_id": session.session_id }),
        );
        let updates_response = parse_rpc_response(&transport.handle_line(&updates_line));
        assert!(updates_response.error.is_none());
        assert_number_id(&updates_response.id, 5);
        let updates: SessionUpdatesResponse =
            serde_json::from_value(updates_response.result.unwrap()).unwrap();
        assert_eq!(updates.updates.len(), 2);

        let cancel_line = rpc_request(
            json!(6),
            "session/cancel",
            json!({ "session_id": session.session_id }),
        );
        let cancel_response = parse_rpc_response(&transport.handle_line(&cancel_line));
        assert!(cancel_response.error.is_none());
        assert_number_id(&cancel_response.id, 6);
        let cancel: SessionCancelResponse =
            serde_json::from_value(cancel_response.result.unwrap()).unwrap();
        assert!(cancel.cancelled);
    }

    #[test]
    fn json_rpc_transport_reports_protocol_and_parse_errors() {
        let mut transport = JsonRpcStdioTransport::new(AcpServer::new());

        let parse_error = parse_rpc_response(&transport.handle_line("{not-json"));
        assert!(parse_error.error.is_some());
        assert_eq!(parse_error.error.unwrap().code, -32700);
        assert_eq!(parse_error.id, JsonRpcId::Null);

        let missing_jsonrpc = parse_rpc_response(
            &transport.handle_line(
                &json!({
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocol_version": ACP_PROTOCOL_VERSION,
                        "client_capabilities": {
                            "terminal": true,
                            "fs_read_text_file": true,
                            "fs_write_text_file": true
                        }
                    }
                })
                .to_string(),
            ),
        );
        assert!(missing_jsonrpc.error.is_some());
        assert_eq!(missing_jsonrpc.error.unwrap().code, -32600);
        assert_number_id(&missing_jsonrpc.id, 1);

        let missing_method = parse_rpc_response(
            &transport.handle_line(
                &json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "params": {}
                })
                .to_string(),
            ),
        );
        assert!(missing_method.error.is_some());
        assert_eq!(missing_method.error.unwrap().code, -32600);
        assert_number_id(&missing_method.id, 2);

        let invalid_id_type = parse_rpc_response(
            &transport.handle_line(
                &json!({
                    "jsonrpc": "2.0",
                    "id": {
                        "kind": "invalid"
                    },
                    "method": "initialize",
                    "params": {}
                })
                .to_string(),
            ),
        );
        assert!(invalid_id_type.error.is_some());
        assert_eq!(invalid_id_type.error.unwrap().code, -32600);
        assert_eq!(invalid_id_type.id, JsonRpcId::Null);

        let string_id = parse_rpc_response(
            &transport.handle_line(
                &json!({
                    "jsonrpc": "2.0",
                    "id": "agent-req-1",
                    "method": "session/unknown",
                    "params": {}
                })
                .to_string(),
            ),
        );
        assert!(string_id.error.is_some());
        assert_eq!(string_id.error.unwrap().code, -32601);
        assert_eq!(string_id.id, JsonRpcId::String("agent-req-1".to_string()));

        let unknown_method = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(3),
            "session/unknown",
            json!({}),
        )));
        assert!(unknown_method.error.is_some());
        assert_eq!(unknown_method.error.unwrap().code, -32601);
        assert_number_id(&unknown_method.id, 3);

        let pre_init = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(4),
            "session/new",
            json!({ "cwd": "/tmp/preinit" }),
        )));
        assert!(pre_init.error.is_some());
        assert_eq!(pre_init.error.unwrap().code, -32001);
        assert_number_id(&pre_init.id, 4);

        let invalid_params = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(5),
            "initialize",
            json!({
                "protocol_version": ACP_PROTOCOL_VERSION,
                "client_capabilities": {
                    "terminal": true
                }
            }),
        )));
        assert!(invalid_params.error.is_some());
        assert_eq!(invalid_params.error.unwrap().code, -32602);
        assert_number_id(&invalid_params.id, 5);

        let unsupported_protocol = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(6),
            "initialize",
            json!({
                "protocol_version": 999,
                "client_capabilities": {
                    "terminal": true,
                    "fs_read_text_file": true,
                    "fs_write_text_file": true
                }
            }),
        )));
        assert!(unsupported_protocol.error.is_some());
        assert_eq!(unsupported_protocol.error.unwrap().code, -32002);
        assert_number_id(&unsupported_protocol.id, 6);

        let init_ok = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(7),
            "initialize",
            json!({
                "protocol_version": ACP_PROTOCOL_VERSION,
                "client_capabilities": {
                    "terminal": true,
                    "fs_read_text_file": true,
                    "fs_write_text_file": true
                }
            }),
        )));
        assert!(init_ok.error.is_none());
        assert_number_id(&init_ok.id, 7);

        let missing_session = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(8),
            "session/prompt",
            json!({
                "session_id": "sess-missing",
                "prompt": "run"
            }),
        )));
        assert!(missing_session.error.is_some());
        assert_eq!(missing_session.error.unwrap().code, -32004);
        assert_number_id(&missing_session.id, 8);

        let zero_id = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(0),
            "session/unknown",
            json!({}),
        )));
        assert!(zero_id.error.is_some());
        assert_eq!(zero_id.error.unwrap().code, -32601);
        assert_number_id(&zero_id.id, 0);
    }

    #[test]
    fn json_rpc_transport_handles_notifications_without_response() {
        let mut transport = JsonRpcStdioTransport::new(AcpServer::new());

        let init_response = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(1),
            "initialize",
            json!({
                "protocol_version": ACP_PROTOCOL_VERSION,
                "client_capabilities": {
                    "terminal": true,
                    "fs_read_text_file": true,
                    "fs_write_text_file": true
                }
            }),
        )));
        assert!(init_response.error.is_none());

        let notification = transport.handle_line(
            &json!({
                "jsonrpc": "2.0",
                "method": "session/new",
                "params": {
                    "cwd": "/tmp/notify"
                }
            })
            .to_string(),
        );
        assert!(notification.is_empty());
    }

    #[test]
    fn runtime_backend_unsupported_is_propagated_without_panic() {
        let mut backend = create_runtime_backend();
        let create_result = backend
            .spawn(CommandSpec::shell("echo acp"))
            .map_err(|err| format!("terminal create failed: {err:?}"));

        let err = create_result.expect_err("runtime backend should not be usable yet");
        assert!(err.contains("Unsupported"));
    }
}
