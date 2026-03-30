use std::collections::{BTreeMap, HashSet};

pub mod command_ids {
    pub const WINDOW_NEW: &str = "window.new";
    pub const WINDOW_NEXT: &str = "window.next";
    pub const WORKSPACE_NEW: &str = "workspace.new";
    pub const WORKSPACE_NEXT: &str = "workspace.next";
    pub const WORKSPACE_PREV: &str = "workspace.prev";
    pub const TAB_NEW: &str = "tab.new";
    pub const TAB_NEXT: &str = "tab.next";
    pub const TAB_PREV: &str = "tab.prev";
    pub const PANE_NEXT: &str = "pane.next";
    pub const PANE_PREV: &str = "pane.prev";
    pub const PANE_SPLIT_RIGHT: &str = "pane.split.right";
    pub const PANE_SPLIT_DOWN: &str = "pane.split.down";
    pub const SIDEBAR_TOGGLE_POSITION: &str = "sidebar.position.toggle";
    pub const SIDEBAR_NEXT: &str = "sidebar.next";
    pub const SIDEBAR_PREV: &str = "sidebar.prev";
    pub const CURSOR_STYLE_BAR: &str = "cursor.style.bar";
    pub const CURSOR_STYLE_BLOCK: &str = "cursor.style.block";
    pub const CURSOR_STYLE_UNDERLINE: &str = "cursor.style.underline";
    pub const INPUT_POSITION_TOP: &str = "input.position.top";
    pub const INPUT_POSITION_TOP_REVERSE: &str = "input.position.top.reverse";
    pub const INPUT_POSITION_BOTTOM: &str = "input.position.bottom";
}

