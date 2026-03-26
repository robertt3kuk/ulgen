mod app_shell;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use app_shell::{default_state_path, AppShell, AppShellCommand};
use ulgen_domain::BlockStatus;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--smoke") {
        run_smoke();
        return;
    }

    let state_path = default_state_path();
    let mut app_shell = match AppShell::bootstrap(state_path.clone()) {
        Ok(shell) => shell,
        Err(err) => {
            eprintln!(
                "error: app shell bootstrap failed for {}: {err}",
                state_path.display()
            );
            std::process::exit(2);
        }
    };

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

    let run_block_id = match apply_run_command_args(&mut app_shell, &args) {
        Ok(block_id) => block_id,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(2);
        }
    };

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
    println!("Tracked blocks: {}", app_shell.blocks().len());
    if let Some(block_id) = run_block_id {
        println!("Recorded block: {block_id}");
    }
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

fn flag_value_from_args(
    args: &[String],
    flag: &str,
    value_label: &str,
    allow_option_like_with_equals: bool,
) -> Result<Option<String>, String> {
    let inline_prefix = format!("{flag}=");

    for (idx, token) in args.iter().enumerate() {
        if token == flag {
            let Some(value) = args.get(idx + 1) else {
                return Err(format!("{flag} requires a {value_label} value"));
            };

            if value.starts_with('-') {
                if allow_option_like_with_equals {
                    return Err(format!(
                        "{flag} values that start with '-' must use {flag}=<value>"
                    ));
                }
                return Err(format!(
                    "{flag} requires a {value_label} value, got option-like token '{value}'"
                ));
            }

            return Ok(Some(value.clone()));
        }

        if let Some(value) = token.strip_prefix(&inline_prefix) {
            if value.is_empty() {
                return Err(format!("{flag} requires a {value_label} value"));
            }
            return Ok(Some(value.to_string()));
        }
    }

    Ok(None)
}

fn run_command_from_args(args: &[String]) -> Result<Option<String>, String> {
    flag_value_from_args(args, "--run-command", "command", true)
}

fn run_output_from_args(args: &[String]) -> Result<Option<String>, String> {
    flag_value_from_args(args, "--run-output", "output", true)
}

fn run_status_from_args(args: &[String]) -> Result<Option<BlockStatus>, String> {
    let Some(value) = flag_value_from_args(args, "--run-status", "status", false)? else {
        return Ok(None);
    };

    let normalized = value.trim().to_ascii_lowercase();
    let status = match normalized.as_str() {
        "succeeded" | "success" => BlockStatus::Succeeded,
        "failed" | "failure" => BlockStatus::Failed,
        "cancelled" | "canceled" => BlockStatus::Cancelled,
        "running" => {
            return Err(
                "--run-status only accepts terminal statuses (succeeded|failed|cancelled)"
                    .to_string(),
            )
        }
        _ => {
            return Err(format!(
                "--run-status expects one of succeeded|failed|cancelled, got '{value}'"
            ))
        }
    };

    Ok(Some(status))
}

