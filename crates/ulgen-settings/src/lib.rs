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
pub enum ThemePreset {
    Horizon,
    Grove,
    Ember,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeTokens {
    pub surface_bg: String,
    pub surface_fg: String,
    pub surface_muted: String,
    pub border: String,
    pub accent: String,
    pub success: String,
    pub warning: String,
    pub danger: String,
    pub terminal_bg: String,
    pub terminal_fg: String,
    pub terminal_cursor: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedTheme {
    pub id: String,
    pub name: String,
    pub light: ThemeTokens,
    pub dark: ThemeTokens,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTheme {
    pub mode: ThemeMode,
    pub preset: ThemePreset,
    pub custom_theme_id: Option<String>,
    pub custom_theme_name: Option<String>,
    pub tokens: ThemeTokens,
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
    pub theme_preset: ThemePreset,
    pub custom_themes: Vec<ImportedTheme>,
    pub active_custom_theme_id: Option<String>,
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
            theme_preset: ThemePreset::Horizon,
            custom_themes: Vec::new(),
            active_custom_theme_id: None,
            cursor_style: CursorStyle::Block,
            input_position: InputPosition::Bottom,
            notifications_policy: NotificationsPolicy::InAppAndOs,
            agent_permissions: AgentPermissionDefault::AlwaysAsk,
        }
    }
}

pub fn resolve_theme(
    mode: ThemeMode,
    preset: ThemePreset,
    system_mode: Option<ThemeMode>,
) -> ResolvedTheme {
    resolve_theme_with_custom(mode, preset, system_mode, &[], None)
}

pub fn resolve_theme_with_custom(
    mode: ThemeMode,
    preset: ThemePreset,
    system_mode: Option<ThemeMode>,
    custom_themes: &[ImportedTheme],
    active_custom_theme_id: Option<&str>,
) -> ResolvedTheme {
    let mode = effective_theme_mode(mode, system_mode);
    if let Some(active_id) = active_custom_theme_id {
        if let Some(custom_theme) = custom_themes.iter().find(|theme| theme.id == active_id) {
            let tokens = match mode {
                ThemeMode::Light => custom_theme.light.clone(),
                ThemeMode::Dark => custom_theme.dark.clone(),
                ThemeMode::System => {
                    unreachable!("system mode is normalized before token resolve")
                }
            };
            return ResolvedTheme {
                mode,
                preset,
                custom_theme_id: Some(custom_theme.id.clone()),
                custom_theme_name: Some(custom_theme.name.clone()),
                tokens,
            };
        }
    }

    let tokens = match (preset, mode) {
        (ThemePreset::Horizon, ThemeMode::Light) => ThemeTokens {
            surface_bg: "#f8f3e8".to_string(),
            surface_fg: "#2f2a22".to_string(),
            surface_muted: "#9a8f7f".to_string(),
            border: "#d8cdbd".to_string(),
            accent: "#cf6f2e".to_string(),
            success: "#1f8a5b".to_string(),
            warning: "#b77600".to_string(),
            danger: "#b93a42".to_string(),
            terminal_bg: "#fffaf0".to_string(),
            terminal_fg: "#2f2a22".to_string(),
            terminal_cursor: "#cf6f2e".to_string(),
        },
        (ThemePreset::Horizon, ThemeMode::Dark) => ThemeTokens {
            surface_bg: "#171411".to_string(),
            surface_fg: "#f1e7d8".to_string(),
            surface_muted: "#aa9c8a".to_string(),
            border: "#3a3027".to_string(),
            accent: "#f79d4b".to_string(),
            success: "#4ecb8f".to_string(),
            warning: "#f3b44e".to_string(),
            danger: "#ef6c75".to_string(),
            terminal_bg: "#0f0c0a".to_string(),
            terminal_fg: "#f1e7d8".to_string(),
            terminal_cursor: "#f79d4b".to_string(),
        },
        (ThemePreset::Grove, ThemeMode::Light) => ThemeTokens {
            surface_bg: "#eef6ee".to_string(),
            surface_fg: "#1f3024".to_string(),
            surface_muted: "#6c8573".to_string(),
            border: "#c7ddcb".to_string(),
            accent: "#2f9d57".to_string(),
            success: "#26834a".to_string(),
            warning: "#c08b12".to_string(),
            danger: "#c24f58".to_string(),
            terminal_bg: "#f5fbf5".to_string(),
            terminal_fg: "#1f3024".to_string(),
            terminal_cursor: "#2f9d57".to_string(),
        },
        (ThemePreset::Grove, ThemeMode::Dark) => ThemeTokens {
            surface_bg: "#0f1712".to_string(),
            surface_fg: "#dcf0df".to_string(),
            surface_muted: "#94ad98".to_string(),
            border: "#2b4332".to_string(),
            accent: "#5ad688".to_string(),
            success: "#48c879".to_string(),
            warning: "#e0b957".to_string(),
            danger: "#e67d84".to_string(),
            terminal_bg: "#090f0b".to_string(),
            terminal_fg: "#dcf0df".to_string(),
            terminal_cursor: "#5ad688".to_string(),
        },
        (ThemePreset::Ember, ThemeMode::Light) => ThemeTokens {
            surface_bg: "#f6f0ee".to_string(),
            surface_fg: "#332422".to_string(),
            surface_muted: "#8f7572".to_string(),
            border: "#ddccca".to_string(),
            accent: "#b6462e".to_string(),
            success: "#1f8a6f".to_string(),
            warning: "#bb7600".to_string(),
            danger: "#b93a42".to_string(),
            terminal_bg: "#fcf6f5".to_string(),
            terminal_fg: "#332422".to_string(),
            terminal_cursor: "#b6462e".to_string(),
        },
        (ThemePreset::Ember, ThemeMode::Dark) => ThemeTokens {
            surface_bg: "#160f0f".to_string(),
            surface_fg: "#f2dfdc".to_string(),
            surface_muted: "#af9390".to_string(),
            border: "#3f2c2b".to_string(),
            accent: "#ff7a5c".to_string(),
            success: "#4dc4a8".to_string(),
            warning: "#f0b45d".to_string(),
            danger: "#f17f87".to_string(),
            terminal_bg: "#0c0808".to_string(),
            terminal_fg: "#f2dfdc".to_string(),
            terminal_cursor: "#ff7a5c".to_string(),
        },
        (_, ThemeMode::System) => unreachable!("system mode is normalized before token resolve"),
    };

    ResolvedTheme {
        mode,
        preset,
        custom_theme_id: None,
        custom_theme_name: None,
        tokens,
    }
}

fn effective_theme_mode(mode: ThemeMode, system_mode: Option<ThemeMode>) -> ThemeMode {
    match mode {
        ThemeMode::Light => ThemeMode::Light,
        ThemeMode::Dark => ThemeMode::Dark,
        ThemeMode::System => match system_mode {
            Some(ThemeMode::Dark) => ThemeMode::Dark,
            Some(ThemeMode::Light) => ThemeMode::Light,
            _ => ThemeMode::Dark,
        },
    }
}

pub fn import_theme_definition(serialized: &str) -> Result<ImportedTheme, String> {
    let theme: ImportedTheme = serde_json::from_str(serialized)
        .map_err(|error| format!("invalid theme definition json: {error}"))?;
    validate_theme_definition(&theme)?;
    Ok(theme)
}

pub fn export_theme_definition(theme: &ImportedTheme) -> Result<String, String> {
    validate_theme_definition(theme)?;
    serde_json::to_string_pretty(theme)
        .map_err(|error| format!("failed to serialize theme: {error}"))
}

fn validate_theme_definition(theme: &ImportedTheme) -> Result<(), String> {
    if theme.id.trim().is_empty() {
        return Err("theme id must not be empty".to_string());
    }
    if theme.name.trim().is_empty() {
        return Err("theme name must not be empty".to_string());
    }

    let token_sets = [(&theme.light, "light"), (&theme.dark, "dark")];
    for (tokens, mode_label) in token_sets {
        for (token_name, token_value) in [
            ("surface_bg", &tokens.surface_bg),
            ("surface_fg", &tokens.surface_fg),
            ("surface_muted", &tokens.surface_muted),
            ("border", &tokens.border),
            ("accent", &tokens.accent),
            ("success", &tokens.success),
            ("warning", &tokens.warning),
            ("danger", &tokens.danger),
            ("terminal_bg", &tokens.terminal_bg),
            ("terminal_fg", &tokens.terminal_fg),
            ("terminal_cursor", &tokens.terminal_cursor),
        ] {
            if token_value.trim().is_empty() {
                return Err(format!(
                    "theme token must not be empty: {mode_label}.{token_name}"
                ));
            }
        }
    }

    Ok(())
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
        assert_eq!(settings.theme_preset, ThemePreset::Horizon);
        assert!(settings.custom_themes.is_empty());
        assert_eq!(settings.active_custom_theme_id, None);
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

    #[test]
    fn resolve_theme_uses_system_mode_fallback() {
        let resolved = resolve_theme(
            ThemeMode::System,
            ThemePreset::Grove,
            Some(ThemeMode::Light),
        );
        assert_eq!(resolved.mode, ThemeMode::Light);
        assert_eq!(resolved.preset, ThemePreset::Grove);
        assert_eq!(resolved.custom_theme_id, None);
    }

    #[test]
    fn resolve_theme_differs_between_presets() {
        let horizon = resolve_theme(ThemeMode::Dark, ThemePreset::Horizon, None);
        let ember = resolve_theme(ThemeMode::Dark, ThemePreset::Ember, None);
        assert_ne!(horizon.tokens.accent, ember.tokens.accent);
        assert_ne!(horizon.tokens.terminal_bg, ember.tokens.terminal_bg);
    }

    #[test]
    fn resolve_theme_system_none_defaults_to_dark() {
        let resolved = resolve_theme(ThemeMode::System, ThemePreset::Horizon, None);
        assert_eq!(resolved.mode, ThemeMode::Dark);
    }

    #[test]
    fn import_export_theme_definition_roundtrips() {
        let serialized = r##"{
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

        let imported = import_theme_definition(serialized).unwrap();
        assert_eq!(imported.id, "aurora");
        assert_eq!(imported.name, "Aurora");

        let exported = export_theme_definition(&imported).unwrap();
        let reparsed = import_theme_definition(&exported).unwrap();
        assert_eq!(imported, reparsed);
    }

    #[test]
    fn resolve_theme_uses_active_custom_theme_when_available() {
        let custom = ImportedTheme {
            id: "aurora".to_string(),
            name: "Aurora".to_string(),
            light: ThemeTokens {
                surface_bg: "#ffffff".to_string(),
                surface_fg: "#111111".to_string(),
                surface_muted: "#666666".to_string(),
                border: "#dddddd".to_string(),
                accent: "#3f67ff".to_string(),
                success: "#1f8a5b".to_string(),
                warning: "#b77500".to_string(),
                danger: "#c43a45".to_string(),
                terminal_bg: "#f9fbff".to_string(),
                terminal_fg: "#111111".to_string(),
                terminal_cursor: "#3f67ff".to_string(),
            },
            dark: ThemeTokens {
                surface_bg: "#121212".to_string(),
                surface_fg: "#efefef".to_string(),
                surface_muted: "#9e9e9e".to_string(),
                border: "#2b2b2b".to_string(),
                accent: "#7e96ff".to_string(),
                success: "#52cb92".to_string(),
                warning: "#f2bf63".to_string(),
                danger: "#ef7b84".to_string(),
                terminal_bg: "#090b17".to_string(),
                terminal_fg: "#efefef".to_string(),
                terminal_cursor: "#7e96ff".to_string(),
            },
        };

        let resolved = resolve_theme_with_custom(
            ThemeMode::Dark,
            ThemePreset::Horizon,
            None,
            &[custom.clone()],
            Some("aurora"),
        );
        assert_eq!(resolved.custom_theme_id, Some("aurora".to_string()));
        assert_eq!(resolved.custom_theme_name, Some("Aurora".to_string()));
        assert_eq!(resolved.tokens.accent, custom.dark.accent);
    }
}
