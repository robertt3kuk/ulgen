use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerminalId(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            cols: 120,
            rows: 40,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
}

impl CommandSpec {
    pub fn shell(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: "/".to_string(),
            env: Vec::new(),
        }
    }
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
    Unsupported {
        backend: &'static str,
        operation: &'static str,
    },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    Memory,
    UnixPty,
    WindowsConpty,
}

pub fn default_backend_kind() -> BackendKind {
    #[cfg(windows)]
    {
        return BackendKind::WindowsConpty;
    }

    #[cfg(not(windows))]
    {
        BackendKind::UnixPty
    }
}

pub fn create_default_backend() -> Box<dyn TerminalBackend> {
    create_backend(default_backend_kind())
}

pub fn create_backend(kind: BackendKind) -> Box<dyn TerminalBackend> {
    match kind {
        BackendKind::Memory => Box::new(MemoryTerminalBackend::new()),
        BackendKind::UnixPty => Box::new(UnixPtyBackend::new()),
        BackendKind::WindowsConpty => Box::new(WindowsConptyBackend::new()),
    }
}

#[derive(Default)]
pub struct MemoryTerminalBackend {
    next_id: u64,
    sessions: BTreeMap<String, MemoryTerminalSession>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MemoryTerminalSession {
    _spec: CommandSpec,
    size: TerminalSize,
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
                size: TerminalSize::default(),
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

        session.size = size;
        Ok(())
    }

    fn kill(&mut self, terminal_id: &TerminalId) -> Result<(), TerminalError> {
        let session = self
            .sessions
            .get_mut(&terminal_id.0)
            .ok_or(TerminalError::NotFound)?;

        if session.exit.is_none() {
            session.exit = Some(TerminalExitStatus {
                exit_code: None,
                signal: Some("KILL".to_string()),
            });
        }
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

#[derive(Default)]
pub struct UnixPtyBackend {
    inner: Option<MemoryTerminalBackend>,
}

impl UnixPtyBackend {
    pub fn new() -> Self {
        #[cfg(unix)]
        {
            return Self {
                inner: Some(MemoryTerminalBackend::new()),
            };
        }

        #[cfg(not(unix))]
        {
            Self { inner: None }
        }
    }
}

impl TerminalBackend for UnixPtyBackend {
    fn spawn(&mut self, spec: CommandSpec) -> Result<TerminalId, TerminalError> {
        let Some(inner) = self.inner.as_mut() else {
            return Err(TerminalError::Unsupported {
                backend: "unix-pty",
                operation: "spawn",
            });
        };

        inner.spawn(spec)
    }

    fn write(&mut self, terminal_id: &TerminalId, input: &str) -> Result<(), TerminalError> {
        let Some(inner) = self.inner.as_mut() else {
            return Err(TerminalError::Unsupported {
                backend: "unix-pty",
                operation: "write",
            });
        };

        inner.write(terminal_id, input)
    }

    fn resize(
        &mut self,
        terminal_id: &TerminalId,
        size: TerminalSize,
    ) -> Result<(), TerminalError> {
        let Some(inner) = self.inner.as_mut() else {
            return Err(TerminalError::Unsupported {
                backend: "unix-pty",
                operation: "resize",
            });
        };

        inner.resize(terminal_id, size)
    }

    fn kill(&mut self, terminal_id: &TerminalId) -> Result<(), TerminalError> {
        let Some(inner) = self.inner.as_mut() else {
            return Err(TerminalError::Unsupported {
                backend: "unix-pty",
                operation: "kill",
            });
        };

        inner.kill(terminal_id)
    }

    fn output(&self, terminal_id: &TerminalId) -> Result<String, TerminalError> {
        let Some(inner) = self.inner.as_ref() else {
            return Err(TerminalError::Unsupported {
                backend: "unix-pty",
                operation: "output",
            });
        };

        inner.output(terminal_id)
    }

    fn wait_for_exit(
        &self,
        terminal_id: &TerminalId,
    ) -> Result<Option<TerminalExitStatus>, TerminalError> {
        let Some(inner) = self.inner.as_ref() else {
            return Err(TerminalError::Unsupported {
                backend: "unix-pty",
                operation: "wait_for_exit",
            });
        };

        inner.wait_for_exit(terminal_id)
    }
}

#[derive(Default)]
pub struct WindowsConptyBackend {
    inner: Option<MemoryTerminalBackend>,
}

impl WindowsConptyBackend {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            return Self {
                inner: Some(MemoryTerminalBackend::new()),
            };
        }

        #[cfg(not(windows))]
        {
            Self { inner: None }
        }
    }
}

impl TerminalBackend for WindowsConptyBackend {
    fn spawn(&mut self, spec: CommandSpec) -> Result<TerminalId, TerminalError> {
        let Some(inner) = self.inner.as_mut() else {
            return Err(TerminalError::Unsupported {
                backend: "windows-conpty",
                operation: "spawn",
            });
        };

        inner.spawn(spec)
    }

    fn write(&mut self, terminal_id: &TerminalId, input: &str) -> Result<(), TerminalError> {
        let Some(inner) = self.inner.as_mut() else {
            return Err(TerminalError::Unsupported {
                backend: "windows-conpty",
                operation: "write",
            });
        };

        inner.write(terminal_id, input)
    }

    fn resize(
        &mut self,
        terminal_id: &TerminalId,
        size: TerminalSize,
    ) -> Result<(), TerminalError> {
        let Some(inner) = self.inner.as_mut() else {
            return Err(TerminalError::Unsupported {
                backend: "windows-conpty",
                operation: "resize",
            });
        };

        inner.resize(terminal_id, size)
    }

    fn kill(&mut self, terminal_id: &TerminalId) -> Result<(), TerminalError> {
        let Some(inner) = self.inner.as_mut() else {
            return Err(TerminalError::Unsupported {
                backend: "windows-conpty",
                operation: "kill",
            });
        };

        inner.kill(terminal_id)
    }

    fn output(&self, terminal_id: &TerminalId) -> Result<String, TerminalError> {
        let Some(inner) = self.inner.as_ref() else {
            return Err(TerminalError::Unsupported {
                backend: "windows-conpty",
                operation: "output",
            });
        };

        inner.output(terminal_id)
    }

    fn wait_for_exit(
        &self,
        terminal_id: &TerminalId,
    ) -> Result<Option<TerminalExitStatus>, TerminalError> {
        let Some(inner) = self.inner.as_ref() else {
            return Err(TerminalError::Unsupported {
                backend: "windows-conpty",
                operation: "wait_for_exit",
            });
        };

        inner.wait_for_exit(terminal_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backend_kind_matches_target_platform() {
        #[cfg(windows)]
        assert_eq!(default_backend_kind(), BackendKind::WindowsConpty);

        #[cfg(not(windows))]
        assert_eq!(default_backend_kind(), BackendKind::UnixPty);
    }

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

        backend.write(&id, "hello\\n").unwrap();
        assert_eq!(backend.output(&id).unwrap(), "hello\\n");
    }
}
