use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use claude_local_core::{
    claude, doctor, env, keepalive, terminal, Check, Config, Dot, DotColor, Keepalive, Session,
    SessionKind, Target, TerminalApp, ToolEnv,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::TRAY_ID;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    #[serde(flatten)]
    pub session: Session,
    pub project: String,
    pub attachable: bool,
    /// Has a live process; a stopped background session doesn't.
    pub running: bool,
    /// The status dot: colour from `state`, pulsing while `status` is busy.
    pub dot: Dot,
    pub kept_alive: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSession {
    pub cwd: String,
    pub name: String,
    pub prompt: String,
    pub background: bool,
}

/// Outcome of deleting a session, for the confirmation dialog.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum DeleteResult {
    Deleted {
        message: String,
    },
    /// Claude refused because work would be lost; ask again before discarding.
    NeedsDiscard {
        message: String,
    },
}

/// Everything the list window renders; pushed as the `snapshot` event.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub sessions: Vec<SessionView>,
    pub error: Option<String>,
    pub keepalive_enabled: bool,
    pub keepalive_errors: Vec<String>,
    /// Dependency checks; failing ones are shown as a "Setup needed" card.
    pub checks: Vec<Check>,
    /// The configured terminal's name, for "Open in …" tooltips.
    pub terminal_label: String,
    /// Unix time in milliseconds.
    pub updated_at: u64,
}

