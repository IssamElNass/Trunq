// Home page: the scroll-drawn trunk, unfolding hero, million-row table, feature deck,
// "try it" console, the Tuesday switch, the receipt, FAQ chat and download dock. Needs site.js.

(() => {
  "use strict";

  const { $, $$, esc, MOD, isMac, reduceMotion, addCommands, spy } = window.Trunq;
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const smooth = reduceMotion ? "auto" : "smooth";

  // Stamps for the three classic mistakes; the console calls caught() when it stops one.
  const stamped = new Set();
  function caught(key) {
    const el = $(`[data-stamp="${key}"]`);
    if (!el || stamped.has(key)) return;
    stamped.add(key);
    el.classList.add("is-caught");
    $("[data-score]").textContent = stamped.size === 3
      ? "3 of 3 caught. That's a quiet on-call week."
      : `${stamped.size} of 3 caught. ${3 - stamped.size} to go.`;
  }

  // ───────────── SQL highlighter ─────────────
  const KEYWORDS = new Set(("select from where join left right inner outer on as and or not in is null group by order " +
    "having limit offset desc asc insert into values update set delete create index concurrently unique table alter " +
    "add column default drop truncate type using constraint foreign key references primary valid validate vacuum full " +
    "analyze explain begin commit rollback with distinct case when then else end lock exclusive access mode cascade " +
    "if exists text integer bigint numeric boolean timestamptz uuid jsonb varchar check returning").split(" "));
  const FUNCS = /^(count|sum|max|min|avg|now|random|gen_random_uuid|clock_timestamp|coalesce|lower|upper|uuid_generate_v4)$/i;
  const TOKEN = /(--[^\n]*)|('(?:[^']|'')*'?)|(\b\d+(?:\.\d+)?\b)|(:[a-z_]\w*)|([a-z_][\w$]*)|(::|[=<>!]+|[(),;.*+\-/])/gi;

  function highlight(sql, mark) {
    let out = "", last = 0, m;
    TOKEN.lastIndex = 0;
    while ((m = TOKEN.exec(sql))) {
      out += esc(sql.slice(last, m.index));
      const [t, cm, str, num, param, word, op] = m;
      let cls = "";
      if (cm) cls = "t-cm";
      else if (str) cls = "t-str";
      else if (num) cls = "t-num";
      else if (param) cls = "t-param";
      else if (word) {
        const lw = word.toLowerCase();
        if (lw === "true" || lw === "false") cls = "t-bool";
        else if (KEYWORDS.has(lw)) cls = "t-kw";
        else if (FUNCS.test(word) && sql[m.index + word.length] === "(") cls = "t-fn";
      } else if (op) cls = "t-op";
      if (mark && word && word.toUpperCase() === mark) cls += " t-new";
      out += cls ? `<span class="${cls.trim()}">${esc(t)}</span>` : esc(t);
      last = m.index + t.length;
    }
    return out + esc(sql.slice(last)) + "\n";
  }

  // ───────────── the console ─────────────
  const con = $("#console");
  const input = $("[data-input]", con);
  const hl = $("[data-hl]", con);
  const gutter = $("[data-gutter]", con);
  const out = $("[data-out]", con);
  const status = $("[data-status]", con);
  let mark = null; // word to flash after a quick fix
  let runId = 0; // cancels stale async runs
  let blockerActive = false;

  const PRESETS = {
    select: `-- Top customers by delivered revenue
SELECT c.full_name, c.country, c.is_vip,
       count(o.id)  AS orders,
       sum(o.total) AS revenue,
       max(o.placed_at) AS last_order
FROM shop.customers c
JOIN shop.orders o ON o.customer_id = c.id
WHERE o.status = 'delivered'
GROUP BY c.id
ORDER BY revenue DESC
LIMIT 500`,
    index: `-- Speed up the orders page
CREATE INDEX orders_customer_id_idx
  ON shop.orders (customer_id);`,
    default: `-- Track where each order came from
ALTER TABLE shop.orders
  ADD COLUMN source text DEFAULT gen_random_uuid()::text;`,
    nowhere: `-- Everyone in NL becomes VIP… right?
UPDATE shop.customers
SET is_vip = true;`,
    blocked: `-- deploy-migration is holding a lock on shop.customers
UPDATE shop.customers
SET is_vip = true
WHERE country = 'NL';`,
  };

  const CUSTOMERS = [
    ["Ines Silva", "NL", true, 10, "6640.37", "2026-10-20 13:49"],
    ["Dua Kim", "MA", false, 10, "6565.37", "2026-10-21 13:14"],
    ["Gus Berg", "NL", true, 10, "6514.26", "2026-10-21 04:19"],
    ["Chen Wang", "NL", true, 10, "6472.54", "2026-10-21 13:39"],
    ["Jun Lipa", "MA", false, 10, "6425.98", "2026-10-21 17:44"],
    ["Hiro Jansen", "MA", false, 10, "6404.21", "2026-10-20 00:34"],
    ["Bram Tanaka", "MA", false, 10, "6399.22", "2026-10-21 22:24"],
    ["Fatima Yılmaz", "MA", false, 10, "6339.79", "2026-10-20 22:24"],
    ["Noor Haddad", "NL", false, 9, "6301.10", "2026-10-21 09:02"],
    ["Lena Vos", "NL", true, 9, "6288.45", "2026-10-19 18:51"],
    ["Omar Benali", "MA", false, 9, "6270.03", "2026-10-21 11:27"],
    ["Sara de Wit", "NL", false, 9, "6244.90", "2026-10-20 07:15"],
  ];

  const ICON = {
    warn: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0zM12 9v4M12 17h.01"/></svg>',
    shield: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z"/><path d="m9 12 2 2 4-4"/></svg>',
    lock: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/></svg>',
  };

  function render() {
    hl.innerHTML = highlight(input.value, mark);
    const lines = input.value.split("\n").length;
    gutter.textContent = Array.from({ length: lines }, (_, i) => i + 1).join("\n");
    input.style.height = "auto";
    input.style.height = input.scrollHeight + "px";
  }

  function setStatus(text, kind = "") {
    status.textContent = text;
    status.className = "console__status" + (kind ? " is-" + kind : "");
  }

  function empty(html) {
    out.innerHTML = `<div class="out-empty"><p>${html}</p></div>`;
  }

  function loadPreset(name, { focus = false } = {}) {
    $$("[data-preset]", con).forEach((b) => b.setAttribute("aria-selected", b.dataset.preset === name));
    input.value = PRESETS[name];
    mark = null;
    runId++;
    blockerActive = name === "blocked";
    render();
    setStatus("Ready");
    empty(`Press <b>Run</b> or <b>${MOD} + ⏎</b>.<br>Feel free to edit the SQL, too.`);
    if (focus) input.focus({ preventScroll: true });
  }

  // Strip comments and string contents so the checks below only see SQL structure.
  function structure(sql) {
    return sql.replace(/--[^\n]*/g, " ").replace(/'(?:[^']|'')*'/g, "''").replace(/\s+/g, " ").trim();
  }

  const VOLATILE = /\bdefault\s+[^,;]*\b(now|random|gen_random_uuid|clock_timestamp|uuid_generate_v4|timeofday)\s*\(/i;

  // A small, honest subset of the rules Trunq's migration linter applies.
  function lint(s) {
    const found = [];
    if (/\bcreate\s+(unique\s+)?index\b/i.test(s) && !/\bconcurrently\b/i.test(s)) {
      found.push({
        title: "CREATE INDEX blocks writes",
        body: "A plain CREATE INDEX takes a SHARE lock. Reads still work, but every INSERT, UPDATE and DELETE on the table waits until the index has been built.",
        fix: "Build it with CONCURRENTLY. It takes longer but never blocks writes.",
        quick: "Add CONCURRENTLY",
      });
    }
    if (/\badd\s+(column\s+)?\w+/i.test(s) && VOLATILE.test(s)) {
      found.push({
        title: "Volatile DEFAULT rewrites the whole table",
        body: "A default like now() / random() / gen_random_uuid() must be computed per row, so PostgreSQL rewrites the table under an ACCESS EXCLUSIVE lock.",
        fix: "Add the column without a default (or with a constant), then backfill in batches.",
      });
    }
    if (/\balter\s+column\s+\w+\s+(set\s+data\s+)?type\b/i.test(s)) {
      found.push({
        title: "Changing a column type rewrites the table",
        body: "Most type changes rewrite every row and rebuild indexes while holding an ACCESS EXCLUSIVE lock.",
        fix: "Add a new column, backfill it in batches, then swap the columns.",
      });
    }
    if (/\bset\s+not\s+null\b/i.test(s)) {
      found.push({
        title: "SET NOT NULL scans the whole table",
        body: "PostgreSQL checks every row while holding an ACCESS EXCLUSIVE lock.",
        fix: "Add CHECK (col IS NOT NULL) NOT VALID, VALIDATE it, then SET NOT NULL.",
      });
    }
    if (/\b(foreign\s+key|references)\b/i.test(s) && /\balter\s+table\b/i.test(s) && !/\bnot\s+valid\b/i.test(s)) {
      found.push({
        title: "Adding a foreign key locks both tables",
        body: "Validating the constraint scans the table while blocking writes to both tables.",
        fix: "Add it with NOT VALID, then run VALIDATE CONSTRAINT separately.",
      });
    }
    if (/\bvacuum\s+(\(\s*)?full\b/i.test(s)) {
      found.push({
        title: "VACUUM FULL rewrites the table",
        body: "The table is locked for reads and writes until the rewrite finishes.",
        fix: "Use a plain VACUUM, or pg_repack for an online rebuild.",
      });
    }
    return found;
  }

  async function run({ dry = false } = {}) {
    const id = ++runId;
    const s = structure(input.value);
    const head = s.split(" ")[0]?.toLowerCase() || "";
    const isDml = /^(insert|update|delete)$/.test(head);
    const hasWhere = /\bwhere\b/i.test(s);

    if (!s) { setStatus("Nothing to run"); return empty("The editor is empty. Pick an example above."); }

    if (dry) {
      if (!isDml) {
        setStatus("Nothing to preview", "warn");
        return empty(head === "select"
          ? "Preview is for statements that <b>change</b> data.<br>Looking things up is always safe."
          : "Preview covers data changes. Structure changes go through<br>Trunq's <b>safety check</b>. Press Run to see it.");
      }
      return dryRun(id, s, hasWhere);
    }

    const issues = lint(s);
    if (issues.length) return showLint(id, issues);

    if (/^(drop|truncate)$/.test(head)) {
      return confirmBox(id, "danger", `${head.toUpperCase()} permanently removes data`,
        "There's no undo button for this one, so Trunq always asks first.", false);
    }
    if (/^(update|delete)$/.test(head) && !hasWhere) {
      return confirmBox(id, "danger", `${head.toUpperCase()} without WHERE affects every row`,
        `This would change <b>all 12,408 customers</b>, not just the ones in the Netherlands. Want to preview it first?`, true);
    }
    if (isDml && blockerActive && /\bcustomers\b/i.test(s)) return blocked(id, s);
    if (head === "select" && /\bcustomers\b/i.test(s)) return selectCustomers(id);

    setStatus("Running…", "busy");
    out.innerHTML = "";
    await sleep(350);
    if (id !== runId) return;
    if (isDml) {
      setStatus("Done · 9 ms", "ok");
      return message(`<span class="ok">${head.toUpperCase()} ${hasWhere ? 412 : 12408}</span> · committed (simulated)`);
    }
    if (/^(create|alter)$/.test(head)) {
      setStatus("Done · 4 ms", "ok");
      return message(`<span class="ok">${head.toUpperCase()} ${s.split(" ")[1].toUpperCase()}</span>. The linter found nothing risky.`);
    }
    setStatus("Simulated", "warn");
    empty("This demo isn't connected to a real database. It only knows<br><b>shop.customers</b> and <b>shop.orders</b>. In the app, your rows stream in here.");
  }

  function message(html) {
    const t = new Date().toTimeString().slice(0, 8);
    out.insertAdjacentHTML("beforeend", `<div class="msg"><time>${t}</time>${html}</div>`);
  }

  async function selectCustomers(id) {
    setStatus("Running…", "busy");
    out.innerHTML = "";
    await sleep(240);
    if (id !== runId) return;
    out.innerHTML = `<table class="grid"><thead><tr><th>#</th>
      <th>full_name<small>text</small></th><th>country<small>character</small></th><th>is_vip<small>boolean</small></th>
      <th>orders<small>bigint</small></th><th>revenue<small>numeric</small></th><th>last_order<small>timestamptz</small></th>
      </tr></thead><tbody></tbody></table>
      <div class="out-foot"><b>500 rows</b><span>28 ms</span><span>48.5 KB</span><span>streamed via server-side cursor</span></div>`;
    const body = $("tbody", out);
    for (const [i, r] of CUSTOMERS.entries()) {
      if (id !== runId) return;
      body.insertAdjacentHTML("beforeend", `<tr><td class="rn">${i + 1}</td><td>${esc(r[0])}</td><td>${r[1]}</td>
        <td class="b">${r[2]}</td><td class="n">${r[3]}</td><td class="n">${r[4]}</td><td class="d">${r[5]}</td></tr>`);
      if (!reduceMotion) await sleep(28);
    }
    setStatus("500 rows · 28 ms", "ok");
  }

  function showLint(id, issues) {
    caught("lock");
    setStatus("Paused by Trunq", "warn");
    const quick = issues.find((i) => i.quick);
    out.innerHTML = `<div class="alert" role="alertdialog" aria-label="This may lock production tables">
      <div class="alert__title">${ICON.warn}This may lock production tables</div>
      ${issues.map((i) => `<div class="alert__item"><h4>${i.title}</h4>${esc(i.body)}<div class="fix">${esc(i.fix)}</div></div>`).join("")}
      <div class="alert__item"><h4 class="info">Tip: set a lock_timeout</h4>Even a fast DDL statement waits in the lock queue behind long-running queries, and everything else queues behind it.<div class="fix">Run SET lock_timeout = '5s'; first, and retry if it times out.</div></div>
      <div class="alert__actions">
        ${quick ? `<button class="abtn abtn--primary" data-act="fix">✦ ${quick.quick}</button>` : ""}
        <button class="abtn abtn--danger" data-act="anyway">Run anyway</button>
        <button class="abtn" data-act="cancel">Cancel</button>
      </div></div>`;
    actions(id, {
      fix: () => {
        input.value = input.value.replace(/\bcreate(\s+unique)?\s+index\b/i, (m) => m.replace(/index$/i, "INDEX CONCURRENTLY"));
        mark = "CONCURRENTLY";
        render();
        run();
      },
      anyway: async () => {
        setStatus("Running…", "busy");
        out.innerHTML = "";
        await sleep(500);
        if (id !== runId) return;
        setStatus("Done (simulated)", "danger");
        message(`<span class="warn">Simulated.</span> On a big production table, this would freeze it for everyone until it finished. That's exactly what Trunq warned you about.`);
      },
    });
    $("[data-act]", out)?.focus({ preventScroll: true });
  }

  function confirmBox(id, kind, title, body, offerDry) {
    if (offerDry) caught("mass");
    setStatus("Waiting for confirmation", "warn");
    out.innerHTML = `<div class="alert alert--${kind}" role="alertdialog" aria-label="${esc(title)}">
      <div class="alert__title">${ICON.warn}${esc(title)}</div>
      <div class="alert__item">${body}</div>
      <div class="alert__actions">
        ${offerDry ? `<button class="abtn abtn--primary" data-act="dry">${ICON.shield} Preview first</button>` : ""}
        <button class="abtn abtn--danger" data-act="anyway">Run anyway</button>
        <button class="abtn" data-act="cancel">Cancel</button>
      </div></div>`;
    actions(id, {
      dry: () => run({ dry: true }),
      anyway: async () => {
        setStatus("Running…", "busy");
        out.innerHTML = "";
        await sleep(400);
        if (id !== runId) return;
        setStatus("Done (simulated)", "danger");
        message(`<span class="err">Simulated.</span> Nothing was harmed here, but on a real database you'd be restoring from a backup now.`);
      },
    });
    $("[data-act]", out)?.focus({ preventScroll: true });
  }

  // One delegated handler for the buttons inside the output pane; each run installs its own.
  let actHandlers = {};
  function actions(id, handlers) {
    actHandlers = {
      cancel: () => {
        runId++;
        setStatus("Cancelled");
        empty("Cancelled. Nothing ran.");
      },
      ...handlers,
    };
    actHandlers.id = id;
  }
  out.addEventListener("click", (e) => {
    const b = e.target.closest("[data-act]");
    const fn = b && actHandlers.id === runId && actHandlers[b.dataset.act];
    if (!fn) return;
    actHandlers = {};
    fn();
  });

  async function dryRun(id, s, hasWhere) {
    setStatus("Previewing…", "busy");
    out.innerHTML = "";
    await sleep(420);
    if (id !== runId) return;
    const rows = CUSTOMERS.filter((r) => !r[2] && (!hasWhere || r[1] === "NL")).slice(0, 6);
    const n = hasWhere ? 412 : 12408;
    out.innerHTML = `<div class="alert alert--ok" role="status">
      <div class="alert__title">${ICON.shield}Preview: ${n.toLocaleString("en-US")} rows would change. Nothing was saved.</div>
      <div class="alert__item">Trunq ran it in a transaction, showed you exactly what would change, then undid it.</div></div>
      <table class="grid"><thead><tr><th>#</th><th>full_name<small>text</small></th><th>country<small>character</small></th><th>is_vip<small>boolean</small></th></tr></thead><tbody>
      ${rows.map((r, i) => `<tr><td class="rn">${i + 1}</td><td>${esc(r[0])}</td><td>${r[1]}</td>
        <td class="b"><span class="was">false</span><span class="now">true</span></td></tr>`).join("")}
      </tbody></table>
      <div class="out-foot"><b>${n.toLocaleString("en-US")} rows</b><span>rolled back</span><span>${hasWhere ? "" : "Tip: add WHERE country = 'NL' to only change NL customers"}</span></div>`;
    setStatus("Previewed · nothing saved", "ok");
  }

  async function blocked(id) {
    const started = performance.now();
    setStatus("Running… 0.00 s", "busy");
    out.innerHTML = "";
    message(`Connected to shop (PostgreSQL 17.10), backend pid 74209`);
    const tick = setInterval(() => {
      if (id !== runId) return clearInterval(tick);
      setStatus(`Running… ${((performance.now() - started) / 1000).toFixed(2)} s`, "busy");
    }, 50);
    await sleep(1500);
    if (id !== runId) return clearInterval(tick);
    const age0 = 13;
    caught("jam");
    out.insertAdjacentHTML("afterbegin", `<div class="lockbar" role="alert">
      <div class="lockbar__head">${ICON.lock}<strong>Blocked.</strong> Another session is holding the table. Your query can't continue until it's done:</div>
      <div class="lockbar__session">
        <button class="abtn" data-act="cancelq">Cancel its query</button>
        <button class="abtn abtn--warn" data-act="terminate">Terminate session</button>
        <span><b>pid 74202</b></span><span>dev · deploy-migration · active</span>
        <span class="age">transaction open <span data-age>${age0}.00</span> s</span>
        <span class="q">BEGIN; LOCK TABLE shop.customers IN ACCESS EXCLUSIVE MODE; SELECT pg_sleep(600);</span>
      </div></div>`);
    const ageEl = $("[data-age]", out);
    const ageTick = setInterval(() => {
      if (id !== runId || !ageEl.isConnected) return clearInterval(ageTick);
      ageEl.textContent = (age0 + (performance.now() - started) / 1000 - 1.5).toFixed(2);
    }, 50);
    const resolve = (how) => async () => {
      clearInterval(ageTick);
      blockerActive = false;
      $(".lockbar", out).remove();
      message(`<span class="warn">${how}</span>. The table is free again.`);
      await sleep(260);
      clearInterval(tick);
      if (id !== runId) return;
      const secs = ((performance.now() - started) / 1000).toFixed(2);
      message(`<span class="ok">UPDATE 412</span> · ${secs} s`);
      setStatus(`Done · ${secs} s`, "ok");
    };
    actions(id, {
      cancelq: resolve("Query of pid 74202 cancelled"),
      terminate: resolve("Session 74202 terminated"),
    });
  }

  // Editor wiring
  input.addEventListener("input", () => { mark = null; render(); });
  input.addEventListener("keydown", (e) => {
    const mod = isMac ? e.metaKey : e.ctrlKey;
    if (mod && e.key === "Enter") {
      e.preventDefault();
      run({ dry: e.altKey });
    } else if (e.key === "Tab" && !e.shiftKey) {
      e.preventDefault();
      const { selectionStart: a, selectionEnd: b } = input;
      input.setRangeText("  ", a, b, "end");
      render();
    } else if (e.key === "Escape" && status.classList.contains("is-busy")) {
      runId++;
      setStatus("Cancelled");
      message(`<span class="err">ERROR: canceling statement due to user request</span>`);
    }
  });
  $("[data-run]", con).addEventListener("click", () => run());
  $("[data-dry]", con).addEventListener("click", () => run({ dry: true }));
  $$("[data-preset]", con).forEach((b) => b.addEventListener("click", () => loadPreset(b.dataset.preset)));
  $(".console__presets", con).addEventListener("keydown", (e) => {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    const tabs = $$("[data-preset]", con);
    const i = tabs.indexOf(document.activeElement);
    const next = tabs[(i + (e.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length];
    next.focus();
    loadPreset(next.dataset.preset);
  });
  loadPreset("select");
  $$("[data-try]").forEach((b) => b.addEventListener("click", () => {
    loadPreset(b.dataset.try);
    if (matchMedia("(max-width: 1100px)").matches) con.scrollIntoView({ behavior: reduceMotion ? "auto" : "smooth", block: "center" });
    run();
  }));

  // ───────────── unfolding hero + annotation lines ─────────────
  const unfold = $("[data-unfold]");
  const pinLines = $("[data-pin-lines]");
  const notes = $$(".annotations li", unfold);
  const wide = matchMedia("(min-width: 1101px)");

  function placeNotes() {
    pinLines.innerHTML = "";
    if (!wide.matches || !unfold.classList.contains("is-flat")) return;
    const box = unfold.getBoundingClientRect();
    const win = $(".window", unfold).getBoundingClientRect();
    let lastBottom = { left: -Infinity, right: -Infinity };
    const paths = [];
    for (const li of notes) {
      const pin = $(`[data-pin="${li.dataset.for}"]`, unfold).getBoundingClientRect();
      const side = li.dataset.side;
      const px = pin.left + pin.width / 2 - box.left, py = pin.top + pin.height / 2 - box.top;
      const h = li.offsetHeight;
      const top = Math.max(py - h / 2, lastBottom[side] + 16);
      lastBottom[side] = top + h;
      li.style.top = `${top}px`;
      const ex = side === "left" ? win.left - box.left - 26 : win.right - box.left + 26;
      const ey = top + 14;
      const cx = (px + ex) / 2;
      paths.push(`<path d="M${px},${py} C${cx},${py} ${cx},${ey} ${ex},${ey}"/><circle cx="${ex}" cy="${ey}" r="3"/>`);
    }
    pinLines.innerHTML = paths.join("");
  }

  // ───────────── the trunk: one line that draws itself down the page ─────────────
  const main = $("#main");
  const trunk = $("[data-trunk]");
  const ink = $(".trunk__ink", trunk), ghost = $(".trunk__ghost", trunk), tip = $(".trunk__tip", trunk);
  let trunkTable = [], trunkLen = 0;

  function buildTrunk() {
    const mainBox = main.getBoundingClientRect();
    const W = main.clientWidth, H = main.scrollHeight;
    trunk.setAttribute("viewBox", `0 0 ${W} ${H}`);
    trunk.style.height = H + "px";
    const wrap = Math.min(1200, W - 48);
    const gutter = Math.max(20, (W - wrap) / 4);
    const X = [gutter, W - gutter];
    const y = (el) => el.getBoundingClientRect().top - mainBox.top;
    const stops = $$("[data-trunk-stop]");
    const end = $("[data-trunk-end]").getBoundingClientRect();
    const bL = end.left - mainBox.left, bR = end.right - mainBox.left, bT = end.top - mainBox.top, bB = end.bottom - mainBox.top;

    // Start under the hero window, then weave between the margins, crossing in the gaps between sections.
    const fig = unfold.getBoundingClientRect();
    let x = W / 2, cy = fig.bottom - mainBox.top + 40;
    let d = `M${x},${cy}`;
    stops.forEach((s, i) => {
      const nx = X[i % 2];
      const sy = y(s) + 200;
      const k = (sy - cy) * 0.75;
      d += ` C${x},${cy + k} ${nx},${sy - k} ${nx},${sy}`;
      x = nx; cy = sy;
      const next = stops[i + 1] ? y(stops[i + 1]) - 90 : bT - 60;
      d += ` L${x},${next}`;
      cy = next;
    });
    // Finish like the logo's tail: sweep under the download button and hook up beside it.
    const sgn = x > (bL + bR) / 2 ? 1 : -1;
    const g = 30, under = bB + g, mid = (bL + bR) / 2, hookX = sgn > 0 ? bL - g : bR + g;
    d += ` C${x},${under + 50} ${mid + sgn * 220},${under} ${mid},${under}`;
    d += ` C${hookX - sgn * 4},${under} ${hookX},${under - 4} ${hookX},${(bT + bB) / 2}`;
    ghost.setAttribute("d", d);
    ink.setAttribute("d", d);
    trunkLen = ink.getTotalLength();
    ink.style.strokeDasharray = `${trunkLen}`;
    // Lookup table from page height to path length (monotonic max-y so the curl draws last).
    trunkTable = [];
    let maxY = 0;
    for (let l = 0; l <= trunkLen; l += 12) {
      maxY = Math.max(maxY, ink.getPointAtLength(l).y);
      trunkTable.push([l, maxY]);
    }
    trunkTable.push([trunkLen, Infinity]);
  }

  function drawTrunk() {
    if (!trunkLen) return;
    const mainTop = main.getBoundingClientRect().top;
    const pen = -mainTop + innerHeight * 0.72;
    const nearEnd = innerHeight + scrollY >= document.documentElement.scrollHeight - 40;
    let len = 0;
    if (reduceMotion || nearEnd) len = trunkLen;
    else for (const [l, my] of trunkTable) { if (my <= pen) len = l; else break; }
    ink.style.strokeDashoffset = `${trunkLen - len}`;
    const p = ink.getPointAtLength(len);
    tip.setAttribute("cx", p.x);
    tip.setAttribute("cy", p.y);
    tip.style.opacity = len > 4 ? 1 : 0;
  }

  // ───────────── scroll loop ─────────────
  let ticking = false;
  function onScroll() {
    if (ticking) return;
    ticking = true;
    requestAnimationFrame(() => {
      ticking = false;
      const r = unfold.getBoundingClientRect();
      const p = reduceMotion ? 0 : clamp((r.top - innerHeight * 0.12) / (innerHeight * 0.55), 0, 1);
      unfold.style.setProperty("--p", p.toFixed(3));
      const flat = p < 0.06;
      if (flat !== unfold.classList.contains("is-flat")) unfold.classList.toggle("is-flat", flat);
      placeNotes();
      drawTrunk();
    });
  }
  function relayout() { buildTrunk(); onScroll(); }
  addEventListener("scroll", onScroll, { passive: true });
  addEventListener("resize", relayout);
  addEventListener("load", relayout);
  new ResizeObserver(() => relayout()).observe(main);
  relayout();

  // ───────────── a million rows ─────────────
  const N = 1_000_000, RH = 34;
  const vScroll = $("[data-vgrid-scroll]"), vSpacer = $("[data-vgrid-spacer]"), vRows = $("[data-vgrid-rows]");
  const rowNow = $("[data-row-now]");
  const FIRST = ["Ines", "Dua", "Gus", "Chen", "Jun", "Hiro", "Bram", "Fatima", "Noor", "Lena", "Omar", "Sara", "Yuki", "Mila", "Tariq", "Ava", "Luca", "Zara", "Ilya", "Amara"];
  const LAST = ["Silva", "Kim", "Berg", "Wang", "Lipa", "Jansen", "Tanaka", "Yılmaz", "Haddad", "Vos", "Benali", "de Wit", "Novak", "Okafor", "Rossi", "Larsen", "Moreau", "Singh"];
  const CITY = ["Amsterdam", "Casablanca", "Lisbon", "Seoul", "Oslo", "Utrecht", "Tokyo", "Istanbul", "Ghent", "Rabat", "Berlin", "Toronto", "Porto", "Lyon"];
  const STATUS = [["delivered", "ok"], ["shipped", "info"], ["paid", "info"], ["pending", "warn"], ["cancelled", "bad"]];
  const fmt = (n) => n.toLocaleString("en-US");
  const hash = (i, s) => { let h = (i + 1) * 2654435761 + s * 40503; h ^= h >>> 15; h = Math.imul(h, 2246822519); h ^= h >>> 13; return h >>> 0; };

  let spacerH = 0, vTop = -1;
  function sizeGrid() {
    spacerH = Math.min(N * RH, 8_000_000);
    vSpacer.style.height = spacerH + "px";
    vTop = -1;
    renderGrid();
  }
  function renderGrid() {
    const st = vScroll.scrollTop, view = vScroll.clientHeight;
    if (st === vTop) return;
    vTop = st;
    const visible = Math.ceil(view / RH) + 1;
    const max = Math.max(1, spacerH - view);
    const exact = (st / max) * (N - visible + 1);
    const first = Math.min(N - visible, Math.floor(exact));
    const frac = exact - first;
    let html = "";
    for (let i = first; i < Math.min(N, first + visible + 1); i++) {
      const h = hash(i, 1), h2 = hash(i, 2);
      const [st2, cls] = STATUS[h2 % STATUS.length];
      html += `<div class="vrow"><span class="rn">${fmt(i + 1)}</span><span>${FIRST[h % FIRST.length]} ${LAST[h2 % LAST.length]}</span><span>${CITY[(h >>> 8) % CITY.length]}</span><span class="n">$${(5 + (h % 99000) / 100).toFixed(2)}</span><span><i class="pill pill--${cls}">${st2}</i></span></div>`;
    }
    vRows.innerHTML = html;
    vRows.style.transform = `translateY(${st - frac * RH}px)`;
    rowNow.textContent = fmt(first + 1);
  }
  vScroll.addEventListener("scroll", () => requestAnimationFrame(renderGrid), { passive: true });
  addEventListener("resize", sizeGrid);
  sizeGrid();

  function glide(to) {
    const from = vScroll.scrollTop, t0 = performance.now(), dur = reduceMotion ? 0 : 1100;
    const step = (now) => {
      const k = dur ? Math.min(1, (now - t0) / dur) : 1;
      const e = k < 0.5 ? 4 * k * k * k : 1 - Math.pow(-2 * k + 2, 3) / 2;
      vScroll.scrollTop = from + (to - from) * e;
      if (k < 1) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }
  $$("[data-jump]").forEach((b) => b.addEventListener("click", () => {
    const max = spacerH - vScroll.clientHeight;
    if (b.dataset.jump === "end") {
      const atEnd = vScroll.scrollTop > max - 2;
      glide(atEnd ? 0 : max);
      b.textContent = atEnd ? "Jump to row 1,000,000" : "Back to row 1";
    } else {
      glide(Math.random() * max);
    }
  }));

  // ───────────── feature deck ─────────────
  const cards = $$(".deck__card");
  const picks = $$("[data-card]");
  let order = cards.map((_, i) => i);

  function layoutDeck() {
    order.forEach((ci, k) => {
      const c = cards[ci];
      const tilt = k === 0 ? 0 : (k % 2 ? 2.4 : -1.8) * Math.min(k, 3) / 1.4;
      c.style.transform = `translate(${k * 22}px, ${-k * 18}px) rotate(${tilt}deg) scale(${1 - k * 0.05})`;
      c.style.zIndex = String(50 - k);
      c.style.opacity = k < 4 ? "1" : "0";
      c.style.filter = k ? `brightness(${1 - k * 0.14}) saturate(${1 - k * 0.2})` : "none";
      c.setAttribute("aria-hidden", k ? "true" : "false");
    });
  }
  async function pick(i) {
    if (order[0] === i) return;
    const prev = cards[order[0]];
    picks.forEach((b) => b.setAttribute("aria-pressed", +b.dataset.card === i));
    if (!reduceMotion) {
      prev.classList.add("is-flying");
      await sleep(260);
      prev.classList.remove("is-flying");
    }
    order = [i, ...order.filter((x) => x !== i && x !== order[0]), order[0]];
    layoutDeck();
  }
  picks.forEach((b) => b.addEventListener("click", () => pick(+b.dataset.card)));
  $("[data-deck]").addEventListener("click", () => pick(order[1]));
  layoutDeck();

  // ───────────── the Tuesday switch ─────────────
  const flipSec = $("#flip");
  const sw = $("[data-switch]");
  const flipWord = $("[data-flip-word]");
  let flipTouched = false;
  function setFlip(on) {
    flipSec.classList.toggle("is-with", on);
    sw.setAttribute("aria-checked", on);
    flipWord.textContent = on ? "with" : "without";
  }
  sw.addEventListener("click", () => { flipTouched = true; setFlip(sw.getAttribute("aria-checked") !== "true"); });
  // Show the trick once: flip on its own shortly after the section comes into view.
  new IntersectionObserver(([e], io) => {
    if (!e.isIntersecting) return;
    io.disconnect();
    setTimeout(() => { if (!flipTouched) setFlip(true); }, reduceMotion ? 0 : 1800);
  }, { threshold: 0.6 }).observe($(".day"));

  // ───────────── receipt ─────────────
  $("[data-receipt-date]").textContent = new Date().toLocaleDateString("en-GB", { day: "2-digit", month: "short", year: "numeric" }).toUpperCase();
  new IntersectionObserver(([e], io) => {
    if (e.isIntersecting) { $("[data-receipt]").classList.add("is-printed"); io.disconnect(); }
  }, { threshold: 0.35 }).observe($("[data-receipt]"));

  // ───────────── FAQ chat (questions live in faq.js) ─────────────
  const faq = window.Trunq.faq;
  const thread = $("[data-thread]");
  const chips = $("[data-chips]");
  const topicsEl = $("[data-topics]");
  const askForm = $("[data-ask]");
  const askInput = $("[data-ask-input]");
  const AVATAR = '<img src="assets/app-icon.png" alt="" width="28" height="28">';
  const asked = new Set();
  let topic = faq.topics[0]?.[0];

  const say = (cls, html) => {
    thread.insertAdjacentHTML("beforeend", `<div class="bubble bubble--${cls}">${html}</div>`);
    thread.scrollTo({ top: thread.scrollHeight, behavior: smooth });
    return thread.lastElementChild;
  };
  async function reply(html) {
    const typing = say("them", `${AVATAR}<span class="typing" aria-label="Typing"><i></i><i></i><i></i></span>`);
    await sleep(reduceMotion ? 0 : 600);
    typing.innerHTML = `${AVATAR}<span>${html}</span>`;
    thread.scrollTo({ top: thread.scrollHeight, behavior: smooth });
  }
  function chip(item) {
    return `<button type="button" data-q="${item.id}"${asked.has(item.id) ? ' class="is-asked"' : ""}>${esc(item.q)}</button>`;
  }
  function renderTopics() {
    topicsEl.innerHTML = faq.topics.map(([id, label]) => {
      const n = faq.items.filter((i) => i.topic === id).length;
      return `<button type="button" role="tab" data-topic="${id}" aria-selected="${id === topic}" tabindex="${id === topic ? 0 : -1}">${label} <span>${n}</span></button>`;
    }).join("");
  }
  function renderChips(list = faq.items.filter((i) => i.topic === topic)) {
    chips.innerHTML = list.map(chip).join("");
  }
  async function ask(item, typed) {
    asked.add(item.id);
    say("me", esc(typed || item.q));
    renderChips(askInput.value.trim() ? faq.search(askInput.value).slice(0, 4).map((r) => r.item) : undefined);
    await reply(typed ? `<b>${esc(item.q)}</b><br>${faq.answer(item)}` : faq.answer(item));
  }

  topicsEl.addEventListener("click", (e) => {
    const b = e.target.closest("[data-topic]");
    if (!b) return;
    topic = b.dataset.topic;
    askInput.value = "";
    renderTopics();
    renderChips();
  });
  topicsEl.addEventListener("keydown", (e) => {
    const d = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
    if (!d) return;
    const ids = faq.topics.map((t) => t[0]);
    topic = ids[(ids.indexOf(topic) + d + ids.length) % ids.length];
    renderTopics();
    renderChips();
    $(`[data-topic="${topic}"]`, topicsEl).focus();
  });
  chips.addEventListener("click", (e) => {
    const b = e.target.closest("[data-q]");
    if (b) ask(faq.items.find((i) => i.id === b.dataset.q));
  });
  // Typing narrows the chips to the closest questions; Enter asks the best match.
  askInput.addEventListener("input", () => {
    const q = askInput.value.trim();
    topicsEl.classList.toggle("is-dim", !!q);
    if (!q) return renderChips();
    const hits = faq.search(q).slice(0, 4).map((r) => r.item);
    chips.innerHTML = hits.length ? hits.map(chip).join("") : `<p class="chat__none">No exact match yet. Press Enter and I'll point you somewhere useful.</p>`;
  });
  askForm.addEventListener("submit", async (e) => {
    e.preventDefault();
    const q = askInput.value.trim();
    if (!q) return askInput.focus();
    const [best] = faq.search(q);
    if (best) return ask(best.item, q);
    say("me", esc(q));
    await reply(`I don't have an answer for that one yet. Try <a href="docs/">the docs</a> (or press ${MOD}K to search them), or <a href="https://github.com/IssamElNass/trunq/issues/new/choose">ask on GitHub</a> and we'll add it here.`);
  });
  renderTopics();
  renderChips();

  // ───────────── download dock ─────────────
  const OS = {
    mac: ["Download for macOS", "Trunq-<version>-macos-universal.dmg", "macOS 11 or newer, Apple Silicon & Intel"],
    win: ["Download for Windows", "Trunq-<version>-windows-x86_64.zip", "Windows 10 & 11, 64-bit"],
    deb: ["Download for Debian / Ubuntu", "trunq_<version>_amd64.deb", "64-bit, installs with apt"],
    linux: ["Download for Linux", "trunq-<version>-linux-x86_64.tar.gz", "64-bit, unpack and run"],
  };
  const osButtons = $$("[data-os-pick] [role=radio]");
  function pickOs(os) {
    osButtons.forEach((b) => { const on = b.dataset.os === os; b.setAttribute("aria-checked", on); b.tabIndex = on ? 0 : -1; });
    $("[data-desk]").dataset.osView = os;
    const [label, file, req] = OS[os];
    $("[data-dl-label]").textContent = label;
    $("[data-dl-file]").textContent = file;
    $("[data-dl-req]").textContent = req;
    relayout(); // the button width changed, so the trunk's hook moves with it
  }
  osButtons.forEach((b, i) => {
    b.addEventListener("click", () => pickOs(b.dataset.os));
    b.addEventListener("keydown", (e) => {
      const d = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
      if (!d) return;
      e.preventDefault();
      const n = osButtons[(i + d + osButtons.length) % osButtons.length];
      pickOs(n.dataset.os);
      n.focus();
    });
  });
  pickOs(window.Trunq.os || "mac");
  // The dock icon bounces like a launching app before heading to the release page.
  $(".desk__app").addEventListener("click", async (e) => {
    if (reduceMotion || e.metaKey || e.ctrlKey) return;
    e.preventDefault();
    const a = e.currentTarget;
    a.classList.add("is-bouncing");
    await sleep(900);
    a.classList.remove("is-bouncing");
    location.href = a.href;
  });

  // ───────────── reveal, nav, palette ─────────────
  const revealObserver = new IntersectionObserver((entries) => {
    for (const e of entries) {
      if (e.isIntersecting) { e.target.classList.add("is-in"); revealObserver.unobserve(e.target); }
    }
  }, { threshold: 0.12 });
  $$(".million__copy, .million__table, .deck-head, .index li, .try__copy, .flip__head, .bill__copy, .ask__inner > div, .download__inner > *").forEach((el, i) => {
    el.classList.add("reveal");
    el.style.transitionDelay = `${(i % 4) * 50}ms`;
    revealObserver.observe(el);
  });

  spy($$(".bar__nav a"));

  const jump = (sel) => () => $(sel).scrollIntoView({ behavior: smooth });
  addCommands([
    { group: "On this page", label: "Scroll a million rows", action: jump("#speed"), extra: "speed demo" },
    { group: "On this page", label: "Features", action: jump("#tour") },
    { group: "On this page", label: "Try it in your browser", icon: "play", action: jump("#try"), extra: "demo console" },
    { group: "On this page", label: "With and without Trunq", action: jump("#flip"), extra: "compare" },
    { group: "On this page", label: "Questions", action: jump("#faq"), extra: "faq" },
    ...faq.items.map((item) => ({
      group: "Questions", label: item.q, extra: "faq question", body: `${item.a} ${item.k || ""}`.toLowerCase(), searchOnly: true,
      action: () => { $("#faq").scrollIntoView({ behavior: smooth }); ask(item); },
    })),
    ...[["index", "Freeze a table"], ["nowhere", "Update everyone"], ["blocked", "Get stuck"], ["default", "Add a column"]].map(([p, label]) => ({
      group: "Try it", label: `Try: ${label}`, icon: "play", extra: "demo",
      action: () => { con.scrollIntoView({ behavior: smooth, block: "center" }); loadPreset(p, { focus: true }); },
    })),
  ]);
})();
