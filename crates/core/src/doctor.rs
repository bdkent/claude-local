//! Startup checks: is everything the app shells out to actually installed?

use std::ffi::OsString;

use serde::Serialize;

use crate::config::Config;
use crate::env::{which, ToolEnv};
use crate::terminal;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    /// What was found, or what is missing.
    pub summary: String,
    /// What the dependency is for.
    pub why: &'static str,
    /// A command that fixes it, shown with a copy button.
    pub fix: Option<String>,
    /// Anything else worth saying, such as a setting that avoids the need.
    pub hint: Option<String>,
}

impl Check {
    fn ok(name: &'static str, why: &'static str, summary: String) -> Self {
        Self {
            name,
            ok: true,
            summary,
            why,
            fix: None,
            hint: None,
        }
    }
}

/// Run every check. `env` is the result of finding `claude`; `path` is the
/// search PATH, available even when `claude` wasn't found.
pub fn run(env: &Result<ToolEnv, String>, path: &OsString, config: &Config) -> Vec<Check> {
    let mut checks = vec![check_claude(env)];
    if cfg!(unix) && config.keepalive {
        checks.push(check_tmux(path));
    }
    checks.push(check_terminal(path, config));
    checks
}

fn check_claude(env: &Result<ToolEnv, String>) -> Check {
    const WHY: &str = "Needed to list, open and start sessions";
    match env {
        Ok(env) => Check::ok("claude", WHY, format!("claude at {}", env.claude.display())),
        Err(e) => Check {
            name: "claude",
            ok: false,
            summary: e.clone(),
            why: WHY,
            fix: Some(if cfg!(windows) {
                "irm https://claude.ai/install.ps1 | iex".into()
            } else {
                "curl -fsSL https://claude.ai/install.sh | bash".into()
            }),
            hint: Some("If it's installed somewhere unusual, set its path in Settings.".into()),
        },
    }
}

fn check_tmux(path: &OsString) -> Check {
    const WHY: &str = "Needed to keep background sessions alive";
    if let Some(tmux) = which(&["tmux"], path) {
        return Check::ok("tmux", WHY, format!("tmux at {}", tmux.display()));
    }
    let off = "Or turn off keep-alive in Settings.";
    let (fix, hint) = if cfg!(target_os = "macos") {
        if which(&["brew"], path).is_some() {
            ("brew install tmux", off.to_string())
        } else {
            (
                "brew install tmux",
                format!("Install Homebrew from https://brew.sh first. {off}"),
            )
        }
    } else {
        (
            "sudo apt install tmux",
            format!("Or use your distribution's package manager. {off}"),
        )
    };
    Check {
        name: "tmux",
        ok: false,
        summary: "tmux not found".into(),
        why: WHY,
        fix: Some(fix.into()),
        hint: Some(hint),
    }
}

fn check_terminal(path: &OsString, config: &Config) -> Check {
    const WHY: &str = "Used to open sessions";
    match terminal::check_installed(config.terminal, &config.custom_terminal, path) {
        Ok(()) => Check::ok(
            "terminal",
            WHY,
            format!("Opening sessions in {}", config.terminal.label()),
        ),
        Err(summary) => Check {
            name: "terminal",
            ok: false,
            summary,
            why: WHY,
            fix: None,
            hint: Some("Install it, or pick another terminal in Settings.".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TerminalApp;

    #[test]
    fn missing_claude_and_tmux_are_reported_with_fixes() {
        let config = Config {
            terminal: TerminalApp::Custom,
            custom_terminal: "/definitely/not/a/terminal".into(),
            ..Config::default()
        };
        let empty = OsString::new();
        let checks = run(
            &Err("`claude` not found on your PATH".into()),
            &empty,
            &config,
        );

        let claude = checks.iter().find(|c| c.name == "claude").unwrap();
        assert!(!claude.ok && claude.fix.is_some());

        if cfg!(unix) {
            let tmux = checks.iter().find(|c| c.name == "tmux").unwrap();
            assert!(!tmux.ok);
            assert!(tmux.fix.as_deref().unwrap().contains("install tmux"));
        }

        let term = checks.iter().find(|c| c.name == "terminal").unwrap();
        assert!(!term.ok && term.summary.contains("/definitely/not/a/terminal"));
    }

    #[test]
    fn tmux_is_not_checked_when_keepalive_is_off() {
        let config = Config {
            keepalive: false,
            ..Config::default()
        };
        let checks = run(&Err("x".into()), &OsString::new(), &config);
        assert!(checks.iter().all(|c| c.name != "tmux"));
    }
}