pub struct AppState {
    pub visible: AtomicBool,
    /// When the list window last lost focus, for the tray click toggle.
    pub focus_lost_at: Mutex<Option<std::time::Instant>>,
    config: Mutex<Config>,
    /// The login shell's PATH. Finding it starts a shell, so it's cached.
    path: Mutex<Option<OsString>>,
    env: Mutex<Option<Result<ToolEnv, String>>>,
    checks: Mutex<Option<Vec<Check>>>,
    /// Override flags `claude rm` reported per session, kept here so the UI
    /// can only confirm them, never supply its own.
    pending_rm: Mutex<HashMap<String, Vec<String>>>,
    keepalive: Mutex<Option<Keepalive>>,
    snapshot: Mutex<Snapshot>,
    wake: (Mutex<bool>, Condvar),
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self {
            visible: AtomicBool::new(false),
            focus_lost_at: Mutex::new(None),
            config: Mutex::new(config),
            path: Mutex::new(None),
            env: Mutex::new(None),
            checks: Mutex::new(None),
            pending_rm: Mutex::new(HashMap::new()),
            keepalive: Mutex::new(None),
            snapshot: Mutex::new(Snapshot::default()),
            wake: (Mutex::new(false), Condvar::new()),
        }
    }

    pub fn config(&self) -> Config {
        lock(&self.config).clone()
    }

    pub fn snapshot(&self) -> Snapshot {
        lock(&self.snapshot).clone()
    }

    /// Resolved `claude` path or the reason it couldn't be found.
    pub fn env_status(&self) -> (Option<String>, Option<String>) {
        match &*lock(&self.env) {
            Some(Ok(env)) => (Some(env.claude.display().to_string()), None),
            Some(Err(e)) => (None, Some(e.clone())),
            None => (None, None),
        }
    }

    /// Ask the poller to refresh now (and retry tool discovery if it failed).
    pub fn wake(&self) {
        let (flag, cv) = &self.wake;
        *lock(flag) = true;
        cv.notify_one();
    }

    /// Sleep until `timeout` passes or `wake` is called. Returns true if woken.
    fn wait(&self, timeout: Duration) -> bool {
        let (flag, cv) = &self.wake;
        let guard = lock(flag);
        let (mut guard, _) = cv
            .wait_timeout_while(guard, timeout, |woken| !*woken)
            .unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *guard)
    }

    /// The search PATH, re-read from the login shell when `refresh` is set.
    fn search_path(&self, refresh: bool) -> OsString {
        let mut path = lock(&self.path);
        if refresh || path.is_none() {
            *path = Some(env::search_path());
        }
        path.clone().expect("set above")
    }

    /// Terminal presets installed on this machine, plus Custom.
    pub fn installed_terminals(&self) -> Vec<TerminalApp> {
        terminal::installed(&self.search_path(false))
    }

    /// Find `claude`. Once found it's cached; until then every call looks
    /// again (cheaply), and `rediscover` also re-reads the login shell's PATH.
    fn tool_env(&self, rediscover: bool) -> Result<ToolEnv, String> {
        let mut env = lock(&self.env);
        if let Some(Ok(found)) = &*env {
            return Ok(found.clone());
        }
        let path = self.search_path(rediscover && env.is_some());
        let config = self.config();
        let claude = (!config.claude_path.is_empty()).then(|| Path::new(&config.claude_path));
        let found = ToolEnv::from_path(path, claude).map_err(|e| format!("{e:#}"));
        *env = Some(found.clone());
        found
    }

    /// Dependency checks. Re-run when forced or while any are failing, so
    /// installing the missing tool clears the warning by itself.
    fn checks(&self, env: &Result<ToolEnv, String>, config: &Config, force: bool) -> Vec<Check> {
        let mut cached = lock(&self.checks);
        let stale = force || cached.as_ref().is_none_or(|c| c.iter().any(|c| !c.ok));
        if stale {
            *cached = Some(doctor::run(env, &self.search_path(false), config));
        }
        cached.clone().expect("set above")
    }

    fn with_keepalive<T>(&self, env: &ToolEnv, f: impl FnOnce(&mut Keepalive) -> T) -> Result<T> {
        let mut ka = lock(&self.keepalive);
        if ka.is_none() {
            *ka = Some(Keepalive::new(keepalive::default_backend(env)?));
        }
        Ok(f(ka.as_mut().expect("set above")))
    }

    pub fn refresh(&self, app: &AppHandle, rediscover: bool) {
        let config = self.config();
        let mut snap = Snapshot {
            keepalive_enabled: config.keepalive,
            terminal_label: match config.terminal {
                TerminalApp::Custom => "your terminal".into(),
                app => app.label().into(),
            },
            updated_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or_default(),
            ..Default::default()
        };

        let env = self.tool_env(rediscover);
        snap.checks = self.checks(&env, &config, rediscover);
        let tmux_missing = snap.checks.iter().any(|c| c.name == "tmux" && !c.ok);

        match env {
            // The "Setup needed" card explains a missing `claude`.
            Err(_) => {}
            // Never reconcile on a failed listing: an empty list would tear
            // down every keep-alive.
            Ok(env) => match claude::list_sessions(&env) {
                Err(e) => snap.error = Some(format!("{e:#}")),
                Ok(sessions) => {
                    let mut alive = HashSet::new();
                    if config.keepalive {
                        match self.with_keepalive(&env, |ka| ka.reconcile(&env, &sessions)) {
                            Ok(report) => {
                                alive = report.alive;
                                snap.keepalive_errors = report.errors;
                            }
                            // A missing tmux is already explained by the setup card.
                            Err(_) if tmux_missing => {}
                            Err(e) => snap.keepalive_errors.push(format!("{e:#}")),
                        }
                    }
                    snap.sessions = sessions
                        .into_iter()
                        .map(|s| SessionView {
                            project: s.project(),
                            attachable: s.attach_id().is_some(),
                            running: s.is_running(),
                            dot: s.dot(),
                            kept_alive: s.attach_id().is_some_and(|id| alive.contains(id)),
                            session: s,
                        })
                        .collect();
                    snap.sessions.sort_by(|a, b| {
                        (a.project.to_lowercase(), a.session.display_name())
                            .cmp(&(b.project.to_lowercase(), b.session.display_name()))
                    });
                }
            },
        }

        update_tray(app, &snap);
        *lock(&self.snapshot) = snap.clone();
        let _ = app.emit("snapshot", &snap);
    }

    pub fn open_session(&self, id: &str) -> Result<()> {
        let snap = self.snapshot();
        let view = snap
            .sessions
            .iter()
            .find(|v| v.session.attach_id() == Some(id))
            .ok_or_else(|| anyhow!("session {id} is not running in the background"))?;
        let env = self.tool_env(true).map_err(|e| anyhow!(e))?;
        let config = self.config();
        let target = Target::attach(id, &view.session.cwd, view.session.display_name());
        terminal::build_launch(
            config.terminal,
            &config.custom_terminal,
            &env.claude,
            &target,
        )?
        .spawn(&env)
    }

    /// Start a session in `cwd`: in the background via `claude --bg`, or
    /// interactively in the user's terminal. Returns a short status message.
    pub fn start_session(&self, req: &NewSession) -> Result<String> {
        let env = self.tool_env(true).map_err(|e| anyhow!(e))?;
        let config = self.config();
        let name = Some(req.name.as_str());
        let prompt = Some(req.prompt.as_str());
        let message = if req.background {
            let out = claude::start_background(&env, &req.cwd, name, prompt)?;
            self.wake();
            if out.is_empty() {
                "Started in the background".to_string()
            } else {
                out
            }
        } else {
            let target = Target::new_session(&req.cwd, name, prompt);
            terminal::build_launch(
                config.terminal,
                &config.custom_terminal,
                &env.claude,
                &target,
            )?
            .spawn(&env)?;
            "Opened in your terminal".to_string()
        };

        let mut config = lock(&self.config);
        config.remember_project(&req.cwd);
        // Failing to save the recent list shouldn't fail the start.
        let _ = config.save();
        Ok(message)
    }

    /// Folders to offer in the New Session dialog: recent ones first, then
    /// any other folder that currently has a session.
    pub fn project_choices(&self) -> Vec<String> {
        let mut out = self.config().recent_projects;
        for v in self.snapshot().sessions {
            if !out.contains(&v.session.cwd) {
                out.push(v.session.cwd);
            }
        }
        out
    }

    /// Delete a background session with `claude rm`. With `discard`, pass the
    /// override flags Claude reported on the previous attempt.
    pub fn delete_session(&self, id: &str, discard: bool) -> Result<DeleteResult> {
        let env = self.tool_env(true).map_err(|e| anyhow!(e))?;
        if self
            .snapshot()
            .sessions
            .iter()
            .all(|v| v.session.attach_id() != Some(id))
        {
            return Err(anyhow!("session {id} is not a background session"));
        }
        // Let go of our hidden client first. If the delete fails, the next
        // refresh re-attaches it.
        if self.config().keepalive {
            let _ = self.with_keepalive(&env, |ka| ka.release(id));
        }

        let flags = if discard {
            lock(&self.pending_rm).remove(id).unwrap_or_default()
        } else {
            Vec::new()
        };
        let result = claude::remove_session(&env, id, &flags);
        self.wake();
        match result? {
            claude::Removal::Removed(message) => {
                lock(&self.pending_rm).remove(id);
                Ok(DeleteResult::Deleted { message })
            }
            claude::Removal::NeedsConfirmation { message, flags } => {
                lock(&self.pending_rm).insert(id.to_owned(), flags);
                Ok(DeleteResult::NeedsDiscard { message })
            }
        }
    }

    pub fn set_config(&self, mut new: Config) -> Result<()> {
        // The settings form doesn't edit the recent list; keep the live one.
        new.recent_projects = self.config().recent_projects;
        new.save()?;
        let old = std::mem::replace(&mut *lock(&self.config), new.clone());
        if old.claude_path != new.claude_path {
            *lock(&self.env) = None;
        }
        if old.keepalive && !new.keepalive {
            let env = self.tool_env(true).map_err(|e| anyhow!(e))?;
            let report = self.with_keepalive(&env, |ka| ka.stop_all())?;
            if let Some(e) = report.errors.first() {
                return Err(anyhow!(e.clone())).context("stopping keep-alives");
            }
        }
        self.wake();
        Ok(())
    }
}

