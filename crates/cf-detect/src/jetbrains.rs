//! JetBrains IDEs (IntelliJ IDEA, PyCharm, CLion, WebStorm, Rider...): which
//! project each window has open.
//!
//! A JetBrains window title starts with the project name: `HelloWorld –
//! Another.py` (older versions: `HelloWorld [C:\path] – Another.py`). Each IDE
//! records the projects it has opened, with their paths, the title it last
//! showed and when each was last active, in
//! `%APPDATA%\JetBrains\<IDE><version>\options\recentProjects.xml`. The title is
//! matched against that list (the most recently active match wins), and the
//! project reopens with `<ide>64.exe <path>`.

use std::path::{Path, PathBuf};

use crate::{Candidate, Sources};

const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Executables of JetBrains IDEs, as application ids.
const IDES: &[&str] = &[
    "idea64",
    "idea",
    "pycharm64",
    "pycharm",
    "clion64",
    "clion",
    "webstorm64",
    "webstorm",
    "rider64",
    "rider",
    "goland64",
    "goland",
    "phpstorm64",
    "phpstorm",
    "rubymine64",
    "rubymine",
    "datagrip64",
    "datagrip",
    "rustrover64",
    "rustrover",
    "dataspell64",
    "dataspell",
    "studio64",
];

/// Whether an application id is a JetBrains IDE (Android Studio included).
pub fn is_jetbrains(application_id: &str) -> bool {
    IDES.contains(&application_id)
}

/// The project name from a window title, and a path when the title shows one.
/// `"HelloWorld – Another.py"` gives `HelloWorld`; `"App [C:\work\app] – x"`
/// gives `App` and `C:\work\app`.
pub fn parse_title(title: &str) -> (Option<String>, Option<String>) {
    let t = title.trim();
    let first = t.split(" \u{2013} ").next().unwrap_or(t);
    let first = first.split(" - ").next().unwrap_or(first).trim();
    let (name, path) = match (first.find(" ["), first.ends_with(']')) {
        (Some(i), true) => (&first[..i], Some(first[i + 2..first.len() - 1].to_string())),
        _ => (first, None),
    };
    let name = name.trim();
    ((!name.is_empty()).then(|| name.to_string()), path)
}

/// For restore: the project name of an open window.
pub fn name_from_title(title: &str) -> Option<String> {
    parse_title(title).0
}

/// One project an IDE has opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentProject {
    pub path: String,
    /// The title the IDE last showed for it, if recorded.
    pub frame_title: Option<String>,
    /// When it was last active (milliseconds since 1970), 0 if unknown.
    pub activated: u64,
}

impl RecentProject {
    /// The names its windows may show: from the recorded title, and the folder's own name.
    fn names(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(n) = self.frame_title.as_deref().and_then(name_from_title) {
            out.push(n);
        }
        if let Some(n) = Path::new(&self.path).file_name() {
            out.push(n.to_string_lossy().to_string());
        }
        out
    }
}

fn unescape(s: &str) -> String {
    s.replace("&#8211;", "\u{2013}")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn attr<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("{name}=\"");
    let start = text.find(&marker)? + marker.len();
    let end = text[start..].find('"')? + start;
    Some(&text[start..end])
}

