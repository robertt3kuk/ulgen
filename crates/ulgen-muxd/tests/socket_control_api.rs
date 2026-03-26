#[cfg(unix)]
mod unix_tests {
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::net::Shutdown;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::process;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use ulgen_muxd::{
        serve_unix_socket_once, MuxState, RpcResponseEnvelope, SocketApiError,
        DEFAULT_MAX_REQUEST_BYTES,
    };

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_socket_path(label: &str) -> PathBuf {
        let seq = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        std::env::temp_dir().join(format!("u-{label}-{}-{epoch_ms}-{seq}.sock", process::id()))
    }

    fn connect_with_retry(path: &PathBuf) -> UnixStream {
        for _ in 0..80 {
            if let Ok(stream) = UnixStream::connect(path) {
                return stream;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("failed to connect to unix socket {}", path.display());
    }

    #[test]
    fn unix_socket_roundtrip_controls_mux_state() {
        let socket_path = temp_socket_path("roundtrip");
        let server_path = socket_path.clone();

        let server = thread::spawn(move || {
            let mut mux = MuxState::new();
            serve_unix_socket_once(&mut mux, &server_path, DEFAULT_MAX_REQUEST_BYTES).unwrap();
            mux
        });

        let mut stream = connect_with_retry(&socket_path);
        let mut reader = BufReader::new(stream.try_clone().unwrap());

        stream
            .write_all(
                br#"{"id":"req-1","v":"v0","method":"workspace.create","params":{"name":"api"}}
{"id":"req-2","v":"v0","method":"pane.split","params":{"direction":"right"}}
{"id":"req-3","v":"v0","method":"pane.focus","params":{"pane_id":"pane-8"}}
{"id":"req-4","v":"v0","method":"workspace.list","params":{}}
"#,
            )
            .unwrap();
        stream.flush().unwrap();
        stream.shutdown(Shutdown::Write).unwrap();

        let mut responses = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            responses.push(serde_json::from_str::<RpcResponseEnvelope>(line.trim()).unwrap());
        }

        let mux = server.join().unwrap();
        assert_eq!(responses.len(), 4);
        assert!(responses[0].ok);
        assert!(responses[1].ok);
        assert!(responses[2].ok);
        assert!(responses[3].ok);
        assert_eq!(
            responses[2].result.as_ref().unwrap()["pane_id"].as_str(),
            Some("pane-8")
        );
        assert_eq!(
            responses[3].result.as_ref().unwrap()["workspaces"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(mux.workspaces.len(), 2);
        assert_eq!(mux.workspaces[mux.active_workspace].tabs[0].active_pane, 0);
        assert!(!socket_path.exists());
    }

    #[test]
    fn unix_socket_rejects_existing_non_socket_path() {
        let socket_path = temp_socket_path("non-socket");
        fs::write(&socket_path, b"not-a-socket").unwrap();

        let mut mux = MuxState::new();
        let result = serve_unix_socket_once(&mut mux, &socket_path, DEFAULT_MAX_REQUEST_BYTES);
        assert!(matches!(&result, Err(SocketApiError::Io(_))));
        let message = match result {
            Err(error) => error.to_string(),
            Ok(()) => String::new(),
        };
        assert!(message.contains("not a unix socket"));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn unix_socket_rejects_existing_active_socket_path() {
        let socket_path = temp_socket_path("active-socket");
        let listener = UnixListener::bind(&socket_path).unwrap();

        let mut mux = MuxState::new();
        let result = serve_unix_socket_once(&mut mux, &socket_path, DEFAULT_MAX_REQUEST_BYTES);
        assert!(matches!(&result, Err(SocketApiError::Io(_))));
        let message = match result {
            Err(error) => error.to_string(),
            Ok(()) => String::new(),
        };
        assert!(message.contains("already active"));

        drop(listener);
        let _ = fs::remove_file(socket_path);
    }
}
