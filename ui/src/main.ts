import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";

type Kind = "interactive" | "background" | "other";

interface SessionView {
  pid: number;
  id?: string;
  cwd: string;
  kind: Kind;
  startedAt: number;
  sessionId: string;
  name?: string;
  status?: string;
  state?: string;
  project: string;
  attachable: boolean;
  keptAlive: boolean;
}

interface Check {
  name: string;
  ok: boolean;
  summary: string;
  why: string;
  fix?: string;
  hint?: string;
}

interface Snapshot {
  sessions: SessionView[];
  error?: string;
  keepaliveEnabled: boolean;
  keepaliveErrors: string[];
  checks: Check[];
  updatedAt: number;
}

interface Config {
  terminal: string;
  customTerminal: string;
  keepalive: boolean;
  claudePath: string;
  pollVisibleSecs: number;
  pollHiddenSecs: number;
  newSessionBackground: boolean;
  recentProjects: string[];
}

interface SettingsView {
  config: Config;
  terminals: { value: string; label: string }[];
  claudeResolved?: string;
  envError?: string;
  autostart: boolean;
  configPath?: string;
}

type KindFilter = "all" | "background" | "interactive";

const $ = <T extends HTMLElement>(sel: string) => document.querySelector(sel) as T;

// Per-viewer UI preferences; storage can be unavailable, so never depend on it.
const prefs = {
  load<T>(key: string, fallback: T): T {
    try {
      const raw = localStorage.getItem(key);
      return raw ? (JSON.parse(raw) as T) : fallback;
    } catch {
      return fallback;
    }
  },
  save(key: string, value: unknown) {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch {
      /* ignore */
    }
  },
};

let snapshot: Snapshot | null = null;
let kindFilter: KindFilter = prefs.load("kind", "all");
let projectFilter: string = prefs.load("project", "");
let search = "";
// A short-lived message in the footer, e.g. after starting a session.
let flash: { text: string; until: number } | null = null;
const collapsed = new Set<string>(prefs.load<string[]>("collapsed", []));