/// The projects in one `recentProjects.xml`. `home` replaces `$USER_HOME$`.
pub fn parse_recent(xml: &str, home: &str) -> Vec<RecentProject> {
    let mut out = Vec::new();
    for chunk in xml.split("<entry key=\"").skip(1) {
        let Some(end) = chunk.find('"') else { continue };
        let path = unescape(&chunk[..end])
            .replace("$USER_HOME$", home)
            .replace('/', "\\");
        // Only this entry's own text.
        let body = chunk.split("</entry>").next().unwrap_or(chunk);
        if !body.contains("RecentProjectMetaInfo") {
            continue;
        }
        let frame_title = attr(body, "frameTitle").map(unescape);
        let activated = body
            .split("name=\"activationTimestamp\"")
            .nth(1)
            .and_then(|rest| attr(rest, "value"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        out.push(RecentProject {
            path,
            frame_title,
            activated,
        });
    }
    out
}

/// Every installed JetBrains IDE's `recentProjects.xml`.
pub fn default_recent_files() -> Vec<PathBuf> {
    let Some(base) = std::env::var_os("APPDATA") else {
        return Vec::new();
    };
    let Ok(dirs) = std::fs::read_dir(PathBuf::from(base).join("JetBrains")) else {
        return Vec::new();
    };
    dirs.flatten()
        .map(|d| d.path().join("options").join("recentProjects.xml"))
        .filter(|p| p.is_file())
        .collect()
}

/// The recent projects of every installed IDE.
pub fn read_recent(files: &[PathBuf]) -> Vec<RecentProject> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let mut all: Vec<RecentProject> = Vec::new();
    for f in files {
        if !std::fs::metadata(f).is_ok_and(|m| m.len() <= MAX_FILE_BYTES) {
            continue;
        }
        let Ok(xml) = std::fs::read_to_string(f) else {
            continue;
        };
        for p in parse_recent(&xml, &home) {
            match all
                .iter_mut()
                .find(|a| a.path.eq_ignore_ascii_case(&p.path))
            {
                Some(a) if p.activated > a.activated => *a = p,
                Some(_) => {}
                None => all.push(p),
            }
        }
    }
    all
}

/// The recent projects as candidates, ranked by when each was last active.
fn to_candidates(recent: &[RecentProject]) -> Vec<Candidate> {
    recent
        .iter()
        .map(|r| Candidate {
            names: r.names(),
            path: r.path.clone(),
            rank: r.activated,
        })
        .collect()
}

pub(crate) fn candidates(sources: &Sources) -> Option<Vec<Candidate>> {
    let files = &sources.jetbrains_recent;
    (!files.is_empty()).then(|| to_candidates(&read_recent(files)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;

    fn build_state(
        titles: &[String],
        list: &[RecentProject],
        exists: &dyn Fn(&str) -> bool,
    ) -> State {
        crate::build_state(
            crate::app(crate::JETBRAINS),
            titles,
            &to_candidates(list),
            exists,
        )
    }

    const XML: &str = r#"<application>
  <component name="RecentProjectsManager">
    <option name="additionalInfo">
      <map>
        <entry key="$USER_HOME$/C_Languages/C++">
          <value>
            <RecentProjectMetaInfo frameTitle="C__ &#8211; exercise.cpp" projectWorkspaceId="a">
              <option name="activationTimestamp" value="1740746061542" />
            </RecentProjectMetaInfo>
          </value>
        </entry>
        <entry key="$USER_HOME$/PycharmProjects/HelloWorld">
          <value>
            <RecentProjectMetaInfo frameTitle="HelloWorld – Another.py" projectWorkspaceId="b">
              <option name="activationTimestamp" value="1696172907759" />
            </RecentProjectMetaInfo>
          </value>
        </entry>
      </map>
    </option>
  </component>
</application>"#;

    #[test]
    fn titles_give_the_project_name_and_any_path_shown() {
        assert_eq!(
            parse_title("HelloWorld – Another.py"),
            (Some("HelloWorld".into()), None)
        );
        assert_eq!(
            parse_title(r"App [C:\work\app] – Main.kt"),
            (Some("App".into()), Some(r"C:\work\app".into()))
        );
        assert_eq!(
            parse_title("Welcome to PyCharm").0.as_deref(),
            Some("Welcome to PyCharm")
        );
    }

    #[test]
    fn recent_projects_are_read_with_the_home_folder_filled_in() {
        let r = parse_recent(XML, r"C:\Users\me");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].path, r"C:\Users\me\C_Languages\C++");
        assert_eq!(r[1].activated, 1_696_172_907_759);
    }

    #[test]
    fn windows_match_by_recorded_title_or_folder_name() {
        let r = parse_recent(XML, r"C:\Users\me");
        let titles = vec![
            "HelloWorld – main.py".to_string(),
            "C++ – exercise.cpp".to_string(),
        ];
        let s = build_state(&titles, &r, &|_| true);
        assert_eq!(
            s.windows[0].path.as_deref(),
            Some(r"C:\Users\me\PycharmProjects\HelloWorld")
        );
        assert_eq!(
            s.windows[1].path.as_deref(),
            Some(r"C:\Users\me\C_Languages\C++"),
            "by folder name"
        );
        assert_eq!(
            build_state(&titles, &r, &|_| false).path_count(),
            0,
            "gone from disk"
        );
        assert!(is_jetbrains("pycharm64") && is_jetbrains("clion64") && !is_jetbrains("code"));
    }
}
