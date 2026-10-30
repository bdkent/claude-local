//! Finding `claude` and `tmux` from a GUI process.
//!
//! Apps launched from Finder/Dock on macOS get a minimal PATH
//! (`/usr/bin:/bin:/usr/sbin:/sbin`), so Homebrew and `~/.local/bin` are
//! invisible. We ask the user's login shell for its PATH once and use it for
//! every child process.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
pub struct ToolEnv {
    /// PATH handed to every child process.
    pub path: OsString,
    pub claude: PathBuf,
}

impl ToolEnv {
    /// Resolve the environment. `claude_override` comes from the config file.
    pub fn discover(claude_override: Option<&Path>) -> Result<Self> {
        let path = login_path().unwrap_or_else(|| std::env::var_os("PATH").unwrap_or_default());
        let path = with_fallback_dirs(path);

        let claude = match claude_override {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => which(claude_names(), &path).ok_or_else(|| {
                anyhow!("could not find `claude` on PATH; set its location in Settings")
            })?,
        };
        Ok(Self { path, claude })
    }

    /// A command with PATH set and, on Windows, no console window flashing up.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut cmd = Command::new(program);
        cmd.env("PATH", &self.path);
        hide_console(&mut cmd);
        cmd
    }

    pub fn claude_command(&self) -> Command {
        self.command(&self.claude)
    }

    pub fn which(&self, names: &[&str]) -> Option<PathBuf> {
        which(names, &self.path)
    }
}

fn claude_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["claude.exe", "claude.cmd"]
    } else {
        &["claude"]
    }
}

pub fn which(names: &[&str], path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
}

fn with_fallback_dirs(path: OsString) -> OsString {
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    let mut extra = Vec::new();
    if let Some(home) = dirs::home_dir() {
        extra.push(home.join(".local/bin"));
        extra.push(home.join(".claude/local"));
    }
    if cfg!(unix) {
        extra.push("/opt/homebrew/bin".into());
        extra.push("/usr/local/bin".into());
    }
    for dir in extra {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).unwrap_or(path)
}

#[cfg(unix)]
fn login_path() -> Option<OsString> {
    use std::process::Stdio;
    use std::time::Duration;

    const START: &str = "__CLAUDE_LOCAL_PATH_START__";
    const END: &str = "__CLAUDE_LOCAL_PATH_END__";

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    // -i as well as -l: many people set PATH in .zshrc rather than .zprofile.
    let child = Command::new(shell)
        .args([
            "-l",
            "-i",
            "-c",
            &format!("printf '{START}%s{END}' \"$PATH\""),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let output = wait_with_timeout(child, Duration::from_secs(5)).ok()?;
    let stdout = String::from_utf8_lossy(&output);
    let start = stdout.find(START)? + START.len();
    let end = stdout[start..].find(END)? + start;
    Some(stdout[start..end].into())
}

#[cfg(not(unix))]
fn login_path() -> Option<OsString> {
    // GUI apps on Windows already inherit the user's PATH.
    None
}

#[cfg(unix)]
fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: std::time::Duration,
) -> Result<Vec<u8>> {
    use anyhow::Context;
    use std::io::Read;
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            let mut out = Vec::new();
            child
                .stdout
                .take()
                .context("no stdout")?
                .read_to_end(&mut out)?;
            return Ok(out);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err(anyhow!("login shell timed out"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_cmd: &mut Command) {}
