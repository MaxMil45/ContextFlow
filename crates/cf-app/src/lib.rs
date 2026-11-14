//! Application services: capture, restore sessions, desktop shortcuts, icons,
//! maintenance and logging. The `contextflow` CLI, the windowless launcher and
//! the desktop app are thin front ends over this crate.

pub mod autosave;
pub mod capture;
pub mod icons;
pub mod logging;
pub mod maintenance;
pub mod session;
pub mod shortcut;
