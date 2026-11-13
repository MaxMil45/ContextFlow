//! `contextflow-browser-host`: started by Chrome/Edge (never by hand) when the
//! ContextFlow extension connects. It receives tab lists on stdin, validates
//! them, and keeps the latest copy on disk for `contextflow capture`.

use std::io::stdin;
use std::path::Path;

/// The troubleshooting log is emptied once it grows past this size.
const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Empty `path` if it is larger than `max_bytes`. A missing file is fine.
fn truncate_if_large(path: &Path, max_bytes: u64) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > max_bytes) {
        let _ = std::fs::write(path, b"");
    }
}

fn main() {
    let Some(dir) = cf_browser::default_state_dir() else {
        eprintln!("contextflow-browser-host: no per-user data directory");
        std::process::exit(1);
    };
    // stderr is discarded by the browser; keep a short log for troubleshooting.
    let log_path = dir.with_file_name("host.log");
    truncate_if_large(&log_path, MAX_LOG_BYTES);
    let mut log = |m: &str| {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let _ = writeln!(f, "{} {m}", chrono::Utc::now().to_rfc3339());
        }
    };
    let mut input = stdin().lock();
    cf_browser::host::run(&mut input, &dir, &mut log);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_logs_are_emptied_and_small_ones_kept() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("host.log");

        // Missing file: nothing happens, nothing is created.
        truncate_if_large(&log, 100);
        assert!(!log.exists());

        // At the limit: kept. One byte over: emptied.
        std::fs::write(&log, vec![b'x'; 100]).unwrap();
        truncate_if_large(&log, 100);
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 100);
        std::fs::write(&log, vec![b'x'; 101]).unwrap();
        truncate_if_large(&log, 100);
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 0);
    }
}
