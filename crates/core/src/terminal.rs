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

/// What to run in the new terminal: `claude <args>` inside `cwd`.
#[derive(Debug, Clone)]
pub struct Target {
    pub cwd: String,
    /// Window or tab title, where the terminal supports one.
    pub title: String,
    /// Arguments after `claude`.
    pub args: Vec<String>,
}

impl Target {
    /// `claude attach <id>` for an existing background session.
    pub fn attach(id: &str, cwd: &str, title: &str) -> Self {
        Self {
            cwd: cwd.into(),
            title: title.into(),
            args: vec!["attach".into(), id.into()],
        }
    }

    /// A fresh interactive session, optionally named and with a first prompt.
    pub fn new_session(cwd: &str, name: Option<&str>, prompt: Option<&str>) -> Self {
        Self {
            cwd: cwd.into(),
            title: name.unwrap_or("claude").into(),
            args: crate::claude::new_session_args(name, prompt),
        }
    }
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
    let cwd = t.cwd.as_str();
    // `claude <args>` as argv, wrapping batch files for Windows.
    let mut argv: Vec<String> = if is_batch_file(claude) {
        vec!["cmd.exe".into(), "/c".into(), claude_str.clone()]
    } else {
        vec![claude_str.clone()]
    };
    argv.extend(t.args.iter().cloned());
    // The same, as a line typed into the user's shell. Single-quoted strings
    // (including multi-line prompts) parse the same in sh, bash, zsh and fish.
    // `claude <args>` as one shell-quoted command line.
    let command_line = std::iter::once(&claude_str)
        .chain(&t.args)
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    let shell_line = format!("cd {} && {command_line}", shell_quote(cwd));

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
        // Ghostty 1.3+ can be scripted: open a tab in the front window. Older
        // versions only take a command at launch, which means a new instance.
        Ghostty if find_mac_app(app).is_some_and(|p| ghostty_scriptable(&p)) => {
            let lines = ghostty_tab_script(cwd, &t.title, &command_line);
            osascript(&lines.iter().map(String::as_str).collect::<Vec<_>>())
        }
        Ghostty => open_app(
            app,
            [format!("--working-directory={cwd}"), "-e".into()]
                .into_iter()
                .chain(argv),
        ),
        Wezterm => open_app(
            app,
            ["start".into(), "--cwd".into(), cwd.into(), "--".into()]
                .into_iter()
                .chain(argv),
        ),
        Kitty => open_app(
            app,
            ["--directory".into(), cwd.into()].into_iter().chain(argv),
        ),
        Alacritty => open_app(
            app,
            ["--working-directory".into(), cwd.into(), "-e".into()]
                .into_iter()
                .chain(argv),
        ),
        // wt treats a bare `;` as "next command", so escape it in arguments.
        WindowsTerminal => Launch::new(
            "wt.exe",
            ["-w", "0", "new-tab", "--title", &t.title, "-d", cwd]
                .into_iter()
                .map(String::from)
                .chain(argv.into_iter().map(|a| a.replace(';', "\\;"))),
        ),
        Powershell => {
            let command = std::iter::once("&".to_string())
                .chain(
                    std::iter::once(&claude_str)
                        .chain(&t.args)
                        .map(|a| powershell_string(a)),
                )
                .collect::<Vec<_>>()
                .join(" ");
            let mut l = Launch::new(
                "powershell.exe",
                ["-NoExit".to_string(), "-Command".into(), command],
            );
            l.cwd = Some(cwd.into());
            l.new_console = true;
            l
        }
        XTerminalEmulator => {
            let mut l = Launch::new(
                "x-terminal-emulator",
                ["-e".to_string()].into_iter().chain(argv),
            );
            l.cwd = Some(cwd.into());
            l
        }
        Custom => {
            let parts = split_args(custom);
            if parts.is_empty() {
                bail!("the custom terminal command is empty; set it in Settings");
            }
            // `{args}` as its own word splices in the whole claude argv.
            let filled: Vec<String> = parts
                .iter()
                .flat_map(|p| {
                    if p == "{args}" {
                        argv.clone()
                    } else {
                        vec![p
                            .replace("{cwd}", cwd)
                            .replace("{name}", &t.title)
                            .replace("{command}", &shell_line)]
                    }
                })
                .collect();
            let mut l = Launch::new(&filled[0], filled[1..].iter().cloned());
            l.cwd = Some(cwd.into());
            l
        }
    };
    Ok(launch)
}

