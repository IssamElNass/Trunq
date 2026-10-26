"use strict";
const $ = (id) => document.getElementById(id);
const state = { csrf: "", connections: [], connected: false, busy: false, connection: null, schemas: [], tabs: [], active: 0, history: [], editingId: "", generation: 0 };
let noticeTimer;
function readStorage(key, fallback) { try { return JSON.parse(localStorage.getItem(key)) ?? fallback; } catch { return fallback; } }
function writeStorage(key, value) { try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* Storage is optional. */ } }
function element(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}
function notify(message) {
  $("notification").textContent = message;
  $("notification").hidden = false;
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => { $("notification").hidden = true; }, 8000);
}
async function api(path, data) {
  const response = await fetch(`/api/${path}`, {
    method: data === undefined ? "GET" : "POST", credentials: "same-origin",
    headers: data === undefined ? {} : { "Content-Type": "application/json", "X-Trunq-CSRF": state.csrf },
    body: data === undefined ? undefined : JSON.stringify(data)
  });
  let result;
  try { result = await response.json(); } catch { throw new Error(`Request failed (${response.status}).`); }
  if (!response.ok) throw new Error(result.error || `Request failed (${response.status}).`);
  return result;
}
function activeTab() { return state.tabs[state.active]; }
function persistTabs() {
  if (activeTab()) activeTab().sql = $("editor").value;
  writeStorage("trunq.web.tabs", state.tabs.map(({ title, sql }) => ({ title, sql })));
}
function setBusy(busy) {
  state.busy = busy;
  $("run").disabled = busy || !state.connected;
  $("cancel").disabled = !busy || !state.connected;
  $("refresh").disabled = busy || !state.connected;
  $("disconnect").disabled = busy || !state.connected;
  $("new-connection").disabled = busy;
  $("edit-connection").disabled = busy;
  $("connections").disabled = busy;
  $("fetch").disabled = busy;
  $("status").textContent = busy ? "Working…" : state.connected ? `Connected · ${state.connection.database}` : "Disconnected";
}
function renderTabs() {
  $("tabs").replaceChildren();
  state.tabs.forEach((tab, index) => {
    const button = element("button", tab.title, "tab");
    button.setAttribute("role", "tab");
    button.setAttribute("aria-selected", String(index === state.active));
    button.addEventListener("click", () => selectTab(index));
    const close = element("span", "×", "close-tab");
    close.setAttribute("role", "button");
    close.setAttribute("tabindex", "0");
    close.setAttribute("aria-label", `Close ${tab.title}`);
    const closeTab = (event) => {
      event.stopPropagation();
      if (state.busy) return notify("Wait for the running request before closing a tab.");
      persistTabs();
      state.tabs.splice(index, 1);
      if (!state.tabs.length) state.tabs.push(newTab());
      if (index < state.active) state.active--;
      state.active = Math.min(state.active, state.tabs.length - 1);
      $("editor").value = activeTab().sql;
      renderTabs(); renderResults(); persistTabs();
    };
    close.addEventListener("click", closeTab);
    close.addEventListener("keydown", (event) => { if (event.key === "Enter" || event.key === " ") closeTab(event); });
    button.append(close); $("tabs").append(button);
  });
}
function newTab(sql = "SELECT current_database(), current_user, version();", title) {
  return { title: title || `Query ${state.tabs.length + 1}`, sql, results: [], resultIndex: 0, queryId: "", summary: "Ready" };
}
function selectTab(index) {
  persistTabs(); state.active = index;
  $("editor").value = activeTab().sql;
  $("result-filter").value = "";
  renderTabs(); renderResults();
}
function addTab(sql, title) {
  persistTabs(); state.tabs.push(newTab(sql, title)); selectTab(state.tabs.length - 1); $("editor").focus();
}
function renderConnections() {
  const selected = $("connections").value;
  $("connections").replaceChildren(new Option("Choose a connection…", ""));
  for (const c of state.connections) $("connections").add(new Option(c.name || `${c.user}@${c.host}/${c.database}`, c.id));
  $("connections").value = selected;
}
function showConnection(connection = null) {
  const form = $("connection-form"); form.reset();
  state.editingId = connection?.id || "";
  if (connection) {
    for (const name of ["name", "host", "port", "database", "user", "ssl_mode", "environment"]) form.elements[name].value = connection[name];
    for (const name of ["root_ca", "client_cert", "client_key"]) form.elements[name].value = connection.tls[name] || "";
    form.elements.read_only.checked = connection.read_only;
  }
  $("connection-error").hidden = true;
  $("delete-connection").hidden = !state.editingId;
  $("connection-dialog").showModal();
}
function clearConnection() {
  state.connected = false; state.connection = null; state.schemas = []; state.generation++;
  for (const tab of state.tabs) for (const result of tab.results) result.more = false;
  $("identity").textContent = "No database connected";
  delete $("identity").dataset.environment;
  $("server-version").textContent = ""; $("transaction").textContent = "";
  renderCatalog(); setBusy(false); renderResults();
}
async function connect(event) {
  event.preventDefault();
  const form = $("connection-form"), values = new FormData(form);
  const connection = { id: state.editingId, name: values.get("name"), host: values.get("host"), port: Number(values.get("port")), database: values.get("database"), user: values.get("user"), ssl_mode: values.get("ssl_mode"), environment: values.get("environment"), read_only: values.has("read_only"), tls: { root_ca: values.get("root_ca"), client_cert: values.get("client_cert"), client_key: values.get("client_key") } };
  $("connect-submit").disabled = true; $("connection-error").hidden = true;
  try {
    const password = values.get("password");
    form.elements.password.value = "";
    const result = await api("connect", { connection, password: password || null });
    connection.id = result.connection.id;
    state.connected = true; state.connection = connection; state.generation++;
    for (const tab of state.tabs) for (const rows of tab.results) rows.more = false;
    $("identity").textContent = `${connection.environment === "None" ? "" : connection.environment.toUpperCase() + " · "}${connection.database} · ${connection.user}@${connection.host}${connection.read_only ? " · READ ONLY" : ""}`;
    $("identity").dataset.environment = connection.environment;
    $("server-version").textContent = `PostgreSQL ${result.server_version}`;
    $("transaction").textContent = "";
    if (values.has("save")) {
      try {
        const saved = await api("connections", connection); state.connections = saved.connections; renderConnections();
        const match = state.connections.find((c) => c.id === connection.id || (!connection.id && c.name === connection.name && c.host === connection.host && c.database === connection.database && c.user === connection.user));
        if (match) $("connections").value = match.id;
      } catch (error) { notify(`Connected, but saving failed: ${error.message}`); }
    }
    $("connection-dialog").close(); setBusy(false); renderResults();
    await refreshCatalog();
  } catch (error) { $("connection-error").textContent = error.message; $("connection-error").hidden = false; }
  finally { $("connect-submit").disabled = false; }
}
function renderCatalog() {
  const root = $("catalog"), filter = $("object-filter").value.toLowerCase(); root.replaceChildren();
  if (!state.connected) { root.append(element("p", "Connect to PostgreSQL to browse schemas, tables and views.", "hint")); return; }
  let count = 0;
  for (const schema of state.schemas) {
    const relations = schema.relations.filter((r) => `${schema.name}.${r.name}`.toLowerCase().includes(filter));
    if (!relations.length) continue;
    const details = element("details"); details.open = true;
    details.append(element("summary", `${schema.name} (${relations.length})`));
    for (const relation of relations) {
      const button = element("button", relation.name, "relation");
      button.title = `${relation.kind} · ${relation.rows < 0 ? "unknown" : relation.rows.toLocaleString()} estimated rows\n${relation.columns.map((c) => `${c.name}: ${c.type}`).join("\n")}`;
      if (relation.kind.includes("view")) button.append(element("small", "view"));
      button.addEventListener("click", () => { addTab(`SELECT * FROM ${relation.sql_name};`, relation.name); runQuery(); });
      details.append(button); count++;
    }
    root.append(details);
  }
  if (!count) root.append(element("p", "No matching tables or views.", "hint"));
}
async function refreshCatalog() {
  setBusy(true);
  try { const result = await api("catalog", {}); state.schemas = result.schemas; renderCatalog(); }
  catch (error) { notify(error.message); }
  finally { setBusy(false); }
}
function renderHistory() {
  $("history").replaceChildren(new Option("Recent queries…", ""));
  state.history.forEach((entry, i) => $("history").add(new Option(entry.sql.replace(/\s+/g, " ").slice(0, 70), String(i))));
}
async function runQuery() {
  if (!state.connected || state.busy) return;
  persistTabs();
  const tab = activeTab(), editor = $("editor");
  const sql = editor.selectionStart !== editor.selectionEnd ? editor.value.slice(editor.selectionStart, editor.selectionEnd) : editor.value;
  if (!sql.trim()) return;
  const generation = state.generation;
  setBusy(true); tab.summary = "Running…";
  // The server owns one cursor per browser. Starting another query replaces it.
  for (const other of state.tabs) for (const result of other.results) result.more = false;
  try {
    const result = await api("query", { sql });
    if (generation !== state.generation) return;
    tab.results = result.results; tab.resultIndex = result.results.length - 1; tab.queryId = result.query_id;
    tab.summary = `${result.ms} ms${result.notices.length ? " · " + result.notices.join(" · ") : ""}`;
    $("transaction").textContent = result.transaction === "Idle" ? "" : result.transaction === "Failed" ? "Transaction failed · run ROLLBACK" : "Transaction open · run COMMIT or ROLLBACK";
    state.history = [{ sql }, ...state.history.filter((entry) => entry.sql !== sql)].slice(0, 100);
    writeStorage("trunq.web.history", state.history); renderHistory();
  } catch (error) { tab.results = [{ error: error.message }]; tab.resultIndex = 0; tab.summary = "Request failed"; }
  finally { setBusy(false); renderResults(); }
}
function selectedResult() { return activeTab()?.results[activeTab().resultIndex]; }
function renderResults() {
  const tab = activeTab(), root = $("results"), result = selectedResult();
  $("result-tabs").replaceChildren();
  tab.results.forEach((_, index) => {
    const button = element("button", `Result ${index + 1}`);
    button.setAttribute("aria-pressed", String(index === tab.resultIndex));
    button.addEventListener("click", () => { tab.resultIndex = index; renderResults(); });
    $("result-tabs").append(button);
  });
  $("fetch").hidden = !result?.more || !state.connected;
  $("csv").disabled = $("json").disabled = !result?.columns?.length;
  if (!result) {
    root.replaceChildren();
    const empty = element("div", undefined, "empty-state");
    empty.append(element("div", "⌘", "empty-icon"), element("h1", "Your database, a little closer."), element("p", state.connected ? "Choose a table or run a SQL query to see results." : "Add a connection, explore a table, or write your first query."));
    if (!state.connected) { const button = element("button", "Add a connection", "primary"); button.addEventListener("click", () => showConnection()); empty.append(button); }
    root.append(empty); $("result-summary").textContent = tab.summary; return;
  }
  root.replaceChildren();
  if (result.error) { root.append(element("pre", result.error, "error")); $("result-summary").textContent = "Query failed · " + tab.summary; return; }
  const filter = $("result-filter").value.toLowerCase();
  const rows = result.rows.map((row, i) => ({ row, i })).filter(({ row }) => !filter || row.some((cell) => cell !== null && cell.toLowerCase().includes(filter)));
  const table = element("table"), head = element("thead"), headerRow = element("tr"), body = element("tbody");
  headerRow.append(element("th", "#", "row-number"));
  result.columns.forEach((column) => { const th = element("th", column.name); th.append(element("small", column.type)); headerRow.append(th); });
  head.append(headerRow); table.append(head);
  // Render a bounded window even after many pages have been fetched.
  const windowSize = 100;
  const rowHeight = 34;
  let rangeStart = -1;
  const spacer = (count) => {
    const tr = element("tr"), td = element("td"); td.colSpan = result.columns.length + 1;
    const space = element("div");
    space.style.height = `${count * rowHeight}px`;
    td.className = "spacer-cell"; td.append(space); tr.append(td); return tr;
  };
  const paint = () => {
    const start = Math.max(0, Math.floor(root.scrollTop / rowHeight) - 15);
    if (start === rangeStart) return;
    rangeStart = start; body.replaceChildren();
    if (start) body.append(spacer(start));
    for (const { row, i } of rows.slice(start, start + windowSize)) {
      const tr = element("tr"); tr.append(element("td", String(i + 1), "row-number"));
      for (const value of row) { const td = element("td", value === null ? "NULL" : value.replace(/[\r\n]/g, " ↵ "), value === null ? "null" : undefined); if (value !== null) td.title = value; tr.append(td); }
      body.append(tr);
    }
    const remaining = rows.length - start - windowSize;
    if (remaining > 0) body.append(spacer(remaining));
  };
  if (result.columns.length) { table.append(body); root.append(table); root.scrollTop = 0; root.onscroll = paint; paint(); }
  else { root.append(element("p", result.tag || "Statement completed.", "hint statement-complete")); root.onscroll = null; }
  $("result-summary").textContent = `${rows.length.toLocaleString()}${filter ? " matching / " + result.rows.length.toLocaleString() : ""} rows loaded${result.more ? " · more available" : ""}${result.truncated ? " · earlier statement limited to 500 rows" : ""} · ${tab.summary}`;
}
async function fetchMore() {
  const tab = activeTab(), result = selectedResult();
  if (!result?.more || state.busy) return;
  setBusy(true);
  try { const page = await api("fetch", { id: tab.queryId }); result.rows.push(...page.rows); result.more = page.more; result.tag = page.tag; }
  catch (error) { result.more = false; notify(error.message); }
  finally { setBusy(false); renderResults(); }
}
function download(name, content, type) {
  const url = URL.createObjectURL(new Blob([content], { type }));
  const link = element("a"); link.href = url; link.download = name; document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
function exportResult(format) {
  const result = selectedResult(); if (!result?.columns) return;
  if (format === "json") {
    // Keep PostgreSQL text values intact, including bigint and numeric precision.
    download("trunq-loaded-rows.json", JSON.stringify({ columns: result.columns, rows: result.rows }, null, 2), "application/json");
  } else {
    const csv = (value) => value === null ? "" : `"${value.replace(/"/g, '""')}"`;
    const lines = [result.columns.map((c) => csv(c.name)).join(","), ...result.rows.map((row) => row.map(csv).join(","))];
    download("trunq-loaded-rows.csv", lines.join("\r\n") + "\r\n", "text/csv;charset=utf-8");
  }
}
function handle(promise) { promise.catch((error) => notify(error.message)); }
$("new-connection").addEventListener("click", () => showConnection());
$("welcome-connect").addEventListener("click", () => showConnection());
$("edit-connection").addEventListener("click", () => showConnection(state.connections.find((c) => c.id === $("connections").value) || state.connection));
$("connections").addEventListener("change", () => { const connection = state.connections.find((c) => c.id === $("connections").value); if (connection) showConnection(connection); });
$("connection-form").addEventListener("submit", connect);
$("close-dialog").addEventListener("click", () => $("connection-dialog").close());
$("connection-dialog").addEventListener("close", () => { $("connection-form").elements.password.value = ""; });
$("delete-connection").addEventListener("click", () => handle((async () => { const result = await api("connections/delete", { id: state.editingId }); state.connections = result.connections; renderConnections(); $("connection-dialog").close(); })()));
$("disconnect").addEventListener("click", () => handle((async () => { await api("disconnect", {}); clearConnection(); })()));
$("refresh").addEventListener("click", refreshCatalog);
$("object-filter").addEventListener("input", renderCatalog);
$("new-tab").addEventListener("click", () => addTab());
$("run").addEventListener("click", runQuery);
$("cancel").addEventListener("click", () => handle(api("cancel", {})));
$("fetch").addEventListener("click", fetchMore);
$("result-filter").addEventListener("input", renderResults);
$("editor").addEventListener("input", persistTabs);
$("format").addEventListener("click", () => handle((async () => { const result = await api("format", { sql: $("editor").value }); $("editor").value = result.sql; persistTabs(); })()));
$("save-sql").addEventListener("click", () => download("query.sql", $("editor").value, "text/plain;charset=utf-8"));
$("open-sql").addEventListener("click", () => $("sql-file").click());
$("sql-file").addEventListener("change", () => handle((async () => { const file = $("sql-file").files[0]; if (file) { if (file.size > 1024 * 1024) throw new Error("Open SQL files up to 1 MiB."); addTab(await file.text(), file.name); } $("sql-file").value = ""; })()));
$("history").addEventListener("change", () => { if ($("history").value !== "") addTab(state.history[Number($("history").value)].sql); $("history").value = ""; });
$("csv").addEventListener("click", () => exportResult("csv"));
$("json").addEventListener("click", () => exportResult("json"));
$("theme").addEventListener("click", () => { const theme = document.documentElement.dataset.theme === "light" ? "dark" : "light"; document.documentElement.dataset.theme = theme; writeStorage("trunq.web.theme", theme); });
$("editor").addEventListener("keydown", (event) => {
  if ((event.ctrlKey || event.metaKey) && event.key === "Enter") { event.preventDefault(); runQuery(); }
  if (event.key === "Tab") { event.preventDefault(); const editor = $("editor"); editor.setRangeText("  ", editor.selectionStart, editor.selectionEnd, "end"); persistTabs(); }
});
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && state.busy && state.connected && !$("connection-dialog").open) handle(api("cancel", {}));
});
async function boot() {
  document.documentElement.dataset.theme = readStorage("trunq.web.theme", "dark");
  const saved = readStorage("trunq.web.tabs", []);
  state.tabs = Array.isArray(saved) ? saved.filter((t) => typeof t?.sql === "string" && typeof t?.title === "string").slice(0, 30).map((t) => newTab(t.sql, t.title)) : [];
  if (!state.tabs.length) state.tabs.push(newTab());
  const history = readStorage("trunq.web.history", []);
  state.history = Array.isArray(history) ? history.filter((h) => typeof h?.sql === "string").slice(0, 100) : [];
  $("editor").value = activeTab().sql; renderTabs(); renderHistory(); renderResults();
  const session = await api("session"); state.csrf = session.csrf; state.connections = session.connections; renderConnections();
  if (session.current) {
    const connection = session.current.connection;
    state.connected = true; state.connection = connection;
    $("identity").textContent = `${connection.environment === "None" ? "" : connection.environment.toUpperCase() + " · "}${connection.database} · ${connection.user}@${connection.host}${connection.read_only ? " · READ ONLY" : ""}`;
    $("identity").dataset.environment = connection.environment;
    $("server-version").textContent = `PostgreSQL ${session.current.server_version}`;
    $("connections").value = connection.id;
    await refreshCatalog();
  }
  setBusy(false); renderResults();
}
handle(boot());