function ago(ms: number): string {
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function visibleSessions(all: SessionView[]): SessionView[] {
  const q = search.trim().toLowerCase();
  return all.filter(
    (s) =>
      (kindFilter === "all" || s.kind === kindFilter) &&
      (!projectFilter || s.project === projectFilter) &&
      (!q || (s.name ?? s.sessionId).toLowerCase().includes(q) || s.project.toLowerCase().includes(q)),
  );
}

async function openSession(s: SessionView) {
  if (!s.attachable || !s.id) return;
  try {
    await invoke("open_session", { id: s.id });
  } catch (e) {
    showBanner(String(e), "error");
  }
}

function sessionRow(s: SessionView): HTMLLIElement {
  const status = s.status ?? "unknown";
  const dotClass = status === "busy" ? "busy" : status === "idle" ? "idle" : "unknown";
  const meta = [s.kind === "background" ? "bg" : s.kind, s.state ?? status, ago(s.startedAt)].join(" · ");

  const row = el(
    "li",
    {
      className: `session ${s.attachable ? "attachable" : "inert"}`,
      tabIndex: 0,
      title: s.attachable
        ? `${s.cwd}\nDouble-click to open in your terminal`
        : `${s.cwd}\nInteractive sessions live in the terminal that started them`,
    },
    el("span", { className: `dot ${dotClass}`, title: status }),
    el(
      "span",
      { className: "label" },
      el("span", { className: "name" }, s.name ?? s.sessionId.slice(0, 8)),
      el("span", { className: "meta" }, meta),
    ),
  );
  if (s.keptAlive) {
    row.append(el("span", { className: "keepalive", title: "Kept alive" }, "♥"));
  }
  if (s.attachable) {
    const open = el("button", { className: "open", textContent: "Open", title: "Open in terminal" });
    open.addEventListener("click", (e) => {
      e.stopPropagation();
      openSession(s);
    });
    row.append(open);
    row.addEventListener("dblclick", () => openSession(s));
    row.addEventListener("keydown", (e) => {
      if (e.key === "Enter") openSession(s);
    });
  }
  return row;
}

function render() {
  if (!snapshot) return;
  const { sessions } = snapshot;

  // Project dropdown: keep the selection even if that project has no sessions right now.
  const projects = [...new Set(sessions.map((s) => s.project))].sort((a, b) => a.localeCompare(b));
  if (projectFilter && !projects.includes(projectFilter)) projects.push(projectFilter);
  const select = $<HTMLSelectElement>("#project");
  // Only rebuild when the list changes, so a poll doesn't close an open dropdown.
  const key = projects.join("\n");
  if (select.dataset.key !== key) {
    select.dataset.key = key;
    select.replaceChildren(
      el("option", { value: "", textContent: "All projects" }),
      ...projects.map((p) => el("option", { value: p, textContent: p })),
    );
  }
  select.value = projectFilter;

  const shown = visibleSessions(sessions);
  const groups = new Map<string, SessionView[]>();
  for (const s of shown) groups.set(s.project, [...(groups.get(s.project) ?? []), s]);

  $("#groups").replaceChildren(
    ...[...groups].map(([project, items]) => {
      const isCollapsed = collapsed.has(project);
      const header = el(
        "button",
        { className: "group-header", ariaExpanded: String(!isCollapsed) },
        el("span", { className: "caret" }, isCollapsed ? "▸" : "▾"),
        `${project} `,
        el("span", { className: "count" }, `(${items.length})`),
      );
      header.addEventListener("click", () => {
        if (collapsed.has(project)) collapsed.delete(project);
        else collapsed.add(project);
        prefs.save("collapsed", [...collapsed]);
        render();
      });
      const list = el("ul", { className: "sessions" }, ...(isCollapsed ? [] : items.map(sessionRow)));
      return el("li", { className: "group" }, header, list);
    }),
  );

  const failing = snapshot.checks.filter((c) => !c.ok);
  renderSetup(failing);

  const empty = $("#empty");
  empty.hidden = shown.length > 0 || !!snapshot.error || failing.some((c) => c.name === "claude");
  empty.textContent = sessions.length === 0 ? "No Claude sessions running." : "No sessions match these filters.";

  if (snapshot.error) showBanner(snapshot.error, "error");
  else if (snapshot.keepaliveErrors.length) showBanner(snapshot.keepaliveErrors.join("\n"), "warn");
  else hideBanner();

  const bg = sessions.filter((s) => s.kind === "background");
  const kept = bg.filter((s) => s.keptAlive).length;
  const time = new Date(snapshot.updatedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  $("#status").textContent =
    flash && Date.now() < flash.until
      ? flash.text
      : snapshot.keepaliveEnabled
        ? `${time} · keeping ${kept}/${bg.length} alive`
        : `${time} · keep-alive off`;
}

function showFlash(text: string) {
  const ms = 6000;
  flash = { text, until: Date.now() + ms };
  render();
  setTimeout(render, ms + 50);
}

// --- setup card ----------------------------------------------------------

async function copyText(text: string, code: HTMLElement, button: HTMLButtonElement) {
  try {
    await navigator.clipboard.writeText(text);
    button.textContent = "Copied";
    setTimeout(() => (button.textContent = "Copy"), 1500);
  } catch {
    // No clipboard access: select the command so Cmd/Ctrl+C works.
    const range = document.createRange();
    range.selectNodeContents(code);
    getSelection()?.removeAllRanges();
    getSelection()?.addRange(range);
  }
}

function renderSetup(failing: Check[]) {
  const setup = $("#setup");
  // Rebuild only on change so polling doesn't reset the copy buttons.
  const key = JSON.stringify(failing);
  if (setup.dataset.key === key) return;
  setup.dataset.key = key;
  setup.hidden = failing.length === 0;
  if (!failing.length) return;

  const items = failing.map((c) => {
    const item = el(
      "div",
      { className: "setup-item" },
      el("strong", {}, c.summary),
      el("small", {}, c.why),
    );
    if (c.fix) {
      const code = el("code", {}, c.fix);
      const copy = el("button", { textContent: "Copy" });
      copy.addEventListener("click", () => copyText(c.fix!, code, copy));
      item.append(el("div", { className: "fix" }, code, copy));
    }
    if (c.hint) item.append(el("small", {}, c.hint));
    return item;
  });

  const again = el("button", { textContent: "Check again" });
  again.addEventListener("click", () => {
    again.textContent = "Checking…";
    again.disabled = true;
    // The next snapshot rebuilds the card (or hides it once everything is fixed).
    setup.dataset.key = "";
    invoke("refresh");
  });
  const settings = el("button", { textContent: "Settings" });
  settings.addEventListener("click", showSettings);

  setup.replaceChildren(
    el("h2", {}, "Setup needed"),
    ...items,
    el("div", { className: "setup-actions" }, again, settings),
  );
}

function showBanner(text: string, kind: "error" | "warn") {
  const banner = $("#banner");
  banner.textContent = text;
  banner.className = `banner ${kind}`;
  banner.hidden = false;
}

function hideBanner() {
  $("#banner").hidden = true;
}

// --- filters -------------------------------------------------------------

function setKind(kind: KindFilter) {
  kindFilter = kind;
  prefs.save("kind", kind);
  document.querySelectorAll<HTMLButtonElement>(".segmented button").forEach((b) => {
    b.classList.toggle("active", b.dataset.kind === kind);
  });
  render();
}

document.querySelectorAll<HTMLButtonElement>(".segmented button").forEach((b) => {
  b.addEventListener("click", () => setKind(b.dataset.kind as KindFilter));
});

$<HTMLSelectElement>("#project").addEventListener("change", (e) => {
  projectFilter = (e.target as HTMLSelectElement).value;
  prefs.save("project", projectFilter);
  render();
});

$<HTMLInputElement>("#search").addEventListener("input", (e) => {
  search = (e.target as HTMLInputElement).value;
  render();
});

$("#refresh").addEventListener("click", () => invoke("refresh"));

// --- settings ------------------------------------------------------------

const form = $<HTMLFormElement>("#settings");
let loadedConfig: Config | null = null;
const field = <T extends HTMLElement>(name: string) => form.elements.namedItem(name) as unknown as T;

function syncCustomRow() {
  $("#custom-row").hidden = field<HTMLSelectElement>("terminal").value !== "custom";
}

async function showSettings() {
  const s = await invoke<SettingsView>("get_settings");
  loadedConfig = s.config;
  const terminal = field<HTMLSelectElement>("terminal");
  terminal.replaceChildren(...s.terminals.map((t) => el("option", { value: t.value, textContent: t.label })));
  terminal.value = s.config.terminal;
  field<HTMLInputElement>("customTerminal").value = s.config.customTerminal;
  field<HTMLInputElement>("keepalive").checked = s.config.keepalive;
  field<HTMLInputElement>("newSessionBackground").checked = s.config.newSessionBackground;
  field<HTMLInputElement>("autostart").checked = s.autostart;
  field<HTMLInputElement>("claudePath").value = s.config.claudePath;
  $("#claude-resolved").textContent = s.envError ?? (s.claudeResolved ? `Using ${s.claudeResolved}` : "");
  $("#config-path").textContent = s.configPath ? `Config: ${s.configPath}` : "";
  $("#settings-error").hidden = true;
  syncCustomRow();
  $("#list-view").hidden = true;
  $("#settings-view").hidden = false;
}

function hideSettings() {
  $("#settings-view").hidden = true;
  $("#list-view").hidden = false;
}

field<HTMLSelectElement>("terminal").addEventListener("change", syncCustomRow);
$("#open-settings").addEventListener("click", showSettings);
$("#cancel-settings").addEventListener("click", hideSettings);

form.addEventListener("submit", async (e) => {
  e.preventDefault();
  if (!loadedConfig) return;
  // Start from what was loaded so fields this form doesn't show are kept.
  const config: Config = {
    ...loadedConfig,
    terminal: field<HTMLSelectElement>("terminal").value,
    customTerminal: field<HTMLInputElement>("customTerminal").value,
    keepalive: field<HTMLInputElement>("keepalive").checked,
    newSessionBackground: field<HTMLInputElement>("newSessionBackground").checked,
    claudePath: field<HTMLInputElement>("claudePath").value.trim(),
  };
  try {
    await invoke("save_settings", { settings: { config, autostart: field<HTMLInputElement>("autostart").checked } });
    hideSettings();
  } catch (err) {
    const box = $("#settings-error");
    box.textContent = String(err);
    box.hidden = false;
  }
});

// --- new session ---------------------------------------------------------

const dialog = $<HTMLDialogElement>("#new-session");
const newForm = $<HTMLFormElement>("#new-session-form");
const newField = <T extends HTMLElement>(name: string) => newForm.elements.namedItem(name) as unknown as T;
const cwdSelect = newField<HTMLSelectElement>("cwd");

const basename = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

function setFolders(folders: string[], selected?: string) {
  cwdSelect.replaceChildren(...folders.map((f) => el("option", { value: f, textContent: basename(f), title: f })));
  if (selected) cwdSelect.value = selected;
  $("#cwd-path").textContent = cwdSelect.value;
}

async function showNewSession() {
  const d = await invoke<{ projects: string[]; background: boolean }>("new_session_defaults");
  // Default to the project the list is filtered to, if any.
  const preferred = d.projects.find((p) => basename(p) === projectFilter);
  setFolders(d.projects, preferred);
  newField<HTMLInputElement>("name").value = "";
  newField<HTMLTextAreaElement>("prompt").value = "";
  newField<HTMLInputElement>("background").checked = d.background;
  $("#new-session-error").hidden = true;
  dialog.showModal();
  (d.projects.length ? newField<HTMLInputElement>("name") : $<HTMLButtonElement>("#choose-folder")).focus();
}

cwdSelect.addEventListener("change", () => {
  $("#cwd-path").textContent = cwdSelect.value;
});

$("#choose-folder").addEventListener("click", async () => {
  const dir = await pickFolder({ directory: true, defaultPath: cwdSelect.value || undefined });
  if (typeof dir !== "string") return;
  const existing = [...cwdSelect.options].map((o) => o.value).filter((v) => v !== dir);
  setFolders([dir, ...existing], dir);
});

$("#new-session-button").addEventListener("click", showNewSession);
$("#cancel-new-session").addEventListener("click", () => dialog.close());

// Cmd/Ctrl+Enter in the prompt starts the session.
newField<HTMLTextAreaElement>("prompt").addEventListener("keydown", (e) => {
  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) newForm.requestSubmit();
});

newForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const error = $("#new-session-error");
  const submit = newForm.querySelector<HTMLButtonElement>("button[type=submit]")!;
  if (!cwdSelect.value) {
    error.textContent = "Choose a project folder first.";
    error.hidden = false;
    return;
  }
  submit.disabled = true;
  submit.textContent = "Starting…";
  try {
    const message = await invoke<string>("start_session", {
      session: {
        cwd: cwdSelect.value,
        name: newField<HTMLInputElement>("name").value,
        prompt: newField<HTMLTextAreaElement>("prompt").value,
        background: newField<HTMLInputElement>("background").checked,
      },
    });
    dialog.close();
    showFlash(message.split("\n")[0]);
  } catch (err) {
    error.textContent = String(err);
    error.hidden = false;
  } finally {
    submit.disabled = false;
    submit.textContent = "Start";
  }
});

// --- startup -------------------------------------------------------------

setKind(kindFilter);
listen<Snapshot>("snapshot", (e) => {
  snapshot = e.payload;
  render();
});
invoke<Snapshot>("get_snapshot").then((s) => {
  if (s.updatedAt) {
    snapshot = s;
    render();
  }
});
// Keep the "3h" ages honest between polls.
setInterval(render, 60_000);
