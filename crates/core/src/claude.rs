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

/// Result of `claude rm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    Removed(String),
    /// Claude refused because deleting would lose work (e.g. unpushed commits
    /// in the session's worktree). `flags` are the override flags, with the
    /// values Claude reported, that would make it go ahead.
    NeedsConfirmation {
        message: String,
        flags: Vec<String>,
    },
}

/// Flags `claude rm` reports when it refuses to delete a worktree.
const OVERRIDE_FLAGS: &[&str] = &["--discard-unpushed", "--force-remove-worktree"];

/// Delete a background session with `claude rm <id>`, plus any override
/// `flags` from an earlier `NeedsConfirmation`.
pub fn remove_session(env: &ToolEnv, id: &str, flags: &[String]) -> Result<Removal> {
    let output = env
        .claude_command()
        .arg("rm")
        .arg(id)
        .args(flags)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {}", env.claude.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let message = [stdout.as_str(), stderr.as_str()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n");

    // Claude may report override flags whether or not it exits non-zero, so
    // look for them first.
    let reported = override_flags(&message);
    if !reported.is_empty() && reported != flags {
        return Ok(Removal::NeedsConfirmation {
            message,
            flags: reported,
        });
    }
    if !output.status.success() {
        bail!(
            "{}",
            if message.is_empty() {
                format!("`claude rm` exited with {}", output.status)
            } else {
                message
            }
        );
    }
    Ok(Removal::Removed(message))
}

/// Pull `--discard-unpushed <value>` / `--force-remove-worktree <value>`
/// pairs out of Claude's message, tolerating quotes around the value.
fn override_flags(message: &str) -> Vec<String> {
    let clean = |w: &str| {
        w.trim_matches(|c: char| "`'\"(),.;".contains(c))
            .to_string()
    };
    // Split `--flag=value` so both spellings look like `--flag value`.
    let words: Vec<String> = message
        .split_whitespace()
        .flat_map(|w| {
            let w = clean(w);
            match w.split_once('=') {
                Some((flag, value)) if OVERRIDE_FLAGS.contains(&flag) => {
                    vec![flag.to_string(), clean(value)]
                }
                _ => vec![w],
            }
        })
        .collect();
    let mut flags: Vec<String> = Vec::new();
    for pair in words.windows(2) {
        let (flag, value) = (&pair[0], &pair[1]);
        if OVERRIDE_FLAGS.contains(&flag.as_str()) && !value.is_empty() && !value.starts_with('-') {
            let pair = [flag.clone(), value.clone()];
            if !flags.chunks(2).any(|c| c == pair) {
                flags.extend(pair);
            }
        }
    }
    flags
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

    #[test]
    fn override_flags_are_parsed_from_claude_messages() {
        let msg = "Worktree has 2 unpushed commits. To discard them, run `claude rm c42d8a15 --discard-unpushed 1a2b3c@wt-9`.";
        assert_eq!(override_flags(msg), ["--discard-unpushed", "1a2b3c@wt-9"]);
        assert!(override_flags("Removed c42d8a15").is_empty());
        assert!(override_flags("see --discard-unpushed --help").is_empty());
        assert_eq!(
            override_flags(
                "Pass '--force-remove-worktree=wt-3' and --discard-unpushed=\"ab@wt-3\"."
            ),
            [
                "--force-remove-worktree",
                "wt-3",
                "--discard-unpushed",
                "ab@wt-3"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn remove_session_asks_before_discarding_work() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("claude-local-rm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("claude");
        // Refuses until given the override flag it reported.
        std::fs::write(
            &fake,
            "#!/bin/sh\n[ \"$3\" = --discard-unpushed ] && [ \"$4\" = c0ffee@wt-1 ] && { echo \"Removed $2\"; exit 0; }\necho \"Worktree has unpushed commits; pass --discard-unpushed c0ffee@wt-1 to discard them.\" >&2; exit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = ToolEnv {
            path: std::env::var_os("PATH").unwrap_or_default(),
            claude: fake,
        };

        let Removal::NeedsConfirmation { message, flags } =
            remove_session(&env, "abc", &[]).unwrap()
        else {
            panic!("expected a confirmation request");
        };
        assert!(message.contains("unpushed"));
        assert_eq!(flags, ["--discard-unpushed", "c0ffee@wt-1"]);

        assert_eq!(
            remove_session(&env, "abc", &flags).unwrap(),
            Removal::Removed("Removed abc".into())
        );
        let _ = std::fs::remove_dir_all(dir);
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
