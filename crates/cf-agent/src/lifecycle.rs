//! Finding, starting and stopping the agent.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context as _, Result};

use cf_ipc::{CallError, RequestBody, ResponseBody};

use crate::CONNECT_WAIT;

const AGENT: &str = "contextflow-agent.exe";
/// How long a freshly started agent gets to answer, and a stopping one to go.
const SETTLE: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(100);

/// A running agent, as it describes itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub version: String,
    /// The newest protocol it speaks.
    pub protocol: u32,
    /// The oldest protocol it still answers.
    pub min_protocol: u32,
}

impl AgentInfo {
    /// Whether this build's requests are ones the agent answers. If not, front
    /// ends work in-process until the agent is restarted.
    pub fn speaks_our_protocol(&self) -> bool {
        (self.min_protocol..=self.protocol).contains(&cf_ipc::PROTOCOL_VERSION)
    }
}

/// The agent on `pipe`, or `None` when none is running there. `hello` is
/// answered by every protocol version, so this also finds an older agent.
pub fn status(pipe: &str) -> Result<Option<AgentInfo>> {
    let hello = RequestBody::Hello {
        client: client_name(),
    };
    // Ask in the oldest dialect so an agent of any age answers.
    match cf_ipc::Client::connect(pipe, CONNECT_WAIT).and_then(|mut c| c.call_as(1, hello)) {
        Ok(ResponseBody::Hello {
            agent_version,
            protocol,
            min_protocol,
        }) => Ok(Some(AgentInfo {
            version: agent_version,
            protocol,
            min_protocol,
        })),
        Ok(other) => bail!("unexpected reply from the agent: {other:?}"),
        Err(CallError::Unavailable(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn client_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "contextflow".into())
}

/// Ask the agent on `pipe` to stop and wait until it has (it finishes
/// requests in flight first). `Ok(false)` when none was running.
pub fn stop(pipe: &str) -> Result<bool> {
    // `shutdown` exists since v1, so this also stops an older agent.
    let sent = cf_ipc::Client::connect(pipe, CONNECT_WAIT)
        .and_then(|mut c| c.call_as(1, RequestBody::Shutdown));
    match sent {
        Ok(ResponseBody::ShuttingDown) => {}
        Ok(other) => bail!("the agent would not stop: {other:?}"),
        Err(CallError::Unavailable(_)) => return Ok(false),
        Err(e) => return Err(e.into()),
    }
    let deadline = Instant::now() + SETTLE;
    while status(pipe)?.is_some() {
        if Instant::now() >= deadline {
            bail!("the agent is still finishing a request; try again in a moment");
        }
        std::thread::sleep(POLL);
    }
    Ok(true)
}

/// Where the agent program is: next to the running program, else on `PATH`.
pub fn agent_exe() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("could not locate the running program")?;
    let beside = exe.with_file_name(AGENT);
    if beside.is_file() {
        return Ok(beside);
    }
    if let Some(found) = std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(AGENT))
        .find(|p| p.is_file())
    {
        return Ok(found);
    }
    bail!(
        "{AGENT} was not found next to {} or on PATH. Install it with: cargo install --path crates/cf-agent",
        exe.display()
    )
}

/// What [`start`] found or did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Started {
    AlreadyRunning(AgentInfo),
    Started(AgentInfo),
}

/// Start the agent in the background unless one already answers on `pipe`,
/// and wait until it does.
pub fn start(pipe: &str) -> Result<Started> {
    if let Some(info) = status(pipe)? {
        return Ok(Started::AlreadyRunning(info));
    }
    let exe = agent_exe()?;
    spawn_detached(&exe).with_context(|| format!("starting {}", exe.display()))?;
    let deadline = Instant::now() + SETTLE;
    loop {
        if let Some(info) = status(pipe)? {
            return Ok(Started::Started(info));
        }
        if Instant::now() >= deadline {
            bail!("the agent was started but did not answer; see the log file for why");
        }
        std::thread::sleep(POLL);
    }
}

fn spawn_detached(exe: &std::path::Path) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(exe);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Its own process group and no console, so closing this terminal or
        // pressing Ctrl+C here never takes the agent down with it.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().map(drop)
}
