use std::path::Path;

use serde::{Deserialize, Serialize};

/// One entry from `claude agents --json`.
///
/// Two fields describe what a session is doing:
/// - `status` (`busy` / `idle`, sometimes absent): whether a turn is in
///   progress right now.
/// - `state` (`working`, `blocked`, `done`, seen on background sessions):
///   where the session's task stands.
///
/// Either may be missing.
///
/// Background sessions also carry a short `id` (what `claude attach` takes).
/// `pid` is sometimes absent, even for active sessions.
///
/// Every field is optional and unknown fields are ignored, so a newer Claude
/// Code that adds, drops or renames something doesn't break the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Session {
    pub pid: Option<u32>,
    pub id: Option<String>,
    pub cwd: String,
    pub kind: SessionKind,
    pub started_at: Option<u64>,
    pub session_id: String,
    pub name: Option<String>,
    pub status: Option<String>,
    pub state: Option<String>,
    /// Whether plain `claude agents --json` lists this session, i.e. Claude
    /// counts it as active. The rest only appear with `--all`, which Claude's
    /// help calls "completed background sessions". Set by
    /// `claude::list_sessions`, never parsed.
    #[serde(skip_deserializing)]
    pub active: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            pid: None,
            id: None,
            cwd: String::new(),
            kind: SessionKind::Other,
            started_at: None,
            session_id: String::new(),
            name: None,
            status: None,
            state: None,
            active: false,
        }
    }
}

/// The status dot, mirroring `claude agents`: `state` picks the colour and
/// `status: busy` makes it pulse. Either field may be missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Dot {
    pub color: DotColor,
    pub pulsing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DotColor {
    /// `state: working`
    Green,
    /// `state: done`
    Blue,
    /// `state: blocked`
    Orange,
    /// No state, or one not listed above.
    Gray,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    Interactive,
    Background,
    #[serde(other)]
    Other,
}

impl Session {
    /// The id `claude attach` accepts, if this session can be attached to.
    pub fn attach_id(&self) -> Option<&str> {
        match self.kind {
            SessionKind::Background => self.id.as_deref(),
            _ => None,
        }
    }

    /// Identifies the same session across the `--json` and `--json --all`
    /// listings.
    pub fn key(&self) -> &str {
        if self.session_id.is_empty() {
            self.id.as_deref().unwrap_or("")
        } else {
            &self.session_id
        }
    }

    pub fn dot(&self) -> Dot {
        let color = match self.state.as_deref() {
            Some("working") => DotColor::Green,
            Some("done") => DotColor::Blue,
            Some("blocked") => DotColor::Orange,
            _ => DotColor::Gray,
        };
        Dot {
            color,
            pulsing: self.status.as_deref() == Some("busy"),
        }
    }

    /// Last path component of `cwd`, used to group sessions by project.
    pub fn project(&self) -> String {
        Path::new(&self.cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.cwd.clone())
    }

    pub fn display_name(&self) -> &str {
        [
            self.name.as_deref(),
            self.id.as_deref(),
            Some(self.session_id.as_str()),
        ]
        .into_iter()
        .flatten()
        .find(|s| !s.is_empty())
        .unwrap_or("unnamed session")
    }
}

