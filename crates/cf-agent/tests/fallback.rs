//! When a front end uses the agent and when it does the work in-process.

use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;

use cf_agent::{call_or_fallback, Service, Via};
use cf_ipc::{ErrorCode, RequestBody, ResponseBody, RestoreRequest, RestoreSummary};

fn restore(ws: &str) -> RequestBody {
    RequestBody::Restore(RestoreRequest {
        workspace: ws.into(),
        version: None,
    })
}

fn local_summary(ws: &str) -> ResponseBody {
    ResponseBody::Restored(RestoreSummary {
        workspace: format!("local:{ws}"),
        ..Default::default()
    })
}

#[test]
fn skipped_agent_runs_in_process() {
    let (reply, via) = call_or_fallback(None, restore("A"), |r| {
        assert_eq!(r, restore("A"));
        Ok(local_summary("A"))
    })
    .unwrap();
    assert_eq!(reply, local_summary("A"));
    assert!(matches!(via, Via::InProcess(_)));
}

#[cfg(windows)]
mod with_pipe {
    use super::*;
    use cf_ipc::{Handler, Server, StopHandle};

    fn unique_name() -> String {
        static N: AtomicU32 = AtomicU32::new(0);
        format!(
            r"\\.\pipe\ContextFlow.AgentTest.{}.{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        )
    }

    fn start(handler: impl Handler) -> (String, StopHandle, thread::JoinHandle<()>) {
        let name = unique_name();
        let server = Server::bind(&name).unwrap();
        let stop = server.stop_handle();
        let t = thread::spawn(move || server.run(handler).unwrap());
        (name, stop, t)
    }

    #[test]
    fn no_agent_running_falls_back() {
        let name = unique_name();
        let (reply, via) =
            call_or_fallback(Some(&name), restore("A"), |_| Ok(local_summary("A"))).unwrap();
        assert_eq!(reply, local_summary("A"));
        assert!(
            matches!(via, Via::InProcess(ref why) if why.contains("not running")),
            "{via:?}"
        );
    }

    #[test]
    fn running_agent_answers_and_local_never_runs() {
        let (name, stop, t) = start(|req| match req {
            RequestBody::Restore(r) => ResponseBody::Restored(RestoreSummary {
                workspace: format!("agent:{}", r.workspace),
                ..Default::default()
            }),
            _ => ResponseBody::Busy,
        });
        let (reply, via) = call_or_fallback(Some(&name), restore("A"), |_| {
            panic!("must not run in-process while the agent answers")
        })
        .unwrap();
        assert_eq!(via, Via::Agent);
        assert!(matches!(reply, ResponseBody::Restored(ref s) if s.workspace == "agent:A"));
        stop.stop();
        t.join().unwrap();
    }

    #[test]
    fn busy_agent_reply_is_passed_through() {
        let (name, stop, t) = start(|_| ResponseBody::Busy);
        let (reply, via) =
            call_or_fallback(Some(&name), restore("A"), |_| panic!("no fallback")).unwrap();
        assert_eq!((reply, via), (ResponseBody::Busy, Via::Agent));
        stop.stop();
        t.join().unwrap();
    }

    #[test]
    fn agent_refusing_before_acting_falls_back() {
        for code in [ErrorCode::UnsupportedVersion, ErrorCode::UnknownOperation] {
            let (name, stop, t) = start(move |_| ResponseBody::error(code, "old agent"));
            let (reply, via) =
                call_or_fallback(Some(&name), restore("A"), |_| Ok(local_summary("A"))).unwrap();
            assert_eq!(reply, local_summary("A"));
            assert!(matches!(via, Via::InProcess(_)), "{code:?}");
            stop.stop();
            t.join().unwrap();
        }
    }

    #[test]
    fn agent_failure_is_reported_not_retried() {
        let (name, stop, t) =
            start(|_| ResponseBody::error(ErrorCode::Failed, "no such workspace"));
        let (reply, via) =
            call_or_fallback(Some(&name), restore("A"), |_| panic!("no fallback")).unwrap();
        assert_eq!(via, Via::Agent);
        assert!(matches!(reply, ResponseBody::Error(ref e) if e.message == "no such workspace"));
        stop.stop();
        t.join().unwrap();
    }

    /// The agent dies after taking a restore: the client must not restore again.
    #[test]
    fn agent_dying_mid_request_is_an_error() {
        let (name, stop, t) = start(|_| -> ResponseBody { panic!("agent crashed") });
        let err = call_or_fallback(Some(&name), restore("A"), |_| panic!("no fallback"))
            .expect_err("a lost reply is an error");
        assert!(err.to_string().contains("did not answer"), "{err}");
        stop.stop();
        t.join().unwrap();
    }

    /// The real handler over the real pipe, against an empty database.
    #[test]
    fn service_over_the_pipe() {
        let dir = tempfile::tempdir().unwrap();
        let db = cf_storage::Database::open(&dir.path().join("cf.db")).unwrap();
        let service = Service::new(dir.path().join("config.toml"), db);
        let (name, stop, t) = start(service);

        let hello = cf_ipc::request(
            &name,
            RequestBody::Hello {
                client: "test".into(),
            },
            cf_agent::CONNECT_WAIT,
        )
        .unwrap();
        assert!(matches!(
            hello,
            ResponseBody::Hello {
                protocol: cf_ipc::PROTOCOL_VERSION,
                ..
            }
        ));

        // An unknown workspace fails in the agent and comes back as an error.
        let (reply, via) = call_or_fallback(Some(&name), restore("does-not-exist"), |_| {
            panic!("no fallback")
        })
        .unwrap();
        assert_eq!(via, Via::Agent);
        assert!(
            matches!(reply, ResponseBody::Error(ref e) if e.code == ErrorCode::Failed),
            "{reply:?}"
        );
        stop.stop();
        t.join().unwrap();
    }
}

/// The in-process path is the same handler.
#[test]
fn service_in_process_reports_unknown_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let db = cf_storage::Database::open(&dir.path().join("cf.db")).unwrap();
    let service = Service::new(dir.path().join("config.toml"), db);
    let reply = service.handle(restore("does-not-exist"));
    assert!(
        matches!(reply, ResponseBody::Error(ref e) if e.code == ErrorCode::Failed),
        "{reply:?}"
    );
    assert_eq!(
        service.handle(RequestBody::Shutdown),
        ResponseBody::ShuttingDown
    );
}
