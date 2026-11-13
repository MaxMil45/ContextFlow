//! Redaction helpers for anything written to log files.
//!
//! Log files may be attached to bug reports, so they must not contain secrets
//! that commonly hide in URLs (tokens in query strings, credentials in the
//! authority) or the user's account name in paths.

/// Remove credentials, query string and fragment from a URL.
///
/// `https://user:pw@example.com/a/b?token=x#frag` -> `https://example.com/a/b?[redacted]`
pub fn url(s: &str) -> String {
    let cut_at = s.find(['?', '#']);
    let base = cut_at.map_or(s, |i| &s[..i]);
    let suffix = if cut_at.is_some() { "?[redacted]" } else { "" };

    let Some(scheme_end) = base.find("://") else {
        return format!("{base}{suffix}");
    };
    let (scheme, rest) = base.split_at(scheme_end + 3);
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let (authority, path) = rest.split_at(authority_end);
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    format!("{scheme}{host}{path}{suffix}")
}

/// Replace the user's home directory prefix with `~`.
pub fn home(path: &str) -> String {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok();
    home_with(path, home.as_deref())
}

/// [`home`] with an explicit home directory (case-insensitive on Windows paths).
pub fn home_with(path: &str, home: Option<&str>) -> String {
    let Some(home) = home.filter(|h| !h.is_empty()) else {
        return path.to_string();
    };
    let normalise = |s: &str| s.replace('\\', "/").to_lowercase();
    let (p, h) = (
        normalise(path),
        normalise(home.trim_end_matches(['\\', '/'])),
    );
    if p == h || p.starts_with(&format!("{h}/")) {
        format!("~{}", &path[h.len()..])
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_query_fragment_and_credentials() {
        assert_eq!(
            url("https://user:secret@example.com/a/b?token=abc&x=1#frag"),
            "https://example.com/a/b?[redacted]"
        );
        assert_eq!(url("https://example.com/docs"), "https://example.com/docs");
        assert_eq!(
            url("https://example.com#top"),
            "https://example.com?[redacted]"
        );
        assert_eq!(url("not a url?q=1"), "not a url?[redacted]");
        assert!(!url("https://x.io/cb?code=SECRET123").contains("SECRET123"));
    }

    #[test]
    fn replaces_home_prefix_only() {
        let home = Some(r"C:\Users\alex");
        assert_eq!(home_with(r"C:\Users\alex\src\cf", home), r"~\src\cf");
        assert_eq!(home_with(r"c:\users\ALEX", home), "~");
        assert_eq!(
            home_with(r"C:\Users\alexander\x", home),
            r"C:\Users\alexander\x"
        );
        assert_eq!(home_with(r"D:\work", home), r"D:\work");
        assert_eq!(home_with(r"D:\work", None), r"D:\work");
    }
}
