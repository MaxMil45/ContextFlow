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
        RequestBody::Leave(_) => ResponseBody::Left(LeaveSummary::default()),
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
            .call(RequestBody::Restore(RestoreRequest::switch_to(ws)))
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
                    RequestBody::Restore(RestoreRequest::switch_to(format!("W{i}"))),
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

/// Stopping while a request is being answered: that request finishes and gets
/// its reply, the agent still says hello, new work is turned away in a way
/// that lets the client do it itself, and only then does the server exit.
#[test]
fn stop_waits_for_the_request_in_flight() {
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let (name, stop, t) = start(move |req| {
        if matches!(req, RequestBody::Restore(_)) {
            started_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
        }
        echo(req)
    });
    let slow = {
        let name = name.clone();
        thread::spawn(move || {
            request(
                &name,
                RequestBody::Restore(RestoreRequest::switch_to("slow")),
                WAIT,
            )
        })
    };
    started_rx.recv().unwrap();
    stop.stop();
    assert!(!t.is_finished(), "must not exit with a request in flight");

    // Still there, and says so.
    let hello = request(&name, RequestBody::Hello { client: "t".into() }, WAIT).unwrap();
    assert!(matches!(hello, ResponseBody::Hello { .. }));
    // New work is refused before anything happens: safe to do in-process.
    let refused = request(
        &name,
        RequestBody::Restore(RestoreRequest::switch_to("late")),
        WAIT,
    );
    assert!(
        matches!(refused, Err(CallError::Unavailable(_))),
        "{refused:?}"
    );

    release_tx.send(()).unwrap();
    let reply = slow.join().unwrap().unwrap();
    assert!(matches!(reply, ResponseBody::Restored(ref s) if s.workspace == "slow"));
    t.join().unwrap().unwrap();
    assert!(matches!(
        request(&name, RequestBody::Hello { client: "t".into() }, WAIT),
        Err(CallError::Unavailable(_))
    ));
}

/// A client that keeps its connection open and says nothing must not keep a
/// stopping agent alive.
#[test]
fn idle_connection_does_not_block_shutdown() {
    let (name, stop, t) = start(echo);
    let mut idle = Client::connect(&name, WAIT).unwrap();
    idle.call(RequestBody::Hello { client: "t".into() })
        .unwrap();
    stop.stop();
    t.join().unwrap().unwrap();
    drop(idle);
}
