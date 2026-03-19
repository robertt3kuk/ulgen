use std::collections::BTreeMap;

use ulgen_domain::{Pane, Surface, Tab, Workspace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncScope {
    CurrentTab,
    AllTabs,
    AllWorkspaces,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
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

pub trait MuxRpc {
    fn handle(&mut self, request: MuxRequest) -> Result<MuxResponse, MuxError>;
}

#[derive(Default)]
pub struct MuxState {
    pub workspaces: Vec<Workspace>,
    pub active_workspace: usize,
    pub detached_sessions: BTreeMap<String, String>,
    pub sync_scope: Option<SyncScope>,
    next_id: u64,
}

impl MuxState {
    pub fn new() -> Self {
        let mut state = Self::default();
        let initial = state.create_workspace("Default".to_string());
        state.workspaces.push(initial);
        state
    }

    fn generate_id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{}-{}", prefix, self.next_id)
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
                    .position(|w| w.id == workspace_id)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
