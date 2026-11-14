//! `contextflow agent`: start, stop and inspect the background agent.

use anyhow::{Context as _, Result};

use cf_agent::lifecycle::{self, AgentInfo, Started};

use crate::AgentCmd;

fn describe(info: &AgentInfo) -> String {
    let mut s = format!(
        "v{} (protocol v{}..=v{})",
        info.version, info.min_protocol, info.protocol
    );
    if !info.speaks_our_protocol() {
        s.push_str(
            "\nIt does not speak this version's protocol, so commands run without it. \
             Restart it with: contextflow agent stop, then contextflow agent start",
        );
    }
    s
}

pub(crate) fn agent_cmd(cmd: &AgentCmd) -> Result<()> {
    let pipe = cf_ipc::default_pipe_name().context("could not work out the agent's pipe")?;
    match cmd {
        AgentCmd::Status => match lifecycle::status(&pipe)? {
            Some(info) => println!("The agent is running: {}", describe(&info)),
            None => {
                println!("The agent is not running. Commands do their work themselves.");
                println!("Start it with: contextflow agent start");
            }
        },
        AgentCmd::Start => match lifecycle::start(&pipe)? {
            Started::AlreadyRunning(info) => {
                println!("The agent is already running: {}", describe(&info))
            }
            Started::Started(info) => println!("Started the agent: {}", describe(&info)),
        },
        AgentCmd::Stop => {
            if lifecycle::stop(&pipe)? {
                println!("Stopped the agent.");
            } else {
                println!("The agent is not running.");
            }
        }
    }
    Ok(())
}
