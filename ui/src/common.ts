// Helpers shared by the session list (main.ts) and the New Session window.

export const $ = <T extends HTMLElement>(sel: string) => document.querySelector(sel) as T;

// Per-viewer UI preferences; storage can be unavailable, so never depend on it.
export const prefs = {
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

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

/** Give an element a hover/focus tooltip (see installTooltips). */
export function withTip<T extends HTMLElement>(node: T, text: string): T {
  node.dataset.tip = text;
  return node;
}

export const basename = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

// --- tooltips ------------------------------------------------------------
// One floating element for every [data-tip]. Native `title` tooltips are slow,
// look different per OS, never show for keyboard focus, and sit underneath
// modal dialogs.

const tooltip = el("div", { className: "tooltip", role: "tooltip" });
let tipTarget: HTMLElement | null = null;
let installed = false;
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

export function hideTip() {
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

/** Hide the tooltip if the element it belongs to was re-rendered away. */
export function hideDetachedTip() {
  if (tipTarget && !tipTarget.isConnected) hideTip();
}

/** Show a tooltip for every [data-tip] element on the page. */
export function installTooltips() {
  if (installed) return;
  installed = true;
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
}
