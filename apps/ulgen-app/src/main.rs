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

    match command_id_from_args(&args) {
        Ok(Some(command_id)) => {
            if let Err(err) = app_shell.route_command_id(&command_id) {
                eprintln!("error: {err}");
                std::process::exit(2);
            }
        }
        Ok(None) => {}
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(2);
        }
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

fn command_id_from_args(args: &[String]) -> Result<Option<String>, String> {
    let Some(idx) = args.iter().position(|a| a == "--command") else {
        return Ok(None);
    };

    let Some(value) = args.get(idx + 1) else {
        return Err("--command requires a command id value".to_string());
    };

    if value.starts_with('-') {
        return Err(format!(
            "--command requires a command id value, got option-like token '{value}'"
        ));
    }

    Ok(Some(value.clone()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::command_id_from_args;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn parses_valid_command_value() {
        let parsed =
            command_id_from_args(&args(&["ulgen-app", "--command", "workspace.next"])).unwrap();
        assert_eq!(parsed, Some("workspace.next".to_string()));
    }

    #[test]
    fn returns_none_when_flag_not_provided() {
        let parsed = command_id_from_args(&args(&["ulgen-app", "--new-window"])).unwrap();
        assert_eq!(parsed, None);
    }

    #[test]
    fn rejects_missing_command_value() {
        let err = command_id_from_args(&args(&["ulgen-app", "--command"])).unwrap_err();
        assert!(err.contains("--command requires a command id value"));
    }

    #[test]
    fn rejects_option_like_command_value() {
        let err =
            command_id_from_args(&args(&["ulgen-app", "--command", "--new-window"])).unwrap_err();
        assert!(err.contains("option-like token"));
    }
}
