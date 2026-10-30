use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use claude_local_core::{
    claude, keepalive, terminal, Config, Keepalive, Session, SessionKind, Target, ToolEnv,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::TRAY_ID;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    #[serde(flatten)]
    pub session: Session,
    pub project: String,
    pub attachable: bool,
    pub kept_alive: bool,
}

/// Everything the list window renders; pushed as the `snapshot` event.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub sessions: Vec<SessionView>,
    pub error: Option<String>,
    pub keepalive_enabled: bool,
    pub keepalive_errors: Vec<String>,
    /// Unix time in milliseconds.
    pub updated_at: u64,
}

pub struct AppState {
    pub visible: AtomicBool,
    config: Mutex<Config>,
    /// Cached tool discovery; it runs a login shell, so we don't redo it every poll.
    env: Mutex<Option<Result<ToolEnv, String>>>,
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
            config: Mutex::new(config),
            env: Mutex::new(None),
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

    fn tool_env(&self, rediscover_failed: bool) -> Result<ToolEnv, String> {
        let mut env = lock(&self.env);
        let stale = match &*env {
            None => true,
            Some(Err(_)) => rediscover_failed,
            Some(Ok(_)) => false,
        };
        if stale {
            let config = self.config();
            let claude = (!config.claude_path.is_empty()).then(|| Path::new(&config.claude_path));
            *env = Some(ToolEnv::discover(claude).map_err(|e| format!("{e:#}")));
        }
        env.clone().expect("set above")
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
            updated_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or_default(),
            ..Default::default()
        };

        match self.tool_env(rediscover) {
            Err(e) => snap.error = Some(e),
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
                            Err(e) => snap.keepalive_errors.push(format!("{e:#}")),
                        }
                    }
                    snap.sessions = sessions
                        .into_iter()
                        .map(|s| SessionView {
                            project: s.project(),
                            attachable: s.attach_id().is_some(),
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
        let target = Target {
            id,
            cwd: &view.session.cwd,
            name: view.session.display_name(),
        };
        terminal::build_launch(
            config.terminal,
            &config.custom_terminal,
            &env.claude,
            &target,
        )?
        .spawn(&env)
    }

    pub fn set_config(&self, new: Config) -> Result<()> {
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
        Some(_) => "Claude Local: can't list sessions".to_string(),
        None => format!(
            "Claude Local: {} background, {} interactive",
            count(SessionKind::Background),
            count(SessionKind::Interactive)
        ),
    };
    let _ = tray.set_tooltip(Some(tooltip));
}

pub fn spawn_poller(app: AppHandle, state: Arc<AppState>) {
    std::thread::spawn(move || {
        let mut woken = false;
        loop {
            state.refresh(&app, woken);
            let config = state.config();
            let secs = if state.visible.load(Ordering::SeqCst) {
                config.poll_visible_secs
            } else {
                config.poll_hidden_secs
            };
            woken = state.wait(Duration::from_secs(secs.max(1)));
        }
    });
}