fn update_tray(app: &AppHandle, snap: &Snapshot) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let count = |kind| {
        snap.sessions
            .iter()
            .filter(|s| s.session.kind == kind)
            .count()
    };
    let tooltip = match &snap.error {
        _ if snap.checks.iter().any(|c| !c.ok) => "Claude Local: setup needed".to_string(),
        Some(_) => "Claude Local: can't list sessions".to_string(),
        None => {
            let waiting = snap
                .sessions
                .iter()
                .filter(|s| s.dot.color == DotColor::Orange)
                .count();
            let waiting = if waiting > 0 {
                format!(" · {waiting} blocked")
            } else {
                String::new()
            };
            format!(
                "Claude Local: {} background, {} interactive{waiting}",
                count(SessionKind::Background),
                count(SessionKind::Interactive)
            )
        }
    };
    let _ = tray.set_tooltip(Some(tooltip));
}

pub fn spawn_poller(app: AppHandle, state: Arc<AppState>) {
    std::thread::spawn(move || {
        state.refresh(&app, false);
        // Nothing else would surface a missing dependency in a tray-only app,
        // so open the list to show the setup card.
        if state.snapshot().checks.iter().any(|c| !c.ok) {
            crate::show_window(&app);
        }
        loop {
            let woken = state.wait(Duration::from_secs(poll_secs(&state)));
            state.refresh(&app, woken);
        }
    });
}

fn poll_secs(state: &AppState) -> u64 {
    let config = state.config();
    let secs = if state.visible.load(Ordering::SeqCst) {
        config.poll_visible_secs
    } else {
        config.poll_hidden_secs
    };
    secs.max(1)
}
