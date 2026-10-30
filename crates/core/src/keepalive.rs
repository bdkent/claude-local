//! Keeping background sessions from idling out.
//!
//! Claude Code idles a background session after 8 hours with no attached
//! client. We keep one hidden `claude attach <id>` running per background
//! session. Attaching a visible terminal alongside it is fine; sessions accept
//! more than one client.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Serialize;

use crate::env::ToolEnv;
use crate::session::Session;

/// Where hidden attach clients live.
pub trait Backend: Send {
    /// Attach ids that currently have a live keep-alive client.
    fn running(&mut self) -> Result<HashSet<String>>;
    fn start(&mut self, env: &ToolEnv, id: &str, cwd: &str) -> Result<()>;
    fn stop(&mut self, id: &str) -> Result<()>;
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Attach ids with a keep-alive after this pass.
    pub alive: HashSet<String>,
    pub started: Vec<String>,
    pub stopped: Vec<String>,
    pub errors: Vec<String>,
}

pub struct Keepalive {
    backend: Box<dyn Backend>,
    last_start: HashMap<String, Instant>,
    /// Don't restart a client that died more recently than this, so a session
    /// that refuses attaches doesn't get hammered every poll.
    retry_after: Duration,
}

impl Keepalive {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self {
            backend,
            last_start: HashMap::new(),
            retry_after: Duration::from_secs(60),
        }
    }

    /// Start a client for every background session that lacks one, and stop
    /// clients whose session has gone away.
    pub fn reconcile(&mut self, env: &ToolEnv, sessions: &[Session]) -> Report {
        let mut report = Report::default();
        let wanted: HashMap<&str, &Session> = sessions
            .iter()
            .filter_map(|s| s.attach_id().map(|id| (id, s)))
            .collect();

        let running = match self.backend.running() {
            Ok(r) => r,
            Err(e) => {
                report.errors.push(format!("listing keep-alives: {e:#}"));
                return report;
            }
        };

        for id in running
            .iter()
            .filter(|id| !wanted.contains_key(id.as_str()))
        {
            match self.backend.stop(id) {
                Ok(()) => report.stopped.push(id.clone()),
                Err(e) => report
                    .errors
                    .push(format!("stopping keep-alive {id}: {e:#}")),
            }
        }

        for (&id, session) in &wanted {
            if running.contains(id) {
                report.alive.insert(id.to_owned());
                continue;
            }
            if self
                .last_start
                .get(id)
                .is_some_and(|t| t.elapsed() < self.retry_after)
            {
                continue;
            }
            self.last_start.insert(id.to_owned(), Instant::now());
            match self.backend.start(env, id, &session.cwd) {
                Ok(()) => {
                    report.started.push(id.to_owned());
                    report.alive.insert(id.to_owned());
                }
                Err(e) => report.errors.push(format!("keeping {id} alive: {e:#}")),
            }
        }

        self.last_start
            .retain(|id, _| wanted.contains_key(id.as_str()));
        report
    }

    /// Stop every keep-alive client (used when the feature is switched off).
    pub fn stop_all(&mut self) -> Report {
        let mut report = Report::default();
        match self.backend.running() {
            Ok(running) => {
                for id in running {
                    match self.backend.stop(&id) {
                        Ok(()) => report.stopped.push(id),
                        Err(e) => report
                            .errors
                            .push(format!("stopping keep-alive {id}: {e:#}")),
                    }
                }
            }
            Err(e) => report.errors.push(format!("listing keep-alives: {e:#}")),
        }
        self.last_start.clear();
        report
    }
}

/// The platform's default backend: tmux on macOS/Linux, ConPTY on Windows.
pub fn default_backend(env: &ToolEnv) -> Result<Box<dyn Backend>> {
    #[cfg(unix)]
    {
        let tmux = env.which(&["tmux"]).ok_or_else(|| {
            anyhow::anyhow!("tmux not found; install it with `brew install tmux`")
        })?;
        Ok(Box::new(tmux::Tmux::new(tmux)))
    }
    #[cfg(windows)]
    {
        let _ = env;
        Ok(Box::new(conpty::ConPty::default()))
    }
}

#[cfg(unix)]
pub mod tmux {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::process::Command;

