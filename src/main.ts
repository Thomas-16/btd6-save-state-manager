import { invoke } from "@tauri-apps/api/core";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import "@fontsource-variable/instrument-sans";
import "@fontsource-variable/jetbrains-mono";

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
  auto: boolean;
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
  mode: "merge" | "raw";
  applied: string[];
  backup?: Meta;
}

type ItemStatus = "add" | "replace" | "same" | "kept";

interface EventProgress {
  kind: "boss" | "odyssey";
  snapshot: string | null;
  live: string | null;
  changed: boolean;
}

interface GameItem {
  id: string;
  keys: string[];
  kind: "boss" | "odyssey" | "map";
  status: ItemStatus;
  label: string;
  snapshot: SavedMapInfo | null;
  live: SavedMapInfo | null;
  snapshotContinueRound: number | null;
  liveContinueRound: number | null;
  differentGame: boolean;
  event: EventProgress | null;
  defaultChecked: boolean;
}

interface CategoryDiff {
  id: string;
  label: string;
  changed: boolean;
  changedKeys: string[];
  summary: string;
}

interface LockedDiff {
  key: string;
  label: string;
  snapshot: string;
  live: string;
  differs: boolean;
}

interface RestorePreview {
  snapshotId: string;
  snapshotName: string;
  mergeAvailable: boolean;
  mergeError: string | null;
  games: GameItem[];
  categories: CategoryDiff[];
  locked: LockedDiff[];
  identical: boolean;
  snapshotGameVersion: string | null;
  liveGameVersion: string | null;
  mapSavePresent: boolean;
  mapSaveHasGame: boolean;
  mapSaveItem: string | null;
}

interface RestoreSelection {
  raw: boolean;
  games: string[];
  events: string[];
  categories: string[];
}

// ---- State -----------------------------------------------------------------------------

let status: Status | null = null;
let snapshots: Meta[] = [];
let selectedId: string | null = null;
let settingsAutoOpened = false;
let settingsDialogOpen = false;
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

function takenTime(iso: string): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  return new Date(t).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/** `FrozenOver` -> `Frozen Over`, mirrors `humanize_map_name` in saveformat.rs. */
function humanizeMap(name: string): string {
  let out = "";
  for (let i = 0; i < name.length; i++) {
    const c = name[i];
    if (i > 0 && c >= "A" && c <= "Z") {
      const prev = name[i - 1];
      const next = name[i + 1] ?? "";
      const prevLower = prev >= "a" && prev <= "z";
      const prevDigit = prev >= "0" && prev <= "9";
      const prevUpper = prev >= "A" && prev <= "Z";
      const nextLower = next >= "a" && next <= "z";
      if (prevLower || prevDigit || (prevUpper && nextLower)) out += " ";
    }
    out += c;
  }
  return out;
}

function gameKind(m: SavedMapInfo): string {
  if (m.boss) return "Boss";
  if (m.gameType === 3) return "Odyssey";
  return m.mode || "—";
}

function bossChip(m: SavedMapInfo): string | null {
  if (!m.boss) return null;
  let s = m.boss;
  if (m.elite) s += " · Elite";
  if (m.tier !== undefined && m.tier !== null) s += ` · T${m.tier}`;
  return s;
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

function selected(): Meta | null {
  return snapshots.find((s) => s.id === selectedId) ?? null;
}

function gearIcon(): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("width", "16");
  svg.setAttribute("height", "16");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.8");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  const circle = document.createElementNS(ns, "circle");
  circle.setAttribute("cx", "12");
  circle.setAttribute("cy", "12");
  circle.setAttribute("r", "3");
  const path = document.createElementNS(ns, "path");
  path.setAttribute(
    "d",
    "M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h0a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v0a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
  );
  svg.append(circle, path);
  return svg;
}

// ---- Dialogs ---------------------------------------------------------------------------