fn apply_run_command_args(
    app_shell: &mut AppShell,
    args: &[String],
) -> Result<Option<String>, String> {
    let run_command = run_command_from_args(args)?;
    let run_output = run_output_from_args(args)?;
    let run_status = run_status_from_args(args)?;

    if run_command.is_none() && (run_output.is_some() || run_status.is_some()) {
        return Err("--run-output and --run-status require --run-command".to_string());
    }

    let Some(input) = run_command else {
        return Ok(None);
    };

    let block_id = app_shell.start_command_block_for_active_session(input)?;
    if let Some(output) = run_output {
        app_shell.append_block_output(&block_id, output)?;
    }
    if let Some(status) = run_status {
        app_shell.finish_block(&block_id, status)?;
    }

    Ok(Some(block_id))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        apply_run_command_args, command_id_from_args, key_chord_from_args, run_command_from_args,
        run_output_from_args, run_status_from_args, workspace_name_from_args, AppShell,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::sync::atomic::{AtomicU64, Ordering};
    use ulgen_domain::BlockStatus;

    static TEST_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    fn temp_state_path() -> PathBuf {
        let seq = TEST_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ulgen-main-test-{}-{}-{}.json",
            process::id(),
            super::now_ms(),
            seq
        ))
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

    #[test]
    fn parses_valid_run_command_value() {
        let parsed =
            run_command_from_args(&args(&["ulgen-app", "--run-command", "cargo test"])).unwrap();
        assert_eq!(parsed, Some("cargo test".to_string()));
    }

    #[test]
    fn rejects_missing_run_command_value() {
        let err = run_command_from_args(&args(&["ulgen-app", "--run-command"])).unwrap_err();
        assert!(err.contains("--run-command requires a command value"));
    }

    #[test]
    fn rejects_option_like_run_command_value() {
        let err = run_command_from_args(&args(&["ulgen-app", "--run-command", "--new-window"]))
            .unwrap_err();
        assert!(err.contains("must use --run-command=<value>"));
    }

    #[test]
    fn accepts_option_like_run_command_value_with_equals_form() {
        let parsed =
            run_command_from_args(&args(&["ulgen-app", "--run-command=--new-window"])).unwrap();
        assert_eq!(parsed, Some("--new-window".to_string()));
    }

    #[test]
    fn parses_valid_run_output_value() {
        let parsed = run_output_from_args(&args(&["ulgen-app", "--run-output", "line 1"])).unwrap();
        assert_eq!(parsed, Some("line 1".to_string()));
    }

    #[test]
    fn accepts_option_like_run_output_value_with_equals_form() {
        let parsed = run_output_from_args(&args(&["ulgen-app", "--run-output=--raw"])).unwrap();
        assert_eq!(parsed, Some("--raw".to_string()));
    }

    #[test]
    fn parses_valid_run_status_value() {
        let parsed =
            run_status_from_args(&args(&["ulgen-app", "--run-status", "Succeeded"])).unwrap();
        assert_eq!(parsed, Some(BlockStatus::Succeeded));
    }

    #[test]
    fn rejects_invalid_run_status_value() {
        let err =
            run_status_from_args(&args(&["ulgen-app", "--run-status", "pending"])).unwrap_err();
        assert!(err.contains("expects one of"));
    }

    #[test]
    fn run_command_flow_records_and_persists_block() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let block_id = apply_run_command_args(
            &mut shell,
            &args(&[
                "ulgen-app",
                "--run-command",
                "echo hi",
                "--run-output",
                "hi",
                "--run-status",
                "succeeded",
            ]),
        )
        .unwrap()
        .unwrap();
        let block = shell.block_by_id(&block_id).unwrap();
        assert_eq!(block.input, "echo hi");
        assert_eq!(block.output_chunks.len(), 1);
        assert_eq!(block.output_chunks[0].text, "hi");
        assert_eq!(block.status, BlockStatus::Succeeded);

        shell.save().unwrap();
        let restored = AppShell::bootstrap(path.clone()).unwrap();
        let restored_block = restored.block_by_id(&block_id).unwrap();
        assert_eq!(restored_block.input, "echo hi");
        assert_eq!(restored_block.status, BlockStatus::Succeeded);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn run_command_flow_requires_run_command_for_output_or_status() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let err = apply_run_command_args(&mut shell, &args(&["ulgen-app", "--run-output", "line"]))
            .unwrap_err();
        assert!(err.contains("require --run-command"));

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn run_command_flow_accepts_dash_prefixed_values_with_equals_form() {
        let path = temp_state_path();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let mut shell = AppShell::bootstrap(path.clone()).unwrap();
        let block_id = apply_run_command_args(
            &mut shell,
            &args(&[
                "ulgen-app",
                "--run-command=--new-window",
                "--run-output=--raw",
                "--run-status",
                "failed",
            ]),
        )
        .unwrap()
        .unwrap();

        let block = shell.block_by_id(&block_id).unwrap();
        assert_eq!(block.input, "--new-window");
        assert_eq!(block.output_chunks[0].text, "--raw");
        assert_eq!(block.status, BlockStatus::Failed);

        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }
}
