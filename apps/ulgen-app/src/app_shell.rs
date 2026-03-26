use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ulgen_command::{
    command_ids, resolve_keymap, CommandAction, CommandRegistry, KeyBinding,
    KeymapProfile as CommandKeymapProfile, ResolvedKeymap,
};
use ulgen_domain::{Pane, Surface, Tab, Workspace};
use ulgen_settings::{AppSettings, KeymapOverride, KeymapProfile};

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
    #[serde(default)]
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
    SelectPreviousWorkspace,
    CreateTab,
    SelectNextTab,
    SelectPreviousTab,
    SelectNextPane,
    SelectPreviousPane,
    SplitPaneRight,
    SplitPaneDown,
}

#[derive(Clone)]
struct KeymapCache {
    profile: KeymapProfile,
    overrides: Vec<KeymapOverride>,
    resolved: ResolvedKeymap,
}

pub struct AppShell {
    state: AppShellState,
    state_path: PathBuf,
    commands: CommandRegistry,
    keymap_cache: Option<KeymapCache>,
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
            keymap_cache: None,
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

    pub fn resolve_active_keymap(&mut self) -> ResolvedKeymap {
        let profile = self.state.settings.keymap_profile;
        let overrides = self.state.settings.keymap_overrides.clone();

        if let Some(cache) = &self.keymap_cache {
            if cache.profile == profile && cache.overrides == overrides {
                return cache.resolved.clone();
            }
        }

        let profile = match self.state.settings.keymap_profile {
            KeymapProfile::Warp => CommandKeymapProfile::Warp,
            KeymapProfile::Tmux => CommandKeymapProfile::Tmux,
        };
        let override_bindings = self
            .state
            .settings
            .keymap_overrides
            .iter()
            .map(|override_entry| KeyBinding {
                chord: override_entry.chord.clone(),
                command_id: override_entry.command_id.clone(),
            })
            .collect::<Vec<_>>();
        let resolved = resolve_keymap(profile, &override_bindings);

        self.keymap_cache = Some(KeymapCache {
            profile: self.state.settings.keymap_profile,
            overrides: self.state.settings.keymap_overrides.clone(),
            resolved: resolved.clone(),
        });

        resolved
    }

    pub fn route_key_chord(&mut self, chord: &str) -> Result<(), String> {
        let keymap = self.resolve_active_keymap();
        let command_id = keymap
            .command_for_chord(chord)
            .ok_or_else(|| format!("no command bound to chord: {chord}"))?
            .to_string();
        self.route_command_id(&command_id)
    }

