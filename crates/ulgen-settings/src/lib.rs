use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarPosition {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeymapProfile {
    Warp,
    Tmux,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Light,
    Dark,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorStyle {
    Bar,
    Block,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputPosition {
    TopClassic,
    TopReverse,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationsPolicy {
    InAppOnly,
    OsOnly,
    InAppAndOs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentPermissionDefault {
    AlwaysAsk,
    AskOnce,
    AlwaysAllow,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    pub sidebar_position: SidebarPosition,
    pub keymap_profile: KeymapProfile,
    pub theme_mode: ThemeMode,
    pub cursor_style: CursorStyle,
    pub input_position: InputPosition,
    pub notifications_policy: NotificationsPolicy,
    pub agent_permissions: AgentPermissionDefault,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            sidebar_position: SidebarPosition::Left,
            keymap_profile: KeymapProfile::Warp,
            theme_mode: ThemeMode::System,
            cursor_style: CursorStyle::Block,
            input_position: InputPosition::Bottom,
            notifications_policy: NotificationsPolicy::InAppAndOs,
            agent_permissions: AgentPermissionDefault::AlwaysAsk,
        }
    }
}
