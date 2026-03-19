use std::env;

use ulgen_command::{CommandAction, CommandRegistry};
use ulgen_muxd::{MuxRequest, MuxRpc, MuxState};
use ulgen_settings::AppSettings;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--smoke") {
        run_smoke();
        return;
    }

    println!("Ulgen app shell bootstrap is ready.");
    println!("Run with --smoke to execute a minimal command-flow simulation.");
}

fn run_smoke() {
    let settings = AppSettings::default();
    let mut mux = MuxState::new();

    let mut commands = CommandRegistry::new();
    commands.register(CommandAction {
        id: "workspace.create".to_string(),
        title: "Create Workspace".to_string(),
        description: "Create a new workspace in muxd".to_string(),
    });

    let _ = mux
        .handle(MuxRequest::WorkspaceCreate {
            name: "smoke".to_string(),
        })
        .expect("workspace should be created");

    println!(
        "Smoke OK: sidebar={:?}, keymap={:?}, workspaces={}.",
        settings.sidebar_position,
        settings.keymap_profile,
        mux.workspaces.len()
    );
}
