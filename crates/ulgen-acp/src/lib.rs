use std::collections::BTreeMap;

use ulgen_pty::{TerminalExitStatus, TerminalId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientCapabilities {
    pub terminal: bool,
    pub fs_read_text_file: bool,
    pub fs_write_text_file: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitializeRequest {
    pub protocol_version: u32,
    pub client_capabilities: ClientCapabilities,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitializeResponse {
    pub protocol_version: u32,
    pub terminal_supported: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Default)]
pub struct SessionRegistry {
    sessions: BTreeMap<String, Session>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_session(&mut self, cwd: String) -> Session {
        let session_id = format!("sess-{}", self.sessions.len() + 1);
        let session = Session {
            session_id: session_id.clone(),
            cwd,
        };
        self.sessions.insert(session_id, session.clone());
        session
    }

    pub fn load_session(&self, session_id: &str) -> Option<Session> {
        self.sessions.get(session_id).cloned()
    }

    pub fn cancel_session(&mut self, session_id: &str) -> bool {
        self.sessions.remove(session_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulgen_pty::{create_runtime_backend, CommandSpec};

    #[test]
    fn can_create_and_load_session() {
        let mut registry = SessionRegistry::new();
        let session = registry.create_session("/tmp/project".to_string());
        let loaded = registry.load_session(&session.session_id);
        assert_eq!(loaded, Some(session));
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
