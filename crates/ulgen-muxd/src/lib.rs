use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use ulgen_domain::{Pane, Surface, Tab, Workspace};

static JOURNAL_TMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const JOURNAL_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncScope {
    CurrentTab,
    AllTabs,
    AllWorkspaces,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MuxRequest {
    WorkspaceList,
    WorkspaceCreate { name: String },
    WorkspaceSelect { workspace_id: String },
    PaneSplit { direction: SplitDirection },
    SurfaceSendText { text: String },
    SessionDetach { session_id: String },
    SessionAttach { session_id: String },
    SyncSetScope { scope: Option<SyncScope> },
}

impl MuxRequest {
    fn mutates_state(&self) -> bool {
        !matches!(
            self,
            MuxRequest::WorkspaceList | MuxRequest::SurfaceSendText { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MuxResponse {
    WorkspaceList { workspaces: Vec<Workspace> },
    WorkspaceCreate { workspace: Workspace },
    WorkspaceSelect { workspace_id: String },
    PaneSplit { pane_id: String },
    SurfaceSendText,
    SessionDetach,
    SessionAttach,
    SyncSetScope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MuxError {
    NotFound(String),
    InvalidState(String),
}

impl std::fmt::Display for MuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(message) => write!(f, "not found: {message}"),
            Self::InvalidState(message) => write!(f, "invalid state: {message}"),
        }
    }
}

impl std::error::Error for MuxError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MuxDaemonError {
    Io(String),
    Serialization(String),
    UnsupportedJournalVersion(u32),
    State(MuxError),
}

impl std::fmt::Display for MuxDaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => write!(f, "io error: {message}"),
            Self::Serialization(message) => write!(f, "serialization error: {message}"),
            Self::UnsupportedJournalVersion(version) => {
                write!(f, "unsupported mux journal version: {version}")
            }
            Self::State(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for MuxDaemonError {}

impl From<MuxError> for MuxDaemonError {
    fn from(value: MuxError) -> Self {
        Self::State(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestorePolicy {
    Always,
    Never,
}

pub trait MuxRpc {
    fn handle(&mut self, request: MuxRequest) -> Result<MuxResponse, MuxError>;
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MuxState {
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub active_workspace: usize,
    #[serde(default)]
    pub detached_sessions: BTreeMap<String, String>,
    #[serde(default)]
    pub sync_scope: Option<SyncScope>,
    #[serde(default)]
    next_id: u64,
}

impl MuxState {
    pub fn new() -> Self {
        let mut state = Self::default();
        let initial = state.create_workspace("Default".to_string());
        state.workspaces.push(initial);
        state.prepare_for_runtime();
        state
    }

    fn generate_id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}-{}", self.next_id)
    }

    fn create_workspace(&mut self, name: String) -> Workspace {
        let workspace_id = self.generate_id("ws");
        let tab_id = self.generate_id("tab");
        let pane_id = self.generate_id("pane");
        let surface_id = self.generate_id("surface");
        let session_id = self.generate_id("session");

        Workspace {
            id: workspace_id,
            name,
            tabs: vec![Tab {
                id: tab_id,
                title: "main".to_string(),
                panes: vec![Pane {
                    id: pane_id,
                    surfaces: vec![Surface {
                        id: surface_id,
                        session_id,
                        cwd: "/".to_string(),
                    }],
                    active_surface: 0,
                }],
                active_pane: 0,
            }],
            active_tab: 0,
        }
    }

    fn active_workspace_mut(&mut self) -> Result<&mut Workspace, MuxError> {
        self.workspaces
            .get_mut(self.active_workspace)
            .ok_or_else(|| {
                MuxError::InvalidState("active workspace index out of bounds".to_string())
            })
    }

    fn has_valid_topology(&self) -> bool {
        if self.workspaces.is_empty() {
            return false;
        }

        for workspace in &self.workspaces {
            if workspace.tabs.is_empty() {
                return false;
            }

            for tab in &workspace.tabs {
                if tab.panes.is_empty() {
                    return false;
                }

                for pane in &tab.panes {
                    if pane.surfaces.is_empty() {
                        return false;
                    }
                }
            }
        }

        true
    }

    fn prepare_for_runtime(&mut self) {
        if !self.has_valid_topology() {
            *self = Self::new();
            return;
        }

        self.active_workspace = self.active_workspace.min(self.workspaces.len() - 1);

        for workspace in &mut self.workspaces {
            workspace.active_tab = workspace.active_tab.min(workspace.tabs.len() - 1);

            for tab in &mut workspace.tabs {
                tab.active_pane = tab.active_pane.min(tab.panes.len() - 1);

                for pane in &mut tab.panes {
                    pane.active_surface = pane.active_surface.min(pane.surfaces.len() - 1);
                }
            }
        }

        self.reconcile_next_id();
    }

    fn reconcile_next_id(&mut self) {
        let mut max_seen = self.next_id;

        for workspace in &self.workspaces {
            update_max_seen_id(&mut max_seen, &workspace.id);

            for tab in &workspace.tabs {
                update_max_seen_id(&mut max_seen, &tab.id);

                for pane in &tab.panes {
                    update_max_seen_id(&mut max_seen, &pane.id);

                    for surface in &pane.surfaces {
                        update_max_seen_id(&mut max_seen, &surface.id);
                        update_max_seen_id(&mut max_seen, &surface.session_id);
                    }
                }
            }
        }

        for session_id in self.detached_sessions.keys() {
            update_max_seen_id(&mut max_seen, session_id);
        }

        self.next_id = max_seen;
    }
}

impl MuxRpc for MuxState {
    fn handle(&mut self, request: MuxRequest) -> Result<MuxResponse, MuxError> {
        match request {
            MuxRequest::WorkspaceList => Ok(MuxResponse::WorkspaceList {
                workspaces: self.workspaces.clone(),
            }),
            MuxRequest::WorkspaceCreate { name } => {
                let workspace = self.create_workspace(name);
                self.workspaces.push(workspace.clone());
                self.active_workspace = self.workspaces.len() - 1;
                Ok(MuxResponse::WorkspaceCreate { workspace })
            }
            MuxRequest::WorkspaceSelect { workspace_id } => {
                let idx = self
                    .workspaces
                    .iter()
                    .position(|workspace| workspace.id == workspace_id)
                    .ok_or_else(|| MuxError::NotFound("workspace not found".to_string()))?;
                self.active_workspace = idx;
                Ok(MuxResponse::WorkspaceSelect { workspace_id })
            }
            MuxRequest::PaneSplit { direction: _ } => {
                let pane_id = self.generate_id("pane");
                let surface_id = self.generate_id("surface");
                let session_id = self.generate_id("session");
                let workspace = self.active_workspace_mut()?;
                let tab = workspace
                    .tabs
                    .get_mut(workspace.active_tab)
                    .ok_or_else(|| MuxError::InvalidState("active tab missing".to_string()))?;
                tab.panes.push(Pane {
                    id: pane_id.clone(),
                    surfaces: vec![Surface {
                        id: surface_id,
                        session_id,
                        cwd: "/".to_string(),
                    }],
                    active_surface: 0,
                });
                tab.active_pane = tab.panes.len() - 1;
                Ok(MuxResponse::PaneSplit { pane_id })
            }
            MuxRequest::SurfaceSendText { text: _ } => Ok(MuxResponse::SurfaceSendText),
            MuxRequest::SessionDetach { session_id } => {
                self.detached_sessions
                    .insert(session_id, "detached".to_string());
                Ok(MuxResponse::SessionDetach)
            }
            MuxRequest::SessionAttach { session_id } => {
                self.detached_sessions.remove(&session_id);
                Ok(MuxResponse::SessionAttach)
            }
            MuxRequest::SyncSetScope { scope } => {
                self.sync_scope = scope;
                Ok(MuxResponse::SyncSetScope)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct MuxJournalSnapshot {
    version: u32,
    state: MuxState,
}

pub struct MuxDaemon {
    state: MuxState,
    journal_path: PathBuf,
    restore_policy: RestorePolicy,
}

impl MuxDaemon {
    pub fn from_journal_path(
        journal_path: impl AsRef<Path>,
        restore_policy: RestorePolicy,
    ) -> Result<Self, MuxDaemonError> {
        let journal_path = journal_path.as_ref().to_path_buf();
        let state = match restore_policy {
            RestorePolicy::Always => match load_state_from_journal(&journal_path) {
                Ok(Some(state)) => state,
                Ok(None) => MuxState::new(),
                Err(MuxDaemonError::Serialization(_))
                | Err(MuxDaemonError::UnsupportedJournalVersion(_)) => {
                    quarantine_corrupt_journal(&journal_path)?;
                    MuxState::new()
                }
                Err(error) => return Err(error),
            },
            RestorePolicy::Never => MuxState::new(),
        };

        Ok(Self {
            state,
            journal_path,
            restore_policy,
        })
    }

    pub fn state(&self) -> &MuxState {
        &self.state
    }

    pub fn journal_path(&self) -> &Path {
        &self.journal_path
    }

    pub fn restore_policy(&self) -> RestorePolicy {
        self.restore_policy
    }

    pub fn persist_now(&self) -> Result<(), MuxDaemonError> {
        persist_state_to_journal(&self.journal_path, &self.state)
    }

    pub fn handle_persistent(
        &mut self,
        request: MuxRequest,
    ) -> Result<MuxResponse, MuxDaemonError> {
        if request.mutates_state() {
            let mut candidate_state = self.state.clone();
            let response = candidate_state
                .handle(request)
                .map_err(MuxDaemonError::from)?;
            persist_state_to_journal(&self.journal_path, &candidate_state)?;
            self.state = candidate_state;
            Ok(response)
        } else {
            self.state.handle(request).map_err(MuxDaemonError::from)
        }
    }
}

impl MuxRpc for MuxDaemon {
    fn handle(&mut self, request: MuxRequest) -> Result<MuxResponse, MuxError> {
        match self.handle_persistent(request) {
            Ok(response) => Ok(response),
            Err(MuxDaemonError::State(error)) => Err(error),
            Err(error) => Err(MuxError::InvalidState(error.to_string())),
        }
    }
}

fn load_state_from_journal(path: &Path) -> Result<Option<MuxState>, MuxDaemonError> {
    if !path.exists() {
        return Ok(None);
    }

    let bytes = fs::read(path)
        .map_err(|error| MuxDaemonError::Io(format!("read journal {}: {error}", path.display())))?;

    if bytes.is_empty() {
        return Ok(None);
    }

    let snapshot: MuxJournalSnapshot = serde_json::from_slice(&bytes).map_err(|error| {
        MuxDaemonError::Serialization(format!("parse journal {}: {error}", path.display()))
    })?;

    if snapshot.version != JOURNAL_VERSION {
        return Err(MuxDaemonError::UnsupportedJournalVersion(snapshot.version));
    }

    let mut state = snapshot.state;
    state.prepare_for_runtime();
    Ok(Some(state))
}

fn persist_state_to_journal(path: &Path, state: &MuxState) -> Result<(), MuxDaemonError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| {
                MuxDaemonError::Io(format!("create journal dir {}: {error}", parent.display()))
            })?;
        }
    }

    let snapshot = MuxJournalSnapshot {
        version: JOURNAL_VERSION,
        state: state.clone(),
    };
    let serialized = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| MuxDaemonError::Serialization(format!("serialize journal: {error}")))?;

    let temp_path = temporary_journal_path(path);
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temp_path)
        .map_err(|error| {
            MuxDaemonError::Io(format!(
                "open temp journal {}: {error}",
                temp_path.display()
            ))
        })?;
    file.write_all(&serialized).map_err(|error| {
        MuxDaemonError::Io(format!(
            "write temp journal {}: {error}",
            temp_path.display()
        ))
    })?;
    file.sync_all().map_err(|error| {
        MuxDaemonError::Io(format!(
            "sync temp journal {}: {error}",
            temp_path.display()
        ))
    })?;
    drop(file);

    if path.exists() {
        let backup_path = backup_journal_path(path);
        fs::rename(path, &backup_path).map_err(|error| {
            MuxDaemonError::Io(format!(
                "backup existing journal {} -> {}: {error}",
                path.display(),
                backup_path.display()
            ))
        })?;

        if let Err(rename_error) = fs::rename(&temp_path, path) {
            let rollback_result = fs::rename(&backup_path, path);
            let _ = fs::remove_file(&temp_path);
            return match rollback_result {
                Ok(()) => Err(MuxDaemonError::Io(format!(
                    "replace journal rename {} -> {}: {rename_error}",
                    temp_path.display(),
                    path.display()
                ))),
                Err(rollback_error) => Err(MuxDaemonError::Io(format!(
                    "replace journal failed {} -> {}: {rename_error}; rollback failed {} -> {}: {rollback_error}",
                    temp_path.display(),
                    path.display(),
                    backup_path.display(),
                    path.display()
                ))),
            };
        }

        let _ = fs::remove_file(&backup_path);
    } else if let Err(rename_error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(MuxDaemonError::Io(format!(
            "rename temp journal {} -> {}: {rename_error}",
            temp_path.display(),
            path.display()
        )));
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Ok(parent_dir) = File::open(parent) {
                let _ = parent_dir.sync_all();
            }
        }
    }

    Ok(())
}

fn temporary_journal_path(path: &Path) -> PathBuf {
    journal_sidecar_path(path, "tmp")
}

fn backup_journal_path(path: &Path) -> PathBuf {
    journal_sidecar_path(path, "bak")
}

fn corrupt_journal_path(path: &Path) -> PathBuf {
    journal_sidecar_path(path, "corrupt")
}

fn journal_sidecar_path(path: &Path, marker: &str) -> PathBuf {
    let sequence = JOURNAL_TMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mux-state.json");
    path.with_file_name(format!("{file_name}.{marker}-{}-{sequence}", process::id()))
}

fn quarantine_corrupt_journal(path: &Path) -> Result<(), MuxDaemonError> {
    if !path.exists() {
        return Ok(());
    }

    let corrupt_path = corrupt_journal_path(path);
    fs::rename(path, &corrupt_path).map_err(|error| {
        MuxDaemonError::Io(format!(
            "quarantine corrupt journal {} -> {}: {error}",
            path.display(),
            corrupt_path.display()
        ))
    })?;
    Ok(())
}

fn update_max_seen_id(max_seen: &mut u64, id: &str) {
    if let Some(value) = id_numeric_suffix(id) {
        *max_seen = (*max_seen).max(value);
    }
}

fn id_numeric_suffix(id: &str) -> Option<u64> {
    let (_, suffix) = id.rsplit_once('-')?;
    suffix.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_journal_path(label: &str) -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let epoch_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ulgen-muxd-{label}-{}-{epoch_nanos}-{sequence}.json",
            process::id()
        ))
    }

    fn find_sidecar_paths(path: &Path, marker: &str) -> Vec<PathBuf> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let prefix = format!("{file_name}.{marker}-");
        let parent = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(std::env::temp_dir);

        let mut matches = Vec::new();
        if let Ok(entries) = fs::read_dir(parent) {
            for entry in entries.flatten() {
                let candidate = entry.path();
                let candidate_name = candidate.file_name().and_then(|name| name.to_str());
                if candidate_name
                    .map(|name| name.starts_with(&prefix))
                    .unwrap_or(false)
                {
                    matches.push(candidate);
                }
            }
        }

        matches
    }

    #[test]
    fn create_and_select_workspace() {
        let mut mux = MuxState::new();
        let created = mux
            .handle(MuxRequest::WorkspaceCreate {
                name: "api".to_string(),
            })
            .unwrap();

        let ws_id = match created {
            MuxResponse::WorkspaceCreate { workspace } => workspace.id,
            _ => panic!("unexpected response"),
        };

        mux.handle(MuxRequest::WorkspaceSelect {
            workspace_id: ws_id.clone(),
        })
        .unwrap();

        assert_eq!(mux.workspaces[mux.active_workspace].id, ws_id);
    }

    #[test]
    fn split_adds_pane() {
        let mut mux = MuxState::new();

        let before = mux.workspaces[mux.active_workspace].tabs[0].panes.len();
        mux.handle(MuxRequest::PaneSplit {
            direction: SplitDirection::Right,
        })
        .unwrap();
        let after = mux.workspaces[mux.active_workspace].tabs[0].panes.len();

        assert_eq!(after, before + 1);
    }

    #[test]
    fn daemon_persists_and_restores_topology() {
        let path = temp_journal_path("restore");
        let ws_id = {
            let mut daemon = MuxDaemon::from_journal_path(&path, RestorePolicy::Always).unwrap();
            let created = daemon
                .handle_persistent(MuxRequest::WorkspaceCreate {
                    name: "api".to_string(),
                })
                .unwrap();
            let ws_id = match created {
                MuxResponse::WorkspaceCreate { workspace } => workspace.id,
                _ => panic!("unexpected response"),
            };

            daemon
                .handle_persistent(MuxRequest::WorkspaceSelect {
                    workspace_id: ws_id.clone(),
                })
                .unwrap();
            daemon
                .handle_persistent(MuxRequest::PaneSplit {
                    direction: SplitDirection::Right,
                })
                .unwrap();

            ws_id
        };

        let restored = MuxDaemon::from_journal_path(&path, RestorePolicy::Always).unwrap();
        assert_eq!(restored.state.workspaces.len(), 2);
        assert_eq!(
            restored.state.workspaces[restored.state.active_workspace].id,
            ws_id
        );
        assert_eq!(
            restored.state.workspaces[restored.state.active_workspace].tabs[0]
                .panes
                .len(),
            2
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn restore_policy_never_starts_fresh_state() {
        let path = temp_journal_path("no-restore");
        {
            let mut daemon = MuxDaemon::from_journal_path(&path, RestorePolicy::Always).unwrap();
            daemon
                .handle_persistent(MuxRequest::WorkspaceCreate {
                    name: "api".to_string(),
                })
                .unwrap();
        }

        let daemon = MuxDaemon::from_journal_path(&path, RestorePolicy::Never).unwrap();
        assert_eq!(daemon.state.workspaces.len(), 1);
        assert_eq!(daemon.state.workspaces[0].name, "Default");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn daemon_writes_versioned_journal_snapshot() {
        let path = temp_journal_path("journal-shape");
        let mut daemon = MuxDaemon::from_journal_path(&path, RestorePolicy::Always).unwrap();
        daemon
            .handle_persistent(MuxRequest::WorkspaceCreate {
                name: "ops".to_string(),
            })
            .unwrap();

        let contents = fs::read(&path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&contents).unwrap();

        assert_eq!(json["version"].as_u64(), Some(JOURNAL_VERSION as u64));
        assert!(json["state"]["workspaces"].as_array().unwrap().len() >= 1);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn handle_persistent_keeps_state_unchanged_if_persist_fails() {
        let parent_file_path = temp_journal_path("parent-file");
        fs::write(&parent_file_path, b"not-a-directory").unwrap();
        let journal_path = parent_file_path.join("state.json");

        let mut daemon = MuxDaemon::from_journal_path(&journal_path, RestorePolicy::Never).unwrap();
        let before = daemon.state().clone();

        let result = daemon.handle_persistent(MuxRequest::WorkspaceCreate {
            name: "api".to_string(),
        });
        assert!(matches!(result, Err(MuxDaemonError::Io(_))));
        assert_eq!(daemon.state(), &before);

        let _ = fs::remove_file(parent_file_path);
    }

    #[test]
    fn corrupt_journal_is_quarantined_and_restored_from_defaults() {
        let path = temp_journal_path("corrupt");
        fs::write(&path, b"{not-json").unwrap();

        let daemon = MuxDaemon::from_journal_path(&path, RestorePolicy::Always).unwrap();
        assert_eq!(daemon.state().workspaces.len(), 1);
        assert_eq!(daemon.state().workspaces[0].name, "Default");
        assert!(!path.exists());

        let quarantined = find_sidecar_paths(&path, "corrupt");
        assert!(!quarantined.is_empty());

        for sidecar in quarantined {
            let _ = fs::remove_file(sidecar);
        }
    }

    #[test]
    fn reconcile_next_id_avoids_collisions_after_restore() {
        let path = temp_journal_path("reconcile");
        let mut state = MuxState::new();
        state.workspaces[0].id = "ws-41".to_string();
        state.workspaces[0].tabs[0].id = "tab-42".to_string();
        state.workspaces[0].tabs[0].panes[0].id = "pane-43".to_string();
        state.workspaces[0].tabs[0].panes[0].surfaces[0].id = "surface-44".to_string();
        state.workspaces[0].tabs[0].panes[0].surfaces[0].session_id = "session-45".to_string();
        state
            .detached_sessions
            .insert("session-99".to_string(), "detached".to_string());
        state.next_id = 0;

        let snapshot = MuxJournalSnapshot {
            version: JOURNAL_VERSION,
            state,
        };
        let data = serde_json::to_vec_pretty(&snapshot).unwrap();
        fs::write(&path, data).unwrap();

        let mut daemon = MuxDaemon::from_journal_path(&path, RestorePolicy::Always).unwrap();
        let created = daemon
            .handle_persistent(MuxRequest::WorkspaceCreate {
                name: "next".to_string(),
            })
            .unwrap();

        let workspace_id = match created {
            MuxResponse::WorkspaceCreate { workspace } => workspace.id,
            _ => panic!("unexpected response"),
        };
        assert_eq!(workspace_id, "ws-100");

        let _ = fs::remove_file(path);
    }
}
