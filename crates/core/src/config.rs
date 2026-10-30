use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::terminal::TerminalApp;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub terminal: TerminalApp,
    /// Used when `terminal` is `custom`. Placeholders: {claude} {id} {cwd}
    /// {name} {command} (the last is a full `cd … && claude attach …` line).
    pub custom_terminal: String,
    pub keepalive: bool,
    /// Explicit path to `claude`; empty means search PATH.
    pub claude_path: String,
    /// Poll interval while the list window is visible.
    pub poll_visible_secs: u64,
    /// Poll interval while hidden (keep-alive checks only need to be occasional).
    pub poll_hidden_secs: u64,
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
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("claude-local").join("config.json"))
    }

    /// Load the config, falling back to defaults if it is missing or unreadable.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
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
