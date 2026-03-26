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

    match workspace_name_from_args(&args) {
        Ok(Some(name)) => {
            if let Err(err) = app_shell.route_command(AppShellCommand::CreateWorkspace { name }) {
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

    match key_chord_from_args(&args) {
        Ok(Some(chord)) => {
            if let Err(err) = app_shell.route_key_chord(&chord) {
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

    let resolved_keymap = app_shell.resolve_active_keymap();

    println!("Ulgen app shell bootstrap is ready.");
    println!("{}", app_shell.startup_summary());
    println!("State path: {}", app_shell.state_path().display());
    println!(
        "Registered commands: {}",
        app_shell.command_registry().search("").len()
    );
    println!(
        "Active keymap: profile={:?}, bindings={}, rejected_overrides={}.",
        app_shell.state().settings.keymap_profile,
        resolved_keymap.bindings().len(),
        resolved_keymap.rejected_overrides().len()
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

fn key_chord_from_args(args: &[String]) -> Result<Option<String>, String> {
    let Some(idx) = args.iter().position(|a| a == "--key-chord") else {
        return Ok(None);
    };

    let Some(value) = args.get(idx + 1) else {
        return Err("--key-chord requires a key chord value".to_string());
    };

    if value.starts_with('-') {
        return Err(format!(
            "--key-chord requires a key chord value, got option-like token '{value}'"
        ));
    }

    Ok(Some(value.clone()))
}

fn workspace_name_from_args(args: &[String]) -> Result<Option<String>, String> {
    let Some(idx) = args.iter().position(|a| a == "--new-workspace") else {
        return Ok(None);
    };

    let Some(value) = args.get(idx + 1) else {
        return Err("--new-workspace requires a workspace name value".to_string());
    };

    if value.starts_with('-') {
        return Err(format!(
            "--new-workspace requires a workspace name value, got option-like token '{value}'"
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
    use super::{command_id_from_args, key_chord_from_args, workspace_name_from_args};

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

    #[test]
    fn parses_valid_key_chord_value() {
        let parsed = key_chord_from_args(&args(&["ulgen-app", "--key-chord", "ctrl+b c"])).unwrap();
        assert_eq!(parsed, Some("ctrl+b c".to_string()));
    }

    #[test]
    fn rejects_missing_key_chord_value() {
        let err = key_chord_from_args(&args(&["ulgen-app", "--key-chord"])).unwrap_err();
        assert!(err.contains("--key-chord requires a key chord value"));
    }

    #[test]
    fn rejects_option_like_key_chord_value() {
        let err =
            key_chord_from_args(&args(&["ulgen-app", "--key-chord", "--command"])).unwrap_err();
        assert!(err.contains("option-like token"));
    }

    #[test]
    fn parses_valid_workspace_name_value() {
        let parsed =
            workspace_name_from_args(&args(&["ulgen-app", "--new-workspace", "api"])).unwrap();
        assert_eq!(parsed, Some("api".to_string()));
    }

    #[test]
    fn rejects_missing_workspace_name_value() {
        let err = workspace_name_from_args(&args(&["ulgen-app", "--new-workspace"])).unwrap_err();
        assert!(err.contains("--new-workspace requires a workspace name value"));
    }

    #[test]
    fn rejects_option_like_workspace_name_value() {
        let err = workspace_name_from_args(&args(&[
            "ulgen-app",
            "--new-workspace",
            "--command",
            "window.new",
        ]))
        .unwrap_err();
        assert!(err.contains("option-like token"));
    }
}
