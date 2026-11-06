// User Management: the Registry's users and units, through /api/v1/admin/registry/*. A password set
// here is temporary, and the server makes its owner replace it at their next sign-in.
const $ = (id) => document.getElementById(id);
const PER_PAGE = 10;
const SYSTEM = ["admin", "aumentumws", "mcconfig"];
const S = { users: [], groups: [], q: "", status: "all", unit: "", page: 1, writable: false, problem: null, loading: true };

function h(tag, attrs, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (k === "class") e.className = v;
    else if (k in e && k !== "list") e[k] = v;
    else e.setAttribute(k, v);
  }
  e.append(...kids.flat().filter((x) => x != null && x !== false));
  return e;
}

async function api(path, opts = {}) {
  const r = await fetch(path, { method: opts.method || "GET", headers: opts.body ? { "content-type": "application/json" } : {}, body: opts.body ? JSON.stringify(opts.body) : undefined });
  if (r.status === 401) { location.assign("/login"); throw new Error("Sign in first."); }
  const data = await r.json().catch(() => ({}));
  if (!r.ok) {
    const e = new Error(r.status === 404 ? "User management is not part of this server's build." : data.message || "The server could not be reached. Try again.");
    e.status = r.status; throw e;
  }
  return data;
}

const unitLabel = (g) => String(g || "").replace(/^GROUP_/, "");
const fullName = (u) => [u.first_name, u.last_name].filter(Boolean).join(" ") || u.user_name;
const statusOf = (u) => (!u.has_login || u.enabled == null ? "none" : u.enabled ? "active" : "disabled");
const isSystem = (u) => SYSTEM.includes(String(u.user_name).toLowerCase());

function toast(text) {
  const t = $("toast"); t.textContent = text; t.hidden = false;
  clearTimeout(toast.timer); toast.timer = setTimeout(() => { t.hidden = true; }, 7000);
}

// A password to read out: no characters that are easily confused.
function makePassword() {
  const letters = "abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ", digits = "23456789";
  return [...crypto.getRandomValues(new Uint8Array(12))].map((b, i) => { const set = i % 4 === 3 ? digits : letters; return set[b % set.length]; }).join("");
}

async function load() {
  S.loading = true; S.problem = null; render();
  try { const me = await api("/api/v1/me"); $("me").textContent = me.username || me.subject || ""; } catch { /* the header can wait */ }
  try { S.writable = !!(await api("/api/v1/registry/status")).write_enabled; } catch { S.writable = false; }
  try { [S.users, S.groups] = await Promise.all([api("/api/v1/admin/registry/users"), api("/api/v1/admin/registry/groups")]); }
  catch (e) { S.users = []; S.groups = []; S.problem = e.message; }
  S.loading = false; render();
}

function filtered() {
  const q = S.q.trim().toLowerCase();
  return S.users.filter((u) => (S.status === "all" || statusOf(u) === S.status)
    && (!S.unit || u.groups.includes(S.unit))
    && (!q || fullName(u).toLowerCase().includes(q) || u.user_name.toLowerCase().includes(q)));
}

function emptyRow(cols, title, text) {
  return h("tr", {}, h("td", { colSpan: cols, class: "empty" }, h("b", {}, title), text));
}

function statusPill(u) {
  const s = statusOf(u);
  return h("span", { class: "pill " + (s === "active" ? "on" : s === "disabled" ? "off" : "none") }, s === "active" ? "Active" : s === "disabled" ? "Disabled" : "No login");
}

function unitChips(u) {
  const shown = u.groups.slice(0, 2).map((g) => h("span", { class: "chip" }, unitLabel(g)));
  const rest = u.groups.length - shown.length;
  return h("div", { class: "chips", title: u.groups.map(unitLabel).join(", ") }, shown, rest > 0 ? h("span", { class: "chip more" }, `+${rest}`) : null);
}