/// Parse `claude agents --json`. Entries are parsed one at a time and any that
/// don't fit are skipped, so one unexpected entry can't hide every session.
pub fn parse_agents_json(json: &str) -> serde_json::Result<Vec<Session>> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(json)?;
    Ok(entries
        .into_iter()
        .filter_map(|e| serde_json::from_value(e).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {
        "pid": 41022,
        "cwd": "/Users/you/projects/meadow",
        "kind": "interactive",
        "startedAt": 1793280000000,
        "sessionId": "5d0c2f7e-1a3b-4c8d-9e6f-0a1b2c3d4e5f",
        "name": "meadow-refactor",
        "status": "busy"
      },
      {
        "pid": 41377,
        "id": "c42d8a15",
        "cwd": "/Users/you/projects/harbor",
        "kind": "background",
        "startedAt": 1793300000000,
        "sessionId": "c42d8a15-6e7f-4a8b-9c0d-1e2f3a4b5c6d",
        "name": "harbor-build",
        "status": "idle",
        "state": "done",
        "someFutureField": true
      }
    ]"#;

    #[test]
    fn parses_interactive_and_background() {
        let sessions = parse_agents_json(SAMPLE).unwrap();
        assert_eq!(sessions.len(), 2);

        let interactive = &sessions[0];
        assert_eq!(interactive.kind, SessionKind::Interactive);
        assert_eq!(interactive.attach_id(), None);
        assert_eq!(interactive.project(), "meadow");

        let bg = &sessions[1];
        assert_eq!(bg.kind, SessionKind::Background);
        assert_eq!(bg.attach_id(), Some("c42d8a15"));
        assert_eq!(bg.state.as_deref(), Some("done"));
        assert_eq!(bg.project(), "harbor");
    }

    #[test]
    fn entries_without_pid_or_other_fields_still_parse() {
        // A blocked background session can have no pid or status; a completed
        // one (from `--all`) has only a state.
        let json = r#"[
          {"id":"c42d8a15","cwd":"/p/harbor","kind":"background","startedAt":1,"sessionId":"c42d8a15-6e7f","name":"harbor-build","state":"blocked"},
          {"id":"5d0c2f7e","cwd":"/p/meadow","kind":"background","sessionId":"5d0c2f7e-1a3b","state":"done"},
          "not an object",
          {"pid":"not a number","kind":"background"}
        ]"#;
        let sessions = parse_agents_json(json).unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions[0].pid.is_none() && sessions[0].status.is_none());
        assert_eq!(sessions[0].attach_id(), Some("c42d8a15"));
        assert_eq!(sessions[1].key(), "5d0c2f7e-1a3b");
        // `active` comes from comparing listings, never from the JSON.
        assert!(sessions.iter().all(|s| !s.active));
        assert!(!parse_agents_json(r#"[{"kind":"background","active":true}]"#).unwrap()[0].active);
        assert!(parse_agents_json("{}").is_err());
    }

    /// Every status/state combination seen in real `claude agents --json`
    /// output, plus missing and unexpected values.
    #[test]
    fn dot_colour_follows_state_and_busy_pulses() {
        use DotColor::*;
        let check = |status: Option<&str>, state: Option<&str>, color: DotColor, pulsing: bool| {
            let s = Session {
                status: status.map(Into::into),
                state: state.map(Into::into),
                ..Session::default()
            };
            assert_eq!(
                s.dot(),
                Dot { color, pulsing },
                "status {status:?}, state {state:?}"
            );
        };
        check(Some("idle"), Some("done"), Blue, false);
        check(None, Some("blocked"), Orange, false);
        check(Some("idle"), Some("blocked"), Orange, false);
        check(Some("busy"), Some("blocked"), Orange, true);
        check(Some("busy"), Some("working"), Green, true);
        check(None, Some("working"), Green, false);
        check(Some("busy"), None, Gray, true);
        check(Some("idle"), None, Gray, false);
        check(None, None, Gray, false);
        check(Some("busy"), Some("something-new"), Gray, true);
    }

    #[test]
    fn display_name_falls_back_sensibly() {
        let mut s = Session {
            id: Some("abc".into()),
            ..Session::default()
        };
        assert_eq!(s.display_name(), "abc");
        s.name = Some("named".into());
        assert_eq!(s.display_name(), "named");
        assert_eq!(Session::default().display_name(), "unnamed session");
    }

    #[test]
    fn unknown_kind_does_not_fail() {
        let json =
            r#"[{"pid":1,"cwd":"C:\\dev\\game","kind":"remote","startedAt":0,"sessionId":"x"}]"#;
        let sessions = parse_agents_json(json).unwrap();
        assert_eq!(sessions[0].kind, SessionKind::Other);
        assert_eq!(sessions[0].display_name(), "x");
    }
}