    use anyhow::{bail, Context, Result};

    use super::Backend;
    use crate::env::ToolEnv;
    use crate::terminal::shell_quote;

    /// A private tmux server, so keep-alives never show up in the user's own
    /// `tmux ls`. Inspect with `tmux -L claude-local ls`.
    pub const SOCKET: &str = "claude-local";
    const PREFIX: &str = "keepalive-";

    pub struct Tmux {
        tmux: PathBuf,
        socket: String,
    }

    impl Tmux {
        pub fn new(tmux: PathBuf) -> Self {
            Self::with_socket(tmux, SOCKET)
        }

        pub fn with_socket(tmux: PathBuf, socket: &str) -> Self {
            Self {
                tmux,
                socket: socket.into(),
            }
        }

        fn cmd(&self) -> Command {
            let mut cmd = Command::new(&self.tmux);
            cmd.args(["-L", &self.socket]);
            cmd
        }
    }

    pub fn session_name(id: &str) -> String {
        let safe: String = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{PREFIX}{safe}")
    }

    impl Backend for Tmux {
        fn running(&mut self) -> Result<HashSet<String>> {
            let output = self
                .cmd()
                .args(["list-sessions", "-F", "#{session_name}"])
                .output()
                .context("running tmux")?;
            // A non-zero exit here almost always means "no server running yet".
            if !output.status.success() {
                return Ok(HashSet::new());
            }
            Ok(String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|l| l.strip_prefix(PREFIX))
                .map(str::to_owned)
                .collect())
        }

        fn start(&mut self, env: &ToolEnv, id: &str, cwd: &str) -> Result<()> {
            let command = format!(
                "{} attach {}",
                shell_quote(&env.claude.to_string_lossy()),
                shell_quote(id)
            );
            let output = self
                .cmd()
                .env("PATH", &env.path)
                .args(["new-session", "-d", "-x", "200", "-y", "50"])
                .args(["-s", &session_name(id), "-c", cwd, &command])
                .output()
                .context("running tmux")?;
            if !output.status.success() {
                bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
            }
            Ok(())
        }

