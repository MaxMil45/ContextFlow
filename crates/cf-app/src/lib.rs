//! Application services: capture, restore sessions, desktop shortcuts, icons,
//! maintenance and logging. The `contextflow` CLI, the windowless launcher and
//! the desktop app are thin front ends over this crate.

pub mod autosave;
pub mod capture;
pub mod commands;
pub mod display;
pub mod health;
pub mod icons;
pub mod logging;
pub mod maintenance;
pub mod park;
pub mod seal;
pub mod search;
pub mod service;
pub mod session;
pub mod shortcut;
