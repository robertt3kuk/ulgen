use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Number, Value};
use ulgen_pty::{
    create_contract_backend, CommandSpec, TerminalBackend, TerminalError, TerminalExitStatus,
    TerminalId,
};

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

pub const DEFAULT_OUTPUT_BYTE_LIMIT: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCreateRequest {
    #[serde(alias = "sessionId")]
    pub session_id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: String,
    #[serde(default = "default_output_byte_limit", alias = "outputByteLimit")]
    pub output_byte_limit: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalInputSource {
    Agent,
    User,
}

impl Default for TerminalInputSource {
    fn default() -> Self {
        Self::Agent
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalHandleRequest {
    #[serde(alias = "sessionId")]
    pub session_id: String,
    #[serde(alias = "terminalId")]
    pub terminal_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInputRequest {
    #[serde(alias = "sessionId")]
    pub session_id: String,
    #[serde(alias = "terminalId")]
    pub terminal_id: String,
    pub input: String,
    #[serde(default)]
    pub source: TerminalInputSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalExitStatusPayload {
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
}

impl From<TerminalExitStatus> for TerminalExitStatusPayload {
    fn from(value: TerminalExitStatus) -> Self {
        Self {
            exit_code: value.exit_code,
            signal: value.signal,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCreateResponse {
    pub session_id: String,
    pub terminal_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOutputResponse {
    pub session_id: String,
    pub terminal_id: String,
    pub output: String,
    pub exit_status: Option<TerminalExitStatusPayload>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalWaitResponse {
    pub session_id: String,
    pub terminal_id: String,
    pub exit_status: Option<TerminalExitStatusPayload>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalMutationResponse {
    pub session_id: String,
    pub terminal_id: String,
    pub acknowledged: bool,
}

fn default_output_byte_limit() -> usize {
    DEFAULT_OUTPUT_BYTE_LIMIT
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ManagedTerminal {
    session_id: String,
    output_byte_limit: usize,
    interactive_mode: bool,
    input_owner: TerminalInputSource,
}

pub trait AcpTerminalBridge {
    fn terminal_create(&mut self, request: TerminalCreateRequest) -> Result<TerminalId, String>;
    fn terminal_input(&mut self, request: TerminalInputRequest) -> Result<(), String>;
    fn terminal_output(
        &mut self,
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

pub struct LocalAcpTerminalBridge {
    backend: Box<dyn TerminalBackend>,
    terminals: BTreeMap<String, ManagedTerminal>,
}

impl LocalAcpTerminalBridge {
    pub fn new(backend: Box<dyn TerminalBackend>) -> Self {
        Self {
            backend,
            terminals: BTreeMap::new(),
        }
    }

    fn terminal_record(
        &self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<&ManagedTerminal, String> {
        let record = self
            .terminals
            .get(&terminal_id.0)
            .ok_or_else(|| format!("terminal missing: {}", terminal_id.0))?;
        if record.session_id != session_id {
            return Err(format!(
                "terminal {} is not owned by session {}",
                terminal_id.0, session_id
            ));
        }
        Ok(record)
    }

    fn terminal_record_mut(
        &mut self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<&mut ManagedTerminal, String> {
        let record = self
            .terminals
            .get_mut(&terminal_id.0)
            .ok_or_else(|| format!("terminal missing: {}", terminal_id.0))?;
        if record.session_id != session_id {
            return Err(format!(
                "terminal {} is not owned by session {}",
                terminal_id.0, session_id
            ));
        }
        Ok(record)
    }

    fn sync_interactive_mode_from_output(
        &mut self,
        session_id: &str,
        terminal_id: &TerminalId,
        output: &str,
    ) -> Result<(), String> {
        let current_interactive_mode = {
            let record = self.terminal_record(session_id, terminal_id)?;
            record.interactive_mode
        };
        let next_interactive_mode = resolve_interactive_mode(current_interactive_mode, output);
        if next_interactive_mode != current_interactive_mode {
            let record = self.terminal_record_mut(session_id, terminal_id)?;
            record.interactive_mode = next_interactive_mode;
            if !next_interactive_mode {
                record.input_owner = TerminalInputSource::Agent;
            }
        }
        Ok(())
    }

    fn sync_interactive_mode(
        &mut self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<(), String> {
        self.terminal_record(session_id, terminal_id)?;
        let output = self
            .backend
            .output(terminal_id)
            .map_err(|error| format_terminal_error("terminal output", error))?;
        self.sync_interactive_mode_from_output(session_id, terminal_id, &output)
    }

    fn release_session_terminals(&mut self, session_id: &str) -> Result<(), String> {
        let terminal_ids = self
            .terminals
            .iter()
            .filter_map(|(terminal_id, record)| {
                (record.session_id == session_id).then(|| TerminalId(terminal_id.clone()))
            })
            .collect::<Vec<_>>();

        for terminal_id in &terminal_ids {
            match self.backend.kill(terminal_id) {
                Ok(()) | Err(TerminalError::AlreadyExited) | Err(TerminalError::NotFound) => {}
                Err(error) => return Err(format_terminal_error("terminal kill", error)),
            }
        }

        for terminal_id in terminal_ids {
            self.terminals.remove(&terminal_id.0);
        }

        Ok(())
    }
}

fn output_has_enter_interactive_mode(output: &str) -> Option<usize> {
    ["\u{1b}[?1049h", "\u{1b}[?1047h", "\u{1b}[?47h"]
        .iter()
        .filter_map(|marker| output.rfind(marker))
        .max()
}

fn output_has_exit_interactive_mode(output: &str) -> Option<usize> {
    ["\u{1b}[?1049l", "\u{1b}[?1047l", "\u{1b}[?47l"]
        .iter()
        .filter_map(|marker| output.rfind(marker))
        .max()
}

fn resolve_interactive_mode(current: bool, output: &str) -> bool {
    match (
        output_has_enter_interactive_mode(output),
        output_has_exit_interactive_mode(output),
    ) {
        (None, None) => current,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (Some(enter), Some(exit)) => enter > exit,
    }
}

fn resolve_input_owner_for_mode(
    source: TerminalInputSource,
    interactive_mode: bool,
) -> TerminalInputSource {
    if interactive_mode {
        source
    } else {
        TerminalInputSource::Agent
    }
}

impl AcpTerminalBridge for LocalAcpTerminalBridge {
    fn terminal_create(&mut self, request: TerminalCreateRequest) -> Result<TerminalId, String> {
        if request.command.is_empty() {
            return Err("terminal create failed: command must not be empty".to_string());
        }

        let spec = CommandSpec {
            command: request.command,
            args: request.args,
            cwd: PathBuf::from(request.cwd),
            env: Vec::new(),
        };

        let terminal_id = self
            .backend
            .spawn(spec)
            .map_err(|error| format_terminal_error("terminal create", error))?;
        self.terminals.insert(
            terminal_id.0.clone(),
            ManagedTerminal {
                session_id: request.session_id,
                output_byte_limit: request.output_byte_limit,
                interactive_mode: false,
                input_owner: TerminalInputSource::Agent,
            },
        );
        Ok(terminal_id)
    }

    fn terminal_input(&mut self, request: TerminalInputRequest) -> Result<(), String> {
        let terminal_id = TerminalId(request.terminal_id.clone());
        self.sync_interactive_mode(&request.session_id, &terminal_id)?;
        {
            let record = self.terminal_record(&request.session_id, &terminal_id)?;
            if record.interactive_mode
                && record.input_owner == TerminalInputSource::User
                && request.source == TerminalInputSource::Agent
            {
                return Err(format!(
                    "terminal input denied: {} is under user control",
                    terminal_id.0
                ));
            }
        }

        self.backend
            .write(&terminal_id, &request.input)
            .map_err(|error| format_terminal_error("terminal input", error))?;

        let record = self.terminal_record_mut(&request.session_id, &terminal_id)?;
        let next_interactive_mode =
            resolve_interactive_mode(record.interactive_mode, &request.input);
        record.interactive_mode = next_interactive_mode;
        record.input_owner = resolve_input_owner_for_mode(request.source, next_interactive_mode);
        Ok(())
    }

    fn terminal_output(
        &mut self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<(String, Option<TerminalExitStatus>), String> {
        self.terminal_record(session_id, terminal_id)?;
        let output = self
            .backend
            .output(terminal_id)
            .map_err(|error| format_terminal_error("terminal output", error))?;
        self.sync_interactive_mode_from_output(session_id, terminal_id, &output)?;

        let (output_byte_limit, interactive_mode, input_owner) = {
            let record = self.terminal_record(session_id, terminal_id)?;
            (
                record.output_byte_limit,
                record.interactive_mode,
                record.input_owner,
            )
        };

        if interactive_mode && input_owner == TerminalInputSource::User {
            return Err(format!(
                "terminal output denied: {} is in interactive mode under user control",
                terminal_id.0
            ));
        }

        let output = truncate_output(&output, output_byte_limit);
        let exit_status = self
            .backend
            .wait_for_exit(terminal_id)
            .map_err(|error| format_terminal_error("terminal wait", error))?;
        Ok((output, exit_status))
    }

    fn terminal_wait_for_exit(
        &self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<Option<TerminalExitStatus>, String> {
        self.terminal_record(session_id, terminal_id)?;
        self.backend
            .wait_for_exit(terminal_id)
            .map_err(|error| format_terminal_error("terminal wait", error))
    }

    fn terminal_kill(&mut self, session_id: &str, terminal_id: &TerminalId) -> Result<(), String> {
        self.sync_interactive_mode(session_id, terminal_id)?;
        let record = self.terminal_record(session_id, terminal_id)?;
        if record.interactive_mode && record.input_owner == TerminalInputSource::User {
            return Err(format!(
                "terminal kill denied: {} is in interactive mode under user control",
                terminal_id.0
            ));
        }
        self.backend
            .kill(terminal_id)
            .map_err(|error| format_terminal_error("terminal kill", error))
    }

    fn terminal_release(
        &mut self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<(), String> {
        self.sync_interactive_mode(session_id, terminal_id)?;
        let record = self.terminal_record(session_id, terminal_id)?;
        if record.interactive_mode && record.input_owner == TerminalInputSource::User {
            return Err(format!(
                "terminal release denied: {} is in interactive mode under user control",
                terminal_id.0
            ));
        }
        match self.backend.kill(terminal_id) {
            Ok(()) | Err(TerminalError::AlreadyExited) | Err(TerminalError::NotFound) => {}
            Err(error) => return Err(format_terminal_error("terminal release", error)),
        }
        self.terminals.remove(&terminal_id.0);
        Ok(())
    }
}

fn truncate_output(output: &str, output_byte_limit: usize) -> String {
    if output_byte_limit == 0 {
        return String::new();
    }
    if output.len() <= output_byte_limit {
        return output.to_string();
    }
    let mut start = output.len().saturating_sub(output_byte_limit);
    while start < output.len() && !output.is_char_boundary(start) {
        start += 1;
    }
    output[start..].to_string()
}

fn format_terminal_error(operation: &str, error: TerminalError) -> String {
    match error {
        TerminalError::NotFound => format!("{operation} failed: terminal not found"),
        TerminalError::AlreadyExited => format!("{operation} failed: terminal already exited"),
        TerminalError::Unsupported {
            backend,
            operation: backend_operation,
        } => {
            format!(
                "{operation} failed: unsupported backend={backend} operation={backend_operation}"
            )
        }
    }
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
    TerminalOperation(String),
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
            Self::TerminalOperation(message) => write!(f, "{message}"),
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

pub struct AcpServer {
    initialized: bool,
    sessions: SessionRegistry,
    terminal_bridge: LocalAcpTerminalBridge,
}

impl Default for AcpServer {
    fn default() -> Self {
        Self {
            initialized: false,
            sessions: SessionRegistry::new(),
            terminal_bridge: LocalAcpTerminalBridge::new(create_contract_backend()),
        }
    }
}

impl AcpServer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_terminal_backend(backend: Box<dyn TerminalBackend>) -> Self {
        Self {
            initialized: false,
            sessions: SessionRegistry::new(),
            terminal_bridge: LocalAcpTerminalBridge::new(backend),
        }
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
        self.sessions.load_session(session_id)?;
        self.terminal_bridge
            .release_session_terminals(session_id)
            .map_err(AcpServerError::TerminalOperation)?;
        self.sessions.cancel_session(session_id)
    }

    pub fn drain_session_updates(
        &mut self,
        session_id: &str,
    ) -> Result<Vec<SessionUpdate>, AcpServerError> {
        self.ensure_initialized()?;
        self.sessions.drain_updates(session_id)
    }

    pub fn terminal_create(
        &mut self,
        request: TerminalCreateRequest,
    ) -> Result<TerminalId, AcpServerError> {
        self.ensure_initialized()?;
        self.sessions.load_session(&request.session_id)?;
        self.terminal_bridge
            .terminal_create(request)
            .map_err(AcpServerError::TerminalOperation)
    }

    pub fn terminal_input(&mut self, request: TerminalInputRequest) -> Result<(), AcpServerError> {
        self.ensure_initialized()?;
        self.sessions.load_session(&request.session_id)?;
        self.terminal_bridge
            .terminal_input(request)
            .map_err(AcpServerError::TerminalOperation)
    }

    pub fn terminal_output(
        &mut self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<(String, Option<TerminalExitStatus>), AcpServerError> {
        self.ensure_initialized()?;
        self.sessions.load_session(session_id)?;
        self.terminal_bridge
            .terminal_output(session_id, terminal_id)
            .map_err(AcpServerError::TerminalOperation)
    }

    pub fn terminal_wait_for_exit(
        &self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<Option<TerminalExitStatus>, AcpServerError> {
        self.ensure_initialized()?;
        self.sessions.load_session(session_id)?;
        self.terminal_bridge
            .terminal_wait_for_exit(session_id, terminal_id)
            .map_err(AcpServerError::TerminalOperation)
    }

    pub fn terminal_kill(
        &mut self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<(), AcpServerError> {
        self.ensure_initialized()?;
        self.sessions.load_session(session_id)?;
        self.terminal_bridge
            .terminal_kill(session_id, terminal_id)
            .map_err(AcpServerError::TerminalOperation)
    }

    pub fn terminal_release(
        &mut self,
        session_id: &str,
        terminal_id: &TerminalId,
    ) -> Result<(), AcpServerError> {
        self.ensure_initialized()?;
        self.sessions.load_session(session_id)?;
        self.terminal_bridge
            .terminal_release(session_id, terminal_id)
            .map_err(AcpServerError::TerminalOperation)
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
            "terminal/create" => {
                let params: TerminalCreateRequest = parse_params(request.params)?;
                let session_id = params.session_id.clone();
                let terminal_id = self
                    .server
                    .terminal_create(params)
                    .map_err(server_error_to_rpc_error)?;
                Ok(json!(TerminalCreateResponse {
                    session_id,
                    terminal_id: terminal_id.0,
                }))
            }
            "terminal/input" => {
                let params: TerminalInputRequest = parse_params(request.params)?;
                let session_id = params.session_id.clone();
                let terminal_id = params.terminal_id.clone();
                self.server
                    .terminal_input(params)
                    .map_err(server_error_to_rpc_error)?;
                Ok(json!(TerminalMutationResponse {
                    session_id,
                    terminal_id,
                    acknowledged: true,
                }))
            }
            "terminal/output" => {
                let params: TerminalHandleRequest = parse_params(request.params)?;
                let terminal_id = TerminalId(params.terminal_id.clone());
                let (output, exit_status) = self
                    .server
                    .terminal_output(&params.session_id, &terminal_id)
                    .map_err(server_error_to_rpc_error)?;
                Ok(json!(TerminalOutputResponse {
                    session_id: params.session_id,
                    terminal_id: terminal_id.0,
                    output,
                    exit_status: exit_status.map(TerminalExitStatusPayload::from),
                }))
            }
            "terminal/wait_for_exit" => {
                let params: TerminalHandleRequest = parse_params(request.params)?;
                let terminal_id = TerminalId(params.terminal_id.clone());
                let exit_status = self
                    .server
                    .terminal_wait_for_exit(&params.session_id, &terminal_id)
                    .map_err(server_error_to_rpc_error)?;
                Ok(json!(TerminalWaitResponse {
                    session_id: params.session_id,
                    terminal_id: terminal_id.0,
                    exit_status: exit_status.map(TerminalExitStatusPayload::from),
                }))
            }
            "terminal/kill" => {
                let params: TerminalHandleRequest = parse_params(request.params)?;
                let terminal_id = TerminalId(params.terminal_id.clone());
                self.server
                    .terminal_kill(&params.session_id, &terminal_id)
                    .map_err(server_error_to_rpc_error)?;
                Ok(json!(TerminalMutationResponse {
                    session_id: params.session_id,
                    terminal_id: terminal_id.0,
                    acknowledged: true,
                }))
            }
            "terminal/release" => {
                let params: TerminalHandleRequest = parse_params(request.params)?;
                let terminal_id = TerminalId(params.terminal_id.clone());
                self.server
                    .terminal_release(&params.session_id, &terminal_id)
                    .map_err(server_error_to_rpc_error)?;
                Ok(json!(TerminalMutationResponse {
                    session_id: params.session_id,
                    terminal_id: terminal_id.0,
                    acknowledged: true,
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
        AcpServerError::TerminalOperation(_) => JsonRpcError {
            code: -32005,
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
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use ulgen_pty::{
        create_runtime_backend, CommandSpec, MemoryTerminalBackend, TerminalBackend, TerminalSize,
    };

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

    struct CountingOutputBackend {
        inner: MemoryTerminalBackend,
        output_calls: Arc<AtomicUsize>,
    }

    impl CountingOutputBackend {
        fn new(output_calls: Arc<AtomicUsize>) -> Self {
            Self {
                inner: MemoryTerminalBackend::new(),
                output_calls,
            }
        }
    }

    impl TerminalBackend for CountingOutputBackend {
        fn spawn(&mut self, spec: CommandSpec) -> Result<TerminalId, TerminalError> {
            self.inner.spawn(spec)
        }

        fn write(&mut self, terminal_id: &TerminalId, input: &str) -> Result<(), TerminalError> {
            self.inner.write(terminal_id, input)
        }

        fn resize(
            &mut self,
            terminal_id: &TerminalId,
            size: TerminalSize,
        ) -> Result<(), TerminalError> {
            self.inner.resize(terminal_id, size)
        }

        fn kill(&mut self, terminal_id: &TerminalId) -> Result<(), TerminalError> {
            self.inner.kill(terminal_id)
        }

        fn output(&self, terminal_id: &TerminalId) -> Result<String, TerminalError> {
            self.output_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.output(terminal_id)
        }

        fn wait_for_exit(
            &self,
            terminal_id: &TerminalId,
        ) -> Result<Option<TerminalExitStatus>, TerminalError> {
            self.inner.wait_for_exit(terminal_id)
        }
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

    #[test]
    fn server_error_mapping_includes_terminal_operation() {
        let rpc_error =
            server_error_to_rpc_error(AcpServerError::TerminalOperation("terminal failed".into()));
        assert_eq!(rpc_error.code, -32005);
        assert_eq!(rpc_error.message, "terminal failed");
    }

    #[test]
    fn terminal_error_messages_are_stable_and_structured() {
        let unsupported = format_terminal_error(
            "terminal create",
            TerminalError::Unsupported {
                backend: "unix-pty",
                operation: "spawn",
            },
        );
        assert_eq!(
            unsupported,
            "terminal create failed: unsupported backend=unix-pty operation=spawn"
        );
    }

    #[test]
    fn terminal_create_request_accepts_aliases_and_defaults() {
        let request: TerminalCreateRequest = serde_json::from_value(json!({
            "sessionId": "sess-1",
            "command": "echo",
            "cwd": "/tmp/acp"
        }))
        .unwrap();
        assert_eq!(request.session_id, "sess-1");
        assert_eq!(request.command, "echo");
        assert_eq!(request.cwd, "/tmp/acp");
        assert!(request.args.is_empty());
        assert_eq!(request.output_byte_limit, DEFAULT_OUTPUT_BYTE_LIMIT);
    }

    #[test]
    fn terminal_input_request_defaults_source_to_agent() {
        let request: TerminalInputRequest = serde_json::from_value(json!({
            "sessionId": "sess-1",
            "terminalId": "term-1",
            "input": "ls -la\n"
        }))
        .unwrap();
        assert_eq!(request.session_id, "sess-1");
        assert_eq!(request.terminal_id, "term-1");
        assert_eq!(request.source, TerminalInputSource::Agent);
    }

    #[test]
    fn cancel_session_cleans_up_owned_terminals() {
        let mut server = AcpServer::new();
        server
            .initialize(InitializeRequest {
                protocol_version: ACP_PROTOCOL_VERSION,
                client_capabilities: ClientCapabilities {
                    terminal: true,
                    fs_read_text_file: true,
                    fs_write_text_file: true,
                },
            })
            .unwrap();

        let session = server.new_session("/tmp/acp".to_string()).unwrap();
        let terminal_id = server
            .terminal_create(TerminalCreateRequest {
                session_id: session.session_id.clone(),
                command: "echo".to_string(),
                args: vec!["acp".to_string()],
                cwd: "/tmp/acp".to_string(),
                output_byte_limit: DEFAULT_OUTPUT_BYTE_LIMIT,
            })
            .unwrap();

        assert!(server
            .terminal_bridge
            .terminals
            .contains_key(&terminal_id.0));

        server.cancel_session(&session.session_id).unwrap();

        assert!(!server
            .terminal_bridge
            .terminals
            .contains_key(&terminal_id.0));
        let exit_status = server
            .terminal_bridge
            .backend
            .wait_for_exit(&terminal_id)
            .unwrap();
        assert_eq!(
            exit_status,
            Some(TerminalExitStatus {
                exit_code: None,
                signal: Some("KILL".to_string())
            })
        );
    }

    #[test]
    fn terminal_output_respects_utf8_truncation_limit() {
        let mut bridge = LocalAcpTerminalBridge::new(create_contract_backend());
        let request = TerminalCreateRequest {
            session_id: "sess-1".to_string(),
            command: "echo".to_string(),
            args: vec!["hello".to_string()],
            cwd: "/tmp".to_string(),
            output_byte_limit: 4,
        };
        let terminal_id = bridge.terminal_create(request).unwrap();
        bridge.backend.write(&terminal_id, "ok Привет").unwrap();

        let (output, exit_status) = bridge.terminal_output("sess-1", &terminal_id).unwrap();
        assert_eq!(output, "ет");
        assert!(exit_status.is_none());
    }

    #[test]
    fn interactive_mode_blocks_agent_output_when_user_controls_terminal() {
        let mut bridge = LocalAcpTerminalBridge::new(create_contract_backend());
        let terminal_id = bridge
            .terminal_create(TerminalCreateRequest {
                session_id: "sess-1".to_string(),
                command: "echo".to_string(),
                args: vec!["interactive".to_string()],
                cwd: "/tmp".to_string(),
                output_byte_limit: DEFAULT_OUTPUT_BYTE_LIMIT,
            })
            .unwrap();

        bridge
            .terminal_input(TerminalInputRequest {
                session_id: "sess-1".to_string(),
                terminal_id: terminal_id.0.clone(),
                input: "\u{1b}[?1049h".to_string(),
                source: TerminalInputSource::User,
            })
            .unwrap();

        let denied = bridge.terminal_output("sess-1", &terminal_id).unwrap_err();
        assert!(denied.contains("interactive mode under user control"));

        bridge.backend.write(&terminal_id, "\u{1b}[?1049l").unwrap();
        let (output, _) = bridge.terminal_output("sess-1", &terminal_id).unwrap();
        assert!(output.contains("\u{1b}[?1049l"));
    }

    #[test]
    fn interactive_mode_is_refreshed_for_kill_without_prior_output_poll() {
        let mut bridge = LocalAcpTerminalBridge::new(create_contract_backend());
        let terminal_id = bridge
            .terminal_create(TerminalCreateRequest {
                session_id: "sess-1".to_string(),
                command: "echo".to_string(),
                args: vec!["interactive".to_string()],
                cwd: "/tmp".to_string(),
                output_byte_limit: DEFAULT_OUTPUT_BYTE_LIMIT,
            })
            .unwrap();

        let record = bridge.terminals.get_mut(&terminal_id.0).unwrap();
        record.input_owner = TerminalInputSource::User;
        bridge.backend.write(&terminal_id, "\u{1b}[?1049h").unwrap();

        let denied = bridge.terminal_kill("sess-1", &terminal_id).unwrap_err();
        assert!(denied.contains("interactive mode under user control"));
    }

    #[test]
    fn cursor_save_restore_sequences_do_not_toggle_interactive_mode() {
        let mut bridge = LocalAcpTerminalBridge::new(create_contract_backend());
        let terminal_id = bridge
            .terminal_create(TerminalCreateRequest {
                session_id: "sess-1".to_string(),
                command: "echo".to_string(),
                args: vec!["cursor".to_string()],
                cwd: "/tmp".to_string(),
                output_byte_limit: DEFAULT_OUTPUT_BYTE_LIMIT,
            })
            .unwrap();

        bridge.backend.write(&terminal_id, "\u{1b}[?1048h").unwrap();
        let (output, _) = bridge.terminal_output("sess-1", &terminal_id).unwrap();
        assert!(output.contains("\u{1b}[?1048h"));
        assert!(
            !bridge
                .terminals
                .get(&terminal_id.0)
                .unwrap()
                .interactive_mode
        );
    }

    #[test]
    fn terminal_input_write_failure_keeps_owner_state() {
        let mut bridge = LocalAcpTerminalBridge::new(create_contract_backend());
        let terminal_id = bridge
            .terminal_create(TerminalCreateRequest {
                session_id: "sess-1".to_string(),
                command: "echo".to_string(),
                args: vec!["failure".to_string()],
                cwd: "/tmp".to_string(),
                output_byte_limit: DEFAULT_OUTPUT_BYTE_LIMIT,
            })
            .unwrap();
        bridge.backend.kill(&terminal_id).unwrap();

        let input_error = bridge
            .terminal_input(TerminalInputRequest {
                session_id: "sess-1".to_string(),
                terminal_id: terminal_id.0.clone(),
                input: "still-fails".to_string(),
                source: TerminalInputSource::User,
            })
            .unwrap_err();
        assert!(input_error.contains("already exited"));
        assert_eq!(
            bridge.terminals.get(&terminal_id.0).unwrap().input_owner,
            TerminalInputSource::Agent
        );
    }

    #[test]
    fn terminal_release_terminates_before_untracking() {
        let mut bridge = LocalAcpTerminalBridge::new(create_contract_backend());
        let terminal_id = bridge
            .terminal_create(TerminalCreateRequest {
                session_id: "sess-1".to_string(),
                command: "echo".to_string(),
                args: vec!["test".to_string()],
                cwd: "/tmp".to_string(),
                output_byte_limit: DEFAULT_OUTPUT_BYTE_LIMIT,
            })
            .unwrap();

        bridge.terminal_release("sess-1", &terminal_id).unwrap();
        assert!(!bridge.terminals.contains_key(&terminal_id.0));
        let exit_status = bridge.backend.wait_for_exit(&terminal_id).unwrap();
        assert_eq!(
            exit_status,
            Some(TerminalExitStatus {
                exit_code: None,
                signal: Some("KILL".to_string())
            })
        );
    }

    #[test]
    fn terminal_bridge_rejects_cross_session_access() {
        let mut server = AcpServer::new();
        server
            .initialize(InitializeRequest {
                protocol_version: ACP_PROTOCOL_VERSION,
                client_capabilities: ClientCapabilities {
                    terminal: true,
                    fs_read_text_file: true,
                    fs_write_text_file: true,
                },
            })
            .unwrap();

        let session_a = server.new_session("/tmp/a".to_string()).unwrap();
        let session_b = server.new_session("/tmp/b".to_string()).unwrap();
        let terminal_id = server
            .terminal_create(TerminalCreateRequest {
                session_id: session_a.session_id.clone(),
                command: "echo".to_string(),
                args: vec!["owned".to_string()],
                cwd: "/tmp/a".to_string(),
                output_byte_limit: DEFAULT_OUTPUT_BYTE_LIMIT,
            })
            .unwrap();

        let error = server
            .terminal_output(&session_b.session_id, &terminal_id)
            .unwrap_err();
        assert!(matches!(error, AcpServerError::TerminalOperation(_)));
        assert!(error
            .to_string()
            .contains(&format!("not owned by session {}", session_b.session_id)));
    }

    #[test]
    fn cross_session_output_rejects_before_backend_read() {
        let output_calls = Arc::new(AtomicUsize::new(0));
        let backend = Box::new(CountingOutputBackend::new(output_calls.clone()));
        let mut server = AcpServer::with_terminal_backend(backend);
        server
            .initialize(InitializeRequest {
                protocol_version: ACP_PROTOCOL_VERSION,
                client_capabilities: ClientCapabilities {
                    terminal: true,
                    fs_read_text_file: true,
                    fs_write_text_file: true,
                },
            })
            .unwrap();

        let session_a = server.new_session("/tmp/a".to_string()).unwrap();
        let session_b = server.new_session("/tmp/b".to_string()).unwrap();
        let terminal_id = server
            .terminal_create(TerminalCreateRequest {
                session_id: session_a.session_id.clone(),
                command: "echo".to_string(),
                args: vec!["owned".to_string()],
                cwd: "/tmp/a".to_string(),
                output_byte_limit: DEFAULT_OUTPUT_BYTE_LIMIT,
            })
            .unwrap();

        let error = server
            .terminal_output(&session_b.session_id, &terminal_id)
            .unwrap_err();
        assert!(matches!(error, AcpServerError::TerminalOperation(_)));
        assert_eq!(output_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn json_rpc_transport_supports_terminal_lifecycle() {
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

        let new_response = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(2),
            "session/new",
            json!({ "cwd": "/tmp/acp" }),
        )));
        assert!(new_response.error.is_none());
        let session: Session = serde_json::from_value(new_response.result.unwrap()).unwrap();

        let create_response = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(3),
            "terminal/create",
            json!({
                "sessionId": session.session_id,
                "command": "echo",
                "args": ["hello"],
                "cwd": "/tmp/acp",
                "outputByteLimit": 3
            }),
        )));
        assert!(create_response.error.is_none());
        let created: TerminalCreateResponse =
            serde_json::from_value(create_response.result.unwrap()).unwrap();
        assert_eq!(created.session_id, session.session_id);
        let terminal_id = created.terminal_id.clone();
        let input_response = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(4),
            "terminal/input",
            json!({
                "session_id": session.session_id,
                "terminal_id": terminal_id.clone(),
                "input": "abcdef"
            }),
        )));
        assert!(input_response.error.is_none());

        let output_response = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(5),
            "terminal/output",
            json!({
                "session_id": session.session_id,
                "terminal_id": terminal_id
            }),
        )));
        assert!(output_response.error.is_none());
        let output: TerminalOutputResponse =
            serde_json::from_value(output_response.result.unwrap()).unwrap();
        assert_eq!(output.output, "def");
        assert!(output.exit_status.is_none());

        let kill_response = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(6),
            "terminal/kill",
            json!({
                "session_id": session.session_id,
                "terminal_id": output.terminal_id
            }),
        )));
        assert!(kill_response.error.is_none());

        let wait_response = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(7),
            "terminal/wait_for_exit",
            json!({
                "session_id": session.session_id,
                "terminal_id": output.terminal_id
            }),
        )));
        assert!(wait_response.error.is_none());
        let wait: TerminalWaitResponse =
            serde_json::from_value(wait_response.result.unwrap()).unwrap();
        assert_eq!(
            wait.exit_status,
            Some(TerminalExitStatusPayload {
                exit_code: None,
                signal: Some("KILL".to_string()),
            })
        );

        let release_response = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(8),
            "terminal/release",
            json!({
                "session_id": session.session_id,
                "terminal_id": wait.terminal_id
            }),
        )));
        assert!(release_response.error.is_none());
    }

    #[test]
    fn json_rpc_interactive_mode_blocks_agent_control_until_exit() {
        let mut transport = JsonRpcStdioTransport::new(AcpServer::new());
        assert!(parse_rpc_response(&transport.handle_line(&rpc_request(
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
        )))
        .error
        .is_none());

        let session: Session = serde_json::from_value(
            parse_rpc_response(&transport.handle_line(&rpc_request(
                json!(2),
                "session/new",
                json!({ "cwd": "/tmp/interactive" }),
            )))
            .result
            .unwrap(),
        )
        .unwrap();

        let created: TerminalCreateResponse = serde_json::from_value(
            parse_rpc_response(&transport.handle_line(&rpc_request(
                json!(3),
                "terminal/create",
                json!({
                    "session_id": session.session_id,
                    "command": "echo",
                    "cwd": "/tmp/interactive"
                }),
            )))
            .result
            .unwrap(),
        )
        .unwrap();
        let terminal_id = created.terminal_id.clone();

        let user_input = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(4),
            "terminal/input",
            json!({
                "session_id": session.session_id,
                "terminal_id": terminal_id.clone(),
                "input": "\u{1b}[?1049h",
                "source": "user"
            }),
        )));
        assert!(user_input.error.is_none());
        assert!(
            transport
                .server()
                .terminal_bridge
                .terminals
                .get(&terminal_id)
                .unwrap()
                .interactive_mode
        );
        assert_eq!(
            transport
                .server()
                .terminal_bridge
                .terminals
                .get(&terminal_id)
                .unwrap()
                .input_owner,
            TerminalInputSource::User
        );

        let blocked_output = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(5),
            "terminal/output",
            json!({
                "session_id": session.session_id,
                "terminal_id": terminal_id.clone()
            }),
        )));
        assert!(blocked_output.error.is_some());
        assert_eq!(blocked_output.error.unwrap().code, -32005);

        let blocked_kill = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(6),
            "terminal/kill",
            json!({
                "session_id": session.session_id,
                "terminal_id": terminal_id.clone()
            }),
        )));
        assert!(blocked_kill.error.is_some());
        assert_eq!(blocked_kill.error.unwrap().code, -32005);

        let user_exit = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(7),
            "terminal/input",
            json!({
                "session_id": session.session_id,
                "terminal_id": terminal_id.clone(),
                "input": "\u{1b}[?1049l",
                "source": "user"
            }),
        )));
        assert!(user_exit.error.is_none());

        let unblocked_output = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(8),
            "terminal/output",
            json!({
                "session_id": session.session_id,
                "terminal_id": terminal_id.clone()
            }),
        )));
        assert!(unblocked_output.error.is_none());

        let unblocked_kill = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(9),
            "terminal/kill",
            json!({
                "session_id": session.session_id,
                "terminal_id": terminal_id
            }),
        )));
        assert!(unblocked_kill.error.is_none());
    }

    #[test]
    fn json_rpc_terminal_output_rejects_wrong_session_owner() {
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

        let session_a: Session = serde_json::from_value(
            parse_rpc_response(&transport.handle_line(&rpc_request(
                json!(2),
                "session/new",
                json!({ "cwd": "/tmp/a" }),
            )))
            .result
            .unwrap(),
        )
        .unwrap();
        let session_b: Session = serde_json::from_value(
            parse_rpc_response(&transport.handle_line(&rpc_request(
                json!(3),
                "session/new",
                json!({ "cwd": "/tmp/b" }),
            )))
            .result
            .unwrap(),
        )
        .unwrap();

        let created: TerminalCreateResponse = serde_json::from_value(
            parse_rpc_response(&transport.handle_line(&rpc_request(
                json!(4),
                "terminal/create",
                json!({
                    "session_id": session_a.session_id,
                    "command": "echo",
                    "cwd": "/tmp/a"
                }),
            )))
            .result
            .unwrap(),
        )
        .unwrap();

        let wrong_owner = parse_rpc_response(&transport.handle_line(&rpc_request(
            json!(5),
            "terminal/output",
            json!({
                "session_id": session_b.session_id,
                "terminal_id": created.terminal_id
            }),
        )));
        assert!(wrong_owner.error.is_some());
        assert_eq!(wrong_owner.error.unwrap().code, -32005);
    }
}
