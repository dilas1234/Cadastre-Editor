// The sign-in page: username, password and workstation, then the first-time password change.
const $ = (id) => document.getElementById(id);
const LAST_VIEW = "cad.workstation";
let next = "/digitising", current = "";

$("year").textContent = new Date().getFullYear();
try { const v = localStorage.getItem(LAST_VIEW); if (v) $("view").value = v; } catch { /* storage */ }

function show(box, text) { box.textContent = text || ""; box.hidden = !text; }

async function post(path, body) {
  const r = await fetch(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  const data = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(data.message || "The server could not be reached. Try again.");
  return data;
}

function busy(form, on) { form.querySelector(".primary").disabled = on; }

function askForNewPassword(knownCurrent) {
  current = knownCurrent || "";
  $("signIn").hidden = true; $("changePw").hidden = false;
  $("currentRow").hidden = !!current;
  (current ? $("newpw") : $("current")).focus();
}

$("signIn").addEventListener("submit", async (e) => {
  e.preventDefault();
  const username = $("username").value.trim(), password = $("password").value, view = $("view").value;
  if (!username || !password) return show($("msg"), "Enter your username and password.");
  if (!view) return show($("msg"), "Select your workstation.");
  show($("msg"), ""); busy(e.target, true);
  try {
    const r = await post("/api/v1/auth/login", { username, password, view, remember: $("remember").checked });
    try { localStorage.setItem(LAST_VIEW, view); } catch { /* storage */ }
    next = r.redirect;
    if (r.must_change) askForNewPassword(password); else location.assign(next);
  } catch (err) {
    show($("msg"), err.message); $("password").value = ""; $("password").focus();
  } finally { busy(e.target, false); }
});

$("changePw").addEventListener("submit", async (e) => {
  e.preventDefault();
  const pw = $("newpw").value;
  if (pw !== $("confirm").value) return show($("msg2"), "The two new passwords do not match.");
  show($("msg2"), ""); busy(e.target, true);
  try {
    await post("/api/v1/auth/change-password", { current_password: current || $("current").value, new_password: pw });
    location.assign(next);
  } catch (err) { show($("msg2"), err.message); } finally { busy(e.target, false); }
});

document.querySelectorAll("[data-eye]").forEach((b) => b.addEventListener("click", () => {
  const input = $(b.dataset.eye), reveal = input.type === "password";
  input.type = reveal ? "text" : "password";
  b.setAttribute("aria-pressed", String(reveal));
  b.setAttribute("aria-label", reveal ? "Hide password" : "Show password");
}));
document.querySelectorAll("[data-help]").forEach((b) => b.addEventListener("click", () => { $("help").hidden = false; }));

const q = new URLSearchParams(location.search);
if (q.get("change")) askForNewPassword("");
if (q.get("denied")) show($("msg"), "Your account cannot open that workstation. Choose another one.");
