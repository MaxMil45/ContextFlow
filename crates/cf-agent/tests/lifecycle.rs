//! Requests added in protocol v2, and finding and stopping an agent.

use std::time::Duration;

use cf_agent::{restore_args, Service};
use cf_config::Config;
use cf_ipc::{LeaveRequest, RequestBody, ResponseBody, RestoreRequest};

fn service(dir: &std::path::Path) -> Service {
    let db = cf_storage::Database::open(&dir.join("cf.db")).unwrap();
    Service::new(dir.join("config.toml"), db)
}

#[test]
fn restore_request_options_reach_the_restore() {
    let cfg = Config::default();

    let plain = restore_args(&cfg, &RestoreRequest::switch_to("A"));
    assert_eq!(plain.version, None);
    assert!(plain.only.is_empty());
    assert_eq!(plain.timeout, Duration::from_secs(30));
    assert!(!plain.skip_cleanup && !plain.dry_run);
    assert!(plain.save_before_leaving.is_some(), "a switch saves first");

    let custom = restore_args(
        &cfg,
        &RestoreRequest {
            version: Some(4),
            only: vec!["code".into()],
            timeout_secs: Some(5),
            no_cleanup: true,
            no_save: true,
            ..RestoreRequest::switch_to("A")
        },
    );
    assert_eq!(custom.version, Some(4));
    assert_eq!(custom.only, ["code"]);
    assert_eq!(custom.timeout, Duration::from_secs(5));
    assert!(custom.skip_cleanup);
    assert!(custom.save_before_leaving.is_none());
}

/// A client cannot make the agent wait forever, or not at all.
#[test]
fn timeout_is_clamped_by_the_agent() {
    let cfg = Config::default();
    let with = |secs| {
        restore_args(
            &cfg,
            &RestoreRequest {
                timeout_secs: Some(secs),
                ..RestoreRequest::switch_to("A")
            },
        )
        .timeout
    };
    assert_eq!(with(0), Duration::from_secs(1));
    assert_eq!(with(u64::MAX), Duration::from_secs(600));
}

#[test]
fn leave_with_no_active_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let reply = service(dir.path()).handle(RequestBody::Leave(LeaveRequest::default()));
    let ResponseBody::Left(out) = reply else {
        panic!("expected a leave summary, got {reply:?}");
    };
    assert_eq!(out.workspace, None);
    assert!(out.cleanup.is_none());
}

#[cfg(windows)]
mod with_pipe {
    use super::*;
    use cf_agent::lifecycle;
    use cf_ipc::{Server, PROTOCOL_VERSION};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::thread;

    fn unique_name() -> String {
        static N: AtomicU32 = AtomicU32::new(0);
        format!(
            r"\\.\pipe\ContextFlow.LifecycleTest.{}.{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        )
    }

    #[test]
    fn status_of_no_agent_is_none() {
        assert_eq!(lifecycle::status(&unique_name()).unwrap(), None);
        assert!(!lifecycle::stop(&unique_name()).unwrap());
    }

    #[test]
    fn status_then_stop() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name();
        let server = Server::bind(&name).unwrap();
        let svc = service(dir.path());
        let t = thread::spawn(move || server.run(svc).unwrap());

        let info = lifecycle::status(&name).unwrap().expect("agent answers");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(info.protocol, PROTOCOL_VERSION);
        assert!(info.speaks_our_protocol());

        // Leave over the pipe carries the summary back.
        let reply = cf_ipc::request(
            &name,
            RequestBody::Leave(LeaveRequest::default()),
            cf_agent::CONNECT_WAIT,
        )
        .unwrap();
        assert!(matches!(reply, ResponseBody::Left(ref s) if s.workspace.is_none()));

        assert!(lifecycle::stop(&name).unwrap(), "it was running");
        t.join().unwrap();
        assert_eq!(lifecycle::status(&name).unwrap(), None);
    }

    /// An agent from before this build's protocol is still found and can be
    /// stopped, and says it cannot take this build's requests.
    #[test]
    fn older_agent_is_found_and_stopped() {
        let name = unique_name();
        let server = Server::bind(&name).unwrap();
        let t = thread::spawn(move || {
            server
                .run(|req: RequestBody| match req {
                    RequestBody::Hello { .. } => ResponseBody::Hello {
                        agent_version: "0.0.1".into(),
                        protocol: 1,
                        min_protocol: 1,
                    },
                    RequestBody::Shutdown => ResponseBody::ShuttingDown,
                    _ => ResponseBody::Busy,
                })
                .unwrap()
        });
        let info = lifecycle::status(&name).unwrap().unwrap();
        assert_eq!(info.version, "0.0.1");
        assert!(!info.speaks_our_protocol());
        assert!(lifecycle::stop(&name).unwrap());
        t.join().unwrap();
    }
}