function modal(build: (close: () => void) => HTMLElement, cls = ""): Promise<void> {
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
    const box = el("div", { class: `modal ${cls}`, role: "dialog", "aria-modal": "true" }, build(close));
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
    const ok = el("button", { class: opts.danger ? "btn danger" : "btn ink", text: opts.okLabel });
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
    const name = el("input", { type: "text", class: "input", id: "snap-name", placeholder: "Snapshot name" });
    name.value = defaultName;
    const note = el("textarea", { class: "input", id: "snap-note", rows: "2", placeholder: "Note (optional)" });
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
    const ok = el("button", { class: "btn ink", text: "Save" });
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

/** Numeric strings get thousands separators; anything else is shown as-is. */
function fmtValue(s: string): string {
  return /^-?\d+(\.\d+)?$/.test(s) ? Number(s).toLocaleString() : s;
}

/** `Lych Elite T3 · Frozen Over` / `Lotus Island · Odyssey` / `Cubism · Easy Standard`, mirrors `describe` in saveformat.rs. */
function describeGame(m: SavedMapInfo): string {
  const map = humanizeMap(m.map);
  if (m.boss) {
    let head = m.boss;
    if (m.elite) head += " Elite";
    if (m.tier !== undefined && m.tier !== null) head += ` T${m.tier}`;
    return `${head} · ${map}`;
  }
  const kind = m.gameType === 3 ? "Odyssey" : m.mode.toLowerCase() === "standard" && m.difficulty ? `${m.difficulty} ${m.mode}` : m.mode;
  return `${map} · ${kind}`;
}

/** `bossMedals` -> `boss medals`, mirrors `humanize_key` in restore.rs. */
function humanizeKey(key: string): string {
  return humanizeMap(key).toLowerCase();
}

function keyList(keys: string[]): string {
  const shown = keys.slice(0, 4).map(humanizeKey);
  return keys.length > 4 ? `${shown.join(", ")} +${keys.length - 4} more` : shown.join(", ");
}

/**
 * The load dialog: a two-sided sheet, what the profile has now against what the snapshot
 * would put there, one row per saved game, event progress record, category and locked value.
 * A ticked row paints its outcome. Resolves with the selection or null when cancelled.
 */
function restoreDialog(meta: Meta, preview: RestorePreview): Promise<RestoreSelection | null> {
  let result: RestoreSelection | null = null;
  return modal((close) => {
    const root = el("div", { class: "rroot" });
    const mergeOk = preview.mergeAvailable;
    const versionDiffers =
      !!preview.snapshotGameVersion && !!preview.liveGameVersion && preview.snapshotGameVersion !== preview.liveGameVersion;

    // ---- Header: title and the mode switch --------------------------------------------
    const pickRadio = el("input", { type: "radio", name: "rmode", value: "pick" });
    const rawRadio = el("input", { type: "radio", name: "rmode", value: "raw" });
    pickRadio.checked = mergeOk;
    rawRadio.checked = !mergeOk;
    pickRadio.disabled = !mergeOk;
    const seg = el(
      "div",
      { class: "seg", role: "radiogroup", "aria-label": "Load mode" },
      el("label", {}, pickRadio, "Pick parts"),
      el("label", {}, rawRadio, el("span", { class: "rawword", text: "Whole file" })),
    );
    root.append(
      el(
        "div",
        { class: "rhead" },
        el("div", {}, el("h2", { text: `Load '${meta.name}'` }), el("p", { class: "hint", text: "A backup of the current save is taken first, so this can be undone." })),
        seg,
      ),
    );
    const body = el("div", { class: "rbody" });
    root.append(body);
    if (!meta.readable) {
      body.append(el("p", { class: "warn", text: "This snapshot could not be decrypted when it was taken. It may be corrupt." }));
    }

    // ---- Whole-file note, shown only in raw mode --------------------------------------
    const rawNote = el("div", { class: "raw-note" });
    const mm = preview.locked.find((l) => l.key === "monkeyMoney");
    const mmText = mm ? ` Monkey money goes ${fmtValue(mm.live)} → ${fmtValue(mm.snapshot)};` : "";
    if (!mergeOk) {
      rawNote.append(el("b", { text: "Only a whole-file overwrite is possible." }), ` ${preview.mergeError ?? "The profile could not be decoded"}. Every value will come from the snapshot, including monkey money and settings.`);
    } else {
      rawNote.append(el("b", { text: "Raw copy of the snapshot." }), ` Every value is replaced.${mmText} rank, trophies and settings follow. The sheet below is not consulted.`);
    }
    if (versionDiffers) {
      rawNote.append(el("p", { class: "warn", text: `Game version differs (${preview.snapshotGameVersion} vs ${preview.liveGameVersion}); a whole-file overwrite across versions may drop trophy store items.` }));
    }
    body.append(rawNote);

    // ---- The sheet ---------------------------------------------------------------------
    const boxes: HTMLInputElement[] = [];
    const tbody = el("tbody");
    const sheet = el("table", { class: "sheet" });
    if (mergeOk) {
      const side = (title: string, detail: string, cls: string) =>
        el("th", { class: `side ${cls}` }, el("span", { class: "ct", text: title }), el("span", { class: "cs", text: detail }));
      sheet.append(
        el("colgroup", {}, el("col", { class: "g" }), el("col", { class: "b" }), el("col"), el("col", { class: "now" }), el("col", { class: "snap" })),
        el(
          "thead",
          {},
          el(
            "tr",
            {},
            el("th"),
            el("th"),
            el("th", {}, el("span", { class: "ct", text: "Item" })),
            side("Now", `your profile${preview.liveGameVersion ? ` · game ${preview.liveGameVersion}` : ""}`, "now"),
            side("Snapshot", `${takenTime(meta.createdAt)}${preview.snapshotGameVersion ? ` · game ${preview.snapshotGameVersion}` : ""}`, versionDiffers ? "snap vdiff" : "snap"),
          ),
        ),
        tbody,
      );
    }

    const checkbox = (kind: "game" | "event" | "category", id: string, checked: boolean) => {
      const box = el("input", { type: "checkbox", "data-kind": kind, "data-id": id });
      box.checked = checked;
      boxes.push(box);
      return box;
    };
    const cell = (cls: string, ...children: (Node | string | null)[]) => el("td", { class: cls }, ...children);
    const value = (text: string, mono = false) => el("span", { class: mono ? "v mono" : "v", text });
    const detail = (text: string | null) => (text ? el("span", { class: "d", text }) : null);
    const pill = (text: string) => el("span", { class: "pill", text });
    const round = (r: number) => value(`R${r}`, true);
    const checkpoint = (r: number | null) => detail(r === null ? null : `continue checkpoint R${r}`);

    interface Row {
      cls: string;
      glyph: string;
      box: HTMLInputElement | null;
      group?: string;
      name: string;
      sub?: boolean;
      note?: (Node | string | null)[];
      now: (Node | string | null)[];
      snap: (Node | string | null)[];
    }
    const row = (r: Row) => {
      const tr = el("tr", { class: `${r.box ? "r" : "q"} ${r.cls}${r.sub ? " sub" : ""}`, "data-g": r.group });
      const item = cell("item", r.sub ? r.name : el("span", { class: "name", text: r.name }));
      for (const n of r.note ?? []) if (n) item.append(typeof n === "string" ? el("span", { class: "note", text: n }) : n);
      tr.append(cell("g", r.glyph), cell("b", r.box), item, cell("side now", ...r.now), cell("side snap", ...r.snap));
      return tr;
    };
    const group = (label: string, key: string | null, fold: string | null) => {
      const bar = el("div", { class: "bar" }, el("span", { class: "label", text: label }));
      if (key) {
        bar.append(
          el("span", { class: "ops" }, el("button", { type: "button", class: "lnk", "data-all": key, text: "all" }), el("button", { type: "button", class: "lnk", "data-none": key, text: "none" })),
        );
      }
      return el("tr", { class: "grp" }, el("td", { colspan: "5" }, bar, fold ? el("div", { class: "fold", text: fold }) : null));
    };

    if (mergeOk) {
      if (preview.identical) {
        body.append(el("p", { class: "hint empty", text: "Nothing selectable differs from your profile. Whole file is still available for a raw copy." }));
      }

      // In-progress games, with the tied event progress as a sub-row.
      const anyGame = preview.games.some((g) => g.status === "add" || g.status === "replace");
      tbody.append(group("In-progress games", anyGame ? "games" : null, null));
      if (preview.games.length === 0) {
        tbody.append(el("tr", { class: "none" }, el("td", { colspan: "5", text: "Neither profile holds a game in progress." })));
      }
      for (const g of preview.games) {
        const selectable = g.status === "add" || g.status === "replace";
        const note: (Node | string | null)[] = [];
        if (preview.mapSaveItem === g.id) note.push("Includes its mid-run autosave");
        if (g.differentGame && g.live) note.push(el("span", { class: "note warn", text: `Replaces a different run: ${describeGame(g.live)} R${g.live.round}` }));
        const snapSide = g.snapshot ? [round(g.snapshot.round), checkpoint(g.snapshotContinueRound)] : [value("not in snapshot")];
        let nowSide: (Node | string | null)[] = [];
        switch (g.status) {
          case "add":
            nowSide = [value("no game")];
            break;
          case "replace":
            nowSide = [round(g.live!.round), g.differentGame ? detail(describeGame(g.live!)) : checkpoint(g.liveContinueRound)];
            break;
          case "same":
            nowSide = [round(g.live!.round)];
            snapSide.push(pill("same"));
            break;
          case "kept":
            nowSide = [round(g.live!.round), pill("stays")];
            break;
        }
        const glyph = { add: "+", replace: "~", same: "=", kept: "·" }[g.status];
        const cls = { add: "add", replace: "rep", same: "", kept: "" }[g.status];
        const box = selectable ? checkbox("game", g.id, g.defaultChecked) : null;
        tbody.append(row({ cls, glyph, box, group: "games", name: g.label, note, now: nowSide, snap: snapSide }));

        if (g.event && (g.event.snapshot || g.event.live)) {
          const ev = g.event;
          const name = ev.kind === "boss" ? "Boss event progress" : "Odyssey progress";
          const evBox = ev.changed ? checkbox("event", g.id, g.defaultChecked) : null;
          const evRow = row({
            cls: ev.changed ? "rep" : "",
            glyph: "",
            box: evBox,
            group: "games",
            sub: true,
            name,
            now: [value(ev.live ?? "none")],
            snap: [value(ev.snapshot ?? "none in snapshot"), ev.changed ? null : pill("same")],
          });
          // An event row follows its run: unticking the run unticks its progress.
          if (box && evBox) {
            box.addEventListener("change", () => {
              evBox.checked = box.checked;
            });
          }
          tbody.append(evRow);
        }
      }

      // Whole-key categories, unticked by default; identical ones fold into the header.
      const changed = preview.categories.filter((c) => c.changed);
      const same = preview.categories.filter((c) => !c.changed);
      const fold = same.length === 0 ? null : changed.length === 0 ? "All identical already" : `Identical already: ${same.map((c) => c.label).join(" · ")}`;
      tbody.append(group("Progress & records", changed.length ? "cats" : null, fold));
      for (const c of changed) {
        const n = c.changedKeys.length;
        tbody.append(
          row({
            cls: "rep",
            glyph: "~",
            box: checkbox("category", c.id, false),
            group: "cats",
            name: c.label,
            note: [keyList(c.changedKeys)],
            now: [value("current values")],
            snap: [value(`${n} key${n === 1 ? "" : "s"}`)],
          }),
        );
      }

      // Locked values: monkey money always, others only when they differ.
      const locked = preview.locked.filter((l) => l.key === "monkeyMoney" || l.differs);
      if (locked.length) {
        tbody.append(group("Always kept as it is now", null, null));
        for (const l of locked) {
          tbody.append(row({ cls: "lock", glyph: "🔒", box: null, name: l.label, now: [value(fmtValue(l.live), true), pill("stays")], snap: [value(fmtValue(l.snapshot), true)] }));
        }
      }

      body.append(sheet);
      const legend = el("div", { class: "legend" });
      const key = (glyph: string, cls: string, text: string) => el("span", {}, el("span", { class: `k ${cls}`, text: glyph }), text);
      const swatch = (cls: string, text: string) => el("span", {}, el("span", { class: `sw ${cls}` }), text);
      legend.append(key("+", "a", "new"), key("~", "r", "replaces"), key("=", "", "same"), key("·", "", "not in snapshot"), swatch("in", "comes in"), swatch("out", "goes away"));
      body.append(legend);
    }

    // ---- Footer ------------------------------------------------------------------------
    const summary = el("span", { class: "rsum" });
    const ok = el("button", { class: "btn ink" });
    const cancel = el("button", { class: "btn", text: "Cancel" });
    cancel.addEventListener("click", close);

    const current = (): RestoreSelection => {
      const sel: RestoreSelection = { raw: rawRadio.checked, games: [], events: [], categories: [] };
      if (sel.raw) return sel;
      for (const b of boxes) {
        if (!b.checked) continue;
        const id = b.dataset.id!;
        if (b.dataset.kind === "game") sel.games.push(id);
        else if (b.dataset.kind === "event") sel.events.push(id);
        else sel.categories.push(id);
      }
      return sel;
    };
    const refresh = () => {
      const raw = rawRadio.checked;
      if (raw && !root.classList.contains("raw")) body.scrollTop = 0;
      root.classList.toggle("raw", raw);
      for (const b of boxes) {
        b.disabled = raw;
        b.closest("tr")?.classList.toggle("on", b.checked);
      }
      if (raw) {
        summary.replaceChildren(el("span", { class: "stat", text: "whole file" }), " backup first");
        ok.textContent = "Overwrite";
        ok.className = "btn danger";
        ok.disabled = false;
        return;
      }
      const sel = current();
      let add = 0;
      let rep = 0;
      for (const b of boxes) {
        if (b.checked && b.dataset.kind === "game") {
          if (b.closest("tr")?.classList.contains("add")) add++;
          else rep++;
        }
      }
      const n = sel.games.length + sel.events.length + sel.categories.length;
      if (n === 0) {
        summary.textContent = preview.identical ? "Nothing to load" : "Nothing selected";
      } else {
        const stat = el("span", { class: "stat" });
        if (add) stat.append(el("span", { class: "a", text: `+${add}` }));
        if (rep) stat.append(el("span", { class: "r", text: `~${rep}` }));
        const bits: string[] = [];
        if (sel.events.length) bits.push(`${sel.events.length} progress`);
        bits.push(sel.categories.length ? `${sel.categories.length} record set${sel.categories.length === 1 ? "" : "s"}` : "records untouched");
        summary.replaceChildren(stat.childElementCount ? stat : "", `${bits.join(" · ")} · backup first`);
      }
      ok.textContent = n ? `Load ${n} part${n === 1 ? "" : "s"}` : "Load";
      ok.className = "btn ink";
      ok.disabled = n === 0;
    };
    root.addEventListener("change", refresh);
    // Whole rows toggle their box; all/none set a whole group.
    root.addEventListener("click", (ev) => {
      const target = ev.target as HTMLElement;
      const lnk = target.closest<HTMLElement>(".lnk");
      if (lnk) {
        const key = lnk.dataset.all ?? lnk.dataset.none;
        const on = lnk.dataset.all !== undefined;
        for (const b of boxes) if (!b.disabled && b.closest("tr")?.dataset.g === key) b.checked = on;
        refresh();
        return;
      }
      if (target.tagName === "INPUT" || target.tagName === "LABEL" || target.tagName === "BUTTON") return;
      const tr = target.closest<HTMLTableRowElement>("tr.r");
      const box = tr?.querySelector<HTMLInputElement>("input[type=checkbox]");
      if (!box || box.disabled) return;
      box.checked = !box.checked;
      refresh();
    });
    refresh();
    ok.addEventListener("click", () => {
      result = current();
      close();
    });
    setTimeout(() => cancel.focus(), 0);
    root.append(el("div", { class: "modal-actions" }, summary, cancel, ok));
    return root;
  }, "restore").then(() => result);
}

function settingsDialog(): Promise<void> {
  if (settingsDialogOpen) return Promise.resolve();
  settingsDialogOpen = true;
  return modal((close) => {
    const profile = el("input", { type: "text", class: "input mono", id: "path-profile", placeholder: status?.defaultProfilePath ?? "" });
    const mapSave = el("input", { type: "text", class: "input mono", id: "path-mapsave", placeholder: status?.defaultMapSavePath ?? "" });
    invoke<Settings>("get_settings")
      .then((s) => {
        profile.value = s.profilePath ?? "";
        mapSave.value = s.mapSavePath ?? "";
      })
      .catch(() => {});

    const browse = (input: HTMLInputElement, title: string) => {
      const b = el("button", { class: "btn", text: "Browse…" });
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

    const apply = el("button", { class: "btn ink", text: "Apply" });
    apply.addEventListener("click", async () => {
      try {
        await invoke<Settings>("set_paths", { profilePath: profile.value, mapSavePath: mapSave.value });
        toast("Paths saved");
        close();
        await refreshStatus();
      } catch (e) {
        toast(errText(e), "err");
      }
    });
    const reset = el("button", { class: "btn ghost", text: "Use defaults" });
    reset.addEventListener("click", () => {
      profile.value = "";
      mapSave.value = "";
    });
    const cancel = el("button", { class: "btn", text: "Close" });
    cancel.addEventListener("click", close);

    return el(
      "div",
      {},
      el("h2", { text: "File paths" }),
      el("p", { class: "hint", text: "Leave a field empty to use the resolved default shown as placeholder." }),
      el("label", { class: "field" },
        el("span", {}, "Profile.Save ", found(status?.profileExists)),
        el("div", { class: "row-inline" }, profile, browse(profile, "Pick Profile.Save"))),
      el("label", { class: "field" },
        el("span", {}, "MapSaveData ", found(status?.mapSaveExists)),
        el("div", { class: "row-inline" }, mapSave, browse(mapSave, "Pick MapSaveData"))),
      el("p", { class: "muted mono", text: `Snapshots: ${status?.savesDir ?? ""}` }),
      el("div", { class: "modal-actions" }, reset, cancel, apply),
    );
  }, "wide").finally(() => {
    settingsDialogOpen = false;
  });
}

// ---- Actions ---------------------------------------------------------------------------

async function refreshStatus() {
  try {
    status = await invoke<Status>("get_status");
  } catch (e) {
    console.error(e);
  }
  renderTop();
  if (status && !status.profileExists && !settingsAutoOpened) {
    settingsAutoOpened = true;
    void settingsDialog();
  }
  refreshTimes();
}

async function refreshList() {
  try {
    snapshots = await invoke<Meta[]>("list_snapshots");
  } catch (e) {
    toast(errText(e), "err");
  }
  if (!snapshots.some((s) => s.id === selectedId)) selectedId = snapshots[0]?.id ?? null;
  renderList();
  renderDetail();
}

async function saveCurrent() {
  if (busy) return;
  if (!status?.profileExists) {
    toast("Profile.Save not found. Set the path in settings.", "err");
    return;
  }
  const live = status?.live ?? null;
  const defaultName = live?.summary ?? `Snapshot ${new Date().toLocaleString()}`;
  const hint = status?.running ? "BTD6 is running. Best taken from the main menu." : null;
  const r = await nameDialog(defaultName, hint);
  if (!r) return;
  busy = true;
  renderTop();
  try {
    const meta = await invoke<Meta>("create_snapshot", { name: r.name, note: r.note });
    toast(meta.readable ? `Saved "${meta.name}"` : `Saved "${meta.name}" (unreadable, may be corrupt)`,
      meta.readable ? "ok" : "err");
    selectedId = meta.id;
    await refreshList();
  } catch (e) {
    toast(`Save failed: ${errText(e)}`, "err");
  } finally {
    busy = false;
    renderTop();
  }
}

async function loadSnapshot(meta: Meta) {
  if (busy) return;
  if (status?.running) {
    toast("Close BTD6 first", "err");
    return;
  }
  let preview: RestorePreview;
  try {
    preview = await invoke<RestorePreview>("preview_restore", { id: meta.id });
  } catch (e) {
    toast(`Cannot read snapshot: ${errText(e)}`, "err");
    return;
  }
  const selection = await restoreDialog(meta, preview);
  if (!selection) return;
  busy = true;
  renderTop();
  try {
    const report = await invoke<RestoreReport>("restore_snapshot", { id: meta.id, selection });
    const what = report.mode === "raw" ? "whole file" : `${report.applied.length} change${report.applied.length === 1 ? "" : "s"}`;
    toast(`Loaded "${report.meta.name}" (${what}) · backup saved`);
    await refreshList();
    await refreshStatus();
  } catch (e) {
    toast(`Load failed: ${errText(e)}`, "err");
    await refreshList();
  } finally {
    busy = false;
    renderTop();
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
    renderList();
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

async function openSavesDir() {
  try {
    await invoke("open_saves_dir");
  } catch (e) {
    toast(errText(e), "err");
  }
}

function select(id: string) {
  if (id === selectedId) return;
  selectedId = id;
  renderList();
  renderDetail();
}

// ---- Rendering -------------------------------------------------------------------------

const top = el("header", { class: "top" });
const body = el("div", { class: "body" });
const list = el("aside", { class: "list" });
const rows = el("div", { class: "rows", role: "listbox", "aria-label": "Snapshots" });
const listHead = el("div", { class: "lh" });
const listFoot = el("div", { class: "lf" });
const detail = el("section", { class: "detail" });
list.append(listHead, rows, listFoot);
body.append(list, detail);
app.append(top, body);

let topSignature = "";

function renderTop() {
  const running = status?.running ?? false;
  const signature = JSON.stringify([
    status !== null,
    running,
    busy,
    status?.profileExists,
    status?.liveError,
    status?.live?.gameVersion,
  ]);
  if (signature === topSignature) return;
  topSignature = signature;
  top.replaceChildren();

  let dotClass = "";
  let text = "Checking…";
  let title = "";
  let warnText = false;
  if (status) {
    if (!status.profileExists) {
      dotClass = "bad";
      text = "Profile.Save not found · set the path in settings";
      warnText = true;
    } else if (running) {
      dotClass = "bad";
      text = `${status.processName} running · close it before loading`;
    } else if (status.liveError) {
      dotClass = "warn";
      text = "BTD6 closed · live profile unreadable";
      title = status.liveError;
      warnText = true;
    } else {
      dotClass = "good";
      const ver = status.live?.gameVersion ? ` · v${status.live.gameVersion}` : "";
      text = `BTD6 closed · profile found${ver}`;
    }
  }
  const statusEl = el("span", { class: "status", title: title || undefined },
    el("span", { class: `dot ${dotClass}` }),
    el("span", { class: warnText ? "warn-text" : undefined, text }),
  );

  const gear = el("button", { class: "btn icon", title: "Settings (file paths)", "aria-label": "Settings" }, gearIcon());
  gear.addEventListener("click", () => void settingsDialog());

  const save = el("button", { class: "btn ink" }, busy ? "Working…" : "Save current", el("kbd", { text: "Ctrl S" }));
  save.disabled = busy || !status?.profileExists;
  if (!status?.profileExists) save.title = "Profile.Save not found. Set the path in settings.";
  save.addEventListener("click", saveCurrent);

  top.append(el("span", { class: "brand", text: "Save States" }), statusEl, el("span", { class: "spacer" }), gear, save);

  // Keep the Load button in sync with the running state without rebuilding the pane.
  const load = detail.querySelector<HTMLButtonElement>("button[data-load]");
  if (load) {
    load.disabled = running || busy;
    load.title = running ? "Close BTD6 first" : "";
  }
}

function renderRow(meta: Meta): HTMLElement {
  const sub: Node[] = [];
  const liveVer = status?.live?.gameVersion ?? null;
  if (meta.gameVersion) {
    const mismatch = liveVer !== null && liveVer !== meta.gameVersion;
    sub.push(el("span", { class: mismatch ? "warn" : undefined, text: mismatch ? `v${meta.gameVersion} · differs from live` : `v${meta.gameVersion}` }));
  }
  if (meta.auto) sub.push(el("span", { class: "tag", text: "auto backup" }));
  if (!meta.readable) sub.push(el("span", { class: "bad", text: "unreadable" }));
  if (meta.mapSaveHasGame) sub.push(el("span", { text: "mid-run autosave" }));
  if (meta.lastRestoredAt) {
    sub.push(el("span", { class: "good loaded-at", "data-loaded": meta.lastRestoredAt, text: `loaded ${relativeTime(meta.lastRestoredAt)}` }));
  }

  const row = el("button", {
    class: `row${meta.id === selectedId ? " sel" : ""}`,
    role: "option",
    "aria-selected": meta.id === selectedId ? "true" : "false",
    "data-id": meta.id,
    tabindex: meta.id === selectedId ? "0" : "-1",
  },
    el("span", { class: "nm", text: meta.name }),
    el("span", { class: "tm time", "data-iso": meta.createdAt, title: fullTime(meta.createdAt), text: relativeTime(meta.createdAt) }),
    sub.length ? el("span", { class: "sub" }, ...sub) : null,
  );
  row.addEventListener("click", () => select(meta.id));
  return row;
}

function renderList() {
  listHead.replaceChildren(
    el("span", { class: "label", text: "Snapshots" }),
    el("span", { class: "label", text: String(snapshots.length) }),
  );
  rows.replaceChildren();
  if (snapshots.length === 0) {
    rows.append(el("div", { class: "list-empty", text: "No snapshots yet." }));
  } else {
    for (const m of snapshots) rows.append(renderRow(m));
  }
  const openBtn = el("button", { class: "btn link", text: "Open folder" });
  openBtn.addEventListener("click", openSavesDir);
  const count = snapshots.length === 1 ? "1 folder" : `${snapshots.length} folders`;
  listFoot.replaceChildren(
    el("span", { class: "path", title: status?.savesDir ?? "", text: `saves\\ · ${count}` }),
    openBtn,
  );
}

function renderDetail() {
  detail.replaceChildren();
  const meta = selected();
  if (!meta) {
    detail.append(
      el("div", { class: "detail-empty" },
        el("h2", { text: snapshots.length ? "Pick a snapshot" : "No snapshots yet" }),
        el("p", { text: "A snapshot copies Profile.Save (Steam userdata) and MapSaveData (LocalLow) into a folder under saves/ next to this app." }),
        el("p", { text: "Take one from the BTD6 main menu after exiting a game, then load it later with BTD6 closed." }),
      ),
    );
    return;
  }

  const name = el("input", { type: "text", class: "title", id: "detail-name", title: "Click to rename", spellcheck: "false", "aria-label": "Snapshot name" });
  name.value = meta.name;
  name.addEventListener("blur", () => commitName(meta, name));
  name.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") name.blur();
    if (ev.key === "Escape") {
      name.value = meta.name;
      name.blur();
    }
  });

  const when = el("div", { class: "when" },
    `Taken ${takenTime(meta.createdAt)} · `,
    el("span", { class: "time", "data-iso": meta.createdAt, title: fullTime(meta.createdAt), text: relativeTime(meta.createdAt) }),
  );

  const note = el("input", { type: "text", class: "note", id: "detail-note", placeholder: "Add a note…", spellcheck: "false", "aria-label": "Note" });
  note.value = meta.note;
  note.addEventListener("blur", () => commitNote(meta, note));
  note.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") note.blur();
    if (ev.key === "Escape") {
      note.value = meta.note;
      note.blur();
    }
  });
  const noteLine = el("div", { class: "note-line" }, el("span", { class: "label", text: "Note" }), note);

  const liveVer = status?.live?.gameVersion ?? null;
  const mismatch = !!(liveVer && meta.gameVersion && liveVer !== meta.gameVersion);
  const stat = (k: string, v: Node) => el("div", { class: "st" }, el("div", { class: "label", text: k }), v);
  const stats = el("div", { class: "stats" },
    stat("Rank", el("div", { class: "v mono", text: meta.rank !== null && meta.rank !== undefined ? String(meta.rank) : "—" })),
    stat("Monkey money", el("div", { class: "v mono", text: meta.monkeyMoney !== null && meta.monkeyMoney !== undefined ? meta.monkeyMoney.toLocaleString() : "—" })),
    stat("Game version", el("div", { class: `v mono${mismatch ? " warn" : ""}`, text: meta.gameVersion ?? "—" })),
    stat("Files", el("div", { class: "v" }, meta.mapSaveDataPresent ? "2" : "1",
      el("small", { text: meta.mapSaveDataPresent ? "Profile + MapSave" : "Profile only" }))),
  );

  const notices: HTMLElement[] = [];
  if (meta.auto) {
    notices.push(el("div", { class: "notice", text: "Taken automatically right before a snapshot was loaded. Load it to undo that restore." }));
  }
  if (!meta.readable) {
    notices.push(el("div", { class: "notice bad", title: meta.error ?? "", text: "This snapshot could not be decrypted when it was taken. It may be corrupt." }));
  }
  if (mismatch) {
    notices.push(el("div", { class: "notice warn", text: `Taken on game version ${meta.gameVersion}; the live profile is ${liveVer}. Loading across versions may drop trophy store items.` }));
  }
  if (meta.mapSaveHasGame) {
    notices.push(el("div", { class: "notice", text: "MapSaveData held a mid-run autosave when this snapshot was taken." }));
  }

  const games = el("div", { class: "games" }, el("span", { class: "label", text: "In-progress games in this snapshot" }));
  if (meta.savedMaps.length === 0) {
    games.append(el("div", { class: "none", text: meta.readable ? "None. The profile had no games in progress." : "Unknown, the profile could not be read." }));
  } else {
    const sorted = [...meta.savedMaps].sort((a, b) => b.gameId - a.gameId || b.attemptId - a.attemptId);
    const table = el("table", {},
      el("tr", {}, el("th", { text: "Map" }), el("th", { text: "Mode" }), el("th", { text: "Difficulty" }), el("th", { class: "r", text: "Round" })),
    );
    for (const m of sorted) {
      const chip = bossChip(m);
      table.append(
        el("tr", { class: m.key === meta.lastPlayedKey ? "last" : undefined, title: m.key === meta.lastPlayedKey ? "Last played" : undefined },
          el("td", {}, humanizeMap(m.map), chip ? el("span", { class: "chip", text: chip }) : null),
          el("td", { text: gameKind(m) }),
          el("td", { text: m.difficulty || "—" }),
          el("td", { class: "r mono", text: String(m.round) }),
        ),
      );
    }
    games.append(table);
  }

  const running = status?.running ?? false;
  const load = el("button", { class: "btn ink", "data-load": meta.id, text: "Load this snapshot" });
  load.disabled = running || busy;
  load.title = running ? "Close BTD6 first" : "";
  load.addEventListener("click", () => loadSnapshot(meta));
  const rename = el("button", { class: "btn", text: "Rename" });
  rename.addEventListener("click", () => {
    name.focus();
    name.select();
  });
  const del = el("button", { class: "btn ghost danger-text", text: "Delete" });
  del.addEventListener("click", () => deleteSnapshot(meta));
  const loaded = el("span", { class: "loaded loaded-at", "data-loaded": meta.lastRestoredAt ?? "" });
  if (meta.lastRestoredAt) {
    loaded.textContent = `Last loaded ${relativeTime(meta.lastRestoredAt)}`;
    loaded.title = fullTime(meta.lastRestoredAt);
  }
  const actions = el("div", { class: "actions" }, load, rename, del, loaded);

  detail.append(name, when, noteLine, stats, ...notices, games, actions);
}

