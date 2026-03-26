use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use ulgen_domain::{Pane, Surface, Tab, Workspace};

static JOURNAL_TMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const JOURNAL_VERSION: u32 = 1;

mod socket_api;

pub use socket_api::{
    handle_rpc_line, serve_connection, serve_unix_socket_once, RpcErrorBody, RpcErrorCode,
    RpcResponseEnvelope, SocketApiError, DEFAULT_MAX_REQUEST_BYTES, RPC_VERSION_V0,
};

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
    PaneFocus { pane_id: String },
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
    PaneFocus { pane_id: String },
    SurfaceSendText { targeted_sessions: Vec<String> },
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

    fn active_workspace(&self) -> Result<&Workspace, MuxError> {
        self.workspaces.get(self.active_workspace).ok_or_else(|| {
            MuxError::InvalidState("active workspace index out of bounds".to_string())
        })
    }

    fn active_tab(&self) -> Result<&Tab, MuxError> {
        let workspace = self.active_workspace()?;
        workspace
            .tabs
            .get(workspace.active_tab)
            .ok_or_else(|| MuxError::InvalidState("active tab missing".to_string()))
    }

    fn active_surface(&self) -> Result<&Surface, MuxError> {
        let tab = self.active_tab()?;
        let pane = tab
            .panes
            .get(tab.active_pane)
            .ok_or_else(|| MuxError::InvalidState("active pane missing".to_string()))?;
        pane.surfaces
            .get(pane.active_surface)
            .ok_or_else(|| MuxError::InvalidState("active surface missing".to_string()))
    }

    fn collect_session_ids_from_tab(tab: &Tab, sessions: &mut BTreeSet<String>) {
        for pane in &tab.panes {
            if let Some(surface) = pane.surfaces.get(pane.active_surface) {
                sessions.insert(surface.session_id.clone());
            }
        }
    }

    fn scoped_session_ids(&self, scope: Option<SyncScope>) -> Result<Vec<String>, MuxError> {
        let mut sessions = BTreeSet::new();
        match scope {
            None => {
                sessions.insert(self.active_surface()?.session_id.clone());
            }
            Some(SyncScope::CurrentTab) => {
                let tab = self.active_tab()?;
                Self::collect_session_ids_from_tab(tab, &mut sessions);
            }
            Some(SyncScope::AllTabs) => {
                let workspace = self.active_workspace()?;
                for tab in &workspace.tabs {
                    Self::collect_session_ids_from_tab(tab, &mut sessions);
                }
            }
            Some(SyncScope::AllWorkspaces) => {
                for workspace in &self.workspaces {
                    for tab in &workspace.tabs {
                        Self::collect_session_ids_from_tab(tab, &mut sessions);
                    }
                }
            }
        }

        if sessions.is_empty() {
            return Err(MuxError::InvalidState(
                "sync scope resolved to no sessions".to_string(),
            ));
        }

        Ok(sessions.into_iter().collect())
    }

    fn topology_session_ids(&self) -> BTreeSet<String> {
        let mut sessions = BTreeSet::new();
        for workspace in &self.workspaces {
            for tab in &workspace.tabs {
                for pane in &tab.panes {
                    for surface in &pane.surfaces {
                        sessions.insert(surface.session_id.clone());
                    }
                }
            }
        }
        sessions
    }

    fn session_in_topology(&self, session_id: &str) -> bool {
        self.topology_session_ids().contains(session_id)
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

        let valid_session_ids = self.topology_session_ids();
        self.detached_sessions
            .retain(|session_id, _| valid_session_ids.contains(session_id));

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
            MuxRequest::PaneSplit { direction } => {
                let pane_id = self.generate_id("pane");
                let surface_id = self.generate_id("surface");
                let session_id = self.generate_id("session");
                let workspace = self.active_workspace_mut()?;
                let tab = workspace
                    .tabs
                    .get_mut(workspace.active_tab)
                    .ok_or_else(|| MuxError::InvalidState("active tab missing".to_string()))?;
                let insert_at = match direction {
                    SplitDirection::Left | SplitDirection::Up => tab.active_pane,
                    SplitDirection::Right | SplitDirection::Down => tab.active_pane + 1,
                };
                tab.panes.insert(
                    insert_at,
                    Pane {
                        id: pane_id.clone(),
                        surfaces: vec![Surface {
                            id: surface_id,
                            session_id,
                            cwd: "/".to_string(),
                        }],
                        active_surface: 0,
                    },
                );
                tab.active_pane = insert_at;
                Ok(MuxResponse::PaneSplit { pane_id })
            }
            MuxRequest::PaneFocus { pane_id } => {
                let workspace = self.active_workspace_mut()?;
                let tab = workspace
                    .tabs
                    .get_mut(workspace.active_tab)
                    .ok_or_else(|| MuxError::InvalidState("active tab missing".to_string()))?;
                let pane_index = tab
                    .panes
                    .iter()
                    .position(|pane| pane.id == pane_id)
                    .ok_or_else(|| MuxError::NotFound("pane not found".to_string()))?;
                tab.active_pane = pane_index;
                Ok(MuxResponse::PaneFocus { pane_id })
            }
            MuxRequest::SurfaceSendText { text: _ } => {
                let scoped_session_ids = self.scoped_session_ids(self.sync_scope)?;
                let targeted_sessions = scoped_session_ids
                    .into_iter()
                    .filter(|session_id| !self.detached_sessions.contains_key(session_id))
                    .collect::<Vec<_>>();

                if targeted_sessions.is_empty() {
                    return Err(MuxError::InvalidState(
                        "no attached sessions available for the current sync scope".to_string(),
                    ));
                }

                Ok(MuxResponse::SurfaceSendText { targeted_sessions })
            }
            MuxRequest::SessionDetach { session_id } => {
                if self.session_in_topology(&session_id) {
                    self.detached_sessions
                        .insert(session_id, "detached".to_string());
                }
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
                    quarantine_corrupt_artifacts(&journal_path)?;
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
    let Some(source_path) = resolve_journal_source_path(path)? else {
        return Ok(None);
    };

    let bytes = fs::read(&source_path).map_err(|error| {
        MuxDaemonError::Io(format!("read journal {}: {error}", source_path.display()))
    })?;

    if bytes.is_empty() {
        return Ok(None);
    }

    let snapshot: MuxJournalSnapshot = serde_json::from_slice(&bytes).map_err(|error| {
        MuxDaemonError::Serialization(format!("parse journal {}: {error}", source_path.display()))
    })?;

    if snapshot.version != JOURNAL_VERSION {
        return Err(MuxDaemonError::UnsupportedJournalVersion(snapshot.version));
    }

    if !snapshot.state.has_valid_topology() {
        return Err(MuxDaemonError::Serialization(format!(
            "invalid topology in journal {}",
            source_path.display()
        )));
    }

    let mut state = snapshot.state;
    state.prepare_for_runtime();

    if source_path != path && !path.exists() {
        let _ = fs::rename(&source_path, path);
    }

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

    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let parent_dir = File::open(parent).map_err(|error| {
                MuxDaemonError::Io(format!("open journal dir {}: {error}", parent.display()))
            })?;
            parent_dir.sync_all().map_err(|error| {
                MuxDaemonError::Io(format!("sync journal dir {}: {error}", parent.display()))
            })?;
        }
    }

    Ok(())
}

