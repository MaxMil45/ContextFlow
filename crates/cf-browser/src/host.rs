//! The native messaging host: the program Chrome/Edge start and talk to over
//! stdin. Messages are a 4-byte little-endian length followed by that many
//! bytes of JSON.
//!
//! The host only *receives*; it writes validated state files and deletes them
//! when the browser disconnects. It never writes anything back to the browser.

use std::collections::HashSet;
use std::io::{ErrorKind, Read};
use std::path::Path;

use chrono::Utc;

use crate::protocol::{sanitize, IncomingMessage, MAX_MESSAGE_BYTES};
use crate::state::{remove_state, write_state};

/// Process messages from `input` until it closes. Malformed or invalid
/// messages are logged and skipped; only an oversized or truncated frame (the
/// stream can no longer be trusted) ends the loop early.
pub fn run(input: &mut dyn Read, dir: &Path, log: &mut dyn FnMut(&str)) {
    let mut seen: HashSet<(String, String)> = HashSet::new();

    loop {
        let mut len_buf = [0u8; 4];
        match input.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => break, // browser closed the pipe
            Err(e) => {
                log(&format!("read error: {e}"));
                break;
            }
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        if len == 0 || len > MAX_MESSAGE_BYTES {
            log(&format!("refusing frame of {len} bytes; closing"));
            break;
        }
        let mut body = vec![0u8; len];
        if let Err(e) = input.read_exact(&mut body) {
            log(&format!("truncated message: {e}"));
            break;
        }

        let msg: IncomingMessage = match serde_json::from_slice(&body) {
            Ok(m) => m,
            Err(e) => {
                log(&format!("ignoring malformed message: {e}"));
                continue;
            }
        };
        match sanitize(msg, Utc::now()) {
            Ok(state) => match write_state(dir, &state) {
                Ok(()) => {
                    seen.insert((state.browser.clone(), state.profile_id.clone()));
                }
                Err(e) => log(&format!("could not save state: {e}")),
            },
            Err(e) => log(&format!("ignoring invalid message: {e}")),
        }
    }

    for (browser, profile) in seen {
        remove_state(dir, &browser, &profile);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::read_live_states;
    use std::io::Cursor;

    fn frame(json: &str) -> Vec<u8> {
        let mut v = (json.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(json.as_bytes());
        v
    }

    fn state_msg(profile: &str, url: &str) -> String {
        format!(
            r#"{{"type":"state","browser":"chrome","profile_id":"{profile}","sent_at":1,
                "windows":[{{"focused":true,"state":"normal","tabs":[
                {{"index":0,"url":"{url}","title":"T","pinned":false,"active":true}}]}}]}}"#
        )
    }

    fn run_bytes(bytes: Vec<u8>, dir: &Path) -> Vec<String> {
        let mut logs = Vec::new();
        run(&mut Cursor::new(bytes), dir, &mut |m| {
            logs.push(m.to_string())
        });
        logs
    }

    #[test]
    fn state_is_visible_while_connected_and_removed_on_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        // A reader that reports what it could see mid-stream.
        struct Probe<'a> {
            data: Cursor<Vec<u8>>,
            dir: &'a Path,
            seen: &'a std::cell::RefCell<usize>,
        }
        impl Read for Probe<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.data.read(buf)?;
                if n == 0 {
                    *self.seen.borrow_mut() = read_live_states(self.dir, "chrome").len();
                }
                Ok(n)
            }
        }
        let seen = std::cell::RefCell::new(0);
        let mut input = Probe {
            data: Cursor::new(frame(&state_msg("p1", "https://drive.google.com"))),
            dir: dir.path(),
            seen: &seen,
        };
        run(&mut input, dir.path(), &mut |_| {});
        assert_eq!(*seen.borrow(), 1, "state existed before the pipe closed");
        assert!(
            read_live_states(dir.path(), "chrome").is_empty(),
            "removed after"
        );
    }

    #[test]
    fn latest_message_wins() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = frame(&state_msg("p1", "https://one.com"));
        bytes.extend(frame(&state_msg("p1", "https://two.com")));
        // Stop reading before EOF cleanup by inspecting via a custom run: use
        // the lower-level pieces instead.
        let mut cur = Cursor::new(bytes);
        let mut seen = HashSet::new();
        for _ in 0..2 {
            let mut l = [0u8; 4];
            cur.read_exact(&mut l).unwrap();
            let mut b = vec![0u8; u32::from_le_bytes(l) as usize];
            cur.read_exact(&mut b).unwrap();
            let m: IncomingMessage = serde_json::from_slice(&b).unwrap();
            let s = sanitize(m, Utc::now()).unwrap();
            write_state(dir.path(), &s).unwrap();
            seen.insert(s.profile_id);
        }
        let states = read_live_states(dir.path(), "chrome");
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].windows[0].tabs[0].url, "https://two.com");
    }

    #[test]
    fn bad_input_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = frame("{not json");
        bytes.extend(frame(&state_msg("../evil", "https://a.com")));
        bytes.extend(frame(
            r#"{"type":"state","browser":"firefox","profile_id":"x","windows":[]}"#,
        ));
        let logs = run_bytes(bytes, dir.path());
        assert_eq!(logs.len(), 3, "{logs:?}");
        assert!(
            std::fs::read_dir(dir.path())
                .map(|d| d.count())
                .unwrap_or(0)
                == 0
        );
    }

    #[test]
    fn oversized_and_truncated_frames_stop_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let huge = ((MAX_MESSAGE_BYTES + 1) as u32).to_le_bytes().to_vec();
        assert!(run_bytes(huge, dir.path())[0].contains("refusing"));
        let zero = 0u32.to_le_bytes().to_vec();
        assert!(run_bytes(zero, dir.path())[0].contains("refusing"));
        let mut cut = 100u32.to_le_bytes().to_vec();
        cut.extend_from_slice(b"short");
        assert!(run_bytes(cut, dir.path())[0].contains("truncated"));
    }
}
