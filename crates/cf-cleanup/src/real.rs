//! The real system.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use cf_windows::ProcessInfo;

use crate::Processes;

pub struct WindowsProcesses;

impl Processes for WindowsProcesses {
    fn list(&self) -> Vec<ProcessInfo> {
        cf_windows::list_processes().unwrap_or_default()
    }

    fn request_close(&self, pid: u32) -> usize {
        cf_windows::request_close(pid)
    }

    fn terminate(&self, pid: u32) -> Result<(), String> {
        cf_windows::terminate_process(pid).map_err(|e| e.to_string())
    }

    fn own_session(&self) -> Option<u32> {
        cf_windows::current_session()
    }

    /// This program and every program that started it (the shell, a terminal,
    /// the app that launched it): closing one would end this very run.
    fn protected_pids(&self) -> HashSet<u32> {
        let parents: HashMap<u32, u32> =
            self.list().iter().map(|p| (p.pid, p.parent_pid)).collect();
        let mut out = HashSet::new();
        let mut pid = std::process::id();
        while out.insert(pid) {
            match parents.get(&pid) {
                Some(&parent) if parent != 0 => pid = parent,
                _ => break,
            }
        }
        out
    }

    fn logical_cpus(&self) -> usize {
        std::thread::available_parallelism().map_or(1, usize::from)
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}
