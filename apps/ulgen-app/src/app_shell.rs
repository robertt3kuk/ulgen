use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ulgen_command::{CommandAction, CommandRegistry};
use ulgen_domain::{Pane, Surface, Tab, Workspace};
use ulgen_settings::AppSettings;

const APP_STATE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowState {
    pub id: String,
    pub title: String,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppShellState {
    pub version: u32,
    pub windows: Vec<WindowState>,
    pub active_window: usize,
    pub settings: AppSettings,
    pub next_id: u64,
    pub last_started_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppShellCommand {
    NewWindow,
    SelectNextWindow,
    CreateWorkspace { name: String },
    SelectNextWorkspace,
}

pub struct AppShell {
    state: AppShellState,
    state_path: PathBuf,
    commands: CommandRegistry,
}

impl AppShell {
    pub fn bootstrap(state_path: PathBuf) -> io::Result<Self> {
        let state = if state_path.exists() {
            Self::load_from_path(&state_path)?
        } else {
            Self::default_state()
        };

        let mut shell = Self {
            state,
            state_path,
            commands: CommandRegistry::new(),
        };
        shell.register_builtin_commands();
        shell.state.last_started_at_ms = now_ms();

        Ok(shell)
    }

    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    pub fn state(&self) -> &AppShellState {
        &self.state
    }

    pub fn command_registry(&self) -> &CommandRegistry {
        &self.commands
    }

    pub fn route_command_id(&mut self, command_id: &str) -> Result<(), String> {
        match command_id {
            "window.new" => self.route_command(AppShellCommand::NewWindow),
            "window.next" => self.route_command(AppShellCommand::SelectNextWindow),
            "workspace.new" => self.route_command(AppShellCommand::CreateWorkspace {
                name: "workspace".to_string(),
            }),
            "workspace.next" => self.route_command(AppShellCommand::SelectNextWorkspace),
            _ => Err(format!("unknown command id: {command_id}")),
        }
    }

    pub fn route_command(&mut self, command: AppShellCommand) -> Result<(), String> {
        match command {
            AppShellCommand::NewWindow => {
                let window_id = self.next_id("window");
                let title = format!("Window {}", self.state.windows.len() + 1);
                let workspace = self.make_workspace("Default".to_string());
                self.state.windows.push(WindowState {
                    id: window_id,
                    title,
                    workspaces: vec![workspace],
                    active_workspace: 0,
                });
                self.state.active_window = self.state.windows.len() - 1;
                Ok(())
            }
            AppShellCommand::SelectNextWindow => {
                if self.state.windows.is_empty() {
                    return Err("no windows available".to_string());
                }
                self.state.active_window =
                    (self.state.active_window + 1) % self.state.windows.len();
                Ok(())
            }
            AppShellCommand::CreateWorkspace { name } => {
                let workspace = self.make_workspace(name);
                let window = self.active_window_mut()?;
                window.workspaces.push(workspace);
                window.active_workspace = window.workspaces.len() - 1;
                Ok(())
            }
            AppShellCommand::SelectNextWorkspace => {
                let window = self.active_window_mut()?;
                if window.workspaces.is_empty() {
                    return Err("active window has no workspaces".to_string());
                }
                window.active_workspace = (window.active_workspace + 1) % window.workspaces.len();
                Ok(())
            }
        }
    }

    pub fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.state_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let data = serde_json::to_vec_pretty(&self.state)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        fs::write(&self.state_path, data)?;
        Ok(())
    }

    pub fn startup_summary(&self) -> String {
        let active_window = self.state.windows.get(self.state.active_window);
        let workspace_count = active_window.map(|w| w.workspaces.len()).unwrap_or(0);
        format!(
            "AppShell version={}, windows={}, active_window={}, active_window_workspaces={}",
            self.state.version,
            self.state.windows.len(),
            self.state.active_window,
            workspace_count
        )
    }

    fn register_builtin_commands(&mut self) {
        self.commands.register(CommandAction {
            id: "window.new".to_string(),
            title: "New Window".to_string(),
            description: "Create a new app shell window".to_string(),
        });
        self.commands.register(CommandAction {
            id: "window.next".to_string(),
            title: "Next Window".to_string(),
            description: "Select the next window in app shell".to_string(),
        });
        self.commands.register(CommandAction {
            id: "workspace.new".to_string(),
            title: "New Workspace".to_string(),
            description: "Create a workspace in the active window".to_string(),
        });
        self.commands.register(CommandAction {
            id: "workspace.next".to_string(),
            title: "Next Workspace".to_string(),
            description: "Select next workspace in active window".to_string(),
        });
    }

    fn default_state() -> AppShellState {
        let initial_workspace = Workspace {
            id: "workspace-1".to_string(),
            name: "Default".to_string(),
            tabs: vec![Tab {
                id: "tab-2".to_string(),
                title: "main".to_string(),
                panes: vec![Pane {
                    id: "pane-3".to_string(),
                    surfaces: vec![Surface {
                        id: "surface-4".to_string(),
                        session_id: "session-5".to_string(),
                        cwd: "/".to_string(),
                    }],
                    active_surface: 0,
                }],
                active_pane: 0,
            }],
            active_tab: 0,
        };

        AppShellState {
            version: APP_STATE_VERSION,
            windows: vec![WindowState {
                id: "window-6".to_string(),
                title: "Window 1".to_string(),
                workspaces: vec![initial_workspace],
                active_workspace: 0,
            }],
            active_window: 0,
            settings: AppSettings::default(),
            next_id: 6,
            last_started_at_ms: now_ms(),
        }
    }

    fn load_from_path(path: &Path) -> io::Result<AppShellState> {
        let bytes = fs::read(path)?;
        let state = serde_json::from_slice::<AppShellState>(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(state)
    }

    fn active_window_mut(&mut self) -> Result<&mut WindowState, String> {
        self.state
            .windows
            .get_mut(self.state.active_window)
            .ok_or_else(|| "active window missing".to_string())
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.state.next_id += 1;
        format!("{prefix}-{}", self.state.next_id)
    }

    fn make_workspace(&mut self, name: String) -> Workspace {
        Workspace {
            id: self.next_id("workspace"),
            name,
            tabs: vec![Tab {
                id: self.next_id("tab"),
                title: "main".to_string(),
                panes: vec![Pane {
                    id: self.next_id("pane"),
                    surfaces: vec![Surface {
                        id: self.next_id("surface"),
                        session_id: self.next_id("session"),
                        cwd: "/".to_string(),
                    }],
                    active_surface: 0,
                }],
                active_pane: 0,
            }],
            active_tab: 0,
        }
    }
}

pub fn default_state_path() -> PathBuf {
    if let Ok(path) = std::env::var("ULGEN_STATE_PATH") {
        return PathBuf::from(path);
    }

    platform_state_dir()
        .unwrap_or_else(|| std::env::temp_dir().join("ulgen"))
        .join("state.json")
}

fn platform_state_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
            .map(|root| root.join("Ulgen"));
    }

    #[cfg(target_os = "macos")]
    {
        return std::env::var_os("HOME").map(PathBuf::from).map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("Ulgen")
        });
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
            return Some(PathBuf::from(path).join("ulgen"));
        }
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".local").join("state").join("ulgen"));
    }

    #[allow(unreachable_code)]
    None
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_state_path() -> PathBuf {
        std::env::temp_dir().join(format!("ulgen-app-shell-test-{}.json", now_ms()))
    }

    #[test]
    fn defaults_when_state_file_missing() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let shell = AppShell::bootstrap(path).unwrap();
        assert_eq!(shell.state().windows.len(), 1);
        assert_eq!(shell.state().active_window, 0);
        assert_eq!(shell.state().windows[0].workspaces[0].name, "Default");
    }

    #[test]
    fn restores_state_after_save() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell.route_command(AppShellCommand::NewWindow).unwrap();
        shell
            .route_command(AppShellCommand::CreateWorkspace {
                name: "api".to_string(),
            })
            .unwrap();
        shell.save().unwrap();

        let restored = AppShell::bootstrap(path.clone()).unwrap();
        assert_eq!(restored.state().windows.len(), 2);
        assert_eq!(restored.state().active_window, 1);
        assert_eq!(
            restored.state().windows[1].workspaces[1].name,
            "api".to_string()
        );

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }
}