    pub fn route_command_id(&mut self, command_id: &str) -> Result<(), String> {
        match command_id {
            command_ids::WINDOW_NEW => self.route_command(AppShellCommand::NewWindow),
            command_ids::WINDOW_NEXT => self.route_command(AppShellCommand::SelectNextWindow),
            command_ids::WORKSPACE_NEW => self.route_command(AppShellCommand::CreateWorkspace {
                name: "workspace".to_string(),
            }),
            command_ids::WORKSPACE_NEXT => self.route_command(AppShellCommand::SelectNextWorkspace),
            command_ids::WORKSPACE_PREV => {
                self.route_command(AppShellCommand::SelectPreviousWorkspace)
            }
            command_ids::TAB_NEW => self.route_command(AppShellCommand::CreateTab),
            command_ids::TAB_NEXT => self.route_command(AppShellCommand::SelectNextTab),
            command_ids::TAB_PREV => self.route_command(AppShellCommand::SelectPreviousTab),
            command_ids::PANE_NEXT => self.route_command(AppShellCommand::SelectNextPane),
            command_ids::PANE_PREV => self.route_command(AppShellCommand::SelectPreviousPane),
            command_ids::PANE_SPLIT_RIGHT => self.route_command(AppShellCommand::SplitPaneRight),
            command_ids::PANE_SPLIT_DOWN => self.route_command(AppShellCommand::SplitPaneDown),
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
            AppShellCommand::SelectPreviousWorkspace => {
                let window = self.active_window_mut()?;
                if window.workspaces.is_empty() {
                    return Err("active window has no workspaces".to_string());
                }
                window.active_workspace =
                    previous_index(window.active_workspace, window.workspaces.len());
                Ok(())
            }
            AppShellCommand::CreateTab => {
                let cwd = self.active_surface_cwd().unwrap_or_else(|| "/".to_string());
                let tab = self.make_tab("main".to_string(), cwd);
                let workspace = self.active_workspace_mut()?;
                workspace.tabs.push(tab);
                workspace.active_tab = workspace.tabs.len() - 1;
                Ok(())
            }
            AppShellCommand::SelectNextTab => {
                let workspace = self.active_workspace_mut()?;
                if workspace.tabs.is_empty() {
                    return Err("active workspace has no tabs".to_string());
                }
                workspace.active_tab = (workspace.active_tab + 1) % workspace.tabs.len();
                Ok(())
            }
            AppShellCommand::SelectPreviousTab => {
                let workspace = self.active_workspace_mut()?;
                if workspace.tabs.is_empty() {
                    return Err("active workspace has no tabs".to_string());
                }
                workspace.active_tab = previous_index(workspace.active_tab, workspace.tabs.len());
                Ok(())
            }
            AppShellCommand::SelectNextPane => {
                let tab = self.active_tab_mut()?;
                if tab.panes.is_empty() {
                    return Err("active tab has no panes".to_string());
                }
                tab.active_pane = (tab.active_pane + 1) % tab.panes.len();
                Ok(())
            }
            AppShellCommand::SelectPreviousPane => {
                let tab = self.active_tab_mut()?;
                if tab.panes.is_empty() {
                    return Err("active tab has no panes".to_string());
                }
                tab.active_pane = previous_index(tab.active_pane, tab.panes.len());
                Ok(())
            }
            AppShellCommand::SplitPaneRight | AppShellCommand::SplitPaneDown => {
                let cwd = self.active_surface_cwd().unwrap_or_else(|| "/".to_string());
                let pane = self.make_pane(cwd);
                let tab = self.active_tab_mut()?;
                tab.panes.push(pane);
                tab.active_pane = tab.panes.len() - 1;
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
        let temp_path = temp_save_path(&self.state_path);
        fs::write(&temp_path, data)?;
        replace_state_file(&temp_path, &self.state_path)?;
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
            id: command_ids::WINDOW_NEW.to_string(),
            title: "New Window".to_string(),
            description: "Create a new app shell window".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::WINDOW_NEXT.to_string(),
            title: "Next Window".to_string(),
            description: "Select the next window in app shell".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::WORKSPACE_NEW.to_string(),
            title: "New Workspace".to_string(),
            description: "Create a workspace in the active window".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::WORKSPACE_NEXT.to_string(),
            title: "Next Workspace".to_string(),
            description: "Select next workspace in active window".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::WORKSPACE_PREV.to_string(),
            title: "Previous Workspace".to_string(),
            description: "Select previous workspace in active window".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::TAB_NEW.to_string(),
            title: "New Tab".to_string(),
            description: "Create a tab in the active workspace".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::TAB_NEXT.to_string(),
            title: "Next Tab".to_string(),
            description: "Select next tab in active workspace".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::TAB_PREV.to_string(),
            title: "Previous Tab".to_string(),
            description: "Select previous tab in active workspace".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::PANE_NEXT.to_string(),
            title: "Next Pane".to_string(),
            description: "Select next pane in active tab".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::PANE_PREV.to_string(),
            title: "Previous Pane".to_string(),
            description: "Select previous pane in active tab".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::PANE_SPLIT_RIGHT.to_string(),
            title: "Split Pane Right".to_string(),
            description: "Split active pane and focus the new right pane".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::PANE_SPLIT_DOWN.to_string(),
            title: "Split Pane Down".to_string(),
            description: "Split active pane and focus the new lower pane".to_string(),
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

    fn active_workspace_mut(&mut self) -> Result<&mut Workspace, String> {
        let window = self.active_window_mut()?;
        window
            .workspaces
            .get_mut(window.active_workspace)
            .ok_or_else(|| "active workspace missing".to_string())
    }

    fn active_tab_mut(&mut self) -> Result<&mut Tab, String> {
        let workspace = self.active_workspace_mut()?;
        workspace
            .tabs
            .get_mut(workspace.active_tab)
            .ok_or_else(|| "active tab missing".to_string())
    }

    fn active_surface_cwd(&self) -> Option<String> {
        let window = self.state.windows.get(self.state.active_window)?;
        let workspace = window.workspaces.get(window.active_workspace)?;
        let tab = workspace.tabs.get(workspace.active_tab)?;
        let pane = tab.panes.get(tab.active_pane)?;
        let surface = pane.surfaces.get(pane.active_surface)?;
        Some(surface.cwd.clone())
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.state.next_id += 1;
        format!("{prefix}-{}", self.state.next_id)
    }

    fn make_pane(&mut self, cwd: String) -> Pane {
        Pane {
            id: self.next_id("pane"),
            surfaces: vec![Surface {
                id: self.next_id("surface"),
                session_id: self.next_id("session"),
                cwd,
            }],
            active_surface: 0,
        }
    }

    fn make_tab(&mut self, title: String, cwd: String) -> Tab {
        Tab {
            id: self.next_id("tab"),
            title,
            panes: vec![self.make_pane(cwd)],
            active_pane: 0,
        }
    }

    fn make_workspace(&mut self, name: String) -> Workspace {
        Workspace {
            id: self.next_id("workspace"),
            name,
            tabs: vec![self.make_tab("main".to_string(), "/".to_string())],
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

fn previous_index(current: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    if current == 0 {
        return len - 1;
    }
    current - 1
}

fn temp_save_path(target: &Path) -> PathBuf {
    let mut tmp_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("state.json")
        .to_string();
    tmp_name.push_str(&format!(".tmp-{}-{}", process::id(), now_ms()));
    target.with_file_name(tmp_name)
}

fn replace_state_file(temp_path: &Path, target_path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::rename(temp_path, target_path)
    }

    #[cfg(not(unix))]
    {
        let backup_path = target_path.with_file_name(format!(
            "{}.bak-{}",
            target_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("state.json"),
            now_ms()
        ));

        if target_path.exists() {
            fs::rename(target_path, &backup_path)?;
            if let Err(err) = fs::rename(temp_path, target_path) {
                let _ = fs::rename(&backup_path, target_path);
                return Err(err);
            }
            let _ = fs::remove_file(backup_path);
            return Ok(());
        }

        fs::rename(temp_path, target_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use ulgen_command::command_ids;
    use ulgen_domain::{Pane, Surface, Tab};
    use ulgen_settings::{KeymapOverride, KeymapProfile};

    static TEST_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_state_path() -> PathBuf {
        let seq = TEST_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ulgen-app-shell-test-{}-{}-{}.json",
            process::id(),
            now_ms(),
            seq
        ))
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

    #[test]
    fn save_replaces_existing_file_with_valid_json() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell
            .route_command(AppShellCommand::CreateWorkspace {
                name: "first".to_string(),
            })
            .unwrap();
        shell.save().unwrap();

        shell
            .route_command(AppShellCommand::CreateWorkspace {
                name: "second".to_string(),
            })
            .unwrap();
        shell.save().unwrap();

        let restored = AppShell::bootstrap(path.clone()).unwrap();
        let active_window = &restored.state().windows[restored.state().active_window];
        assert_eq!(active_window.workspaces.last().unwrap().name, "second");

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn command_ids_support_tab_and_pane_navigation() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let active_window = &mut shell.state.windows[shell.state.active_window];
        let active_workspace = &mut active_window.workspaces[active_window.active_workspace];
        active_workspace.tabs.push(Tab {
            id: "tab-extra".to_string(),
            title: "extra".to_string(),
            panes: vec![Pane {
                id: "pane-extra".to_string(),
                surfaces: vec![Surface {
                    id: "surface-extra".to_string(),
                    session_id: "session-extra".to_string(),
                    cwd: "/tmp".to_string(),
                }],
                active_surface: 0,
            }],
            active_pane: 0,
        });

        shell.route_command_id(command_ids::TAB_NEXT).unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].workspaces[0].active_tab,
            1
        );

        shell
            .route_command_id(command_ids::PANE_SPLIT_RIGHT)
            .unwrap();
        let tab = &shell.state.windows[shell.state.active_window].workspaces[0].tabs[1];
        assert_eq!(tab.panes.len(), 2);
        assert_eq!(tab.active_pane, 1);

        shell.route_command_id(command_ids::PANE_NEXT).unwrap();
        let tab = &shell.state.windows[shell.state.active_window].workspaces[0].tabs[1];
        assert_eq!(tab.active_pane, 0);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn route_key_chord_honors_profile_defaults() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell.state.settings.keymap_profile = KeymapProfile::Tmux;

        shell.route_key_chord("CTRL+B C").unwrap();
        assert_eq!(shell.state.windows.len(), 2);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn conflicting_override_is_reported_and_keeps_existing_binding() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell.state.settings.keymap_overrides.push(KeymapOverride {
            chord: "ctrl+tab".to_string(),
            command_id: command_ids::WORKSPACE_NEXT.to_string(),
        });

        let resolved = shell.resolve_active_keymap();
        assert_eq!(resolved.rejected_overrides().len(), 1);
        assert_eq!(
            resolved.command_for_chord("ctrl+tab"),
            Some(command_ids::TAB_NEXT)
        );

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn valid_override_can_drive_command_routing() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell
            .route_command(AppShellCommand::CreateWorkspace {
                name: "ops".to_string(),
            })
            .unwrap();
        shell
            .route_command(AppShellCommand::SelectNextWorkspace)
            .unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            0
        );

        shell.state.settings.keymap_overrides.push(KeymapOverride {
            chord: "alt+.".to_string(),
            command_id: command_ids::WORKSPACE_NEXT.to_string(),
        });
        shell.route_key_chord("alt+.").unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            1
        );

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn previous_navigation_wraps_for_workspace_tab_and_pane() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell
            .route_command(AppShellCommand::CreateWorkspace {
                name: "ops".to_string(),
            })
            .unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            1
        );

        shell.route_command_id(command_ids::WORKSPACE_PREV).unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            0
        );
        shell.route_command_id(command_ids::WORKSPACE_PREV).unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            1
        );

        shell.route_command_id(command_ids::TAB_NEW).unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].workspaces[1].active_tab,
            1
        );
        shell.route_command_id(command_ids::TAB_PREV).unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].workspaces[1].active_tab,
            0
        );

        shell
            .route_command_id(command_ids::PANE_SPLIT_RIGHT)
            .unwrap();
        shell.route_command_id(command_ids::PANE_PREV).unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].workspaces[1].tabs[0].active_pane,
            0
        );

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn restores_legacy_state_without_settings_field() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let legacy_state = r#"{
            "version":1,
            "windows":[
                {
                    "id":"window-1",
                    "title":"Window 1",
                    "workspaces":[
                        {
                            "id":"workspace-1",
                            "name":"Default",
                            "tabs":[
                                {
                                    "id":"tab-1",
                                    "title":"main",
                                    "panes":[
                                        {
                                            "id":"pane-1",
                                            "surfaces":[
                                                {
                                                    "id":"surface-1",
                                                    "session_id":"session-1",
                                                    "cwd":"/"
                                                }
                                            ],
                                            "active_surface":0
                                        }
                                    ],
                                    "active_pane":0
                                }
                            ],
                            "active_tab":0
                        }
                    ],
                    "active_workspace":0
                }
            ],
            "active_window":0,
            "next_id":1,
            "last_started_at_ms":0
        }"#;

        fs::write(&path, legacy_state).unwrap();
        let shell = AppShell::bootstrap(path.clone()).unwrap();
        assert_eq!(shell.state.settings.keymap_profile, KeymapProfile::Warp);
        assert!(shell.state.settings.keymap_overrides.is_empty());

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn baseline_commands_match_registry_and_route_ids() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();

        let mut registry_ids = shell
            .command_registry()
            .search("")
            .into_iter()
            .map(|action| action.id)
            .collect::<Vec<_>>();
        registry_ids.sort();

        let mut baseline_ids = ulgen_command::baseline_command_ids()
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>();
        baseline_ids.sort();

        assert_eq!(registry_ids, baseline_ids);
        for command_id in ulgen_command::baseline_command_ids() {
            if let Err(error) = shell.route_command_id(command_id) {
                assert!(
                    !error.contains("unknown command id"),
                    "unexpected unknown command id error for {command_id}: {error}"
                );
            }
        }

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn persists_keymap_overrides_across_save_and_restore() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell.state.settings.keymap_overrides.push(KeymapOverride {
            chord: "alt+.".to_string(),
            command_id: command_ids::WORKSPACE_NEXT.to_string(),
        });
        shell.save().unwrap();

        let mut restored = AppShell::bootstrap(path.clone()).unwrap();
        assert_eq!(restored.state.settings.keymap_overrides.len(), 1);
        assert_eq!(
            restored.state.settings.keymap_overrides[0].command_id,
            command_ids::WORKSPACE_NEXT
        );

        restored
            .route_command(AppShellCommand::CreateWorkspace {
                name: "ops".to_string(),
            })
            .unwrap();
        restored
            .route_command(AppShellCommand::SelectNextWorkspace)
            .unwrap();
        restored.route_key_chord("alt+.").unwrap();
        assert_eq!(
            restored.state.windows[restored.state.active_window].active_workspace,
            1
        );

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }
}
