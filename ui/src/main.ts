import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { $, el, hideDetachedTip, installTooltips, prefs, withTip } from "./common";

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
  /** In plain `claude agents --json` (not only `--all`); decides keep-alive. */
  active: boolean;
  dot: Dot;
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


let snapshot: Snapshot | null = null;
// "All" on: everything `claude agents --json --all` lists. Off: only what
// plain `claude agents --json` lists (`active`).
let showAll: boolean = prefs.load("showAll", true);
// Bg and Interactive toggle independently.
const kinds: Record<"background" | "interactive", boolean> = prefs.load("kinds", {
  background: true,
  interactive: true,
});
let projectFilter: string = prefs.load("project", "");

/** `state` values with their own filter toggle and dot colour; the rest are "other". */
const STATES = ["working", "blocked", "done", "other"] as const;
type StateKey = (typeof STATES)[number];
const stateFilter: Record<StateKey, boolean> = {
  working: true,
  blocked: true,
  done: true,
  other: true,
  ...prefs.load<Partial<Record<StateKey, boolean>>>("states", {}),
};

type GroupBy = "project" | "state" | "none";
type SortBy = "name" | "started";
let groupBy: GroupBy = prefs.load<string>("groupBy", "project") as GroupBy;
if (!["project", "state", "none"].includes(groupBy)) groupBy = "project";
let sortBy: SortBy = prefs.load("sortBy", "name");

function stateKey(s: SessionView): StateKey {
  return s.state === "working" || s.state === "blocked" || s.state === "done" ? s.state : "other";
}
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
      (showAll || s.active) &&
      stateFilter[stateKey(s)] &&
      (s.kind === "other" || kinds[s.kind]) &&
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

/**
 * The status dot, worked out in the Rust core (`Session::dot`) the way
 * `claude agents` shows it: `state` picks the colour, `status: busy` pulses.
 */
interface Dot {
  color: "green" | "blue" | "orange" | "gray";
  pulsing: boolean;
}

/** Claude's own `state` and `status` values, exactly as reported. */
function rawValues(s: SessionView): string[] {
  return [s.state, s.status].filter((v): v is string => !!v);
}

function statusDot(s: SessionView): HTMLSpanElement {
  const cls = `dot ${s.dot.color}${s.dot.pulsing ? " pulsing" : ""}`;
  const tip = `state: ${s.state ?? "—"} · status: ${s.status ?? "—"}`;
  return withTip(el("span", { className: cls }), tip);
}

const LINK_ICON =
  '<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6.5 9.5a3 3 0 0 0 4.2 0l2.1-2.1a3 3 0 0 0-4.2-4.2l-.9.9M9.5 6.5a3 3 0 0 0-4.2 0L3.2 8.6a3 3 0 0 0 4.2 4.2l.9-.9"/></svg>';

const TRASH_ICON =
  '<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M2.5 4h11M6.5 4V2.5h3V4M4 4l.7 9.5h6.6L12 4M6.75 6.5v4.5M9.25 6.5v4.5"/></svg>';

