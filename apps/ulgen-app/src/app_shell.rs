use std::collections::BTreeMap;
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
use ulgen_domain::{
    Block, BlockOutputChunk, BlockStatus, NotificationEvent, Pane, Surface, Tab, Workspace,
};
use ulgen_notify::NotificationBus;
use ulgen_settings::{
    export_theme_definition, import_theme_definition, resolve_theme_with_custom, AppSettings,
    CursorStyle, InputPosition, KeymapOverride, KeymapProfile, ResolvedTheme, SidebarPosition,
    ThemeMode, ThemePreset,
};

const APP_STATE_VERSION: u32 = 2;
const LEGACY_APP_STATE_VERSION: u32 = 1;
const PALETTE_RECENT_LIMIT: usize = 25;

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
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub palette_recent: Vec<String>,
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
    ToggleSidebarPosition,
    SelectNextSidebarTarget,
    SelectPreviousSidebarTarget,
    SetCursorStyle { style: CursorStyle },
    SetInputPosition { position: InputPosition },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarNodeKind {
    Workspace,
    Tab,
    Pane,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarNode {
    pub id: String,
    pub kind: SidebarNodeKind,
    pub title: String,
    pub depth: u8,
    pub is_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarTree {
    pub position: SidebarPosition,
    pub nodes: Vec<SidebarNode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SidebarTarget {
    Workspace {
        workspace_idx: usize,
    },
    Tab {
        workspace_idx: usize,
        tab_idx: usize,
    },
    Pane {
        workspace_idx: usize,
        tab_idx: usize,
        pane_idx: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SidebarEntry {
    node: SidebarNode,
    target: SidebarTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteItemKind {
    Command,
    Workspace,
    Tab,
    Pane,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    pub id: String,
    pub kind: PaletteItemKind,
    pub title: String,
    pub subtitle: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PaletteCandidate {
    item: PaletteItem,
    query_fields: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockNavigationTarget {
    pub block_id: String,
    pub session_id: String,
    pub window_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
}

#[derive(Clone)]
struct KeymapCache {
    profile: KeymapProfile,
    overrides: Vec<KeymapOverride>,
    resolved: ResolvedKeymap,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BlockIndex {
    by_id: BTreeMap<String, usize>,
    by_session: BTreeMap<String, Vec<String>>,
}

impl BlockIndex {
    fn from_blocks(blocks: &[Block]) -> Result<Self, String> {
        let mut index = Self::default();
        for (position, block) in blocks.iter().enumerate() {
            index.record(block, position)?;
        }
        Ok(index)
    }

    fn record(&mut self, block: &Block, position: usize) -> Result<(), String> {
        if self.by_id.contains_key(&block.id) {
            return Err(format!("duplicate block id detected: {}", block.id));
        }

        self.by_id.insert(block.id.clone(), position);
        self.by_session
            .entry(block.session_id.clone())
            .or_default()
            .push(block.id.clone());
        Ok(())
    }

    fn position_for_block_id(&self, block_id: &str) -> Option<usize> {
        self.by_id.get(block_id).copied()
    }
}

pub struct AppShell {
    state: AppShellState,
    state_path: PathBuf,
    commands: CommandRegistry,
    notification_bus: NotificationBus,
    keymap_cache: Option<KeymapCache>,
    block_index: BlockIndex,
    sidebar_selection_id: Option<String>,
}

impl AppShell {
    pub fn bootstrap(state_path: PathBuf) -> io::Result<Self> {
        let state = if state_path.exists() {
            Self::load_from_path(&state_path)?
        } else {
            Self::default_state()
        };
        let notifications_policy = state.settings.notifications_policy;
        let block_index = BlockIndex::from_blocks(&state.blocks)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

        let mut shell = Self {
            state,
            state_path,
            commands: CommandRegistry::new(),
            notification_bus: NotificationBus::new(notifications_policy),
            keymap_cache: None,
            block_index,
            sidebar_selection_id: None,
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

    pub fn sidebar_position(&self) -> SidebarPosition {
        self.state.settings.sidebar_position
    }

    pub fn theme_mode(&self) -> ThemeMode {
        self.state.settings.theme_mode
    }

    pub fn theme_preset(&self) -> ThemePreset {
        self.state.settings.theme_preset
    }

    pub fn cursor_style(&self) -> CursorStyle {
        self.state.settings.cursor_style
    }

    pub fn input_position(&self) -> InputPosition {
        self.state.settings.input_position
    }

    pub fn set_theme_mode(&mut self, mode: ThemeMode) {
        self.state.settings.theme_mode = mode;
    }

    pub fn set_theme_preset(&mut self, preset: ThemePreset) {
        self.state.settings.theme_preset = preset;
    }

    pub fn set_cursor_style(&mut self, style: CursorStyle) {
        self.state.settings.cursor_style = style;
    }

    pub fn set_input_position(&mut self, position: InputPosition) {
        self.state.settings.input_position = position;
    }

    pub fn resolve_theme(&self, system_mode: Option<ThemeMode>) -> ResolvedTheme {
        resolve_theme_with_custom(
            self.state.settings.theme_mode,
            self.state.settings.theme_preset,
            system_mode,
            &self.state.settings.custom_themes,
            self.state.settings.active_custom_theme_id.as_deref(),
        )
    }

    pub fn import_theme_definition(&mut self, serialized: &str) -> Result<(), String> {
        let imported = import_theme_definition(serialized)?;
        match self
            .state
            .settings
            .custom_themes
            .iter_mut()
            .find(|theme| theme.id == imported.id)
        {
            Some(existing) => *existing = imported.clone(),
            None => self.state.settings.custom_themes.push(imported.clone()),
        }
        self.state.settings.active_custom_theme_id = Some(imported.id);
        Ok(())
    }

    pub fn export_theme_definition(&self, theme_id: &str) -> Result<Option<String>, String> {
        let Some(theme) = self
            .state
            .settings
            .custom_themes
            .iter()
            .find(|theme| theme.id == theme_id)
        else {
            return Ok(None);
        };
        export_theme_definition(theme).map(Some)
    }

    pub fn activate_custom_theme(&mut self, theme_id: Option<&str>) -> Result<(), String> {
        match theme_id {
            Some(id) => {
                let exists = self
                    .state
                    .settings
                    .custom_themes
                    .iter()
                    .any(|theme| theme.id == id);
                if !exists {
                    return Err(format!("unknown custom theme id: {id}"));
                }
                self.state.settings.active_custom_theme_id = Some(id.to_string());
            }
            None => self.state.settings.active_custom_theme_id = None,
        }
        Ok(())
    }

    pub fn sidebar_tree(&self) -> Result<SidebarTree, String> {
        let entries = self.sidebar_entries()?;
        Ok(SidebarTree {
            position: self.sidebar_position(),
            nodes: entries.into_iter().map(|entry| entry.node).collect(),
        })
    }

    pub fn toggle_sidebar_position(&mut self) {
        self.state.settings.sidebar_position = match self.state.settings.sidebar_position {
            SidebarPosition::Left => SidebarPosition::Right,
            SidebarPosition::Right => SidebarPosition::Left,
        };
    }

    pub fn select_next_sidebar_target(&mut self) -> Result<(), String> {
        let entries = self.sidebar_entries()?;
        let current_idx = entries
            .iter()
            .position(|entry| self.sidebar_selection_id.as_deref() == Some(entry.node.id.as_str()))
            .or_else(|| {
                let current_target = self.current_sidebar_target().ok()?;
                entries
                    .iter()
                    .position(|entry| entry.target == current_target)
            })
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % entries.len();
        let next_entry = &entries[next_idx];
        self.activate_sidebar_target(next_entry.target)?;
        self.sidebar_selection_id = Some(next_entry.node.id.clone());
        Ok(())
    }

    pub fn select_previous_sidebar_target(&mut self) -> Result<(), String> {
        let entries = self.sidebar_entries()?;
        let current_idx = entries
            .iter()
            .position(|entry| self.sidebar_selection_id.as_deref() == Some(entry.node.id.as_str()))
            .or_else(|| {
                let current_target = self.current_sidebar_target().ok()?;
                entries
                    .iter()
                    .position(|entry| entry.target == current_target)
            })
            .unwrap_or(0);
        let prev_idx = previous_index(current_idx, entries.len());
        let prev_entry = &entries[prev_idx];
        self.activate_sidebar_target(prev_entry.target)?;
        self.sidebar_selection_id = Some(prev_entry.node.id.clone());
        Ok(())
    }

    pub fn select_sidebar_node_by_id(&mut self, node_id: &str) -> Result<(), String> {
        let entries = self.sidebar_entries()?;
        let target = entries
            .iter()
            .find(|entry| entry.node.id == node_id)
            .map(|entry| entry.target)
            .ok_or_else(|| format!("sidebar node missing: {node_id}"))?;
        self.activate_sidebar_target(target)?;
        self.sidebar_selection_id = Some(node_id.to_string());
        Ok(())
    }

    pub fn sidebar_fuzzy_matches(&self, query: &str) -> Result<Vec<SidebarNode>, String> {
        let normalized = query.trim().to_ascii_lowercase();
        let entries = self.sidebar_entries()?;
        if normalized.is_empty() {
            return Ok(entries.into_iter().map(|entry| entry.node).collect());
        }

        Ok(entries
            .into_iter()
            .filter(|entry| {
                entry.node.title.to_ascii_lowercase().contains(&normalized)
                    || entry.node.id.to_ascii_lowercase().contains(&normalized)
            })
            .map(|entry| entry.node)
            .collect())
    }

    pub fn sidebar_fuzzy_jump(&mut self, query: &str) -> Result<Option<SidebarNode>, String> {
        let normalized = query.trim().to_ascii_lowercase();
        let entries = self.sidebar_entries()?;
        let Some(entry) = entries.into_iter().find(|entry| {
            normalized.is_empty()
                || entry.node.title.to_ascii_lowercase().contains(&normalized)
                || entry.node.id.to_ascii_lowercase().contains(&normalized)
        }) else {
            return Ok(None);
        };
        self.activate_sidebar_target(entry.target)?;
        self.sidebar_selection_id = Some(entry.node.id.clone());
        Ok(Some(entry.node))
    }

    pub fn palette_search(&self, query: &str) -> Result<Vec<PaletteItem>, String> {
        let normalized = query.trim().to_ascii_lowercase();
        let candidates = self.palette_candidates()?;
        let recency_rank = self
            .state
            .palette_recent
            .iter()
            .enumerate()
            .map(|(idx, item_id)| (item_id.as_str(), idx))
            .collect::<BTreeMap<_, _>>();

        let mut ranked = Vec::new();
        for candidate in candidates {
            let Some(base_score) = (if normalized.is_empty() {
                Some(0)
            } else {
                palette_match_score(&candidate.query_fields, &normalized)
            }) else {
                continue;
            };

            let recency_bonus = recency_rank
                .get(candidate.item.id.as_str())
                .and_then(|idx| {
                    if *idx < PALETTE_RECENT_LIMIT {
                        Some((PALETTE_RECENT_LIMIT - *idx) as i32)
                    } else {
                        None
                    }
                })
                .unwrap_or(0);

            ranked.push((candidate.item, base_score + recency_bonus));
        }

        ranked.sort_by(|(left_item, left_score), (right_item, right_score)| {
            right_score
                .cmp(left_score)
                .then_with(|| {
                    left_item
                        .title
                        .to_ascii_lowercase()
                        .cmp(&right_item.title.to_ascii_lowercase())
                })
                .then_with(|| left_item.id.cmp(&right_item.id))
        });

        Ok(ranked.into_iter().map(|(item, _)| item).collect())
    }

    pub fn palette_recent_items(&self) -> Result<Vec<PaletteItem>, String> {
        let candidates = self.palette_candidates()?;
        let by_id = candidates
            .into_iter()
            .map(|candidate| (candidate.item.id.clone(), candidate.item))
            .collect::<BTreeMap<_, _>>();

        Ok(self
            .state
            .palette_recent
            .iter()
            .filter_map(|item_id| by_id.get(item_id))
            .cloned()
            .collect())
    }

    pub fn palette_execute(&mut self, palette_item_id: &str) -> Result<(), String> {
        if let Some(command_id) = palette_item_id.strip_prefix("cmd:") {
            self.route_command_id(command_id)?;
            self.record_palette_recent(palette_item_id);
            return Ok(());
        }

        if let Some(node_id) = palette_item_id.strip_prefix("node:") {
            self.select_sidebar_node_by_id(node_id)?;
            self.record_palette_recent(palette_item_id);
            return Ok(());
        }

        Err(format!("unknown palette item id: {palette_item_id}"))
    }

    pub fn notification_history(&self) -> Vec<NotificationEvent> {
        self.notification_bus.history()
    }

    pub fn mark_block_approval_required(
        &self,
        block_id: &str,
        reason: impl Into<String>,
    ) -> Result<(), String> {
        let block = self
            .block_by_id(block_id)
            .ok_or_else(|| format!("block missing: {block_id}"))?;
        let summary = notification_input_summary(&block.input);
        self.notification_bus.publish_approval_required(
            format!("Approval required for {summary}"),
            reason.into(),
            Some(block.id.clone()),
        );
        Ok(())
    }

    pub fn resolve_block_navigation_target(
        &self,
        block_id: &str,
    ) -> Result<BlockNavigationTarget, String> {
        let block = self
            .block_by_id(block_id)
            .ok_or_else(|| format!("block missing: {block_id}"))?;

        for window in &self.state.windows {
            for workspace in &window.workspaces {
                for tab in &workspace.tabs {
                    for pane in &tab.panes {
                        if pane
                            .surfaces
                            .iter()
                            .any(|surface| surface.session_id == block.session_id)
                        {
                            return Ok(BlockNavigationTarget {
                                block_id: block.id.clone(),
                                session_id: block.session_id.clone(),
                                window_id: window.id.clone(),
                                workspace_id: workspace.id.clone(),
                                tab_id: tab.id.clone(),
                                pane_id: pane.id.clone(),
                            });
                        }
                    }
                }
            }
        }

        Err(format!(
            "navigation target missing for block session: {}",
            block.session_id
        ))
    }

    pub fn resolve_notification_target(
        &self,
        event: &NotificationEvent,
    ) -> Result<Option<BlockNavigationTarget>, String> {
        let Some(block_id) = event.block_id.as_deref() else {
            return Ok(None);
        };
        self.resolve_block_navigation_target(block_id).map(Some)
    }

    pub fn blocks(&self) -> &[Block] {
        &self.state.blocks
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn block_by_id(&self, block_id: &str) -> Option<&Block> {
        let position = self.block_index.position_for_block_id(block_id)?;
        self.state.blocks.get(position)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn blocks_for_session(&self, session_id: &str) -> Vec<&Block> {
        let Some(block_ids) = self.block_index.by_session.get(session_id) else {
            return Vec::new();
        };

        block_ids
            .iter()
            .filter_map(|block_id| self.block_by_id(block_id))
            .collect()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn replay_block_output(&self, block_id: &str) -> Result<String, String> {
        let block = self
            .block_by_id(block_id)
            .ok_or_else(|| format!("block missing: {block_id}"))?;
        Ok(block
            .output_chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect())
    }

    pub fn start_command_block_for_active_session(
        &mut self,
        input: impl Into<String>,
    ) -> Result<String, String> {
        let session_id = self
            .active_surface_session_id()
            .ok_or_else(|| "active surface missing".to_string())?;
        self.start_command_block_for_session(session_id, input)
    }

    pub fn start_command_block_for_session(
        &mut self,
        session_id: impl Into<String>,
        input: impl Into<String>,
    ) -> Result<String, String> {
        let session_id = session_id.into();
        if !self.session_exists(&session_id) {
            return Err(format!("session missing: {session_id}"));
        }

        let block = Block {
            id: self.next_id("block"),
            session_id,
            input: input.into(),
            output_chunks: Vec::new(),
            status: BlockStatus::Running,
            started_at_ms: now_ms(),
            finished_at_ms: None,
        };

        let position = self.state.blocks.len();
        let block_id = block.id.clone();
        self.block_index.record(&block, position)?;
        self.state.blocks.push(block);
        Ok(block_id)
    }

    pub fn append_block_output(
        &mut self,
        block_id: &str,
        text: impl Into<String>,
    ) -> Result<u64, String> {
        let block = self.block_by_id_mut(block_id)?;
        if block.status != BlockStatus::Running {
            return Err(format!(
                "cannot append output to block in status {:?}",
                block.status
            ));
        }

        let chunk_id = block
            .output_chunks
            .last()
            .map(|chunk| chunk.chunk_id + 1)
            .unwrap_or(1);
        block.output_chunks.push(BlockOutputChunk {
            chunk_id,
            text: text.into(),
        });
        Ok(chunk_id)
    }

    pub fn finish_block(&mut self, block_id: &str, status: BlockStatus) -> Result<(), String> {
        if status == BlockStatus::Running {
            return Err("finish_block requires a terminal status".to_string());
        }

        let (block_input, final_status) = {
            let block = self.block_by_id_mut(block_id)?;
            if block.status != BlockStatus::Running {
                return Err(format!(
                    "block already finalized with status {:?}",
                    block.status
                ));
            }

            block.status = status;
            block.finished_at_ms = Some(now_ms());
            (block.input.clone(), block.status.clone())
        };
        self.publish_block_status_notification(block_id, &block_input, &final_status);
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn rerun_block(&mut self, block_id: &str) -> Result<String, String> {
        let (session_id, input) = {
            let block = self
                .block_by_id(block_id)
                .ok_or_else(|| format!("block missing: {block_id}"))?;
            (block.session_id.clone(), block.input.clone())
        };
        self.start_command_block_for_session(session_id, input)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn rerun_block_with_edit(
        &mut self,
        block_id: &str,
        updated_input: impl Into<String>,
    ) -> Result<String, String> {
        let session_id = {
            let block = self
                .block_by_id(block_id)
                .ok_or_else(|| format!("block missing: {block_id}"))?;
            block.session_id.clone()
        };
        self.start_command_block_for_session(session_id, updated_input)
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
            command_ids::SIDEBAR_TOGGLE_POSITION => {
                self.route_command(AppShellCommand::ToggleSidebarPosition)
            }
            command_ids::SIDEBAR_NEXT => {
                self.route_command(AppShellCommand::SelectNextSidebarTarget)
            }
            command_ids::SIDEBAR_PREV => {
                self.route_command(AppShellCommand::SelectPreviousSidebarTarget)
            }
            command_ids::CURSOR_STYLE_BAR => self.route_command(AppShellCommand::SetCursorStyle {
                style: CursorStyle::Bar,
            }),
            command_ids::CURSOR_STYLE_BLOCK => {
                self.route_command(AppShellCommand::SetCursorStyle {
                    style: CursorStyle::Block,
                })
            }
            command_ids::CURSOR_STYLE_UNDERLINE => {
                self.route_command(AppShellCommand::SetCursorStyle {
                    style: CursorStyle::Underline,
                })
            }
            command_ids::INPUT_POSITION_TOP => {
                self.route_command(AppShellCommand::SetInputPosition {
                    position: InputPosition::TopClassic,
                })
            }
            command_ids::INPUT_POSITION_TOP_REVERSE => {
                self.route_command(AppShellCommand::SetInputPosition {
                    position: InputPosition::TopReverse,
                })
            }
            command_ids::INPUT_POSITION_BOTTOM => {
                self.route_command(AppShellCommand::SetInputPosition {
                    position: InputPosition::Bottom,
                })
            }
            _ => Err(format!("unknown command id: {command_id}")),
        }
    }

    pub fn route_command(&mut self, command: AppShellCommand) -> Result<(), String> {
        let keep_sidebar_selection = matches!(
            command,
            AppShellCommand::ToggleSidebarPosition
                | AppShellCommand::SelectNextSidebarTarget
                | AppShellCommand::SelectPreviousSidebarTarget
        );
        if !keep_sidebar_selection {
            self.sidebar_selection_id = None;
        }

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
            AppShellCommand::ToggleSidebarPosition => {
                self.toggle_sidebar_position();
                Ok(())
            }
            AppShellCommand::SelectNextSidebarTarget => self.select_next_sidebar_target(),
            AppShellCommand::SelectPreviousSidebarTarget => self.select_previous_sidebar_target(),
            AppShellCommand::SetCursorStyle { style } => {
                self.set_cursor_style(style);
                Ok(())
            }
            AppShellCommand::SetInputPosition { position } => {
                self.set_input_position(position);
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
            "AppShell version={}, windows={}, active_window={}, active_window_workspaces={}, theme_mode={:?}, theme_preset={:?}, cursor_style={:?}, input_position={:?}",
            self.state.version,
            self.state.windows.len(),
            self.state.active_window,
            workspace_count,
            self.state.settings.theme_mode,
            self.state.settings.theme_preset,
            self.state.settings.cursor_style,
            self.state.settings.input_position
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
        self.commands.register(CommandAction {
            id: command_ids::SIDEBAR_TOGGLE_POSITION.to_string(),
            title: "Toggle Sidebar Position".to_string(),
            description: "Toggle sidebar position between left and right".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::SIDEBAR_NEXT.to_string(),
            title: "Next Sidebar Target".to_string(),
            description: "Select next workspace/tab/pane target in sidebar order".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::SIDEBAR_PREV.to_string(),
            title: "Previous Sidebar Target".to_string(),
            description: "Select previous workspace/tab/pane target in sidebar order".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::CURSOR_STYLE_BAR.to_string(),
            title: "Set Cursor Style: Bar".to_string(),
            description: "Set terminal cursor style to bar".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::CURSOR_STYLE_BLOCK.to_string(),
            title: "Set Cursor Style: Block".to_string(),
            description: "Set terminal cursor style to block".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::CURSOR_STYLE_UNDERLINE.to_string(),
            title: "Set Cursor Style: Underline".to_string(),
            description: "Set terminal cursor style to underline".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::INPUT_POSITION_TOP.to_string(),
            title: "Set Input Position: Top".to_string(),
            description: "Set terminal input position to top classic".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::INPUT_POSITION_TOP_REVERSE.to_string(),
            title: "Set Input Position: Top Reverse".to_string(),
            description: "Set terminal input position to top reverse".to_string(),
        });
        self.commands.register(CommandAction {
            id: command_ids::INPUT_POSITION_BOTTOM.to_string(),
            title: "Set Input Position: Bottom".to_string(),
            description: "Set terminal input position to bottom".to_string(),
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
            blocks: Vec::new(),
            palette_recent: Vec::new(),
            next_id: 6,
            last_started_at_ms: now_ms(),
        }
    }

    fn load_from_path(path: &Path) -> io::Result<AppShellState> {
        let bytes = fs::read(path)?;
        let state = serde_json::from_slice::<AppShellState>(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        Self::migrate_loaded_state(state)
    }

    fn migrate_loaded_state(mut state: AppShellState) -> io::Result<AppShellState> {
        match state.version {
            APP_STATE_VERSION => Ok(state),
            LEGACY_APP_STATE_VERSION => {
                state.version = APP_STATE_VERSION;
                Ok(state)
            }
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported app shell state version: {other}"),
            )),
        }
    }

    fn sidebar_entries(&self) -> Result<Vec<SidebarEntry>, String> {
        let window = self.active_window()?;
        let mut entries = Vec::new();

        for (workspace_idx, workspace) in window.workspaces.iter().enumerate() {
            let workspace_active = workspace_idx == window.active_workspace;
            entries.push(SidebarEntry {
                node: SidebarNode {
                    id: workspace.id.clone(),
                    kind: SidebarNodeKind::Workspace,
                    title: workspace.name.clone(),
                    depth: 0,
                    is_active: workspace_active,
                },
                target: SidebarTarget::Workspace { workspace_idx },
            });

            for (tab_idx, tab) in workspace.tabs.iter().enumerate() {
                let tab_active = workspace_active && tab_idx == workspace.active_tab;
                entries.push(SidebarEntry {
                    node: SidebarNode {
                        id: tab.id.clone(),
                        kind: SidebarNodeKind::Tab,
                        title: tab.title.clone(),
                        depth: 1,
                        is_active: tab_active,
                    },
                    target: SidebarTarget::Tab {
                        workspace_idx,
                        tab_idx,
                    },
                });

                for (pane_idx, pane) in tab.panes.iter().enumerate() {
                    let pane_active = tab_active && pane_idx == tab.active_pane;
                    entries.push(SidebarEntry {
                        node: SidebarNode {
                            id: pane.id.clone(),
                            kind: SidebarNodeKind::Pane,
                            title: format!("Pane {}", pane_idx + 1),
                            depth: 2,
                            is_active: pane_active,
                        },
                        target: SidebarTarget::Pane {
                            workspace_idx,
                            tab_idx,
                            pane_idx,
                        },
                    });
                }
            }
        }

        if entries.is_empty() {
            return Err("active window has no sidebar entries".to_string());
        }
        Ok(entries)
    }

    fn palette_candidates(&self) -> Result<Vec<PaletteCandidate>, String> {
        let mut candidates = Vec::new();
        for action in self.commands.search("") {
            let mut query_fields = vec![
                action.id.to_ascii_lowercase(),
                action.title.to_ascii_lowercase(),
                action.description.to_ascii_lowercase(),
            ];
            query_fields.push("command".to_string());
            candidates.push(PaletteCandidate {
                item: PaletteItem {
                    id: format!("cmd:{}", action.id),
                    kind: PaletteItemKind::Command,
                    title: action.title,
                    subtitle: action.description,
                },
                query_fields,
            });
        }

        let window = self.active_window()?;
        for entry in self.sidebar_entries()? {
            let (kind, subtitle, mut query_fields) = match entry.target {
                SidebarTarget::Workspace { workspace_idx } => {
                    let workspace_name = window
                        .workspaces
                        .get(workspace_idx)
                        .map(|workspace| workspace.name.clone())
                        .unwrap_or_else(|| entry.node.title.clone());
                    (
                        PaletteItemKind::Workspace,
                        "Workspace".to_string(),
                        vec!["workspace".to_string(), workspace_name.to_ascii_lowercase()],
                    )
                }
                SidebarTarget::Tab {
                    workspace_idx,
                    tab_idx,
                } => {
                    let workspace_name = window
                        .workspaces
                        .get(workspace_idx)
                        .map(|workspace| workspace.name.clone())
                        .unwrap_or_else(|| "workspace".to_string());
                    let tab_title = window
                        .workspaces
                        .get(workspace_idx)
                        .and_then(|workspace| workspace.tabs.get(tab_idx))
                        .map(|tab| tab.title.clone())
                        .unwrap_or_else(|| entry.node.title.clone());
                    (
                        PaletteItemKind::Tab,
                        format!("Tab in {workspace_name}"),
                        vec![
                            "tab".to_string(),
                            workspace_name.to_ascii_lowercase(),
                            tab_title.to_ascii_lowercase(),
                        ],
                    )
                }
                SidebarTarget::Pane {
                    workspace_idx,
                    tab_idx,
                    pane_idx,
                } => {
                    let workspace_name = window
                        .workspaces
                        .get(workspace_idx)
                        .map(|workspace| workspace.name.clone())
                        .unwrap_or_else(|| "workspace".to_string());
                    let tab_title = window
                        .workspaces
                        .get(workspace_idx)
                        .and_then(|workspace| workspace.tabs.get(tab_idx))
                        .map(|tab| tab.title.clone())
                        .unwrap_or_else(|| "tab".to_string());
                    (
                        PaletteItemKind::Pane,
                        format!("Pane {} in {workspace_name} / {tab_title}", pane_idx + 1),
                        vec![
                            "pane".to_string(),
                            workspace_name.to_ascii_lowercase(),
                            tab_title.to_ascii_lowercase(),
                            format!("pane {}", pane_idx + 1),
                        ],
                    )
                }
            };

            query_fields.push(entry.node.id.to_ascii_lowercase());
            query_fields.push(entry.node.title.to_ascii_lowercase());
            query_fields.push(subtitle.to_ascii_lowercase());

            candidates.push(PaletteCandidate {
                item: PaletteItem {
                    id: format!("node:{}", entry.node.id),
                    kind,
                    title: entry.node.title,
                    subtitle,
                },
                query_fields,
            });
        }

        Ok(candidates)
    }

    fn current_sidebar_target(&self) -> Result<SidebarTarget, String> {
        let window = self.active_window()?;
        let workspace = window
            .workspaces
            .get(window.active_workspace)
            .ok_or_else(|| "active workspace missing".to_string())?;

        if workspace.tabs.is_empty() {
            return Ok(SidebarTarget::Workspace {
                workspace_idx: window.active_workspace,
            });
        }
        let tab_idx = workspace.active_tab.min(workspace.tabs.len() - 1);
        let tab = workspace
            .tabs
            .get(tab_idx)
            .ok_or_else(|| "active tab missing".to_string())?;

        if tab.panes.is_empty() {
            return Ok(SidebarTarget::Tab {
                workspace_idx: window.active_workspace,
                tab_idx,
            });
        }
        let pane_idx = tab.active_pane.min(tab.panes.len() - 1);
        Ok(SidebarTarget::Pane {
            workspace_idx: window.active_workspace,
            tab_idx,
            pane_idx,
        })
    }

    fn activate_sidebar_target(&mut self, target: SidebarTarget) -> Result<(), String> {
        let window = self.active_window_mut()?;
        match target {
            SidebarTarget::Workspace { workspace_idx } => {
                if workspace_idx >= window.workspaces.len() {
                    return Err(format!("workspace index out of bounds: {workspace_idx}"));
                }
                window.active_workspace = workspace_idx;
                let workspace = &mut window.workspaces[workspace_idx];
                if workspace.tabs.is_empty() {
                    workspace.active_tab = 0;
                    return Ok(());
                }
                if workspace.active_tab >= workspace.tabs.len() {
                    workspace.active_tab = 0;
                }
                let tab = &mut workspace.tabs[workspace.active_tab];
                if tab.panes.is_empty() || tab.active_pane >= tab.panes.len() {
                    tab.active_pane = 0;
                }
                Ok(())
            }
            SidebarTarget::Tab {
                workspace_idx,
                tab_idx,
            } => {
                if workspace_idx >= window.workspaces.len() {
                    return Err(format!("workspace index out of bounds: {workspace_idx}"));
                }
                window.active_workspace = workspace_idx;
                let workspace = &mut window.workspaces[workspace_idx];
                if tab_idx >= workspace.tabs.len() {
                    return Err(format!("tab index out of bounds: {tab_idx}"));
                }
                workspace.active_tab = tab_idx;
                let tab = &mut workspace.tabs[tab_idx];
                if tab.panes.is_empty() || tab.active_pane >= tab.panes.len() {
                    tab.active_pane = 0;
                }
                Ok(())
            }
            SidebarTarget::Pane {
                workspace_idx,
                tab_idx,
                pane_idx,
            } => {
                if workspace_idx >= window.workspaces.len() {
                    return Err(format!("workspace index out of bounds: {workspace_idx}"));
                }
                window.active_workspace = workspace_idx;
                let workspace = &mut window.workspaces[workspace_idx];
                if tab_idx >= workspace.tabs.len() {
                    return Err(format!("tab index out of bounds: {tab_idx}"));
                }
                workspace.active_tab = tab_idx;
                let tab = &mut workspace.tabs[tab_idx];
                if pane_idx >= tab.panes.len() {
                    return Err(format!("pane index out of bounds: {pane_idx}"));
                }
                tab.active_pane = pane_idx;
                Ok(())
            }
        }
    }

    fn active_window(&self) -> Result<&WindowState, String> {
        self.state
            .windows
            .get(self.state.active_window)
            .ok_or_else(|| "active window missing".to_string())
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

    fn active_surface_session_id(&self) -> Option<String> {
        let window = self.state.windows.get(self.state.active_window)?;
        let workspace = window.workspaces.get(window.active_workspace)?;
        let tab = workspace.tabs.get(workspace.active_tab)?;
        let pane = tab.panes.get(tab.active_pane)?;
        let surface = pane.surfaces.get(pane.active_surface)?;
        Some(surface.session_id.clone())
    }

    fn record_palette_recent(&mut self, palette_item_id: &str) {
        self.state
            .palette_recent
            .retain(|existing| existing != palette_item_id);
        self.state
            .palette_recent
            .insert(0, palette_item_id.to_string());
        self.state.palette_recent.truncate(PALETTE_RECENT_LIMIT);
    }

    fn publish_block_status_notification(
        &self,
        block_id: &str,
        block_input: &str,
        status: &BlockStatus,
    ) {
        let summary = notification_input_summary(block_input);
        match status {
            BlockStatus::Succeeded => {
                self.notification_bus.publish_task_done(
                    "Block completed",
                    format!("Completed: {summary}"),
                    Some(block_id.to_string()),
                );
            }
            BlockStatus::Failed => {
                self.notification_bus.publish_task_failed(
                    "Block failed",
                    format!("Failed: {summary}"),
                    Some(block_id.to_string()),
                );
            }
            BlockStatus::Cancelled => {
                self.notification_bus.publish_task_failed(
                    "Block cancelled",
                    format!("Cancelled: {summary}"),
                    Some(block_id.to_string()),
                );
            }
            BlockStatus::Running => {}
        }
    }

    fn session_exists(&self, session_id: &str) -> bool {
        self.state.windows.iter().any(|window| {
            window.workspaces.iter().any(|workspace| {
                workspace.tabs.iter().any(|tab| {
                    tab.panes.iter().any(|pane| {
                        pane.surfaces
                            .iter()
                            .any(|surface| surface.session_id == session_id)
                    })
                })
            })
        })
    }

    fn block_by_id_mut(&mut self, block_id: &str) -> Result<&mut Block, String> {
        let position = self
            .block_index
            .position_for_block_id(block_id)
            .ok_or_else(|| format!("block missing: {block_id}"))?;
        self.state
            .blocks
            .get_mut(position)
            .ok_or_else(|| format!("block index out of bounds: {block_id}"))
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

fn palette_match_score(query_fields: &[String], query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let mut best: Option<i32> = None;
    for field in query_fields {
        let score = if field == query {
            300
        } else if field.starts_with(query) {
            200
        } else if field.contains(query) {
            100
        } else {
            continue;
        };
        best = Some(best.map_or(score, |existing| existing.max(score)));
    }

    best
}

fn notification_input_summary(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "(empty command)".to_string();
    }

    const LIMIT: usize = 80;
    let chars = trimmed.chars().collect::<Vec<_>>();
    if chars.len() <= LIMIT {
        return trimmed.to_string();
    }

    chars.into_iter().take(LIMIT).collect::<String>() + "..."
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
    use ulgen_domain::{BlockStatus, NotificationEvent, NotificationEventKind, Pane, Surface, Tab};
    use ulgen_settings::{
        CursorStyle, InputPosition, KeymapOverride, KeymapProfile, SidebarPosition,
    };

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
    fn sidebar_tree_exposes_hierarchy_and_position_toggle() {
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
        shell.route_command(AppShellCommand::CreateTab).unwrap();
        shell
            .route_command(AppShellCommand::SplitPaneRight)
            .unwrap();

        let tree = shell.sidebar_tree().unwrap();
        assert_eq!(tree.position, SidebarPosition::Left);
        assert!(tree.nodes.iter().any(|node| node.depth == 0));
        assert!(tree.nodes.iter().any(|node| node.depth == 1));
        assert!(tree.nodes.iter().any(|node| node.depth == 2));
        assert!(tree.nodes.iter().any(|node| node.is_active));

        shell
            .route_command_id(command_ids::SIDEBAR_TOGGLE_POSITION)
            .unwrap();
        assert_eq!(shell.sidebar_position(), SidebarPosition::Right);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn sidebar_position_persists_across_save_and_restore() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        assert_eq!(shell.sidebar_position(), SidebarPosition::Left);
        shell
            .route_command_id(command_ids::SIDEBAR_TOGGLE_POSITION)
            .unwrap();
        assert_eq!(shell.sidebar_position(), SidebarPosition::Right);
        shell.save().unwrap();

        let restored = AppShell::bootstrap(path.clone()).unwrap();
        assert_eq!(restored.sidebar_position(), SidebarPosition::Right);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn theme_resolution_updates_immediately_when_settings_change() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let initial = shell.resolve_theme(Some(ThemeMode::Light));

        shell.set_theme_mode(ThemeMode::Dark);
        shell.set_theme_preset(ThemePreset::Ember);
        let updated = shell.resolve_theme(Some(ThemeMode::Light));

        assert_eq!(updated.mode, ThemeMode::Dark);
        assert_eq!(updated.preset, ThemePreset::Ember);
        assert_ne!(initial.tokens.accent, updated.tokens.accent);
        assert_ne!(initial.tokens.terminal_bg, updated.tokens.terminal_bg);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn theme_settings_persist_across_save_and_restore() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell.set_theme_mode(ThemeMode::Light);
        shell.set_theme_preset(ThemePreset::Grove);
        shell.save().unwrap();

        let restored = AppShell::bootstrap(path.clone()).unwrap();
        assert_eq!(restored.theme_mode(), ThemeMode::Light);
        assert_eq!(restored.theme_preset(), ThemePreset::Grove);
        let resolved = restored.resolve_theme(Some(ThemeMode::Dark));
        assert_eq!(resolved.mode, ThemeMode::Light);
        assert_eq!(resolved.preset, ThemePreset::Grove);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn pointer_and_input_settings_persist_across_save_and_restore() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell.set_cursor_style(CursorStyle::Underline);
        shell.set_input_position(InputPosition::TopReverse);
        shell.save().unwrap();

        let restored = AppShell::bootstrap(path.clone()).unwrap();
        assert_eq!(restored.cursor_style(), CursorStyle::Underline);
        assert_eq!(restored.input_position(), InputPosition::TopReverse);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn command_ids_support_pointer_and_input_position() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell
            .route_command_id(command_ids::CURSOR_STYLE_BAR)
            .unwrap();
        assert_eq!(shell.cursor_style(), CursorStyle::Bar);

        shell
            .route_command_id(command_ids::CURSOR_STYLE_UNDERLINE)
            .unwrap();
        assert_eq!(shell.cursor_style(), CursorStyle::Underline);

        shell
            .route_command_id(command_ids::INPUT_POSITION_TOP)
            .unwrap();
        assert_eq!(shell.input_position(), InputPosition::TopClassic);

        shell
            .route_command_id(command_ids::INPUT_POSITION_TOP_REVERSE)
            .unwrap();
        assert_eq!(shell.input_position(), InputPosition::TopReverse);

        shell
            .route_command_id(command_ids::INPUT_POSITION_BOTTOM)
            .unwrap();
        assert_eq!(shell.input_position(), InputPosition::Bottom);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn imported_theme_pipeline_activates_exports_and_persists() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let custom_theme = r##"{
            "id":"aurora",
            "name":"Aurora",
            "light":{
                "surface_bg":"#f5f7ff",
                "surface_fg":"#1b2240",
                "surface_muted":"#6c7391",
                "border":"#c7d1ff",
                "accent":"#3f67ff",
                "success":"#1f8a5b",
                "warning":"#b77500",
                "danger":"#c43a45",
                "terminal_bg":"#f9fbff",
                "terminal_fg":"#1b2240",
                "terminal_cursor":"#3f67ff"
            },
            "dark":{
                "surface_bg":"#0f1224",
                "surface_fg":"#dce2ff",
                "surface_muted":"#8a93ba",
                "border":"#2a3463",
                "accent":"#7e96ff",
                "success":"#52cb92",
                "warning":"#f2bf63",
                "danger":"#ef7b84",
                "terminal_bg":"#090b17",
                "terminal_fg":"#dce2ff",
                "terminal_cursor":"#7e96ff"
            }
        }"##;

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell.import_theme_definition(custom_theme).unwrap();
        let themed = shell.resolve_theme(Some(ThemeMode::Light));
        assert_eq!(themed.custom_theme_id, Some("aurora".to_string()));
        assert_eq!(themed.custom_theme_name, Some("Aurora".to_string()));
        assert_eq!(themed.tokens.accent, "#3f67ff");

        let exported = shell
            .export_theme_definition("aurora")
            .unwrap()
            .expect("theme should be exportable");
        assert!(exported.contains("\"id\": \"aurora\""));
        assert!(exported.contains("\"name\": \"Aurora\""));

        shell.activate_custom_theme(None).unwrap();
        let fallback = shell.resolve_theme(Some(ThemeMode::Light));
        assert_eq!(fallback.custom_theme_id, None);

        shell.activate_custom_theme(Some("aurora")).unwrap();
        shell.save().unwrap();

        let restored = AppShell::bootstrap(path.clone()).unwrap();
        let resolved = restored.resolve_theme(Some(ThemeMode::Dark));
        assert_eq!(resolved.custom_theme_id, Some("aurora".to_string()));
        assert_eq!(resolved.custom_theme_name, Some("Aurora".to_string()));
        assert_eq!(resolved.tokens.accent, "#7e96ff");

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn sidebar_next_previous_traversal_wraps_and_updates_active_context() {
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
        shell.route_command(AppShellCommand::CreateTab).unwrap();
        shell
            .route_command(AppShellCommand::SplitPaneRight)
            .unwrap();

        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            1
        );
        assert_eq!(
            shell.state.windows[shell.state.active_window].workspaces[1].active_tab,
            1
        );
        assert_eq!(
            shell.state.windows[shell.state.active_window].workspaces[1].tabs[1].active_pane,
            1
        );

        shell.route_command_id(command_ids::SIDEBAR_NEXT).unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            0
        );

        shell.route_command_id(command_ids::SIDEBAR_PREV).unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            1
        );
        assert_eq!(
            shell.state.windows[shell.state.active_window].workspaces[1].active_tab,
            1
        );
        assert_eq!(
            shell.state.windows[shell.state.active_window].workspaces[1].tabs[1].active_pane,
            1
        );

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn sidebar_select_by_id_and_fuzzy_jump_activate_targets() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell
            .route_command(AppShellCommand::CreateWorkspace {
                name: "api".to_string(),
            })
            .unwrap();

        let tree = shell.sidebar_tree().unwrap();
        let api_workspace_id = tree
            .nodes
            .iter()
            .find(|node| node.kind == SidebarNodeKind::Workspace && node.title == "api")
            .unwrap()
            .id
            .clone();

        shell.select_sidebar_node_by_id(&api_workspace_id).unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            1
        );

        let fuzzy_results = shell.sidebar_fuzzy_matches("pane").unwrap();
        assert!(!fuzzy_results.is_empty());
        assert!(fuzzy_results
            .iter()
            .all(|node| node.kind == SidebarNodeKind::Pane));

        let jumped = shell.sidebar_fuzzy_jump("default").unwrap();
        assert!(jumped.is_some());
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            0
        );

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn sidebar_select_by_id_reports_missing_node() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let error = shell.select_sidebar_node_by_id("missing-workspace");
        assert!(error.is_err());
        assert!(error
            .unwrap_err()
            .contains("sidebar node missing: missing-workspace"));

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn non_sidebar_navigation_clears_sidebar_selection_cache() {
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
        shell.route_command(AppShellCommand::CreateTab).unwrap();
        shell
            .route_command(AppShellCommand::SplitPaneRight)
            .unwrap();

        let workspace0_id = shell
            .sidebar_tree()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.kind == SidebarNodeKind::Workspace && node.title == "Default")
            .unwrap()
            .id
            .clone();
        shell.select_sidebar_node_by_id(&workspace0_id).unwrap();
        shell
            .route_command(AppShellCommand::SelectNextWorkspace)
            .unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            1
        );
        assert_eq!(
            shell.state.windows[shell.state.active_window].workspaces[1].active_tab,
            1
        );
        assert_eq!(
            shell.state.windows[shell.state.active_window].workspaces[1].tabs[1].active_pane,
            1
        );

        shell.route_command_id(command_ids::SIDEBAR_PREV).unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            1
        );
        assert_eq!(
            shell.state.windows[shell.state.active_window].workspaces[1].active_tab,
            1
        );
        assert_eq!(
            shell.state.windows[shell.state.active_window].workspaces[1].tabs[1].active_pane,
            0
        );

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn palette_search_includes_commands_and_sidebar_entities() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell
            .route_command(AppShellCommand::CreateWorkspace {
                name: "api".to_string(),
            })
            .unwrap();
        shell.route_command(AppShellCommand::CreateTab).unwrap();

        let command_matches = shell.palette_search("split pane").unwrap();
        assert!(command_matches.iter().any(|item| {
            item.id == format!("cmd:{}", command_ids::PANE_SPLIT_RIGHT)
                && item.kind == PaletteItemKind::Command
        }));

        let entity_matches = shell.palette_search("api").unwrap();
        assert!(entity_matches.iter().any(|item| {
            item.kind == PaletteItemKind::Workspace
                && item.title == "api"
                && item.id.starts_with("node:")
        }));

        assert!(shell
            .palette_search("query-that-will-not-match")
            .unwrap()
            .is_empty());

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn palette_execute_quick_switch_updates_context_and_recent_history() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell
            .route_command(AppShellCommand::CreateWorkspace {
                name: "api".to_string(),
            })
            .unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            1
        );

        let default_workspace_item_id = shell
            .palette_search("default")
            .unwrap()
            .into_iter()
            .find(|item| item.kind == PaletteItemKind::Workspace && item.title == "Default")
            .unwrap()
            .id;

        shell.palette_execute(&default_workspace_item_id).unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            0
        );

        shell
            .palette_execute(&format!("cmd:{}", command_ids::WORKSPACE_NEXT))
            .unwrap();
        assert_eq!(
            shell.state.windows[shell.state.active_window].active_workspace,
            1
        );

        let workspace_query = shell.palette_search("next").unwrap();
        assert_eq!(
            workspace_query.first().unwrap().id,
            format!("cmd:{}", command_ids::WORKSPACE_NEXT)
        );

        let recents = shell.palette_recent_items().unwrap();
        assert_eq!(
            recents.first().unwrap().id,
            format!("cmd:{}", command_ids::WORKSPACE_NEXT)
        );
        assert!(recents
            .iter()
            .any(|item| item.id == default_workspace_item_id));

        shell.save().unwrap();
        let restored = AppShell::bootstrap(path.clone()).unwrap();
        let restored_recents = restored.palette_recent_items().unwrap();
        assert_eq!(
            restored_recents.first().unwrap().id,
            format!("cmd:{}", command_ids::WORKSPACE_NEXT)
        );

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn palette_execute_rejects_unknown_item_id() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let error = shell.palette_execute("unknown:target");
        assert!(error.is_err());
        assert!(error.unwrap_err().contains("unknown palette item id"));

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn palette_execute_rejects_stale_typed_ids_without_recent_history_mutation() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        shell
            .route_command(AppShellCommand::CreateWorkspace {
                name: "api".to_string(),
            })
            .unwrap();
        shell
            .palette_execute(&format!("cmd:{}", command_ids::WORKSPACE_NEXT))
            .unwrap();
        let before = shell.palette_recent_items().unwrap();

        let stale_command_error = shell.palette_execute("cmd:missing.command");
        assert!(stale_command_error.is_err());
        assert!(stale_command_error
            .unwrap_err()
            .contains("unknown command id: missing.command"));

        let stale_node_error = shell.palette_execute("node:missing-node");
        assert!(stale_node_error.is_err());
        assert!(stale_node_error
            .unwrap_err()
            .contains("sidebar node missing: missing-node"));

        let after = shell.palette_recent_items().unwrap();
        assert_eq!(after, before);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn palette_match_score_prioritizes_exact_prefix_then_contains() {
        let exact = vec!["workspace".to_string()];
        assert_eq!(palette_match_score(&exact, "workspace"), Some(300));

        let prefix = vec!["workspace.next".to_string()];
        assert_eq!(palette_match_score(&prefix, "workspace"), Some(200));

        let contains = vec!["next workspace".to_string()];
        assert_eq!(palette_match_score(&contains, "workspace"), Some(100));

        let no_match = vec!["pane".to_string()];
        assert_eq!(palette_match_score(&no_match, "workspace"), None);
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
        assert_eq!(shell.state.version, APP_STATE_VERSION);
        assert_eq!(shell.state.settings.keymap_profile, KeymapProfile::Warp);
        assert!(shell.state.settings.keymap_overrides.is_empty());
        assert!(shell.state.blocks.is_empty());

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn rejects_unsupported_state_version() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let unsupported = r#"{
            "version":999,
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
            "blocks":[],
            "next_id":1,
            "last_started_at_ms":0
        }"#;

        fs::write(&path, unsupported).unwrap();
        let err = AppShell::bootstrap(path.clone())
            .err()
            .expect("unsupported version should fail bootstrap");
        assert!(err
            .to_string()
            .contains("unsupported app shell state version"));

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn restore_state_defaults_missing_theme_fields_in_settings() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let state = r#"{
            "version":2,
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
            "settings":{
                "theme_mode":"System"
            },
            "blocks":[],
            "palette_recent":[],
            "next_id":1,
            "last_started_at_ms":0
        }"#;

        fs::write(&path, state).unwrap();
        let shell = AppShell::bootstrap(path.clone()).unwrap();
        assert_eq!(shell.theme_mode(), ThemeMode::System);
        assert_eq!(shell.theme_preset(), ThemePreset::Horizon);
        assert!(shell.state.settings.custom_themes.is_empty());
        assert_eq!(shell.state.settings.active_custom_theme_id, None);
        assert_eq!(shell.cursor_style(), CursorStyle::Block);
        assert_eq!(shell.input_position(), InputPosition::Bottom);
        shell.save().unwrap();

        let restored = AppShell::bootstrap(path.clone()).unwrap();
        assert_eq!(restored.theme_preset(), ThemePreset::Horizon);

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

    #[test]
    fn block_lifecycle_supports_start_append_finish_and_replay() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let block_id = shell
            .start_command_block_for_active_session("cargo test --workspace")
            .unwrap();
        assert_eq!(shell.blocks().len(), 1);

        let first_chunk_id = shell
            .append_block_output(&block_id, "running tests...\n")
            .unwrap();
        let second_chunk_id = shell.append_block_output(&block_id, "ok\n").unwrap();
        assert_eq!(first_chunk_id, 1);
        assert_eq!(second_chunk_id, 2);

        shell
            .finish_block(&block_id, BlockStatus::Succeeded)
            .unwrap();

        let block = shell.block_by_id(&block_id).unwrap();
        assert_eq!(block.status, BlockStatus::Succeeded);
        assert!(block.finished_at_ms.is_some());
        assert_eq!(block.output_chunks.len(), 2);
        assert_eq!(
            shell.replay_block_output(&block_id).unwrap(),
            "running tests...\nok\n"
        );

        let append_err = shell
            .append_block_output(&block_id, "late output")
            .unwrap_err();
        assert!(append_err.contains("cannot append output"));

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn block_rerun_and_edit_create_new_running_blocks() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let first_block_id = shell
            .start_command_block_for_active_session("npm test")
            .unwrap();
        shell
            .finish_block(&first_block_id, BlockStatus::Failed)
            .unwrap();

        let rerun_block_id = shell.rerun_block(&first_block_id).unwrap();
        let edited_rerun_block_id = shell
            .rerun_block_with_edit(&first_block_id, "npm test --watch=false")
            .unwrap();
        assert_ne!(first_block_id, rerun_block_id);
        assert_ne!(first_block_id, edited_rerun_block_id);

        let session_id = shell
            .block_by_id(&first_block_id)
            .unwrap()
            .session_id
            .clone();
        let session_blocks = shell.blocks_for_session(&session_id);
        assert_eq!(session_blocks.len(), 3);
        assert_eq!(session_blocks[0].input, "npm test");
        assert_eq!(session_blocks[1].input, "npm test");
        assert_eq!(session_blocks[2].input, "npm test --watch=false");
        assert_eq!(session_blocks[1].status, BlockStatus::Running);
        assert_eq!(session_blocks[2].status, BlockStatus::Running);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn block_state_persists_and_rebuilds_index_on_restore() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let block_id = shell
            .start_command_block_for_active_session("cargo build")
            .unwrap();
        shell
            .append_block_output(&block_id, "Compiling ulgen\n")
            .unwrap();
        shell
            .finish_block(&block_id, BlockStatus::Succeeded)
            .unwrap();
        shell.save().unwrap();

        let restored = AppShell::bootstrap(path.clone()).unwrap();
        let restored_block = restored.block_by_id(&block_id).unwrap();
        assert_eq!(restored_block.input, "cargo build");
        assert_eq!(restored_block.status, BlockStatus::Succeeded);
        assert_eq!(
            restored.replay_block_output(&block_id).unwrap(),
            "Compiling ulgen\n"
        );

        let session_id = restored_block.session_id.clone();
        let session_blocks = restored.blocks_for_session(&session_id);
        assert_eq!(session_blocks.len(), 1);
        assert_eq!(session_blocks[0].id, block_id);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn finishing_blocks_emits_completion_and_failure_notifications() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let succeeded_block_id = shell
            .start_command_block_for_active_session("cargo test")
            .unwrap();
        shell
            .finish_block(&succeeded_block_id, BlockStatus::Succeeded)
            .unwrap();

        let failed_block_id = shell
            .start_command_block_for_active_session("cargo test --bad-flag")
            .unwrap();
        shell
            .finish_block(&failed_block_id, BlockStatus::Failed)
            .unwrap();

        let history = shell.notification_history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].kind, NotificationEventKind::TaskDone);
        assert_eq!(
            history[0].block_id.as_deref(),
            Some(succeeded_block_id.as_str())
        );
        assert_eq!(history[1].kind, NotificationEventKind::TaskFailed);
        assert_eq!(
            history[1].block_id.as_deref(),
            Some(failed_block_id.as_str())
        );

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn approval_notifications_and_deep_link_resolution_work_for_blocks() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let block_id = shell
            .start_command_block_for_active_session("rm -rf ./tmp")
            .unwrap();

        shell
            .mark_block_approval_required(&block_id, "User confirmation required")
            .unwrap();

        let event = shell.notification_history().last().cloned().unwrap();
        assert_eq!(event.kind, NotificationEventKind::ApprovalRequired);
        assert_eq!(event.block_id.as_deref(), Some(block_id.as_str()));

        let target = shell
            .resolve_notification_target(&event)
            .unwrap()
            .expect("approval event should deep-link to block target");
        assert_eq!(target.block_id, block_id);

        let no_block_event = NotificationEvent {
            id: 999,
            kind: NotificationEventKind::TaskDone,
            title: "no block".to_string(),
            message: "event without block".to_string(),
            block_id: None,
        };
        assert!(shell
            .resolve_notification_target(&no_block_event)
            .unwrap()
            .is_none());

        let stale_block_event = NotificationEvent {
            id: 1000,
            kind: NotificationEventKind::TaskDone,
            title: "stale block".to_string(),
            message: "event with stale block id".to_string(),
            block_id: Some("block-missing".to_string()),
        };
        let stale_error = shell
            .resolve_notification_target(&stale_block_event)
            .unwrap_err();
        assert!(stale_error.contains("block missing: block-missing"));

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn cancelled_blocks_emit_failed_notifications() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let block_id = shell
            .start_command_block_for_active_session("sleep 120")
            .unwrap();
        shell
            .finish_block(&block_id, BlockStatus::Cancelled)
            .unwrap();

        let event = shell.notification_history().last().cloned().unwrap();
        assert_eq!(event.kind, NotificationEventKind::TaskFailed);
        assert_eq!(event.block_id.as_deref(), Some(block_id.as_str()));
        assert!(event.title.to_ascii_lowercase().contains("cancelled"));

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn resolve_block_navigation_target_errors_when_session_is_missing() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let block_id = shell
            .start_command_block_for_active_session("echo orphan")
            .unwrap();
        shell.state.windows[0].workspaces[0].tabs[0].panes[0].surfaces[0].session_id =
            "session-replaced".to_string();

        let error = shell
            .resolve_block_navigation_target(&block_id)
            .unwrap_err();
        assert!(error.contains("navigation target missing for block session"));

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn notification_input_summary_handles_empty_and_truncation() {
        assert_eq!(notification_input_summary("   "), "(empty command)");
        assert_eq!(notification_input_summary("cargo test"), "cargo test");

        let long = "x".repeat(120);
        let summarized = notification_input_summary(&long);
        assert!(summarized.ends_with("..."));
        assert!(summarized.len() < long.len());
    }

    #[test]
    fn rejects_restore_with_duplicate_block_ids() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let duplicate_blocks = r#"{
            "version":2,
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
            "blocks":[
                {
                    "id":"block-1",
                    "session_id":"session-1",
                    "input":"echo one",
                    "output_chunks":[],
                    "status":"Succeeded",
                    "started_at_ms":1,
                    "finished_at_ms":2
                },
                {
                    "id":"block-1",
                    "session_id":"session-1",
                    "input":"echo two",
                    "output_chunks":[],
                    "status":"Failed",
                    "started_at_ms":3,
                    "finished_at_ms":4
                }
            ],
            "next_id":2,
            "last_started_at_ms":0
        }"#;

        fs::write(&path, duplicate_blocks).unwrap();
        let err = AppShell::bootstrap(path.clone())
            .err()
            .expect("duplicate block ids should fail bootstrap");
        assert!(err.to_string().contains("duplicate block id detected"));

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn block_engine_rejects_missing_targets_and_invalid_transitions() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();

        let missing_session = shell
            .start_command_block_for_session("session-missing", "ls")
            .unwrap_err();
        assert!(missing_session.contains("session missing"));

        let missing_block = shell
            .append_block_output("block-missing", "output")
            .unwrap_err();
        assert!(missing_block.contains("block missing"));

        let block_id = shell
            .start_command_block_for_active_session("echo ok")
            .unwrap();
        let invalid_status = shell
            .finish_block(&block_id, BlockStatus::Running)
            .unwrap_err();
        assert!(invalid_status.contains("terminal status"));
        shell
            .finish_block(&block_id, BlockStatus::Succeeded)
            .unwrap();
        let double_finish = shell
            .finish_block(&block_id, BlockStatus::Cancelled)
            .unwrap_err();
        assert!(double_finish.contains("already finalized"));

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }
}
