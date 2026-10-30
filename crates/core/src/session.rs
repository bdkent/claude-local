use std::path::Path;

use serde::{Deserialize, Serialize};

/// One entry from `claude agents --json`.
///
/// Background sessions carry a short `id` (what `claude attach` takes) and a
/// `state`; interactive sessions have neither. Unknown fields are ignored so a
/// newer Claude Code doesn't break parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub pid: u32,
    #[serde(default)]
    pub id: Option<String>,
    pub cwd: String,
    pub kind: SessionKind,
    pub started_at: u64,
    pub session_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
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

    /// Last path component of `cwd`, used to group sessions by project.
    pub fn project(&self) -> String {
        Path::new(&self.cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.cwd.clone())
    }

    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.session_id)
    }
}

pub fn parse_agents_json(json: &str) -> serde_json::Result<Vec<Session>> {
    serde_json::from_str(json)
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
    fn unknown_kind_does_not_fail() {
        let json =
            r#"[{"pid":1,"cwd":"C:\\dev\\game","kind":"remote","startedAt":0,"sessionId":"x"}]"#;
        let sessions = parse_agents_json(json).unwrap();
        assert_eq!(sessions[0].kind, SessionKind::Other);
        assert_eq!(sessions[0].display_name(), "x");
    }
}
