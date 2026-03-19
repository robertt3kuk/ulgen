use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalId(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalExitStatus {
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalError {
    NotFound,
    AlreadyExited,
    Unsupported,
}

pub trait TerminalBackend {
    fn spawn(&mut self, spec: CommandSpec) -> Result<TerminalId, TerminalError>;
    fn write(&mut self, terminal_id: &TerminalId, input: &str) -> Result<(), TerminalError>;
    fn resize(&mut self, terminal_id: &TerminalId, size: TerminalSize)
        -> Result<(), TerminalError>;
    fn kill(&mut self, terminal_id: &TerminalId) -> Result<(), TerminalError>;
    fn output(&self, terminal_id: &TerminalId) -> Result<String, TerminalError>;
    fn wait_for_exit(
        &self,
        terminal_id: &TerminalId,
    ) -> Result<Option<TerminalExitStatus>, TerminalError>;
}

#[derive(Default)]
pub struct MemoryTerminalBackend {
    next_id: u64,
    sessions: BTreeMap<String, MemoryTerminalSession>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MemoryTerminalSession {
    _spec: CommandSpec,
    _size: TerminalSize,
    output: String,
    exit: Option<TerminalExitStatus>,
}

impl MemoryTerminalBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TerminalBackend for MemoryTerminalBackend {
    fn spawn(&mut self, spec: CommandSpec) -> Result<TerminalId, TerminalError> {
        self.next_id += 1;
        let id = format!("term-{}", self.next_id);
        self.sessions.insert(
            id.clone(),
            MemoryTerminalSession {
                _spec: spec,
                _size: TerminalSize {
                    cols: 120,
                    rows: 40,
                },
                output: String::new(),
                exit: None,
            },
        );
        Ok(TerminalId(id))
    }

    fn write(&mut self, terminal_id: &TerminalId, input: &str) -> Result<(), TerminalError> {
        let session = self
            .sessions
            .get_mut(&terminal_id.0)
            .ok_or(TerminalError::NotFound)?;
        if session.exit.is_some() {
            return Err(TerminalError::AlreadyExited);
        }
        session.output.push_str(input);
        Ok(())
    }

    fn resize(
        &mut self,
        terminal_id: &TerminalId,
        size: TerminalSize,
    ) -> Result<(), TerminalError> {
        let session = self
            .sessions
            .get_mut(&terminal_id.0)
            .ok_or(TerminalError::NotFound)?;
        session._size = size;
        Ok(())
    }

    fn kill(&mut self, terminal_id: &TerminalId) -> Result<(), TerminalError> {
        let session = self
            .sessions
            .get_mut(&terminal_id.0)
            .ok_or(TerminalError::NotFound)?;
        session.exit = Some(TerminalExitStatus {
            exit_code: None,
            signal: Some("SIGKILL".to_string()),
        });
        Ok(())
    }

    fn output(&self, terminal_id: &TerminalId) -> Result<String, TerminalError> {
        let session = self
            .sessions
            .get(&terminal_id.0)
            .ok_or(TerminalError::NotFound)?;
        Ok(session.output.clone())
    }

    fn wait_for_exit(
        &self,
        terminal_id: &TerminalId,
    ) -> Result<Option<TerminalExitStatus>, TerminalError> {
        let session = self
            .sessions
            .get(&terminal_id.0)
            .ok_or(TerminalError::NotFound)?;
        Ok(session.exit.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_backend_captures_writes() {
        let mut backend = MemoryTerminalBackend::new();
        let id = backend
            .spawn(CommandSpec {
                command: "echo".to_string(),
                args: vec!["hello".to_string()],
                cwd: "/tmp".to_string(),
                env: vec![],
            })
            .unwrap();

        backend.write(&id, "hello\n").unwrap();
        assert_eq!(backend.output(&id).unwrap(), "hello\n");
    }
}
