import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";

type Kind = "interactive" | "background" | "other";

interface SessionView {
  pid?: number;
  id?: string;
  cwd: string;
  kind: Kind;
  startedAt?: number;
  sessionId: string;
  name?: string;
  status?: string;
  state?: string;
  project: string;
  attachable: boolean;
  running: boolean;
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
  terminalLabel: string;
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

/** Give an element a hover/focus tooltip (see the tooltip section below). */
function withTip<T extends HTMLElement>(node: T, text: string): T {
  node.dataset.tip = text;
  return node;
}

/** Name of the configured terminal, for tooltips. */
function terminalLabel(): string {
  return snapshot?.terminalLabel ?? "your terminal";
}

function displayName(s: SessionView): string {
  return s.name || s.id || s.sessionId.slice(0, 8) || "unnamed session";
}

function visibleSessions(all: SessionView[]): SessionView[] {
  const q = search.trim().toLowerCase();
  return all.filter(
    (s) =>
      (kindFilter === "all" || s.kind === kindFilter) &&
      (!projectFilter || s.project === projectFilter) &&
      (!q || displayName(s).toLowerCase().includes(q) || s.project.toLowerCase().includes(q)),
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

const STATUS_TIPS: Record<string, string> = {
  busy: "Busy: Claude is working",
  idle: "Idle: waiting for your input",
  blocked: "Blocked: waiting for you (for example a permission prompt). Open it to respond.",
  stopped: "Stopped: no process is running. Opening it resumes the session.",
  unknown: "Status unknown",
};

const LINK_ICON =
  '<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6.5 9.5a3 3 0 0 0 4.2 0l2.1-2.1a3 3 0 0 0-4.2-4.2l-.9.9M9.5 6.5a3 3 0 0 0-4.2 0L3.2 8.6a3 3 0 0 0 4.2 4.2l.9-.9"/></svg>';

const TRASH_ICON =
  '<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M2.5 4h11M6.5 4V2.5h3V4M4 4l.7 9.5h6.6L12 4M6.75 6.5v4.5M9.25 6.5v4.5"/></svg>';

function sessionRow(s: SessionView): HTMLLIElement {
  // `blocked` (waiting for you) matters most, and often arrives without a status.
  const status = !s.running ? "stopped" : s.state === "blocked" ? "blocked" : (s.status ?? "unknown");
  const dotClass = ["busy", "idle", "blocked", "stopped"].includes(status) ? status : "unknown";
  const meta = [
    s.kind === "background" ? "bg" : s.kind,
    s.running ? (s.state ?? status) : "stopped",
    s.startedAt ? ago(s.startedAt) : "",
  ]
    .filter(Boolean)
    .join(" · ");

  const kind = s.kind === "background" ? "Background session" : s.kind === "interactive" ? "Interactive session" : "Session";
  const started = s.startedAt
    ? ` · started ${new Date(s.startedAt).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })}`
    : "";
  const row = withTip(
    el("li", { className: `session ${s.attachable ? "attachable" : "inert"}`, tabIndex: 0 }),
    [
      s.cwd,
      `${kind}${s.state ? ` · ${s.state}` : ""}${started}`,
      !s.attachable
        ? "Interactive sessions live in the terminal that started them"
        : s.running
          ? `Double-click to open in ${terminalLabel()}`
          : `Not running. Double-click to resume it in ${terminalLabel()}`,
    ]
      .filter(Boolean)
      .join("\n"),
  );
  row.append(
    withTip(el("span", { className: `dot ${dotClass}` }), STATUS_TIPS[dotClass]),
    el(
      "span",
      { className: "label" },
      el("span", { className: "name" }, displayName(s)),
      el("span", { className: "meta" }, meta),
    ),
  );
  if (s.keptAlive) {
    const link = withTip(
      el("span", { className: "keepalive", ariaLabel: "Kept alive" }),
      "Kept alive: Claude Local keeps a hidden client attached so this session won't idle out after 8 hours",
    );
    link.innerHTML = LINK_ICON;
    row.append(link);
  }
  if (s.attachable) {
    const open = withTip(
      el("button", { className: "open", textContent: "Open" }),
      `Open in ${terminalLabel()} (claude attach ${s.id})`,
    );
    open.addEventListener("click", (e) => {
      e.stopPropagation();
      openSession(s);
    });
    const del = withTip(
      el("button", { className: "delete", ariaLabel: "Delete session" }),
      "Delete this session… (asks first)",
    );
    del.innerHTML = TRASH_ICON;
    del.addEventListener("click", (e) => {
      e.stopPropagation();
      confirmDelete(s);
    });
    del.addEventListener("dblclick", (e) => e.stopPropagation());
    row.append(open, del);
    row.addEventListener("dblclick", () => openSession(s));
    row.addEventListener("keydown", (e) => {
      if (e.key === "Enter") openSession(s);
      if (e.key === "Delete" || e.key === "Backspace") confirmDelete(s);
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

  // Rebuild rows only when what they show changes (ages tick once a minute),
  // so hover tooltips and keyboard focus survive routine polls.
  const groupsEl = $("#groups");
  const rowsKey = JSON.stringify([shown, [...collapsed], terminalLabel(), Math.floor(Date.now() / 60_000)]);
  if (groupsEl.dataset.key !== rowsKey) {
    groupsEl.dataset.key = rowsKey;
    renderGroups(groupsEl, groups);
    if (tipTarget && !tipTarget.isConnected) hideTip();
  }

  const failing = snapshot.checks.filter((c) => !c.ok);
  renderSetup(failing);

  const empty = $("#empty");
  empty.hidden = shown.length > 0 || !!snapshot.error || failing.some((c) => c.name === "claude");
  empty.textContent = sessions.length === 0 ? "No Claude sessions running." : "No sessions match these filters.";

  if (snapshot.error) showBanner(snapshot.error, "error");
  else if (snapshot.keepaliveErrors.length) showBanner(snapshot.keepaliveErrors.join("\n"), "warn");
  else hideBanner();

  // Stopped sessions aren't kept alive (that would resume them), so don't count them.
  const bg = sessions.filter((s) => s.kind === "background" && s.running);
  const kept = bg.filter((s) => s.keptAlive).length;
  const time = new Date(snapshot.updatedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  $("#status").textContent =
    flash && Date.now() < flash.until
      ? flash.text
      : snapshot.keepaliveEnabled
        ? `${time} · keeping ${kept}/${bg.length} alive`
        : `${time} · keep-alive off`;
}

function renderGroups(container: HTMLElement, groups: Map<string, SessionView[]>) {
  container.replaceChildren(
    ...[...groups].map(([project, items]) => {
      const isCollapsed = collapsed.has(project);
      const header = withTip(
        el(
          "button",
          { className: "group-header", ariaExpanded: String(!isCollapsed) },
          el("span", { className: "caret" }, isCollapsed ? "▸" : "▾"),
          `${project} `,
          el("span", { className: "count" }, `(${items.length})`),
        ),
        `${isCollapsed ? "Expand" : "Collapse"} ${project}`,
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
      const copy = withTip(el("button", { textContent: "Copy" }), "Copy this command, then paste it into a terminal");
      copy.addEventListener("click", () => copyText(c.fix!, code, copy));
      item.append(el("div", { className: "fix" }, code, copy));
    }
    if (c.hint) item.append(el("small", {}, c.hint));
    return item;
  });

  const again = withTip(el("button", { textContent: "Check again" }), "Look for the missing tools again");
  again.addEventListener("click", () => {
    again.textContent = "Checking…";
    again.disabled = true;
    // The next snapshot rebuilds the card (or hides it once everything is fixed).
    setup.dataset.key = "";
    invoke("refresh");
  });
  const settings = withTip(el("button", { textContent: "Settings" }), "Open Settings");
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

const isMac = navigator.userAgent.includes("Mac");
newField<HTMLTextAreaElement>("prompt").dataset.tip = `Optional. Sent as the session's first message. ${isMac ? "⌘" : "Ctrl+"}Enter starts the session.`;

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

// --- delete session ------------------------------------------------------

type DeleteResult = { status: "deleted"; message: string } | { status: "needsDiscard"; message: string };

const deleteDialog = $<HTMLDialogElement>("#confirm-delete");
const deleteForm = $<HTMLFormElement>("#delete-form");
const deleteButton = $<HTMLButtonElement>("#confirm-delete-button");
// The session being deleted, and whether Claude has asked to discard work.
let deleting: { session: SessionView; discard: boolean } | null = null;

function confirmDelete(s: SessionView) {
  if (!s.attachable || !s.id) return;
  deleting = { session: s, discard: false };
  const name = displayName(s);
  $("#delete-title").textContent = `Delete “${name}”?`;
  $("#delete-body").textContent =
    "This stops the session and deletes it, along with its worktree if it has one. Its conversation can't be resumed afterwards.";
  $("#delete-where").textContent = `${s.cwd} · ${s.id}`;
  $("#delete-claude").hidden = true;
  $("#delete-error").hidden = true;
  deleteButton.textContent = "Delete";
  deleteButton.dataset.tip = `Runs claude rm ${s.id}`;
  deleteButton.disabled = false;
  deleteDialog.showModal();
  // Safer default: Enter cancels until the user moves to Delete.
  $<HTMLButtonElement>("#cancel-delete").focus();
}

$("#cancel-delete").addEventListener("click", () => deleteDialog.close());
deleteDialog.addEventListener("close", () => (deleting = null));

deleteForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  if (!deleting) return;
  const { session, discard } = deleting;
  const label = deleteButton.textContent;
  deleteButton.disabled = true;
  deleteButton.textContent = "Deleting…";
  try {
    const result = await invoke<DeleteResult>("delete_session", { id: session.id, discard });
    if (result.status === "deleted") {
      deleteDialog.close();
      showFlash(`Deleted ${displayName(session)}`);
      return;
    }
    // Claude refused because work would be lost: show why, ask once more.
    deleting = { session, discard: true };
    $("#delete-body").textContent =
      "Claude didn't delete it: the session has work that isn't pushed. Deleting anyway discards that work permanently.";
    const claudeSays = $("#delete-claude");
    claudeSays.textContent = result.message;
    claudeSays.hidden = false;
    deleteButton.disabled = false;
    deleteButton.textContent = "Discard unpushed work and delete";
    deleteButton.dataset.tip = "Runs claude rm again with the discard value Claude reported above. The unpushed work is lost.";
    $<HTMLButtonElement>("#cancel-delete").focus();
  } catch (err) {
    const box = $("#delete-error");
    box.textContent = String(err);
    box.hidden = false;
    deleteButton.disabled = false;
    deleteButton.textContent = label;
  }
});

// --- tooltips ------------------------------------------------------------
// One floating element for every [data-tip]. Native `title` tooltips are slow,
// look different per OS, never show for keyboard focus, and sit underneath
// modal dialogs.

const tooltip = el("div", { className: "tooltip", role: "tooltip" });
let tipTarget: HTMLElement | null = null;
let tipTimer: number | undefined;

function showTip(target: HTMLElement) {
  const text = target.dataset.tip;
  if (!text || !target.isConnected) return;
  // A modal dialog sits in the top layer; the tooltip must live inside it to show.
  (target.closest("dialog") ?? document.body).append(tooltip);
  tooltip.textContent = text;
  tooltip.classList.add("visible");

  const margin = 6;
  const r = target.getBoundingClientRect();
  const t = tooltip.getBoundingClientRect();
  let top = r.bottom + margin;
  if (top + t.height > window.innerHeight - margin) top = r.top - t.height - margin;
  const left = Math.min(
    Math.max(margin, r.left + r.width / 2 - t.width / 2),
    window.innerWidth - t.width - margin,
  );
  tooltip.style.top = `${Math.max(margin, top)}px`;
  tooltip.style.left = `${left}px`;
}

function hideTip() {
  clearTimeout(tipTimer);
  tipTarget = null;
  tooltip.classList.remove("visible");
}

function scheduleTip(target: HTMLElement | null) {
  if (target === tipTarget) return;
  hideTip();
  if (!target) return;
  tipTarget = target;
  tipTimer = window.setTimeout(() => showTip(target), 450);
}

document.addEventListener("mouseover", (e) => {
  scheduleTip((e.target as Element).closest<HTMLElement>("[data-tip]"));
});
document.addEventListener("mouseout", (e) => {
  if (tipTarget && !tipTarget.contains(e.relatedTarget as Node | null)) hideTip();
});
document.addEventListener("focusin", (e) => {
  const target = e.target as HTMLElement;
  if (target.matches(":focus-visible")) scheduleTip(target.closest<HTMLElement>("[data-tip]"));
});
document.addEventListener("focusout", hideTip);
for (const type of ["mousedown", "keydown", "scroll", "blur"]) {
  window.addEventListener(type, hideTip, true);
}

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
