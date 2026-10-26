// Shared by every page: theme, OS detection, tabs, copy buttons and the ⌘K search palette.
// Pages can add their own palette entries with Trunq.addCommands([...]).

(() => {
  "use strict";

  const REPO = "https://github.com/IssamElNass/trunq";
  const ROOT = window.TRUNQ_ROOT || "";
  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
  const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

  // ───────────── platform ─────────────
  const ua = navigator.userAgent;
  const platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  const isMobile = /iphone|ipad|android/i.test(ua);
  const isMac = /mac|iphone|ipad/i.test(platform) || /Mac OS X/.test(ua);
  const os = isMobile ? null : isMac ? "mac" : /win/i.test(platform) || /Windows/.test(ua) ? "win"
    : /ubuntu|debian/i.test(ua) ? "deb" : /linux|x11/i.test(platform + ua) ? "linux" : null;
  const MOD = isMac ? "⌘" : "Ctrl";
  const OS_NAMES = { mac: "macOS", win: "Windows", deb: "Debian / Ubuntu", linux: "Linux" };

  $$("[data-mod]").forEach((k) => (k.textContent = MOD));
  $$("[data-alt]").forEach((k) => (k.textContent = isMac ? "⌥" : "Alt"));
  if (isMac) $$("[data-mod-hint]").forEach((h) => (h.hidden = true));
  if (os) {
    $$("[data-os-label]").forEach((l) => (l.textContent = `Download for ${OS_NAMES[os]}`));
    const card = $(`.platform[data-os="${os}"]`);
    if (card) {
      card.classList.add("is-mine");
      card.parentElement.prepend(card);
    }
    const tab = $(`[role=tab][data-os-tab~="${os}"]`);
    if (tab) selectTab(tab);
  }

  // ───────────── theme ─────────────
  function labelThemeButtons() {
    const label = document.documentElement.dataset.theme === "dark" ? "Switch to light theme" : "Switch to dark theme";
    $$("[data-theme-toggle]").forEach((b) => {
      b.setAttribute("aria-label", label);
      if (b.dataset.tip) b.dataset.tip = label;
    });
  }
  function toggleTheme() {
    const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    try { localStorage.setItem("trunq-theme", next); } catch { /* Storage may be unavailable in private or embedded browsers. */ }
    labelThemeButtons();
  }
  $$("[data-theme-toggle]").forEach((b) => b.addEventListener("click", toggleTheme));
  labelThemeButtons();

  // ───────────── tabs + copy buttons ─────────────
  function selectTab(tab) {
    const list = tab.closest("[role=tablist]");
    $$("[role=tab]", list).forEach((t) => {
      const on = t === tab;
      t.setAttribute("aria-selected", on);
      t.tabIndex = on ? 0 : -1;
      $("#" + t.getAttribute("aria-controls")).hidden = !on;
    });
  }
  $$(".tabs [role=tab]").forEach((t) => {
    t.addEventListener("click", () => selectTab(t));
    t.addEventListener("keydown", (e) => {
      if (!["ArrowRight", "ArrowLeft", "Home", "End"].includes(e.key)) return;
      e.preventDefault();
      const tabs = $$("[role=tab]", t.parentElement);
      const index = e.key === "Home" ? 0 : e.key === "End" ? tabs.length - 1 : (tabs.indexOf(t) + (e.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length;
      const next = tabs[index];
      selectTab(next);
      next.focus();
    });
  });

  // Copy exactly the displayed code, including comments and Unicode.
  $$("pre[data-copyable]").forEach((pre) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "copy";
    btn.textContent = "Copy";
    btn.setAttribute("aria-label", "Copy code to clipboard");
    btn.setAttribute("aria-live", "polite");
    pre.parentElement.classList.add("has-copy");
    pre.insertAdjacentElement("beforebegin", btn);
    btn.addEventListener("click", async () => {
      const text = pre.textContent;
      try {
        await navigator.clipboard.writeText(text);
        btn.textContent = "Copied ✓";
        btn.classList.add("is-done");
      } catch {
        const range = document.createRange();
        range.selectNodeContents(pre);
        const selection = window.getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
        btn.textContent = "Selected — copy manually";
      }
      setTimeout(() => { btn.textContent = "Copy"; btn.classList.remove("is-done"); }, 1800);
    });
  });

  // ───────────── palette ─────────────
  const pal = $("[data-palette]");
  const palInput = $("[data-palette-input]");
  const palList = $("[data-palette-list]");
  const ICONS = {
    page: '<svg viewBox="0 0 24 24"><path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><path d="M14 3v6h6"/></svg>',
    hash: '<svg viewBox="0 0 24 24"><path d="M4 9h16M4 15h16M10 3 8 21M16 3l-2 18"/></svg>',
    book: '<svg viewBox="0 0 24 24"><path d="M4 19.5V5a2 2 0 0 1 2-2h14v16H6.5a2.5 2.5 0 0 0 0 5H20"/></svg>',
    down: '<svg viewBox="0 0 24 24"><path d="M12 3v12m0 0-5-5m5 5 5-5M4 21h16"/></svg>',
    play: '<svg viewBox="0 0 24 24"><path d="M7 4l13 8-13 8z"/></svg>',
    ext: '<svg viewBox="0 0 24 24"><path d="M14 4h6v6M10 14 20 4M19 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h5"/></svg>',
    sun: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2"/></svg>',
  };

  const goto = (href) => () => {
    const url = new URL(href, location.href);
    if (url.pathname === location.pathname && url.hash) {
      const target = document.getElementById(decodeURIComponent(url.hash.slice(1)));
      if (target) {
        if (target.tagName === "DETAILS") target.open = true;
        target.scrollIntoView({ behavior: reduceMotion ? "auto" : "smooth", block: "start" });
        history.replaceState(null, "", url.hash);
        return;
      }
    }
    location.href = url.href;
  };
  const openRepo = (path) => () => window.open(REPO + path, "_blank", "noopener");

  let commands = [];
  function addCommands(list) {
    commands = commands.concat(list.map((c) => ({ extra: "", icon: ICONS.hash, ...c, icon: ICONS[c.icon] || c.icon || ICONS.hash })));
  }

  addCommands([
    { group: "Pages", label: "Home", icon: "page", action: goto(ROOT || "./") },
    { group: "Pages", label: "Documentation", icon: "book", action: goto(ROOT + "docs/"), extra: "docs help guide" },
    { group: "Pages", label: "Download Trunq", icon: "down", action: goto(ROOT + "#download"), extra: "install mac windows linux" },
  ]);
  // Text from an h3 up to the next heading, for full-text search.
  function sectionText(h) {
    let t = "", n = h.nextElementSibling;
    while (n && !/^H[23]$/.test(n.tagName)) { t += " " + n.textContent; n = n.nextElementSibling; }
    return t;
  }
  // The docs page indexes its own headings; elsewhere we link to the most useful ones.
  const docHeadings = $$(".doc h2[id], .doc h3[id]");
  if (docHeadings.length) {
    addCommands(docHeadings.map((h) => ({
      group: "Docs", label: h.textContent.trim(), icon: "book", action: goto("#" + h.id),
      extra: h.closest("section")?.querySelector("h2")?.textContent || "",
      body: (h.tagName === "H2" ? h.closest("section").textContent : sectionText(h)).replace(/\s+/g, " ").toLowerCase(),
    })));
  } else {
    addCommands([
      ["Install Trunq", "install"], ["Nightly builds", "nightly-builds"], ["Connect to your first database", "first-steps"],
      ["Azure & Microsoft Entra ID", "azure"], ["Cloud authentication and token commands", "cloud-auth"], ["Extensions, pg_cron and auditing", "administration"], ["Find and enable PostgreSQL extensions", "extension-management"], ["Safety features", "safety"],
      ["ER diagrams & metadata", "relationships"], ["Docker & GitHub images", "docker"], ["Performance checks", "performance"],
      ["Settings", "settings"], ["Back up your workspace", "backup-restore"], ["TLS verification", "tls-modes"],
      ["Keyboard shortcuts", "shortcuts"], ["Troubleshooting", "troubleshooting"], ["Build from source", "build"],
    ].map(([label, id]) => ({ group: "Docs", label, icon: "book", action: goto(ROOT + "docs/#" + id) })));
  }
  addCommands([
    { group: "Links", label: "GitHub repository", icon: "ext", action: openRepo(""), extra: "source code" },
    { group: "Links", label: "Changelog", icon: "ext", action: openRepo("/blob/main/CHANGELOG.md"), extra: "release notes" },
    { group: "Links", label: "Report a bug", icon: "ext", action: openRepo("/issues/new/choose"), extra: "issue" },
    { group: "Links", label: "Contributing guide", icon: "ext", action: openRepo("/blob/main/CONTRIBUTING.md") },
  ]);
  const prefCommands = [{ group: "Preferences", label: "Toggle light / dark theme", icon: "sun", action: toggleTheme, extra: "theme" }];

  let results = [], active = 0, lastFocus = null;

  // Subsequence fuzzy match. Returns the score and matched indices, or null.
  function fuzzy(q, text) {
    if (!q) return { score: 0, idx: [] };
    const t = text.toLowerCase(); let ti = 0, score = 0, prev = -2; const idx = [];
    for (const ch of q.toLowerCase()) {
      if (ch === " ") continue;
      const f = t.indexOf(ch, ti);
      if (f < 0) return null;
      score += f === prev + 1 ? 3 : (f === 0 || t[f - 1] === " " ? 2 : 1) - Math.min(f - ti, 5) * 0.1;
      idx.push(f); prev = f; ti = f + 1;
    }
    return { score, idx };
  }
  const markText = (text, idx) => { const s = new Set(idx); return [...text].map((c, i) => (s.has(i) ? `<mark>${esc(c)}</mark>` : esc(c))).join(""); };

  function renderPalette() {
    const q = palInput.value.trim();
    // searchOnly entries (like every FAQ question) appear once you start typing.
    results = commands.concat(prefCommands).filter((c) => q || !c.searchOnly).map((c) => {
      const m = fuzzy(q, c.label) || (c.extra && fuzzy(q, c.extra) && { score: 0.5, idx: [] }) ||
        (c.body && q.length > 2 && c.body.includes(q.toLowerCase()) && { score: 0.3, idx: [] });
      return m && { ...c, ...m };
    }).filter(Boolean);
    if (q) results.sort((a, b) => b.score - a.score);
    active = Math.min(active, Math.max(0, results.length - 1));
    if (!results.length) {
      palList.innerHTML = `<li class="palette__empty">No results for “${esc(q)}”</li>`;
      palInput.removeAttribute("aria-activedescendant");
      return;
    }
    let html = "", group = null;
    results.forEach((r, i) => {
      if (!q && r.group !== group) { group = r.group; html += `<li class="palette__group" role="presentation">${group}</li>`; }
      html += `<li class="palette__item" role="option" id="pal-${i}" data-i="${i}" aria-selected="${i === active}">${r.icon}<span>${markText(r.label, r.idx)}</span>${q ? `<span class="hint-r">${r.group}</span>` : ""}</li>`;
    });
    palList.innerHTML = html;
    palInput.setAttribute("aria-activedescendant", "pal-" + active);
    $(`#pal-${active}`)?.scrollIntoView({ block: "nearest" });
  }

  function openPalette() {
    if (!pal.hidden) return;
    if (!menu.hidden) setMenu(false);
    lastFocus = document.activeElement;
    pal.hidden = false;
    palInput.value = "";
    active = 0;
    renderPalette();
    palInput.focus();
    document.body.style.overflow = "hidden";
  }
  function closePalette() {
    pal.hidden = true;
    document.body.style.overflow = "";
    lastFocus?.focus?.({ preventScroll: true });
  }
  function choose(i) {
    const r = results[i];
    if (!r) return;
    closePalette();
    r.action();
  }

  palInput.addEventListener("input", () => { active = 0; renderPalette(); });
  palInput.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!results.length) return;
      active = (active + (e.key === "ArrowDown" ? 1 : results.length - 1)) % results.length;
      renderPalette();
    } else if (e.key === "Enter") {
      e.preventDefault();
      choose(active);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closePalette();
    }
  });
  palList.addEventListener("mousemove", (e) => {
    const li = e.target.closest("[data-i]");
    if (li && +li.dataset.i !== active) { active = +li.dataset.i; renderPalette(); }
  });
  palList.addEventListener("click", (e) => { const li = e.target.closest("[data-i]"); if (li) choose(+li.dataset.i); });
  $$("[data-open-palette]").forEach((b) => b.addEventListener("click", openPalette));
  $$("[data-close-palette]").forEach((b) => b.addEventListener("click", closePalette));

  document.addEventListener("keydown", (e) => {
    if (!pal.hidden && e.key === "Escape") {
      e.preventDefault();
      closePalette();
      return;
    }
    if (!pal.hidden && e.key === "Tab") {
      const focusable = $$("input, button", pal).filter((el) => !el.disabled);
      const index = focusable.indexOf(document.activeElement);
      e.preventDefault();
      focusable[(index + (e.shiftKey ? focusable.length - 1 : 1)) % focusable.length].focus();
      return;
    }
    const mod = isMac ? e.metaKey : e.ctrlKey;
    if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "k") {
      e.preventDefault();
      pal.hidden ? openPalette() : closePalette();
      return;
    }
    if (pal.hidden && e.key === "/" && !mod && !e.target.matches("input, textarea, [contenteditable]")) {
      e.preventDefault();
      openPalette();
    }
  });

  // Highlight the nav link for the section under the reading line. A link can cover several
  // sections (data-spy="speed tour"); links to other pages are left alone.
  function spy(links) {
    const targets = links.map((a) => {
      const ids = a.dataset.spy ? a.dataset.spy.split(" ") : [a.getAttribute("href").split("#")[1]];
      return { a, els: ids.map((id) => id && document.getElementById(id)).filter(Boolean) };
    }).filter((t) => t.els.length);
    let queued = false;
    const update = () => {
      queued = false;
      const line = innerHeight * 0.35;
      const hit = targets.find((t) => t.els.some((el) => { const r = el.getBoundingClientRect(); return r.top <= line && r.bottom > line; }));
      targets.forEach((t) => t.a.toggleAttribute("aria-current", t === hit));
    };
    addEventListener("scroll", () => { if (!queued) { queued = true; requestAnimationFrame(update); } }, { passive: true });
    update();
  }

  // ───────────── mobile menu ─────────────
  const bar = $("[data-bar]");
  const menuBtn = $("[data-menu-btn]");
  const menu = $("[data-menu]");
  function setMenu(open) {
    menu.hidden = !open;
    menuBtn.setAttribute("aria-expanded", open);
    bar.classList.toggle("is-open", open);
    if (open) $("a", menu).focus({ preventScroll: true });
  }
  menuBtn.addEventListener("click", () => setMenu(menu.hidden));
  // Close after picking a link, on Escape, on outside click, or when the screen gets wide again.
  menu.addEventListener("click", (e) => { if (e.target.closest("a")) setMenu(false); });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !menu.hidden) { setMenu(false); menuBtn.focus(); }
  });
  document.addEventListener("click", (e) => { if (!menu.hidden && !bar.contains(e.target)) setMenu(false); });
  matchMedia("(min-width: 1101px)").addEventListener("change", (e) => { if (e.matches) setMenu(false); });

  // ───────────── FAQ helpers (data in faq.js) ─────────────
  const FAQ = window.TRUNQ_FAQ || { topics: [], items: [] };
  const docsHref = location.pathname.includes("/docs") ? "" : ROOT + "docs/";
  function faqAnswer(item) {
    return item.a
      .replace(/\{repo\}/g, REPO)
      .replace(/\{docs:([\w-]+)\}/g, (_, id) => `<a href="${docsHref}#${id}">the docs</a>`);
  }
  // Word-overlap search over questions (weighted) and answers. Returns [{item, score}] best first.
  const STOP = new Set("a an and are can do does for how i if in is it my of on or the to trunq what when where which will with you your".split(" "));
  const words = (t) => t.toLowerCase().replace(/<[^>]+>/g, " ").match(/[a-z0-9]+/g)?.filter((w) => w.length > 1 && !STOP.has(w)) || [];
  const faqIndex = FAQ.items.map((item) => ({ item, q: [...new Set(words(item.q))], k: [...new Set(words(item.k || ""))], a: [...new Set(words(item.a))] }));
  // A term matches a word it starts ("tunn" → "tunnel"); a longer form ("windows" → "window") counts for less.
  const pre = (list, t) => list.some((w) => w.startsWith(t));
  const longer = (list, t) => list.some((w) => w.length > 3 && t.startsWith(w));
  function faqSearch(text) {
    const terms = words(text);
    if (!terms.length) return [];
    return faqIndex.map(({ item, q, k, a }) => {
      let score = 0;
      for (const t of terms) score += pre(q, t) ? 3 : pre(k, t) ? 2.5 : longer(q, t) || longer(k, t) ? 2 : pre(a, t) ? 1 : 0;
      return { item, score: score / terms.length };
    }).filter((r) => r.score >= 0.75).sort((x, y) => y.score - x.score);
  }

  window.Trunq = { $, $$, esc, MOD, isMac, os, reduceMotion, addCommands, spy, goto, faq: { ...FAQ, answer: faqAnswer, search: faqSearch } };
})();
