import { invoke } from "@tauri-apps/api/core";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

// ---- Types mirrored from the Rust side -------------------------------------------------

interface SavedMapInfo {
  key: string;
  map: string;
  mode: string;
  difficulty: string;
  gameType: number;
  round: number;
  boss?: string;
  elite: boolean;
  tier?: number;
  challengeId?: string;
  gameId: number;
  attemptId: number;
  matchWon: boolean;
  gameVersion?: string;
}

interface ProfileInfo {
  gameVersion: string | null;
  rank: number | null;
  monkeyMoney: number | null;
  savedMaps: SavedMapInfo[];
  lastPlayedKey: string | null;
  summary: string | null;
}

interface Status {
  running: boolean;
  processName: string;
  profilePath: string;
  profileExists: boolean;
  mapSavePath: string;
  mapSaveExists: boolean;
  savesDir: string;
  live: ProfileInfo | null;
  liveError: string | null;
  defaultProfilePath: string;
  defaultMapSavePath: string;
}

interface Meta {
  id: string;
  name: string;
  note: string;
  createdAt: string;
  gameVersion: string | null;
  rank: number | null;
  monkeyMoney: number | null;
  savedMaps: SavedMapInfo[];
  lastPlayedKey: string | null;
  summary: string | null;
  mapSaveDataPresent: boolean;
  mapSaveHasGame: boolean;
  readable: boolean;
  error?: string;
  lastRestoredAt?: string;
}

interface Settings {
  window: unknown;
  profilePath: string | null;
  mapSavePath: string | null;
}

interface RestoreReport {
  meta: Meta;
  profileWritten: string;
  mapSaveWritten: string | null;
}

// ---- State -----------------------------------------------------------------------------

let status: Status | null = null;
let snapshots: Meta[] = [];
let settingsOpen = false;
let settingsAutoOpened = false;
let busy = false;

const app = document.getElementById("app")!;

// ---- Helpers ---------------------------------------------------------------------------

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | boolean | undefined> = {},
  ...children: (Node | string | null | undefined)[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === false) continue;
    if (k === "class") node.className = String(v);
    else if (k === "text") node.textContent = String(v);
    else if (v === true) node.setAttribute(k, "");
    else node.setAttribute(k, v);
  }
  for (const c of children) {
    if (c === null || c === undefined) continue;
    node.append(typeof c === "string" ? document.createTextNode(c) : c);
  }
  return node;
}