/// Check that the chosen terminal is installed. `path` is the PATH used to
/// launch it. The error says what is missing.
/// Presets from `TerminalApp::available` that are installed, plus `Custom`.
pub fn installed(path: &std::ffi::OsString) -> Vec<TerminalApp> {
    TerminalApp::available()
        .iter()
        .copied()
        .filter(|&app| is_installed(app, path))
        .collect()
}

/// Whether a preset's app is installed. Always true for `Custom`; use
/// `check_installed` to validate the custom command itself.
pub fn is_installed(app: TerminalApp, path: &std::ffi::OsString) -> bool {
    use TerminalApp::*;
    let found = |name: &str| crate::env::which(&[name], path).is_some();
    match app {
        // Built into their platforms.
        Terminal | Powershell | Custom => true,
        Iterm | Ghostty | Wezterm | Kitty | Alacritty => find_mac_app(app).is_some(),
        WindowsTerminal => found("wt.exe"),
        XTerminalEmulator => found("x-terminal-emulator"),
    }
}

/// Check that the chosen terminal is installed. `path` is the PATH used to
/// launch it. The error says what is missing.
pub fn check_installed(
    app: TerminalApp,
    custom: &str,
    path: &std::ffi::OsString,
) -> Result<(), String> {
    if app != TerminalApp::Custom {
        return if is_installed(app, path) {
            Ok(())
        } else {
            Err(format!("{} isn't installed", app.label()))
        };
    }
    let program = split_args(custom).into_iter().next().unwrap_or_default();
    if program.is_empty() {
        return Err("the custom terminal command is empty".into());
    }
    let as_path = Path::new(&program);
    if as_path.is_absolute() && as_path.is_file()
        || crate::env::which(&[program.as_str()], path).is_some()
    {
        Ok(())
    } else {
        Err(format!(
            "`{program}` (your custom terminal command) wasn't found"
        ))
    }
}

/// A macOS terminal's bundle identifier and `.app` folder name.
fn mac_bundle(app: TerminalApp) -> Option<(&'static str, &'static str)> {
    use TerminalApp::*;
    Some(match app {
        Terminal => ("com.apple.Terminal", "Terminal.app"),
        Iterm => ("com.googlecode.iterm2", "iTerm.app"),
        Ghostty => ("com.mitchellh.ghostty", "Ghostty.app"),
        Wezterm => ("com.github.wez.wezterm", "WezTerm.app"),
        Kitty => ("net.kovidgoyal.kitty", "kitty.app"),
        Alacritty => ("org.alacritty", "Alacritty.app"),
        _ => return None,
    })
}

/// Locate a macOS terminal app without launching or revealing it: the usual
/// Applications folders first, then a Spotlight lookup by bundle identifier
/// for apps installed anywhere else.
pub fn find_mac_app(app: TerminalApp) -> Option<std::path::PathBuf> {
    let (bundle_id, bundle) = mac_bundle(app)?;
    let mut folders = vec![
        std::path::PathBuf::from("/Applications"),
        "/Applications/Utilities".into(),
        "/System/Applications/Utilities".into(),
    ];
    if let Some(home) = dirs::home_dir() {
        folders.push(home.join("Applications"));
    }
    if let Some(found) = folders.iter().map(|d| d.join(bundle)).find(|p| p.is_dir()) {
        return Some(found);
    }
    if !cfg!(target_os = "macos") {
        return None;
    }
    let output = Command::new("mdfind")
        .arg(format!("kMDItemCFBundleIdentifier == '{bundle_id}'"))
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(std::path::PathBuf::from)
        .find(|p| p.extension().is_some_and(|e| e == "app") && p.is_dir())
}

/// AppleScript that opens `command` in a new Ghostty tab of the front window
/// (or a new window if none is open), titled `title`. Uses the scripting
/// dictionary Ghostty added in 1.3.
fn ghostty_tab_script(cwd: &str, title: &str, command: &str) -> Vec<String> {
    vec![
        "tell application \"Ghostty\"".into(),
        "activate".into(),
        "set cfg to new surface configuration".into(),
        format!(
            "set initial working directory of cfg to {}",
            applescript_string(cwd)
        ),
        format!("set command of cfg to {}", applescript_string(command)),
        "if (count of windows) > 0 then".into(),
        "set tb to new tab in front window with configuration cfg".into(),
        "else".into(),
        "set win to new window with configuration cfg".into(),
        "set tb to selected tab of win".into(),
        "end if".into(),
        format!(
            "perform action {} on focused terminal of tb",
            applescript_string(&format!("set_tab_title:{title}"))
        ),
        "end tell".into(),
    ]
}

