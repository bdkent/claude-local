//! Opening a session in the user's terminal app.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::env::ToolEnv;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalApp {
    // macOS
    Terminal,
    Iterm,
    Ghostty,
    Wezterm,
    Kitty,
    Alacritty,
    // Windows
    WindowsTerminal,
    Powershell,
    // Linux (development)
    XTerminalEmulator,
    /// `Config::custom_terminal` with placeholders.
    Custom,
}

impl TerminalApp {
    pub fn platform_default() -> Self {
        if cfg!(target_os = "macos") {
            Self::Terminal
        } else if cfg!(windows) {
            Self::WindowsTerminal
        } else {
            Self::XTerminalEmulator
        }
    }

    /// Presets offered in Settings on this platform.
    pub fn available() -> &'static [TerminalApp] {
        use TerminalApp::*;
        if cfg!(target_os = "macos") {
            &[Terminal, Iterm, Ghostty, Wezterm, Kitty, Alacritty, Custom]
        } else if cfg!(windows) {
            &[WindowsTerminal, Powershell, Custom]
        } else {
            &[XTerminalEmulator, Custom]
        }
    }

    pub fn label(self) -> &'static str {
        use TerminalApp::*;
        match self {
            Terminal => "Terminal",
            Iterm => "iTerm2",
            Ghostty => "Ghostty",
            Wezterm => "WezTerm",
            Kitty => "kitty",
            Alacritty => "Alacritty",
            WindowsTerminal => "Windows Terminal",
            Powershell => "PowerShell window",
            XTerminalEmulator => "x-terminal-emulator",
            Custom => "Custom command…",
        }
    }
}

/// The session to open.
#[derive(Debug, Clone)]
pub struct Target<'a> {
    pub id: &'a str,
    pub cwd: &'a str,
    pub name: &'a str,
}

/// A fully resolved process to spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// Windows: give the process its own visible console.
    pub new_console: bool,
}

impl Launch {
    fn new(program: &str, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            cwd: None,
            new_console: false,
        }
    }

    pub fn spawn(&self, env: &ToolEnv) -> Result<()> {
        let mut cmd = if self.new_console {
            let mut c = Command::new(&self.program);
            c.env("PATH", &env.path);
            new_console(&mut c);
            c
        } else {
            env.command(&self.program)
        };
        cmd.args(&self.args);
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }
        cmd.spawn()
            .with_context(|| format!("launching {}", self.program))?;
        Ok(())
    }
}

pub fn build_launch(app: TerminalApp, custom: &str, claude: &Path, t: &Target) -> Result<Launch> {
    use TerminalApp::*;
    let claude_str = claude.to_string_lossy().into_owned();
    // `claude attach <id>` as argv, wrapping batch files for Windows.
    let attach_argv: Vec<String> = if is_batch_file(claude) {
        vec![
            "cmd.exe".into(),
            "/c".into(),
            claude_str.clone(),
            "attach".into(),
            t.id.into(),
        ]
    } else {
        vec![claude_str.clone(), "attach".into(), t.id.into()]
    };
    // The same, as a line typed into a POSIX shell.
    let shell_line = format!(
        "cd {} && {} attach {}",
        shell_quote(t.cwd),
        shell_quote(&claude_str),
        shell_quote(t.id)
    );

    let launch = match app {
        Terminal => osascript(&[
            "tell application \"Terminal\"",
            "activate",
            &format!("do script {}", applescript_string(&shell_line)),
            "end tell",
        ]),
        Iterm => osascript(&[
            "tell application \"iTerm\"",
            "activate",
            "set w to (create window with default profile)",
            &format!(
                "tell current session of w to write text {}",
                applescript_string(&shell_line)
            ),
            "end tell",
        ]),
        Ghostty => open_app(
            "Ghostty",
            [format!("--working-directory={}", t.cwd), "-e".into()]
                .into_iter()
                .chain(attach_argv),
        ),
        Wezterm => open_app(
            "WezTerm",
            ["start".into(), "--cwd".into(), t.cwd.into(), "--".into()]
                .into_iter()
                .chain(attach_argv),
        ),
        Kitty => open_app(
            "kitty",
            ["--directory".into(), t.cwd.into()]
                .into_iter()
                .chain(attach_argv),
        ),
        Alacritty => open_app(
            "Alacritty",
            ["--working-directory".into(), t.cwd.into(), "-e".into()]
                .into_iter()
                .chain(attach_argv),
        ),
        WindowsTerminal => Launch::new(
            "wt.exe",
            ["-w", "0", "new-tab", "--title", t.name, "-d", t.cwd]
                .into_iter()
                .map(String::from)
                .chain(attach_argv),
        ),
        Powershell => {
            let mut l = Launch::new(
                "powershell.exe",
                [
                    "-NoExit".to_string(),
                    "-Command".into(),
                    format!(
                        "& {} attach {}",
                        powershell_string(&claude_str),
                        powershell_string(t.id)
                    ),
                ],
            );
            l.cwd = Some(t.cwd.into());
            l.new_console = true;
            l
        }
        XTerminalEmulator => {
            let mut l = Launch::new(
                "x-terminal-emulator",
                ["-e".to_string()].into_iter().chain(attach_argv),
            );
            l.cwd = Some(t.cwd.into());
            l
        }
        Custom => {
            let parts = split_args(custom);
            let Some((program, args)) = parts.split_first() else {
                bail!("the custom terminal command is empty; set it in Settings");
            };
            let fill = |s: &str| {
                s.replace("{claude}", &claude_str)
                    .replace("{id}", t.id)
                    .replace("{cwd}", t.cwd)
                    .replace("{name}", t.name)
                    .replace("{command}", &shell_line)
            };
            let mut l = Launch::new(&fill(program), args.iter().map(|a| fill(a)));
            l.cwd = Some(t.cwd.into());
            l
        }
    };
    Ok(launch)
}