pub fn baseline_command_ids() -> &'static [&'static str] {
    &[
        command_ids::WINDOW_NEW,
        command_ids::WINDOW_NEXT,
        command_ids::WORKSPACE_NEW,
        command_ids::WORKSPACE_NEXT,
        command_ids::WORKSPACE_PREV,
        command_ids::TAB_NEW,
        command_ids::TAB_NEXT,
        command_ids::TAB_PREV,
        command_ids::PANE_NEXT,
        command_ids::PANE_PREV,
        command_ids::PANE_SPLIT_RIGHT,
        command_ids::PANE_SPLIT_DOWN,
        command_ids::SIDEBAR_TOGGLE_POSITION,
        command_ids::SIDEBAR_NEXT,
        command_ids::SIDEBAR_PREV,
        command_ids::CURSOR_STYLE_BAR,
        command_ids::CURSOR_STYLE_BLOCK,
        command_ids::CURSOR_STYLE_UNDERLINE,
        command_ids::INPUT_POSITION_TOP,
        command_ids::INPUT_POSITION_TOP_REVERSE,
        command_ids::INPUT_POSITION_BOTTOM,
    ]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandAction {
    pub id: String,
    pub title: String,
    pub description: String,
}

#[derive(Default)]
pub struct CommandRegistry {
    actions: Vec<CommandAction>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, action: CommandAction) {
        self.actions.push(action);
    }

    pub fn contains_id(&self, id: &str) -> bool {
        self.actions.iter().any(|action| action.id == id)
    }

    pub fn search(&self, query: &str) -> Vec<CommandAction> {
        let q = query.to_lowercase();
        self.actions
            .iter()
            .filter(|a| {
                a.title.to_lowercase().contains(&q)
                    || a.id.to_lowercase().contains(&q)
                    || a.description.to_lowercase().contains(&q)
            })
            .cloned()
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeymapProfile {
    Warp,
    Tmux,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    pub chord: String,
    pub command_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RejectedKeyBindingReason {
    EmptyChord,
    UnknownCommand,
    ChordConflict { existing_command_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedKeyBinding {
    pub binding: KeyBinding,
    pub reason: RejectedKeyBindingReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedKeymap {
    bindings: Vec<KeyBinding>,
    rejected_overrides: Vec<RejectedKeyBinding>,
    chord_to_command: BTreeMap<String, String>,
}

impl ResolvedKeymap {
    pub fn bindings(&self) -> &[KeyBinding] {
        &self.bindings
    }

    pub fn rejected_overrides(&self) -> &[RejectedKeyBinding] {
        &self.rejected_overrides
    }

    pub fn command_for_chord(&self, chord: &str) -> Option<&str> {
        let normalized = normalize_chord(chord);
        self.chord_to_command.get(&normalized).map(String::as_str)
    }
}

pub fn default_keybindings(profile: KeymapProfile) -> Vec<KeyBinding> {
    use command_ids::*;
    match profile {
        KeymapProfile::Warp => vec![
            KeyBinding {
                chord: "ctrl+shift+n".to_string(),
                command_id: WINDOW_NEW.to_string(),
            },
            KeyBinding {
                chord: "ctrl+alt+]".to_string(),
                command_id: WINDOW_NEXT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+shift+enter".to_string(),
                command_id: WORKSPACE_NEW.to_string(),
            },
            KeyBinding {
                chord: "ctrl+alt+down".to_string(),
                command_id: WORKSPACE_NEXT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+alt+up".to_string(),
                command_id: WORKSPACE_PREV.to_string(),
            },
            KeyBinding {
                chord: "ctrl+t".to_string(),
                command_id: TAB_NEW.to_string(),
            },
            KeyBinding {
                chord: "ctrl+tab".to_string(),
                command_id: TAB_NEXT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+shift+tab".to_string(),
                command_id: TAB_PREV.to_string(),
            },
            KeyBinding {
                chord: "ctrl+shift+]".to_string(),
                command_id: PANE_NEXT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+shift+[".to_string(),
                command_id: PANE_PREV.to_string(),
            },
            KeyBinding {
                chord: "ctrl+shift+d".to_string(),
                command_id: PANE_SPLIT_RIGHT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+shift+v".to_string(),
                command_id: PANE_SPLIT_DOWN.to_string(),
            },
            KeyBinding {
                chord: "ctrl+alt+s".to_string(),
                command_id: SIDEBAR_TOGGLE_POSITION.to_string(),
            },
            KeyBinding {
                chord: "ctrl+alt+j".to_string(),
                command_id: SIDEBAR_NEXT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+alt+k".to_string(),
                command_id: SIDEBAR_PREV.to_string(),
            },
        ],
        KeymapProfile::Tmux => vec![
            KeyBinding {
                chord: "ctrl+b c".to_string(),
                command_id: WINDOW_NEW.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b n".to_string(),
                command_id: WINDOW_NEXT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b w".to_string(),
                command_id: WORKSPACE_NEW.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b )".to_string(),
                command_id: WORKSPACE_NEXT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b (".to_string(),
                command_id: WORKSPACE_PREV.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b t".to_string(),
                command_id: TAB_NEW.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b l".to_string(),
                command_id: TAB_NEXT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b p".to_string(),
                command_id: TAB_PREV.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b o".to_string(),
                command_id: PANE_NEXT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b ;".to_string(),
                command_id: PANE_PREV.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b %".to_string(),
                command_id: PANE_SPLIT_RIGHT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b \"".to_string(),
                command_id: PANE_SPLIT_DOWN.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b s".to_string(),
                command_id: SIDEBAR_TOGGLE_POSITION.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b ]".to_string(),
                command_id: SIDEBAR_NEXT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+b [".to_string(),
                command_id: SIDEBAR_PREV.to_string(),
            },
        ],
    }
}

pub fn resolve_keymap(profile: KeymapProfile, overrides: &[KeyBinding]) -> ResolvedKeymap {
    let known_commands = baseline_command_ids()
        .iter()
        .copied()
        .collect::<HashSet<_>>();

    let mut chord_to_command = BTreeMap::<String, String>::new();
    let mut command_to_chord = BTreeMap::<String, String>::new();

    for binding in default_keybindings(profile) {
        apply_binding(&mut command_to_chord, &mut chord_to_command, binding);
    }

    let mut rejected = Vec::new();
    let mut normalized_overrides = Vec::new();
    for override_binding in overrides {
        let normalized_chord = normalize_chord(&override_binding.chord);
        let command_id = override_binding.command_id.trim().to_string();
        let normalized = KeyBinding {
            chord: normalized_chord.clone(),
            command_id: command_id.clone(),
        };

        if normalized_chord.is_empty() {
            rejected.push(RejectedKeyBinding {
                binding: normalized,
                reason: RejectedKeyBindingReason::EmptyChord,
            });
            continue;
        }

        if command_id.is_empty() || !known_commands.contains(command_id.as_str()) {
            rejected.push(RejectedKeyBinding {
                binding: normalized,
                reason: RejectedKeyBindingReason::UnknownCommand,
            });
            continue;
        }

        normalized_overrides.push(normalized);
    }

    let mut last_index_by_command = BTreeMap::<String, usize>::new();
    for (index, binding) in normalized_overrides.iter().enumerate() {
        last_index_by_command.insert(binding.command_id.clone(), index);
    }

    let effective_overrides = normalized_overrides
        .into_iter()
        .enumerate()
        .filter_map(|(index, binding)| {
            let last_index = last_index_by_command.get(&binding.command_id)?;
            if *last_index == index {
                return Some(binding);
            }
            None
        })
        .collect::<Vec<_>>();

    let mut previous_bindings = BTreeMap::<String, String>::new();
    for binding in &effective_overrides {
        if let Some(previous_chord) = command_to_chord.remove(&binding.command_id) {
            chord_to_command.remove(&previous_chord);
            previous_bindings.insert(binding.command_id.clone(), previous_chord);
        }
    }

    for binding in effective_overrides {
        // Policy for collisions among effective overrides:
        // first successfully applied binding keeps the chord; later conflicting
        // overrides are rejected and their previous bindings are restored when possible.
        if let Some(existing_command_id) = chord_to_command.get(&binding.chord).cloned() {
            if existing_command_id != binding.command_id {
                if let Some(previous_chord) = previous_bindings.get(&binding.command_id) {
                    if !chord_to_command.contains_key(previous_chord) {
                        chord_to_command.insert(previous_chord.clone(), binding.command_id.clone());
                        command_to_chord.insert(binding.command_id.clone(), previous_chord.clone());
                    }
                }

                rejected.push(RejectedKeyBinding {
                    binding,
                    reason: RejectedKeyBindingReason::ChordConflict {
                        existing_command_id,
                    },
                });
                continue;
            }
        }

        apply_binding(&mut command_to_chord, &mut chord_to_command, binding);
    }

    let bindings = command_to_chord
        .into_iter()
        .map(|(command_id, chord)| KeyBinding { chord, command_id })
        .collect();

    ResolvedKeymap {
        bindings,
        rejected_overrides: rejected,
        chord_to_command,
    }
}

fn apply_binding(
    command_to_chord: &mut BTreeMap<String, String>,
    chord_to_command: &mut BTreeMap<String, String>,
    binding: KeyBinding,
) {
    if let Some(previous_chord) =
        command_to_chord.insert(binding.command_id.clone(), binding.chord.clone())
    {
        chord_to_command.remove(&previous_chord);
    }
    chord_to_command.insert(binding.chord, binding.command_id);
}

fn normalize_chord(chord: &str) -> String {
    chord
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" + ", "+")
        .replace(" +", "+")
        .replace("+ ", "+")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_ids;

    #[test]
    fn search_returns_matching_actions() {
        let mut registry = CommandRegistry::new();
        registry.register(CommandAction {
            id: "workspace.select".to_string(),
            title: "Select Workspace".to_string(),
            description: "Switch to workspace by id".to_string(),
        });

        let matches = registry.search("workspace");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "workspace.select");
    }

    #[test]
    fn resolve_keymap_returns_baseline_bindings() {
        let resolved = resolve_keymap(KeymapProfile::Warp, &[]);

        assert_eq!(resolved.rejected_overrides().len(), 0);
        assert_eq!(
            resolved.command_for_chord("ctrl+shift+n"),
            Some(command_ids::WINDOW_NEW)
        );
        assert_eq!(
            resolved.command_for_chord("CTRL+SHIFT+V"),
            Some(command_ids::PANE_SPLIT_DOWN)
        );
        assert_eq!(
            resolved.command_for_chord("ctrl+shift+tab"),
            Some(command_ids::TAB_PREV)
        );
        assert_eq!(
            resolved.command_for_chord("ctrl+alt+s"),
            Some(command_ids::SIDEBAR_TOGGLE_POSITION)
        );
        assert_eq!(
            resolved.command_for_chord("ctrl+alt+j"),
            Some(command_ids::SIDEBAR_NEXT)
        );
        assert_eq!(
            resolved.command_for_chord("ctrl+alt+k"),
            Some(command_ids::SIDEBAR_PREV)
        );
    }

    #[test]
    fn resolve_tmux_keymap_includes_sidebar_bindings() {
        let resolved = resolve_keymap(KeymapProfile::Tmux, &[]);

        assert_eq!(resolved.rejected_overrides().len(), 0);
        assert_eq!(
            resolved.command_for_chord("ctrl+b c"),
            Some(command_ids::WINDOW_NEW)
        );
        assert_eq!(
            resolved.command_for_chord("ctrl+b s"),
            Some(command_ids::SIDEBAR_TOGGLE_POSITION)
        );
        assert_eq!(
            resolved.command_for_chord("ctrl+b ]"),
            Some(command_ids::SIDEBAR_NEXT)
        );
        assert_eq!(
            resolved.command_for_chord("ctrl+b ["),
            Some(command_ids::SIDEBAR_PREV)
        );
    }

    #[test]
    fn valid_override_rebinds_command() {
        let overrides = vec![KeyBinding {
            chord: "alt+.".to_string(),
            command_id: command_ids::WORKSPACE_NEXT.to_string(),
        }];
        let resolved = resolve_keymap(KeymapProfile::Warp, &overrides);

        assert_eq!(resolved.rejected_overrides().len(), 0);
        assert_eq!(
            resolved.command_for_chord("alt+."),
            Some(command_ids::WORKSPACE_NEXT)
        );
        assert_eq!(resolved.command_for_chord("ctrl+alt+down"), None);
    }

    #[test]
    fn conflicting_override_is_rejected() {
        let overrides = vec![KeyBinding {
            chord: "ctrl+tab".to_string(),
            command_id: command_ids::WORKSPACE_NEXT.to_string(),
        }];
        let resolved = resolve_keymap(KeymapProfile::Warp, &overrides);

        assert_eq!(resolved.rejected_overrides().len(), 1);
        assert_eq!(
            resolved.rejected_overrides()[0].reason,
            RejectedKeyBindingReason::ChordConflict {
                existing_command_id: command_ids::TAB_NEXT.to_string()
            }
        );
        assert_eq!(
            resolved.command_for_chord("ctrl+tab"),
            Some(command_ids::TAB_NEXT)
        );
        assert_eq!(
            resolved.command_for_chord("ctrl+alt+down"),
            Some(command_ids::WORKSPACE_NEXT)
        );
    }

    #[test]
    fn unknown_command_override_is_rejected() {
        let overrides = vec![KeyBinding {
            chord: "alt+x".to_string(),
            command_id: "workspace.missing".to_string(),
        }];
        let resolved = resolve_keymap(KeymapProfile::Tmux, &overrides);

        assert_eq!(resolved.rejected_overrides().len(), 1);
        assert_eq!(
            resolved.rejected_overrides()[0].reason,
            RejectedKeyBindingReason::UnknownCommand
        );
    }

    #[test]
    fn remap_last_wins_for_same_command() {
        let overrides = vec![
            KeyBinding {
                chord: "alt+1".to_string(),
                command_id: command_ids::WORKSPACE_NEXT.to_string(),
            },
            KeyBinding {
                chord: "alt+2".to_string(),
                command_id: command_ids::WORKSPACE_NEXT.to_string(),
            },
        ];
        let resolved = resolve_keymap(KeymapProfile::Warp, &overrides);

        assert_eq!(resolved.rejected_overrides().len(), 0);
        assert_eq!(
            resolved.command_for_chord("alt+2"),
            Some(command_ids::WORKSPACE_NEXT)
        );
        assert_eq!(resolved.command_for_chord("alt+1"), None);
    }

    #[test]
    fn swap_remaps_apply_without_false_conflicts() {
        let overrides = vec![
            KeyBinding {
                chord: "ctrl+tab".to_string(),
                command_id: command_ids::WORKSPACE_NEXT.to_string(),
            },
            KeyBinding {
                chord: "ctrl+alt+down".to_string(),
                command_id: command_ids::TAB_NEXT.to_string(),
            },
        ];
        let resolved = resolve_keymap(KeymapProfile::Warp, &overrides);

        assert_eq!(resolved.rejected_overrides().len(), 0);
        assert_eq!(
            resolved.command_for_chord("ctrl+tab"),
            Some(command_ids::WORKSPACE_NEXT)
        );
        assert_eq!(
            resolved.command_for_chord("ctrl+alt+down"),
            Some(command_ids::TAB_NEXT)
        );
    }

    #[test]
    fn conflicting_overrides_for_same_chord_keep_first_applied_binding() {
        let overrides = vec![
            KeyBinding {
                chord: "alt+1".to_string(),
                command_id: command_ids::WORKSPACE_NEXT.to_string(),
            },
            KeyBinding {
                chord: "alt+1".to_string(),
                command_id: command_ids::TAB_NEXT.to_string(),
            },
        ];
        let resolved = resolve_keymap(KeymapProfile::Warp, &overrides);

        assert_eq!(resolved.rejected_overrides().len(), 1);
        assert_eq!(
            resolved.command_for_chord("alt+1"),
            Some(command_ids::WORKSPACE_NEXT)
        );
    }

    #[test]
    fn normalize_chord_treats_plus_spacing_consistently() {
        let resolved = resolve_keymap(
            KeymapProfile::Warp,
            &[KeyBinding {
                chord: "ctrl + tab".to_string(),
                command_id: command_ids::TAB_NEXT.to_string(),
            }],
        );

        assert_eq!(resolved.rejected_overrides().len(), 0);
        assert_eq!(
            resolved.command_for_chord("ctrl+tab"),
            Some(command_ids::TAB_NEXT)
        );
    }
}
