//! The named pipe end to end: framing, versioning, the access check and stop.
#![cfg(windows)]

use std::io::{Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use cf_ipc::*;

fn unique_name() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        r"\\.\pipe\ContextFlow.Test.{}.{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    )
}

fn echo(req: RequestBody) -> ResponseBody {
    match req {
        RequestBody::Hello { client } => ResponseBody::Hello {
            agent_version: client,
            protocol: PROTOCOL_VERSION,
            min_protocol: MIN_SUPPORTED,
        },
        RequestBody::Restore(r) => ResponseBody::Restored(RestoreSummary {
            workspace: r.workspace,
            ..Default::default()
        }),
        RequestBody::Shutdown => ResponseBody::ShuttingDown,
    }
}

/// Start a server on a fresh pipe; returns its name, stop handle and thread.
fn start(handler: impl Handler) -> (String, StopHandle, thread::JoinHandle<std::io::Result<()>>) {
    let name = unique_name();
    let server = Server::bind(&name).unwrap();
    let stop = server.stop_handle();
    let t = thread::spawn(move || server.run(handler));
    (name, stop, t)
}

const WAIT: Duration = Duration::from_secs(2);

#[test]
fn same_user_client_is_served_in_order() {
    let (name, stop, t) = start(echo);
    let mut client = Client::connect(&name, WAIT).unwrap();
    for ws in ["A", "B", "C"] {
        let reply = client
            .call(RequestBody::Restore(RestoreRequest {
                workspace: ws.into(),
                version: None,
            }))
            .unwrap();
        assert!(matches!(reply, ResponseBody::Restored(ref s) if s.workspace == ws));
    }
    let hello = client
        .call(RequestBody::Hello {
            client: "test".into(),
        })
        .unwrap();
    assert!(matches!(
        hello,
        ResponseBody::Hello {
            protocol: PROTOCOL_VERSION,
            ..
        }
    ));
    drop(client);
    stop.stop();
    t.join().unwrap().unwrap();
}

#[test]
fn several_clients_at_once() {
    let (name, stop, t) = start(|req| {
        thread::sleep(Duration::from_millis(100));
        echo(req)
    });
    let clients: Vec<_> = (0..4)
        .map(|i| {
            let name = name.clone();
            thread::spawn(move || {
                request(
                    &name,
                    RequestBody::Restore(RestoreRequest {
                        workspace: format!("W{i}"),
                        version: None,
                    }),
                    WAIT,
                )
                .unwrap()
            })
        })
        .collect();
    for (i, c) in clients.into_iter().enumerate() {
        let reply = c.join().unwrap();
        assert!(matches!(reply, ResponseBody::Restored(ref s) if s.workspace == format!("W{i}")));
    }
    stop.stop();
    t.join().unwrap().unwrap();
}

#[test]
fn only_one_server_per_name() {
    let (name, stop, t) = start(echo);
    let err = Server::bind(&name).err().expect("second bind must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
    stop.stop();
    t.join().unwrap().unwrap();
}

#[test]
fn no_server_is_unavailable() {
    let err = request(
        &unique_name(),
        RequestBody::Hello { client: "t".into() },
        WAIT,
    )
    .unwrap_err();
    assert!(matches!(err, CallError::Unavailable(_)), "{err}");
}

/// A raw client speaking a future protocol version gets a clean refusal.
#[test]
fn future_version_is_refused_before_acting() {
    let acted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = acted.clone();
    let (name, stop, t) = start(move |req| {
        seen.store(true, Ordering::SeqCst);
        echo(req)
    });
    let mut pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&name)
        .unwrap();
    write_frame(&mut pipe, br#"{"v":99,"id":42,"op":"teleport"}"#).unwrap();
    let reply: Response = serde_json::from_slice(&read_frame(&mut pipe).unwrap().unwrap()).unwrap();
    assert_eq!(reply.id, 42);
    assert!(
        matches!(reply.body, ResponseBody::Error(ref e) if e.code == ErrorCode::UnsupportedVersion)
    );
    assert!(!acted.load(Ordering::SeqCst));
    drop(pipe);
    stop.stop();
    t.join().unwrap().unwrap();
}

/// A garbage length prefix closes the connection without killing the server.
#[test]
fn oversized_frame_drops_only_that_connection() {
    let (name, stop, t) = start(echo);
    let mut pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&name)
        .unwrap();
    pipe.write_all(&u32::MAX.to_le_bytes()).unwrap();
    let mut buf = [0u8; 1];
    assert!(matches!(pipe.read(&mut buf), Ok(0) | Err(_)));
    // The server still answers others.
    assert!(request(&name, RequestBody::Hello { client: "t".into() }, WAIT).is_ok());
    stop.stop();
    t.join().unwrap().unwrap();
}

#[test]
fn shutdown_request_stops_the_server() {
    let (name, _stop, t) = start(echo);
    let reply = request(&name, RequestBody::Shutdown, WAIT).unwrap();
    assert_eq!(reply, ResponseBody::ShuttingDown);
    t.join().unwrap().unwrap();
    // The pipe is gone.
    assert!(matches!(
        request(&name, RequestBody::Hello { client: "t".into() }, WAIT),
        Err(CallError::Unavailable(_))
    ));
}

#[test]
fn default_pipe_name_is_ours() {
    let me = Identity::current().unwrap();
    assert!(me.sid.starts_with("S-1-"));
    assert_eq!(default_pipe_name().unwrap(), pipe_name_for(&me));
}
