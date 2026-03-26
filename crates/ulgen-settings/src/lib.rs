use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarPosition {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeymapProfile {
    #[serde(alias = "warp")]
    Warp,
    #[serde(alias = "tmux")]
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
pub struct KeymapOverride {
    pub chord: String,
    pub command_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub sidebar_position: SidebarPosition,
    pub keymap_profile: KeymapProfile,
    pub keymap_overrides: Vec<KeymapOverride>,
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
            keymap_overrides: Vec::new(),
            theme_mode: ThemeMode::System,
            cursor_style: CursorStyle::Block,
            input_position: InputPosition::Bottom,
            notifications_policy: NotificationsPolicy::InAppAndOs,
            agent_permissions: AgentPermissionDefault::AlwaysAsk,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_backward_compatible_without_keymap_overrides() {
        let data = r#"{
            "sidebar_position":"Left",
            "keymap_profile":"Warp",
            "theme_mode":"System",
            "cursor_style":"Block",
            "input_position":"Bottom",
            "notifications_policy":"InAppAndOs",
            "agent_permissions":"AlwaysAsk"
        }"#;

        let settings: AppSettings = serde_json::from_str(data).unwrap();
        assert_eq!(settings.keymap_overrides, Vec::<KeymapOverride>::new());
        assert_eq!(settings.keymap_profile, KeymapProfile::Warp);
    }

    #[test]
    fn deserializes_lowercase_keymap_profile_alias() {
        let data = r#"{
            "sidebar_position":"Left",
            "keymap_profile":"tmux",
            "theme_mode":"System",
            "cursor_style":"Block",
            "input_position":"Bottom",
            "notifications_policy":"InAppAndOs",
            "agent_permissions":"AlwaysAsk",
            "keymap_overrides":[]
        }"#;

        let settings: AppSettings = serde_json::from_str(data).unwrap();
        assert_eq!(settings.keymap_profile, KeymapProfile::Tmux);
    }
}
