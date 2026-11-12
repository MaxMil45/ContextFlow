//! Read-only Git inspection.
//!
//! ContextFlow must never checkout, reset, stash or otherwise mutate a
//! repository. Every command here is read-only and runs with
//! `--no-optional-locks` so it cannot contend with the user's own Git usage.

use std::path::Path;
use std::process::Command;

use cf_core::GitState;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("could not run git (is it installed and on PATH?): {0}")]
    Spawn(#[source] std::io::Error),
    #[error("git {args} failed: {stderr}")]
    Command { args: String, stderr: String },
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let mut cmd = Command::new("git");
    cmd.arg("--no-optional-locks").arg("-C").arg(dir).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output().map_err(GitError::Spawn)?;
    if !output.status.success() {
        return Err(GitError::Command {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Inspect the repository containing `path`.
pub fn inspect(path: &Path) -> Result<GitState, GitError> {
    let toplevel = run_git(path, &["rev-parse", "--show-toplevel"])?;
    let status = run_git(path, &["status", "--porcelain=v2", "--branch"])?;
    let mut state = parse_status_v2(&status);
    state.repository_path = native_path(toplevel.trim());
    tracing::debug!(repo = %state.repository_path, branch = ?state.branch, "inspected repository");
    Ok(state)
}

/// git prints forward slashes on Windows; store native separators.
fn native_path(p: &str) -> String {
    if cfg!(windows) {
        p.replace('/', "\\")
    } else {
        p.to_string()
    }
}

/// Parse `git status --porcelain=v2 --branch` output.
/// `repository_path` is left empty for the caller to fill in.
pub fn parse_status_v2(output: &str) -> GitState {
    let mut s = GitState::default();
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("# branch.oid ") {
            s.head_commit = (rest != "(initial)").then(|| rest.to_string());
        } else if let Some(rest) = line.strip_prefix("# branch.head ") {
            s.branch = (rest != "(detached)").then(|| rest.to_string());
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            let mut parts = rest.split_whitespace();
            s.ahead = parts
                .next()
                .and_then(|a| a.trim_start_matches('+').parse().ok());
            s.behind = parts
                .next()
                .and_then(|b| b.trim_start_matches('-').parse().ok());
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            let xy = line.as_bytes().get(2..4).unwrap_or(b"..");
            if xy[0] != b'.' {
                s.staged += 1;
            }
            if xy[1] != b'.' {
                s.modified += 1;
            }
        } else if line.starts_with("u ") {
            s.conflicted += 1;
        } else if line.starts_with("? ") {
            s.untracked += 1;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_headers_and_entries() {
        let out = "\
# branch.oid 1234567890abcdef1234567890abcdef12345678
# branch.head feature/database
# branch.upstream origin/feature/database
# branch.ab +3 -1
1 M. N... 100644 100644 100644 aaa bbb src/staged.rs
1 .M N... 100644 100644 100644 aaa bbb src/modified.rs
1 MM N... 100644 100644 100644 aaa bbb src/both.rs
2 R. N... 100644 100644 100644 aaa bbb R100 new.rs\told.rs
u UU N... 100644 100644 100644 100644 a b c conflict.rs
? notes.txt
? scratch/
! target/
";
        let s = parse_status_v2(out);
        assert_eq!(s.branch.as_deref(), Some("feature/database"));
        assert_eq!(
            s.head_commit.as_deref(),
            Some("1234567890abcdef1234567890abcdef12345678")
        );
        assert_eq!((s.ahead, s.behind), (Some(3), Some(1)));
        assert_eq!(s.staged, 3);
        assert_eq!(s.modified, 2);
        assert_eq!(s.conflicted, 1);
        assert_eq!(s.untracked, 2);
    }

    #[test]
    fn parses_initial_and_detached() {
        let s = parse_status_v2("# branch.oid (initial)\n# branch.head (detached)\n");
        assert_eq!(s.head_commit, None);
        assert_eq!(s.branch, None);
        assert_eq!((s.ahead, s.behind), (None, None));
    }

    fn git_available() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args([
                "-c",
                "user.name=ContextFlow Test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    #[test]
    fn inspects_a_real_repository() {
        if !git_available() {
            eprintln!("skipping: git not installed");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        git(repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "1").unwrap();
        git(repo, &["add", "a.txt"]);
        git(repo, &["commit", "-q", "-m", "init"]);
        std::fs::write(repo.join("a.txt"), "2").unwrap();
        std::fs::write(repo.join("b.txt"), "new").unwrap();

        let s = inspect(repo).unwrap();
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.head_commit.as_ref().map(String::len), Some(40));
        assert_eq!((s.staged, s.modified, s.untracked), (0, 1, 1));
        assert!(!s.repository_path.is_empty());

        assert!(inspect(&repo.join("does-not-exist")).is_err());
    }
}