fn resolve_journal_source_path(path: &Path) -> Result<Option<PathBuf>, MuxDaemonError> {
    if path.exists() {
        return Ok(Some(path.to_path_buf()));
    }

    let backups = list_sidecar_paths(path, "bak")?;
    if backups.is_empty() {
        return Ok(None);
    }

    let mut best: Option<(SystemTime, PathBuf)> = None;
    for candidate in backups {
        let modified = fs::metadata(&candidate)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        match &best {
            Some((current, _)) if modified <= *current => {}
            _ => best = Some((modified, candidate)),
        }
    }

    Ok(best.map(|(_, path)| path))
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

fn list_sidecar_paths(path: &Path, marker: &str) -> Result<Vec<PathBuf>, MuxDaemonError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mux-state.json");
    let prefix = format!("{file_name}.{marker}-");
    let parent = path.parent().unwrap_or_else(|| Path::new("."));

    if !parent.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(parent)
        .map_err(|error| MuxDaemonError::Io(format!("read dir {}: {error}", parent.display())))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| MuxDaemonError::Io(format!("read dir entry: {error}")))?;
        let candidate = entry.path();
        let candidate_name = candidate.file_name().and_then(|name| name.to_str());
        if candidate_name
            .map(|name| name.starts_with(&prefix))
            .unwrap_or(false)
        {
            paths.push(candidate);
        }
    }

    Ok(paths)
}

