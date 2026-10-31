// The New Session window: pick a folder, name, first prompt, then start the
// session in the background or in a terminal. Opened from the list's ＋.

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";

import { $, basename, el, installTooltips, prefs } from "./common";

const form = $<HTMLFormElement>("#new-session-form");
const field = <T extends HTMLElement>(name: string) => form.elements.namedItem(name) as unknown as T;
const cwdSelect = field<HTMLSelectElement>("cwd");
const close = () => getCurrentWindow().close();

function setFolders(folders: string[], selected?: string) {
  cwdSelect.replaceChildren(...folders.map((f) => el("option", { value: f, textContent: basename(f), title: f })));
  if (selected) cwdSelect.value = selected;
  $("#cwd-path").textContent = cwdSelect.value;
}

async function load() {
  const d = await invoke<{ projects: string[]; background: boolean }>("new_session_defaults");
  // Default to the project the list is filtered to, if any (the list saves it).
  const projectFilter = prefs.load("project", "");
  setFolders(
    d.projects,
    d.projects.find((p) => basename(p) === projectFilter),
  );
  field<HTMLInputElement>("background").checked = d.background;
  (d.projects.length ? field<HTMLInputElement>("name") : $<HTMLButtonElement>("#choose-folder")).focus();
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

$("#cancel").addEventListener("click", close);
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") close();
});

const isMac = navigator.userAgent.includes("Mac");
const prompt = field<HTMLTextAreaElement>("prompt");
prompt.dataset.tip = `Optional. Sent as the session's first message. ${isMac ? "⌘" : "Ctrl+"}Enter starts the session.`;
// Cmd/Ctrl+Enter in the prompt starts the session.
prompt.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) form.requestSubmit();
});

form.addEventListener("submit", async (e) => {
  e.preventDefault();
  const error = $("#new-session-error");
  const submit = form.querySelector<HTMLButtonElement>("button[type=submit]")!;
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
        name: field<HTMLInputElement>("name").value,
        prompt: prompt.value,
        background: field<HTMLInputElement>("background").checked,
      },
    });
    // The list shows the result.
    await emit("session-started", message);
    await close();
  } catch (err) {
    error.textContent = String(err);
    error.hidden = false;
    submit.disabled = false;
    submit.textContent = "Start";
  }
});

installTooltips();
load();