function relativeTime(iso: string, now = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 5) return "just now";
  if (s < 60) return `${s} s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.round(h / 24);
  if (d < 14) return `${d} d ago`;
  return new Date(t).toLocaleDateString();
}

function fullTime(iso: string): string {
  const t = Date.parse(iso);
  return Number.isNaN(t) ? iso : new Date(t).toLocaleString();
}

function statsLine(x: { rank: number | null; monkeyMoney: number | null }): string {
  const parts: string[] = [];
  if (x.rank !== null && x.rank !== undefined) parts.push(`Rank ${x.rank}`);
  if (x.monkeyMoney !== null && x.monkeyMoney !== undefined)
    parts.push(`${x.monkeyMoney.toLocaleString()} MM`);
  return parts.join(" · ");
}

function toast(message: string, kind: "ok" | "err" = "ok") {
  const t = el("div", { class: `toast ${kind}`, text: message });
  document.body.append(t);
  requestAnimationFrame(() => t.classList.add("show"));
  setTimeout(() => {
    t.classList.remove("show");
    setTimeout(() => t.remove(), 300);
  }, kind === "err" ? 6000 : 2500);
}

function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}

// ---- Dialogs ---------------------------------------------------------------------------

function modal(build: (close: () => void) => HTMLElement): Promise<void> {
  return new Promise((resolve) => {
    const overlay = el("div", { class: "overlay" });
    const close = () => {
      overlay.remove();
      document.removeEventListener("keydown", onKey);
      resolve();
    };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") close();
    };
    document.addEventListener("keydown", onKey);
    const box = el("div", { class: "modal", role: "dialog", "aria-modal": "true" }, build(close));
    overlay.append(box);
    overlay.addEventListener("mousedown", (ev) => {
      if (ev.target === overlay) close();
    });
    document.body.append(overlay);
  });
}

function confirmDialog(opts: {
  title: string;
  message: string;
  warnings?: string[];
  okLabel: string;
  danger?: boolean;
}): Promise<boolean> {
  let result = false;
  return modal((close) => {
    const ok = el("button", { class: opts.danger ? "btn danger" : "btn primary", text: opts.okLabel });
    ok.addEventListener("click", () => {
      result = true;
      close();
    });
    const cancel = el("button", { class: "btn", text: "Cancel" });
    cancel.addEventListener("click", close);
    setTimeout(() => cancel.focus(), 0);
    return el(
      "div",
      {},
      el("h2", { text: opts.title }),
      el("p", { text: opts.message }),
      ...(opts.warnings ?? []).map((w) => el("p", { class: "warn", text: w })),
      el("div", { class: "modal-actions" }, cancel, ok),
    );
  }).then(() => result);
}

function nameDialog(defaultName: string, hint: string | null): Promise<{ name: string; note: string } | null> {
  let result: { name: string; note: string } | null = null;
  return modal((close) => {
    const name = el("input", { type: "text", class: "input", placeholder: "Snapshot name" });
    name.value = defaultName;
    const note = el("textarea", { class: "input", rows: "2", placeholder: "Note (optional)" });
    const submit = () => {
      const n = name.value.trim();
      if (!n) {
        name.focus();
        return;
      }
      result = { name: n, note: note.value.trim() };
      close();
    };
    name.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter") {
        ev.preventDefault();
        submit();
      }
    });
    note.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey)) submit();
    });
    const ok = el("button", { class: "btn primary", text: "Save" });
    ok.addEventListener("click", submit);
    const cancel = el("button", { class: "btn", text: "Cancel" });
    cancel.addEventListener("click", close);
    setTimeout(() => {
      name.focus();
      name.select();
    }, 0);
    return el(
      "div",
      {},
      el("h2", { text: "Save current" }),
      hint ? el("p", { class: "hint", text: hint }) : null,
      el("label", { class: "field" }, el("span", { text: "Name" }), name),
      el("label", { class: "field" }, el("span", { text: "Note" }), note),
      el("div", { class: "modal-actions" }, cancel, ok),
    );
  }).then(() => result);
}

// ---- Actions ---------------------------------------------------------------------------

async function refreshStatus() {
  try {
    status = await invoke<Status>("get_status");
  } catch (e) {
    console.error(e);
  }
  renderHeader();
  if (status && !status.profileExists && !settingsAutoOpened) {
    settingsAutoOpened = true;
    settingsOpen = true;
    renderSettings();
  }
  refreshTimes();
}

async function refreshList() {
  try {
    snapshots = await invoke<Meta[]>("list_snapshots");
  } catch (e) {
    toast(errText(e), "err");
  }
  renderList();
}

async function saveCurrent() {
  if (busy) return;
  const live = status?.live ?? null;
  const defaultName = live?.summary ?? `Snapshot ${new Date().toLocaleString()}`;
  const hint = status?.running ? "BTD6 is running. Best taken from the main menu." : null;
  const r = await nameDialog(defaultName, hint);
  if (!r) return;
  busy = true;
  renderHeader();
  try {
    const meta = await invoke<Meta>("create_snapshot", { name: r.name, note: r.note });
    toast(meta.readable ? `Saved "${meta.name}"` : `Saved "${meta.name}" (unreadable, may be corrupt)`,
      meta.readable ? "ok" : "err");
    await refreshList();
  } catch (e) {
    toast(`Save failed: ${errText(e)}`, "err");
  } finally {
    busy = false;
    renderHeader();
  }
}

async function loadSnapshot(meta: Meta) {
  if (busy) return;
  if (status?.running) {
    toast("Close BTD6 first", "err");
    return;
  }
  const warnings: string[] = [];
  const liveVer = status?.live?.gameVersion ?? null;
  if (liveVer && meta.gameVersion && liveVer !== meta.gameVersion) {
    warnings.push(
      `Game version differs: snapshot is ${meta.gameVersion}, current profile is ${liveVer}. ` +
        "Restoring across versions may drop trophy store items.",
    );
  }
  if (!meta.readable) {
    warnings.push("This snapshot could not be decrypted when it was taken. It may be corrupt.");
  }
  const ok = await confirmDialog({
    title: "Load snapshot",
    message: `Overwrite your current BTD6 save with '${meta.name}'? This cannot be undone.`,
    warnings,
    okLabel: "Overwrite",
    danger: true,
  });
  if (!ok) return;
  busy = true;
  try {
    const report = await invoke<RestoreReport>("restore_snapshot", { id: meta.id });
    toast(`Loaded "${report.meta.name}"`);
    await refreshList();
    await refreshStatus();
  } catch (e) {
    toast(`Load failed: ${errText(e)}`, "err");
  } finally {
    busy = false;
  }
}

async function deleteSnapshot(meta: Meta) {
  const ok = await confirmDialog({
    title: "Delete snapshot",
    message: `Delete '${meta.name}'? The snapshot folder is removed permanently.`,
    okLabel: "Delete",
    danger: true,
  });
  if (!ok) return;
  try {
    await invoke("delete_snapshot", { id: meta.id });
    toast(`Deleted "${meta.name}"`);
  } catch (e) {
    toast(`Delete failed: ${errText(e)}`, "err");
  }
  await refreshList();
}

async function commitName(meta: Meta, input: HTMLInputElement) {
  const name = input.value.trim();
  if (!name) {
    input.value = meta.name;
    return;
  }
  if (name === meta.name) return;
  try {
    const updated = await invoke<Meta>("rename_snapshot", { id: meta.id, name });
    Object.assign(meta, updated);
  } catch (e) {
    toast(`Rename failed: ${errText(e)}`, "err");
    input.value = meta.name;
  }
}

async function commitNote(meta: Meta, input: HTMLInputElement) {
  const note = input.value.trim();
  if (note === meta.note) return;
  try {
    const updated = await invoke<Meta>("set_note", { id: meta.id, note });
    Object.assign(meta, updated);
  } catch (e) {
    toast(`Saving note failed: ${errText(e)}`, "err");
    input.value = meta.note;
  }
}

// ---- Rendering -------------------------------------------------------------------------

const header = el("header", { class: "header" });
const hintBar = el("div", { class: "hintbar", hidden: true });
const settingsPane = el("section", { class: "settings", hidden: true });
const list = el("main", { class: "list" });
app.append(header, hintBar, settingsPane, list);

let headerSignature = "";

function renderHeader() {
  const running = status?.running ?? false;
  // Only rebuild when something visible changed, so a click never lands on a replaced button.
  const signature = JSON.stringify([
    status !== null,
    running,
    busy,
    status?.profileExists,
    status?.liveError,
  ]);
  if (signature === headerSignature) return;
  headerSignature = signature;
  header.replaceChildren();
  const dotTitle = status
    ? running
      ? `${status.processName} is running`
      : "BTD6 not running"
    : "Checking…";
  const dot = el("span", {
    class: `dot ${status ? (running ? "red" : "green") : "grey"}`,
    title: dotTitle,
    "aria-label": dotTitle,
  });
  const title = el("h1", {}, el("span", { text: "BTD6 Save States" }), dot);
  const gear = el("button", {
    class: "btn icon",
    title: "Settings (file paths)",
    "aria-label": "Settings",
    text: "⚙",
  });
  gear.addEventListener("click", () => {
    settingsOpen = !settingsOpen;
    renderSettings();
  });
  const save = el("button", { class: "btn primary", text: busy ? "Saving…" : "Save current" });
  save.disabled = busy || !status?.profileExists;
  if (!status?.profileExists) save.title = "Profile.Save not found. Set the path in settings.";
  save.addEventListener("click", saveCurrent);
  header.append(title, el("div", { class: "spacer" }), gear, save);

  const hints: string[] = [];
  if (running) hints.push("BTD6 is running. Snapshots are best taken from the main menu.");
  if (status?.liveError) hints.push(`Live profile could not be read: ${status.liveError}`);
  if (status && !status.profileExists) hints.push("Profile.Save not found. Check the paths in settings.");
  hintBar.hidden = hints.length === 0;
  hintBar.replaceChildren(...hints.map((h) => el("div", { text: h })));

  // Keep row Load buttons in sync with the running state.
  for (const b of list.querySelectorAll<HTMLButtonElement>("button[data-load]")) {
    b.disabled = running || busy;
    b.title = running ? "Close BTD6 first" : "";
  }
}

function renderSettings() {
  settingsPane.hidden = !settingsOpen;
  if (!settingsOpen) return;
  settingsPane.replaceChildren();
  const profile = el("input", { type: "text", class: "input mono", placeholder: status?.defaultProfilePath ?? "" });
  const mapSave = el("input", { type: "text", class: "input mono", placeholder: status?.defaultMapSavePath ?? "" });
  invoke<Settings>("get_settings")
    .then((s) => {
      profile.value = s.profilePath ?? "";
      mapSave.value = s.mapSavePath ?? "";
    })
    .catch(() => {});

  const browse = (input: HTMLInputElement, title: string) => {
    const b = el("button", { class: "btn small", text: "Browse…" });
    b.addEventListener("click", async () => {
      try {
        const picked = await openFileDialog({ title, multiple: false, directory: false });
        if (typeof picked === "string") input.value = picked;
      } catch (e) {
        toast(errText(e), "err");
      }
    });
    return b;
  };

  const found = (ok: boolean | undefined) =>
    el("span", { class: `found ${ok ? "yes" : "no"}`, text: ok ? "found" : "missing" });

  const apply = el("button", { class: "btn primary small", text: "Apply" });
  apply.addEventListener("click", async () => {
    try {
      await invoke<Settings>("set_paths", { profilePath: profile.value, mapSavePath: mapSave.value });
      toast("Paths saved");
      await refreshStatus();
      renderSettings();
    } catch (e) {
      toast(errText(e), "err");
    }
  });
  const reset = el("button", { class: "btn small", text: "Use defaults" });
  reset.addEventListener("click", () => {
    profile.value = "";
    mapSave.value = "";
  });
  const closeBtn = el("button", { class: "btn small", text: "Close" });
  closeBtn.addEventListener("click", () => {
    settingsOpen = false;
    renderSettings();
  });

  settingsPane.append(
    el("h2", { text: "File paths" }),
    el("p", { class: "muted", text: "Leave a field empty to use the resolved default shown as placeholder." }),
    el("label", { class: "field" },
      el("span", {}, "Profile.Save ", found(status?.profileExists)),
      el("div", { class: "row" }, profile, browse(profile, "Pick Profile.Save"))),
    el("label", { class: "field" },
      el("span", {}, "MapSaveData ", found(status?.mapSaveExists)),
      el("div", { class: "row" }, mapSave, browse(mapSave, "Pick MapSaveData"))),
    el("p", { class: "muted mono", text: `Snapshots: ${status?.savesDir ?? ""}` }),
    el("div", { class: "modal-actions" }, closeBtn, reset, apply),
  );
}

function renderRow(meta: Meta): HTMLElement {
  const name = el("input", { type: "text", class: "name", title: "Click to rename", spellcheck: "false" });
  name.value = meta.name;
  name.addEventListener("blur", () => commitName(meta, name));
  name.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") name.blur();
    if (ev.key === "Escape") {
      name.value = meta.name;
      name.blur();
    }
  });

  const time = el("span", { class: "time", "data-iso": meta.createdAt, title: fullTime(meta.createdAt), text: relativeTime(meta.createdAt) });

  const infoParts: string[] = [];
  if (meta.summary) infoParts.push(meta.summary);
  else if (meta.readable) infoParts.push("No in-progress games");
  const stats = statsLine(meta);
  const info = el("div", { class: "info" },
    el("span", { text: infoParts.join(" · ") }),
    stats ? el("span", { class: "stats", text: stats }) : null,
    meta.gameVersion ? el("span", { class: "ver", title: "Game version", text: `v${meta.gameVersion}` }) : null,
    !meta.readable ? el("span", { class: "bad", title: meta.error ?? "", text: "unreadable, may be corrupt" }) : null,
    meta.mapSaveHasGame ? el("span", { class: "tag", title: "MapSaveData held a mid-run autosave when this snapshot was taken", text: "mid-run autosave" }) : null,
  );

  const note = el("input", { type: "text", class: "note", placeholder: "Add a note…", spellcheck: "false" });
  note.value = meta.note;
  note.addEventListener("blur", () => commitNote(meta, note));
  note.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") note.blur();
    if (ev.key === "Escape") {
      note.value = meta.note;
      note.blur();
    }
  });

  const running = status?.running ?? false;
  const load = el("button", { class: "btn primary small", "data-load": meta.id, text: "Load" });
  load.disabled = running || busy;
  load.title = running ? "Close BTD6 first" : "";
  load.addEventListener("click", () => loadSnapshot(meta));
  const rename = el("button", { class: "btn small", text: "Rename" });
  rename.addEventListener("click", () => {
    name.focus();
    name.select();
  });
  const del = el("button", { class: "btn small danger-text", text: "Delete" });
  del.addEventListener("click", () => deleteSnapshot(meta));

  const loaded = el("span", { class: "loaded", "data-loaded": meta.lastRestoredAt ?? "" });
  if (meta.lastRestoredAt) {
    loaded.textContent = `Loaded ${relativeTime(meta.lastRestoredAt)}`;
    loaded.title = fullTime(meta.lastRestoredAt);
  }

  return el("article", { class: "row-card", "data-id": meta.id },
    el("div", { class: "line" }, name, time),
    info,
    note,
    el("div", { class: "actions" }, load, rename, del, loaded),
  );
}

function renderList() {
  list.replaceChildren();
  if (snapshots.length === 0) {
    list.append(
      el("div", { class: "empty" },
        el("p", { text: "No snapshots yet." }),
        el("p", { text: "A snapshot copies Profile.Save (Steam userdata) and MapSaveData (LocalLow) into a folder under saves/ next to this app. Take it from the BTD6 main menu after exiting a game, then load it later with BTD6 closed." }),
      ),
    );
    return;
  }
  for (const m of snapshots) list.append(renderRow(m));
}

function refreshTimes() {
  const now = Date.now();
  for (const t of list.querySelectorAll<HTMLElement>(".time[data-iso]")) {
    t.textContent = relativeTime(t.dataset.iso!, now);
  }
  for (const l of list.querySelectorAll<HTMLElement>(".loaded[data-loaded]")) {
    const iso = l.dataset.loaded;
    if (iso) l.textContent = `Loaded ${relativeTime(iso, now)}`;
  }
}

// ---- Boot ------------------------------------------------------------------------------

renderHeader();
renderList();
void refreshStatus().then(refreshList);
setInterval(refreshStatus, 2000);