fn osascript(lines: &[&str]) -> Launch {
    Launch::new("osascript", lines.iter().flat_map(|l| ["-e", l]))
}

/// `open -na <App> --args …` starts the app even when it isn't on PATH.
fn open_app(app: &str, args: impl IntoIterator<Item = String>) -> Launch {
    Launch::new(
        "open",
        ["-na".to_string(), app.into(), "--args".into()]
            .into_iter()
            .chain(args),
    )
}

pub fn is_batch_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
}

/// Quote for a POSIX shell, leaving plain words alone.
pub fn shell_quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c));
    if plain {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn powershell_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Split a command line on whitespace, honouring single and double quotes.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    out
}

#[cfg(windows)]
fn new_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    cmd.creation_flags(CREATE_NEW_CONSOLE);
}

#[cfg(not(windows))]
fn new_console(_cmd: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> Target<'static> {
        Target {
            id: "3b9e1c07",
            cwd: "/Users/you/my projects/starship",
            name: "starship-export",
        }
    }

    #[test]
    fn terminal_app_types_a_quoted_command() {
        let l = build_launch(
            TerminalApp::Terminal,
            "",
            Path::new("/Users/you/.local/bin/claude"),
            &target(),
        )
        .unwrap();
        assert_eq!(l.program, "osascript");
        assert!(l.args.contains(
            &r#"do script "cd '/Users/you/my projects/starship' && /Users/you/.local/bin/claude attach 3b9e1c07""#.to_string()
        ));
    }

    #[test]
    fn ghostty_passes_argv() {
        let l = build_launch(
            TerminalApp::Ghostty,
            "",
            Path::new("/bin/claude"),
            &target(),
        )
        .unwrap();
        assert_eq!(
            l.args,
            [
                "-na",
                "Ghostty",
                "--args",
                "--working-directory=/Users/you/my projects/starship",
                "-e",
                "/bin/claude",
                "attach",
                "3b9e1c07"
            ]
        );
    }

    #[test]
    fn windows_terminal_wraps_cmd_shims() {
        let l = build_launch(
            TerminalApp::WindowsTerminal,
            "",
            Path::new(r"C:\Users\me\AppData\Roaming\npm\claude.cmd"),
            &Target {
                id: "abc",
                cwd: r"C:\dev\game",
                name: "n",
            },
        )
        .unwrap();
        assert_eq!(l.program, "wt.exe");
        assert_eq!(
            &l.args[l.args.len() - 5..],
            [
                "cmd.exe",
                "/c",
                r"C:\Users\me\AppData\Roaming\npm\claude.cmd",
                "attach",
                "abc"
            ]
        );
    }

    #[test]
    fn custom_fills_placeholders() {
        let l = build_launch(
            TerminalApp::Custom,
            r#"my-term --title "{name}" -- {claude} attach {id}"#,
            Path::new("/bin/claude"),
            &target(),
        )
        .unwrap();
        assert_eq!(l.program, "my-term");
        assert_eq!(
            l.args,
            [
                "--title",
                "starship-export",
                "--",
                "/bin/claude",
                "attach",
                "3b9e1c07"
            ]
        );
        assert!(build_launch(TerminalApp::Custom, "  ", Path::new("c"), &target()).is_err());
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("abc-1"), "abc-1");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(split_args(r#"a "b c" 'd"e' ''"#), ["a", "b c", "d\"e", ""]);
    }
}