function refreshTimes() {
  const now = Date.now();
  for (const t of app.querySelectorAll<HTMLElement>(".time[data-iso]")) {
    t.textContent = relativeTime(t.dataset.iso!, now);
  }
  for (const l of app.querySelectorAll<HTMLElement>(".loaded-at[data-loaded]")) {
    const iso = l.dataset.loaded;
    if (!iso) continue;
    const rel = relativeTime(iso, now);
    l.textContent = l.classList.contains("loaded") ? `Last loaded ${rel}` : `loaded ${rel}`;
  }
}

// ---- Keyboard --------------------------------------------------------------------------

rows.addEventListener("keydown", (ev) => {
  if (ev.key !== "ArrowDown" && ev.key !== "ArrowUp") return;
  if (snapshots.length === 0) return;
  ev.preventDefault();
  const i = snapshots.findIndex((s) => s.id === selectedId);
  const next = ev.key === "ArrowDown" ? Math.min(snapshots.length - 1, i + 1) : Math.max(0, i - 1);
  select(snapshots[next].id);
  rows.querySelector<HTMLElement>(`.row[data-id="${CSS.escape(snapshots[next].id)}"]`)?.focus();
});

document.addEventListener("keydown", (ev) => {
  if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "s") {
    ev.preventDefault();
    if (!document.querySelector(".overlay")) void saveCurrent();
  }
});

// ---- Boot ------------------------------------------------------------------------------

renderTop();
renderList();
renderDetail();
void refreshStatus().then(refreshList);
setInterval(refreshStatus, 2000);
setInterval(refreshTimes, 30_000);
