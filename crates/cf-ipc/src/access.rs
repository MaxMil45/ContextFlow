//! The access rule: the agent and its clients must be the same user in the
//! same login session. The pipe's DACL enforces the user part in the kernel;
//! [`check_peer`] is the check both ends make on top of it.

use thiserror::Error;

/// A process's user and Windows login session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The user's SID, e.g. `S-1-5-21-…-1001`.
    pub sid: String,
    /// The Windows session id (console, RDP, …).
    pub session: u32,
}

impl Identity {
    /// This process's identity.
    pub fn current() -> std::io::Result<Self> {
        #[cfg(windows)]
        {
            crate::pipe::current_identity()
        }
        #[cfg(not(windows))]
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "the agent channel needs Windows",
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AccessDenied {
    #[error("the other end runs as a different user ({0})")]
    OtherUser(String),
    #[error("the other end is in login session {theirs}, not {ours}")]
    OtherSession { ours: u32, theirs: u32 },
    #[error("the agent runs as administrator and the client does not")]
    LessPrivileged,
}

/// Allow a peer only if it is the same user in the same login session.
pub fn check_peer(own: &Identity, peer: &Identity) -> Result<(), AccessDenied> {
    // SID strings are canonical (`S-1-…`, decimal), so plain equality is exact.
    if own.sid != peer.sid {
        return Err(AccessDenied::OtherUser(peer.sid.clone()));
    }
    if own.session != peer.session {
        return Err(AccessDenied::OtherSession {
            ours: own.session,
            theirs: peer.session,
        });
    }
    Ok(())
}

/// The agent's check of a client: the same user and session, and never a
/// client with fewer rights than the agent. An agent started "as
/// administrator" starts every program it restores as administrator too, so
/// answering an ordinary client would hand it those rights. Such a client is
/// turned away and does the work itself, with its own rights.
pub fn check_client(
    agent: &Identity,
    agent_elevated: bool,
    client: &Identity,
    client_elevated: bool,
) -> Result<(), AccessDenied> {
    check_peer(agent, client)?;
    if agent_elevated && !client_elevated {
        return Err(AccessDenied::LessPrivileged);
    }
    Ok(())
}

/// Whether this process runs with administrator rights.
pub fn is_elevated() -> std::io::Result<bool> {
    #[cfg(windows)]
    {
        crate::pipe::current_elevated()
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(sid: &str, session: u32) -> Identity {
        Identity {
            sid: sid.into(),
            session,
        }
    }

    #[test]
    fn same_user_same_session_is_allowed() {
        let me = id("S-1-5-21-1-2-3-1001", 1);
        assert_eq!(check_peer(&me, &me.clone()), Ok(()));
    }

    #[test]
    fn other_user_is_denied() {
        let me = id("S-1-5-21-1-2-3-1001", 1);
        for sid in [
            "S-1-5-21-1-2-3-1002", // another account on the machine
            "S-1-5-18",            // LocalSystem
            "S-1-5-32-544",        // the Administrators group
            "S-1-5-21-1-2-3-100",  // a prefix of ours
            "",
        ] {
            assert_eq!(
                check_peer(&me, &id(sid, 1)),
                Err(AccessDenied::OtherUser(sid.into())),
                "{sid}"
            );
        }
    }

    #[test]
    fn same_user_other_session_is_denied() {
        let me = id("S-1-5-21-1-2-3-1001", 1);
        assert_eq!(
            check_peer(&me, &id("S-1-5-21-1-2-3-1001", 2)),
            Err(AccessDenied::OtherSession { ours: 1, theirs: 2 })
        );
    }

    #[test]
    fn elevated_agent_serves_only_elevated_clients() {
        let me = id("S-1-5-21-1-2-3-1001", 1);
        // (agent elevated, client elevated) -> allowed?
        for (agent, client, allowed) in [
            (false, false, true),
            // An elevated client may ask an ordinary agent: nothing is gained.
            (false, true, true),
            (true, true, true),
            (true, false, false),
        ] {
            let verdict = check_client(&me, agent, &me, client);
            assert_eq!(verdict.is_ok(), allowed, "agent {agent}, client {client}");
        }
        assert_eq!(
            check_client(&me, true, &me, false),
            Err(AccessDenied::LessPrivileged)
        );
        // The user and session rules still come first.
        assert!(matches!(
            check_client(&me, false, &id("S-1-5-18", 1), true),
            Err(AccessDenied::OtherUser(_))
        ));
    }

    #[test]
    fn pipe_name_separates_users_and_sessions() {
        let a = crate::pipe_name_for(&id("S-1-5-21-9-1001", 1));
        assert_eq!(a, r"\\.\pipe\ContextFlow.Agent.S-1-5-21-9-1001.1");
        assert_ne!(a, crate::pipe_name_for(&id("S-1-5-21-9-1002", 1)));
        assert_ne!(a, crate::pipe_name_for(&id("S-1-5-21-9-1001", 2)));
    }
}
