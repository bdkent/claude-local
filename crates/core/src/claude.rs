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