/// Whether the Ghostty app at `app` is 1.3 or newer, i.e. scriptable.
fn ghostty_scriptable(app: &Path) -> bool {
    let plist = app.join("Contents/Info.plist");
    Command::new("plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(&plist)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| version_at_least(&String::from_utf8_lossy(&o.stdout), (1, 3)))
}

/// Compare a "major.minor[.patch]" version string against `(major, minor)`.
fn version_at_least(version: &str, min: (u32, u32)) -> bool {
    let mut parts = version
        .trim()
        .split('.')
        .map(|p| p.parse::<u32>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    (major, minor) >= min
}

fn osascript(lines: &[&str]) -> Launch {
    Launch::new("osascript", lines.iter().flat_map(|l| ["-e", l]))
}

/// `open -na <App> --args …` starts a new instance of a macOS app. Uses the
/// app's location when we can find it, so renamed or moved apps still work.
fn open_app(app: TerminalApp, args: impl IntoIterator<Item = String>) -> Launch {
    let target = find_mac_app(app)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            mac_bundle(app)
                .map_or("Terminal", |(_, b)| b)
                .trim_end_matches(".app")
                .to_owned()
        });
    Launch::new(
        "open",
        ["-na".to_string(), target, "--args".into()]
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
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
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

    fn target() -> Target {
        Target::attach(
            "3b9e1c07",
            "/Users/you/my projects/starship",
            "starship-export",
        )
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
            &Target::attach("abc", r"C:\dev\game", "n"),
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
            r#"my-term --title "{name}" -- {args}"#,
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
    fn new_session_prompt_survives_every_route() {
        let t = Target::new_session("/p", Some("fix shaders"), Some("it's red;\nfix it"));
        let claude = Path::new("/bin/claude");

        let ghostty = build_launch(TerminalApp::Ghostty, "", claude, &t).unwrap();
        assert_eq!(
            &ghostty.args[5..],
            ["/bin/claude", "--name", "fix shaders", "it's red;\nfix it"]
        );

        let terminal = build_launch(TerminalApp::Terminal, "", claude, &t).unwrap();
        assert!(terminal.args.contains(
            &r#"do script "cd /p && /bin/claude --name 'fix shaders' 'it'\\''s red;\nfix it'""#
                .to_string()
        ));

        let wt = build_launch(TerminalApp::WindowsTerminal, "", claude, &t).unwrap();
        assert_eq!(wt.args.last().unwrap(), "it's red\\;\nfix it");

        let ps = build_launch(TerminalApp::Powershell, "", claude, &t).unwrap();
        assert_eq!(
            ps.args[2],
            "& '/bin/claude' '--name' 'fix shaders' 'it''s red;\nfix it'"
        );
    }

    #[test]
    fn only_installed_presets_are_offered() {
        let empty = std::ffi::OsString::new();
        let list = installed(&empty);
        // Custom is always offered; nothing that needs PATH is found on an empty one.
        assert!(list.contains(&TerminalApp::Custom));
        assert!(!list.contains(&TerminalApp::XTerminalEmulator));
        assert!(!list.contains(&TerminalApp::WindowsTerminal));
        assert!(check_installed(TerminalApp::Custom, "nope-term {args}", &empty).is_err());
    }

    #[test]
    fn ghostty_tab_script_opens_a_titled_tab() {
        let script = ghostty_tab_script(
            "/Users/you/my projects/starship",
            "say \"hi\"",
            "/bin/claude attach 3b9e1c07",
        );
        assert_eq!(script[0], "tell application \"Ghostty\"");
        assert!(script.contains(
            &r#"set initial working directory of cfg to "/Users/you/my projects/starship""#
                .to_string()
        ));
        assert!(
            script.contains(&r#"set command of cfg to "/bin/claude attach 3b9e1c07""#.to_string())
        );
        assert!(script
            .contains(&"set tb to new tab in front window with configuration cfg".to_string()));
        assert!(script.contains(
            &r#"perform action "set_tab_title:say \"hi\"" on focused terminal of tb"#.to_string()
        ));
    }

    #[test]
    fn ghostty_version_check() {
        assert!(version_at_least("1.4.0", (1, 3)));
        assert!(version_at_least("1.3", (1, 3)));
        assert!(version_at_least("2.0.1\n", (1, 3)));
        assert!(!version_at_least("1.2.3", (1, 3)));
        assert!(!version_at_least("", (1, 3)));
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("abc-1"), "abc-1");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(split_args(r#"a "b c" 'd"e' ''"#), ["a", "b c", "d\"e", ""]);
    }
}
