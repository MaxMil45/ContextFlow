//! Versioned request/response types and their wire framing.
//!
//! A frame is a 4-byte little-endian length followed by that many bytes of
//! UTF-8 JSON. See `docs/AGENT.md`.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

/// The protocol version this build speaks.
///
/// * v1: `hello`, `restore` (workspace, version), `shutdown`.
/// * v2: `restore` options an older agent must not ignore (`only`,
///   `timeout_secs`, `no_cleanup`, `no_save`), `leave`, and fuller summaries.
pub const PROTOCOL_VERSION: u32 = 2;
/// The oldest client version the agent still answers.
pub const MIN_SUPPORTED: u32 = 1;
/// Largest frame either side accepts.
pub const MAX_FRAME: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Protocol version of the sender.
    pub v: u32,
    /// Echoed in the response.
    pub id: u64,
    #[serde(flatten)]
    pub body: RequestBody,
}

impl Request {
    pub fn new(id: u64, body: RequestBody) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            body,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RequestBody {
    /// Ask who is there. Has no side effects.
    Hello {
        /// The client program's name, for the agent's log.
        client: String,
    },
    /// Switch to a workspace: save the one being left, then restore this one.
    Restore(RestoreRequest),
    /// Leave the active workspace without entering another.
    Leave(LeaveRequest),
    /// Stop accepting connections and exit once in-flight requests finish.
    Shutdown,
}

impl RequestBody {
    /// Whether the request changes anything. A client never falls back
    /// in-process after one of these has been sent.
    pub fn has_side_effects(&self) -> bool {
        !matches!(self, RequestBody::Hello { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RestoreRequest {
    /// Workspace name or id. Never a path.
    pub workspace: String,
    /// A snapshot version; `None` picks it the usual way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    /// Restore only these applications (lower-case program names, no `.exe`).
    /// Not a switch: nothing is saved or closed first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<String>,
    /// Seconds to wait for each application's window; `None` = the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Don't close any programs, whatever the cleanup rules say.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_cleanup: bool,
    /// Don't save the workspace being left first.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_save: bool,
}

impl RestoreRequest {
    /// A plain switch to `workspace`.
    pub fn switch_to(workspace: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
            version: None,
            only: Vec::new(),
            timeout_secs: None,
            no_cleanup: false,
            no_save: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LeaveRequest {
    /// A note saved with the workspace, shown first on return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    pub id: u64,
    #[serde(flatten)]
    pub body: ResponseBody,
}

impl Response {
    pub fn new(id: u64, body: ResponseBody) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            body,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResponseBody {
    Hello {
        agent_version: String,
        protocol: u32,
        min_protocol: u32,
    },
    Restored(RestoreSummary),
    Left(LeaveSummary),
    /// Another restore holds the lock; this one did nothing.
    Busy,
    ShuttingDown,
    Error(ErrorReply),
}

impl ResponseBody {
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        ResponseBody::Error(ErrorReply {
            code,
            message: message.into(),
        })
    }
}

/// What a restore did, as much as a front end needs to tell the user.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RestoreSummary {
    /// The workspace's display name.
    pub workspace: String,
    /// Things that need the user's attention; empty when all went well.
    #[serde(default)]
    pub problems: Vec<Problem>,
    /// Set when a layout saved for the current screens was chosen.
    #[serde(default)]
    pub layout_note: Option<String>,
    /// The snapshot version that was restored.
    #[serde(default)]
    pub version: u32,
    /// The overall result: `success`, `partial_success`, `failed`, ...
    #[serde(default)]
    pub overall: String,
    /// One line per application, in the order they were restored.
    #[serde(default)]
    pub steps: Vec<Step>,
    /// What happened to the workspace that was left, if there was one.
    #[serde(default)]
    pub left: Option<Saved>,
    /// Cleanups that ran first.
    #[serde(default)]
    pub cleanups: Vec<Cleanup>,
    /// The note saved with the snapshot.
    #[serde(default)]
    pub note: Option<String>,
    /// Progress lines, when the work ran in the agent (in-process they are
    /// shown live instead).
    #[serde(default)]
    pub log: Vec<String>,
}

/// One application's restore.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub application: String,
    /// `success`, `partial_success`, `failed`, `skipped`,
    /// `requires_user_intervention`.
    pub status: String,
    pub message: String,
}

/// What happened to a workspace when it was left.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub workspace: String,
    /// The version it was saved as, if it was.
    #[serde(default)]
    pub version: Option<u32>,
    /// Why it was not saved, in words for the user.
    #[serde(default)]
    pub not_saved: Option<String>,
}

/// One cleanup: the programs a workspace closes on entering or leaving.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cleanup {
    pub phase: CleanupPhase,
    pub workspace: String,
    #[serde(default)]
    pub results: Vec<CleanupResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupPhase {
    Leaving,
    Entering,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CleanupResult {
    pub process: String,
    pub outcome: CleanupOutcome,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupOutcome {
    /// The program was closed or ended.
    Closed,
    /// Nothing to do: not running, or kept for the workspace being entered.
    Skipped,
    /// A protected program; never closed.
    Refused,
    /// Still running or could not be closed: the user should look.
    NeedsAttention,
    #[serde(other)]
    Other,
}

/// What leaving the active workspace did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LeaveSummary {
    /// The workspace that was left; `None` when none was active.
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub saved: Saved,
    /// `None` when it has no "when leaving" rules.
    #[serde(default)]
    pub cleanup: Option<Cleanup>,
    #[serde(default)]
    pub log: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    /// The application or program concerned.
    pub subject: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorReply {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request's `v` is outside what the agent speaks. Nothing happened.
    UnsupportedVersion,
    /// The agent does not know the `op`. Nothing happened.
    UnknownOperation,
    /// The frame was not a valid request.
    BadRequest,
    /// The client failed the access check. Nothing happened.
    Denied,
    /// The agent is stopping and takes no new work. Nothing happened.
    ShuttingDown,
    /// The operation ran and failed.
    Failed,
    /// Any code a newer agent may add.
    #[serde(other)]
    Other,
}

impl ErrorCode {
    /// Errors that prove the agent did nothing, so the client may safely do
    /// the work in-process instead.
    pub fn safe_to_fall_back(self) -> bool {
        matches!(
            self,
            ErrorCode::UnsupportedVersion
                | ErrorCode::UnknownOperation
                | ErrorCode::Denied
                | ErrorCode::ShuttingDown
        )
    }
}

/// Server side: turn a raw frame into a request, or into the error reply to
/// send back. The version is checked before the operation is parsed, so an
/// old agent answers a newer client with `unsupported_version` rather than a
/// parse error.
pub fn decode_request(frame: &[u8]) -> Result<Request, Box<Response>> {
    decode(frame).map_err(|(id, code, message)| {
        Box::new(Response::new(id, ResponseBody::error(code, message)))
    })
}

/// The request, or the id, code and message of the refusal.
fn decode(frame: &[u8]) -> Result<Request, (u64, ErrorCode, String)> {
    let value: serde_json::Value = serde_json::from_slice(frame)
        .map_err(|e| (0, ErrorCode::BadRequest, format!("not JSON: {e}")))?;
    let id = value.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
    let Some(v) = value.get("v").and_then(|v| v.as_u64()) else {
        return Err((id, ErrorCode::BadRequest, "missing protocol version".into()));
    };
    if v < u64::from(MIN_SUPPORTED) || v > u64::from(PROTOCOL_VERSION) {
        return Err((
            id,
            ErrorCode::UnsupportedVersion,
            format!(
                "protocol v{v} not supported; this agent speaks v{MIN_SUPPORTED}..=v{PROTOCOL_VERSION}"
            ),
        ));
    }
    let op = value.get("op").and_then(|o| o.as_str()).map(str::to_owned);
    serde_json::from_value::<Request>(value).map_err(|e| {
        let known = matches!(
            op.as_deref(),
            Some("hello" | "restore" | "leave" | "shutdown")
        );
        if known {
            (id, ErrorCode::BadRequest, format!("invalid request: {e}"))
        } else {
            (
                id,
                ErrorCode::UnknownOperation,
                format!("unknown operation {:?}", op.unwrap_or_default()),
            )
        }
    })
}

/// Write one frame.
pub fn write_frame(w: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    if payload.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame too large",
        ));
    }
    let len = payload.len() as u32;
    // One write so a message-oriented reader never sees a split header.
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()
}

/// Read one frame. `Ok(None)` on a clean end of stream before a header.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut header = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut header[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            // A closed pipe at a frame boundary is a clean end.
            Err(e) if got == 0 && is_closed(&e) => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    let len = u32::from_le_bytes(header) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds the {MAX_FRAME}-byte limit"),
        ));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok(Some(payload))
}