function render() {
  const can = S.writable && !S.problem && !S.loading;
  $("addUser").disabled = !can; $("addUnit").disabled = !can; $("export").disabled = !S.users.length;
  const n = $("notice");
  n.textContent = S.problem ? "" : !S.loading && !S.writable ? "Changes are turned off on this server (CADASTRE_REGISTRY_WRITE), so users can be viewed but not changed." : "";
  n.hidden = !n.textContent;

  const list = filtered();
  const pages = Math.max(1, Math.ceil(list.length / PER_PAGE));
  S.page = Math.min(S.page, pages);
  const rows = list.slice((S.page - 1) * PER_PAGE, S.page * PER_PAGE).map((u) => h("tr", {},
    h("td", { class: "name" }, fullName(u)), h("td", {}, u.user_name), h("td", {}, statusPill(u)), h("td", {}, unitChips(u)),
    h("td", {}, h("button", { type: "button", class: "icon", title: `Edit ${fullName(u)}`, "aria-label": `Edit ${fullName(u)}`, onclick: () => editUser(u), innerHTML: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m4 20 1-4L16.5 4.5a2.1 2.1 0 0 1 3 3L8 19l-4 1ZM14.5 6.5l3 3"/></svg>' }))));
  $("rows").replaceChildren(...(S.loading ? [emptyRow(5, "Loading users…", "")]
    : S.problem ? [emptyRow(5, "Users could not be loaded", S.problem)]
    : rows.length ? rows : [emptyRow(5, "No users found", S.users.length ? "Clear the search or filters." : "Add the first user.")]));
  $("count").textContent = list.length ? `${list.length} user${list.length === 1 ? "" : "s"}` : "";
  $("pages").replaceChildren(...(pages < 2 ? [] : Array.from({ length: pages }, (_, i) => i + 1).filter((p) => p === 1 || p === pages || Math.abs(p - S.page) < 3)
    .map((p) => h("button", { type: "button", class: p === S.page ? "on" : "", "aria-current": p === S.page ? "page" : null, onclick: () => { S.page = p; render(); } }, String(p).padStart(2, "0")))));

  const keep = S.unit;
  $("unit").replaceChildren(h("option", { value: "" }, "All"), ...S.groups.map((g) => h("option", { value: g.group_name }, g.display_name || unitLabel(g.group_name))));
  $("unit").value = keep;

  $("unitRows").replaceChildren(...(S.loading ? [emptyRow(3, "Loading units…", "")]
    : S.problem ? [emptyRow(3, "Units could not be loaded", S.problem)]
    : S.groups.length ? S.groups.map((g) => h("tr", {}, h("td", { class: "name" }, g.display_name || unitLabel(g.group_name)), h("td", {}, g.group_name), h("td", {}, String(g.members))))
    : [emptyRow(3, "No units found", "")]));
}

function field(label, control, wide) { return h("div", { class: "f" + (wide ? " wide" : "") }, h("label", {}, label, control)); }

function unitChecks(checked) {
  return h("div", { class: "checks" }, S.groups.map((g) => h("label", {},
    h("input", { type: "checkbox", value: g.group_name, checked: checked.includes(g.group_name) }), g.display_name || unitLabel(g.group_name))));
}
const checkedUnits = (box) => [...box.querySelectorAll("input:checked")].map((i) => i.value);

// One dialog at a time. `save` throws to keep it open and show why.
function openDialog(title, sub, content, save, okLabel, left) {
  const err = h("div", { class: "dErr", role: "alert", hidden: true });
  const ok = save ? h("button", { type: "submit", class: "btn solid" }, okLabel) : null;
  const form = h("form", { method: "dialog", onsubmit: async (e) => {
    e.preventDefault(); if (!save) return close();
    err.hidden = true; ok.disabled = true;
    try { await save(); } catch (x) { err.textContent = x.message; err.hidden = false; }
    ok.disabled = false;
  } },
  h("div", { class: "dHead" }, title), sub ? h("div", { class: "dSub" }, sub) : null, content, err,
  h("div", { class: "dFoot" }, left ? h("div", { class: "left" }, left) : null,
    h("button", { type: "button", class: "btn", onclick: () => close() }, save ? "Cancel" : "Close"), ok));
  $("dlgBody").replaceChildren(form);
  if (!$("dlg").open) $("dlg").showModal();
}
function close() { $("dlg").close(); }

function showPassword(login, password, note) {
  openDialog("Temporary password", `Give this to ${login}. It is shown only once, and they must replace it at their next sign-in.`,
    h("div", {}, h("div", { class: "secret" }, password), note ? h("div", { class: "dSub" }, note) : null), null);
}

function addUser() {
  const first = h("input", { required: true, autocomplete: "off" }), last = h("input", { required: true, autocomplete: "off" });
  const login = h("input", { required: true, autocomplete: "off", pattern: "[A-Za-z0-9._\\-]+", title: "Letters, digits, dot, dash and underscore" });
  const password = h("input", { required: true, minLength: 6, autocomplete: "off", value: makePassword() });
  const ref = h("select", { required: true }, S.users.filter((u) => !isSystem(u)).map((u) => h("option", { value: u.user_name }, `${fullName(u)} (${u.user_name})`)));
  if (S.users.some((u) => u.user_name === "cgugin")) ref.value = "cgugin";
  const units = unitChecks([]);
  openDialog("Add User", "They get the units and office of the person copied, plus any units ticked here.",
    h("div", { class: "dForm" }, field("First name", first), field("Last name", last), field("Username", login),
      field("Temporary password", h("div", { class: "pw" }, password, h("button", { type: "button", class: "btn", onclick: () => { password.value = makePassword(); } }, "New"))),
      field("Copy units and office from", ref, true), h("div", { class: "f wide" }, h("span", { class: "lbl" }, "Units"), units)),
    async () => {
      const r = await api("/api/v1/admin/registry/users", { method: "POST", body: { user_name: login.value.trim(), first_name: first.value.trim(), last_name: last.value.trim(), password: password.value, ref_user: ref.value, extra_groups: checkedUnits(units).join(",") || null } });
      showPassword(login.value.trim().toLowerCase(), password.value, r.note); load();
    }, "Create user");
}

function editUser(u) {
  const first = h("input", { value: u.first_name || "" }), last = h("input", { value: u.last_name || "" }), office = h("input", { value: u.office_id || "" });
  const units = unitChecks(u.groups);
  const path = `/api/v1/admin/registry/users/${encodeURIComponent(u.user_name)}`;
  const locked = !S.writable || !!S.problem;
  const reset = isSystem(u) ? null : h("button", { type: "button", class: "btn", disabled: locked, onclick: async () => {
    if (!confirm(`Set a new temporary password for ${u.user_name}?`)) return;
    const password = makePassword();
    try { const r = await api(path + "/password", { method: "POST", body: { password } }); showPassword(u.user_name, password, r.note); } catch (x) { toast(x.message); }
  } }, "Reset password");
  const active = statusOf(u) === "active";
  const toggle = isSystem(u) || statusOf(u) === "none" ? null : h("button", { type: "button", class: "btn" + (active ? " danger" : ""), disabled: locked, onclick: async () => {
    if (!confirm(`${active ? "Disable" : "Enable"} ${u.user_name}?`)) return;
    try { const r = await api(path + "/enabled", { method: "POST", body: { enabled: !active } }); close(); toast(r.note); load(); } catch (x) { toast(x.message); }
  } }, active ? "Disable" : "Enable");
  openDialog(`Edit ${fullName(u)}`, u.user_name,
    h("div", { class: "dForm" }, field("First name", first), field("Last name", last), field("Office id", office, true),
      h("div", { class: "f wide" }, h("span", { class: "lbl" }, "Units"), units)),
    async () => {
      if (locked) throw new Error("Changes are turned off on this server.");
      const changed = {};
      if (first.value.trim() !== (u.first_name || "")) changed.first_name = first.value.trim();
      if (last.value.trim() !== (u.last_name || "")) changed.last_name = last.value.trim();
      if (office.value.trim() !== (u.office_id || "")) changed.office_id = office.value.trim();
      const want = checkedUnits(units), known = S.groups.map((g) => g.group_name);
      const add = want.filter((g) => !u.groups.includes(g)), drop = u.groups.filter((g) => known.includes(g) && !want.includes(g));
      if (!Object.keys(changed).length && !add.length && !drop.length) return close();
      let note = "Saved.";
      try {
        if (Object.keys(changed).length) note = (await api(path, { method: "PUT", body: changed })).note;
        for (const group of add) note = (await api(path + "/groups", { method: "POST", body: { group } })).note;
        for (const group of drop) note = (await api(`${path}/groups/${encodeURIComponent(group)}`, { method: "DELETE" })).note;
      } catch (x) { load(); throw x; }
      close(); toast(note); load();
    }, "Save changes", [reset, toggle]);
}

function addUnit() {
  const name = h("input", { required: true, autocomplete: "off", placeholder: "e.g. Title Deed Plan" });
  const ref = h("select", {}, h("option", { value: "" }, "Default (Map Compilation Officer)"), S.groups.map((g) => h("option", { value: g.group_name }, g.display_name || unitLabel(g.group_name))));
  openDialog("Add Unit", null, h("div", { class: "dForm" }, field("Unit name", name, true), field("Copy structure from", ref, true)), async () => {
    const r = await api("/api/v1/admin/registry/groups", { method: "POST", body: { display_name: name.value.trim(), ref_group: ref.value || null } });
    close(); toast(r.note); load();
  }, "Create unit");
}

// A cell starting with one of these would run as a formula in Excel.
const csvCell = (v) => { let s = String(v ?? ""); if (/^[=+\-@\t\r]/.test(s)) s = "'" + s; return `"${s.replace(/"/g, '""')}"`; };
function exportCsv() {
  const lines = [["Name", "Username", "Status", "Units", "Office"], ...filtered().map((u) => [fullName(u), u.user_name, statusOf(u), u.groups.map(unitLabel).join("; "), u.office_id])];
  const url = URL.createObjectURL(new Blob(["﻿" + lines.map((l) => l.map(csvCell).join(",")).join("\r\n")], { type: "text/csv" }));
  h("a", { href: url, download: "plagis-users.csv" }).click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function showView(units) {
  $("usersView").hidden = units; $("unitsView").hidden = !units;
  $("navUsers").classList.toggle("on", !units); $("navUnits").classList.toggle("on", units);
}

$("navUsers").onclick = () => showView(false);
$("navUnits").onclick = () => showView(true);
$("addUser").onclick = addUser;
$("addUnit").onclick = addUnit;
$("export").onclick = exportCsv;
$("q").oninput = (e) => { S.q = e.target.value; S.page = 1; render(); };
$("unit").onchange = (e) => { S.unit = e.target.value; S.page = 1; render(); };
document.querySelectorAll("[data-status]").forEach((b) => { b.onclick = () => {
  S.status = b.dataset.status; S.page = 1;
  document.querySelectorAll("[data-status]").forEach((x) => x.classList.toggle("on", x === b));
  render();
}; });
$("signOut").onclick = async () => { await fetch("/api/v1/auth/logout", { method: "POST" }).catch(() => {}); location.assign("/login"); };
load();