fn quarantine_corrupt_artifacts(path: &Path) -> Result<(), MuxDaemonError> {
    let mut candidates = Vec::new();
    if path.exists() {
        candidates.push(path.to_path_buf());
    }
    candidates.extend(list_sidecar_paths(path, "bak")?);

    if candidates.is_empty() {
        return Ok(());
    }

    for candidate in candidates {
        quarantine_corrupt_path(&candidate)?;
    }

    Ok(())
}

fn quarantine_corrupt_path(path: &Path) -> Result<(), MuxDaemonError> {
    let corrupt_path = corrupt_journal_path(path);
    fs::rename(path, &corrupt_path).map_err(|error| {
        MuxDaemonError::Io(format!(
            "quarantine corrupt journal {} -> {}: {error}",
            path.display(),
            corrupt_path.display()
        ))
    })
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

    fn cleanup_journal_artifacts(path: &Path) {
        let _ = fs::remove_file(path);
        for marker in ["tmp", "bak", "corrupt"] {
            for sidecar in find_sidecar_paths(path, marker) {
                let _ = fs::remove_file(sidecar);
            }
        }
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
    fn split_direction_controls_insert_position() {
        let mut mux = MuxState::new();
        let original_pane_id = mux.workspaces[mux.active_workspace].tabs[0].panes[0]
            .id
            .clone();

        let right_split = mux
            .handle(MuxRequest::PaneSplit {
                direction: SplitDirection::Right,
            })
            .unwrap();
        let right_pane_id = match right_split {
            MuxResponse::PaneSplit { pane_id } => pane_id,
            _ => panic!("unexpected response"),
        };
        assert_eq!(mux.workspaces[mux.active_workspace].tabs[0].active_pane, 1);
        assert_eq!(
            mux.workspaces[mux.active_workspace].tabs[0].panes[1].id,
            right_pane_id
        );

        mux.handle(MuxRequest::PaneFocus {
            pane_id: original_pane_id,
        })
        .unwrap();
        let left_split = mux
            .handle(MuxRequest::PaneSplit {
                direction: SplitDirection::Left,
            })
            .unwrap();
        let left_pane_id = match left_split {
            MuxResponse::PaneSplit { pane_id } => pane_id,
            _ => panic!("unexpected response"),
        };
        assert_eq!(mux.workspaces[mux.active_workspace].tabs[0].active_pane, 0);
        assert_eq!(
            mux.workspaces[mux.active_workspace].tabs[0].panes[0].id,
            left_pane_id
        );

        let mut mux_vertical = MuxState::new();
        let original_vertical = mux_vertical.workspaces[mux_vertical.active_workspace].tabs[0]
            .panes[0]
            .id
            .clone();

        let down_split = mux_vertical
            .handle(MuxRequest::PaneSplit {
                direction: SplitDirection::Down,
            })
            .unwrap();
        let down_pane_id = match down_split {
            MuxResponse::PaneSplit { pane_id } => pane_id,
            _ => panic!("unexpected response"),
        };
        assert_eq!(
            mux_vertical.workspaces[mux_vertical.active_workspace].tabs[0].active_pane,
            1
        );
        assert_eq!(
            mux_vertical.workspaces[mux_vertical.active_workspace].tabs[0].panes[1].id,
            down_pane_id
        );

        mux_vertical
            .handle(MuxRequest::PaneFocus {
                pane_id: original_vertical,
            })
            .unwrap();
        let up_split = mux_vertical
            .handle(MuxRequest::PaneSplit {
                direction: SplitDirection::Up,
            })
            .unwrap();
        let up_pane_id = match up_split {
            MuxResponse::PaneSplit { pane_id } => pane_id,
            _ => panic!("unexpected response"),
        };
        assert_eq!(
            mux_vertical.workspaces[mux_vertical.active_workspace].tabs[0].active_pane,
            0
        );
        assert_eq!(
            mux_vertical.workspaces[mux_vertical.active_workspace].tabs[0].panes[0].id,
            up_pane_id
        );
    }

    #[test]
    fn focus_selects_requested_pane() {
        let mut mux = MuxState::new();
        let original_pane_id = mux.workspaces[mux.active_workspace].tabs[0].panes[0]
            .id
            .clone();

        mux.handle(MuxRequest::PaneSplit {
            direction: SplitDirection::Right,
        })
        .unwrap();

        mux.handle(MuxRequest::PaneFocus {
            pane_id: original_pane_id.clone(),
        })
        .unwrap();
        assert_eq!(mux.workspaces[mux.active_workspace].tabs[0].active_pane, 0);

        let result = mux.handle(MuxRequest::PaneFocus {
            pane_id: "pane-missing".to_string(),
        });
        assert!(matches!(result, Err(MuxError::NotFound(_))));
    }

    #[test]
    fn detach_attach_is_idempotent_and_unknown_ids_are_noop() {
        let mut mux = MuxState::new();
        let session_id = mux.workspaces[mux.active_workspace].tabs[0].panes[0].surfaces[0]
            .session_id
            .clone();

        mux.handle(MuxRequest::SessionDetach {
            session_id: "session-missing".to_string(),
        })
        .unwrap();
        assert!(!mux.detached_sessions.contains_key("session-missing"));

        mux.handle(MuxRequest::SessionDetach {
            session_id: session_id.clone(),
        })
        .unwrap();
        assert!(mux.detached_sessions.contains_key(&session_id));

        mux.handle(MuxRequest::SessionDetach {
            session_id: session_id.clone(),
        })
        .unwrap();
        assert_eq!(mux.detached_sessions.len(), 1);

        mux.handle(MuxRequest::SessionAttach {
            session_id: session_id.clone(),
        })
        .unwrap();
        assert!(!mux.detached_sessions.contains_key(&session_id));

        mux.handle(MuxRequest::SessionAttach {
            session_id: session_id.clone(),
        })
        .unwrap();
        assert!(!mux.detached_sessions.contains_key(&session_id));

        mux.handle(MuxRequest::SessionAttach {
            session_id: "session-missing".to_string(),
        })
        .unwrap();
    }

    #[test]
    fn surface_send_text_targets_scope_and_skips_detached_sessions() {
        let mut mux = MuxState::new();
        let workspace0_id = mux.workspaces[0].id.clone();
        let session_one = mux.workspaces[0].tabs[0].panes[0].surfaces[0]
            .session_id
            .clone();

        mux.handle(MuxRequest::PaneSplit {
            direction: SplitDirection::Right,
        })
        .unwrap();
        let session_two = mux.workspaces[0].tabs[0].panes[1].surfaces[0]
            .session_id
            .clone();

        let created = mux
            .handle(MuxRequest::WorkspaceCreate {
                name: "ops".to_string(),
            })
            .unwrap();
        let workspace1_id = match created {
            MuxResponse::WorkspaceCreate { workspace } => workspace.id,
            _ => panic!("unexpected response"),
        };
        let session_three = mux.workspaces[1].tabs[0].panes[0].surfaces[0]
            .session_id
            .clone();

        mux.handle(MuxRequest::WorkspaceSelect {
            workspace_id: workspace0_id,
        })
        .unwrap();
        mux.handle(MuxRequest::SyncSetScope {
            scope: Some(SyncScope::CurrentTab),
        })
        .unwrap();
        let current_tab_targets = mux
            .handle(MuxRequest::SurfaceSendText {
                text: "echo hi".to_string(),
            })
            .unwrap();
        assert_eq!(
            current_tab_targets,
            MuxResponse::SurfaceSendText {
                targeted_sessions: vec![session_one.clone(), session_two.clone()],
            }
        );

        mux.handle(MuxRequest::SessionDetach {
            session_id: session_two.clone(),
        })
        .unwrap();
        let after_detach = mux
            .handle(MuxRequest::SurfaceSendText {
                text: "echo hi".to_string(),
            })
            .unwrap();
        assert_eq!(
            after_detach,
            MuxResponse::SurfaceSendText {
                targeted_sessions: vec![session_one.clone()],
            }
        );

        let extra_tab_id = mux.generate_id("tab");
        let extra_pane_id = mux.generate_id("pane");
        let extra_surface_id = mux.generate_id("surface");
        let extra_session = mux.generate_id("session");
        {
            let workspace = mux.active_workspace_mut().unwrap();
            workspace.tabs.push(Tab {
                id: extra_tab_id,
                title: "extra".to_string(),
                panes: vec![Pane {
                    id: extra_pane_id,
                    surfaces: vec![Surface {
                        id: extra_surface_id,
                        session_id: extra_session.clone(),
                        cwd: "/".to_string(),
                    }],
                    active_surface: 0,
                }],
                active_pane: 0,
            });
        }

        mux.handle(MuxRequest::SyncSetScope {
            scope: Some(SyncScope::AllTabs),
        })
        .unwrap();
        let all_tabs_targets = mux
            .handle(MuxRequest::SurfaceSendText {
                text: "echo hi".to_string(),
            })
            .unwrap();
        let mut actual_tab_targets = match all_tabs_targets {
            MuxResponse::SurfaceSendText { targeted_sessions } => targeted_sessions,
            _ => panic!("unexpected response variant"),
        };
        actual_tab_targets.sort();
        let mut expected_tab_targets = vec![session_one.clone(), extra_session.clone()];
        expected_tab_targets.sort();
        assert_eq!(actual_tab_targets, expected_tab_targets);

        mux.handle(MuxRequest::SyncSetScope {
            scope: Some(SyncScope::AllWorkspaces),
        })
        .unwrap();
        let all_workspaces_targets = mux
            .handle(MuxRequest::SurfaceSendText {
                text: "echo hi".to_string(),
            })
            .unwrap();
        let mut actual_targets = match all_workspaces_targets {
            MuxResponse::SurfaceSendText { targeted_sessions } => targeted_sessions,
            _ => panic!("unexpected response variant"),
        };
        actual_targets.sort();
        let mut expected_targets = vec![session_one, session_three, extra_session];
        expected_targets.sort();
        assert_eq!(actual_targets, expected_targets);

        mux.handle(MuxRequest::WorkspaceSelect {
            workspace_id: workspace1_id,
        })
        .unwrap();
    }

    #[test]
    fn surface_send_text_errors_when_scope_has_only_detached_sessions() {
        let mut mux = MuxState::new();
        let session_id = mux.workspaces[0].tabs[0].panes[0].surfaces[0]
            .session_id
            .clone();
        mux.handle(MuxRequest::SessionDetach { session_id })
            .unwrap();

        let result = mux.handle(MuxRequest::SurfaceSendText {
            text: "echo blocked".to_string(),
        });
        assert!(matches!(result, Err(MuxError::InvalidState(_))));
    }

    #[test]
    fn detach_attach_persists_across_daemon_restore() {
        let path = temp_journal_path("detach-attach-persist");
        let session_id = {
            let mut daemon = MuxDaemon::from_journal_path(&path, RestorePolicy::Always).unwrap();
            let session = daemon.state().workspaces[0].tabs[0].panes[0].surfaces[0]
                .session_id
                .clone();
            daemon
                .handle_persistent(MuxRequest::SessionDetach {
                    session_id: session.clone(),
                })
                .unwrap();
            session
        };

        let mut restored = MuxDaemon::from_journal_path(&path, RestorePolicy::Always).unwrap();
        assert!(restored.state().detached_sessions.contains_key(&session_id));

        restored
            .handle_persistent(MuxRequest::SessionAttach {
                session_id: session_id.clone(),
            })
            .unwrap();

        let final_state = MuxDaemon::from_journal_path(&path, RestorePolicy::Always).unwrap();
        assert!(!final_state
            .state()
            .detached_sessions
            .contains_key(&session_id));

        cleanup_journal_artifacts(&path);
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

        cleanup_journal_artifacts(&path);
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

        cleanup_journal_artifacts(&path);
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
        assert!(!json["state"]["workspaces"].as_array().unwrap().is_empty());

        cleanup_journal_artifacts(&path);
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

        cleanup_journal_artifacts(&parent_file_path);
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
        cleanup_journal_artifacts(&path);
    }

    #[test]
    fn missing_primary_restores_from_backup_snapshot() {
        let path = temp_journal_path("backup-restore");
        let mut state = MuxState::new();
        state
            .handle(MuxRequest::WorkspaceCreate {
                name: "api".to_string(),
            })
            .unwrap();

        let snapshot = MuxJournalSnapshot {
            version: JOURNAL_VERSION,
            state,
        };
        let backup_path = backup_journal_path(&path);
        fs::write(&backup_path, serde_json::to_vec_pretty(&snapshot).unwrap()).unwrap();

        let daemon = MuxDaemon::from_journal_path(&path, RestorePolicy::Always).unwrap();
        assert_eq!(daemon.state().workspaces.len(), 2);
        assert!(path.exists());

        cleanup_journal_artifacts(&path);
        cleanup_journal_artifacts(&backup_path);
    }

    #[test]
    fn unsupported_version_is_quarantined_and_defaults_are_restored() {
        let path = temp_journal_path("bad-version");
        let payload = serde_json::json!({
            "version": JOURNAL_VERSION + 99,
            "state": MuxState::new()
        });
        fs::write(&path, serde_json::to_vec_pretty(&payload).unwrap()).unwrap();

        let daemon = MuxDaemon::from_journal_path(&path, RestorePolicy::Always).unwrap();
        assert_eq!(daemon.state().workspaces.len(), 1);
        assert_eq!(daemon.state().workspaces[0].name, "Default");
        assert!(!path.exists());
        assert!(!find_sidecar_paths(&path, "corrupt").is_empty());

        cleanup_journal_artifacts(&path);
    }

    #[test]
    fn invalid_topology_is_quarantined_and_defaults_are_restored() {
        let path = temp_journal_path("invalid-topology");
        let payload = serde_json::json!({
            "version": JOURNAL_VERSION,
            "state": {
                "workspaces": [],
                "active_workspace": 0,
                "detached_sessions": {},
                "sync_scope": null,
                "next_id": 0
            }
        });
        fs::write(&path, serde_json::to_vec_pretty(&payload).unwrap()).unwrap();

        let daemon = MuxDaemon::from_journal_path(&path, RestorePolicy::Always).unwrap();
        assert_eq!(daemon.state().workspaces.len(), 1);
        assert_eq!(daemon.state().workspaces[0].name, "Default");
        assert!(!path.exists());
        assert!(!find_sidecar_paths(&path, "corrupt").is_empty());

        cleanup_journal_artifacts(&path);
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
        assert_eq!(workspace_id, "ws-46");
        assert!(!daemon.state().detached_sessions.contains_key("session-99"));

        cleanup_journal_artifacts(&path);
    }
}
