// The global workspace switcher. Its own small page with no framework: the
// window is created on first use and kept, so it stays light.
import { invoke } from "@tauri-apps/api/core";

type Workspace = {
  id: string;
  name: string;
  isActive: boolean;
  isTrusted: boolean;
  isArchived: boolean;
  latest: { createdAt: string } | null;
};

const q = document.getElementById("q") as HTMLInputElement;
const list = document.getElementById("list")!;
const note = document.getElementById("note") as HTMLInputElement;
const noteRow = document.getElementById("note-row")!;
const noteLabel = document.getElementById("note-label")!;
let all: Workspace[] = [];
let shown: Workspace[] = [];
let index = 0;

function color(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return `hsl(${h % 360} 52% 50%)`;
}

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
function ago(iso: string): string {
  const s = Math.round((new Date(iso).getTime() - Date.now()) / 1000);
  for (const [u, n] of [["day", 86400], ["hour", 3600], ["minute", 60]] as const) {
    if (Math.abs(s) >= n) return rtf.format(Math.round(s / n), u);
  }
  return rtf.format(s, "second");
}

const switchable = (w: Workspace) => w.latest !== null && w.isTrusted;

function render() {
  list.replaceChildren();
  if (!shown.length) {
    const none = document.createElement("div");
    none.className = "none";
    none.textContent = all.length ? "No workspace matches." : "No workspaces yet. Create one in ContextFlow.";
    list.append(none);
    return;
  }
  shown.forEach((w, i) => {
    const row = document.createElement("button");
    row.className = "row";
    row.setAttribute("role", "option");
    row.setAttribute("aria-selected", String(i === index));
    row.disabled = !switchable(w);
    const dot = document.createElement("span");
    dot.className = "dot";
    dot.style.background = color(w.id);
    dot.textContent = w.name.charAt(0);
    const text = document.createElement("span");
    text.className = "text";
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = w.name;
    const meta = document.createElement("span");
    meta.className = "meta";
    meta.textContent = !w.latest
      ? "Nothing saved yet"
      : !w.isTrusted
        ? "Imported: approve it in ContextFlow first"
        : w.isActive
          ? "You're here now"
          : `Saved ${ago(w.latest.createdAt)}`;
    text.append(name, meta);
    row.append(dot, text);
    row.addEventListener("mousemove", () => {
      if (index !== i) {
        index = i;
        mark();
      }
    });
    row.addEventListener("click", () => choose(w));
    list.append(row);
  });
}

function mark() {
  [...list.children].forEach((el, i) => el.setAttribute("aria-selected", String(i === index)));
  list.children[index]?.scrollIntoView({ block: "nearest" });
}

function filter() {
  const words = q.value.toLowerCase().split(/\s+/).filter(Boolean);
  shown = all.filter((w) => words.every((x) => w.name.toLowerCase().includes(x)));
  // Start on the first one you can switch *to* (not the one you're already in).
  const away = shown.findIndex((w) => switchable(w) && !w.isActive);
  index = Math.max(0, away >= 0 ? away : shown.findIndex(switchable));
  render();
}

function choose(w: Workspace | undefined) {
  if (!w || !switchable(w)) return;
  // A note for the workspace you're leaving, shown first when you come back.
  void invoke("quick_switch", { target: w.id, note: note.value.trim() || null });
}

async function open() {
  q.value = "";
  try {
    all = (await invoke<Workspace[]>("list_workspaces", { includeArchived: false })).filter((w) => !w.isArchived);
  } catch {
    all = [];
  }
  note.value = "";
  const here = all.find((w) => w.isActive);
  noteRow.hidden = !here;
  if (here) noteLabel.textContent = `Note for “${here.name}” (optional)`;
  filter();
  q.focus();
}

q.addEventListener("input", filter);
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") void invoke("hide_switcher");
  else if (e.key === "ArrowDown") {
    e.preventDefault();
    index = Math.min(index + 1, shown.length - 1);
    mark();
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    index = Math.max(index - 1, 0);
    mark();
  } else if (e.key === "Enter") choose(shown[index]);
});

// The picker is a fresh window each time it opens.
void open();
