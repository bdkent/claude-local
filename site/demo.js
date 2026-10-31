// The live demo in the hero: a working replica of the Claude Local window,
// plus scroll reveals. No dependencies.
(() => {
  const $ = (sel) => document.querySelector(sel);
  const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

  const LINK =
    '<svg viewBox="0 0 16 16"><path d="M6.5 9.5a3 3 0 0 0 4.2 0l2.1-2.1a3 3 0 0 0-4.2-4.2l-.9.9M9.5 6.5a3 3 0 0 0-4.2 0L3.2 8.6a3 3 0 0 0 4.2 4.2l.9-.9"/></svg>';
  const TRASH =
    '<svg viewBox="0 0 16 16"><path d="M2.5 4h11M6.5 4V2.5h3V4M4 4l.7 9.5h6.6L12 4M6.75 6.5v4.5M9.25 6.5v4.5"/></svg>';

  const minutesAgo = (m) => Date.now() - m * 60_000;
  const sessions = [
    { id: "3b9e1c07", name: "starship-export", project: "starship", kind: "background", status: "idle", state: "done", started: minutesAgo(190), kept: true },
    { id: "a11b2c3d", name: "starship-shaders", project: "starship", kind: "background", status: "busy", state: "working", started: minutesAgo(41), kept: true },
    { id: null, name: "meadow-refactor", project: "meadow", kind: "interactive", status: "busy", started: minutesAgo(1560), kept: false },
    { id: "c42d8a15", name: "harbor-build", project: "harbor", kind: "background", status: "idle", state: "done", started: minutesAgo(300), kept: true },
    // Only `claude agents --json --all` lists this one; its process has ended.
    { id: "9d41e2b0", name: "harbor-audio-old", project: "harbor", kind: "background", state: "done", started: minutesAgo(900), kept: false, inactive: true },
  ];
  const newNames = [
    ["harbor", "harbor-lightmaps"],
    ["starship", "starship-navmesh"],
    ["meadow", "meadow-ui"],
    ["harbor", "harbor-audio"],
  ];

  let showAll = true;
  const kinds = { background: true, interactive: true };
  const list = $("#demo-list");
  const stage = $(".stage");
  if (!list || !stage) return;

  const ago = (t) => {
    const m = Math.max(0, (Date.now() - t) / 60_000);
    if (m < 1) return "now";
    if (m < 60) return `${Math.floor(m)}m`;
    if (m < 1440) return `${Math.floor(m / 60)}h`;
    return `${Math.floor(m / 1440)}d`;
  };

  function h(tag, attrs = {}, ...kids) {
    const n = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (k === "html") n.innerHTML = v;
      else if (k.startsWith("data-")) n.setAttribute(k, v);
      else n[k] = v;
    }
    n.append(...kids);
    return n;
  }

  function render() {
    const shown = sessions.filter((s) => (showAll || !s.inactive) && kinds[s.kind]);
    const groups = new Map();
    for (const s of shown) groups.set(s.project, [...(groups.get(s.project) || []), s]);
    const out = [];
    for (const [project, items] of [...groups].sort((a, b) => a[0].localeCompare(b[0]))) {
      out.push(h("li", { className: "grp" }, `▾ ${project} `, h("em", {}, `(${items.length})`)));
      for (const s of items) out.push(row(s));
    }
    list.replaceChildren(...out);

    const bg = sessions.filter((s) => s.kind === "background" && !s.inactive);
    $("#demo-status").textContent = `keeping ${bg.filter((s) => s.kept).length}/${bg.length} alive`;
    $("#demo-tray-count").textContent = String(sessions.length);
  }

  function row(s) {
    const bg = s.kind === "background";
    const li = h(
      "li",
      {
        className: `row ${bg ? "" : "inert"} ${s.fresh ? "new" : ""}`,
        "data-tip": bg
          ? `/Users/you/projects/${s.project}\nBackground session · double-click to open`
          : "Interactive sessions live in the terminal that started them",
      },
      // Same rule as the app: `state` sets the colour, `status: busy` pulses.
      h("span", {
        className: `dot ${({ working: "green", done: "blue", blocked: "orange" })[s.state] || "gray"}${s.status === "busy" ? " pulsing" : ""}`,
        "data-tip": `state: ${s.state || "—"} · status: ${s.status || "—"}`,
      }),
      h(
        "span",
        { className: "lbl" },
        h("span", { className: "nm" }, s.name),
        h("span", { className: "mt" }, [bg ? "bg" : "interactive", s.state, s.status, ago(s.started)].filter(Boolean).join(" · ")),
      ),
    );
    li._session = s;
    if (s.kept) {
      li.append(
        h("span", {
          className: "link",
          html: LINK,
          "data-tip": "Kept alive: a hidden client stays attached so this session won't idle out after 8 hours",
        }),
      );
    }
    if (bg) {
      const open = h("button", { className: "open", textContent: "Open", "data-tip": `Open in Ghostty (claude attach ${s.id})` });
      open.addEventListener("click", (e) => {
        e.stopPropagation();
        openTerminal(s);
      });
      const del = h("button", { className: "del", html: TRASH, "data-tip": "Delete this session… (asks first, twice if work is unpushed)" });
      li.append(open, del);
      li.addEventListener("dblclick", () => openTerminal(s));
    }
    return li;
  }

  // --- interactions ------------------------------------------------------

  // Same filters as the app: "All" toggles --all; Bg and Interactive toggle independently.
  $("#demo-all").addEventListener("click", (e) => {
    showAll = !showAll;
    e.currentTarget.classList.toggle("on", showAll);
    render();
    userActed();
  });
  document.querySelectorAll(".seg [data-kind]").forEach((b) =>
    b.addEventListener("click", () => {
      kinds[b.dataset.kind] = !kinds[b.dataset.kind];
      b.classList.toggle("on", kinds[b.dataset.kind]);
      render();
      userActed();
    }),
  );

  let added = 0;
  function addSession() {
    const [project, name] = newNames[added++ % newNames.length];
    const s = { id: Math.random().toString(16).slice(2, 10), name: added > newNames.length ? `${name}-${added}` : name, project, kind: "background", status: "busy", state: "working", started: Date.now(), kept: false };
    s.fresh = true;
    sessions.push(s);
    if (!kinds.background) {
      kinds.background = true;
      document.querySelector('.seg [data-kind="background"]').classList.add("on");
    }
    render();
    // The keep-alive attaches on the next poll.
    setTimeout(() => {
      s.kept = true;
      s.fresh = false;
      render();
    }, 1200);
  }
  $("#demo-new").addEventListener("click", () => {
    addSession();
    userActed();
  });

  const app = $("#demo-app");
  $("#demo-tray").addEventListener("click", () => {
    app.classList.toggle("closed");
    $("#demo-term").hidden = true;
    userActed();
  });

  // Typing terminal for Open / double-click.
  let typing = 0;
  function openTerminal(s) {
    userActed();
    const term = $("#demo-term");
    const body = $("#demo-term-body");
    term.hidden = false;
    term.style.animation = "none";
    void term.offsetWidth;
    term.style.animation = "";
    $("#demo-term-title").textContent = `Ghostty · ${s.name}`;
    const run = ++typing;
    const cmd = `claude attach ${s.id}`;
    const lines = [
      `<span class="ok">✓</span> attached to <span class="acc">${s.name}</span>`,
      `  ${s.project} · ${s.state || s.status} · started ${ago(s.started)} ago`,
      "",
      `<span class="acc">›</span> <span class="caret"></span>`,
    ];
    body.innerHTML = '<span class="acc">$</span> ';
    let i = 0;
    const step = () => {
      if (run !== typing) return;
      if (i < cmd.length) {
        body.innerHTML = `<span class="acc">$</span> ${cmd.slice(0, ++i)}<span class="caret"></span>`;
        setTimeout(step, reduceMotion ? 0 : 28 + Math.random() * 40);
      } else {
        body.innerHTML = `<span class="acc">$</span> ${cmd}\n`;
        setTimeout(() => {
          if (run === typing) body.innerHTML += lines.join("\n");
        }, reduceMotion ? 0 : 350);
      }
    };
    step();
  }
  document.addEventListener("click", (e) => {
    const term = $("#demo-term");
    if (!term.hidden && !e.target.closest("#demo-term, .open, .row")) term.hidden = true;
  });

  // Sessions drift between working and done, like real ones.
  setInterval(() => {
    const bg = sessions.filter((s) => s.kind === "background" && !s.inactive);
    const s = bg[Math.floor(Math.random() * bg.length)];
    s.status = s.status === "busy" ? "idle" : "busy";
    s.state = s.status === "busy" ? "working" : "done";
    const li = [...list.children].find((n) => n._session === s);
    if (li && !li.matches(":hover")) li.replaceWith(row(s));
  }, 2600);

  // One automatic "＋" so the demo shows off before anyone touches it.
  let acted = false;
  function userActed() {
    acted = true;
  }
  setTimeout(() => {
    if (!acted && !reduceMotion) addSession();
  }, 3200);

  const clock = () => {
    $("#demo-clock").textContent = new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  };
  clock();
  setInterval(clock, 15_000);

  // --- tooltips inside the stage (same feel as the app) --------------------

  const tip = $("#demo-tip");
  let tipFor = null;
  let timer;
  stage.addEventListener("mouseover", (e) => {
    const t = e.target.closest("[data-tip]");
    if (t === tipFor) return;
    clearTimeout(timer);
    tip.classList.remove("on");
    tipFor = t;
    if (!t) return;
    timer = setTimeout(() => {
      tip.textContent = t.dataset.tip;
      tip.style.whiteSpace = "pre-line";
      const s = stage.getBoundingClientRect();
      const r = t.getBoundingClientRect();
      tip.classList.add("on");
      const w = tip.offsetWidth;
      let left = r.left - s.left + r.width / 2 - w / 2;
      left = Math.max(4, Math.min(left, s.width - w - 4));
      tip.style.left = `${left}px`;
      tip.style.top = `${r.bottom - s.top + 6}px`;
    }, 350);
  });
  stage.addEventListener("mouseleave", () => {
    clearTimeout(timer);
    tipFor = null;
    tip.classList.remove("on");
  });

  render();

  // --- page: scroll reveal + card glow ---------------------------------------

  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (e.isIntersecting) {
          e.target.classList.add("in");
          io.unobserve(e.target);
        }
      }
    },
    { threshold: 0.12 },
  );
  document.querySelectorAll(".reveal").forEach((el, i) => {
    el.style.transitionDelay = `${Math.min(i % 6, 5) * 60}ms`;
    io.observe(el);
  });

  document.querySelectorAll(".card").forEach((card) =>
    card.addEventListener("pointermove", (e) => {
      const r = card.getBoundingClientRect();
      card.style.setProperty("--mx", `${e.clientX - r.left}px`);
      card.style.setProperty("--my", `${e.clientY - r.top}px`);
    }),
  );
})();
