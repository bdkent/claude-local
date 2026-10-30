use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::terminal::TerminalApp;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub terminal: TerminalApp,
    /// Used when `terminal` is `custom`. Placeholders: `{args}` (the full
    /// `claude …` argv, as its own word), `{cwd}`, `{name}`, and `{command}`
    /// (a `cd … && claude …` line for a shell).
    pub custom_terminal: String,
    pub keepalive: bool,
    /// Explicit path to `claude`; empty means search PATH.
    pub claude_path: String,
    /// Poll interval while the list window is visible.
    pub poll_visible_secs: u64,
    /// Poll interval while hidden (keep-alive checks only need to be occasional).
    pub poll_hidden_secs: u64,
    /// Default for the "Start in background" box in the New Session dialog.
    pub new_session_background: bool,
    /// Folders new sessions were started in, most recent first.
    pub recent_projects: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            terminal: TerminalApp::platform_default(),
            custom_terminal: String::new(),
            keepalive: true,
            claude_path: String::new(),
            poll_visible_secs: 3,
            poll_hidden_secs: 30,
            new_session_background: true,
            recent_projects: Vec::new(),
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("claude-local").join("config.json"))
    }

    /// Whether a config file has been saved yet (false on first run).
    pub fn exists() -> bool {
        Self::path().is_some_and(|p| p.is_file())
    }

    /// Load the config, falling back to defaults if it is missing or unreadable.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn remember_project(&mut self, cwd: &str) {
        const MAX: usize = 12;
        self.recent_projects.retain(|p| p != cwd);
        self.recent_projects.insert(0, cwd.to_owned());
        self.recent_projects.truncate(MAX);
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path().context("no config directory on this system")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_config_files_get_new_defaults() {
        let c: Config = serde_json::from_str(r#"{"terminal":"ghostty"}"#).unwrap();
        assert_eq!(c.terminal, TerminalApp::Ghostty);
        assert!(c.new_session_background && c.keepalive);
    }

    #[test]
    fn recent_projects_dedupe_and_cap() {
        let mut c = Config::default();
        for i in 0..20 {
            c.remember_project(&format!("/p{i}"));
        }
        c.remember_project("/p5");
        assert_eq!(c.recent_projects.len(), 12);
        assert_eq!(c.recent_projects[0], "/p5");
        assert_eq!(c.recent_projects.iter().filter(|p| *p == "/p5").count(), 1);
    }
}
