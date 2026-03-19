mod app_shell;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use app_shell::{default_state_path, AppShell, AppShellCommand};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--smoke") {
        run_smoke();
        return;
    }

    let state_path = default_state_path();
    let mut app_shell =
        AppShell::bootstrap(state_path.clone()).expect("app shell bootstrap should succeed");

    if args.iter().any(|a| a == "--new-window") {
        app_shell
            .route_command(AppShellCommand::NewWindow)
            .expect("window creation should succeed");
    }

    if let Some(idx) = args.iter().position(|a| a == "--new-workspace") {
        let name = args
            .get(idx + 1)
            .cloned()
            .unwrap_or_else(|| "workspace".to_string());
        app_shell
            .route_command(AppShellCommand::CreateWorkspace { name })
            .expect("workspace creation should succeed");
    }

    if let Some(idx) = args.iter().position(|a| a == "--command") {
        let command_id = args
            .get(idx + 1)
            .cloned()
            .unwrap_or_else(|| "workspace.next".to_string());
        app_shell
            .route_command_id(&command_id)
            .expect("command routing should succeed");
    }

    app_shell
        .save()
        .expect("saving app shell state should succeed");

    println!("Ulgen app shell bootstrap is ready.");
    println!("{}", app_shell.startup_summary());
    println!("State path: {}", app_shell.state_path().display());
    println!(
        "Registered commands: {}",
        app_shell.command_registry().search("").len()
    );
    println!("Run with --smoke to execute a deterministic bootstrap and restore simulation.");
}

fn run_smoke() {
    let state_path = smoke_state_path();
    let mut app_shell =
        AppShell::bootstrap(state_path.clone()).expect("smoke app shell should bootstrap");
    app_shell
        .route_command(AppShellCommand::NewWindow)
        .expect("smoke window should be created");
    app_shell
        .route_command(AppShellCommand::CreateWorkspace {
            name: "smoke".to_string(),
        })
        .expect("smoke workspace should be created");
    app_shell.save().expect("smoke save should succeed");

    let restored = AppShell::bootstrap(state_path.clone()).expect("smoke restore should succeed");
    let settings = &restored.state().settings;

    println!(
        "Smoke OK: sidebar={:?}, keymap={:?}, windows={}, active_window_workspaces={}.",
        settings.sidebar_position,
        settings.keymap_profile,
        restored.state().windows.len(),
        restored.state().windows[restored.state().active_window]
            .workspaces
            .len()
    );

    if state_path.exists() {
        let _ = fs::remove_file(state_path);
    }
}

fn smoke_state_path() -> PathBuf {
    std::env::temp_dir().join(format!("ulgen-smoke-state-{}.json", now_ms()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