/// `ERROR_BROKEN_PIPE` / `ERROR_PIPE_NOT_CONNECTED`: the other end closed.
pub(crate) fn is_closed(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::BrokenPipe || matches!(e.raw_os_error(), Some(109 | 233))
}

pub fn write_message<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(msg).map_err(io::Error::other)?;
    write_frame(w, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(req: &Request) -> Request {
        let mut buf = Vec::new();
        write_message(&mut buf, req).unwrap();
        let frame = read_frame(&mut buf.as_slice()).unwrap().unwrap();
        decode_request(&frame).unwrap()
    }

    #[test]
    fn requests_roundtrip() {
        for body in [
            RequestBody::Hello {
                client: "test".into(),
            },
            RequestBody::Restore(RestoreRequest {
                version: Some(3),
                only: vec!["code".into()],
                timeout_secs: Some(10),
                no_cleanup: true,
                no_save: true,
                ..RestoreRequest::switch_to("Client A")
            }),
            RequestBody::Leave(LeaveRequest {
                note: Some("back at 3".into()),
            }),
            RequestBody::Shutdown,
        ] {
            let req = Request::new(9, body);
            assert_eq!(roundtrip(&req), req);
        }
    }

    #[test]
    fn wire_format_is_stable() {
        let req = Request::new(
            7,
            RequestBody::Restore(RestoreRequest::switch_to("Client A")),
        );
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            r#"{"v":2,"id":7,"op":"restore","workspace":"Client A"}"#
        );
        let resp = Response::new(7, ResponseBody::Busy);
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            r#"{"v":2,"id":7,"status":"busy"}"#
        );
        let err = Response::new(1, ResponseBody::error(ErrorCode::UnknownOperation, "nope"));
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"v":2,"id":1,"status":"error","code":"unknown_operation","message":"nope"}"#
        );
    }

    #[test]
    fn responses_roundtrip() {
        for body in [
            ResponseBody::Hello {
                agent_version: "0.1.0".into(),
                protocol: 1,
                min_protocol: 1,
            },
            ResponseBody::Restored(RestoreSummary {
                workspace: "A".into(),
                problems: vec![Problem {
                    subject: "code".into(),
                    message: "did not start".into(),
                }],
                layout_note: Some("v3".into()),
                version: 3,
                overall: "partial_success".into(),
                steps: vec![Step {
                    application: "code".into(),
                    status: "failed".into(),
                    message: "did not start".into(),
                }],
                left: Some(Saved {
                    workspace: "B".into(),
                    version: Some(4),
                    not_saved: None,
                }),
                cleanups: vec![Cleanup {
                    phase: CleanupPhase::Leaving,
                    workspace: "B".into(),
                    results: vec![CleanupResult {
                        process: "slack".into(),
                        outcome: CleanupOutcome::NeedsAttention,
                        message: "still running".into(),
                    }],
                }],
                note: Some("n".into()),
                log: vec!["saving".into()],
            }),
            ResponseBody::Left(LeaveSummary::default()),
            ResponseBody::Busy,
            ResponseBody::ShuttingDown,
            ResponseBody::error(ErrorCode::Failed, "x"),
        ] {
            let resp = Response::new(2, body);
            let json = serde_json::to_vec(&resp).unwrap();
            assert_eq!(serde_json::from_slice::<Response>(&json).unwrap(), resp);
        }
    }

    #[test]
    fn newer_fields_are_ignored() {
        let json = br#"{"v":1,"id":3,"op":"restore","workspace":"A","stage":"fast"}"#;
        let req = decode_request(json).unwrap();
        assert!(matches!(req.body, RequestBody::Restore(ref r) if r.workspace == "A"));
        let reply: Response = serde_json::from_slice(
            br#"{"v":1,"id":3,"status":"restored","workspace":"A","took_ms":5}"#,
        )
        .unwrap();
        assert!(matches!(reply.body, ResponseBody::Restored(ref s) if s.problems.is_empty()));
        let code: ErrorCode = serde_json::from_str(r#""rate_limited""#).unwrap();
        assert_eq!(code, ErrorCode::Other);
    }

    fn rejected(json: &[u8]) -> (u64, ErrorCode) {
        let resp = *decode_request(json).unwrap_err();
        match resp.body {
            ResponseBody::Error(e) => (resp.id, e.code),
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn version_is_checked_before_the_operation() {
        // A future client with an op this agent has never heard of.
        let (id, code) = rejected(br#"{"v":99,"id":5,"op":"teleport"}"#);
        assert_eq!((id, code), (5, ErrorCode::UnsupportedVersion));
        assert!(code.safe_to_fall_back());
        assert_eq!(
            rejected(br#"{"v":0,"id":1,"op":"shutdown"}"#).1,
            ErrorCode::UnsupportedVersion
        );
    }

    #[test]
    fn unknown_and_malformed_requests() {
        let (id, code) = rejected(br#"{"v":1,"id":4,"op":"teleport"}"#);
        assert_eq!((id, code), (4, ErrorCode::UnknownOperation));
        assert!(code.safe_to_fall_back());
        // A known op with a missing field is a client bug, not a fallback case.
        let code = rejected(br#"{"v":1,"id":4,"op":"restore"}"#).1;
        assert_eq!(code, ErrorCode::BadRequest);
        assert!(!code.safe_to_fall_back());
        assert_eq!(rejected(b"not json").1, ErrorCode::BadRequest);
        assert_eq!(
            rejected(br#"{"id":1,"op":"shutdown"}"#).1,
            ErrorCode::BadRequest
        );
        assert!(!ErrorCode::Failed.safe_to_fall_back());
        assert!(ErrorCode::Denied.safe_to_fall_back());
        assert!(ErrorCode::ShuttingDown.safe_to_fall_back());
    }

    #[test]
    fn framing_limits() {
        // Clean end of stream.
        assert!(read_frame(&mut &b""[..]).unwrap().is_none());
        // Truncated header and truncated payload are errors.
        assert!(read_frame(&mut &b"\x05\x00"[..]).is_err());
        assert!(read_frame(&mut &b"\x05\x00\x00\x00ab"[..]).is_err());
        // An oversized length is refused without allocating it.
        let huge = ((MAX_FRAME + 1) as u32).to_le_bytes();
        let err = read_frame(&mut &huge[..]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(write_frame(&mut Vec::new(), &vec![0u8; MAX_FRAME + 1]).is_err());
        // Two frames back to back.
        let mut buf = Vec::new();
        write_frame(&mut buf, b"one").unwrap();
        write_frame(&mut buf, b"two").unwrap();
        let mut r = buf.as_slice();
        assert_eq!(read_frame(&mut r).unwrap().unwrap(), b"one");
        assert_eq!(read_frame(&mut r).unwrap().unwrap(), b"two");
        assert!(read_frame(&mut r).unwrap().is_none());
    }

    #[test]
    fn only_hello_is_side_effect_free() {
        assert!(!RequestBody::Hello { client: "x".into() }.has_side_effects());
        assert!(RequestBody::Shutdown.has_side_effects());
    }
}

#[cfg(test)]
mod v2_tests {
    use super::*;

    /// A v1 client's restore is still a valid request, with the defaults.
    #[test]
    fn v1_restore_is_still_understood() {
        let req = decode_request(br#"{"v":1,"id":1,"op":"restore","workspace":"A"}"#).unwrap();
        assert_eq!(
            req.body,
            RequestBody::Restore(RestoreRequest::switch_to("A"))
        );
    }

    /// A v1 agent's short summary still reads, with the new fields empty.
    #[test]
    fn v1_summary_is_still_understood() {
        let reply: Response = serde_json::from_slice(
            br#"{"v":1,"id":1,"status":"restored","workspace":"A","problems":[],"layout_note":null}"#,
        )
        .unwrap();
        let ResponseBody::Restored(s) = reply.body else {
            panic!("not a summary");
        };
        assert!(s.steps.is_empty() && s.left.is_none() && s.overall.is_empty());
    }

    #[test]
    fn leave_wire_format() {
        let req = Request::new(3, RequestBody::Leave(LeaveRequest::default()));
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            r#"{"v":2,"id":3,"op":"leave"}"#
        );
        assert!(RequestBody::Leave(LeaveRequest::default()).has_side_effects());
        let outcome: CleanupOutcome = serde_json::from_str(r#""vaporised""#).unwrap();
        assert_eq!(outcome, CleanupOutcome::Other);
    }
}
