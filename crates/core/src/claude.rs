use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context, Result};

use crate::env::ToolEnv;
use crate::session::{parse_agents_json, Session};

/// Run `claude agents --json` and parse the result.
pub fn list_sessions(env: &ToolEnv) -> Result<Vec<Session>> {
    let output = env
        .claude_command()
        .args(["agents", "--json"])
        .output()
        .with_context(|| format!("running {}", env.claude.display()))?;
    if !output.status.success() {
        bail!(
            "`claude agents --json` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_agents_json(stdout.trim()).context("parsing `claude agents --json` output")
}

/// Arguments after `claude` that start a new session. Blank values are left out.
pub fn new_session_args(name: Option<&str>, prompt: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
        args.extend(["--name".to_string(), name.to_string()]);
    }
    if let Some(prompt) = prompt.filter(|p| !p.trim().is_empty()) {
        // A prompt that starts with `-` would otherwise be read as a flag.
        if prompt.starts_with('-') {
            args.push("--".into());
        }
        args.push(prompt.to_string());
    }
    args
}

/// Start a background session with `claude --bg` in `cwd`. Returns what
/// `claude` printed (it includes the new session's id).
pub fn start_background(
    env: &ToolEnv,
    cwd: &str,
    name: Option<&str>,
    prompt: Option<&str>,
) -> Result<String> {
    if !Path::new(cwd).is_dir() {
        bail!("{cwd} is not a folder");
    }
    let output = env
        .claude_command()
        .arg("--bg")
        .args(new_session_args(name, prompt))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {}", env.claude.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!("{}", if stderr.is_empty() { stdout } else { stderr });
    }
    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_args_skip_blanks_and_guard_dashes() {
        assert!(new_session_args(Some("  "), Some("\n")).is_empty());
        assert_eq!(
            new_session_args(Some(" export "), Some("build it")),
            ["--name", "export", "build it"]
        );
        assert_eq!(
            new_session_args(None, Some("-v is broken")),
            ["--", "-v is broken"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn start_background_reports_claude_errors() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("claude-local-bg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("claude");
        // Echo the args on success; fail like an untrusted workspace otherwise.
        std::fs::write(
            &fake,
            "#!/bin/sh\n[ \"$2\" = --name ] && { echo \"started $*\"; exit 0; }\necho 'Workspace not trusted.' >&2; exit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = ToolEnv {
            path: std::env::var_os("PATH").unwrap_or_default(),
            claude: fake,
        };
        let cwd = dir.to_str().unwrap();

        let ok = start_background(&env, cwd, Some("n"), Some("go")).unwrap();
        assert_eq!(ok, "started --bg --name n go");

        let err = start_background(&env, cwd, None, Some("go")).unwrap_err();
        assert_eq!(err.to_string(), "Workspace not trusted.");

        assert!(start_background(&env, "/definitely/not/here", None, None).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