        fn stop(&mut self, id: &str) -> Result<()> {
            let target = format!("={}", session_name(id));
            let output = self.cmd().args(["kill-session", "-t", &target]).output()?;
            if !output.status.success() {
                bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        /// Drives a real tmux server with a stand-in `claude` that just sleeps.
        #[test]
        fn start_list_stop_against_real_tmux() {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let Some(tmux) = crate::env::which(&["tmux"], &path) else {
                eprintln!("tmux not installed; skipping");
                return;
            };
            let dir =
                std::env::temp_dir().join(format!("claude-local-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let fake = dir.join("fake claude");
            std::fs::write(&fake, "#!/bin/sh\nsleep 30\n").unwrap();
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

            let env = ToolEnv { path, claude: fake };
            let socket = format!("claude-local-test-{}", std::process::id());
            let mut backend = Tmux::with_socket(tmux, &socket);

            assert!(backend.running().unwrap().is_empty());
            backend
                .start(&env, "abc123", dir.to_str().unwrap())
                .unwrap();
            assert_eq!(
                backend.running().unwrap(),
                HashSet::from(["abc123".to_string()])
            );
            backend.stop("abc123").unwrap();
            assert!(backend.running().unwrap().is_empty());

            let _ = backend.cmd().arg("kill-server").output();
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(windows)]
pub mod conpty {
    //! Windows has no tmux, so the app holds each hidden client in a ConPTY.
    //! Unlike tmux these die with the app, which is why the app should run at
    //! login.

    use std::collections::{HashMap, HashSet};
    use std::io::Read;

    use anyhow::{anyhow, Result};
    use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

    use super::Backend;
    use crate::env::ToolEnv;

    struct Client {
        child: Box<dyn Child + Send + Sync>,
        _master: Box<dyn MasterPty + Send>,
    }

    #[derive(Default)]
    pub struct ConPty {
        clients: HashMap<String, Client>,
    }

    impl Backend for ConPty {
        fn running(&mut self) -> Result<HashSet<String>> {
            self.clients
                .retain(|_, c| matches!(c.child.try_wait(), Ok(None)));
            Ok(self.clients.keys().cloned().collect())
        }

        fn start(&mut self, env: &ToolEnv, id: &str, cwd: &str) -> Result<()> {
            let pair = native_pty_system()
                .openpty(PtySize {
                    rows: 50,
                    cols: 200,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| anyhow!("{e}"))?;

            let mut cmd = if crate::terminal::is_batch_file(&env.claude) {
                let mut c = CommandBuilder::new("cmd.exe");
                c.arg("/c");
                c.arg(&env.claude);
                c
            } else {
                CommandBuilder::new(&env.claude)
            };
            cmd.args(["attach", id]);
            cmd.cwd(cwd);
            cmd.env("PATH", &env.path);

            let child = pair.slave.spawn_command(cmd).map_err(|e| anyhow!("{e}"))?;
            drop(pair.slave);

            // Drain output; a full pipe would stall the client.
            let mut reader = pair.master.try_clone_reader().map_err(|e| anyhow!("{e}"))?;
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                while matches!(reader.read(&mut buf), Ok(n) if n > 0) {}
            });

            self.clients.insert(
                id.to_owned(),
                Client {
                    child,
                    _master: pair.master,
                },
            );
            Ok(())
        }

        fn stop(&mut self, id: &str) -> Result<()> {
            if let Some(mut client) = self.clients.remove(id) {
                client.child.kill()?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::parse_agents_json;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Fake {
        running: Arc<Mutex<HashSet<String>>>,
        fail_start: bool,
    }

    impl Backend for Fake {
        fn running(&mut self) -> Result<HashSet<String>> {
            Ok(self.running.lock().unwrap().clone())
        }
        fn start(&mut self, _: &ToolEnv, id: &str, _: &str) -> Result<()> {
            if self.fail_start {
                anyhow::bail!("nope");
            }
            self.running.lock().unwrap().insert(id.into());
            Ok(())
        }
        fn stop(&mut self, id: &str) -> Result<()> {
            self.running.lock().unwrap().remove(id);
            Ok(())
        }
    }

    fn env() -> ToolEnv {
        ToolEnv {
            path: Default::default(),
            claude: "claude".into(),
        }
    }

    fn sessions(ids: &[&str]) -> Vec<Session> {
        let mut entries: Vec<String> = ids
            .iter()
            .map(|id| {
                format!(
                    r#"{{"pid":1,"id":"{id}","cwd":"/p","kind":"background","startedAt":0,"sessionId":"{id}-full"}}"#
                )
            })
            .collect();
        entries.push(
            r#"{"pid":2,"cwd":"/p","kind":"interactive","startedAt":0,"sessionId":"i"}"#.into(),
        );
        parse_agents_json(&format!("[{}]", entries.join(","))).unwrap()
    }

    #[test]
    fn starts_missing_and_stops_stale() {
        let running = Arc::new(Mutex::new(HashSet::from([
            "gone".to_string(),
            "a".to_string(),
        ])));
        let mut ka = Keepalive::new(Box::new(Fake {
            running: running.clone(),
            fail_start: false,
        }));

        let report = ka.reconcile(&env(), &sessions(&["a", "b"]));

        assert_eq!(report.started, vec!["b".to_string()]);
        assert_eq!(report.stopped, vec!["gone".to_string()]);
        assert_eq!(
            report.alive,
            HashSet::from(["a".to_string(), "b".to_string()])
        );
        assert_eq!(
            *running.lock().unwrap(),
            HashSet::from(["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn interactive_sessions_are_ignored() {
        let mut ka = Keepalive::new(Box::new(Fake::default()));
        let report = ka.reconcile(&env(), &sessions(&[]));
        assert!(report.alive.is_empty() && report.started.is_empty());
    }

    #[test]
    fn failed_start_is_not_retried_immediately() {
        let mut ka = Keepalive::new(Box::new(Fake {
            fail_start: true,
            ..Default::default()
        }));
        let first = ka.reconcile(&env(), &sessions(&["a"]));
        assert_eq!(first.errors.len(), 1);
        let second = ka.reconcile(&env(), &sessions(&["a"]));
        assert!(second.errors.is_empty() && second.started.is_empty());
    }
}
