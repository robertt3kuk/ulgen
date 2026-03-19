use std::path::PathBuf;

use ulgen_pty::{
    create_backend, create_contract_backend, create_default_backend,
    preferred_platform_backend_kind, BackendKind, CommandSpec, TerminalBackend, TerminalError,
    TerminalId, TerminalSize,
};

fn sample_spec() -> CommandSpec {
    CommandSpec {
        command: "echo".to_string(),
        args: vec!["hello".to_string()],
        cwd: PathBuf::from("/tmp"),
        env: vec![("ULGEN_TEST".to_string(), "1".to_string())],
    }
}

fn assert_backend_contract(mut backend: Box<dyn TerminalBackend>) {
    let terminal_id = backend.spawn(sample_spec()).expect("spawn should succeed");

    backend
        .write(&terminal_id, "hello")
        .expect("write should succeed");
    backend
        .write(&terminal_id, " \u{1F30D}")
        .expect("unicode write should succeed");

    assert_eq!(
        backend
            .output(&terminal_id)
            .expect("output should be readable"),
        "hello \u{1F30D}"
    );

    backend
        .resize(
            &terminal_id,
            TerminalSize {
                cols: 160,
                rows: 48,
            },
        )
        .expect("resize should succeed");

    assert_eq!(
        backend
            .wait_for_exit(&terminal_id)
            .expect("wait_for_exit should succeed"),
        None
    );

    backend.kill(&terminal_id).expect("kill should succeed");

    let exit = backend
        .wait_for_exit(&terminal_id)
        .expect("wait_for_exit should succeed after kill")
        .expect("killed terminal should have an exit status");
    assert!(exit.exit_code.is_some() || exit.signal.is_some());

    assert_eq!(
        backend.write(&terminal_id, "!"),
        Err(TerminalError::AlreadyExited)
    );

    assert_eq!(
        backend.output(&TerminalId("missing".to_string())),
        Err(TerminalError::NotFound)
    );
}

#[test]
fn memory_backend_matches_contract() {
    assert_backend_contract(create_backend(BackendKind::Memory));
}

#[test]
fn contract_backend_matches_contract() {
    assert_backend_contract(create_contract_backend());
}

#[test]
fn unix_backend_reports_unsupported_until_implemented() {
    let mut backend = create_backend(BackendKind::UnixPty);
    assert_eq!(
        backend.spawn(sample_spec()),
        Err(TerminalError::Unsupported {
            backend: "unix-pty",
            operation: "spawn"
        })
    );
}

#[test]
fn windows_backend_reports_unsupported_until_implemented() {
    let mut backend = create_backend(BackendKind::WindowsConpty);
    assert_eq!(
        backend.spawn(sample_spec()),
        Err(TerminalError::Unsupported {
            backend: "windows-conpty",
            operation: "spawn"
        })
    );
}

#[test]
fn preferred_platform_backend_reports_unsupported_until_implemented() {
    let preferred_backend = preferred_platform_backend_kind();
    let expected_backend_name = match preferred_backend {
        BackendKind::UnixPty => "unix-pty",
        BackendKind::WindowsConpty => "windows-conpty",
        BackendKind::Memory => "memory",
    };
    let mut backend = create_backend(preferred_backend);
    let error = backend
        .spawn(sample_spec())
        .expect_err("preferred platform backend is not implemented yet");

    match error {
        TerminalError::Unsupported { backend, operation } => {
            assert_eq!(operation, "spawn");
            assert_eq!(backend, expected_backend_name);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn default_runtime_backend_reports_unsupported_until_implemented() {
    let mut backend = create_default_backend();
    let error = backend
        .write(&TerminalId("missing".to_string()), "x")
        .expect_err("default runtime backend is not implemented yet");

    match error {
        TerminalError::Unsupported { backend, operation } => {
            assert_eq!(operation, "write");
            assert!(backend == "unix-pty" || backend == "windows-conpty");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
