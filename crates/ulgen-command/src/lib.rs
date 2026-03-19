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

#[cfg(test)]
mod tests {
    use super::*;

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
}
