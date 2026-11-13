//! One module per group of commands. `main.rs` only parses arguments and dispatches.

pub(crate) mod browser;
pub(crate) mod capture_cmd;
pub(crate) mod config;
pub(crate) mod history;
pub(crate) mod maintenance;
pub(crate) mod restore;
pub(crate) mod util;
pub(crate) mod workspace;