function sessionRow(s: SessionView): HTMLLIElement {
  const meta = [s.kind === "background" ? "bg" : s.kind, ...rawValues(s), s.startedAt ? ago(s.startedAt) : ""]
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
      s.attachable
        ? `Double-click to open in ${terminalLabel()}`
        : "Interactive sessions live in the terminal that started them",
    ]
      .filter(Boolean)
      .join("\n"),
  );
  row.append(
    statusDot(s),
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

  const shown = sortSessions(visibleSessions(sessions));
  const groups = groupSessions(shown);

  // Rebuild rows only when what they show changes (ages tick once a minute),
  // so hover tooltips and keyboard focus survive routine polls.
  const groupsEl = $("#groups");
  const rowsKey = JSON.stringify([shown, groupBy, [...collapsed], terminalLabel(), Math.floor(Date.now() / 60_000)]);
  if (groupsEl.dataset.key !== rowsKey) {
    groupsEl.dataset.key = rowsKey;
    renderGroups(groupsEl, groups);
    hideDetachedTip();
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
  // Sessions only `--all` lists have timed out; keep-alive leaves them be.
  const bg = sessions.filter((s) => s.kind === "background" && s.active);
  const kept = bg.filter((s) => s.keptAlive).length;
  const time = new Date(snapshot.updatedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  $("#status").textContent =
    flash && Date.now() < flash.until
      ? flash.text
      : snapshot.keepaliveEnabled
        ? `${time} · keeping ${kept}/${bg.length} alive`
        : `${time} · keep-alive off`;
}

function sortSessions(list: SessionView[]): SessionView[] {
  const byName = (a: SessionView, b: SessionView) =>
    displayName(a).localeCompare(displayName(b), undefined, { numeric: true, sensitivity: "base" });
  return [...list].sort((a, b) =>
    sortBy === "started" ? (b.startedAt ?? -1) - (a.startedAt ?? -1) || byName(a, b) : byName(a, b),
  );
}

/** Group (already sorted) sessions; the order of groups depends on the grouping. */
function groupSessions(list: SessionView[]): Map<string, SessionView[]> {
  const keyOf = (s: SessionView): string =>
    groupBy === "project" ? s.project : groupBy === "state" ? stateKey(s) : "";
  const groups = new Map<string, SessionView[]>();
  for (const s of list) groups.set(keyOf(s), [...(groups.get(keyOf(s)) ?? []), s]);

  const order: string[] =
    groupBy === "state"
      ? [...STATES]
      : [...groups.keys()].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
  return new Map(order.filter((k) => groups.has(k)).map((k) => [k, groups.get(k)!]));
}

function renderGroups(container: HTMLElement, groups: Map<string, SessionView[]>) {
  // `.cols` flows groups into side-by-side columns when the window is wide.
  if (groupBy === "none") {
    const items = groups.get("") ?? [];
    container.replaceChildren(
      el("ul", { className: "cols" }, el("li", { className: "group flat" }, el("ul", { className: "sessions" }, ...items.map(sessionRow)))),
    );
    return;
  }
  container.replaceChildren(
    el(
      "ul",
      { className: "cols" },
      ...[...groups].map(([label, items]) => {
      // Collapse state is remembered per grouping.
      const key = `${groupBy}:${label}`;
      const isCollapsed = collapsed.has(key);
      const header = withTip(
        el(
          "button",
          { className: "group-header", ariaExpanded: String(!isCollapsed) },
          el("span", { className: "caret" }, isCollapsed ? "▸" : "▾"),
          `${label} `,
          el("span", { className: "count" }, `(${items.length})`),
        ),
        `${isCollapsed ? "Expand" : "Collapse"} ${label}`,
      );
      header.addEventListener("click", () => {
        if (collapsed.has(key)) collapsed.delete(key);
        else collapsed.add(key);
        prefs.save("collapsed", [...collapsed]);
        render();
      });
      const list = el("ul", { className: "sessions" }, ...(isCollapsed ? [] : items.map(sessionRow)));
      return el("li", { className: "group" }, header, list);
      }),
    ),
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

function syncToggles() {
  $("#show-all").setAttribute("aria-pressed", String(showAll));
  document.querySelectorAll<HTMLButtonElement>("[data-state]").forEach((b) => {
    b.setAttribute("aria-pressed", String(stateFilter[b.dataset.state as StateKey]));
  });
  document.querySelectorAll<HTMLButtonElement>("[data-kind]").forEach((b) => {
    b.setAttribute("aria-pressed", String(kinds[b.dataset.kind as keyof typeof kinds]));
  });
}

document.querySelectorAll<HTMLButtonElement>("[data-state]").forEach((b) => {
  b.addEventListener("click", () => {
    const key = b.dataset.state as StateKey;
    stateFilter[key] = !stateFilter[key];
    prefs.save("states", stateFilter);
    syncToggles();
    render();
  });
});

const groupSelect = $<HTMLSelectElement>("#group-by");
const sortSelect = $<HTMLSelectElement>("#sort-by");
groupSelect.value = groupBy;
sortSelect.value = sortBy;
groupSelect.addEventListener("change", () => {
  groupBy = groupSelect.value as GroupBy;
  prefs.save("groupBy", groupBy);
  render();
});
sortSelect.addEventListener("change", () => {
  sortBy = sortSelect.value as SortBy;
  prefs.save("sortBy", sortBy);
  render();
});

$("#show-all").addEventListener("click", () => {
  showAll = !showAll;
  prefs.save("showAll", showAll);
  syncToggles();
  render();
});

document.querySelectorAll<HTMLButtonElement>("[data-kind]").forEach((b) => {
  b.addEventListener("click", () => {
    const kind = b.dataset.kind as keyof typeof kinds;
    kinds[kind] = !kinds[kind];
    prefs.save("kinds", kinds);
    syncToggles();
    render();
  });
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
// The form lives in its own window (new-session.html), so it isn't squeezed
// into the narrow list. It reports back with a `session-started` event.

$("#new-session-button").addEventListener("click", () => invoke("open_new_session_window"));
listen<string>("session-started", (e) => showFlash(e.payload.split("\n")[0]));

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

// --- startup -------------------------------------------------------------

installTooltips();
syncToggles();
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
